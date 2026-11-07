// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/Movement.java (update, prepared, validPositions,
// toBreak, toPlace), MovementHelper.java (moveTowards, openDoors, isDoorPassable, isGatePassable, steppingOnBlocks) and
// movements/Movement{Traverse,Ascend,Descend,Diagonal,Fall,Pillar,Downward}.java (updateState, safeToCancel, prepared).
// LGPL-3.0-or-later, see lib.rs.
//
// The rules of each movement, with breaking and placing: `prepared` breaks what is in the way, and the bridge, the ascend and the pillar place
// blocks. A branch that needs swimming or parkour, or a click on a door the glue cannot make, ends the movement as `Failed` with a
// `MoveFailure`: nothing is skipped silently. Comments of the form "upstream:" say what the branch does there.
//
// A movement does not touch the game. It reads the player (`PlayerView`), the hotbar (`Inventory`) and the world (`Ctx`), and writes a
// `MoveState`.
use super::helper::{attempt_to_place_a_block, is_looking_at, move_towards_with_slight_rotation, reachable, switch_to_best_tool_for, PlaceResult};
use super::state::*;
use crate::blocks::{Block, Direction};
use crate::costs::COST_INF;
use crate::inventory::{find_hotbar_slot, select_throwaway, Inventory, NoInventory};
use crate::look::{block_pos_center, calc_rotation_from_vec3d, AimProcessor, Rotation};
use crate::movement::diagonal::is_blocking_door;
use crate::movement::helper::{is_bottom_slab, is_climbable, is_liquid, is_water, Bs, Ctx};
use crate::movement::step::{Movement, MovementKind};
use crate::pos::Pos;
use crate::settings::ExecSettings;
use crate::world::WorldView;

/// What the executor keeps for each movement of the path, next to the movement itself.
#[derive(Clone, Debug)]
pub struct MoveRuntime {
    pub state: MoveState,
    /// `MovementDescend.numTicks` and `MovementDownward.numTicks`.
    pub num_ticks: i32,
    /// `MovementAscend.ticksWithoutPlacement`.
    pub ticks_without_placement: i32,
    /// `MovementTraverse.wasTheBridgeBlockAlwaysThere`.
    pub was_the_bridge_block_always_there: bool,
    /// `MovementDescend.forceSafeMode`.
    pub force_safe_mode: bool,
    /// `Movement.getValidPositions`, computed once.
    pub valid: Vec<Pos>,
}

impl MoveRuntime {
    pub fn new(mv: &Movement) -> MoveRuntime {
        MoveRuntime { state: MoveState::new(), num_ticks: 0, ticks_without_placement: 0, was_the_bridge_block_always_there: true, force_safe_mode: false, valid: valid_positions(mv) }
    }

    /// `Movement.reset`:
    /// `MovementAscend.reset`:
    /// `MovementDescend.reset`:
    /// `MovementDownward.reset`:
    /// `MovementTraverse.reset`:
    /// `Movement.reset` and the resets of the subclasses.
    pub fn reset(&mut self) {
        self.state = MoveState::new();
        self.num_ticks = 0;
        self.ticks_without_placement = 0;
        self.was_the_bridge_block_always_there = true;
        self.force_safe_mode = false;
    }
}

/// What a movement reads of the game besides the world, the player and the hotbar.
#[derive(Clone, Copy, Debug)]
pub struct GameFacts {
    /// The reach for blocks (upstream's `getBlockReachDistance`): 4.5 in survival, 5 in creative.
    pub reach: f64,
    /// The world's dimension is the Nether (`Level.NETHER`): a water bucket evaporates there.
    pub nether: bool,
    /// Upstream's aim processor (`LookBehavior.getAimProcessor`): what the game is asked to turn to for an aim.
    pub aim: AimProcessor,
}

impl Default for GameFacts {
    fn default() -> Self {
        GameFacts { reach: 4.5, nether: false, aim: AimProcessor::new(0.5) }
    }
}

/// What a movement reads: the world through the context, the player, the hotbar, the settings and where a new path would start.
pub struct Env<'a, W: WorldView> {
    pub ctx: &'a Ctx<'a, W>,
    pub p: &'a PlayerView,
    pub set: &'a ExecSettings,
    /// `PathingBehavior.pathStart`, which `Movement.playerInValidPosition` also accepts.
    pub path_start: Pos,
    pub inv: &'a dyn Inventory,
    pub game: GameFacts,
}

impl<'a, W: WorldView> Env<'a, W> {
    /// An environment with an empty hotbar and the default facts of the game.
    pub fn new(ctx: &'a Ctx<'a, W>, p: &'a PlayerView, set: &'a ExecSettings, path_start: Pos) -> Env<'a, W> {
        Env { ctx, p, set, path_start, inv: &NoInventory, game: GameFacts::default() }
    }

    pub fn get(&self, p: Pos) -> Bs<'a> {
        self.ctx.get(p.x, p.y, p.z)
    }
    /// `MovementHelper.canWalkThrough(IPlayerContext,BetterBlockPos)`.
    pub fn can_walk_through(&self, p: Pos) -> bool {
        self.ctx.can_walk_through_at(p.x, p.y, p.z)
    }
    /// `MovementHelper.canWalkOn(IPlayerContext,BlockPos)`.
    pub fn can_walk_on(&self, p: Pos) -> bool {
        self.ctx.can_walk_on_at(p.x, p.y, p.z)
    }
    /// `MovementHelper.fullyPassable(IPlayerContext,BlockPos)`.
    pub fn fully_passable(&self, p: Pos) -> bool {
        self.ctx.fully_passable_at(p.x, p.y, p.z)
    }
    /// `MovementHelper.isLiquid(IPlayerContext,BlockPos)`.
    pub fn is_liquid(&self, p: Pos) -> bool {
        is_liquid(&self.get(p))
    }
    /// `MovementHelper.isWater(IPlayerContext,BlockPos)`.
    pub fn is_water(&self, p: Pos) -> bool {
        is_water(&self.get(p))
    }
}

// ---------------------------------------------------------------- small helpers

/// Upstream's `Vec3i.offset`: the sum of two positions as vectors.
pub fn add(a: Pos, b: Pos) -> Pos {
    Pos::new(a.x.wrapping_add(b.x), a.y.wrapping_add(b.y), a.z.wrapping_add(b.z))
}

/// `Movement.getDirection`: `dest - src`.
pub fn direction(mv: &Movement) -> Pos {
    mv.dest.subtract(mv.src)
}

/// `Vec3i.below` on a vector.
pub fn below(p: Pos) -> Pos {
    p.below(1)
}

/// `Vec3i.above` on a vector.
pub fn above(p: Pos) -> Pos {
    p.above(1)
}

fn opposite(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::East => Direction::West,
        Direction::West => Direction::East,
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
    }
}

/// Upstream's `Direction.getClockWise`: a quarter turn clockwise seen from above. Up and down turn into themselves, as vanilla throws there.
fn clockwise(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        o => o,
    }
}

/// `Direction.getCounterClockWise`.
fn counter_clockwise(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::West,
        Direction::West => Direction::South,
        Direction::South => Direction::East,
        Direction::East => Direction::North,
        o => o,
    }
}

/// The directions in upstream's `Direction.values()` order, with which `openDoors` looks for the one from a position to the next.
const ALL_DIRECTIONS: [Direction; 6] = [Direction::Down, Direction::Up, Direction::North, Direction::South, Direction::West, Direction::East];

/// Upstream's `Direction.from2DDataValue`: 0 south, 1 west, 2 north, 3 east.
const HORIZONTAL_2D: [Direction; 4] = [Direction::South, Direction::West, Direction::North, Direction::East];

fn is_gate(b: &Bs<'_>) -> bool {
    b.gate
}

/// `MovementHelper.moveTowards`: look at the centre of a block, keeping the pitch, and press forward.
fn move_towards<W: WorldView>(env: &Env<'_, W>, st: &mut MoveState, pos: Pos) {
    let rot = calc_rotation_from_vec3d(env.p.head(), block_pos_center(pos), env.p.rotations()).with_pitch(env.p.pitch);
    st.target = Some(Target { rotation: rot, force: false });
    st.keys.forward = true;
}

/// Ends the movement because it needs something this version does not do.
fn fail(st: &mut MoveState, why: MoveFailure) {
    st.failure = Some(why);
    st.status = MoveStatus::Failed;
}

/// The right click on a door or gate: `MovementHelper.openDoors` and the gate branch of `MovementTraverse.updateState` set a
/// forced target at the block and `CLICK_RIGHT`. Without `can_use_blocks` the movement fails instead.
/// Upstream aims at the middle of the block's collision shape (`VecUtils.calculateBlockCenter`), and for a gate asks `RotationUtils.reachable`
/// for a rotation that a ray trace confirms. There are no shapes or ray traces here: the aim is the middle of the block.
fn use_block<W: WorldView>(env: &Env<'_, W>, st: &mut MoveState, pos: Pos) {
    if !env.set.can_use_blocks {
        fail(st, MoveFailure::NeedsUseBlock(pos));
        return;
    }
    let rot = calc_rotation_from_vec3d(env.p.head(), block_pos_center(pos), env.p.rotations());
    st.target = Some(Target { rotation: rot, force: true });
    st.keys.click_right = true;
    st.use_pos = Some(pos);
}

/// `MovementHelper.isDoorPassable`: The open state is `WorldView::is_open`.
/// `MovementHelper.isDoorPassable`.
fn is_door_passable<W: WorldView>(env: &Env<'_, W>, pos: Pos, door: &Bs<'_>, side: Direction) -> bool {
    let open = env.ctx.world.is_open(pos.x, pos.y, pos.z);
    let closed_facing = door.door_facing;
    let open_facing = if door.door_hinge_left { clockwise(closed_facing) } else { counter_clockwise(closed_facing) };
    side != if open { open_facing } else { closed_facing }
}

/// `MovementHelper.isGatePassable`: The open state is `WorldView::is_open`.
/// `MovementHelper.isGatePassable`.
fn is_gate_passable<W: WorldView>(env: &Env<'_, W>, gate_pos: Pos, player_pos: Pos) -> bool {
    if player_pos == gate_pos {
        return false;
    }
    let state = env.get(gate_pos);
    if !state.gate {
        return true;
    }
    env.ctx.world.is_open(gate_pos.x, gate_pos.y, gate_pos.z)
}

/// `MovementHelper.openDoors`: false when a closed wooden door is in the way, and the movement has asked for it to be opened (or failed).
/// Upstream finds the direction with `.findFirst().get()`, which throws when `to` is not next to `from`; no movement does that, and here it answers true.
pub fn open_doors<W: WorldView>(env: &Env<'_, W>, st: &mut MoveState, from: Pos, to: Pos) -> bool {
    let Some(dir) = ALL_DIRECTIONS.iter().copied().find(|d| from.relative(*d, 1) == to) else {
        return true;
    };
    for pos in [from, to, from.above(1), to.above(1)] {
        let side = if pos == to || pos == to.above(1) { dir } else { opposite(dir) };
        let door = env.get(pos);
        if door.door && door.wooden_door && !is_door_passable(env, pos, &door, side) {
            use_block(env, st, pos);
            return false;
        }
    }
    true
}

/// `MovementHelper.steppingOnBlocks`: the blocks below the cells the player's box overlaps, for a 0.6 by 1.8 box.
pub fn stepping_on_blocks(p: &PlayerView) -> Vec<Pos> {
    let bp = p.block_position();
    let (min_x, max_x, min_y, max_y, min_z, max_z) = (p.x - 0.3, p.x + 0.3, p.y, p.y + 1.8, p.z - 0.3, p.z + 0.3);
    let mut blocks = vec![];
    for x in -1i32..=1 {
        for z in -1i32..=1 {
            let (lx, ly, lz) = (bp.x.wrapping_add(x) as f64, bp.y as f64, bp.z.wrapping_add(z) as f64);
            if min_x < lx + 1.0 && max_x > lx && min_y < ly + 1.0 && max_y > ly && min_z < lz + 1.0 && max_z > lz {
                blocks.push(Pos::new(bp.x.wrapping_add(x), bp.y.wrapping_sub(1), bp.z.wrapping_add(z)));
            }
        }
    }
    blocks
}

fn stepping_on_magma<W: WorldView>(env: &Env<'_, W>) -> bool {
    stepping_on_blocks(env.p).into_iter().any(|b| env.get(b).block == Block::MagmaBlock)
}

// ---------------------------------------------------------------- positions

/// `Movement.toBreakAll`: the cells the movement must be able to walk through (upstream's `positionsToBreak`). The order is upstream's.
pub fn positions_to_break(mv: &Movement) -> Vec<Pos> {
    let (src, dest) = (mv.src, mv.dest);
    match mv.kind {
        MovementKind::Traverse => vec![dest.above(1), dest],
        MovementKind::Ascend => vec![dest, src.above(2), dest.above(1)],
        MovementKind::Descend => vec![dest.above(2), dest.above(1), dest],
        MovementKind::Diagonal => {
            let c1 = Pos::new(dest.x, src.y, src.z);
            let c2 = Pos::new(src.x, src.y, dest.z);
            vec![c1, c1.above(1), c2, c2.above(1), dest, dest.above(1)]
        }
        MovementKind::Pillar => vec![src.above(2)],
        MovementKind::Downward => vec![dest],
        MovementKind::Fall => fall_positions_to_break(src, dest),
        MovementKind::Parkour { .. } => vec![],
    }
}

/// `MovementFall.buildPositionsToBreak`:
/// The cells of a fall: the column above the destination, from one above the source down to the destination and one more.
fn fall_positions_to_break(src: Pos, dest: Pos) -> Vec<Pos> {
    let diff_y = src.y.wrapping_sub(dest.y).wrapping_abs();
    (0..diff_y.wrapping_add(2)).map(|i| Pos::new(dest.x, src.y.wrapping_add(1).wrapping_sub(i), dest.z)).collect()
}

/// Upstream's `Movement.positionToPlace`: the block the movement stands on or against, which would be placed if missing.
pub fn position_to_place(mv: &Movement) -> Option<Pos> {
    match mv.kind {
        MovementKind::Traverse | MovementKind::Ascend | MovementKind::Descend => Some(mv.dest.below(1)),
        MovementKind::Pillar => Some(mv.src),
        _ => None,
    }
}

/// `MovementTraverse.calculateValidPositions`: `valid_positions`, by kind.
/// `MovementAscend.calculateValidPositions`: `valid_positions`, by kind.
/// `MovementDescend.calculateValidPositions`: `valid_positions`, by kind.
/// `MovementDiagonal.calculateValidPositions`: `valid_positions`, by kind.
/// `MovementDownward.calculateValidPositions`: `valid_positions`, by kind.
/// `MovementFall.calculateValidPositions`: `valid_positions`, by kind.
/// `MovementPillar.calculateValidPositions`: `valid_positions`, by kind.
/// `Movement.calculateValidPositions` of each movement: where the player may be while this movement is the current one.
pub fn valid_positions(mv: &Movement) -> Vec<Pos> {
    let (src, dest) = (mv.src, mv.dest);
    match mv.kind {
        MovementKind::Traverse | MovementKind::Pillar | MovementKind::Downward | MovementKind::Parkour { .. } => vec![src, dest],
        MovementKind::Ascend => {
            // sometimes we back up to place the block, also sprint ascends, also skip descend to straight ascend
            let prior = src.subtract(direction(mv)).above(1);
            vec![src, src.above(1), dest, prior, prior.above(1)]
        }
        MovementKind::Descend => vec![src, dest.above(1), dest],
        MovementKind::Diagonal => {
            let diag_a = Pos::new(src.x, src.y, dest.z);
            let diag_b = Pos::new(dest.x, src.y, src.z);
            if dest.y < src.y {
                vec![src, dest.above(1), diag_a, diag_b, dest, diag_a.below(1), diag_b.below(1)]
            } else if dest.y > src.y {
                vec![src, src.above(1), diag_a, diag_b, dest, diag_a.above(1), diag_b.above(1)]
            } else {
                vec![src, dest, diag_a, diag_b]
            }
        }
        MovementKind::Fall => {
            let mut set = vec![src];
            let mut y = src.y.wrapping_sub(dest.y);
            while y >= 0 {
                set.push(dest.above(y));
                y -= 1;
            }
            set
        }
    }
}

/// `MovementDiagonal.toBreak`: The diagonal looks at its last two cells only.
/// `Movement.toBreak`: the positions to break that cannot be walked through now. For the diagonal, upstream looks at its last two only.
pub fn to_break<W: WorldView>(mv: &Movement, ctx: &Ctx<'_, W>) -> Vec<Pos> {
    let all = positions_to_break(mv);
    let from = if mv.kind == MovementKind::Diagonal { 4 } else { 0 };
    all[from..].iter().copied().filter(|p| !ctx.can_walk_through_at(p.x, p.y, p.z)).collect()
}

/// `Movement.toPlace`: the position to place when it cannot be walked on now.
pub fn to_place<W: WorldView>(mv: &Movement, ctx: &Ctx<'_, W>) -> Vec<Pos> {
    match position_to_place(mv) {
        Some(p) if !ctx.can_walk_on_at(p.x, p.y, p.z) => vec![p],
        _ => vec![],
    }
}

/// `Movement.toWalkInto`: Empty except for the diagonal.
/// `MovementDiagonal.toWalkInto`: the first four positions to break, when they cannot be walked through. Empty for the others.
pub fn to_walk_into<W: WorldView>(mv: &Movement, ctx: &Ctx<'_, W>) -> Vec<Pos> {
    if mv.kind != MovementKind::Diagonal {
        return vec![];
    }
    positions_to_break(mv)[..4].iter().copied().filter(|p| !ctx.can_walk_through_at(p.x, p.y, p.z)).collect()
}

// ---------------------------------------------------------------- safeToCancel

/// `Movement.safeToCancel()`: By kind.
/// `Movement.safeToCancel(MovementState)`: True unless the kind says otherwise.
/// `MovementTraverse.safeToCancel`:
/// `MovementAscend.safeToCancel`: not once a block had to be placed.
/// `MovementFall.safeToCancel`:
/// `Movement.safeToCancel()` with each movement's override: whether the path may be cut or stopped now.
pub fn safe_to_cancel<W: WorldView>(mv: &Movement, rt: &MoveRuntime, env: &Env<'_, W>) -> bool {
    let running = rt.state.status == MoveStatus::Running;
    match mv.kind {
        // MovementTraverse.safeToCancel: not while on the way over a gap (the block is placed upstream)
        MovementKind::Traverse => !running || env.can_walk_on(mv.dest.below(1)),
        // MovementAscend.safeToCancel: if we had to place, don't allow pause
        MovementKind::Ascend => !running || rt.ticks_without_placement == 0,
        MovementKind::Diagonal => diagonal_safe_to_cancel(mv, env),
        // MovementFall.safeToCancel
        MovementKind::Fall => env.p.feet() == mv.src || !running,
        _ => true,
    }
}

/// `MovementDiagonal.safeToCancel`:
/// `MovementDiagonal.safeToCancel`.
fn diagonal_safe_to_cancel<W: WorldView>(mv: &Movement, env: &Env<'_, W>) -> bool {
    let (src, dest) = (mv.src, mv.dest);
    let offset = 0.25;
    let (x, y, z) = (env.p.x, env.p.y - 1.0, env.p.z);
    // standard
    if env.p.feet() == src {
        return true;
    }
    // both corners are walkable
    if env.can_walk_on(Pos::new(src.x, src.y - 1, dest.z)) && env.can_walk_on(Pos::new(dest.x, src.y - 1, src.z)) {
        return true;
    }
    // we are in a likely unwalkable corner, check for a supporting block
    let feet = env.p.feet();
    if feet == Pos::new(src.x, src.y, dest.z) || feet == Pos::new(dest.x, src.y, src.z) {
        let at = |dx: f64, dz: f64| env.can_walk_on(Pos::new((x + dx).floor() as i32, y.floor() as i32, (z + dz).floor() as i32));
        return at(offset, offset) || at(offset, -offset) || at(-offset, offset) || at(-offset, -offset);
    }
    true
}

// ---------------------------------------------------------------- update

/// Whether an incidental left click (the traverse's "something is in the way", the pillar's block at the feet) may be pressed: `allowBreak` is on, and
/// the block under the crosshair is not one that must never be broken (docs/deviations.md D28; upstream presses it whatever it is, U11).
fn may_click_left<W: WorldView>(env: &Env<'_, W>) -> bool {
    env.ctx.cfg.allow_break && !env.p.looking_at.is_some_and(|h| env.ctx.never_break_at(h.pos.x, h.pos.y, h.pos.z))
}

/// `Movement.prepared`: breaks what is in the way. It waits for a falling block at the cell, then, for the first cell that cannot be walked
/// through, asks for the best tool and aims at a face of the block that the ray reaches (and presses the left button once the view is on it);
/// when no face is reachable it breaks the block anyway, aimed at its middle. True when nothing is in the way. Upstream's final case, a
/// block in the way with no aim at all, sets the status to unreachable; that branch cannot be reached, since both paths of the loop return
/// (docs/deviations.md D27).
fn prepared_base<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) -> bool {
    if rt.state.status == MoveStatus::Waiting {
        return true;
    }
    for block_pos in positions_to_break(mv) {
        if env.set.pause_mining_for_falling_blocks && env.ctx.world.falling_block_near(block_pos.x, block_pos.y, block_pos.z) {
            return false;
        }
        if !env.can_walk_through(block_pos) {
            // can't break air, so don't try
            if env.ctx.mining_duration_at(block_pos.x, block_pos.y, block_pos.z, false) >= COST_INF {
                // Not upstream (docs/deviations.md D28): the settings do not allow this block to be broken (`allowBreak`, `allowBreakAnyway`,
                // `blocksToDisallowBreaking`, a liquid next to it, a tool that cannot dig it). Upstream relies on the cost check that cancels
                // the path, which does not run while the movement is not safe to cancel; then it breaks the block all the same.
                rt.state.status = MoveStatus::Unreachable;
                return true;
            }
            switch_to_best_tool_for(env, &mut rt.state, block_pos);
            if let Some(rot_towards_block) = reachable(env, block_pos, env.game.reach, false) {
                rt.state.target = Some(Target { rotation: rot_towards_block, force: true });
                if is_looking_at(env.p, block_pos) || env.p.rotations().is_really_close_to(rot_towards_block) {
                    rt.state.keys.click_left = true;
                }
                return false;
            }
            // get rekt minecraft
            // i'm doing it anyway
            // i dont care if theres snow in the way!!!!!!!
            // you dont own me!!!!
            rt.state.target = Some(Target { rotation: calc_rotation_from_vec3d(env.p.head(), block_pos_center(block_pos), env.p.rotations()), force: true });
            // don't check selectedblock on this one, this is a fallback when we can't see any face directly, it's intended to be breaking the "incorrect" block
            rt.state.keys.click_left = true;
            return false;
        }
    }
    // (upstream: if `somethingInTheWay`, "we have no target rotation to reach any", the status becomes unreachable. Both ways out of the
    // `if` above return, so `somethingInTheWay` is always false here.)
    true
}

/// `MovementDiagonal.prepared`: Always ready.
/// `MovementFall.prepared`:
/// `MovementTraverse.prepared`:
/// `MovementPillar.prepared`:
/// `prepared` of each movement.
fn prepared<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) -> bool {
    let feet = env.p.feet();
    match mv.kind {
        // MovementDiagonal.prepared
        MovementKind::Diagonal => true,
        // MovementFall.prepared: only if one of the first three cells needs breaking
        MovementKind::Fall => {
            if rt.state.status == MoveStatus::Waiting {
                return true;
            }
            let all = positions_to_break(mv);
            for p in all.iter().take(4) {
                if !env.can_walk_through(*p) {
                    return prepared_base(mv, rt, env);
                }
            }
            true
        }
        MovementKind::Traverse => {
            // MovementTraverse.prepared: on a ladder or vine, sneak to stay on it
            if (feet == mv.src || feet == mv.src.below(1)) && is_climbable(&env.get(mv.src.below(1))) {
                rt.state.keys.sneak = true;
            }
            prepared_base(mv, rt, env)
        }
        MovementKind::Pillar => {
            // MovementPillar.prepared
            if (feet == mv.src || feet == mv.src.below(1)) && is_climbable(&env.get(mv.src.below(1))) {
                rt.state.keys.sneak = true;
            }
            if env.is_water(mv.dest.above(1)) {
                return true;
            }
            prepared_base(mv, rt, env)
        }
        _ => prepared_base(mv, rt, env),
    }
}

/// `Movement.updateState`: prepared, then waiting, then running.
fn base_update_state<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    if !prepared(mv, rt, env) {
        rt.state.status = MoveStatus::Prepping;
        return;
    } else if rt.state.status == MoveStatus::Prepping {
        rt.state.status = MoveStatus::Waiting;
    }
    if rt.state.status == MoveStatus::Waiting {
        rt.state.status = MoveStatus::Running;
    }
}

/// `Movement.playerInValidPosition`:
/// `Movement.playerInValidPosition`.
fn player_in_valid_position<W: WorldView>(rt: &MoveRuntime, env: &Env<'_, W>) -> bool {
    rt.valid.contains(&env.p.feet()) || rt.valid.contains(&env.path_start)
}

/// `Movement.update`: one tick of one movement. The movement's inputs replace the controls, as upstream's `clearAllKeys` and
/// `setInputForceState` do; the rotation it wants becomes the look target.
pub fn update<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>, controls: &mut Controls) -> MoveStatus {
    // the keys of the last tick are gone, and the movement sets what it needs now
    rt.state.keys = Keys::default();
    rt.state.use_pos = None;
    rt.state.select = None;
    update_state(mv, rt, env);
    let feet = env.p.feet();
    if env.is_liquid(feet) || env.p.in_water {
        if env.ctx.cfg.allow_swimming && env.p.in_water {
            // upstream: shouldSwim, holdWaterline and preTurnIntoTheBend
            fail(&mut rt.state, MoveFailure::Swimming);
        } else {
            if env.ctx.cfg.allow_swimming && env.p.swimming {
                // standing up to breathe
                rt.state.keys.sprint = false;
            }
            if env.p.y < mv.dest.y as f64 + 0.6 {
                rt.state.keys.jump = true;
            }
        }
    }
    if env.p.in_wall && !rt.state.status.is_complete() {
        // upstream: switch to the best tool and press the left button
        fail(&mut rt.state, MoveFailure::InsideWall);
    }

    // If the movement target has to force the new rotations, or we aren't using silent move, then force the rotations
    if let Some(t) = rt.state.target {
        controls.look = Some(t.rotation);
    }
    controls.clear_all_keys();
    controls.keys = rt.state.keys;
    controls.use_pos = rt.state.use_pos;
    if rt.state.select.is_some() {
        controls.select = rt.state.select;
    }
    rt.state.keys = Keys::default();
    // If the current status indicates a completed movement
    if rt.state.status.is_complete() {
        controls.clear_all_keys();
    }
    rt.state.status
}

/// `updateState` of each movement.
fn update_state<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    match mv.kind {
        MovementKind::Traverse => traverse(mv, rt, env),
        MovementKind::Ascend => ascend(mv, rt, env),
        MovementKind::Descend => descend(mv, rt, env),
        MovementKind::Diagonal => diagonal(mv, rt, env),
        MovementKind::Fall => fall(mv, rt, env),
        MovementKind::Pillar => pillar(mv, rt, env),
        MovementKind::Downward => downward(mv, rt, env),
        MovementKind::Parkour { .. } => {
            base_update_state(mv, rt, env);
            fail(&mut rt.state, MoveFailure::Parkour);
        }
    }
}

/// `MovementTraverse.updateState`: breaks the blocks ahead (walking on while it does, if `walkWhileBreaking`), opens doors and gates, walks
/// on, or bridges: sneaks to the edge, places the block under the destination, and backs up to place it against the block it left.
fn traverse<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    let (src, dest) = (mv.src, mv.dest);
    base_update_state(mv, rt, env);
    let positions = positions_to_break(mv);
    let pb0 = env.get(positions[0]);
    let pb1 = env.get(positions[1]);
    if rt.state.status != MoveStatus::Running {
        // if the setting is enabled
        if !env.set.walk_while_breaking {
            return;
        }
        // and if we're prepping (aka mining the block in front)
        if rt.state.status != MoveStatus::Prepping {
            return;
        }
        // and if it's fine to walk into the blocks in front
        if crate::movement::helper::avoid_walking_into(&pb0, env.ctx.cfg) {
            return;
        }
        if crate::movement::helper::avoid_walking_into(&pb1, env.ctx.cfg) {
            return;
        }
        // and we aren't already pressed up against the block
        let dist = (env.p.x - (f64::from(dest.x) + 0.5)).abs().max((env.p.z - (f64::from(dest.z) + 0.5)).abs());
        if dist < 0.83 {
            return;
        }
        let Some(target) = rt.state.target else {
            // this can happen rarely when the server lags and doesn't send the falling sand entity until you've already walked through the block and are now mining the next one
            return;
        };
        // combine the yaw to the center of the destination, and the pitch to the specific block we're trying to break
        // it's safe to do this since the two blocks we break (in a traverse) are right on top of each other and so will have the same yaw
        let yaw_to_dest = calc_rotation_from_vec3d(env.p.head(), block_pos_center(dest), env.p.rotations()).yaw;
        let mut pitch_to_break = target.rotation.pitch;
        if crate::movement::helper::is_block_normal_cube(&pb0) || (pb0.air && (crate::movement::helper::is_block_normal_cube(&pb1) || pb1.air)) {
            // in the meantime, before we're right up against the block, we can break efficiently at this angle
            pitch_to_break = 26.0;
        }
        rt.state.target = Some(Target { rotation: Rotation::new(yaw_to_dest, pitch_to_break), force: true });
        rt.state.keys.forward = true;
        rt.state.keys.sprint = true;
        return;
    }
    let st = &mut rt.state;
    let fd = env.get(src.below(1));
    let ladder = is_climbable(&fd);

    // sneak may have been set while mining an adjacent block, but we still want it when the player is about to go on magma
    st.keys.sneak = env.ctx.cfg.allow_walk_on_magma && stepping_on_magma(env);

    if !open_doors(env, st, src, dest) {
        return;
    }

    if is_gate(&pb0) || is_gate(&pb1) {
        let blocked = if !is_gate_passable(env, positions[0], src.above(1)) {
            Some(positions[0])
        } else if !is_gate_passable(env, positions[1], src) {
            Some(positions[1])
        } else {
            None
        };
        if let Some(b) = blocked {
            use_block(env, st, b);
            return;
        }
    }

    let place = dest.below(1);
    let is_the_bridge_block_there = env.can_walk_on(place) || ladder || (env.ctx.can_use_frost_walker(&env.get(place)));
    let feet = env.p.feet();
    if feet.y != dest.y && !ladder {
        // upstream logs "Wrong Y coordinate"
        if feet.y < dest.y {
            st.keys.jump = true;
        }
        return;
    }

    if is_the_bridge_block_there {
        if feet == dest {
            st.status = MoveStatus::Success;
            return;
        }
        let dir = direction(mv);
        if env.set.overshoot_traverse && (feet == add(dest, dir) || feet == add(add(dest, dir), dir)) {
            st.status = MoveStatus::Success;
            return;
        }
        let low = env.get(src);
        let high = env.get(src.above(1));
        if env.p.y > src.y as f64 + 0.1 && !env.p.on_ground && !env.p.in_water && (is_climbable(&low) || is_climbable(&high)) {
            // hitting W could cause us to climb the ladder instead of going forward
            // wait until we're on the ground
            return;
        }
        let into = add(dest.subtract(src), dest);
        let into_below = env.get(into);
        let into_above = env.get(into.above(1));
        if rt.was_the_bridge_block_always_there && (!env.is_liquid(feet) || env.set.sprint_in_water) && (!crate::movement::helper::avoid_walking_into(&into_below, env.ctx.cfg) || is_water(&into_below)) && !crate::movement::helper::avoid_walking_into(&into_above, env.ctx.cfg) {
            st.keys.sprint = true;
        }
        let dest_down = env.get(dest.below(1));
        if feet.y != dest.y && ladder && is_climbable(&dest_down) {
            st.keys.jump = true;
        }
        move_towards(env, st, positions[0]);
    } else {
        rt.was_the_bridge_block_always_there = false;
        let standing_on = env.get(feet.below(1));
        if standing_on.block == Block::SoulSand || standing_on.slab {
            // see issue #118
            let dist = (f64::from(dest.x) + 0.5 - env.p.x).abs().max((f64::from(dest.z) + 0.5 - env.p.z).abs());
            if dist < 0.85 {
                // 0.5 + 0.3 + epsilon
                move_towards(env, st, dest);
                st.keys.forward = false;
                st.keys.back = true;
                return;
            }
        }
        let dist1 = (env.p.x - (f64::from(dest.x) + 0.5)).abs().max((env.p.z - (f64::from(dest.z) + 0.5)).abs());
        let assume_safe_walk = env.set.assume_safe_walk;
        let p = attempt_to_place_a_block(env, st, dest.below(1), false, !assume_safe_walk);
        if (p == PlaceResult::ReadyToPlace || dist1 < 0.6) && !assume_safe_walk {
            st.keys.sneak = true;
        }
        match p {
            PlaceResult::ReadyToPlace => {
                if env.p.crouching || assume_safe_walk {
                    st.keys.click_right = true;
                }
                return;
            }
            PlaceResult::Attempting => {
                if dist1 > 0.83 {
                    // might need to go forward a bit
                    let yaw = calc_rotation_from_vec3d(env.p.head(), block_pos_center(dest), env.p.rotations()).yaw;
                    if st.target.is_some_and(|t| (t.rotation.yaw - yaw).abs() < 0.1) {
                        // but only if our attempted place is straight ahead
                        st.keys.forward = true;
                        return;
                    }
                } else if st.target.is_some_and(|t| env.p.rotations().is_really_close_to(t.rotation)) {
                    // well i guess theres something in the way
                    st.keys.click_left = may_click_left(env);
                    return;
                }
                return;
            }
            PlaceResult::NoOption => {}
        }
        if feet == dest {
            // If we are in the block that we are trying to get to, we are sneaking over air and we need to place a block beneath us against the one we just walked off of
            // faceX, faceY, faceZ is the middle of the face between from and to
            let face_x = (f64::from(dest.x) + f64::from(src.x) + 1.0) * 0.5;
            let face_y = (f64::from(dest.y) + f64::from(src.y) - 1.0) * 0.5;
            let face_z = (f64::from(dest.z) + f64::from(src.z) + 1.0) * 0.5;
            let goal_look = src.below(1); // this is the block we were just standing on, and the one we want to place against

            let back_to_face = calc_rotation_from_vec3d(env.p.head(), crate::look::Vec3::new(face_x, face_y, face_z), env.p.rotations());
            let pitch = back_to_face.pitch;
            let dist2 = (env.p.x - face_x).abs().max((env.p.z - face_z).abs());
            if dist2 < 0.29 {
                // see issue #208
                let yaw = calc_rotation_from_vec3d(block_pos_center(dest), env.p.head(), env.p.rotations()).yaw;
                st.target = Some(Target { rotation: Rotation::new(yaw, pitch), force: true });
                st.keys.back = true;
            } else {
                st.target = Some(Target { rotation: back_to_face, force: true });
            }
            if is_looking_at(env.p, goal_look) {
                st.keys.click_right = true; // wait to right click until we are able to place
                return;
            }
            if st.target.is_some_and(|t| env.p.rotations().is_really_close_to(t.rotation)) {
                st.keys.click_left = may_click_left(env);
            }
            return;
        }
        move_towards_with_slight_rotation(env, st, dest);
    }
}

/// `MovementAscend.updateState`: jumps onto the block, or places it first: sneaks, places the block under the destination and, after ten
/// ticks without one, backs up in case the player stands in the way.
fn ascend<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    let (src, dest) = (mv.src, mv.dest);
    if env.p.feet().y < src.y {
        // this check should run even when in preparing state (breaking blocks)
        rt.state.status = MoveStatus::Unreachable;
        return;
    }
    base_update_state(mv, rt, env);
    if rt.state.status != MoveStatus::Running {
        return;
    }
    let st = &mut rt.state;
    let feet = env.p.feet();
    let dir = direction(mv);
    if feet == dest || feet == add(dest, below(dir)) {
        st.status = MoveStatus::Success;
        return;
    }
    if !open_doors(env, st, src.above(1), dest) {
        return;
    }
    let place = dest.below(1);
    let jumping_onto = env.get(place);
    if !env.ctx.can_walk_on(place.x, place.y, place.z, jumping_onto) {
        rt.ticks_without_placement += 1;
        if attempt_to_place_a_block(env, st, dest.below(1), false, true) == PlaceResult::ReadyToPlace {
            st.keys.sneak = true;
            if env.p.crouching {
                st.keys.click_right = true;
            }
        }
        if rt.ticks_without_placement > 10 {
            // After 10 ticks without placement, we might be standing in the way, move back
            st.keys.back = true;
        }
        return;
    }
    move_towards(env, st, dest);

    st.keys.sneak = env.ctx.cfg.allow_walk_on_magma && jumping_onto.block == Block::MagmaBlock;

    if is_bottom_slab(&jumping_onto) && !is_bottom_slab(&env.get(src.below(1))) {
        return; // don't jump while walking from a non double slab into a bottom slab
    }

    if env.set.assume_step || feet == src.above(1) {
        // no need to hit space if we're already jumping
        return;
    }

    let x_axis = (src.x.wrapping_sub(dest.x)).wrapping_abs() as f64; // either 0 or 1
    let z_axis = (src.z.wrapping_sub(dest.z)).wrapping_abs() as f64; // either 0 or 1
    let flat_dist_to_next = x_axis * ((dest.x as f64 + 0.5) - env.p.x).abs() + z_axis * ((dest.z as f64 + 0.5) - env.p.z).abs();
    let side_dist = z_axis * ((dest.x as f64 + 0.5) - env.p.x).abs() + x_axis * ((dest.z as f64 + 0.5) - env.p.z).abs();

    let lateral_motion = x_axis * env.p.vz + z_axis * env.p.vx;
    if lateral_motion.abs() > 0.1 {
        return;
    }

    if head_bonk_clear(mv, env) {
        st.keys.jump = true;
        return;
    }

    if flat_dist_to_next > 1.2 || side_dist > 0.2 {
        return;
    }

    // Once we are pointing the right way and moving, start jumping
    // This is slightly more efficient because otherwise we might start jumping before moving, and fall down without moving onto the block we want to jump onto
    // Also wait until we are close enough, because we might jump and hit our head on an adjacent block
    st.keys.jump = true;
}

/// `MovementAscend.headBonkClear`: the four cells around the cell two above the source can be walked through.
fn head_bonk_clear<W: WorldView>(mv: &Movement, env: &Env<'_, W>) -> bool {
    let start_up = mv.src.above(2);
    for d in HORIZONTAL_2D {
        if !env.can_walk_through(start_up.relative(d, 1)) {
            // We might bonk our head
            return false;
        }
    }
    true
}

/// `MovementDescend.safeMode`:
/// `MovementDescend.safeMode`.
pub fn descend_safe_mode<W: WorldView>(mv: &Movement, rt: &MoveRuntime, env: &Env<'_, W>) -> bool {
    if rt.force_safe_mode {
        return true;
    }
    // (dest - src) + dest is offset 1 more in the same direction
    // so it's the block we'd need to worry about running into if we decide to sprint straight through this descend
    let into = add(mv.dest.subtract(mv.src.below(1)), mv.dest);
    if descend_skip_to_ascend(mv, env) {
        // if dest extends into can't walk through, but the two above are can walk through, then we can overshoot and glitch in that weird way
        return true;
    }
    for y in 0..=2 {
        // we could hit any of the three blocks
        if crate::movement::helper::avoid_walking_into(&env.get(into.above(y)), env.ctx.cfg) {
            return true;
        }
    }
    false
}

/// `MovementDescend.skipToAscend`:
/// `MovementDescend.skipToAscend`.
pub fn descend_skip_to_ascend<W: WorldView>(mv: &Movement, env: &Env<'_, W>) -> bool {
    let into = add(mv.dest.subtract(mv.src.below(1)), mv.dest);
    !env.can_walk_through(into) && env.can_walk_through(into.above(1)) && env.can_walk_through(into.above(2))
}

/// `MovementDescend.updateState`:
/// `MovementDescend.updateState`.
fn descend<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    let (src, dest) = (mv.src, mv.dest);
    base_update_state(mv, rt, env);
    if rt.state.status != MoveStatus::Running {
        return;
    }
    let player_feet = env.p.feet();
    let fake_dest = Pos::new(dest.x.wrapping_mul(2).wrapping_sub(src.x), dest.y, dest.z.wrapping_mul(2).wrapping_sub(src.z));
    if (player_feet == dest || player_feet == fake_dest) && (env.is_liquid(dest) || env.p.y - (dest.y as f64) < 0.5) {
        // lilypads
        // Wait until we're actually on the ground before saying we're done because sometimes we continue to fall if the next action starts immediately
        rt.state.status = MoveStatus::Success;
        return;
    }

    if !open_doors(env, &mut rt.state, src, dest.above(1)) {
        return;
    }

    if descend_safe_mode(mv, rt, env) {
        let dest_x = (src.x as f64 + 0.5) * 0.17 + (dest.x as f64 + 0.5) * 0.83;
        let dest_z = (src.z as f64 + 0.5) * 0.17 + (dest.z as f64 + 0.5) * 0.83;
        let rot = calc_rotation_from_vec3d(env.p.head(), crate::look::Vec3::new(dest_x, dest.y as f64, dest_z), env.p.rotations()).with_pitch(env.p.pitch);
        rt.state.target = Some(Target { rotation: rot, force: false });
        rt.state.keys.forward = true;
        return;
    }
    let diff_x = env.p.x - (dest.x as f64 + 0.5);
    let diff_z = env.p.z - (dest.z as f64 + 0.5);
    let ab = (diff_x * diff_x + diff_z * diff_z).sqrt();
    let x = env.p.x - (src.x as f64 + 0.5);
    let z = env.p.z - (src.z as f64 + 0.5);
    let from_start = (x * x + z * z).sqrt();

    rt.state.keys.sneak = env.ctx.cfg.allow_walk_on_magma && env.get(env.p.block_position().below(1)).block == Block::MagmaBlock;

    if player_feet != dest || ab > 0.25 {
        let early = rt.num_ticks < 20;
        rt.num_ticks += 1;
        if early && from_start < 1.25 {
            move_towards(env, &mut rt.state, fake_dest);
        } else {
            move_towards(env, &mut rt.state, dest);
        }
    }
}

/// `MovementDiagonal.updateState`:
/// `MovementDiagonal.updateState`.
fn diagonal<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    let (src, dest) = (mv.src, mv.dest);
    base_update_state(mv, rt, env);
    if rt.state.status != MoveStatus::Running {
        return;
    }
    let feet = env.p.feet();
    if feet == dest {
        rt.state.status = MoveStatus::Success;
        return;
    } else if !player_in_valid_position(rt, env) && !(env.is_liquid(src) && rt.valid.contains(&feet.above(1))) {
        rt.state.status = MoveStatus::Unreachable;
        return;
    }

    let st = &mut rt.state;
    let a = Pos::new(src.x, src.y, dest.z);
    if !is_blocking_door(env.get(a).s, src.x, dest.z, dest.x, src.z) && (!open_doors(env, st, src, a) || !open_doors(env, st, Pos::new(src.x, dest.y, dest.z), dest)) {
        return;
    }
    let b = Pos::new(dest.x, src.y, src.z);
    if !is_blocking_door(env.get(b).s, dest.x, src.z, src.x, dest.z) && (!open_doors(env, st, src, b) || !open_doors(env, st, Pos::new(dest.x, dest.y, src.z), dest)) {
        return;
    }

    if dest.y > src.y && env.p.y < src.y as f64 + 0.1 && env.p.horizontal_collision {
        st.keys.jump = true;
    }
    if diagonal_sprint(mv, env) {
        st.keys.sprint = true;
    }
    st.keys.sneak = env.ctx.cfg.allow_walk_on_magma && stepping_on_magma(env);
    move_towards(env, st, dest);
}

/// `MovementDiagonal.sprint`:
/// `MovementDiagonal.sprint`.
fn diagonal_sprint<W: WorldView>(mv: &Movement, env: &Env<'_, W>) -> bool {
    if env.is_liquid(env.p.feet()) && !env.set.sprint_in_water {
        return false;
    }
    for p in positions_to_break(mv).iter().take(4) {
        if !env.can_walk_through(*p) {
            return false;
        }
    }
    true
}

/// `MovementFall.willPlaceBucket`: runs `dynamicFallCost` on the movement's own source and destination column and answers its result, which is
/// true on the bucket branch only (a ladder halfway down the fall resets the count, so a fall past one is not a bucket fall).
fn will_place_bucket<W: WorldView>(mv: &Movement, env: &Env<'_, W>) -> bool {
    let ctx = env.ctx;
    let below = ctx.get(mv.dest.x, mv.src.y.wrapping_sub(2), mv.dest.z);
    let mut res = crate::movement::helper::MoveResult::default();
    crate::movement::descend::dynamic_fall_cost(ctx, mv.src.y, mv.dest.x, mv.dest.z, 0.0, below, &mut res)
}

/// `MovementFall.avoid`: the way the ladder below faces, if there is one within 15 blocks under the feet.
fn fall_avoid<W: WorldView>(env: &Env<'_, W>) -> Option<Direction> {
    for i in 0..15 {
        let state = env.get(env.p.feet().below(i));
        if state.block == Block::Ladder {
            return Some(state.door_facing);
        }
    }
    None
}

/// `MovementFall.updateState`: falls, and lands in water with a bucket when the fall is longer than `maxFallHeightNoWater`: it holds the water
/// bucket, looks straight down and clicks when the crosshair is on the landing block, and picks the water up again with an empty bucket.
fn fall<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    let (src, dest) = (mv.src, mv.dest);
    base_update_state(mv, rt, env);
    if rt.state.status != MoveStatus::Running {
        return;
    }
    let st = &mut rt.state;
    let player_feet = env.p.feet();
    let to_dest = calc_rotation_from_vec3d(env.p.head(), block_pos_center(dest), env.p.rotations());
    let dest_state = env.get(dest);

    if env.get(dest.below(1)).block == Block::MagmaBlock && stepping_on_blocks(env.p).iter().all(|b| env.can_walk_through(*b)) {
        st.keys.sneak = true;
    }

    let mut target_rotation: Option<Rotation> = None;
    let is_water_dest = is_water(&dest_state);
    if !is_water_dest && will_place_bucket(mv, env) && player_feet != dest {
        let bucket = find_hotbar_slot(env.inv, "water_bucket");
        if bucket.is_none() || env.game.nether {
            st.status = MoveStatus::Unreachable;
            return;
        }
        if env.p.y - (dest.y as f64) < env.game.reach && !env.p.on_ground {
            st.select = bucket;
            target_rotation = Some(Rotation::new(to_dest.yaw, 90.0));
            if is_looking_at(env.p, dest) || is_looking_at(env.p, dest.below(1)) {
                st.keys.click_right = true;
            }
        }
    }

    if !open_doors(env, st, src, Pos::new(dest.x, src.y, dest.z)) {
        return;
    }

    match target_rotation {
        Some(r) => st.target = Some(Target { rotation: r, force: true }),
        None => st.target = Some(Target { rotation: to_dest, force: false }),
    }
    if player_feet == dest && (env.p.y - (player_feet.y as f64) < 0.094 || is_water_dest) {
        // 0.094 because lilypads
        if is_water_dest {
            // only match water, not flowing water (which we cannot pick up with a bucket)
            if let Some(empty) = find_hotbar_slot(env.inv, "bucket") {
                st.select = Some(empty);
                if env.p.vy >= 0.0 {
                    st.keys.click_right = true;
                }
                return;
            } else if env.p.vy >= 0.0 {
                st.status = MoveStatus::Success;
                return;
            } // don't else return state; we need to stay centered because this water might be flowing under the surface
        } else {
            st.status = MoveStatus::Success;
            return;
        }
    }
    let dest_center = block_pos_center(dest); // we are moving to the 0.5 center not the edge (like if we were falling on a ladder)
    if (env.p.x + env.p.vx - dest_center.x).abs() > 0.1 || (env.p.z + env.p.vz - dest_center.z).abs() > 0.1 {
        if !env.p.on_ground && env.p.vy.abs() > 0.4 {
            st.keys.sneak = true;
        }
        st.keys.forward = true;
    }
    let (avoid_x, avoid_z) = match fall_avoid(env) {
        None => {
            let d = src.subtract(dest);
            (d.x, d.z)
        }
        Some(d) => {
            let (ax, az) = (d.step_x(), d.step_z());
            let dist = (ax as f64 * (dest_center.x - ax as f64 / 2.0 - env.p.x)).abs() + (az as f64 * (dest_center.z - az as f64 / 2.0 - env.p.z)).abs();
            if dist < 0.6 {
                st.keys.forward = true;
            } else if !env.p.on_ground {
                st.keys.sneak = false;
            }
            (ax, az)
        }
    };
    if target_rotation.is_none() {
        let dest_center_offset = crate::look::Vec3::new(dest_center.x + 0.125 * avoid_x as f64, dest_center.y, dest_center.z + 0.125 * avoid_z as f64);
        let rot = calc_rotation_from_vec3d(env.p.head(), dest_center_offset, env.p.rotations());
        st.target = Some(Target { rotation: rot, force: false });
    }
}

/// `MovementPillar.updateState`: climbs a ladder or vine or a column of water, or places a block under itself: sneaks, jumps, looks down and
/// presses the right button at the top of each jump (once the player is crouching and the crosshair is on the block under the feet).
fn pillar<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    let (src, dest) = (mv.src, mv.dest);
    base_update_state(mv, rt, env);
    if rt.state.status != MoveStatus::Running {
        return;
    }
    let st = &mut rt.state;
    if env.p.feet().y < src.y {
        st.status = MoveStatus::Unreachable;
        return;
    }
    let from_down = env.get(src);
    if is_water(&from_down) && env.is_water(dest) {
        // stay centered while swimming up a water column
        let rot = calc_rotation_from_vec3d(env.p.head(), block_pos_center(dest), env.p.rotations());
        st.target = Some(Target { rotation: rot, force: false });
        let dest_center = block_pos_center(dest);
        if (env.p.x - dest_center.x).abs() > 0.2 || (env.p.z - dest_center.z).abs() > 0.2 {
            st.keys.forward = true;
        }
        if env.p.feet() == dest {
            st.status = MoveStatus::Success;
        }
        return;
    }
    let ladder = is_climbable(&from_down);

    let rotation = calc_rotation_from_vec3d(env.p.head(), block_pos_center(src), env.p.rotations());
    if !ladder {
        st.target = Some(Target { rotation: env.p.rotations().with_pitch(rotation.pitch), force: true });
    }

    let mut block_is_there = env.can_walk_on(src) || ladder;
    if ladder {
        if env.p.feet() == dest {
            st.status = MoveStatus::Success;
            return;
        }

        move_towards(env, st, dest);
        st.keys.jump = true;
        return;
    }
    // Get ready to place a throwaway block
    if !env.set.allow_place {
        st.status = MoveStatus::Unreachable;
        return;
    }
    match select_throwaway(env.inv, &env.set.tool.acceptable_throwaway_items) {
        Some(slot) => st.select = Some(slot),
        None => {
            st.status = MoveStatus::Unreachable;
            return;
        }
    }

    st.keys.sneak = true;
    // since (lower down) we only right click once player.isSneaking, and that happens the tick after we request to sneak

    let diff_x = env.p.x - (dest.x as f64 + 0.5);
    let diff_z = env.p.z - (dest.z as f64 + 0.5);
    let dist = (diff_x * diff_x + diff_z * diff_z).sqrt();
    let flat_motion = (env.p.vx * env.p.vx + env.p.vz * env.p.vz).sqrt();
    if dist > 0.17 {
        //why 0.17? because it seemed like a good number, that's why
        //[explanation added after baritone port lol] also because it needs to be less than 0.2 because of the 0.3 sneak limit
        //and 0.17 is reasonably less than 0.2

        // If it's been more than forty ticks of trying to jump and we aren't done yet, go forward, maybe we are stuck
        st.keys.forward = true;

        // revise our target to both yaw and pitch if we're going to be moving forward
        st.target = Some(Target { rotation, force: true });
    } else if flat_motion < 0.05 {
        // If our Y coordinate is above our goal, stop jumping
        st.keys.jump = env.p.y < dest.y as f64;
    }

    if !block_is_there {
        let fr_state = env.get(src);
        // TODO: Evaluate usage of getMaterial().isReplaceable()
        if !(fr_state.air || fr_state.replaceable) {
            if let Some(rot) = reachable(env, src, env.game.reach, false) {
                st.target = Some(Target { rotation: rot, force: true });
            }
            st.keys.jump = false; // breaking is like 5x slower when you're jumping
            st.keys.click_left = may_click_left(env);
            block_is_there = false;
        } else if env.p.crouching && (is_looking_at(env.p, src.below(1)) || is_looking_at(env.p, src)) && env.p.y > dest.y as f64 + 0.1 {
            st.keys.click_right = true;
        }
    }

    // If we are at our goal and the block below us is placed
    if env.p.feet() == dest && block_is_there {
        st.status = MoveStatus::Success;
    }
}

/// `MovementDownward.updateState`:
/// `MovementDownward.updateState`.
fn downward<W: WorldView>(mv: &Movement, rt: &mut MoveRuntime, env: &Env<'_, W>) {
    let dest = mv.dest;
    base_update_state(mv, rt, env);
    if rt.state.status != MoveStatus::Running {
        return;
    }
    if env.p.feet() == dest {
        rt.state.status = MoveStatus::Success;
        return;
    } else if !player_in_valid_position(rt, env) {
        rt.state.status = MoveStatus::Unreachable;
        return;
    }
    let diff_x = env.p.x - (dest.x as f64 + 0.5);
    let diff_z = env.p.z - (dest.z as f64 + 0.5);
    let ab = (diff_x * diff_x + diff_z * diff_z).sqrt();

    let early = rt.num_ticks < 10;
    rt.num_ticks += 1;
    if early && ab < 0.2 {
        return;
    }
    move_towards(env, &mut rt.state, positions_to_break(mv)[0]);
}

/// Whether the cost of a movement is `COST_INF` or more.
pub fn impossible(cost: f64) -> bool {
    cost >= COST_INF
}
