// baritone-rs glue: builds the engine's block table from Horizon's own data. Written for this project.
// LGPL-3.0-or-later, see ../engine/src/lib.rs.
//
// This is the one place where Horizon's block data becomes the engine's `State`. `tools/facts-compare` compiles this file
// and compares every field with Java for every block state, so that the mod and the check run the same code.
//
// `mobs::data::MobData` is private to the client crate, so `land` comes in as an argument. The caller must not use its
// fallback table, which guesses from the name and collision: see `land_from_mob_data` in the comparison tool.

use baritone_engine::blocks::{Block, Direction, Fluid, Half, SlabType, StairShape, State};
use horizon_items::blocks::{BlockFacts, StateFacts};
use horizon_world::fluid::FluidKind;
use horizon_world::level::{data::state_flags, Behaviours};
use horizon_world::registry::{BlockRegistry, BlockStateId};

pub struct Sources<'a> {
    pub registry: &'a BlockRegistry,
    pub facts: &'a BlockFacts,
    pub behaviours: &'a Behaviours,
}

fn has(f: &StateFacts, class: &str) -> bool {
    f.has_class(class)
}

/// The engine's facts for one block state. `land` is `isPathfindable(PathComputationType.LAND)`.
pub fn state_from_horizon(src: &Sources<'_>, id: BlockStateId, land: bool) -> State {
    let desc = src.registry.state(id).expect("a state id of this registry");
    let f = src.facts.get(id);
    let raw = src.behaviours.facts(id);
    let block_id = src.behaviours.block(id);
    let prop = |key: &str| desc.properties.get(key).map(String::as_str);
    let on = |key: &str| prop(key) == Some("true");
    let path = desc.name.strip_prefix("minecraft:").unwrap_or(&desc.name);
    let fluid = src.behaviours.fluid(id);

    let door = has(f, "DoorBlock");
    let trapdoor = has(f, "TrapDoorBlock");
    let liquid_block = has(f, "LiquidBlock");
    let in_tag = |tag: &str| src.behaviours.tag(tag).is_some_and(|t| t.contains(block_id));

    State {
        name: path.to_string(),
        block: Block::from_name(path),
        air: has(f, "AirBlock"),
        amethyst_cluster: has(f, "AmethystClusterBlock"),
        azalea: has(f, "AzaleaBlock"),
        bamboo_stalk: has(f, "BambooStalkBlock"),
        base_fire: has(f, "BaseFireBlock"),
        carpet: has(f, "CarpetBlock"),
        cauldron: has(f, "CauldronBlock"),
        door,
        falling: has(f, "FallingBlock"),
        gate: has(f, "FenceGateBlock"),
        infested: has(f, "InfestedBlock"),
        leaves: has(f, "LeavesBlock"),
        lily: has(f, "LilyPadBlock"),
        liquid_block,
        moving_piston: has(f, "MovingPistonBlock"),
        pointed_dripstone: has(f, "PointedDripstoneBlock"),
        scaffolding: has(f, "ScaffoldingBlock"),
        shulker: has(f, "ShulkerBoxBlock"),
        skull_any: has(f, "AbstractSkullBlock"),
        skull_floor: has(f, "SkullBlock"),
        slab: has(f, "SlabBlock"),
        snow: has(f, "SnowLayerBlock"),
        stair: has(f, "StairBlock"),
        stained_glass: has(f, "StainedGlassBlock"),
        trapdoor,
        end_portal_block: has(f, "EndPortalBlock"),
        wooden_door: door && src.behaviours.data(id).set_type.as_ref().is_some_and(|t| t.can_open_by_hand),
        pathfindable: land,
        // Upstream asks with a null level, and treats a throw as false. Every block that throws has a dynamic shape, and
        // Horizon reads those over an empty level, where a shulker box is full. The comparison holds this for all states.
        shape_full: raw.flags & state_flags::FULL_COLLISION != 0 && !src.behaviours.data(id).dynamic_shape,
        waterlogged: on("waterlogged"),
        top_sturdy: src.behaviours.is_face_sturdy(id, horizon_core::geometry::Direction::Up),
        replaceable: raw.flags & state_flags::REPLACEABLE != 0,
        falling_free: raw.flags & (state_flags::AIR | state_flags::LIQUID | state_flags::REPLACEABLE) != 0 || in_tag("minecraft:fire"),
        frost_melt: src.registry.default_state("minecraft:water") == Some(id),
        fluid: match fluid.kind() {
            FluidKind::Empty => Fluid::None,
            FluidKind::Water => Fluid::Water,
            FluidKind::FlowingWater => Fluid::FlowingWater,
            FluidKind::Lava => Fluid::Lava,
            FluidKind::FlowingLava => Fluid::FlowingLava,
        },
        fluid_empty: fluid.kind() == FluidKind::Empty,
        fluid_amount: i32::from(fluid.amount()),
        fluid_source: fluid.is_source(),
        fluid_flowing_kind: fluid.kind() != FluidKind::Empty,
        level: if liquid_block { prop("level").map_or(0, |v| v.parse().unwrap_or(0)) } else { 0 },
        slab_type: match prop("type") {
            Some("top") => SlabType::Top,
            Some("double") => SlabType::Double,
            _ => SlabType::Bottom,
        },
        stair_half: half(prop("half")),
        stair_shape: match prop("shape") {
            Some("inner_left") => StairShape::InnerLeft,
            Some("inner_right") => StairShape::InnerRight,
            Some("outer_left") => StairShape::OuterLeft,
            Some("outer_right") => StairShape::OuterRight,
            _ => StairShape::Straight,
        },
        trap_open: trapdoor && on("open"),
        trap_half: half(prop("half")),
        layers: prop("layers").map_or(0, |v| v.parse().unwrap_or(0)),
        door_facing: match prop("facing") {
            Some("south") => Direction::South,
            Some("east") => Direction::East,
            Some("west") => Direction::West,
            _ => Direction::North,
        },
        door_hinge_left: door && prop("hinge") == Some("left"),
    }
}

fn half(value: Option<&str>) -> Half {
    if value == Some("top") {
        Half::Top
    } else {
        Half::Bottom
    }
}

/// The two tools the check covers. Every other tool waits for the glue that reads the hotbar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    EmptyHand,
    DiamondPickaxe,
}

/// `ToolSet.calculateSpeedVsBlock` without enchantments. It reads the default state of the block, as `getStrVsBlock` does.
pub fn tool_strength(src: &Sources<'_>, id: BlockStateId, tool: Tool) -> f64 {
    let desc = src.registry.state(id).expect("a state id of this registry");
    let default = src.registry.default_state(&desc.name).unwrap_or(id);
    let f = src.facts.get(default);
    if f.hardness < 0.0 {
        return -1.0;
    }
    let block = src.behaviours.block(default);
    let in_tag = |tag: &str| src.behaviours.tag(tag).is_some_and(|t| t.contains(block));
    // A diamond pickaxe's tool component: drops are denied for this tag first, then the mineable tag gives speed and drops.
    let denied = tool == Tool::DiamondPickaxe && in_tag("minecraft:incorrect_for_diamond_tool");
    let mineable = tool == Tool::DiamondPickaxe && in_tag("minecraft:mineable/pickaxe");
    let speed: f32 = if mineable { 8.0 } else { 1.0 } / f.hardness;
    let correct = mineable && !denied;
    f64::from(if !f.requires_tool || correct { speed / 30.0 } else { speed / 100.0 })
}
