// baritone-rs engine: the settings the executor and the pathing state machine read. Written for this project.
// LGPL-3.0-or-later, see lib.rs.
//
// Upstream reads `Baritone.settings()` from anywhere. Here everything a tick reads is one snapshot, taken by the glue
// before the tick: `Config` (the search and the costs, in `config.rs`) and `ExecSettings` (the executor, this file).
// Defaults are upstream's at commit 1b8e87c, except where the field says otherwise.
use crate::config::Config;

/// The executor's settings. Defaults are upstream's (`Settings.java` at 1b8e87c).
#[derive(Clone, Debug, PartialEq)]
pub struct ExecSettings {
    /// `allowSprint`, default true.
    pub allow_sprint: bool,
    /// `sprintAscends`, default true.
    pub sprint_ascends: bool,
    /// `costVerificationLookahead`, default 5.
    pub cost_verification_lookahead: i32,
    /// `maxCostIncrease`, default 10.
    pub max_cost_increase: f64,
    /// `movementTimeoutTicks`, default 100.
    pub movement_timeout_ticks: i32,
    /// `maxPathHistoryLength`, default 300.
    pub max_path_history_length: i32,
    /// `pathHistoryCutoffAmount`, default 50.
    pub path_history_cutoff_amount: i32,
    /// `splicePath`, default true.
    pub splice_path: bool,
    /// `planningTickLookahead`, default 150.
    pub planning_tick_lookahead: i32,
    /// `overshootTraverse`, default true.
    pub overshoot_traverse: bool,
    /// `allowOvershootDiagonalDescend`, default true.
    pub allow_overshoot_diagonal_descend: bool,
    /// `assumeStep`, default false.
    pub assume_step: bool,
    /// `sprintInWater`, default true.
    pub sprint_in_water: bool,
    /// `cancelOnGoalInvalidation`, default true. Only `REVALIDATE_GOAL_AND_PATH` reads it, which no process sends yet.
    pub cancel_on_goal_invalidation: bool,
    /// `randomLooking` and `randomLooking113`: both are 0 here, see `look.rs`. Upstream's defaults are 0.01 and 2.
    pub mouse_sensitivity: f64,
    /// Not upstream. Whether the glue can carry out an `Intent::use_block` (right click on a door or fence gate). When
    /// false (the default), a movement that needs a closed door or gate opened fails with `MoveFailure::NeedsUseBlock`
    /// and the path is recalculated.
    pub can_use_blocks: bool,
}

impl Default for ExecSettings {
    fn default() -> Self {
        ExecSettings {
            allow_sprint: true,
            sprint_ascends: true,
            cost_verification_lookahead: 5,
            max_cost_increase: 10.0,
            movement_timeout_ticks: 100,
            max_path_history_length: 300,
            path_history_cutoff_amount: 50,
            splice_path: true,
            planning_tick_lookahead: 150,
            overshoot_traverse: true,
            allow_overshoot_diagonal_descend: true,
            assume_step: false,
            sprint_in_water: true,
            cancel_on_goal_invalidation: true,
            mouse_sensitivity: 0.5,
            can_use_blocks: false,
        }
    }
}

impl Config {
    /// A settings snapshot with upstream's defaults, for this version of the mod: walking and sprinting only.
    ///
    /// Differences from upstream's defaults, on purpose:
    /// - `allow_break` is false (upstream: true). Nothing is broken, so a movement that needs a block removed costs `COST_INF`.
    /// - `has_throwaway` is false, which is what upstream's `allowPlace` (true) and an empty inventory give. A movement
    ///   that needs a block placed costs `COST_INF`. The glue must keep it false in this version.
    /// - `has_water_bucket` is false: the fall movement without a bucket only.
    /// - `allow_parkour` is false, as upstream's default.
    ///
    /// The fields the game decides (`can_sprint`, `frost_walker`, `water_mult`) are set to a player without
    /// boots and a full stomach. The glue sets them each tick.
    pub fn upstream_defaults() -> Config {
        Config {
            has_throwaway: false,
            has_water_bucket: false,
            can_sprint: true,
            frost_walker: 0,
            water_mult: 1.0,
            allow_break: false,
            allow_break_anyway: vec![],
            blocks_to_avoid: vec!["tripwire".to_string()],
            blocks_to_disallow_breaking: vec![],
            assume_walk_on_water: false,
            assume_walk_on_lava: false,
            allow_walk_on_magma: false,
            allow_vines: false,
            allow_walk_on_bottom_slab: true,
            strict_liquid_check: false,
            avoid_updating_falling: true,
            allow_place_source: true,
            allow_place_flow: true,
            allow_swimming: false,
            allow_parkour: false,
            allow_parkour_place: false,
            allow_parkour_ascend: true,
            allow_jump_at_build_limit: false,
            allow_diagonal_descend: false,
            allow_diagonal_ascend: false,
            allow_downward: true,
            max_fall_no_water: 3,
            max_fall_bucket: 20,
            place_penalty: 20.0,
            break_penalty: 2.0,
            jump_penalty: 2.0,
            walk_on_water_penalty: 3.0,
            cost_heuristic: 3.563,
            axis_height: 120,
            search: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_never_break_or_place() {
        let c = Config::upstream_defaults();
        assert!(!c.allow_break && !c.has_throwaway && !c.has_water_bucket && !c.allow_parkour && !c.allow_swimming);
        assert!(c.allow_downward && !c.allow_diagonal_descend && !c.allow_diagonal_ascend);
    }

    #[test]
    fn exec_defaults_are_upstreams() {
        let e = ExecSettings::default();
        assert_eq!((e.cost_verification_lookahead, e.movement_timeout_ticks, e.planning_tick_lookahead), (5, 100, 150));
        assert_eq!((e.max_path_history_length, e.path_history_cutoff_amount), (300, 50));
        assert!(e.splice_path && e.sprint_ascends && e.overshoot_traverse && !e.assume_step && !e.can_use_blocks);
    }
}
