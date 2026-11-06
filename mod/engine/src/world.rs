// baritone-rs engine: how the rules see the world. Written for this project. LGPL-3.0-or-later, see lib.rs.

/// What a search reads. The game's implementation holds a snapshot of chunks; tests use [`GridWorld`].
pub trait WorldView {
    /// The block state id at a position, an index into the state table. `y` is a world Y inside the world's height.
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize;
    /// `BlockStateInterface.worldContainsLoadedChunk`: whether the live world has this column loaded.
    fn contains_loaded_chunk(&self, x: i32, z: i32) -> bool;
    /// The lowest Y of the world.
    fn min_y(&self) -> i32;
    /// The world's height in blocks.
    fn height(&self) -> i32;
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
