// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/MovementHelper.java (attemptToPlaceABlock,
// switchToBestToolFor, moveTowardsWithoutRotation, moveTowardsWithSlightRotation), MovementOption.java and, from RotationUtils.java,
// reachable, reachableCenter and reachableOffset. LGPL-3.0-or-later, see lib.rs.
//
// What breaking and placing need from the movements: where to aim, which slot to hold, and whether the crosshair is on the right block.
// The world is read through `Env`; the game is never touched. Everything here writes into a `MoveState` (the aim, the keys and the slot).
use super::movements::Env;
use super::state::{MoveState, MoveStatus, Target};
use crate::blocks::Direction;
use crate::inventory::{select_throwaway_for_location, Throwaway};
use crate::look::{block_pos_center, calc_rotation_from_vec3d, Rotation, Vec3};
use crate::pos::Pos;
use crate::raytrace::{infer_sneaking_eye_position, ray_trace_towards};
use crate::world::WorldView;

/// Upstream's `MovementHelper.PlaceResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceResult {
    ReadyToPlace,
    Attempting,
    NoOption,
}

/// `InventoryBehavior.selectThrowawayForLocation`: the slot to place a block at this position from, as the builder wants it when it is active.
pub fn select_throwaway_at<W: WorldView>(env: &Env<'_, W>, pos: Pos) -> Option<Throwaway> {
    select_throwaway_for_location(env.inv, &env.set.tool.acceptable_throwaway_items, env.hint, pos.x, pos.y, pos.z, env.get(pos).id, env.set.tool.allow_inventory)
}

/// `InventoryBehavior.throwaway` with `select` true: the hotbar slot is selected; a stack from the main inventory is asked to go to hotbar slot 7 first
/// (`requestSwapWithHotBar(i, 7)`), which `update` hands to the inventory mover, and slot 7 is selected whether or not the move is made yet.
pub fn select_found_throwaway(st: &mut MoveState, found: Throwaway) {
    if let Throwaway::Inventory(i) = found {
        st.swap_in = Some(i);
    }
    st.select = Some(found.hand_slot());
}

/// Upstream's `IPlayerContext.isLookingAt`: the crosshair is on this block, as the game says.
pub fn is_looking_at(p: &super::state::PlayerView, pos: Pos) -> bool {
    p.looking_at.is_some_and(|h| h.pos == pos)
}

/// The eyes: where a ray starts. A player who would sneak has the lower eye of a sneaking player.
fn eyes<W: WorldView>(env: &Env<'_, W>, would_sneak: bool) -> Vec3 {
    if would_sneak {
        infer_sneaking_eye_position(env.p.x, env.p.y, env.p.z)
    } else {
        env.p.head()
    }
}

/// `RotationUtils.reachableOffset(IPlayerContext,BlockPos,Vec3,double,boolean)`: the rotation that looks at a point of a block, when the
/// ray along it (after the aim processor has turned it onto the mouse grid) meets that block. A fire block also counts when the ray
/// meets the block under it.
pub fn reachable_offset<W: WorldView>(env: &Env<'_, W>, pos: Pos, offset_pos: Vec3, reach: f64, would_sneak: bool) -> Option<Rotation> {
    let eyes = eyes(env, would_sneak);
    let rotation = calc_rotation_from_vec3d(eyes, offset_pos, env.p.rotations());
    let actual = env.game.aim.peek_rotation(env.p.rotations(), rotation);
    let hit = ray_trace_towards(env.ctx, eyes, actual, reach)?;
    if hit.pos == pos {
        return Some(rotation);
    }
    if env.get(pos).base_fire && hit.pos == pos.below(1) {
        return Some(rotation);
    }
    None
}

/// `RotationUtils.reachableCenter(IPlayerContext,BlockPos,double,boolean)`: `reachableOffset` at the middle of the block. Upstream takes
/// the middle of the block's collision shape (`VecUtils.calculateBlockCenter`); there are no shapes here (docs/deviations.md D25).
pub fn reachable_center<W: WorldView>(env: &Env<'_, W>, pos: Pos, reach: f64, would_sneak: bool) -> Option<Rotation> {
    reachable_offset(env, pos, block_pos_center(pos), reach, would_sneak)
}

/// `RotationUtils.reachable(IPlayerContext,BlockPos,double,boolean)`: a rotation that looks at the block and reaches it. Keeps the view if
/// it is already on the block (`remainWithExistingLookDirection`), then tries the middle, then the middle of each face. The order of the
/// faces is upstream's, which has the names of the first two swapped (its "down" is the top).
pub fn reachable<W: WorldView>(env: &Env<'_, W>, pos: Pos, reach: f64, would_sneak: bool) -> Option<Rotation> {
    if env.set.remain_with_existing_look_direction && is_looking_at(env.p, pos) {
        /*
         * why add 0.0001?
         * to indicate that we actually have a desired pitch
         * the way we indicate that the pitch can be whatever and we only care about the yaw
         * is by setting the desired pitch to the current pitch
         * setting the desired pitch to the current pitch + 0.0001 means that we do have a desired pitch, it's
         * just what it currently is
         */
        let hypothetical = env.p.rotations() + Rotation::new(0.0, 0.0001);
        if would_sneak {
            // the concern here is: what if we're looking at it now, but as soon as we start sneaking we no longer are
            let hit = ray_trace_towards(env.ctx, infer_sneaking_eye_position(env.p.x, env.p.y, env.p.z), hypothetical, reach);
            if hit.is_some_and(|h| h.pos == pos) {
                return Some(hypothetical); // yes, if we sneaked we would still be looking at the block
            }
        } else {
            return Some(hypothetical);
        }
    }
    if let Some(r) = reachable_center(env, pos, reach, would_sneak) {
        return Some(r);
    }
    // the shape is the whole block: min 0 and max 1 on each axis, so the offset is `1 - multiplier`
    const BLOCK_SIDE_MULTIPLIERS: [(f64, f64, f64); 6] = [(0.5, 0.0, 0.5), (0.5, 1.0, 0.5), (0.5, 0.5, 0.0), (0.5, 0.5, 1.0), (0.0, 0.5, 0.5), (1.0, 0.5, 0.5)];
    for (mx, my, mz) in BLOCK_SIDE_MULTIPLIERS {
        let x_diff = 0.0 * mx + 1.0 * (1.0 - mx);
        let y_diff = 0.0 * my + 1.0 * (1.0 - my);
        let z_diff = 0.0 * mz + 1.0 * (1.0 - mz);
        let offset = Vec3::new(f64::from(pos.x) + x_diff, f64::from(pos.y) + y_diff, f64::from(pos.z) + z_diff);
        if let Some(r) = reachable_offset(env, pos, offset, reach, would_sneak) {
            return Some(r);
        }
    }
    None
}

/// `MovementHelper.switchToBestToolFor(IPlayerContext,BlockState)`: asks for the slot of the best tool for the block at this position.
pub fn switch_to_best_tool_for<W: WorldView>(env: &Env<'_, W>, st: &mut MoveState, pos: Pos) {
    let id = env.get(pos).id;
    if let Some(slot) = crate::inventory::switch_to_best_tool_for(env.inv, id, &env.set.tool) {
        st.select = Some(slot);
    }
}

/// `MovementHelper.attemptToPlaceABlock`: finds a face to place against and aims at it. `ReadyToPlace` when the crosshair is already on
/// a block whose face is the cell to fill (the caller then presses the right button once it is sneaking, if it has to), `Attempting` when
/// an aim was found, `NoOption` when none was. When there is no block to place, the movement is `Unreachable`.
pub fn attempt_to_place_a_block<W: WorldView>(env: &Env<'_, W>, st: &mut MoveState, place_at: Pos, prefer_down: bool, would_sneak: bool) -> PlaceResult {
    if !env.set.allow_place {
        // Not upstream (docs/deviations.md D28): nothing is placed when `allowPlace` is off. Upstream relies on the cost check.
        st.status = MoveStatus::Unreachable;
        return PlaceResult::NoOption;
    }
    let direct = reachable(env, place_at, env.game.reach, would_sneak); // we assume that if there is a block there, it must be replacable
    let mut found = false;
    if let Some(r) = direct {
        st.target = Some(Target { rotation: r, force: true });
        found = true;
    }
    // north, south, east, west, down
    for dir in [Direction::North, Direction::South, Direction::East, Direction::West, Direction::Down] {
        let against1 = place_at.relative(dir, 1);
        if env.ctx.can_place_against_bsi(against1.x, against1.y, against1.z) {
            let Some(_found) = select_throwaway_at(env, place_at) else {
                // get ready to place a throwaway block: "bb pls get me some blocks. dirt, netherrack, cobble"
                st.status = MoveStatus::Unreachable;
                return PlaceResult::NoOption;
            };
            let face_x = (f64::from(place_at.x) + f64::from(against1.x) + 1.0) * 0.5;
            let face_y = (f64::from(place_at.y) + f64::from(against1.y) + 0.5) * 0.5;
            let face_z = (f64::from(place_at.z) + f64::from(against1.z) + 1.0) * 0.5;
            let place = calc_rotation_from_vec3d(eyes(env, would_sneak), Vec3::new(face_x, face_y, face_z), env.p.rotations());
            let actual = env.game.aim.peek_rotation(env.p.rotations(), place);
            let res = ray_trace_towards(env.ctx, eyes(env, would_sneak), actual, env.game.reach);
            if let Some(h) = res {
                if h.pos == against1 && h.pos.relative(h.face, 1) == place_at {
                    st.target = Some(Target { rotation: place, force: true });
                    found = true;
                    if !prefer_down {
                        // if preferDown is true, we want the last option
                        // if preferDown is false, we want the first
                        break;
                    }
                }
            }
        }
    }
    if let Some(h) = env.p.looking_at {
        let selected_block = h.pos;
        // only way for selectedBlock.equals(placeAt) to be true is if it's replaceable
        if selected_block == place_at || (env.ctx.can_place_against_bsi(selected_block.x, selected_block.y, selected_block.z) && selected_block.relative(h.face, 1) == place_at) {
            if would_sneak {
                st.keys.sneak = true;
            }
            return match select_throwaway_at(env, place_at) {
                Some(found) => {
                    select_found_throwaway(st, found);
                    PlaceResult::ReadyToPlace
                }
                None => no_throwaway(st),
            };
        }
    }
    if found {
        if would_sneak {
            st.keys.sneak = true;
        }
        return match select_throwaway_at(env, place_at) {
            Some(found) => {
                select_found_throwaway(st, found);
                PlaceResult::Attempting
            }
            None => no_throwaway(st),
        };
    }
    PlaceResult::NoOption
}

/// Not upstream (docs/deviations.md D28): with nothing to place, upstream goes on and clicks with whatever is in the hand. Here the movement
/// is unreachable and nothing is clicked, so a bucket of lava or a tool is never used as a block.
fn no_throwaway(st: &mut MoveState) -> PlaceResult {
    st.status = MoveStatus::Unreachable;
    PlaceResult::NoOption
}

/// The four movement keys: the inputs of a `MovementOption`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Move4 {
    Forward,
    Back,
    Left,
    Right,
}

/// `MovementOption`: one or two movement keys, and the motion they give for a facing, as a (x, z) pair.
#[derive(Clone, Copy, Debug)]
struct MovementOption {
    input1: Move4,
    input2: Option<Move4>,
    motion_x: f32,
    motion_z: f32,
}

impl MovementOption {
    /// `MovementOption.setInputs`: presses the one or two keys of the option.
    fn set_inputs(&self, st: &mut MoveState) {
        for i in [Some(self.input1), self.input2].into_iter().flatten() {
            match i {
                Move4::Forward => st.keys.forward = true,
                Move4::Back => st.keys.back = true,
                Move4::Left => st.keys.left = true,
                Move4::Right => st.keys.right = true,
            }
        }
    }

    /// `MovementOption.distanceToSq`: despite the name, a sum of absolute differences.
    fn distance_to_sq(&self, other_x: f32, other_z: f32) -> f32 {
        (self.motion_x - other_x).abs() + (self.motion_z - other_z).abs()
    }

    /// `MovementOption.getOptions`: the eight combinations of keys.
    fn get_options(motion_x: f32, motion_z: f32, can_sprint: bool) -> [MovementOption; 8] {
        const SPRINT_MULTIPLIER: f32 = 1.3;
        let fx = if can_sprint { motion_x * SPRINT_MULTIPLIER } else { motion_x };
        let fz = if can_sprint { motion_z * SPRINT_MULTIPLIER } else { motion_z };
        let o1 = |i: Move4, x: f32, z: f32| MovementOption { input1: i, input2: None, motion_x: x, motion_z: z };
        let o2 = |a: Move4, b: Move4, x: f32, z: f32| MovementOption { input1: a, input2: Some(b), motion_x: x, motion_z: z };
        [
            o1(Move4::Forward, fx, fz),
            o1(Move4::Back, -motion_x, -motion_z),
            o1(Move4::Left, -motion_z, motion_x),
            o1(Move4::Right, motion_z, -motion_x),
            o2(Move4::Forward, Move4::Left, fx - motion_z, fz + motion_x),
            o2(Move4::Forward, Move4::Right, fx + motion_z, fz - motion_x),
            o2(Move4::Back, Move4::Left, -motion_x - motion_z, -motion_z + motion_x),
            o2(Move4::Back, Move4::Right, -motion_x + motion_z, -motion_z - motion_x),
        ]
    }
}

const DEG_TO_RAD_F: f32 = 0.017_453_292;

/// `MovementHelper.moveTowardsWithoutRotation(IPlayerContext,MovementState,float)`: presses the keys whose motion, for the way the player
/// faces now, is closest to the motion towards the ideal yaw. The first of equals wins (`Stream.min`).
pub fn move_towards_without_rotation_yaw<W: WorldView>(env: &Env<'_, W>, st: &mut MoveState, ideal_yaw: f32) {
    let yaw = env.p.rotations().yaw;
    let options = MovementOption::get_options((yaw * DEG_TO_RAD_F).sin(), (yaw * DEG_TO_RAD_F).cos(), env.set.allow_sprint);
    let (ix, iz) = ((ideal_yaw * DEG_TO_RAD_F).sin(), (ideal_yaw * DEG_TO_RAD_F).cos());
    let mut best = options[0];
    for o in &options[1..] {
        if o.distance_to_sq(ix, iz) < best.distance_to_sq(ix, iz) {
            best = *o;
        }
    }
    best.set_inputs(st);
}

/// `MovementHelper.moveTowardsWithoutRotation(IPlayerContext,MovementState,BlockPos)`: the same, towards the middle of a block.
pub fn move_towards_without_rotation<W: WorldView>(env: &Env<'_, W>, st: &mut MoveState, dest: Pos) {
    let ideal_yaw = calc_rotation_from_vec3d(env.p.head(), block_pos_center(dest), env.p.rotations()).yaw;
    move_towards_without_rotation_yaw(env, st, ideal_yaw);
}

/// `MovementHelper.moveTowardsWithSlightRotation`: turns the view to the nearest multiple of 45 degrees around the way to the block, and
/// walks there with whichever keys give that motion.
pub fn move_towards_with_slight_rotation<W: WorldView>(env: &Env<'_, W>, st: &mut MoveState, dest: Pos) {
    let ideal_yaw = calc_rotation_from_vec3d(env.p.head(), block_pos_center(dest), env.p.rotations()).yaw;
    let distance = crate::look::yaw_distance_from_offset(env.p.rotations().yaw, ideal_yaw) % 45.0f32;
    let new_yaw = if distance > 0.0 {
        if distance > 22.5 {
            distance - 45.0
        } else {
            distance
        }
    } else if distance < -22.5 {
        distance + 45.0
    } else {
        distance
    };
    st.target = Some(Target { rotation: Rotation::new(env.p.rotations().yaw - new_yaw, env.p.rotations().pitch), force: true });
    move_towards_without_rotation_yaw(env, st, ideal_yaw);
}
