// baritone-rs engine: the thread that runs searches. Written for this project (upstream starts a thread per search from
// `PathingBehavior.findPathInNewThread`; see docs/PLAN.md "How it runs"). LGPL-3.0-or-later, see lib.rs.
//
// One worker thread, one search at a time, over an owned snapshot of the world. A new job replaces a job that waits and stops the
// one that runs. A cancelled search delivers nothing. The clock is the caller's.
use crate::process::pathing::{run_search, SearchJob, SearchOutcome, SearchRunner};
use crate::search::{PathCalculationResult, ResultType};
use crate::world::WorldView;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

/// How long one search took on the worker thread. Not upstream; for the in-game measurements (tools/l3).
#[derive(Clone, Copy, Debug)]
pub struct SearchLogEntry {
    pub generation: u64,
    pub micros: u64,
    pub kind: ResultType,
    /// Cancelled before it was delivered.
    pub cancelled: bool,
    /// The number of positions of the path found, 0 for none.
    pub path_positions: usize,
}

/// The most search log entries kept when nobody reads them (not upstream; the log is for the measurements).
const LOG_CAP: usize = 4096;

struct Inner<S> {
    pending: Option<(SearchJob, S)>,
    running_cancel: Option<Arc<AtomicBool>>,
    results: VecDeque<SearchOutcome>,
    log: Vec<SearchLogEntry>,
    shutdown: bool,
}

struct Shared<S> {
    inner: Mutex<Inner<S>>,
    wake: Condvar,
}

fn lock<S>(m: &Mutex<Inner<S>>) -> MutexGuard<'_, Inner<S>> {
    // a panic while holding the lock is only possible in this file's own short sections; take the data as it is
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A clock in milliseconds from the moment of the call, from the system's monotonic clock. `Worker::spawn` takes any clock; this is the real one.
pub fn monotonic_ms_clock() -> impl Fn() -> i64 + Send + 'static {
    let origin = std::time::Instant::now();
    move || origin.elapsed().as_millis() as i64
}

/// The search thread. Dropping it stops the thread: the search that runs is cancelled, and the thread is joined.
pub struct Worker<S: WorldView + Send + 'static> {
    shared: Arc<Shared<S>>,
    handle: Option<JoinHandle<()>>,
}

impl<S: WorldView + Send + 'static> Worker<S> {
    /// Starts the thread. `clock` returns milliseconds, as `System.currentTimeMillis` does for upstream's timeouts.
    pub fn spawn(clock: impl Fn() -> i64 + Send + 'static) -> Worker<S> {
        Self::try_spawn(clock).expect("the OS gave no thread")
    }

    /// `spawn`, but a refusal by the OS to start the thread is an error and not a panic (the glue says so in the chat).
    pub fn try_spawn(clock: impl Fn() -> i64 + Send + 'static) -> std::io::Result<Worker<S>> {
        let shared = Arc::new(Shared { inner: Mutex::new(Inner { pending: None, running_cancel: None, results: VecDeque::new(), log: Vec::new(), shutdown: false }), wake: Condvar::new() });
        let sh = Arc::clone(&shared);
        let handle = std::thread::Builder::new().name("baritone-search".to_string()).spawn(move || Self::run(sh, clock))?;
        Ok(Worker { shared, handle: Some(handle) })
    }

    fn run(shared: Arc<Shared<S>>, clock: impl Fn() -> i64) {
        loop {
            let (job, snapshot, cancel) = {
                let mut g = lock(&shared.inner);
                loop {
                    if g.shutdown {
                        return;
                    }
                    if let Some((job, snap)) = g.pending.take() {
                        let cancel = Arc::new(AtomicBool::new(false));
                        g.running_cancel = Some(Arc::clone(&cancel));
                        break (job, snap, cancel);
                    }
                    g = shared.wake.wait(g).unwrap_or_else(|e| e.into_inner());
                }
            };
            let generation = job.generation;
            let began = std::time::Instant::now();
            let outcome = catch_unwind(AssertUnwindSafe(|| run_search(&job, &snapshot, &cancel, &mut || clock()))).unwrap_or(SearchOutcome { generation, result: PathCalculationResult { kind: ResultType::Exception, path: None, error: None } });
            let micros = began.elapsed().as_micros() as u64;
            let mut g = lock(&shared.inner);
            g.running_cancel = None;
            if g.log.len() >= LOG_CAP {
                g.log.remove(0);
            }
            g.log.push(SearchLogEntry { generation, micros, kind: outcome.result.kind, cancelled: cancel.load(Ordering::SeqCst), path_positions: outcome.result.path.as_ref().map_or(0, |p| p.positions().len()) });
            // a cancelled search never delivers, even if it finished a path before it saw the flag
            if !cancel.load(Ordering::SeqCst) {
                g.results.push_back(outcome);
            }
        }
    }
}

impl<S: WorldView + Send + 'static> Worker<S> {
    /// The searches that finished since the last call, with their times. Not upstream.
    pub fn take_log(&self) -> Vec<SearchLogEntry> {
        std::mem::take(&mut lock(&self.shared.inner).log)
    }
}

impl<S: WorldView + Send + 'static> SearchRunner<S> for Worker<S> {
    fn start(&mut self, job: SearchJob, snapshot: S) {
        let mut g = lock(&self.shared.inner);
        // the job that waits is replaced, and the one that runs is no longer wanted
        g.pending = Some((job, snapshot));
        if let Some(c) = &g.running_cancel {
            c.store(true, Ordering::SeqCst);
        }
        self.shared.wake.notify_one();
    }

    fn cancel(&mut self) {
        let mut g = lock(&self.shared.inner);
        g.pending = None;
        if let Some(c) = &g.running_cancel {
            c.store(true, Ordering::SeqCst);
        }
    }

    fn poll(&mut self) -> Option<SearchOutcome> {
        lock(&self.shared.inner).results.pop_front()
    }
}

impl<S: WorldView + Send + 'static> Drop for Worker<S> {
    fn drop(&mut self) {
        {
            let mut g = lock(&self.shared.inner);
            g.shutdown = true;
            g.pending = None;
            if let Some(c) = &g.running_cancel {
                c.store(true, Ordering::SeqCst);
            }
            self.shared.wake.notify_all();
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::Goal;
    use crate::movement::helper::Border;
    use crate::pos::Pos;
    use crate::process::pathing::Tables;
    use crate::testing::{Fixture, TestWorld};
    use std::time::{Duration, Instant};

    fn job(generation: u64, goal_x: i32, tables: &Arc<Tables>, max_nodes: Option<u32>) -> SearchJob {
        SearchJob { generation, start: Pos::new(1, 1, 1), goal: Goal::Block { x: goal_x, y: 1, z: 1 }, previous: None, feet: Pos::new(1, 1, 1), tables: Arc::clone(tables), border: Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 }, primary_timeout_ms: 500, failure_timeout_ms: 2000, max_nodes }
    }

    fn tables() -> Arc<Tables> {
        let fx = Fixture::new();
        Arc::new(Tables::new(fx.states.clone(), fx.cfg.clone(), vec![1.0, 1.0]))
    }

    fn wait_for(w: &mut Worker<TestWorld>, n: usize) -> Vec<SearchOutcome> {
        let t = Instant::now();
        let mut got = vec![];
        while got.len() < n && t.elapsed() < Duration::from_secs(20) {
            match w.poll() {
                Some(o) => got.push(o),
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        got
    }

    #[test]
    fn a_search_runs_in_the_thread_and_delivers_its_path() {
        let mut w = Worker::spawn(monotonic_ms_clock());
        let t = tables();
        w.start(job(7, 12, &t, None), TestWorld::flat(20, 5));
        let out = wait_for(&mut w, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].generation, 7);
        assert_eq!(out[0].result.kind, ResultType::SuccessToGoal);
        assert_eq!(out[0].result.path.as_ref().unwrap().dest(), Pos::new(12, 1, 1));
    }

    #[test]
    fn a_new_job_replaces_the_one_that_waits_and_stops_the_one_that_runs() {
        // a world big enough that the first search is still going when the others arrive: it has no goal it can reach
        let mut w = Worker::spawn(monotonic_ms_clock());
        let t = tables();
        let big = TestWorld::flat(200, 200);
        let mut slow = job(1, 5000, &t, None);
        slow.primary_timeout_ms = 60_000;
        slow.failure_timeout_ms = 60_000;
        w.start(slow, big.clone());
        std::thread::sleep(Duration::from_millis(20));
        // two more in a row: only the last one runs
        w.start(job(2, 6, &t, None), TestWorld::flat(20, 5));
        w.start(job(3, 9, &t, None), TestWorld::flat(20, 5));
        let out = wait_for(&mut w, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].generation, 3);
        // nothing else comes: neither the stopped one nor the replaced one
        std::thread::sleep(Duration::from_millis(100));
        assert!(w.poll().is_none());
    }

    #[test]
    fn a_cancelled_search_delivers_nothing() {
        let mut w = Worker::spawn(monotonic_ms_clock());
        let t = tables();
        let mut slow = job(1, 5000, &t, None);
        slow.primary_timeout_ms = 60_000;
        slow.failure_timeout_ms = 60_000;
        w.start(slow, TestWorld::flat(200, 200));
        std::thread::sleep(Duration::from_millis(20));
        w.cancel();
        // the worker is free again and runs the next job
        w.start(job(2, 6, &t, None), TestWorld::flat(20, 5));
        let out = wait_for(&mut w, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].generation, 2);
        std::thread::sleep(Duration::from_millis(50));
        assert!(w.poll().is_none());
        // cancel before anything runs: the waiting job is dropped
        w.cancel();
        assert!(w.poll().is_none());
    }

    #[test]
    fn the_clock_is_the_callers() {
        // a clock that jumps far ahead after the first call makes the search time out at once, with the best path it has
        use std::sync::atomic::AtomicI64;
        let ticks = Arc::new(AtomicI64::new(0));
        let c = Arc::clone(&ticks);
        let mut w: Worker<TestWorld> = Worker::spawn(move || c.fetch_add(100_000, Ordering::SeqCst));
        let t = tables();
        w.start(job(1, 150, &t, None), TestWorld::flat(200, 5));
        let out = wait_for(&mut w, 1);
        assert_eq!(out.len(), 1);
        assert!(ticks.load(Ordering::SeqCst) > 0, "the clock was never read");
        assert_ne!(out[0].result.kind, ResultType::SuccessToGoal);
    }

    #[test]
    fn dropping_the_worker_stops_a_running_search() {
        let t = tables();
        let mut w = Worker::spawn(monotonic_ms_clock());
        let mut slow = job(1, 5000, &t, None);
        slow.primary_timeout_ms = 600_000;
        slow.failure_timeout_ms = 600_000;
        w.start(slow, TestWorld::flat(200, 200));
        std::thread::sleep(Duration::from_millis(20));
        let t0 = Instant::now();
        drop(w);
        assert!(t0.elapsed() < Duration::from_secs(10), "the drop waited for the search");
    }
}
