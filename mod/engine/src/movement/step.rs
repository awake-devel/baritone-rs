// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/Movement.java and
// src/main/java/baritone/pathing/movement/movements/Movement{Downward,Pillar,Traverse,Ascend,Descend,Fall,Diagonal,Parkour}.java
// Only what the search and the path assembly use: the movement's kind, its two ends and its cost. What to break, what to place
// and how to carry it out is executor work (S4). LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use super::{ascend, descend, diagonal, downward, parkour, pillar, traverse};
use crate::blocks::Direction;
use crate::costs::*;
use crate::pos::Pos;
use crate::world::WorldView;

/// Which `Movement` subclass a movement is. Parkour keeps the arguments of its constructor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MovementKind {
    Downward,
    Pillar,
    Traverse,
    Ascend,
    Descend,
    Fall,
    Diagonal,
    Parkour { direction: Direction, dist: i32, ascend: bool },
}

/// `Movement`: one step of a path. `cost` is upstream's cached cost, `None` until it is calculated or overridden.
#[derive(Clone, Debug, PartialEq)]
pub struct Movement {
    pub kind: MovementKind,
    pub src: Pos,
    pub dest: Pos,
    pub cost: Option<f64>,
    /// Upstream's `calculatedWhileLoaded`: `None` until `check_loaded_chunk` has run (upstream holds a null `Boolean`).
    pub calculated_while_loaded: Option<bool>,
}

impl Movement {
    /// The constructors of the `Movement` subclasses: only the two ends are kept. `toBreak` and `toPlace` wait for the executor.
    pub fn new(kind: MovementKind, src: Pos, dest: Pos) -> Movement {
        Movement { kind, src, dest, cost: None, calculated_while_loaded: None }
    }

    /// `Movement.calculateCost`: the cost the movement has in this world, from the rule of its subclass. The six that read a `MutableMoveResult` answer `COST_INF` when the result does not end at `dest`.
    /// `MovementDownward.calculateCost`: `MovementDownward.cost` at the source.
    /// `MovementPillar.calculateCost`: `MovementPillar.cost` at the source.
    /// `MovementTraverse.calculateCost`: `MovementTraverse.cost` towards `dest`.
    /// `MovementAscend.calculateCost`: `MovementAscend.cost` towards `dest`.
    /// `MovementDescend.calculateCost`: `MovementDescend.cost`, and `COST_INF` when it ends at another height, which means a fall.
    /// `MovementFall.calculateCost`: the same call as the descend, and `COST_INF` when it ends at another height, which means a descend.
    /// `MovementDiagonal.calculateCost`: `MovementDiagonal.cost`, and `COST_INF` when it ends at another height.
    /// `MovementParkour.calculateCost`: `MovementParkour.cost` in the movement's direction, and `COST_INF` when it ends anywhere but `dest`.
    pub fn calculate_cost<W: WorldView>(&self, ctx: &Ctx<'_, W>) -> f64 {
        let (s, d) = (self.src, self.dest);
        match self.kind {
            MovementKind::Downward => downward::cost(ctx, s.x, s.y, s.z),
            MovementKind::Pillar => pillar::cost(ctx, s.x, s.y, s.z),
            MovementKind::Traverse => traverse::cost(ctx, s.x, s.y, s.z, d.x, d.z),
            MovementKind::Ascend => ascend::cost(ctx, s.x, s.y, s.z, d.x, d.z),
            MovementKind::Descend | MovementKind::Fall => {
                let mut result = MoveResult::default();
                descend::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);
                if result.y != d.y {
                    return COST_INF;
                }
                result.cost
            }
            MovementKind::Diagonal => {
                let mut result = MoveResult::default();
                diagonal::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);
                if result.y != d.y {
                    return COST_INF;
                }
                result.cost
            }
            MovementKind::Parkour { direction, .. } => {
                let mut res = MoveResult::default();
                parkour::cost(ctx, s.x, s.y, s.z, direction, &mut res);
                if res.x != d.x || res.y != d.y || res.z != d.z {
                    return COST_INF;
                }
                res.cost
            }
        }
    }

    /// `Movement.getCost(CalculationContext)`: the cached cost, calculated on first use.
    pub fn get_cost<W: WorldView>(&mut self, ctx: &Ctx<'_, W>) -> f64 {
        if let Some(c) = self.cost {
            return c;
        }
        let c = self.calculate_cost(ctx);
        self.cost = Some(c);
        c
    }

    /// `Movement.getCost()`: the cached cost. Upstream unboxes a null `Double` and throws when it was never set; here that is `None`.
    pub fn cost(&self) -> Option<f64> {
        self.cost
    }

    /// `Movement.recalculateCost`: forget the cached cost and calculate it again.
    pub fn recalculate_cost<W: WorldView>(&mut self, ctx: &Ctx<'_, W>) -> f64 {
        self.cost = None;
        self.get_cost(ctx)
    }

    /// `Movement.override`: set the cost by hand.
    pub fn override_cost(&mut self, cost: f64) {
        self.cost = Some(cost);
    }

    /// `Movement.checkLoadedChunk`: whether the destination's column was loaded, as the live world says.
    pub fn check_loaded_chunk<W: WorldView>(&mut self, ctx: &Ctx<'_, W>) {
        self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.dest.x, self.dest.z));
    }

    /// `MovementParkour.cost(CalculationContext,BetterBlockPos,Direction)`: the parkour movement `Moves.apply0` builds. When the jump is impossible the result still holds its reset values, so the distance comes out as `|x| + |z|` of the source, as upstream computes it.
    pub fn parkour_from<W: WorldView>(ctx: &Ctx<'_, W>, src: Pos, direction: Direction) -> Movement {
        let mut res = MoveResult::default();
        parkour::cost(ctx, src.x, src.y, src.z, direction, &mut res);
        let dist = res.x.wrapping_sub(src.x).wrapping_abs().wrapping_add(res.z.wrapping_sub(src.z).wrapping_abs());
        let ascend = res.y > src.y;
        let dest = src.relative(direction, dist).above(if ascend { 1 } else { 0 });
        Movement::new(MovementKind::Parkour { direction, dist, ascend }, src, dest)
    }
}
