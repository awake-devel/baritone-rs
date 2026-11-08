// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/cache/CachedChunk.java and ChunkPacker.java. LGPL-3.0-or-later, see lib.rs.
//
// A packed chunk: 2 bits per block (`PathingBlockType`), the top block of each column, the special blocks, a heightmap. The index of a block in the
// bits is `x << 1 | z << 5 | y << 9` with `y` counted from the dimension's lowest Y, the bit order of Java's `BitSet`, so the bytes are upstream's.
use super::palette::{CacheEnv, F_AIR, F_AIRLIKE, F_AVOID, F_FLOWING, F_MAGMA, F_WATER};
use crate::pos::Pos;
use std::collections::HashMap;
use std::sync::Arc;

/// `PathingBlockType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathingBlockType {
    Air,
    Water,
    Avoid,
    Solid,
}

impl PathingBlockType {
    /// `getBits`: the two bits, the high one first.
    pub fn bits(self) -> [bool; 2] {
        match self {
            PathingBlockType::Air => [false, false],
            PathingBlockType::Water => [false, true],
            PathingBlockType::Avoid => [true, false],
            PathingBlockType::Solid => [true, true],
        }
    }
    /// `fromBits`.
    pub fn from_bits(b1: bool, b2: bool) -> PathingBlockType {
        match (b1, b2) {
            (true, true) => PathingBlockType::Solid,
            (true, false) => PathingBlockType::Avoid,
            (false, true) => PathingBlockType::Water,
            (false, false) => PathingBlockType::Air,
        }
    }
}

/// What `DimensionType` says that the cache reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimension {
    /// `minY()`.
    pub min_y: i32,
    /// `height()`: a multiple of 16.
    pub height: i32,
    /// `logicalHeight()`.
    pub logical_height: i32,
    pub natural: bool,
    pub ultra_warm: bool,
    pub has_ceiling: bool,
    /// `effectsLocation` is the End's.
    pub end_effects: bool,
}

impl Dimension {
    /// The three vanilla dimensions by id; any other gets the overworld's flags.
    pub fn from_id(id: &str, min_y: i32, height: i32) -> Dimension {
        match id {
            "minecraft:the_nether" => Dimension { min_y, height, logical_height: 128.min(height), natural: false, ultra_warm: true, has_ceiling: true, end_effects: false },
            "minecraft:the_end" => Dimension { min_y, height, logical_height: height, natural: false, ultra_warm: false, has_ceiling: false, end_effects: true },
            _ => Dimension { min_y, height, logical_height: height, natural: true, ultra_warm: false, has_ceiling: false, end_effects: false },
        }
    }
}

/// `CachedChunk.size`: the bits of a chunk of this height.
pub fn size(dimension_height: i32) -> usize {
    2 * 16 * 16 * dimension_height as usize
}

/// `CachedChunk.sizeInBytes`: upstream's method.
pub fn size_in_bytes(size: usize) -> usize {
    size / 8
}

/// `CachedChunk.getPositionIndex`: `y` is counted from the dimension's lowest Y.
pub fn get_position_index(x: i32, y: i32, z: i32) -> usize {
    ((x << 1) | (z << 5) | (y << 9)) as usize
}

/// A special block of a chunk: its name and where it is (chunk-relative `x` and `z`, world `y`).
pub type SpecialBlocks = Vec<(Arc<str>, Vec<[i32; 3]>)>;

#[derive(Clone, Debug)]
pub struct CachedChunk {
    pub x: i32,
    pub z: i32,
    pub height: i32,
    min_y: i32,
    /// `sizeInBytes` bytes, bit `i` is bit `i & 7` of byte `i >> 3` (`BitSet.toByteArray`, padded).
    data: Vec<u8>,
    /// The state of the top block of each column (`z << 4 | x`).
    overview: Vec<usize>,
    height_map: [i32; 256],
    /// Position index (D1: the same Y as the lookup, counted from the lowest Y) to the state of the special block.
    special: Option<HashMap<usize, usize>>,
    locations: SpecialBlocks,
    pub cache_timestamp: i64,
}

impl CachedChunk {
    /// `CachedChunk.validateSize`: ported, see the notes below and docs/deviations.md D41.
    /// `CachedChunk.setSpecial`: ported, see the notes below and docs/deviations.md D41.
    /// Upstream's `CachedChunk.<init>`: `data` is padded or checked to the chunk's size. A name of `locations` the palette does not know is an error (upstream: `stringToBlockRequired` throws).
    #[allow(clippy::too_many_arguments)]
    pub fn new(x: i32, z: i32, dim: &Dimension, data: Vec<u8>, overview: Vec<usize>, locations: SpecialBlocks, cache_timestamp: i64, env: &CacheEnv) -> Result<CachedChunk, String> {
        let want = size_in_bytes(size(dim.height));
        if data.len() > want {
            return Err("BitSet of invalid length provided".to_string());
        }
        let mut data = data;
        data.resize(want, 0);
        if overview.len() != 256 {
            return Err("the overview needs 256 columns".to_string());
        }
        let special = if locations.is_empty() {
            None
        } else {
            let mut map = HashMap::new();
            for (name, list) in &locations {
                let state = env.palette.default_state(name).ok_or_else(|| format!("unknown block {name}"))?;
                for p in list {
                    // D1: upstream keys this by the world Y and looks it up by the Y counted from the lowest one.
                    map.insert(get_position_index(p[0], p[1] - dim.min_y, p[2]), state);
                }
            }
            Some(map)
        };
        let mut chunk = CachedChunk { x, z, height: dim.height, min_y: dim.min_y, data, overview, height_map: [0; 256], special, locations, cache_timestamp };
        chunk.calculate_height_map();
        Ok(chunk)
    }

    fn bit(&self, index: usize) -> bool {
        self.data[index >> 3] >> (index & 7) & 1 != 0
    }

    /// `CachedChunk.getType`: upstream's method.
    fn get_type(&self, index: usize) -> PathingBlockType {
        PathingBlockType::from_bits(self.bit(index), self.bit(index + 1))
    }

    /// `CachedChunk.getBlock`: upstream's method. `y` is counted from the dimension's lowest Y. `None` is upstream's `null` (a solid block in a dimension that is neither natural,
    /// ultra warm nor the End); the reader says air for it.
    pub fn get_block(&self, x: i32, y: i32, z: i32, dim: &Dimension, env: &CacheEnv) -> Option<usize> {
        let index = get_position_index(x, y, z);
        let ty = self.get_type(index);
        let internal = (z << 4 | x) as usize;
        if self.height_map[internal] == y && ty != PathingBlockType::Avoid {
            return Some(self.overview[internal]);
        }
        if let Some(special) = &self.special {
            if let Some(state) = special.get(&index) {
                return Some(*state);
            }
        }
        if ty == PathingBlockType::Solid {
            if y == dim.logical_height - 1 && dim.has_ceiling {
                // nether roof is always unbreakable
                return Some(env.palette.bedrock);
            }
            // D2: upstream compares the Y counted from the lowest one with -59, which is never below it.
            if y + dim.min_y < -59 && dim.natural {
                // solid blocks below 5 are commonly bedrock; discourage paths that break them
                return Some(env.palette.obsidian);
            }
        }
        path_type_to_block(ty, dim, env)
    }

    /// `CachedChunk.calculateHeightMap()`: ported, see the notes below and docs/deviations.md D41.
    /// `CachedChunk.calculateHeightMap(BitSet,int,int[])`: ported, see the notes below and docs/deviations.md D41.
    /// The highest column cell that is not air, 0 for a column that is all air.
    fn calculate_height_map(&mut self) {
        self.height_map = [0; 256];
        let mut left = 256;
        for y in (0..self.height as usize).rev() {
            if left == 0 {
                break;
            }
            let layer = &self.data[y * 64..y * 64 + 64];
            if layer.iter().all(|b| *b == 0) {
                continue;
            }
            for z in 0..16usize {
                for x in 0..16usize {
                    if self.height_map[z << 4 | x] != 0 {
                        continue;
                    }
                    let idx = (x << 1) | (z << 5);
                    if layer[idx >> 3] >> (idx & 7) & 3 != 0 {
                        self.height_map[z << 4 | x] = y as i32;
                        left -= 1;
                    }
                }
            }
        }
    }

    /// `CachedChunk.getOverview`: the state of the top block of each column.
    pub fn overview(&self) -> &[usize] {
        &self.overview
    }

    /// `CachedChunk.getRelativeBlocks`: upstream's method.
    pub fn relative_blocks(&self) -> &SpecialBlocks {
        &self.locations
    }

    /// `CachedChunk.getAbsoluteBlocks`: upstream's method.
    pub fn absolute_blocks(&self, block_type: &str) -> Option<Vec<Pos>> {
        let (_, list) = self.locations.iter().find(|(n, _)| **n == *block_type)?;
        Some(list.iter().map(|p| Pos::new(p[0] + self.x * 16, p[1], p[2] + self.z * 16)).collect())
    }

    /// `CachedChunk.toByteArray`: the packed bits, `sizeInBytes` long (upstream's is cut after the last set bit; the file pads it).
    pub fn to_byte_array(&self) -> &[u8] {
        &self.data
    }

    /// Whether two packed chunks say the same about the world: the same bits, top blocks and special blocks. The timestamp does not count.
    pub fn same_content(&self, other: &CachedChunk) -> bool {
        self.x == other.x && self.z == other.z && self.height == other.height && self.data == other.data && self.overview == other.overview && self.locations == other.locations
    }

    /// The lowest Y of the dimension this chunk was packed in.
    pub fn min_y(&self) -> i32 {
        self.min_y
    }
}

/// `ChunkPacker.pathingTypeToBlock`: upstream's method.
pub fn path_type_to_block(ty: PathingBlockType, dim: &Dimension, env: &CacheEnv) -> Option<usize> {
    match ty {
        PathingBlockType::Air => Some(env.palette.air),
        PathingBlockType::Water => Some(env.palette.water),
        PathingBlockType::Avoid => Some(env.palette.lava),
        PathingBlockType::Solid => {
            if dim.natural {
                Some(env.palette.stone)
            } else if dim.ultra_warm {
                Some(env.palette.netherrack)
            } else if dim.end_effects {
                Some(env.palette.end_stone)
            } else {
                None
            }
        }
    }
}

/// One loaded chunk, as the packer reads it. The glue implements this over the game's chunk; `y` is counted from the dimension's lowest Y.
pub trait PackSource {
    /// The state at a cell of the chunk (`x`, `z` in 0..16).
    fn state(&self, x: i32, y: i32, z: i32) -> usize;
    /// `LevelChunkSection.hasOnlyAir` for section `section`, counted up from the lowest one. Such a section is skipped.
    fn section_has_only_air(&self, section: usize) -> bool;
    /// The 4096 states of a section in the order `y << 8 | z << 4 | x`.
    fn fill_section(&self, section: usize, out: &mut [usize; 4096]) {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.state((i & 15) as i32, (section * 16 + (i >> 8)) as i32, ((i >> 4) & 15) as i32);
        }
    }
    /// The state of the cell just outside the chunk (`x` or `z` is -1 or 16) if that chunk is loaded. Used only for the flow test at the chunk edge (D3).
    fn outside_state(&self, _x: i32, _y: i32, _z: i32) -> Option<usize> {
        None
    }
}

/// D3: whether a still water block on the chunk's edge has a horizontal flow: a neighbour that is flowing water, or an empty neighbour with water below it.
/// Upstream asks the fluid for its flow vector, which reads the chunk next door; this reads the same cells.
fn edge_flow(src: &dyn PackSource, env: &CacheEnv, x: i32, y: i32, z: i32, height: i32) -> bool {
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let (nx, nz) = (x + dx, z + dz);
        let at = |y: i32| -> Option<usize> {
            if y < 0 || y >= height {
                None
            } else if (0..16).contains(&nx) && (0..16).contains(&nz) {
                Some(src.state(nx, y, nz))
            } else {
                src.outside_state(nx, y, nz)
            }
        };
        let Some(n) = at(y) else { continue };
        let f = env.flags(n);
        if f & F_WATER != 0 && f & F_FLOWING != 0 {
            return true;
        }
        if f & F_AIR != 0 {
            if let Some(below) = at(y - 1) {
                if env.flags(below) & F_WATER != 0 {
                    return true;
                }
            }
        }
    }
    false
}

/// `ChunkPacker.getPathingBlockType`: with D3. `x` and `z` are in the chunk, `y` is counted from the lowest Y.
#[allow(clippy::too_many_arguments)]
fn pathing_block_type(src: &dyn PackSource, env: &CacheEnv, allow_walk_on_magma: bool, state: usize, x: i32, y: i32, z: i32, height: i32) -> PathingBlockType {
    let f = env.flags(state);
    if f & F_WATER != 0 {
        // only water source blocks are plausibly usable, flowing water should be avoided
        if f & F_FLOWING != 0 {
            return PathingBlockType::Avoid;
        }
        let flowing_at = |nx: i32, nz: i32| env.flags(src.state(nx, y, nz)) & F_FLOWING != 0;
        if (x != 15 && flowing_at(x + 1, z)) || (x != 0 && flowing_at(x - 1, z)) || (z != 15 && flowing_at(x, z + 1)) || (z != 0 && flowing_at(x, z - 1)) {
            return PathingBlockType::Avoid;
        }
        if (x == 0 || x == 15 || z == 0 || z == 15) && edge_flow(src, env, x, y, z, height) {
            // D3: upstream returns WATER here and AVOID for the still water.
            return PathingBlockType::Avoid;
        }
        return PathingBlockType::Water;
    }
    if f & F_AVOID != 0 || (f & F_MAGMA != 0 && !allow_walk_on_magma) {
        return PathingBlockType::Avoid;
    }
    if f & F_AIRLIKE != 0 {
        return PathingBlockType::Air;
    }
    PathingBlockType::Solid
}

/// `ChunkPacker.pack`: upstream's method.
pub fn pack(src: &dyn PackSource, chunk_x: i32, chunk_z: i32, dim: &Dimension, env: &CacheEnv, allow_walk_on_magma: bool, now_ms: i64) -> CachedChunk {
    let mut data = vec![0u8; size_in_bytes(size(dim.height))];
    let mut special: HashMap<Arc<str>, Vec<[i32; 3]>> = HashMap::new();
    let mut cells = Box::new([0usize; 4096]);
    let mut set = |index: usize, bits: [bool; 2]| {
        for (k, b) in bits.iter().enumerate() {
            let i = index + k;
            if *b {
                data[i >> 3] |= 1 << (i & 7);
            } else {
                data[i >> 3] &= !(1 << (i & 7));
            }
        }
    };
    for y0 in 0..(dim.height / 16) as usize {
        if src.section_has_only_air(y0) {
            // any 16x16x16 area that is all air is zeros already
            continue;
        }
        src.fill_section(y0, &mut cells);
        for y1 in 0..16i32 {
            let y = y1 + (y0 as i32) * 16;
            for z in 0..16i32 {
                for x in 0..16i32 {
                    let state = cells[((y1 << 8) | (z << 4) | x) as usize];
                    let ty = pathing_block_type(src, env, allow_walk_on_magma, state, x, y, z, dim.height);
                    set(get_position_index(x, y, z), ty.bits());
                    if let Some(name) = env.special(state) {
                        special.entry(Arc::clone(name)).or_default().push([x, y + dim.min_y, z]);
                    }
                }
            }
        }
    }
    let probe = CachedChunk { x: chunk_x, z: chunk_z, height: dim.height, min_y: dim.min_y, data: data.clone(), overview: vec![], height_map: [0; 256], special: None, locations: vec![], cache_timestamp: now_ms };
    let mut overview = vec![env.palette.air; 256];
    for z in 0..16i32 {
        for x in 0..16i32 {
            for y in (0..dim.height).rev() {
                let index = get_position_index(x, y, z);
                if probe.bit(index) || probe.bit(index + 1) {
                    overview[(z << 4 | x) as usize] = src.state(x, y, z);
                    break;
                }
            }
        }
    }
    let mut locations: SpecialBlocks = special.into_iter().collect();
    locations.sort_by(|a, b| a.0.cmp(&b.0));
    // the names come from the palette, so they resolve
    CachedChunk::new(chunk_x, chunk_z, dim, data, overview, locations, now_ms, env).expect("a packed chunk is valid")
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::cache::palette::fixture::*;

    /// A chunk in a vector, 16 x height x 16, indexed `(y * 16 + z) * 16 + x`.
    pub struct Cells {
        pub cells: Vec<usize>,
        pub outside: HashMap<(i32, i32, i32), usize>,
    }

    impl Cells {
        pub fn new(height: i32) -> Cells {
            Cells { cells: vec![AIR; (height * 256) as usize], outside: HashMap::new() }
        }
        pub fn set(&mut self, x: i32, y: i32, z: i32, s: usize) {
            self.cells[((y * 16 + z) * 16 + x) as usize] = s;
        }
    }

    impl PackSource for Cells {
        fn state(&self, x: i32, y: i32, z: i32) -> usize {
            self.cells[((y * 16 + z) * 16 + x) as usize]
        }
        fn section_has_only_air(&self, section: usize) -> bool {
            self.cells[section * 4096..section * 4096 + 4096].iter().all(|s| *s == AIR)
        }
        fn outside_state(&self, x: i32, y: i32, z: i32) -> Option<usize> {
            self.outside.get(&(x, y, z)).copied()
        }
    }

    pub fn overworld() -> Dimension {
        Dimension::from_id("minecraft:overworld", -64, 384)
    }

    #[test]
    fn the_index_and_the_bit_order_are_javas() {
        assert_eq!(get_position_index(0, 0, 0), 0);
        assert_eq!(get_position_index(1, 0, 0), 2);
        assert_eq!(get_position_index(0, 0, 1), 32);
        assert_eq!(get_position_index(0, 1, 0), 512);
        assert_eq!(size_in_bytes(size(384)), 24576);
        for t in [PathingBlockType::Air, PathingBlockType::Water, PathingBlockType::Avoid, PathingBlockType::Solid] {
            let [a, b] = t.bits();
            assert_eq!(PathingBlockType::from_bits(a, b), t);
        }
        // a solid block at (1, 0, 0) sets bits 2 and 3, which is 0b1100 in byte 0
        let env = env();
        let mut c = Cells::new(384);
        c.set(1, 0, 0, STONE);
        let p = pack(&c, 0, 0, &overworld(), &env, false, 0);
        assert_eq!(p.to_byte_array()[0], 0b1100);
        assert_eq!(p.to_byte_array().len(), 24576);
    }

    #[test]
    fn what_a_block_packs_to() {
        let env = env();
        let mut c = Cells::new(384);
        c.set(5, 10, 5, STONE);
        c.set(6, 10, 5, LAVA);
        c.set(7, 10, 5, GRASS);
        c.set(8, 10, 5, SLAB);
        c.set(9, 10, 5, WATER);
        c.set(9, 10, 8, WATER);
        c.set(10, 10, 8, FLOWING); // beside a flowing block: the still water is avoided
        let p = pack(&c, 0, 0, &overworld(), &env, false, 0);
        let dim = overworld();
        assert_eq!(p.get_type(get_position_index(5, 10, 5)), PathingBlockType::Solid);
        assert_eq!(p.get_type(get_position_index(6, 10, 5)), PathingBlockType::Avoid);
        assert_eq!(p.get_type(get_position_index(7, 10, 5)), PathingBlockType::Air);
        assert_eq!(p.get_type(get_position_index(8, 10, 5)), PathingBlockType::Avoid);
        assert_eq!(p.get_type(get_position_index(9, 10, 5)), PathingBlockType::Water);
        assert_eq!(p.get_type(get_position_index(9, 10, 8)), PathingBlockType::Avoid);
        assert_eq!(p.get_type(get_position_index(10, 10, 8)), PathingBlockType::Avoid);
        // not the top block of its column: the type decides
        c.set(5, 11, 5, STONE);
        let p = pack(&c, 0, 0, &dim, &env, false, 0);
        assert_eq!(p.get_block(5, 10, 5, &dim, &env), Some(env.palette.stone));
        assert_eq!(p.get_block(5, 11, 5, &dim, &env), Some(STONE), "the top block is the overview's");
        assert_eq!(p.get_block(5, 20, 5, &dim, &env), Some(env.palette.air));
    }

    #[test]
    fn magma_follows_the_setting() {
        let mut st = states();
        st[STONE].block = crate::blocks::Block::MagmaBlock;
        let default_of: Vec<usize> = (0..st.len()).collect();
        let env = CacheEnv::new(&st, &default_of, &vec![false; st.len()], &|s| !s.fluid_empty);
        let mut c = Cells::new(384);
        c.set(1, 5, 1, STONE);
        let dim = overworld();
        assert_eq!(pack(&c, 0, 0, &dim, &env, false, 0).get_type(get_position_index(1, 5, 1)), PathingBlockType::Avoid);
        assert_eq!(pack(&c, 0, 0, &dim, &env, true, 0).get_type(get_position_index(1, 5, 1)), PathingBlockType::Solid);
    }

    /// D1: a chest is found at the height it was packed at. Upstream keys it by the world Y and asks with the Y counted from the lowest one.
    #[test]
    fn d1_a_special_block_comes_back_at_its_own_height() {
        let env = env();
        let dim = overworld();
        let mut c = Cells::new(384);
        // world y 70 is cell 134
        c.set(3, 134, 4, CHEST);
        c.set(3, 135, 4, STONE);
        let p = pack(&c, 2, -3, &dim, &env, false, 0);
        assert_eq!(p.absolute_blocks("chest"), Some(vec![Pos::new(2 * 16 + 3, 70, -3 * 16 + 4)]));
        // not the top block (stone is above), not a plain type: the special block answers
        assert_eq!(p.get_block(3, 134, 4, &dim, &env), Some(CHEST));
        // upstream looked it up 64 blocks too low and answered "solid" (stone) at the cell it was packed in
        assert_ne!(p.get_block(3, 134, 4, &dim, &env), Some(env.palette.stone));
        // and at the cell 64 below nothing special is found
        assert_eq!(p.get_block(3, 70, 4, &dim, &env), Some(env.palette.air));
    }

    /// D2: below world Y -59 a solid block in a natural dimension is obsidian.
    #[test]
    fn d2_the_bottom_layers_are_cautious() {
        let env = env();
        let dim = overworld();
        let mut c = Cells::new(384);
        c.set(1, 2, 1, STONE); // world y -62
        c.set(1, 3, 1, STONE);
        c.set(1, 4, 1, STONE); // world -60
        c.set(1, 5, 1, STONE); // world -59: stone
        c.set(1, 6, 1, STONE);
        let p = pack(&c, 0, 0, &dim, &env, false, 0);
        assert_eq!(p.get_block(1, 2, 1, &dim, &env), Some(env.palette.obsidian));
        assert_eq!(p.get_block(1, 4, 1, &dim, &env), Some(env.palette.obsidian));
        assert_eq!(p.get_block(1, 5, 1, &dim, &env), Some(env.palette.stone));
        // in a dimension that is not natural: no caution
        let nether = Dimension::from_id("minecraft:the_nether", 0, 256);
        let mut n = Cells::new(256);
        n.set(1, 1, 1, STONE);
        n.set(1, 2, 1, STONE);
        let p = pack(&n, 0, 0, &nether, &env, false, 0);
        assert_eq!(p.get_block(1, 1, 1, &nether, &env), Some(env.palette.netherrack));
    }

    /// D3: still water on the edge of the chunk is water, unless it flows.
    #[test]
    fn d3_water_at_the_chunk_edge_is_not_inverted() {
        let env = env();
        let dim = overworld();
        let mut c = Cells::new(384);
        c.set(0, 100, 7, WATER);
        c.set(0, 100, 9, WATER);
        c.set(15, 100, 3, WATER);
        // beyond the west edge of (0, 100, 9) there is flowing water; beyond the east edge of (15, 100, 3) a still source
        c.outside.insert((-1, 100, 9), FLOWING);
        c.outside.insert((16, 100, 3), WATER);
        let p = pack(&c, 0, 0, &dim, &env, false, 0);
        // upstream: the still water without a flow is AVOID (the flow test is the wrong way round)
        assert_eq!(p.get_type(get_position_index(0, 100, 7)), PathingBlockType::Water);
        assert_eq!(p.get_type(get_position_index(15, 100, 3)), PathingBlockType::Water);
        assert_eq!(p.get_type(get_position_index(0, 100, 9)), PathingBlockType::Avoid);
        // an interior still block is water as before
        let mut d = Cells::new(384);
        d.set(8, 100, 8, WATER);
        assert_eq!(pack(&d, 0, 0, &dim, &env, false, 0).get_type(get_position_index(8, 100, 8)), PathingBlockType::Water);
    }

    #[test]
    fn the_nether_roof_is_bedrock_and_other_dimensions_answer_per_kind() {
        let env = env();
        let nether = Dimension::from_id("minecraft:the_nether", 0, 256);
        assert_eq!(nether.logical_height, 128);
        let mut n = Cells::new(256);
        n.set(2, 100, 2, STONE);
        n.set(2, 127, 2, STONE);
        n.set(2, 128, 2, STONE);
        let p = pack(&n, 0, 0, &nether, &env, false, 0);
        // below the top block the roof rule still applies at logical height - 1
        assert_eq!(p.get_block(2, 127, 2, &nether, &env), Some(env.palette.bedrock));
        let end = Dimension::from_id("minecraft:the_end", 0, 256);
        assert_eq!(path_type_to_block(PathingBlockType::Solid, &end, &env), Some(env.palette.end_stone));
        let odd = Dimension { natural: false, ..end };
        let odd = Dimension { end_effects: false, ..odd };
        assert_eq!(path_type_to_block(PathingBlockType::Solid, &odd, &env), None);
    }

    #[test]
    fn the_height_map_and_overview_hold_the_top_block() {
        let env = env();
        let dim = overworld();
        let mut c = Cells::new(384);
        for y in 0..70 {
            c.set(4, y, 4, STONE);
        }
        c.set(4, 70, 4, DIRT);
        c.set(4, 90, 4, GRASS); // air-like: not a top block
        let p = pack(&c, 0, 0, &dim, &env, false, 0);
        assert_eq!(p.height_map[4 * 16 + 4], 70);
        assert_eq!(p.overview()[4 * 16 + 4], DIRT);
        assert_eq!(p.overview()[0], AIR);
        assert_eq!(p.height_map[0], 0);
    }

    #[test]
    fn random_chunks_round_trip_through_their_bytes() {
        let env = env();
        let dim = overworld();
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..6 {
            let mut c = Cells::new(384);
            for _ in 0..4000 {
                let r = next();
                let (x, y, z) = ((r % 16) as i32, ((r >> 8) % 384) as i32, ((r >> 20) % 16) as i32);
                c.set(x, y, z, ((r >> 40) % 15) as usize);
            }
            let p = pack(&c, 1, 2, &dim, &env, false, 77);
            // rebuild from the bytes, the overview and the special list: the same answers everywhere
            let q = CachedChunk::new(1, 2, &dim, p.to_byte_array().to_vec(), p.overview().to_vec(), p.relative_blocks().clone(), 77, &env).unwrap();
            for _ in 0..3000 {
                let r = next();
                let (x, y, z) = ((r % 16) as i32, ((r >> 8) % 384) as i32, ((r >> 20) % 16) as i32);
                assert_eq!(p.get_block(x, y, z, &dim, &env), q.get_block(x, y, z, &dim, &env));
            }
            assert_eq!(p.height_map, q.height_map);
        }
    }

    #[test]
    fn a_too_long_bit_set_or_an_unknown_name_is_refused() {
        let env = env();
        let dim = overworld();
        assert!(CachedChunk::new(0, 0, &dim, vec![0; 24577], vec![0; 256], vec![], 0, &env).is_err());
        let bad: SpecialBlocks = vec![(Arc::from("no_such_block"), vec![[0, 0, 0]])];
        assert!(CachedChunk::new(0, 0, &dim, vec![], vec![0; 256], bad, 0, &env).is_err());
    }
}
