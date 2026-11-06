// baritone-rs engine: the facts a block state gives the rules. Written for this project. LGPL-3.0-or-later, see lib.rs.
//
// The glue builds a table of these, one per block state id, and the rules read nothing else about a block. Only game
// facts live here: Baritone's own rules are derived from them in `movement::helper`.

/// The blocks that upstream's rules compare by identity (`block == Blocks.X`). Everything else is `Other`, and is told
/// apart by the class facts on [`State`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Block {
    #[default]
    Other,
    Air,
    BigDripleaf,
    BubbleColumn,
    Cactus,
    Chest,
    Cobweb,
    Cocoa,
    DirtPath,
    EndPortal,
    EndRod,
    EnderChest,
    Farmland,
    Glass,
    HoneyBlock,
    Ice,
    Ladder,
    LargeFern,
    Lava,
    LilyPad,
    MagmaBlock,
    PointedDripstone,
    PowderSnow,
    Scaffolding,
    SoulSand,
    SweetBerryBush,
    TallGrass,
    TrappedChest,
    Tripwire,
    TwistingVines,
    TwistingVinesPlant,
    Vine,
    Water,
    WeepingVines,
    WeepingVinesPlant,
}

impl Block {
    /// The identity of a block from its registry path, such as `soul_sand`.
    pub fn from_name(name: &str) -> Block {
        match name {
            "air" => Block::Air,
            "big_dripleaf" => Block::BigDripleaf,
            "bubble_column" => Block::BubbleColumn,
            "cactus" => Block::Cactus,
            "chest" => Block::Chest,
            "cobweb" => Block::Cobweb,
            "cocoa" => Block::Cocoa,
            "dirt_path" => Block::DirtPath,
            "end_portal" => Block::EndPortal,
            "end_rod" => Block::EndRod,
            "ender_chest" => Block::EnderChest,
            "farmland" => Block::Farmland,
            "glass" => Block::Glass,
            "honey_block" => Block::HoneyBlock,
            "ice" => Block::Ice,
            "ladder" => Block::Ladder,
            "large_fern" => Block::LargeFern,
            "lava" => Block::Lava,
            "lily_pad" => Block::LilyPad,
            "magma_block" => Block::MagmaBlock,
            "pointed_dripstone" => Block::PointedDripstone,
            "powder_snow" => Block::PowderSnow,
            "scaffolding" => Block::Scaffolding,
            "soul_sand" => Block::SoulSand,
            "sweet_berry_bush" => Block::SweetBerryBush,
            "tall_grass" => Block::TallGrass,
            "trapped_chest" => Block::TrappedChest,
            "tripwire" => Block::Tripwire,
            "twisting_vines" => Block::TwistingVines,
            "twisting_vines_plant" => Block::TwistingVinesPlant,
            "vine" => Block::Vine,
            "water" => Block::Water,
            "weeping_vines" => Block::WeepingVines,
            "weeping_vines_plant" => Block::WeepingVinesPlant,
            _ => Block::Other,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Fluid {
    #[default]
    None,
    Water,
    FlowingWater,
    Lava,
    FlowingLava,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SlabType {
    #[default]
    Bottom,
    Top,
    Double,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Half {
    #[default]
    Bottom,
    Top,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StairShape {
    #[default]
    Straight,
    InnerLeft,
    InnerRight,
    OuterLeft,
    OuterRight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Direction {
    #[default]
    North,
    South,
    East,
    West,
    Up,
    Down,
}

impl Direction {
    /// `Direction.getStepX`.
    pub fn step_x(self) -> i32 {
        match self {
            Direction::East => 1,
            Direction::West => -1,
            _ => 0,
        }
    }
    /// `Direction.getStepY`.
    pub fn step_y(self) -> i32 {
        match self {
            Direction::Up => 1,
            Direction::Down => -1,
            _ => 0,
        }
    }
    /// `Direction.getStepZ`.
    pub fn step_z(self) -> i32 {
        match self {
            Direction::South => 1,
            Direction::North => -1,
            _ => 0,
        }
    }
}

/// One block state. Fields named after a class mean `block instanceof ThatClass`.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// Registry path. Only read when the settings lists are applied, never in a search.
    pub name: String,
    pub block: Block,
    // Classes
    pub air: bool,
    pub amethyst_cluster: bool,
    pub azalea: bool,
    pub bamboo_stalk: bool,
    pub base_fire: bool,
    pub carpet: bool,
    pub cauldron: bool,
    pub door: bool,
    pub falling: bool,
    pub gate: bool,
    pub infested: bool,
    pub leaves: bool,
    pub lily: bool,
    pub liquid_block: bool,
    pub moving_piston: bool,
    pub pointed_dripstone: bool,
    pub scaffolding: bool,
    pub shulker: bool,
    pub skull_any: bool,
    pub skull_floor: bool,
    pub slab: bool,
    pub snow: bool,
    pub stair: bool,
    pub stained_glass: bool,
    pub trapdoor: bool,
    /// `block instanceof EndPortalBlock`.
    pub end_portal_block: bool,
    // State facts
    pub wooden_door: bool,
    pub pathfindable: bool,
    /// `Block.isShapeFullBlock(state.getCollisionShape(null, null))`, false where that throws.
    pub shape_full: bool,
    pub waterlogged: bool,
    /// `state.isFaceSturdy(.., Direction.UP)`.
    pub top_sturdy: bool,
    pub replaceable: bool,
    /// `FallingBlock.isFree(state)`.
    pub falling_free: bool,
    /// `state == FrostedIceBlock.meltsInto()`.
    pub frost_melt: bool,
    // Fluid
    pub fluid: Fluid,
    pub fluid_empty: bool,
    pub fluid_amount: i32,
    pub fluid_source: bool,
    pub fluid_flowing_kind: bool,
    // Properties, each meaningful only for its own class
    pub level: i32,
    pub slab_type: SlabType,
    pub stair_half: Half,
    pub stair_shape: StairShape,
    pub trap_open: bool,
    pub trap_half: Half,
    pub layers: i32,
    pub door_facing: Direction,
    pub door_hinge_left: bool,
}
