// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/cache/CachedWorld.java (the PackerThread, the save timer, `queueForPacking`),
// WorldData.java and the world cache of WorldProvider.java. LGPL-3.0-or-later, see lib.rs.
//
// Two threads per cached world, started when it is first opened and living as long as the process, as upstream's executor tasks do: one packs the
// chunks that are queued, one saves the world 30 seconds after the start and every 10 minutes. Leaving a world saves it on a thread of its own
// (`WorldData.onClose`). None of this runs on the game thread.
use super::chunk::{pack, PackSource};
use super::world::CachedWorld;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A chunk waiting to be packed.
pub struct PackJob {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub source: Box<dyn PackSource + Send>,
}

#[derive(Default)]
struct Queue {
    /// The positions in the order they were queued (`toPackQueue`).
    order: VecDeque<(i32, i32)>,
    /// The chunk of each position (`toPackMap`): a newer one replaces the one that waits.
    jobs: HashMap<(i32, i32), PackJob>,
    stop: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    /// The packer waits on this one and the saver on `tick`: one condition variable for both lost a wake-up when `notify_one` woke the saver (review finding 2).
    wake: Condvar,
    tick: Condvar,
    packed: std::sync::atomic::AtomicU64,
    dropped: std::sync::atomic::AtomicU64,
    /// Chunks packed to what the cache already held, so not stored.
    unchanged: std::sync::atomic::AtomicU64,
    /// Microseconds spent in `pack`, in all and the longest.
    pack_us: std::sync::atomic::AtomicU64,
    pack_max_us: std::sync::atomic::AtomicU64,
    /// `allowWalkOnMagmaBlocks`, read at each pack.
    allow_walk_on_magma: AtomicBool,
}

/// `WorldData`: a cached world with its packer and saver threads.
pub struct WorldData {
    pub world: Arc<CachedWorld>,
    shared: Arc<Shared>,
}

impl WorldData {
    /// Upstream's `WorldData.<init>` and `CachedWorld.<init>`'s two executor tasks.
    pub fn start(world: Arc<CachedWorld>) -> std::io::Result<WorldData> {
        let shared = Arc::new(Shared { queue: Mutex::new(Queue::default()), wake: Condvar::new(), tick: Condvar::new(), packed: Default::default(), dropped: Default::default(), unchanged: Default::default(), pack_us: Default::default(), pack_max_us: Default::default(), allow_walk_on_magma: AtomicBool::new(false) });
        {
            let (shared, world) = (Arc::clone(&shared), Arc::clone(&world));
            std::thread::Builder::new().name("baritone-rs-pack".into()).spawn(move || packer(&shared, &world))?;
        }
        {
            let (shared, world) = (Arc::clone(&shared), Arc::clone(&world));
            std::thread::Builder::new().name("baritone-rs-cache-save".into()).spawn(move || saver(&shared, &world))?;
        }
        Ok(WorldData { world, shared })
    }

    /// `CachedWorld.queueForPacking`: upstream's method.
    pub fn queue_for_packing(&self, job: PackJob) {
        let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        let pos = (job.chunk_x, job.chunk_z);
        if q.jobs.insert(pos, job).is_none() {
            q.order.push_back(pos);
        }
        drop(q);
        self.shared.wake.notify_one();
    }

    pub fn set_allow_walk_on_magma(&self, on: bool) {
        self.shared.allow_walk_on_magma.store(on, Ordering::Relaxed);
    }

    /// Whether the queue has room for more: under half of `chunkPackerQueueMaxSize`. The glue queues a chunk only when it has, so a flood (a world entered with a
    /// far render distance) is not thrown away by the over-full rule; the chunks it did not queue are tried again on the next tick (review finding 5).
    pub fn has_room(&self) -> bool {
        let max = i64::from(self.world.settings().chunk_packer_queue_max_size).max(2);
        (self.pending() as i64) * 2 < max
    }

    /// Chunks waiting to be packed.
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().unwrap_or_else(|e| e.into_inner()).order.len()
    }

    /// Chunks packed since the start, and chunks thrown away because the queue was over its limit.
    pub fn counts(&self) -> (u64, u64) {
        (self.shared.packed.load(Ordering::Relaxed), self.shared.dropped.load(Ordering::Relaxed))
    }

    /// Chunks packed that the cache already held in the same state, the time `pack` took in all and the longest one, in microseconds.
    pub fn pack_stats(&self) -> (u64, u64, u64) {
        (self.shared.unchanged.load(Ordering::Relaxed), self.shared.pack_us.load(Ordering::Relaxed), self.shared.pack_max_us.load(Ordering::Relaxed))
    }

    /// `WorldData.onClose`: saves the world on a thread of its own.
    pub fn on_close(&self) {
        let world = Arc::clone(&self.world);
        let _ = std::thread::Builder::new().name("baritone-rs-cache-close".into()).spawn(move || {
            world.save();
        });
    }

    /// Stops the threads. Chunks still queued are not packed.
    pub fn shutdown(&self) {
        let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.stop = true;
        drop(q);
        self.shared.wake.notify_all();
        self.shared.tick.notify_all();
    }
}

impl Drop for WorldData {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// `CachedWorld.PackerThread.run`: upstream's method.
fn packer(shared: &Shared, world: &CachedWorld) {
    loop {
        let (job, backlog) = {
            let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if q.stop {
                    return;
                }
                if let Some(pos) = q.order.pop_front() {
                    let job = q.jobs.remove(&pos);
                    break (job, q.order.len());
                }
                q = shared.wake.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let Some(job) = job else { continue };
        let max = world.settings().chunk_packer_queue_max_size;
        if backlog as i64 > i64::from(max) {
            // the oldest chunks are discarded, the newest kept
            shared.dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let began = std::time::Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pack(&*job.source, job.chunk_x, job.chunk_z, &world.dimension, &world.env, shared.allow_walk_on_magma.load(Ordering::Relaxed), world.now_ms())));
        let us = began.elapsed().as_micros() as u64;
        shared.pack_us.fetch_add(us, Ordering::Relaxed);
        shared.pack_max_us.fetch_max(us, Ordering::Relaxed);
        // in the case of an exception, keep consuming from the queue
        if let Ok(chunk) = result {
            if !world.update_cached_chunk_if_changed(chunk) {
                shared.unchanged.fetch_add(1, Ordering::Relaxed);
            }
            shared.packed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The save timer of `CachedWorld.<init>`: the first save after 30 seconds, then every 10 minutes ("since a region only saves if it has been modified since
/// its last save, saving every 10 minutes means that once it is time to exit we will only have a couple regions to save").
fn saver(shared: &Shared, world: &CachedWorld) {
    let mut wait = Duration::from_secs(30);
    loop {
        let q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        let (q, _) = shared.tick.wait_timeout_while(q, wait, |q| !q.stop).unwrap_or_else(|e| e.into_inner());
        if q.stop {
            return;
        }
        drop(q);
        world.save();
        wait = Duration::from_secs(600);
    }
}

/// Upstream's `WorldProvider.worldCache` map: one `WorldData` per folder, kept for the process, so coming back to a world finds its regions in memory.
#[derive(Default)]
pub struct WorldProvider {
    worlds: Mutex<HashMap<PathBuf, Arc<WorldData>>>,
}

impl WorldProvider {
    pub fn new() -> WorldProvider {
        WorldProvider::default()
    }

    /// Every world opened so far.
    pub fn all(&self) -> Vec<Arc<WorldData>> {
        self.worlds.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }

    /// `worldCache.computeIfAbsent(worldDataDir, d -> new WorldData(...))`.
    pub fn open(&self, directory: PathBuf, make: impl FnOnce() -> Arc<CachedWorld>) -> std::io::Result<Arc<WorldData>> {
        let mut map = self.worlds.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(w) = map.get(&directory) {
            return Ok(Arc::clone(w));
        }
        let data = Arc::new(WorldData::start(make())?);
        map.insert(directory, Arc::clone(&data));
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::chunk::tests::{overworld, Cells};
    use crate::cache::palette::fixture::*;
    use crate::cache::world::tests::{temp_dir, world_at};

    struct Src(Cells);
    impl PackSource for Src {
        fn state(&self, x: i32, y: i32, z: i32) -> usize {
            self.0.state(x, y, z)
        }
        fn section_has_only_air(&self, s: usize) -> bool {
            self.0.section_has_only_air(s)
        }
    }

    fn job(cx: i32, cz: i32, stone_at: i32) -> PackJob {
        let mut c = Cells::new(384);
        c.set(1, stone_at, 1, STONE);
        PackJob { chunk_x: cx, chunk_z: cz, source: Box::new(Src(c)) }
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let t = std::time::Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn queued_chunks_are_packed_off_the_calling_thread_and_reach_the_world() {
        let dir = temp_dir("packer");
        let data = WorldData::start(Arc::new(world_at(&dir))).unwrap();
        for i in 0..20 {
            data.queue_for_packing(job(i, 0, 100));
        }
        wait_for("the packer", || data.counts().0 == 20);
        assert_eq!(data.world.unsaved_regions(), 1);
        assert!(data.world.is_cached(5 * 16, 0));
        assert_eq!(data.pending(), 0);
        data.on_close();
        wait_for("the close save", || dir.join("r.0.0.bcr").exists() && data.world.unsaved_regions() == 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_chunk_replaces_the_one_waiting_and_an_overfull_queue_drops_the_oldest() {
        let dir = temp_dir("queue");
        let w = Arc::new(world_at(&dir));
        let mut s = w.settings();
        s.chunk_packer_queue_max_size = 2;
        w.set_settings(s);
        // hold the packer back: take the queue lock while queueing
        let data = WorldData::start(Arc::clone(&w)).unwrap();
        {
            let _hold = data.shared.queue.lock().unwrap();
            // the packer is blocked on the lock as soon as it wakes; queue through the shared state directly
            let mut q = _hold;
            for i in 0..6 {
                let pos = (i, 0);
                if q.jobs.insert(pos, job(i, 0, 50)).is_none() {
                    q.order.push_back(pos);
                }
            }
            // the same position again: replaced, not queued twice
            let again = (3, 0);
            assert!(q.jobs.insert(again, job(3, 0, 60)).is_some());
            assert_eq!(q.order.len(), 6);
        }
        data.shared.wake.notify_all();
        wait_for("the queue to drain", || {
            let (p, d) = data.counts();
            p + d == 6
        });
        let (packed, dropped) = data.counts();
        // 6 queued, limit 2: the first takes find 5, 4, 3 behind it and are dropped; the rest are kept
        assert!(dropped >= 1 && packed >= 3, "packed {packed}, dropped {dropped}");
        assert!(w.is_cached(5 * 16, 0), "the newest chunk is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chunk_queued_while_both_threads_wait_is_always_packed() {
        // review finding 2: the saver and the packer shared a condition variable; `notify_one` could wake the saver and leave the packer asleep
        let dir = temp_dir("lostwake");
        let data = WorldData::start(Arc::new(world_at(&dir))).unwrap();
        for i in 0..60 {
            std::thread::sleep(Duration::from_millis(20)); // both threads are waiting again
            data.queue_for_packing(job(i, 0, 100));
            let t = std::time::Instant::now();
            while data.counts().0 < i as u64 + 1 {
                assert!(t.elapsed() < Duration::from_millis(1500), "chunk {i} waited with the packer asleep");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_queue_has_room_until_half_the_limit() {
        let dir = temp_dir("room");
        let w = Arc::new(world_at(&dir));
        let mut s = w.settings();
        s.chunk_packer_queue_max_size = 10;
        w.set_settings(s);
        let data = WorldData::start(Arc::clone(&w)).unwrap();
        assert!(data.has_room());
        // Stop the packer first: a packer that starts late would take the jobs below before the check (it looks at `stop` before it takes one).
        data.shutdown();
        {
            let mut q = data.shared.queue.lock().unwrap();
            for i in 0..5 {
                q.jobs.insert((i, 0), job(i, 0, 50));
                q.order.push_back((i, 0));
            }
        }
        assert!(!data.has_room(), "5 waiting of a limit of 10");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_provider_gives_the_same_world_for_the_same_folder() {
        let dir = temp_dir("provider");
        let p = WorldProvider::new();
        let a = p.open(dir.clone(), || Arc::new(world_at(&dir))).unwrap();
        let b = p.open(dir.clone(), || panic!("a second world over the same folder")).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        let _ = overworld();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
