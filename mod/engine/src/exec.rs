// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/path/PathExecutor.java. LGPL-3.0-or-later, see lib.rs.
//
// The executor follows a path one tick at a time. It reads a world view and a player view and fills a `Controls` (the keys and the
// rotation it wants); `process::Pathing` turns that into an `Intent`. It never touches the game.
//
// Left out of this version, each with a row marked `later` in docs/port-ledger.tsv:
// - `SprintJump` (`flight`, `fly`, `steer`): `sprintJumping` is off by default upstream.
// - `headHitJump` and its helpers: `headHitters` is off by default upstream.
// - the lists of blocks to break and place that upstream recalculates each tick (`recalcBP`): they only feed the renderer and the
//   builder. `blocks_to_break` and `blocks_to_place` answer on demand.
// - `shouldPause` needs the best partial path of the running search, which the game thread cannot read. See `Progress`.
pub mod helper;
pub mod movements;
pub mod state;

#[cfg(test)]
mod break_place_tests;
#[cfg(test)]
mod review_tests;

use crate::look::Vec3;
use crate::movement::helper::{avoid_walking_into, Ctx};
use crate::movement::step::{Movement, MovementKind};
use crate::pos::Pos;
use crate::search::{Path, SearchError};
use crate::world::WorldView;
use movements::{add, descend_safe_mode, descend_skip_to_ascend, direction, safe_to_cancel, to_break, to_place, to_walk_into, update, Env, MoveRuntime};
use state::{CancelReason, Controls, MoveStatus, PlayerView};

/// `PathExecutor.MAX_MAX_DIST_FROM_PATH`.
const MAX_MAX_DIST_FROM_PATH: f64 = 3.0;
/// `PathExecutor.MAX_DIST_FROM_PATH`.
const MAX_DIST_FROM_PATH: f64 = 2.0;
/// Upstream's `MAX_TICKS_AWAY`: 10 seconds, and at least 5.5 seconds (issue #102).
const MAX_TICKS_AWAY: i32 = 200;

/// What the executor knows of the search that is running: upstream's `behavior.getInProgress()`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Progress<'a> {
    /// A search is running.
    pub searching: bool,
    /// The best path of that search so far (upstream: `bestPathSoFar`). The search runs in another thread and cannot be read
    /// halfway, so the glue passes `None` here: `shouldPause` then never pauses. See docs/deviations.md.
    pub best_positions: Option<&'a [Pos]>,
}

/// `PathExecutor`.
#[derive(Clone, Debug)]
pub struct PathExecutor {
    path: Path,
    /// The movements of the path, owned: their costs change as they are checked, and `rt` holds their state.
    movements: Vec<Movement>,
    rt: Vec<MoveRuntime>,
    path_position: i32,
    ticks_away: i32,
    ticks_on_current: i32,
    current_movement_original_cost_estimate: Option<f64>,
    cost_estimate_index: Option<i32>,
    failed: bool,
    sprint_next_tick: bool,
    cancel_reason: Option<CancelReason>,
}

impl PathExecutor {
    /// `PathExecutor.<init>`. The path must be verified (the result of a search is). Upstream's `(Movement)` casts fail on a path
    /// that is not; here that is an error.
    pub fn new(path: Path) -> Result<PathExecutor, SearchError> {
        let movements = path.movements()?.to_vec();
        let rt = movements.iter().map(MoveRuntime::new).collect();
        Ok(PathExecutor { path, movements, rt, path_position: 0, ticks_away: 0, ticks_on_current: 0, current_movement_original_cost_estimate: None, cost_estimate_index: None, failed: false, sprint_next_tick: false, cancel_reason: None })
    }

    /// `PathExecutor.getPath`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `PathExecutor.getPosition`:
    /// `PathExecutor.getPosition`.
    pub fn position(&self) -> i32 {
        self.path_position
    }

    /// `PathExecutor.failed`:
    /// `PathExecutor.failed`.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// `PathExecutor.finished`:
    /// `PathExecutor.finished`.
    pub fn finished(&self) -> bool {
        self.path_position >= self.path.length()
    }

    /// `PathExecutor.isSprinting`:
    /// `PathExecutor.isSprinting`.
    pub fn is_sprinting(&self) -> bool {
        self.sprint_next_tick
    }

    /// Why `cancel` was called, if it was. Not upstream (it only logs).
    pub fn cancel_reason(&self) -> Option<CancelReason> {
        self.cancel_reason
    }

    /// The movements, with the costs they have now.
    pub fn movements(&self) -> &[Movement] {
        &self.movements
    }

    /// `PathExecutor.toBreak`: the cells the movements from the current one on would have to break. With `allowBreak` off this is
    /// empty for any path a search returns; a block that appears later ends the movement as unreachable.
    pub fn blocks_to_break<W: WorldView>(&self, ctx: &Ctx<'_, W>) -> Vec<Pos> {
        let from = self.path_position.max(0) as usize;
        self.movements.iter().skip(from).flat_map(|m| to_break(m, ctx)).collect()
    }

    /// `PathExecutor.toPlace`: the same for the blocks to place.
    pub fn blocks_to_place<W: WorldView>(&self, ctx: &Ctx<'_, W>) -> Vec<Pos> {
        let from = self.path_position.max(0) as usize;
        self.movements.iter().skip(from).flat_map(|m| to_place(m, ctx)).collect()
    }

    /// `PathExecutor.toWalkInto`.
    pub fn blocks_to_walk_into<W: WorldView>(&self, ctx: &Ctx<'_, W>) -> Vec<Pos> {
        let from = self.path_position.max(0) as usize;
        self.movements.iter().skip(from).flat_map(|m| to_walk_into(m, ctx)).collect()
    }

    /// `IPath.ticksRemainingFrom`: the sum of the cached costs from a movement on. This is what `ticksRemainingInSegment` adds up.
    pub fn ticks_remaining_from(&self, path_position: i32) -> f64 {
        let from = path_position.max(0) as usize;
        self.movements.iter().skip(from).map(|m| m.cost.unwrap_or(0.0)).sum()
    }

    /// `PathExecutor.clearKeys`:
    /// `PathExecutor.clearKeys`.
    fn clear_keys(controls: &mut Controls) {
        controls.clear_all_keys();
    }

    /// `PathExecutor.onChangeInPathPosition`:
    /// `PathExecutor.onChangeInPathPosition`.
    fn on_change_in_path_position(&mut self, controls: &mut Controls) {
        Self::clear_keys(controls);
        self.ticks_on_current = 0;
    }

    /// `PathExecutor.cancel`:
    /// `PathExecutor.cancel`. Upstream also stops breaking a block, which nothing here starts.
    fn cancel(&mut self, controls: &mut Controls, why: CancelReason) {
        Self::clear_keys(controls);
        self.path_position = self.path.length().wrapping_add(3);
        self.failed = true;
        self.cancel_reason = Some(why);
    }

    /// Cancel from outside: the same as `cancel`, with the reason an external stop has.
    pub fn abort(&mut self, controls: &mut Controls) {
        self.cancel(controls, CancelReason::Status(MoveStatus::Canceled));
    }

    /// `PathExecutor.onTick`: one tick. Returns true if a movement just finished or the path may be cut onto the next one (the player
    /// is in a "stable" state, not sneaking out over lava); false otherwise.
    pub fn on_tick<W: WorldView>(&mut self, env: &Env<'_, W>, controls: &mut Controls, progress: &Progress<'_>) -> bool {
        // (upstream: `flight != null && fly()` first. There is no flight in this version.)
        if self.path_position == self.path.length().wrapping_sub(1) {
            self.path_position += 1;
        }
        if self.path_position >= self.path.length() {
            return true; // stop bugging me, I'm done
        }
        let pos = self.path_position as usize;
        let where_am_i = env.p.feet();
        if !self.rt[pos].valid.contains(&where_am_i) {
            // this happens for example when you lag out and get teleported back a couple blocks
            let mut i = 0;
            while i < self.path_position && i < self.path.length() {
                if self.rt[i as usize].valid.contains(&where_am_i) {
                    let previous_pos = self.path_position;
                    self.path_position = i;
                    for j in self.path_position..=previous_pos {
                        self.rt[j as usize].reset();
                    }
                    self.on_change_in_path_position(controls);
                    self.on_tick(env, controls, progress);
                    return false;
                }
                i += 1;
            }
            // dont check pathPosition+1. the movement tells us when it's done (e.g. sneak placing)
            // also don't check pathPosition+2 because reasons
            let mut i = self.path_position + 3;
            while i < self.path.length() - 1 {
                if self.rt[i as usize].valid.contains(&where_am_i) {
                    // upstream logs "Skipping forward" when i - pathPosition > 2
                    self.path_position = i - 1;
                    self.on_change_in_path_position(controls);
                    self.on_tick(env, controls, progress);
                    return false;
                }
                i += 1;
            }
        }
        let pos = self.path_position as usize;
        let (dist, _) = self.closest_path_pos(env.p);
        if self.possibly_off_path(dist, MAX_DIST_FROM_PATH, env.p) {
            self.ticks_away += 1;
            // upstream prints "FAR AWAY FROM PATH FOR n TICKS"
            if self.ticks_away > MAX_TICKS_AWAY {
                // Too far away from path for too long, cancelling path
                self.cancel(controls, CancelReason::AwayTooLong);
                return false;
            }
        } else {
            self.ticks_away = 0;
        }
        if self.possibly_off_path(dist, MAX_MAX_DIST_FROM_PATH, env.p) {
            // ok, stop right away, we're way too far.
            self.cancel(controls, CancelReason::TooFarFromPath);
            return false;
        }
        // (upstream recalculates its lists of blocks to break and place here)
        if pos < self.movements.len() - 1 {
            let next = &self.movements[pos + 1];
            if !env.ctx.world.contains_loaded_chunk(next.dest.x, next.dest.z) {
                // Pausing since destination is at edge of loaded chunks
                Self::clear_keys(controls);
                return true;
            }
        }
        let can_cancel = safe_to_cancel(&self.movements[pos], &self.rt[pos], env);
        if self.cost_estimate_index != Some(self.path_position) {
            self.cost_estimate_index = Some(self.path_position);
            // do this only once, when the movement starts, and deliberately get the cost as cached when this path was calculated, not the cost as it is right now
            self.current_movement_original_cost_estimate = Some(self.movements[pos].cost.unwrap_or(0.0));
            let mut i = 1;
            while i < env.set.cost_verification_lookahead && self.path_position + i < self.path.length() - 1 {
                if self.movements[(self.path_position + i) as usize].calculate_cost(env.ctx) >= crate::costs::COST_INF && can_cancel {
                    // Something has changed in the world and a future movement has become impossible. Cancelling.
                    self.cancel(controls, CancelReason::LookaheadImpossible);
                    return true;
                }
                i += 1;
            }
        }
        let current_cost = self.movements[pos].recalculate_cost(env.ctx);
        if current_cost >= crate::costs::COST_INF && can_cancel {
            // Something has changed in the world and this movement has become impossible. Cancelling.
            self.cancel(controls, CancelReason::MovementImpossible);
            return true;
        }
        let original = self.current_movement_original_cost_estimate.unwrap_or(0.0);
        // upstream unboxes a Boolean that is null if the movement was never checked; paths from a search have it set
        if !self.movements[pos].calculated_while_loaded.unwrap_or(true) && current_cost - original > env.set.max_cost_increase && can_cancel {
            // don't do this if the movement was calculated while loaded
            // that means that this isn't a cache error, it's just part of the path interfering with a later part
            self.cancel(controls, CancelReason::CostIncreased);
            return true;
        }
        if self.should_pause(env, progress) {
            // Pausing since current best path is a backtrack
            Self::clear_keys(controls);
            return true;
        }
        let movement_status = update(&self.movements[pos], &mut self.rt[pos], env, controls);
        if movement_status == MoveStatus::Unreachable || movement_status == MoveStatus::Failed {
            let why = match self.rt[pos].state.failure {
                Some(f) => CancelReason::Unsupported(f),
                None => CancelReason::Status(movement_status),
            };
            self.cancel(controls, why);
            return true;
        }
        if movement_status == MoveStatus::Success {
            self.path_position += 1;
            self.on_change_in_path_position(controls);
            self.on_tick(env, controls, progress);
            return true;
        } else {
            self.sprint_next_tick = self.should_sprint_next_tick(env, controls, progress);
            // (upstream: `if (!sprintNextTick) setSprinting(false)`. The intent says it: `Intent::sprint` is false.)
            self.ticks_on_current += 1;
            if self.ticks_on_current as f64 > original + env.set.movement_timeout_ticks as f64 {
                // only cancel if the total time has exceeded the initial estimate
                // as you break the blocks required, the remaining cost goes down, to the point where
                // ticksOnCurrent is greater than recalculateCost + 100
                // this is why we cache cost at the beginning, and don't recalculate for this comparison every tick
                self.cancel(controls, CancelReason::TimedOut);
                return true;
            }
        }
        can_cancel // movement is in progress, but if it reports cancellable, PathingBehavior is good to cut onto the next path
    }

    /// `PathExecutor.closestPathPos`: the distance from the player to the closest valid position of any movement, and that position.
    fn closest_path_pos(&self, p: &PlayerView) -> (f64, Option<Pos>) {
        let mut best = -1.0;
        let mut best_pos = None;
        for rt in &self.rt {
            for pos in &rt.valid {
                let dist = crate::look::distance_to_center(*pos, p.x, p.y, p.z);
                if dist < best || best == -1.0 {
                    best = dist;
                    best_pos = Some(*pos);
                }
            }
        }
        (best, best_pos)
    }

    /// `PathExecutor.possiblyOffPath`:
    /// `PathExecutor.possiblyOffPath`.
    fn possibly_off_path(&self, distance_from_path: f64, leniency: f64, p: &PlayerView) -> bool {
        if distance_from_path > leniency {
            // when we're midair in the middle of a fall, we're very far from both the beginning and the end, but we aren't actually off path
            let pos = self.path_position as usize;
            if self.movements[pos].kind == MovementKind::Fall {
                let fall_dest = self.path.positions()[pos + 1]; // .get(pathPosition) is the block we fell off of
                                                                // ignore Y by using flat distance
                crate::look::distance_to_center(fall_dest, p.x, fall_dest.y as f64 + 0.5, p.z) >= leniency
            } else {
                true
            }
        } else {
            false
        }
    }

    /// `PathExecutor.shouldPause`: while a new search is running whose best path so far passes through the player's position and goes
    /// on from there, wait for it instead of walking a path that is about to be replaced.
    fn should_pause<W: WorldView>(&self, env: &Env<'_, W>, progress: &Progress<'_>) -> bool {
        if !progress.searching {
            return false;
        }
        if !env.p.on_ground {
            return false;
        }
        let feet = env.p.feet();
        if !env.can_walk_on(feet.below(1)) {
            // we're in some kind of sketchy situation, maybe parkouring
            return false;
        }
        if !env.can_walk_through(feet) || !env.can_walk_through(feet.above(1)) {
            // suffocating?
            return false;
        }
        let pos = self.path_position as usize;
        if !safe_to_cancel(&self.movements[pos], &self.rt[pos], env) {
            return false;
        }
        let Some(positions) = progress.best_positions else {
            return false;
        };
        if positions.len() < 3 {
            return false; // not long enough yet to justify pausing, its far from certain we'll actually take this route
        }
        // the first block of the next path will always overlap
        // no need to pause our very last movement when it would have otherwise cleanly exited with MovementStatus SUCCESS
        positions[1..].contains(&feet)
    }

    /// `PathExecutor.snipsnapifpossible`: regardless of the current path position, snap to the player's feet if possible. Returns
    /// whether that was possible.
    pub fn snipsnapifpossible<W: WorldView>(&mut self, env: &Env<'_, W>, controls: &mut Controls) -> bool {
        let feet = env.p.feet();
        if !env.p.on_ground && env.get(feet).fluid_empty {
            // if we're falling in the air, and not in water, don't splice
            return false;
        } else {
            // we are either onGround or in liquid
            if env.p.vy < -0.1 {
                // if we are strictly moving downwards (not stationary)
                // we could be falling through water, which could be unsafe to splice
                return false; // so don't
            }
        }
        let Some(index) = self.path.positions().iter().position(|p| *p == feet) else {
            return false;
        };
        self.path_position = index as i32; // jump directly to current position
        Self::clear_keys(controls);
        true
    }

    /// The pieces of `shouldSprintNextTick` that are about where the movement's direction points: `Vec3i.cross`.
    fn cross(a: Pos, b: Pos) -> Pos {
        Pos::new(a.y.wrapping_mul(b.z).wrapping_sub(a.z.wrapping_mul(b.y)), a.z.wrapping_mul(b.x).wrapping_sub(a.x.wrapping_mul(b.z)), a.x.wrapping_mul(b.y).wrapping_sub(a.y.wrapping_mul(b.x)))
    }

    /// `PathExecutor.shouldSprintNextTick`: Without `sprintJumping` and `headHitters`: both are off by default upstream.
    /// `PathExecutor.shouldSprintNextTick`, without `sprintJumping` and `headHitters`, which are off by default upstream.
    fn should_sprint_next_tick<W: WorldView>(&mut self, env: &Env<'_, W>, controls: &mut Controls, progress: &Progress<'_>) -> bool {
        let requested = controls.keys.sprint;

        // we'll take it from here, no need for minecraft to see we're holding down control and sprint for us
        controls.keys.sprint = false;

        // first and foremost, if allowSprint is off, or if we don't have enough hunger, don't try and sprint
        if !env.set.allow_sprint || env.p.food_level <= 6 {
            return false;
        }
        let pp = self.path_position;
        let cur = self.movements[pp as usize].clone();
        let len = self.path.length();

        // traverse requests sprinting, so we need to do this check first
        if cur.kind == MovementKind::Traverse && pp < len - 3 {
            let next = &self.movements[(pp + 1) as usize];
            // (when the player is too far to the side to sprint into the ascend safely, upstream logs it and goes on)
            if next.kind == MovementKind::Ascend && sprintable_ascend(env, &cur, next, &self.movements[(pp + 2) as usize]) && skip_now(env, &cur) {
                // Skipping traverse to straight ascend
                self.path_position += 1;
                self.on_change_in_path_position(controls);
                self.on_tick(env, controls, progress);
                controls.keys.jump = true;
                return true;
            }
        }

        // if the movement requested sprinting, then we're done
        if requested {
            return true;
        }

        // however, descend and ascend don't request sprinting, because they don't know the context of what movement comes after it
        if cur.kind == MovementKind::Descend {
            if pp < len - 2 {
                // keep this out of onTick, even if that means a tick of delay before it has an effect
                let next = self.movements[(pp + 1) as usize].clone();
                if env.ctx.can_use_frost_walker(&env.get(next.dest.below(1))) {
                    // frostwalker only works if you cross the edge of the block on ground so in some cases we may not overshoot
                    // Since MovementDescend can't know the next movement we have to tell it
                    if next.kind == MovementKind::Traverse || matches!(next.kind, MovementKind::Parkour { .. }) {
                        let could_place_instead = env.ctx.cfg.has_throwaway && matches!(next.kind, MovementKind::Parkour { .. }); // traverse doesn't react fast enough
                                                                                                                                  // this is true if the next movement does not ascend or descends and goes into the same cardinal direction as the descend
                        let same_flat_direction = add(direction(&cur).above(1), direction(&next)) != Pos::new(0, 0, 0) && Self::cross(direction(&cur).above(1), direction(&next)) == Pos::new(0, 0, 0);
                        if same_flat_direction && !could_place_instead {
                            self.rt[pp as usize].force_safe_mode = true;
                        }
                    }
                }
            }
            if descend_safe_mode(&cur, &self.rt[pp as usize], env) && !descend_skip_to_ascend(&cur, env) {
                // Sprinting would be unsafe
                return false;
            }

            if pp < len - 2 {
                let next = self.movements[(pp + 1) as usize].clone();
                if next.kind == MovementKind::Ascend && direction(&cur).above(1) == direction(&next).below(1) {
                    // a descend then an ascend in the same direction
                    self.path_position += 1;
                    self.on_change_in_path_position(controls);
                    self.on_tick(env, controls, progress);
                    // okay to skip clearKeys and / or onChangeInPathPosition here since this isn't possible to repeat, since it's asymmetric
                    // Skipping descend to straight ascend
                    return true;
                }
                if can_sprint_from_descend_into(env, &cur, &next) {
                    if next.kind == MovementKind::Descend && pp < len - 3 {
                        let next_next = self.movements[(pp + 2) as usize].clone();
                        if next_next.kind == MovementKind::Descend && !can_sprint_from_descend_into(env, &next, &next_next) {
                            return false;
                        }
                    }
                    if env.p.feet() == cur.dest {
                        self.path_position += 1;
                        self.on_change_in_path_position(controls);
                        self.on_tick(env, controls, progress);
                    }

                    return true;
                }
            }
        }
        if cur.kind == MovementKind::Ascend && pp != 0 {
            let prev = self.movements[(pp - 1) as usize].clone();
            if prev.kind == MovementKind::Descend && direction(&prev).above(1) == direction(&cur).below(1) {
                let center = cur.src.above(1);
                // playerFeet adds 0.1251 to account for soul sand
                // farmland is 0.9375
                // 0.07 is to account for farmland
                if env.p.y >= center.y as f64 - 0.07 {
                    controls.keys.jump = false;
                    return true;
                }
            }
            if pp < len - 2 && prev.kind == MovementKind::Traverse && sprintable_ascend(env, &prev, &cur, &self.movements[(pp + 1) as usize]) {
                return true;
            }
        }
        if cur.kind == MovementKind::Fall {
            if let Some((data_pos, fall_dest)) = self.override_fall(env, &cur) {
                if !self.path.positions().contains(&fall_dest) {
                    // upstream throws IllegalStateException("Fall override ... returned illegal destination")
                    self.cancel(controls, CancelReason::IllegalState);
                    return true;
                }
                if env.p.feet() == fall_dest {
                    self.path_position = self.path.positions().iter().position(|p| *p == fall_dest).unwrap_or(0) as i32;
                    self.on_change_in_path_position(controls);
                    self.on_tick(env, controls, progress);
                    return true;
                }
                Self::clear_keys(controls);
                // we are not ticking the movement, so we gotta do this ourselves
                let (src, dest) = (cur.src, cur.dest);
                let mut fake = state::MoveState::new();
                // a scratch runtime for the door check only
                let mut rt_fake = MoveRuntime::new(&cur);
                std::mem::swap(&mut rt_fake.state, &mut fake);
                let opened = movements::open_doors(env, &mut rt_fake.state, src, Pos::new(dest.x, src.y, dest.z));
                if !opened {
                    if rt_fake.state.status == MoveStatus::Failed {
                        let why = rt_fake.state.failure.map(CancelReason::Unsupported).unwrap_or(CancelReason::Status(MoveStatus::Failed));
                        self.cancel(controls, why);
                        return true;
                    }
                    if let Some(t) = rt_fake.state.target {
                        controls.look = Some(t.rotation);
                        controls.look_force = t.force;
                    }
                    controls.keys = rt_fake.state.keys;
                    controls.use_pos = rt_fake.state.use_pos;
                    return true;
                }
                controls.look = Some(crate::look::calc_rotation_from_vec3d(env.p.head(), data_pos, env.p.rotations()));
                controls.look_force = false;
                controls.keys.forward = true;
                return true;
            }
        }
        false
    }

    /// `PathExecutor.overrideFall`: a fall of three blocks or less that is followed by flat walking in the same direction is walked
    /// through to its end, so the player does not stop to centre on the landing block. Returns the point to aim at and the position
    /// the walk ends in.
    fn override_fall<W: WorldView>(&self, env: &Env<'_, W>, movement: &Movement) -> Option<(Vec3, Pos)> {
        let dir = direction(movement);
        if dir.y < -3 {
            return None;
        }
        if !to_break(movement, env.ctx).is_empty() {
            return None; // it's breaking
        }
        let flat_dir = Pos::new(dir.x, 0, dir.z);
        let len = self.path.length();
        let mut i = self.path_position + 1;
        'outer: while i < len - 1 && i < self.path_position + 3 {
            let next = &self.movements[i as usize];
            if next.kind != MovementKind::Traverse {
                break;
            }
            if flat_dir != direction(next) {
                break;
            }
            let mut y = next.dest.y;
            while y <= movement.src.y + 1 {
                let chk = Pos::new(next.dest.x, y, next.dest.z);
                if !env.fully_passable(chk) {
                    break 'outer;
                }
                y += 1;
            }
            if !env.can_walk_on(next.dest.below(1)) {
                break;
            }
            i += 1;
        }
        i -= 1;
        if i == self.path_position {
            return None; // no valid extension exists
        }
        let len = (i - self.path_position) as f64 - 0.4;
        Some((Vec3::new(flat_dir.x as f64 * len + movement.dest.x as f64 + 0.5, movement.dest.y as f64, flat_dir.z as f64 * len + movement.dest.z as f64 + 0.5), add(movement.dest, Pos::new(flat_dir.x * (i - self.path_position), 0, flat_dir.z * (i - self.path_position)))))
    }

    /// `PathExecutor.trySplice`: join the planned next path onto this one when they meet (`splicePath`), else cut the start off a path
    /// that has grown too long. `next` stays with the caller. Upstream returns a new executor; this one changes in place, keeping what
    /// the new one is given: the position, the cost estimate and its index, and the ticks on the current movement.
    pub fn try_splice(&mut self, next: Option<&PathExecutor>, max_path_history_length: i32, path_history_cutoff_amount: i32) -> Result<(), SearchError> {
        let Some(next) = next else {
            return self.cut_if_too_long(max_path_history_length, path_history_cutoff_amount);
        };
        match Path::try_splice(Some(&self.path), Some(&next.path), false)? {
            Some(spliced) => {
                if spliced.dest() != next.path.dest() {
                    return Err(SearchError::SanityCheck("Path has another end after splicing"));
                }
                // SplicedPath holds the movements of the first path, then the second's
                self.movements.extend(next.movements.iter().cloned());
                self.rt.extend(next.rt.iter().cloned());
                self.path = spliced;
                self.ticks_away = 0;
                self.failed = false;
                self.sprint_next_tick = false;
                self.cancel_reason = None;
                Ok(())
            }
            None => self.cut_if_too_long(max_path_history_length, path_history_cutoff_amount),
        }
    }

    /// `PathExecutor.cutIfTooLong`:
    /// `PathExecutor.cutIfTooLong`.
    fn cut_if_too_long(&mut self, max_path_history_length: i32, cutoff_amt: i32) -> Result<(), SearchError> {
        if self.path_position > max_path_history_length {
            let new_path = Path::cutoff(&self.path, cutoff_amt, self.path.length() - 1)?;
            if new_path.dest() != self.path.dest() {
                return Err(SearchError::SanityCheck("Path has another end after trimming its start"));
            }
            // Discarding earliest segment movements
            let n = cutoff_amt as usize;
            self.movements.drain(..n);
            self.rt.drain(..n);
            self.path = new_path;
            self.path_position -= cutoff_amt;
            if let Some(i) = self.cost_estimate_index {
                self.cost_estimate_index = Some(i - cutoff_amt);
            }
            self.ticks_away = 0;
            self.failed = false;
            self.sprint_next_tick = false;
        }
        Ok(())
    }
}

/// `PathExecutor.skipNow`: whether the player is centred enough, and far enough from the entrance, to skip a traverse into an ascend.
fn skip_now<W: WorldView>(env: &Env<'_, W>, current: &Movement) -> bool {
    let dir = direction(current);
    let off_target = (dir.x as f64 * (current.src.z as f64 + 0.5 - env.p.z)).abs() + (dir.z as f64 * (current.src.x as f64 + 0.5 - env.p.x)).abs();
    if off_target > 0.1 {
        return false;
    }
    // we are centered
    let head_bonk = current.src.subtract(dir).above(2);
    if env.fully_passable(head_bonk) {
        return true;
    }
    // wait 0.3
    let flat_dist = (dir.x as f64 * (head_bonk.x as f64 + 0.5 - env.p.x)).abs() + (dir.z as f64 * (head_bonk.z as f64 + 0.5 - env.p.z)).abs();
    flat_dist > 0.8
}

/// `PathExecutor.sprintableAscend`: a traverse into an ascend in the same direction can be run through without stopping.
fn sprintable_ascend<W: WorldView>(env: &Env<'_, W>, current: &Movement, next: &Movement, nextnext: &Movement) -> bool {
    if !env.set.sprint_ascends {
        return false;
    }
    if direction(current) != direction(next).below(1) {
        return false;
    }
    let (nn, n) = (direction(nextnext), direction(next));
    if nn.x != n.x || nn.z != n.z {
        return false;
    }
    if !env.can_walk_on(current.dest.below(1)) {
        return false;
    }
    if !env.can_walk_on(next.dest.below(1)) {
        return false;
    }
    if !to_break(next, env.ctx).is_empty() {
        return false; // it's breaking
    }
    for x in 0..2 {
        for y in 0..3 {
            let mut chk = current.src.above(y);
            if x == 1 {
                chk = add(chk, direction(current));
            }
            if !env.fully_passable(chk) {
                return false;
            }
        }
    }
    if avoid_walking_into(&env.get(current.src.above(3)), env.ctx.cfg) {
        return false;
    }
    !avoid_walking_into(&env.get(next.dest.above(2)), env.ctx.cfg)
}

/// `PathExecutor.canSprintFromDescendInto`:
/// `PathExecutor.canSprintFromDescendInto`.
fn can_sprint_from_descend_into<W: WorldView>(env: &Env<'_, W>, current: &Movement, next: &Movement) -> bool {
    if next.kind == MovementKind::Descend && direction(next) == direction(current) {
        return true;
    }
    if !env.can_walk_on(add(current.dest, direction(current))) {
        return false;
    }
    if next.kind == MovementKind::Traverse && direction(next) == direction(current) {
        return true;
    }
    next.kind == MovementKind::Diagonal && env.set.allow_overshoot_diagonal_descend
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{Direction, State};
    use crate::config::Config;
    use crate::exec::state::{Keys, MoveFailure};
    use crate::movement::helper::{Border, Precomputed};
    use crate::pos::Pos;
    use crate::process::pathing::Tables;
    use crate::settings::ExecSettings;
    use crate::testing::{chain_path, test_config, TestWorld, AIR, STONE};

    const BORDER: Border = Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 };

    fn p(x: i32, z: i32) -> Pos {
        Pos::new(x, 1, z)
    }

    fn tables() -> Tables {
        let fx = crate::testing::Fixture::new();
        Tables::new(fx.states.clone(), fx.cfg.clone(), vec![1.0, 1.0])
    }

    /// An executor over a straight path to the east, and the context pieces.
    fn east(from: i32, to: i32) -> (PathExecutor, TestWorld, Tables) {
        let t = tables();
        let world = TestWorld::flat(30, 10);
        let ctx = t.ctx(&world, BORDER);
        let pts: Vec<Pos> = (from..=to).map(|x| p(x, 3)).collect();
        let ex = PathExecutor::new(chain_path(&ctx, &pts).unwrap()).unwrap();
        (ex, world, t)
    }

    fn tick(ex: &mut PathExecutor, t: &Tables, w: &TestWorld, player: &PlayerView, set: &ExecSettings, c: &mut Controls) -> bool {
        let ctx = t.ctx(w, BORDER);
        let env = Env::new(&ctx, player, set, player.feet());
        ex.on_tick(&env, c, &Progress::default())
    }

    fn at(x: f64, z: f64) -> PlayerView {
        PlayerView::standing(x, 1.0, z)
    }

    #[test]
    fn walks_forward_and_sprints_towards_the_destination() {
        let (mut ex, w, t) = east(1, 8);
        let mut c = Controls::default();
        let set = ExecSettings::default();
        tick(&mut ex, &t, &w, &at(1.5, 3.5), &set, &mut c);
        assert!(c.keys.forward && !c.keys.jump && !c.keys.sneak);
        assert!(ex.is_sprinting(), "a traverse over open ground sprints");
        // the yaw looks east: -90
        let look = c.look.expect("a rotation is wanted");
        assert!((look.yaw + 90.0).abs() < 0.01, "{look:?}");
        assert!(!c.look_force, "a plain walk is not forced");
        // hunger: no sprint
        let mut hungry = at(1.5, 3.5);
        hungry.food_level = 6;
        tick(&mut ex, &t, &w, &hungry, &set, &mut c);
        assert!(!ex.is_sprinting());
        // the setting
        let off = ExecSettings { allow_sprint: false, ..Default::default() };
        tick(&mut ex, &t, &w, &at(1.5, 3.5), &off, &mut c);
        assert!(!ex.is_sprinting());
    }

    #[test]
    fn a_movement_that_finishes_moves_on_and_the_path_ends() {
        let (mut ex, w, t) = east(1, 3);
        let mut c = Controls::default();
        let set = ExecSettings::default();
        tick(&mut ex, &t, &w, &at(1.5, 3.5), &set, &mut c);
        assert_eq!(ex.position(), 0);
        tick(&mut ex, &t, &w, &at(2.5, 3.5), &set, &mut c);
        assert_eq!(ex.position(), 1);
        // at the last position: done, with the keys released
        tick(&mut ex, &t, &w, &at(3.5, 3.5), &set, &mut c);
        assert!(ex.finished() && !ex.failed());
        assert_eq!(c.keys, Keys::default());
    }

    #[test]
    fn far_from_the_path_cancels_at_once_and_a_little_far_only_after_a_long_time() {
        let (mut ex, w, t) = east(1, 8);
        let mut c = Controls::default();
        let set = ExecSettings::default();
        tick(&mut ex, &t, &w, &at(4.5, 8.5), &set, &mut c);
        assert!(ex.failed());
        assert_eq!(ex.cancel_reason(), Some(CancelReason::TooFarFromPath));
        assert_eq!(c.keys, Keys::default());
        // 2.5 blocks from the path: not at once, but after 200 ticks
        let (mut ex, w, t) = east(1, 8);
        let mut c = Controls::default();
        let set = ExecSettings { movement_timeout_ticks: 100_000, ..Default::default() };
        for n in 0..=MAX_TICKS_AWAY {
            assert!(!ex.failed(), "cancelled early at {n}");
            tick(&mut ex, &t, &w, &at(4.5, 6.0), &set, &mut c);
        }
        assert_eq!(ex.cancel_reason(), Some(CancelReason::AwayTooLong), "{:?}", ex.cancel_reason());
    }

    #[test]
    fn a_player_thrown_back_resumes_from_the_movement_it_is_in() {
        let (mut ex, w, t) = east(1, 10);
        let mut c = Controls::default();
        let set = ExecSettings::default();
        for x in 1..=5 {
            tick(&mut ex, &t, &w, &at(x as f64 + 0.5, 3.5), &set, &mut c);
        }
        assert_eq!(ex.position(), 4);
        tick(&mut ex, &t, &w, &at(2.5, 3.5), &set, &mut c);
        assert_eq!(ex.position(), 1);
        // and forward by more than two movements: skips
        tick(&mut ex, &t, &w, &at(8.5, 3.5), &set, &mut c);
        assert_eq!(ex.position(), 7);
    }

    #[test]
    fn a_movement_that_takes_too_long_is_cancelled() {
        let (mut ex, w, t) = east(1, 8);
        let mut c = Controls::default();
        let set = ExecSettings { movement_timeout_ticks: 3, ..Default::default() };
        let est = ex.movements()[0].cost.unwrap();
        let mut n = 0;
        while !ex.failed() && n < 50 {
            tick(&mut ex, &t, &w, &at(1.5, 3.5), &set, &mut c);
            n += 1;
        }
        assert_eq!(ex.cancel_reason(), Some(CancelReason::TimedOut));
        assert_eq!(n as f64, (est + 3.0).floor() + 1.0);
    }

    #[test]
    fn a_future_movement_that_became_impossible_cancels_when_a_movement_starts() {
        let (mut ex, mut w, t) = east(1, 10);
        let mut c = Controls::default();
        let set = ExecSettings::default();
        // a wall four movements ahead
        for y in 1..=2 {
            w.set(5, y, 3, STONE);
        }
        tick(&mut ex, &t, &w, &at(1.5, 3.5), &set, &mut c);
        assert_eq!(ex.cancel_reason(), Some(CancelReason::LookaheadImpossible));
        // further than the lookahead (5): not seen yet
        let (mut ex, mut w, t) = east(1, 14);
        for y in 1..=2 {
            w.set(9, y, 3, STONE);
        }
        tick(&mut ex, &t, &w, &at(1.5, 3.5), &set, &mut c);
        assert!(!ex.failed());
    }

    #[test]
    fn the_movement_that_is_blocked_now_cancels() {
        let (mut ex, mut w, t) = east(1, 10);
        let mut c = Controls::default();
        let set = ExecSettings::default();
        tick(&mut ex, &t, &w, &at(1.5, 3.5), &set, &mut c);
        for y in 1..=2 {
            w.set(2, y, 3, STONE);
        }
        tick(&mut ex, &t, &w, &at(1.6, 3.5), &set, &mut c);
        assert_eq!(ex.cancel_reason(), Some(CancelReason::MovementImpossible));
    }

    #[test]
    fn the_player_in_a_wall_or_swimming_fails_the_movement_by_name() {
        let (mut ex, w, t) = east(1, 8);
        let mut c = Controls::default();
        let set = ExecSettings::default();
        let mut pv = at(1.5, 3.5);
        pv.in_wall = true;
        tick(&mut ex, &t, &w, &pv, &set, &mut c);
        assert_eq!(ex.cancel_reason(), Some(CancelReason::Unsupported(MoveFailure::InsideWall)));
        // swimming allowed and in water
        let states = crate::testing::Fixture::new().states;
        let cfg = Config { allow_swimming: true, ..test_config() };
        let t2 = Tables::new(states, cfg, vec![1.0, 1.0]);
        let (mut ex, w, _) = east(1, 8);
        let mut pv = at(1.5, 3.5);
        pv.in_water = true;
        tick(&mut ex, &t2, &w, &pv, &set, &mut c);
        assert_eq!(ex.cancel_reason(), Some(CancelReason::Unsupported(MoveFailure::Swimming)));
    }

    #[test]
    fn a_missing_floor_and_a_pillar_without_a_block_to_place_are_unreachable_and_a_parkour_fails_by_name() {
        use crate::exec::movements::{update, MoveRuntime};
        use crate::movement::step::{Movement, MovementKind};
        let t = tables();
        let w = TestWorld::flat(10, 10);
        let ctx = t.ctx(&w, BORDER);
        let set = ExecSettings::default();
        let mut c = Controls::default();
        let player = at(1.5, 3.5);
        let env = Env::new(&ctx, &player, &set, player.feet());
        // a traverse onto a position with no floor, and nothing to place with: upstream's attemptToPlaceABlock sets UNREACHABLE
        let mv = Movement::new(MovementKind::Traverse, Pos::new(1, 1, 3), Pos::new(2, 1, 3));
        let mut w2 = TestWorld::flat(10, 10);
        w2.set(2, 0, 3, AIR);
        let ctx2 = t.ctx(&w2, BORDER);
        let env2 = Env::new(&ctx2, &player, &set, player.feet());
        let mut rt = MoveRuntime::new(&mv);
        assert_eq!(update(&mv, &mut rt, &env2, &mut c), MoveStatus::Unreachable);
        assert_eq!(c.keys, Keys::default());
        // a pillar on plain ground
        let mv = Movement::new(MovementKind::Pillar, Pos::new(1, 1, 3), Pos::new(1, 2, 3));
        let mut rt = MoveRuntime::new(&mv);
        assert_eq!(update(&mv, &mut rt, &env, &mut c), MoveStatus::Unreachable);
        // parkour
        let mv = Movement::new(MovementKind::Parkour { direction: Direction::East, dist: 3, ascend: false }, Pos::new(1, 1, 3), Pos::new(4, 1, 3));
        let mut rt = MoveRuntime::new(&mv);
        assert_eq!(update(&mv, &mut rt, &env, &mut c), MoveStatus::Failed);
        assert_eq!(rt.state.failure, Some(MoveFailure::Parkour));
        // a movement whose cell holds a block in the way, with breaking off: nothing is aimed or clicked, and the movement is unreachable
        let mut w3 = TestWorld::flat(10, 10);
        w3.set(2, 1, 3, STONE);
        let ctx3 = t.ctx(&w3, BORDER);
        let env3 = Env::new(&ctx3, &player, &set, player.feet());
        let mv = Movement::new(MovementKind::Traverse, Pos::new(1, 1, 3), Pos::new(2, 1, 3));
        let mut rt = MoveRuntime::new(&mv);
        let mut c = Controls::default();
        assert_eq!(update(&mv, &mut rt, &env3, &mut c), MoveStatus::Unreachable);
        assert!(c.look.is_none() && c.select.is_none() && c.keys == Keys::default());
    }

    /// States for the door tests: air, stone, a closed-or-open wooden door facing east, a fence gate.
    fn door_tables() -> Tables {
        let fx = crate::testing::Fixture::new();
        let mut states = fx.states.clone();
        states.push(State { name: "oak_door".into(), door: true, wooden_door: true, pathfindable: true, fluid_empty: true, door_facing: Direction::East, ..State::default() });
        states.push(State { name: "oak_fence_gate".into(), gate: true, pathfindable: true, fluid_empty: true, ..State::default() });
        let n = states.len();
        let _ = Precomputed::new(&states, &fx.cfg);
        Tables::new(states, fx.cfg.clone(), vec![1.0; n])
    }

    struct DoorWorld {
        inner: TestWorld,
        door: Vec<(i32, i32, i32, usize)>,
        open: bool,
    }

    impl WorldView for DoorWorld {
        fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
            self.door.iter().find(|d| (d.0, d.1, d.2) == (x, y, z)).map(|d| d.3).unwrap_or_else(|| self.inner.state_id(x, y, z))
        }
        fn contains_loaded_chunk(&self, x: i32, z: i32) -> bool {
            self.inner.contains_loaded_chunk(x, z)
        }
        fn is_open(&self, _x: i32, _y: i32, _z: i32) -> bool {
            self.open
        }
        fn min_y(&self) -> i32 {
            0
        }
        fn height(&self) -> i32 {
            16
        }
    }

    fn run_door(open: bool, can_use: bool, id: usize) -> (PathExecutor, Controls) {
        let t = door_tables();
        let world = DoorWorld { inner: TestWorld::flat(10, 10), door: vec![(3, 1, 3, id), (3, 2, 3, id)], open };
        let ctx = t.ctx(&world, BORDER);
        let mut ex = PathExecutor::new(chain_path(&ctx, &[p(2, 3), p(3, 3), p(4, 3)]).unwrap()).unwrap();
        let set = ExecSettings { can_use_blocks: can_use, ..Default::default() };
        let player = at(2.7, 3.5);
        let mut c = Controls::default();
        let env = Env::new(&ctx, &player, &set, player.feet());
        ex.on_tick(&env, &mut c, &Progress::default());
        (ex, c)
    }

    #[test]
    fn a_closed_wooden_door_is_opened_by_a_use_when_the_glue_can_and_fails_by_name_when_it_cannot() {
        // the door faces east and the path goes east through it: closed, it blocks
        let (ex, c) = run_door(false, true, 2);
        assert!(!ex.failed());
        assert!(c.keys.click_right && !c.keys.forward);
        assert_eq!(c.use_pos, Some(Pos::new(3, 1, 3)));
        assert!(c.look.is_some());
        let (ex, c) = run_door(false, false, 2);
        assert!(ex.failed());
        assert_eq!(ex.cancel_reason(), Some(CancelReason::Unsupported(MoveFailure::NeedsUseBlock(Pos::new(3, 1, 3)))));
        assert_eq!(c.keys, Keys::default());
        // open: walk through
        let (ex, c) = run_door(true, false, 2);
        assert!(!ex.failed() && c.keys.forward && !c.keys.click_right);
    }

    #[test]
    fn a_closed_fence_gate_is_the_same() {
        let (ex, c) = run_door(false, true, 3);
        // upstream looks at the upper cell of the gate first
        assert!(!ex.failed() && c.keys.click_right && c.use_pos == Some(Pos::new(3, 2, 3)), "{:?} {:?}", ex.cancel_reason(), c);
        let (ex, _) = run_door(false, false, 3);
        assert_eq!(ex.cancel_reason(), Some(CancelReason::Unsupported(MoveFailure::NeedsUseBlock(Pos::new(3, 2, 3)))));
        let (ex, c) = run_door(true, false, 3);
        assert!(!ex.failed() && c.keys.forward);
    }

    #[test]
    fn splicing_joins_the_next_path_and_a_long_history_is_cut() {
        let t = tables();
        let w = TestWorld::flat(30, 10);
        let ctx = t.ctx(&w, BORDER);
        let first: Vec<Pos> = (1..=6).map(|x| p(x, 3)).collect();
        let second: Vec<Pos> = (6..=12).map(|x| p(x, 3)).collect();
        let mut a = PathExecutor::new(chain_path(&ctx, &first).unwrap()).unwrap();
        let b = PathExecutor::new(chain_path(&ctx, &second).unwrap()).unwrap();
        a.path_position = 2;
        a.ticks_on_current = 3;
        a.try_splice(Some(&b), 300, 50).unwrap();
        assert_eq!(a.path().dest(), p(12, 3));
        assert_eq!(a.movements().len(), 11);
        assert_eq!((a.position(), a.ticks_on_current), (2, 3));
        // cut: more than 3 movements done, 2 are cut off
        let before = a.path().length();
        a.path_position = 6;
        a.cost_estimate_index = Some(6);
        a.try_splice(None, 3, 2).unwrap();
        assert_eq!(a.path().length(), before - 2);
        assert_eq!((a.position(), a.cost_estimate_index), (4, Some(4)));
        assert_eq!(a.movements().len() as i32, a.path().length() - 1);
        assert_eq!(a.path().dest(), p(12, 3));
    }

    #[test]
    fn a_descend_sprints_on_into_another_descend_but_not_into_a_traverse() {
        // Steps down: feet at y 3 for x 0 to 3, y 2 at x 4, y 1 from x 5. The path walks to the edge, down, down, and on.
        let t = tables();
        let mut w = TestWorld::flat(20, 10);
        for x in 0..=3 {
            for z in 0..10 {
                w.set(x, 1, z, STONE);
                w.set(x, 2, z, STONE);
            }
        }
        for z in 0..10 {
            w.set(4, 1, z, STONE);
        }
        let ctx = t.ctx(&w, BORDER);
        let pts = [Pos::new(2, 3, 3), Pos::new(3, 3, 3), Pos::new(4, 2, 3), Pos::new(5, 1, 3), Pos::new(6, 1, 3), Pos::new(7, 1, 3)];
        let ex0 = PathExecutor::new(chain_path(&ctx, &pts).unwrap()).unwrap();
        let set = ExecSettings::default();
        let mut c = Controls::default();
        let mut ex = ex0.clone();
        let mut pv = PlayerView::standing(3.9, 3.0, 3.5);
        pv.on_ground = true;
        tick(&mut ex, &t, &w, &pv, &set, &mut c);
        assert_eq!(ex.position(), 1);
        assert!(ex.is_sprinting(), "a descend into a descend in the same direction sprints on");
        // The second descend, into a traverse in the same direction. Upstream's `canSprintFromDescendInto` compares the direction of
        // the traverse (1, 0, 0) with the descend's (1, -1, 0) and never finds them equal: at 1b8e87c it does not sprint on here.
        // (Older Baritone compared `next.getDirection().down()`.) Kept as it is; see docs/deviations.md.
        let mut ex = ex0.clone();
        ex.path_position = 2;
        let mut pv = PlayerView::standing(4.5, 2.0, 3.5);
        pv.on_ground = true;
        tick(&mut ex, &t, &w, &pv, &set, &mut c);
        assert_eq!(ex.position(), 2);
        assert!(!ex.is_sprinting());
        assert!(c.keys.forward);
    }
}
