// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/movements/MovementDescend.java
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use crate::blocks::Block;
use crate::costs::*;
use crate::world::WorldView;

/// The field `CalculationContext.minFallHeight`: fixed at 3 upstream.
const MIN_FALL_HEIGHT: i32 = 3;

/// `MovementDescend.cost`: cost of stepping down, or the start of a longer fall.
pub fn cost<W: WorldView>(ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32, dest_x: i32, dest_z: i32, res: &mut MoveResult) {
    let mut total_cost = 0.0;
    let dest_down = ctx.get(dest_x, y - 1, dest_z);
    total_cost += ctx.mining_duration(dest_x, y - 1, dest_z, dest_down, false);
    if total_cost >= COST_INF {
        return;
    }
    total_cost += ctx.mining_duration_at(dest_x, y, dest_z, false);
    if total_cost >= COST_INF {
        return;
    }
    total_cost += ctx.mining_duration_at(dest_x, y + 1, dest_z, true);
    if total_cost >= COST_INF {
        return;
    }
    let from_down = ctx.get(x, y - 1, z);
    if is_climbable(&from_down) {
        return;
    }
    let below = ctx.get(dest_x, y - 2, dest_z);
    if !ctx.can_walk_on(dest_x, y - 2, dest_z, below) {
        dynamic_fall_cost(ctx, y, dest_x, dest_z, total_cost, below, res);
        return;
    }
    if dest_down.block == Block::Ladder || dest_down.block == Block::Vine {
        return;
    }
    if ctx.can_use_frost_walker(&dest_down) {
        return;
    }
    let mut walk = WALK_OFF_BLOCK_COST;
    if from_down.block == Block::SoulSand {
        walk *= WALK_ONE_OVER_SOUL_SAND_COST / WALK_ONE_BLOCK_COST;
    }
    total_cost += walk + fall_n_blocks_cost()[1].max(CENTER_AFTER_FALL_COST);
    res.x = dest_x;
    res.y = y - 1;
    res.z = dest_z;
    res.cost = total_cost;
}

/// `MovementDescend.dynamicFallCost`: cost of a fall of more than one block. `allowFallIntoLava` is false outside the elytra code.
pub(crate) fn dynamic_fall_cost<W: WorldView>(ctx: &Ctx<'_, W>, y: i32, dest_x: i32, dest_z: i32, front_break: f64, below: Bs<'_>, res: &mut MoveResult) -> bool {
    if front_break != 0.0 && ctx.get(dest_x, y + 2, dest_z).falling {
        return false;
    }
    if !ctx.can_walk_through(dest_x, y - 2, dest_z, below) {
        return false;
    }
    let fall = fall_n_blocks_cost();
    let mut cost_so_far = 0.0;
    let mut effective_start_height = y;
    let mut fall_height = 3;
    loop {
        let new_y = y - fall_height;
        if new_y < ctx.min_y {
            return false;
        }
        let reached_minimum = fall_height >= MIN_FALL_HEIGHT;
        let onto = ctx.get(dest_x, new_y, dest_z);
        let unprotected = fall_height - (y - effective_start_height);
        let tentative = WALK_OFF_BLOCK_COST + fall[unprotected as usize] + front_break + cost_so_far;
        if reached_minimum && is_water(&onto) && !is_waterlogged_solid_top(&onto) {
            if !ctx.can_walk_through(dest_x, new_y, dest_z, onto) {
                return false;
            }
            if ctx.cfg.assume_walk_on_water {
                return false;
            }
            if ctx.is_flowing(dest_x, new_y, dest_z, &onto) {
                return false;
            }
            if !ctx.can_walk_on_at(dest_x, new_y - 1, dest_z) {
                return false;
            }
            res.x = dest_x;
            res.y = new_y;
            res.z = dest_z;
            res.cost = tentative;
            return false;
        }
        // allowFallIntoLava is false outside ElytraBehavior
        if unprotected <= 11 && is_climbable(&onto) {
            cost_so_far += fall[(unprotected - 1) as usize];
            cost_so_far += LADDER_DOWN_ONE_COST;
            effective_start_height = new_y;
            fall_height += 1;
            continue;
        }
        if ctx.can_walk_through(dest_x, new_y, dest_z, onto) {
            fall_height += 1;
            continue;
        }
        if !ctx.can_walk_on(dest_x, new_y, dest_z, onto) {
            return false;
        }
        if is_bottom_slab(&onto) {
            return false;
        }
        if reached_minimum && unprotected <= ctx.cfg.max_fall_no_water + 1 {
            res.x = dest_x;
            res.y = new_y + 1;
            res.z = dest_z;
            res.cost = tentative;
            return false;
        }
        if reached_minimum && ctx.cfg.has_water_bucket && unprotected <= ctx.cfg.max_fall_bucket + 1 {
            res.x = dest_x;
            res.y = new_y + 1;
            res.z = dest_z;
            res.cost = tentative + ctx.cfg.place_penalty;
            return true;
        } else {
            return false;
        }
    }
}
