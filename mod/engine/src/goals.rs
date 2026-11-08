// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/pathing/goals/ (Goal, GoalBlock, GoalXZ, GoalYLevel, GoalTwoBlocks, GoalGetToBlock, GoalNear, GoalAxis, GoalStrictDirection, GoalRunAway, GoalComposite, GoalInverted)
// LGPL-3.0-or-later, see lib.rs.
//
// Upstream has one class per goal behind the `Goal` interface; here that is one enum. `equals`, `hashCode`, `toString` and the `BlockPos`
// overloads are dropped; the render-position interface waits for the glue. Settings upstream reads from `BaritoneAPI.getSettings()` come in as `GoalSettings`.
// Java `int` arithmetic wraps, so every int step here uses `wrapping_*`; `Math.min`/`Math.max` on doubles go through `java_min`/`java_max`.
use crate::config::Config;
use crate::costs::*;

/// The two settings the goals read: `costHeuristic` (GoalXZ, GoalAxis) and `axisHeight` (GoalAxis).
#[derive(Clone, Copy, Debug)]
pub struct GoalSettings {
    pub cost_heuristic: f64,
    pub axis_height: i32,
}

impl Config {
    /// The goal-relevant slice of the settings snapshot.
    pub fn goal_settings(&self) -> GoalSettings {
        GoalSettings { cost_heuristic: self.cost_heuristic, axis_height: self.axis_height }
    }
}

/// `Goal` and its implementations.
#[derive(Clone, Debug, PartialEq)]
pub enum Goal {
    Block {
        x: i32,
        y: i32,
        z: i32,
    },
    Xz {
        x: i32,
        z: i32,
    },
    YLevel {
        level: i32,
    },
    TwoBlocks {
        x: i32,
        y: i32,
        z: i32,
    },
    GetToBlock {
        x: i32,
        y: i32,
        z: i32,
    },
    Near {
        x: i32,
        y: i32,
        z: i32,
        range_sq: i32,
    },
    Axis,
    StrictDirection {
        x: i32,
        y: i32,
        z: i32,
        dx: i32,
        dz: i32,
    },
    RunAway {
        from: Vec<(i32, i32, i32)>,
        distance_sq: i32,
        maintain_y: Option<i32>,
    },
    Composite(Vec<Goal>),
    Inverted(Box<Goal>),
    /// Upstream's `MineProcess.GoalThreeBlocks`: `GoalTwoBlocks` with one more block below.
    ThreeBlocks {
        x: i32,
        y: i32,
        z: i32,
    },
    /// The anonymous `GoalRunAway` subclasses of `MineProcess.updateGoal` and `GetToBlockProcess.onTick`, which override `isInGoal` to
    /// `false` and `heuristic()` to negative infinity: a goal that is never reached, only walked away from. Holds the `GoalRunAway`.
    NeverInGoal(Box<Goal>),
}

/// `GoalAxis.SQRT_2_OVER_2`.
const SQRT_2_OVER_2: f64 = SQRT_2 / 2.0;

/// `GoalNear.<init>`: `rangeSq = range * range` in int arithmetic.
pub fn goal_near(x: i32, y: i32, z: i32, range: i32) -> Goal {
    Goal::Near { x, y, z, range_sq: range.wrapping_mul(range) }
}

/// `GoalStrictDirection.<init>`: `dx` and `dz` are the direction's step; throws `IllegalArgumentException` when both are 0.
pub fn goal_strict_direction(x: i32, y: i32, z: i32, dx: i32, dz: i32) -> Goal {
    assert!(!(dx == 0 && dz == 0), "GoalStrictDirection: direction has no horizontal step");
    Goal::StrictDirection { x, y, z, dx, dz }
}

/// `GoalRunAway.<init>(double,Integer,BlockPos...)`: `distanceSq = (int) (distance * distance)`; throws `IllegalArgumentException` when `from` is empty.
/// `GoalRunAway.<init>(double,BlockPos...)`: the same, with no `maintainY`.
pub fn goal_run_away(distance: f64, maintain_y: Option<i32>, from: Vec<(i32, i32, i32)>) -> Goal {
    assert!(!from.is_empty(), "Positions to run away from must not be empty");
    Goal::RunAway { from, distance_sq: (distance * distance) as i32, maintain_y }
}

/// `GoalYLevel.calculate`: ticks to change from `current_y` to `goal_y`.
pub fn goal_y_level_calculate(goal_y: i32, current_y: i32) -> f64 {
    if current_y > goal_y {
        // need to descend
        return fall_n_blocks_cost()[2] / 2.0 * (current_y.wrapping_sub(goal_y) as f64);
    }
    if current_y < goal_y {
        // need to ascend
        return (goal_y.wrapping_sub(current_y) as f64) * jump_one_block_cost();
    }
    0.0
}

/// `GoalXZ.calculate`: the pythagorean and manhattan mixture, scaled by `costHeuristic`.
pub fn goal_xz_calculate(x_diff: f64, z_diff: f64, s: &GoalSettings) -> f64 {
    let x = x_diff.abs();
    let z = z_diff.abs();
    let straight;
    let mut diagonal;
    if x < z {
        straight = z - x;
        diagonal = x;
    } else {
        straight = x - z;
        diagonal = z;
    }
    diagonal *= SQRT_2;
    (diagonal + straight) * s.cost_heuristic
}

/// `GoalBlock.calculate`: the Y part plus the XZ part.
pub fn goal_block_calculate(x_diff: f64, y_diff: i32, z_diff: f64, s: &GoalSettings) -> f64 {
    let mut heuristic = 0.0;
    heuristic += goal_y_level_calculate(0, y_diff);
    heuristic += goal_xz_calculate(x_diff, z_diff, s);
    heuristic
}

/// The shared tail of `GoalNear.heuristic()` and `GoalRunAway.heuristic()`: `maybeAlwaysInside` is only used for its maximum, so a Vec stands in for the DoubleOpenHashSet.
fn max_inside(maybe_always_inside: &[f64], min_outside: f64) -> f64 {
    let mut max_inside = f64::NEG_INFINITY;
    for &inside in maybe_always_inside {
        if inside < min_outside {
            max_inside = java_max(max_inside, inside);
        }
    }
    max_inside
}

impl Goal {
    /// `Goal.isInGoal(int,int,int)`: whether the position satisfies this goal.
    /// `GoalAxis.isInGoal`: its arm below.
    /// `GoalBlock.isInGoal`: its arm below.
    /// `GoalComposite.isInGoal`: its arm below.
    /// `GoalGetToBlock.isInGoal`: its arm below.
    /// `GoalInverted.isInGoal`: its arm below.
    /// `GoalNear.isInGoal`: its arm below.
    /// `MineProcess.GoalThreeBlocks.isInGoal`: its arm below.
    /// `GoalRunAway.isInGoal`: its arm below.
    /// `GoalStrictDirection.isInGoal`: its arm below.
    /// `GoalTwoBlocks.isInGoal`: its arm below.
    /// `GoalXZ.isInGoal`: its arm below.
    /// `GoalYLevel.isInGoal`: its arm below.
    pub fn is_in_goal(&self, x: i32, y: i32, z: i32, s: &GoalSettings) -> bool {
        match self {
            // `GoalBlock.isInGoal`
            Goal::Block { x: gx, y: gy, z: gz } => x == *gx && y == *gy && z == *gz,
            // `GoalXZ.isInGoal`
            Goal::Xz { x: gx, z: gz } => x == *gx && z == *gz,
            // `GoalYLevel.isInGoal`
            Goal::YLevel { level } => y == *level,
            // `GoalTwoBlocks.isInGoal`
            Goal::TwoBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy || y == gy.wrapping_sub(1)) && z == *gz,
            // `GoalGetToBlock.isInGoal`
            Goal::GetToBlock { x: gx, y: gy, z: gz } => {
                let x_diff = x.wrapping_sub(*gx);
                let y_diff = y.wrapping_sub(*gy);
                let z_diff = z.wrapping_sub(*gz);
                x_diff.wrapping_abs().wrapping_add((if y_diff < 0 { y_diff + 1 } else { y_diff }).wrapping_abs()).wrapping_add(z_diff.wrapping_abs()) <= 1
            }
            // `GoalNear.isInGoal`
            Goal::Near { x: gx, y: gy, z: gz, range_sq } => {
                let x_diff = x.wrapping_sub(*gx);
                let y_diff = y.wrapping_sub(*gy);
                let z_diff = z.wrapping_sub(*gz);
                x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) <= *range_sq
            }
            // `GoalAxis.isInGoal`
            Goal::Axis => y == s.axis_height && (x == 0 || z == 0 || x.wrapping_abs() == z.wrapping_abs()),
            // `GoalStrictDirection.isInGoal`
            Goal::StrictDirection { .. } => false,
            // `GoalRunAway.isInGoal`
            Goal::RunAway { from, distance_sq, maintain_y } => {
                if let Some(my) = maintain_y {
                    if *my != y {
                        return false;
                    }
                }
                for p in from {
                    let diff_x = x.wrapping_sub(p.0);
                    let diff_z = z.wrapping_sub(p.2);
                    let dist_sq = diff_x.wrapping_mul(diff_x).wrapping_add(diff_z.wrapping_mul(diff_z));
                    if dist_sq < *distance_sq {
                        return false;
                    }
                }
                true
            }
            // `GoalComposite.isInGoal`
            Goal::Composite(goals) => {
                for goal in goals {
                    if goal.is_in_goal(x, y, z, s) {
                        return true;
                    }
                }
                false
            }
            // `GoalInverted.isInGoal`
            Goal::Inverted(_) => false,
            // `MineProcess.GoalThreeBlocks.isInGoal`
            Goal::ThreeBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy || y == gy.wrapping_sub(1) || y == gy.wrapping_sub(2)) && z == *gz,
            // the anonymous `GoalRunAway` of the processes: `isInGoal` is overridden to false
            Goal::NeverInGoal(_) => false,
        }
    }

    /// `Goal.heuristic(int,int,int)`: estimated ticks from the position to satisfy this goal.
    /// `GoalAxis.heuristic`: its arm below.
    /// `GoalBlock.heuristic`: its arm below.
    /// `GoalComposite.heuristic(int,int,int)`: its arm below.
    /// `GoalGetToBlock.heuristic`: its arm below.
    /// `GoalInverted.heuristic(int,int,int)`: its arm below.
    /// `GoalNear.heuristic(int,int,int)`: its arm below.
    /// `MineProcess.GoalThreeBlocks.heuristic`: its arm below.
    /// `GoalRunAway.heuristic(int,int,int)`: its arm below.
    /// `GoalStrictDirection.heuristic(int,int,int)`: its arm below.
    /// `GoalTwoBlocks.heuristic`: its arm below.
    /// `GoalXZ.heuristic`: its arm below.
    /// `GoalYLevel.heuristic`: its arm below.
    pub fn heuristic(&self, x: i32, y: i32, z: i32, s: &GoalSettings) -> f64 {
        match self {
            // `GoalBlock.heuristic`
            Goal::Block { x: gx, y: gy, z: gz } => {
                let x_diff = x.wrapping_sub(*gx);
                let y_diff = y.wrapping_sub(*gy);
                let z_diff = z.wrapping_sub(*gz);
                goal_block_calculate(x_diff as f64, y_diff, z_diff as f64, s)
            }
            // `GoalXZ.heuristic`
            Goal::Xz { x: gx, z: gz } => {
                let x_diff = x.wrapping_sub(*gx);
                let z_diff = z.wrapping_sub(*gz);
                goal_xz_calculate(x_diff as f64, z_diff as f64, s)
            }
            // `GoalYLevel.heuristic`
            Goal::YLevel { level } => goal_y_level_calculate(*level, y),
            // `GoalTwoBlocks.heuristic`
            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {
                let x_diff = x.wrapping_sub(*gx);
                let y_diff = y.wrapping_sub(*gy);
                let z_diff = z.wrapping_sub(*gz);
                goal_block_calculate(x_diff as f64, if y_diff < 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)
            }
            // `GoalGetToBlock.heuristic`
            Goal::GetToBlock { x: gx, y: gy, z: gz } => {
                let x_diff = x.wrapping_sub(*gx);
                let y_diff = y.wrapping_sub(*gy);
                let z_diff = z.wrapping_sub(*gz);
                goal_block_calculate(x_diff as f64, if y_diff < 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)
            }
            // `GoalNear.heuristic(int,int,int)`
            Goal::Near { x: gx, y: gy, z: gz, .. } => {
                let x_diff = x.wrapping_sub(*gx);
                let y_diff = y.wrapping_sub(*gy);
                let z_diff = z.wrapping_sub(*gz);
                goal_block_calculate(x_diff as f64, y_diff, z_diff as f64, s)
            }
            // `GoalAxis.heuristic`
            Goal::Axis => {
                let x = x.wrapping_abs();
                let z = z.wrapping_abs();
                let shrt = x.min(z);
                let lng = x.max(z);
                let diff = lng.wrapping_sub(shrt);
                let flat_axis_distance = java_min(x as f64, java_min(z as f64, diff as f64 * SQRT_2_OVER_2));
                flat_axis_distance * s.cost_heuristic + goal_y_level_calculate(s.axis_height, y)
            }
            // `GoalStrictDirection.heuristic`
            Goal::StrictDirection { x: gx, y: gy, z: gz, dx, dz } => {
                let distance_from_start_in_desired_direction = x.wrapping_sub(*gx).wrapping_mul(*dx).wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dz));
                let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_abs().wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx).wrapping_abs());
                let vertical_distance_from_start = y.wrapping_sub(*gy).wrapping_abs();
                // we want heuristic to decrease as desiredDirection increases
                let mut heuristic = distance_from_start_in_desired_direction.wrapping_neg().wrapping_mul(100) as f64;
                heuristic += distance_from_start_in_incorrect_direction.wrapping_mul(1000) as f64;
                heuristic += vertical_distance_from_start.wrapping_mul(1000) as f64;
                heuristic
            }
            // `GoalRunAway.heuristic(int,int,int)`
            Goal::RunAway { from, maintain_y, .. } => {
                let mut min = f64::MAX;
                for p in from {
                    let h = goal_xz_calculate(p.0.wrapping_sub(x) as f64, p.2.wrapping_sub(z) as f64, s);
                    if h < min {
                        min = h;
                    }
                }
                min = -min;
                if let Some(my) = maintain_y {
                    min = min * 0.6 + goal_y_level_calculate(*my, y) * 1.5;
                }
                min
            }
            // `GoalComposite.heuristic(int,int,int)`
            Goal::Composite(goals) => {
                let mut min = f64::MAX;
                for g in goals {
                    min = java_min(min, g.heuristic(x, y, z, s)); // whichever is closest
                }
                min
            }
            // `GoalInverted.heuristic(int,int,int)`
            Goal::Inverted(origin) => -origin.heuristic(x, y, z, s),
            // `MineProcess.GoalThreeBlocks.heuristic`
            Goal::ThreeBlocks { x: gx, y: gy, z: gz } => {
                let x_diff = x.wrapping_sub(*gx);
                let y_diff = y.wrapping_sub(*gy);
                let z_diff = z.wrapping_sub(*gz);
                goal_block_calculate(
                    x_diff as f64,
                    if y_diff < -1 {
                        y_diff.wrapping_add(2)
                    } else if y_diff == -1 {
                        0
                    } else {
                        y_diff
                    },
                    z_diff as f64,
                    s,
                )
            }
            // the anonymous `GoalRunAway` overrides only `heuristic()`: the one with a position is the `GoalRunAway`'s
            Goal::NeverInGoal(inner) => inner.heuristic(x, y, z, s),
        }
    }

    /// `Goal.heuristic()`: the heuristic at the goal, 0 by default (the interface default; GoalNear, GoalRunAway, GoalComposite, GoalStrictDirection and GoalInverted override it).
    /// `GoalComposite.heuristic()`: its arm below.
    /// `GoalInverted.heuristic()`: its arm below.
    /// `GoalNear.heuristic()`: its arm below.
    /// `GoalRunAway.heuristic()`: its arm below.
    /// `GoalStrictDirection.heuristic()`: its arm below.
    pub fn heuristic_inside(&self, s: &GoalSettings) -> f64 {
        match self {
            // `GoalNear.heuristic()`
            Goal::Near { x: gx, y: gy, z: gz, range_sq } => {
                let range = (*range_sq as f64).sqrt().ceil() as i32;
                let mut maybe_always_inside: Vec<f64> = Vec::new(); // see pull request #1978
                let mut min_outside = f64::INFINITY;
                for dx in -range..=range {
                    for dy in -range..=range {
                        for dz in -range..=range {
                            let h = self.heuristic(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s);
                            if h < min_outside && self.is_in_goal(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s) {
                                maybe_always_inside.push(h);
                            } else {
                                min_outside = java_min(min_outside, h);
                            }
                        }
                    }
                }
                max_inside(&maybe_always_inside, min_outside)
            }
            // `GoalStrictDirection.heuristic()`
            Goal::StrictDirection { .. } => f64::NEG_INFINITY,
            // `GoalRunAway.heuristic()`
            Goal::RunAway { from, distance_sq, .. } => {
                let distance = (*distance_sq as f64).sqrt().ceil() as i32;
                let mut min_x = i32::MAX;
                let mut min_y = i32::MAX;
                let mut min_z = i32::MAX;
                let mut max_x = i32::MIN;
                let mut max_y = i32::MIN;
                let mut max_z = i32::MIN;
                for p in from {
                    min_x = min_x.min(p.0.wrapping_sub(distance));
                    min_y = min_y.min(p.1.wrapping_sub(distance));
                    min_z = min_z.min(p.2.wrapping_sub(distance));
                    max_x = max_x.max(p.0.wrapping_add(distance));
                    max_y = max_y.max(p.1.wrapping_add(distance));
                    max_z = max_z.max(p.2.wrapping_add(distance));
                }
                let mut maybe_always_inside: Vec<f64> = Vec::new(); // see pull request #1978
                let mut min_outside = f64::INFINITY;
                // Java's `x++` would wrap and loop forever when a max is Integer.MAX_VALUE; an inclusive Rust range stops there instead.
                for x in min_x..=max_x {
                    for y in min_y..=max_y {
                        for z in min_z..=max_z {
                            let h = self.heuristic(x, y, z, s);
                            if h < min_outside && self.is_in_goal(x, y, z, s) {
                                maybe_always_inside.push(h);
                            } else {
                                min_outside = java_min(min_outside, h);
                            }
                        }
                    }
                }
                max_inside(&maybe_always_inside, min_outside)
            }
            // `GoalComposite.heuristic()`
            Goal::Composite(goals) => {
                let mut min = f64::MAX;
                for g in goals {
                    // just take the highest value that is guaranteed to be inside the goal
                    min = java_min(min, g.heuristic_inside(s));
                }
                min
            }
            // `GoalInverted.heuristic()`
            Goal::Inverted(_) => f64::NEG_INFINITY,
            // the anonymous `GoalRunAway`: `heuristic()` is overridden to negative infinity
            Goal::NeverInGoal(_) => f64::NEG_INFINITY,
            // `Goal.heuristic()` default: GoalBlock, GoalXZ, GoalYLevel, GoalTwoBlocks, GoalGetToBlock, GoalAxis
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> GoalSettings {
        GoalSettings { cost_heuristic: 1.0, axis_height: 120 }
    }

    #[test]
    fn java_min_max_signed_zero_and_nan() {
        assert_eq!(java_max(-0.0, 0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(java_max(0.0, -0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(java_min(0.0, -0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(java_min(-0.0, 0.0).to_bits(), (-0.0f64).to_bits());
        assert!(java_min(f64::NAN, 1.0).is_nan() && java_min(1.0, f64::NAN).is_nan());
        assert!(java_max(f64::NAN, 1.0).is_nan() && java_max(1.0, f64::NAN).is_nan());
    }

    #[test]
    fn block_goal() {
        let g = Goal::Block { x: 1, y: 2, z: 3 };
        assert!(g.is_in_goal(1, 2, 3, &s()));
        assert!(!g.is_in_goal(1, 3, 3, &s()));
        // 3 east and 4 south: diagonal 3 * sqrt(2) plus straight 1
        assert!((g.heuristic(4, 2, 7, &s()) - (3.0 * SQRT_2 + 1.0)).abs() < 1e-12);
        assert_eq!(g.heuristic(1, 2, 3, &s()), 0.0);
        assert_eq!(g.heuristic_inside(&s()), 0.0);
    }

    #[test]
    fn y_level_calculate() {
        assert_eq!(goal_y_level_calculate(5, 5), 0.0);
        assert_eq!(goal_y_level_calculate(5, 3), 2.0 * jump_one_block_cost());
        assert_eq!(goal_y_level_calculate(3, 5), fall_n_blocks_cost()[2] / 2.0 * 2.0);
    }

    #[test]
    fn two_blocks_and_get_to_block() {
        let g = Goal::TwoBlocks { x: 0, y: 10, z: 0 };
        assert!(g.is_in_goal(0, 10, 0, &s()) && g.is_in_goal(0, 9, 0, &s()) && !g.is_in_goal(0, 8, 0, &s()));
        // yDiff -1 becomes 0, so standing in the lower block costs nothing
        assert_eq!(g.heuristic(0, 9, 0, &s()), 0.0);
        let g = Goal::GetToBlock { x: 0, y: 0, z: 0 };
        assert!(g.is_in_goal(1, 0, 0, &s()) && g.is_in_goal(0, 1, 0, &s()) && g.is_in_goal(0, -2, 0, &s()));
        assert!(!g.is_in_goal(1, 1, 0, &s()) && !g.is_in_goal(0, 2, 0, &s()) && !g.is_in_goal(0, -3, 0, &s()));
    }

    #[test]
    fn near_goal() {
        let g = goal_near(0, 0, 0, 2);
        assert_eq!(g, Goal::Near { x: 0, y: 0, z: 0, range_sq: 4 });
        assert!(g.is_in_goal(1, 1, 0, &s()) && g.is_in_goal(2, 0, 0, &s()) && !g.is_in_goal(2, 1, 0, &s()));
        assert_eq!(goal_near(0, 0, 0, 0).heuristic_inside(&s()), 0.0);
        // 65536 * 65536 wraps to 0 in Java
        assert_eq!(goal_near(0, 0, 0, 65536), Goal::Near { x: 0, y: 0, z: 0, range_sq: 0 });
    }

    #[test]
    fn axis_goal() {
        let g = Goal::Axis;
        assert!(g.is_in_goal(0, 120, 7, &s()) && g.is_in_goal(5, 120, -5, &s()) && !g.is_in_goal(5, 120, 3, &s()) && !g.is_in_goal(0, 119, 7, &s()));
        // x=3, z=7: diff 4, 4 * sqrt(2)/2 is about 2.83 and below 3
        assert!((g.heuristic(3, 120, 7, &s()) - 4.0 * SQRT_2_OVER_2).abs() < 1e-12);
    }

    #[test]
    fn strict_direction() {
        let g = goal_strict_direction(0, 0, 0, 1, 0);
        assert!(!g.is_in_goal(0, 0, 0, &s()));
        assert_eq!(g.heuristic(5, 0, 0, &s()), -500.0);
        // desired 2, incorrect |4|, vertical 3: -200 + 4000 + 3000
        assert_eq!(g.heuristic(2, 3, 4, &s()), 6800.0);
        assert_eq!(g.heuristic_inside(&s()), f64::NEG_INFINITY);
    }

    #[test]
    #[should_panic(expected = "no horizontal step")]
    fn strict_direction_needs_a_step() {
        goal_strict_direction(0, 0, 0, 0, 0);
    }

    #[test]
    #[should_panic(expected = "must not be empty")]
    fn run_away_needs_a_point() {
        goal_run_away(3.0, None, vec![]);
    }

    #[test]
    fn run_away() {
        let g = goal_run_away(3.0, None, vec![(0, 0, 0)]);
        assert_eq!(g, Goal::RunAway { from: vec![(0, 0, 0)], distance_sq: 9, maintain_y: None });
        assert!(g.is_in_goal(3, 5, 0, &s()) && !g.is_in_goal(2, 5, 0, &s()));
        // 4 blocks away straight: heuristic is -4
        assert_eq!(g.heuristic(4, 0, 0, &s()), -4.0);
        let m = goal_run_away(3.0, Some(10), vec![(0, 0, 0)]);
        assert!(!m.is_in_goal(3, 9, 0, &s()) && m.is_in_goal(3, 10, 0, &s()));
        // -3 * 0.6 + 0 * 1.5
        assert!((m.heuristic(3, 10, 0, &s()) - -1.8).abs() < 1e-12);
        // the inside heuristic is the largest value that is certainly inside: -3 on the axis
        assert_eq!(g.heuristic_inside(&s()), -3.0 * 1.0 - 0.0);
    }

    #[test]
    fn composite_and_inverted() {
        let c = Goal::Composite(vec![Goal::Block { x: 0, y: 0, z: 0 }, Goal::Block { x: 10, y: 0, z: 0 }]);
        assert!(c.is_in_goal(10, 0, 0, &s()) && !c.is_in_goal(5, 0, 0, &s()));
        assert_eq!(c.heuristic(9, 0, 0, &s()), 1.0);
        assert_eq!(c.heuristic_inside(&s()), 0.0);
        assert_eq!(Goal::Composite(vec![]).heuristic(0, 0, 0, &s()), f64::MAX);
        let i = Goal::Inverted(Box::new(Goal::Block { x: 0, y: 0, z: 0 }));
        assert!(!i.is_in_goal(0, 0, 0, &s()));
        assert_eq!(i.heuristic(3, 0, 0, &s()), -3.0);
        assert_eq!(i.heuristic_inside(&s()), f64::NEG_INFINITY);
        // -min of 0 is -0.0
        assert_eq!(i.heuristic(0, 0, 0, &s()).to_bits(), (-0.0f64).to_bits());
    }

    #[test]
    fn three_blocks_and_the_goal_the_processes_never_reach() {
        let g = Goal::ThreeBlocks { x: 0, y: 10, z: 0 };
        assert!(g.is_in_goal(0, 10, 0, &s()) && g.is_in_goal(0, 9, 0, &s()) && g.is_in_goal(0, 8, 0, &s()) && !g.is_in_goal(0, 7, 0, &s()) && !g.is_in_goal(0, 11, 0, &s()));
        // yDiff -1 and -2 cost nothing more than being under the block: -1 is 0, -2 is 0 (-2 + 2), -3 is -1 + 0 block lower
        assert_eq!(g.heuristic(0, 9, 0, &s()), 0.0);
        assert_eq!(g.heuristic(0, 8, 0, &s()), 0.0);
        assert_eq!(g.heuristic(0, 11, 0, &s()), goal_block_calculate(0.0, 1, 0.0, &s()));
        assert_eq!(g.heuristic(0, 6, 0, &s()), goal_block_calculate(0.0, -2, 0.0, &s()));
        assert_eq!(g.heuristic_inside(&s()), 0.0);
        let run = goal_run_away(1.0, Some(5), vec![(0, 5, 0)]);
        let never = Goal::NeverInGoal(Box::new(run.clone()));
        // far from the point, where the run-away goal would be satisfied, it still is not
        assert!(run.is_in_goal(9, 5, 9, &s()) && !never.is_in_goal(9, 5, 9, &s()));
        assert_eq!(never.heuristic(9, 5, 9, &s()), run.heuristic(9, 5, 9, &s()));
        assert_eq!(never.heuristic_inside(&s()), f64::NEG_INFINITY);
    }
}
