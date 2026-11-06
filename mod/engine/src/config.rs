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
}
