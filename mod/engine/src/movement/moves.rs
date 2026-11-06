// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/Moves.java
// `apply0` builds the plain `Movement` of `step.rs`.
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use super::step::{Movement, MovementKind};
use super::{ascend, descend, diagonal, downward, parkour, pillar, traverse};
use crate::blocks::Direction;
use crate::pos::Pos;
use crate::world::WorldView;

/// `Moves`: all possible movements attached to all possible directions they could be taken in, in upstream's ordinal order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Moves {
    Downward,
    Pillar,
    TraverseNorth,
    TraverseSouth,
    TraverseEast,
    TraverseWest,
    AscendNorth,
    AscendSouth,
    AscendEast,
    AscendWest,
    DescendEast,
    DescendWest,
    DescendNorth,
    DescendSouth,
    DiagonalNortheast,
    DiagonalNorthwest,
    DiagonalSoutheast,
    DiagonalSouthwest,
    ParkourNorth,
    ParkourSouth,
    ParkourEast,
    ParkourWest,
}

impl Moves {
    /// The order of `Moves.values()`: every constant in declaration order.
    pub const ALL: [Moves; 22] = [
        Moves::Downward,
        Moves::Pillar,
        Moves::TraverseNorth,
        Moves::TraverseSouth,
        Moves::TraverseEast,
        Moves::TraverseWest,
        Moves::AscendNorth,
        Moves::AscendSouth,
        Moves::AscendEast,
        Moves::AscendWest,
        Moves::DescendEast,
        Moves::DescendWest,
        Moves::DescendNorth,
        Moves::DescendSouth,
        Moves::DiagonalNortheast,
        Moves::DiagonalNorthwest,
        Moves::DiagonalSoutheast,
        Moves::DiagonalSouthwest,
        Moves::ParkourNorth,
        Moves::ParkourSouth,
        Moves::ParkourEast,
        Moves::ParkourWest,
    ];

    /// Java's `Moves.name()`: the upstream constant name.
    pub fn name(self) -> &'static str {
        match self {
            Moves::Downward => "DOWNWARD",
            Moves::Pillar => "PILLAR",
            Moves::TraverseNorth => "TRAVERSE_NORTH",
            Moves::TraverseSouth => "TRAVERSE_SOUTH",
            Moves::TraverseEast => "TRAVERSE_EAST",
            Moves::TraverseWest => "TRAVERSE_WEST",
            Moves::AscendNorth => "ASCEND_NORTH",
            Moves::AscendSouth => "ASCEND_SOUTH",
            Moves::AscendEast => "ASCEND_EAST",
            Moves::AscendWest => "ASCEND_WEST",
            Moves::DescendEast => "DESCEND_EAST",
            Moves::DescendWest => "DESCEND_WEST",
            Moves::DescendNorth => "DESCEND_NORTH",
            Moves::DescendSouth => "DESCEND_SOUTH",
            Moves::DiagonalNortheast => "DIAGONAL_NORTHEAST",
            Moves::DiagonalNorthwest => "DIAGONAL_NORTHWEST",
            Moves::DiagonalSoutheast => "DIAGONAL_SOUTHEAST",
            Moves::DiagonalSouthwest => "DIAGONAL_SOUTHWEST",
            Moves::ParkourNorth => "PARKOUR_NORTH",
            Moves::ParkourSouth => "PARKOUR_SOUTH",
            Moves::ParkourEast => "PARKOUR_EAST",
            Moves::ParkourWest => "PARKOUR_WEST",
        }
    }

    /// The field `Moves.xOffset`: the constructor's x argument.
    pub fn x_offset(self) -> i32 {
        match self {
            Moves::TraverseEast | Moves::AscendEast | Moves::DescendEast | Moves::DiagonalNortheast | Moves::DiagonalSoutheast => 1,
            Moves::TraverseWest | Moves::AscendWest | Moves::DescendWest | Moves::DiagonalNorthwest | Moves::DiagonalSouthwest => -1,
            Moves::ParkourEast => 4,
            Moves::ParkourWest => -4,
            _ => 0,
        }
    }

    /// The field `Moves.yOffset`: the constructor's y argument.
    pub fn y_offset(self) -> i32 {
        match self {
            Moves::Downward | Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth => -1,
            Moves::Pillar | Moves::AscendNorth | Moves::AscendSouth | Moves::AscendEast | Moves::AscendWest => 1,
            _ => 0,
        }
    }

    /// The field `Moves.zOffset`: the constructor's z argument.
    pub fn z_offset(self) -> i32 {
        match self {
            Moves::TraverseNorth | Moves::AscendNorth | Moves::DescendNorth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest => -1,
            Moves::TraverseSouth | Moves::AscendSouth | Moves::DescendSouth | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest => 1,
            Moves::ParkourNorth => -4,
            Moves::ParkourSouth => 4,
            _ => 0,
        }
    }

    /// The field `Moves.dynamicXZ`: true for the parkour moves only.
    pub fn dynamic_xz(self) -> bool {
        matches!(self, Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)
    }

    /// The field `Moves.dynamicY`: true for descend, diagonal and parkour moves.
    pub fn dynamic_y(self) -> bool {
        matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)
    }

    /// `Moves.apply`: the base version fills the result from the fixed offsets and `cost`; the dynamic constants override it and call their movement's cost with the result. Upstream's base version throws `UnsupportedOperationException` for a dynamic constant, but every dynamic constant overrides it, so that branch is unreachable here.
    /// `Moves.DESCEND_EAST.apply`: the override.
    /// `Moves.DESCEND_WEST.apply`: the override.
    /// `Moves.DESCEND_NORTH.apply`: the override.
    /// `Moves.DESCEND_SOUTH.apply`: the override.
    /// `Moves.DIAGONAL_NORTHEAST.apply`: the override.
    /// `Moves.DIAGONAL_NORTHWEST.apply`: the override.
    /// `Moves.DIAGONAL_SOUTHEAST.apply`: the override.
    /// `Moves.DIAGONAL_SOUTHWEST.apply`: the override.
    /// `Moves.PARKOUR_NORTH.apply`: the override.
    /// `Moves.PARKOUR_SOUTH.apply`: the override.
    /// `Moves.PARKOUR_EAST.apply`: the override.
    /// `Moves.PARKOUR_WEST.apply`: the override.
    pub fn apply<W: WorldView>(self, ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32, res: &mut MoveResult) {
        match self {
            Moves::DescendEast => descend::cost(ctx, x, y, z, x.wrapping_add(1), z, res),
            Moves::DescendWest => descend::cost(ctx, x, y, z, x.wrapping_sub(1), z, res),
            Moves::DescendNorth => descend::cost(ctx, x, y, z, x, z.wrapping_sub(1), res),
            Moves::DescendSouth => descend::cost(ctx, x, y, z, x, z.wrapping_add(1), res),
            Moves::DiagonalNortheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_sub(1), res),
            Moves::DiagonalNorthwest => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_sub(1), res),
            Moves::DiagonalSoutheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_add(1), res),
            Moves::DiagonalSouthwest => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_add(1), res),
            Moves::ParkourNorth => parkour::cost(ctx, x, y, z, Direction::North, res),
            Moves::ParkourSouth => parkour::cost(ctx, x, y, z, Direction::South, res),
            Moves::ParkourEast => parkour::cost(ctx, x, y, z, Direction::East, res),
            Moves::ParkourWest => parkour::cost(ctx, x, y, z, Direction::West, res),
            _ => {
                res.x = x.wrapping_add(self.x_offset());
                res.y = y.wrapping_add(self.y_offset());
                res.z = z.wrapping_add(self.z_offset());
                res.cost = self.cost(ctx, x, y, z);
            }
        }
    }

    /// `Moves.cost`: the cost of the fixed-offset movements. Upstream throws `UnsupportedOperationException` for the constants that override `apply` instead, which is a panic here.
    /// `Moves.DOWNWARD.cost`: the override.
    /// `Moves.PILLAR.cost`: the override.
    /// `Moves.TRAVERSE_NORTH.cost`: the override.
    /// `Moves.TRAVERSE_SOUTH.cost`: the override.
    /// `Moves.TRAVERSE_EAST.cost`: the override.
    /// `Moves.TRAVERSE_WEST.cost`: the override.
    /// `Moves.ASCEND_NORTH.cost`: the override.
    /// `Moves.ASCEND_SOUTH.cost`: the override.
    /// `Moves.ASCEND_EAST.cost`: the override.
    /// `Moves.ASCEND_WEST.cost`: the override.
    pub fn cost<W: WorldView>(self, ctx: &Ctx<'_, W>, x: i32, y: i32, z: i32) -> f64 {
        match self {
            Moves::Downward => downward::cost(ctx, x, y, z),
            Moves::Pillar => pillar::cost(ctx, x, y, z),
            Moves::TraverseNorth => traverse::cost(ctx, x, y, z, x, z.wrapping_sub(1)),
            Moves::TraverseSouth => traverse::cost(ctx, x, y, z, x, z.wrapping_add(1)),
            Moves::TraverseEast => traverse::cost(ctx, x, y, z, x.wrapping_add(1), z),
            Moves::TraverseWest => traverse::cost(ctx, x, y, z, x.wrapping_sub(1), z),
            Moves::AscendNorth => ascend::cost(ctx, x, y, z, x, z.wrapping_sub(1)),
            Moves::AscendSouth => ascend::cost(ctx, x, y, z, x, z.wrapping_add(1)),
            Moves::AscendEast => ascend::cost(ctx, x, y, z, x.wrapping_add(1), z),
            Moves::AscendWest => ascend::cost(ctx, x, y, z, x.wrapping_sub(1), z),
            _ => panic!("Movements must override `cost` or `apply`"),
        }
    }
}

impl Moves {
    /// `Moves.apply0`: the `Movement` this move makes from `src`. The fixed moves are built at their offset. The descends and the diagonals run `apply` first and take the end from the result, and the parkour moves take it from the parkour cost. Nothing checks that the movement is possible: when `apply` leaves its result at the reset values, the movement ends at those, as in upstream.
    /// `Moves.DOWNWARD.apply0`: the override.
    /// `Moves.PILLAR.apply0`: the override.
    /// `Moves.TRAVERSE_NORTH.apply0`: the override.
    /// `Moves.TRAVERSE_SOUTH.apply0`: the override.
    /// `Moves.TRAVERSE_EAST.apply0`: the override.
    /// `Moves.TRAVERSE_WEST.apply0`: the override.
    /// `Moves.ASCEND_NORTH.apply0`: the override.
    /// `Moves.ASCEND_SOUTH.apply0`: the override.
    /// `Moves.ASCEND_EAST.apply0`: the override.
    /// `Moves.ASCEND_WEST.apply0`: the override.
    /// `Moves.DESCEND_EAST.apply0`: the override.
    /// `Moves.DESCEND_WEST.apply0`: the override.
    /// `Moves.DESCEND_NORTH.apply0`: the override.
    /// `Moves.DESCEND_SOUTH.apply0`: the override.
    /// `Moves.DIAGONAL_NORTHEAST.apply0`: the override.
    /// `Moves.DIAGONAL_NORTHWEST.apply0`: the override.
    /// `Moves.DIAGONAL_SOUTHEAST.apply0`: the override.
    /// `Moves.DIAGONAL_SOUTHWEST.apply0`: the override.
    /// `Moves.PARKOUR_NORTH.apply0`: the override.
    /// `Moves.PARKOUR_SOUTH.apply0`: the override.
    /// `Moves.PARKOUR_EAST.apply0`: the override.
    /// `Moves.PARKOUR_WEST.apply0`: the override.
    pub fn apply0<W: WorldView>(self, ctx: &Ctx<'_, W>, src: Pos) -> Movement {
        match self {
            Moves::Downward => Movement::new(MovementKind::Downward, src, src.below(1)),
            Moves::Pillar => Movement::new(MovementKind::Pillar, src, src.above(1)),
            Moves::TraverseNorth | Moves::TraverseSouth | Moves::TraverseEast | Moves::TraverseWest => Movement::new(MovementKind::Traverse, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y, src.z.wrapping_add(self.z_offset()))),
            Moves::AscendNorth | Moves::AscendSouth | Moves::AscendEast | Moves::AscendWest => Movement::new(MovementKind::Ascend, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y.wrapping_add(1), src.z.wrapping_add(self.z_offset()))),
            Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth => {
                let mut res = MoveResult::default();
                self.apply(ctx, src.x, src.y, src.z, &mut res);
                let kind = if res.y == src.y.wrapping_sub(1) { MovementKind::Descend } else { MovementKind::Fall };
                Movement::new(kind, src, Pos::new(res.x, res.y, res.z))
            }
            Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest => {
                let mut res = MoveResult::default();
                self.apply(ctx, src.x, src.y, src.z, &mut res);
                let (dir1, dir2) = match self {
                    Moves::DiagonalNortheast => (Direction::North, Direction::East),
                    Moves::DiagonalNorthwest => (Direction::North, Direction::West),
                    Moves::DiagonalSoutheast => (Direction::South, Direction::East),
                    _ => (Direction::South, Direction::West),
                };
                let dy = res.y.wrapping_sub(src.y);
                Movement::new(MovementKind::Diagonal, src, src.relative(dir1, 1).relative(dir2, 1).above(dy))
            }
            Moves::ParkourNorth => Movement::parkour_from(ctx, src, Direction::North),
            Moves::ParkourSouth => Movement::parkour_from(ctx, src, Direction::South),
            Moves::ParkourEast => Movement::parkour_from(ctx, src, Direction::East),
            Moves::ParkourWest => Movement::parkour_from(ctx, src, Direction::West),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::step::MovementKind;
    use crate::testing::*;

    #[test]
    fn apply0_builds_the_fixed_moves_at_their_offsets() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let src = Pos::new(5, 1, 5);
        for m in [Moves::Downward, Moves::Pillar, Moves::TraverseNorth, Moves::TraverseSouth, Moves::TraverseEast, Moves::TraverseWest, Moves::AscendNorth, Moves::AscendSouth, Moves::AscendEast, Moves::AscendWest] {
            let mv = m.apply0(&ctx, src);
            assert_eq!(mv.src, src);
            assert_eq!(mv.dest, Pos::new(5 + m.x_offset(), 1 + m.y_offset(), 5 + m.z_offset()), "{m:?}");
            assert_eq!(mv.cost, None);
            assert_eq!(mv.calculated_while_loaded, None);
        }
        assert_eq!(Moves::TraverseEast.apply0(&ctx, src).kind, MovementKind::Traverse);
        assert_eq!(Moves::AscendWest.apply0(&ctx, src).kind, MovementKind::Ascend);
    }

    #[test]
    fn apply0_descend_or_fall_by_the_result_height() {
        let fx = Fixture::new();
        // a stone block at (5,1,5): feet at y = 2, and the floor east of it is at y = 0, a drop of one
        let mut world = TestWorld::flat(20, 20);
        world.set(5, 1, 5, STONE);
        let ctx = fx.ctx(&world);
        let mv = Moves::DescendEast.apply0(&ctx, Pos::new(5, 2, 5));
        assert_eq!((mv.kind, mv.dest), (MovementKind::Descend, Pos::new(6, 1, 5)));
        let mut r = MoveResult::default();
        Moves::DescendEast.apply(&ctx, 5, 2, 5, &mut r);
        assert_eq!(mv.calculate_cost(&ctx), r.cost);
        assert!(r.cost < crate::costs::COST_INF);
        // a tower of three: a drop of three is a fall
        for y in 2..4 {
            world.set(5, y, 5, STONE);
        }
        let ctx = fx.ctx(&world);
        let mv = Moves::DescendEast.apply0(&ctx, Pos::new(5, 4, 5));
        assert_eq!((mv.kind, mv.dest), (MovementKind::Fall, Pos::new(6, 1, 5)));
        assert!(mv.calculate_cost(&ctx) < crate::costs::COST_INF);
        // the Descend movement to the same column is not valid: the result height differs
        let as_descend = Movement::new(MovementKind::Descend, Pos::new(5, 4, 5), Pos::new(6, 3, 5));
        assert_eq!(as_descend.calculate_cost(&ctx), crate::costs::COST_INF);
    }

    #[test]
    fn apply0_of_a_move_that_fails_ends_at_the_reset_values() {
        // Upstream quirk, kept: nothing checks that the move is possible. On flat ground a descend is impossible, its result stays
        // (0, 0, 0), and the movement ends there: a Descend when 0 is one below the source height, else a Fall.
        let fx = Fixture::new();
        let mut world = TestWorld::flat(20, 20);
        for (x, y) in [(5, 1), (5, 2), (6, 1), (6, 2)] {
            world.set(x, y, 5, STONE);
        }
        let ctx = fx.ctx(&world);
        let mut r0 = MoveResult::default();
        Moves::DescendEast.apply(&ctx, 5, 1, 5, &mut r0);
        assert_eq!(r0.cost, crate::costs::COST_INF);
        let down1 = Moves::DescendEast.apply0(&ctx, Pos::new(5, 1, 5));
        assert_eq!((down1.kind, down1.dest), (MovementKind::Descend, Pos::new(0, 0, 0)));
        let fall = Moves::DescendEast.apply0(&ctx, Pos::new(5, 3, 5));
        assert_eq!((fall.kind, fall.dest), (MovementKind::Fall, Pos::new(0, 0, 0)));
        assert_eq!(fall.calculate_cost(&ctx), crate::costs::COST_INF);
        // a diagonal that fails ends at height 0 (dy = 0 - y) beside the source
        let diag = Moves::DiagonalNortheast.apply0(&ctx, Pos::new(5, 7, 5));
        // an impossible diagonal at y = 7 in air: no floor under it, so check it is the failing case first
        let mut r = MoveResult::default();
        Moves::DiagonalNortheast.apply(&ctx, 5, 7, 5, &mut r);
        assert_eq!(r.cost, crate::costs::COST_INF);
        assert_eq!((diag.kind, diag.dest), (MovementKind::Diagonal, Pos::new(6, 0, 4)));
        // a parkour that fails: |x| + |z| blocks east of the source
        let pk = Moves::ParkourEast.apply0(&ctx, Pos::new(3, 1, 4));
        assert_eq!(pk.kind, MovementKind::Parkour { direction: crate::blocks::Direction::East, dist: 7, ascend: false });
        assert_eq!(pk.dest, Pos::new(10, 1, 4));
    }

    #[test]
    fn apply0_diagonal_takes_the_height_from_the_result() {
        let fx = Fixture::new();
        let mut world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let flat = Moves::DiagonalSoutheast.apply0(&ctx, Pos::new(5, 1, 5));
        assert_eq!((flat.kind, flat.dest), (MovementKind::Diagonal, Pos::new(6, 1, 6)));
        assert!(flat.calculate_cost(&ctx) < crate::costs::COST_INF);
        // with diagonal descents allowed, a one block drop to the north-east gives a destination one lower
        let mut cfg = test_config();
        cfg.allow_diagonal_descend = true;
        let fx2 = Fixture::with_config(cfg);
        world.set(5, 1, 5, STONE);
        let ctx = fx2.ctx(&world);
        let down = Moves::DiagonalNortheast.apply0(&ctx, Pos::new(5, 2, 5));
        assert_eq!((down.kind, down.dest), (MovementKind::Diagonal, Pos::new(6, 1, 4)));
        assert!(down.calculate_cost(&ctx) < crate::costs::COST_INF);
    }

    #[test]
    fn apply0_parkour_from_a_real_jump() {
        let mut cfg = test_config();
        cfg.allow_parkour = true;
        let fx2 = Fixture::with_config(cfg);
        let mut world = TestWorld::flat(20, 20);
        // a gap of two blocks east of x = 5: no floor at x = 6 and 7, floor again at x = 8
        world.set(6, 0, 5, AIR);
        world.set(7, 0, 5, AIR);
        let ctx = fx2.ctx(&world);
        let mv = Moves::ParkourEast.apply0(&ctx, Pos::new(5, 1, 5));
        assert_eq!(mv.kind, MovementKind::Parkour { direction: crate::blocks::Direction::East, dist: 3, ascend: false });
        assert_eq!(mv.dest, Pos::new(8, 1, 5));
        assert!(mv.calculate_cost(&ctx) < crate::costs::COST_INF);
    }

    #[test]
    fn movement_cost_is_cached_until_recalculated() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let mut mv = Moves::TraverseEast.apply0(&ctx, Pos::new(5, 1, 5));
        assert_eq!(mv.cost(), None);
        let c = mv.get_cost(&ctx);
        assert_eq!(mv.cost(), Some(c));
        mv.override_cost(1.25);
        assert_eq!(mv.get_cost(&ctx), 1.25);
        assert_eq!(mv.recalculate_cost(&ctx), c);
        mv.check_loaded_chunk(&ctx);
        assert_eq!(mv.calculated_while_loaded, Some(true));
    }
}
