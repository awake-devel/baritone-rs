// baritone-rs engine: a small simulator of walking, for the tests of the executor and the pathing state machine. Written for this
// project. LGPL-3.0-or-later, see lib.rs.
//
// THE SIMULATOR IS NOT EVIDENCE OF REAL PHYSICS. It moves a box through a grid of full blocks at fixed speeds (4.317 blocks/s walking,
// 5.612 sprinting, 1.295 sneaking, as the cost constants say), with a jump of 0.42, gravity of 0.08 and drag 0.98 per tick, and no
// friction, no slabs and no water. A sneaking player does not walk off an edge. It follows `Intent`s, in the order Horizon runs a tick:
// the game's input step first (the view the intent asked for, then the held buttons: a dig that takes the ticks the tool says, a block
// placed against the face the crosshair is on), then the player moves with the last intent, then the mod ticks. Its crosshair is found by
// marching along the look vector in small steps, which shares nothing with the engine's ray trace. It shows that the rules of the
// executor lead a player to a goal in a world shaped like this, and that the state machine around them behaves. Whether a real player
// does the same in the game is for the L3 scenarios.
use crate::blocks::Direction;
use crate::config::Config;
use crate::exec::movements::GameFacts;
use crate::exec::state::{Intent, PlayerView};
use crate::goals::Goal;
use crate::inventory::testing::TestInventory;
use crate::inventory::{get_str_vs_block, has_generic_throwaway, Inventory, SlotInfo, ToolSettings};
use crate::look::Vec3;
use crate::movement::helper::Border;
use crate::pos::Pos;
use crate::process::pathing::{run_search, PathEvent, Pathing, SearchJob, SearchOutcome, SearchRunner, Tables, TickEnv, TickOutput};
use crate::process::{ItemEntity, ProcessInputs};
use crate::raytrace::BlockHit;
use crate::settings::ExecSettings;
use crate::testing::{Fixture, TestWorld, AIR, ORE, STONE};
use crate::world::WorldView;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const WALK: f64 = 4.317 / 20.0;
const SPRINT: f64 = 5.612 / 20.0;
const HALF_WIDTH: f64 = 0.3;
const HEIGHT: f64 = 1.8;

fn solid(w: &TestWorld, x: i32, y: i32, z: i32) -> bool {
    (0..16).contains(&y) && matches!(w.state_id(x, y, z), 1 | 2)
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
    /// The pose: crouching from the tick after sneaking was asked for.
    pub crouching: bool,
}

const SNEAK_FACTOR: f64 = 0.3;
const REACH: f64 = 4.5;

impl SimPlayer {
    /// Standing in the middle of a block, looking south.
    pub fn at(x: i32, y: i32, z: i32) -> SimPlayer {
        SimPlayer { x: x as f64 + 0.5, y: y as f64, z: z as f64 + 0.5, vx: 0.0, vy: 0.0, vz: 0.0, yaw: 0.0, pitch: 0.0, on_ground: true, horizontal_collision: false, crouching: false }
    }

    pub fn eye_height(&self) -> f64 {
        if self.crouching {
            1.27
        } else {
            1.62
        }
    }

    pub fn view(&self) -> PlayerView {
        PlayerView { x: self.x, y: self.y, z: self.z, vx: self.vx, vy: self.vy, vz: self.vz, yaw: self.yaw, pitch: self.pitch, on_ground: self.on_ground, horizontal_collision: self.horizontal_collision, crouching: self.crouching, eye_height: self.eye_height(), ..PlayerView::standing(0.0, 0.0, 0.0) }
    }

    /// The view the intent asked for: the glue sets the yaw and the pitch at the end of the mod's tick, before the game's next input step.
    pub fn apply_rotation(&mut self, i: &Intent) {
        if let Some(r) = i.rotation {
            self.yaw = r.yaw;
            self.pitch = r.pitch;
        }
    }

    /// What the crosshair is on: the first solid cube along the look vector within reach, found by marching in steps of
    /// 1/2000 of a block. The face is the one the last step came in by.
    pub fn looking_at(&self, w: &TestWorld) -> Option<BlockHit> {
        let (yaw, pitch) = ((self.yaw as f64).to_radians(), (self.pitch as f64).to_radians());
        let d = (-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos());
        let eye = (self.x, self.y + self.eye_height(), self.z);
        let cell = |t: f64| ((eye.0 + d.0 * t).floor() as i32, (eye.1 + d.1 * t).floor() as i32, (eye.2 + d.2 * t).floor() as i32);
        let mut prev = cell(0.0);
        if solid(w, prev.0, prev.1, prev.2) {
            return Some(BlockHit { pos: Pos::new(prev.0, prev.1, prev.2), face: Direction::Up, point: Vec3::new(eye.0, eye.1, eye.2) });
        }
        let n = (REACH * 2000.0) as i32;
        for i in 1..=n {
            let t = f64::from(i) / 2000.0;
            let c = cell(t);
            if c != prev {
                if solid(w, c.0, c.1, c.2) {
                    let face = if c.0 != prev.0 {
                        if c.0 > prev.0 {
                            Direction::West
                        } else {
                            Direction::East
                        }
                    } else if c.1 != prev.1 {
                        if c.1 > prev.1 {
                            Direction::Down
                        } else {
                            Direction::Up
                        }
                    } else if c.2 > prev.2 {
                        Direction::North
                    } else {
                        Direction::South
                    };
                    return Some(BlockHit { pos: Pos::new(c.0, c.1, c.2), face, point: Vec3::new(eye.0 + d.0 * t, eye.1 + d.1 * t, eye.2 + d.2 * t) });
                }
                prev = c;
            }
        }
        None
    }

    /// Whether the player's box overlaps a block cell.
    pub fn overlaps_cell(&self, x: i32, y: i32, z: i32) -> bool {
        self.x - HALF_WIDTH < f64::from(x + 1) && self.x + HALF_WIDTH > f64::from(x) && self.y < f64::from(y + 1) && self.y + HEIGHT > f64::from(y) && self.z - HALF_WIDTH < f64::from(z + 1) && self.z + HALF_WIDTH > f64::from(z)
    }

    fn box_collides(&self, w: &TestWorld, x: f64, y: f64, z: f64) -> bool {
        for cy in (y + 1e-6).floor() as i32..=(y + HEIGHT - 1e-6).floor() as i32 {
            for cx in (x - HALF_WIDTH + 1e-6).floor() as i32..=(x + HALF_WIDTH - 1e-6).floor() as i32 {
                for cz in (z - HALF_WIDTH + 1e-6).floor() as i32..=(z + HALF_WIDTH - 1e-6).floor() as i32 {
                    if solid(w, cx, cy, cz) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Vanilla's `Player.maybeBackOffFromEdge`: a sneaking player on the ground does not move to where a box 0.6 lower would hang in the air.
    fn back_off_from_edge(&self, w: &TestWorld, mut dx: f64, mut dz: f64) -> (f64, f64) {
        if !self.crouching || !self.on_ground {
            return (dx, dz);
        }
        let step = |d: f64| {
            if (-0.05..0.05).contains(&d) {
                0.0
            } else if d > 0.0 {
                d - 0.05
            } else {
                d + 0.05
            }
        };
        while dx != 0.0 && !self.box_collides(w, self.x + dx, self.y - 0.6, self.z) {
            dx = step(dx);
        }
        while dz != 0.0 && !self.box_collides(w, self.x, self.y - 0.6, self.z + dz) {
            dz = step(dz);
        }
        while dx != 0.0 && dz != 0.0 && !self.box_collides(w, self.x + dx, self.y - 0.6, self.z + dz) {
            dx = step(dx);
            dz = step(dz);
        }
        (dx, dz)
    }

    pub fn feet(&self) -> Pos {
        self.view().feet()
    }

    /// One tick of movement with an intent. A neutral intent leaves the player standing (the real keys are not simulated).
    pub fn step(&mut self, w: &TestWorld, i: &Intent) {
        self.apply_rotation(i);
        self.crouching = i.active && i.sneak;
        let (f, s) = if i.active { (i.forward_impulse() as f64, i.strafe_impulse() as f64) } else { (0.0, 0.0) };
        let sprinting = i.active && i.sprint && f > 0.0 && !self.crouching;
        let speed = if sprinting {
            SPRINT
        } else if self.crouching {
            WALK * SNEAK_FACTOR
        } else {
            WALK
        };
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
        let (dx, dz) = self.back_off_from_edge(w, mx * speed, mz * speed);
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
    /// The hotbar of the player. Its slot in hand follows `Intent::select_slot`.
    pub inv: TestInventory,
    pub game: GameFacts,
    /// The settings of the tables when they follow the hotbar every tick (`Rig::configured`): `has_throwaway` and the tool strength
    /// are then made from it, as the glue does.
    configured: Option<(Config, bool)>,
    /// The dig in progress: the block and how much of it is done.
    pub dig: Option<(Pos, f64)>,
    dig_delay: i32,
    use_cooldown: i32,
    /// Every block the game broke for the player, and every block it placed, in order.
    pub broken: Vec<Pos>,
    pub placed: Vec<Pos>,
    /// The ticks in which an intent held the left button, and the ticks in which one pressed the right one.
    pub attack_ticks: u32,
    pub use_ticks: u32,
    /// The dropped items the processes see (`ProcessInputs::item_entities`).
    pub entities: Vec<ItemEntity>,
    /// Everything the processes said, in order.
    pub messages: Vec<String>,
    /// The scans that were run between ticks, and the position of the player at the time of each one.
    pub scans_run: u32,
    /// The ticks in which the process in control was a given one: (tick, mine active, get-to-block active, goal process active).
    pub who: Vec<(u32, bool, bool, bool)>,
}

impl<R: SearchRunner<TestWorld>> Rig<R> {
    pub fn with_runner(world: TestWorld, start: (i32, i32, i32), runner: R) -> Rig<R> {
        let fx = Fixture::with_ore(crate::testing::test_config());
        let tables = Arc::new(Tables::new(fx.states.clone(), fx.cfg.clone(), vec![1.0, 1.0, 1.0]));
        let mut pathing = Pathing::new(runner);
        pathing.max_nodes = Some(5000);
        Rig {
            tables,
            world,
            p: SimPlayer::at(start.0, start.1, start.2),
            pathing,
            exec: ExecSettings::default(),
            intent: Intent::neutral(),
            events: vec![],
            ticks: 0,
            intents: vec![],
            kinds: vec![],
            inv: TestInventory::empty(),
            game: GameFacts::default(),
            configured: None,
            dig: None,
            dig_delay: 0,
            use_cooldown: 0,
            broken: vec![],
            placed: vec![],
            attack_ticks: 0,
            use_ticks: 0,
            entities: vec![],
            messages: vec![],
            scans_run: 0,
            who: vec![],
        }
    }

    /// A rig whose tables follow the hotbar: `cfg` holds the settings (`allow_break` and the others), and every tick `has_throwaway` is
    /// `allow_place` and a block to place in the hand or the hotbar, and the strength of each block is the best tool's.
    pub fn configured(world: TestWorld, start: (i32, i32, i32), runner: R, cfg: Config, allow_place: bool, inv: TestInventory) -> Rig<R> {
        let mut rig = Rig::with_runner(world, start, runner);
        rig.configured = Some((cfg, allow_place));
        rig.exec.allow_place = allow_place;
        rig.inv = inv;
        rig.refresh_tables();
        rig
    }

    /// What the glue does at the top of each tick: the settings with the facts of the hotbar.
    fn refresh_tables(&mut self) {
        let Some((cfg, allow_place)) = &self.configured else {
            return;
        };
        let fx = Fixture::with_ore(cfg.clone());
        let mut cfg = cfg.clone();
        cfg.has_throwaway = *allow_place && has_generic_throwaway(&self.inv, &self.exec.tool.acceptable_throwaway_items);
        let ts: &ToolSettings = &self.exec.tool;
        let strength = vec![1.0, get_str_vs_block(&self.inv, 1, "stone", ts), get_str_vs_block(&self.inv, ORE as usize, "diamond_ore", ts)];
        self.tables = Arc::new(Tables::new(fx.states, cfg, strength));
    }

    /// The game's input step for the player, before the player moves: the held buttons act on what the crosshair is on.
    fn interact(&mut self, i: &Intent) {
        if self.use_cooldown > 0 {
            self.use_cooldown -= 1;
        }
        let hit = self.p.looking_at(&self.world);
        if i.attack {
            self.attack_ticks += 1;
            match hit {
                Some(h) if matches!(self.world.state_id(h.pos.x, h.pos.y, h.pos.z), 1 | 2) => {
                    if self.dig_delay > 0 {
                        self.dig_delay -= 1;
                    } else if let Some((pos, done)) = self.dig {
                        if pos == h.pos {
                            let done = done + self.inv.speed_vs_state(self.inv.selected, self.world.state_id(pos.x, pos.y, pos.z));
                            if done >= 1.0 {
                                if self.world.state_id(pos.x, pos.y, pos.z) == ORE as usize {
                                    // the drop goes straight into the inventory: picking up is not what these tests are about
                                    self.inv.main.push(SlotInfo::of("diamond", 1));
                                }
                                self.world.set(pos.x, pos.y, pos.z, AIR);
                                self.broken.push(pos);
                                self.dig = None;
                                self.dig_delay = 5;
                            } else {
                                self.dig = Some((pos, done));
                            }
                        } else {
                            self.dig = Some((h.pos, 0.0));
                        }
                    } else {
                        self.dig = Some((h.pos, 0.0)); // the tick that sends the start packet
                    }
                }
                _ => self.dig = None,
            }
        } else {
            self.dig = None;
        }
        if i.use_item && !i.attack {
            self.use_ticks += 1;
            if self.use_cooldown == 0 {
                self.use_cooldown = 4;
                let held = self.inv.slots[self.inv.selected].clone();
                if let (Some(h), true) = (hit, !held.is_empty() && matches!(held.item.as_str(), "stone" | "dirt" | "cobblestone" | "netherrack")) {
                    let at = h.pos.relative(h.face, 1);
                    let inside = at.x >= 0 && at.x < self.world.w && at.z >= 0 && at.z < self.world.d && (0..16).contains(&at.y);
                    if inside && self.world.state_id(at.x, at.y, at.z) == AIR as usize && !self.p.overlaps_cell(at.x, at.y, at.z) {
                        self.world.set(at.x, at.y, at.z, STONE);
                        self.placed.push(at);
                        let slot = &mut self.inv.slots[self.inv.selected];
                        slot.count -= 1;
                        if slot.count == 0 {
                            *slot = SlotInfo::empty();
                        }
                    }
                }
            }
        }
    }

    /// One game tick, in Horizon's order: the input step (the view the last intent set, then its buttons), the player moves with the last
    /// intent, then the mod ticks and makes the next one.
    pub fn tick(&mut self) -> TickOutput {
        self.refresh_tables();
        let last = self.intent.clone();
        self.p.apply_rotation(&last);
        self.interact(&last);
        self.p.step(&self.world, &last);
        let mut view = self.p.view();
        view.looking_at = self.p.looking_at(&self.world);
        // the head is in a block (`Entity.isInWall`)
        let head = (self.p.x.floor() as i32, (self.p.y + self.p.eye_height()).floor() as i32, self.p.z.floor() as i32);
        view.in_wall = solid(&self.world, head.0, head.1, head.2);
        let snap = self.world.clone();
        let env = TickEnv { tables: &self.tables, world: &self.world, border: Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 }, player: &view, exec: &self.exec, inv: &self.inv, game: self.game };
        self.pathing.set_inputs(ProcessInputs { now_ms: i64::from(self.ticks) * 50, item_entities: self.entities.clone(), container_open: false });
        let out = self.pathing.tick(&env, &mut || snap.clone());
        // the scans the processes asked for run between ticks, as the glue's worker does, and their answers come in before the next one
        for job in self.pathing.take_scan_jobs() {
            let result = job.run(&self.tables.ctx(&self.world, Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 }));
            self.pathing.deliver_scan(result, &self.tables.cfg, &self.exec);
            self.scans_run += 1;
        }
        self.messages.extend(self.pathing.take_messages());
        self.who.push((self.ticks, self.pathing.mine_process().is_active(), self.pathing.get_to_block_process().is_active(), self.pathing.process().is_active()));
        self.intent = out.intent.clone();
        if let Some(slot) = out.intent.select_slot {
            self.inv.selected = slot;
        }
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
    fn a_new_goal_forgets_the_last_goals_cancel() {
        // Found on 6b6t: the glue announced the previous goal's "cancelled: TimedOut" again when a new goto started.
        let mut rig = Rig::new(TestWorld::flat(24, 8), (1, 1, 3));
        rig.pathing.goto(block(18, 1, 3));
        for _ in 0..200 {
            rig.tick();
            if rig.p.x > 7.0 && rig.world.state_id(11, 1, 3) == AIR as usize {
                for z in 0..6 {
                    for y in 1..=3 {
                        rig.world.set(11, y, z, STONE);
                    }
                }
            }
            if rig.pathing.last_cancel().is_some() {
                break;
            }
        }
        assert!(rig.pathing.last_cancel().is_some(), "the wall should cancel the path");
        rig.pathing.goto(block(1, 1, 3));
        assert_eq!(rig.pathing.last_cancel(), None);
        rig.pathing.path();
        assert_eq!(rig.pathing.last_cancel(), None);
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
                let env = TickEnv { tables: &rig.tables, world: &rig.world, border: Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 }, player: &view, exec: &rig.exec, inv: &rig.inv, game: rig.game };
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

/// Paths that break, bridge, pillar and dig down, on the simulator (S4, breaking and placing). The simulator's dig takes the ticks the tool
/// says and its placing needs the face the crosshair is on, so these show that the executor's aim, sneak and tool rules work together; they do
/// not show that the game does the same (that is for the L3 scenarios).
#[cfg(test)]
mod break_place_tests {
    use super::*;
    use crate::inventory::SlotInfo;
    use crate::testing::test_config;
    use std::collections::HashSet;

    /// A hotbar with an iron pickaxe in slot 3 (the one that mines stone at 0.2 per tick) and 64 stone in slot 5; the hand is in slot 0.
    fn kit(blocks: u32) -> TestInventory {
        let mut inv = TestInventory::empty();
        inv.put(3, SlotInfo { material_cost: 2, pickaxe: true, ..SlotInfo::of("iron_pickaxe", 1) }, &[0.0, 0.2]);
        if blocks > 0 {
            inv.put(5, SlotInfo::of("stone", blocks), &[0.0, 0.0125]);
        }
        inv
    }

    fn rig_with(world: TestWorld, start: (i32, i32, i32), allow_break: bool, allow_place: bool, inv: TestInventory) -> Rig<ImmediateRunner> {
        let cfg = Config { allow_break, allow_downward: true, allow_place_source: true, allow_place_flow: true, ..test_config() };
        Rig::configured(world, start, ImmediateRunner::default(), cfg, allow_place, inv)
    }

    fn goal(x: i32, y: i32, z: i32) -> Goal {
        Goal::Block { x, y, z }
    }

    /// A corridor along x, one block wide at z = 3, with walls on both sides (too high to climb over) and the floor below them.
    fn corridor(len: i32) -> TestWorld {
        let mut w = TestWorld::flat(len, 8);
        for x in 0..len {
            for y in 1..=10 {
                w.set(x, y, 2, STONE);
                w.set(x, y, 4, STONE);
            }
        }
        w
    }

    #[test]
    fn a_block_landing_in_the_head_is_mined_and_the_walk_goes_on() {
        // upstream `Movement.update`: with the head in a block, the best tool and the left button on what the crosshair is on; no new path
        let mut rig = rig_with(corridor(20), (1, 1, 3), true, false, kit(0));
        rig.pathing.goto(goal(16, 1, 3));
        for _ in 0..12 {
            rig.tick();
        }
        assert!(rig.p.x > 2.0, "the walk should have started: x {}", rig.p.x);
        // the sand lands where the head is
        let (hx, hy, hz) = (rig.p.x.floor() as i32, (rig.p.y + rig.p.eye_height()).floor() as i32, rig.p.z.floor() as i32);
        rig.world.set(hx, hy, hz, STONE);
        let attacks_before = rig.attack_ticks;
        for _ in 0..40 {
            rig.tick();
            assert!(rig.pathing.last_cancel().is_none(), "the path was cancelled: {:?}", rig.pathing.last_cancel());
        }
        assert!(rig.attack_ticks > attacks_before, "the left button was never pressed");
        assert!(rig.broken.contains(&Pos::new(hx, hy, hz)), "the block in the head was not mined: {:?}", rig.broken);
        assert!(rig.intents.iter().any(|i| i.select_slot == Some(3)), "the pickaxe was not asked for");
        rig.run_until_idle(300);
        assert!(rig.p.x > 15.0, "the walk did not finish: x {}", rig.p.x);
    }

    /// Every cell the movements of every followed path would break or place, by the executor's own lists.
    fn planned<R: SearchRunner<TestWorld>>(rig: &Rig<R>, brk: &mut HashSet<Pos>, plc: &mut HashSet<Pos>) {
        if let Some(c) = rig.pathing.current() {
            for m in c.movements() {
                brk.extend(crate::exec::movements::positions_to_break(m));
                plc.extend(crate::exec::movements::position_to_place(m));
            }
        }
    }

    fn run(rig: &mut Rig<ImmediateRunner>, g: Goal, max: u32) -> u32 {
        rig.pathing.goto(g);
        rig.run_until_idle(max)
    }

    #[test]
    fn a_path_breaks_a_wall_with_the_best_tool() {
        let mut w = corridor(30);
        for y in 1..=2 {
            w.set(10, y, 3, STONE);
        }
        let mut rig = rig_with(w, (3, 1, 3), true, false, kit(0));
        rig.pathing.goto(goal(16, 1, 3));
        let mut changes = 0;
        for _ in 0..600 {
            let held = rig.inv.selected;
            let out = rig.tick();
            // a click never comes on the tick the slot changes, and the pickaxe is in the hand whenever the button goes down
            if out.intent.select_slot.is_some_and(|s| s != held) {
                changes += 1;
                assert!(!out.intent.attack && !out.intent.use_item, "a click with a slot change");
            }
            if out.intent.attack {
                assert_eq!(held, 3, "digging with slot {held}");
            }
            if !rig.pathing.is_active() && !rig.pathing.is_pathing() && !rig.pathing.is_calculating() {
                break;
            }
        }
        assert_eq!(rig.p.feet(), Pos::new(16, 1, 3), "cancel {:?}", rig.pathing.last_cancel());
        let mut broken = rig.broken.clone();
        broken.sort_by_key(|p| p.y);
        assert_eq!(broken, vec![Pos::new(10, 1, 3), Pos::new(10, 2, 3)]);
        assert!(rig.placed.is_empty() && rig.use_ticks == 0);
        assert!(rig.attack_ticks >= 10, "{} ticks of digging", rig.attack_ticks);
        assert_eq!(changes, 1, "the pickaxe is chosen once and kept");
        assert_eq!(rig.inv.selected, 3);
    }

    #[test]
    fn a_dig_takes_the_ticks_the_tool_says() {
        // 0.2 per tick: five ticks of progress, one to start, then the 5 ticks the game waits before the next block
        let mut w = corridor(30);
        w.set(10, 1, 3, STONE);
        let mut rig = rig_with(w, (7, 1, 3), true, false, kit(0));
        run(&mut rig, goal(14, 1, 3), 400);
        assert_eq!(rig.p.feet(), Pos::new(14, 1, 3));
        assert_eq!(rig.broken, vec![Pos::new(10, 1, 3)]);
        assert_eq!(rig.attack_ticks, 6, "one start tick and five of progress");
    }

    #[test]
    fn a_path_bridges_a_gap_with_a_floor_beside_it() {
        let mut w = corridor(30);
        for x in 10..=12 {
            w.set(x, 0, 3, AIR);
        }
        let mut rig = rig_with(w, (4, 1, 3), false, true, kit(64));
        let n = run(&mut rig, goal(18, 1, 3), 800);
        assert_eq!(rig.p.feet(), Pos::new(18, 1, 3), "stopped at {:?} after {n} ticks; cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert_eq!(rig.placed, vec![Pos::new(10, 0, 3), Pos::new(11, 0, 3), Pos::new(12, 0, 3)]);
        assert_eq!(rig.inv.slots[5].count, 61);
        assert!(rig.broken.is_empty() && rig.attack_ticks == 0, "broke {:?}", rig.broken);
        // it sneaked to place, and the block was placed while crouching
        assert!(rig.intents.iter().any(|i| i.sneak));
    }

    #[test]
    fn a_path_bridges_a_gap_with_nothing_around_it() {
        // a one block wide lane of floor in the void: the bridge block can only be placed against the block the player left
        let mut w = TestWorld::flat(30, 8);
        for x in 0..30 {
            for z in 0..8 {
                if z != 3 {
                    w.set(x, 0, z, AIR);
                }
            }
        }
        for x in 10..=11 {
            w.set(x, 0, 3, AIR);
        }
        let mut rig = rig_with(w, (4, 1, 3), false, true, kit(64));
        let n = run(&mut rig, goal(16, 1, 3), 900);
        assert_eq!(rig.p.feet(), Pos::new(16, 1, 3), "stopped at {:?} after {n} ticks; cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert_eq!(rig.placed, vec![Pos::new(10, 0, 3), Pos::new(11, 0, 3)]);
        assert_eq!(rig.inv.slots[5].count, 62);
    }

    #[test]
    fn a_path_pillars_up_and_places_a_block_under_its_feet_at_the_top_of_each_jump() {
        let mut rig = rig_with(TestWorld::flat(10, 10), (4, 1, 4), false, true, kit(64));
        let n = run(&mut rig, goal(4, 4, 4), 400);
        assert_eq!(rig.p.feet(), Pos::new(4, 4, 4), "stopped at {:?} after {n} ticks; cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert_eq!(rig.placed, vec![Pos::new(4, 1, 4), Pos::new(4, 2, 4), Pos::new(4, 3, 4)]);
        assert_eq!(rig.inv.slots[5].count, 61);
        assert_eq!(rig.inv.selected, 5);
    }

    #[test]
    fn a_path_digs_down() {
        let mut w = TestWorld::flat(10, 10);
        for x in 0..10 {
            for z in 0..10 {
                for y in 1..=3 {
                    w.set(x, y, z, STONE);
                }
            }
        }
        let mut rig = rig_with(w, (4, 4, 4), true, false, kit(0));
        let n = run(&mut rig, goal(4, 1, 4), 600);
        assert_eq!(rig.p.feet(), Pos::new(4, 1, 4), "stopped at {:?} after {n} ticks; cancel {:?}", rig.p.feet(), rig.pathing.last_cancel());
        assert_eq!(rig.broken, vec![Pos::new(4, 3, 4), Pos::new(4, 2, 4), Pos::new(4, 1, 4)]);
        assert!(rig.placed.is_empty());
    }

    #[test]
    fn the_executor_never_attacks_or_places_when_the_settings_forbid_it() {
        // a corridor with a wall, a gap and a ceiling to dig through; pickaxe and blocks in hand; breaking and placing both off
        let mut w = corridor(30);
        for y in 1..=2 {
            w.set(8, y, 3, STONE);
        }
        w.set(14, 0, 3, AIR);
        for allow in [(false, false), (true, false), (false, true)] {
            let mut rig = rig_with(w.clone(), (3, 1, 3), allow.0, allow.1, kit(64));
            rig.pathing.goto(goal(20, 1, 3));
            for _ in 0..500 {
                rig.tick();
                // a block that appears in the way while it walks
                if rig.ticks == 40 {
                    rig.world.set(12, 1, 3, STONE);
                }
            }
            if !allow.0 {
                assert_eq!(rig.attack_ticks, 0, "{allow:?}: it attacked");
                assert!(rig.broken.is_empty());
            }
            if !allow.1 {
                assert_eq!(rig.use_ticks, 0, "{allow:?}: it pressed use");
                assert!(rig.placed.is_empty());
            }
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// A random world with walls, gaps and ceilings, a wall across the whole width (with a hole half of the time) at x = 10 and a strip with no floor at
    /// x = 14, so that a path to x = 20 has to break and place. Returns the world and where the player starts.
    fn obstacle_world(seed: u64) -> (TestWorld, (i32, i32, i32)) {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut w = TestWorld::flat(24, 24);
        for _ in 0..60 {
            let (x, z) = (rng.below(24) as i32, rng.below(24) as i32);
            match rng.below(3) {
                0 => w.set(x, 0, z, AIR),
                _ => {
                    for y in 1..=(1 + rng.below(3) as i32) {
                        w.set(x, y, z, STONE);
                    }
                }
            }
        }
        let hole = rng.below(24) as i32;
        for z in 0..24 {
            if z != hole || rng.below(2) == 0 {
                for y in 1..=3 {
                    w.set(10, y, z, STONE);
                }
            }
            w.set(14, 0, z, AIR);
        }
        let start = (1 + rng.below(3) as i32, 1, 1 + rng.below(3) as i32);
        for y in 0..4 {
            w.set(start.0, y, start.2, if y == 0 { STONE } else { AIR });
        }
        (w, start)
    }

    #[test]
    fn what_is_broken_and_placed_is_what_the_path_planned() {
        // fixed seeds: random worlds, everything allowed
        let (mut dug, mut put) = (0, 0);
        for seed in 1..=40u64 {
            let (w, start) = obstacle_world(seed);
            let mut rig = rig_with(w, start, true, true, kit(64));
            rig.pathing.goto(goal(20, 1, 20));
            let (mut brk, mut plc) = (HashSet::new(), HashSet::new());
            for _ in 0..400 {
                planned(&rig, &mut brk, &mut plc);
                rig.tick();
                if let Some(i) = rig.intents.last() {
                    assert!(!(i.attack && i.use_item), "seed {seed}: both buttons");
                }
            }
            planned(&rig, &mut brk, &mut plc);
            for b in &rig.broken {
                assert!(brk.contains(b), "seed {seed}: broke {b:?}, which no movement planned to break");
            }
            for b in &rig.placed {
                assert!(plc.contains(b), "seed {seed}: placed {b:?}, which no movement planned to place");
            }
            dug += rig.broken.len();
            put += rig.placed.len();
        }
        assert!(dug > 20 && put > 5, "the seeds exercised {dug} breaks and {put} placements");
    }

    #[test]
    fn with_breaking_or_placing_off_the_button_is_never_pressed_in_random_worlds_with_edits() {
        // the same worlds, with a pickaxe and blocks in the hotbar, and blocks and holes that appear in the player's way while it walks
        let mut ticks_with_path = 0;
        for (allow_break, allow_place) in [(false, false), (true, false), (false, true)] {
            for seed in 1..=30u64 {
                let (w, start) = obstacle_world(seed + 100);
                let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
                let mut rig = rig_with(w, start, allow_break, allow_place, kit(64));
                rig.pathing.goto(goal(20, 1, 20));
                for _ in 0..300 {
                    if rng.below(100) < 8 {
                        let (px, pz) = (rig.p.x.floor() as i32, rig.p.z.floor() as i32);
                        let (cx, cz) = ((px + rng.below(4) as i32).clamp(0, 23), (pz + rng.below(4) as i32).clamp(0, 23));
                        if rng.below(2) == 0 {
                            rig.world.set(cx, 1, cz, STONE);
                            rig.world.set(cx, 2, cz, STONE);
                        } else {
                            rig.world.set(cx, 0, cz, AIR);
                        }
                    }
                    rig.tick();
                    if rig.pathing.has_path() {
                        ticks_with_path += 1;
                    }
                }
                if !allow_break {
                    assert_eq!(rig.attack_ticks, 0, "seed {seed}, break {allow_break}, place {allow_place}: the left button was pressed");
                    assert!(rig.broken.is_empty());
                }
                if !allow_place {
                    assert_eq!(rig.use_ticks, 0, "seed {seed}, break {allow_break}, place {allow_place}: the right button was pressed");
                    assert!(rig.placed.is_empty());
                }
            }
        }
        assert!(ticks_with_path > 3000, "only {ticks_with_path} ticks followed a path");
    }
}

/// The estimates behind `/b eta`.
#[cfg(test)]
mod eta_tests {
    use super::*;

    #[test]
    fn the_estimate_follows_how_far_the_player_got_and_is_zero_in_the_goal() {
        let mut rig = Rig::new(TestWorld::flat(40, 8), (1, 1, 1));
        let gs = rig.tables.cfg.goal_settings();
        rig.pathing.goto(Goal::Block { x: 30, y: 1, z: 1 });
        // nothing has been walked yet
        assert_eq!(rig.pathing.estimated_ticks_to_goal(rig.p.feet(), &gs), None);
        assert!(!rig.pathing.in_control());
        for _ in 0..40 {
            rig.tick();
        }
        assert!(rig.pathing.in_control());
        let eta = rig.pathing.estimated_ticks_to_goal(rig.p.feet(), &gs).expect("an estimate");
        // 30 blocks at about 5.6 blocks per second is 107 ticks; the estimate is made from the heuristic, not the costs, so it is near
        assert!(eta > 20.0 && eta < 300.0, "{eta}");
        let segment = rig.pathing.ticks_remaining_in_segment(true).expect("a segment");
        assert!(segment > 0.0 && segment < 200.0, "{segment}");
        assert!(rig.pathing.ticks_remaining_in_segment(false).unwrap() <= segment);
        rig.run_until_idle(400);
        // the process answered on the tick it arrived; one tick later it is not in control, and the goal is gone
        rig.tick();
        assert_eq!(rig.pathing.estimated_ticks_to_goal(rig.p.feet(), &gs), None, "the goal was dropped on arrival");
        assert!(!rig.pathing.in_control());
        assert_eq!(rig.pathing.ticks_remaining_in_segment(true), None);
        // a goal set where the player stands: the estimate is 0
        rig.pathing.set_goal(Goal::Block { x: rig.p.feet().x, y: 1, z: rig.p.feet().z });
        rig.tick();
        rig.tick();
        assert_eq!(rig.pathing.estimated_ticks_to_goal(rig.p.feet(), &gs), Some(0.0));
    }
}

/// `mine` and `goto <block>` with the simulated player: the processes, the manager that hands control between them, and the scans between ticks.
mod mine_tests {
    use super::*;
    use crate::blockmeta::{test_index, BlockOptionalMeta, BlockOptionalMetaLookup};
    use crate::testing::test_config;

    /// An iron pickaxe in slot 3 that mines stone and ore alike at 0.2 per tick.
    fn pick() -> TestInventory {
        let mut inv = TestInventory::empty();
        inv.put(3, SlotInfo { material_cost: 2, pickaxe: true, ..SlotInfo::of("iron_pickaxe", 1) }, &[0.0, 0.2, 0.2]);
        inv
    }

    fn rig(world: TestWorld, start: (i32, i32, i32)) -> Rig<ImmediateRunner> {
        let cfg = Config { allow_break: true, allow_downward: true, ..test_config() };
        Rig::configured(world, start, ImmediateRunner::default(), cfg, false, pick())
    }

    fn ore_lookup() -> BlockOptionalMetaLookup {
        BlockOptionalMetaLookup::from_names(&["diamond_ore"], &test_index()).unwrap()
    }

    fn ore_block() -> BlockOptionalMeta {
        BlockOptionalMeta::parse("diamond_ore", &test_index()).unwrap()
    }

    fn start_mine(rig: &mut Rig<ImmediateRunner>, quantity: i32) {
        let cfg = rig.tables.cfg.clone();
        rig.pathing.mine(quantity, ore_lookup(), &cfg);
    }

    fn ores_left(rig: &Rig<ImmediateRunner>) -> usize {
        let mut n = 0;
        for x in 0..rig.world.w {
            for y in 0..rig.world.h {
                for z in 0..rig.world.d {
                    if rig.world.state_id(x, y, z) == ORE as usize {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    fn open_field() -> TestWorld {
        TestWorld::flat(40, 12)
    }

    #[test]
    fn the_first_scan_is_a_job_and_the_process_holds_still_until_it_answers() {
        let mut w = open_field();
        w.set(10, 1, 3, ORE);
        let mut r = rig(w, (2, 1, 3));
        start_mine(&mut r, 0);
        r.tick();
        // the game thread did not scan: the answer came after the tick, from a job
        assert!(r.pathing.mine_process().known_ore_locations().len() == 1 && r.scans_run == 1);
        let at = (r.p.x, r.p.z);
        r.tick();
        assert_eq!((r.p.x, r.p.z), at, "no path was started before the answer was known");
        // the first tick only paused (the initial job was its one scan); the process's own tick 0, with its rescan, was this one
        assert_eq!(r.scans_run, 2);
        let used = r.run_until_idle(2000);
        assert!(used < 2000, "{:?}", r.messages);
        assert_eq!(ores_left(&r), 0);
    }

    #[test]
    fn mine_three_ores_around_and_stop_at_three() {
        let mut w = open_field();
        for (x, y, z) in [(10, 1, 3), (16, 1, 7), (22, 1, 4), (30, 1, 9)] {
            w.set(x, y, z, ORE);
        }
        let mut r = rig(w, (2, 1, 3));
        start_mine(&mut r, 3);
        let used = r.run_until_idle(3000);
        assert!(used < 3000, "the process never ended: {:?}", r.messages);
        assert!(r.messages.iter().any(|m| m == "Have 3 valid items"), "{:?}", r.messages);
        assert_eq!(ores_left(&r), 1, "three of the four were mined");
        assert!(r.world.state_id(10, 1, 3) == AIR as usize && r.world.state_id(16, 1, 7) == AIR as usize && r.world.state_id(22, 1, 4) == AIR as usize);
        assert!(!r.pathing.is_active());
        assert!(r.scans_run >= 2, "the world was rescanned while it walked: {}", r.scans_run);
    }

    #[test]
    fn mine_with_no_quantity_takes_every_ore_in_reach_and_then_stops_when_not_exploring() {
        let mut w = open_field();
        for (x, y, z) in [(8, 1, 3), (14, 1, 6), (20, 1, 2)] {
            w.set(x, y, z, ORE);
        }
        let mut r = rig(w, (2, 1, 3));
        r.exec.mine.explore_for_blocks = false;
        start_mine(&mut r, 0);
        let used = r.run_until_idle(3000);
        assert!(used < 3000, "never ended: {:?}", r.messages);
        assert_eq!(ores_left(&r), 0);
        assert!(!r.pathing.is_active());
    }

    #[test]
    fn mine_with_nothing_known_and_no_exploring_says_so_at_once() {
        let mut r = rig(open_field(), (2, 1, 3));
        r.exec.mine.explore_for_blocks = false;
        start_mine(&mut r, 0);
        r.tick();
        assert!(!r.pathing.is_active());
        assert!(r.messages.iter().any(|m| m.starts_with("No locations for BlockOptionalMetaLookup{") && m.ends_with("known, cancelling")), "{:?}", r.messages);
    }

    #[test]
    fn mine_refuses_blocks_it_may_not_break() {
        let cfg = Config { allow_break: false, ..test_config() };
        let mut r = Rig::configured(open_field(), (2, 1, 3), ImmediateRunner::default(), cfg.clone(), false, pick());
        r.pathing.mine(0, ore_lookup(), &cfg);
        assert!(!r.pathing.is_active());
        assert_eq!(r.pathing.take_messages(), vec!["Unable to mine when allowBreak is false and target block is not in allowBreakAnyway!".to_string()]);
        // in allowBreakAnyway it is fine
        let cfg = Config { allow_break: false, allow_break_anyway: vec!["diamond_ore".to_string()], ..test_config() };
        r.pathing.mine(0, ore_lookup(), &cfg);
        assert!(r.pathing.is_active());
    }

    #[test]
    fn stop_ends_a_mine_at_once_and_neutral() {
        let mut w = open_field();
        w.set(30, 1, 3, ORE);
        let mut r = rig(w, (2, 1, 3));
        start_mine(&mut r, 0);
        for _ in 0..40 {
            r.tick();
        }
        assert!(r.pathing.is_active() && r.pathing.is_pathing());
        let neutral = r.pathing.stop();
        assert_eq!(neutral, Intent::neutral());
        assert!(!r.pathing.is_active() && !r.pathing.mine_process().is_active());
        for _ in 0..5 {
            r.tick();
            assert!(!r.pathing.is_active());
            assert_eq!(r.intent, Intent::neutral());
        }
    }

    #[test]
    fn goto_after_mine_takes_control_and_the_mine_is_gone() {
        let mut w = open_field();
        w.set(34, 1, 3, ORE);
        let mut r = rig(w, (2, 1, 3));
        start_mine(&mut r, 0);
        for _ in 0..25 {
            r.tick();
        }
        assert!(r.pathing.mine_process().is_active());
        r.pathing.goto(Goal::Block { x: 6, y: 1, z: 9 });
        r.tick();
        assert!(!r.pathing.mine_process().is_active(), "mine kept control");
        assert!(r.pathing.process().is_active());
        r.run_until_idle(600);
        let f = r.p.feet();
        assert_eq!((f.x, f.z), (6, 9));
        assert!(!r.pathing.is_active());
        assert_eq!(ores_left(&r), 1, "the ore was not touched");
    }

    #[test]
    fn mine_after_goto_takes_control_from_the_goal_process() {
        let mut w = open_field();
        w.set(20, 1, 3, ORE);
        let mut r = rig(w, (2, 1, 3));
        r.pathing.goto(Goal::Block { x: 36, y: 1, z: 10 });
        for _ in 0..25 {
            r.tick();
        }
        assert!(r.pathing.process().is_active());
        start_mine(&mut r, 1);
        r.tick();
        assert!(!r.pathing.process().is_active(), "the goal process kept control");
        r.run_until_idle(1500);
        assert_eq!(ores_left(&r), 0);
        assert!(r.p.feet().x < 30, "it did not go on to the old goal: {:?}", r.p.feet());
    }

    #[test]
    fn at_most_one_process_is_in_control_after_each_start() {
        let mut w = open_field();
        w.set(30, 1, 5, ORE);
        w.set(34, 1, 8, ORE);
        let mut r = rig(w, (2, 1, 3));
        r.pathing.goto(Goal::Block { x: 38, y: 1, z: 3 });
        for t in 0..400 {
            match t {
                10 => start_mine(&mut r, 0),
                60 => {
                    let feet = r.p.feet();
                    r.pathing.get_to_block(ore_block(), feet)
                }
                120 => r.pathing.goto(Goal::Block { x: 4, y: 1, z: 4 }),
                200 => {
                    r.pathing.stop();
                }
                _ => {}
            }
            r.tick();
        }
        // the tick after each start, the newest process is the only one active
        for t in [10, 60, 120] {
            let (_, mine, gtb, goal) = r.who[t];
            assert_eq!([mine, gtb, goal].iter().filter(|a| **a).count(), 1, "tick {t}: mine {mine} gtb {gtb} goal {goal}");
        }
        let (_, mine, gtb, goal) = r.who[210];
        assert!(!mine && !gtb && !goal, "after stop");
    }

    #[test]
    fn goto_a_block_walks_next_to_it_and_leaves_it_alone() {
        let mut w = open_field();
        w.set(30, 1, 6, ORE);
        let mut r = rig(w, (2, 1, 3));
        let feet = r.p.feet();
        r.pathing.get_to_block(ore_block(), feet);
        let used = r.run_until_idle(1500);
        assert!(used < 1500, "{:?}", r.messages);
        let f = r.p.feet();
        assert_eq!((f.x - 30).abs() + (f.y - 1).abs() + (f.z - 6).abs(), 1, "next to the ore: {f:?}");
        assert_eq!(ores_left(&r), 1);
        assert!(r.broken.is_empty(), "nothing was broken on the way: {:?}", r.broken);
    }

    #[test]
    fn goto_a_block_that_is_not_there_explores_or_gives_up_when_told_not_to() {
        let mut r = rig(open_field(), (5, 1, 5));
        let feet = r.p.feet();
        r.pathing.get_to_block(ore_block(), feet);
        for _ in 0..30 {
            r.tick();
        }
        assert!(r.pathing.get_to_block_process().is_active());
        assert!(matches!(r.pathing.goal(), Some(Goal::NeverInGoal(_))), "{:?}", r.pathing.goal());
        r.pathing.stop();
        let mut r = rig(open_field(), (5, 1, 5));
        r.exec.mine.explore_for_blocks = false;
        let feet = r.p.feet();
        r.pathing.get_to_block(ore_block(), feet);
        r.tick();
        r.tick();
        assert!(!r.pathing.is_active());
        assert!(r.messages.iter().any(|m| m.starts_with("No known locations of BlockOptionalMeta{") && m.ends_with(", canceling GetToBlock")), "{:?}", r.messages);
    }

    #[test]
    fn a_pause_request_stands_the_path_still_for_one_tick_when_it_is_safe() {
        let mut r = rig(open_field(), (2, 1, 3));
        r.pathing.goto(Goal::Block { x: 36, y: 1, z: 3 });
        for _ in 0..30 {
            r.tick();
        }
        assert!(r.pathing.is_pathing());
        let before = r.pathing.current().unwrap().position();
        r.pathing.request_pause();
        let out = r.tick();
        // `safeToCancel` is what the path said last tick; walking straight is safe
        assert!(!r.pathing.is_pathing(), "paused this tick");
        assert!(r.pathing.has_path(), "the path is still there");
        assert_eq!(r.pathing.current().unwrap().position(), before, "the executor did not tick");
        assert!(!out.intent.forward && !out.intent.active, "{:?}", out.intent);
        r.tick();
        assert!(r.pathing.is_pathing(), "it goes on");
        r.run_until_idle(600);
        assert!(r.p.x > 35.0);
    }

    #[test]
    fn an_ore_in_the_players_column_is_broken_where_he_stands_with_the_path_paused() {
        let mut w = open_field();
        w.set(5, 3, 3, ORE);
        let mut r = rig(w, (5, 1, 3));
        // a walk in progress is not safe to cancel at its start: the pause waits for a safe moment, so the player first has a path
        start_mine(&mut r, 1);
        let mut paused = 0;
        for _ in 0..200 {
            r.tick();
            if r.pathing.mine_process().is_active() && !r.pathing.is_pathing() && r.intent.attack {
                paused += 1;
            }
            if !r.pathing.is_active() {
                break;
            }
        }
        assert!(r.broken.contains(&Pos::new(5, 3, 3)), "{:?} {:?}", r.broken, r.messages);
        assert!(paused > 0, "the left button was held with no path followed");
        assert!((r.p.x - 5.5).abs() < 0.6 && (r.p.z - 3.5).abs() < 0.6, "it did not walk: {} {}", r.p.x, r.p.z);
        assert!(r.messages.iter().any(|m| m == "Have 1 valid items"), "{:?}", r.messages);
        assert!(r.intents.iter().any(|i| i.select_slot == Some(3)), "the pickaxe was asked for");
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn random_fields_mine_exactly_what_they_were_asked_to() {
        for seed in 1..=12u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut w = TestWorld::flat(36, 14);
            let mut ores = vec![];
            let n = 3 + rng.below(6) as usize;
            while ores.len() < n {
                let p = (6 + rng.below(28) as i32, 1 + rng.below(2) as i32, 1 + rng.below(12) as i32);
                if !ores.contains(&p) {
                    ores.push(p);
                    w.set(p.0, p.1, p.2, ORE);
                }
            }
            let want = 1 + rng.below(n as u64) as i32;
            let mut r = rig(w, (2, 1, 7));
            start_mine(&mut r, want);
            let used = r.run_until_idle(4000);
            assert!(used < 4000, "seed {seed}: never ended: {:?} {:?}", r.messages, r.p.feet());
            assert!(r.messages.iter().any(|m| m == &format!("Have {want} valid items")), "seed {seed}: {:?}", r.messages);
            assert_eq!(ores_left(&r), n - want as usize, "seed {seed}: it mined exactly {want}");
            assert!(!r.pathing.is_active(), "seed {seed}");
        }
    }
}
