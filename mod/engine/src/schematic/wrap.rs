// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/schematic/{MaskSchematic,ShellSchematic,WallsSchematic,ReplaceSchematic,
// SubstituteSchematic,RotatedSchematic,MirroredSchematic}.java and src/main/java/baritone/utils/schematic/MapArtSchematic.java.
// LGPL-3.0-or-later, see lib.rs.
//
// The schematics that wrap another schematic. The mask schematics are one type here, [`MaskSchematic`], with the mask as an enum:
// upstream's anonymous subclass of `MaskSchematic` (from `create`), `ShellSchematic`, `WallsSchematic`, `ReplaceSchematic` and
// `MapArtSchematic` differ only in their `partOfMask`, and `ReplaceSchematic` in its `reset`.
use super::mask::Mask;
use super::{BlockMirror, BlockRotation, BlockStates, Schematic, StateId, StaticSchematicView};
use crate::blockmeta::BlockOptionalMetaLookup;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// The number of cells above which a [`MaskSchematic::replace`] keeps no cache (2^30). Upstream would run out of memory.
const MAX_CACHE_CELLS: usize = 1 << 30;

/// What `partOfMask` is.
#[derive(Debug)]
enum MaskKind {
    /// Like `MaskSchematic.create`: a [`Mask`] decides.
    Custom(Arc<dyn Mask>),
    /// `ShellSchematic`.
    Shell,
    /// `WallsSchematic`.
    Walls,
    /// `ReplaceSchematic`: the first answer for each cell is kept for good. 0 is unknown, 1 is false, 2 is true.
    Replace { filter: BlockOptionalMetaLookup, cache: Vec<AtomicU8> },
    /// `MapArtSchematic`: `height_map[x * length_z + z]` is the highest y with a block, or `i32::MAX`.
    MapArt { height_map: Vec<i32> },
}

/// `MaskSchematic`: the inner schematic, but only where the mask says so. Its size is the inner schematic's.
///
/// Upstream's `reset` is not passed to the inner schematic here either (`ISchematic.reset` is empty and `MaskSchematic` does not
/// override it); only a replace mask clears its own cache.
#[derive(Debug)]
pub struct MaskSchematic {
    inner: Arc<dyn Schematic>,
    kind: MaskKind,
    width_x: i32,
    height_y: i32,
    length_z: i32,
}

impl MaskSchematic {
    fn with_kind(inner: Arc<dyn Schematic>, kind: MaskKind) -> MaskSchematic {
        let (width_x, height_y, length_z) = (inner.width_x(), inner.height_y(), inner.length_z());
        MaskSchematic { inner, kind, width_x, height_y, length_z }
    }

    /// `MaskSchematic.create`: a schematic and a mask.
    /// `MaskSchematic.create`.
    pub fn create(inner: Arc<dyn Schematic>, mask: Arc<dyn Mask>) -> MaskSchematic {
        MaskSchematic::with_kind(inner, MaskKind::Custom(mask))
    }

    /// `ShellSchematic.<init>`: only the outer layer of the box.
    pub fn shell(inner: Arc<dyn Schematic>) -> MaskSchematic {
        MaskSchematic::with_kind(inner, MaskKind::Shell)
    }

    /// `WallsSchematic.<init>`: only the four sides, all the way up.
    pub fn walls(inner: Arc<dyn Schematic>) -> MaskSchematic {
        MaskSchematic::with_kind(inner, MaskKind::Walls)
    }

    /// `ReplaceSchematic.<init>`: only the positions whose current state the filter has, decided once for each position until `reset`.
    pub fn replace(inner: Arc<dyn Schematic>, filter: BlockOptionalMetaLookup) -> MaskSchematic {
        let (w, h, l) = (inner.width_x(), inner.height_y(), inner.length_z());
        let cells = if w > 0 && h > 0 && l > 0 { (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(l as usize)).filter(|&n| n <= MAX_CACHE_CELLS) } else { Some(0) };
        // a box too large to cache is asked every time
        let cache = (0..cells.unwrap_or(0)).map(|_| AtomicU8::new(0)).collect();
        MaskSchematic::with_kind(inner, MaskKind::Replace { filter, cache })
    }

    /// `MapArtSchematic.generateHeightMap`: the height of each column.
    /// `MapArtSchematic.lastIndexMatching`: the top block of a column.
    /// `MapArtSchematic.<init>` and `generateHeightMap`: only the positions at or above the highest block of their column, so what is
    /// below the top of a map art does not matter. A column with no block is all part of the mask. The `Vec` holds the lines upstream
    /// prints (`System.out`). `Err` is the exception upstream throws when the inner schematic has no state for a position of a column.
    pub fn map_art(inner: Arc<dyn StaticSchematicView>, states: &BlockStates) -> Result<(MaskSchematic, Vec<String>), String> {
        let (w, l) = (inner.width_x().max(0), inner.length_z().max(0));
        let cells = (w as usize).checked_mul(l as usize).filter(|&n| n <= MAX_CACHE_CELLS).ok_or_else(|| "schematic too large".to_string())?;
        let mut height_map = Vec::with_capacity(cells);
        let mut missing_columns = 0usize;
        for x in 0..w {
            for z in 0..l {
                let column = inner.get_column(x, z).ok_or_else(|| "couldn't find schematic for this position".to_string())?;
                // `lastIndexMatching`: from the top down, the first state that is not air
                match column.iter().rposition(|&s| !states.is_air(s)) {
                    Some(y) => height_map.push(y as i32),
                    None => {
                        missing_columns += 1;
                        height_map.push(i32::MAX);
                    }
                }
            }
        }
        let mut lines = vec![];
        if missing_columns != 0 {
            lines.push(format!("{missing_columns} columns had no block despite being in a map art, letting them be whatever"));
        }
        let inner: Arc<dyn Schematic> = inner;
        Ok((MaskSchematic::with_kind(inner, MaskKind::MapArt { height_map }), lines))
    }

    /// The index of a cell in a table laid out `[x][y][z]` (or `[x][z]` with `y == 0` and `height_y` taken as 1), or `None` outside
    /// the box.
    fn in_box(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0 && y >= 0 && z >= 0 && x < self.width_x && y < self.height_y && z < self.length_z
    }

    /// `MaskSchematic.partOfMask`: the mask's answer, by kind.
    /// `ShellSchematic.partOfMask`: the shell kind.
    /// `WallsSchematic.partOfMask`: the walls kind.
    /// `ReplaceSchematic.partOfMask`: the replace kind, with its sticky cache.
    /// `MapArtSchematic.partOfMask`: the map art kind.
    /// `MaskSchematic.partOfMask` of each kind.
    fn part_of_mask(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        match &self.kind {
            MaskKind::Custom(mask) => mask.part_of_mask(x, y, z, current),
            // `ShellSchematic.partOfMask`
            MaskKind::Shell => x == 0 || y == 0 || z == 0 || x == self.width_x - 1 || y == self.height_y - 1 || z == self.length_z - 1,
            // `WallsSchematic.partOfMask`
            MaskKind::Walls => x == 0 || z == 0 || x == self.width_x - 1 || z == self.length_z - 1,
            // `ReplaceSchematic.partOfMask`. A position outside the box is false (upstream throws); a state of `None` is in no filter.
            MaskKind::Replace { filter, cache } => {
                if !self.in_box(x, y, z) {
                    return false;
                }
                let ask = || current.is_some_and(|c| filter.has(c));
                let (h, l) = (self.height_y as usize, self.length_z as usize);
                let index = (x as usize * h + y as usize) * l + z as usize;
                match cache.get(index) {
                    None => ask(),
                    Some(cell) => match cell.load(Ordering::Relaxed) {
                        1 => false,
                        2 => true,
                        _ => {
                            let v = ask();
                            cell.store(if v { 2 } else { 1 }, Ordering::Relaxed);
                            v
                        }
                    },
                }
            }
            // `MapArtSchematic.partOfMask`
            MaskKind::MapArt { height_map } => {
                if x < 0 || z < 0 || x >= self.width_x || z >= self.length_z {
                    return false;
                }
                height_map.get(x as usize * self.length_z as usize + z as usize).is_some_and(|&top| y >= top)
            }
        }
    }
}

impl Schematic for MaskSchematic {
    /// `MaskSchematic.inSchematic`: the inner schematic says yes and the mask says yes.
    fn in_schematic(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        self.inner.in_schematic(x, y, z, current) && self.part_of_mask(x, y, z, current)
    }

    /// `MaskSchematic.desiredState`: the inner schematic's.
    fn desired_state(&self, x: i32, y: i32, z: i32, current: Option<StateId>, approx_placeable: &[StateId]) -> StateId {
        self.inner.desired_state(x, y, z, current, approx_placeable)
    }

    /// `ReplaceSchematic.reset`: forgets every answer. The other masks have no `reset`, and none passes it on.
    fn reset(&self) {
        if let MaskKind::Replace { cache, .. } = &self.kind {
            for cell in cache {
                cell.store(0, Ordering::Relaxed);
            }
        }
    }
    fn width_x(&self) -> i32 {
        self.width_x
    }
    fn height_y(&self) -> i32 {
        self.height_y
    }
    fn length_z(&self) -> i32 {
        self.length_z
    }
}

/// `SubstituteSchematic`: where the inner schematic wants a block that has substitutes, takes the one that is already there, else one
/// that can be placed.
#[derive(Debug)]
pub struct SubstituteSchematic {
    inner: Arc<dyn Schematic>,
    /// Desired block to its substitutes, in `BlockUtils` form. A key given twice keeps the last list, as `Map.put`.
    substitutions: HashMap<String, Vec<String>>,
    states: Arc<BlockStates>,
    width_x: i32,
    height_y: i32,
    length_z: i32,
}

impl SubstituteSchematic {
    /// `SubstituteSchematic.<init>`. Upstream keeps `withBlock` answers in a map; here each is worked out again (see D46).
    pub fn new(inner: Arc<dyn Schematic>, substitutions: Vec<(String, Vec<String>)>, states: Arc<BlockStates>) -> SubstituteSchematic {
        let norm = |s: &str| s.strip_prefix("minecraft:").unwrap_or(s).to_string();
        let substitutions = substitutions.into_iter().map(|(k, v)| (norm(&k), v.iter().map(|b| norm(b)).collect())).collect();
        let (width_x, height_y, length_z) = (inner.width_x(), inner.height_y(), inner.length_z());
        SubstituteSchematic { inner, substitutions, states, width_x, height_y, length_z }
    }

    /// `substitute instanceof AirBlock`.
    fn is_air_block(&self, block: &str) -> bool {
        self.states.default_state(block).is_some_and(|d| self.states.is_air(d))
    }
}

impl Schematic for SubstituteSchematic {
    /// `SubstituteSchematic.inSchematic`: the inner schematic's.
    fn in_schematic(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        self.inner.in_schematic(x, y, z, current)
    }

    /// `SubstituteSchematic.desiredState`: the substitutes, in order.
    /// `SubstituteSchematic.desiredState`. A current state of `None` is no block: upstream throws a `NullPointerException` for it.
    fn desired_state(&self, x: i32, y: i32, z: i32, current: Option<StateId>, approx_placeable: &[StateId]) -> StateId {
        let desired = self.inner.desired_state(x, y, z, current, approx_placeable);
        let states = &*self.states;
        let Some(substitutes) = self.substitutions.get(states.block(desired)) else {
            return desired;
        };
        // an empty list cannot come from the settings; upstream throws `IndexOutOfBoundsException` at `substitutes.get(0)`
        let Some(first) = substitutes.first() else {
            return desired;
        };
        if let Some(c) = current {
            let current_block = states.block(c);
            // don't preserve air, it's almost always there and almost never wanted
            if substitutes.iter().any(|s| s == current_block) && !states.is_air(c) {
                return states.with_block(desired, current_block).unwrap_or(desired);
            }
        }
        for substitute in substitutes {
            if self.is_air_block(substitute) {
                // can always "place" air
                return match current {
                    Some(c) if states.is_air(c) => c,
                    _ => states.air(),
                };
            }
            for &placeable in approx_placeable {
                if states.block(placeable) == substitute {
                    return states.with_block(desired, states.block(placeable)).unwrap_or(desired);
                }
            }
        }
        states.default_state(first).unwrap_or(desired)
    }

    /// `ISchematic.reset` (not overridden by `SubstituteSchematic`): nothing, and it is not passed on.
    fn reset(&self) {}
    fn width_x(&self) -> i32 {
        self.width_x
    }
    fn height_y(&self) -> i32 {
        self.height_y
    }
    fn length_z(&self) -> i32 {
        self.length_z
    }
}

/// `RotatedSchematic`: the inner schematic turned about the vertical axis.
#[derive(Debug)]
pub struct RotatedSchematic {
    inner: Arc<dyn Schematic>,
    rotation: BlockRotation,
    inverse_rotation: BlockRotation,
    states: Arc<BlockStates>,
}

impl RotatedSchematic {
    /// `RotatedSchematic.<init>`: the inverse is the rotation applied three times.
    pub fn new(inner: Arc<dyn Schematic>, rotation: BlockRotation, states: Arc<BlockStates>) -> RotatedSchematic {
        let inverse_rotation = rotation.get_rotated(rotation).get_rotated(rotation);
        RotatedSchematic { inner, rotation, inverse_rotation, states }
    }

    /// `RotatedSchematic.flipsCoordinates`: whether the rotation swaps x and z.
    fn flips_coordinates(rotation: BlockRotation) -> bool {
        rotation == BlockRotation::Clockwise90 || rotation == BlockRotation::Counterclockwise90
    }

    /// `RotatedSchematic.rotateX`: the turned X.
    /// `RotatedSchematic.rotateX`. The result of `size_z - z - 1` wraps as Java's `int` does.
    fn rotate_x(x: i32, z: i32, size_x: i32, size_z: i32, rotation: BlockRotation) -> i32 {
        match rotation {
            BlockRotation::None => x,
            BlockRotation::Clockwise90 => size_z.wrapping_sub(z).wrapping_sub(1),
            BlockRotation::Clockwise180 => size_x.wrapping_sub(x).wrapping_sub(1),
            BlockRotation::Counterclockwise90 => z,
        }
    }

    /// `RotatedSchematic.rotateZ`: the turned Z.
    /// `RotatedSchematic.rotateZ`.
    fn rotate_z(x: i32, z: i32, size_x: i32, size_z: i32, rotation: BlockRotation) -> i32 {
        match rotation {
            BlockRotation::None => z,
            BlockRotation::Clockwise90 => x,
            BlockRotation::Clockwise180 => size_z.wrapping_sub(z).wrapping_sub(1),
            BlockRotation::Counterclockwise90 => size_x.wrapping_sub(x).wrapping_sub(1),
        }
    }
}

impl Schematic for RotatedSchematic {
    /// `RotatedSchematic.inSchematic`: the inner schematic at the turned position.
    /// `RotatedSchematic.inSchematic`.
    fn in_schematic(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        let (sx, sz) = (self.width_x(), self.length_z());
        self.inner.in_schematic(Self::rotate_x(x, z, sx, sz, self.inverse_rotation), y, Self::rotate_z(x, z, sx, sz, self.inverse_rotation), current.map(|c| self.states.rotate(c, self.inverse_rotation)))
    }

    /// `RotatedSchematic.desiredState`: asks the inner schematic in its own frame (current state and placeable states turned back by
    /// the inverse), then turns the answer.
    fn desired_state(&self, x: i32, y: i32, z: i32, current: Option<StateId>, approx_placeable: &[StateId]) -> StateId {
        let (sx, sz) = (self.width_x(), self.length_z());
        let placeable: Vec<StateId> = approx_placeable.iter().map(|&s| self.states.rotate(s, self.inverse_rotation)).collect();
        let desired = self.inner.desired_state(Self::rotate_x(x, z, sx, sz, self.inverse_rotation), y, Self::rotate_z(x, z, sx, sz, self.inverse_rotation), current.map(|c| self.states.rotate(c, self.inverse_rotation)), &placeable);
        self.states.rotate(desired, self.rotation)
    }

    /// `RotatedSchematic.reset`: passed on.
    fn reset(&self) {
        self.inner.reset();
    }

    /// `RotatedSchematic.widthX`: the inner length for a quarter turn.
    fn width_x(&self) -> i32 {
        if Self::flips_coordinates(self.rotation) {
            self.inner.length_z()
        } else {
            self.inner.width_x()
        }
    }
    /// `RotatedSchematic.heightY`: the height is the inner one.
    fn height_y(&self) -> i32 {
        self.inner.height_y()
    }
    /// `RotatedSchematic.lengthZ`: the inner width for a quarter turn.
    fn length_z(&self) -> i32 {
        if Self::flips_coordinates(self.rotation) {
            self.inner.width_x()
        } else {
            self.inner.length_z()
        }
    }
}

/// `MirroredSchematic`: the inner schematic flipped along x or z.
#[derive(Debug)]
pub struct MirroredSchematic {
    inner: Arc<dyn Schematic>,
    mirror: BlockMirror,
    states: Arc<BlockStates>,
}

impl MirroredSchematic {
    /// `MirroredSchematic.<init>`.
    pub fn new(inner: Arc<dyn Schematic>, mirror: BlockMirror, states: Arc<BlockStates>) -> MirroredSchematic {
        MirroredSchematic { inner, mirror, states }
    }

    /// `MirroredSchematic.mirrorX`: the mirrored X.
    /// `MirroredSchematic.mirrorX`.
    fn mirror_x(x: i32, size_x: i32, mirror: BlockMirror) -> i32 {
        match mirror {
            BlockMirror::None | BlockMirror::LeftRight => x,
            BlockMirror::FrontBack => size_x.wrapping_sub(x).wrapping_sub(1),
        }
    }

    /// `MirroredSchematic.mirrorZ`: the mirrored Z.
    /// `MirroredSchematic.mirrorZ`.
    fn mirror_z(z: i32, size_z: i32, mirror: BlockMirror) -> i32 {
        match mirror {
            BlockMirror::None | BlockMirror::FrontBack => z,
            BlockMirror::LeftRight => size_z.wrapping_sub(z).wrapping_sub(1),
        }
    }
}

impl Schematic for MirroredSchematic {
    /// `MirroredSchematic.inSchematic`: the inner schematic at the mirrored position.
    /// `MirroredSchematic.inSchematic`.
    fn in_schematic(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        self.inner.in_schematic(Self::mirror_x(x, self.width_x(), self.mirror), y, Self::mirror_z(z, self.length_z(), self.mirror), current.map(|c| self.states.mirror(c, self.mirror)))
    }

    /// `MirroredSchematic.desiredState`: asks the inner schematic in its own frame, then flips the answer.
    fn desired_state(&self, x: i32, y: i32, z: i32, current: Option<StateId>, approx_placeable: &[StateId]) -> StateId {
        let placeable: Vec<StateId> = approx_placeable.iter().map(|&s| self.states.mirror(s, self.mirror)).collect();
        let desired = self.inner.desired_state(Self::mirror_x(x, self.width_x(), self.mirror), y, Self::mirror_z(z, self.length_z(), self.mirror), current.map(|c| self.states.mirror(c, self.mirror)), &placeable);
        self.states.mirror(desired, self.mirror)
    }

    /// `MirroredSchematic.reset`: passed on.
    fn reset(&self) {
        self.inner.reset();
    }
    /// `MirroredSchematic.widthX`: the inner size.
    fn width_x(&self) -> i32 {
        self.inner.width_x()
    }
    /// `MirroredSchematic.heightY`: the inner size.
    fn height_y(&self) -> i32 {
        self.inner.height_y()
    }
    /// `MirroredSchematic.lengthZ`: the inner size.
    fn length_z(&self) -> i32 {
        self.inner.length_z()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockmeta::{test_index, BlockOptionalMeta};
    use crate::schematic::basic::{FillSchematic, StaticSchematic};
    use crate::schematic::mask::{SphereMask, StaticMask};
    use std::sync::atomic::AtomicUsize;

    fn ts() -> Arc<BlockStates> {
        Arc::new(BlockStates::test_states())
    }

    fn id(s: &BlockStates, block: &str, props: &[(&str, &str)]) -> StateId {
        (0..s.len()).find(|&i| s.block(i) == block && s.props(i).len() == props.len() && props.iter().all(|(k, v)| s.prop(i, k) == Some(*v))).unwrap()
    }

    /// A static schematic whose state at (x, y, z) is `f(x, y, z)`.
    fn grid(w: i32, h: i32, l: i32, f: impl Fn(i32, i32, i32) -> StateId) -> Arc<StaticSchematic> {
        let mut v = vec![];
        for x in 0..w {
            for z in 0..l {
                for y in 0..h {
                    v.push(f(x, y, z));
                }
            }
        }
        Arc::new(StaticSchematic::new(w, h, l, v, 0))
    }

    #[test]
    fn four_quarter_turns_are_the_identity_on_coordinates_and_states() {
        let s = ts();
        // 3 x 2 x 4 with a different state at every cell: oak_stairs of every facing, so the states turn too
        let stairs: Vec<StateId> = (43..43 + 24).collect();
        let base = grid(3, 2, 4, |x, y, z| stairs[(x * 8 + y * 4 + z) as usize]);
        let mut cur: Arc<dyn Schematic> = base.clone();
        for _ in 0..4 {
            cur = Arc::new(RotatedSchematic::new(cur, BlockRotation::Clockwise90, s.clone()));
        }
        assert_eq!((cur.width_x(), cur.height_y(), cur.length_z()), (3, 2, 4));
        for x in 0..3 {
            for y in 0..2 {
                for z in 0..4 {
                    assert_eq!(cur.desired_state(x, y, z, None, &[]), base.desired_state(x, y, z, None, &[]), "at {x},{y},{z}");
                    assert!(cur.in_schematic(x, y, z, None));
                }
            }
        }
        // one rotation of each kind composed with its inverse
        for (r, inv) in [(BlockRotation::Clockwise90, BlockRotation::Counterclockwise90), (BlockRotation::Clockwise180, BlockRotation::Clockwise180)] {
            let two: Arc<dyn Schematic> = Arc::new(RotatedSchematic::new(Arc::new(RotatedSchematic::new(base.clone(), r, s.clone())), inv, s.clone()));
            for x in 0..3 {
                for z in 0..4 {
                    assert_eq!(two.desired_state(x, 1, z, None, &[]), base.desired_state(x, 1, z, None, &[]));
                }
            }
        }
    }

    #[test]
    fn a_quarter_turn_swaps_the_sizes_and_a_half_turn_does_not() {
        let s = ts();
        let base = grid(3, 2, 4, |_, _, _| 2);
        let r = |rot| RotatedSchematic::new(base.clone(), rot, s.clone());
        assert_eq!((r(BlockRotation::Clockwise90).width_x(), r(BlockRotation::Clockwise90).length_z()), (4, 3));
        assert_eq!((r(BlockRotation::Counterclockwise90).width_x(), r(BlockRotation::Counterclockwise90).length_z()), (4, 3));
        assert_eq!((r(BlockRotation::Clockwise180).width_x(), r(BlockRotation::Clockwise180).length_z()), (3, 4));
        assert_eq!((r(BlockRotation::None).width_x(), r(BlockRotation::None).length_z()), (3, 4));
        assert_eq!(r(BlockRotation::Clockwise90).height_y(), 2);
        // the rotated box is the new box
        assert!(r(BlockRotation::Clockwise90).in_schematic(3, 1, 2, None));
        assert!(!r(BlockRotation::Clockwise90).in_schematic(0, 0, 3, None));
    }

    #[test]
    fn a_rotation_then_a_mirror_matches_a_hand_computed_grid() {
        // inner 3 x 1 x 2 (x = 0..2, z = 0..1), states 2..: stone, cobblestone, dirt, oak_planks, glass, sand
        //   z=0:  x0 = 2 (stone)  x1 = 3 (cobblestone) x2 = 4 (dirt)
        //   z=1:  x0 = 5 (planks) x1 = 6 (glass)       x2 = 7 (sand)
        let s = ts();
        let base = grid(3, 1, 2, |x, _, z| 2 + (z * 3 + x) as usize);
        // CLOCKWISE_90: the result is 2 wide (x) and 3 long (z). rotateX = sizeZ - z - 1 with the inverse CCW90: result (x, z) reads
        // inner (x', z') with the inverse rotation COUNTERCLOCKWISE_90: x' = z, z' = sizeX - x - 1, sizes of the result (2, 3).
        let rot: Arc<dyn Schematic> = Arc::new(RotatedSchematic::new(base, BlockRotation::Clockwise90, s.clone()));
        let at = |sc: &Arc<dyn Schematic>, x, z| sc.desired_state(x, 0, z, None, &[]);
        // result (x, z): x' = z, z' = 2 - x. Hand-computed:
        //   (0,0) -> inner (0, 1) = 5   (1,0) -> inner (0, 0) = 2
        //   (0,1) -> inner (1, 1) = 6   (1,1) -> inner (1, 0) = 3
        //   (0,2) -> inner (2, 1) = 7   (1,2) -> inner (2, 0) = 4
        assert_eq!([[at(&rot, 0, 0), at(&rot, 1, 0)], [at(&rot, 0, 1), at(&rot, 1, 1)], [at(&rot, 0, 2), at(&rot, 1, 2)]], [[5, 2], [6, 3], [7, 4]]);
        // then FRONT_BACK (flip x of the 2 wide result): (x, z) reads rotated (1 - x, z)
        let mir: Arc<dyn Schematic> = Arc::new(MirroredSchematic::new(rot.clone(), BlockMirror::FrontBack, s.clone()));
        assert_eq!([[at(&mir, 0, 0), at(&mir, 1, 0)], [at(&mir, 0, 1), at(&mir, 1, 1)], [at(&mir, 0, 2), at(&mir, 1, 2)]], [[2, 5], [3, 6], [4, 7]]);
        // LEFT_RIGHT flips z of the 3 long result
        let lr: Arc<dyn Schematic> = Arc::new(MirroredSchematic::new(rot, BlockMirror::LeftRight, s));
        assert_eq!([[at(&lr, 0, 0), at(&lr, 1, 0)], [at(&lr, 0, 1), at(&lr, 1, 1)], [at(&lr, 0, 2), at(&lr, 1, 2)]], [[7, 4], [6, 3], [5, 2]]);
        assert_eq!((lr.width_x(), lr.height_y(), lr.length_z()), (2, 1, 3));
    }

    #[test]
    fn states_turn_with_the_schematic() {
        let s = ts();
        let north = id(&s, "furnace", &[("facing", "north")]);
        let east = id(&s, "furnace", &[("facing", "east")]);
        let west = id(&s, "furnace", &[("facing", "west")]);
        let base = grid(1, 1, 1, |_, _, _| north);
        let cw: Arc<dyn Schematic> = Arc::new(RotatedSchematic::new(base.clone(), BlockRotation::Clockwise90, s.clone()));
        assert_eq!(cw.desired_state(0, 0, 0, None, &[]), east);
        let mirrored: Arc<dyn Schematic> = Arc::new(MirroredSchematic::new(grid(1, 1, 1, |_, _, _| east), BlockMirror::FrontBack, s.clone()));
        assert_eq!(mirrored.desired_state(0, 0, 0, None, &[]), west);
    }

    type Asked = (i32, i32, i32, Option<StateId>, Vec<StateId>);

    /// A schematic that records what it was asked, and answers with a fixed state.
    #[derive(Debug)]
    struct Spy {
        seen: std::sync::Mutex<Vec<Asked>>,
    }
    impl Schematic for Spy {
        fn desired_state(&self, x: i32, y: i32, z: i32, c: Option<StateId>, a: &[StateId]) -> StateId {
            self.seen.lock().unwrap().push((x, y, z, c, a.to_vec()));
            0
        }
        fn width_x(&self) -> i32 {
            2
        }
        fn height_y(&self) -> i32 {
            1
        }
        fn length_z(&self) -> i32 {
            3
        }
    }

    #[test]
    fn the_inner_schematic_sees_the_inverse_turned_world() {
        let s = ts();
        let north = id(&s, "furnace", &[("facing", "north")]);
        let east = id(&s, "furnace", &[("facing", "east")]);
        let south = id(&s, "furnace", &[("facing", "south")]);
        let west = id(&s, "furnace", &[("facing", "west")]);
        let spy = Arc::new(Spy { seen: Default::default() });
        // clockwise 90: the inverse is counterclockwise 90, which turns east into north and north into west
        let r = RotatedSchematic::new(spy.clone(), BlockRotation::Clockwise90, s.clone());
        r.desired_state(1, 0, 0, Some(east), &[north, south]);
        let seen = spy.seen.lock().unwrap().clone();
        // result is 3 x 2 (x, z); inverse CCW90 reads inner (z, sizeX - x - 1) = (0, 3 - 1 - 1) = (0, 1)
        assert_eq!(seen, vec![(0, 0, 1, Some(north), vec![west, east])]);
    }

    #[test]
    fn shell_and_walls_on_a_3x3x3_box() {
        let inner = grid(3, 3, 3, |_, _, _| 2);
        let shell = MaskSchematic::shell(inner.clone());
        let walls = MaskSchematic::walls(inner);
        let mut shell_count = 0;
        let mut walls_count = 0;
        for x in 0..3 {
            for y in 0..3 {
                for z in 0..3 {
                    let edge_xz = x != 1 || z != 1;
                    if shell.in_schematic(x, y, z, None) {
                        shell_count += 1;
                    }
                    if walls.in_schematic(x, y, z, None) {
                        walls_count += 1;
                    }
                    assert_eq!(walls.in_schematic(x, y, z, None), edge_xz);
                    assert_eq!(shell.in_schematic(x, y, z, None), edge_xz || y != 1);
                }
            }
        }
        assert_eq!(shell_count, 26); // everything but the centre
        assert_eq!(walls_count, 24); // 8 columns of 3
        assert!(!shell.in_schematic(3, 0, 0, None) && !walls.in_schematic(-1, 0, 0, None));
        assert_eq!(shell.desired_state(1, 1, 1, None, &[]), 2);
        assert_eq!((shell.width_x(), shell.height_y(), shell.length_z()), (3, 3, 3));
    }

    #[test]
    fn a_mask_schematic_uses_the_mask_and_does_not_reset_the_inner_one() {
        #[derive(Debug)]
        struct Counting(Arc<AtomicUsize>);
        impl Schematic for Counting {
            fn desired_state(&self, _: i32, _: i32, _: i32, _: Option<StateId>, _: &[StateId]) -> StateId {
                3
            }
            fn reset(&self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            fn width_x(&self) -> i32 {
                5
            }
            fn height_y(&self) -> i32 {
                5
            }
            fn length_z(&self) -> i32 {
                5
            }
        }
        let resets = Arc::new(AtomicUsize::new(0));
        let sphere: Arc<dyn Mask> = Arc::new(SphereMask::new(5, 5, 5, true));
        let m = MaskSchematic::create(Arc::new(Counting(resets.clone())), sphere);
        assert!(m.in_schematic(2, 2, 2, None) && !m.in_schematic(0, 0, 0, None) && !m.in_schematic(5, 2, 2, None));
        assert_eq!(m.desired_state(0, 0, 0, None, &[]), 3);
        m.reset();
        assert_eq!(resets.load(Ordering::SeqCst), 0, "upstream does not pass reset to the inner schematic of a mask");
        let sphere = SphereMask::new(5, 5, 5, true);
        assert!(sphere.part_of_mask_static(2, 2, 2));
    }

    #[test]
    fn replace_is_sticky_until_reset_and_shared_by_threads() {
        let idx = test_index();
        let inner = grid(2, 1, 1, |_, _, _| 1);
        let filter = BlockOptionalMetaLookup::from_names(&["stone"], &idx).unwrap(); // state 1
        let r = Arc::new(MaskSchematic::replace(inner, filter));
        // first answer: stone is there, so it is part of the mask
        assert!(r.in_schematic(0, 0, 0, Some(1)));
        // the world changed to air (0), but the cell keeps its first answer
        assert!(r.in_schematic(0, 0, 0, Some(0)));
        // a cell first seen as air stays out, even when stone appears
        assert!(!r.in_schematic(1, 0, 0, Some(0)));
        assert!(!r.in_schematic(1, 0, 0, Some(1)));
        // None is in no filter
        r.reset();
        assert!(!r.in_schematic(0, 0, 0, Some(0)));
        assert!(r.in_schematic(1, 0, 0, Some(1)));
        assert!(!r.in_schematic(0, 0, 0, None));
        // outside the box is false and does not panic
        assert!(!r.in_schematic(-1, 0, 0, Some(1)) && !r.in_schematic(2, 0, 0, Some(1)));
        // two threads on one object: the first answer wins and both agree afterwards
        r.reset();
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let r = r.clone();
                std::thread::spawn(move || {
                    let mut firsts = vec![];
                    for _ in 0..1000 {
                        firsts.push(r.in_schematic(0, 0, 0, Some(if t % 2 == 0 { 1 } else { 0 })));
                    }
                    firsts
                })
            })
            .collect();
        let results: Vec<Vec<bool>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let settled = r.in_schematic(0, 0, 0, Some(0));
        for res in results {
            // whatever answer was stored first, each thread's last answers are the settled one
            assert_eq!(*res.last().unwrap(), settled);
        }
    }

    #[test]
    fn replace_reset_clears_every_cell() {
        let idx = test_index();
        let inner = grid(2, 2, 2, |_, _, _| 1);
        let r = MaskSchematic::replace(inner, BlockOptionalMetaLookup::from_names(&["stone"], &idx).unwrap());
        for x in 0..2 {
            for y in 0..2 {
                for z in 0..2 {
                    assert!(r.in_schematic(x, y, z, Some(1)));
                }
            }
        }
        r.reset();
        for x in 0..2 {
            for y in 0..2 {
                for z in 0..2 {
                    assert!(!r.in_schematic(x, y, z, Some(0)));
                }
            }
        }
    }

    fn sub(map: &[(&str, &[&str])], inner: Arc<dyn Schematic>, s: &Arc<BlockStates>) -> SubstituteSchematic {
        SubstituteSchematic::new(inner, map.iter().map(|(k, v)| (k.to_string(), v.iter().map(|x| x.to_string()).collect())).collect(), s.clone())
    }

    #[test]
    fn substitute_every_branch() {
        let s = ts();
        let stone = 2;
        let cobble = 3;
        let dirt = 4;
        let planks = 5;
        let inner = grid(1, 1, 1, |_, _, _| stone);
        // the desired block has no substitutes: unchanged
        let other = sub(&[("dirt", &["sand"])], inner.clone(), &s);
        assert_eq!(other.desired_state(0, 0, 0, Some(0), &[]), stone);
        // the block that is there is one of the substitutes: kept (and not air)
        let m = sub(&[("stone", &["cobblestone", "dirt"])], inner.clone(), &s);
        assert_eq!(m.desired_state(0, 0, 0, Some(cobble), &[dirt]), cobble);
        // a substitute is placeable: the first substitute (in list order) that is, not the first placeable
        assert_eq!(m.desired_state(0, 0, 0, Some(0), &[dirt, cobble]), cobble);
        assert_eq!(m.desired_state(0, 0, 0, Some(0), &[dirt]), dirt);
        // nothing placeable: the default state of the first substitute
        assert_eq!(m.desired_state(0, 0, 0, Some(0), &[planks]), cobble);
        // current is None: nothing is kept
        assert_eq!(m.desired_state(0, 0, 0, None, &[dirt]), dirt);
        // an air substitute: air where it is air, else the air state
        let air = sub(&[("stone", &["air"])], inner.clone(), &s);
        assert_eq!(air.desired_state(0, 0, 0, Some(1), &[]), 1);
        assert_eq!(air.desired_state(0, 0, 0, Some(stone), &[]), 0);
        assert_eq!(air.desired_state(0, 0, 0, None, &[]), 0);
        // the current block is a substitute but is air: not preserved
        let air_first = sub(&[("stone", &["cave_air", "dirt"])], inner.clone(), &s);
        assert_eq!(air_first.desired_state(0, 0, 0, Some(1), &[dirt]), 1);
        // an earlier substitute that is not placeable does not stop a later air one, and the order is the list's
        let order = sub(&[("stone", &["dirt", "air"])], inner.clone(), &s);
        assert_eq!(order.desired_state(0, 0, 0, Some(stone), &[]), 0);
        assert_eq!(order.desired_state(0, 0, 0, Some(stone), &[dirt]), dirt);
        // names with a namespace work
        let ns = sub(&[("minecraft:stone", &["minecraft:cobblestone"])], inner.clone(), &s);
        assert_eq!(ns.desired_state(0, 0, 0, Some(0), &[]), cobble);
        // an empty list returns the desired state
        let empty = sub(&[("stone", &[])], inner, &s);
        assert_eq!(empty.desired_state(0, 0, 0, Some(0), &[]), stone);
        assert_eq!((empty.width_x(), empty.height_y(), empty.length_z()), (1, 1, 1));
        assert!(empty.in_schematic(0, 0, 0, None) && !empty.in_schematic(1, 0, 0, None));
    }

    #[test]
    fn substitute_keeps_the_properties_through_with_block() {
        let s = ts();
        let stairs = id(&s, "oak_stairs", &[("facing", "east"), ("half", "top"), ("shape", "inner_left"), ("waterlogged", "false")]);
        let inner = grid(1, 1, 1, |_, _, _| stairs);
        let m = sub(&[("oak_stairs", &["oak_planks", "furnace"])], inner.clone(), &s);
        // planks are placeable: the stairs become planks
        assert_eq!(m.desired_state(0, 0, 0, Some(0), &[5]), 5);
        // a furnace is placeable: facing carries over
        let furnace_east = id(&s, "furnace", &[("facing", "east")]);
        let furnace_south = id(&s, "furnace", &[("facing", "south")]);
        assert_eq!(m.desired_state(0, 0, 0, Some(0), &[furnace_south]), furnace_east);
        // a furnace is already there
        assert_eq!(m.desired_state(0, 0, 0, Some(furnace_south), &[]), furnace_east);
        // stairs to stairs of another block would copy all four properties (same block here)
        let same = sub(&[("oak_stairs", &["oak_stairs"])], inner, &s);
        assert_eq!(same.desired_state(0, 0, 0, Some(stairs), &[]), stairs);
    }

    #[test]
    fn map_art_heights_and_the_missing_column_message() {
        let s = BlockStates::test_states();
        // 2 x 3 x 2: column (0,0) top block at y=1, column (0,1) all air, column (1,0) top at y=2, column (1,1) has a block at y=0 only
        let schem = grid(2, 3, 2, |x, y, z| match (x, z) {
            (0, 0) => [2, 2, 0][y as usize],
            (0, 1) => 0,
            (1, 0) => [0, 0, 5][y as usize],
            _ => [3, 0, 0][y as usize],
        });
        let (m, lines) = MaskSchematic::map_art(schem, &s).unwrap();
        assert_eq!(lines, vec!["1 columns had no block despite being in a map art, letting them be whatever".to_string()]);
        // the mask is y >= top, and below the top it does not matter
        for y in 0..3 {
            assert_eq!(m.in_schematic(0, y, 0, None), y >= 1);
            assert!(!m.in_schematic(0, y, 1, None)); // the empty column has height i32::MAX: nothing in it is built (U27)
            assert_eq!(m.in_schematic(1, y, 0, None), y >= 2);
            assert_eq!(m.in_schematic(1, y, 1, None), y >= 0);
        }
        assert_eq!(m.desired_state(1, 2, 0, None, &[]), 5);
        assert_eq!((m.width_x(), m.height_y(), m.length_z()), (2, 3, 2));
    }

    #[test]
    fn map_art_with_no_missing_column_prints_nothing_and_a_gappy_composite_is_an_error() {
        let s = BlockStates::test_states();
        let schem = grid(1, 2, 1, |_, y, _| [2, 2][y as usize]);
        let (_, lines) = MaskSchematic::map_art(schem, &s).unwrap();
        assert!(lines.is_empty());
        let mut c = crate::schematic::basic::CompositeSchematic::new(2, 1, 1, 0);
        c.put(grid(1, 1, 1, |_, _, _| 2), 0, 0, 0);
        let err = MaskSchematic::map_art(Arc::new(c), &s).unwrap_err();
        assert_eq!(err, "couldn't find schematic for this position");
    }

    #[test]
    fn fill_through_a_mask_chain_compiles_with_the_contract() {
        // the types of the contract fit together: a fill, masked by a sphere, then shelled
        let idx = test_index();
        let fill: Arc<dyn Schematic> = Arc::new(FillSchematic::new(5, 5, 5, BlockOptionalMeta::parse("stone", &idx).unwrap()));
        let sphere: Arc<dyn Mask> = Arc::new(SphereMask::new(5, 5, 5, true));
        let masked: Arc<dyn Schematic> = Arc::new(MaskSchematic::create(fill, sphere));
        let shell = MaskSchematic::shell(masked);
        assert!(shell.in_schematic(2, 0, 2, None)); // on the floor of the box and inside the sphere's lowest layer
        assert!(!shell.in_schematic(0, 0, 0, None));
        assert!(!shell.in_schematic(2, 2, 2, None)); // the centre is not on the shell
    }
}
