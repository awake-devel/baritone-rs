// baritone-rs test tool: a Rust translation of parts of Baritone, compared against the Java original.
// Ported to Rust on 2026-11-05 from cabaletta/baritone at commit 1b8e87cac675d72d8094e2bea1ec7207cdbc971b:
//   src/api/java/baritone/api/pathing/movement/ActionCosts.java
//   src/main/java/baritone/pathing/movement/MovementHelper.java
//   src/main/java/baritone/pathing/movement/CalculationContext.java
//   src/main/java/baritone/pathing/precompute/PrecomputedData.java
//   src/main/java/baritone/pathing/movement/movements/MovementTraverse.java
//   src/main/java/baritone/pathing/movement/movements/MovementDescend.java
//
// This file is part of Baritone.
//
// Baritone is free software: you can redistribute it and/or modify
// it under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// Baritone is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Lesser General Public License for more details.
//
// You should have received a copy of the GNU Lesser General Public License
// along with Baritone.  If not, see <https://www.gnu.org/licenses/>.

use std::sync::OnceLock;

// ---------- ActionCosts ----------
pub const WALK_ONE_BLOCK_COST: f64 = 20.0 / 4.317;
pub const WALK_ONE_IN_WATER_COST: f64 = 20.0 / 2.2;
pub const SWIM_ONE_BLOCK_COST: f64 = 20.0 / 2.86;
pub const WALK_ONE_OVER_SOUL_SAND_COST: f64 = WALK_ONE_BLOCK_COST * 2.0;
pub const LADDER_DOWN_ONE_COST: f64 = 20.0 / 3.0;
pub const SNEAK_ONE_BLOCK_COST: f64 = 20.0 / 1.3;
pub const SPRINT_ONE_BLOCK_COST: f64 = 20.0 / 5.612;
pub const SPRINT_MULTIPLIER: f64 = SPRINT_ONE_BLOCK_COST / WALK_ONE_BLOCK_COST;
pub const WALK_OFF_BLOCK_COST: f64 = WALK_ONE_BLOCK_COST * 0.8;
pub const CENTER_AFTER_FALL_COST: f64 = WALK_ONE_BLOCK_COST - WALK_OFF_BLOCK_COST;
pub const COST_INF: f64 = 1000000.0;

/// `ActionCosts.velocity`: fall speed after the given number of ticks.
fn velocity(ticks: i32) -> f64 {
    (0.98f64.powf(ticks as f64) - 1.0) * -3.92
}

/// `ActionCosts.distanceToTicks`: ticks needed to fall the given distance.
pub fn distance_to_ticks(distance: f64) -> f64 {
    if distance == 0.0 {
        return 0.0;
    }
    let mut tmp_distance = distance;
    let mut tick_count = 0;
    loop {
        let fall_distance = velocity(tick_count);
        if tmp_distance <= fall_distance {
            return tick_count as f64 + tmp_distance / fall_distance;
        }
        tmp_distance -= fall_distance;
        tick_count += 1;
    }
}

/// `ActionCosts.generateFallNBlocksCost`: the 4097-entry fall table (FALL_N_BLOCKS_COST), built once and cached.
pub fn fall_n_blocks_cost() -> &'static [f64] {
    static T: OnceLock<Vec<f64>> = OnceLock::new();
    T.get_or_init(|| (0..4097).map(|i| distance_to_ticks(i as f64)).collect())
}

// ---------- Game facts, the stand-in for Horizon's StateFacts ----------
#[derive(Clone, Debug, Default)]
pub struct State {
    pub block: String,
    pub air: bool,
    pub base_fire: bool,
    pub skull: bool,
    pub skull_block: bool,
    pub shulker: bool,
    pub slab: bool,
    pub trapdoor: bool,
    pub amethyst: bool,
    pub azalea: bool,
    pub door: bool,
    pub wooden_door: bool,
    pub gate: bool,
    pub carpet: bool,
    pub snow: bool,
    pub cauldron: bool,
    pub stair: bool,
    pub stained_glass: bool,
    pub liquid_block: bool,
    pub infested: bool,
    pub lily: bool,
    pub falling: bool,
    pub leaves: bool,
    pub pathfindable: bool,
    pub normal_cube: bool,
    pub waterlogged_solid_top: bool,
    pub replaceable: bool,
    pub falling_free: bool,
    pub frost_melt: bool,
    pub fluid: String,
    pub fluid_empty: bool,
    pub fluid_amount: i32,
    pub fluid_source: bool,
    pub fluid_flowing_kind: bool,
    pub level: i32,
    pub slab_type: String,
    pub stair_half: String,
    pub stair_shape: String,
    pub trap_open: bool,
    pub trap_half: String,
    pub layers: i32,
    pub str_pick: f64,
    pub str_hand: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ternary {
    Yes,
    Maybe,
    No,
}
use Ternary::*;

#[derive(Clone, Debug)]
pub struct Config {
    pub pick: bool,
    pub has_throwaway: bool,
    pub has_water_bucket: bool,
    pub can_sprint: bool,
    pub frost_walker: i32,
    pub water_mult: f32,
    pub allow_break: bool,
    pub assume_walk_on_water: bool,
    pub allow_walk_on_magma: bool,
    pub allow_vines: bool,
    pub allow_walk_on_bottom_slab: bool,
    pub strict_liquid_check: bool,
    pub avoid_updating_falling: bool,
    pub allow_place_source: bool,
    pub allow_place_flow: bool,
    pub allow_swimming: bool,
    pub max_fall_no_water: i32,
    pub max_fall_bucket: i32,
    pub place_penalty: f64,
    pub break_penalty: f64,
    pub walk_on_water_penalty: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Border {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
}

impl Border {
    /// `BetterWorldBorder.canPlaceAt`: one block in from every edge, because a block cannot be placed against the border itself.
    fn can_place_at(&self, x: i32, z: i32) -> bool {
        (x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z
    }
}

pub const AIR_ID: u16 = 0;

pub struct World<'a> {
    pub w: i32,
    pub h: i32,
    pub d: i32,
    pub cells: &'a [u16],
    pub min_y: i32,
}

#[derive(Clone, Copy, Default)]
struct Flags {
    walk_on: Option<bool>,
    walk_through: Option<bool>,
    place_against: bool,
    never_break: bool,
}

// One search-facing context. It holds a settings snapshot and reads nothing global.
pub struct Ctx<'a> {
    pub world: World<'a>,
    pub states: &'a [State],
    pub cfg: &'a Config,
    pub border: Border,
    pub water_walk_speed: f64,
    flags: Vec<Flags>,
}

#[derive(Clone, Copy, Debug)]
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

// ---------- MovementHelper, state half (PrecomputedData.fillData) ----------
/// `MovementHelper.isWater(BlockState)`: water or flowing water.
fn is_water(s: &State) -> bool {
    s.fluid == "water" || s.fluid == "flowing_water"
}
/// `MovementHelper.isLava`: lava or flowing lava.
fn is_lava(s: &State) -> bool {
    s.fluid == "lava" || s.fluid == "flowing_lava"
}
/// `MovementHelper.isClimbable`: ladders and vines.
fn is_climbable(b: &str) -> bool {
    matches!(b, "ladder" | "vine" | "weeping_vines" | "weeping_vines_plant" | "twisting_vines" | "twisting_vines_plant")
}

/// The `blocksToAvoid` setting at its default (tripwire only), read by `MovementHelper.canWalkThrough`. Not a method of upstream.
fn blocks_to_avoid(b: &str) -> bool {
    b == "tripwire"
}

/// `MovementHelper.canWalkThroughBlockState`: the position independent half of canWalkThrough. `isPathfindable` is a fact; `blocksToAvoid` is fixed to tripwire.
pub fn can_walk_through_state(s: &State) -> Ternary {
    let b = s.block.as_str();
    if s.air {
        return Yes;
    }
    if s.base_fire || b == "cobweb" || b == "end_portal" || b == "cocoa" || s.skull || b == "bubble_column" || s.shulker || s.slab || s.trapdoor || b == "honey_block" || b == "end_rod" || b == "sweet_berry_bush" || b == "pointed_dripstone" || s.amethyst || s.azalea {
        return No;
    }
    if b == "big_dripleaf" {
        return No;
    }
    if b == "powder_snow" {
        return No;
    }
    if blocks_to_avoid(b) {
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

/// `MovementHelper.canWalkOnBlockState`: the position independent half of canWalkOn. The `assumeWalkOnLava` branch is not translated (the setting stays off).
pub fn can_walk_on_state(s: &State, c: &Config) -> Ternary {
    let b = s.block.as_str();
    if s.normal_cube && (b != "magma_block" || c.allow_walk_on_magma) && b != "bubble_column" && b != "honey_block" {
        return Yes;
    }
    if s.azalea {
        return Yes;
    }
    if b == "ladder" || (is_climbable(b) && c.allow_vines) {
        return Yes;
    }
    if b == "farmland" || b == "dirt_path" || b == "soul_sand" {
        return Yes;
    }
    if b == "ender_chest" || b == "chest" || b == "trapped_chest" {
        return Yes;
    }
    if b == "glass" || s.stained_glass {
        return Yes;
    }
    if s.stair {
        return Yes;
    }
    if is_water(s) && !s.waterlogged_solid_top {
        return Maybe;
    }
    // assumeWalkOnLava stays at its default of false in this spike
    if s.slab {
        if !c.allow_walk_on_bottom_slab {
            return if s.slab_type != "BOTTOM" { Yes } else { No };
        }
        return Yes;
    }
    No
}

/// `MovementHelper.isBottomSlab`: a slab in its bottom half.
fn is_bottom_slab(s: &State) -> bool {
    s.slab && s.slab_type == "BOTTOM"
}

/// `MovementHelper.possiblyFlowing`: flowing fluid that is not a full source.
fn possibly_flowing(s: &State) -> bool {
    s.fluid_flowing_kind && s.fluid_amount != 8
}

impl<'a> Ctx<'a> {
    /// `PrecomputedData.fillData`: per-state flags, built for all states when the context is made. The fullyPassable flags are not translated: traverse and descend do not use them.
    /// `CalculationContext.<init>(IBaritone,boolean,Level,WorldData,BlockStateInterface,ToolSet,boolean,boolean,boolean,int,float)`: only the `waterWalkSpeed` computation, with float arithmetic as in Java. The other fields come from `Config`.
    pub fn new(world: World<'a>, states: &'a [State], cfg: &'a Config, border: Border) -> Self {
        // `1 - waterSpeedMultiplier` is float arithmetic in Java, and only then widened
        let mult = cfg.water_mult;
        let mut water_speed = WALK_ONE_IN_WATER_COST * ((1.0f32 - mult) as f64) + WALK_ONE_BLOCK_COST * (mult as f64);
        if cfg.allow_swimming && cfg.can_sprint {
            water_speed = water_speed.min(SWIM_ONE_BLOCK_COST);
        }
        let flags = states
            .iter()
            .map(|s| Flags {
                walk_on: match can_walk_on_state(s, cfg) {
                    Yes => Some(true),
                    No => Some(false),
                    Maybe => None,
                },
                walk_through: match can_walk_through_state(s) {
                    Yes => Some(true),
                    No => Some(false),
                    Maybe => None,
                },
                place_against: s.normal_cube || s.block == "glass" || s.stained_glass,
                never_break: s.block == "ice" || s.infested,
            })
            .collect();
        Ctx { world, states, cfg, border, water_walk_speed: water_speed, flags }
    }

    fn id(&self, x: i32, y: i32, z: i32) -> usize {
        let w = &self.world;
        let y = y - w.min_y;
        if y < 0 || y >= w.h || x < 0 || x >= w.w || z < 0 || z >= w.d {
            return AIR_ID as usize;
        }
        w.cells[((y * w.d + z) * w.w + x) as usize] as usize
    }

    /// `CalculationContext.get(int,int,int)`: the state at a position. Outside the world it is air (a redesign of `BlockStateInterface.get0`).
    pub fn get(&self, x: i32, y: i32, z: i32) -> &'a State {
        &self.states[self.id(x, y, z)]
    }

    /// `MovementHelper.isFlowing`: flowing, or next to a flowing source.
    fn is_flowing(&self, x: i32, y: i32, z: i32, s: &State) -> bool {
        if !s.fluid_flowing_kind {
            return false;
        }
        if s.fluid_amount != 8 {
            return true;
        }
        possibly_flowing(self.get(x + 1, y, z)) || possibly_flowing(self.get(x - 1, y, z)) || possibly_flowing(self.get(x, y, z + 1)) || possibly_flowing(self.get(x, y, z - 1))
    }

    /// `MovementHelper.canWalkThroughPosition`: the position dependent half of canWalkThrough. `worldContainsLoadedChunk` is always true here.
    fn can_walk_through_position(&self, x: i32, y: i32, z: i32, s: &State) -> bool {
        if s.carpet {
            return self.can_walk_on_at(x, y - 1, z);
        }
        if s.snow {
            // worldContainsLoadedChunk is always true in the harness
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
            return s.fluid == "water" || s.fluid == "flowing_water";
        }
        s.pathfindable
    }

    /// `MovementHelper.canWalkOnPosition`: the position dependent half of canWalkOn. The `assumeWalkOnLava` branch is not translated.
    fn can_walk_on_position(&self, x: i32, y: i32, z: i32, s: &State) -> bool {
        if is_water(s) {
            let up = self.get(x, y + 1, z);
            if up.block == "lily_pad" || up.carpet {
                return true;
            }
            if self.is_flowing(x, y, z, s) || up.fluid == "flowing_water" {
                return is_water(up) && !self.cfg.assume_walk_on_water;
            }
            return is_water(up) ^ self.cfg.assume_walk_on_water;
        }
        false
    }

    /// `MovementHelper.canWalkThrough(CalculationContext,int,int,int,BlockState)`: state flags first, then the position check if the state says maybe.
    /// `PrecomputedData.canWalkThrough`: same lookup.
    pub fn can_walk_through(&self, x: i32, y: i32, z: i32, s: &State, id: usize) -> bool {
        match self.flags[id].walk_through {
            Some(v) => v,
            None => self.can_walk_through_position(x, y, z, s),
        }
    }

    /// `MovementHelper.canWalkOn(CalculationContext,int,int,int,BlockState)`: state flags first, then the position check if the state says maybe.
    /// `PrecomputedData.canWalkOn`: same lookup.
    pub fn can_walk_on(&self, x: i32, y: i32, z: i32, s: &State, id: usize) -> bool {
        match self.flags[id].walk_on {
            Some(v) => v,
            None => self.can_walk_on_position(x, y, z, s),
        }
    }

    /// `MovementHelper.canWalkOn(CalculationContext,int,int,int)`: reads the state at the position, then asks canWalkOn.
    fn can_walk_on_at(&self, x: i32, y: i32, z: i32) -> bool {
        let id = self.id(x, y, z);
        self.can_walk_on(x, y, z, &self.states[id], id)
    }

    /// `MovementHelper.canPlaceAgainst(CalculationContext,int,int,int)`: world border check, then the state flag.
    /// `PrecomputedData.canPlaceAgainst`: the block half: normal cube, glass or stained glass.
    fn can_place_against(&self, x: i32, y: i32, z: i32) -> bool {
        self.border.can_place_at(x, z) && self.flags[self.id(x, y, z)].place_against
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

    /// `MovementHelper.avoidBreaking(CalculationContext,int,int,int,BlockState)`: border, never-break flag, then the five neighbours.
    /// `PrecomputedData.neverBreak`: the flag lookup. `blocksToDisallowBreaking` stays empty.
    fn avoid_breaking(&self, x: i32, y: i32, z: i32, id: usize) -> bool {
        !self.border.can_place_at(x, z) || self.flags[id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)
    }

    /// `CalculationContext.breakCostMultiplierAt`: `allowBreakAnyway` stays empty; `isPossiblyProtected` is always false upstream.
    fn break_cost_multiplier(&self) -> f64 {
        // allowBreakAnyway stays empty in the spike
        if !self.cfg.allow_break {
            return COST_INF;
        }
        1.0
    }

    /// `MovementHelper.getMiningDurationTicks0`: uncached mining time. Tool strength is a fact (`str_pick` or `str_hand`), not a ToolSet call; upstream's memo cache is not translated.
    pub fn mining_duration(&self, x: i32, y: i32, z: i32, include_falling: bool) -> f64 {
        let id = self.id(x, y, z);
        let s = &self.states[id];
        if !self.can_walk_through(x, y, z, s, id) {
            if !s.fluid_empty {
                return COST_INF;
            }
            let mult = self.break_cost_multiplier();
            if mult >= COST_INF {
                return COST_INF;
            }
            if self.avoid_breaking(x, y, z, id) {
                return COST_INF;
            }
            let str_vs_block = if self.cfg.pick { s.str_pick } else { s.str_hand };
            if str_vs_block <= 0.0 {
                return COST_INF;
            }
            let mut result = 1.0 / str_vs_block;
            result += self.cfg.break_penalty;
            result *= mult;
            if include_falling {
                let above = self.get(x, y + 1, z);
                if above.falling {
                    result += self.mining_duration(x, y + 1, z, true);
                }
            }
            return result;
        }
        0.0
    }

    /// `MovementHelper.canUseFrostWalker(CalculationContext,BlockState)`: frost walker level, a melting frosted ice state, source level 0.
    fn can_use_frost_walker(&self, s: &State) -> bool {
        self.cfg.frost_walker != 0 && s.frost_melt && s.level == 0
    }

    /// `MovementHelper.mustBeSolidToWalkOn`: blocks that count as ground for frost walking.
    fn must_be_solid_to_walk_on(&self, x: i32, y: i32, z: i32, s: &State) -> bool {
        if is_climbable(&s.block) {
            return false;
        }
        if !s.fluid_empty {
            if s.slab {
                if s.slab_type != "BOTTOM" {
                    return true;
                }
            } else if s.stair {
                if s.stair_half == "TOP" {
                    return true;
                }
                if s.stair_shape == "INNER_LEFT" || s.stair_shape == "INNER_RIGHT" {
                    return true;
                }
            } else if s.trapdoor {
                if !s.trap_open && s.trap_half == "TOP" {
                    return true;
                }
            } else if s.block == "scaffolding" || s.leaves {
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

    /// `MovementHelper.isReplaceable`: air, one layer of snow, large ferns and tall grass, else the replaceable flag. `worldContainsLoadedChunk` is always true here.
    fn is_replaceable(&self, s: &State) -> bool {
        if s.air {
            return true;
        }
        if s.snow {
            return s.layers == 1;
        }
        if s.block == "large_fern" || s.block == "tall_grass" {
            return true;
        }
        s.replaceable
    }

    /// `CalculationContext.costOfPlacingAt`: `isPossiblyProtected` is always false upstream.
    fn cost_of_placing_at(&self, x: i32, z: i32, current: &State) -> f64 {
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

    // ---------- MovementTraverse.cost ----------
    /// `MovementTraverse.cost`: cost of one step to a neighbouring block.
    pub fn traverse_cost(&self, x: i32, y: i32, z: i32, dest_x: i32, dest_z: i32) -> f64 {
        let pb0 = self.get(dest_x, y + 1, dest_z);
        let pb1 = self.get(dest_x, y, dest_z);
        let dest_on_id = self.id(dest_x, y - 1, dest_z);
        let dest_on = &self.states[dest_on_id];
        let src_down = self.get(x, y - 1, z);
        let src_down_block = src_down.block.as_str();
        let standing_on_a_block = self.must_be_solid_to_walk_on(x, y - 1, z, src_down);
        let frost_walker = standing_on_a_block && !self.cfg.assume_walk_on_water && self.can_use_frost_walker(dest_on);
        if frost_walker || self.can_walk_on(dest_x, y - 1, dest_z, dest_on, dest_on_id) {
            let mut wc = WALK_ONE_BLOCK_COST;
            let mut water = false;
            let mut sneaking = false;
            if is_water(pb0) || is_water(pb1) {
                wc = self.water_walk_speed;
                water = true;
            } else {
                if dest_on.block == "soul_sand" {
                    wc += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;
                } else if frost_walker {
                    // with frostwalker we can walk on water without the penalty
                } else if dest_on.block == "water" {
                    wc += self.cfg.walk_on_water_penalty;
                }
                if src_down_block == "soul_sand" {
                    wc += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;
                } else if self.cfg.allow_walk_on_magma && src_down_block == "magma_block" {
                    sneaking = true;
                    wc += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;
                }
            }
            let mut hardness1 = self.mining_duration(dest_x, y, dest_z, false);
            if hardness1 >= COST_INF {
                return COST_INF;
            }
            let mut hardness2 = self.mining_duration(dest_x, y + 1, dest_z, true);
            if hardness1 == 0.0 && hardness2 == 0.0 {
                if !water && !sneaking && self.cfg.can_sprint {
                    wc *= SPRINT_MULTIPLIER;
                }
                return wc;
            }
            if is_climbable(src_down_block) {
                hardness1 *= 5.0;
                hardness2 *= 5.0;
            }
            wc + hardness1 + hardness2
        } else {
            if is_climbable(src_down_block) {
                return COST_INF;
            }
            if self.is_replaceable(dest_on) {
                let through_water = is_water(pb0) || is_water(pb1);
                if is_water(dest_on) && through_water {
                    return COST_INF;
                }
                let place_cost = self.cost_of_placing_at(dest_x, dest_z, dest_on);
                if place_cost >= COST_INF {
                    return COST_INF;
                }
                let hardness1 = self.mining_duration(dest_x, y, dest_z, false);
                if hardness1 >= COST_INF {
                    return COST_INF;
                }
                let hardness2 = self.mining_duration(dest_x, y + 1, dest_z, true);
                let mut wc = if through_water { self.water_walk_speed } else { WALK_ONE_BLOCK_COST };
                // north, south, east, west, down
                for (sx, sy, sz) in [(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)] {
                    let against_x = dest_x + sx;
                    let against_y = y - 1 + sy;
                    let against_z = dest_z + sz;
                    if against_x == x && against_z == z {
                        continue;
                    }
                    if self.can_place_against(against_x, against_y, against_z) {
                        return wc + place_cost + hardness1 + hardness2;
                    }
                }
                if src_down_block == "soul_sand" || (src_down.slab && src_down.slab_type != "DOUBLE") {
                    return COST_INF;
                }
                if !standing_on_a_block {
                    return COST_INF;
                }
                let block_src = self.get(x, y, z);
                if (block_src.block == "lily_pad" || block_src.carpet) && !src_down.fluid_empty {
                    return COST_INF;
                }
                wc *= SNEAK_ONE_BLOCK_COST / WALK_ONE_BLOCK_COST;
                return wc + place_cost + hardness1 + hardness2;
            }
            COST_INF
        }
    }

    // ---------- MovementDescend.cost ----------
    /// `MovementDescend.cost`: cost of stepping down, or the start of a longer fall.
    pub fn descend_cost(&self, x: i32, y: i32, z: i32, dest_x: i32, dest_z: i32, res: &mut MoveResult) {
        let mut total_cost = 0.0;
        let dest_down = self.get(dest_x, y - 1, dest_z);
        total_cost += self.mining_duration(dest_x, y - 1, dest_z, false);
        if total_cost >= COST_INF {
            return;
        }
        total_cost += self.mining_duration(dest_x, y, dest_z, false);
        if total_cost >= COST_INF {
            return;
        }
        total_cost += self.mining_duration(dest_x, y + 1, dest_z, true);
        if total_cost >= COST_INF {
            return;
        }
        let from_down = self.get(x, y - 1, z).block.as_str();
        if is_climbable(from_down) {
            return;
        }
        let below_id = self.id(dest_x, y - 2, dest_z);
        let below = &self.states[below_id];
        if !self.can_walk_on(dest_x, y - 2, dest_z, below, below_id) {
            self.dynamic_fall_cost(y, dest_x, dest_z, total_cost, below, below_id, res);
            return;
        }
        if dest_down.block == "ladder" || dest_down.block == "vine" {
            return;
        }
        if self.can_use_frost_walker(dest_down) {
            return;
        }
        let mut walk = WALK_OFF_BLOCK_COST;
        if from_down == "soul_sand" {
            walk *= WALK_ONE_OVER_SOUL_SAND_COST / WALK_ONE_BLOCK_COST;
        }
        total_cost += walk + fall_n_blocks_cost()[1].max(CENTER_AFTER_FALL_COST);
        res.x = dest_x;
        res.y = y - 1;
        res.z = dest_z;
        res.cost = total_cost;
    }

    /// `MovementDescend.dynamicFallCost`: cost of a fall of more than one block. `allowFallIntoLava` is false outside the elytra code.
    #[allow(clippy::too_many_arguments)]
    fn dynamic_fall_cost(&self, y: i32, dest_x: i32, dest_z: i32, front_break: f64, below: &State, below_id: usize, res: &mut MoveResult) -> bool {
        if front_break != 0.0 && self.get(dest_x, y + 2, dest_z).falling {
            return false;
        }
        if !self.can_walk_through(dest_x, y - 2, dest_z, below, below_id) {
            return false;
        }
        let fall = fall_n_blocks_cost();
        let mut cost_so_far = 0.0;
        let mut effective_start_height = y;
        let mut fall_height = 3;
        loop {
            let new_y = y - fall_height;
            if new_y < self.world.min_y {
                return false;
            }
            let reached_minimum = fall_height >= 3;
            let onto_id = self.id(dest_x, new_y, dest_z);
            let onto = &self.states[onto_id];
            let unprotected = fall_height - (y - effective_start_height);
            let tentative = WALK_OFF_BLOCK_COST + fall[unprotected as usize] + front_break + cost_so_far;
            if reached_minimum && is_water(onto) && !onto.waterlogged_solid_top {
                if !self.can_walk_through(dest_x, new_y, dest_z, onto, onto_id) {
                    return false;
                }
                if self.cfg.assume_walk_on_water {
                    return false;
                }
                if self.is_flowing(dest_x, new_y, dest_z, onto) {
                    return false;
                }
                if !self.can_walk_on_at(dest_x, new_y - 1, dest_z) {
                    return false;
                }
                res.x = dest_x;
                res.y = new_y;
                res.z = dest_z;
                res.cost = tentative;
                return false;
            }
            // allowFallIntoLava is false outside ElytraBehavior
            if unprotected <= 11 && is_climbable(&onto.block) {
                cost_so_far += fall[(unprotected - 1) as usize];
                cost_so_far += LADDER_DOWN_ONE_COST;
                effective_start_height = new_y;
                fall_height += 1;
                continue;
            }
            if self.can_walk_through(dest_x, new_y, dest_z, onto, onto_id) {
                fall_height += 1;
                continue;
            }
            if !self.can_walk_on(dest_x, new_y, dest_z, onto, onto_id) {
                return false;
            }
            if is_bottom_slab(onto) {
                return false;
            }
            if reached_minimum && unprotected <= self.cfg.max_fall_no_water + 1 {
                res.x = dest_x;
                res.y = new_y + 1;
                res.z = dest_z;
                res.cost = tentative;
                return false;
            }
            if reached_minimum && self.cfg.has_water_bucket && unprotected <= self.cfg.max_fall_bucket + 1 {
                res.x = dest_x;
                res.y = new_y + 1;
                res.z = dest_z;
                res.cost = tentative + self.cfg.place_penalty;
                return true;
            } else {
                return false;
            }
        }
    }
}

/// Not from upstream: exposes `is_lava` to callers outside the crate and silences the unused warning.
pub fn is_lava_state(s: &State) -> bool {
    is_lava(s)
}
