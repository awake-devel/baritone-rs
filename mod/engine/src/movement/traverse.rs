// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/movements/MovementTraverse.java
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use crate::blocks::{Block, SlabType};
use crate::costs::*;
use crate::world::WorldView;

/// `MovementTraverse.cost`: cost of one step to a neighbouring block.
pub fn cost<W: WorldView>(ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32, dest_x: i32, dest_z: i32) -> f64 {
    let pb0 = ctx.get(dest_x, y + 1, dest_z);
    let pb1 = ctx.get(dest_x, y, dest_z);
    let dest_on = ctx.get(dest_x, y - 1, dest_z);
    let src_down = ctx.get(x, y - 1, z);
    let standing_on_a_block = ctx.must_be_solid_to_walk_on(x, y - 1, z, &src_down);
    let frost_walker = standing_on_a_block && !ctx.cfg.assume_walk_on_water && ctx.can_use_frost_walker(&dest_on);
    if frost_walker || ctx.can_walk_on(dest_x, y - 1, dest_z, dest_on) {
        let mut wc = WALK_ONE_BLOCK_COST;
        let mut water = false;
        let mut sneaking = false;
        if is_water(&pb0) || is_water(&pb1) {
            wc = ctx.water_walk_speed;
            water = true;
        } else {
            if dest_on.block == Block::SoulSand {
                wc += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;
            } else if frost_walker {
                // with frostwalker we can walk on water without the penalty
            } else if dest_on.block == Block::Water {
                wc += ctx.cfg.walk_on_water_penalty;
            }
            if src_down.block == Block::SoulSand {
                wc += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;
            } else if ctx.cfg.allow_walk_on_magma && src_down.block == Block::MagmaBlock {
                sneaking = true;
                wc += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;
            }
        }
        let mut hardness1 = ctx.mining_duration(dest_x, y, dest_z, pb1, false);
        if hardness1 >= COST_INF {
            return COST_INF;
        }
        let mut hardness2 = ctx.mining_duration(dest_x, y + 1, dest_z, pb0, true);
        if hardness1 == 0.0 && hardness2 == 0.0 {
            if !water && !sneaking && ctx.cfg.can_sprint {
                wc *= SPRINT_MULTIPLIER;
            }
            return wc;
        }
        if is_climbable(&src_down) {
            hardness1 *= 5.0;
            hardness2 *= 5.0;
        }
        wc + hardness1 + hardness2
    } else {
        if is_climbable(&src_down) {
            return COST_INF;
        }
        if ctx.is_replaceable(dest_x, dest_z, &dest_on) {
            let through_water = is_water(&pb0) || is_water(&pb1);
            if is_water(&dest_on) && through_water {
                return COST_INF;
            }
            let place_cost = ctx.cost_of_placing_at(dest_x, y - 1, dest_z, dest_on);
            if place_cost >= COST_INF {
                return COST_INF;
            }
            let hardness1 = ctx.mining_duration(dest_x, y, dest_z, pb1, false);
            if hardness1 >= COST_INF {
                return COST_INF;
            }
            let hardness2 = ctx.mining_duration(dest_x, y + 1, dest_z, pb0, true);
            let mut wc = if through_water { ctx.water_walk_speed } else { WALK_ONE_BLOCK_COST };
            // north, south, east, west, down
            for (sx, sy, sz) in [(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)] {
                let against_x = dest_x + sx;
                let against_y = y - 1 + sy;
                let against_z = dest_z + sz;
                if against_x == x && against_z == z {
                    continue;
                }
                if ctx.can_place_against(against_x, against_y, against_z) {
                    return wc + place_cost + hardness1 + hardness2;
                }
            }
            if src_down.block == Block::SoulSand || (src_down.slab && src_down.slab_type != SlabType::Double) {
                return COST_INF;
            }
            if !standing_on_a_block {
                return COST_INF;
            }
            let block_src = ctx.get(x, y, z);
            if (block_src.block == Block::LilyPad || block_src.carpet) && !src_down.fluid_empty {
                return COST_INF;
            }
            wc *= SNEAK_ONE_BLOCK_COST / WALK_ONE_BLOCK_COST;
            return wc + place_cost + hardness1 + hardness2;
        }
        COST_INF
    }
}
