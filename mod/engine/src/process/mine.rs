// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/process/MineProcess.java. LGPL-3.0-or-later, see lib.rs.
//
// `mine <blocks>`: finds the blocks in the loaded world, goes to the nearest and breaks them (the path executor breaks them on the way), until
// told to stop or until the inventory holds enough of what they drop. The decisions and their order are upstream's.
//
// What is new structure: upstream runs `rescan` on its executor thread and writes the answer into a field when it is done. Here a scan is a
// [`ScanJob`] that owns what it reads; the glue takes it with `Pathing::take_scan_jobs`, runs it on any thread over a snapshot with
// [`ScanJob::run`], and gives the [`ScanResult`] back with `Pathing::deliver_scan`. The first scan of `mine` (upstream: inside `mine`) is
// run by `Pathing::tick` on the game thread before the process is asked, so that the process never sees "nothing known yet".
use super::custom_goal::{PathingCommand, PathingCommandType};
use super::ProcessInputs;
use crate::blockmeta::BlockOptionalMetaLookup;
use crate::config::Config;
use crate::costs::COST_INF;
use crate::exec::helper::{is_looking_at, reachable};
use crate::exec::movements::Env;
use crate::exec::state::Controls;
use crate::goals::{goal_run_away, Goal};
use crate::movement::helper::{is_block_normal_cube, is_transparent, Ctx};
use crate::pos::Pos;
use crate::scan::scan_chunk_radius;
use crate::settings::MineSettings;
use crate::world::WorldView;
use std::collections::HashSet;

/// Which process asked for a scan: the answer goes back to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanKind {
    Mine,
    GetToBlock,
}

/// A scan for the blocks of a filter, with everything `MineProcess.rescan` reads of the process and the player when it starts. Run it over a world
/// with [`ScanJob::run`]: that is `MineProcess.searchWorld` and the part of `rescan` after it.
#[derive(Clone, Debug)]
pub struct ScanJob {
    pub kind: ScanKind,
    /// Of the session of the process that asked: an answer of another generation is dropped.
    pub generation: u64,
    filter: BlockOptionalMetaLookup,
    max: i32,
    already: Vec<Pos>,
    blacklist: Vec<Pos>,
    dropped: Vec<Pos>,
    /// `playerFeet`: where the chunk scan starts.
    feet: Pos,
    /// `player().blockPosition()`: what the answer is sorted by.
    block_pos: Pos,
    settings: MineSettings,
    /// The context of upstream's `GetToBlockProcess.GetToBlockCalculationContext`, whose `breakCostMultiplierAt` is 1: `allowBreak` does not count.
    ignore_allow_break: bool,
}

/// The answer to a [`ScanJob`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanResult {
    pub kind: ScanKind,
    pub generation: u64,
    /// The blocks found, nearest first, and for the mine process the drops that are not explained by a block in the list.
    pub locations: Vec<Pos>,
}

impl ScanJob {
    /// Runs the scan over the world of `ctx`. `MineProcess.rescan` (the part that does not touch the process) and `searchWorld`.
    pub fn run<W: WorldView>(&self, ctx: &Ctx<'_, W>) -> ScanResult {
        let mut dropped = self.dropped.clone();
        let patched;
        let ctx = if self.ignore_allow_break && !ctx.cfg.allow_break {
            let mut cfg: Config = ctx.cfg.clone();
            cfg.allow_break = true;
            patched = cfg;
            Ctx::new(ctx.world, ctx.states, ctx.pre, &patched, ctx.border, ctx.tool_strength)
        } else {
            Ctx::new(ctx.world, ctx.states, ctx.pre, ctx.cfg, ctx.border, ctx.tool_strength)
        };
        let mut locations = search_world(&ctx, &self.settings, &self.filter, self.max, &self.already, &self.blacklist, &mut dropped, self.feet, self.block_pos);
        if self.kind == ScanKind::Mine {
            locations.extend(dropped);
        }
        ScanResult { kind: self.kind, generation: self.generation, locations }
    }

    /// A scan for `GetToBlockProcess.rescan`: at most 64 places, the current ones kept.
    pub(super) fn for_get_to_block(generation: u64, filter: BlockOptionalMetaLookup, known: Vec<Pos>, blacklist: Vec<Pos>, settings: MineSettings, feet: Pos, block_pos: Pos) -> ScanJob {
        ScanJob { kind: ScanKind::GetToBlock, generation, filter, max: 64, already: known, blacklist, dropped: vec![], feet, block_pos, settings, ignore_allow_break: true }
    }
}

/// Java's `BlockPos.distSqr(Vec3i)`, the squared distance in doubles.
pub(super) fn dist_sq(a: Pos, b: Pos) -> f64 {
    let dx = f64::from(a.x) - f64::from(b.x);
    let dy = f64::from(a.y) - f64::from(b.y);
    let dz = f64::from(a.z) - f64::from(b.z);
    dx * dx + dy * dy + dz * dz
}

/// `MineProcess.searchWorld`: There is no cached world, so every block is "untracked" and read from the loaded chunks (docs/deviations.md D37).
#[allow(clippy::too_many_arguments)]
fn search_world<W: WorldView>(ctx: &Ctx<'_, W>, ms: &MineSettings, filter: &BlockOptionalMetaLookup, max: i32, already_known: &[Pos], blacklist: &[Pos], dropped: &mut Vec<Pos>, feet: Pos, block_pos: Pos) -> Vec<Pos> {
    let mut locs = prune(ctx, ms, vec![], filter, max, blacklist, dropped, block_pos);
    if !filter.blocks().is_empty() {
        locs.extend(scan_chunk_radius(ctx.world, feet, filter, max, 32)); // maxSearchRadius is NOT sq
    }
    locs.extend(already_known.iter().copied());
    prune(ctx, ms, locs, filter, max, blacklist, dropped, block_pos)
}

/// `MineProcess.prune`: drops the places that are no longer what we want or are not worth going to, sorts the rest by distance from the player and keeps `max`.
#[allow(clippy::too_many_arguments)]
fn prune<W: WorldView>(ctx: &Ctx<'_, W>, ms: &MineSettings, locs2: Vec<Pos>, filter: &BlockOptionalMetaLookup, max: i32, blacklist: &[Pos], dropped: &mut Vec<Pos>, block_pos: Pos) -> Vec<Pos> {
    dropped.retain(|drop| {
        for pos in &locs2 {
            if dist_sq(*pos, *drop) <= 9.0 && filter.has(ctx.get(pos.x, pos.y, pos.z).id) && plausible_to_break(ctx, *pos) {
                return false;
            }
        }
        true
    });
    let mut seen: HashSet<Pos> = HashSet::new();
    let mut locs: Vec<Pos> = locs2
        .into_iter()
        .filter(|p| seen.insert(*p)) // distinct, the first of each
        // remove any that are within loaded chunks that aren't actually what we want
        .filter(|pos| !ctx.world.contains_loaded_chunk(pos.x, pos.z) || filter.has(ctx.get(pos.x, pos.y, pos.z).id) || dropped.contains(pos))
        // remove any that are implausible to mine (encased in bedrock, or touching lava)
        .filter(|pos| plausible_to_break(ctx, *pos))
        .filter(|pos| !ms.allow_only_exposed_ores || is_next_to_air(ctx, ms, *pos))
        .filter(|pos| i64::from(pos.y) >= i64::from(ms.min_y_level_while_mining) + i64::from(ctx.min_y))
        .filter(|pos| pos.y <= ms.max_y_level_while_mining)
        .filter(|pos| !blacklist.contains(pos))
        .collect();
    locs.sort_by(|a, b| dist_sq(*a, block_pos).total_cmp(&dist_sq(*b, block_pos)));
    let keep = usize::try_from(max).unwrap_or(0);
    locs.truncate(keep);
    locs
}

/// `MineProcess.isNextToAir`: a transparent block (air, water, lava) within the Manhattan distance `allowOnlyExposedOresDistance`.
pub fn is_next_to_air<W: WorldView>(ctx: &Ctx<'_, W>, ms: &MineSettings, pos: Pos) -> bool {
    let radius = ms.allow_only_exposed_ores_distance;
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                if dx.abs() + dy.abs() + dz.abs() <= radius && is_transparent(&ctx.get(pos.x.wrapping_add(dx), pos.y.wrapping_add(dy), pos.z.wrapping_add(dz))) {
                    return true;
                }
            }
        }
    }
    false
}

/// `MineProcess.plausibleToBreak`: the block can be mined at all, is not one to avoid breaking, and is not between two bedrock blocks.
pub fn plausible_to_break<W: WorldView>(ctx: &Ctx<'_, W>, pos: Pos) -> bool {
    let state = ctx.get(pos.x, pos.y, pos.z);
    if ctx.mining_duration(pos.x, pos.y, pos.z, state, true) >= COST_INF {
        return false;
    }
    if ctx.avoid_breaking(pos.x, pos.y, pos.z, state) {
        return false;
    }
    // bedrock above and below makes it implausible, otherwise we're good
    !(ctx.get(pos.x, pos.y.wrapping_add(1), pos.z).s.name == "bedrock" && ctx.get(pos.x, pos.y.wrapping_sub(1), pos.z).s.name == "bedrock")
}

/// `MineProcess`.
#[derive(Clone, Debug, Default)]
pub struct MineProcess {
    filter: Option<BlockOptionalMetaLookup>,
    known_ore_locations: Vec<Pos>,
    blacklist: Vec<Pos>,
    /// Block and the time (ms) until which it counts as a drop to wait for. Upstream's map, in the order the entries were made.
    anticipated_drops: Vec<(Pos, i64)>,
    branch_point: Option<Pos>,
    branch_point_runaway: Option<Goal>,
    desired_quantity: i32,
    tick_count: i32,
    /// The session: counts `mine` calls and stops, so an answer to an older scan is dropped.
    generation: u64,
    /// `mine` ran, and its first scan has not: `Pathing::tick` runs it.
    initial_scan_pending: bool,
    /// The first scan of the session is out (or about to be) and has not come back: the process asks to pause until it does (docs/deviations.md D39).
    awaiting_first_scan: bool,
    /// A scan is out and has not come back; no other starts (docs/deviations.md D37).
    scan_in_flight: bool,
    jobs: Vec<ScanJob>,
    messages: Vec<String>,
}

impl MineProcess {
    /// `MineProcess.<init>`.
    pub fn new() -> MineProcess {
        MineProcess::default()
    }

    /// `MineProcess.isActive`: a filter is set.
    pub fn is_active(&self) -> bool {
        self.filter.is_some()
    }

    /// `MineProcess.displayName0`: the name for a status line.
    pub fn display_name(&self) -> String {
        match &self.filter {
            Some(f) => format!("Mine {f}"),
            None => "INACTIVE".to_string(),
        }
    }

    /// The things the process said, as chat lines (`logDirect`). Takes them.
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    /// The places it knows now, nearest first, for the HUD and the tests.
    pub fn known_ore_locations(&self) -> &[Pos] {
        &self.known_ore_locations
    }

    /// `MineProcess.mine`: starts mining the blocks of `filter` (`None`: stops). `quantity` is how many of their drops to
    /// have in the inventory before stopping, or 0 for no end. The first scan runs on the next tick.
    pub fn mine(&mut self, quantity: i32, filter: Option<BlockOptionalMetaLookup>, cfg: &Config) {
        self.filter = filter;
        if self.filter_filter(cfg).is_none() {
            self.filter = None;
        }
        self.desired_quantity = quantity;
        self.known_ore_locations = vec![];
        self.blacklist = vec![];
        self.branch_point = None;
        self.branch_point_runaway = None;
        self.anticipated_drops = vec![];
        self.generation = self.generation.wrapping_add(1);
        self.scan_in_flight = false;
        self.jobs.clear();
        self.initial_scan_pending = self.filter.is_some();
        self.awaiting_first_scan = self.filter.is_some();
    }

    /// `MineProcess.onLostControl`: forgets the filter, the places and the blacklist.
    pub fn on_lost_control(&mut self) {
        self.mine(0, None, &Config::upstream_defaults());
    }

    /// `IMineProcess.cancel`.
    fn cancel(&mut self) {
        self.on_lost_control();
    }

    /// `MineProcess.filterFilter`: the filter, without the blocks that may not be broken (`allowBreak` off and not in `allowBreakAnyway`). `None`
    /// when nothing is left, or there is no filter.
    pub fn filter_filter(&mut self, cfg: &Config) -> Option<BlockOptionalMetaLookup> {
        let f = self.filter.as_ref()?;
        if !cfg.allow_break {
            let boms: Vec<_> = f.blocks().iter().filter(|e| cfg.allow_break_anyway.iter().any(|b| b == e.block())).cloned().collect();
            if boms.is_empty() {
                self.messages.push("Unable to mine when allowBreak is false and target block is not in allowBreakAnyway!".to_string());
                return None;
            }
            return Some(BlockOptionalMetaLookup::new(boms));
        }
        Some(f.clone())
    }

    /// The first scan: `Pathing::tick` queues it as a job for a thread before the process is asked for a command, and the process waits for the answer.
    pub(super) fn take_initial_scan<W: WorldView>(&mut self, env: &Env<'_, W>, inputs: &ProcessInputs) -> Option<ScanJob> {
        if !self.initial_scan_pending {
            return None;
        }
        self.initial_scan_pending = false;
        let job = self.make_job(env, inputs, vec![]);
        self.awaiting_first_scan = job.is_some();
        job
    }

    /// Queues the first scan as a job (see [`MineProcess::take_jobs`]) if it is due.
    pub(super) fn queue_initial_scan<W: WorldView>(&mut self, env: &Env<'_, W>, inputs: &ProcessInputs) {
        if let Some(job) = self.take_initial_scan(env, inputs) {
            self.jobs.push(job);
        }
    }

    /// The scans that wait for a thread. Takes them.
    pub fn take_jobs(&mut self) -> Vec<ScanJob> {
        std::mem::take(&mut self.jobs)
    }

    /// The end of `MineProcess.rescan`: the answer of a scan becomes the known places. An answer of another session is dropped.
    pub fn deliver(&mut self, result: ScanResult, cfg: &Config, explore_for_blocks: bool) {
        if result.kind != ScanKind::Mine || result.generation != self.generation {
            return;
        }
        self.scan_in_flight = false;
        self.awaiting_first_scan = false;
        if self.filter_filter(cfg).is_none() {
            return;
        }
        if result.locations.is_empty() && !explore_for_blocks {
            let f = self.filter.clone().map(|f| f.to_string()).unwrap_or_default();
            self.messages.push(format!("No locations for {f} known, cancelling"));
            self.cancel();
            return;
        }
        self.known_ore_locations = result.locations;
    }

    /// `MineProcess.rescan`: the start of it, the job a thread runs. `None` in legit mine, which does not scan, and without a filter.
    fn make_job<W: WorldView>(&mut self, env: &Env<'_, W>, inputs: &ProcessInputs, already: Vec<Pos>) -> Option<ScanJob> {
        let filter = self.filter_filter(env.ctx.cfg)?;
        if env.set.mine.legit_mine {
            return None;
        }
        let dropped = self.dropped_items_scan(&env.set.mine, inputs);
        self.scan_in_flight = true;
        Some(ScanJob { kind: ScanKind::Mine, generation: self.generation, filter, max: env.set.mine.mine_max_ore_locations_count, already, blacklist: self.blacklist.clone(), dropped, feet: env.p.feet(), block_pos: env.p.block_position(), settings: env.set.mine.clone(), ignore_allow_break: false })
    }

    /// The count of the drops of the filter in the inventory (`player().getInventory().items`).
    fn drops_in_inventory<W: WorldView>(&self, env: &Env<'_, W>, filter: &BlockOptionalMetaLookup) -> i64 {
        env.inv.main_slots().iter().filter(|s| !s.is_empty() && filter.has_item(&s.item)).map(|s| i64::from(s.count)).sum()
    }

    /// `MineProcess.onTick`: `calc_failed` is that the last path of this process failed to be found; `safe_to_cancel` is `PathingBehavior.isSafeToCancel`.
    /// `None` is upstream's `null`: the process gave up (or is done) and is not active any more.
    pub fn on_tick<W: WorldView>(&mut self, calc_failed: bool, safe_to_cancel: bool, env: &Env<'_, W>, inputs: &ProcessInputs, controls: &mut Controls) -> Option<PathingCommand> {
        let ms = &env.set.mine;
        let cfg = env.ctx.cfg;
        if self.awaiting_first_scan {
            // upstream scans inside `mine`, on the game thread; here the scan runs on a thread and the process holds still until it answers
            return Some(PathingCommand { goal: None, kind: PathingCommandType::RequestPause });
        }
        if self.desired_quantity > 0 {
            let filter = self.filter.clone()?;
            let curr = self.drops_in_inventory(env, &filter);
            if curr >= i64::from(self.desired_quantity) {
                self.messages.push(format!("Have {curr} valid items"));
                self.cancel();
                return None;
            }
        }
        let feet = env.p.feet();
        if calc_failed {
            let f = self.filter.clone().map(|f| f.to_string()).unwrap_or_default();
            if !self.known_ore_locations.is_empty() && ms.blacklist_closest_on_failure {
                self.messages.push(format!("Unable to find any path to {f}, blacklisting presumably unreachable closest instance..."));
                // `Stream.min` keeps the first of equals
                let closest = self.known_ore_locations.iter().copied().fold(None, |best: Option<Pos>, p| match best {
                    Some(b) if dist_sq(b, feet) <= dist_sq(p, feet) => Some(b),
                    _ => Some(p),
                });
                if let Some(c) = closest {
                    self.blacklist.push(c);
                }
                let bl = self.blacklist.clone();
                self.known_ore_locations.retain(|p| !bl.contains(p));
            } else {
                self.messages.push(format!("Unable to find any path to {f}, canceling mine"));
                self.cancel();
                return None;
            }
        }

        self.update_louca_system(ms, inputs, env);
        let mine_goal_update_interval = ms.mine_goal_update_interval;
        let curr: Vec<Pos> = self.known_ore_locations.clone();
        // rescan does nothing in legit mode, so no job is made for it to do nothing
        if mine_goal_update_interval != 0
            && {
                let t = self.tick_count;
                self.tick_count = self.tick_count.wrapping_add(1);
                t.wrapping_rem(mine_goal_update_interval) == 0
            }
            && !ms.legit_mine
            && !self.scan_in_flight
        {
            if let Some(job) = self.make_job(env, inputs, curr.clone()) {
                self.jobs.push(job);
            }
        }
        if ms.legit_mine && !self.add_nearby(env, inputs) {
            self.cancel();
            return None;
        }
        let above_feet = feet.above(1);
        let shaft = curr
            .iter()
            .copied()
            .filter(|pos| pos.x == feet.x && pos.z == feet.z)
            .filter(|pos| pos.y >= feet.y)
            .filter(|pos| !env.ctx.get(pos.x, pos.y, pos.z).air) // after breaking a block, it takes mineGoalUpdateInterval ticks for it to actually update this list =(
            .fold(None, |best: Option<Pos>, p| match best {
                Some(b) if dist_sq(b, above_feet) <= dist_sq(p, above_feet) => Some(b),
                _ => Some(p),
            });
        controls.clear_all_keys();
        if let (Some(pos), true) = (shaft, env.p.on_ground) {
            let state = env.ctx.get(pos.x, pos.y, pos.z);
            if !env.ctx.avoid_breaking(pos.x, pos.y, pos.z, state) {
                let rot = reachable(env, pos, env.game.reach, false);
                if let (Some(rot), true) = (rot, safe_to_cancel) {
                    controls.look = Some(rot);
                    controls.look_force = true;
                    if let Some(slot) = crate::inventory::switch_to_best_tool_for(env.inv, state.id, &env.set.tool) {
                        controls.select = Some(slot);
                    }
                    if is_looking_at(env.p, pos) || env.p.rotations().is_really_close_to(rot) {
                        controls.keys.click_left = true;
                    }
                    return Some(PathingCommand { goal: None, kind: PathingCommandType::RequestPause });
                }
            }
        }
        let command = self.update_goal(env, inputs, cfg);
        if command.is_none() {
            // none in range
            // maybe say something in chat? (ahem impact)
            self.cancel();
            return None;
        }
        command
    }

    /// `MineProcess.updateLoucaSystem`: a block the crosshair is on that is one of the known places stays a drop to wait for for `mineDropLoiterDurationMSThanksLouca`
    /// after it was last on it; older ones are forgotten.
    fn update_louca_system<W: WorldView>(&mut self, ms: &MineSettings, inputs: &ProcessInputs, env: &Env<'_, W>) {
        let mut copy = self.anticipated_drops.clone();
        if let Some(h) = env.p.looking_at {
            if self.known_ore_locations.contains(&h.pos) {
                let until = inputs.now_ms.wrapping_add(ms.mine_drop_loiter_duration_ms);
                match copy.iter_mut().find(|(p, _)| *p == h.pos) {
                    Some(e) => e.1 = until,
                    None => copy.push((h.pos, until)),
                }
            }
        }
        // the keys of the old map only, as upstream's loop: a place put in just now is not looked at
        for (pos, _) in &self.anticipated_drops {
            if let Some(i) = copy.iter().position(|(p, _)| p == pos) {
                if copy[i].1 < inputs.now_ms {
                    copy.remove(i);
                }
            }
        }
        self.anticipated_drops = copy;
    }

    /// `MineProcess.droppedItemsScan`: where the dropped items the filter wants are, and the places held as drops.
    pub fn dropped_items_scan(&self, ms: &MineSettings, inputs: &ProcessInputs) -> Vec<Pos> {
        if !ms.mine_scan_dropped_items {
            return vec![];
        }
        let Some(filter) = &self.filter else {
            return vec![];
        };
        let mut ret: Vec<Pos> = inputs.item_entities.iter().filter(|e| filter.has_item(&e.item)).map(|e| e.pos).collect();
        ret.extend(self.anticipated_drops.iter().map(|(p, _)| *p));
        ret
    }

    /// `MineProcess.updateGoal`: the command for this tick.
    fn update_goal<W: WorldView>(&mut self, env: &Env<'_, W>, inputs: &ProcessInputs, cfg: &Config) -> Option<PathingCommand> {
        let ms = &env.set.mine;
        let filter = self.filter_filter(cfg)?;
        let legit = ms.legit_mine;
        if !self.known_ore_locations.is_empty() {
            let mut dropped = self.dropped_items_scan(ms, inputs);
            let locs2 = prune(env.ctx, ms, self.known_ore_locations.clone(), &filter, ms.mine_max_ore_locations_count, &self.blacklist, &mut dropped, env.p.block_position());
            // can't reassign locs, gotta make a new var locs2, because we use it in a lambda right here, and variables you use in a lambda must be effectively final
            let goals: Vec<Goal> = locs2.iter().map(|loc| self.coalesce(*loc, &locs2, env)).collect();
            self.known_ore_locations = locs2;
            return Some(PathingCommand { goal: Some(Goal::Composite(goals)), kind: if legit { PathingCommandType::ForceRevalidateGoalAndPath } else { PathingCommandType::RevalidateGoalAndPath } });
        }
        // we don't know any ore locations at the moment
        if !legit && !ms.explore_for_blocks {
            return None;
        }
        // only when we should explore for blocks or are in legit mode we do this
        let y = ms.legit_mine_y_level;
        if self.branch_point.is_none() {
            self.branch_point = Some(env.p.feet());
        }
        // TODO shaft mode, mine 1x1 shafts to either side
        // TODO also, see if the GoalRunAway with maintain Y at 11 works even from the surface
        if self.branch_point_runaway.is_none() {
            let b = self.branch_point?;
            self.branch_point_runaway = Some(Goal::NeverInGoal(Box::new(goal_run_away(1.0, Some(y), vec![(b.x, b.y, b.z)]))));
        }
        Some(PathingCommand { goal: self.branch_point_runaway.clone(), kind: PathingCommandType::RevalidateGoalAndPath })
    }

    /// `MineProcess.internalMiningGoal`: whether a place next to the one we aim for is another one to mine, so a single path can serve both.
    fn internal_mining_goal<W: WorldView>(&self, pos: Pos, env: &Env<'_, W>, locs: &[Pos]) -> bool {
        // Here, BlockStateInterface is used because the position may be in a cached chunk (the targeted block is one that is kept track of)
        if locs.contains(&pos) {
            return true;
        }
        let state = env.ctx.get(pos.x, pos.y, pos.z);
        if env.set.mine.internal_mining_air_exception && state.air {
            return true;
        }
        self.filter.as_ref().is_some_and(|f| f.has(state.id)) && plausible_to_break(env.ctx, pos)
    }

    /// `MineProcess.coalesce`: the goal for one place to mine.
    fn coalesce<W: WorldView>(&self, loc: Pos, locs: &[Pos], env: &Env<'_, W>) -> Goal {
        let assume_vertical_shaft_mine = !env.ctx.get(loc.x, loc.y.wrapping_add(1), loc.z).falling;
        if !env.set.mine.force_internal_mining {
            if assume_vertical_shaft_mine {
                // we can get directly below the block
                return Goal::ThreeBlocks { x: loc.x, y: loc.y, z: loc.z };
            } else {
                // we need to get feet or head into the block
                return Goal::TwoBlocks { x: loc.x, y: loc.y, z: loc.z };
            }
        }
        let upward_goal = self.internal_mining_goal(loc.above(1), env, locs);
        let downward_goal = self.internal_mining_goal(loc.below(1), env, locs);
        let double_downward_goal = self.internal_mining_goal(loc.below(2), env, locs);
        if upward_goal == downward_goal {
            // symmetric
            if double_downward_goal && assume_vertical_shaft_mine {
                // we have a checkerboard like pattern
                // this one, and the one two below it
                // therefore it's fine to path to immediately below this one, since your feet will be in the doubleDownwardGoal
                // but only if assumeVerticalShaftMine
                return Goal::ThreeBlocks { x: loc.x, y: loc.y, z: loc.z };
            } else {
                // this block has nothing interesting two below, but is symmetric vertically so we can get either feet or head into it
                return Goal::TwoBlocks { x: loc.x, y: loc.y, z: loc.z };
            }
        }
        if upward_goal {
            // downwardGoal known to be false
            // ignore the gap then potential doubleDownward, because we want to path feet into this one and head into upwardGoal
            return Goal::Block { x: loc.x, y: loc.y, z: loc.z };
        }
        // upwardGoal known to be false, downwardGoal known to be true
        let below = loc.below(1);
        if double_downward_goal && assume_vertical_shaft_mine {
            // this block and two below it are goals
            // path into the center of the one below, because that includes directly below this one
            return Goal::TwoBlocks { x: below.x, y: below.y, z: below.z };
        }
        // upwardGoal false, downwardGoal true, doubleDownwardGoal false
        // just this block and the one immediately below, no others
        Goal::Block { x: below.x, y: below.y, z: below.z }
    }

    /// `MineProcess.addNearby`: legit mine learns of the blocks within ten of the player that it could see (a ray of up to 20 reaches them), without scanning the world.
    fn add_nearby<W: WorldView>(&mut self, env: &Env<'_, W>, inputs: &ProcessInputs) -> bool {
        let ms = &env.set.mine;
        let dropped = self.dropped_items_scan(ms, inputs);
        self.known_ore_locations.extend(dropped.iter().copied());
        let player_feet = env.p.feet();
        let Some(filter) = self.filter_filter(env.ctx.cfg) else {
            return false;
        };
        let search_dist = 10;
        let faked_block_reach_distance = 20.0; // at least 10 * sqrt(3) with some extra space to account for positioning within the block
                                               // ores we already know about were getting raytraced again every tick (up to 7 rays each) just to be added a
                                               // second time and deduplicated by prune
        let mut already_known: HashSet<Pos> = self.known_ore_locations.iter().copied().collect();
        for x in player_feet.x - search_dist..=player_feet.x + search_dist {
            for y in player_feet.y - search_dist..=player_feet.y + search_dist {
                for z in player_feet.z - search_dist..=player_feet.z + search_dist {
                    // crucial to only add blocks we can see because otherwise this
                    // is an x-ray and it'll get caught
                    if filter.has(env.ctx.get(x, y, z).id) && !already_known.contains(&Pos::new(x, y, z)) {
                        let pos = Pos::new(x, y, z);
                        if (ms.legit_mine_include_diagonals && self.known_ore_locations.iter().any(|ore| dist_sq(*ore, pos) <= 2.0 /* sq means this is pytha dist <= sqrt(2) */)) || reachable(env, pos, faked_block_reach_distance, false).is_some() {
                            self.known_ore_locations.push(pos);
                            already_known.insert(pos);
                        }
                    }
                }
            }
        }
        let mut dropped = dropped;
        self.known_ore_locations = prune(env.ctx, ms, std::mem::take(&mut self.known_ore_locations), &filter, ms.mine_max_ore_locations_count, &self.blacklist, &mut dropped, env.p.block_position());
        true
    }
}

/// `GetToBlockProcess` needs the same test for a block that cannot be walked into.
pub(super) fn is_normal_cube_at<W: WorldView>(ctx: &Ctx<'_, W>, pos: Pos) -> bool {
    is_block_normal_cube(&ctx.get(pos.x, pos.y, pos.z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockmeta::test_index;
    use crate::blocks::{Direction, State};
    use crate::exec::state::PlayerView;
    use crate::inventory::testing::TestInventory;
    use crate::inventory::SlotInfo;
    use crate::look::Rotation;
    use crate::look::Vec3;
    use crate::movement::helper::Precomputed;
    use crate::raytrace::BlockHit;
    use crate::settings::ExecSettings;
    use crate::testing::{test_config, Fixture, TestWorld, AIR, ORE, STONE};

    const SAND: u16 = 3;
    const BEDROCK: u16 = 4;

    /// Air, stone, ore, sand (falling) and bedrock, with breaking on.
    fn fixture() -> Fixture {
        let cfg = Config { allow_break: true, ..test_config() };
        let mut fx = Fixture::with_ore(cfg);
        fx.states.push(State { name: "sand".into(), shape_full: true, top_sturdy: true, fluid_empty: true, falling: true, ..State::default() });
        fx.states.push(State { name: "bedrock".into(), shape_full: true, top_sturdy: true, fluid_empty: true, ..State::default() });
        fx.pre = Precomputed::new(&fx.states, &fx.cfg);
        fx.tool = vec![1.0, 1.0, 1.0, 1.0, 0.0];
        fx
    }

    fn lookup(names: &[&str]) -> BlockOptionalMetaLookup {
        BlockOptionalMetaLookup::from_names(names, &test_index()).unwrap()
    }

    fn standing(x: f64, y: f64, z: f64) -> PlayerView {
        PlayerView::standing(x, y, z)
    }

    fn hit(x: i32, y: i32, z: i32) -> Option<BlockHit> {
        Some(BlockHit { pos: Pos::new(x, y, z), face: Direction::Down, point: Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5) })
    }

    /// A process that has mined the filter and run its first scan.
    fn started(fx: &Fixture, world: &TestWorld, p: &PlayerView, set: &ExecSettings, quantity: i32, filter: BlockOptionalMetaLookup, inputs: &ProcessInputs) -> MineProcess {
        let ctx = fx.ctx(world);
        let env = Env::new(&ctx, p, set, p.feet());
        let mut m = MineProcess::new();
        m.mine(quantity, Some(filter), &fx.cfg);
        if let Some(job) = m.take_initial_scan(&env, inputs) {
            let r = job.run(&ctx);
            m.deliver(r, &fx.cfg, set.mine.explore_for_blocks);
        }
        m
    }

    fn goals_of(c: &PathingCommand) -> Vec<Goal> {
        match &c.goal {
            Some(Goal::Composite(g)) => g.clone(),
            other => panic!("not a composite goal: {other:?}"),
        }
    }

    #[test]
    fn the_scan_prunes_sorts_and_limits_what_it_found() {
        let fx = fixture();
        let mut w = TestWorld::flat(32, 32);
        // ores at distances 3, 5, 9, 12 from the player at (4, 1, 4) (squared: 9, 25, 81, 144 by x alone)
        for x in [7, 9, 13, 16] {
            w.set(x, 1, 4, ORE);
        }
        let p = standing(4.5, 1.0, 4.5);
        let mut set = ExecSettings::default();
        let inputs = ProcessInputs::default();
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        let mut m = MineProcess::new();
        m.mine(0, Some(lookup(&["diamond_ore"])), &fx.cfg);
        let job = m.take_initial_scan(&env, &inputs).unwrap();
        let got = job.run(&ctx).locations;
        // nearest first, whatever the order they were found in
        assert_eq!(got, vec![Pos::new(7, 1, 4), Pos::new(9, 1, 4), Pos::new(13, 1, 4), Pos::new(16, 1, 4)]);
        // the limit
        set.mine.mine_max_ore_locations_count = 2;
        let env = Env::new(&ctx, &p, &set, p.feet());
        let mut m = MineProcess::new();
        m.mine(0, Some(lookup(&["diamond_ore"])), &fx.cfg);
        let got = m.take_initial_scan(&env, &inputs).unwrap().run(&ctx).locations;
        assert_eq!(got, vec![Pos::new(7, 1, 4), Pos::new(9, 1, 4)]);
    }

    #[test]
    fn the_y_bounds_the_exposure_and_the_blacklist_remove_places() {
        let fx = fixture();
        let mut w = TestWorld::flat(32, 32);
        w.set(8, 1, 8, ORE);
        w.set(12, 5, 8, ORE);
        // an ore inside a block of stone: not next to air
        for x in 18..=20 {
            for y in 1..=3 {
                for z in 7..=9 {
                    w.set(x, y, z, STONE);
                }
            }
        }
        w.set(19, 2, 8, ORE);
        let p = standing(4.5, 1.0, 8.5);
        let ctx = fx.ctx(&w);
        let scan = |set: &ExecSettings, black: &[Pos]| {
            let env = Env::new(&ctx, &p, set, p.feet());
            let mut m = MineProcess::new();
            m.mine(0, Some(lookup(&["diamond_ore"])), &fx.cfg);
            m.blacklist = black.to_vec();
            m.take_initial_scan(&env, &ProcessInputs::default()).unwrap().run(&ctx).locations
        };
        let base = ExecSettings::default();
        assert_eq!(scan(&base, &[]), vec![Pos::new(8, 1, 8), Pos::new(12, 5, 8), Pos::new(19, 2, 8)]);
        let mut s = base.clone();
        s.mine.min_y_level_while_mining = 3; // relative to the world's lowest y, which is 0
        assert_eq!(scan(&s, &[]), vec![Pos::new(12, 5, 8)]);
        let mut s = base.clone();
        s.mine.max_y_level_while_mining = 4;
        assert_eq!(scan(&s, &[]), vec![Pos::new(8, 1, 8), Pos::new(19, 2, 8)]);
        let mut s = base.clone();
        s.mine.allow_only_exposed_ores = true;
        assert_eq!(scan(&s, &[]), vec![Pos::new(8, 1, 8), Pos::new(12, 5, 8)]);
        s.mine.allow_only_exposed_ores_distance = 2; // the one in the stone has air two away (the shell is 1 thick)
        assert!(scan(&s, &[]).contains(&Pos::new(19, 2, 8)));
        assert_eq!(scan(&base, &[Pos::new(12, 5, 8)]), vec![Pos::new(8, 1, 8), Pos::new(19, 2, 8)]);
        // in a loaded chunk, a place that is not the block any more is dropped; in an unloaded chunk it is kept (the world cannot say)
        let mut w2 = TestWorld::flat(64, 32);
        w2.set(40, 1, 8, ORE);
        w2.loaded_below_x = 32;
        let ctx2 = fx.ctx(&w2);
        let env = Env::new(&ctx2, &p, &base, p.feet());
        let m = MineProcess::new();
        let mut dropped = vec![];
        let kept = prune(env.ctx, &base.mine, vec![Pos::new(40, 1, 8), Pos::new(10, 1, 8), Pos::new(40, 1, 8)], &lookup(&["diamond_ore"]), 64, &[], &mut dropped, p.block_position());
        assert_eq!(kept, vec![Pos::new(40, 1, 8)], "the unloaded place is kept once, the empty loaded one is dropped");
        let _ = m;
    }

    #[test]
    fn a_block_between_two_bedrock_blocks_is_not_worth_it_and_an_unbreakable_one_is_not_either() {
        let fx = fixture();
        let mut w = TestWorld::flat(16, 16);
        w.set(5, 5, 5, ORE);
        w.set(5, 6, 5, BEDROCK);
        w.set(5, 4, 5, BEDROCK);
        w.set(8, 5, 5, ORE);
        w.set(8, 6, 5, BEDROCK);
        let ctx = fx.ctx(&w);
        assert!(!plausible_to_break(&ctx, Pos::new(5, 5, 5)));
        assert!(plausible_to_break(&ctx, Pos::new(8, 5, 5)), "bedrock on one side only");
        assert!(!plausible_to_break(&ctx, Pos::new(5, 6, 5)), "bedrock itself cannot be mined");
    }

    #[test]
    fn what_the_goal_is_for_each_place_follows_coalesce() {
        let fx = fixture();
        let inputs = ProcessInputs::default();
        let p = standing(2.5, 1.0, 2.5);
        let set = ExecSettings::default();
        let goal_of = |w: &TestWorld, set: &ExecSettings, at: Pos| {
            let mut m = started(&fx, w, &p, set, 0, lookup(&["diamond_ore"]), &inputs);
            let ctx = fx.ctx(w);
            let env = Env::new(&ctx, &p, set, p.feet());
            let c = m.on_tick(false, true, &env, &inputs, &mut Controls::default()).unwrap();
            let all = goals_of(&c);
            let locs = m.known_ore_locations().to_vec();
            let i = locs.iter().position(|l| *l == at).unwrap();
            (all[i].clone(), c.kind)
        };
        // an ore on the ground with air over it: feet into it (the block above is air, so the head is free)
        let mut w = TestWorld::flat(16, 16);
        w.set(8, 1, 8, ORE);
        assert_eq!(goal_of(&w, &set, Pos::new(8, 1, 8)), (Goal::Block { x: 8, y: 1, z: 8 }, PathingCommandType::RevalidateGoalAndPath));
        // with the air exception off, neither side is a goal and the cell above is nothing: the two cells
        let mut s = set.clone();
        s.mine.internal_mining_air_exception = false;
        assert_eq!(goal_of(&w, &s, Pos::new(8, 1, 8)).0, Goal::TwoBlocks { x: 8, y: 1, z: 8 });
        // two ores one over the other: the lower one is entered from the side, the upper one is symmetric
        let mut w = TestWorld::flat(16, 16);
        w.set(8, 1, 8, ORE);
        w.set(8, 2, 8, ORE);
        assert_eq!(goal_of(&w, &set, Pos::new(8, 1, 8)).0, Goal::Block { x: 8, y: 1, z: 8 });
        assert_eq!(goal_of(&w, &set, Pos::new(8, 2, 8)).0, Goal::TwoBlocks { x: 8, y: 2, z: 8 });
        // a checkerboard: ores at y 3 and 1 with air between: standing under the upper one, feet in the lower one's cell, serves both
        let mut w = TestWorld::flat(16, 16);
        w.set(8, 1, 8, ORE);
        w.set(8, 3, 8, ORE);
        assert_eq!(goal_of(&w, &set, Pos::new(8, 3, 8)).0, Goal::ThreeBlocks { x: 8, y: 3, z: 8 });
        // the one above is on its own below: stone under it does not matter, the air above does
        // without the internal mining, a shaft under the place is assumed unless sand is over it
        let mut s = set.clone();
        s.mine.force_internal_mining = false;
        let mut w = TestWorld::flat(16, 16);
        w.set(8, 2, 8, ORE);
        assert_eq!(goal_of(&w, &s, Pos::new(8, 2, 8)).0, Goal::ThreeBlocks { x: 8, y: 2, z: 8 });
        w.set(8, 3, 8, SAND);
        assert_eq!(goal_of(&w, &s, Pos::new(8, 2, 8)).0, Goal::TwoBlocks { x: 8, y: 2, z: 8 });
        // both neighbours up and down are goals, and so is two below: the cell below serves them all
        let mut w = TestWorld::flat(16, 16);
        w.set(8, 1, 8, ORE);
        w.set(8, 2, 8, ORE);
        w.set(8, 3, 8, ORE);
        w.set(8, 4, 8, STONE);
        // the middle one: up is a place, down is a place, two below is the floor (stone, not wanted)
        assert_eq!(goal_of(&w, &set, Pos::new(8, 2, 8)).0, Goal::TwoBlocks { x: 8, y: 2, z: 8 });
        // the top one: nothing above that wants it (stone), down is a place, two below is a place: the two cells below the top
        assert_eq!(goal_of(&w, &set, Pos::new(8, 3, 8)).0, Goal::TwoBlocks { x: 8, y: 2, z: 8 });
        // the lowest: up is a place, down is the floor
        assert_eq!(goal_of(&w, &set, Pos::new(8, 1, 8)).0, Goal::Block { x: 8, y: 1, z: 8 });
        // legit mine forces the revalidation
        let mut s = set.clone();
        s.mine.legit_mine = true;
        let mut w = TestWorld::flat(16, 16);
        w.set(6, 1, 2, ORE);
        let mut m = MineProcess::new();
        m.mine(0, Some(lookup(&["diamond_ore"])), &fx.cfg);
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &s, p.feet());
        assert!(m.take_initial_scan(&env, &inputs).is_none());
        let c = m.on_tick(false, true, &env, &inputs, &mut Controls::default()).unwrap();
        assert_eq!(c.kind, PathingCommandType::ForceRevalidateGoalAndPath);
        assert!(m.take_jobs().is_empty(), "legit mine does not scan the world");
    }

    #[test]
    fn a_place_in_the_players_own_column_is_mined_from_where_he_stands() {
        let fx = fixture();
        let inputs = ProcessInputs::default();
        let mut w = TestWorld::flat(16, 16);
        w.set(5, 3, 5, ORE);
        let mut p = standing(5.5, 1.0, 5.5);
        let set = ExecSettings::default();
        let mut m = started(&fx, &w, &p, &set, 0, lookup(&["diamond_ore"]), &inputs);
        let ctx = fx.ctx(&w);
        // a pickaxe in slot 3
        let mut inv = TestInventory::empty();
        inv.put(3, SlotInfo { material_cost: 2, pickaxe: true, ..SlotInfo::of("iron_pickaxe", 1) }, &[0.0, 0.2, 0.2]);
        let mut env = Env::new(&ctx, &p, &set, p.feet());
        env.inv = &inv;
        let mut c = Controls::default();
        let cmd = m.on_tick(false, true, &env, &inputs, &mut c).unwrap();
        assert_eq!((cmd.kind, cmd.goal), (PathingCommandType::RequestPause, None));
        assert!(c.look.is_some() && c.look_force, "it aims");
        let aim = c.look.unwrap();
        assert_eq!(c.select, Some(3), "with the pickaxe");
        // the view is not on the block yet and is far from the aim: no click
        assert!(!c.keys.click_left, "{c:?}");
        // looking at the block: the left button
        p.looking_at = hit(5, 3, 5);
        let env2 = Env { p: &p, ..Env::new(&ctx, &p, &set, p.feet()) };
        let mut env2 = env2;
        env2.inv = &inv;
        let mut c = Controls::default();
        let cmd = m.on_tick(false, true, &env2, &inputs, &mut c).unwrap();
        assert_eq!(cmd.kind, PathingCommandType::RequestPause);
        assert!(c.keys.click_left);
        // the view already really close to the aim also clicks
        let mut p3 = standing(5.5, 1.0, 5.5);
        p3.yaw = aim.yaw;
        p3.pitch = aim.pitch;
        let mut env3 = Env::new(&ctx, &p3, &set, p3.feet());
        env3.inv = &inv;
        let mut c3 = Controls::default();
        m.on_tick(false, true, &env3, &inputs, &mut c3).unwrap();
        assert!(c3.keys.click_left);
        // not safe to cancel the path, or not on the ground: it walks instead
        let mut c4 = Controls::default();
        let cmd = m.on_tick(false, false, &env2, &inputs, &mut c4).unwrap();
        assert_eq!(cmd.kind, PathingCommandType::RevalidateGoalAndPath);
        assert!(!c4.keys.click_left);
        let mut air = p;
        air.on_ground = false;
        let mut env5 = Env::new(&ctx, &air, &set, air.feet());
        env5.inv = &inv;
        let cmd = m.on_tick(false, true, &env5, &inputs, &mut Controls::default()).unwrap();
        assert_eq!(cmd.kind, PathingCommandType::RevalidateGoalAndPath);
    }

    #[test]
    fn a_failed_path_blacklists_the_closest_place_or_gives_up() {
        let fx = fixture();
        let inputs = ProcessInputs::default();
        let mut w = TestWorld::flat(32, 16);
        w.set(8, 1, 3, ORE);
        w.set(20, 1, 3, ORE);
        let p = standing(2.5, 1.0, 3.5);
        let set = ExecSettings::default();
        let mut m = started(&fx, &w, &p, &set, 0, lookup(&["diamond_ore"]), &inputs);
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        let c = m.on_tick(true, true, &env, &inputs, &mut Controls::default()).unwrap();
        assert!(m.take_messages().iter().any(|l| l.contains("blacklisting presumably unreachable closest instance")));
        assert_eq!(m.known_ore_locations(), &[Pos::new(20, 1, 3)]);
        assert_eq!(goals_of(&c).len(), 1);
        // the setting off: it gives up, and is not active any more
        let mut s = set.clone();
        s.mine.blacklist_closest_on_failure = false;
        let mut m = started(&fx, &w, &p, &s, 0, lookup(&["diamond_ore"]), &inputs);
        let env = Env::new(&ctx, &p, &s, p.feet());
        assert!(m.on_tick(true, true, &env, &inputs, &mut Controls::default()).is_none());
        assert!(!m.is_active());
        assert!(m.take_messages().iter().any(|l| l.ends_with(", canceling mine")));
        // nothing known and a failure: also gives up
        let mut m = started(&fx, &TestWorld::flat(32, 16), &p, &set, 0, lookup(&["diamond_ore"]), &inputs);
        let ctx2 = fx.ctx(&w);
        let _ = ctx2;
        let w0 = TestWorld::flat(32, 16);
        let ctx0 = fx.ctx(&w0);
        let env = Env::new(&ctx0, &p, &set, p.feet());
        assert!(m.on_tick(true, true, &env, &inputs, &mut Controls::default()).is_none());
        assert!(!m.is_active());
    }

    #[test]
    fn the_quantity_counts_the_drops_in_the_whole_inventory() {
        let fx = fixture();
        let inputs = ProcessInputs::default();
        let mut w = TestWorld::flat(32, 16);
        w.set(20, 1, 3, ORE);
        let p = standing(2.5, 1.0, 3.5);
        let set = ExecSettings::default();
        let mut m = started(&fx, &w, &p, &set, 5, lookup(&["diamond_ore"]), &inputs);
        let ctx = fx.ctx(&w);
        let mut inv = TestInventory::empty();
        inv.put(0, SlotInfo::of("diamond", 2), &[]);
        inv.put(1, SlotInfo::of("stick", 9), &[]);
        inv.main.push(SlotInfo::of("diamond", 2));
        let mut env = Env::new(&ctx, &p, &set, p.feet());
        env.inv = &inv;
        assert!(m.on_tick(false, true, &env, &inputs, &mut Controls::default()).is_some(), "4 of 5");
        inv.main.push(SlotInfo::of("diamond", 1));
        let mut env = Env::new(&ctx, &p, &set, p.feet());
        env.inv = &inv;
        assert!(m.on_tick(false, true, &env, &inputs, &mut Controls::default()).is_none());
        assert!(!m.is_active());
        assert_eq!(m.take_messages().last().unwrap(), "Have 5 valid items");
    }

    #[test]
    fn dropped_items_are_places_to_go_and_a_mined_block_is_held_for_a_while() {
        let fx = fixture();
        let mut w = TestWorld::flat(32, 16);
        w.set(10, 1, 3, ORE);
        let p = standing(2.5, 1.0, 3.5);
        let mut set = ExecSettings::default();
        let mut inputs = ProcessInputs { now_ms: 1000, item_entities: vec![super::super::ItemEntity { item: "diamond".into(), pos: Pos::new(25, 1, 12) }, super::super::ItemEntity { item: "stick".into(), pos: Pos::new(6, 1, 6) }], container_open: false };
        let mut m = started(&fx, &w, &p, &set, 0, lookup(&["diamond_ore"]), &inputs);
        // the diamond lies where there is no ore: it is a place of its own; the stick is nothing
        assert_eq!(m.known_ore_locations(), &[Pos::new(10, 1, 3), Pos::new(25, 1, 12)]);
        // an item next to an ore that explains it is not a place of its own
        inputs.item_entities.push(super::super::ItemEntity { item: "diamond".into(), pos: Pos::new(11, 1, 4) });
        let m2 = started(&fx, &w, &p, &set, 0, lookup(&["diamond_ore"]), &inputs);
        assert_eq!(m2.known_ore_locations().len(), 2, "{:?}", m2.known_ore_locations());
        assert!(!m2.known_ore_locations().contains(&Pos::new(11, 1, 4)));
        // the setting off: no scan of items
        set.mine.mine_scan_dropped_items = false;
        assert!(m.dropped_items_scan(&set.mine, &inputs).is_empty());
        set.mine.mine_scan_dropped_items = true;
        // the crosshair on a known place holds it as a drop for 250 ms after the last time
        let ctx = fx.ctx(&w);
        let mut looking = p;
        looking.looking_at = hit(10, 1, 3);
        let env = Env::new(&ctx, &looking, &set, looking.feet());
        m.update_louca_system(&set.mine, &inputs, &env);
        assert_eq!(m.anticipated_drops, vec![(Pos::new(10, 1, 3), 1250)]);
        assert!(m.dropped_items_scan(&set.mine, &inputs).contains(&Pos::new(10, 1, 3)));
        // looking away: still held until its time is up, then forgotten
        let env = Env::new(&ctx, &p, &set, p.feet());
        inputs.now_ms = 1249;
        m.update_louca_system(&set.mine, &inputs, &env);
        assert_eq!(m.anticipated_drops.len(), 1);
        inputs.now_ms = 1251;
        m.update_louca_system(&set.mine, &inputs, &env);
        assert!(m.anticipated_drops.is_empty());
    }

    #[test]
    fn legit_mine_only_learns_of_what_it_could_see_and_otherwise_digs_down_to_its_level() {
        let fx = fixture();
        let inputs = ProcessInputs::default();
        let mut w = TestWorld::flat(32, 32);
        w.set(8, 1, 5, ORE); // in the open
                             // an ore inside a shell of stone: no ray reaches it
        for x in 14..=16 {
            for y in 1..=3 {
                for z in 4..=6 {
                    w.set(x, y, z, STONE);
                }
            }
        }
        w.set(15, 2, 5, ORE);
        let p = standing(5.5, 1.0, 5.5);
        let mut set = ExecSettings::default();
        set.mine.legit_mine = true;
        let mut m = MineProcess::new();
        m.mine(0, Some(lookup(&["diamond_ore"])), &fx.cfg);
        let ctx = fx.ctx(&w);
        let env = Env::new(&ctx, &p, &set, p.feet());
        assert!(m.take_initial_scan(&env, &inputs).is_none(), "legit mine never scans the world");
        let c = m.on_tick(false, true, &env, &inputs, &mut Controls::default()).unwrap();
        assert_eq!(m.known_ore_locations(), &[Pos::new(8, 1, 5)]);
        assert_eq!(c.kind, PathingCommandType::ForceRevalidateGoalAndPath);
        // with the diagonals setting, the one next to a known one is known too, seen or not
        let mut w2 = TestWorld::flat(32, 32);
        w2.set(8, 1, 5, ORE);
        w2.set(9, 2, 5, ORE);
        for (x, y, z) in [(9, 3, 5), (10, 2, 5), (9, 2, 4), (9, 2, 6), (9, 1, 5), (10, 1, 5), (9, 1, 4), (9, 1, 6)] {
            w2.set(x, y, z, STONE);
        }
        let ctx2 = fx.ctx(&w2);
        let mut s2 = set.clone();
        s2.mine.legit_mine_include_diagonals = true;
        let env2 = Env::new(&ctx2, &p, &s2, p.feet());
        let mut m2 = MineProcess::new();
        m2.mine(0, Some(lookup(&["diamond_ore"])), &fx.cfg);
        assert!(m2.take_initial_scan(&env2, &inputs).is_none());
        m2.on_tick(false, true, &env2, &inputs, &mut Controls::default()).unwrap();
        assert!(m2.known_ore_locations().contains(&Pos::new(9, 2, 5)), "{:?}", m2.known_ore_locations());
        // nothing in sight: it explores with a goal that is never reached, to the legit y level, from where it started; the same goal every tick
        let empty = TestWorld::flat(32, 32);
        let ctx3 = fx.ctx(&empty);
        let env3 = Env::new(&ctx3, &p, &set, p.feet());
        let mut m3 = MineProcess::new();
        m3.mine(0, Some(lookup(&["diamond_ore"])), &fx.cfg);
        assert!(m3.take_initial_scan(&env3, &inputs).is_none());
        let a = m3.on_tick(false, true, &env3, &inputs, &mut Controls::default()).unwrap();
        let b = m3.on_tick(false, true, &env3, &inputs, &mut Controls::default()).unwrap();
        assert_eq!(a.kind, PathingCommandType::RevalidateGoalAndPath);
        assert_eq!(a, b);
        match a.goal {
            Some(Goal::NeverInGoal(g)) => assert_eq!(*g, goal_run_away(1.0, Some(-59), vec![(5, 1, 5)])),
            other => panic!("{other:?}"),
        }
        let _ = (AIR, Rotation::new(0.0, 0.0));
    }
}
