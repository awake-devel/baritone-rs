// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/movements/MovementPillar.java
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use crate::blocks::Block;
use crate::costs::*;
use crate::world::WorldView;

/// `MovementPillar.cost`: cost of jumping straight up one block, placing a block under the feet (or climbing a ladder or vine).
pub fn cost<W: WorldView>(ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32) -> f64 {
    let from_state = ctx.get(x, y, z);
    let ladder = is_climbable(&from_state);
    let from_down = ctx.get(x, y - 1, z);
    if !ladder {
        if is_climbable(&from_down) {
            return COST_INF;
        }
        if is_bottom_slab(&from_down) {
            return COST_INF;
        }
    }
    let to_break = ctx.get(x, y + 2, z);
    if to_break.gate {
        return COST_INF;
    }
    let mut src_up = None;
    if is_water(&to_break) && is_water(&from_state) {
        let up = ctx.get(x, y + 1, z);
        src_up = Some(up);
        if is_water(&up) {
            return LADDER_UP_ONE_COST;
        }
    }
    let mut place_cost = 0.0;
    if !ladder {
        place_cost = ctx.cost_of_placing_at(x, z, &from_state);
        if place_cost >= COST_INF {
            return COST_INF;
        }
        if from_down.air {
            place_cost += 0.1;
        }
    }
    if (is_liquid(&from_state) && !ctx.can_place_against(x, y - 1, z)) || (is_liquid(&from_down) && !is_waterlogged_solid_top(&from_down) && ctx.cfg.assume_walk_on_water) {
        return COST_INF;
    }
    if (from_state.block == Block::LilyPad || from_state.carpet) && !from_down.fluid_empty {
        return COST_INF;
    }
    let mut hardness = ctx.mining_duration(x, y + 2, z, to_break, true);
    if hardness >= COST_INF {
        return COST_INF;
    }
    if hardness != 0.0 {
        if is_climbable(&to_break) {
            hardness = 0.0;
        } else {
            let check = ctx.get(x, y + 3, z);
            if check.falling {
                if src_up.is_none() {
                    src_up = Some(ctx.get(x, y + 1, z));
                }
                if !to_break.falling || !src_up.unwrap().falling {
                    return COST_INF;
                }
            }
        }
    }
    if ladder {
        LADDER_UP_ONE_COST + hardness * 5.0
    } else {
        jump_one_block_cost() + place_cost + ctx.cfg.jump_penalty + hardness
    }
}
