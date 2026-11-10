// Ported from cabaletta/baritone at 1b8e87c:
//   src/main/java/baritone/pathing/movement/MovementHelper.java
//   src/main/java/baritone/pathing/movement/CalculationContext.java
//   src/main/java/baritone/pathing/precompute/PrecomputedData.java
//   src/main/java/baritone/utils/pathing/BetterWorldBorder.java
// LGPL-3.0-or-later, see lib.rs.
use crate::blocks::{Block, Fluid, Half, SlabType, StairShape, State};
use crate::config::Config;
use crate::costs::*;
use crate::world::WorldView;
use std::ops::Deref;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ternary {
    Yes,
    Maybe,
    No,
}
use Ternary::*;

#[derive(Clone, Copy, Debug)]
pub struct Border {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
}

impl Border {
    /// `BetterWorldBorder.canPlaceAt`: one block in from every edge, because a block cannot be placed against the border itself.
    pub fn can_place_at(&self, x: i32, z: i32) -> bool {
        (x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z
    }
}

/// `MutableMoveResult`: where a movement ends and what it costs. `MovementX.cost` leaves it untouched when the movement is impossible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveResult {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub cost: f64,
}

impl Default for MoveResult {
    fn default() -> Self {
        MoveResult { x: 0, y: 0, z: 0, cost: COST_INF }
    }
}

// ---------- MovementHelper, position independent rules ----------
/// `MovementHelper.isWater(BlockState)`: water or flowing water.
pub fn is_water(s: &State) -> bool {
    s.fluid == Fluid::Water || s.fluid == Fluid::FlowingWater
}
/// `MovementHelper.isLava`: lava or flowing lava.
pub fn is_lava(s: &State) -> bool {
    s.fluid == Fluid::Lava || s.fluid == Fluid::FlowingLava
}
/// `MovementHelper.isLiquid(BlockState)`: any fluid, including a waterlogged block.
pub fn is_liquid(s: &State) -> bool {
    !s.fluid_empty
}
/// `MovementHelper.isClimbable`: ladders and vines.
pub fn is_climbable(s: &State) -> bool {
    matches!(s.block, Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant)
}
/// `MovementHelper.isBottomSlab`: a slab in its bottom half.
pub fn is_bottom_slab(s: &State) -> bool {
    s.slab && s.slab_type == SlabType::Bottom
}
/// `MovementHelper.possiblyFlowing`: flowing fluid that is not a full source.
pub fn possibly_flowing(s: &State) -> bool {
    s.fluid_flowing_kind && s.fluid_amount != 8
}
/// `MovementHelper.isTransparent`: air, or the water and lava blocks.
pub fn is_transparent(s: &State) -> bool {
    s.air || s.block == Block::Lava || s.block == Block::Water
}
/// `MovementHelper.isWaterloggedSolidTop`: waterlogged, with a top face that can be stood on.
pub fn is_waterlogged_solid_top(s: &State) -> bool {
    s.waterlogged && s.top_sturdy
}
/// `MovementHelper.isBlockNormalCube`: a full collision shape, apart from a few classes.
pub fn is_block_normal_cube(s: &State) -> bool {
    if s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster {
        return false;
    }
    s.shape_full
}

/// `MovementHelper.avoidWalkingInto`: blocks that hurt or slow whoever walks into them.
pub fn avoid_walking_into(s: &State, c: &Config) -> bool {
    !s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn
}

/// `MovementHelper.canWalkThroughBlockState`: the position independent half of canWalkThrough.
pub fn can_walk_through_state(s: &State, c: &Config) -> Ternary {
    let b = s.block;
    if s.air {
        return Yes;
    }
    if s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea {
        return No;
    }
    if b == Block::BigDripleaf {
        return No;
    }
    if b == Block::PowderSnow {
        return No;
    }
    if c.blocks_to_avoid.contains(&s.name) {
        return No;
    }
    if s.door {
        return if s.wooden_door { Yes } else { No };
    }
    if s.gate {
        return Yes;
    }
    if s.carpet {
        return Maybe;
    }
    if s.snow {
        return Maybe;
    }
    if !s.fluid_empty {
        return if s.fluid_amount != 8 { No } else { Maybe };
    }
    if s.cauldron {
        return No;
    }
    if s.pathfindable {
        Yes
    } else {
        No
    }
}

/// `MovementHelper.fullyPassableBlockState`: the position independent half of fullyPassable.
pub fn fully_passable_state(s: &State) -> Ternary {
    let b = s.block;
    if s.air {
        return Yes;
    }
    if s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker {
        return No;
    }
    if s.pathfindable {
        Yes
    } else {
        No
    }
}

/// `MovementHelper.canWalkOnBlockState`: the position independent half of canWalkOn.
pub fn can_walk_on_state(s: &State, c: &Config) -> Ternary {
    let b = s.block;
    if is_block_normal_cube(s) && (b != Block::MagmaBlock || c.allow_walk_on_magma) && b != Block::BubbleColumn && b != Block::HoneyBlock {
        return Yes;
    }
    if s.azalea {
        return Yes;
    }
    if b == Block::Ladder || (is_climbable(s) && c.allow_vines) {
        return Yes;
    }
    if b == Block::Farmland || b == Block::DirtPath || b == Block::SoulSand {
        return Yes;
    }
    if b == Block::EnderChest || b == Block::Chest || b == Block::TrappedChest {
        return Yes;
    }
    if b == Block::Glass || s.stained_glass {
        return Yes;
    }
    if s.stair {
        return Yes;
    }
    if is_water(s) && !is_waterlogged_solid_top(s) {
        return Maybe;
    }
    if is_lava(s) && c.assume_walk_on_lava {
        return Maybe;
    }
    if s.slab {
        if !c.allow_walk_on_bottom_slab {
            return if s.slab_type != SlabType::Bottom { Yes } else { No };
        }
        return Yes;
    }
    No
}

#[derive(Clone, Copy, Default)]
struct Flags {
    walk_on: Option<bool>,
    walk_through: Option<bool>,
    fully_passable: Option<bool>,
    place_against: bool,
    never_break: bool,
}

fn ternary(t: Ternary) -> Option<bool> {
    match t {
        Yes => Some(true),
        No => Some(false),
        Maybe => None,
    }
}

/// Per-state flags, built for all states at once from one settings snapshot and shared read-only.
pub struct Precomputed {
    flags: Vec<Flags>,
    air: usize,
}

impl Precomputed {
    /// `PrecomputedData.fillData`: the flags of every state.
    pub fn new(states: &[State], c: &Config) -> Self {
        let flags = states.iter().map(|s| Flags { walk_on: ternary(can_walk_on_state(s, c)), walk_through: ternary(can_walk_through_state(s, c)), fully_passable: ternary(fully_passable_state(s)), place_against: is_block_normal_cube(s) || s.block == Block::Glass || s.stained_glass, never_break: c.blocks_to_disallow_breaking.contains(&s.name) || s.block == Block::Ice || s.infested }).collect();
        let air = states.iter().position(|s| s.block == Block::Air).expect("the state table has no air");
        Precomputed { flags, air }
    }
    /// The id of plain air in the state table, what the world holds above and below its height.
    pub fn air(&self) -> usize {
        self.air
    }
}

/// A block state as read from the world: what upstream calls `BlockState` in a rule. Reads as a [`State`], and carries its id for the flag table.
#[derive(Clone, Copy)]
pub struct Bs<'a> {
    pub id: usize,
    pub s: &'a State,
}

impl Deref for Bs<'_> {
    type Target = State;
    fn deref(&self) -> &State {
        self.s
    }
}

/// What a process puts over the two costs a context asks of the world: the cost of placing a block at a position and the multiplier of
/// breaking one. Upstream's `BuilderProcess.BuilderCalculationContext` is a subclass of `CalculationContext` that overrides
/// `costOfPlacingAt` and `breakCostMultiplierAt`; here the context stays one type and carries an optional overlay (docs/deviations.md D45).
/// `current` is the id of the block state at the position. A search thread reads the same object, so an overlay holds snapshots only.
pub trait CostOverlay: Send + Sync {
    /// `BuilderProcess.BuilderCalculationContext.costOfPlacingAt`: replaces `CalculationContext.costOfPlacingAt`.
    fn cost_of_placing_at(&self, cfg: &Config, border: &Border, x: i32, y: i32, z: i32, current: usize) -> f64;
    /// `BuilderProcess.BuilderCalculationContext.breakCostMultiplierAt`: replaces `CalculationContext.breakCostMultiplierAt`.
    fn break_cost_multiplier_at(&self, cfg: &Config, x: i32, y: i32, z: i32, current: usize) -> f64;
}

/// `CalculationContext`: one search-facing context. It holds a settings snapshot and reads nothing global.
pub struct Ctx<'a, W: WorldView> {
    pub world: &'a W,
    pub states: &'a [State],
    pub pre: &'a Precomputed,
    pub cfg: &'a Config,
    pub border: Border,
    /// `ToolSet.getStrVsBlock` for each state id, with the best tool the player holds.
    pub tool_strength: &'a [f64],
    pub water_walk_speed: f64,
    pub min_y: i32,
    pub max_y: i32,
    height: i32,
    /// The overlay of the process the context was made for, or none (the plain `CalculationContext`).
    costs: Option<&'a dyn CostOverlay>,
}

impl<'a, W: WorldView> Ctx<'a, W> {
    /// `CalculationContext.<init>(IBaritone,boolean,Level,WorldData,BlockStateInterface,ToolSet,boolean,boolean,boolean,int,float)`: only the `waterWalkSpeed` computation, with float arithmetic as in Java. The other fields come from `Config`.
    pub fn new(world: &'a W, states: &'a [State], pre: &'a Precomputed, cfg: &'a Config, border: Border, tool_strength: &'a [f64]) -> Self {
        // `1 - waterSpeedMultiplier` is float arithmetic in Java, and only then widened
        let mult = cfg.water_mult;
        let mut water_speed = WALK_ONE_IN_WATER_COST * ((1.0f32 - mult) as f64) + WALK_ONE_BLOCK_COST * (mult as f64);
        if cfg.allow_swimming && cfg.can_sprint {
            water_speed = java_min(water_speed, SWIM_ONE_BLOCK_COST);
        }
        let min_y = world.min_y();
        let height = world.height();
        Ctx { world, states, pre, cfg, border, tool_strength, water_walk_speed: water_speed, min_y, max_y: min_y + height - 1, height, costs: None }
    }

    /// The same context with a process's overlay over the placing and breaking costs (`BuilderCalculationContext`).
    pub fn with_costs(mut self, costs: &'a dyn CostOverlay) -> Self {
        self.costs = Some(costs);
        self
    }

    /// `CalculationContext.get(int,int,int)`: the state at a position. Above and below the world it is air, as in `BlockStateInterface.get0`.
    pub fn get(&self, x: i32, y: i32, z: i32) -> Bs<'a> {
        let rel = y - self.min_y;
        let id = if rel < 0 || rel >= self.height { self.pre.air } else { self.world.state_id(x, y, z) };
        Bs { id, s: &self.states[id] }
    }

    /// `MovementHelper.isFlowing`: flowing, or next to a flowing source.
    pub fn is_flowing(&self, x: i32, y: i32, z: i32, s: &State) -> bool {
        if !s.fluid_flowing_kind {
            return false;
        }
        if s.fluid_amount != 8 {
            return true;
        }
        possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1)) || possibly_flowing(&self.get(x, y, z - 1))
    }

    /// `MovementHelper.canWalkThroughPosition`: the position dependent half of canWalkThrough.
    fn can_walk_through_position(&self, x: i32, y: i32, z: i32, s: &State) -> bool {
        if s.carpet {
            return self.can_walk_on_at(x, y - 1, z);
        }
        if s.snow {
            if !self.world.contains_loaded_chunk(x, z) {
                return true;
            }
            if s.layers >= 3 {
                return false;
            }
            return self.can_walk_on_at(x, y - 1, z);
        }
        if !s.fluid_empty {
            if self.is_flowing(x, y, z, s) {
                return false;
            }
            if self.cfg.assume_walk_on_water {
                return false;
            }
            let up = self.get(x, y + 1, z);
            if !up.fluid_empty || up.lily {
                return false;
            }
            return s.fluid == Fluid::Water || s.fluid == Fluid::FlowingWater;
        }
        s.pathfindable
    }

    /// `MovementHelper.canWalkOnPosition`: the position dependent half of canWalkOn.
    fn can_walk_on_position(&self, x: i32, y: i32, z: i32, s: &State) -> bool {
        if is_water(s) {
            let up = self.get(x, y + 1, z);
            if up.block == Block::LilyPad || up.carpet {
                return true;
            }
            if self.is_flowing(x, y, z, s) || up.fluid == Fluid::FlowingWater {
                return is_water(&up) && !self.cfg.assume_walk_on_water;
            }
            return is_water(&up) ^ self.cfg.assume_walk_on_water;
        }
        if is_lava(s) && !self.is_flowing(x, y, z, s) && self.cfg.assume_walk_on_lava {
            return true;
        }
        false
    }

    /// `MovementHelper.canWalkThrough(CalculationContext,int,int,int,BlockState)`: state flags first, then the position check if the state says maybe.
    /// `PrecomputedData.canWalkThrough`: same lookup.
    pub fn can_walk_through(&self, x: i32, y: i32, z: i32, b: Bs<'_>) -> bool {
        match self.pre.flags[b.id].walk_through {
            Some(v) => v,
            None => self.can_walk_through_position(x, y, z, b.s),
        }
    }

    /// `MovementHelper.canWalkThrough(CalculationContext,int,int,int)`: reads the state at the position, then asks canWalkThrough.
    pub fn can_walk_through_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.can_walk_through(x, y, z, self.get(x, y, z))
    }

    /// `MovementHelper.canWalkOn(CalculationContext,int,int,int,BlockState)`: state flags first, then the position check if the state says maybe.
    /// `PrecomputedData.canWalkOn`: same lookup.
    pub fn can_walk_on(&self, x: i32, y: i32, z: i32, b: Bs<'_>) -> bool {
        match self.pre.flags[b.id].walk_on {
            Some(v) => v,
            None => self.can_walk_on_position(x, y, z, b.s),
        }
    }

    /// `MovementHelper.canWalkOn(CalculationContext,int,int,int)`: reads the state at the position, then asks canWalkOn.
    pub fn can_walk_on_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.can_walk_on(x, y, z, self.get(x, y, z))
    }

    /// `MovementHelper.fullyPassable(CalculationContext,int,int,int,BlockState)`: canWalkThrough, but nothing that would impede movement at all.
    /// `PrecomputedData.fullyPassable`: the flag lookup.
    /// `MovementHelper.fullyPassablePosition`: the maybe case, which reads `pathfindable`. `fullyPassableBlockState` never returns maybe today.
    pub fn fully_passable(&self, b: Bs<'_>) -> bool {
        match self.pre.flags[b.id].fully_passable {
            Some(v) => v,
            None => b.s.pathfindable,
        }
    }

    /// `MovementHelper.fullyPassable(CalculationContext,int,int,int)`: reads the state at the position, then asks fullyPassable.
    pub fn fully_passable_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.fully_passable(self.get(x, y, z))
    }

    /// `MovementHelper.canPlaceAgainst(CalculationContext,int,int,int)`: reads the state at the position, then asks canPlaceAgainst.
    /// `MovementHelper.canPlaceAgainst(CalculationContext,int,int,int,BlockState)`: world border check, then the state flag.
    /// `PrecomputedData.canPlaceAgainst`: the block half: normal cube, glass or stained glass.
    pub fn can_place_against(&self, x: i32, y: i32, z: i32) -> bool {
        self.border.can_place_at(x, z) && self.pre.flags[self.get(x, y, z).id].place_against
    }

    /// `MovementHelper.canPlaceAgainst(BlockStateInterface,int,int,int)`: the executor's variant: the world border of the
    /// `BlockStateInterface` (this context's border), then the block half: normal cube, glass or stained glass.
    /// `MovementHelper.canPlaceAgainst(BlockStateInterface,int,int,int,BlockState)`: the same with the state given.
    /// `MovementHelper.canPlaceAgainst(IPlayerContext,BlockPos)`: the same for the player's world.
    pub fn can_place_against_bsi(&self, x: i32, y: i32, z: i32) -> bool {
        self.border.can_place_at(x, z) && self.pre.flags[self.get(x, y, z).id].place_against
    }

    /// `MovementHelper.avoidAdjacentBreaking`: do not break a block next to falling blocks or liquid.
    fn avoid_adjacent_breaking(&self, x: i32, y: i32, z: i32, directly_above: bool) -> bool {
        let s = self.get(x, y, z);
        if !directly_above && s.falling && self.cfg.avoid_updating_falling && self.get(x, y - 1, z).falling_free {
            return true;
        }
        if s.liquid_block {
            if directly_above || self.cfg.strict_liquid_check {
                return true;
            }
            if s.level == 0 {
                return true;
            }
            return !self.get(x, y - 1, z).liquid_block;
        }
        !s.fluid_empty
    }

    /// `PrecomputedData.neverBreak` at a position: the block is in `blocksToDisallowBreaking`, or is ice or infested.
    pub fn never_break_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.pre.flags[self.get(x, y, z).id].never_break
    }

    /// `MovementHelper.avoidBreaking(CalculationContext,int,int,int,BlockState)`: border, never-break flag, then the five neighbours.
    /// `PrecomputedData.neverBreak`: the flag lookup.
    pub fn avoid_breaking(&self, x: i32, y: i32, z: i32, b: Bs<'_>) -> bool {
        !self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)
    }

    /// `CalculationContext.breakCostMultiplierAt`: `isPossiblyProtected` is always false upstream. With an overlay, the overlay's.
    fn break_cost_multiplier(&self, x: i32, y: i32, z: i32, b: Bs<'_>) -> f64 {
        if let Some(o) = self.costs {
            return o.break_cost_multiplier_at(self.cfg, x, y, z, b.id);
        }
        let s = b.s;
        if !self.cfg.allow_break && !self.cfg.allow_break_anyway.contains(&s.name) {
            return COST_INF;
        }
        1.0
    }

    /// `MovementHelper.getMiningDurationTicks0`: uncached mining time.
    /// `MovementHelper.getMiningDurationTicks(CalculationContext,int,int,int,BlockState,boolean)`: upstream's memo in front of it is not translated, and gives the same value for a world that does not change during a search.
    pub fn mining_duration(&self, x: i32, y: i32, z: i32, b: Bs<'_>, include_falling: bool) -> f64 {
        if !self.can_walk_through(x, y, z, b) {
            if !b.fluid_empty {
                return COST_INF;
            }
            let mult = self.break_cost_multiplier(x, y, z, b);
            if mult >= COST_INF {
                return COST_INF;
            }
            if self.avoid_breaking(x, y, z, b) {
                return COST_INF;
            }
            let str_vs_block = self.tool_strength[b.id];
            if str_vs_block <= 0.0 {
                return COST_INF;
            }
            let mut result = 1.0 / str_vs_block;
            result += self.cfg.break_penalty;
            result *= mult;
            if include_falling {
                let above = self.get(x, y + 1, z);
                if above.falling {
                    result += self.mining_duration(x, y + 1, z, above, true);
                }
            }
            return result;
        }
        0.0
    }

    /// `MovementHelper.getMiningDurationTicks(CalculationContext,int,int,int,boolean)`: reads the state at the position, then asks for its mining time.
    pub fn mining_duration_at(&self, x: i32, y: i32, z: i32, include_falling: bool) -> f64 {
        self.mining_duration(x, y, z, self.get(x, y, z), include_falling)
    }

    /// `MovementHelper.canUseFrostWalker(CalculationContext,BlockState)`: frost walker level, a melting frosted ice state, source level 0.
    pub fn can_use_frost_walker(&self, s: &State) -> bool {
        self.cfg.frost_walker != 0 && s.frost_melt && s.level == 0
    }

    /// `MovementHelper.mustBeSolidToWalkOn`: blocks that count as ground for frost walking.
    pub fn must_be_solid_to_walk_on(&self, x: i32, y: i32, z: i32, s: &State) -> bool {
        if is_climbable(s) {
            return false;
        }
        if !s.fluid_empty {
            if s.slab {
                if s.slab_type != SlabType::Bottom {
                    return true;
                }
            } else if s.stair {
                if s.stair_half == Half::Top {
                    return true;
                }
                if s.stair_shape == StairShape::InnerLeft || s.stair_shape == StairShape::InnerRight {
                    return true;
                }
            } else if s.trapdoor {
                if !s.trap_open && s.trap_half == Half::Top {
                    return true;
                }
            } else if s.block == Block::Scaffolding || s.leaves {
                // upstream tests scaffolding and leaves in two branches with the same result
                return true;
            }
            if self.cfg.assume_walk_on_water {
                return false;
            }
            if self.get(x, y + 1, z).liquid_block {
                return false;
            }
        }
        true
    }

    /// `MovementHelper.isReplaceable`: air, one layer of snow, large ferns and tall grass, else the replaceable flag.
    pub fn is_replaceable(&self, x: i32, z: i32, s: &State) -> bool {
        if s.air {
            return true;
        }
        if s.snow {
            if !self.world.contains_loaded_chunk(x, z) {
                return true;
            }
            return s.layers == 1;
        }
        if s.block == Block::LargeFern || s.block == Block::TallGrass {
            return true;
        }
        s.replaceable
    }

    /// `CalculationContext.costOfPlacingAt`: `isPossiblyProtected` is always false upstream. With an overlay, the overlay's.
    pub fn cost_of_placing_at(&self, x: i32, y: i32, z: i32, current: Bs<'_>) -> f64 {
        if let Some(o) = self.costs {
            return o.cost_of_placing_at(self.cfg, &self.border, x, y, z, current.id);
        }
        let current = current.s;
        if !self.cfg.has_throwaway {
            return COST_INF;
        }
        if !self.border.can_place_at(x, z) {
            return COST_INF;
        }
        if !self.cfg.allow_place_source && current.fluid_source {
            return COST_INF;
        }
        if !self.cfg.allow_place_flow && !current.fluid_empty && !current.fluid_source {
            return COST_INF;
        }
        self.cfg.place_penalty
    }
}

impl Border {
    /// `BetterWorldBorder.entirelyContains`: whether the block column lies inside the border. `x + 1` is `int` arithmetic before it becomes a double.
    pub fn entirely_contains(&self, x: i32, z: i32) -> bool {
        (x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z
    }
}
