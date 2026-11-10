// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/schematic/{ISchematic,IStaticSchematic}.java and the enums they use.
// LGPL-3.0-or-later, see lib.rs.
//
// The schematic API: what the builder asks of a schematic, and the schematics that wrap or combine others. Everything here is plain
// Rust over block state ids ([`StateId`], the ids of `blockmeta::BlockIndex` and `WorldView::state_id`). Upstream's `BlockState`
// objects become ids, and the facts about them (properties, rotating, mirroring) come from one shared [`BlockStates`] table.
//
// Where upstream passes a null state (`current` is "the current state of that block in the world, or null"), the id is an `Option`.
pub mod basic;
pub mod format;
pub mod mask;
pub mod states;
pub mod wrap;

pub use basic::{CompositeSchematic, FillSchematic, StaticSchematic};
pub use mask::{BooleanBinaryOperators, CylinderMask, Mask, SphereMask, StaticMask};
pub use states::BlockStates;
pub use wrap::{MaskSchematic, MirroredSchematic, RotatedSchematic, SubstituteSchematic};

/// A block state id: the same ids as the game's state table, `BlockIndex` and `WorldView::state_id`.
pub type StateId = usize;

/// `net.minecraft.world.level.block.Rotation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BlockRotation {
    #[default]
    None,
    Clockwise90,
    Clockwise180,
    Counterclockwise90,
}

impl BlockRotation {
    /// The number of clockwise quarter turns: NONE 0, CLOCKWISE_90 1, CLOCKWISE_180 2, COUNTERCLOCKWISE_90 3.
    fn quarters(self) -> u8 {
        match self {
            BlockRotation::None => 0,
            BlockRotation::Clockwise90 => 1,
            BlockRotation::Clockwise180 => 2,
            BlockRotation::Counterclockwise90 => 3,
        }
    }

    /// Like `Rotation.getRotated(Rotation)`: the quarter turns added, modulo four.
    pub fn get_rotated(self, other: BlockRotation) -> BlockRotation {
        match (self.quarters() + other.quarters()) % 4 {
            0 => BlockRotation::None,
            1 => BlockRotation::Clockwise90,
            2 => BlockRotation::Clockwise180,
            _ => BlockRotation::Counterclockwise90,
        }
    }

    /// `Enum.name`.
    pub fn name(self) -> &'static str {
        match self {
            BlockRotation::None => "NONE",
            BlockRotation::Clockwise90 => "CLOCKWISE_90",
            BlockRotation::Clockwise180 => "CLOCKWISE_180",
            BlockRotation::Counterclockwise90 => "COUNTERCLOCKWISE_90",
        }
    }

    /// Like `Enum.valueOf`: the exact upper-case name.
    pub fn from_name(s: &str) -> Option<BlockRotation> {
        [BlockRotation::None, BlockRotation::Clockwise90, BlockRotation::Clockwise180, BlockRotation::Counterclockwise90].into_iter().find(|r| r.name() == s)
    }
}

/// `net.minecraft.world.level.block.Mirror`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BlockMirror {
    #[default]
    None,
    LeftRight,
    FrontBack,
}

impl BlockMirror {
    /// `Enum.name`.
    pub fn name(self) -> &'static str {
        match self {
            BlockMirror::None => "NONE",
            BlockMirror::LeftRight => "LEFT_RIGHT",
            BlockMirror::FrontBack => "FRONT_BACK",
        }
    }

    /// Like `Enum.valueOf`: the exact upper-case name.
    pub fn from_name(s: &str) -> Option<BlockMirror> {
        [BlockMirror::None, BlockMirror::LeftRight, BlockMirror::FrontBack].into_iter().find(|m| m.name() == s)
    }
}

/// `Direction.Axis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// `ISchematic`: the size of a schematic and the state it wants at a position relative to the origin.
pub trait Schematic: Send + Sync + std::fmt::Debug {
    /// `ISchematic.inSchematic`: does the block at this position matter to the schematic? `current` is the state in the world, if known.
    /// The default is a check against the box (see [`in_bounds`]).
    fn in_schematic(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        let _ = current;
        in_bounds(self, x, y, z)
    }

    /// `ISchematic.size`: the size on an axis.
    /// `ISchematic.size(Direction.Axis)`.
    fn size(&self, axis: Axis) -> i32 {
        match axis {
            Axis::X => self.width_x(),
            Axis::Y => self.height_y(),
            Axis::Z => self.length_z(),
        }
    }

    /// `ISchematic.desiredState`: the state wanted at this position. `approx_placeable` holds the states estimated to be placeable.
    fn desired_state(&self, x: i32, y: i32, z: i32, current: Option<StateId>, approx_placeable: &[StateId]) -> StateId;

    /// `ISchematic.reset`: clears caches, for when the schematic is moved around. It takes `&self`: a cache uses interior mutability,
    /// because a search thread reads the same object.
    fn reset(&self) {}

    /// `ISchematic.widthX`: size on X.
    /// `ISchematic.widthX`.
    fn width_x(&self) -> i32;

    /// `ISchematic.heightY`: size on Y.
    /// `ISchematic.heightY`.
    fn height_y(&self) -> i32;

    /// `ISchematic.lengthZ`: size on Z.
    /// `ISchematic.lengthZ`.
    fn length_z(&self) -> i32;
}

/// `ISchematic.inSchematic`'s default body: the position is inside the box. For the wrappers that call `ISchematic.super.inSchematic`.
pub fn in_bounds<S: Schematic + ?Sized>(s: &S, x: i32, y: i32, z: i32) -> bool {
    x >= 0 && x < s.width_x() && y >= 0 && y < s.height_y() && z >= 0 && z < s.length_z()
}

/// `IStaticSchematic`: a schematic that gives the state at a position with no other context, and does not change.
pub trait StaticSchematicView: Schematic {
    /// `IStaticSchematic.getDirect`: the state of a cell, with no current state.
    /// `IStaticSchematic.getDirect`. `None` is upstream's `IllegalStateException` ("couldn't find schematic for this position"), which
    /// a [`CompositeSchematic`] throws in a gap between its entries.
    fn get_direct(&self, x: i32, y: i32, z: i32) -> Option<StateId>;

    /// `IStaticSchematic.getColumn`: the states of the column, indexed by y. `None` when a position has none.
    fn get_column(&self, x: i32, z: i32) -> Option<Vec<StateId>> {
        (0..self.height_y().max(0)).map(|y| self.get_direct(x, y, z)).collect()
    }
}
