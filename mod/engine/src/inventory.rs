// baritone-rs engine: the hotbar as the executor reads it, and the choice of tool and of block to place.
// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/ToolSet.java (the choice of the best slot, the
// speed formula and the strength used by the costs) and, in new structure, the parts of
// src/main/java/baritone/behavior/InventoryBehavior.java that a path needs (`throwaway`, `selectThrowawayForLocation`,
// `hasGenericThrowaway`). LGPL-3.0-or-later, see lib.rs.
//
// Upstream reads `player.getInventory()` everywhere. Here the glue answers the questions through [`Inventory`] and the engine
// never touches the game: it decides which slot to select, and the glue sets `game.selected`.
//
// `allowInventory` (D24 left it out, D53 brought it in): [`InventoryMover`] is `InventoryBehavior`'s state and its tick, and it only decides. A
// move comes out as a [`SwapRequest`] that the glue carries out as a window click (`ClickType.SWAP`). The builder's `placeAt` branch of
// `selectThrowawayForLocation` is here (D45): the builder gives a [`ThrowawayHint`].
use crate::process::build::InvSlot;

/// The hotbar has nine slots.
pub const HOTBAR: usize = 9;

/// The main inventory, hotbar included: `Inventory.items` has 36 stacks.
pub const MAIN: usize = 36;

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
    /// `ToolSet.calculateSpeedVsBlock` for a stack of the main inventory, slot 0 to 35 (`InventoryBehavior.bestToolAgainst` reads all of them). The
    /// default answers the hotbar and 0 for the rest: a glue that can price the main inventory overrides it.
    fn main_speed_vs_state(&self, slot: usize, state: usize) -> f64 {
        if slot < HOTBAR {
            self.speed_vs_state(slot, state)
        } else {
            0.0
        }
    }
    /// Upstream's `ToolSet.potionAmplifier`: 1 without effects.
    fn potion_amplifier(&self) -> f64 {
        1.0
    }
    /// Upstream's `Inventory.items`: the 36 stacks of the main inventory, hotbar included, which `MineProcess` counts to know whether it has enough. The default
    /// is the hotbar alone: a glue that can read the whole inventory overrides it.
    fn main_slots(&self) -> Vec<SlotInfo> {
        (0..HOTBAR).map(|i| self.slot(i)).collect()
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
    /// `allowInventory`, default false: `InventoryBehavior` may move stacks of the main inventory to the hotbar.
    pub allow_inventory: bool,
    /// `ticksBetweenInventoryMoves`, default 1.
    pub ticks_between_inventory_moves: i32,
    /// `inventoryMoveOnlyIfStationary`, default false.
    pub inventory_move_only_if_stationary: bool,
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
            allow_inventory: false,
            ticks_between_inventory_moves: 1,
            inventory_move_only_if_stationary: false,
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

/// Where `InventoryBehavior.throwaway` found the stack it wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Throwaway {
    /// In a hotbar slot, or in the off hand with a slot that does not right click chosen for the main hand: select this slot.
    Hotbar(usize),
    /// In slot 9 to 35 of the main inventory (only with `allowInventory`): to hold it, upstream's `select` swaps it with hotbar slot 7 and selects 7.
    Inventory(usize),
}

/// The hotbar slot `InventoryBehavior.throwaway` swaps a stack from the main inventory into (`requestSwapWithHotBar(i, 7)`). Its own comments and
/// `onTick` use 8 for the throwaway block (U43).
pub const THROWAWAY_HOTBAR: usize = 7;

impl Throwaway {
    /// The hotbar slot that holds the stack once the move is done: the one to select.
    pub fn hand_slot(self) -> usize {
        match self {
            Throwaway::Hotbar(s) => s,
            Throwaway::Inventory(_) => THROWAWAY_HOTBAR,
        }
    }
}

/// Upstream's `InventoryBehavior.throwaway(boolean,Predicate,boolean)`: where a stack that `desired` accepts is, or `None` when there is none. The first
/// hotbar slot that matches wins. A match in the off hand needs the main hand to hold something that does not right click: the first empty slot or
/// pickaxe. With `allow_inventory`, slots 9 to 35 are looked at last. `InventoryBehavior.throwaway(boolean,Predicate)` is this with the setting.
pub fn throwaway(inv: &dyn Inventory, desired: &dyn Fn(&SlotInfo) -> bool, allow_inventory: bool) -> Option<Throwaway> {
    throwaway_slot(inv, &|_, s| desired(s), allow_inventory)
}

/// `InventoryBehavior.throwaway(boolean,Predicate,boolean)` for a test that also needs to know which slot it looks at (the builder's hint compares the
/// state a slot would place, which the stack alone does not tell).
pub fn throwaway_slot(inv: &dyn Inventory, desired: &dyn Fn(InvSlot, &SlotInfo) -> bool, allow_inventory: bool) -> Option<Throwaway> {
    for i in 0..HOTBAR {
        if desired(InvSlot::Hotbar(i), &inv.slot(i)) {
            return Some(Throwaway::Hotbar(i));
        }
    }
    if desired(InvSlot::Offhand, &inv.offhand()) {
        // main hand takes precedence over off hand
        // that means that if we have block A selected in main hand and block B in off hand, right clicking places block B
        // we've already checked above ^ and the main hand can't possible have an acceptablethrowawayitem
        // so we need to select in the main hand something that doesn't right click
        // so not a shovel, not a hoe, not a block, etc
        for i in 0..HOTBAR {
            let item = inv.slot(i);
            if item.is_empty() || item.pickaxe {
                return Some(Throwaway::Hotbar(i));
            }
        }
    }
    if allow_inventory {
        let main = inv.main_slots();
        for i in HOTBAR..MAIN {
            if main.get(i).is_some_and(|s| desired(InvSlot::Main(i), s)) {
                return Some(Throwaway::Inventory(i));
            }
        }
    }
    None
}

/// Upstream's `InventoryBehavior.selectThrowawayForLocation(boolean,int,int,int)`, with no builder (`placeAt` is `null`): where to get a block that is placed to
/// walk on. The first acceptable item of the list that is found wins, not the first slot.
pub fn select_throwaway(inv: &dyn Inventory, acceptable: &[String], allow_inventory: bool) -> Option<Throwaway> {
    for item in acceptable {
        if let Some(found) = throwaway(inv, &|s: &SlotInfo| !s.is_empty() && s.item == *item, allow_inventory) {
            return Some(found);
        }
    }
    None
}

/// What the builder tells `selectThrowawayForLocation` (`BuilderProcess.placeAt` and the states the stacks would place), which the engine
/// cannot read itself: the state of a stack depends on the game's `getStateForPlacement`. Implemented by the builder; `None` is "no builder".
pub trait ThrowawayHint {
    /// `BuilderProcess.placeAt`: the state the schematic wants at this position, or `None` (not building, outside the schematic, or air).
    /// `current` is the state in the world there.
    fn place_at(&self, x: i32, y: i32, z: i32, current: usize) -> Option<usize>;
    /// Like `maybe.equals(((BlockItem) stack.getItem()).getBlock().getStateForPlacement(...))`: the stack in the slot would place exactly `maybe`.
    fn slot_places(&self, slot: InvSlot, maybe: usize) -> bool;
    /// `((BlockItem) stack.getItem()).getBlock().equals(maybe.getBlock())`: the stack in the slot is an item of the block of `maybe`.
    fn slot_is_block_of(&self, slot: InvSlot, maybe: usize) -> bool;
}

/// `InventoryBehavior.selectThrowawayForLocation`: when the builder wants a block at this position, the slot
/// that places exactly that state, else the slot with an item of that block, else (and always outside the builder) [`select_throwaway`].
/// `current` is the state in the world at the position. `allow_inventory` is the setting, which `throwaway` reads in each of the three steps.
#[allow(clippy::too_many_arguments)]
pub fn select_throwaway_for_location(inv: &dyn Inventory, acceptable: &[String], hint: Option<&dyn ThrowawayHint>, x: i32, y: i32, z: i32, current: usize, allow_inventory: bool) -> Option<Throwaway> {
    if let Some(h) = hint {
        if let Some(maybe) = h.place_at(x, y, z, current) {
            if let Some(found) = throwaway_slot(inv, &|s, _| h.slot_places(s, maybe), allow_inventory) {
                return Some(found); // gotem
            }
            if let Some(found) = throwaway_slot(inv, &|s, _| h.slot_is_block_of(s, maybe), allow_inventory) {
                return Some(found);
            }
        }
    }
    select_throwaway(inv, acceptable, allow_inventory)
}

/// Upstream's `InventoryBehavior.hasGenericThrowaway`. The glue sets `Config::has_throwaway` to this and `allowPlace`.
pub fn has_generic_throwaway(inv: &dyn Inventory, acceptable: &[String], allow_inventory: bool) -> bool {
    select_throwaway(inv, acceptable, allow_inventory).is_some()
}

/// The first hotbar slot that holds this item, like `Inventory.findSlotMatchingItem` followed by `Inventory.isHotbarSlot` (a match
/// in the main inventory only counts when no hotbar slot has the item before it, which this reproduces by looking at the hotbar first).
pub fn find_hotbar_slot(inv: &dyn Inventory, item: &str) -> Option<usize> {
    (0..HOTBAR).find(|&i| {
        let s = inv.slot(i);
        !s.is_empty() && s.item == item
    })
}

// ---------------------------------------------------------------- InventoryBehavior: moving stacks to the hotbar

/// One `ctx.playerController().windowClick(inventoryMenu.containerId, slot, hotbar, ClickType.SWAP, player)`: the stack in `in_inventory` (0 to 35,
/// `Inventory.items` numbering, so 0 to 8 is the hotbar) swaps with hotbar slot `in_hotbar`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapRequest {
    pub in_inventory: usize,
    pub in_hotbar: usize,
}

impl SwapRequest {
    /// The slot number of the click in the player's inventory menu (window 0): `inInventory < 9 ? inInventory + 36 : inInventory`.
    pub fn window_slot(self) -> usize {
        if self.in_inventory < HOTBAR {
            self.in_inventory + 36
        } else {
            self.in_inventory
        }
    }
}

/// `InventoryBehavior`'s state, with the state of `InventoryPauserProcess` that it asks for (`stationaryForInventoryMove`). It decides and never
/// clicks: the moves it makes are [`SwapRequest`]s that the glue carries out, in order, after the tick (`take_requests`). The seed of
/// `getTempHotbarSlot`'s `new Random()` is set once by [`InventoryMover::seed`] (D53).
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryMover {
    /// `InventoryBehavior.ticksSinceLastInventoryMove`.
    ticks_since_last_inventory_move: i32,
    /// Upstream's `lastTickRequestedMove` field: not everything asks every tick, so the request is remembered while coming to a halt.
    last_tick_requested_move: Option<(usize, usize)>,
    /// `InventoryPauserProcess.pauseRequestedLastTick`.
    pause_requested_last_tick: bool,
    /// `InventoryPauserProcess.safeToCancelLastTick`.
    safe_to_cancel_last_tick: bool,
    /// `InventoryPauserProcess.ticksOfStationary`.
    ticks_of_stationary: i32,
    /// The clicks asked for since the glue last took them.
    pending: Vec<SwapRequest>,
    rng: u64,
    seeded: bool,
}

impl Default for InventoryMover {
    fn default() -> Self {
        InventoryMover { ticks_since_last_inventory_move: 0, last_tick_requested_move: None, pause_requested_last_tick: false, safe_to_cancel_last_tick: false, ticks_of_stationary: 0, pending: vec![], rng: 0x9E37_79B9_7F4A_7C15, seeded: false }
    }
}

impl InventoryMover {
    /// Sets the seed of the random choice of a temporary hotbar slot, once (the later calls do nothing). Upstream's `new Random()` is seeded by the clock.
    pub fn seed(&mut self, seed: u64) {
        if !self.seeded {
            self.seeded = true;
            self.rng = seed ^ 0x9E37_79B9_7F4A_7C15;
            if self.rng == 0 {
                self.rng = 0x9E37_79B9_7F4A_7C15;
            }
        }
    }

    /// `Random.nextInt(bound)` for a bound above 0: xorshift64*, not Java's generator (there is no seed to match).
    fn next_below(&mut self, bound: usize) -> usize {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % bound
    }

    /// The clicks to carry out now, oldest first.
    pub fn take_requests(&mut self) -> Vec<SwapRequest> {
        std::mem::take(&mut self.pending)
    }

    /// `InventoryBehavior.onTick`: `ticksSinceLastInventoryMove` counts, a path that runs wants a throwaway block in hotbar slot 8 and the best pickaxe in
    /// slot 0, and a move that was asked for and delayed is asked for again. Called each tick after the pathing behavior's tick (`registerBehavior` order).
    /// `container_open` is `containerMenu != inventoryMenu`, `is_pathing` is `PathingBehavior.isPathing`, `stone` is the state of `Blocks.STONE`
    /// (`None`: the glue cannot say, so no pickaxe is picked). The `TickEvent.Type.OUT` check is the glue's: it does not tick outside a world.
    pub fn on_tick(&mut self, inv: &dyn Inventory, ts: &ToolSettings, container_open: bool, is_pathing: bool, stone: Option<usize>) {
        if !ts.allow_inventory {
            return;
        }
        if container_open {
            // we have a crafting table or a chest or something open
            return;
        }
        self.ticks_since_last_inventory_move = self.ticks_since_last_inventory_move.wrapping_add(1);
        if is_pathing {
            let main = inv.main_slots();
            let throwaway = first_valid_throwaway(&main, &ts.acceptable_throwaway_items);
            if let Some(i) = throwaway.filter(|&i| i >= HOTBAR) {
                // aka there are none on the hotbar, but there are some in main inventory
                self.request_swap_with_hotbar(i, 8, ts);
            }
            if let Some(state) = stone {
                if let Some(pick) = best_tool_against(inv, &main, state, ts).filter(|&p| p >= HOTBAR) {
                    self.request_swap_with_hotbar(pick, 0, ts);
                }
            }
        }
        if let Some((in_inventory, in_hotbar)) = self.last_tick_requested_move {
            // "Remembering to move ... from a previous tick" (logDebug, dropped)
            self.request_swap_with_hotbar(in_inventory, in_hotbar, ts);
        }
    }

    /// `InventoryBehavior.attemptToPutOnHotbar`: asks for the stack in `in_main_invy` to go to a temporary hotbar slot that `disallowed_hotbar` does not
    /// forbid. False when the move was asked for and delayed (the caller waits); true when it was asked for and done, and also when there is no slot.
    pub fn attempt_to_put_on_hotbar(&mut self, inv: &dyn Inventory, in_main_invy: usize, ts: &ToolSettings, disallowed_hotbar: &dyn Fn(usize) -> bool) -> bool {
        if let Some(destination) = self.get_temp_hotbar_slot(inv, disallowed_hotbar) {
            if !self.request_swap_with_hotbar(in_main_invy, destination, ts) {
                return false;
            }
        }
        true
    }

    /// `InventoryBehavior.getTempHotbarSlot`: a random empty slot among 1 to 7 that is not forbidden, else a random one among 1 to 7 that is not forbidden
    /// (0 and 8 are for the pickaxe and the throwaway block), else `None`.
    pub fn get_temp_hotbar_slot(&mut self, inv: &dyn Inventory, disallowed_hotbar: &dyn Fn(usize) -> bool) -> Option<usize> {
        // we're using 0 and 8 for pickaxe and throwaway
        let mut candidates: Vec<usize> = (1..8).filter(|&i| inv.slot(i).is_empty() && !disallowed_hotbar(i)).collect();
        if candidates.is_empty() {
            candidates = (1..8).filter(|&i| !disallowed_hotbar(i)).collect();
        }
        if candidates.is_empty() {
            return None;
        }
        let pick = self.next_below(candidates.len());
        Some(candidates[pick])
    }

    /// `InventoryBehavior.requestSwapWithHotBar`: remembers the request, then makes it unless `ticksBetweenInventoryMoves` have not passed since the last
    /// move, or `inventoryMoveOnlyIfStationary` is on and the player is not stationary yet (which also asks the pauser to pause the path). True when the move
    /// is made.
    pub fn request_swap_with_hotbar(&mut self, in_inventory: usize, in_hotbar: usize, ts: &ToolSettings) -> bool {
        self.last_tick_requested_move = Some((in_inventory, in_hotbar));
        if self.ticks_since_last_inventory_move < ts.ticks_between_inventory_moves {
            // "Inventory move requested but delaying" (logDebug, dropped)
            return false;
        }
        if ts.inventory_move_only_if_stationary && !self.stationary_for_inventory_move() {
            // "Inventory move requested but delaying until stationary" (logDebug, dropped)
            return false;
        }
        self.pending.push(SwapRequest { in_inventory, in_hotbar });
        self.ticks_since_last_inventory_move = 0;
        self.last_tick_requested_move = None;
        true
    }

    /// `InventoryPauserProcess.stationaryForInventoryMove`: asks the pauser to pause the path next tick, and says whether the player has stood still for
    /// more than one tick in a place where the path may be paused.
    pub fn stationary_for_inventory_move(&mut self) -> bool {
        self.pause_requested_last_tick = true;
        self.safe_to_cancel_last_tick && self.ticks_of_stationary > 1
    }

    /// `InventoryPauserProcess.onTick`: true is `REQUEST_PAUSE`, false is `DEFER`. `stationary_now` is `InventoryPauserProcess.stationaryNow`: the
    /// horizontal speed is below 0.00001.
    pub fn pauser_on_tick(&mut self, is_safe_to_cancel: bool, stationary_now: bool) -> bool {
        self.safe_to_cancel_last_tick = is_safe_to_cancel;
        if self.pause_requested_last_tick {
            self.pause_requested_last_tick = false;
            if stationary_now {
                self.ticks_of_stationary = self.ticks_of_stationary.wrapping_add(1);
            }
            return true;
        }
        self.ticks_of_stationary = 0;
        false
    }
}

/// `InventoryBehavior.firstValidThrowaway`: the first slot of the 36 that holds an acceptable throwaway item. (Its TODO about the off hand stays.)
fn first_valid_throwaway(main: &[SlotInfo], acceptable: &[String]) -> Option<usize> {
    main.iter().take(MAIN).position(|s| !s.is_empty() && acceptable.contains(&s.item))
}

/// `InventoryBehavior.bestToolAgainst`: as `onTick` calls it, with `Blocks.STONE` and `PickaxeItem.class`: the slot of the 36 whose pickaxe mines the block fastest (the first of equals;
/// stacks `itemSaver` protects are skipped), or `None`. `state` is the default state of the block.
fn best_tool_against(inv: &dyn Inventory, main: &[SlotInfo], state: usize, ts: &ToolSettings) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_speed = -1.0f64;
    for (i, stack) in main.iter().enumerate().take(MAIN) {
        if stack.is_empty() {
            continue;
        }
        if ts.item_saver && stack.damage.wrapping_add(ts.item_saver_threshold) >= stack.max_damage && stack.max_damage > 1 {
            continue;
        }
        if stack.pickaxe {
            let speed = inv.main_speed_vs_state(i, state); // takes into account enchants
            if speed > best_speed {
                best_speed = speed;
                best = Some(i);
            }
        }
    }
    best
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
        /// Stacks of the main inventory outside the hotbar (slot 9 on), for the mine process's count and for `allowInventory`.
        pub main: Vec<SlotInfo>,
        /// For `main[i]` (slot 9 + i): the speed against state 0, 1, 2, ... (missing: 0.01).
        pub main_speeds: Vec<Vec<f64>>,
    }

    impl TestInventory {
        pub fn empty() -> TestInventory {
            TestInventory { selected: 0, slots: vec![SlotInfo::empty(); HOTBAR], offhand: SlotInfo::empty(), speeds: vec![vec![]; HOTBAR], amplifier: 1.0, main: vec![], main_speeds: vec![] }
        }

        /// Puts a stack in slot 9 or later of the main inventory (the slots in between are empty), with its speed against each state.
        pub fn put_main(&mut self, slot: usize, info: SlotInfo, speeds: &[f64]) {
            let i = slot - HOTBAR;
            if self.main.len() <= i {
                self.main.resize(i + 1, SlotInfo::empty());
                self.main_speeds.resize(i + 1, vec![]);
            }
            self.main[i] = info;
            self.main_speeds[i] = speeds.to_vec();
        }

        /// A window click of type SWAP (`ClickType.SWAP`): the stack in `in_inventory` and the one in hotbar slot `in_hotbar` change places, with their speeds.
        pub fn apply_swap(&mut self, swap: SwapRequest) {
            let hb = swap.in_hotbar;
            if swap.in_inventory < HOTBAR {
                self.slots.swap(swap.in_inventory, hb);
                self.speeds.swap(swap.in_inventory, hb);
                return;
            }
            let i = swap.in_inventory - HOTBAR;
            if self.main.len() <= i {
                self.main.resize(i + 1, SlotInfo::empty());
                self.main_speeds.resize(i + 1, vec![]);
            }
            std::mem::swap(&mut self.slots[hb], &mut self.main[i]);
            std::mem::swap(&mut self.speeds[hb], &mut self.main_speeds[i]);
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
        fn main_speed_vs_state(&self, slot: usize, state: usize) -> f64 {
            if slot < HOTBAR {
                self.speed_vs_state(slot, state)
            } else {
                self.main_speeds.get(slot - HOTBAR).and_then(|v| v.get(state)).copied().unwrap_or(0.01)
            }
        }
        fn potion_amplifier(&self) -> f64 {
            self.amplifier
        }
        fn main_slots(&self) -> Vec<SlotInfo> {
            let mut all: Vec<SlotInfo> = self.slots.iter().cloned().chain(self.main.iter().cloned()).collect();
            if !self.main.is_empty() {
                all.resize(MAIN, SlotInfo::empty());
            }
            all
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

    fn hand(t: Option<Throwaway>) -> Option<usize> {
        t.map(Throwaway::hand_slot)
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
        assert_eq!(hand(select_throwaway(&inv, &ts.acceptable_throwaway_items, false)), Some(4));
        assert!(has_generic_throwaway(&inv, &ts.acceptable_throwaway_items, false));
        // the list order is the user's
        assert_eq!(hand(select_throwaway(&inv, &["stone".to_string(), "dirt".to_string()], false)), Some(1));
        // nothing acceptable
        let mut inv = TestInventory::empty();
        inv.put(0, SlotInfo::of("oak_planks", 64), &[]);
        assert_eq!(hand(select_throwaway(&inv, &ts.acceptable_throwaway_items, false)), None);
        assert!(!has_generic_throwaway(&inv, &ts.acceptable_throwaway_items, false));
        // an empty stack of an acceptable item is not a block
        let mut inv = TestInventory::empty();
        inv.put(0, SlotInfo::of("dirt", 0), &[]);
        assert_eq!(hand(select_throwaway(&inv, &ts.acceptable_throwaway_items, false)), None);
    }

    #[test]
    fn a_throwaway_block_in_the_off_hand_needs_a_hand_that_does_not_click() {
        let ts = ToolSettings::default();
        let mut inv = TestInventory::empty();
        inv.offhand = SlotInfo::of("dirt", 20);
        // every slot is empty: the first empty slot is selected
        assert_eq!(hand(select_throwaway(&inv, &ts.acceptable_throwaway_items, false)), Some(0));
        // slots 0 and 1 hold a shovel and a sword; slot 2 holds a pickaxe
        inv.put(0, tool("iron_shovel", 2), &[]);
        inv.put(1, tool("iron_sword", 2), &[]);
        inv.put(2, SlotInfo { pickaxe: true, ..tool("iron_pickaxe", 2) }, &[]);
        assert_eq!(hand(select_throwaway(&inv, &ts.acceptable_throwaway_items, false)), Some(2));
        // a full hotbar of things that click: no
        for i in 0..HOTBAR {
            inv.put(i, tool("iron_shovel", 2), &[]);
        }
        assert_eq!(hand(select_throwaway(&inv, &ts.acceptable_throwaway_items, false)), None);
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

    // ------------------------------------------------------------ allowInventory (InventoryBehavior)

    fn allow() -> ToolSettings {
        ToolSettings { allow_inventory: true, ..ToolSettings::default() }
    }

    fn pick(item: &str) -> SlotInfo {
        SlotInfo { pickaxe: true, ..tool(item, 2) }
    }

    /// `InventoryBehavior.throwaway(boolean,Predicate,boolean)`: slots 9 to 35 are looked at only with the setting, and after the hotbar.
    #[test]
    fn the_main_inventory_is_searched_only_with_allow_inventory() {
        let ts = ToolSettings::default();
        let mut inv = TestInventory::empty();
        inv.put_main(20, SlotInfo::of("cobblestone", 64), &[]);
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items, false), None);
        assert!(!has_generic_throwaway(&inv, &ts.acceptable_throwaway_items, false));
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items, true), Some(Throwaway::Inventory(20)));
        assert!(has_generic_throwaway(&inv, &ts.acceptable_throwaway_items, true));
        // the list order still decides first (`selectThrowawayForLocation` asks for each item in turn): cobblestone in the main inventory beats stone on the hotbar
        inv.put(5, SlotInfo::of("stone", 1), &[]);
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items, true), Some(Throwaway::Inventory(20)));
        // for one item, the hotbar wins over the main inventory
        inv.put(6, SlotInfo::of("cobblestone", 1), &[]);
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items, true), Some(Throwaway::Hotbar(6)));
    }

    /// `InventoryBehavior.throwaway(boolean,Predicate,boolean)`: the first match of 9 to 35 wins, and holding it means hotbar slot 7.
    #[test]
    fn the_first_main_slot_wins_and_is_held_in_slot_7() {
        let mut inv = TestInventory::empty();
        inv.put_main(30, SlotInfo::of("dirt", 2), &[]);
        inv.put_main(12, SlotInfo::of("dirt", 9), &[]);
        let found = throwaway(&inv, &|s: &SlotInfo| s.item == "dirt", true);
        assert_eq!(found, Some(Throwaway::Inventory(12)));
        assert_eq!(found.map(Throwaway::hand_slot), Some(7));
        assert_eq!(THROWAWAY_HOTBAR, 7);
    }

    /// `InventoryBehavior.throwaway(boolean,Predicate,boolean)`: a block in the off hand and a hotbar with nothing that does not click does not stop the search
    /// (upstream falls out of the `if`), so the main inventory is still looked at.
    #[test]
    fn a_blocked_off_hand_falls_through_to_the_main_inventory() {
        let mut inv = TestInventory::empty();
        inv.offhand = SlotInfo::of("dirt", 20);
        for i in 0..HOTBAR {
            inv.put(i, tool("iron_shovel", 2), &[]);
        }
        inv.put_main(15, SlotInfo::of("dirt", 5), &[]);
        let ts = ToolSettings::default();
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items, false), None);
        assert_eq!(select_throwaway(&inv, &ts.acceptable_throwaway_items, true), Some(Throwaway::Inventory(15)));
    }

    /// `InventoryBehavior.selectThrowawayForLocation`: each of its three steps reads `allowInventory`.
    #[test]
    fn the_builders_two_steps_look_in_the_main_inventory_too() {
        struct Hint;
        impl ThrowawayHint for Hint {
            fn place_at(&self, _x: i32, _y: i32, _z: i32, _current: usize) -> Option<usize> {
                Some(7)
            }
            fn slot_places(&self, slot: InvSlot, _maybe: usize) -> bool {
                slot == InvSlot::Main(22)
            }
            fn slot_is_block_of(&self, slot: InvSlot, _maybe: usize) -> bool {
                slot == InvSlot::Main(11)
            }
        }
        let mut inv = TestInventory::empty();
        inv.put_main(22, SlotInfo::of("oak_planks", 3), &[]);
        inv.put_main(11, SlotInfo::of("oak_stairs", 3), &[]);
        let acceptable = ToolSettings::default().acceptable_throwaway_items;
        let h: &dyn ThrowawayHint = &Hint;
        assert_eq!(select_throwaway_for_location(&inv, &acceptable, Some(h), 0, 0, 0, 0, true), Some(Throwaway::Inventory(22)));
        assert_eq!(select_throwaway_for_location(&inv, &acceptable, Some(h), 0, 0, 0, 0, false), None);
    }

    /// `InventoryBehavior.requestSwapWithHotBar`: `windowClick(..., inInventory < 9 ? inInventory + 36 : inInventory, inHotbar, SWAP)`.
    #[test]
    fn a_swap_is_a_click_on_the_inventory_menus_numbering() {
        assert_eq!(SwapRequest { in_inventory: 3, in_hotbar: 8 }.window_slot(), 39);
        assert_eq!(SwapRequest { in_inventory: 9, in_hotbar: 0 }.window_slot(), 9);
        assert_eq!(SwapRequest { in_inventory: 35, in_hotbar: 7 }.window_slot(), 35);
    }

    /// `InventoryBehavior.onTick`: a path that runs and has no throwaway block on the hotbar gets the first one of the 36 into slot 8.
    #[test]
    fn a_path_that_runs_brings_a_throwaway_block_to_slot_8() {
        let mut inv = TestInventory::empty();
        inv.put_main(25, SlotInfo::of("netherrack", 64), &[]);
        inv.put_main(14, SlotInfo::of("dirt", 64), &[]);
        let ts = allow();
        let mut m = InventoryMover::default();
        m.on_tick(&inv, &ts, false, true, None);
        assert_eq!(m.take_requests(), [SwapRequest { in_inventory: 14, in_hotbar: 8 }]);
        // a block already on the hotbar: nothing to move
        inv.put(2, SlotInfo::of("dirt", 1), &[]);
        let mut m = InventoryMover::default();
        m.on_tick(&inv, &ts, false, true, None);
        assert!(m.take_requests().is_empty());
    }

    /// `InventoryBehavior.onTick`: nothing happens with the setting off or a container open, and no path means no throwaway or pickaxe request.
    #[test]
    fn the_tick_does_nothing_with_the_setting_off_a_container_open_or_no_path() {
        let mut inv = TestInventory::empty();
        inv.put_main(14, SlotInfo::of("dirt", 64), &[]);
        let mut m = InventoryMover::default();
        m.on_tick(&inv, &ToolSettings::default(), false, true, None);
        m.on_tick(&inv, &allow(), true, true, None);
        m.on_tick(&inv, &allow(), false, false, None);
        assert!(m.take_requests().is_empty());
        // the container tick did not count: the first real tick is tick 1, and a move needs ticksBetweenInventoryMoves (1) of them
        m.on_tick(&inv, &allow(), false, true, None);
        assert_eq!(m.take_requests().len(), 1);
    }

    /// `InventoryBehavior.onTick`: the best pickaxe of the 36 goes to slot 0 when it is not on the hotbar; `>` keeps the first of equals; a pickaxe on the
    /// hotbar that is the best is not moved; `itemSaver` skips a worn one.
    #[test]
    fn the_best_pickaxe_goes_to_slot_0() {
        let stone = 0usize;
        let mut inv = TestInventory::empty();
        inv.put(4, pick("wooden_pickaxe"), &[0.05]);
        inv.put_main(10, pick("iron_pickaxe"), &[0.133]);
        inv.put_main(11, pick("diamond_pickaxe"), &[0.133]);
        inv.put_main(12, SlotInfo { sword: true, ..tool("diamond_sword", 4) }, &[9.0]);
        let ts = allow();
        let mut m = InventoryMover::default();
        m.on_tick(&inv, &ts, false, true, Some(stone));
        assert_eq!(m.take_requests(), [SwapRequest { in_inventory: 10, in_hotbar: 0 }], "the first of the two that tie, and not the sword");
        // a pickaxe on the hotbar that is the fastest: nothing
        inv.put(6, pick("netherite_pickaxe"), &[0.4]);
        let mut m = InventoryMover::default();
        m.on_tick(&inv, &ts, false, true, Some(stone));
        assert!(m.take_requests().is_empty());
        // itemSaver: the worn netherite pickaxe (damage 2000 of 2031, threshold 10) is skipped, so the main inventory's iron one is the best again
        inv.put(6, SlotInfo { damage: 2025, max_damage: 2031, ..pick("netherite_pickaxe") }, &[0.4]);
        let saver = ToolSettings { item_saver: true, ..allow() };
        let mut m = InventoryMover::default();
        m.on_tick(&inv, &saver, false, true, Some(stone));
        assert_eq!(m.take_requests(), [SwapRequest { in_inventory: 10, in_hotbar: 0 }]);
    }

    /// `InventoryBehavior.requestSwapWithHotBar`: only one move per `ticksBetweenInventoryMoves`; the one that is delayed is remembered, and asked for again
    /// in a later tick even when no path is running; a move that is made forgets it.
    #[test]
    fn a_delayed_move_is_remembered_and_made_on_a_later_tick() {
        let mut inv = TestInventory::empty();
        inv.put_main(14, SlotInfo::of("dirt", 64), &[0.0]);
        inv.put_main(15, pick("iron_pickaxe"), &[0.133]);
        let ts = allow();
        let mut m = InventoryMover::default();
        // tick 1: the throwaway move is made, the pickaxe one comes too soon (0 ticks since < 1)
        m.on_tick(&inv, &ts, false, true, Some(0));
        assert_eq!(m.take_requests(), [SwapRequest { in_inventory: 14, in_hotbar: 8 }]);
        // tick 2, with no path: the remembered pickaxe move is made
        m.on_tick(&inv, &ts, false, false, Some(0));
        assert_eq!(m.take_requests(), [SwapRequest { in_inventory: 15, in_hotbar: 0 }]);
        // and it is not made again
        m.on_tick(&inv, &ts, false, false, Some(0));
        assert!(m.take_requests().is_empty());
    }

    /// `InventoryBehavior.requestSwapWithHotBar`: `ticksBetweenInventoryMoves` of 0 allows several moves in one tick, 3 makes three ticks pass first.
    #[test]
    fn ticks_between_inventory_moves_spaces_the_moves() {
        let mut m = InventoryMover::default();
        let none = ToolSettings { ticks_between_inventory_moves: 0, ..allow() };
        assert!(m.request_swap_with_hotbar(10, 1, &none));
        assert!(m.request_swap_with_hotbar(11, 2, &none));
        assert_eq!(m.take_requests().len(), 2);
        let three = ToolSettings { ticks_between_inventory_moves: 3, ..allow() };
        let inv = TestInventory::empty();
        let mut m = InventoryMover::default();
        for _ in 0..2 {
            m.on_tick(&inv, &three, false, false, None);
            assert!(!m.request_swap_with_hotbar(10, 1, &three));
        }
        // the third tick: the remembered request is made by the tick itself
        m.on_tick(&inv, &three, false, false, None);
        assert_eq!(m.take_requests(), [SwapRequest { in_inventory: 10, in_hotbar: 1 }]);
    }

    /// `InventoryBehavior.getTempHotbarSlot`: slots 1 to 7 only, empty ones first, never a forbidden one.
    #[test]
    fn a_temporary_slot_is_an_empty_one_of_1_to_7_that_is_not_forbidden() {
        let mut inv = TestInventory::empty();
        for i in 0..HOTBAR {
            inv.put(i, SlotInfo::of("dirt", 1), &[]);
        }
        inv.put(3, SlotInfo::empty(), &[]);
        inv.put(5, SlotInfo::empty(), &[]);
        let mut m = InventoryMover::default();
        for _ in 0..50 {
            let s = m.get_temp_hotbar_slot(&inv, &|i| i == 5);
            assert_eq!(s, Some(3));
        }
        // both empty slots are allowed: both get picked, nothing else
        let mut seen = [false; 9];
        for _ in 0..200 {
            seen[m.get_temp_hotbar_slot(&inv, &|_| false).unwrap()] = true;
        }
        assert_eq!(seen, [false, false, false, true, false, true, false, false, false]);
        // every empty one forbidden: any allowed one of 1 to 7
        let mut seen = [false; 9];
        for _ in 0..400 {
            seen[m.get_temp_hotbar_slot(&inv, &|i| i == 3 || i == 5 || i == 1).unwrap()] = true;
        }
        assert_eq!(seen, [false, false, true, false, true, false, true, true, false]);
        // everything forbidden: no slot
        assert_eq!(m.get_temp_hotbar_slot(&inv, &|_| true), None);
    }

    /// `InventoryBehavior.attemptToPutOnHotbar`: true when the move is made, false while it is delayed, and true when there is no slot to put it in.
    #[test]
    fn putting_a_stack_on_the_hotbar_waits_for_the_move() {
        let inv = TestInventory::empty();
        let ts = allow();
        let mut m = InventoryMover::default();
        // too soon (no tick has passed): false, and the request is remembered
        assert!(!m.attempt_to_put_on_hotbar(&inv, 20, &ts, &|_| false));
        assert!(m.take_requests().is_empty());
        m.on_tick(&inv, &ts, false, false, None);
        // the remembered request of the tick above was made by the tick itself, into the same destination
        let made = m.take_requests();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].in_inventory, 20);
        assert!((1..8).contains(&made[0].in_hotbar));
        // no slot at all: true, nothing asked
        let mut m = InventoryMover::default();
        assert!(m.attempt_to_put_on_hotbar(&inv, 20, &ts, &|_| true));
        assert!(m.take_requests().is_empty());
    }

    /// `InventoryBehavior.requestSwapWithHotBar` with `inventoryMoveOnlyIfStationary`, and `InventoryPauserProcess`: the move waits for a path that may be
    /// cancelled and for the player to stand still for more than one tick, and the pauser asks for a pause on the ticks after each request.
    #[test]
    fn a_move_waits_until_the_player_stands_still() {
        let ts = ToolSettings { inventory_move_only_if_stationary: true, ticks_between_inventory_moves: 0, ..allow() };
        let mut m = InventoryMover::default();
        // not asked yet: DEFER, and the count stays 0
        assert!(!m.pauser_on_tick(true, true));
        assert!(!m.request_swap_with_hotbar(10, 1, &ts), "the request itself asks to pause; nothing is known yet");
        // tick 1 of the pause: stationary counts 1
        assert!(m.pauser_on_tick(true, true));
        assert!(!m.request_swap_with_hotbar(10, 1, &ts), "1 is not more than 1");
        // tick 2: counts 2
        assert!(m.pauser_on_tick(true, true));
        assert!(m.request_swap_with_hotbar(10, 1, &ts));
        assert_eq!(m.take_requests(), [SwapRequest { in_inventory: 10, in_hotbar: 1 }]);
        // moving resets nothing while the pause lasts, but a tick with no request resets the count
        assert!(m.pauser_on_tick(true, false));
        assert!(!m.pauser_on_tick(true, true));
        assert!(!m.request_swap_with_hotbar(10, 1, &ts));
        // not safe to cancel: never
        let mut m = InventoryMover::default();
        m.pauser_on_tick(false, true);
        for _ in 0..5 {
            assert!(!m.request_swap_with_hotbar(10, 1, &ts));
            assert!(m.pauser_on_tick(false, true));
        }
    }

    /// `InventoryBehavior.onTick` and `getTempHotbarSlot`: the seed is taken once.
    #[test]
    fn the_seed_is_set_once() {
        let mut a = InventoryMover::default();
        let mut b = InventoryMover::default();
        a.seed(1);
        a.seed(2);
        b.seed(1);
        let inv = TestInventory::empty();
        for _ in 0..20 {
            assert_eq!(a.get_temp_hotbar_slot(&inv, &|_| false), b.get_temp_hotbar_slot(&inv, &|_| false));
        }
    }
}
