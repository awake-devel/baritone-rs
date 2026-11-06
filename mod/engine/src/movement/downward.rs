// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/movements/MovementDownward.java
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use crate::blocks::Block;
use crate::costs::*;
use crate::world::WorldView;

/// `MovementDownward.cost`: cost of digging or dropping one block straight down, or climbing down a ladder or vine.
pub fn cost<W: WorldView>(ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32) -> f64 {
    if !ctx.cfg.allow_downward {
        return COST_INF;
    }
    if !ctx.can_walk_on_at(x, y - 2, z) {
        return COST_INF;
    }
    let down = ctx.get(x, y - 1, z);
    if down.block == Block::Ladder || down.block == Block::Vine {
        LADDER_DOWN_ONE_COST
    } else {
        fall_n_blocks_cost()[1] + ctx.mining_duration(x, y - 1, z, down, false)
    }
}
