// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/process/GetToBlockProcess.java. LGPL-3.0-or-later, see lib.rs.
//
// `goto <block>`: finds the nearest places of a kind of block in the loaded world and walks to a block next to the nearest (into a nether portal; onto
// the block above a chest that has a block on top), then opens a container if it is one. The scanning is the mine process's, in new structure: see mine.rs.
use super::custom_goal::{PathingCommand, PathingCommandType};
use super::mine::{dist_sq, is_normal_cube_at, ScanJob, ScanKind, ScanResult};
use super::ProcessInputs;
use crate::blockmeta::{BlockOptionalMeta, BlockOptionalMetaLookup};
use crate::exec::helper::reachable;
use crate::exec::movements::Env;
use crate::exec::state::Controls;
use crate::goals::{goal_run_away, Goal};
use crate::pos::Pos;
use crate::world::WorldView;

/// `GetToBlockProcess`.
#[derive(Clone, Debug, Default)]
pub struct GetToBlockProcess {
    getting_to: Option<BlockOptionalMeta>,
    known_locations: Option<Vec<Pos>>,
    /// Locations we failed to calc to.
    blacklist: Vec<Pos>,
    start: Option<Pos>,
    tick_count: i32,
    arrival_tick_count: i32,
    generation: u64,
    initial_scan_pending: bool,
    /// The first scan is out (or about to be): the process asks to pause until it answers (docs/deviations.md D39).
    awaiting_first_scan: bool,
    scan_in_flight: bool,
    jobs: Vec<ScanJob>,
    messages: Vec<String>,
}

/// `GetToBlockProcess.rightClickOnArrival`'s blocks.
fn right_click_block(block: &str) -> bool {
    matches!(block, "crafting_table" | "furnace" | "blast_furnace" | "ender_chest" | "chest" | "trapped_chest")
}

impl GetToBlockProcess {
    /// `GetToBlockProcess.<init>`.
    pub fn new() -> GetToBlockProcess {
        GetToBlockProcess::default()
    }

    /// `GetToBlockProcess.getToBlock`: starts going to the nearest place of this block. `feet` is where the player is now; a path that finds
    /// nothing runs away from it. The first scan is queued on the next tick.
    pub fn get_to_block(&mut self, block: BlockOptionalMeta, feet: Pos) {
        let mut sink = Controls::default(); // the caller clears the keys that matter
        self.on_lost_control(&mut sink);
        self.getting_to = Some(block);
        self.start = Some(feet);
        self.blacklist = vec![];
        self.arrival_tick_count = 0;
        self.generation = self.generation.wrapping_add(1);
        self.initial_scan_pending = true;
        self.awaiting_first_scan = true;
        self.known_locations = None;
    }

    /// `GetToBlockProcess.isActive`: a block is the target.
    pub fn is_active(&self) -> bool {
        self.getting_to.is_some()
    }

    /// `GetToBlockProcess.displayName0`: the name for a status line.
    pub fn display_name(&self) -> String {
        let Some(g) = &self.getting_to else {
            return "INACTIVE".to_string();
        };
        match &self.known_locations {
            Some(k) if !k.is_empty() => format!("Get To {g}, {} known locations", k.len()),
            _ => format!("Exploring randomly to find {g}, no known locations"),
        }
    }

    /// `GetToBlockProcess.onLostControl`: `controls` are the keys it clears.
    pub fn on_lost_control(&mut self, controls: &mut Controls) {
        self.getting_to = None;
        self.known_locations = None;
        self.start = None;
        self.blacklist = vec![];
        self.generation = self.generation.wrapping_add(1);
        self.initial_scan_pending = false;
        self.awaiting_first_scan = false;
        self.scan_in_flight = false;
        self.jobs.clear();
        controls.clear_all_keys();
    }

    /// The things the process said, as chat lines (`logDirect`). Takes them.
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    /// The places it knows now, for the HUD and the tests.
    pub fn known_locations(&self) -> &[Pos] {
        self.known_locations.as_deref().unwrap_or(&[])
    }

    /// The scans that wait for a thread. Takes them.
    pub fn take_jobs(&mut self) -> Vec<ScanJob> {
        std::mem::take(&mut self.jobs)
    }

    /// The first scan waits: `Pathing::tick` takes the job and runs it before the process is asked for a command.
    pub(super) fn take_initial_scan<W: WorldView>(&mut self, env: &Env<'_, W>) -> Option<ScanJob> {
        if !self.initial_scan_pending {
            return None;
        }
        self.initial_scan_pending = false;
        let job = self.make_job(env, vec![]);
        self.awaiting_first_scan = job.is_some();
        job
    }

    /// Queues the first scan as a job (see [`GetToBlockProcess::take_jobs`]) if it is due.
    pub(super) fn queue_initial_scan<W: WorldView>(&mut self, env: &Env<'_, W>) {
        if let Some(job) = self.take_initial_scan(env) {
            self.jobs.push(job);
        }
    }

    /// The end of `GetToBlockProcess.rescan`: the answer becomes the known places, less the blacklisted ones. An answer for something else is dropped.
    pub fn deliver(&mut self, result: ScanResult) {
        if result.kind != ScanKind::GetToBlock || result.generation != self.generation || self.getting_to.is_none() {
            return; // we're getting to something else now (or nothing), this answer is for the old one
        }
        self.scan_in_flight = false;
        self.awaiting_first_scan = false;
        let mut positions = result.locations;
        positions.retain(|p| !self.blacklist.contains(p));
        self.known_locations = Some(positions);
    }

    /// `GetToBlockProcess.rescan`: the start of it, the job a thread runs.
    fn make_job<W: WorldView>(&mut self, env: &Env<'_, W>, known: Vec<Pos>) -> Option<ScanJob> {
        let target = self.getting_to.clone()?; // cancelled before the executor got to us
        self.scan_in_flight = true;
        Some(ScanJob::for_get_to_block(self.generation, BlockOptionalMetaLookup::new(vec![target]), known, self.blacklist.clone(), env.set.mine.clone(), env.p.feet(), env.p.block_position()))
    }

    /// `GetToBlockProcess.onTick`: Never null: when it gives up it says so with `CancelAndSetGoal` and no goal.
    pub fn on_tick<W: WorldView>(&mut self, calc_failed: bool, safe_to_cancel: bool, env: &Env<'_, W>, inputs: &ProcessInputs, controls: &mut Controls) -> PathingCommand {
        let ms = &env.set.mine;
        let Some(getting_to) = self.getting_to.clone() else {
            // not active: nothing to do. Upstream would throw; the manager never asks an inactive process.
            return PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal };
        };
        if self.awaiting_first_scan {
            // upstream scans inside `getToBlock`, on the game thread; here the scan runs on a thread and the process holds still until it answers
            return PathingCommand { goal: None, kind: PathingCommandType::RequestPause };
        }
        let feet = env.p.feet();
        if self.known_locations.is_none() {
            // upstream scans here, on this thread; the first scan is queued before (`Pathing::tick`)
            self.known_locations = Some(vec![]);
        }
        let known_empty = self.known_locations.as_ref().is_none_or(|k| k.is_empty());
        if known_empty {
            if ms.explore_for_blocks && !calc_failed {
                let start = self.start.unwrap_or(feet);
                return PathingCommand { goal: Some(Goal::NeverInGoal(Box::new(goal_run_away(1.0, None, vec![(start.x, start.y, start.z)])))), kind: PathingCommandType::ForceRevalidateGoalAndPath };
            }
            self.messages.push(format!("No known locations of {getting_to}, canceling GetToBlock"));
            if safe_to_cancel {
                self.on_lost_control(controls);
            }
            return PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal };
        }
        let goal = Goal::Composite(self.known_locations().iter().map(|p| self.create_goal(*p, &getting_to, env)).collect());
        if calc_failed {
            if ms.blacklist_closest_on_failure {
                self.messages.push(format!("Unable to find any path to {getting_to}, blacklisting presumably unreachable closest instances..."));
                self.blacklist_closest(feet);
                return self.on_tick(false, safe_to_cancel, env, inputs, controls);
            // gamer moment
            } else {
                self.messages.push(format!("Unable to find any path to {getting_to}, canceling GetToBlock"));
                if safe_to_cancel {
                    self.on_lost_control(controls);
                }
                return PathingCommand { goal: Some(goal), kind: PathingCommandType::CancelAndSetGoal };
            }
        }
        let interval = ms.mine_goal_update_interval;
        if interval != 0
            && {
                let t = self.tick_count;
                self.tick_count = self.tick_count.wrapping_add(1);
                t.wrapping_rem(interval) == 0
            }
            && !self.scan_in_flight
        {
            let current = self.known_locations().to_vec();
            if let Some(job) = self.make_job(env, current) {
                self.jobs.push(job);
            }
        }
        let gs = env.ctx.cfg.goal_settings();
        if goal.is_in_goal(feet.x, feet.y, feet.z, &gs) && goal.is_in_goal(env.path_start.x, env.path_start.y, env.path_start.z, &gs) && safe_to_cancel {
            // we're there
            if self.right_click_on_arrival(getting_to.block(), env) {
                if self.right_click(env, inputs, controls) {
                    self.on_lost_control(controls);
                    return PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal };
                }
            } else {
                self.on_lost_control(controls);
                return PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal };
            }
        }
        PathingCommand { goal: Some(goal), kind: PathingCommandType::RevalidateGoalAndPath }
    }

    /// `GetToBlockProcess.blacklistClosest`: the nearest place and the places that touch it, in a chain, are not tried again.
    pub fn blacklist_closest(&mut self, feet: Pos) -> bool {
        let Some(known) = self.known_locations.as_mut() else {
            return false;
        };
        let mut new_blacklist: Vec<Pos> = vec![];
        // `Stream.min` keeps the first of equals
        let closest = known.iter().copied().fold(None, |best: Option<Pos>, p| match best {
            Some(b) if dist_sq(b, feet) <= dist_sq(p, feet) => Some(b),
            _ => Some(p),
        });
        new_blacklist.extend(closest);
        // the first place that touches one on the list goes on it and off the known places, until none does
        while let Some(i) = known.iter().position(|k| new_blacklist.iter().any(|b| are_adjacent(*k, *b))) {
            let k = known.remove(i);
            new_blacklist.push(k);
        }
        let any = !new_blacklist.is_empty();
        self.blacklist.extend(new_blacklist);
        any
    }

    /// `GetToBlockProcess.createGoal`: the goal for one place.
    fn create_goal<W: WorldView>(&self, pos: Pos, getting_to: &BlockOptionalMeta, env: &Env<'_, W>) -> Goal {
        let block = getting_to.block();
        if self.walk_into_instead_of_adjacent(block, env) {
            return Goal::TwoBlocks { x: pos.x, y: pos.y, z: pos.z };
        }
        let above = pos.above(1);
        if self.block_on_top_must_be_removed(block, env) && is_normal_cube_at(env.ctx, above) {
            // TODO this should be the check for chest openability
            return Goal::Block { x: above.x, y: above.y, z: above.z };
        }
        Goal::GetToBlock { x: pos.x, y: pos.y, z: pos.z }
    }

    /// `GetToBlockProcess.rightClick`: aims at the nearest known place that can be reached and presses the right button; true when it is done or gave up.
    fn right_click<W: WorldView>(&mut self, env: &Env<'_, W>, inputs: &ProcessInputs, controls: &mut Controls) -> bool {
        let known = self.known_locations().to_vec();
        for pos in &known {
            let reach = reachable(env, *pos, env.game.reach, false);
            if let Some(rot) = reach {
                controls.look = Some(rot);
                controls.look_force = true;
                if env.p.looking_at.is_some_and(|h| known.contains(&h.pos)) {
                    controls.keys.click_right = true; // TODO find some way to right click even if we're in an ESC menu
                    if inputs.container_open {
                        return true;
                    }
                }
                let n = self.arrival_tick_count;
                self.arrival_tick_count = n.wrapping_add(1);
                if n > 20 {
                    self.messages.push("Right click timed out".to_string());
                    return true;
                }
                return false; // trying to right click, will do it next tick or so
            }
        }
        self.messages.push("Arrived but failed to right click open".to_string());
        true
    }

    /// `GetToBlockProcess.walkIntoInsteadOfAdjacent`: a nether portal is walked into.
    fn walk_into_instead_of_adjacent<W: WorldView>(&self, block: &str, env: &Env<'_, W>) -> bool {
        if !env.set.mine.enter_portal {
            return false;
        }
        block == "nether_portal"
    }

    /// `GetToBlockProcess.rightClickOnArrival`: the setting is on and the block is one that opens.
    fn right_click_on_arrival<W: WorldView>(&self, block: &str, env: &Env<'_, W>) -> bool {
        if !env.set.mine.right_click_container_on_arrival {
            return false;
        }
        right_click_block(block)
    }

    /// `GetToBlockProcess.blockOnTopMustBeRemoved`: only the chests; you can open a crafting table or furnace even with a block on top.
    fn block_on_top_must_be_removed<W: WorldView>(&self, block: &str, env: &Env<'_, W>) -> bool {
        if !self.right_click_on_arrival(block, env) {
            // only if we plan to actually open it on arrival
            return false;
        }
        matches!(block, "ender_chest" | "chest" | "trapped_chest")
    }
}

/// `GetToBlockProcess.areAdjacent`: the Manhattan distance is 1 (safer than a direct comparison of squared distances).
fn are_adjacent(a: Pos, b: Pos) -> bool {
    let diff_x = (i64::from(a.x) - i64::from(b.x)).abs();
    let diff_y = (i64::from(a.y) - i64::from(b.y)).abs();
    let diff_z = (i64::from(a.z) - i64::from(b.z)).abs();
    diff_x + diff_y + diff_z == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockmeta::test_index;
    use crate::blocks::Direction;
    use crate::config::Config;
    use crate::exec::state::PlayerView;
    use crate::look::Vec3;
    use crate::raytrace::BlockHit;
    use crate::settings::ExecSettings;
    use crate::testing::{test_config, Fixture, TestWorld, ORE};

    fn bom(name: &str) -> BlockOptionalMeta {
        BlockOptionalMeta::parse(name, &test_index()).unwrap()
    }

    fn fixture() -> Fixture {
        Fixture::with_ore(Config { allow_break: true, ..test_config() })
    }

    /// A block table in which state 2 (the fixture's ore) is a chest.
    fn chest_bom() -> BlockOptionalMeta {
        use crate::blockmeta::{BlockIndex, StateEntry};
        let e = |b: &str| StateEntry { block: b.to_string(), props: vec![] };
        BlockOptionalMeta::parse("chest", &BlockIndex::new(vec![e("air"), e("stone"), e("chest")], Default::default())).unwrap()
    }

    /// A process that is going to `name`, with its first scan done.
    fn going(fx: &Fixture, w: &TestWorld, p: &PlayerView, set: &ExecSettings, name: &str) -> GetToBlockProcess {
        going_to(fx, w, p, set, if name == "chest" { chest_bom() } else { bom(name) })
    }

    fn going_to(fx: &Fixture, w: &TestWorld, p: &PlayerView, set: &ExecSettings, target: BlockOptionalMeta) -> GetToBlockProcess {
        let ctx = fx.ctx(w);
        let env = Env::new(&ctx, p, set, p.feet());
        let mut g = GetToBlockProcess::new();
        g.get_to_block(target, p.feet());
        if let Some(job) = g.take_initial_scan(&env) {
            g.deliver(job.run(&ctx));
        }
        g
    }

    #[test]
    fn the_goal_for_a_place_depends_on_the_block() {
        let fx = fixture();
        let set = ExecSettings::default();
        let p = PlayerView::standing(2.5, 1.0, 2.5);
        let mut w = TestWorld::flat(16, 16);
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        let g = GetToBlockProcess::new();
        let at = Pos::new(8, 1, 8);
        assert_eq!(g.create_goal(at, &bom("diamond_ore"), &env), Goal::GetToBlock { x: 8, y: 1, z: 8 });
        // a nether portal is walked into
        assert_eq!(g.create_goal(at, &bom("nether_portal"), &env), Goal::TwoBlocks { x: 8, y: 1, z: 8 });
        let mut off = set.clone();
        off.mine.enter_portal = false;
        let env_off = Env::new(&ctx, &p, &off, p.feet());
        assert_eq!(g.create_goal(at, &bom("nether_portal"), &env_off), Goal::GetToBlock { x: 8, y: 1, z: 8 });
        // a chest is stood next to, unless there is a block on top: then on top of that
        assert_eq!(g.create_goal(at, &bom("chest"), &env), Goal::GetToBlock { x: 8, y: 1, z: 8 });
        w.set(8, 2, 8, ORE);
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        assert_eq!(g.create_goal(at, &bom("chest"), &env), Goal::Block { x: 8, y: 2, z: 8 });
        assert_eq!(g.create_goal(at, &bom("furnace"), &env), Goal::GetToBlock { x: 8, y: 1, z: 8 }, "a furnace opens with a block on top");
        let env_off = Env::new(&ctx, &p, &off, p.feet());
        let mut no_click = off.clone();
        no_click.mine.right_click_container_on_arrival = false;
        let env_nc = Env::new(&ctx, &p, &no_click, p.feet());
        assert_eq!(g.create_goal(at, &bom("chest"), &env_nc), Goal::GetToBlock { x: 8, y: 1, z: 8 });
        let _ = env_off;
    }

    #[test]
    fn the_closest_place_and_the_ones_touching_it_are_blacklisted_in_a_chain() {
        let fx = fixture();
        let set = ExecSettings::default();
        let p = PlayerView::standing(2.5, 1.0, 2.5);
        let mut w = TestWorld::flat(32, 16);
        // a vein of three, 20 away from a lone block that is farther than the vein's near end
        for pos in [(10, 1, 3), (11, 1, 3), (12, 1, 3), (11, 2, 3)] {
            w.set(pos.0, pos.1, pos.2, ORE);
        }
        w.set(25, 1, 12, ORE);
        let mut g = going(&fx, &w, &p, &set, "diamond_ore");
        assert_eq!(g.known_locations().len(), 5);
        assert!(g.blacklist_closest(p.feet()));
        // the whole vein is off the known places: the closest itself goes once a neighbour is on the list (and is on the list twice, as upstream's loop makes it)
        assert_eq!(g.known_locations(), &[Pos::new(25, 1, 12)]);
        assert_eq!(g.blacklist.len(), 5);
        assert_eq!(g.blacklist.iter().filter(|p| **p == Pos::new(10, 1, 3)).count(), 2);
        // a scan after that is told to leave them out
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        let job = g.make_job(&env, g.known_locations().to_vec()).unwrap();
        let r = job.run(&ctx);
        assert_eq!(r.locations, vec![Pos::new(25, 1, 12)]);
        g.deliver(r);
        assert_eq!(g.known_locations(), &[Pos::new(25, 1, 12)]);
    }

    #[test]
    fn a_failed_path_blacklists_and_goes_on_or_gives_up() {
        let fx = fixture();
        let set = ExecSettings::default();
        let p = PlayerView::standing(2.5, 1.0, 3.5);
        let mut w = TestWorld::flat(32, 16);
        w.set(8, 1, 3, ORE);
        w.set(25, 1, 12, ORE);
        let mut g = going(&fx, &w, &p, &set, "diamond_ore");
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        let mut c = Controls::default();
        let cmd = g.on_tick(true, true, &env, &ProcessInputs::default(), &mut c);
        assert_eq!(cmd.kind, PathingCommandType::RevalidateGoalAndPath);
        assert!(g.take_messages().iter().any(|l| l.contains("blacklisting presumably unreachable closest instances")));
        // the closest is still in the list, but the goal after the retry is for the places that are left, and the closest is blacklisted for the next scan
        assert_eq!(g.blacklist, vec![Pos::new(8, 1, 3)]);
        let mut off = set.clone();
        off.mine.blacklist_closest_on_failure = false;
        let mut g = going(&fx, &w, &p, &off, "diamond_ore");
        let env = Env::new(&ctx, &p, &off, p.feet());
        let cmd = g.on_tick(true, true, &env, &ProcessInputs::default(), &mut c);
        assert_eq!(cmd.kind, PathingCommandType::CancelAndSetGoal);
        assert!(cmd.goal.is_some(), "the goal is still handed on");
        assert!(!g.is_active());
        assert!(g.take_messages().last().unwrap().ends_with(", canceling GetToBlock"));
        // not safe to cancel: it stays
        let mut g = going(&fx, &w, &p, &off, "diamond_ore");
        g.on_tick(true, false, &env, &ProcessInputs::default(), &mut c);
        assert!(g.is_active());
    }

    #[test]
    fn nothing_known_explores_from_the_start_unless_the_last_path_failed() {
        let fx = fixture();
        let set = ExecSettings::default();
        let p = PlayerView::standing(2.5, 1.0, 3.5);
        let w = TestWorld::flat(32, 16);
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        let mut g = going(&fx, &w, &p, &set, "diamond_ore");
        let mut c = Controls::default();
        let cmd = g.on_tick(false, true, &env, &ProcessInputs::default(), &mut c);
        assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
        assert_eq!(cmd.goal, Some(Goal::NeverInGoal(Box::new(crate::goals::goal_run_away(1.0, None, vec![(2, 1, 3)])))));
        assert!(g.display_name().starts_with("Exploring randomly to find BlockOptionalMeta{"));
        // the path of the exploring failed: no more of it
        let cmd = g.on_tick(true, true, &env, &ProcessInputs::default(), &mut c);
        assert_eq!((cmd.kind, cmd.goal), (PathingCommandType::CancelAndSetGoal, None));
        assert!(!g.is_active());
    }

    #[test]
    fn arriving_next_to_a_block_ends_it_and_next_to_a_chest_opens_it() {
        let fx = fixture();
        let set = ExecSettings::default();
        let mut w = TestWorld::flat(16, 16);
        w.set(8, 1, 5, ORE); // stands in for the chest: only its position matters
        let mut p = PlayerView::standing(7.5, 1.0, 5.5);
        let ctx = fx.ctx(&w);
        let none = ProcessInputs::default();
        // a plain block: there, so done
        let mut g = going(&fx, &w, &p, &set, "diamond_ore");
        let env = Env::new(&ctx, &p, &set, p.feet());
        let mut c = Controls::default();
        let cmd = g.on_tick(false, true, &env, &none, &mut c);
        assert_eq!((cmd.kind, cmd.goal), (PathingCommandType::CancelAndSetGoal, None));
        assert!(!g.is_active());
        // not safe to cancel: it waits
        let mut g = going(&fx, &w, &p, &set, "diamond_ore");
        let cmd = g.on_tick(false, false, &env, &none, &mut c);
        assert_eq!(cmd.kind, PathingCommandType::RevalidateGoalAndPath);
        // a chest: aims, and presses the right button once the crosshair is on it; done when a container is open
        let mut g = going(&fx, &w, &p, &set, "chest");
        let mut c = Controls::default();
        let cmd = g.on_tick(false, true, &env, &none, &mut c);
        assert_eq!(cmd.kind, PathingCommandType::RevalidateGoalAndPath, "it waits for the click to land");
        assert!(c.look.is_some() && c.look_force);
        assert!(!c.keys.click_right, "the crosshair is not on it yet");
        p.looking_at = Some(BlockHit { pos: Pos::new(8, 1, 5), face: Direction::West, point: Vec3::new(8.0, 1.5, 5.5) });
        let env = Env::new(&ctx, &p, &set, p.feet());
        let mut c = Controls::default();
        let cmd = g.on_tick(false, true, &env, &none, &mut c);
        assert!(c.keys.click_right);
        assert_eq!(cmd.kind, PathingCommandType::RevalidateGoalAndPath, "no container yet");
        let open = ProcessInputs { container_open: true, ..Default::default() };
        let cmd = g.on_tick(false, true, &env, &open, &mut Controls::default());
        assert_eq!((cmd.kind, cmd.goal), (PathingCommandType::CancelAndSetGoal, None));
        assert!(!g.is_active());
        // it never opens: gives up after 21 ticks
        let mut g = going(&fx, &w, &p, &set, "chest");
        let mut said = false;
        for _ in 0..30 {
            let cmd = g.on_tick(false, true, &env, &none, &mut Controls::default());
            if cmd.goal.is_none() {
                said = true;
                break;
            }
        }
        assert!(said);
        assert_eq!(g.take_messages().last().unwrap(), "Right click timed out");
        // setting off: a chest is just walked to
        let mut off = set.clone();
        off.mine.right_click_container_on_arrival = false;
        let env = Env::new(&ctx, &p, &off, p.feet());
        let mut g = going(&fx, &w, &p, &off, "chest");
        let cmd = g.on_tick(false, true, &env, &none, &mut Controls::default());
        assert_eq!((cmd.kind, cmd.goal), (PathingCommandType::CancelAndSetGoal, None));
    }

    #[test]
    fn an_answer_for_an_earlier_target_is_dropped() {
        let fx = fixture();
        let set = ExecSettings::default();
        let p = PlayerView::standing(2.5, 1.0, 3.5);
        let mut w = TestWorld::flat(32, 16);
        w.set(8, 1, 3, ORE);
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        let mut g = GetToBlockProcess::new();
        g.get_to_block(bom("diamond_ore"), p.feet());
        let job = g.take_initial_scan(&env).unwrap();
        let old = job.run(&ctx);
        // another goto before the answer comes
        g.get_to_block(bom("stone"), p.feet());
        g.deliver(old);
        assert!(g.known_locations().is_empty());
        // and after a stop
        let job = g.take_initial_scan(&env).unwrap();
        let answer = job.run(&ctx);
        g.on_lost_control(&mut Controls::default());
        g.deliver(answer);
        assert!(!g.is_active() && g.known_locations().is_empty());
    }
}
