// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/movements/MovementDiagonal.java
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use crate::blocks::{Block, State};
use crate::costs::*;
use crate::world::WorldView;

/// `MovementDiagonal.cost`: cost of one diagonal step, flat, ascending or descending by one. `res` is left untouched when the movement is impossible.
pub fn cost<W: WorldView>(ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32, dest_x: i32, dest_z: i32, res: &mut MoveResult) {
    let dest_into_upper = ctx.get(dest_x, y + 1, dest_z);
    if !ctx.can_walk_through(dest_x, y + 1, dest_z, dest_into_upper) {
        return;
    }
    let dest_into = ctx.get(dest_x, y, dest_z);
    let from_down;
    let mut ascend = false;
    let dest_walk_on;
    let mut descend = false;
    let mut frost_walker = false;
    let mut sneaking = false;
    if !ctx.can_walk_through(dest_x, y, dest_z, dest_into) {
        ascend = true;
        if !ctx.cfg.allow_diagonal_ascend || !ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into) || !ctx.can_walk_through_at(dest_x, y + 2, dest_z) {
            return;
        }
        dest_walk_on = dest_into;
        from_down = ctx.get(x, y - 1, z);
    } else {
        dest_walk_on = ctx.get(dest_x, y - 1, dest_z);
        from_down = ctx.get(x, y - 1, z);
        let standing_on_a_block = ctx.must_be_solid_to_walk_on(x, y - 1, z, &from_down);
        frost_walker = standing_on_a_block && ctx.can_use_frost_walker(&dest_walk_on);
        if !frost_walker && !ctx.can_walk_on(dest_x, y - 1, dest_z, dest_walk_on) {
            descend = true;
            if !ctx.cfg.allow_diagonal_descend || !ctx.can_walk_on_at(dest_x, y - 2, dest_z) || !ctx.can_walk_through(dest_x, y - 1, dest_z, dest_walk_on) {
                return;
            }
        }
        frost_walker &= !ctx.cfg.assume_walk_on_water; // do this after checking for descends because jesus can't prevent the water from freezing, it just prevents us from relying on the water freezing
    }
    let start_state = ctx.get(x, y, z);
    // upstream passes y as the last argument of the second call, where a z coordinate would be expected
    if is_blocking_door(&start_state, x, z, dest_x, dest_z) || is_blocking_door(if ascend { &dest_into_upper } else { &dest_into }, dest_x, dest_z, x, y) {
        return; // Easier to just traverse instead
    }
    let mut multiplier = WALK_ONE_BLOCK_COST;
    // For either possible soul sand, that affects half of our walking
    if dest_walk_on.block == Block::SoulSand {
        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;
    } else if ctx.cfg.allow_walk_on_magma && dest_walk_on.block == Block::MagmaBlock {
        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;
        sneaking = true;
    } else if frost_walker {
        // frostwalker lets us walk on water without the penalty
    } else if dest_walk_on.block == Block::Water {
        multiplier += ctx.cfg.walk_on_water_penalty * SQRT_2;
    }
    if is_climbable(&from_down) {
        return;
    }
    if from_down.block == Block::SoulSand {
        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;
    } else if ctx.cfg.allow_walk_on_magma && from_down.block == Block::MagmaBlock {
        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;
        sneaking = true;
    }
    let cutting_over1 = ctx.get(x, y - 1, dest_z);
    if (!ctx.cfg.allow_walk_on_magma && cutting_over1.block == Block::MagmaBlock) || is_lava(&cutting_over1) {
        return;
    }
    let cutting_over2 = ctx.get(dest_x, y - 1, z);
    if (!ctx.cfg.allow_walk_on_magma && cutting_over2.block == Block::MagmaBlock) || is_lava(&cutting_over2) {
        return;
    }
    let mut water = false;
    if is_water(&start_state) || is_water(&dest_into) {
        if ascend {
            return;
        }
        // Ignore previous multiplier
        // Whatever we were walking on (possibly soul sand) doesn't matter as we're actually floating on water
        // Not even touching the blocks below
        multiplier = ctx.water_walk_speed;
        water = true;
    }
    let pb0 = ctx.get(x, y, dest_z);
    let pb2 = ctx.get(dest_x, y, z);
    if ascend {
        let a_top = ctx.can_walk_through_at(x, y + 2, dest_z);
        let a_mid = ctx.can_walk_through_at(x, y + 1, dest_z);
        let mut a_low = ctx.can_walk_through(x, y, dest_z, pb0);
        let b_top = ctx.can_walk_through_at(dest_x, y + 2, z);
        let b_mid = ctx.can_walk_through_at(dest_x, y + 1, z);
        let mut b_low = ctx.can_walk_through(dest_x, y, z, pb2);
        a_low &= !is_blocking_door(&pb0, x, dest_z, dest_x, z);
        b_low &= !is_blocking_door(&pb2, dest_x, z, x, dest_z);
        if (!(a_top && a_mid && a_low) && !(b_top && b_mid && b_low)) || avoid_walking_into(&pb0, ctx.cfg) || avoid_walking_into(&pb2, ctx.cfg) || (a_top && a_mid && ctx.can_walk_on(x, y, dest_z, pb0)) || (b_top && b_mid && ctx.can_walk_on(dest_x, y, z, pb2)) || (!a_top && a_mid && a_low) || (!b_top && b_mid && b_low) {
            return;
        }
        res.cost = multiplier * SQRT_2 + jump_one_block_cost() + ctx.cfg.jump_penalty;
        res.x = dest_x;
        res.z = dest_z;
        res.y = y + 1;
        return;
    }
    let mut option_a = ctx.mining_duration(x, y, dest_z, pb0, false);
    let mut option_b = ctx.mining_duration(dest_x, y, z, pb2, false);
    option_a += if is_blocking_door(&pb0, x, dest_z, dest_x, z) { 1.0 } else { 0.0 };
    option_b += if is_blocking_door(&pb2, dest_x, z, x, dest_z) { 1.0 } else { 0.0 };
    if option_a != 0.0 && option_b != 0.0 {
        // check these one at a time -- if pb0 and pb2 were nonzero, we already know that (optionA != 0 && optionB != 0)
        // so no need to check pb1 as well, might as well return early here
        return;
    }
    let pb1 = ctx.get(x, y + 1, dest_z);
    option_a += ctx.mining_duration(x, y + 1, dest_z, pb1, true);
    if option_a != 0.0 && option_b != 0.0 {
        // same deal, if pb1 makes optionA nonzero and option B already was nonzero, pb3 can't affect the result
        return;
    }
    let pb3 = ctx.get(dest_x, y + 1, z);
    if option_a == 0.0 && ((avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) || avoid_walking_into(&pb3, ctx.cfg)) {
        // at this point we're done calculating optionA, so we can check if it's actually possible to edge around in that direction
        return;
    }
    option_b += ctx.mining_duration(dest_x, y + 1, z, pb3, true);
    if option_a != 0.0 && option_b != 0.0 {
        // and finally, if the cost is nonzero for both ways to approach this diagonal, it's not possible
        return;
    }
    if option_b == 0.0 && ((avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) || avoid_walking_into(&pb1, ctx.cfg)) {
        // and now that option B is fully calculated, see if we can edge around that way
        return;
    }
    if option_a != 0.0 || option_b != 0.0 {
        multiplier *= SQRT_2 - 0.001; // TODO tune
        if is_climbable(&start_state) {
            // edging around doesn't work if doing so would climb a ladder or vine instead of moving sideways
            return;
        }
    } else {
        // only can sprint if not edging around
        if ctx.cfg.can_sprint && !water && !sneaking {
            // If we aren't edging around anything, and we aren't in water
            // We can sprint =D
            // Don't check for soul sand, since we can sprint on that too
            multiplier *= SPRINT_MULTIPLIER;
        }
    }
    res.cost = multiplier * SQRT_2;
    if descend {
        res.cost += fall_n_blocks_cost()[1].max(CENTER_AFTER_FALL_COST);
        res.y = y - 1;
    } else {
        res.y = y;
    }
    res.x = dest_x;
    res.z = dest_z;
}

// Same shape as upstream on purpose: the late init stays, so the two can be read side by side.
/// `MovementDiagonal.isBlockingDoor`: whether `other` is the diagonal neighbour of the door in the direction of the door's hinge.
#[allow(clippy::needless_late_init)]
fn is_blocking_door(state: &State, door_x: i32, door_z: i32, other_x: i32, other_z: i32) -> bool {
    if !state.door {
        return false;
    }

    let ox = state.door_facing.step_x();
    let oz = state.door_facing.step_z();

    let nbr_x;
    let nbr_z;
    if state.door_hinge_left {
        nbr_x = door_x.wrapping_sub(ox).wrapping_add(oz);
        nbr_z = door_z.wrapping_sub(oz).wrapping_sub(ox);
    } else {
        nbr_x = door_x.wrapping_sub(ox).wrapping_sub(oz);
        nbr_z = door_z.wrapping_sub(oz).wrapping_add(ox);
    }

    nbr_x == other_x && nbr_z == other_z
}
