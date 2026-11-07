// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/behavior/PathingBehavior.java (onTick, tickPath, pathStart,
// secretInternalSetGoalAndPath, the cancel methods, findPathInNewThread, and what the thread does with its result) and
// src/main/java/baritone/utils/PathingControlManager.java (preTick, postTick, executeProcesses, forceRevalidate, revalidateGoal, for
// the one process that exists), src/main/java/baritone/utils/InputOverrideHandler.java (inControl). LGPL-3.0-or-later, see lib.rs.
//
// A pure state machine, driven once per game tick by `Pathing::tick`. Searches run behind the `SearchRunner` trait: this file starts them
// and takes their results, and never waits for one. Each search has a generation number. A result whose generation is not the one in
// progress is dropped, and a search that was cancelled never installs a path.
use super::custom_goal::{CustomGoalProcess, PathingCommand, PathingCommandType};
use crate::blocks::State;
use crate::config::Config;
use crate::exec::movements::Env;
use crate::exec::state::{CancelReason, Controls, Intent, PlayerView};
use crate::exec::{PathExecutor, Progress};
use crate::goals::{Goal, GoalSettings};
use crate::look::AimProcessor;
use crate::movement::helper::{Border, Ctx, Precomputed};
use crate::plan;
use crate::pos::Pos;
use crate::search::{PathCalculationResult, ResultType, SearchLimits};
use crate::settings::ExecSettings;
use crate::world::WorldView;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Everything a context is made of besides the world: the block table, the flags made from it and the settings, shared read-only by
/// the executor on the game thread and the searches on the worker. The flags depend on the settings, so they are made together:
/// a new settings snapshot means a new `Tables`.
pub struct Tables {
    pub states: Vec<State>,
    pub pre: Precomputed,
    pub cfg: Config,
    /// `ToolSet.getStrVsBlock` for each state id.
    pub tool_strength: Vec<f64>,
}

impl Tables {
    /// Makes the flags from the states and the settings. `tool_strength` has one entry per state.
    pub fn new(states: Vec<State>, cfg: Config, tool_strength: Vec<f64>) -> Tables {
        let pre = Precomputed::new(&states, &cfg);
        Tables { states, pre, cfg, tool_strength }
    }

    /// A context over a world.
    pub fn ctx<'a, W: WorldView>(&'a self, world: &'a W, border: Border) -> Ctx<'a, W> {
        Ctx::new(world, &self.states, &self.pre, &self.cfg, border, &self.tool_strength)
    }
}

/// A search to run. Owns everything but the world snapshot, which goes next to it.
#[derive(Clone)]
pub struct SearchJob {
    pub generation: u64,
    pub start: Pos,
    pub goal: Goal,
    /// The path being followed, whose positions are favored: set for the next segment.
    pub previous: Option<crate::search::Path>,
    pub feet: Pos,
    pub tables: Arc<Tables>,
    pub border: Border,
    pub primary_timeout_ms: i64,
    pub failure_timeout_ms: i64,
    /// A node cap in addition to the time limits; none by default.
    pub max_nodes: Option<u32>,
}

/// What a search answers.
#[derive(Clone, Debug)]
pub struct SearchOutcome {
    pub generation: u64,
    pub result: PathCalculationResult,
}

/// `findPathInNewThread`'s body: runs one search over a snapshot. Used by the worker, and by test runners that search at once.
pub fn run_search<S: WorldView>(job: &SearchJob, snapshot: &S, cancel: &AtomicBool, clock: &mut dyn FnMut() -> i64) -> SearchOutcome {
    let ctx = job.tables.ctx(snapshot, job.border);
    let mut finder = plan::create_pathfinder(job.start, &job.goal, job.previous.as_ref(), &ctx, job.feet, &[]);
    let limits = SearchLimits { max_nodes: job.max_nodes, cancel: Some(cancel) };
    let result = finder.calculate(job.primary_timeout_ms, job.failure_timeout_ms, &limits, clock);
    SearchOutcome { generation: job.generation, result }
}

/// Where searches run. `S` is the owned snapshot of the world a search reads.
///
/// - `start` runs the job, now or later, replacing any job that waits; it never blocks on a search.
/// - `cancel` stops the search that runs and drops the one that waits. A cancelled search must not deliver a result.
/// - `poll` gives a finished result, if there is one. Results may arrive late; the caller checks the generation.
pub trait SearchRunner<S> {
    fn start(&mut self, job: SearchJob, snapshot: S);
    fn cancel(&mut self);
    fn poll(&mut self) -> Option<SearchOutcome>;
}

/// `PathEvent`: what happened in the pathing, for the chat and the log. The state machine itself reads only `CalcFailed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathEvent {
    CalcStarted,
    CalcFinishedNowExecuting,
    CalcFailed,
    NextSegmentCalcStarted,
    NextSegmentCalcFinished,
    ContinuingOntoPlannedNext,
    SplicingOntoNextEarly,
    AtGoal,
    PathFinishedNextStillCalculating,
    NextCalcFailed,
    DiscardNext,
    Canceled,
}

/// What `Pathing::tick` reads: the world, the player, the settings. All of it a snapshot taken for this tick.
pub struct TickEnv<'a, W: WorldView> {
    pub tables: &'a Arc<Tables>,
    pub world: &'a W,
    pub border: Border,
    pub player: &'a PlayerView,
    pub exec: &'a ExecSettings,
}

/// What `Pathing::tick` answers.
#[derive(Clone, Debug)]
pub struct TickOutput {
    pub intent: Intent,
    pub events: Vec<PathEvent>,
}

/// A search that is running: upstream's `inProgress`, as far as the game thread sees it.
#[derive(Clone, Copy, Debug)]
struct InProgress {
    generation: u64,
    start: Pos,
}

/// `PathingBehavior.pathStart`: where a new path starts. Usually the feet; but a player standing off the edge of a block, or in the middle
/// of a jump, starts from the block that supports them.
pub fn path_start<W: WorldView>(ctx: &Ctx<'_, W>, p: &PlayerView) -> Pos {
    let feet = p.feet();
    if !ctx.can_walk_on_at(feet.x, feet.y.wrapping_sub(1), feet.z) {
        if p.on_ground {
            let (player_x, player_z) = (p.x, p.z);
            let mut closest = vec![];
            for dx in -1..=1 {
                for dz in -1..=1 {
                    closest.push(Pos::new(feet.x.wrapping_add(dx), feet.y, feet.z.wrapping_add(dz)));
                }
            }
            let key = |pos: &Pos| ((pos.x as f64 + 0.5) - player_x) * ((pos.x as f64 + 0.5) - player_x) + ((pos.z as f64 + 0.5) - player_z) * ((pos.z as f64 + 0.5) - player_z);
            // `List.sort` is stable, as is this
            closest.sort_by(|a, b| key(a).total_cmp(&key(b)));
            for possible_support in closest.iter().take(4) {
                let x_dist = ((possible_support.x as f64 + 0.5) - player_x).abs();
                let z_dist = ((possible_support.z as f64 + 0.5) - player_z).abs();
                if x_dist > 0.8 && z_dist > 0.8 {
                    // can't possibly be sneaking off of this one, we're too far away
                    continue;
                }
                let s = *possible_support;
                if ctx.can_walk_on_at(s.x, s.y.wrapping_sub(1), s.z) && ctx.can_walk_through_at(s.x, s.y, s.z) && ctx.can_walk_through_at(s.x, s.y.wrapping_add(1), s.z) {
                    // this is plausible
                    return s;
                }
            }
        } else {
            // !onGround
            // we're in the middle of a jump
            if ctx.can_walk_on_at(feet.x, feet.y.wrapping_sub(2), feet.z) {
                return feet.below(1);
            }
        }
    }
    feet
}

/// Bundles what the methods of one tick share.
struct Tk<'a, 'c, W: WorldView, S> {
    env: &'a TickEnv<'a, W>,
    ctx: &'c Ctx<'c, W>,
    snapshot: &'a mut dyn FnMut() -> S,
}

/// `PathingBehavior` with `PathingControlManager` and the `CustomGoalProcess` it drives.
pub struct Pathing<R: SearchRunner<S>, S> {
    runner: R,
    process: CustomGoalProcess,
    aim: AimProcessor,
    controls: Controls,

    current: Option<PathExecutor>,
    next: Option<PathExecutor>,
    goal: Option<Goal>,

    ticks_elapsed_so_far: i32,
    start_position: Option<Pos>,
    safe_to_cancel: bool,
    cancel_requested: bool,
    calc_failed_last_tick: bool,
    in_progress: Option<InProgress>,
    expected_segment_start: Pos,
    events: Vec<PathEvent>,
    out_events: Vec<PathEvent>,
    next_generation: u64,

    // PathingControlManager
    in_control_last_tick: bool,
    in_control_this_tick: bool,
    command: Option<PathingCommand>,

    /// A node cap for every search; none by default.
    pub max_nodes: Option<u32>,
    /// Why the last executor ended, if it failed. Not upstream; for the chat and the tests.
    last_cancel: Option<CancelReason>,
    /// A search ended in `ResultType::Exception` (the worker caught a panic). Not upstream; see D23.
    search_failed: bool,
    _snapshot: std::marker::PhantomData<S>,
}

impl<R: SearchRunner<S>, S> Pathing<R, S> {
    pub fn new(runner: R) -> Pathing<R, S> {
        Pathing {
            runner,
            process: CustomGoalProcess::new(),
            aim: AimProcessor::new(0.5),
            controls: Controls::default(),
            current: None,
            next: None,
            goal: None,
            ticks_elapsed_so_far: 0,
            start_position: None,
            safe_to_cancel: true,
            cancel_requested: false,
            calc_failed_last_tick: false,
            in_progress: None,
            expected_segment_start: Pos::new(0, 0, 0),
            events: vec![],
            out_events: vec![],
            next_generation: 1,
            in_control_last_tick: false,
            in_control_this_tick: false,
            command: None,
            max_nodes: None,
            last_cancel: None,
            search_failed: false,
            _snapshot: std::marker::PhantomData,
        }
    }

    // ------------------------------------------------------------ what the commands call

    /// Upstream's `ICustomGoalProcess.setGoalAndPath`: what `/b goto` does. The search starts on the next tick.
    pub fn goto(&mut self, goal: Goal) {
        self.process.set_goal_and_path(goal);
    }

    /// Upstream's `ICustomGoalProcess.setGoal`: `/b goal`. Sets the goal without a path.
    pub fn set_goal(&mut self, goal: Goal) {
        self.process.set_goal(goal);
    }

    /// `ICustomGoalProcess.path`.
    pub fn path(&mut self) {
        self.process.path();
    }

    /// `PathingBehavior.cancelEverything`: the processes lose control now; the current segment is cut only if that is safe, else when
    /// it becomes safe. Returns whether the segment was cut.
    pub fn cancel_everything(&mut self) -> bool {
        let do_it = self.is_safe_to_cancel();
        if do_it {
            self.segment_cancel();
        }
        // regardless of if we can stop the current segment, we can still stop the processes
        self.manager_cancel_everything();
        do_it
    }

    /// `PathingBehavior.forceCancel`: everything stops now, safe or not. Returns the neutral intent to put in place of the last one.
    ///
    /// `/b stop` calls this and not `cancel_everything`, so that the intent after it is neutral at once. Upstream's `#stop` is
    /// `cancelEverything`, which waits for a safe moment. See docs/deviations.md.
    pub fn stop(&mut self) -> Intent {
        self.force_cancel();
        self.controls = Controls::default();
        Intent::neutral()
    }

    /// Whether a search panicked since the last call. Not upstream (D23): the glue stops the goto and says so, where upstream
    /// would start the same search again on the next tick.
    pub fn take_search_failure(&mut self) -> bool {
        std::mem::take(&mut self.search_failed)
    }

    /// `PathingBehavior.forceCancel`.
    pub fn force_cancel(&mut self) {
        self.cancel_everything();
        self.segment_cancel();
        self.in_progress = None;
    }

    // ------------------------------------------------------------ what the glue reads

    /// `PathingBehavior.getGoal`:
    /// `PathingBehavior.getGoal`.
    pub fn goal(&self) -> Option<&Goal> {
        self.goal.as_ref()
    }

    /// `PathingBehavior.hasPath`.
    pub fn has_path(&self) -> bool {
        self.current.is_some()
    }

    /// `PathingBehavior.isPathing`:
    /// `PathingBehavior.isPathing` (no process pauses, so this is `hasPath`).
    pub fn is_pathing(&self) -> bool {
        self.current.is_some()
    }

    /// Whether a search is running.
    pub fn is_calculating(&self) -> bool {
        self.in_progress.is_some()
    }

    /// Whether the process behind `goto` holds the goal.
    pub fn is_active(&self) -> bool {
        self.process.is_active()
    }

    /// `PathingBehavior.getCurrent`:
    /// `PathingBehavior.getCurrent`.
    pub fn current(&self) -> Option<&PathExecutor> {
        self.current.as_ref()
    }

    /// `PathingBehavior.getNext`:
    /// `PathingBehavior.getNext`.
    pub fn next(&self) -> Option<&PathExecutor> {
        self.next.as_ref()
    }

    /// The process.
    pub fn process(&self) -> &CustomGoalProcess {
        &self.process
    }

    /// Why the last path was cancelled by the executor, if it was.
    pub fn last_cancel(&self) -> Option<CancelReason> {
        self.last_cancel
    }

    /// The search runner (the worker), for the glue.
    pub fn runner_mut(&mut self) -> &mut R {
        &mut self.runner
    }

    /// `PathingBehavior.isSafeToCancel`: There is no elytra process.
    /// `PathingBehavior.isSafeToCancel` (there is no elytra process).
    pub fn is_safe_to_cancel(&self) -> bool {
        if self.current.is_none() {
            return true;
        }
        self.safe_to_cancel
    }

    // `PathingBehavior.estimatedTicksToGoal` is left for later: it only feeds the `eta` command.

    // ------------------------------------------------------------ the tick

    /// `PathingBehavior.onTick`: The tick in the world. `PathingControlManager.postTick` follows it.
    /// `PathingBehavior.onTick` (the tick in the world), then `PathingControlManager.postTick`. Call once per game tick, after the
    /// player has moved, with the player as it is now. Returns the intent for the next movement of the player.
    pub fn tick<W: WorldView>(&mut self, env: &TickEnv<'_, W>, snapshot: &mut dyn FnMut() -> S) -> TickOutput {
        let ctx = env.tables.ctx(env.world, env.border);
        let mut tk = Tk { env, ctx: &ctx, snapshot };
        // results that came in since the last tick: upstream's thread installs them under `pathPlanLock`, between ticks
        while let Some(outcome) = self.runner.poll() {
            self.install(outcome, &tk);
        }
        self.dispatch_events();
        self.expected_segment_start = path_start(tk.ctx, env.player);
        self.pre_tick(&mut tk);
        self.tick_path(&mut tk);
        self.ticks_elapsed_so_far = self.ticks_elapsed_so_far.wrapping_add(1);
        self.dispatch_events();
        self.post_tick(&mut tk);
        let intent = self.build_intent(env.player, env.exec.mouse_sensitivity);
        TickOutput { intent, events: std::mem::take(&mut self.out_events) }
    }

    /// `PathingBehavior.queuePathEvent`:
    /// `PathingBehavior.queuePathEvent`.
    fn queue_path_event(&mut self, e: PathEvent) {
        self.events.push(e);
    }

    /// `PathingBehavior.dispatchEvents`: only `calcFailedLastTick` is read by the state machine; the rest goes to the caller.
    fn dispatch_events(&mut self) {
        let curr = std::mem::take(&mut self.events);
        self.calc_failed_last_tick = curr.contains(&PathEvent::CalcFailed);
        self.out_events.extend(curr);
    }

    /// The intent after the tick: what `InputOverrideHandler`, `PlayerMovementInput`, `LookBehavior` and `onPlayerSprintState` make of the
    /// state of the executor.
    fn build_intent(&mut self, player: &PlayerView, mouse_sensitivity: f64) -> Intent {
        let k = self.controls.keys;
        // `InputOverrideHandler.inControl`
        let in_control = k.forward || k.back || k.left || k.right || k.sneak || k.jump || self.is_pathing();
        let look = self.controls.look.take();
        if !in_control && look.is_none() {
            return Intent::neutral();
        }
        // upstream reads the player's own mouse setting each time (`LookBehavior`, `options.sensitivity()`)
        self.aim.sensitivity = mouse_sensitivity;
        let rotation = look.map(|t| self.aim.peek_rotation(player.rotations(), t));
        Intent {
            active: in_control,
            rotation,
            forward: k.forward,
            back: k.back,
            left: k.left,
            right: k.right,
            jump: k.jump,
            sneak: k.sneak,
            // `PathingBehavior.onPlayerSprintState`
            sprint: self.current.as_ref().is_some_and(|c| c.is_sprinting()),
            use_block: if k.click_right { self.controls.use_pos } else { None },
            cancel_flying: self.current.is_some(),
            disable_auto_jump: self.current.is_some(),
        }
    }

    // ------------------------------------------------------------ PathingControlManager

    /// `PathingControlManager.cancelEverything`:
    /// `PathingControlManager.cancelEverything`.
    fn manager_cancel_everything(&mut self) {
        self.in_control_last_tick = false;
        self.in_control_this_tick = false;
        self.command = None;
        self.process.on_lost_control();
    }

    /// `PathingControlManager.executeProcesses`, for the one process.
    fn execute_processes<W: WorldView>(&mut self, tk: &Tk<'_, '_, W, S>) -> Option<PathingCommand> {
        if !self.process.is_active() {
            return None;
        }
        let feet = tk.env.player.feet();
        let calc_failed = self.in_control_last_tick && self.calc_failed_last_tick;
        let cmd = self.process.on_tick(calc_failed, feet, self.expected_segment_start, &tk.ctx.cfg.goal_settings());
        self.in_control_this_tick = true;
        Some(cmd)
    }

    /// `PathingControlManager.preTick`: For the one process.
    /// `PathingControlManager.preTick`.
    fn pre_tick<W: WorldView>(&mut self, tk: &mut Tk<'_, '_, W, S>) {
        self.in_control_last_tick = self.in_control_this_tick;
        self.in_control_this_tick = false;
        self.command = self.execute_processes(tk);
        let Some(cmd) = self.command.clone() else {
            self.cancel_segment_if_safe();
            self.goal = None;
            return;
        };
        match cmd.kind {
            PathingCommandType::CancelAndSetGoal => {
                self.goal = cmd.goal.clone();
                self.cancel_segment_if_safe();
            }
            PathingCommandType::ForceRevalidateGoalAndPath => {
                if !self.is_pathing() && self.in_progress.is_none() {
                    self.set_goal_and_path(&cmd, tk);
                }
            }
            PathingCommandType::SetGoalAndPath => {
                // now this i can do
                if cmd.goal.is_some() {
                    self.set_goal_and_path(&cmd, tk);
                }
            }
        }
    }

    /// `PathingControlManager.postTick`: after the tick of the path, so the time between ticks is calculation time.
    fn post_tick<W: WorldView>(&mut self, tk: &mut Tk<'_, '_, W, S>) {
        let Some(cmd) = self.command.clone() else {
            return;
        };
        if cmd.kind == PathingCommandType::ForceRevalidateGoalAndPath {
            let gs = tk.ctx.cfg.goal_settings();
            let cancel = match &cmd.goal {
                None => true,
                Some(g) => self.force_revalidate(g, &gs) || self.revalidate_goal(g, &gs),
            };
            if cancel {
                // pwnage
                self.soft_cancel_if_safe();
            }
            self.set_goal_and_path(&cmd, tk);
        }
    }

    /// `PathingControlManager.forceRevalidate`:
    /// `PathingControlManager.forceRevalidate`.
    fn force_revalidate(&self, new_goal: &Goal, gs: &GoalSettings) -> bool {
        if let Some(current) = &self.current {
            let d = current.path().dest();
            if new_goal.is_in_goal(d.x, d.y, d.z, gs) {
                return false;
            }
            return new_goal != current.path().goal();
        }
        false
    }

    /// `PathingControlManager.revalidateGoal`:
    /// `PathingControlManager.revalidateGoal`.
    fn revalidate_goal(&self, new_goal: &Goal, gs: &GoalSettings) -> bool {
        if let Some(current) = &self.current {
            let intended = current.path().goal();
            let end = current.path().dest();
            if intended.is_in_goal(end.x, end.y, end.z, gs) && !new_goal.is_in_goal(end.x, end.y, end.z, gs) {
                // this path used to end in the goal
                // but the goal has changed, so there's no reason to continue...
                return true;
            }
        }
        false
    }
}

impl<R: SearchRunner<S>, S> Pathing<R, S> {
    /// `PathingBehavior.secretInternalSetGoalAndPath`:
    /// `PathingBehavior.secretInternalSetGoalAndPath`.
    fn set_goal_and_path<W: WorldView>(&mut self, cmd: &PathingCommand, tk: &mut Tk<'_, '_, W, S>) -> bool {
        self.goal = cmd.goal.clone();
        let Some(goal) = self.goal.clone() else {
            return false;
        };
        let gs = tk.ctx.cfg.goal_settings();
        let feet = tk.env.player.feet();
        if goal.is_in_goal(feet.x, feet.y, feet.z, &gs) {
            return false;
        }
        if self.current.is_some() {
            return false;
        }
        if self.in_progress.is_some() {
            return false;
        }
        self.queue_path_event(PathEvent::CalcStarted);
        let start = self.expected_segment_start;
        self.find_path_in_new_thread(start, tk);
        true
    }

    /// `PathingBehavior.findPathInNewThread`: starts a search through the runner. It does not wait.
    fn find_path_in_new_thread<W: WorldView>(&mut self, start: Pos, tk: &mut Tk<'_, '_, W, S>) {
        if self.in_progress.is_some() {
            return; // upstream throws "Already doing it": the callers check
        }
        let Some(goal) = self.goal.clone() else {
            return; // no goal
        };
        let (primary, failure) = plan::timeouts(self.current.is_some(), &tk.ctx.cfg.search);
        let generation = self.next_generation;
        self.next_generation += 1;
        let job = SearchJob { generation, start, goal, previous: self.current.as_ref().map(|c| c.path().clone()), feet: tk.env.player.feet(), tables: Arc::clone(tk.env.tables), border: tk.env.border, primary_timeout_ms: primary, failure_timeout_ms: failure, max_nodes: self.max_nodes };
        self.in_progress = Some(InProgress { generation, start });
        let snapshot = (tk.snapshot)();
        self.runner.start(job, snapshot);
    }

    /// What the thread of `findPathInNewThread` does with its result, under `pathPlanLock`. A result of any other generation than the
    /// one in progress is dropped: the search was cancelled or replaced.
    fn install<W: WorldView>(&mut self, outcome: SearchOutcome, _tk: &Tk<'_, '_, W, S>) {
        let Some(ip) = self.in_progress else {
            return;
        };
        if ip.generation != outcome.generation {
            return;
        }
        let start = ip.start;
        let result = outcome.result;
        if result.kind == ResultType::Exception {
            self.search_failed = true;
        }
        let executor = result.path.and_then(|p| PathExecutor::new(p).ok());
        if self.current.is_none() {
            if let Some(executor) = executor {
                if executor.path().positions().contains(&self.expected_segment_start) {
                    self.queue_path_event(PathEvent::CalcFinishedNowExecuting);
                    self.current = Some(executor);
                    self.reset_estimated_ticks_to_goal(start);
                }
                // else: upstream logs "discarding orphan path segment with incorrect start"
            } else if result.kind != ResultType::Cancellation && result.kind != ResultType::Exception {
                // don't dispatch CALC_FAILED on cancellation
                self.queue_path_event(PathEvent::CalcFailed);
            }
        } else if self.next.is_none() {
            if let Some(executor) = executor {
                if self.current.as_ref().is_some_and(|c| executor.path().src() == c.path().dest()) {
                    self.queue_path_event(PathEvent::NextSegmentCalcFinished);
                    self.next = Some(executor);
                }
                // else: "discarding orphan next segment with incorrect start"
            } else {
                self.queue_path_event(PathEvent::NextCalcFailed);
            }
        }
        // else: "PathingBehavior illegal state! Discarding invalid path!"
        self.in_progress = None;
    }

    /// `PathingBehavior.resetEstimatedTicksToGoal(BetterBlockPos)`:
    /// `PathingBehavior.resetEstimatedTicksToGoal`.
    fn reset_estimated_ticks_to_goal(&mut self, start: Pos) {
        self.ticks_elapsed_so_far = 0;
        self.start_position = Some(start);
    }

    /// Stops the search that runs, and forgets it, so that its result is dropped.
    fn cancel_in_progress(&mut self) {
        if self.in_progress.take().is_some() {
            self.runner.cancel();
        }
    }

    /// `PathingBehavior.cancelSegmentIfSafe`:
    /// `PathingBehavior.cancelSegmentIfSafe`.
    fn cancel_segment_if_safe(&mut self) -> bool {
        if self.is_safe_to_cancel() {
            self.segment_cancel();
            return true;
        }
        false
    }

    /// `PathingBehavior.secretInternalSegmentCancel`:
    /// `PathingBehavior.secretInternalSegmentCancel`.
    fn segment_cancel(&mut self) {
        self.queue_path_event(PathEvent::Canceled);
        self.cancel_in_progress();
        if self.current.is_some() {
            self.current = None;
            self.next = None;
            self.controls.clear_all_keys();
        }
    }

    /// `PathingBehavior.softCancelIfSafe`: everything but clearing the keys.
    fn soft_cancel_if_safe(&mut self) {
        self.cancel_in_progress(); // only cancel ours
        if !self.is_safe_to_cancel() {
            return;
        }
        self.current = None;
        self.next = None;
        self.cancel_requested = true;
    }

    /// `PathingBehavior.tickPath`: The pause request is not here: no process sends one.
    /// `PathingBehavior.tickPath`.
    fn tick_path<W: WorldView>(&mut self, tk: &mut Tk<'_, '_, W, S>) {
        let player = tk.env.player;
        let feet = player.feet();
        if self.cancel_requested {
            self.cancel_requested = false;
            self.controls.clear_all_keys();
        }
        if let Some(ip) = self.in_progress {
            // we are calculating
            // are we calculating the right thing though?
            let calc_from = ip.start;
            // The best path so far of the running search cannot be read from here: the condition about it is taken as "there is none".
            if (self.current.as_ref().is_none_or(|c| c.path().dest() != calc_from)) && calc_from != feet && calc_from != self.expected_segment_start {
                // when it was *just* started, currentBest will be empty so we need to also check calcFrom since that's always present
                self.cancel_in_progress(); // cancellation doesn't dispatch any events
            }
        }
        let path_start = self.expected_segment_start;
        let progress = Progress { searching: self.in_progress.is_some(), best_positions: None };
        let senv = Env { ctx: tk.ctx, p: player, set: tk.env.exec, path_start };
        let Some(current) = self.current.as_mut() else {
            return;
        };
        self.safe_to_cancel = current.on_tick(&senv, &mut self.controls, &progress);
        let gs = tk.ctx.cfg.goal_settings();
        if current.failed() || current.finished() {
            self.last_cancel = current.cancel_reason();
            self.current = None;
            let at_goal = match &self.goal {
                None => true,
                Some(g) => g.is_in_goal(feet.x, feet.y, feet.z, &gs),
            };
            if at_goal {
                // All done.
                self.queue_path_event(PathEvent::AtGoal);
                self.next = None;
                return;
            }
            let next_has_us = self.next.as_ref().is_none_or(|n| n.path().positions().contains(&feet) || n.path().positions().contains(&path_start));
            if self.next.is_some() && !next_has_us {
                // if the current path failed, we may not actually be on the next one, so make sure
                // for example if we had a nicely planned ahead path that starts where current ends
                // that's all fine and good
                // but if we fail in the middle of current
                // we're nowhere close to our planned ahead path
                // so need to discard it sadly.
                self.queue_path_event(PathEvent::DiscardNext);
                self.next = None;
            }
            if let Some(mut next) = self.next.take() {
                self.queue_path_event(PathEvent::ContinuingOntoPlannedNext);
                next.on_tick(&senv, &mut self.controls, &progress); // don't waste a tick doing nothing, get started right away
                self.current = Some(next);
                return;
            }
            // at this point, current just ended, but we aren't in the goal and have no plan for the future
            if self.in_progress.is_some() {
                self.queue_path_event(PathEvent::PathFinishedNextStillCalculating);
                return;
            }
            // we aren't calculating
            self.queue_path_event(PathEvent::CalcStarted);
            self.find_path_in_new_thread(path_start, tk);
            return;
        }
        // at this point, we know current is in progress
        if self.safe_to_cancel && self.next.is_some() {
            let snapped = self.next.as_mut().is_some_and(|n| n.snipsnapifpossible(&senv, &mut self.controls));
            if snapped {
                // a movement just ended; jump directly onto the next path
                self.queue_path_event(PathEvent::SplicingOntoNextEarly);
                let mut next = self.next.take().expect("next was checked");
                next.on_tick(&senv, &mut self.controls, &progress);
                self.current = Some(next);
                return;
            }
        }
        if tk.env.exec.splice_path {
            let cur = self.current.as_mut().expect("current was checked");
            if cur.try_splice(self.next.as_ref(), tk.env.exec.max_path_history_length, tk.env.exec.path_history_cutoff_amount).is_err() {
                // upstream throws IllegalStateException; a path that cannot be spliced is not used
                self.next = None;
            }
        }
        let cur = self.current.as_ref().expect("current was checked");
        if self.next.as_ref().is_some_and(|n| cur.path().dest() == n.path().dest()) {
            self.next = None;
        }
        if self.in_progress.is_some() {
            // if we aren't calculating right now
            return;
        }
        if self.next.is_some() {
            // and we have no plan for what to do next
            return;
        }
        let dest = cur.path().dest();
        let goal_done = match &self.goal {
            None => true,
            Some(g) => g.is_in_goal(dest.x, dest.y, dest.z, &gs),
        };
        if goal_done {
            // and this path doesn't get us all the way there
            return;
        }
        // don't include the current movement so a very long last movement (e.g. descend) doesn't trip it up
        // if we actually included current, it wouldn't start planning ahead until the last movement was done, if the last movement took more than 7.5 seconds on its own
        if cur.ticks_remaining_from(cur.position() + 1) < tk.env.exec.planning_tick_lookahead as f64 {
            // Path almost over. Planning ahead...
            self.queue_path_event(PathEvent::NextSegmentCalcStarted);
            self.find_path_in_new_thread(dest, tk);
        }
    }
}
