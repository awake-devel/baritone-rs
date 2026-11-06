// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/pathing/Favoring.java and Avoidance.java
// LGPL-3.0-or-later, see lib.rs.
//
// Which mobs and spawners are avoided is the glue's job: `Avoidance::create` takes their positions.
use super::path::Path;
use crate::config::SearchSettings;
use crate::pos::{long_hash, Pos};
use std::collections::HashMap;

/// `Avoidance`: a sphere around a centre where the cost of arriving is multiplied.
#[derive(Clone, Debug, PartialEq)]
pub struct Avoidance {
    pub center_x: i32,
    pub center_y: i32,
    pub center_z: i32,
    pub coefficient: f64,
    pub radius: i32,
    pub radius_sq: i32,
}

impl Avoidance {
    /// `Avoidance.<init>(int,int,int,double,int)`: `radiusSq` is computed in `int` arithmetic.
    pub fn new(center_x: i32, center_y: i32, center_z: i32, coefficient: f64, radius: i32) -> Avoidance {
        Avoidance { center_x, center_y, center_z, coefficient, radius, radius_sq: radius.wrapping_mul(radius) }
    }

    /// `Avoidance.coefficient`: the coefficient inside the sphere, 1 outside.
    pub fn coefficient(&self, x: i32, y: i32, z: i32) -> f64 {
        let x_diff = x.wrapping_sub(self.center_x);
        let y_diff = y.wrapping_sub(self.center_y);
        let z_diff = z.wrapping_sub(self.center_z);
        if x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) <= self.radius_sq {
            self.coefficient
        } else {
            1.0
        }
    }

    /// `Avoidance.create`: the avoidances for the spawners and mobs the caller found. Upstream asks the cache and the entity list itself and filters the mobs (spiders by light, piglins by anger, endermen by stare); here the caller has done that. A kind whose coefficient is 1 adds nothing, as upstream, and nothing is added when `avoidance` is off.
    pub fn create(s: &SearchSettings, spawners: &[Pos], mobs: &[Pos]) -> Vec<Avoidance> {
        let mut res = Vec::new();
        if !s.avoidance {
            return res;
        }
        if s.mob_spawner_avoidance_coefficient != 1.0 {
            for p in spawners {
                res.push(Avoidance::new(p.x, p.y, p.z, s.mob_spawner_avoidance_coefficient, s.mob_spawner_avoidance_radius));
            }
        }
        if s.mob_avoidance_coefficient != 1.0 {
            for p in mobs {
                res.push(Avoidance::new(p.x, p.y, p.z, s.mob_avoidance_coefficient, s.mob_avoidance_radius));
            }
        }
        res
    }

    /// `Avoidance.applySpherical`: multiplies the entry of every position in the sphere, in the loop order x, y, z. The entries are keyed on `longHash`, as upstream, so positions that share a hash share an entry.
    pub fn apply_spherical(&self, map: &mut HashMap<i64, f64>) {
        let radius = self.radius;
        let mut x = radius.wrapping_neg();
        while x <= radius {
            let mut y = radius.wrapping_neg();
            while y <= radius {
                let mut z = radius.wrapping_neg();
                while z <= radius {
                    if x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y)).wrapping_add(z.wrapping_mul(z)) <= radius.wrapping_mul(radius) {
                        let hash = long_hash(self.center_x.wrapping_add(x), self.center_y.wrapping_add(y), self.center_z.wrapping_add(z));
                        let old = map.get(&hash).copied().unwrap_or(1.0);
                        map.insert(hash, old * self.coefficient);
                    }
                    if z == i32::MAX {
                        break;
                    }
                    z += 1;
                }
                if y == i32::MAX {
                    break;
                }
                y += 1;
            }
            if x == i32::MAX {
                break;
            }
            x += 1;
        }
    }
}

/// `Favoring`: a multiplier on the cost of arriving at a position, keyed on `longHash`. 1 where nothing is set.
pub struct Favoring {
    favorings: HashMap<i64, f64>,
}

impl Favoring {
    /// `Favoring.<init>(IPath,CalculationContext)`: the positions of the previous path get `backtrackCostFavoringCoefficient`, unless it is 1. The coefficient comes from the settings.
    pub fn from_previous(previous: Option<&Path>, s: &SearchSettings) -> Favoring {
        let mut favorings = HashMap::new();
        let coeff = s.backtrack_cost_favoring_coefficient;
        if coeff != 1.0 {
            if let Some(prev) = previous {
                for pos in prev.positions() {
                    favorings.insert(pos.long_hash(), coeff);
                }
            }
        }
        Favoring { favorings }
    }

    /// `Favoring.<init>(IPlayerContext,IPath,CalculationContext)`: the same, then every avoidance applied in order. The caller makes the avoidances with `Avoidance::create`.
    pub fn new(previous: Option<&Path>, s: &SearchSettings, avoidances: &[Avoidance]) -> Favoring {
        let mut f = Favoring::from_previous(previous, s);
        for avoid in avoidances {
            avoid.apply_spherical(&mut f.favorings);
        }
        f
    }

    /// `Favoring.isEmpty`: nothing is favored.
    pub fn is_empty(&self) -> bool {
        self.favorings.is_empty()
    }

    /// `Favoring.calculate`: the multiplier for a hash, 1 where there is none.
    pub fn calculate(&self, hash: i64) -> f64 {
        self.favorings.get(&hash).copied().unwrap_or(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[test]
    fn previous_path_positions_get_the_backtrack_coefficient() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let path = chain_path(&ctx, &[Pos::new(1, 1, 1), Pos::new(2, 1, 1), Pos::new(3, 1, 1)]).unwrap();
        let s = SearchSettings::default();
        let f = Favoring::from_previous(Some(&path), &s);
        assert!(!f.is_empty());
        assert_eq!(f.calculate(Pos::new(2, 1, 1).long_hash()), 0.5);
        assert_eq!(f.calculate(Pos::new(2, 1, 2).long_hash()), 1.0);
        // no previous path, or a coefficient of 1, leaves it empty
        assert!(Favoring::from_previous(None, &s).is_empty());
        let one = SearchSettings { backtrack_cost_favoring_coefficient: 1.0, ..Default::default() };
        assert!(Favoring::from_previous(Some(&path), &one).is_empty());
    }

    #[test]
    fn avoidance_sphere_and_coefficient() {
        let a = Avoidance::new(10, 20, 30, 1.5, 2);
        assert_eq!(a.radius_sq, 4);
        assert_eq!(a.coefficient(10, 20, 32), 1.5);
        assert_eq!(a.coefficient(11, 21, 31), 1.5);
        assert_eq!(a.coefficient(12, 21, 30), 1.0);
        let mut map = HashMap::new();
        Avoidance::new(0, 0, 0, 2.0, 1).apply_spherical(&mut map);
        // the centre and the six neighbours
        assert_eq!(map.len(), 7);
        assert_eq!(map[&long_hash(0, 0, 0)], 2.0);
        assert_eq!(map[&long_hash(0, 1, 0)], 2.0);
        assert!(!map.contains_key(&long_hash(1, 1, 0)));
        // a second sphere multiplies where they overlap
        Avoidance::new(1, 0, 0, 3.0, 1).apply_spherical(&mut map);
        assert_eq!(map[&long_hash(0, 0, 0)], 6.0);
        assert_eq!(map[&long_hash(1, 0, 0)], 6.0);
        assert_eq!(map[&long_hash(2, 0, 0)], 3.0);
        assert_eq!(map[&long_hash(-1, 0, 0)], 2.0);
    }

    #[test]
    fn a_sphere_with_coefficient_one_still_fills_the_map() {
        let mut map = HashMap::new();
        Avoidance::new(0, 0, 0, 1.0, 1).apply_spherical(&mut map);
        assert_eq!(map.len(), 7);
        let s = SearchSettings { avoidance: true, mob_avoidance_coefficient: 1.5, mob_spawner_avoidance_coefficient: 1.0, ..Default::default() };
        let spawners = [Pos::new(1, 1, 1)];
        let mobs = [Pos::new(5, 5, 5), Pos::new(6, 6, 6)];
        let a = Avoidance::create(&s, &spawners, &mobs);
        // a kind with coefficient 1 adds nothing
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].coefficient, a[0].radius), (1.5, 8));
        let s2 = SearchSettings { avoidance: true, ..Default::default() };
        let b = Avoidance::create(&s2, &spawners, &mobs);
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].coefficient, b[0].radius), (2.0, 16));
        // off: nothing
        assert!(Avoidance::create(&SearchSettings::default(), &spawners, &mobs).is_empty());
    }

    #[test]
    fn favoring_with_avoidances_applies_them_after_the_previous_path() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let path = chain_path(&ctx, &[Pos::new(1, 1, 1), Pos::new(2, 1, 1)]).unwrap();
        let s = SearchSettings::default();
        let f = Favoring::new(Some(&path), &s, &[Avoidance::new(2, 1, 1, 4.0, 1)]);
        assert_eq!(f.calculate(Pos::new(2, 1, 1).long_hash()), 2.0);
        assert_eq!(f.calculate(Pos::new(1, 1, 1).long_hash()), 2.0);
        assert_eq!(f.calculate(Pos::new(3, 1, 1).long_hash()), 4.0);
        // an empty previous path and no avoidance is empty
        assert!(Favoring::new(None, &s, &[]).is_empty());
    }

    #[test]
    fn long_hash_is_upstreams_polynomial() {
        // 3241 * 3457689 + x, then * 8734625 + y, then * 2873465 + z, in long arithmetic: worked out in 128 bits and cut to 64
        let wide = |x: i128, y: i128, z: i128| (((3457689 * 3241 + x) * 8734625 + y) * 2873465 + z) as i64;
        assert_eq!(long_hash(7, 8, 9), wide(7, 8, 9));
        assert_eq!(long_hash(-30_000_000, 319, 29_999_999), wide(-30_000_000, 319, 29_999_999));
        assert_eq!(long_hash(i32::MIN, i32::MAX, i32::MIN), wide(i32::MIN as i128, i32::MAX as i128, i32::MIN as i128));
        // it wraps already for small coordinates: the last product is about 3e23
        assert!((((3457689i128 * 3241 + 7) * 8734625 + 8) * 2873465 + 9) > i64::MAX as i128);
    }
}
