// baritone-rs engine: how the rules see the world. Written for this project. LGPL-3.0-or-later, see lib.rs.

/// What a search reads. The game's implementation holds a snapshot of chunks; tests use [`GridWorld`].
/// What the persistent cache says about a chunk for `ExploreProcess`: upstream's private `ExploreProcess.Status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkStatus {
    /// The cache holds the chunk.
    Explored,
    /// The region of the chunk is in memory and has no such chunk.
    NotExplored,
    /// The region has not been read from disk yet: a read was asked for, and the answer comes in a later tick.
    Unknown,
}

pub trait WorldView {
    /// The block state id at a position, an index into the state table. `y` is a world Y inside the world's height.
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize;
    /// `BlockStateInterface.worldContainsLoadedChunk`: whether the live world has this column loaded.
    fn contains_loaded_chunk(&self, x: i32, z: i32) -> bool;
    /// `CalculationContext.isLoaded`: whether the column is loaded or held by the cache, as the search asks it.
    /// `BlockStateInterface.isLoaded`: the glue answers with its cache. The default is the live check alone, which is what a world without a cache means.
    fn is_loaded(&self, x: i32, z: i32) -> bool {
        self.contains_loaded_chunk(x, z)
    }
    /// The state of the block if its chunk is loaded, `None` if not. One lookup for what `contains_loaded_chunk` and `state_id` ask apart; the cache
    /// layer (`cache::WithCache`) uses it to read a block from the loaded world first.
    fn loaded_state_id(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        self.contains_loaded_chunk(x, z).then(|| self.state_id(x, y, z))
    }
    /// `ctx.worldData.getCachedWorld().getLocationsOf`: the places the persistent cache holds of a special block (`cache::BLOCKS_TO_KEEP_TRACK_OF`), searched
    /// around the region of `(center_x, center_z)` out to `max_region_distance_sq`. The default is a world without a cache: none.
    fn cached_locations(&self, _block: &str, _maximum: i32, _center_x: i32, _center_z: i32, _max_region_distance_sq: i32) -> Vec<crate::pos::Pos> {
        vec![]
    }
    /// Upstream's `ExploreProcess.BaritoneChunkCache.isAlreadyExplored`: what the cache says about the chunk at these chunk coordinates. Asking about a chunk of a region that is
    /// not in memory starts reading that region on another thread and answers [`ChunkStatus::Unknown`]. The default is a world without a cache (the glue always has one
    /// while a world is open): a loaded chunk is explored, any other is not.
    fn chunk_status(&self, chunk_x: i32, chunk_z: i32) -> ChunkStatus {
        if self.chunk_loaded(chunk_x, chunk_z) {
            ChunkStatus::Explored
        } else {
            ChunkStatus::NotExplored
        }
    }
    /// Whether the door or fence gate at this position is open. Not a fact of the block table: it is the block's `open` property,
    /// which only the executor reads (`MovementHelper.isDoorPassable`, `isGatePassable`). The default says open, so a world that
    /// does not know has no door that needs a click.
    fn is_open(&self, _x: i32, _y: i32, _z: i32) -> bool {
        true
    }
    /// Whether a falling block (an entity) is in the cell above or at this position, where a movement would mine
    /// (`Movement.prepared`: `getEntitiesOfClass(FallingBlockEntity.class, AABB(0, 0, 0, 1, 1.1, 1).move(pos))`). The default says none.
    fn falling_block_near(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }
    /// `FasterWorldScanner`: whether the chunk at these chunk coordinates is loaded (`ClientChunkCache.getChunk(x, z, null, false) != null`). The
    /// default asks about the chunk's first column.
    fn chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.contains_loaded_chunk(chunk_x.wrapping_mul(16), chunk_z.wrapping_mul(16))
    }
    /// `FasterWorldScanner`: `LevelChunkSection.hasOnlyAir` for section number `section` of a loaded chunk, counted up from `min_y`. The scan skips such
    /// a section. The default says no, so the scan reads every cell: the same answer unless the search looks for air, which the section rule would skip.
    fn section_has_only_air(&self, _chunk_x: i32, _chunk_z: i32, _section: i32) -> bool {
        false
    }
    /// The lowest Y of the world.
    fn min_y(&self) -> i32;
    /// The world's height in blocks.
    fn height(&self) -> i32;
}

/// Whether a falling block with its feet at `f` (a box 0.98 wide and tall, centred on the feet) intersects
/// `AABB(0, 0, 0, 1, 1.1, 1).move(x, y, z)`, as `Level.getEntitiesOfClass` tests it: strictly, on all three axes
/// (`Movement.prepared`). The size is the game's `f32` 0.98.
pub fn falling_box_hits(f: [f64; 3], x: i32, y: i32, z: i32) -> bool {
    const SIZE: f64 = 0.98_f32 as f64;
    const HALF: f64 = SIZE / 2.0;
    let (bx, by, bz) = (f64::from(x), f64::from(y), f64::from(z));
    f[0] - HALF < bx + 1.0 && f[0] + HALF > bx && f[1] < by + 1.1 && f[1] + SIZE > by && f[2] - HALF < bz + 1.0 && f[2] + HALF > bz
}

/// A state id the block table has, or `air` if it has none. Not upstream: Java's tables are as long as the game's registry, and the
/// glue's ids come from a palette that can be wider than the registry; an index past the table would panic in the rules.
pub fn known_state(id: usize, table_len: usize, air: usize) -> usize {
    if id < table_len {
        id
    } else {
        air
    }
}

/// A small box of blocks for tests. Everything outside it is `air`.
pub struct GridWorld<'a> {
    pub w: i32,
    pub h: i32,
    pub d: i32,
    /// Indexed `(y * d + z) * w + x`.
    pub cells: &'a [u16],
    pub air: usize,
    pub loaded: bool,
}

impl WorldView for GridWorld<'_> {
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
        if x < 0 || x >= self.w || z < 0 || z >= self.d {
            return self.air;
        }
        self.cells[((y * self.d + z) * self.w + x) as usize] as usize
    }
    fn contains_loaded_chunk(&self, _x: i32, _z: i32) -> bool {
        self.loaded
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        self.h
    }
}

#[cfg(test)]
mod tests {
    use super::{falling_box_hits, known_state};

    #[test]
    fn a_falling_block_in_or_just_above_the_cell_pauses_mining() {
        // in the cell, one cell above (the box reaches 1.1 up), and far away
        assert!(falling_box_hits([5.5, 10.0, 5.5], 5, 10, 5));
        assert!(falling_box_hits([5.5, 11.05, 5.5], 5, 10, 5));
        assert!(!falling_box_hits([5.5, 11.1, 5.5], 5, 10, 5));
        assert!(!falling_box_hits([5.5, 20.0, 5.5], 5, 10, 5));
        assert!(!falling_box_hits([7.5, 10.0, 5.5], 5, 10, 5));
        // beside the cell: out of reach at 4.5 (right edge 4.99), inside at 4.6
        assert!(!falling_box_hits([4.5, 10.0, 5.5], 5, 10, 5));
        assert!(falling_box_hits([4.6, 10.0, 5.5], 5, 10, 5));
        // below the cell: the block's top (y + 0.98) must pass the cell's floor
        assert!(!falling_box_hits([5.5, 8.99, 5.5], 5, 10, 5));
        assert!(falling_box_hits([5.5, 9.1, 5.5], 5, 10, 5));
    }

    #[test]
    fn an_id_the_table_lacks_reads_as_air() {
        assert_eq!(known_state(5, 10, 0), 5);
        assert_eq!(known_state(9, 10, 0), 9);
        assert_eq!(known_state(10, 10, 0), 0);
        assert_eq!(known_state(65535, 10, 3), 3);
    }
}
