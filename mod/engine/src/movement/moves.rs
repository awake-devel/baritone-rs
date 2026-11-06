// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/movement/Moves.java
// `apply0` (which builds Movement objects) is left for a later stage.
// LGPL-3.0-or-later, see lib.rs.
use super::helper::*;
use super::{ascend, descend, diagonal, downward, parkour, pillar, traverse};
use crate::blocks::Direction;
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
