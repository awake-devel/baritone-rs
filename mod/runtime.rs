// baritone-rs, the mod: connects the engine to Horizon. Written for this project. LGPL-3.0-or-later, see ../CREDITS.md.
//
// Everything the game gives and takes goes through here:
//   * `on_tick` (native_tick, after the player has moved): reads the player, runs the state machine of the engine, applies what
//     it answers (view angles, sprint, and the keys that `on_movement` hands to the next tick's movement).
//   * `on_movement` (native_movement_input, before the player moves): replaces the player's keys, only while a path runs.
//   * `on_input_head` / `on_input_return` (the game's own input step, pinned): hold the left and right buttons for the executor, also
//     with the chat open (the screen is Playing for that one call). The slot, the aim and the buttons come from the intent.
//   * `on_command`: `/b goto` (coordinates or a block), `mine`, `goal`, `path`, `eta`, `set`, `stop`, `help`, `trace`. `suggest_provider`: the words after `/b` for the chat box.
//   * `on_primitives`: the path, drawn in the world.
//   * `on_level_gone`: the level was dropped, or the connection ends.
// With no goal ever given, the mod touches no key and walks nowhere. It does read the chunks the game loads, in a world: the persistent chunk cache
// (`cache_tick`, docs/deviations.md D41) packs them on a thread of its own and saves them to the mod's data folder, as upstream's does.
//
// The block table comes from `baritone_rs_facts` (mod/glue/state_facts.rs, the file tools/facts-compare also compiles).
// The test-only scenario trigger at the end does nothing unless BARITONE_RS_SCENARIO_DIR is set.
use super::{Game, Screen};
use baritone_engine::blocks::{Direction, State};
use baritone_engine::buttons::Buttons;
use baritone_engine::blockmeta::{BlockIndex, StateEntry};
use baritone_engine::cache::worker::{PackJob, WorldData, WorldProvider};
use baritone_engine::cache::{CacheEnv, CacheReader, CachedWorld, Dimension, PackSource, WithCache};
use baritone_engine::command::{complete_blocks, complete_follow, eta_command, explore_command, explore_filter_command, follow_command, goal_command, goal_to_string, goto_block, goto_goal, mine_command, path_command, GoalAction, GotoError, PathAction, PlayerRef};
use baritone_engine::config::Config;
use baritone_engine::exec::movements::GameFacts;
use baritone_engine::exec::state::{CancelReason, PlayerView};
use baritone_engine::goals::Goal;
use baritone_engine::inventory::{calculate_speed_vs_block, get_str_vs_block, potion_amplifier, Inventory, SlotInfo, ToolSettings, HOTBAR};
use baritone_engine::look::Vec3;
use baritone_engine::movement::helper::Border;
use baritone_engine::pos::Pos;
use baritone_engine::process::pathing::{PathEvent, Pathing, Tables, TickEnv};
use baritone_engine::process::{EntityInfo, ItemEntity as DroppedItem, ProcessInputs, ScanJob, ScanResult};
use baritone_engine::raytrace::BlockHit;
use baritone_engine::set::{Registry, SetEffect, Settings};
use baritone_engine::freelook::{restored_view, LookSettings, LookState, Plan};
use baritone_engine::look::Rotation;
use baritone_engine::settings::ExecSettings;
use baritone_engine::worker::{monotonic_ms_clock, Worker};
use baritone_engine::world::{falling_box_hits, known_state, WorldView};
use std::cell::OnceCell;
use horizon_client::motion::PlayerMotion;
use horizon_items::blocks::BlockFacts;
use horizon_items::items::{ItemStack, Items};
use horizon_client_render::chunk::Chunk;
use horizon_client_render::world::World;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

pub const ID: &str = "baritone_rs";
pub const NAME: &str = "baritone-rs";
pub const VERSION: &str = "0.1.0";
pub const DESCRIPTION: &str = "Pathfinding. /b goto <x> <y> <z> or <block>, /b mine <block>, /b follow <entities>, /b explore [x z], /b explorefilter <file> [invert], /b goal, /b path, /b eta, /b set, /b stop. Walks, sprints, breaks and places blocks. Parkour and swimming are off.";

const PREFIX: &str = "[baritone-rs]";

// ---------------------------------------------------------------- the world as the engine reads it

/// A copy of the loaded chunks, taken on the game thread and read by the worker. The chunks are `Arc`s: the copy is a handful of
/// pointer clones, and a block change in the game copies the chunk it touches (copy on write), never this snapshot.
pub struct Snapshot {
    chunks: HashMap<(i32, i32), Arc<Chunk>>,
    min_y: i32,
    height: i32,
    air: usize,
    /// The length of the block table: an id at or past it reads as air.
    limit: usize,
}

impl Snapshot {
    fn take(world: &World, limit: usize) -> Snapshot {
        let chunks = world.chunks.iter().map(|(k, c)| (*k, Arc::clone(c))).collect();
        Snapshot { chunks, min_y: world.bottom_y(), height: world.top_y() - world.bottom_y(), air: world.air.0 as usize, limit }
    }
}

impl WorldView for Snapshot {
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
        match self.chunks.get(&(x >> 4, z >> 4)) {
            Some(chunk) => known_state(chunk.get(x & 15, y, z & 15) as usize, self.limit, self.air),
            None => self.air,
        }
    }
    fn contains_loaded_chunk(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&(x >> 4, z >> 4))
    }
    fn chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.chunks.contains_key(&(chunk_x, chunk_z))
    }
    fn section_has_only_air(&self, chunk_x: i32, chunk_z: i32, section: i32) -> bool {
        let air = self.air as u32;
        usize::try_from(section).ok().and_then(|s| self.chunks.get(&(chunk_x, chunk_z))?.sections.get(s)).is_some_and(|sec| sec.is_uniform(air))
    }
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.height
    }
}

/// What the search and the scans read: the snapshot of the loaded chunks with the persistent cache under it (`BlockStateInterface`).
type CSnapshot = WithCache<Snapshot>;

/// The live world, for the executor's rechecks on the game thread. It shares no chunk copy with the worker.
struct Live<'a> {
    world: &'a World,
    registry: &'a BlockRegistry,
    /// Feet positions of the falling blocks the game holds (`FallingBlockEntity`), for `Movement.prepared`.
    falling: Vec<[f64; 3]>,
}

/// The falling blocks of the game as plain positions.
fn falling_positions(game: &Game<'_>) -> Vec<[f64; 3]> {
    game.falling.iter().map(|f| [f.position.x, f.position.y, f.position.z]).collect()
}

impl WorldView for Live<'_> {
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
        known_state(self.world.block(x, y, z).0 as usize, self.registry.len(), self.world.air.0 as usize)
    }
    fn contains_loaded_chunk(&self, x: i32, z: i32) -> bool {
        self.world.loaded(x, z)
    }
    fn falling_block_near(&self, x: i32, y: i32, z: i32) -> bool {
        self.falling.iter().any(|f| falling_box_hits(*f, x, y, z))
    }
    fn is_open(&self, x: i32, y: i32, z: i32) -> bool {
        let id = self.world.block(x, y, z);
        self.registry.state(id).is_some_and(|s| s.properties.get("open").map(String::as_str) == Some("true"))
    }
    fn min_y(&self) -> i32 {
        self.world.bottom_y()
    }
    fn height(&self) -> i32 {
        self.world.top_y() - self.world.bottom_y()
    }
}

// ---------------------------------------------------------------- state

/// What the block table was made from. A change in any of these means a new table (`Sig`): the settings, the tools in the hotbar (the
/// strength of each block), whether a block to place or a water bucket is at hand, whether food allows sprinting, the potion.
#[derive(Clone, PartialEq)]
struct Sig {
    settings_rev: u64,
    has_throwaway: bool,
    has_water_bucket: bool,
    can_sprint: bool,
    amplifier_bits: u64,
    /// Per hotbar slot: the item, its enchantments, and its damage when `itemSaver` reads it; `None` for an empty slot.
    hotbar: Vec<Option<(String, Vec<(String, u32)>, u32)>>,
}

/// The per-state facts that do not change while the game runs: the states as the engine reads them, the default state of each block
/// (`ToolSet` reads that one) and the name of each block without its namespace.
struct Base {
    states: Vec<State>,
    default_of: Vec<usize>,
    paths: Vec<String>,
    /// Per state: the block is a `TallGrassBlock`, `DoublePlantBlock` or `FlowerBlock` (`ChunkPacker.getPathingBlockType` calls those air).
    air_like: Vec<bool>,
    /// The blocks, their states and what they drop, for `mine` and `goto <block>`.
    index: BlockIndex,
}

/// Scans run off the game thread (the mine and get-to-block processes' `rescan`): a request goes in, the answer comes back and is handed to the
/// process on a later tick. The thread lives as long as the mod state.
struct Scanner {
    tx: std::sync::mpsc::Sender<ScanRequest>,
    rx: std::sync::mpsc::Receiver<ScanResult>,
}

struct ScanRequest {
    job: ScanJob,
    snapshot: CSnapshot,
    tables: Arc<Tables>,
    border: Border,
}

impl Scanner {
    fn spawn() -> std::io::Result<Scanner> {
        let (tx, jobs) = std::sync::mpsc::channel::<ScanRequest>();
        let (results, rx) = std::sync::mpsc::channel::<ScanResult>();
        std::thread::Builder::new().name("baritone-rs-scan".into()).spawn(move || {
            for req in jobs {
                let (kind, generation) = (req.job.kind, req.job.generation);
                let began = std::time::Instant::now();
                let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| req.job.run(&req.tables.ctx(&req.snapshot, req.border))));
                let result = match answer {
                    Ok(r) => r,
                    Err(_) => {
                        log::error!("{PREFIX} a scan panicked; it found nothing");
                        ScanResult { kind, generation, locations: vec![] }
                    }
                };
                log::info!("{PREFIX} scan {kind:?} took {} ms, found {}", began.elapsed().as_millis(), result.locations.len());
                if results.send(result).is_err() {
                    break;
                }
            }
        })?;
        Ok(Scanner { tx, rx })
    }
}

/// The block names for tab completion, from the registry, once. Built with the first tick, long before anything is typed.
static COMPLETION_BLOCKS: OnceLock<BlockIndex> = OnceLock::new();

/// What `follow` completes: the entity types (once) and the players of the world (every few ticks). The completion has no game at hand.
#[derive(Default)]
struct CompletionWorld {
    types: Vec<String>,
    players: Vec<PlayerRef>,
}
static COMPLETION_WORLD: Mutex<CompletionWorld> = Mutex::new(CompletionWorld { types: Vec::new(), players: Vec::new() });

struct Mod {
    pathing: Pathing<Worker<CSnapshot>, CSnapshot>,
    /// The names and flags the chunk cache packs with, made from the block table.
    cache_env: Arc<CacheEnv>,
    tables: Arc<Tables>,
    base: Base,
    scanner: Scanner,
    sig: Option<Sig>,
    /// The settings table this `exec` and `tables` were made from.
    settings: Settings,
    settings_rev: u64,
    ts: ToolSettings,
    exec: ExecSettings,
    /// Free look (`freeLook` and its companions) and the rotation the server was last sent.
    look_settings: LookSettings,
    look: LookState,
    /// The dimension the goal was given in.
    dim: &'static str,
    /// The goal of the last `/b goto` or `/b path`, until the path ends.
    target: Option<Goal>,
    /// The last tick's intent took control of the keys.
    was_active: bool,
    /// The game's Auto-Jump option, put aside while a path runs.
    auto_jump_saved: Option<bool>,
    reported_cancel: Option<CancelReason>,
    /// Stop the sprint on the next tick: the path ended where no player was at hand.
    unsprint: bool,
}

impl Mod {
    fn busy(&self) -> bool {
        self.pathing.is_active() || self.pathing.is_pathing() || self.pathing.is_calculating() || self.was_active
    }
}

static STATE: Mutex<Option<Mod>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<Mod>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the movement hook puts in place of the player's keys.
#[derive(Clone, Copy)]
struct Held {
    forward: f32,
    strafe: f32,
    jump: bool,
    sneak: bool,
}

static INPUT: Mutex<Option<Held>> = Mutex::new(None);

fn input() -> MutexGuard<'static, Option<Held>> {
    INPUT.lock().unwrap_or_else(|e| e.into_inner())
}

static BUTTONS: Mutex<Buttons> = Mutex::new(Buttons::new());
/// The screen was Chat and was swapped for Playing in this input step.
static SWAPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn buttons() -> MutexGuard<'static, Buttons> {
    BUTTONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sets what the next input step holds: the left button (`Intent::attack`) and the right button (one press, `Intent::use_item` or `Intent::use_block`). A button the mod
/// held last time and does not want now is let go of once (`Buttons`), so the dig stops or the press ends.
fn set_buttons(left: bool, right: bool) {
    buttons().set(left, right);
}

struct Draw {
    current: Vec<[i32; 3]>,
    next: Vec<[i32; 3]>,
    goal: Option<Goal>,
    base_y: f64,
}

static DRAW: Mutex<Draw> = Mutex::new(Draw { current: Vec::new(), next: Vec::new(), goal: None, base_y: 0.0 });

fn draw() -> MutexGuard<'static, Draw> {
    DRAW.lock().unwrap_or_else(|e| e.into_inner())
}

fn clear_draw() {
    let mut d = draw();
    d.current.clear();
    d.next.clear();
    d.goal = None;
}

/// Chat lines the mod said, for the scenario's log. Kept only while the scenario runs.
static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn say(game: &mut Game<'_>, text: &str) {
    game.native_chat(&format!("{PREFIX} {text}"));
    log::info!("{PREFIX} {text}");
    if scenario_dir().is_some() || l3_spec().is_some() {
        NOTES.lock().unwrap_or_else(|e| e.into_inner()).push(text.to_string());
    }
}

// ---------------------------------------------------------------- the settings

/// The settings of the mod: one table for the game, loaded from `settings.txt` in the mod's own folder (`horizon-mods/.horizon-mod-data/baritone_rs/`,
/// per game install) the first time anything asks, and saved after every change as upstream does.
static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);
/// Counts changes of `SETTINGS`. The tick compares it with the one its snapshots were made from.
static SETTINGS_REV: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The registries `set` checks block and item names against.
struct GameRegistry<'a> {
    registry: &'a BlockRegistry,
    items: &'a Items,
}

impl Registry for GameRegistry<'_> {
    fn has_block(&self, path: &str) -> bool {
        self.registry.default_state(&format!("minecraft:{path}")).is_some()
    }
    fn has_item(&self, name: &str) -> bool {
        self.items.get(name).is_some()
    }
}

fn settings_file(game: &Game<'_>, name: &str) -> std::path::PathBuf {
    game.native_data_dir(ID).join(name)
}

/// Runs `f` on the settings table, loading it from the file first if it was never loaded.
fn with_settings<R>(game: &Game<'_>, f: impl FnOnce(&mut Settings, &dyn Registry) -> R) -> R {
    let mut guard = SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
    let reg = GameRegistry { registry: game.pack.assets.registry, items: game.items() };
    let st = guard.get_or_insert_with(|| {
        let mut st = Settings::new();
        let path = settings_file(game, "settings.txt");
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                for problem in st.apply_file_text(&text, &reg) {
                    log::warn!("{PREFIX} settings.txt: {problem}");
                }
                SETTINGS_REV.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("{PREFIX} settings.txt cannot be read: {e}"),
        }
        st
    });
    f(st, &reg)
}

fn save_settings(game: &Game<'_>, st: &Settings) -> Result<(), String> {
    let path = settings_file(game, "settings.txt");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, st.to_file_text()).map_err(|e| format!("cannot write settings.txt: {e}"))
}

// ---------------------------------------------------------------- the hotbar

/// The nine hotbar stacks, the off hand and the numbers `ToolSet` reads, taken once per tick (or per table).
struct Hotbar {
    stacks: Vec<Option<ItemStack>>,
    offhand: Option<ItemStack>,
    /// Per hotbar slot: what the stack's first mining-efficiency attribute adds (0 when it has none).
    efficiency: Vec<f32>,
    infos: Vec<SlotInfo>,
    off_info: SlotInfo,
    selected: usize,
    amplifier: f64,
}

/// `ToolSet.getMaterialCost`'s tags, cheapest first. Upstream tests the stack against the tags of the tool *materials* (planks, cobblestone, iron
/// ingots...), which a tool is never in, so this is -1 for every tool (docs/deviations.md, U13).
const MATERIAL_TAGS: [&str; 6] = ["minecraft:wooden_tool_materials", "minecraft:stone_tool_materials", "minecraft:iron_tool_materials", "minecraft:gold_tool_materials", "minecraft:diamond_tool_materials", "minecraft:netherite_tool_materials"];

fn slot_info(items: &Items, stack: &ItemStack) -> SlotInfo {
    let path = stack.item.strip_prefix("minecraft:").unwrap_or(&stack.item).to_string();
    SlotInfo {
        material_cost: MATERIAL_TAGS.iter().position(|t| items.item_in_tag(t, &stack.item)).map_or(-1, |i| i as i32),
        silk_touch: stack.enchantment_level("minecraft:silk_touch") > 0,
        sword: items.item_in_tag("minecraft:swords", &stack.item),
        pickaxe: path.ends_with("_pickaxe"),
        damage: stack.damage as i32,
        max_damage: items.get(&stack.item).map_or(0, |i| i.max_damage as i32),
        count: stack.count,
        item: path,
    }
}

fn read_hotbar(game: &Game<'_>) -> Hotbar {
    let items = game.items();
    let stacks: Vec<Option<ItemStack>> = (0..HOTBAR).map(|i| game.inventory.get(i).and_then(|s| s.clone()).filter(|s| s.count > 0)).collect();
    let offhand = game.offhand.clone().filter(|s| s.count > 0);
    let efficiency = stacks
        .iter()
        .map(|s| {
            s.as_ref()
                .and_then(|stack| game.stations().enchantments.attributes(stack).into_iter().find(|(attr, _, _, _)| attr == "minecraft:mining_efficiency"))
                .map_or(0.0, |(_, _, amount, _)| amount as f32)
        })
        .collect();
    let infos = stacks.iter().map(|s| s.as_ref().map_or_else(SlotInfo::empty, |st| slot_info(items, st))).collect();
    let off_info = offhand.as_ref().map_or_else(SlotInfo::empty, |st| slot_info(items, st));
    let effects = &game.potions.effects;
    Hotbar { stacks, offhand, efficiency, infos, off_info, selected: game.selected.min(HOTBAR - 1), amplifier: potion_amplifier(effects.amplifier("haste"), effects.amplifier("mining_fatigue")) }
}

/// The hotbar as the engine's `Inventory`. The speed is `ToolSet.calculateSpeedVsBlock` over Horizon's tool numbers.
struct GameInv<'a> {
    items: &'a Items,
    facts: &'a BlockFacts,
    hb: &'a Hotbar,
    /// The player's 36 main slots (`game.inventory`), read into `SlotInfo`s only when the engine asks (`mine` counting its drops).
    main: &'a [Option<ItemStack>],
    main_infos: OnceCell<Vec<SlotInfo>>,
}

impl Inventory for GameInv<'_> {
    fn selected(&self) -> usize {
        self.hb.selected
    }
    fn slot(&self, i: usize) -> SlotInfo {
        self.hb.infos.get(i).cloned().unwrap_or_else(SlotInfo::empty)
    }
    fn offhand(&self) -> SlotInfo {
        self.hb.off_info.clone()
    }
    fn speed_vs_state(&self, slot: usize, state: usize) -> f64 {
        let f = self.facts.get(BlockStateId(state as u32));
        let stack = self.hb.stacks.get(slot).and_then(|s| s.as_ref());
        let item_speed = self.items.mining_speed(stack, &f.name);
        let correct = stack.is_some() && self.items.correct_for_drops(stack, &f.name);
        calculate_speed_vs_block(f.hardness, item_speed, self.hb.efficiency.get(slot).copied().unwrap_or(0.0), f.requires_tool, stack.is_none(), correct)
    }
    fn potion_amplifier(&self) -> f64 {
        self.hb.amplifier
    }
    fn main_slots(&self) -> Vec<SlotInfo> {
        self.main_infos
            .get_or_init(|| self.main.iter().take(36).map(|s| s.as_ref().filter(|st| st.count > 0).map_or_else(SlotInfo::empty, |st| slot_info(self.items, st))).collect())
            .clone()
    }
}

// ---------------------------------------------------------------- the block table

/// `MobData::load` falls back to guesses when `mob-data.json` is missing or has the wrong number of rows. The engine cannot work
/// from guesses: `isPathfindable` decides where the player may walk. So the file is read here, and the mod refuses without it.
fn land_from_mob_data(root: &std::path::Path, states: usize, facts: usize) -> Result<Vec<bool>, String> {
    let path = root.join("mob-data.json");
    let bytes = std::fs::read(&path).map_err(|e| format!("mob-data.json cannot be read ({e}); without it Horizon only guesses where mobs may walk"))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("mob-data.json is not valid ({e})"))?;
    let rows = v["states"].as_array().ok_or_else(|| "mob-data.json has no list of states".to_string())?;
    if rows.len() != states || rows.len() != facts {
        return Err(format!("mob-data.json has {} states, the game has {} (and {} block facts); the file does not match this game", rows.len(), states, facts));
    }
    // `mobs::data::LAND`, the bit of the second column.
    rows.iter().map(|r| r[1].as_u64().map(|f| f & 1 != 0).ok_or_else(|| "mob-data.json has a state without flags".to_string())).collect()
}

/// The states as the engine reads them, from Horizon's block data.
fn build_base(game: &Game<'_>) -> Result<Base, String> {
    use super::baritone_rs_facts::{state_from_horizon, Sources};
    let started = std::time::Instant::now();
    let registry: &BlockRegistry = game.pack.assets.registry;
    let facts = game.res.facts();
    let behaviours = game.res.behaviours();
    let n = registry.len();
    let land = land_from_mob_data(&game.vanilla_root(), n, facts.states.len())?;
    let src = Sources { registry, facts, behaviours };
    let mut states = Vec::with_capacity(n);
    let mut default_of = Vec::with_capacity(n);
    let mut paths = Vec::with_capacity(n);
    let mut air_like = Vec::with_capacity(n);
    for (i, &is_land) in land.iter().enumerate() {
        let id = BlockStateId(i as u32);
        states.push(state_from_horizon(&src, id, is_land));
        let f = facts.get(id);
        air_like.push(f.has_class("TallGrassBlock") || f.has_class("DoublePlantBlock") || f.has_class("FlowerBlock"));
        let desc = registry.state(id).ok_or_else(|| format!("state {i} is not in the registry"))?;
        default_of.push(registry.default_state(&desc.name).map_or(i, |d| d.0 as usize));
        paths.push(desc.name.strip_prefix("minecraft:").unwrap_or(&desc.name).to_string());
    }
    log::info!("{PREFIX} block facts of {n} states read in {} ms", started.elapsed().as_millis());
    let index = build_index(game, registry);
    log::info!("{PREFIX} block index (states and drops) ready in {} ms", started.elapsed().as_millis());
    Ok(Base { states, default_of, paths, air_like, index })
}

/// The name the engine keeps a block under (`BlockUtils.blockToString`): the path for `minecraft:`, the whole name otherwise.
fn block_key(name: &str) -> String {
    name.strip_prefix("minecraft:").unwrap_or(name).to_string()
}

/// Whether a loot entry or pool only gives something to a tool in hand: it has a `match_tool` condition (and no `inverted` one, which would
/// make the empty hand pass). `mine` asks what a block gives when it is broken, not with silk touch or shears.
fn needs_tool(node: &serde_json::Value) -> bool {
    node.get("conditions").is_some_and(|c| {
        let text = c.to_string();
        text.contains("match_tool") && !text.contains("inverted")
    })
}

fn loot_entry_items(entry: &serde_json::Value, out: &mut Vec<String>) {
    if needs_tool(entry) {
        return;
    }
    if entry.get("type").and_then(|t| t.as_str()) == Some("minecraft:item") {
        if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
            let key = block_key(name);
            if !out.contains(&key) {
                out.push(key);
            }
        }
    }
    if let Some(children) = entry.get("children").and_then(|c| c.as_array()) {
        for child in children {
            loot_entry_items(child, out);
        }
    }
}

/// The items a block's loot table can give with an empty hand: what `BlockOptionalMeta.drops` is for. Every pool and entry that does not
/// need a tool is listed, whatever its chance (docs/deviations.md D38).
fn table_items(table: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    for pool in table.get("pools").and_then(|p| p.as_array()).into_iter().flatten() {
        if needs_tool(pool) {
            continue;
        }
        for entry in pool.get("entries").and_then(|e| e.as_array()).into_iter().flatten() {
            loot_entry_items(entry, &mut out);
        }
    }
    out
}

/// Every block state with its block and properties, and the items each block drops (from the vanilla loot tables, `blocks/<name>`).
fn build_index(game: &Game<'_>, registry: &BlockRegistry) -> BlockIndex {
    let mut states = Vec::with_capacity(registry.len());
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for i in 0..registry.len() {
        let Some(desc) = registry.state(BlockStateId(i as u32)) else {
            states.push(StateEntry::default());
            continue;
        };
        if seen.insert(desc.name.clone()) {
            names.push(desc.name.clone());
        }
        states.push(StateEntry { block: block_key(&desc.name), props: desc.properties.iter().map(|(k, v)| (k.clone(), v.clone())).collect() });
    }
    let mut drops: HashMap<String, Vec<String>> = HashMap::new();
    match horizon_world::loot::shared_tables(&game.options.resources, &game.pack_files()) {
        Ok(tables) => {
            for name in &names {
                let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
                if let Some(table) = tables.get(&format!("{ns}:blocks/{path}")) {
                    let items = table_items(table);
                    if !items.is_empty() {
                        drops.insert(block_key(name), items);
                    }
                }
            }
        }
        Err(e) => log::warn!("{PREFIX} loot tables cannot be read ({e}); mine will not know what blocks drop"),
    }
    BlockIndex::new(states, drops)
}

/// The block names for tab completion: the registry's distinct names, with no states and no drops. Made once, with the first tick.
fn completion_blocks(game: &Game<'_>) {
    COMPLETION_BLOCKS.get_or_init(|| {
        let registry: &BlockRegistry = game.pack.assets.registry;
        let mut seen = std::collections::HashSet::new();
        let mut states = Vec::new();
        for i in 0..registry.len() {
            if let Some(desc) = registry.state(BlockStateId(i as u32)) {
                if seen.insert(desc.name.clone()) {
                    states.push(StateEntry { block: block_key(&desc.name), props: vec![] });
                }
            }
        }
        BlockIndex::new(states, HashMap::new())
    });
}

/// Makes the table again when what it was made from changed: the settings, a tool or block in the hotbar, food, the potion. A table costs
/// about one `ToolSet` lookup per block and slot (the strength is that of the default state, as upstream reads it), so this runs when something
/// changed, not every tick. The searches that are running keep the table they were given.
fn refresh_tables(game: &Game<'_>, m: &mut Mod, hb: &Hotbar, food: i32) {
    // the settings changed (`/b set`, `/b set load`)
    let rev = SETTINGS_REV.load(std::sync::atomic::Ordering::SeqCst);
    if rev != m.settings_rev {
        if let Some(st) = SETTINGS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            m.settings = st.clone();
            m.settings.apply_to_exec(&mut m.exec);
            m.look_settings = m.settings.look_settings();
            m.ts = m.settings.tool_settings();
        }
        m.settings_rev = rev;
    }
    let nether = game.dim.id() == "minecraft:the_nether";
    let inv = GameInv { items: game.items(), facts: game.res.facts(), hb, main: &game.inventory, main_infos: OnceCell::new() };
    let has_throwaway = m.settings.get_bool("allowPlace") && baritone_engine::inventory::has_generic_throwaway(&inv, &m.ts.acceptable_throwaway_items);
    let has_water_bucket = m.settings.get_bool("allowWaterBucketFall") && baritone_engine::inventory::find_hotbar_slot(&inv, "water_bucket").is_some() && !nether;
    let can_sprint = m.settings.get_bool("allowSprint") && food > 6;
    let sig = Sig {
        settings_rev: rev,
        has_throwaway,
        has_water_bucket,
        can_sprint,
        amplifier_bits: hb.amplifier.to_bits(),
        hotbar: hb
            .stacks
            .iter()
            .map(|s| {
                s.as_ref().map(|st| {
                    let enchantments = st.enchantments().iter().map(|(e, l)| (e.to_string(), *l)).collect();
                    (st.item.to_string(), enchantments, if m.ts.item_saver { st.damage } else { 0 })
                })
            })
            .collect(),
    };
    if m.sig.as_ref() == Some(&sig) {
        return;
    }
    let started = std::time::Instant::now();
    let mut cfg = Config::upstream_defaults();
    m.settings.apply_to_config(&mut cfg);
    m.settings.apply_inventory(&mut cfg, &inv, nether);
    cfg.can_sprint = can_sprint;
    let n = m.base.states.len();
    let mut strength = vec![0.0; n];
    for i in 0..n {
        if m.base.default_of[i] == i {
            strength[i] = get_str_vs_block(&inv, i, &m.base.paths[i], &m.ts);
        }
    }
    for i in 0..n {
        let d = m.base.default_of[i];
        if d != i {
            strength[i] = strength[d];
        }
    }
    m.tables = Arc::new(Tables::new(m.base.states.clone(), cfg, strength));
    m.sig = Some(sig);
    log::info!("{PREFIX} block table made in {} ms (throwaway {has_throwaway}, water bucket {has_water_bucket}, sprint {can_sprint})", started.elapsed().as_millis());
}

/// Makes the state on the first goal.
fn ensure_started(game: &Game<'_>) -> Result<(), String> {
    if state().is_some() {
        return Ok(());
    }
    let base = build_base(game)?;
    let worker = Worker::try_spawn(monotonic_ms_clock()).map_err(|e| format!("could not start the search thread: {e}"))?;
    let scanner = Scanner::spawn().map_err(|e| format!("could not start the scan thread: {e}"))?;
    let pathing = Pathing::new(worker);
    let settings = with_settings(game, |st, _| st.clone());
    let settings_rev = SETTINGS_REV.load(std::sync::atomic::Ordering::SeqCst);
    let mut exec = ExecSettings::default();
    settings.apply_to_exec(&mut exec);
    let ts = settings.tool_settings();
    let look_settings = settings.look_settings();
    // The real table is made from the hotbar just below; this one only holds the place.
    let tables = Arc::new(Tables::new(base.states.clone(), Config::upstream_defaults(), vec![0.0; base.states.len()]));
    let cache_env = {
        // magma blocks are left to the setting at each pack, so the table is made as if walking on them were allowed
        let mut cfg = Config::upstream_defaults();
        cfg.allow_walk_on_magma = true;
        Arc::new(CacheEnv::new(&base.states, &base.default_of, &base.air_like, &|s: &State| baritone_engine::movement::helper::avoid_walking_into(s, &cfg)))
    };
    let mut m = Mod { pathing, cache_env, tables, base, scanner, sig: None, settings, settings_rev, ts, exec, look_settings, look: LookState::new(), dim: game.dim.id(), target: None, was_active: false, auto_jump_saved: None, reported_cancel: None, unsprint: false };
    let food = game.player.as_ref().map_or(20, |p| p.health.food.food);
    refresh_tables(game, &mut m, &read_hotbar(game), food);
    let mut s = state();
    if s.is_none() {
        *s = Some(m);
    }
    Ok(())
}

// ---------------------------------------------------------------- the player

fn player_view(game: &Game<'_>, p: &PlayerMotion, tables: &Tables) -> PlayerView {
    let eye = f64::from(game.eye);
    let head = (p.position.x.floor() as i32, (p.position.y + eye).floor() as i32, p.position.z.floor() as i32);
    let id = game.world.block(head.0, head.1, head.2).0 as usize;
    let feet = (p.position.x.floor() as i32, (p.position.y + 0.1251).floor() as i32, p.position.z.floor() as i32);
    let feet_id = game.world.block(feet.0, feet.1, feet.2).0 as usize;
    PlayerView {
        x: p.position.x,
        y: p.position.y,
        z: p.position.z,
        vx: p.velocity.x,
        vy: p.velocity.y,
        vz: p.velocity.z,
        yaw: game.yaw,
        pitch: game.pitch,
        on_ground: p.ground.on_ground,
        in_water: p.fluids.was_touching_water,
        swimming: p.swimming,
        crouching: p.local_crouching,
        horizontal_collision: p.collision.horizontal,
        in_wall: tables.states.get(id).is_some_and(|s| s.shape_full),
        food_level: p.health.food.food,
        eye_height: eye,
        feet_in_slab: tables.states.get(feet_id).is_some_and(|s| s.slab),
        looking_at: looking_at(game),
    }
}

/// What the game's crosshair is on, if it is a block: `Minecraft.hitResult` as `IPlayerContext.getSelectedBlock` reads it.
fn looking_at(game: &Game<'_>) -> Option<BlockHit> {
    let hit = game.pick()?;
    let face = match hit.face {
        [0, 1, 0] => Direction::Up,
        [0, -1, 0] => Direction::Down,
        [1, 0, 0] => Direction::East,
        [-1, 0, 0] => Direction::West,
        [0, 0, 1] => Direction::South,
        [0, 0, -1] => Direction::North,
        _ => return None,
    };
    Some(BlockHit { pos: Pos::new(hit.pos[0], hit.pos[1], hit.pos[2]), face, point: Vec3::new(hit.point[0], hit.point[1], hit.point[2]) })
}

// ---------------------------------------------------------------- stopping

/// Hands the player back: the search is cancelled, the keys are the player's again, the path is not drawn. `player` is the
/// local player when the caller can reach it; otherwise the sprint is stopped on the next tick.
fn release(m: &mut Mod, player: Option<&mut PlayerMotion>) {
    let _ = m.pathing.stop();
    m.target = None;
    m.look.clear();
    swap().plan = None;
    *input() = None;
    set_buttons(false, false);
    clear_draw();
    match player {
        Some(p) => {
            if m.was_active {
                let _ = p.set_sprinting(false);
            }
            if let Some(v) = m.auto_jump_saved.take() {
                p.auto_jump_enabled = v;
            }
        }
        None => m.unsprint |= m.was_active,
    }
    m.was_active = false;
}

/// The level is gone or the connection ends (native_client_level_unload, native_client_disconnecting).
pub(super) fn on_level_gone(window: bool, why: &str) {
    if !window {
        return;
    }
    cache_close(why);
    if let Some(m) = state().as_mut() {
        if m.busy() {
            log::info!("{PREFIX} stopped: {why}");
        }
        release(m, None);
    }
}

// ---------------------------------------------------------------- the tick

pub(super) fn on_tick(game: &mut Game<'_>) {
    if !game.native_window() {
        return;
    }
    // The player's tick is over (`tick_remote` runs this right after it): the player's view comes back before anything reads it.
    end_silent_look(game);
    register_completion(game);
    refresh_completion_players(game);
    cache_tick(game);
    let began = std::time::Instant::now();
    tick_path(game);
    PATH_TICK_US.store(began.elapsed().as_micros() as u64, std::sync::atomic::Ordering::Relaxed);
    scenario_tick(game);
    l3_tick(game);
}

/// Wall time of the last `tick_path`, in microseconds, and the snapshots taken since the L3 player last looked (tick-independent
/// numbers for the measurements of tools/l3). Cheap enough to keep always: two clock reads per tick.
static PATH_TICK_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SNAPSHOTS: Mutex<Vec<(u64, usize)>> = Mutex::new(Vec::new());

/// Whether a container is open, for `GetToBlockProcess.rightClick` (`inventoryScreenIsOpen`). A crafting table opens as the inventory screen
/// with a 3 x 3 grid, not as a `container` or a `station`.
fn container_is_open(game: &Game<'_>) -> bool {
    game.container.is_some() || game.station.is_some() || game.craft_table.is_some() || (game.screen == Screen::Inventory && game.craft_size == 3)
}

fn tick_path(game: &mut Game<'_>) {
    let mut guard = state();
    let Some(m) = guard.as_mut() else {
        return;
    };
    let dim = game.dim.id();
    if m.unsprint {
        if let Some(p) = game.player.as_mut() {
            let _ = p.set_sprinting(false);
            m.unsprint = false;
        }
    }
    // Leaving the world, dying, another dimension, or the mod turned off: the path means nothing now.
    let dead = game.screen == Screen::Dead || game.player.as_ref().is_some_and(|p| p.health.dead_or_dying());
    let why = if game.player.is_none() {
        Some("no player")
    } else if dead {
        Some("the player died")
    } else if dim != m.dim {
        Some("the dimension changed")
    } else if !game.native_on(ID) {
        Some("the mod was turned off")
    } else {
        None
    };
    if let Some(why) = why {
        let was_busy = m.busy();
        // Nothing to hand back when nothing was going on; releasing every tick would only queue events that nobody drains.
        if was_busy || input().is_some() || m.target.is_some() || m.pathing.wants_entities() {
            release(m, game.player.as_mut());
        }
        m.dim = dim;
        if was_busy {
            drop(guard);
            say(game, &format!("Stopped: {why}."));
        }
        return;
    }
    let Some(p) = game.player.as_ref() else {
        return;
    };
    let hotbar = read_hotbar(game);
    refresh_tables(game, m, &hotbar, p.health.food.food);
    // `IPlayerContext.playerRotations` and `objectMouseOver`: with free look, the executor sees the rotation the server was sent, and the
    // crosshair is the ray along it, not along the player's view.
    let own = Rotation::new(game.yaw, game.pitch);
    let eff = m.look.effective(&m.look_settings, own);
    let view = if eff == own {
        player_view(game, p, &m.tables)
    } else {
        game.yaw = eff.yaw;
        game.pitch = eff.pitch;
        let v = player_view(game, p, &m.tables);
        game.yaw = own.yaw;
        game.pitch = own.pitch;
        v
    };
    // the aim lands on the player's own mouse grid, as upstream (D17)
    m.exec.mouse_sensitivity = f64::from(game.settings.sensitivity);
    let [min_x, min_z, max_x, max_z] = game.border.bounds();
    let border = Border { min_x, max_x, min_z, max_z };
    let registry: &BlockRegistry = game.pack.assets.registry;
    // What `mine` and `goto <block>` read besides the world: the clock, the dropped items, whether a container is open. The answers of the scans
    // that finished since the last tick go to their processes first.
    if m.pathing.is_active() || m.pathing.wants_entities() {
        let item_entities = game
            .dropped
            .iter()
            .map(|e| DroppedItem { item: e.stack.item.strip_prefix("minecraft:").unwrap_or(&e.stack.item).to_string(), pos: Pos::new(e.position.x.floor() as i32, e.position.y.floor() as i32, e.position.z.floor() as i32) })
            .collect();
        // The follow process decides whether it is active by looking at them, so they are listed while it holds a filter, and only then.
        let entities = if m.pathing.wants_entities() { follow_entities(game) } else { Vec::new() };
        m.pathing.set_inputs(ProcessInputs { now_ms: wall_ms(), item_entities, container_open: container_is_open(game), entities });
    }
    while let Ok(result) = m.scanner.rx.try_recv() {
        m.pathing.deliver_scan(result, &m.tables.cfg, &m.exec);
    }
    let reader = cache_reader();
    let out = {
        let live = WithCache::new(Live { world: &game.world, registry, falling: falling_positions(game) }, reader.clone());
        let inv = GameInv { items: game.items(), facts: game.res.facts(), hb: &hotbar, main: &game.inventory, main_infos: OnceCell::new() };
        let facts = GameFacts { reach: game.block_reach, nether: game.dim.id() == "minecraft:the_nether", aim: baritone_engine::look::AimProcessor::new(m.exec.mouse_sensitivity) };
        let env = TickEnv { tables: &m.tables, world: &live, border, player: &view, exec: &m.exec, inv: &inv, game: facts };
        let mut snapshot = || {
            let began = std::time::Instant::now();
            let snap = Snapshot::take(&game.world, registry.len());
            let mut log = SNAPSHOTS.lock().unwrap_or_else(|e| e.into_inner());
            if log.len() >= 4096 {
                log.remove(0);
            }
            log.push((began.elapsed().as_micros() as u64, snap.chunks.len()));
            WithCache::new(snap, reader.clone())
        };
        m.pathing.tick(&env, &mut snapshot)
    };

    let mut lines: Vec<String> = m.pathing.take_messages();
    // The scans the processes want (one per process at a time): the chunks are copied here, the search of them runs on the scan thread.
    let jobs = m.pathing.take_scan_jobs();
    let mut scan_died = false;
    if !jobs.is_empty() {
        let began = std::time::Instant::now();
        for job in jobs {
            let snapshot = WithCache::new(Snapshot::take(&game.world, registry.len()), reader.clone());
            scan_died |= m.scanner.tx.send(ScanRequest { job, snapshot, tables: Arc::clone(&m.tables), border }).is_err();
        }
        log::info!("{PREFIX} scan requested, chunks copied in {} us", began.elapsed().as_micros());
    }
    let mut arrived = false;
    // `AtGoal` also ends each walk of `mine` and `goto <block>`, which upstream does not announce: only a `/b goto` or `/b path` does.
    let announces = m.target.is_some();
    for e in &out.events {
        match e {
            PathEvent::AtGoal => arrived |= announces,
            PathEvent::CalcFailed => lines.push("No path to the goal was found.".to_string()),
            _ => {}
        }
    }
    // Upstream's process gives up control as soon as the feet are in the goal, which can be before the path ticks and says
    // `AtGoal`. So arriving is also: the goal process is done, and the player is in the goal.
    if m.target.is_some() && !m.pathing.is_active() {
        let (f, gs) = (view.feet(), m.tables.cfg.goal_settings());
        arrived |= m.target.as_ref().is_some_and(|g| g.is_in_goal(f.x, f.y, f.z, &gs));
        m.target = None;
    }
    if scan_died {
        release(m, game.player.as_mut());
        lines.push("The scan thread is gone. Stopped.".to_string());
        drop(guard);
        for line in lines {
            say(game, &line);
        }
        return;
    }
    if m.pathing.take_search_failure() {
        // D23: a search that panics would start again on the next tick, forever, with the player standing still.
        release(m, game.player.as_mut());
        lines.push("The search failed with an internal error (see the log). The goto was stopped.".to_string());
        drop(guard);
        for line in lines {
            say(game, &line);
        }
        return;
    }
    if arrived {
        lines.push("Arrived.".to_string());
        ARRIVED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    if let Some(reason) = m.pathing.last_cancel() {
        if m.reported_cancel != Some(reason) {
            m.reported_cancel = Some(reason);
            log::info!("{PREFIX} the path was cancelled: {reason:?}");
        }
    }

    // Apply the intent. The keys go to the next tick's movement; the view and the sprint are the player's now.
    let intent = &out.intent;
    *input() = intent.active.then(|| Held { forward: intent.forward_impulse(), strafe: intent.strafe_impulse(), jump: intent.jump, sneak: intent.sneak });
    // Free look (docs/deviations.md D35): the rotation is the player's own view (mode client), or is put in for the next player tick only.
    swap().plan = None;
    if let Some(plan) = m.look.plan(&m.look_settings, intent.rotation, intent.look_target, intent.look_force) {
        match restored_view(&plan, Rotation::new(game.yaw, game.pitch)) {
            None => {
                game.yaw = plan.rotation.yaw;
                game.pitch = plan.rotation.pitch;
            }
            Some(_) => swap().plan = Some(plan),
        }
    }
    // The slot: the game tells the server on its next tick (probed, with any screen). Nothing is put back when the path ends, as upstream.
    if let Some(slot) = intent.select_slot {
        game.selected = slot.min(HOTBAR - 1);
    }
    // The buttons act in the next input step (`on_input_head`). Left wins over right, as the intent says. The right button is a press for a placement or a
    // bucket (`use_item`) and for a closed door or fence gate (`use_block`: the aim is the movement's, the game's own use acts on what the crosshair is on).
    // Also while the keys are not forced: `mine` pauses the path and clicks (a block in the player's own column), which is `active == false`.
    set_buttons(intent.attack, (intent.use_item || intent.use_block.is_some()) && !intent.attack);
    if let Some(p) = game.player.as_mut() {
        if intent.active {
            if p.sprinting != intent.sprint {
                let _ = p.set_sprinting(intent.sprint);
            }
            if intent.cancel_flying && p.flying {
                p.flying = false;
            }
            if intent.disable_auto_jump && m.auto_jump_saved.is_none() {
                m.auto_jump_saved = Some(p.auto_jump_enabled);
                p.auto_jump_enabled = false;
            }
        } else if m.was_active {
            // The path just ended: the game's own sprint rules take over from a walking player.
            let _ = p.set_sprinting(false);
        }
        if !(intent.active && intent.disable_auto_jump) {
            if let Some(v) = m.auto_jump_saved.take() {
                p.auto_jump_enabled = v;
            }
        }
    }
    if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
        trace_row(game, m, intent);
    }
    m.was_active = intent.active;
    publish_draw(m, view.y);
    drop(guard);
    for line in lines {
        say(game, &line);
    }
}

// ---------------------------------------------------------------- the chunk cache

/// One loaded chunk, for the packer thread. The chunk is an `Arc` the game copies on write, so this is the chunk as it was when it was queued.
struct GlueChunk {
    chunk: Arc<Chunk>,
    min_y: i32,
    air: usize,
    limit: usize,
    /// The chunks to the west, east, north and south, for the water flow at the edge (D3).
    next: [Option<Arc<Chunk>>; 4],
}

impl PackSource for GlueChunk {
    fn state(&self, x: i32, y: i32, z: i32) -> usize {
        known_state(self.chunk.get(x, y + self.min_y, z) as usize, self.limit, self.air)
    }
    fn section_has_only_air(&self, section: usize) -> bool {
        self.chunk.sections.get(section).is_none_or(|s| s.is_uniform(self.air as u32))
    }
    fn fill_section(&self, section: usize, out: &mut [usize; 4096]) {
        let mut cells = [0u32; 4096];
        if let Some(s) = self.chunk.sections.get(section) {
            s.unpack(&mut cells);
        }
        for (slot, v) in out.iter_mut().zip(cells.iter()) {
            *slot = known_state(*v as usize, self.limit, self.air);
        }
    }
    fn outside_state(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let (n, cx, cz) = match (x, z) {
            (-1, 0..=15) => (0, 15, z),
            (16, 0..=15) => (1, 0, z),
            (0..=15, -1) => (2, x, 15),
            (0..=15, 16) => (3, x, 0),
            _ => return None,
        };
        let c = self.next[n].as_ref()?;
        Some(known_state(c.get(cx, y + self.min_y, cz) as usize, self.limit, self.air))
    }
}

/// The chunks within this many chunks of the player are packed again every `REPACK_EVERY` (`repackOnAnyBlockChange`).
const REPACK_RADIUS: i32 = 5;
const REPACK_EVERY: std::time::Duration = std::time::Duration::from_secs(4);

/// The cached world in use, with the chunks that are loaded.
struct ActiveCache {
    /// Of the world or server, and the dimension: when either changes the cache is closed and another opened.
    key: String,
    dim: &'static str,
    data: Arc<WorldData>,
    /// The chunks that are loaded, and when each was last queued for packing. No chunk is held here: a held `Arc` would make the game copy the chunk
    /// each time it changes one in place.
    tracked: HashMap<(i32, i32), std::time::Instant>,
    settings_rev: u64,
    opened: std::time::Instant,
    ticks: u64,
}

struct CacheHost {
    provider: WorldProvider,
    active: Option<ActiveCache>,
}

static CACHE: Mutex<Option<CacheHost>> = Mutex::new(None);
/// Microseconds the last cache tick took on the game thread, and the largest since the world was entered.
static CACHE_TICK_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CACHE_TICK_MAX_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn cache_host() -> MutexGuard<'static, Option<CacheHost>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A reader of the cached world in use, for a snapshot. `None` outside a world.
fn cache_reader() -> Option<CacheReader> {
    cache_host().as_ref()?.active.as_ref().map(|a| CacheReader::new(Arc::clone(&a.data.world)))
}

/// Leaving the world, or the dimension: the cache is saved on a thread of its own (`WorldData.onClose`) and the chunks being tracked are forgotten.
/// Upstream packs nothing that unloads after the new world is up, so nothing is packed here either.
fn cache_close(why: &str) {
    let Some(host) = cache_host().as_mut().and_then(|h| h.active.take()) else {
        return;
    };
    log::info!("{PREFIX} cache closed ({why}): {} chunks tracked, {} regions in memory, {} waiting to be packed", host.tracked.len(), host.data.world.region_count(), host.data.pending());
    host.data.world.set_player(None);
    host.data.on_close();
}

/// The server begins an orderly stop (`native_server_stopping`): every cached world is saved now, on this thread, because the process may end as soon as the stop is over.
/// A save from `cache_close` runs on a thread of its own and can be cut off by the end of the process. The files are written whole or not at all, so a cut-off save only
/// loses what it had not written.
pub(super) fn on_server_stopping() {
    let worlds = cache_host().as_ref().map(|h| h.provider.all()).unwrap_or_default();
    for w in worlds {
        let began = std::time::Instant::now();
        let report = w.world.save();
        for note in w.world.take_notes() {
            log::info!("{PREFIX} {note}");
        }
        log::info!("{PREFIX} cache saved at the server stop: {} regions, {} bytes, {} failed, {} ms", report.saved, report.bytes, report.failed, began.elapsed().as_millis());
    }
}

/// The folder of the world or server, named so that no address is in it: a singleplayer world by its name, a server by a hash of its address.
fn cache_world_key(game: &Game<'_>) -> Option<String> {
    if let Some(i) = game.integrated.as_ref() {
        return Some(baritone_engine::cache::singleplayer_key(&i.world_name));
    }
    let address = game.remote.as_ref().map(|r| r.session.address.clone())?;
    Some(baritone_engine::cache::server_key(&address))
}

/// The cache's part of a tick, whatever the mod is doing: opens the cached world of the player's world and dimension, packs the chunks that came or
/// went, and repacks the ones that changed. The packing and the saving run on their own threads.
fn cache_tick(game: &mut Game<'_>) {
    let in_world = game.player.is_some() && game.native_on(ID);
    if !in_world {
        cache_close("not in a world");
        return;
    }
    let began = std::time::Instant::now();
    let Some(key) = cache_world_key(game) else {
        cache_close("no world key");
        return;
    };
    let dim = game.dim.id();
    // the chunk table the cache packs with comes with the mod's state, which is made here if no command did
    // (the guard of `state()` must be gone before `ensure_started` takes it: a `match` on it would keep it to the end)
    let have = state().as_ref().map(|m| Arc::clone(&m.cache_env));
    let env = match have {
        Some(e) => e,
        None => {
            if let Err(e) = ensure_started(game) {
                static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    log::warn!("{PREFIX} the chunk cache cannot start: {e}");
                }
                return;
            }
            let made = state().as_ref().map(|m| Arc::clone(&m.cache_env));
            match made {
                Some(e) => e,
                None => return,
            }
        }
    };
    let (min_y, height) = (game.world.bottom_y(), game.world.top_y() - game.world.bottom_y());
    let dimension = Dimension::from_id(dim, min_y, height);
    let rev = SETTINGS_REV.load(std::sync::atomic::Ordering::SeqCst);
    let cache_settings = with_settings(game, |st, _| (st.cache_settings(), st.get_bool("allowWalkOnMagmaBlocks")));
    let data_dir = game.native_data_dir(ID).join("cache");
    let mut guard = cache_host();
    let host = guard.get_or_insert_with(|| CacheHost { provider: WorldProvider::new(), active: None });
    if host.active.as_ref().is_some_and(|a| a.key != key || a.dim != dim) {
        drop(guard);
        cache_close("another world or dimension");
        guard = cache_host();
    }
    let host = guard.get_or_insert_with(|| CacheHost { provider: WorldProvider::new(), active: None });
    if host.active.is_none() {
        let dir = data_dir.join(&key).join(baritone_engine::cache::dimension_folder(dim, dimension.logical_height)).join("cache");
        let opened = host.provider.open(dir.clone(), || {
            let clock: Box<dyn Fn() -> i64 + Send + Sync> = Box::new(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64));
            Arc::new(CachedWorld::new(dir.clone(), dimension, Arc::clone(&env), cache_settings.0.clone(), clock))
        });
        match opened {
            Ok(data) => {
                log::info!("{PREFIX} cache opened for {key} {dim}: {} regions in memory", data.world.region_count());
                host.active = Some(ActiveCache { key: key.clone(), dim, data, tracked: HashMap::new(), settings_rev: 0, opened: std::time::Instant::now(), ticks: 0 });
            }
            Err(e) => {
                log::warn!("{PREFIX} the chunk cache cannot start its threads: {e}");
                return;
            }
        }
    }
    let Some(active) = host.active.as_mut() else { return };
    active.ticks += 1;
    if active.settings_rev != rev {
        active.data.world.set_settings(cache_settings.0.clone());
        active.data.set_allow_walk_on_magma(cache_settings.1);
        active.settings_rev = rev;
    }
    // where the player is, for the pruning of far regions
    if active.ticks % 20 == 1 {
        if let Some(p) = game.player.as_ref() {
            active.data.world.set_player(Some((p.position.x.floor() as i32, p.position.z.floor() as i32)));
        }
    }
    let repack = cache_settings.0.repack_on_any_block_change;
    let limit = game.pack.assets.registry.len();
    let air = game.world.air.0 as usize;
    let now = std::time::Instant::now();
    let queue = |data: &WorldData, key: (i32, i32), chunk: &Arc<Chunk>, world: &World| {
        let next = [(key.0 - 1, key.1), (key.0 + 1, key.1), (key.0, key.1 - 1), (key.0, key.1 + 1)].map(|k| world.chunks.get(&k).cloned());
        data.queue_for_packing(PackJob { chunk_x: key.0, chunk_z: key.1, source: Box::new(GlueChunk { chunk: Arc::clone(chunk), min_y, air, limit, next }) });
    };
    // chunks that came (`ChunkEvent` post populate)
    for (k, c) in game.world.chunks.iter() {
        if c.min_y() != min_y {
            continue; // the chunk of another dimension, while the level is being replaced
        }
        if !active.tracked.contains_key(k) {
            if !active.data.has_room() {
                break; // the rest wait for the next tick: a full queue would throw its oldest chunks away unpacked
            }
            queue(&active.data, *k, c, &game.world);
            active.tracked.insert(*k, now);
        }
    }
    // The game gives no event for a changed block, and a held chunk would be copied at each change, so the chunks around the player are packed again from
    // time to time (docs/deviations.md D41); a pack that says what the cache holds already stores nothing. A chunk that goes is not packed: it is gone.
    if repack {
        if let Some(p) = game.player.as_ref() {
            let (pcx, pcz) = ((p.position.x.floor() as i32) >> 4, (p.position.z.floor() as i32) >> 4);
            for dx in -REPACK_RADIUS..=REPACK_RADIUS {
                for dz in -REPACK_RADIUS..=REPACK_RADIUS {
                    let k = (pcx + dx, pcz + dz);
                    if let (Some(c), Some(t)) = (game.world.chunks.get(&k), active.tracked.get_mut(&k)) {
                        if now.duration_since(*t) >= REPACK_EVERY {
                            queue(&active.data, k, c, &game.world);
                            *t = now;
                        }
                    }
                }
            }
        }
    }
    active.tracked.retain(|k, _| game.world.chunks.contains_key(k));
    for note in active.data.world.take_notes() {
        log::info!("{PREFIX} {note}");
    }
    let us = began.elapsed().as_micros() as u64;
    CACHE_TICK_US.store(us, std::sync::atomic::Ordering::Relaxed);
    if active.ticks == 1 {
        // the first tick can include making the block table, when no command did
        log::info!("{PREFIX} cache: first tick {us} us");
    } else {
        CACHE_TICK_MAX_US.fetch_max(us, std::sync::atomic::Ordering::Relaxed);
    }
    if active.ticks % 1200 == 0 {
        let (packed, dropped) = active.data.counts();
        let (unchanged, pack_us, pack_max_us) = active.data.pack_stats();
        log::info!(
            "{PREFIX} cache: {} chunks tracked, {packed} packed ({unchanged} unchanged, {} us each on average, longest {pack_max_us} us), {dropped} dropped, {} waiting, {} regions in memory, {} unsaved; game-thread tick {us} us (largest since the first {} us); open {} s",
            active.tracked.len(),
            pack_us / packed.max(1),
            active.data.pending(),
            active.data.world.region_count(),
            active.data.world.unsaved_regions(),
            CACHE_TICK_MAX_US.load(std::sync::atomic::Ordering::Relaxed),
            active.opened.elapsed().as_secs()
        );
    }
}

/// A millisecond clock for the processes (`System.currentTimeMillis`): any clock that only counts up.
fn wall_ms() -> i64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as i64
}

fn publish_draw(m: &Mod, base_y: f64) {
    let positions = |e: &baritone_engine::exec::PathExecutor, skip: usize| e.path().positions().iter().skip(skip).map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>();
    let mut d = draw();
    d.current = m.pathing.current().map(|c| positions(c, c.position().max(0) as usize)).unwrap_or_default();
    d.next = m.pathing.next().map(|n| positions(n, 0)).unwrap_or_default();
    d.goal = m.pathing.goal().cloned();
    d.base_y = base_y;
}

/// native_movement_input: runs before the player moves, with whatever the last tick decided.
// ---------------------------------------------------------------- /b trace

/// `/b trace`: one log line per tick while a path runs, to see why a walk fails where the scenarios cannot go (a server).
static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Calls of the movement hook, and the calls in which the mod's keys replaced the player's.
static HOOK_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static HOOK_APPLIED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn trace_row(game: &Game<'_>, m: &Mod, intent: &baritone_engine::exec::state::Intent) {
    use std::sync::atomic::Ordering::Relaxed;
    let Some(p) = game.player.as_ref() else {
        return;
    };
    let (calls, applied) = (HOOK_CALLS.swap(0, Relaxed), HOOK_APPLIED.swap(0, Relaxed));
    if !m.busy() && !intent.active && applied == 0 {
        return;
    }
    let mut mov = String::from("none");
    if let Some(c) = m.pathing.current() {
        let i = c.position();
        if let Some(mv) = usize::try_from(i).ok().and_then(|i| c.movements().get(i)) {
            mov = format!("{i}/{} {:?} {},{},{}>{},{},{} cost={:.2}", c.movements().len(), mv.kind, mv.src.x, mv.src.y, mv.src.z, mv.dest.x, mv.dest.y, mv.dest.z, mv.cost.unwrap_or(f64::NAN));
        }
    }
    log::info!(
        "{PREFIX} trace pos={:.3},{:.3},{:.3} vel={:.3},{:.3},{:.3} ground={} yaw={:.1} pitch={:.1} sent_yaw={} sprinting={} | intent active={} fwd={} strafe={} jump={} sneak={} sprint={} | hook calls={calls} applied={applied} | pathing={} calculating={} screen_playing={} | move {mov} | cancel={:?}",
        p.position.x,
        p.position.y,
        p.position.z,
        p.velocity.x,
        p.velocity.y,
        p.velocity.z,
        p.ground.on_ground as u8,
        game.yaw,
        game.pitch,
        m.look.server_rotation().map_or("-".to_string(), |r| format!("{:.1}/{:.1}", r.yaw, r.pitch)),
        p.sprinting as u8,
        intent.active as u8,
        intent.forward_impulse(),
        intent.strafe_impulse(),
        intent.jump as u8,
        intent.sneak as u8,
        intent.sprint as u8,
        m.pathing.is_pathing() as u8,
        m.pathing.is_calculating() as u8,
        (game.screen == Screen::Playing) as u8,
        m.pathing.last_cancel()
    );
}

/// Start of the game's input step (`remote_handle_input`, pinned): puts the mod's button values where a held mouse button would be, and with the
/// chat open lets the step see `Screen::Playing` for this one call. Without the swap the step does nothing but abort a dig (docs/PLAN.md).
pub(super) fn on_input_head(game: &mut Game<'_>) {
    begin_silent_look(game);
    let b = *buttons();
    if !b.driving() {
        return;
    }
    if b.drive_left {
        game.left = b.left;
    }
    if b.drive_right {
        game.right = b.right;
    }
    if game.screen == Screen::Chat {
        game.screen = Screen::Playing;
        SWAPPED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    // A button let go of is done after one step the game really ran: not on a screen where the step does nothing (inventory, a dialog), where the
    // let-go would be lost and the dig go on.
    if matches!(game.screen, Screen::Playing) {
        buttons().step_done();
    }
}

/// End of the input step: the chat comes back.
pub(super) fn on_input_return(game: &mut Game<'_>) {
    // Only if the step left the screen as the swap set it: a step that opened a container or ended in a death keeps its own screen.
    if SWAPPED.swap(false, std::sync::atomic::Ordering::SeqCst) && game.screen == Screen::Playing {
        game.screen = Screen::Chat;
    }
}

pub(super) fn on_movement(forward: &mut f32, strafe: &mut f32, jump: &mut bool, sneak: &mut bool) {
    HOOK_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Some(h) = *input() {
        HOOK_APPLIED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *forward = h.forward;
        *strafe = h.strafe;
        *jump = h.jump;
        *sneak = h.sneak;
    }
}

// ---------------------------------------------------------------- free look

/// A rotation put in for one player tick (free look, mode server, or smooth look): what to put in, and the view that was there.
#[derive(Default)]
struct Swap {
    plan: Option<Plan>,
    /// Set while the rotation is in: the player's view before it, until `end_silent_look` puts it back.
    saved: Option<Rotation>,
}

static SWAP: Mutex<Swap> = Mutex::new(Swap { plan: None, saved: None });

fn swap() -> MutexGuard<'static, Swap> {
    SWAP.lock().unwrap_or_else(|e| e.into_inner())
}

/// Start of the game's input step, which is the start of what the tick does for the player: the picks for the clicks, the movement, the
/// packet with the rotation. The rotation of the plan is the game's view from here to `end_silent_look`.
fn begin_silent_look(game: &mut Game<'_>) {
    if !game.native_window() {
        return;
    }
    // The last tick ended without `end_silent_look` (it returned early): the player's view first.
    end_silent_look(game);
    let mut sw = swap();
    // the plan stays, to know how the view comes back
    if let Some(plan) = sw.plan {
        sw.saved = Some(Rotation::new(game.yaw, game.pitch));
        game.yaw = plan.rotation.yaw;
        game.pitch = plan.rotation.pitch;
    }
}

/// End of the player's tick (the mod's own tick runs right after it): the player's view comes back, so the next frame draws it and the mouse
/// goes on adding to it.
fn end_silent_look(game: &mut Game<'_>) {
    let mut sw = swap();
    if let Some(before) = sw.saved.take() {
        if let Some(plan) = sw.plan.take() {
            if let Some(v) = restored_view(&plan, before) {
                game.yaw = v.yaw;
                game.pitch = v.pitch;
            }
        }
    }
}

// ---------------------------------------------------------------- drawing

const COLOR_CURRENT: u32 = 0xFFFF_3030;
const COLOR_NEXT: u32 = 0xFFFF_30FF;
const COLOR_GOAL: u32 = 0xFF30_FF60;

/// native_world_primitives: the path as lines through walls (upstream's `renderPathIgnoreDepth` is on), the goal as a box or a beam.
pub(super) fn on_primitives(primitives: &mut horizon_client::world_primitives::WorldPrimitives) {
    let d = draw();
    let line = |primitives: &mut horizon_client::world_primitives::WorldPrimitives, path: &[[i32; 3]], color: u32| {
        for w in path.windows(2) {
            let a = [f64::from(w[0][0]) + 0.5, f64::from(w[0][1]) + 0.1, f64::from(w[0][2]) + 0.5];
            let b = [f64::from(w[1][0]) + 0.5, f64::from(w[1][1]) + 0.1, f64::from(w[1][2]) + 0.5];
            primitives.line(a, b, color, 3.0).through_walls();
        }
    };
    line(primitives, &d.current, COLOR_CURRENT);
    line(primitives, &d.next, COLOR_NEXT);
    match &d.goal {
        Some(Goal::Block { x, y, z }) => {
            let min = [f64::from(*x), f64::from(*y), f64::from(*z)];
            primitives.box_outline(min, [min[0] + 1.0, min[1] + 2.0, min[2] + 1.0], COLOR_GOAL, 3.0).through_walls();
        }
        Some(Goal::Xz { x, z }) => {
            primitives.beam([f64::from(*x) + 0.5, d.base_y.floor() - 1.0, f64::from(*z) + 0.5], 256, COLOR_GOAL);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- commands

pub(super) fn on_commands(commands: &mut Vec<&'static str>) {
    commands.push("b");
    commands.push("baritone");
}

/// The commands, with the words `HelpCommand` lists them under.
fn help_lines() -> Vec<String> {
    let mut lines = vec!["Commands (also /baritone):".to_string()];
    for (names, description) in baritone_engine::command::COMMANDS {
        lines.push(format!("/b {} - {}", names.join(" | "), description));
    }
    lines.push("/b help - This list".to_string());
    lines.push("/b trace - One log line per tick while a path runs, for bug reports".to_string());
    lines.push("/b goto <block>, /b mine [quantity] <block> [<block>...]: blocks by name (minecraft: is optional), also with properties, as stone or oak_stairs[facing=east]".to_string());
    lines.push("/b follow entities | players | entity <type...> | player <name...>: follow those (entity types by id, as pig or minecraft:pig). /b stop ends it.".to_string());
    lines.push("/b explore [x z]: walk to the closest chunks the cache lacks, around you or around x z. /b explorefilter <file> [invert]: a JSON list [{\"x\":0,\"z\":0},...] of chunks (not blocks) to count as explored, or with invert as the only ones not explored; a relative path is in the mod data folder.".to_string());
    lines.push("/b goto <x> <y> <z> | <x> <z> | <y>, /b goal [x y z | clear], /b set <name> [value] | list | modified | reset | toggle | save | load".to_string());
    lines.push("A ~ in front of a number is relative to you. Tab completes the commands, the settings and their values.".to_string());
    lines
}

pub(super) fn on_command(game: &mut Game<'_>, command: &str, ran: &mut bool) {
    let mut words = command.split_whitespace();
    if !matches!(words.next(), Some("b" | "baritone")) {
        return;
    }
    // Claimed also when the mod is off, so that the command is not sent to the server as chat.
    *ran = true;
    if !game.native_on(ID) {
        say(game, "baritone-rs is turned off in the mod settings.");
        return;
    }
    let args: Vec<&str> = words.collect();
    let name = args.first().map(|a| a.to_lowercase());
    let rest = args.get(1..).unwrap_or(&[]);
    match name.as_deref() {
        Some("goto") => cmd_goto(game, rest),
        Some("mine") => cmd_mine(game, rest),
        Some("follow") => cmd_follow(game, rest),
        Some("explore") => cmd_explore(game, rest),
        Some("explorefilter") => cmd_explore_filter(game, rest),
        Some("goal") => cmd_goal(game, rest),
        Some("path") => cmd_path(game, rest),
        Some("eta") => cmd_eta(game, rest),
        Some("set" | "setting" | "settings") => cmd_set(game, rest),
        Some("stop" | "cancel") => cmd_stop(game),
        Some("trace") => {
            let on = !TRACE.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
            say(game, if on { "Trace on: one line per tick in the game log while a path runs. /b trace again to stop." } else { "Trace off." });
        }
        None | Some("help") => {
            for line in help_lines() {
                say(game, &line);
            }
        }
        Some(_) => say(game, &format!("Unknown command \"{}\". Try /b help.", args[0])),
    }
}

fn censor(game: &Game<'_>) -> bool {
    with_settings(game, |st, _| st.get_bool("censorCoordinates"))
}

/// Makes the state, and says why not when it cannot. The player's feet (`IPlayerContext.playerFeet`, with the soul sand offset and the slab rule).
fn start_and_feet(game: &mut Game<'_>) -> Option<Pos> {
    if game.player.is_none() {
        say(game, "Not in a world.");
        return None;
    }
    if let Err(why) = ensure_started(game) {
        say(game, &format!("Not pathing: {why}."));
        return None;
    }
    let guard = state();
    let (Some(m), Some(p)) = (guard.as_ref(), game.player.as_ref()) else {
        return None;
    };
    Some(player_view(game, p, &m.tables).feet())
}

fn cmd_goto(game: &mut Game<'_>, args: &[&str]) {
    let Some(feet) = start_and_feet(game) else {
        return;
    };
    let goal = match goto_goal(args, feet) {
        Ok(goal) => goal,
        Err(GotoError::NoArguments) => {
            say(game, "Usage: /b goto <x> <y> <z>, /b goto <x> <z> or /b goto <y>.");
            return;
        }
        Err(GotoError::TooManyArguments) => {
            say(game, "Too many arguments: /b goto takes at most three coordinates.");
            return;
        }
        Err(GotoError::BlockName) => {
            cmd_goto_block(game, args, feet);
            return;
        }
    };
    let text = goal_to_string(&goal, censor(game));
    if let Some(m) = state().as_mut() {
        m.dim = game.dim.id();
        m.reported_cancel = None;
        m.target = Some(goal.clone());
        m.pathing.goto(goal);
    }
    say(game, &format!("Going to: {text}"));
}

/// `/b goto <block>`: walk to the nearest block of that kind (`GetToBlockProcess`).
fn cmd_goto_block(game: &mut Game<'_>, args: &[&str], feet: Pos) {
    let bom = {
        let guard = state();
        let Some(m) = guard.as_ref() else {
            return;
        };
        match goto_block(args, &m.base.index) {
            Ok(bom) => bom,
            Err(e) => {
                drop(guard);
                say(game, &e);
                return;
            }
        }
    };
    if let Some(m) = state().as_mut() {
        m.dim = game.dim.id();
        m.reported_cancel = None;
        m.target = None;
        m.pathing.get_to_block(bom, feet);
    }
}

/// `/b mine [quantity] <block> [<block>...]`: `MineProcess`. The quantity counts the drops in the inventory.
fn cmd_mine(game: &mut Game<'_>, args: &[&str]) {
    if start_and_feet(game).is_none() {
        return;
    }
    let request = {
        let guard = state();
        let Some(m) = guard.as_ref() else {
            return;
        };
        match mine_command(args, &m.base.index) {
            Ok(r) => r,
            Err(e) => {
                drop(guard);
                say(game, &e);
                return;
            }
        }
    };
    if let Some(m) = state().as_mut() {
        m.dim = game.dim.id();
        m.reported_cancel = None;
        m.target = None;
        let cfg = m.tables.cfg.clone();
        m.pathing.mine(request.quantity, request.filter.clone(), &cfg);
    }
    say(game, &request.message());
}

/// `/b follow entities | players | entity <type...> | player <name...>`: `FollowProcess`.
fn cmd_follow(game: &mut Game<'_>, args: &[&str]) {
    if start_and_feet(game).is_none() {
        return;
    }
    let outcome = follow_command(args, &world_players(game, true), &entity_type_ids(game));
    // upstream sets the filter first, then prints, and throws last: `follow player <nobody>` sets an empty filter and then complains
    if let Some(filter) = outcome.filter {
        if let Some(m) = state().as_mut() {
            m.dim = game.dim.id();
            m.reported_cancel = None;
            m.target = None;
            m.pathing.follow(filter);
        }
    }
    for line in &outcome.lines {
        say(game, line);
    }
    if let Some(error) = &outcome.error {
        say(game, error);
    }
}

/// `/b explore [x z]`: `ExploreProcess`.
fn cmd_explore(game: &mut Game<'_>, args: &[&str]) {
    let Some(feet) = start_and_feet(game) else {
        return;
    };
    match explore_command(args, feet) {
        Ok(request) => {
            if let Some(m) = state().as_mut() {
                m.dim = game.dim.id();
                m.reported_cancel = None;
                m.target = None;
                m.pathing.explore(request.x, request.z);
            }
            say(game, &request.message(censor(game)));
        }
        Err(e) => say(game, &e),
    }
}

/// The biggest file `explorefilter` reads. 16 MiB is about 700 000 chunks; the file is read on the game thread, so it is kept small.
const MAX_FILTER_FILE: u64 = 16 << 20;

/// Reads the file the user named, and only that: a path as typed (a relative one is in the mod's data folder), which must be a regular file. Nothing else is opened or listed.
fn read_filter_file(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let meta = std::fs::metadata(path).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "File not found".to_string() } else { "The file could not be read".to_string() })?;
    if !meta.is_file() {
        return Err("That is not a file".to_string());
    }
    if meta.len() > MAX_FILTER_FILE {
        return Err(format!("The file is too big (over {} MiB)", MAX_FILTER_FILE >> 20));
    }
    let file = std::fs::File::open(path).map_err(|_| "The file could not be read".to_string())?;
    let mut bytes = Vec::new();
    // the size can grow after the check: read no more than the limit plus one byte
    file.take(MAX_FILTER_FILE + 1).read_to_end(&mut bytes).map_err(|_| "The file could not be read".to_string())?;
    if bytes.len() as u64 > MAX_FILTER_FILE {
        return Err(format!("The file is too big (over {} MiB)", MAX_FILTER_FILE >> 20));
    }
    String::from_utf8(bytes).map_err(|_| baritone_engine::process::explore::INVALID_JSON.to_string())
}

/// `/b explorefilter <file> [invert]`: `ExploreProcess.applyJsonFilter`.
fn cmd_explore_filter(game: &mut Game<'_>, args: &[&str]) {
    if start_and_feet(game).is_none() {
        return;
    }
    let (name, invert) = match explore_filter_command(args) {
        Ok(r) => r,
        Err(e) => {
            say(game, &e);
            return;
        }
    };
    let path = std::path::Path::new(&name);
    let path = if path.is_absolute() { path.to_path_buf() } else { game.native_data_dir(ID).join(path) };
    let filter = match read_filter_file(&path).and_then(|text| baritone_engine::process::JsonChunkFilter::parse(&text, invert)) {
        Ok(f) => f,
        Err(e) => {
            say(game, &e);
            return;
        }
    };
    let lines = {
        let mut guard = state();
        let Some(m) = guard.as_mut() else {
            return;
        };
        m.pathing.apply_json_filter(filter);
        m.pathing.take_messages()
    };
    for line in &lines {
        say(game, line);
    }
    say(game, &format!("Explore filter applied. Inverted: {invert}"));
}

/// The entities `FollowProcess` can see: the living ones the game simulates and the dropped items, never the player itself. Bounded: past
/// `MAX_FOLLOW_ENTITIES` the rest are not listed (a farm of thousands would otherwise be copied every tick).
const MAX_FOLLOW_ENTITIES: usize = 4096;

fn follow_entities(game: &Game<'_>) -> Vec<EntityInfo> {
    let mut out: Vec<EntityInfo> = Vec::new();
    for m in game.mobs.iter() {
        if out.len() >= MAX_FOLLOW_ENTITIES {
            return out;
        }
        if m.removed {
            continue;
        }
        let kind: &str = &m.kind.id;
        let player_name = if kind == "minecraft:player" {
            // another player the server sent; the player itself has no uuid here, and is never followed
            if game.remote_player_uuid(m.id).is_none() {
                continue;
            }
            Some(game.remote_player_name(m.id).unwrap_or("").to_string())
        } else {
            None
        };
        out.push(EntityInfo { id: m.id, kind: kind.to_string(), player_name, item: None, position: [m.position.x, m.position.y, m.position.z], living: true, alive: m.alive() });
    }
    for e in game.dropped.iter() {
        if out.len() >= MAX_FOLLOW_ENTITIES {
            break;
        }
        out.push(EntityInfo { id: e.id, kind: "minecraft:item".to_string(), player_name: None, item: Some(e.stack.item.to_string()), position: [e.position.x, e.position.y, e.position.z], living: false, alive: true });
    }
    out
}

/// The name the player joined with, read once (the options read a file).
fn own_name(game: &Game<'_>) -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| game.options.player_name()).clone()
}

/// `NearbyPlayer`'s players (`world().players()`): the player itself and the other players the server sent. `positions` looks the others up in the
/// mobs; the completion does not need them.
fn world_players(game: &Game<'_>, positions: bool) -> Vec<PlayerRef> {
    let mut out = Vec::new();
    if let Some(p) = game.player.as_ref() {
        out.push(PlayerRef { id: u32::MAX, name: own_name(game), local: true, position: [p.position.x, p.position.y, p.position.z] });
    }
    for (id, name) in game.remote_player_names() {
        let position = if positions { game.mobs.iter().find(|m| m.id == id).map_or([0.0; 3], |m| [m.position.x, m.position.y, m.position.z]) } else { [0.0; 3] };
        out.push(PlayerRef { id, name, local: false, position });
    }
    out
}

/// The registry ids of the entity types the game knows (`BuiltInRegistries.ENTITY_TYPE`): its mob types, the player and the dropped item.
fn entity_type_ids(game: &Game<'_>) -> Vec<String> {
    let mut ids: Vec<String> = game.res.mob_types().keys().cloned().collect();
    for extra in ["minecraft:player", "minecraft:item"] {
        if !ids.iter().any(|i| i == extra) {
            ids.push(extra.to_string());
        }
    }
    ids.sort();
    ids
}

/// Keeps what `follow` completes up to date: the types once, the players about twice a second.
fn refresh_completion_players(game: &Game<'_>) {
    static TICKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut w = COMPLETION_WORLD.lock().unwrap_or_else(|e| e.into_inner());
    if w.types.is_empty() {
        w.types = entity_type_ids(game);
    }
    if n % 10 == 0 {
        w.players = world_players(game, false);
    }
}

/// `/b goal`: sets the goal without walking. No arguments is the player's feet; `clear` erases it.
fn cmd_goal(game: &mut Game<'_>, args: &[&str]) {
    if matches!(args.first().map(|a| a.to_lowercase()).as_deref(), Some("reset" | "clear" | "none")) && state().is_none() {
        // nothing was ever started: there is no goal to clear
        match goal_command(args, Pos::new(0, 0, 0), None) {
            Ok(action) => say(game, &action.message(censor(game))),
            Err(e) => say(game, &e),
        }
        return;
    }
    let Some(feet) = start_and_feet(game) else {
        return;
    };
    let current = state().as_ref().and_then(|m| m.pathing.goal().cloned());
    match goal_command(args, feet, current.as_ref()) {
        Ok(action) => {
            if let Some(m) = state().as_mut() {
                match &action {
                    GoalAction::Clear { .. } => m.pathing.clear_goal(),
                    GoalAction::Set(goal) => m.pathing.set_goal(goal.clone()),
                }
            }
            say(game, &action.message(censor(game)));
        }
        Err(e) => say(game, &e),
    }
}

/// `/b path`: head for the goal that was set with `/b goal`.
fn cmd_path(game: &mut Game<'_>, args: &[&str]) {
    let has_goal = state().as_ref().is_some_and(|m| m.pathing.goal().is_some());
    let action = match path_command(args, has_goal) {
        Ok(action) => action,
        Err(e) => {
            say(game, &e);
            return;
        }
    };
    if action == PathAction::Path {
        if game.player.is_none() {
            say(game, "Not in a world.");
            return;
        }
        if let Some(m) = state().as_mut() {
            m.dim = game.dim.id();
            m.reported_cancel = None;
            m.target = m.pathing.goal().cloned();
            m.pathing.path();
        }
    }
    say(game, action.message());
}

/// `/b eta`: the time the current segment and the whole way will take, as upstream estimates it.
fn cmd_eta(game: &mut Game<'_>, args: &[&str]) {
    let result = {
        let mut guard = state();
        match (guard.as_mut(), game.player.as_ref()) {
            (Some(m), Some(p)) => {
                let feet = player_view(game, p, &m.tables).feet();
                let gs = m.tables.cfg.goal_settings();
                let segment = m.pathing.ticks_remaining_in_segment(true);
                let goal = m.pathing.estimated_ticks_to_goal(feet, &gs);
                eta_command(args, m.pathing.in_control(), segment, goal)
            }
            _ => eta_command(args, false, None, None),
        }
    };
    match result {
        Ok(lines) => {
            for line in lines.iter().flat_map(|l| l.split('\n')) {
                say(game, line);
            }
        }
        Err(e) => say(game, &e),
    }
}

/// `/b set`: list, show, change, toggle, reset, save and load settings. Every change is saved at once, as upstream does.
fn cmd_set(game: &mut Game<'_>, args: &[&str]) {
    let (outcome, effect_done) = with_settings(game, |st, reg| {
        let outcome = st.run_set(args, reg);
        let mut notes: Vec<String> = Vec::new();
        let mut changed = false;
        match &outcome.effect {
            SetEffect::None => {}
            SetEffect::Changed => {
                changed = true;
                if let Err(e) = save_settings(game, st) {
                    notes.push(e);
                }
            }
            SetEffect::Save => {
                changed = true;
                if let Err(e) = save_settings(game, st) {
                    notes.push(e);
                }
            }
            SetEffect::Load(file) => {
                let name = std::path::Path::new(file).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "settings.txt".to_string());
                match std::fs::read_to_string(settings_file(game, &name)) {
                    Ok(text) => {
                        st.reset_all();
                        notes.extend(st.apply_file_text(&text, reg));
                        changed = true;
                    }
                    Err(e) => notes.push(format!("Cannot read {name}: {e}")),
                }
            }
        }
        if changed {
            SETTINGS_REV.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        (outcome, notes)
    });
    for line in &outcome.lines {
        say(game, line);
    }
    if let Some(e) = &outcome.error {
        say(game, e);
    }
    for note in &effect_done {
        say(game, note);
    }
}

fn cmd_stop(game: &mut Game<'_>) {
    if let Some(m) = state().as_mut() {
        release(m, game.player.as_mut());
    }
    say(game, "ok canceled");
}

// ---------------------------------------------------------------- completion

/// The completion of what is typed after `/b`, for the chat box. Horizon's stable `native_commands` completes only the name; this is called from a
/// pinned hook on the command tree's suggestions (`horizon_command::baritone_rs_suggest`, docs/PLAN.md) with the text up to the cursor.
/// Answers `None` when the text is not ours. `Some((start, list))`: the suggestions replace the text from byte `start` to the cursor.
fn suggest_provider(text: &str, target: Option<[i32; 3]>) -> Option<(usize, Vec<(String, Option<String>)>)> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let body = text.strip_prefix('/')?;
    let (name, after) = body.split_once(' ')?;
    if name != "b" && name != "baritone" {
        return None;
    }
    let current = after.rsplit(' ').next().unwrap_or("");
    let before: Vec<&str> = after[..after.len() - current.len()].split_whitespace().collect();
    let start = text.len() - current.len();
    let mut list: Vec<(String, Option<String>)> = Vec::new();
    let settings = SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
    let defaults;
    let st = match settings.as_ref() {
        Some(s) => s,
        None => {
            // nothing has loaded the file yet: the names and the defaults are the same
            defaults = Settings::new();
            &defaults
        }
    };
    // The block the crosshair is on, as vanilla's `/tp` offers it: the coordinates from this argument on, one, two or three of them.
    if let (Some(t), Some(first)) = (target, before.first()) {
        if matches!(first.to_lowercase().as_str(), "goto" | "goal") {
            let given = before.len() - 1;
            let coords = [t[0].to_string(), t[1].to_string(), t[2].to_string()];
            if given < 3 && before[1..].iter().all(|a| baritone_engine::command::relative_coordinate(a, 0.0).is_some()) {
                for end in given..3 {
                    let text = coords[given..=end].join(" ");
                    if text.starts_with(current) {
                        list.push((text, None));
                    }
                }
            }
        }
    }
    // Block names after `mine` (anywhere, after an optional number) and after `goto` (the one argument): not for a number or a relative coordinate.
    if let (Some(first), Some(index)) = (before.first(), COMPLETION_BLOCKS.get()) {
        let first = first.to_lowercase();
        let wants = first == "mine" || (first == "goto" && before.len() == 1);
        if wants && !current.starts_with(|c: char| c.is_ascii_digit() || c == '~' || c == '-') {
            for name in complete_blocks(current, index) {
                list.push((name, None));
            }
        }
    }
    // The types and the players after `follow`.
    if before.first().is_some_and(|f| f.eq_ignore_ascii_case("follow")) {
        let w = COMPLETION_WORLD.lock().unwrap_or_else(|e| e.into_inner());
        for name in complete_follow(&before[1..], current, &w.players, &w.types) {
            list.push((name, None));
        }
    }
    let descriptions = baritone_engine::command::COMMANDS;
    for word in baritone_engine::command::complete(&before, current, st) {
        let tooltip = if before.is_empty() { descriptions.iter().find(|(names, _)| names.contains(&word.as_str())).map(|(_, d)| d.to_string()) } else { None };
        list.push((word, tooltip));
    }
    // `trace` is the mod's own command, not upstream's, so the engine's list does not have it.
    if before.is_empty() && "trace".starts_with(&current.to_lowercase()) {
        list.push(("trace".to_string(), Some("One log line per tick while a path runs".to_string())));
    }
    log::debug!("{PREFIX} completion of {text:?}: {} suggestions", list.len());
    Some((start, list))
}

/// Mirrors `game.native_on(ID)` for the completion, which has no game at hand.
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static REGISTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Hands the completion to the command tree's pinned hook, once.
fn register_completion(game: &Game<'_>) {
    ENABLED.store(game.native_on(ID), std::sync::atomic::Ordering::Relaxed);
    if !REGISTERED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        completion_blocks(game);
        horizon_client::command::baritone_rs_suggest::register(suggest_provider);
    }
}

// ---------------------------------------------------------------- test-only scenario trigger
//
// With BARITONE_RS_SCENARIO_DIR set to a folder, the mod runs a fixed set of walks by itself once the player is in a single-player
// world, and writes what it saw there: it builds a stone platform through the server's console (the call horizon-ctl's `command`
// makes), teleports the player, and gives commands through `on_command`, the path a typed command takes. It sends no input to
// the desktop. Without the variable, none of this runs.

static ARRIVED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn scenario_dir() -> Option<&'static std::path::PathBuf> {
    static DIR: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| std::env::var_os("BARITONE_RS_SCENARIO_DIR").map(Into::into)).as_ref()
}

enum Act {
    Console(&'static str),
    Command(&'static str),
}

struct Run {
    name: &'static str,
    acts: &'static [(u64, Act)],
    end: u64,
    until_arrived: bool,
    /// The x and z of the block the walk must end in; only for the runs that walk to a goal.
    goal: (i32, i32),
}

const TP_START: &str = "execute in minecraft:overworld run tp @p 1000.5 100 1000.5 0 0";
const GOTO_30: &str = "b goto 1030 100 1000";
const GOTO_AT: u64 = 30;

const RUNS: [Run; 6] = [
    // 30 blocks along +x on flat stone.
    Run { name: "smoke", acts: &[(0, Act::Console(TP_START)), (GOTO_AT, Act::Command(GOTO_30))], end: 600, until_arrived: true, goal: (1030, 1000) },
    // Relative coordinates, and a diagonal.
    Run { name: "diagonal", acts: &[(0, Act::Console(TP_START)), (GOTO_AT, Act::Command("b goto ~20 ~ ~20"))], end: 600, until_arrived: true, goal: (1020, 1020) },
    // Two coordinates: any height will do.
    Run { name: "xz", acts: &[(0, Act::Console(TP_START)), (GOTO_AT, Act::Command("b goto 1030 1010"))], end: 600, until_arrived: true, goal: (1030, 1010) },
    // `/b stop` in the middle of the walk.
    Run { name: "stop", acts: &[(0, Act::Console(TP_START)), (GOTO_AT, Act::Command(GOTO_30)), (60, Act::Command("b stop"))], end: 130, until_arrived: false, goal: (0, 0) },
    // Another dimension while walking, then back.
    Run { name: "dimension", acts: &[(0, Act::Console(TP_START)), (GOTO_AT, Act::Command(GOTO_30)), (50, Act::Console("effect give @p minecraft:resistance 30 4 true")), (60, Act::Console("execute in minecraft:the_nether run tp @p 0 100 0")), (110, Act::Console(TP_START))], end: 150, until_arrived: false, goal: (0, 0) },
    // Death while walking.
    Run { name: "death", acts: &[(0, Act::Console(TP_START)), (GOTO_AT, Act::Command(GOTO_30)), (60, Act::Console("kill @p"))], end: 150, until_arrived: false, goal: (0, 0) },
];

struct Row {
    t: u64,
    pos: [f64; 3],
    vel: [f64; 3],
    yaw: f32,
    on_ground: bool,
    sprinting: bool,
    keys_forced: bool,
    pathing: bool,
    calculating: bool,
    dead: bool,
    dim: &'static str,
}

#[derive(Default)]
struct Scn {
    present: u64,
    filled_at: Option<u64>,
    begun: bool,
    run: usize,
    t: u64,
    arrived_at: Option<u64>,
    rows: Vec<Row>,
}

static SCN: Mutex<Option<Scn>> = Mutex::new(None);

fn console(game: &Game<'_>, line: &str) -> String {
    let answer = game
        .integrated
        .as_ref()
        .and_then(|server| {
            let sent = line.to_string();
            server.call(move |g| g.owner_console_command(&sent))
        })
        .map(|lines| lines.join(" | "))
        .unwrap_or_else(|| "NO SERVER ANSWER".to_string());
    scenario_write("scenario-log.txt", &format!("console: {line} -> {answer}\n"));
    answer
}

fn scenario_write(file: &str, text: &str) {
    use std::io::Write;
    let Some(dir) = scenario_dir() else { return };
    let _ = std::fs::create_dir_all(dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(file)) {
        let _ = f.write_all(text.as_bytes());
    }
}

fn scenario_tick(game: &mut Game<'_>) {
    if scenario_dir().is_none() {
        return;
    }
    // Taken out while the tick runs: the commands below lock the mod's state and call `say`.
    let mut s = SCN.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
    scenario_step(game, &mut s);
    *SCN.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
}

/// Whether the open world is a single-player world made for the tests: its name starts with `l3-`. The test triggers clear the inventory and fill
/// large boxes with blocks, so they refuse any other world, also when their environment variable is set by mistake.
fn in_test_world(game: &Game<'_>) -> bool {
    game.integrated.as_ref().is_some_and(|i| i.world_name.starts_with("l3-"))
}

fn scenario_step(game: &mut Game<'_>, s: &mut Scn) {
    if s.run >= RUNS.len() {
        return;
    }
    if game.player.is_none() || !in_test_world(game) {
        s.present = 0;
        return;
    }
    s.present += 1;
    if !s.begun {
        if s.present == 20 {
            for c in ["difficulty peaceful", "time set day", "gamemode survival", "forceload add 970 970 1060 1060"] {
                console(game, c);
            }
        }
        // A new world has not generated the chunks yet: the fill says so until it has. Try again every second.
        if s.filled_at.is_none() && s.present >= 60 && s.present % 20 == 0 {
            let answer = console(game, "fill 980 99 980 1050 99 1050 stone");
            if !answer.contains("not loaded") && !answer.contains("NO SERVER") {
                // A fill takes at most 32768 blocks: 71 x 71 x 6 each.
                console(game, "fill 980 100 980 1050 105 1050 air");
                console(game, "fill 980 106 980 1050 111 1050 air");
                s.filled_at = Some(s.present);
            }
        }
        if s.filled_at.is_some_and(|f| s.present >= f + 40) {
            console(game, "execute if block 1000 99 1000 minecraft:stone");
            s.begun = true;
            scenario_write("scenario-log.txt", "begin\n");
        }
        return;
    }
    let run = &RUNS[s.run];
    for (at, act) in run.acts {
        if *at != s.t {
            continue;
        }
        match act {
            Act::Console(c) => {
                console(game, c);
            }
            Act::Command(c) => {
                let mut ran = false;
                on_command(game, c, &mut ran);
                scenario_write("scenario-log.txt", &format!("t={} command: {c} ran={ran}\n", s.t));
            }
        }
    }
    for note in std::mem::take(&mut *NOTES.lock().unwrap_or_else(|e| e.into_inner())) {
        scenario_write("scenario-log.txt", &format!("t={} chat: {note}\n", s.t));
    }
    if ARRIVED.swap(false, std::sync::atomic::Ordering::SeqCst) && s.arrived_at.is_none() {
        s.arrived_at = Some(s.t);
    }
    let (pathing, calculating) = state().as_ref().map_or((false, false), |m| (m.pathing.is_pathing(), m.pathing.is_calculating()));
    if let Some(p) = game.player.as_ref() {
        s.rows.push(Row { t: s.t, pos: [p.position.x, p.position.y, p.position.z], vel: [p.velocity.x, p.velocity.y, p.velocity.z], yaw: game.yaw, on_ground: p.ground.on_ground, sprinting: p.sprinting, keys_forced: input().is_some(), pathing, calculating, dead: game.screen == Screen::Dead, dim: game.dim.id() });
    }
    let finished = s.t >= run.end || (run.until_arrived && s.arrived_at.is_some_and(|a| s.t >= a + 10));
    if !finished {
        s.t += 1;
        return;
    }
    let verdict = evaluate(run.name, s);
    scenario_write("scenario-result.txt", &format!("{}: {verdict}\n", run.name));
    let mut csv = String::from("t,x,y,z,vx,vz,yaw,on_ground,sprinting,keys_forced,pathing,calculating,dead,dim\n");
    for r in &s.rows {
        csv += &format!("{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2},{},{},{},{},{},{},{}\n", r.t, r.pos[0], r.pos[1], r.pos[2], r.vel[0], r.vel[2], r.yaw, r.on_ground as u8, r.sprinting as u8, r.keys_forced as u8, r.pathing as u8, r.calculating as u8, r.dead as u8, r.dim);
    }
    scenario_write(&format!("{}.csv", run.name), &csv);
    s.rows.clear();
    s.arrived_at = None;
    s.t = 0;
    s.run += 1;
    if s.run == RUNS.len() {
        scenario_write("scenario-result.txt", "finished\n");
        game.native_chat("baritone-rs scenario finished");
    }
}

fn at(rows: &[Row], t: u64) -> Option<&Row> {
    rows.iter().find(|r| r.t == t)
}

/// How far the player moved on the ground plane in tick `t`, in blocks. The game's velocity field is not the whole movement: the
/// input is added when the player moves.
fn step_len(rows: &[Row], t: u64) -> Option<f64> {
    let (a, b) = (at(rows, t.checked_sub(1)?)?, at(rows, t)?);
    Some(((b.pos[0] - a.pos[0]).powi(2) + (b.pos[2] - a.pos[2]).powi(2)).sqrt())
}

fn evaluate(name: &str, s: &Scn) -> String {
    let rows = &s.rows;
    let last = rows.last();
    match name {
        "smoke" | "diagonal" | "xz" => {
            let goal = RUNS.iter().find(|r| r.name == name).map_or((0, 0), |r| r.goal);
            let Some(a) = s.arrived_at else {
                return format!("FAIL: no arrival within {} ticks, last position {:?}", RUNS.iter().find(|r| r.name == name).map_or(0, |r| r.end), last.map(|r| r.pos));
            };
            let r = last.expect("rows");
            let there = r.pos[0].floor() as i32 == goal.0 && r.pos[2].floor() as i32 == goal.1 && (99.9..100.6).contains(&r.pos[1]);
            let sprint_ticks = rows.iter().filter(|r| r.sprinting).count();
            let top = (GOTO_AT + 2..=a + 10).filter_map(|t| step_len(rows, t)).fold(0.0, f64::max);
            format!("{}: arrived {} ticks after the command, final position {:.2} {:.2} {:.2}, {} ticks sprinting, top speed {:.3} blocks per tick (sprinting is 0.28)", if there { "PASS" } else { "FAIL" }, a - GOTO_AT, r.pos[0], r.pos[1], r.pos[2], sprint_ticks, top)
        }
        "stop" => {
            let before = at(rows, 59).map(|r| (r.keys_forced, step_len(rows, 59).unwrap_or(0.0)));
            let after = at(rows, 62).map(|r| (r.keys_forced, r.pathing));
            let rest = step_len(rows, 125);
            let ok = matches!(before, Some((true, v)) if v > 0.2) && matches!(after, Some((false, false))) && rest.is_some_and(|v| v < 0.01);
            format!("{}: one tick before the stop (keys forced, blocks moved) {before:?}, two ticks after (keys forced, pathing) {after:?}, blocks moved in the last tick {rest:?}", if ok { "PASS" } else { "FAIL" })
        }
        "dimension" => {
            let changed = rows.iter().any(|r| r.dim != "minecraft:overworld");
            let when = rows.iter().find(|r| r.dim != "minecraft:overworld").map(|r| r.t);
            let after = when.and_then(|w| at(rows, w + 2)).map(|r| (r.keys_forced, r.pathing, r.calculating));
            let ok = changed && matches!(after, Some((false, false, false)));
            format!(
                "{}: the dimension changed at {when:?}, two ticks later (keys forced, pathing, calculating) {after:?}",
                if ok {
                    "PASS"
                } else if changed {
                    "FAIL"
                } else {
                    "INCONCLUSIVE, the dimension never changed"
                }
            )
        }
        "death" => {
            let died = rows.iter().find(|r| r.dead).map(|r| r.t);
            let after = died.and_then(|d| at(rows, d + 2)).map(|r| (r.keys_forced, r.pathing, r.calculating));
            let ok = died.is_some() && matches!(after, Some((false, false, false)));
            format!(
                "{}: dead at {died:?}, two ticks later (keys forced, pathing, calculating) {after:?}",
                if ok {
                    "PASS"
                } else if died.is_some() {
                    "FAIL"
                } else {
                    "INCONCLUSIVE, the player did not die"
                }
            )
        }
        _ => "no check".to_string(),
    }
}

// ---------------------------------------------------------------- test-only L3 script player (tools/l3)
//
// With BARITONE_RS_L3_SPEC set to a JSON file, the mod plays the scenarios in it, one after the other, in the single-player world
// that is open: it prepares the world through the single-player server's console (forceload, fill, setblock), teleports the player,
// opens or closes the chat, gives `/b` commands through `on_command` and server commands at fixed ticks, and records one row per
// tick. It judges nothing: `tools/l3/run.py` reads the files and applies the pass criteria. It sends no input to the desktop.
// Without the variable none of this runs.
//
// Spec: {"out": dir, "scenarios": [{"name", "chat", "prep": [console lines], "tp": console line or null, "wait_loaded": [[x, z]],
// "watch": [x0, y0, z0, x1, y1, z1] or null, "events": [{"t", "console" | "command" | "chat": "open" | "close" | "rebase": true, "probe": [x, z]}],
// "end": ticks, "stop_when": "arrived" | "idle" | "never", "slot": hotbar slot at the start or absent, "after": [commands run when it is
// over]}]}; an event {"t", "set": command} is a setup command that does not start the clock.

fn l3_spec() -> Option<&'static serde_json::Value> {
    static SPEC: OnceLock<Option<serde_json::Value>> = OnceLock::new();
    SPEC.get_or_init(|| {
        let path = std::env::var_os("BARITONE_RS_L3_SPEC")?;
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    })
    .as_ref()
}

fn l3_write(file: &str, text: &str) {
    use std::io::Write;
    let Some(dir) = l3_spec().and_then(|s| s["out"].as_str()) else { return };
    let _ = std::fs::create_dir_all(dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::path::Path::new(dir).join(file)) {
        let _ = f.write_all(text.as_bytes());
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum L3Phase {
    #[default]
    Waiting,
    Prep,
    Settle,
    Teleport,
    Run,
    Done,
}

#[derive(Default)]
struct L3 {
    phase: L3Phase,
    /// Ticks the player and the server have been there; the whole run waits for the world.
    present: u64,
    index: usize,
    /// Ticks in this phase.
    phase_t: u64,
    prep_i: usize,
    prep_tries: u32,
    stable: u64,
    t: u64,
    csv: String,
    segments: String,
    searches: String,
    snaps: String,
    notes: String,
    seg_key: Option<(Pos, Pos, usize)>,
    seg_id: i32,
    cmd_t: Option<u64>,
    ever_busy: bool,
    idle_run: u64,
    arrived_at: Option<u64>,
    baseline: Vec<u32>,
    changed_by_server: bool,
    min_health: f32,
    dead_ticks: u64,
    setup_error: Option<String>,
    /// The hotbar and the selected slot when the scenario began.
    hotbar_start: String,
    slot_start: usize,
    /// Ticks in which a container or a station (a crafting table) was open, for the scenarios that right click a block.
    container_ticks: u64,
}

static L3_STATE: Mutex<Option<L3>> = Mutex::new(None);

fn l3_console(game: &Game<'_>, line: &str) -> String {
    let answer = game
        .integrated
        .as_ref()
        .and_then(|server| {
            let sent = line.to_string();
            server.call(move |g| g.owner_console_command(&sent))
        })
        .map(|lines| lines.join(" | "))
        .unwrap_or_else(|| "NO SERVER ANSWER".to_string());
    l3_write("l3-log.txt", &format!("console: {line} -> {answer}\n"));
    answer
}

fn l3_watch(game: &Game<'_>, w: &serde_json::Value) -> Vec<u32> {
    let n = |i: usize| w[i].as_i64().unwrap_or(0) as i32;
    let mut out = Vec::new();
    for x in n(0)..=n(3) {
        for y in n(1)..=n(4) {
            for z in n(2)..=n(5) {
                out.push(game.world.block(x, y, z).0);
            }
        }
    }
    out
}

/// The hotbar as text for the records: `slot:item xCOUNT dDAMAGE` for the filled slots.
fn l3_hotbar_text(game: &Game<'_>) -> String {
    let hb = read_hotbar(game);
    hb.stacks
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.as_ref().map(|st| format!("{i}:{} x{} d{}", st.item.strip_prefix("minecraft:").unwrap_or(&st.item), st.count, st.damage)))
        .collect::<Vec<_>>()
        .join("; ")
}

fn l3_cell_name(game: &Game<'_>, id: u32) -> String {
    let registry: &BlockRegistry = game.pack.assets.registry;
    registry.state(BlockStateId(id)).map_or("?".to_string(), |b| b.name.clone())
}

fn l3_tick(game: &mut Game<'_>) {
    let Some(spec) = l3_spec() else { return };
    let mut s = L3_STATE.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
    l3_step(game, spec, &mut s);
    *L3_STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
}

fn l3_next(s: &mut L3, phase: L3Phase) {
    s.phase = phase;
    s.phase_t = 0;
    s.stable = 0;
}

fn l3_step(game: &mut Game<'_>, spec: &serde_json::Value, s: &mut L3) {
    use std::sync::atomic::Ordering;
    let scenarios = spec["scenarios"].as_array().map_or(&[][..], |v| &v[..]);
    if s.phase == L3Phase::Done {
        return;
    }
    if game.player.is_none() || !in_test_world(game) {
        s.present = 0;
        return;
    }
    s.present += 1;
    s.phase_t += 1;
    let dead = game.screen == Screen::Dead || game.player.as_ref().is_some_and(|p| p.health.dead_or_dying());
    if s.phase != L3Phase::Run && dead && s.phase_t % 20 == 1 {
        let _ = game.respawn_player();
    }
    let Some(sc) = scenarios.get(s.index) else {
        l3_write("l3-result.txt", "finished\n");
        game.native_chat("baritone-rs L3 finished");
        s.phase = L3Phase::Done;
        return;
    };
    match s.phase {
        L3Phase::Waiting => {
            if s.present == 100 {
                for c in ["difficulty peaceful", "gamemode survival", "time set day", "weather clear"] {
                    l3_console(game, c);
                }
                l3_write("l3-log.txt", "begin\n");
                l3_next(s, L3Phase::Prep);
                s.prep_i = 0;
            }
        }
        L3Phase::Prep => {
            let prep = sc["prep"].as_array().map_or(&[][..], |v| &v[..]);
            // One line every 4 ticks. A line the server cannot do yet because the chunks are still being made is tried again.
            if s.phase_t % 4 != 0 {
                return;
            }
            let Some(line) = prep.get(s.prep_i).and_then(|l| l.as_str()) else {
                l3_next(s, L3Phase::Settle);
                return;
            };
            let answer = l3_console(game, line);
            let waiting = answer.contains("not loaded") || answer.contains("NO SERVER") || answer.contains("isn't loaded") || answer.contains("outside of the world");
            if waiting && s.prep_tries < 400 {
                s.prep_tries += 1;
            } else {
                if waiting {
                    s.setup_error = Some(format!("prep line never accepted: {line}"));
                }
                s.prep_i += 1;
                s.prep_tries = 0;
            }
        }
        L3Phase::Settle => {
            if s.phase_t >= 40 && !dead {
                l3_console(game, "time set day");
                l3_next(s, L3Phase::Teleport);
            }
        }
        L3Phase::Teleport => {
            if s.phase_t == 1 {
                if let Some(tp) = sc["tp"].as_str() {
                    l3_console(game, tp);
                }
                return;
            }
            let chunks_ready = sc["wait_loaded"].as_array().map_or(true, |a| a.iter().all(|c| game.world.loaded(c[0].as_i64().unwrap_or(0) as i32, c[1].as_i64().unwrap_or(0) as i32)));
            let calm = game.player.as_ref().is_some_and(|p| p.ground.on_ground && !p.sprinting);
            s.stable = if calm && chunks_ready && !dead && game.screen == Screen::Playing { s.stable + 1 } else { 0 };
            if s.stable >= 30 {
                s.baseline = sc["watch"].as_array().filter(|a| a.len() == 6).map(|_| l3_watch(game, &sc["watch"])).unwrap_or_default();
                s.csv = String::from("t,x,y,z,vx,vy,vz,yaw,on_ground,sprinting,forced,forward,health,dead,chat,pathing,calculating,seg,pos,tick_us,water,mk,mcost,msx,msy,msz,mdx,mdy,mdz\n");
                s.segments.clear();
                s.searches.clear();
                s.snaps.clear();
                s.notes.clear();
                s.seg_key = None;
                s.seg_id = -1;
                s.cmd_t = None;
                s.ever_busy = false;
                s.idle_run = 0;
                s.arrived_at = None;
                s.changed_by_server = false;
                s.min_health = f32::MAX;
                s.dead_ticks = 0;
                s.t = 0;
                SNAPSHOTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
                if let Some(m) = state().as_mut() {
                    let _ = m.pathing.runner_mut().take_log();
                }
                NOTES.lock().unwrap_or_else(|e| e.into_inner()).clear();
                ARRIVED.store(false, Ordering::SeqCst);
                if let Some(slot) = sc["slot"].as_u64() {
                    game.selected = (slot as usize).min(HOTBAR - 1);
                }
                s.slot_start = game.selected;
                s.hotbar_start = l3_hotbar_text(game);
                l3_next(s, L3Phase::Run);
                l3_write("l3-log.txt", &format!("run {} starts\n", sc["name"].as_str().unwrap_or("?")));
            } else if s.phase_t > 900 {
                s.setup_error = Some(format!("the player never stood still on the ground with the chunks loaded (calm {calm}, chunks {chunks_ready}, screen playing {})", game.screen == Screen::Playing));
                l3_finish(game, sc, s);
            }
        }
        L3Phase::Run => l3_run_tick(game, sc, s),
        L3Phase::Done => {}
    }
}

fn l3_run_tick(game: &mut Game<'_>, sc: &serde_json::Value, s: &mut L3) {
    use std::sync::atomic::Ordering;
    let t = s.t;
    for e in sc["events"].as_array().map_or(&[][..], |v| &v[..]) {
        if e["t"].as_u64() != Some(t) {
            continue;
        }
        if let Some(c) = e["console"].as_str() {
            l3_console(game, c);
            s.changed_by_server = true;
        }
        if let Some(c) = e["set"].as_str() {
            // A setup command (a setting): it does not start the clock of the scenario.
            let mut ran = false;
            on_command(game, c, &mut ran);
            s.notes += &format!("{t}\tsetup\t{c}\n");
        }
        if let Some(c) = e["command"].as_str() {
            let mut ran = false;
            on_command(game, c, &mut ran);
            s.cmd_t.get_or_insert(t);
            s.notes += &format!("{t}\tcommand\t{c}\n");
        }
        match e["chat"].as_str() {
            Some("open") => game.open_chat_method(""),
            Some("close") if game.screen == Screen::Chat => game.close_chat(super::chat::ChatExit::Intentional),
            _ => {}
        }
        if let Some(p) = e["probe"].as_array().filter(|p| p.len() == 2) {
            // Whether the game holds the chunk of a block column right now (for the cache scenarios: it must have been unloaded).
            let (x, z) = (p[0].as_i64().unwrap_or(0) as i32, p[1].as_i64().unwrap_or(0) as i32);
            s.notes += &format!("{t}\tprobe\t{x},{z} loaded={}\n", game.world.loaded(x, z));
        }
        if e["rebase"].as_bool() == Some(true) && sc["watch"].as_array().is_some_and(|a| a.len() == 6) {
            s.baseline = l3_watch(game, &sc["watch"]);
        }
    }
    for note in std::mem::take(&mut *NOTES.lock().unwrap_or_else(|e| e.into_inner())) {
        s.notes += &format!("{t}\tchat\t{note}\n");
    }
    if ARRIVED.swap(false, Ordering::SeqCst) && s.arrived_at.is_none() {
        s.arrived_at = Some(t);
    }
    // The path, its movements and the searches, from the mod's state.
    let mut seg = -1;
    let mut pos = -1;
    let mut mov = String::from(",0,0,0,0,0,0,0,0");
    let mut busy = false;
    let (mut pathing, mut calculating) = (false, false);
    if let Some(m) = state().as_mut() {
        busy = m.busy();
        pathing = m.pathing.is_pathing();
        calculating = m.pathing.is_calculating();
        if let Some(c) = m.pathing.current() {
            let positions = c.path().positions();
            let key = (positions.first().copied().unwrap_or(Pos::new(0, 0, 0)), positions.last().copied().unwrap_or(Pos::new(0, 0, 0)), positions.len());
            if s.seg_key != Some(key) {
                s.seg_key = Some(key);
                s.seg_id += 1;
                let list: Vec<String> = c.movements().iter().map(|mv| format!("{:?}:{},{},{}>{},{},{}:{}", mv.kind, mv.src.x, mv.src.y, mv.src.z, mv.dest.x, mv.dest.y, mv.dest.z, mv.cost.map_or("none".to_string(), |v| format!("{v:.3}")))).collect();
                // What the path means to break and to place, from the world as it is now (upstream's `toBreak` and `toPlace`).
                let registry: &BlockRegistry = game.pack.assets.registry;
                let live = Live { world: &game.world, registry, falling: falling_positions(game) };
                let [min_x, min_z, max_x, max_z] = game.border.bounds();
                let ctx = m.tables.ctx(&live, Border { min_x, max_x, min_z, max_z });
                let cells = |v: Vec<Pos>| v.iter().map(|p| format!("{},{},{}", p.x, p.y, p.z)).collect::<Vec<_>>().join(";");
                let brk: Vec<Pos> = c.movements().iter().flat_map(|mv| baritone_engine::exec::movements::to_break(mv, &ctx)).collect();
                let plc: Vec<Pos> = c.movements().iter().flat_map(|mv| baritone_engine::exec::movements::to_place(mv, &ctx)).collect();
                s.segments += &format!("segment {} t={} movements={} estimated_ticks={:.3} to_break={} to_place={} movements_list={}\n", s.seg_id, t, c.movements().len(), c.ticks_remaining_from(0), cells(brk), cells(plc), list.join(";"));
            }
            seg = s.seg_id;
            pos = c.position();
            if let Some(mv) = usize::try_from(pos).ok().and_then(|i| c.movements().get(i)) {
                mov = format!(",{:?},{:.4},{},{},{},{},{},{}", mv.kind, mv.cost.unwrap_or(f64::NAN), mv.src.x, mv.src.y, mv.src.z, mv.dest.x, mv.dest.y, mv.dest.z);
            }
        }
        for e in m.pathing.runner_mut().take_log() {
            s.searches += &format!("t={t} generation={} micros={} kind={:?} cancelled={} positions={}\n", e.generation, e.micros, e.kind, e.cancelled, e.path_positions);
        }
    }
    for (us, chunks) in std::mem::take(&mut *SNAPSHOTS.lock().unwrap_or_else(|e| e.into_inner())) {
        s.snaps += &format!("t={t} micros={us} chunks={chunks}\n");
    }
    if busy {
        s.ever_busy = true;
    }
    if container_is_open(game) {
        s.container_ticks += 1;
    }
    let forced = input().is_some();
    let forward = input().map_or(0.0, |h| h.forward);
    let dead = game.screen == Screen::Dead || game.player.as_ref().is_some_and(|p| p.health.dead_or_dying());
    if let Some(p) = game.player.as_ref() {
        let health = p.health.health();
        s.min_health = s.min_health.min(health);
        s.csv += &format!(
            "{t},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2},{},{},{},{forward},{health:.2},{},{},{},{},{seg},{pos},{},{}{mov}\n",
            p.position.x,
            p.position.y,
            p.position.z,
            p.velocity.x,
            p.velocity.y,
            p.velocity.z,
            game.yaw,
            p.ground.on_ground as u8,
            p.sprinting as u8,
            forced as u8,
            dead as u8,
            (game.screen == Screen::Chat) as u8,
            pathing as u8,
            calculating as u8,
            PATH_TICK_US.load(Ordering::Relaxed),
            p.fluids.was_touching_water as u8
        );
    }
    s.dead_ticks = if dead { s.dead_ticks + 1 } else { 0 };
    let after_cmd = s.cmd_t.is_some_and(|c| t > c + 5);
    s.idle_run = if after_cmd && s.ever_busy && !busy && !forced { s.idle_run + 1 } else { 0 };
    let end = sc["end"].as_u64().unwrap_or(600);
    let finished = t >= end
        || s.dead_ticks >= 20
        || match sc["stop_when"].as_str() {
            Some("arrived") => s.arrived_at.is_some_and(|a| t >= a + 10),
            Some("idle") => s.idle_run >= 60,
            _ => false,
        };
    s.t += 1;
    if finished {
        l3_finish(game, sc, s);
    }
}

fn l3_finish(game: &mut Game<'_>, sc: &serde_json::Value, s: &mut L3) {
    let name = sc["name"].as_str().unwrap_or("unnamed").to_string();
    let changed = if s.baseline.is_empty() || !sc["watch"].as_array().is_some_and(|a| a.len() == 6) {
        None
    } else {
        let now = l3_watch(game, &sc["watch"]);
        Some(now.iter().zip(&s.baseline).filter(|(a, b)| a != b).count())
    };
    // Which cells differ, with the block before and after (up to 600), for the checks of tools/l3 and for finding out what changed them.
    let mut all_changes: Vec<String> = Vec::new();
    if changed.is_some_and(|c| c > 0) {
        let n = |i: usize| sc["watch"][i].as_i64().unwrap_or(0) as i32;
        let mut i = 0;
        for x in n(0)..=n(3) {
            for y in n(1)..=n(4) {
                for z in n(2)..=n(5) {
                    let now = game.world.block(x, y, z).0;
                    if s.baseline.get(i).is_some_and(|b| *b != now) && all_changes.len() < 600 {
                        all_changes.push(format!("{x},{y},{z}: {} -> {}", l3_cell_name(game, s.baseline[i]), l3_cell_name(game, now)));
                    }
                    i += 1;
                }
            }
        }
    }
    let changed_cells: Vec<String> = all_changes.iter().take(8).cloned().collect();
    let slot_end = game.selected;
    let hotbar_end = l3_hotbar_text(game);
    let final_pos = game.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]);
    let meta = serde_json::json!({
        "name": name,
        "chat": sc["chat"],
        "ticks": s.t,
        "command_tick": s.cmd_t,
        "arrived_tick": s.arrived_at,
        "blocks_changed": changed,
        "changed_cells": changed_cells,
        "all_changes": all_changes,
        "slot_start": s.slot_start,
        "slot_end": slot_end,
        "hotbar_start": s.hotbar_start,
        "hotbar_end": hotbar_end,
        "container_open_ticks": s.container_ticks,
        "watch_cells": s.baseline.len(),
        "server_changed_blocks": s.changed_by_server,
        "min_health": if s.min_health == f32::MAX { serde_json::Value::Null } else { serde_json::json!(s.min_health) },
        "died": s.dead_ticks > 0,
        "final_pos": final_pos,
        "setup_error": s.setup_error,
        "busy_at_end": state().as_ref().is_some_and(|m| m.busy()),
        "forced_at_end": input().is_some(),
    });
    l3_write("l3-result.txt", &format!("{meta}\n"));
    l3_write(&format!("{name}.csv"), &s.csv);
    l3_write(&format!("{name}.segments.txt"), &s.segments);
    l3_write(&format!("{name}.searches.txt"), &s.searches);
    l3_write(&format!("{name}.snapshots.txt"), &s.snaps);
    l3_write(&format!("{name}.notes.txt"), &s.notes);
    // Hand everything back for the next scenario: the settings the scenario changed, the chat, the path.
    for c in sc["after"].as_array().map_or(&[][..], |v| &v[..]) {
        if let Some(c) = c.as_str() {
            let mut ran = false;
            on_command(game, c, &mut ran);
        }
    }
    if game.screen == Screen::Chat {
        game.close_chat(super::chat::ChatExit::Intentional);
    }
    if game.screen == Screen::Inventory {
        // A crafting table that the scenario right clicked open: close it as Escape does, so the next scenario can walk.
        game.close_container();
        game.screen = Screen::Playing;
        game.capture(true);
    }
    if state().as_ref().is_some_and(|m| m.busy()) {
        cmd_stop(game);
    }
    s.index += 1;
    s.prep_i = 0;
    s.prep_tries = 0;
    s.setup_error = None;
    s.csv.clear();
    s.container_ticks = 0;
    l3_next(s, L3Phase::Prep);
}
