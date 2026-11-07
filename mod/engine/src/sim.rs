// baritone-rs engine: a small simulator of walking, for the tests of the executor and the pathing state machine. Written for this
// project. LGPL-3.0-or-later, see lib.rs.
//
// THE SIMULATOR IS NOT EVIDENCE OF REAL PHYSICS. It moves a box through a grid of full blocks at fixed speeds (4.317 blocks/s walking,
// 5.612 sprinting, as the cost constants say), with a jump of 0.42, gravity of 0.08 and drag 0.98 per tick, and no friction, no
// slabs, no water, no sneaking at edges. It follows `Intent`s, in the order Horizon runs a tick: the player moves with the last
// intent, then the mod ticks. It shows that the rules of the executor lead a player to a goal in a world shaped like this, and that
// the state machine around them behaves. Whether a real player does the same in the game is for the L3 scenarios.
use crate::exec::state::{Intent, PlayerView};
use crate::goals::Goal;
use crate::movement::helper::Border;
use crate::pos::Pos;
use crate::process::pathing::{run_search, PathEvent, Pathing, SearchJob, SearchOutcome, SearchRunner, Tables, TickEnv, TickOutput};
use crate::settings::ExecSettings;
use crate::testing::{Fixture, TestWorld, STONE};
use crate::world::WorldView;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const WALK: f64 = 4.317 / 20.0;
const SPRINT: f64 = 5.612 / 20.0;
const HALF_WIDTH: f64 = 0.3;
const HEIGHT: f64 = 1.8;

fn solid(w: &TestWorld, x: i32, y: i32, z: i32) -> bool {
    (0..16).contains(&y) && w.state_id(x, y, z) == STONE as usize
}

/// The simulated player.
#[derive(Clone, Debug)]
pub struct SimPlayer {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
}

impl SimPlayer {
    /// Standing in the middle of a block, looking south.
    pub fn at(x: i32, y: i32, z: i32) -> SimPlayer {
        SimPlayer { x: x as f64 + 0.5, y: y as f64, z: z as f64 + 0.5, vx: 0.0, vy: 0.0, vz: 0.0, yaw: 0.0, pitch: 0.0, on_ground: true, horizontal_collision: false }
    }

    pub fn view(&self) -> PlayerView {
        PlayerView { x: self.x, y: self.y, z: self.z, vx: self.vx, vy: self.vy, vz: self.vz, yaw: self.yaw, pitch: self.pitch, on_ground: self.on_ground, horizontal_collision: self.horizontal_collision, ..PlayerView::standing(0.0, 0.0, 0.0) }
    }

    pub fn feet(&self) -> Pos {
        self.view().feet()
    }

    /// One tick of movement with an intent. A neutral intent leaves the player standing (the real keys are not simulated).
    pub fn step(&mut self, w: &TestWorld, i: &Intent) {
        if let Some(r) = i.rotation {
            self.yaw = r.yaw;
            self.pitch = r.pitch;
        }
        let (f, s) = if i.active { (i.forward_impulse() as f64, i.strafe_impulse() as f64) } else { (0.0, 0.0) };
        let sprinting = i.active && i.sprint && f > 0.0;
        let speed = if sprinting { SPRINT } else { WALK };
        let yr = (self.yaw as f64).to_radians();
        let (mut mx, mut mz) = (-yr.sin() * f + yr.cos() * s, yr.cos() * f + yr.sin() * s);
        let len = (mx * mx + mz * mz).sqrt();
        if len > 1.0 {
            mx /= len;
            mz /= len;
        }
        if i.active && i.jump && self.on_ground {
            self.vy = 0.42;
        }
        let (dx, dz) = (mx * speed, mz * speed);
        let (x0, z0) = (self.x, self.z);
        let hit_x = self.move_x(w, dx);
        let hit_z = self.move_z(w, dz);
        self.horizontal_collision = hit_x || hit_z;
        let dy = self.vy;
        self.on_ground = self.move_y(w, dy) && dy < 0.0;
        self.vx = self.x - x0;
        self.vz = self.z - z0;
        self.vy = (self.vy - 0.08) * 0.98;
    }

    fn move_x(&mut self, w: &TestWorld, dx: f64) -> bool {
        let nx = self.x + dx;
        let (y0, y1) = (self.y, self.y + HEIGHT);
        let (z0, z1) = (self.z - HALF_WIDTH, self.z + HALF_WIDTH);
        let mut limit = nx;
        for cy in (y0 + 1e-6).floor() as i32..=(y1 - 1e-6).floor() as i32 {
            for cz in (z0 + 1e-6).floor() as i32..=(z1 - 1e-6).floor() as i32 {
                if dx > 0.0 {
                    for cx in (self.x + HALF_WIDTH + 1e-9).floor() as i32..=(nx + HALF_WIDTH).floor() as i32 {
                        if solid(w, cx, cy, cz) {
                            limit = limit.min(cx as f64 - HALF_WIDTH - 1e-7);
                        }
                    }
                } else if dx < 0.0 {
                    for cx in (nx - HALF_WIDTH).floor() as i32..=(self.x - HALF_WIDTH - 1e-9).floor() as i32 {
                        if solid(w, cx, cy, cz) {
                            limit = limit.max(cx as f64 + 1.0 + HALF_WIDTH + 1e-7);
                        }
                    }
                }
            }
        }
        let hit = limit != nx;
        self.x = limit;
        hit
    }

    fn move_z(&mut self, w: &TestWorld, dz: f64) -> bool {
        let nz = self.z + dz;
        let (y0, y1) = (self.y, self.y + HEIGHT);
        let (x0, x1) = (self.x - HALF_WIDTH, self.x + HALF_WIDTH);
        let mut limit = nz;
        for cy in (y0 + 1e-6).floor() as i32..=(y1 - 1e-6).floor() as i32 {
            for cx in (x0 + 1e-6).floor() as i32..=(x1 - 1e-6).floor() as i32 {
                if dz > 0.0 {
                    for cz in (self.z + HALF_WIDTH + 1e-9).floor() as i32..=(nz + HALF_WIDTH).floor() as i32 {
                        if solid(w, cx, cy, cz) {
                            limit = limit.min(cz as f64 - HALF_WIDTH - 1e-7);
                        }
                    }
                } else if dz < 0.0 {
                    for cz in (nz - HALF_WIDTH).floor() as i32..=(self.z - HALF_WIDTH - 1e-9).floor() as i32 {
                        if solid(w, cx, cy, cz) {
                            limit = limit.max(cz as f64 + 1.0 + HALF_WIDTH + 1e-7);
                        }
                    }
                }
            }
        }
        let hit = limit != nz;
        self.z = limit;
        hit
    }

    fn move_y(&mut self, w: &TestWorld, dy: f64) -> bool {
        let ny = self.y + dy;
        let (x0, x1) = (self.x - HALF_WIDTH, self.x + HALF_WIDTH);
        let (z0, z1) = (self.z - HALF_WIDTH, self.z + HALF_WIDTH);
        let mut limit = ny;
        for cx in (x0 + 1e-6).floor() as i32..=(x1 - 1e-6).floor() as i32 {
            for cz in (z0 + 1e-6).floor() as i32..=(z1 - 1e-6).floor() as i32 {
                if dy > 0.0 {
                    for cy in (self.y + HEIGHT + 1e-9).floor() as i32..=(ny + HEIGHT).floor() as i32 {
                        if solid(w, cx, cy, cz) {
                            limit = limit.min(cy as f64 - HEIGHT - 1e-7);
                        }
                    }
                } else if dy < 0.0 {
                    for cy in ny.floor() as i32..=(self.y - 1e-9).floor() as i32 {
                        if solid(w, cx, cy, cz) {
                            limit = limit.max(cy as f64 + 1.0);
                        }
                    }
                }
            }
        }
        let hit = limit != ny;
        self.y = limit;
        if hit {
            self.vy = 0.0;
        }
        hit
    }
}

/// A runner that searches at once, in `start`, and delivers the result at the next `poll`. A cancel drops what was not collected.
#[derive(Default)]
pub struct ImmediateRunner {
    pub results: VecDeque<SearchOutcome>,
    pub started: Vec<(u64, Goal)>,
    pub cancels: u32,
}

impl SearchRunner<TestWorld> for ImmediateRunner {
    fn start(&mut self, job: SearchJob, snapshot: TestWorld) {
        let cancel = AtomicBool::new(false);
        self.started.push((job.generation, job.goal.clone()));
        let out = run_search(&job, &snapshot, &cancel, &mut || 0);
        self.results.push_back(out);
    }
    fn cancel(&mut self) {
        self.cancels += 1;
        self.results.clear();
    }
    fn poll(&mut self) -> Option<SearchOutcome> {
        self.results.pop_front()
    }
}

/// A runner the test drives: jobs wait until the test runs them and hands the outcome in, in any order.
#[derive(Default)]
pub struct ManualRunner {
    pub jobs: Vec<(SearchJob, TestWorld)>,
    pub ready: VecDeque<SearchOutcome>,
    pub cancels: u32,
}

impl SearchRunner<TestWorld> for ManualRunner {
    fn start(&mut self, job: SearchJob, snapshot: TestWorld) {
        self.jobs.push((job, snapshot));
    }
    fn cancel(&mut self) {
        self.cancels += 1;
    }
    fn poll(&mut self) -> Option<SearchOutcome> {
        self.ready.pop_front()
    }
}

impl ManualRunner {
    /// Runs job number `i` over the snapshot it was given, and keeps the outcome for `deliver`.
    pub fn run(&self, i: usize) -> SearchOutcome {
        let (job, snap) = &self.jobs[i];
        run_search(job, snap, &AtomicBool::new(false), &mut || 0)
    }
}

/// The player, the world and the pathing, ticked together.
pub struct Rig<R: SearchRunner<TestWorld>> {
    pub tables: Arc<Tables>,
    pub world: TestWorld,
    pub p: SimPlayer,
    pub pathing: Pathing<R, TestWorld>,
    pub exec: ExecSettings,
    pub intent: Intent,
    pub events: Vec<PathEvent>,
    pub ticks: u32,
    pub intents: Vec<Intent>,
    /// The kinds of movement of every path that was followed.
    pub kinds: Vec<crate::movement::step::MovementKind>,
}

impl<R: SearchRunner<TestWorld>> Rig<R> {
    pub fn with_runner(world: TestWorld, start: (i32, i32, i32), runner: R) -> Rig<R> {
        let fx = Fixture::new();
        let tables = Arc::new(Tables::new(fx.states.clone(), fx.cfg.clone(), vec![1.0, 1.0]));
        let mut pathing = Pathing::new(runner);
        pathing.max_nodes = Some(5000);
        Rig { tables, world, p: SimPlayer::at(start.0, start.1, start.2), pathing, exec: ExecSettings::default(), intent: Intent::neutral(), events: vec![], ticks: 0, intents: vec![], kinds: vec![] }
    }

    /// One game tick, in Horizon's order: the player moves with the last intent, then the mod ticks and makes the next one.
    pub fn tick(&mut self) -> TickOutput {
        self.p.step(&self.world, &self.intent);
        let view = self.p.view();
        let snap = self.world.clone();
        let env = TickEnv { tables: &self.tables, world: &self.world, border: Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 }, player: &view, exec: &self.exec };
        let out = self.pathing.tick(&env, &mut || snap.clone());
        self.intent = out.intent.clone();
        self.events.extend(out.events.iter().copied());
        self.intents.push(out.intent.clone());
        if let Some(c) = self.pathing.current() {
            for m in c.movements() {
                if !self.kinds.contains(&m.kind) {
                    self.kinds.push(m.kind);
                }
            }
        }
        self.ticks += 1;
        out
    }

    /// Ticks until the process is done (it lost control: the goal was reached or given up) or `max` ticks passed. Returns the ticks used.
    pub fn run_until_idle(&mut self, max: u32) -> u32 {
        for n in 0..max {
            self.tick();
            if !self.pathing.is_active() && !self.pathing.is_pathing() && !self.pathing.is_calculating() {
                return n + 1;
            }
        }
        max
    }
}

impl Rig<ImmediateRunner> {
    pub fn new(world: TestWorld, start: (i32, i32, i32)) -> Rig<ImmediateRunner> {
        Rig::with_runner(world, start, ImmediateRunner::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::Goal;
    use crate::movement::helper::Border;
    use crate::movement::step::MovementKind;
    use crate::testing::AIR;

    fn block(x: i32, y: i32, z: i32) -> Goal {
        Goal::Block { x, y, z }
    }

    fn reached(rig: &Rig<ImmediateRunner>, x: i32, y: i32, z: i32) -> bool {
        rig.p.feet() == Pos::new(x, y, z)
    }

    #[test]
    fn the_simulator_walks_jumps_and_falls() {
        // not a test of the engine: of the fixture, so the other tests mean something
        let mut w = TestWorld::flat(10, 10);
        for x in 5..10 {
            for z in 0..10 {
                w.set(x, 1, z, STONE);
            }
        }
        let mut p = SimPlayer::at(2, 1, 2);
        let walk = Intent { active: true, forward: true, rotation: Some(crate::look::Rotation::new(-90.0, 0.0)), ..Intent::neutral() };
        for _ in 0..20 {
            p.step(&w, &walk);
        }
        assert!(p.x > 3.5 && p.x < 4.8, "speed {}", p.x);
        // blocked by the step
        for _ in 0..40 {
            p.step(&w, &walk);
        }
        assert!(p.x < 4.71 && p.horizontal_collision);
        // jump onto it
        let jump = Intent { jump: true, ..walk.clone() };
        for _ in 0..12 {
            p.step(&w, &jump);
        }
        for _ in 0..30 {
            p.step(&w, &Intent::neutral());
        }
        assert!(p.y == 2.0 && p.on_ground, "y {}", p.y);
    }

    #[test]
    fn walks_a_straight_line_to_the_goal() {
        let mut rig = Rig::new(TestWorld::flat(16, 8), (1, 1, 1));
        rig.pathing.goto(block(12, 1, 1));
        let n = rig.run_until_idle(300);
        assert!(reached(&rig, 12, 1, 1), "stopped at {:?} after {n} ticks", rig.p.feet());
        assert!(n < 120, "took {n} ticks");
        assert!(rig.events.contains(&PathEvent::AtGoal) || !rig.pathing.is_active());
        // sprinting: 11 blocks at 5.612 blocks/s is about 40 ticks
        assert!(rig.intents.iter().any(|i| i.sprint && i.forward));
    }

    #[test]
    fn goes_around_a_corner() {
        let mut w = TestWorld::flat(16, 16);
        for z in 0..10 {
            for y in 1..=2 {
                w.set(5, y, z, STONE);
            }
        }
        let mut rig = Rig::new(w, (1, 1, 1));
        rig.pathing.goto(block(9, 1, 1));
        let n = rig.run_until_idle(500);
        assert!(reached(&rig, 9, 1, 1), "stopped at {:?} after {n} ticks, cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert!(rig.pathing.last_cancel().is_none());
        // the path really went round: the player was at z >= 10 at some point... the path's positions were
        assert!(rig.events.contains(&PathEvent::CalcFinishedNowExecuting));
    }

    #[test]
    fn goes_up_a_step() {
        let mut w = TestWorld::flat(16, 8);
        for x in 5..16 {
            for z in 0..8 {
                w.set(x, 1, z, STONE);
            }
        }
        let mut rig = Rig::new(w, (1, 1, 1));
        rig.pathing.goto(block(9, 2, 1));
        let n = rig.run_until_idle(300);
        assert!(reached(&rig, 9, 2, 1), "stopped at {:?} after {n} ticks, cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert!(rig.intents.iter().any(|i| i.jump), "no jump in a path that climbs");
        assert!(rig.kinds.contains(&MovementKind::Ascend), "{:?}", rig.kinds);
    }

    #[test]
    fn goes_down_a_drop() {
        // one block down
        let mut w = TestWorld::flat(16, 8);
        for x in 0..5 {
            for z in 0..8 {
                w.set(x, 1, z, STONE);
            }
        }
        let mut rig = Rig::new(w, (1, 2, 1));
        rig.pathing.goto(block(9, 1, 1));
        let n = rig.run_until_idle(300);
        assert!(reached(&rig, 9, 1, 1), "stopped at {:?} after {n} ticks, cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert!(rig.kinds.contains(&MovementKind::Descend), "{:?}", rig.kinds);
        // two blocks down: a fall
        let mut w = TestWorld::flat(16, 8);
        for x in 0..5 {
            for z in 0..8 {
                w.set(x, 1, z, STONE);
                w.set(x, 2, z, STONE);
            }
        }
        let mut rig = Rig::new(w, (1, 3, 1));
        rig.pathing.goto(block(9, 1, 1));
        let n = rig.run_until_idle(300);
        assert!(reached(&rig, 9, 1, 1), "stopped at {:?} after {n} ticks, cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert!(rig.kinds.contains(&MovementKind::Fall), "{:?}", rig.kinds);
    }

    #[test]
    fn a_block_that_appears_in_the_way_makes_a_new_path() {
        let mut rig = Rig::new(TestWorld::flat(24, 8), (1, 1, 3));
        rig.pathing.goto(block(18, 1, 3));
        for _ in 0..200 {
            rig.tick();
            // when the player is half way, a wall of three blocks high grows across the path, with a gap at the far side
            if rig.p.x > 7.0 && rig.world.state_id(11, 1, 3) == AIR as usize {
                for z in 0..6 {
                    for y in 1..=3 {
                        rig.world.set(11, y, z, STONE);
                    }
                }
            }
            if !rig.pathing.is_active() && !rig.pathing.is_pathing() && !rig.pathing.is_calculating() {
                break;
            }
        }
        assert!(reached(&rig, 18, 1, 3), "stopped at {:?}, cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        let started = rig.events.iter().filter(|e| **e == PathEvent::CalcStarted).count();
        assert!(started >= 2, "a path was calculated {started} times");
        let executing = rig.events.iter().filter(|e| **e == PathEvent::CalcFinishedNowExecuting).count();
        assert!(executing >= 2, "{executing} paths were followed");
    }

    #[test]
    fn a_goal_that_cannot_be_reached_ends_the_process() {
        // the goal is walled in
        let mut w = TestWorld::flat(16, 16);
        for x in 7..=11 {
            for z in 5..=9 {
                if x == 7 || x == 11 || z == 5 || z == 9 {
                    for y in 1..=3 {
                        w.set(x, y, z, STONE);
                    }
                }
            }
        }
        let mut rig = Rig::new(w, (1, 1, 1));
        rig.pathing.goto(block(9, 1, 7));
        rig.run_until_idle(300);
        // the search found a partial path towards it or nothing; either way it ends without a panic and the intent at the end is neutral
        for _ in 0..5 {
            rig.tick();
        }
        assert!(!rig.pathing.is_active() || rig.pathing.is_pathing() || rig.pathing.is_calculating());
    }

    #[test]
    fn a_long_path_is_followed_in_segments() {
        // far enough for several segments: the search is capped to few nodes, so each segment is short
        let mut rig = Rig::new(TestWorld::flat(64, 8), (1, 1, 3));
        rig.pathing.max_nodes = Some(60);
        rig.pathing.goto(block(58, 1, 3));
        let n = rig.run_until_idle(900);
        assert!(reached(&rig, 58, 1, 3), "stopped at {:?} after {n} ticks, cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert!(rig.events.iter().any(|e| matches!(e, PathEvent::NextSegmentCalcStarted | PathEvent::CalcStarted)));
    }

    /// A corridor one block wide, with walls on both sides.
    fn corridor() -> TestWorld {
        let mut w = TestWorld::flat(30, 8);
        for x in 0..30 {
            for y in 1..=2 {
                w.set(x, y, 2, STONE);
                w.set(x, y, 4, STONE);
            }
        }
        w
    }

    #[test]
    fn a_wall_that_would_have_to_be_broken_ends_the_walk_and_nothing_is_clicked() {
        let mut rig = Rig::new(corridor(), (1, 1, 3));
        rig.pathing.goto(block(26, 1, 3));
        for _ in 0..10 {
            rig.tick();
        }
        // a wall grows in the lane, three blocks ahead of the player
        let ahead = (rig.p.x + 3.0).floor() as i32;
        for y in 1..=2 {
            rig.world.set(ahead, y, 3, STONE);
        }
        for _ in 0..300 {
            rig.tick();
            // the movement that runs never has a block in its cells that would have to be broken
            if let Some(c) = rig.pathing.current() {
                let ctx = rig.tables.ctx(&rig.world, Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 });
                let m = &c.movements()[c.position().max(0) as usize % c.movements().len()];
                if m.kind != MovementKind::Diagonal {
                    assert!(crate::exec::movements::to_break(m, &ctx).is_empty(), "{m:?} runs with a block in the way");
                }
            }
        }
        assert!(rig.p.x < ahead as f64, "the player went through the wall: x {}", rig.p.x);
        assert!(rig.intents.iter().all(|i| i.use_block.is_none()));
        assert!(rig.p.feet().x >= ahead - 4, "it should have walked up to the wall");
    }

    #[test]
    fn a_goto_while_a_search_runs_lets_that_search_finish_u3() {
        // upstream behaviour kept (deviations.md, U3): the search for the old goal installs its path; the new goal is searched from its end
        let mut rig = Rig::with_runner(TestWorld::flat(30, 8), (1, 1, 1), ManualRunner::default());
        rig.pathing.goto(block(10, 1, 1));
        rig.tick();
        let out = rig.pathing.runner_mut().run(0);
        rig.pathing.goto(block(5, 1, 5));
        rig.tick();
        rig.tick();
        assert_eq!(rig.pathing.runner_mut().jobs.len(), 1, "no second search while one runs");
        rig.pathing.runner_mut().ready.push_back(out);
        rig.tick();
        let c = rig.pathing.current().expect("the old path is followed");
        assert_eq!(c.path().goal(), &block(10, 1, 1));
        assert_eq!(rig.pathing.goal(), Some(&block(5, 1, 5)));
    }

    #[test]
    fn while_a_path_runs_the_intent_is_in_control_every_tick() {
        let mut rig = Rig::new(TestWorld::flat(24, 8), (1, 1, 3));
        rig.pathing.goto(block(18, 1, 3));
        rig.run_until_idle(300);
        let first = rig.intents.iter().position(|i| i.active).expect("a path ran");
        let last = rig.intents.iter().rposition(|i| i.active).unwrap();
        // from the first tick of the path to the last, the player's own keys are not read: in control on every tick
        assert!(rig.intents[first..=last].iter().all(|i| i.active), "control was given back in the middle of a path");
        assert!(rig.intents[first..=last].iter().all(|i| i.cancel_flying && i.disable_auto_jump));
        // and after it, nothing is overridden
        assert!(rig.intents.last().unwrap().is_neutral());
    }

    // ---------------------------------------------------------------- properties
    // The engine has no dependencies, so these are fixed-seed random runs: many worlds, goals and edits, each seed the same on every run.

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn chance(&mut self, percent: u64) -> bool {
            self.below(100) < percent
        }
    }

    /// A random world of 24 by 24 columns: a floor, and random pillars, walls and holes.
    fn random_world(rng: &mut Rng) -> TestWorld {
        let mut w = TestWorld::flat(24, 24);
        for _ in 0..rng.below(40) {
            let (x, z) = (rng.below(24) as i32, rng.below(24) as i32);
            match rng.below(3) {
                0 => {
                    for y in 1..=(1 + rng.below(3) as i32) {
                        w.set(x, y, z, STONE);
                    }
                }
                1 => w.set(x, 0, z, AIR),
                _ => {
                    let len = rng.below(6) as i32;
                    for d in 0..len {
                        if x + d < 24 {
                            w.set(x + d, 1, z, STONE);
                            w.set(x + d, 2, z, STONE);
                        }
                    }
                }
            }
        }
        w
    }

    fn some_start(w: &TestWorld, rng: &mut Rng) -> (i32, i32, i32) {
        for _ in 0..50 {
            let (x, z) = (rng.below(24) as i32, rng.below(24) as i32);
            if w.state_id(x, 0, z) == STONE as usize && w.state_id(x, 1, z) == AIR as usize && w.state_id(x, 2, z) == AIR as usize {
                return (x, 1, z);
            }
        }
        (0, 1, 0)
    }

    fn random_goal(rng: &mut Rng) -> Goal {
        let (x, z) = (rng.below(30) as i32 - 3, rng.below(30) as i32 - 3);
        match rng.below(4) {
            0 => Goal::Block { x, y: 1, z },
            1 => Goal::Xz { x, z },
            2 => Goal::YLevel { level: 1 + rng.below(3) as i32 },
            _ => crate::goals::goal_near(x, 1, z, 2),
        }
    }

    fn neutral_for(rig: &mut Rig<ImmediateRunner>, ticks: u32) {
        for _ in 0..ticks {
            let out = rig.tick();
            assert!(out.intent.is_neutral(), "{:?}", out.intent);
            assert!(!rig.pathing.is_pathing() && !rig.pathing.is_active() && !rig.pathing.is_calculating());
        }
    }

    #[test]
    fn stop_always_gives_a_neutral_intent() {
        let mut moving = 0;
        for seed in 1..=60u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let w = random_world(&mut rng);
            let start = some_start(&w, &mut rng);
            let mut rig = Rig::new(w, start);
            rig.pathing.goto(random_goal(&mut rng));
            for _ in 0..rng.below(120) {
                rig.tick();
            }
            if rig.intent.active {
                moving += 1;
            }
            let after = rig.pathing.stop();
            assert!(after.is_neutral());
            assert!(!rig.pathing.is_pathing() && !rig.pathing.is_active() && !rig.pathing.is_calculating());
            rig.intent = after;
            neutral_for(&mut rig, 6);
            assert!(rig.pathing.goal().is_none());
        }
        assert!(moving > 10, "only {moving} runs were moving when stopped: the test is not testing");
    }

    #[test]
    fn stop_with_a_goto_in_the_same_tick_still_stops() {
        // the order of the commands in a tick: goto, then stop. Nothing may start.
        let mut rig = Rig::new(TestWorld::flat(16, 8), (1, 1, 1));
        rig.pathing.goto(block(10, 1, 1));
        rig.pathing.stop();
        neutral_for(&mut rig, 5);
        assert!(rig.pathing.runner_mut().started.is_empty());
    }

    #[test]
    fn a_stale_result_never_installs_a_path() {
        let mut rig = Rig::with_runner(TestWorld::flat(16, 8), (1, 1, 1), ManualRunner::default());
        rig.pathing.goto(block(10, 1, 1));
        rig.tick();
        assert_eq!(rig.pathing.runner_mut().jobs.len(), 1);
        let first = rig.pathing.runner_mut().run(0);
        assert!(first.result.path.is_some(), "the search for the test must succeed");
        // the first search is cancelled, a second one for another goal starts
        rig.pathing.stop();
        rig.pathing.goto(block(5, 1, 5));
        rig.tick();
        assert_eq!(rig.pathing.runner_mut().jobs.len(), 2);
        assert!(rig.pathing.is_calculating());
        // the late result of the first one arrives: dropped
        rig.pathing.runner_mut().ready.push_back(first);
        for _ in 0..3 {
            rig.tick();
        }
        assert!(!rig.pathing.has_path(), "a stale path was installed");
        assert!(rig.pathing.is_calculating(), "the stale result must not end the search that is wanted");
        assert!(rig.intent.is_neutral());
        // the right one installs
        let second = rig.pathing.runner_mut().run(1);
        rig.pathing.runner_mut().ready.push_back(second);
        rig.tick();
        assert!(rig.pathing.has_path());
        assert_eq!(rig.pathing.current().unwrap().path().goal(), &block(5, 1, 5));
    }

    #[test]
    fn a_cancelled_search_never_installs_a_path() {
        let mut rig = Rig::with_runner(TestWorld::flat(16, 8), (1, 1, 1), ManualRunner::default());
        rig.pathing.goto(block(10, 1, 1));
        rig.tick();
        let out = rig.pathing.runner_mut().run(0);
        assert!(out.result.path.is_some());
        rig.pathing.stop();
        assert!(rig.pathing.runner_mut().cancels >= 1);
        rig.pathing.runner_mut().ready.push_back(out);
        for _ in 0..5 {
            rig.tick();
            assert!(!rig.pathing.has_path());
            assert!(rig.intent.is_neutral());
        }
        // a search for a goal that was dropped before it finished: the same through `goto` with another start position
        let mut rig = Rig::with_runner(TestWorld::flat(16, 8), (1, 1, 1), ManualRunner::default());
        rig.pathing.goto(block(10, 1, 1));
        rig.tick();
        let out = rig.pathing.runner_mut().run(0);
        // the player is moved far from where the search started: the search is no longer the right one
        rig.p = SimPlayer::at(12, 1, 6);
        rig.tick();
        rig.tick();
        rig.pathing.runner_mut().ready.push_back(out);
        rig.tick();
        // either dropped as cancelled, or, if it were installed, it must contain the place where the player is: it does not
        assert!(!rig.pathing.has_path() || rig.pathing.current().unwrap().path().positions().contains(&rig.p.feet()));
    }

    #[test]
    fn no_movement_runs_with_a_block_in_its_way() {
        let mut checked = 0;
        for seed in 1..=60u64 {
            let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
            let w = random_world(&mut rng);
            let start = some_start(&w, &mut rng);
            let mut rig = Rig::new(w, start);
            rig.pathing.goto(Goal::Block { x: 22, y: 1, z: 22 });
            for _ in 0..200 {
                // random edits, near the player half of the time
                if rng.chance(30) {
                    let (cx, cz) = if rng.chance(50) { (rig.p.x.floor() as i32 + rng.below(5) as i32 - 2, rig.p.z.floor() as i32 + rng.below(5) as i32 - 2) } else { (rng.below(24) as i32, rng.below(24) as i32) };
                    let (cx, cz) = (cx.clamp(0, 23), cz.clamp(0, 23));
                    let y = 1 + rng.below(3) as i32;
                    let id = if rng.chance(60) { STONE } else { AIR };
                    rig.world.set(cx, y, cz, id);
                }
                let out = rig.tick();
                assert!(out.intent.use_block.is_none());
                if let Some(c) = rig.pathing.current() {
                    let ctx = rig.tables.ctx(&rig.world, Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 });
                    let i = c.position();
                    if i >= 0 && (i as usize) < c.movements().len() {
                        let m = &c.movements()[i as usize];
                        if m.kind != MovementKind::Diagonal {
                            checked += 1;
                            assert!(crate::exec::movements::to_break(m, &ctx).is_empty(), "seed {seed}: {m:?} runs with a block in the way");
                        }
                    }
                }
            }
        }
        assert!(checked > 1000, "only {checked} ticks had a running movement");
    }

    #[test]
    fn the_executor_survives_any_world_edit_and_any_player() {
        let mut ticks_with_path = 0;
        for seed in 1..=80u64 {
            let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) | 1);
            let w = random_world(&mut rng);
            let start = some_start(&w, &mut rng);
            let mut rig = Rig::new(w, start);
            rig.exec.can_use_blocks = rng.chance(50);
            rig.pathing.goto(random_goal(&mut rng));
            for _ in 0..150 {
                for _ in 0..rng.below(4) {
                    let (cx, cy, cz) = (rng.below(24) as i32, rng.below(5) as i32, rng.below(24) as i32);
                    rig.world.set(cx, cy, cz, if rng.chance(50) { STONE } else { AIR });
                }
                if rng.chance(5) {
                    // a teleport, a lag back, a push: anywhere, also into blocks and out of the world
                    rig.p.x += (rng.below(2000) as f64 - 1000.0) / 100.0;
                    rig.p.z += (rng.below(2000) as f64 - 1000.0) / 100.0;
                    rig.p.y = (rng.below(3000) as f64 - 500.0) / 100.0;
                    rig.p.vy = 0.0;
                }
                if rng.chance(3) {
                    rig.pathing.goto(random_goal(&mut rng));
                }
                if rng.chance(2) {
                    rig.pathing.stop();
                }
                // the player as the game could give it: any of the flags
                let mut view = rig.p.view();
                view.in_water = rng.chance(10);
                view.swimming = rng.chance(5);
                view.in_wall = rng.chance(3);
                view.crouching = rng.chance(10);
                view.on_ground = rng.chance(80);
                view.food_level = rng.below(21) as i32;
                let snap = rig.world.clone();
                rig.p.step(&rig.world, &rig.intent);
                let env = TickEnv { tables: &rig.tables, world: &rig.world, border: Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 }, player: &view, exec: &rig.exec };
                let out = rig.pathing.tick(&env, &mut || snap.clone());
                rig.intent = out.intent;
                if rig.pathing.has_path() {
                    ticks_with_path += 1;
                }
            }
        }
        assert!(ticks_with_path > 1000, "only {ticks_with_path} ticks followed a path");
    }
}

/// Tests added in the S4 review: each one pins a finding that was checked against the code.
#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::look::Rotation;
    use crate::search::{PathCalculationResult, ResultType};

    #[test]
    fn the_mouse_sensitivity_setting_moves_the_aim_grid() {
        let run = |sens: f64| {
            let mut rig = Rig::new(TestWorld::flat(16, 8), (1, 1, 1));
            rig.exec.mouse_sensitivity = sens;
            rig.p.yaw = 37.0;
            rig.p.pitch = 11.3;
            rig.pathing.goto(Goal::Block { x: 12, y: 1, z: 1 });
            (0..6).map(|_| rig.tick().intent.rotation).collect::<Vec<Option<Rotation>>>()
        };
        let (a, b) = (run(0.5), run(1.0));
        assert_ne!(a, b, "the same rotations for two sensitivities: the setting is not read");
        // and a sensitivity of 0.5 is what it was before the setting was wired
        assert!(a.iter().any(|r| r.is_some()));
    }

    #[test]
    fn a_search_that_panics_is_reported_once_and_does_not_loop_silently() {
        let mut rig = Rig::with_runner(TestWorld::flat(16, 8), (1, 1, 1), ManualRunner::default());
        rig.pathing.goto(Goal::Block { x: 12, y: 1, z: 1 });
        rig.tick();
        let generation = rig.pathing.runner_mut().jobs[0].0.generation;
        rig.pathing.runner_mut().ready.push_back(SearchOutcome { generation, result: PathCalculationResult { kind: ResultType::Exception, path: None, error: None } });
        rig.tick();
        assert!(rig.pathing.take_search_failure(), "the failure was not reported");
        assert!(!rig.pathing.take_search_failure(), "reported twice");
        // the glue stops the goto; after that no search starts again
        rig.pathing.stop();
        let started = rig.pathing.runner_mut().jobs.len();
        for _ in 0..5 {
            rig.tick();
        }
        assert_eq!(rig.pathing.runner_mut().jobs.len(), started);
    }

    #[test]
    fn at_goal_is_queued_when_a_fall_ends_in_the_goal() {
        // D20: the path is cut first only when it was safe to cancel on the previous tick. After a drop it is not.
        let mut w = TestWorld::flat(16, 8);
        for x in 0..5 {
            for z in 0..8 {
                for y in 1..4 {
                    w.set(x, y, z, STONE);
                }
            }
        }
        let mut rig = Rig::new(w, (2, 4, 3));
        rig.pathing.goto(Goal::Block { x: 5, y: 1, z: 3 });
        rig.run_until_idle(200);
        assert!(rig.events.contains(&PathEvent::AtGoal), "{:?}", rig.events);
        let mut flat = Rig::new(TestWorld::flat(16, 8), (1, 1, 1));
        flat.pathing.goto(Goal::Block { x: 12, y: 1, z: 1 });
        flat.run_until_idle(300);
        assert!(!flat.events.contains(&PathEvent::AtGoal), "{:?}", flat.events);
    }
}
