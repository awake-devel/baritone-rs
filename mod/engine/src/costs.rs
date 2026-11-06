// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/pathing/movement/ActionCosts.java
// LGPL-3.0-or-later, see lib.rs.
//
// This is the one module that holds the float calls whose rounding can differ between Java and Rust
// (see "Floating point" in docs/PORTING-SAFETY.md).
use std::sync::OnceLock;

pub const WALK_ONE_BLOCK_COST: f64 = 20.0 / 4.317;
pub const WALK_ONE_IN_WATER_COST: f64 = 20.0 / 2.2;
pub const SWIM_ONE_BLOCK_COST: f64 = 20.0 / 2.86;
pub const WALK_ONE_OVER_SOUL_SAND_COST: f64 = WALK_ONE_BLOCK_COST * 2.0;
pub const LADDER_UP_ONE_COST: f64 = 20.0 / 2.35;
pub const LADDER_DOWN_ONE_COST: f64 = 20.0 / 3.0;
pub const SNEAK_ONE_BLOCK_COST: f64 = 20.0 / 1.3;
pub const SPRINT_ONE_BLOCK_COST: f64 = 20.0 / 5.612;
pub const SPRINT_MULTIPLIER: f64 = SPRINT_ONE_BLOCK_COST / WALK_ONE_BLOCK_COST;
pub const WALK_OFF_BLOCK_COST: f64 = WALK_ONE_BLOCK_COST * 0.8;
pub const CENTER_AFTER_FALL_COST: f64 = WALK_ONE_BLOCK_COST - WALK_OFF_BLOCK_COST;
pub const COST_INF: f64 = 1000000.0;
/// `Math.sqrt(2)`, the value of `MovementDiagonal.SQRT_2` and `GoalXZ.SQRT_2`.
pub const SQRT_2: f64 = std::f64::consts::SQRT_2;

/// Java's `Math.min(double,double)`: NaN propagates, and `-0.0` is smaller than `0.0` (Rust's `f64::min` does neither).
pub fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == (-0.0f64).to_bits() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// Java's `Math.max(double,double)`: NaN propagates, and `0.0` is larger than `-0.0` (Rust's `f64::max` does neither).
pub fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0f64).to_bits() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// `ActionCosts.velocity`: fall speed after the given number of ticks.
fn velocity(ticks: i32) -> f64 {
    (0.98f64.powf(ticks as f64) - 1.0) * -3.92
}

/// `ActionCosts.distanceToTicks`: ticks needed to fall the given distance.
pub fn distance_to_ticks(distance: f64) -> f64 {
    if distance == 0.0 {
        return 0.0;
    }
    let mut tmp_distance = distance;
    let mut tick_count = 0;
    loop {
        let fall_distance = velocity(tick_count);
        if tmp_distance <= fall_distance {
            return tick_count as f64 + tmp_distance / fall_distance;
        }
        tmp_distance -= fall_distance;
        tick_count += 1;
    }
}

/// `ActionCosts.generateFallNBlocksCost`: the 4097-entry fall table (FALL_N_BLOCKS_COST), built once and cached.
pub fn fall_n_blocks_cost() -> &'static [f64] {
    static T: OnceLock<Vec<f64>> = OnceLock::new();
    T.get_or_init(|| (0..4097).map(|i| distance_to_ticks(i as f64)).collect())
}

/// The constant `JUMP_ONE_BLOCK_COST`: `FALL_1_25_BLOCKS_COST - FALL_0_25_BLOCKS_COST`, built once and cached.
pub fn jump_one_block_cost() -> f64 {
    static T: OnceLock<f64> = OnceLock::new();
    *T.get_or_init(|| distance_to_ticks(1.25) - distance_to_ticks(0.25))
}
