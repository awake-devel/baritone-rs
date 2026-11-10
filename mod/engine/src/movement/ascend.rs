// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/movements/MovementAscend.java
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use crate::blocks::Block;
use crate::costs::*;
use crate::world::WorldView;

/// `MovementAscend.cost`: cost of stepping up one block onto the neighbouring column, placing the block to land on if needed.
pub fn cost<W: WorldView>(ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32, dest_x: i32, dest_z: i32) -> f64 {
    let to_place = ctx.get(dest_x, y, dest_z);
    let mut additional_placement_cost = 0.0;
    if !ctx.can_walk_on(dest_x, y, dest_z, to_place) {
        additional_placement_cost = ctx.cost_of_placing_at(dest_x, y, dest_z, to_place);
        if additional_placement_cost >= COST_INF {
            return COST_INF;
        }
        if !ctx.is_replaceable(dest_x, dest_z, &to_place) {
            return COST_INF;
        }
        let mut found_place_option = false;
        // north, south, east, west, down
        for (sx, sy, sz) in [(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)] {
            let against_x = dest_x.wrapping_add(sx);
            let against_y = y.wrapping_add(sy);
            let against_z = dest_z.wrapping_add(sz);
            if against_x == x && against_z == z {
                continue;
            }
            if ctx.can_place_against(against_x, against_y, against_z) {
                found_place_option = true;
                break;
            }
        }
        if !found_place_option {
            return COST_INF;
        }
    }
    let src_up2 = ctx.get(x, y + 2, z);
    if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 1, z) || !src_up2.falling) {
        return COST_INF;
    }
    let src_down = ctx.get(x, y - 1, z);
    if is_climbable(&src_down) {
        return COST_INF;
    }
    let jumping_from_bottom_slab = is_bottom_slab(&src_down);
    let jumping_to_bottom_slab = is_bottom_slab(&to_place);
    if jumping_from_bottom_slab && !jumping_to_bottom_slab {
        return COST_INF;
    }
    let mut walk;
    if jumping_to_bottom_slab {
        if jumping_from_bottom_slab {
            walk = jump_one_block_cost().max(WALK_ONE_BLOCK_COST);
            walk += ctx.cfg.jump_penalty;
        } else {
            walk = WALK_ONE_BLOCK_COST;
        }
    } else {
        if to_place.block == Block::SoulSand {
            walk = WALK_ONE_OVER_SOUL_SAND_COST;
        } else if to_place.block == Block::MagmaBlock {
            walk = SNEAK_ONE_BLOCK_COST;
        } else {
            walk = jump_one_block_cost().max(WALK_ONE_BLOCK_COST);
        }
        walk += ctx.cfg.jump_penalty;
    }
    let mut total_cost = walk + additional_placement_cost;
    total_cost += ctx.mining_duration(x, y + 2, z, src_up2, false);
    if total_cost >= COST_INF {
        return COST_INF;
    }
    total_cost += ctx.mining_duration_at(dest_x, y + 1, dest_z, false);
    if total_cost >= COST_INF {
        return COST_INF;
    }
    total_cost += ctx.mining_duration_at(dest_x, y + 2, dest_z, true);
    total_cost
}
