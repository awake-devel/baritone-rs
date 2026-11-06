// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/utils/BetterBlockPos.java (the parts a search uses)
// LGPL-3.0-or-later, see lib.rs.
use crate::blocks::Direction;

/// `BetterBlockPos`: a block position. Java `int` arithmetic wraps, so every step here wraps.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Pos {
    pub fn new(x: i32, y: i32, z: i32) -> Pos {
        Pos { x, y, z }
    }

    /// `BetterBlockPos.above`.
    pub fn above(self, amt: i32) -> Pos {
        Pos { x: self.x, y: self.y.wrapping_add(amt), z: self.z }
    }

    /// `BetterBlockPos.below`.
    pub fn below(self, amt: i32) -> Pos {
        Pos { x: self.x, y: self.y.wrapping_sub(amt), z: self.z }
    }

    /// Moves `dist` steps in a direction (upstream's `relative(Direction,int)`), the product in `int` arithmetic.
    pub fn relative(self, dir: Direction, dist: i32) -> Pos {
        Pos { x: self.x.wrapping_add(dir.step_x().wrapping_mul(dist)), y: self.y.wrapping_add(dir.step_y().wrapping_mul(dist)), z: self.z.wrapping_add(dir.step_z().wrapping_mul(dist)) }
    }

    /// The difference (upstream's `Vec3i.subtract`) of two positions, componentwise.
    pub fn subtract(self, other: Pos) -> Pos {
        Pos { x: self.x.wrapping_sub(other.x), y: self.y.wrapping_sub(other.y), z: self.z.wrapping_sub(other.z) }
    }

    /// `BetterBlockPos.longHash(BetterBlockPos)`.
    pub fn long_hash(self) -> i64 {
        long_hash(self.x, self.y, self.z)
    }
}

/// Upstream's `longHash(int,int,int)`: a polynomial hash in `long` arithmetic. It is not injective, and the favoring map is keyed on it, so collisions are part of upstream's behaviour.
pub fn long_hash(x: i32, y: i32, z: i32) -> i64 {
    let mut hash: i64 = 3241;
    hash = 3457689i64.wrapping_mul(hash).wrapping_add(x as i64);
    hash = 8734625i64.wrapping_mul(hash).wrapping_add(y as i64);
    hash = 2873465i64.wrapping_mul(hash).wrapping_add(z as i64);
    hash
}
