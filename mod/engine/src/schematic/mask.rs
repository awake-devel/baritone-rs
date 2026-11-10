// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/schematic/mask/{Mask,StaticMask,AbstractMask,PreComputedMask}.java,
// mask/operator/{NotMask,BinaryOperatorMask}.java, mask/shape/{SphereMask,CylinderMask}.java and
// src/api/java/baritone/api/utils/BooleanBinaryOperator{,s}.java. LGPL-3.0-or-later, see lib.rs.
//
// A mask says which positions of a box are part of it. Upstream's `default` methods that build new masks (`Mask.not`, `union`,
// `intersection`, `xor`, `StaticMask.compute`) are the free functions at the end, because the trait objects are shared by `Arc`.
// `AbstractMask` only holds the three sizes, so each mask here holds its own.
use super::{Axis, StateId};
use std::sync::Arc;

/// `Mask`: is this position part of the mask? `current` is the state in the world, if known.
pub trait Mask: Send + Sync + std::fmt::Debug {
    /// `Mask.partOfMask`: a mask's answer for a cell.
    /// `Mask.partOfMask`.
    fn part_of_mask(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool;
    /// `Mask.widthX`: size of the mask's box.
    /// `Mask.widthX`.
    fn width_x(&self) -> i32;
    /// `Mask.heightY`: size of the mask's box.
    /// `Mask.heightY`.
    fn height_y(&self) -> i32;
    /// `Mask.lengthZ`: size of the mask's box.
    /// `Mask.lengthZ`.
    fn length_z(&self) -> i32;
}

/// `StaticMask`: a mask that does not depend on the world.
pub trait StaticMask: Mask {
    /// `StaticMask.partOfMask(int, int, int)`.
    fn part_of_mask_static(&self, x: i32, y: i32, z: i32) -> bool;
}

/// `BooleanBinaryOperators`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanBinaryOperators {
    Or,
    And,
    Xor,
}

impl BooleanBinaryOperators {
    /// `BooleanBinaryOperators.applyAsBoolean`: the three operators.
    /// `BooleanBinaryOperator.applyAsBoolean`: the interface method.
    /// `BooleanBinaryOperator.applyAsBoolean`.
    pub fn apply(self, a: bool, b: bool) -> bool {
        match self {
            BooleanBinaryOperators::Or => a || b,
            BooleanBinaryOperators::And => a && b,
            BooleanBinaryOperators::Xor => a ^ b,
        }
    }
}

/// The number of cells above which [`compute`] does not build a table: 2^28 cells is 256 MiB. Upstream has no limit and would run out of
/// memory; here such a mask reads `false` everywhere.
const MAX_PRECOMPUTED_CELLS: usize = 1 << 28;

/// `PreComputedMask`: every answer of a static mask, kept in a table indexed `[y][z][x]` as upstream's.
#[derive(Debug)]
struct PreComputedMask {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    mask: Vec<bool>,
}

impl PreComputedMask {
    /// `PreComputedMask.<init>`.
    fn new(source: &dyn StaticMask) -> PreComputedMask {
        let (w, h, l) = (source.width_x(), source.height_y(), source.length_z());
        let cells = if w > 0 && h > 0 && l > 0 { (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(l as usize)).filter(|&n| n <= MAX_PRECOMPUTED_CELLS) } else { Some(0) };
        let mut mask = Vec::new();
        if let Some(n) = cells {
            mask.reserve_exact(n);
            for y in 0..h.max(0) {
                for z in 0..l.max(0) {
                    for x in 0..w.max(0) {
                        mask.push(source.part_of_mask_static(x, y, z));
                    }
                }
            }
        }
        PreComputedMask { width_x: w, height_y: h, length_z: l, mask }
    }

    fn get(&self, x: i32, y: i32, z: i32) -> bool {
        if x < 0 || y < 0 || z < 0 || x >= self.width_x || y >= self.height_y || z >= self.length_z {
            return false; // upstream throws ArrayIndexOutOfBoundsException
        }
        let (w, l) = (self.width_x as usize, self.length_z as usize);
        self.mask.get((y as usize * l + z as usize) * w + x as usize).copied().unwrap_or(false)
    }
}

/// `StaticMask.compute`: the mask with every answer worked out once.
pub fn compute(m: &dyn StaticMask) -> Arc<dyn StaticMask> {
    Arc::new(PreComputedMask::new(m))
}

/// Implements `Mask` and `StaticMask` for a type that has `width_x`, `height_y`, `length_z` fields and a `get(x, y, z)` method:
/// the static `Mask.partOfMask` ignores the state, as `StaticMask`'s default does.
macro_rules! static_mask {
    ($t:ty) => {
        impl Mask for $t {
            fn part_of_mask(&self, x: i32, y: i32, z: i32, _current: Option<StateId>) -> bool {
                self.part_of_mask_static(x, y, z)
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
    };
}

static_mask!(PreComputedMask);
impl StaticMask for PreComputedMask {
    /// `PreComputedMask.partOfMask`: a read of the table.
    /// `PreComputedMask.partOfMask`.
    fn part_of_mask_static(&self, x: i32, y: i32, z: i32) -> bool {
        self.get(x, y, z)
    }
}

/// `NotMask`.
#[derive(Debug)]
struct NotMask {
    source: Arc<dyn Mask>,
}

impl Mask for NotMask {
    /// `NotMask.partOfMask`: the opposite of the inner mask.
    /// `NotMask.partOfMask`.
    fn part_of_mask(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        !self.source.part_of_mask(x, y, z, current)
    }
    fn width_x(&self) -> i32 {
        self.source.width_x()
    }
    fn height_y(&self) -> i32 {
        self.source.height_y()
    }
    fn length_z(&self) -> i32 {
        self.source.length_z()
    }
}

/// `Mask.not`: a mask that answers the opposite.
/// `Mask.not`.
pub fn not(m: Arc<dyn Mask>) -> Arc<dyn Mask> {
    Arc::new(NotMask { source: m })
}

/// `NotMask.Static`.
#[derive(Debug)]
struct NotStaticMask {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    source: Arc<dyn StaticMask>,
}

static_mask!(NotStaticMask);
impl StaticMask for NotStaticMask {
    /// `NotMask.Static.partOfMask`: the opposite of the inner static mask.
    /// `NotMask.Static.partOfMask`.
    fn part_of_mask_static(&self, x: i32, y: i32, z: i32) -> bool {
        !self.source.part_of_mask_static(x, y, z)
    }
}

/// `StaticMask.not`: the static version of `not`.
/// `StaticMask.not`.
pub fn not_static(m: Arc<dyn StaticMask>) -> Arc<dyn StaticMask> {
    Arc::new(NotStaticMask { width_x: m.width_x(), height_y: m.height_y(), length_z: m.length_z(), source: m })
}

/// `BinaryOperatorMask`: the sizes are the larger of the two. A side counts only when the position is below its size: there is no lower
/// bound, as upstream, so a negative coordinate reaches the side's own `partOfMask`.
#[derive(Debug)]
struct BinaryOperatorMask {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    a: Arc<dyn Mask>,
    b: Arc<dyn Mask>,
    operator: BooleanBinaryOperators,
}

impl Mask for BinaryOperatorMask {
    /// `BinaryOperatorMask.partOfMask(int,int,int,BlockState)`: the operator over both masks, with no lower bound (U29).
    /// `BinaryOperatorMask.partOfMask`.
    fn part_of_mask(&self, x: i32, y: i32, z: i32, current: Option<StateId>) -> bool {
        let side = |m: &Arc<dyn Mask>| x < m.width_x() && y < m.height_y() && z < m.length_z() && m.part_of_mask(x, y, z, current);
        self.operator.apply(side(&self.a), side(&self.b))
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

/// `Mask.union`: `binary` with `Or`.
/// `Mask.intersection`: `binary` with `And`.
/// `Mask.xor`: `binary` with `Xor`.
/// `Mask.union`, `Mask.intersection` and `Mask.xor`, by operator.
pub fn binary(a: Arc<dyn Mask>, b: Arc<dyn Mask>, op: BooleanBinaryOperators) -> Arc<dyn Mask> {
    Arc::new(BinaryOperatorMask { width_x: a.width_x().max(b.width_x()), height_y: a.height_y().max(b.height_y()), length_z: a.length_z().max(b.length_z()), a, b, operator: op })
}

/// `BinaryOperatorMask.Static`.
#[derive(Debug)]
struct BinaryOperatorStaticMask {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    a: Arc<dyn StaticMask>,
    b: Arc<dyn StaticMask>,
    operator: BooleanBinaryOperators,
}

static_mask!(BinaryOperatorStaticMask);
impl StaticMask for BinaryOperatorStaticMask {
    /// `BinaryOperatorMask.Static.partOfMask(int,int,int)`: the operator over both static masks.
    /// `BinaryOperatorMask.Static.partOfMask`.
    fn part_of_mask_static(&self, x: i32, y: i32, z: i32) -> bool {
        let side = |m: &Arc<dyn StaticMask>| x < m.width_x() && y < m.height_y() && z < m.length_z() && m.part_of_mask_static(x, y, z);
        self.operator.apply(side(&self.a), side(&self.b))
    }
}

/// `StaticMask.union`: `binary_static` with `Or`.
/// `StaticMask.intersection`: `binary_static` with `And`.
/// `StaticMask.xor`: `binary_static` with `Xor`.
/// `StaticMask.union`, `StaticMask.intersection` and `StaticMask.xor`, by operator.
pub fn binary_static(a: Arc<dyn StaticMask>, b: Arc<dyn StaticMask>, op: BooleanBinaryOperators) -> Arc<dyn StaticMask> {
    Arc::new(BinaryOperatorStaticMask { width_x: a.width_x().max(b.width_x()), height_y: a.height_y().max(b.height_y()), length_z: a.length_z().max(b.length_z()), a, b, operator: op })
}

/// `SphereMask`: an ellipsoid that fills the box, whole or as a one-block shell.
#[derive(Debug)]
pub struct SphereMask {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    center_x: f64,
    center_y: f64,
    center_z: f64,
    radius_sq_x: f64,
    radius_sq_y: f64,
    radius_sq_z: f64,
    filled: bool,
}

impl SphereMask {
    /// `SphereMask.<init>`.
    pub fn new(width_x: i32, height_y: i32, length_z: i32, filled: bool) -> SphereMask {
        let center_x = width_x as f64 / 2.0;
        let center_y = height_y as f64 / 2.0;
        let center_z = length_z as f64 / 2.0;
        SphereMask { width_x, height_y, length_z, center_x, center_y, center_z, radius_sq_x: center_x * center_x, radius_sq_y: center_y * center_y, radius_sq_z: center_z * center_z, filled }
    }

    /// `SphereMask.outside`: whether a point is outside the sphere.
    /// `SphereMask.outside`.
    fn outside(&self, dx: f64, dy: f64, dz: f64) -> bool {
        dx * dx / self.radius_sq_x + dy * dy / self.radius_sq_y + dz * dz / self.radius_sq_z > 1.0
    }
}

static_mask!(SphereMask);
impl StaticMask for SphereMask {
    /// `SphereMask.partOfMask`: a cell of the sphere.
    /// `SphereMask.partOfMask`.
    fn part_of_mask_static(&self, x: i32, y: i32, z: i32) -> bool {
        let dx = ((x as f64 + 0.5) - self.center_x).abs();
        let dy = ((y as f64 + 0.5) - self.center_y).abs();
        let dz = ((z as f64 + 0.5) - self.center_z).abs();
        if self.outside(dx, dy, dz) {
            return false;
        }
        self.filled || self.outside(dx + 1.0, dy, dz) || self.outside(dx, dy + 1.0, dz) || self.outside(dx, dy, dz + 1.0)
    }
}

/// `CylinderMask`: an elliptical cylinder along an axis, filled or as a one-block tube.
#[derive(Debug)]
pub struct CylinderMask {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    center_a: f64,
    center_b: f64,
    radius_sq_a: f64,
    radius_sq_b: f64,
    filled: bool,
    alignment: Axis,
}

/// `CylinderMask.getA`: the first coordinate of the circle's plane.
fn get_a(x: i32, y: i32, alignment: Axis) -> i32 {
    if alignment == Axis::X {
        y
    } else {
        x
    }
}

/// `CylinderMask.getB`: the second coordinate of the circle's plane.
fn get_b(y: i32, z: i32, alignment: Axis) -> i32 {
    if alignment == Axis::Z {
        y
    } else {
        z
    }
}

impl CylinderMask {
    /// `CylinderMask.<init>`. A box two blocks wide on a circle axis has a radius squared of zero (`centerA = 1`, so `(centerA - 1)^2 = 0`), and divides by zero as upstream's doubles do; one block gives 0.25 and no width at all gives 1.
    pub fn new(width_x: i32, height_y: i32, length_z: i32, filled: bool, alignment: Axis) -> CylinderMask {
        let center_a = get_a(width_x, height_y, alignment) as f64 / 2.0;
        let center_b = get_b(height_y, length_z, alignment) as f64 / 2.0;
        CylinderMask { width_x, height_y, length_z, center_a, center_b, radius_sq_a: (center_a - 1.0) * (center_a - 1.0), radius_sq_b: (center_b - 1.0) * (center_b - 1.0), filled, alignment }
    }

    /// `CylinderMask.outside`: whether a point is outside the circle.
    /// `CylinderMask.outside`.
    fn outside(&self, da: f64, db: f64) -> bool {
        da * da / self.radius_sq_a + db * db / self.radius_sq_b > 1.0
    }
}

static_mask!(CylinderMask);
impl StaticMask for CylinderMask {
    /// `CylinderMask.partOfMask`: a cell of the cylinder.
    /// `CylinderMask.partOfMask`.
    fn part_of_mask_static(&self, x: i32, y: i32, z: i32) -> bool {
        let da = ((get_a(x, y, self.alignment) as f64 + 0.5) - self.center_a).abs();
        let db = ((get_b(y, z, self.alignment) as f64 + 0.5) - self.center_b).abs();
        if self.outside(da, db) {
            return false;
        }
        self.filled || self.outside(da + 1.0, db) || self.outside(da, db + 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A static mask from a function, with its own size.
    #[derive(Debug)]
    struct FnMask {
        w: i32,
        h: i32,
        l: i32,
        f: fn(i32, i32, i32) -> bool,
    }
    impl Mask for FnMask {
        fn part_of_mask(&self, x: i32, y: i32, z: i32, _c: Option<StateId>) -> bool {
            (self.f)(x, y, z)
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
    impl StaticMask for FnMask {
        fn part_of_mask_static(&self, x: i32, y: i32, z: i32) -> bool {
            (self.f)(x, y, z)
        }
    }

    /// The mask as text: one block per y layer, `|` between them, `,` between the rows (z), `#` for part of the mask.
    pub(crate) fn render(m: &dyn StaticMask) -> String {
        let mut layers = vec![];
        for y in 0..m.height_y() {
            let rows: Vec<String> = (0..m.length_z()).map(|z| (0..m.width_x()).map(|x| if m.part_of_mask_static(x, y, z) { '#' } else { '.' }).collect()).collect();
            layers.push(rows.join(","));
        }
        layers.join("|")
    }

    #[test]
    fn operators_apply_like_java() {
        for (a, b) in [(false, false), (false, true), (true, false), (true, true)] {
            assert_eq!(BooleanBinaryOperators::Or.apply(a, b), a || b);
            assert_eq!(BooleanBinaryOperators::And.apply(a, b), a && b);
            assert_eq!(BooleanBinaryOperators::Xor.apply(a, b), a != b);
        }
    }

    #[test]
    fn a_sphere_is_the_hand_computed_grid() {
        // Generated from the formula by an independent script (tools are not in the repository), checked by hand on the centre layer:
        // 5 wide, centre 2.5, radius squared 6.25, so (x, z) = (2, 2) off the middle by (0.5 .. 2) is in when dx^2 + dz^2 <= 6.25.
        assert_eq!(render(&SphereMask::new(5, 5, 5, true)), SPHERE_5_FILLED);
        assert_eq!(render(&SphereMask::new(5, 5, 5, false)), SPHERE_5_HOLLOW);
        assert_eq!(render(&SphereMask::new(4, 3, 6, true)), SPHERE_436_FILLED);
        assert_eq!(render(&SphereMask::new(4, 3, 6, false)), SPHERE_436_HOLLOW);
    }

    #[test]
    fn a_cylinder_is_the_hand_computed_grid_on_each_axis() {
        assert_eq!(render(&CylinderMask::new(5, 5, 5, true, Axis::Y)), CYL_5_Y_FILLED);
        assert_eq!(render(&CylinderMask::new(5, 5, 5, false, Axis::Y)), CYL_5_Y_HOLLOW);
        assert_eq!(render(&CylinderMask::new(4, 3, 6, true, Axis::X)), CYL_436_X_FILLED);
        assert_eq!(render(&CylinderMask::new(4, 3, 6, false, Axis::X)), CYL_436_X_HOLLOW);
        assert_eq!(render(&CylinderMask::new(4, 3, 6, true, Axis::Y)), CYL_436_Y_FILLED);
        assert_eq!(render(&CylinderMask::new(4, 3, 6, false, Axis::Y)), CYL_436_Y_HOLLOW);
        assert_eq!(render(&CylinderMask::new(4, 3, 6, true, Axis::Z)), CYL_436_Z_FILLED);
        assert_eq!(render(&CylinderMask::new(4, 3, 6, false, Axis::Z)), CYL_436_Z_HOLLOW);
    }

    #[test]
    fn masks_of_different_sizes_combine_to_the_larger_box() {
        let small: Arc<dyn StaticMask> = Arc::new(FnMask { w: 2, h: 2, l: 2, f: |_, _, _| true });
        let wide: Arc<dyn StaticMask> = Arc::new(FnMask { w: 4, h: 1, l: 3, f: |x, _, _| x % 2 == 0 });
        let u = binary_static(small.clone(), wide.clone(), BooleanBinaryOperators::Or);
        assert_eq!((u.width_x(), u.height_y(), u.length_z()), (4, 2, 3));
        // the side that is too small for the position counts as false, never reads out of its box
        assert!(!u.part_of_mask_static(3, 0, 2));
        assert!(u.part_of_mask_static(2, 0, 2)); // wide, x even
        assert!(u.part_of_mask_static(1, 1, 1)); // small only
        assert!(!u.part_of_mask_static(3, 1, 1));
        let a = binary_static(small.clone(), wide.clone(), BooleanBinaryOperators::And);
        assert!(a.part_of_mask_static(0, 0, 0) && !a.part_of_mask_static(1, 0, 0) && !a.part_of_mask_static(0, 1, 0));
        let x = binary_static(small, wide, BooleanBinaryOperators::Xor);
        assert!(!x.part_of_mask_static(0, 0, 0) && x.part_of_mask_static(1, 0, 0) && x.part_of_mask_static(2, 0, 0) && x.part_of_mask_static(0, 1, 0));
        // the non-static forms agree and carry the state through
        let m: Arc<dyn Mask> = Arc::new(FnMask { w: 3, h: 3, l: 3, f: |x, _, _| x == 1 });
        let n = not(m.clone());
        assert_eq!((n.width_x(), n.height_y(), n.length_z()), (3, 3, 3));
        assert!(!n.part_of_mask(1, 0, 0, None) && n.part_of_mask(0, 0, 0, Some(5)));
        let b = binary(m, Arc::new(FnMask { w: 5, h: 1, l: 1, f: |_, _, _| true }), BooleanBinaryOperators::Xor);
        assert_eq!((b.width_x(), b.height_y(), b.length_z()), (5, 3, 3));
        assert!(!b.part_of_mask(1, 0, 0, None) && b.part_of_mask(0, 0, 0, None) && !b.part_of_mask(0, 1, 0, None));
    }

    #[test]
    fn not_flips_every_cell_and_keeps_the_size() {
        let s: Arc<dyn StaticMask> = Arc::new(SphereMask::new(5, 5, 5, true));
        let n = not_static(s.clone());
        assert_eq!((n.width_x(), n.height_y(), n.length_z()), (5, 5, 5));
        for y in 0..5 {
            for z in 0..5 {
                for x in 0..5 {
                    assert_eq!(n.part_of_mask_static(x, y, z), !s.part_of_mask_static(x, y, z));
                    assert_eq!(n.part_of_mask(x, y, z, None), !s.part_of_mask_static(x, y, z));
                }
            }
        }
    }

    #[test]
    fn compute_gives_the_same_answers_and_is_safe_outside_the_box() {
        let s = CylinderMask::new(4, 3, 6, false, Axis::Y);
        let c = compute(&s);
        assert_eq!((c.width_x(), c.height_y(), c.length_z()), (4, 3, 6));
        assert_eq!(render(c.as_ref()), render(&s));
        assert!(!c.part_of_mask_static(-1, 0, 0) && !c.part_of_mask_static(4, 0, 0) && !c.part_of_mask_static(0, 3, 0) && !c.part_of_mask_static(0, 0, 6));
        // an empty box
        let e = compute(&SphereMask::new(0, 3, 3, true));
        assert!(!e.part_of_mask_static(0, 0, 0));
    }

    // The expected grids. `|` separates y layers, `,` separates rows (z), each row runs along x.
    const SPHERE_5_FILLED: &str = ".....,.###.,.###.,.###.,.....|.###.,#####,#####,#####,.###.|.###.,#####,#####,#####,.###.|.###.,#####,#####,#####,.###.|.....,.###.,.###.,.###.,.....";
    const SPHERE_5_HOLLOW: &str = ".....,.###.,.###.,.###.,.....|.###.,#...#,#...#,#...#,.###.|.###.,#...#,#...#,#...#,.###.|.###.,#...#,#...#,#...#,.###.|.....,.###.,.###.,.###.,.....";
    const SPHERE_436_FILLED: &str = "....,.##.,.##.,.##.,.##.,....|.##.,####,####,####,####,.##.|....,.##.,.##.,.##.,.##.,....";
    const SPHERE_436_HOLLOW: &str = "....,.##.,.##.,.##.,.##.,....|.##.,#..#,#..#,#..#,#..#,.##.|....,.##.,.##.,.##.,.##.,....";
    const CYL_5_Y_FILLED: &str = ".....,.###.,.###.,.###.,.....|.....,.###.,.###.,.###.,.....|.....,.###.,.###.,.###.,.....|.....,.###.,.###.,.###.,.....|.....,.###.,.###.,.###.,.....";
    const CYL_5_Y_HOLLOW: &str = ".....,.###.,.#.#.,.###.,.....|.....,.###.,.#.#.,.###.,.....|.....,.###.,.#.#.,.###.,.....|.....,.###.,.#.#.,.###.,.....|.....,.###.,.#.#.,.###.,.....";
    const CYL_436_X_FILLED: &str = "....,....,....,....,....,....|....,####,####,####,####,....|....,....,....,....,....,....";
    const CYL_436_X_HOLLOW: &str = "....,....,....,....,....,....|....,####,####,####,####,....|....,....,....,....,....,....";
    const CYL_436_Y_FILLED: &str = "....,.##.,.##.,.##.,.##.,....|....,.##.,.##.,.##.,.##.,....|....,.##.,.##.,.##.,.##.,....";
    const CYL_436_Y_HOLLOW: &str = "....,.##.,.##.,.##.,.##.,....|....,.##.,.##.,.##.,.##.,....|....,.##.,.##.,.##.,.##.,....";
    const CYL_436_Z_FILLED: &str = "....,....,....,....,....,....|.##.,.##.,.##.,.##.,.##.,.##.|....,....,....,....,....,....";
    const CYL_436_Z_HOLLOW: &str = "....,....,....,....,....,....|.##.,.##.,.##.,.##.,.##.,.##.|....,....,....,....,....,....";
}
