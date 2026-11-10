// baritone-rs engine: the settings the executor and the pathing state machine read. Written for this project.
// LGPL-3.0-or-later, see lib.rs.
//
// Upstream reads `Baritone.settings()` from anywhere. Here everything a tick reads is one snapshot, taken by the glue
// before the tick: `Config` (the search and the costs, in `config.rs`) and `ExecSettings` (the executor, this file).
// Defaults are upstream's at commit 1b8e87c, except where the field says otherwise.
use crate::config::Config;
use crate::inventory::ToolSettings;
use crate::schematic::{BlockMirror, BlockRotation};

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
    /// `cancelOnGoalInvalidation`, default true. Only `REVALIDATE_GOAL_AND_PATH` reads it, which the mine and get-to-block processes send.
    pub cancel_on_goal_invalidation: bool,
    /// `randomLooking` and `randomLooking113`: both are 0 here, see `look.rs`. Upstream's defaults are 0.01 and 2.
    pub mouse_sensitivity: f64,
    /// `allowPlace`, default true. `Config::has_throwaway` is this and a block to place; the executor places nothing when it is off.
    pub allow_place: bool,
    /// `assumeSafeWalk`, default false: the player does not fall off an edge when walking, so bridging neither sneaks nor backs up.
    pub assume_safe_walk: bool,
    /// `walkWhileBreaking`, default true: a traverse walks on while it is breaking the blocks ahead.
    pub walk_while_breaking: bool,
    /// `pauseMiningForFallingBlocks`, default true: the movement waits while a falling block is where it would mine.
    pub pause_mining_for_falling_blocks: bool,
    /// `remainWithExistingLookDirection`, default true: an aim at a block the crosshair is on keeps the view.
    pub remain_with_existing_look_direction: bool,
    /// The settings of the tool and the block to place: `autoTool`, `acceptableThrowawayItems` and the others.
    pub tool: ToolSettings,
    /// The settings of the mine and get-to-block processes.
    pub mine: MineSettings,
    /// The settings of the follow process.
    pub follow: FollowSettings,
    /// The settings of the explore process.
    pub explore: ExploreSettings,
}

/// The settings `ExploreProcess` reads. Defaults are upstream's (`Settings.java` at 1b8e87c).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExploreSettings {
    /// `exploreChunkSetMinimumSize`, default 10: how many of the closest unexplored chunks make the goal.
    pub explore_chunk_set_minimum_size: i32,
    /// `worldExploringChunkOffset`, default 0: the goal is this many chunks further out than the middle of the chunk.
    pub world_exploring_chunk_offset: i32,
    /// `exploreMaintainY`, default 64: the height to keep on the way; -1 for none.
    pub explore_maintain_y: i32,
    /// `disableCompletionCheck`, default false: do not stop when no chunk is left to explore.
    pub disable_completion_check: bool,
}

impl Default for ExploreSettings {
    fn default() -> Self {
        ExploreSettings { explore_chunk_set_minimum_size: 10, world_exploring_chunk_offset: 0, explore_maintain_y: 64, disable_completion_check: false }
    }
}

/// The settings `FollowProcess` reads. Defaults are upstream's (`Settings.java` at 1b8e87c).
#[derive(Clone, Debug, PartialEq)]
pub struct FollowSettings {
    /// `followRadius`, default 3: the radius of the `GoalNear` around the target position.
    pub follow_radius: i32,
    /// `followOffsetDistance`, default 0: how far from the entity the `GoalNear` is put. 0 puts it on the entity.
    pub follow_offset_distance: f64,
    /// `followOffsetDirection`, default 0: the direction of that offset, in degrees (a yaw).
    pub follow_offset_direction: f32,
    /// `followTargetMaxDistance`, default 0: entities further than this are not followed. 0 is no limit.
    pub follow_target_max_distance: i32,
}

impl Default for FollowSettings {
    fn default() -> Self {
        FollowSettings { follow_radius: 3, follow_offset_distance: 0.0, follow_offset_direction: 0.0, follow_target_max_distance: 0 }
    }
}

/// The settings `MineProcess` and `GetToBlockProcess` read. Defaults are upstream's (`Settings.java` at 1b8e87c). `notificationOnMineFail` is not
/// here: see docs/deviations.md.
#[derive(Clone, Debug, PartialEq)]
pub struct MineSettings {
    /// `mineGoalUpdateInterval`, default 5: ticks between two scans of the world. 0 never scans again after the first.
    pub mine_goal_update_interval: i32,
    /// `mineMaxOreLocationsCount`, default 64.
    pub mine_max_ore_locations_count: i32,
    /// `maxCachedWorldScanCount`, default 10: after this many places of the block in the cache, the search stops widening.
    pub max_cached_world_scan_count: i32,
    /// `extendCacheOnThreshold`, default false: when the cache gave fewer places than the maximum, scan the loaded world too.
    pub extend_cache_on_threshold: bool,
    /// `minYLevelWhileMining`, default 0: relative to the world's lowest Y.
    pub min_y_level_while_mining: i32,
    /// `maxYLevelWhileMining`, default 2031.
    pub max_y_level_while_mining: i32,
    /// `allowOnlyExposedOres`, default false.
    pub allow_only_exposed_ores: bool,
    /// `allowOnlyExposedOresDistance`, default 1.
    pub allow_only_exposed_ores_distance: i32,
    /// `exploreForBlocks`, default true.
    pub explore_for_blocks: bool,
    /// `mineScanDroppedItems`, default true.
    pub mine_scan_dropped_items: bool,
    /// `mineDropLoiterDurationMSThanksLouca`, default 250.
    pub mine_drop_loiter_duration_ms: i64,
    /// `legitMine`, default false.
    pub legit_mine: bool,
    /// `legitMineYLevel`, default -59.
    pub legit_mine_y_level: i32,
    /// `legitMineIncludeDiagonals`, default false.
    pub legit_mine_include_diagonals: bool,
    /// `forceInternalMining`, default true.
    pub force_internal_mining: bool,
    /// `internalMiningAirException`, default true.
    pub internal_mining_air_exception: bool,
    /// `blacklistClosestOnFailure`, default true.
    pub blacklist_closest_on_failure: bool,
    /// `rightClickContainerOnArrival`, default true.
    pub right_click_container_on_arrival: bool,
    /// `enterPortal`, default true.
    pub enter_portal: bool,
}

impl Default for MineSettings {
    fn default() -> Self {
        MineSettings {
            mine_goal_update_interval: 5,
            mine_max_ore_locations_count: 64,
            max_cached_world_scan_count: 10,
            extend_cache_on_threshold: false,
            min_y_level_while_mining: 0,
            max_y_level_while_mining: 2031,
            allow_only_exposed_ores: false,
            allow_only_exposed_ores_distance: 1,
            explore_for_blocks: true,
            mine_scan_dropped_items: true,
            mine_drop_loiter_duration_ms: 250,
            legit_mine: false,
            legit_mine_y_level: -59,
            legit_mine_include_diagonals: false,
            force_internal_mining: true,
            internal_mining_air_exception: true,
            blacklist_closest_on_failure: true,
            right_click_container_on_arrival: true,
            enter_portal: true,
        }
    }
}

/// The settings `BuilderProcess` and its calculation context read. Defaults are upstream's (`Settings.java` at 1b8e87c). Block lists hold
/// names in `BlockUtils` form (the path for `minecraft:`). `buildOnlySelection` and `notificationOnBuildFinished` are not here (`allowInventory` is in `ToolSettings`):
/// see docs/deviations.md.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildSettings {
    /// `buildInLayers`, default false.
    pub build_in_layers: bool,
    /// `layerOrder`, default false: false builds from the bottom up, true from the top down.
    pub layer_order: bool,
    /// `layerHeight`, default 1.
    pub layer_height: i32,
    /// `startAtLayer`, default 0.
    pub start_at_layer: i32,
    /// `skipFailedLayers`, default false.
    pub skip_failed_layers: bool,
    /// `buildRepeat`, default (0, 0, 0): where the next copy of the build goes.
    pub build_repeat: (i32, i32, i32),
    /// `buildRepeatCount`, default -1: how many copies; -1 is no limit.
    pub build_repeat_count: i32,
    /// `buildRepeatSneaky`, default true.
    pub build_repeat_sneaky: bool,
    /// `breakFromAbove`, default false.
    pub break_from_above: bool,
    /// `goalBreakFromAbove`, default false.
    pub goal_break_from_above: bool,
    /// `mapArtMode`, default false.
    pub map_art_mode: bool,
    /// `okIfWater`, default false.
    pub ok_if_water: bool,
    /// `incorrectSize`, default 100.
    pub incorrect_size: i32,
    /// `breakCorrectBlockPenaltyMultiplier`, default 10.
    pub break_correct_block_penalty_multiplier: f64,
    /// `placeIncorrectBlockPenaltyMultiplier`, default 2.
    pub place_incorrect_block_penalty_multiplier: f64,
    /// `schematicOrientationX`, `Y` and `Z`, default false: the origin is moved by the schematic's size, so it is the far corner.
    pub schematic_orientation_x: bool,
    pub schematic_orientation_y: bool,
    pub schematic_orientation_z: bool,
    /// `buildSchematicRotation`, default NONE.
    pub build_schematic_rotation: BlockRotation,
    /// `buildSchematicMirror`, default NONE.
    pub build_schematic_mirror: BlockMirror,
    /// `builderTickScanRadius`, default 5.
    pub builder_tick_scan_radius: i32,
    /// `distanceTrim`, default true.
    pub distance_trim: bool,
    /// `buildIgnoreBlocks`, default empty: blocks that do not count as wrong where the schematic wants air.
    pub build_ignore_blocks: Vec<String>,
    /// `buildSkipBlocks`, default empty: blocks the schematic wants that are left out of the build.
    pub build_skip_blocks: Vec<String>,
    /// `okIfAir`, default empty: blocks that may be missing from where the schematic wants them.
    pub ok_if_air: Vec<String>,
    /// `buildValidSubstitutes`, default empty: a block and the blocks that count as it.
    pub build_valid_substitutes: Vec<(String, Vec<String>)>,
    /// `buildSubstitutes`, default empty: a block and the blocks to place in its stead.
    pub build_substitutes: Vec<(String, Vec<String>)>,
    /// `buildIgnoreExisting`, default false.
    pub build_ignore_existing: bool,
    /// `buildIgnoreDirection`, default false.
    pub build_ignore_direction: bool,
    /// `buildIgnoreProperties`, default empty: property names that do not count.
    pub build_ignore_properties: Vec<String>,
}

impl Default for BuildSettings {
    fn default() -> Self {
        BuildSettings {
            build_in_layers: false,
            layer_order: false,
            layer_height: 1,
            start_at_layer: 0,
            skip_failed_layers: false,
            build_repeat: (0, 0, 0),
            build_repeat_count: -1,
            build_repeat_sneaky: true,
            break_from_above: false,
            goal_break_from_above: false,
            map_art_mode: false,
            ok_if_water: false,
            incorrect_size: 100,
            break_correct_block_penalty_multiplier: 10.0,
            place_incorrect_block_penalty_multiplier: 2.0,
            schematic_orientation_x: false,
            schematic_orientation_y: false,
            schematic_orientation_z: false,
            build_schematic_rotation: BlockRotation::None,
            build_schematic_mirror: BlockMirror::None,
            builder_tick_scan_radius: 5,
            distance_trim: true,
            build_ignore_blocks: vec![],
            build_skip_blocks: vec![],
            ok_if_air: vec![],
            build_valid_substitutes: vec![],
            build_substitutes: vec![],
            build_ignore_existing: false,
            build_ignore_direction: false,
            build_ignore_properties: vec![],
        }
    }
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
            allow_place: true,
            assume_safe_walk: false,
            walk_while_breaking: true,
            pause_mining_for_falling_blocks: true,
            remain_with_existing_look_direction: true,
            tool: ToolSettings::default(),
            mine: MineSettings::default(),
            follow: FollowSettings::default(),
            explore: ExploreSettings::default(),
        }
    }
}

impl Config {
    /// A settings snapshot with upstream's defaults (`Settings.java` at 1b8e87c): breaking and placing are on, parkour and swimming are off.
    ///
    /// The fields that come from the player's inventory are false until the glue sets them each tick, with
    /// `Settings::apply_inventory`: `has_throwaway` is upstream's `allowPlace` and a block to place, and `has_water_bucket` is
    /// `allowWaterBucketFall` and a water bucket in the hotbar outside the Nether. With an empty hotbar nothing is placed and no fall
    /// lands in a bucket, which is what upstream gives too. The fields the game decides (`can_sprint`, `frost_walker`, `water_mult`) are set
    /// to a player without boots and a full stomach; the glue sets them each tick.
    pub fn upstream_defaults() -> Config {
        Config {
            has_throwaway: false,
            has_water_bucket: false,
            can_sprint: true,
            frost_walker: 0,
            water_mult: 1.0,
            allow_break: true,
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
    fn defaults_are_upstreams_and_wait_for_the_inventory() {
        let c = Config::upstream_defaults();
        // breaking is on; placing and the bucket wait for a block and a bucket in the hotbar
        assert!(c.allow_break && !c.has_throwaway && !c.has_water_bucket && !c.allow_parkour && !c.allow_swimming);
        assert!(c.allow_downward && !c.allow_diagonal_descend && !c.allow_diagonal_ascend);
        assert_eq!((c.max_fall_no_water, c.max_fall_bucket), (3, 20));
        assert!(ExecSettings::default().allow_place);
    }

    #[test]
    fn exec_defaults_are_upstreams() {
        let e = ExecSettings::default();
        assert_eq!((e.cost_verification_lookahead, e.movement_timeout_ticks, e.planning_tick_lookahead), (5, 100, 150));
        assert_eq!((e.max_path_history_length, e.path_history_cutoff_amount), (300, 50));
        assert!(e.splice_path && e.sprint_ascends && e.overshoot_traverse && !e.assume_step);
    }
}
