// baritone-rs engine: the hotbar as the executor reads it, and the choice of tool and of block to place.
// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/ToolSet.java (the choice of the best slot, the
// speed formula and the strength used by the costs) and, in new structure, the parts of
// src/main/java/baritone/behavior/InventoryBehavior.java that a path needs (`throwaway`, `selectThrowawayForLocation`,
// `hasGenericThrowaway`). LGPL-3.0-or-later, see lib.rs.
//
// Upstream reads `player.getInventory()` everywhere. Here the glue answers the questions through [`Inventory`] and the engine
// never touches the game: it decides which slot to select, and the glue sets `game.selected`.
//
// Left out on purpose (docs/deviations.md D24): `allowInventory` (moving a stack from the main inventory into the hotbar needs
// window clicks) and the builder's `placeAt` branch of `selectThrowawayForLocation`, which is `null` outside the builder.

/// The hotbar has nine slots.
pub const HOTBAR: usize = 9;

/// One stack, as the choice of tool and block needs it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotInfo {
    /// The item's registry path without the namespace (`dirt`, `iron_pickaxe`), or an empty string for an empty stack.
    pub item: String,
    pub count: u32,
    /// Upstream's `ToolSet.getMaterialCost`: the index in wooden, stone, iron, gold, diamond, netherite tool materials, or -1 for any other item.
    pub material_cost: i32,
    /// `ToolSet.hasSilkTouch`.
    pub silk_touch: bool,
    /// The stack is in `ItemTags.SWORDS`.
    pub sword: bool,
    /// The item is a `PickaxeItem`.
    pub pickaxe: bool,
    /// `ItemStack.getDamageValue` and `getMaxDamage`.
    pub damage: i32,
    pub max_damage: i32,
}

impl SlotInfo {
    pub fn empty() -> SlotInfo {
        SlotInfo { material_cost: -1, ..SlotInfo::default() }
    }

    /// A stack of a plain item, with nothing special about it.
    pub fn of(item: &str, count: u32) -> SlotInfo {
        SlotInfo { item: item.to_string(), count, material_cost: -1, ..SlotInfo::default() }
    }

    /// `ItemStack.isEmpty`.
    pub fn is_empty(&self) -> bool {
        self.item.is_empty() || self.count == 0
    }
}

/// What the executor asks of the player's inventory. The glue fills it from the real one.
pub trait Inventory {
    /// Upstream's `Inventory.selected`: the hotbar slot in the hand, 0 to 8.
    fn selected(&self) -> usize;
    /// A hotbar slot, 0 to 8.
    fn slot(&self, i: usize) -> SlotInfo;
    /// `Inventory.offhand.get(0)`.
    fn offhand(&self) -> SlotInfo;
    /// Upstream's `ToolSet.calculateSpeedVsBlock` for the stack of the slot and the default state of the block of this state: how much of a block one tick of
    /// mining takes with that stack. Negative for a block that cannot be broken. The glue computes it with [`calculate_speed_vs_block`]
    /// from the item's destroy speed, its efficiency and whether it is the right tool.
    fn speed_vs_state(&self, slot: usize, state: usize) -> f64;
    /// Upstream's `ToolSet.potionAmplifier`: 1 without effects.
    fn potion_amplifier(&self) -> f64 {
        1.0
    }
}

/// An inventory with nothing in it: for tests and for a glue that has no hotbar to show yet.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoInventory;

impl Inventory for NoInventory {
    fn selected(&self) -> usize {
        0
    }
    fn slot(&self, _i: usize) -> SlotInfo {
        SlotInfo::empty()
    }
    fn offhand(&self) -> SlotInfo {
        SlotInfo::empty()
    }
    fn speed_vs_state(&self, _slot: usize, _state: usize) -> f64 {
        0.0
    }
}

/// The settings that `ToolSet` and `MovementHelper.switchToBestToolFor` read. Defaults are upstream's.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSettings {
    /// `autoTool`, default true.
    pub auto_tool: bool,
    /// `assumeExternalAutoTool`, default false.
    pub assume_external_auto_tool: bool,
    /// `preferSilkTouch`, default false.
    pub prefer_silk_touch: bool,
    /// `useSwordToMine`, default true.
    pub use_sword_to_mine: bool,
    /// `itemSaver`, default false.
    pub item_saver: bool,
    /// `itemSaverThreshold`, default 10.
    pub item_saver_threshold: i32,
    /// `considerPotionEffects`, default true.
    pub consider_potion_effects: bool,
    /// `blocksToAvoidBreaking`: registry paths. Default: crafting table, furnace, chest, trapped chest.
    pub blocks_to_avoid_breaking: Vec<String>,
    /// `avoidBreakingMultiplier`, default 0.1.
    pub avoid_breaking_multiplier: f64,
    /// `acceptableThrowawayItems`: registry paths. Default: dirt, cobblestone, netherrack, stone.
    pub acceptable_throwaway_items: Vec<String>,
}

impl Default for ToolSettings {
    fn default() -> Self {
        ToolSettings {
            auto_tool: true,
            assume_external_auto_tool: false,
            prefer_silk_touch: false,
            use_sword_to_mine: true,
            item_saver: false,
            item_saver_threshold: 10,
            consider_potion_effects: true,
            blocks_to_avoid_breaking: ["crafting_table", "furnace", "chest", "trapped_chest"].iter().map(|s| s.to_string()).collect(),
            avoid_breaking_multiplier: 0.1,
            acceptable_throwaway_items: ["dirt", "cobblestone", "netherrack", "stone"].iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// `ToolSet.calculateSpeedVsBlock`: with the game's numbers given. `hardness` is the block's destroy speed (negative: unbreakable),
/// `item_speed` is `item.getDestroySpeed(state)`, `efficiency` is what the first mining-efficiency attribute effect of the item's
/// enchantments adds (0 when there is none; it only counts when `item_speed` is above 1), `requires_tool` is
/// `state.requiresCorrectToolForDrops()`, `item_empty` is `item.isEmpty()` and `correct_tool` is `item.isCorrectToolForDrops(state)`.
/// The arithmetic is `float` as upstream's, widened to `double` at the end.
pub fn calculate_speed_vs_block(hardness: f32, item_speed: f32, efficiency: f32, requires_tool: bool, item_empty: bool, correct_tool: bool) -> f64 {
    if hardness < 0.0 {
        return -1.0;
    }
    let mut speed = item_speed;
    if speed > 1.0 {
        speed += efficiency;
    }
    speed /= hardness;
    if !requires_tool || (!item_empty && correct_tool) {
        f64::from(speed / 30.0)
    } else {
        f64::from(speed / 100.0)
    }
}

/// `ToolSet.getBestSlot(Block,boolean,boolean)`: the hotbar slot that mines the block fastest. Ties go to the cheaper material, and to
/// silk touch when asked for. `state` is a state of the block (upstream reads its default state). `pathing_calculation` asks for the
/// held slot when `autoTool` is off.
/// `ToolSet.getBestSlot(Block,boolean)`: the same with `pathing_calculation` false.
pub fn get_best_slot(inv: &dyn Inventory, state: usize, prefer_silk_touch: bool, pathing_calculation: bool, ts: &ToolSettings) -> usize {
    /*
    If we actually want know what efficiency our held item has instead of the best one
    possible, this lets us make pathing depend on the actual tool to be used (if auto tool is disabled)
    */
    if !ts.auto_tool && pathing_calculation {
        return inv.selected();
    }
    let mut best = 0usize;
    let mut highest_speed = f64::NEG_INFINITY;
    let mut lowest_cost = i32::MIN;
    let mut best_silk_touch = false;
    for i in 0..HOTBAR {
        let item = inv.slot(i);
        if !ts.use_sword_to_mine && item.sword {
            continue;
        }
        if ts.item_saver && item.damage.wrapping_add(ts.item_saver_threshold) >= item.max_damage && item.max_damage > 1 {
            continue;
        }
        let speed = inv.speed_vs_state(i, state);
        let silk_touch = item.silk_touch;
        if speed > highest_speed {
            highest_speed = speed;
            best = i;
            lowest_cost = item.material_cost;
            best_silk_touch = silk_touch;
        } else if speed == highest_speed {
            let cost = item.material_cost;
            if (cost < lowest_cost && (silk_touch || !best_silk_touch)) || (prefer_silk_touch && !best_silk_touch && silk_touch) {
                highest_speed = speed;
                best = i;
                lowest_cost = cost;
                best_silk_touch = silk_touch;
            }
        }
    }
    best
}

/// `ToolSet.avoidanceMultiplier`: the multiplier of a block that is to be avoided breaking, else 1.
fn avoidance_multiplier(name: &str, ts: &ToolSettings) -> f64 {
    if ts.blocks_to_avoid_breaking.iter().any(|b| b == name) {
        ts.avoid_breaking_multiplier
    } else {
        1.0
    }
}

/// `ToolSet.getBestDestructionTime`: the speed with the best tool, times the avoidance multiplier of the block.
pub fn get_best_destruction_time(inv: &dyn Inventory, state: usize, name: &str, ts: &ToolSettings) -> f64 {
    let slot = get_best_slot(inv, state, false, true, ts);
    inv.speed_vs_state(slot, state) * avoidance_multiplier(name, ts)
}

/// `ToolSet.getStrVsBlock`: using the best tool on the hotbar, how fast the block is mined, as 1 / (ticks). This is the number the
/// glue puts in `Tables::tool_strength` for the state. With `considerPotionEffects` the speed is scaled by the effects of the player
/// `ToolSet.<init>`: this function picks what the constructor's lambda picks; `potionAmplifier` is read once per `ToolSet`.
pub fn get_str_vs_block(inv: &dyn Inventory, state: usize, name: &str, ts: &ToolSettings) -> f64 {
    let t = get_best_destruction_time(inv, state, name, ts);
    if ts.consider_potion_effects {
        inv.potion_amplifier() * t
    } else {
        t
    }
}

/// `ToolSet.potionAmplifier`: Haste (`dig_speed_amplifier`, none: `None`) and Mining Fatigue (`dig_slowdown_amplifier`) as multipliers.
pub fn potion_amplifier(haste_amplifier: Option<i32>, mining_fatigue_amplifier: Option<i32>) -> f64 {
    let mut speed = 1.0;
    if let Some(a) = haste_amplifier {
        speed *= 1.0 + f64::from(a + 1) * 0.2;
    }
    if let Some(a) = mining_fatigue_amplifier {
        speed *= match a {
            0 => 0.3,
            1 => 0.09,
            2 => 0.0027, // you might think that 0.09*0.3 = 0.027 so that should be next, that would make too much sense. it's 0.0027.
            _ => 0.00081,
        };
    }
    speed
}

/// `MovementHelper.switchToBestToolFor(IPlayerContext,BlockState,ToolSet,boolean)`: the slot to select to mine the block, if the tool is
/// to be chosen at all (`autoTool` on and `assumeExternalAutoTool` off). `None` leaves the hand alone.
/// `MovementHelper.switchToBestToolFor(IPlayerContext,BlockState)`: the same, with `preferSilkTouch` from the settings.
pub fn switch_to_best_tool_for(inv: &dyn Inventory, state: usize, ts: &ToolSettings) -> Option<usize> {
    if ts.auto_tool && !ts.assume_external_auto_tool {
        Some(get_best_slot(inv, state, ts.prefer_silk_touch, false, ts))
    } else {
        None
    }
}

/// Upstream's `InventoryBehavior.throwaway(boolean,Predicate)` without `allowInventory`: the hotbar slot to select so that a stack that
/// `desired` accepts is in the hand, or `None` when there is none. The first hotbar slot that matches wins. A match in the off hand
/// needs the main hand to hold something that does not right click: the first empty slot or pickaxe.
pub fn throwaway(inv: &dyn Inventory, desired: &dyn Fn(&SlotInfo) -> bool) -> Option<usize> {
    for i in 0..HOTBAR {
        if desired(&inv.slot(i)) {
            return Some(i);
        }
    }
    if desired(&inv.offhand()) {
        // main hand takes precedence over off hand
        // that means that if we have block A selected in main hand and block B in off hand, right clicking places block B
        // we've already checked above ^ and the main hand can't possible have an acceptablethrowawayitem
        // so we need to select in the main hand something that doesn't right click
        // so not a shovel, not a hoe, not a block, etc
        for i in 0..HOTBAR {
            let item = inv.slot(i);
            if item.is_empty() || item.pickaxe {
                return Some(i);
            }
        }
    }
    None
}

/// Upstream's `InventoryBehavior.selectThrowawayForLocation(boolean,int,int,int)`, with no builder (`placeAt` is `null`): the slot to select
/// for a block that is placed to walk on. The first acceptable item of the list that is in the hotbar wins, not the first slot.
pub fn select_throwaway(inv: &dyn Inventory, acceptable: &[String]) -> Option<usize> {
    for item in acceptable {
        if let Some(slot) = throwaway(inv, &|s: &SlotInfo| !s.is_empty() && s.item == *item) {
            return Some(slot);
        }
    }
    None
}

/// Upstream's `InventoryBehavior.hasGenericThrowaway`. The glue sets `Config::has_throwaway` to this and `allowPlace`.
pub fn has_generic_throwaway(inv: &dyn Inventory, acceptable: &[String]) -> bool {
    select_throwaway(inv, acceptable).is_some()
}

/// The first hotbar slot that holds this item, like `Inventory.findSlotMatchingItem` followed by `Inventory.isHotbarSlot` (a match
/// in the main inventory only counts when no hotbar slot has the item before it, which this reproduces by looking at the hotbar first).
pub fn find_hotbar_slot(inv: &dyn Inventory, item: &str) -> Option<usize> {
    (0..HOTBAR).find(|&i| {
        let s = inv.slot(i);
        !s.is_empty() && s.item == item
    })
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// A hotbar for tests. `speeds[slot]` maps a state id to the speed that slot's stack mines it with.
    #[derive(Clone, Debug)]
    pub struct TestInventory {
        pub selected: usize,
        pub slots: Vec<SlotInfo>,
        pub offhand: SlotInfo,
        /// Per slot: the speed against state 0, 1, 2, ... (missing: 1 / 100, an empty hand on a block that needs a tool)
        pub speeds: Vec<Vec<f64>>,
        pub amplifier: f64,
    }

    impl TestInventory {
        pub fn empty() -> TestInventory {
            TestInventory { selected: 0, slots: vec![SlotInfo::empty(); HOTBAR], offhand: SlotInfo::empty(), speeds: vec![vec![]; HOTBAR], amplifier: 1.0 }
        }

        /// Puts a stack in a slot, with its speed against each state.
        pub fn put(&mut self, slot: usize, info: SlotInfo, speeds: &[f64]) {
            self.slots[slot] = info;
            self.speeds[slot] = speeds.to_vec();
        }
    }

    impl Inventory for TestInventory {
        fn selected(&self) -> usize {
            self.selected
        }
        fn slot(&self, i: usize) -> SlotInfo {
            self.slots[i].clone()
        }
        fn offhand(&self) -> SlotInfo {
            self.offhand.clone()
        }
        fn speed_vs_state(&self, slot: usize, state: usize) -> f64 {
            self.speeds[slot].get(state).copied().unwrap_or(0.01)
        }
        fn potion_amplifier(&self) -> f64 {
            self.amplifier
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TestInventory;
    use super::*;

    fn tool(item: &str, cost: i32) -> SlotInfo {
        SlotInfo { material_cost: cost, ..SlotInfo::of(item, 1) }
    }

    #[test]
    fn the_speed_formula_follows_toolset() {
        // iron pickaxe (speed 6) on stone (hardness 1.5), right tool: 6 / 1.5 / 30
        assert_eq!(calculate_speed_vs_block(1.5, 6.0, 0.0, true, false, true), f64::from(6.0f32 / 1.5 / 30.0));
        // an empty hand on stone: needs a tool, so / 100
        assert_eq!(calculate_speed_vs_block(1.5, 1.0, 0.0, true, true, false), f64::from(1.0f32 / 1.5 / 100.0));
        // dirt by hand: no tool needed, / 30
        assert_eq!(calculate_speed_vs_block(0.5, 1.0, 0.0, false, true, false), f64::from(1.0f32 / 0.5 / 30.0));
        // unbreakable
        assert_eq!(calculate_speed_vs_block(-1.0, 6.0, 0.0, true, false, true), -1.0);
        // efficiency counts only above 1
        assert_eq!(calculate_speed_vs_block(1.5, 6.0, 4.0, true, false, true), f64::from(10.0f32 / 1.5 / 30.0));
        assert_eq!(calculate_speed_vs_block(0.5, 1.0, 4.0, false, false, false), f64::from(1.0f32 / 0.5 / 30.0));
        // the right tool for the block but wrong for the drops: / 100
        assert_eq!(calculate_speed_vs_block(1.5, 6.0, 0.0, true, false, false), f64::from(6.0f32 / 1.5 / 100.0));
        // hardness 0: infinite
        assert_eq!(calculate_speed_vs_block(0.0, 1.0, 0.0, false, true, false), f64::INFINITY);
    }

    // Table of cases for the choice of slot. Each row is worked out by hand from ToolSet.getBestSlot at 1b8e87c.
    fn hotbar() -> TestInventory {
        let mut inv = TestInventory::empty();
        // state 0 = stone, state 1 = dirt
        inv.put(0, tool("wooden_pickaxe", 0), &[0.05, 0.1]);
        inv.put(1, tool("iron_pickaxe", 2), &[0.133, 0.1]);
        inv.put(2, tool("diamond_pickaxe", 4), &[0.133, 0.1]);
        inv.put(3, tool("iron_shovel", 2), &[0.01, 0.2]);
        inv.put(4, SlotInfo { sword: true, ..tool("diamond_sword", 4) }, &[0.01, 0.1]);
        inv
    }

    #[test]
    fn the_fastest_slot_wins_and_the_cheaper_material_breaks_a_tie() {
        let inv = hotbar();
        let ts = ToolSettings::default();
        // stone: iron and diamond pickaxe tie at 0.133; the iron one is cheaper (2 < 4), and it comes first anyway
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 1);
        // dirt: the iron shovel
        assert_eq!(get_best_slot(&inv, 1, false, false, &ts), 3);
        // swap the order of the two tied pickaxes: the cheaper one still wins
        let mut inv2 = hotbar();
        inv2.put(1, tool("diamond_pickaxe", 4), &[0.133, 0.1]);
        inv2.put(2, tool("iron_pickaxe", 2), &[0.133, 0.1]);
        assert_eq!(get_best_slot(&inv2, 0, false, false, &ts), 2);
    }

    #[test]
    fn nothing_better_leaves_slot_zero() {
        let inv = TestInventory::empty();
        let ts = ToolSettings::default();
        // every slot is empty and equally slow: slot 0 stays, since a tie needs a strictly lower cost
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 0);
    }

    #[test]
    fn swords_can_be_left_out() {
        let mut inv = TestInventory::empty();
        inv.put(2, SlotInfo { sword: true, ..tool("diamond_sword", 4) }, &[0.5]);
        inv.put(5, tool("stone_pickaxe", 1), &[0.2]);
        let mut ts = ToolSettings::default();
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 2);
        ts.use_sword_to_mine = false;
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 5);
    }

    #[test]
    fn item_saver_skips_nearly_broken_tools() {
        let mut inv = TestInventory::empty();
        inv.put(0, SlotInfo { damage: 240, max_damage: 250, ..tool("iron_pickaxe", 2) }, &[0.3]);
        inv.put(1, tool("stone_pickaxe", 1), &[0.2]);
        let mut ts = ToolSettings::default();
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 0);
        ts.item_saver = true; // 240 + 10 >= 250
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 1);
        // a tool that cannot break (max damage 0 or 1) is never skipped
        inv.put(0, SlotInfo { damage: 5, max_damage: 1, ..tool("iron_pickaxe", 2) }, &[0.3]);
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 0);
    }

    #[test]
    fn silk_touch_wins_a_tie_only_when_preferred_or_cheaper() {
        let mut inv = TestInventory::empty();
        inv.put(0, tool("iron_pickaxe", 2), &[0.3]);
        inv.put(1, SlotInfo { silk_touch: true, ..tool("iron_pickaxe", 2) }, &[0.3]);
        let mut ts = ToolSettings::default();
        // the same cost: the first stays unless silk touch is preferred
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 0);
        assert_eq!(get_best_slot(&inv, 0, true, false, &ts), 1);
        // a cheaper silk touch tool wins without the preference
        inv.put(1, SlotInfo { silk_touch: true, ..tool("stone_pickaxe", 1) }, &[0.3]);
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 1);
        // a cheaper tool without silk touch does not beat a silk touch tool that is already best
        inv.put(0, SlotInfo { silk_touch: true, ..tool("iron_pickaxe", 2) }, &[0.3]);
        inv.put(1, tool("wooden_pickaxe", 0), &[0.3]);
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 0);
        ts.prefer_silk_touch = true;
        assert_eq!(switch_to_best_tool_for(&inv, 0, &ts), Some(0));
    }

    #[test]
    fn without_auto_tool_the_costs_use_the_held_slot() {
        let mut inv = hotbar();
        inv.selected = 6;
        let ts = ToolSettings { auto_tool: false, ..ToolSettings::default() };
        assert_eq!(get_best_slot(&inv, 0, false, true, &ts), 6);
        // the executor's own switch does not happen either
        assert_eq!(switch_to_best_tool_for(&inv, 0, &ts), None);
        // but asking for the best slot as the miner does still finds the pickaxe
        assert_eq!(get_best_slot(&inv, 0, false, false, &ts), 1);
        let ts = ToolSettings { assume_external_auto_tool: true, ..ToolSettings::default() };
        assert_eq!(switch_to_best_tool_for(&inv, 0, &ts), None);
    }

    #[test]
    fn strength_scales_by_potion_and_avoidance() {
        let mut inv = TestInventory::empty();
        inv.put(0, tool("iron_pickaxe", 2), &[0.2, 0.4]);
        inv.amplifier = 1.4;
        let ts = ToolSettings::default();
        assert_eq!(get_str_vs_block(&inv, 0, "stone", &ts), 1.4 * 0.2);
        // a chest is to be avoided: a tenth
        assert_eq!(get_str_vs_block(&inv, 1, "chest", &ts), 1.4 * (0.4 * 0.1));
        let ts = ToolSettings { consider_potion_effects: false, ..ToolSettings::default() };
        assert_eq!(get_str_vs_block(&inv, 0, "stone", &ts), 0.2);
    }

    #[test]
    fn potion_multipliers() {
        assert_eq!(potion_amplifier(None, None), 1.0);
        assert!((potion_amplifier(Some(0), None) - 1.2).abs() < 1e-12);
        assert!((potion_amplifier(Some(1), None) - 1.4).abs() < 1e-12);
        assert_eq!(potion_amplifier(None, Some(0)), 0.3);
        assert_eq!(potion_amplifier(None, Some(2)), 0.0027);
        assert_eq!(potion_amplifier(None, Some(7)), 0.00081);
    }

    #[test]
    fn throwaway_goes_by_the_list_then_the_hotbar() {
        let mut inv = TestInventory::empty();
        inv.put(1, SlotInfo::of("stone", 64), &[]);
        inv.put(4, SlotInfo::of("dirt", 3), &[]);
        let ts = ToolSettings::default();
        // dirt is first in the list, so it wins over stone, which is earlier in the hotbar
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items), Some(4));
        assert!(has_generic_throwaway(&inv, &ts.acceptable_throwaway_items));
        // the list order is the user's
        assert_eq!(select_throwaway(&inv, &["stone".to_string(), "dirt".to_string()]), Some(1));
        // nothing acceptable
        let mut inv = TestInventory::empty();
        inv.put(0, SlotInfo::of("oak_planks", 64), &[]);
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items), None);
        assert!(!has_generic_throwaway(&inv, &ts.acceptable_throwaway_items));
        // an empty stack of an acceptable item is not a block
        let mut inv = TestInventory::empty();
        inv.put(0, SlotInfo::of("dirt", 0), &[]);
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items), None);
    }

    #[test]
    fn a_throwaway_block_in_the_off_hand_needs_a_hand_that_does_not_click() {
        let ts = ToolSettings::default();
        let mut inv = TestInventory::empty();
        inv.offhand = SlotInfo::of("dirt", 20);
        // every slot is empty: the first empty slot is selected
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items), Some(0));
        // slots 0 and 1 hold a shovel and a sword; slot 2 holds a pickaxe
        inv.put(0, tool("iron_shovel", 2), &[]);
        inv.put(1, tool("iron_sword", 2), &[]);
        inv.put(2, SlotInfo { pickaxe: true, ..tool("iron_pickaxe", 2) }, &[]);
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items), Some(2));
        // a full hotbar of things that click: no
        for i in 0..HOTBAR {
            inv.put(i, tool("iron_shovel", 2), &[]);
        }
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items), None);
    }

    #[test]
    fn the_water_bucket_is_found_in_the_hotbar_only() {
        let mut inv = TestInventory::empty();
        inv.put(6, SlotInfo::of("water_bucket", 1), &[]);
        inv.put(2, SlotInfo::of("bucket", 1), &[]);
        assert_eq!(find_hotbar_slot(&inv, "water_bucket"), Some(6));
        assert_eq!(find_hotbar_slot(&inv, "bucket"), Some(2));
        assert_eq!(find_hotbar_slot(&inv, "lava_bucket"), None);
    }
}
