// baritone-rs engine: how block states and block names meet in the cache. Written for this project. LGPL-3.0-or-later, see lib.rs.
//
// A packed chunk stores block names (the top block of each column, the special blocks), and the rules read state ids. The glue builds one
// [`CacheEnv`] from its block table: the name of each state, the default state of each block, which states are water, flowing, to avoid, plant-like
// (the classes `ChunkPacker.getPathingBlockType` tests), and which block is special.
use crate::blocks::{Block, State};
use crate::movement::helper::{is_bottom_slab, is_water, possibly_flowing};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const F_WATER: u8 = 1;
pub(super) const F_FLOWING: u8 = 2;
pub(super) const F_AVOID: u8 = 4;
pub(super) const F_AIRLIKE: u8 = 8;
pub(super) const F_MAGMA: u8 = 16;
pub(super) const F_AIR: u8 = 32;

/// Names and default states.
#[derive(Debug)]
pub struct CachePalette {
    names: Vec<Arc<str>>,
    by_name: HashMap<Arc<str>, usize>,
    pub air: usize,
    pub water: usize,
    pub lava: usize,
    pub stone: usize,
    pub netherrack: usize,
    pub end_stone: usize,
    pub bedrock: usize,
    pub obsidian: usize,
}

impl CachePalette {
    /// The registry path of a state's block.
    pub fn name_of(&self, state: usize) -> Option<&Arc<str>> {
        self.names.get(state)
    }
    /// Upstream's `BlockUtils.stringToBlockRequired(name).defaultBlockState()`: the default state of a block, or `None` for a name that is not a block.
    pub fn default_state(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Everything the cache needs to know about the block table.
#[derive(Debug)]
pub struct CacheEnv {
    pub palette: CachePalette,
    flags: Vec<u8>,
    /// Per state: 0, or one more than the index into `specials`.
    special_of: Vec<u16>,
    specials: Vec<Arc<str>>,
}

impl CacheEnv {
    /// `states` is the block table, `default_of[i]` the default state of the block of state `i`, `air_like[i]` whether the block is a `TallGrassBlock`,
    /// `DoublePlantBlock` or `FlowerBlock`, and `avoid` is `MovementHelper.avoidWalkingInto` for a state with magma blocks left out (the pack asks the
    /// setting for those).
    pub fn new(states: &[State], default_of: &[usize], air_like: &[bool], avoid: &dyn Fn(&State) -> bool) -> CacheEnv {
        let mut interned: HashMap<&str, Arc<str>> = HashMap::new();
        let mut names = Vec::with_capacity(states.len());
        let mut by_name: HashMap<Arc<str>, usize> = HashMap::new();
        let mut flags = Vec::with_capacity(states.len());
        let mut special_of = Vec::with_capacity(states.len());
        let mut specials: Vec<Arc<str>> = vec![];
        for (i, s) in states.iter().enumerate() {
            let name = Arc::clone(interned.entry(s.name.as_str()).or_insert_with(|| Arc::from(s.name.as_str())));
            by_name.entry(Arc::clone(&name)).or_insert_with(|| default_of.get(i).copied().unwrap_or(i));
            let mut f = 0u8;
            if is_water(s) {
                f |= F_WATER;
            }
            if possibly_flowing(s) {
                f |= F_FLOWING;
            }
            if avoid(s) || is_bottom_slab(s) {
                f |= F_AVOID;
            }
            if s.block == Block::MagmaBlock {
                f |= F_MAGMA;
            }
            if s.air || air_like.get(i).copied().unwrap_or(false) {
                f |= F_AIRLIKE;
            }
            if s.air {
                f |= F_AIR;
            }
            flags.push(f);
            if super::keeps_track_of(&s.name) {
                let at = match specials.iter().position(|n| **n == *name) {
                    Some(p) => p,
                    None => {
                        specials.push(Arc::clone(&name));
                        specials.len() - 1
                    }
                };
                special_of.push(at as u16 + 1);
            } else {
                special_of.push(0);
            }
            names.push(name);
        }
        let find = |n: &str| by_name.get(n).copied();
        let air = find("air").unwrap_or(0);
        let palette = CachePalette { air, water: find("water").unwrap_or(air), lava: find("lava").unwrap_or(air), stone: find("stone").unwrap_or(air), netherrack: find("netherrack").unwrap_or(air), end_stone: find("end_stone").unwrap_or(air), bedrock: find("bedrock").unwrap_or(air), obsidian: find("obsidian").unwrap_or(air), names, by_name };
        CacheEnv { palette, flags, special_of, specials }
    }

    pub(super) fn flags(&self, state: usize) -> u8 {
        self.flags.get(state).copied().unwrap_or(F_AIR | F_AIRLIKE)
    }

    /// The special block (by name) a state belongs to, if `BLOCKS_TO_KEEP_TRACK_OF` holds its block.
    pub(super) fn special(&self, state: usize) -> Option<&Arc<str>> {
        match self.special_of.get(state).copied() {
            Some(n) if n > 0 => self.specials.get(usize::from(n) - 1),
            _ => None,
        }
    }
}

/// A block table of a few blocks for the tests of this module and the ones below it.
#[cfg(test)]
pub mod fixture {
    use super::*;
    use crate::blocks::Fluid;

    pub const AIR: usize = 0;
    pub const STONE: usize = 1;
    pub const WATER: usize = 2;
    pub const FLOWING: usize = 3;
    pub const LAVA: usize = 4;
    pub const CHEST: usize = 5;
    pub const GRASS: usize = 6;
    pub const DIRT: usize = 7;
    pub const BEDROCK: usize = 8;
    pub const OBSIDIAN: usize = 9;
    pub const NETHERRACK: usize = 10;
    pub const END_STONE: usize = 11;
    pub const SPAWNER: usize = 12;
    pub const SLAB: usize = 13;
    pub const CHEST2: usize = 14;

    pub fn states() -> Vec<State> {
        let mk = |name: &str| State { name: name.to_string(), block: Block::from_name(name), ..Default::default() };
        let mut v = vec![mk("air"), mk("stone"), mk("water"), mk("water"), mk("lava"), mk("chest"), mk("short_grass"), mk("dirt"), mk("bedrock"), mk("obsidian"), mk("netherrack"), mk("end_stone"), mk("spawner"), mk("stone_slab"), mk("chest")];
        v[AIR].air = true;
        v[WATER].fluid = Fluid::Water;
        v[WATER].fluid_empty = false;
        v[WATER].fluid_source = true;
        v[WATER].fluid_flowing_kind = true;
        v[WATER].fluid_amount = 8;
        v[FLOWING].fluid = Fluid::FlowingWater;
        v[FLOWING].fluid_empty = false;
        v[FLOWING].fluid_flowing_kind = true;
        v[FLOWING].fluid_amount = 5;
        v[LAVA].fluid = Fluid::Lava;
        v[LAVA].fluid_empty = false;
        v[LAVA].fluid_flowing_kind = true;
        v[LAVA].fluid_amount = 8;
        v[SLAB].slab = true;
        for s in &mut v {
            s.fluid_empty = s.fluid == Fluid::None;
        }
        v
    }

    pub fn env() -> CacheEnv {
        let st = states();
        let mut default_of: Vec<usize> = (0..st.len()).collect();
        default_of[FLOWING] = WATER;
        default_of[CHEST2] = CHEST;
        let mut air_like = vec![false; st.len()];
        air_like[GRASS] = true;
        CacheEnv::new(&st, &default_of, &air_like, &|s: &State| !s.fluid_empty)
    }
}
