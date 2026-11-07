// baritone-rs engine: how the rules see the world. Written for this project. LGPL-3.0-or-later, see lib.rs.

/// What a search reads. The game's implementation holds a snapshot of chunks; tests use [`GridWorld`].
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
    /// Whether the door or fence gate at this position is open. Not a fact of the block table: it is the block's `open` property,
    /// which only the executor reads (`MovementHelper.isDoorPassable`, `isGatePassable`). The default says open, so a world that
    /// does not know has no door that needs a click.
    fn is_open(&self, _x: i32, _y: i32, _z: i32) -> bool {
        true
    }
    /// The lowest Y of the world.
    fn min_y(&self) -> i32;
    /// The world's height in blocks.
    fn height(&self) -> i32;
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
    use super::known_state;

    #[test]
    fn an_id_the_table_lacks_reads_as_air() {
        assert_eq!(known_state(5, 10, 0), 5);
        assert_eq!(known_state(9, 10, 0), 9);
        assert_eq!(known_state(10, 10, 0), 0);
        assert_eq!(known_state(65535, 10, 3), 3);
    }
}
