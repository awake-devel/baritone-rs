// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/schematic/StaticSchematic.java and
// src/api/java/baritone/api/schematic/{AbstractSchematic,FillSchematic,CompositeSchematic,CompositeSchematicEntry}.java.
// LGPL-3.0-or-later, see lib.rs.
//
// The schematics that hold or make their own blocks. `AbstractSchematic` only holds the three sizes, so each type here holds its own.
use super::{Schematic, StateId, StaticSchematicView};
use crate::blockmeta::BlockOptionalMeta;
use std::sync::Arc;

/// `StaticSchematic`: a box of states, fixed. Upstream's `states[x][z][y]` is one vector here.
pub struct StaticSchematic {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    states: Vec<StateId>,
    air: StateId,
}

impl std::fmt::Debug for StaticSchematic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StaticSchematic({}x{}x{})", self.width_x, self.height_y, self.length_z)
    }
}

impl StaticSchematic {
    /// `StaticSchematic.<init>(BlockState[][][])`. `states[((x * length_z) + z) * height_y + y]` is upstream's `states[x][z][y]`.
    /// As upstream, a box with an empty side has all three sizes 0. A vector of the wrong length is a `debug_assert` failure, and in a
    /// release build every position the vector does not cover reads as `air`.
    pub fn new(width_x: i32, height_y: i32, length_z: i32, states: Vec<StateId>, air: StateId) -> StaticSchematic {
        let empty = width_x <= 0 || height_y <= 0 || length_z <= 0;
        let (w, h, l) = if empty { (0, 0, 0) } else { (width_x, height_y, length_z) };
        StaticSchematic::with_raw_sizes(w, h, l, states, air)
    }

    /// The MCEdit and Sponge readers: they set `x`, `y` and `z` from the file's `Width`, `Height` and `Length` and never call the
    /// `StaticSchematic(BlockState[][][])` constructor, so a box with one empty side keeps its other sizes (a 0 x 5 x 5 file is 0 x 5 x 5).
    /// The sizes are not negative and, when one is 0, `states` is empty. Every position of such a box is outside it for `get_direct`
    /// and `get_column`.
    pub fn with_raw_sizes(width_x: i32, height_y: i32, length_z: i32, states: Vec<StateId>, air: StateId) -> StaticSchematic {
        debug_assert_eq!(states.len() as u64, width_x.max(0) as u64 * height_y.max(0) as u64 * length_z.max(0) as u64, "StaticSchematic: wrong number of states");
        StaticSchematic { width_x, height_y, length_z, states, air }
    }

    /// The index of a position, or `None` outside the box.
    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        if x < 0 || y < 0 || z < 0 || x >= self.width_x || y >= self.height_y || z >= self.length_z {
            return None;
        }
        Some(((x as usize * self.length_z as usize) + z as usize) * self.height_y as usize + y as usize)
    }

    fn at(&self, x: i32, y: i32, z: i32) -> StateId {
        // upstream throws ArrayIndexOutOfBoundsException outside the box
        self.index(x, y, z).and_then(|i| self.states.get(i)).copied().unwrap_or(self.air)
    }
}

impl Schematic for StaticSchematic {
    /// `StaticSchematic.desiredState`: the stored state.
    /// `StaticSchematic.desiredState`.
    fn desired_state(&self, x: i32, y: i32, z: i32, _current: Option<StateId>, _approx_placeable: &[StateId]) -> StateId {
        self.at(x, y, z)
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

impl StaticSchematicView for StaticSchematic {
    /// `StaticSchematic.getDirect`: the stored state.
    /// `StaticSchematic.getDirect`. `None` outside the box, where upstream throws.
    fn get_direct(&self, x: i32, y: i32, z: i32) -> Option<StateId> {
        self.index(x, y, z).and_then(|i| self.states.get(i)).copied()
    }

    /// `StaticSchematic.getColumn`: `states[x][z]`. `None` outside the box, where upstream throws.
    fn get_column(&self, x: i32, z: i32) -> Option<Vec<StateId>> {
        let start = self.index(x, 0, z)?;
        self.states.get(start..start + self.height_y as usize).map(|s| s.to_vec())
    }
}

/// `FillSchematic`: every position wants the blocks a selector names.
#[derive(Debug)]
pub struct FillSchematic {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    bom: BlockOptionalMeta,
}

impl FillSchematic {
    /// `FillSchematic.<init>(int, int, int, BlockOptionalMeta)`.
    pub fn new(x: i32, y: i32, z: i32, bom: BlockOptionalMeta) -> FillSchematic {
        FillSchematic { width_x: x, height_y: y, length_z: z, bom }
    }

    /// `FillSchematic.getBom`: the selector.
    /// `FillSchematic.getBom`.
    pub fn bom(&self) -> &BlockOptionalMeta {
        &self.bom
    }
}

impl Schematic for FillSchematic {
    /// `FillSchematic.desiredState`: the current state if it matches, else the first placeable state that does, else any state of the
    /// selector. A current state of `None` matches nothing (upstream throws a `NullPointerException`). A selector with no state at all
    /// cannot be made from the settings; it gives state 0, which is air in the game's table (upstream returns null).
    fn desired_state(&self, _x: i32, _y: i32, _z: i32, current: Option<StateId>, approx_placeable: &[StateId]) -> StateId {
        if let Some(c) = current {
            if self.bom.matches_state(c) {
                return c;
            }
        }
        for &placeable in approx_placeable {
            if self.bom.matches_state(placeable) {
                return placeable;
            }
        }
        self.bom.any_block_state().unwrap_or(0)
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

/// `CompositeSchematicEntry`.
#[derive(Debug)]
struct Entry {
    schematic: Arc<dyn Schematic>,
    x: i32,
    y: i32,
    z: i32,
}

/// `CompositeSchematic`: several schematics, each at an offset. The box is as big as the entries reach.
#[derive(Debug)]
pub struct CompositeSchematic {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    entries: Vec<Entry>,
    air: StateId,
}

impl CompositeSchematic {
    /// `CompositeSchematic.<init>`. `air` is what a position in no entry wants (upstream throws; the builder never asks there, because
    /// `in_schematic` is false).
    pub fn new(x: i32, y: i32, z: i32, air: StateId) -> CompositeSchematic {
        CompositeSchematic { width_x: x, height_y: y, length_z: z, entries: vec![], air }
    }

    /// `CompositeSchematic.recalcArr`: the box grows to hold the new entry.
    /// `CompositeSchematic.put`: adds an entry, then `recalcArr` grows the box to reach it. The sums wrap as Java's `int` does.
    pub fn put(&mut self, extra: Arc<dyn Schematic>, x: i32, y: i32, z: i32) {
        // `recalcArr` takes the maximum over every entry, starting from the box it already has, so only the new entry can raise it: the same
        // box, and a parse that costs the number of entries and not its square (D50)
        self.width_x = self.width_x.max(x.wrapping_add(extra.width_x()));
        self.height_y = self.height_y.max(y.wrapping_add(extra.height_y()));
        self.length_z = self.length_z.max(z.wrapping_add(extra.length_z()));
        self.entries.push(Entry { schematic: extra, x, y, z });
    }

    /// `CompositeSchematic.getSchematic`: the first entry whose box starts at or before the position and whose schematic says the
    /// position is in it. As upstream there is no upper bound on the entry's box here: the entry's own `inSchematic` decides.
    fn get_schematic(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> Option<&Entry> {
        self.entries.iter().find(|e| x >= e.x && y >= e.y && z >= e.z && e.schematic.in_schematic(x.wrapping_sub(e.x), y.wrapping_sub(e.y), z.wrapping_sub(e.z), current))
    }
}

impl Schematic for CompositeSchematic {
    /// `CompositeSchematic.inSchematic`: the entry found asks again.
    fn in_schematic(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        match self.get_schematic(x, y, z, current) {
            Some(e) => e.schematic.in_schematic(x.wrapping_sub(e.x), y.wrapping_sub(e.y), z.wrapping_sub(e.z), current),
            None => false,
        }
    }

    /// `CompositeSchematic.desiredState`: the entry's answer. With no entry upstream throws `IllegalStateException`; here it is `air`.
    fn desired_state(&self, x: i32, y: i32, z: i32, current: Option<StateId>, approx_placeable: &[StateId]) -> StateId {
        match self.get_schematic(x, y, z, current) {
            Some(e) => e.schematic.desired_state(x.wrapping_sub(e.x), y.wrapping_sub(e.y), z.wrapping_sub(e.z), current, approx_placeable),
            None => self.air,
        }
    }

    /// `CompositeSchematic.reset`: passed to every entry.
    fn reset(&self) {
        for e in &self.entries {
            e.schematic.reset();
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

impl StaticSchematicView for CompositeSchematic {
    /// What `LitematicaSchematic.getDirect` does: `desiredState(x, y, z, null, emptyList)`. `None` where the position is in no entry,
    /// which upstream throws for.
    fn get_direct(&self, x: i32, y: i32, z: i32) -> Option<StateId> {
        let e = self.get_schematic(x, y, z, None)?;
        Some(e.schematic.desired_state(x.wrapping_sub(e.x), y.wrapping_sub(e.y), z.wrapping_sub(e.z), None, &[]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockmeta::{test_index, BlockOptionalMeta};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A schematic that counts its resets and answers a fixed state.
    #[derive(Debug)]
    struct Probe {
        w: i32,
        h: i32,
        l: i32,
        state: StateId,
        resets: Arc<AtomicUsize>,
    }
    impl Schematic for Probe {
        fn desired_state(&self, _x: i32, _y: i32, _z: i32, _c: Option<StateId>, _a: &[StateId]) -> StateId {
            self.state
        }
        fn reset(&self) {
            self.resets.fetch_add(1, Ordering::SeqCst);
        }
        fn width_x(&self) -> i32 {
            self.w
        }
        fn height_y(&self) -> i32 {
            self.h
        }
        fn length_z(&self) -> i32 {
            self.l
        }
    }

    #[test]
    fn static_schematic_indexes_x_then_z_then_y() {
        // 2 wide, 3 high, 2 long: state = 100*x + 10*z + y
        let mut states = vec![];
        for x in 0..2 {
            for z in 0..2 {
                for y in 0..3 {
                    states.push(100 * x + 10 * z + y);
                }
            }
        }
        let s = StaticSchematic::new(2, 3, 2, states, 0);
        assert_eq!((s.width_x(), s.height_y(), s.length_z()), (2, 3, 2));
        assert_eq!(s.desired_state(1, 2, 0, None, &[]), 102);
        assert_eq!(s.get_direct(0, 1, 1), Some(11));
        assert_eq!(s.get_column(1, 1), Some(vec![110, 111, 112]));
        assert_eq!(s.get_direct(2, 0, 0), None);
        assert_eq!(s.get_direct(0, -1, 0), None);
        assert_eq!(s.get_column(0, 2), None);
        assert!(s.in_schematic(1, 2, 1, None) && !s.in_schematic(2, 0, 0, None) && !s.in_schematic(0, 3, 0, None) && !s.in_schematic(-1, 0, 0, None));
        // outside the box reads as air, never panics
        assert_eq!(s.desired_state(9, 9, 9, None, &[]), 0);
    }

    #[test]
    fn a_box_with_an_empty_side_has_no_size() {
        let s = StaticSchematic::new(3, 0, 3, vec![], 0);
        assert_eq!((s.width_x(), s.height_y(), s.length_z()), (0, 0, 0));
        assert!(!s.in_schematic(0, 0, 0, None));
        assert_eq!(s.get_column(0, 0), None);
    }

    #[test]
    fn fill_takes_the_current_state_then_a_placeable_one_then_any() {
        let idx = test_index();
        let bom = BlockOptionalMeta::parse("oak_log[axis=y]", &idx).unwrap(); // state 5
        let f = FillSchematic::new(2, 2, 2, bom);
        assert_eq!(f.bom().all_block_states(), &[5]);
        // the current state matches
        assert_eq!(f.desired_state(0, 0, 0, Some(5), &[1, 5]), 5);
        // it does not: the first placeable that matches, not the first placeable
        assert_eq!(f.desired_state(0, 0, 0, Some(1), &[1, 4, 5]), 5);
        // nothing placeable matches: any state of the selector
        assert_eq!(f.desired_state(0, 0, 0, Some(1), &[1, 2]), 5);
        // a plain selector with several states keeps a current one that matches, and picks the lowest otherwise
        let any = FillSchematic::new(1, 1, 1, BlockOptionalMeta::parse("oak_log", &idx).unwrap());
        assert_eq!(any.desired_state(0, 0, 0, Some(6), &[]), 6);
        assert_eq!(any.desired_state(0, 0, 0, Some(1), &[]), 4);
        // a current state of None matches nothing
        assert_eq!(any.desired_state(0, 0, 0, None, &[6]), 6);
        assert_eq!(any.desired_state(0, 0, 0, None, &[]), 4);
        assert_eq!((f.width_x(), f.height_y(), f.length_z()), (2, 2, 2));
        assert!(f.in_schematic(1, 1, 1, None) && !f.in_schematic(2, 0, 0, None));
    }

    fn probe(w: i32, h: i32, l: i32, state: StateId, resets: &Arc<AtomicUsize>) -> Arc<dyn Schematic> {
        Arc::new(Probe { w, h, l, state, resets: resets.clone() })
    }

    #[test]
    fn composite_first_entry_wins_and_the_box_grows() {
        let resets = Arc::new(AtomicUsize::new(0));
        let mut c = CompositeSchematic::new(0, 0, 0, 0);
        assert_eq!((c.width_x(), c.height_y(), c.length_z()), (0, 0, 0));
        c.put(probe(2, 2, 2, 7, &resets), 0, 0, 0);
        assert_eq!((c.width_x(), c.height_y(), c.length_z()), (2, 2, 2));
        // overlaps the first at (1, 1, 1)
        c.put(probe(3, 1, 3, 9, &resets), 1, 1, 1);
        assert_eq!((c.width_x(), c.height_y(), c.length_z()), (4, 2, 4));
        assert_eq!(c.desired_state(1, 1, 1, None, &[]), 7); // first entry wins
        assert_eq!(c.desired_state(3, 1, 3, None, &[]), 9);
        assert!(c.in_schematic(3, 1, 3, None));
        // a gap: (3, 0, 0) is in neither entry
        assert!(!c.in_schematic(3, 0, 0, None));
        assert_eq!(c.desired_state(3, 0, 0, None, &[]), 0); // air, where upstream throws
        assert_eq!(c.get_direct(3, 0, 0), None);
        assert_eq!(c.get_direct(1, 1, 1), Some(7));
        assert_eq!(c.get_column(3, 3), None);
        assert_eq!(c.get_column(0, 0), Some(vec![7, 7]));
        // a position before the entry's corner is not in it
        assert!(!c.in_schematic(-1, 0, 0, None));
    }

    #[test]
    fn composite_reset_reaches_every_entry() {
        let resets = Arc::new(AtomicUsize::new(0));
        let mut c = CompositeSchematic::new(1, 1, 1, 0);
        c.put(probe(1, 1, 1, 1, &resets), 0, 0, 0);
        c.put(probe(1, 1, 1, 2, &resets), 5, 0, 0);
        c.reset();
        assert_eq!(resets.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn composite_keeps_the_size_it_was_made_with_if_larger() {
        let resets = Arc::new(AtomicUsize::new(0));
        let mut c = CompositeSchematic::new(10, 10, 10, 0);
        c.put(probe(2, 2, 2, 7, &resets), 1, 1, 1);
        assert_eq!((c.width_x(), c.height_y(), c.length_z()), (10, 10, 10));
    }
}
