// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/movements/MovementParkour.java
// Only the static cost function and its two helpers; the Movement object (updateState and friends) is a later stage.
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use crate::blocks::{Block, Direction};
use crate::costs::*;
use crate::world::WorldView;

/// The field `Movement.HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP`: the five directions a block can be placed against.
const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::West, Direction::Down];

// Same shape as upstream on purpose: the late init and the indexed loop stay, so the two can be read side by side.
/// `MovementParkour.cost(CalculationContext,int,int,int,Direction,MutableMoveResult)`: a jump of two to four blocks along `dir`, flat, ascending or onto a block placed mid-air. Leaves `res` untouched when impossible.
#[allow(clippy::needless_late_init, clippy::needless_range_loop)]
pub fn cost<W: WorldView>(ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32, dir: Direction, res: &mut MoveResult) {
    if !ctx.cfg.allow_parkour {
        return;
    }
    if !ctx.cfg.allow_jump_at_build_limit && y >= ctx.max_y {
        return;
    }
    let x_diff = dir.step_x();
    let z_diff = dir.step_z();
    if !ctx.fully_passable_at(x.wrapping_add(x_diff), y, z.wrapping_add(z_diff)) {
        // most common case at the top -- the adjacent block isn't air
        return;
    }
    let adj = ctx.get(x.wrapping_add(x_diff), y.wrapping_sub(1), z.wrapping_add(z_diff));
    if ctx.can_walk_on(x.wrapping_add(x_diff), y.wrapping_sub(1), z.wrapping_add(z_diff), adj) {
        // second most common case -- we could just traverse not parkour
        return;
    }
    if avoid_walking_into(&adj, ctx.cfg) && !is_water(&adj) {
        // magma sucks
        return;
    }
    if !ctx.fully_passable_at(x.wrapping_add(x_diff), y.wrapping_add(1), z.wrapping_add(z_diff)) {
        return;
    }
    if !ctx.fully_passable_at(x.wrapping_add(x_diff), y.wrapping_add(2), z.wrapping_add(z_diff)) {
        return;
    }
    if !ctx.fully_passable_at(x, y.wrapping_add(2), z) {
        return;
    }
    let standing_on = ctx.get(x, y.wrapping_sub(1), z);
    if is_climbable(&standing_on) || standing_on.stair || is_bottom_slab(&standing_on) {
        return;
    }
    // we can't jump from (frozen) water with assumeWalkOnWater because we can't be sure it will be frozen
    if ctx.cfg.assume_walk_on_water && !standing_on.fluid_empty && !is_waterlogged_solid_top(&standing_on) {
        return;
    }
    if !ctx.get(x, y, z).fluid_empty {
        // can't jump out of water
        return;
    }
    let max_jump: i32;
    if ctx.cfg.allow_walk_on_magma && standing_on.block == Block::MagmaBlock {
        max_jump = 2;
    } else if standing_on.block == Block::SoulSand {
        max_jump = 2; // 1 block gap
    } else if ctx.cfg.can_sprint {
        max_jump = 4;
    } else {
        max_jump = 3;
    }

    // check parkour jumps from smallest to largest for obstacles/walls and landing positions
    let mut verified_max_jump = 1; // i - 1 (when i = 2)
    let mut i = 2;
    while i <= max_jump {
        let dest_x = x.wrapping_add(x_diff.wrapping_mul(i));
        let dest_z = z.wrapping_add(z_diff.wrapping_mul(i));

        // check head/feet
        if !ctx.fully_passable_at(dest_x, y.wrapping_add(1), dest_z) {
            break;
        }
        if !ctx.fully_passable_at(dest_x, y.wrapping_add(2), dest_z) {
            break;
        }

        // check for ascend landing position
        let dest_into = ctx.get(dest_x, y, dest_z);
        if !ctx.fully_passable(dest_into) {
            if i <= 3 && ctx.cfg.allow_parkour_ascend && ctx.cfg.can_sprint && ctx.can_walk_on(dest_x, y, dest_z, dest_into) && check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y.wrapping_add(1), dest_z.wrapping_add(z_diff)) {
                res.x = dest_x;
                res.y = y.wrapping_add(1);
                res.z = dest_z;
                res.cost = (i as f64) * SPRINT_ONE_BLOCK_COST + ctx.cfg.jump_penalty;
                return;
            }
            break;
        }

        // check for flat landing position
        let landing_on = ctx.get(dest_x, y.wrapping_sub(1), dest_z);
        // farmland needs to be canWalkOn otherwise farm can never work at all, but we want to specifically disallow ending a jump on farmland haha
        // frostwalker works here because we can't jump from possibly unfrozen water
        if (landing_on.block != Block::Farmland && ctx.can_walk_on(dest_x, y.wrapping_sub(1), dest_z, landing_on)) || (ctx.cfg.frost_walker.wrapping_add(2).min(16) >= i && ctx.can_use_frost_walker(&landing_on)) {
            if check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y, dest_z.wrapping_add(z_diff)) {
                res.x = dest_x;
                res.y = y;
                res.z = dest_z;
                res.cost = cost_from_jump_distance(i) + ctx.cfg.jump_penalty;
                return;
            }
            break;
        }

        if !ctx.fully_passable_at(dest_x, y.wrapping_add(3), dest_z) {
            break;
        }

        verified_max_jump = i;
        i += 1;
    }

    // parkour place starts here
    if !ctx.cfg.allow_parkour_place {
        return;
    }
    // check parkour jumps from largest to smallest for positions to place blocks
    let mut i = verified_max_jump;
    while i > 1 {
        let dest_x = x.wrapping_add(i.wrapping_mul(x_diff));
        let dest_z = z.wrapping_add(i.wrapping_mul(z_diff));
        let to_replace = ctx.get(dest_x, y.wrapping_sub(1), dest_z);
        let place_cost = ctx.cost_of_placing_at(dest_x, dest_z, &to_replace);
        if place_cost >= COST_INF {
            i -= 1;
            continue;
        }
        if !ctx.is_replaceable(dest_x, dest_z, &to_replace) {
            i -= 1;
            continue;
        }
        if !check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y, dest_z.wrapping_add(z_diff)) {
            i -= 1;
            continue;
        }
        for j in 0..5 {
            let against_x = dest_x.wrapping_add(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_x());
            let against_y = y.wrapping_sub(1).wrapping_add(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_y());
            let against_z = dest_z.wrapping_add(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_z());
            if against_x == dest_x.wrapping_sub(x_diff) && against_z == dest_z.wrapping_sub(z_diff) {
                // we can't turn around that fast
                continue;
            }
            if ctx.can_place_against(against_x, against_y, against_z) {
                res.x = dest_x;
                res.y = y;
                res.z = dest_z;
                res.cost = cost_from_jump_distance(i) + place_cost + ctx.cfg.jump_penalty;
                return;
            }
        }
        i -= 1;
    }
}

/// `MovementParkour.checkOvershootSafety`: the two blocks walked into after the landing must not be avoidWalkingInto.
fn check_overshoot_safety<W: WorldView>(ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32) -> bool {
    // we're going to walk into these two blocks after the landing of the parkour anyway, so make sure they aren't avoidWalkingInto
    !avoid_walking_into(&ctx.get(x, y, z), ctx.cfg) && !avoid_walking_into(&ctx.get(x, y.wrapping_add(1), z), ctx.cfg)
}

/// `MovementParkour.costFromJumpDistance`: the cost of a jump of 2, 3 or 4 blocks. Upstream throws `IllegalStateException` for any other distance, which is a panic here.
fn cost_from_jump_distance(dist: i32) -> f64 {
    match dist {
        2 => WALK_ONE_BLOCK_COST * 2.0, // IDK LOL
        3 => WALK_ONE_BLOCK_COST * 3.0,
        4 => SPRINT_ONE_BLOCK_COST * 4.0,
        _ => panic!("LOL {}", dist),
    }
}
