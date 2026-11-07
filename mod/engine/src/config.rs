// baritone-rs engine: the settings snapshot, the only source of settings for a search. LGPL-3.0-or-later, see lib.rs.
//
// Upstream reads `Baritone.settings()` from anywhere and copies some of it into `CalculationContext`. Here a context
// gets one immutable `Config` and reads nothing global.

#[derive(Clone, Debug)]
pub struct Config {
    // From the player, through the constructor of `CalculationContext`
    pub has_throwaway: bool,
    pub has_water_bucket: bool,
    pub can_sprint: bool,
    pub frost_walker: i32,
    pub water_mult: f32,
    // Settings
    pub allow_break: bool,
    /// Registry paths.
    pub allow_break_anyway: Vec<String>,
    /// Registry paths. Default: tripwire.
    pub blocks_to_avoid: Vec<String>,
    /// Registry paths. Default: empty.
    pub blocks_to_disallow_breaking: Vec<String>,
    pub assume_walk_on_water: bool,
    pub assume_walk_on_lava: bool,
    pub allow_walk_on_magma: bool,
    pub allow_vines: bool,
    pub allow_walk_on_bottom_slab: bool,
    pub strict_liquid_check: bool,
    pub avoid_updating_falling: bool,
    pub allow_place_source: bool,
    pub allow_place_flow: bool,
    pub allow_swimming: bool,
    pub allow_parkour: bool,
    pub allow_parkour_place: bool,
    pub allow_parkour_ascend: bool,
    pub allow_jump_at_build_limit: bool,
    pub allow_diagonal_descend: bool,
    pub allow_diagonal_ascend: bool,
    pub allow_downward: bool,
    pub max_fall_no_water: i32,
    pub max_fall_bucket: i32,
    pub place_penalty: f64,
    pub break_penalty: f64,
    pub jump_penalty: f64,
    pub walk_on_water_penalty: f64,
    /// `costHeuristic`, read by the goals.
    pub cost_heuristic: f64,
    /// `axisHeight`, read by `GoalAxis`.
    pub axis_height: i32,
    /// What the search and the path code read.
    pub search: SearchSettings,
}

/// The settings of the search, the path cutoffs and the favoring. Defaults are upstream's.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchSettings {
    /// `backtrackCostFavoringCoefficient`, default 0.5. 1 turns the favoring of the previous path off.
    pub backtrack_cost_favoring_coefficient: f64,
    /// `pathingMaxChunkBorderFetch`, default 50: the search stops after this many moves into unloaded chunks.
    pub pathing_max_chunk_border_fetch: i32,
    /// `minimumImprovementRepropagation`, default true: a cost must improve by 0.01 to replace the old one.
    pub minimum_improvement_repropagation: bool,
    /// `pathCutoffMinimumLength`, default 30.
    pub path_cutoff_minimum_length: i32,
    /// `pathCutoffFactor`, default 0.9.
    pub path_cutoff_factor: f64,
    /// `cutoffAtLoadBoundary`, default false.
    pub cutoff_at_load_boundary: bool,
    /// `simplifyUnloadedYCoord`, default true.
    pub simplify_unloaded_y_coord: bool,
    /// `primaryTimeoutMS`, default 500. Used when no path is being followed.
    pub primary_timeout_ms: i64,
    /// `failureTimeoutMS`, default 2000.
    pub failure_timeout_ms: i64,
    /// `planAheadPrimaryTimeoutMS`, default 4000. Used when planning the next segment.
    pub plan_ahead_primary_timeout_ms: i64,
    /// `planAheadFailureTimeoutMS`, default 5000.
    pub plan_ahead_failure_timeout_ms: i64,
    /// `avoidance`, default false.
    pub avoidance: bool,
    /// `mobSpawnerAvoidanceCoefficient`, default 2.0.
    pub mob_spawner_avoidance_coefficient: f64,
    /// `mobSpawnerAvoidanceRadius`, default 16.
    pub mob_spawner_avoidance_radius: i32,
    /// `mobAvoidanceCoefficient`, default 1.5.
    pub mob_avoidance_coefficient: f64,
    /// `mobAvoidanceRadius`, default 8.
    pub mob_avoidance_radius: i32,
    /// Not upstream: `allowWalkOnBuildLimit`. Upstream's search refuses any node whose feet are above `maxY`, so a player
    /// standing on the world's highest block layer (feet at `maxY + 1`) can't move at all (U15). When true, feet may be at
    /// `maxY + 1`, one above the top layer, and no higher. False here keeps upstream's search; the mod's settings default it to true.
    pub walk_on_build_limit: bool,
}

impl Default for SearchSettings {
    fn default() -> Self {
        SearchSettings {
            backtrack_cost_favoring_coefficient: 0.5,
            pathing_max_chunk_border_fetch: 50,
            minimum_improvement_repropagation: true,
            path_cutoff_minimum_length: 30,
            path_cutoff_factor: 0.9,
            cutoff_at_load_boundary: false,
            simplify_unloaded_y_coord: true,
            primary_timeout_ms: 500,
            failure_timeout_ms: 2000,
            plan_ahead_primary_timeout_ms: 4000,
            plan_ahead_failure_timeout_ms: 5000,
            avoidance: false,
            mob_spawner_avoidance_coefficient: 2.0,
            mob_spawner_avoidance_radius: 16,
            mob_avoidance_coefficient: 1.5,
            mob_avoidance_radius: 8,
            walk_on_build_limit: false,
        }
    }
}
