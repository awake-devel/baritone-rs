// baritone-rs engine: tests of the builder. Written for this project. LGPL-3.0-or-later, see lib.rs.
//
// The rules are checked against hand-worked cases from `BuilderProcess` at 1b8e87c. The game is a stub (`StubFacts`) and the world a small test world.
use super::*;
use crate::blockmeta::StateEntry;
use crate::blocks::{Block, Fluid, State};
use crate::exec::movements::GameFacts;
use crate::exec::state::PlayerView;
use crate::goals::GoalSettings;
use crate::inventory::testing::TestInventory;
use crate::inventory::{select_throwaway_for_location, SlotInfo, Throwaway, ThrowawayHint};
use crate::movement::helper::{Ctx, Precomputed};
use crate::schematic::StaticSchematic;
use crate::settings::ExecSettings;
use crate::testing::{test_config, TestWorld};
use std::cell::RefCell;
use std::collections::HashMap;

const AIR: u16 = 0;
const STONE: u16 = 1;
const PLANKS: u16 = 2;
const WATER: u16 = 3;
const FLOW: u16 = 4;

/// A block table from a list of blocks with their properties: the states are the product of the values, the first of each being the default.
type PropSpec<'a> = (&'a str, &'a [&'a str]);

fn make_states(spec: &[(&str, &[PropSpec<'_>])]) -> BlockStates {
    let mut entries: Vec<StateEntry> = vec![];
    let mut defaults = HashMap::new();
    for (block, props) in spec {
        defaults.insert(block.to_string(), entries.len());
        let mut idx = vec![0usize; props.len()];
        loop {
            entries.push(StateEntry { block: block.to_string(), props: props.iter().zip(&idx).map(|((k, vals), &i)| (k.to_string(), vals[i].to_string())).collect() });
            // the last property changes fastest
            let mut k = props.len();
            loop {
                if k == 0 {
                    break;
                }
                k -= 1;
                idx[k] += 1;
                if idx[k] < props[k].1.len() {
                    break;
                }
                idx[k] = 0;
                if k == 0 {
                    k = usize::MAX;
                    break;
                }
            }
            if props.is_empty() || k == usize::MAX {
                break;
            }
        }
    }
    let is_air = entries.iter().map(|e| e.block == "air").collect();
    BlockStates::new(entries, defaults, is_air, vec![], vec![], 0)
}

const FACING4: &[&str] = &["north", "south", "west", "east"];
const BOOL: &[&str] = &["false", "true"];

/// The table of these tests: ids 0 to 4 are the ones the test world uses (air, stone, planks, water source, flowing water).
fn table() -> Arc<BlockStates> {
    Arc::new(make_states(&[
        ("air", &[]),
        ("stone", &[]),
        ("oak_planks", &[]),
        ("water", &[("level", &["0", "1"])]),
        ("cobblestone", &[]),
        ("oak_log", &[("axis", &["y", "x", "z"])]),
        ("oak_stairs", &[("facing", FACING4), ("half", &["bottom", "top"]), ("waterlogged", BOOL)]),
        ("dispenser", &[("facing", &["north", "east", "south", "west", "up", "down"])]),
        ("vine", &[("north", BOOL), ("up", BOOL)]),
        ("oak_trapdoor", &[("facing", FACING4), ("half", &["bottom", "top"]), ("open", BOOL)]),
        ("tall_grass", &[("half", &["lower", "upper"])]),
    ]))
}

fn sid(bs: &BlockStates, block: &str, props: &[(&str, &str)]) -> StateId {
    (0..bs.len()).find(|&i| bs.block(i) == block && props.iter().all(|(k, v)| bs.prop(i, k) == Some(*v))).unwrap_or_else(|| panic!("no state {block} {props:?}"))
}

fn liquid_flags(bs: &BlockStates) -> Vec<bool> {
    (0..bs.len()).map(|i| bs.block(i) == "water" || bs.block(i) == "lava").collect()
}

// ---------------------------------------------------------------- valid, sameBlockstate, containsBlockState

fn check(bs: &BlockStates, s: &BuildSettings, current: StateId, desired: Option<StateId>, verify: bool) -> bool {
    let liquid = liquid_flags(bs);
    ValidRules { states: bs, liquid: &liquid, settings: s }.valid(current, desired, verify)
}

#[test]
fn valid_with_the_default_settings() {
    let t = table();
    let s = BuildSettings::default();
    let cobble = sid(&t, "cobblestone", &[]);
    // no wish at all; air for air; the same state; a different block
    assert!(check(&t, &s, 1, None, false));
    assert!(check(&t, &s, 0, Some(0), false));
    assert!(check(&t, &s, 1, Some(1), false));
    assert!(!check(&t, &s, 1, Some(cobble), false));
    assert!(!check(&t, &s, 0, Some(PLANKS as usize), false));
    // air where a block is wanted and a block where air is wanted are wrong
    assert!(!check(&t, &s, 1, Some(0), false));
}

#[test]
fn ok_if_water_excuses_any_liquid_block() {
    let t = table();
    let mut s = BuildSettings::default();
    assert!(!check(&t, &s, WATER as usize, Some(PLANKS as usize), false));
    s.ok_if_water = true;
    assert!(check(&t, &s, WATER as usize, Some(PLANKS as usize), false));
    assert!(check(&t, &s, FLOW as usize, Some(PLANKS as usize), true), "even for an item check: it is the first rule");
    assert!(!check(&t, &s, STONE as usize, Some(PLANKS as usize), false));
}

#[test]
fn ok_if_air_excuses_only_air_and_only_for_the_listed_blocks() {
    let t = table();
    let mut s = BuildSettings { ok_if_air: vec!["oak_planks".into()], ..BuildSettings::default() };
    assert!(check(&t, &s, 0, Some(PLANKS as usize), false));
    assert!(check(&t, &s, 0, Some(PLANKS as usize), true));
    assert!(!check(&t, &s, 0, Some(STONE as usize), false), "stone is not on the list");
    assert!(!check(&t, &s, STONE as usize, Some(PLANKS as usize), false), "a block there is not air");
    s.ok_if_air.clear();
    assert!(!check(&t, &s, 0, Some(PLANKS as usize), false));
}

#[test]
fn build_ignore_blocks_excuse_a_block_where_air_is_wanted() {
    let t = table();
    let s = BuildSettings { build_ignore_blocks: vec!["stone".into()], ..BuildSettings::default() };
    assert!(check(&t, &s, STONE as usize, Some(0), false));
    assert!(check(&t, &s, STONE as usize, Some(0), true));
    assert!(!check(&t, &s, PLANKS as usize, Some(0), false));
    assert!(!check(&t, &s, STONE as usize, Some(PLANKS as usize), false), "only where air is wanted");
}

#[test]
fn build_ignore_existing_counts_every_block_but_not_for_items() {
    let t = table();
    let s = BuildSettings { build_ignore_existing: true, ..BuildSettings::default() };
    assert!(check(&t, &s, STONE as usize, Some(PLANKS as usize), false));
    assert!(!check(&t, &s, STONE as usize, Some(PLANKS as usize), true), "an item that would place stone is not planks");
    assert!(!check(&t, &s, 0, Some(PLANKS as usize), false), "air is still missing a block");
}

#[test]
fn build_valid_substitutes_count_as_the_block_but_not_for_items() {
    let t = table();
    let cobble = sid(&t, "cobblestone", &[]);
    let s = BuildSettings { build_valid_substitutes: vec![("stone".into(), vec!["cobblestone".into(), "oak_planks".into()])], ..BuildSettings::default() };
    assert!(check(&t, &s, cobble, Some(STONE as usize), false));
    assert!(check(&t, &s, PLANKS as usize, Some(STONE as usize), false));
    assert!(!check(&t, &s, cobble, Some(STONE as usize), true));
    assert!(!check(&t, &s, STONE as usize, Some(cobble), false), "the map goes one way");
}

#[test]
fn ignore_direction_ignores_the_orientation_properties_by_identity() {
    let t = table();
    let s = BuildSettings { build_ignore_direction: true, ..BuildSettings::default() };
    let none = BuildSettings::default();
    let stairs = |facing: &str, half: &str, wl: &str| sid(&t, "oak_stairs", &[("facing", facing), ("half", half), ("waterlogged", wl)]);
    // stairs: facing and half are ignored, waterlogged is not
    assert!(!check(&t, &none, stairs("north", "bottom", "false"), Some(stairs("east", "bottom", "false")), false));
    assert!(check(&t, &s, stairs("north", "bottom", "false"), Some(stairs("east", "bottom", "false")), false));
    assert!(check(&t, &s, stairs("north", "bottom", "false"), Some(stairs("west", "top", "false")), false));
    assert!(!check(&t, &s, stairs("north", "bottom", "false"), Some(stairs("north", "bottom", "true")), false));
    // a log's axis (x, y, z)
    assert!(check(&t, &s, sid(&t, "oak_log", &[("axis", "x")]), Some(sid(&t, "oak_log", &[("axis", "z")])), false));
    // a dispenser's facing has six values: it is not HORIZONTAL_FACING, so it still counts
    assert!(!check(&t, &s, sid(&t, "dispenser", &[("facing", "north")]), Some(sid(&t, "dispenser", &[("facing", "up")])), false));
    // tall grass `half` takes lower and upper: not the HALF of stairs and trapdoors
    assert!(!check(&t, &s, sid(&t, "tall_grass", &[("half", "lower")]), Some(sid(&t, "tall_grass", &[("half", "upper")])), false));
    // the vine booleans (PipeBlock) and the trapdoor's open and half
    assert!(check(&t, &s, sid(&t, "vine", &[("north", "false"), ("up", "false")]), Some(sid(&t, "vine", &[("north", "true"), ("up", "true")])), false));
    let trap = |open: &str, half: &str| sid(&t, "oak_trapdoor", &[("facing", "north"), ("half", half), ("open", open)]);
    assert!(check(&t, &s, trap("false", "bottom"), Some(trap("true", "top")), false));
    // another block is another block
    assert!(!check(&t, &s, stairs("north", "bottom", "false"), Some(sid(&t, "oak_log", &[("axis", "y")])), false));
}

#[test]
fn ignore_properties_ignores_the_named_ones() {
    let t = table();
    let s = BuildSettings { build_ignore_properties: vec!["waterlogged".into()], ..BuildSettings::default() };
    let stairs = |facing: &str, wl: &str| sid(&t, "oak_stairs", &[("facing", facing), ("half", "bottom"), ("waterlogged", wl)]);
    assert!(check(&t, &s, stairs("north", "false"), Some(stairs("north", "true")), false));
    assert!(!check(&t, &s, stairs("north", "false"), Some(stairs("east", "true")), false), "facing is not on the list");
    // both settings together
    let both = BuildSettings { build_ignore_direction: true, build_ignore_properties: vec!["waterlogged".into()], ..BuildSettings::default() };
    assert!(check(&t, &both, stairs("north", "false"), Some(stairs("east", "true")), false));
}

#[test]
fn contains_block_state_goes_by_same_blockstate() {
    let t = table();
    let liquid = liquid_flags(&t);
    let s = BuildSettings { build_ignore_direction: true, ..BuildSettings::default() };
    let r = ValidRules { states: &t, liquid: &liquid, settings: &s };
    let east = sid(&t, "oak_stairs", &[("facing", "east"), ("half", "bottom"), ("waterlogged", "false")]);
    let north = sid(&t, "oak_stairs", &[("facing", "north"), ("half", "bottom"), ("waterlogged", "false")]);
    assert!(r.contains_block_state(&[0, STONE as usize, east], north));
    assert!(!r.contains_block_state(&[0, STONE as usize], north));
    assert!(!r.contains_block_state(&[], north));
}

// ---------------------------------------------------------------- the harness

#[derive(Default)]
struct StubFacts {
    approx: Vec<StateId>,
    plausible: bool,
    ray: Option<BlockHit>,
    /// The slot that places this state.
    place: Option<(usize, StateId)>,
    calls: RefCell<Vec<(Vec3, Rotation)>>,
    offhand: StateId,
    blocks: Vec<(InvSlot, String)>,
}

impl BuildFacts for StubFacts {
    fn approx_placeable(&self) -> Vec<StateId> {
        let mut v = self.approx.clone();
        v.resize(36, 0);
        v
    }
    fn offhand_placeable(&self) -> StateId {
        self.offhand
    }
    fn item_block(&self, slot: InvSlot) -> Option<String> {
        self.blocks.iter().find(|(s, _)| *s == slot).map(|(_, b)| b.clone())
    }
    fn state_for_placement(&self, hotbar_slot: usize, _hit: &BlockHit, _rot: Rotation) -> Option<StateId> {
        self.place.filter(|(s, _)| *s == hotbar_slot).map(|(_, st)| st)
    }
    fn can_survive(&self, _state: StateId, _pos: Pos) -> bool {
        true
    }
    fn placement_plausible(&self, _state: StateId, _pos: Pos) -> bool {
        self.plausible
    }
    fn outline_bounds(&self, _pos: Pos) -> Option<Aabb> {
        Some(Aabb { min_x: 0.0, min_y: 0.0, min_z: 0.0, max_x: 1.0, max_y: 1.0, max_z: 1.0 })
    }
    fn ray_trace(&self, start: Vec3, rot: Rotation, _reach: f64) -> Option<BlockHit> {
        self.calls.borrow_mut().push((start, rot));
        self.ray
    }
}

struct H {
    states: Vec<State>,
    pre: Precomputed,
    cfg: Config,
    tool: Vec<f64>,
    world: TestWorld,
    bs: Arc<BlockStates>,
    settings: BuildSettings,
    p: PlayerView,
    exec: ExecSettings,
    inv: TestInventory,
    facts: StubFacts,
    liquid: Arc<Vec<bool>>,
}

fn solid(name: &str) -> State {
    State { name: name.into(), shape_full: true, top_sturdy: true, fluid_empty: true, ..State::default() }
}

fn water(amount: i32) -> State {
    State { name: "water".into(), block: Block::Water, liquid_block: true, fluid: Fluid::Water, fluid_empty: false, fluid_amount: amount, fluid_source: amount == 8, fluid_flowing_kind: true, replaceable: true, ..State::default() }
}

impl H {
    /// A 16 by 16 world with a stone floor at y = 0 and the player standing at (5, 1, 5).
    fn new() -> H {
        let cfg = test_config();
        let air = State { name: "air".into(), block: Block::Air, air: true, pathfindable: true, fluid_empty: true, replaceable: true, ..State::default() };
        let states = vec![air, solid("stone"), solid("oak_planks"), water(8), water(7)];
        let pre = Precomputed::new(&states, &cfg);
        let bs = table();
        let liquid = Arc::new(states.iter().map(|s| s.liquid_block).collect());
        H { states, pre, cfg, tool: vec![1.0; 5], world: TestWorld::flat(16, 16), bs, settings: BuildSettings::default(), p: PlayerView::standing(5.5, 1.0, 5.5), exec: ExecSettings::default(), inv: TestInventory::empty(), facts: StubFacts::default(), liquid }
    }

    fn ctx(&self) -> Ctx<'_, TestWorld> {
        Ctx::new(&self.world, &self.states, &self.pre, &self.cfg, Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 }, &self.tool)
    }

    fn env<'a>(&'a self, ctx: &'a Ctx<'a, TestWorld>) -> Env<'a, TestWorld> {
        Env { ctx, p: &self.p, set: &self.exec, path_start: self.p.feet(), inv: &self.inv, game: GameFacts::default(), hint: None }
    }

    fn be(&self) -> BuildEnv<'_> {
        BuildEnv { facts: &self.facts, settings: &self.settings, states: &self.bs }
    }

    fn rules(&self) -> ValidRules<'_> {
        ValidRules { states: &self.bs, liquid: &self.liquid, settings: &self.settings }
    }

    fn builder(&self, schem: StaticSchematic, origin: Pos) -> BuilderProcess {
        let mut bp = BuilderProcess::new();
        bp.build("test".into(), Arc::new(schem), origin, &self.settings, &self.bs);
        bp
    }

    /// The bits of a tick before the decisions: the list of states of the hotbar, the context of the tick.
    fn prepare(&self, bp: &mut BuilderProcess) -> Bcc {
        let be = self.be();
        bp.approx_placeable = BuilderProcess::facts_approx(&be, 36);
        Bcc { view: bp.view(&self.settings, &self.bs).unwrap(), placeable: BuilderProcess::facts_approx(&be, 9) }
    }
}

/// A box of one state.
fn solid_box(w: i32, h: i32, l: i32, state: u16) -> StaticSchematic {
    StaticSchematic::new(w, h, l, vec![usize::from(state); (w * h * l) as usize], 0)
}

/// Runs one tick of the builder on the harness.
fn tick(h: &H, bp: &mut BuilderProcess, safe: bool, controls: &mut Controls) -> Option<PathingCommand> {
    let ctx = h.ctx();
    let env = h.env(&ctx);
    bp.on_tick(false, safe, &env, &h.be(), controls)
}

// ---------------------------------------------------------------- the cost overlay

fn overlay(h: &H, schem: StaticSchematic, origin: Pos, placeable: Vec<StateId>) -> BuilderOverlay {
    let mut bp = h.builder(schem, origin);
    let _ = h.prepare(&mut bp);
    BuilderOverlay { view: bp.view(&h.settings, &h.bs).unwrap(), placeable, settings: h.settings.clone(), liquid: Arc::clone(&h.liquid) }
}

const OPEN: Border = Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 };

#[test]
fn the_overlay_prices_placing_where_the_schematic_wants_something() {
    let h = H::new();
    // a 2 by 1 by 1 schematic at the origin: planks, then air
    let schem = StaticSchematic::new(2, 1, 1, vec![PLANKS as usize, 0], 0);
    let mut o = overlay(&h, schem, Pos::new(0, 1, 0), vec![PLANKS as usize]);
    let mut cfg = test_config();
    cfg.has_throwaway = false;
    // a state the hotbar places is free, even with nothing to throw away (U39)
    assert_eq!(o.cost_of_placing_at(&cfg, &OPEN, 0, 1, 0, 0), 0.0);
    // where it wants air: place_penalty * placeIncorrectBlockPenaltyMultiplier, with or without a throwaway
    assert_eq!(o.cost_of_placing_at(&cfg, &OPEN, 1, 1, 0, 0), 20.0 * 2.0);
    // a state we do not have: infinite without a throwaway, else 1.5 times as much
    o.placeable.clear();
    assert_eq!(o.cost_of_placing_at(&cfg, &OPEN, 0, 1, 0, 0), COST_INF);
    cfg.has_throwaway = true;
    assert_eq!(o.cost_of_placing_at(&cfg, &OPEN, 0, 1, 0, 0), 20.0 * 1.5 * 2.0);
    // outside the schematic: the plain price with a throwaway, else infinite
    assert_eq!(o.cost_of_placing_at(&cfg, &OPEN, 9, 1, 9, 0), 20.0);
    cfg.has_throwaway = false;
    assert_eq!(o.cost_of_placing_at(&cfg, &OPEN, 9, 1, 9, 0), COST_INF);
    // the border comes first
    cfg.has_throwaway = true;
    let tight = Border { min_x: 0.0, max_x: 10.0, min_z: -5.0, max_z: 10.0 };
    assert_eq!(o.cost_of_placing_at(&cfg, &tight, 0, 1, 0, 0), COST_INF);
    assert_eq!(o.cost_of_placing_at(&cfg, &tight, 1, 1, 0, 0), 20.0 * 2.0);
}

#[test]
fn the_overlay_uses_the_settings_multipliers() {
    let mut h = H::new();
    h.settings.place_incorrect_block_penalty_multiplier = 5.0;
    h.settings.break_correct_block_penalty_multiplier = 7.0;
    let schem = StaticSchematic::new(2, 1, 1, vec![PLANKS as usize, 0], 0);
    let o = overlay(&h, schem, Pos::new(0, 1, 0), vec![]);
    let mut cfg = test_config();
    cfg.has_throwaway = true;
    assert_eq!(o.cost_of_placing_at(&cfg, &OPEN, 1, 1, 0, 0), 20.0 * 5.0);
    assert_eq!(o.cost_of_placing_at(&cfg, &OPEN, 0, 1, 0, 0), 20.0 * 1.5 * 5.0);
    cfg.allow_break = true;
    assert_eq!(o.break_cost_multiplier_at(&cfg, 0, 1, 0, PLANKS as usize), 7.0);
}

#[test]
fn the_overlay_prices_breaking() {
    let h = H::new();
    // planks wanted at x 0, air at x 1
    let schem = StaticSchematic::new(2, 1, 1, vec![PLANKS as usize, 0], 0);
    let o = overlay(&h, schem, Pos::new(0, 1, 0), vec![]);
    let mut cfg = test_config();
    cfg.allow_break = true;
    // the right block is dear to break; a wrong one costs as it would; where air is wanted anything breaks at the plain price
    assert_eq!(o.break_cost_multiplier_at(&cfg, 0, 1, 0, PLANKS as usize), 10.0);
    assert_eq!(o.break_cost_multiplier_at(&cfg, 0, 1, 0, STONE as usize), 1.0);
    assert_eq!(o.break_cost_multiplier_at(&cfg, 1, 1, 0, STONE as usize), 1.0);
    // outside the schematic
    assert_eq!(o.break_cost_multiplier_at(&cfg, 7, 1, 7, STONE as usize), 1.0);
    // with breaking off only `allowBreakAnyway` may break
    cfg.allow_break = false;
    assert_eq!(o.break_cost_multiplier_at(&cfg, 0, 1, 0, STONE as usize), COST_INF);
    cfg.allow_break_anyway = vec!["stone".into()];
    assert_eq!(o.break_cost_multiplier_at(&cfg, 0, 1, 0, STONE as usize), 1.0);
    assert_eq!(o.break_cost_multiplier_at(&cfg, 0, 1, 0, PLANKS as usize), COST_INF);
}

#[test]
fn without_an_overlay_the_costs_are_the_plain_ones() {
    let mut h = H::new();
    h.cfg.has_throwaway = true;
    h.cfg.allow_break = true;
    let ctx = h.ctx();
    let air = ctx.get(3, 3, 3);
    assert_eq!(ctx.cost_of_placing_at(3, 3, 3, air), h.cfg.place_penalty);
    let stone = ctx.get(3, 0, 3);
    assert_eq!(ctx.mining_duration(3, 0, 3, stone, false), 1.0 / 1.0 + h.cfg.break_penalty);
    let mut cfg2 = h.cfg.clone();
    cfg2.has_throwaway = false;
    let h2 = H { cfg: cfg2, ..H::new() };
    assert_eq!(h2.ctx().cost_of_placing_at(3, 3, 3, h2.ctx().get(3, 3, 3)), COST_INF);
}

#[test]
fn an_overlay_in_a_context_replaces_both_costs() {
    let mut h = H::new();
    h.cfg.allow_break = true;
    let schem = StaticSchematic::new(1, 1, 1, vec![PLANKS as usize], 0);
    let o = overlay(&h, schem, Pos::new(3, 1, 3), vec![PLANKS as usize]);
    let plain = h.ctx();
    let with = h.ctx().with_costs(&o);
    // planks are free to place where the schematic wants them
    assert_eq!(with.cost_of_placing_at(3, 1, 3, plain.get(3, 1, 3)), 0.0);
    // breaking the stone at the schematic's position: the wrong block costs the plain multiplier; one block lower is outside: also 1
    assert_eq!(with.mining_duration(3, 0, 3, with.get(3, 0, 3), false), plain.mining_duration(3, 0, 3, plain.get(3, 0, 3), false));
}

// ---------------------------------------------------------------- the goals

fn gs() -> GoalSettings {
    test_config().goal_settings()
}

#[test]
fn break_is_a_get_to_block_that_is_never_met_from_above() {
    let g = Goal::Break { x: 0, y: 5, z: 0 };
    let s = gs();
    // on the block's own column: at the block, one below (the head is next to it), two below (the head is under it): in
    assert!(g.is_in_goal(0, 5, 0, &s));
    assert!(g.is_in_goal(0, 4, 0, &s));
    assert!(g.is_in_goal(0, 3, 0, &s));
    assert!(!g.is_in_goal(0, 2, 0, &s));
    // beside it at its height, and below it
    assert!(g.is_in_goal(1, 5, 0, &s));
    assert!(g.is_in_goal(0, 5, -1, &s));
    assert!(g.is_in_goal(1, 4, 0, &s));
    assert!(!g.is_in_goal(1, 3, 0, &s));
    assert!(!g.is_in_goal(1, 5, 1, &s));
    // never above the block
    assert!(!g.is_in_goal(0, 6, 0, &s));
    assert!(!g.is_in_goal(1, 6, 0, &s));
    // the heuristic is get-to-block's
    let gb = Goal::GetToBlock { x: 0, y: 5, z: 0 };
    for (x, y, z) in [(0, 0, 0), (3, 9, -2), (0, 6, 0)] {
        assert_eq!(g.heuristic(x, y, z, &s), gb.heuristic(x, y, z, &s));
    }
    assert_eq!(g.heuristic_inside(&s), 0.0);
}

#[test]
fn break_from_above_is_the_cell_over_it_and_never_in_it() {
    let g = Goal::BreakFromAbove { x: 0, y: 6, z: 0 };
    let s = gs();
    assert!(!g.is_in_goal(0, 6, 0, &s), "not the cell itself");
    assert!(!g.is_in_goal(0, 7, 0, &s), "not above it");
    assert!(g.is_in_goal(1, 6, 0, &s));
    assert!(g.is_in_goal(0, 5, 0, &s), "the block to break itself: the player would stand in it");
    assert!(g.is_in_goal(1, 5, 0, &s));
    assert!(!g.is_in_goal(2, 6, 0, &s));
}

#[test]
fn adjacent_keeps_off_the_block_the_no_cell_and_too_low() {
    let s = gs();
    let at = Pos::new(0, 0, 0);
    let g = Goal::Adjacent { x: 0, y: 5, z: 0, no: Pos::new(0, 4, 0), allow_same_level: false };
    assert!(!g.is_in_goal(0, 5, 0, &s), "not the block");
    assert!(!g.is_in_goal(0, 4, 0, &s), "not the no cell");
    assert!(g.is_in_goal(1, 5, 0, &s));
    // one below the block's level only with allow_same_level
    assert!(!g.is_in_goal(1, 4, 0, &s));
    let g2 = Goal::Adjacent { x: 0, y: 5, z: 0, no: Pos::new(0, 4, 0), allow_same_level: true };
    assert!(g2.is_in_goal(1, 4, 0, &s));
    // further down is never in
    assert!(!g2.is_in_goal(1, 3, 0, &s));
    // above is in (get-to-block: one step)
    assert!(g2.is_in_goal(0, 6, 0, &s));
    // the heuristic prefers low blocks: y * 100 plus get-to-block's, in int arithmetic
    let gb = Goal::GetToBlock { x: 0, y: 5, z: 0 };
    assert_eq!(g.heuristic(3, 2, 1, &s), 500.0 + gb.heuristic(3, 2, 1, &s));
    let high = Goal::Adjacent { x: 0, y: i32::MAX / 50, z: 0, no: at, allow_same_level: false };
    assert_eq!(high.heuristic(0, 0, 0, &s), f64::from((i32::MAX / 50).wrapping_mul(100)) + Goal::GetToBlock { x: 0, y: i32::MAX / 50, z: 0 }.heuristic(0, 0, 0, &s));
}

#[test]
fn place_is_a_block_goal_over_the_cell_with_a_low_preference() {
    let s = gs();
    let g = goal_place(Pos::new(2, 3, 4));
    assert_eq!(g, Goal::Place { x: 2, y: 4, z: 4 });
    assert!(g.is_in_goal(2, 4, 4, &s));
    assert!(!g.is_in_goal(2, 3, 4, &s));
    let gb = Goal::Block { x: 2, y: 4, z: 4 };
    assert_eq!(g.heuristic(0, 0, 0, &s), 400.0 + gb.heuristic(0, 0, 0, &s));
    assert_ne!(g, gb, "each class is its own");
}

#[test]
fn janky_is_in_when_either_is_and_leans_on_the_primary() {
    let s = gs();
    let g = Goal::Janky { primary: Box::new(Goal::Block { x: 0, y: 1, z: 0 }), fallback: Box::new(Goal::Block { x: 5, y: 1, z: 5 }) };
    assert!(g.is_in_goal(0, 1, 0, &s));
    assert!(g.is_in_goal(5, 1, 5, &s));
    assert!(!g.is_in_goal(1, 1, 1, &s));
    // the heuristic is the primary's alone, even next to the fallback
    let primary = Goal::Block { x: 0, y: 1, z: 0 };
    assert_eq!(g.heuristic(5, 1, 5, &s), primary.heuristic(5, 1, 5, &s));
    assert_eq!(g.heuristic_inside(&s), 0.0);
    // equality is by the two parts
    let same = Goal::Janky { primary: Box::new(Goal::Block { x: 0, y: 1, z: 0 }), fallback: Box::new(Goal::Block { x: 5, y: 1, z: 5 }) };
    let swapped = Goal::Janky { primary: Box::new(Goal::Block { x: 5, y: 1, z: 5 }), fallback: Box::new(Goal::Block { x: 0, y: 1, z: 0 }) };
    assert_eq!(g, same);
    assert_ne!(g, swapped);
}

#[test]
fn the_new_goals_print_as_upstream_prints_them() {
    use crate::command::goal_to_string;
    assert_eq!(goal_to_string(&Goal::Break { x: 1, y: 2, z: 3 }, false), "GoalBreak{x=1,y=2,z=3}");
    assert_eq!(goal_to_string(&Goal::Adjacent { x: 1, y: 2, z: 3, no: Pos::new(0, 0, 0), allow_same_level: true }, false), "GoalAdjacent{x=1,y=2,z=3}");
    assert_eq!(goal_to_string(&Goal::Place { x: 1, y: 2, z: 3 }, true), "GoalPlace{x=<censored>,y=<censored>,z=<censored>}");
    let j = Goal::Janky { primary: Box::new(Goal::Break { x: 1, y: 2, z: 3 }), fallback: Box::new(Goal::BreakFromAbove { x: 1, y: 3, z: 3 }) };
    assert_eq!(goal_to_string(&j, false), "JankyComposite Primary: GoalBreak{x=1,y=2,z=3} Fallback: GoalGetToBlock{x=1,y=3,z=3}");
}

// ---------------------------------------------------------------- recalc

#[test]
fn full_recalc_stops_past_incorrect_size() {
    let mut h = H::new();
    h.settings.incorrect_size = 5;
    // a 4 by 1 by 4 floor of planks where there is air: 16 wrong cells
    let mut bp = h.builder(solid_box(4, 1, 4, PLANKS), Pos::new(2, 1, 2));
    let bcc = h.prepare(&mut bp);
    let ctx = h.ctx();
    let env = h.env(&ctx);
    bp.full_recalc(&env, &bcc, &h.rules(), &h.settings);
    // it stops when the size is more than 5, so at 6
    assert_eq!(bp.incorrect_positions().len(), 6);
    let big = BuildSettings { incorrect_size: 100, ..BuildSettings::default() };
    bp.full_recalc(&env, &bcc, &h.rules(), &big);
    assert_eq!(bp.incorrect_positions().len(), 16);
}

#[test]
fn full_recalc_remembers_what_it_saw_right_when_the_chunk_unloads() {
    let mut h = H::new();
    // two planks cells: x = 2 is placed in the world, x = 8 is not
    h.world.set(2, 1, 2, PLANKS);
    let mut bp = h.builder(StaticSchematic::new(7, 1, 1, vec![usize::from(PLANKS); 7], 0), Pos::new(2, 1, 2));
    let bcc = h.prepare(&mut bp);
    {
        let ctx = h.ctx();
        let env = h.env(&ctx);
        bp.full_recalc(&env, &bcc, &h.rules(), &h.settings);
    }
    // the cell at x = 2 is seen right, the other six are wrong
    assert_eq!(bp.incorrect_positions().len(), 6);
    assert!(bp.observed_completed.as_ref().unwrap().contains(&Pos::new(2, 1, 2)));
    // now everything with x >= 2 is out of render distance
    h.world.loaded_below_x = 2;
    let ctx = h.ctx();
    let env = h.env(&ctx);
    bp.full_recalc(&env, &bcc, &h.rules(), &h.settings);
    let wrong = bp.incorrect_positions();
    assert_eq!(wrong.len(), 6, "the unseen cells stay wrong; the seen one is not counted");
    assert!(!wrong.contains(&Pos::new(2, 1, 2)));
    // a fresh process has seen nothing: all seven are wrong
    let mut fresh = h.builder(StaticSchematic::new(7, 1, 1, vec![usize::from(PLANKS); 7], 0), Pos::new(2, 1, 2));
    fresh.full_recalc(&env, &bcc, &h.rules(), &h.settings);
    assert_eq!(fresh.incorrect_positions().len(), 7);
}

#[test]
fn recalc_nearby_moves_cells_between_wrong_and_observed() {
    let mut h = H::new();
    h.settings.builder_tick_scan_radius = 1;
    // a 6 by 1 by 1 row of planks from x = 3; the player is at x = 5, so x 4..=6 are scanned
    let mut bp = h.builder(StaticSchematic::new(6, 1, 1, vec![usize::from(PLANKS); 6], 0), Pos::new(3, 1, 5));
    let bcc = h.prepare(&mut bp);
    bp.incorrect_positions = Some((3..9).map(|x| (x, 1, 5)).collect());
    h.world.set(5, 1, 5, PLANKS);
    let ctx = h.ctx();
    let env = h.env(&ctx);
    bp.recalc_nearby(&env, &bcc, &h.rules(), &h.settings);
    let wrong = bp.incorrect_positions();
    assert!(!wrong.contains(&Pos::new(5, 1, 5)), "a correct cell leaves the list");
    assert!(bp.observed_completed.as_ref().unwrap().contains(&Pos::new(5, 1, 5)));
    assert_eq!(wrong.len(), 5);
    // a cell that was wrong and was not on the list is added, and loses its observed mark
    bp.incorrect_positions = Some(BTreeSet::new());
    h.world.set(5, 1, 5, AIR);
    let ctx = h.ctx();
    let env = h.env(&ctx);
    bp.recalc_nearby(&env, &bcc, &h.rules(), &h.settings);
    let wrong = bp.incorrect_positions();
    assert_eq!(wrong, vec![Pos::new(4, 1, 5), Pos::new(5, 1, 5), Pos::new(6, 1, 5)]);
    assert!(!bp.observed_completed.as_ref().unwrap().contains(&Pos::new(5, 1, 5)));
}

#[test]
fn trim_drops_the_far_ones_unless_none_would_be_left() {
    let h = H::new();
    let mut bp = h.builder(solid_box(1, 1, 1, PLANKS), Pos::new(0, 1, 0));
    // the player's block position is (5, 1, 5); 14 squared is 196 (kept) and 15 squared is 225 (dropped)
    bp.incorrect_positions = Some([(5, 1, 5), (5 + 14, 1, 5), (5, 1, 5 + 15)].into_iter().collect());
    let ctx = h.ctx();
    let env = h.env(&ctx);
    bp.trim(&env);
    assert_eq!(bp.incorrect_positions(), vec![Pos::new(5, 1, 5), Pos::new(19, 1, 5)]);
    // 201 is more than 200: (10, 10, 1) is 100 + 81 + 16 = 197; (11, 9, 1) is 36 + 64... use a plain far set
    bp.incorrect_positions = Some([(5 + 15, 1, 5), (5, 1, 5 + 16)].into_iter().collect());
    bp.trim(&env);
    assert_eq!(bp.incorrect_positions().len(), 2, "all far: nothing is dropped");
}

// ---------------------------------------------------------------- assemble

fn assemble(h: &H, bp: &mut BuilderProcess, approx: &[StateId], log: bool) -> Option<Goal> {
    let bcc = h.prepare(bp);
    let ctx = h.ctx();
    let env = h.env(&ctx);
    bp.recalc(&env, &bcc, &h.rules(), &h.settings);
    bp.assemble(&env, &h.be(), &bcc, &h.rules(), approx, log)
}

#[test]
fn assemble_makes_the_primary_of_what_can_be_placed_and_the_fallback_of_what_must_go() {
    let mut h = H::new();
    h.facts.plausible = true;
    // a stone block where air is wanted (x = 8), and planks wanted at x = 9 on the floor
    h.world.set(8, 1, 8, STONE);
    let schem = StaticSchematic::new(2, 1, 1, vec![0, usize::from(PLANKS)], 0);
    let mut bp = h.builder(schem, Pos::new(8, 1, 8));
    let goal = assemble(&h, &mut bp, &[PLANKS as usize], false).expect("a goal");
    match goal {
        Goal::Janky { primary, fallback } => {
            // the planks: one place goal, next to the floor under it (down is the first that can be placed against: the others are air)
            let Goal::Composite(places) = *primary else { panic!("primary is a composite") };
            // the west neighbour (the stone block at x = 8) comes before down in the order north, south, east, west, down
            assert_eq!(places, vec![Goal::Adjacent { x: 9, y: 1, z: 8, no: Pos::new(8, 1, 8), allow_same_level: false }]);
            let Goal::Composite(breaks) = *fallback else { panic!("fallback is a composite") };
            assert_eq!(breaks, vec![Goal::Break { x: 8, y: 1, z: 8 }]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn assemble_with_only_breaking_to_do_is_a_plain_composite() {
    let mut h = H::new();
    h.world.set(8, 1, 8, STONE);
    let mut bp = h.builder(solid_box(1, 1, 1, 0), Pos::new(8, 1, 8));
    assert_eq!(assemble(&h, &mut bp, &[], false), Some(Goal::Composite(vec![Goal::Break { x: 8, y: 1, z: 8 }])));
}

#[test]
fn assemble_with_break_from_above_makes_the_janky_pair() {
    let mut h = H::new();
    h.settings.goal_break_from_above = true;
    h.world.set(8, 1, 8, STONE);
    let mut bp = h.builder(solid_box(1, 1, 1, 0), Pos::new(8, 1, 8));
    let goal = assemble(&h, &mut bp, &[], false).unwrap();
    assert_eq!(goal, Goal::Composite(vec![Goal::Janky { primary: Box::new(Goal::Break { x: 8, y: 1, z: 8 }), fallback: Box::new(Goal::BreakFromAbove { x: 8, y: 2, z: 8 }) }]));
    // with something above, the plain one
    h.world.set(8, 2, 8, STONE);
    let mut bp = h.builder(solid_box(1, 2, 1, 0), Pos::new(8, 1, 8));
    let Some(Goal::Composite(g)) = assemble(&h, &mut bp, &[], false) else { panic!() };
    assert_eq!(g[0], Goal::Break { x: 8, y: 1, z: 8 }, "the lower block has another one above it");
    assert!(matches!(g[1], Goal::Janky { .. }), "the upper one has air over it");
}

#[test]
fn assemble_skips_a_place_with_a_place_below_it() {
    let mut h = H::new();
    h.facts.plausible = true;
    // a column of three planks wanted on the floor: only the lowest has no placeable below it
    let schem = solid_box(1, 3, 1, PLANKS);
    let mut bp = h.builder(schem, Pos::new(8, 1, 8));
    let Some(Goal::Janky { primary, .. }) = assemble(&h, &mut bp, &[PLANKS as usize], false) else { panic!() };
    let Goal::Composite(places) = *primary else { panic!() };
    assert_eq!(places.len(), 1);
    assert!(matches!(places[0], Goal::Adjacent { y: 1, .. }), "{:?}", places[0]);
    // and a gap in the column (the cell two below is wrong but not placeable) lets the next one in: not tested further here
}

#[test]
fn assemble_with_no_plausible_face_stands_at_the_block() {
    let mut h = H::new();
    h.facts.plausible = false;
    let mut bp = h.builder(solid_box(1, 1, 1, PLANKS), Pos::new(8, 1, 8));
    let Some(Goal::Janky { primary, .. }) = assemble(&h, &mut bp, &[PLANKS as usize], false) else { panic!() };
    assert_eq!(*primary, Goal::Composite(vec![Goal::Place { x: 8, y: 2, z: 8 }]));
}

#[test]
fn assemble_says_what_is_missing_once_in_the_order_it_found_it() {
    let h = H::new();
    // three planks and two stone wanted, nothing in hand
    let schem = StaticSchematic::new(5, 1, 1, vec![PLANKS as usize, PLANKS as usize, STONE as usize, PLANKS as usize, STONE as usize], 0);
    let mut bp = h.builder(schem, Pos::new(8, 1, 8));
    assert_eq!(assemble(&h, &mut bp, &[], false), None);
    assert!(bp.take_messages().is_empty(), "silent unless asked");
    assert_eq!(assemble(&h, &mut bp, &[], true), None);
    assert_eq!(bp.take_messages(), vec!["Missing materials for at least:".to_string(), "3x Block{minecraft:oak_planks}\n2x Block{minecraft:stone}".to_string()]);
}

#[test]
fn assemble_replaces_source_liquid_and_reports_flowing_liquid() {
    let mut h = H::new();
    h.world.set(8, 1, 8, WATER);
    h.world.set(9, 1, 8, FLOW);
    let schem = StaticSchematic::new(2, 1, 1, vec![PLANKS as usize, PLANKS as usize], 0);
    let mut bp = h.builder(schem, Pos::new(8, 1, 8));
    // a source block is a place to stand over; the flowing one is left
    let goal = assemble(&h, &mut bp, &[PLANKS as usize], false).unwrap();
    let Goal::Janky { primary, .. } = goal else { panic!() };
    assert_eq!(*primary, Goal::Composite(vec![Goal::Block { x: 8, y: 2, z: 8 }]));
    // with only the flowing one: nothing to do, and with logging it says where
    let mut bp = h.builder(StaticSchematic::new(1, 1, 1, vec![PLANKS as usize], 0), Pos::new(9, 1, 8));
    assert_eq!(assemble(&h, &mut bp, &[PLANKS as usize], true), None);
    assert_eq!(bp.take_messages(), vec!["Unreplaceable liquids at at least:".to_string(), "9 1 8".to_string()]);
}

// ---------------------------------------------------------------- possibleToPlace

#[test]
fn possible_to_place_tries_the_faces_in_direction_order_with_the_multipliers() {
    let mut h = H::new();
    // the target (8, 1, 8): stone below it (the floor) and stone to the west; air elsewhere
    h.world.set(7, 1, 8, STONE);
    h.facts.plausible = true;
    h.facts.ray = None;
    let bp = h.builder(solid_box(1, 1, 1, PLANKS), Pos::new(8, 1, 8));
    let ctx = h.ctx();
    let env = h.env(&ctx);
    let found = bp.possible_to_place(PLANKS as usize, 8, 1, 8, &env, &h.be(), &h.rules());
    assert!(found.is_none());
    let calls = h.facts.calls.borrow();
    // DOWN first with five aims, then WEST with two (UP, NORTH, SOUTH are air, EAST too)
    assert_eq!(calls.len(), 7);
    let eye = infer_sneaking_eye_position(h.p.x, h.p.y, h.p.z);
    let aim = |x: f64, y: f64, z: f64| env.game.aim.peek_rotation(h.p.rotations(), calc_rotation_from_vec3d(eye, Vec3::new(x, y, z), h.p.rotations()));
    // the first aim is the top face of the floor block below, in the middle. The other four are upstream's multipliers 0.1 and 0.9 put through
    // `min * m + max * (1 - m)`, which turns them round: 0.1 aims at 0.9 of the way along
    let down = [(8.5, 1.0, 8.5), (8.9, 1.0, 8.5), (8.1, 1.0, 8.5), (8.5, 1.0, 8.9), (8.5, 1.0, 8.1)];
    for (i, (x, y, z)) in down.iter().enumerate() {
        assert_eq!(calls[i].1, aim(*x, *y, *z), "aim {i}");
        assert_eq!(calls[i].0, eye);
    }
    // west: the east face of the block at x = 7; the multipliers 0.25 and 0.75 aim at three quarters and then a quarter of the way up
    assert_eq!(calls[5].1, aim(8.0, 1.75, 8.5));
    assert_eq!(calls[6].1, aim(8.0, 1.25, 8.5));
}

#[test]
fn possible_to_place_needs_the_hit_block_and_face_and_an_item_that_places_it() {
    let mut h = H::new();
    h.facts.plausible = true;
    let floor = Pos::new(8, 0, 8);
    let good_hit = BlockHit { pos: floor, face: Direction::Up, point: Vec3::new(8.5, 1.0, 8.5) };
    let bp = h.builder(solid_box(1, 1, 1, PLANKS), Pos::new(8, 1, 8));
    // a hit on the right block, but its face is the side
    h.facts.ray = Some(BlockHit { face: Direction::North, ..good_hit });
    h.facts.place = Some((4, PLANKS as usize));
    {
        let ctx = h.ctx();
        let env = h.env(&ctx);
        assert!(bp.possible_to_place(PLANKS as usize, 8, 1, 8, &env, &h.be(), &h.rules()).is_none());
    }
    // the right block and face, but no slot that places planks
    h.facts.ray = Some(good_hit);
    h.facts.place = None;
    {
        let ctx = h.ctx();
        let env = h.env(&ctx);
        assert!(bp.possible_to_place(PLANKS as usize, 8, 1, 8, &env, &h.be(), &h.rules()).is_none());
    }
    // a slot that places stone is not valid for planks (item check)
    h.facts.place = Some((4, STONE as usize));
    {
        let ctx = h.ctx();
        let env = h.env(&ctx);
        assert!(bp.possible_to_place(PLANKS as usize, 8, 1, 8, &env, &h.be(), &h.rules()).is_none());
    }
    // and the slot that places planks: slot 4
    h.facts.place = Some((4, PLANKS as usize));
    let ctx = h.ctx();
    let env = h.env(&ctx);
    let p = bp.possible_to_place(PLANKS as usize, 8, 1, 8, &env, &h.be(), &h.rules()).expect("a placement");
    assert_eq!((p.hotbar_selection, p.place_against, p.side), (4, floor, Direction::Up));
    // with an item check that is lenient about nothing: the lowest slot that qualifies wins
    // and a position that cannot be placed at all (not plausible) answers nothing before any aim is tried
    let mut h2 = H::new();
    h2.facts.plausible = false;
    h2.facts.ray = Some(good_hit);
    h2.facts.place = Some((4, PLANKS as usize));
    let ctx2 = h2.ctx();
    let env2 = h2.env(&ctx2);
    assert!(bp.possible_to_place(PLANKS as usize, 8, 1, 8, &env2, &h2.be(), &h2.rules()).is_none());
    assert!(h2.facts.calls.borrow().is_empty());
}

#[test]
fn the_side_multipliers_are_upstreams() {
    assert_eq!(aabb_side_multipliers(Direction::Up).len(), 5);
    assert_eq!(aabb_side_multipliers(Direction::Down)[1], Vec3::new(0.1, 0.0, 0.5));
    // east: x = (1 + 1) / 2 = 1, z = 0.5
    assert_eq!(aabb_side_multipliers(Direction::East), vec![Vec3::new(1.0, 0.25, 0.5), Vec3::new(1.0, 0.75, 0.5)]);
    assert_eq!(aabb_side_multipliers(Direction::North), vec![Vec3::new(0.5, 0.25, 0.0), Vec3::new(0.5, 0.75, 0.0)]);
    assert_eq!(aabb_side_multipliers(Direction::West)[0], Vec3::new(0.0, 0.25, 0.5));
}

// ---------------------------------------------------------------- onTick

/// The harness with a 3 by 1 by 1 job at (7, 1, 5): air wanted at x = 7 where stone stands, planks missing at x = 8. The player is at (5, 1, 5) and has planks in slot 0.
fn break_then_place() -> (H, BuilderProcess) {
    let mut h = H::new();
    h.world.set(7, 1, 5, STONE);
    h.facts.plausible = true;
    h.facts.approx = vec![PLANKS as usize];
    h.facts.place = Some((0, PLANKS as usize));
    h.facts.ray = Some(BlockHit { pos: Pos::new(8, 0, 5), face: Direction::Up, point: Vec3::new(8.5, 1.0, 5.5) });
    h.inv.put(0, SlotInfo::of("oak_planks", 64), &[]);
    let bp = h.builder(StaticSchematic::new(2, 1, 1, vec![0, usize::from(PLANKS)], 0), Pos::new(7, 1, 5));
    (h, bp)
}

#[test]
fn a_wrong_block_in_reach_is_broken_before_anything_is_placed() {
    let (h, mut bp) = break_then_place();
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).expect("a command");
    assert_eq!(cmd, PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal });
    assert!(c.look.is_some() && c.look_force, "it looks at the block to break");
    assert!(!c.keys.click_right, "nothing is placed while a block waits to be broken");
    assert!(c.select.is_some(), "the tool is chosen");
    // not safe to cancel: the break waits and the tick goes on to the goal
    let (h, mut bp) = break_then_place();
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, false, &mut c).unwrap();
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
    // in the air: the same
    let (mut h, mut bp) = break_then_place();
    h.p.on_ground = false;
    let mut c = Controls::default();
    assert_eq!(tick(&h, &mut bp, true, &mut c).unwrap().kind, PathingCommandType::ForceRevalidateGoalAndPath);
}

#[test]
fn the_left_button_is_pressed_when_looking_at_the_block_and_sneaking_is_kept() {
    let (mut h, mut bp) = break_then_place();
    h.p.looking_at = Some(BlockHit { pos: Pos::new(7, 1, 5), face: Direction::West, point: Vec3::new(7.0, 1.5, 5.5) });
    let mut c = Controls::default();
    tick(&h, &mut bp, true, &mut c);
    assert!(c.keys.click_left);
    assert!(!c.keys.sneak);
    // crouching, or sneaking forced last tick, keeps the sneak
    h.p.crouching = true;
    let mut c = Controls::default();
    tick(&h, &mut bp, true, &mut c);
    assert!(c.keys.sneak);
    h.p.crouching = false;
    let mut c = Controls::default();
    c.keys.sneak = true;
    tick(&h, &mut bp, true, &mut c);
    assert!(c.keys.sneak, "isShiftKeyDown was true");
}

#[test]
fn a_place_waits_for_the_ground_the_safe_moment_and_the_ticks() {
    let (mut h, mut bp) = break_then_place();
    h.world.set(7, 1, 5, AIR); // nothing to break now
                               // it places: slot 0, sneaking, and the right button when the crosshair is on the face
    h.p.looking_at = Some(BlockHit { pos: Pos::new(8, 0, 5), face: Direction::Up, point: Vec3::new(8.5, 1.0, 5.5) });
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd, PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal });
    assert_eq!(c.select, Some(0));
    assert!(c.keys.sneak && c.keys.click_right && c.look_force);
    // the face is wrong: the button waits
    h.p.looking_at = Some(BlockHit { pos: Pos::new(8, 0, 5), face: Direction::North, point: Vec3::new(8.5, 1.0, 5.5) });
    let mut c = Controls::default();
    tick(&h, &mut bp, true, &mut c);
    assert!(c.keys.sneak && !c.keys.click_right);
    // really close to the aim is as good
    let rot = c.look.unwrap();
    h.p.yaw = rot.yaw;
    h.p.pitch = rot.pitch;
    let mut c = Controls::default();
    tick(&h, &mut bp, true, &mut c);
    assert!(c.keys.click_right);
    // not on the ground, not safe, or the ticks not run out: it goes on to the goal
    for (ground, safe, ticks) in [(false, true, 0), (true, false, 0), (true, true, 10)] {
        let (mut h2, mut bp2) = break_then_place();
        h2.world.set(7, 1, 5, AIR);
        h2.p.on_ground = ground;
        bp2.ticks = ticks;
        let mut c = Controls::default();
        let cmd = tick(&h2, &mut bp2, safe, &mut c).unwrap();
        assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath, "ground {ground} safe {safe} ticks {ticks}");
        assert!(!c.keys.click_right);
    }
}

#[test]
fn the_ticks_count_down_and_the_left_button_holds_them_at_five() {
    let (mut h, mut bp) = break_then_place();
    h.world.set(7, 1, 5, AIR);
    bp.ticks = 3;
    let mut c = Controls::default();
    tick(&h, &mut bp, true, &mut c);
    assert_eq!(bp.ticks, 2);
    // the left button was forced last tick: five
    let mut c = Controls::default();
    c.keys.click_left = true;
    tick(&h, &mut bp, true, &mut c);
    assert_eq!(bp.ticks, 5);
    // it is a place that waits: with 5 ticks left it does not place
    let mut c = Controls::default();
    c.keys.click_left = true;
    assert_eq!(tick(&h, &mut bp, true, &mut c).unwrap().kind, PathingCommandType::ForceRevalidateGoalAndPath);
}

#[test]
fn the_goal_comes_with_the_context() {
    let (mut h, mut bp) = break_then_place();
    h.world.set(7, 1, 5, AIR);
    h.p.on_ground = false;
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
    assert!(matches!(cmd.goal, Some(Goal::Janky { .. })));
    let ctx = bp.context().expect("the tick made a context");
    // jumpPenalty + 10, and no favoring of the old path
    assert_eq!(ctx.cfg.jump_penalty, h.cfg.jump_penalty + 10.0);
    assert_eq!(ctx.cfg.search.backtrack_cost_favoring_coefficient, 1.0);
    // its overlay prices the planks at the schematic's cell as free (they are in hand)
    assert_eq!(ctx.costs.cost_of_placing_at(&ctx.cfg, &OPEN, 8, 1, 5, 0), 0.0);
    // a tick that answers anything else drops it
    h.p.on_ground = true;
    h.world.set(7, 1, 5, STONE);
    tick(&h, &mut bp, true, &mut c);
    assert!(bp.context().is_none());
}

#[test]
fn missing_materials_pause_and_a_resume_goes_on() {
    let mut h = H::new();
    h.facts.plausible = true;
    let mut bp = h.builder(solid_box(3, 1, 1, PLANKS), Pos::new(8, 1, 8));
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd, PathingCommand { goal: None, kind: PathingCommandType::RequestPause });
    assert!(bp.is_paused());
    assert_eq!(bp.display_name(), "Builder Paused");
    assert_eq!(bp.take_messages(), vec!["Missing materials for at least:", "3x Block{minecraft:oak_planks}", "Unable to do it. Pausing. resume to resume, cancel to cancel"]);
    // while paused: no path and no goal, every tick
    for _ in 0..3 {
        assert_eq!(tick(&h, &mut bp, true, &mut c).unwrap(), PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal });
    }
    assert!(bp.take_messages().is_empty());
    // the planks come, and the builder is resumed
    h.facts.approx = vec![PLANKS as usize];
    bp.resume();
    assert!(!bp.is_paused());
    assert_eq!(bp.display_name(), "Building test");
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
}

#[test]
fn a_finished_build_says_so_once_and_lets_go() {
    let mut h = H::new();
    h.world.set(8, 1, 8, PLANKS);
    let mut bp = h.builder(solid_box(1, 1, 1, PLANKS), Pos::new(8, 1, 8));
    let mut c = Controls::default();
    assert!(tick(&h, &mut bp, true, &mut c).is_none());
    assert_eq!(bp.take_messages(), vec!["Done building"]);
    assert!(bp.take_finished());
    assert!(!bp.take_finished());
    assert!(!bp.is_active());
    h.world.set(8, 1, 8, AIR);
    assert!(tick(&h, &mut bp, true, &mut c).is_none(), "nothing to build: the process is gone");
}

#[test]
fn layers_bottom_up_with_a_layer_height_of_two() {
    let mut h = H::new();
    h.settings.build_in_layers = true;
    h.settings.layer_height = 2;
    h.facts.approx = vec![STONE as usize];
    // a 1 by 4 by 1 column of stone at (8, 1, 8): the lower two are in the world, the upper two are not
    h.world.set(8, 1, 8, STONE);
    h.world.set(8, 2, 8, STONE);
    let mut bp = h.builder(solid_box(1, 4, 1, STONE), Pos::new(8, 1, 8));
    assert_eq!(bp.max_layer(&h.settings), Some(4));
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath, "{:?}", bp.take_messages());
    // layer 0 (bounds 0..=-1) and layer 1 (0..=1) were right: two "Starting layer" lines; now at layer 2 with 0..=3
    assert_eq!(bp.take_messages(), vec!["Starting layer 0", "Starting layer 1"]);
    assert_eq!(bp.layer(), 2);
    assert_eq!(bp.layer_bounds, Some((0, 3)));
    assert_eq!(bp.incorrect_positions(), vec![Pos::new(8, 3, 8), Pos::new(8, 4, 8)]);
    assert_eq!(bp.min_layer(&h.settings), Some(2));
}

#[test]
fn layers_top_down() {
    let mut h = H::new();
    h.settings.build_in_layers = true;
    h.settings.layer_order = true;
    h.settings.layer_height = 2;
    h.facts.approx = vec![STONE as usize];
    // the same column; the top two are wrong. Top down, the first layer is the top two cells, so the first tick finds work at once
    h.world.set(8, 1, 8, STONE);
    h.world.set(8, 2, 8, STONE);
    let mut bp = h.builder(solid_box(1, 4, 1, STONE), Pos::new(8, 1, 8));
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
    // layer 0: bounds (4, 3) is empty ("Starting layer 0"); layer 1: bounds (2, 3) holds the two wrong cells
    assert_eq!(bp.take_messages(), vec!["Starting layer 0"]);
    assert_eq!(bp.layer(), 1);
    assert_eq!(bp.layer_bounds, Some((2, 3)));
    assert_eq!(bp.incorrect_positions(), vec![Pos::new(8, 3, 8), Pos::new(8, 4, 8)]);
}

#[test]
fn the_layer_wrapper_stays_when_the_setting_is_turned_off() {
    let mut h = H::new();
    h.settings.build_in_layers = true;
    h.facts.approx = vec![STONE as usize];
    let mut bp = h.builder(solid_box(1, 3, 1, STONE), Pos::new(8, 1, 8));
    let mut c = Controls::default();
    tick(&h, &mut bp, true, &mut c);
    // layer 1 holds only the lowest cell: bounds (0, 0)
    assert_eq!(bp.layer_bounds, Some((0, 0)));
    // buildInLayers off: the wrapper is not remade, and it still hides the cells above (U37)
    h.settings.build_in_layers = false;
    assert_eq!(bp.place_at(8, 3, 8, 0, &h.settings, &h.bs), None, "above the stale bounds");
    assert_eq!(bp.place_at(8, 1, 8, 0, &h.settings, &h.bs), Some(STONE as usize));
}

#[test]
fn skip_failed_layers_goes_on_to_the_next_layer() {
    let mut h = H::new();
    h.settings.build_in_layers = true;
    h.settings.skip_failed_layers = true;
    // nothing in hand: the first layer cannot be built, the second can (planks wanted over a floor; the first layer wants stone, which is missing)
    h.facts.approx = vec![PLANKS as usize];
    h.facts.plausible = true;
    let schem = StaticSchematic::new(1, 2, 1, vec![STONE as usize, PLANKS as usize], 0);
    let mut bp = h.builder(schem, Pos::new(8, 1, 8));
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    // layer 0 is empty (bounds up to -1): "Starting layer 0"; layer 1 holds the stone: it cannot be built: skipped; layer 2 holds both: the planks can't go (floating) but are placeable
    let msgs = bp.take_messages();
    assert_eq!(msgs[0], "Starting layer 0");
    assert!(msgs.iter().any(|m| m == "Skipping layer that I cannot construct! Layer #1"), "{msgs:?}");
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
    // without the setting it pauses
    h.settings.skip_failed_layers = false;
    let mut bp = h.builder(StaticSchematic::new(1, 2, 1, vec![STONE as usize, PLANKS as usize], 0), Pos::new(8, 1, 8));
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd.kind, PathingCommandType::RequestPause);
    assert!(bp.is_paused());
}

#[test]
fn a_repeat_moves_the_origin_and_counts() {
    let mut h = H::new();
    h.settings.build_repeat = (3, 0, 0);
    h.settings.build_repeat_count = 2;
    // a 1 by 1 by 1 of stone; the world has stone at (4, 1, 5) and (7, 1, 5)
    h.world.set(4, 1, 5, STONE);
    h.world.set(7, 1, 5, STONE);
    let mut bp = h.builder(solid_box(1, 1, 1, STONE), Pos::new(4, 1, 5));
    let mut c = Controls::default();
    assert!(tick(&h, &mut bp, true, &mut c).is_none());
    assert_eq!(bp.take_messages(), vec!["Repeating build in vector Vec3i{x=3, y=0, z=0}, new origin is BlockPos{x=7, y=1, z=5}", "Done building"]);
    assert!(bp.take_finished());
    // with a count of 3 and a world that holds only two copies, the third has to be built
    h.settings.build_repeat_count = 3;
    let mut bp = h.builder(solid_box(1, 1, 1, STONE), Pos::new(4, 1, 5));
    h.facts.approx = vec![STONE as usize];
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
    assert_eq!(bp.origin(), Pos::new(10, 1, 5));
    assert!(!bp.take_finished());
}

#[test]
fn an_endless_repeat_of_nothing_stops_at_the_recursion_cap() {
    let mut h = H::new();
    h.settings.build_repeat = (1, 0, 0);
    // air wanted over air, forever
    let mut bp = h.builder(solid_box(1, 1, 1, AIR), Pos::new(4, 3, 5));
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd, PathingCommand { goal: None, kind: PathingCommandType::SetGoalAndPath });
    // 101 repeats happened before the cap: the origin moved 101 times
    assert_eq!(bp.origin().x, 4 + 101);
    assert!(bp.is_active(), "the build is still on");
}

#[test]
fn stopping_for_a_new_build_resets_the_layer_and_the_pause() {
    let mut h = H::new();
    h.settings.start_at_layer = 2;
    let mut bp = h.builder(solid_box(1, 1, 1, STONE), Pos::new(8, 1, 8));
    assert_eq!(bp.layer(), 2);
    bp.pause();
    bp.on_lost_control();
    assert!(!bp.is_active() && !bp.is_paused());
    assert_eq!(bp.layer(), 2, "startAtLayer");
    assert_eq!(bp.display_name(), "Building null");
}

#[test]
fn build_wraps_the_orientation_offsets_with_the_unrotated_size() {
    let mut h = H::new();
    h.settings.schematic_orientation_x = true;
    h.settings.schematic_orientation_z = true;
    h.settings.schematic_orientation_y = true;
    let bp = h.builder(StaticSchematic::new(3, 2, 5, vec![0; 30], 0), Pos::new(10, 20, 30));
    assert_eq!(bp.origin(), Pos::new(13, 22, 35));
    assert_eq!(bp.stop_at_height, 2);
    // with a rotation of 90 degrees the chain is 5 by 2 by 3, but the offsets still use 3 and 5 (U35)
    h.settings.build_schematic_rotation = crate::schematic::BlockRotation::Clockwise90;
    let bp = h.builder(StaticSchematic::new(3, 2, 5, vec![0; 30], 0), Pos::new(10, 20, 30));
    assert_eq!(bp.origin(), Pos::new(13, 22, 35));
    assert_eq!((bp.chain.as_ref().unwrap().width_x(), bp.chain.as_ref().unwrap().length_z()), (5, 3));
}

#[test]
fn the_skip_mask_reads_the_settings_of_the_tick() {
    let mut h = H::new();
    let bp = h.builder(solid_box(2, 1, 1, STONE), Pos::new(8, 1, 8));
    assert_eq!(bp.place_at(8, 1, 8, 0, &h.settings, &h.bs), Some(STONE as usize));
    h.settings.build_skip_blocks = vec!["stone".into()];
    assert_eq!(bp.place_at(8, 1, 8, 0, &h.settings, &h.bs), None, "skipped blocks are not in the schematic");
    h.settings.build_skip_blocks.clear();
    assert_eq!(bp.place_at(8, 1, 8, 0, &h.settings, &h.bs), Some(STONE as usize));
    // and a position outside, and a wish for air, give none
    assert_eq!(bp.place_at(0, 1, 0, 0, &h.settings, &h.bs), None);
    let air = h.builder(solid_box(1, 1, 1, AIR), Pos::new(8, 1, 8));
    assert_eq!(air.place_at(8, 1, 8, 0, &h.settings, &h.bs), None);
}

#[test]
fn clear_area_wants_air_in_the_box_between_the_corners() {
    let h = H::new();
    let mut bp = BuilderProcess::new();
    bp.clear_area(Pos::new(9, 4, 2), Pos::new(7, 3, 5), &h.settings, &h.bs);
    assert_eq!(bp.origin(), Pos::new(7, 3, 2));
    let chain = bp.chain.as_ref().unwrap();
    assert_eq!((chain.width_x(), chain.height_y(), chain.length_z()), (3, 2, 4));
    assert_eq!(bp.display_name(), "Building clear area");
    // stone there is wrong; air is right
    assert_eq!(bp.place_at(8, 3, 3, STONE as usize, &h.settings, &h.bs), None, "air is never placed");
    let view = bp.view(&h.settings, &h.bs).unwrap();
    assert_eq!(view.get(8, 3, 3, STONE as usize), Some(0));
    assert_eq!(view.get(8, 5, 3, STONE as usize), None);
}

// ---------------------------------------------------------------- the throwaway hint

struct FixedHint {
    wants: Option<usize>,
    places: Vec<(InvSlot, usize)>,
    blocks: Vec<(InvSlot, usize)>,
}

impl ThrowawayHint for FixedHint {
    fn place_at(&self, _x: i32, _y: i32, _z: i32, _current: usize) -> Option<usize> {
        self.wants
    }
    fn slot_places(&self, slot: InvSlot, maybe: usize) -> bool {
        self.places.contains(&(slot, maybe))
    }
    fn slot_is_block_of(&self, slot: InvSlot, maybe: usize) -> bool {
        self.blocks.contains(&(slot, maybe))
    }
}

#[test]
fn the_hint_picks_the_slot_that_places_what_the_schematic_wants() {
    let mut inv = TestInventory::empty();
    inv.put(1, SlotInfo::of("stone", 64), &[]);
    inv.put(4, SlotInfo::of("oak_planks", 64), &[]);
    inv.put(6, SlotInfo::of("oak_stairs", 64), &[]);
    let acceptable = vec!["stone".to_string()];
    // no hint: the acceptable item
    assert_eq!(select_throwaway_for_location(&inv, &acceptable, None, 0, 0, 0, 0, false).map(Throwaway::hand_slot), Some(1));
    // the schematic wants state 7, which slot 6 places exactly: that slot, not stone
    let h = FixedHint { wants: Some(7), places: vec![(InvSlot::Hotbar(6), 7)], blocks: vec![(InvSlot::Hotbar(4), 7)] };
    assert_eq!(select_throwaway_for_location(&inv, &acceptable, Some(&h), 0, 0, 0, 0, false).map(Throwaway::hand_slot), Some(6));
    // none places it exactly, but slot 4 holds an item of its block
    let h = FixedHint { wants: Some(7), places: vec![], blocks: vec![(InvSlot::Hotbar(4), 7)] };
    assert_eq!(select_throwaway_for_location(&inv, &acceptable, Some(&h), 0, 0, 0, 0, false).map(Throwaway::hand_slot), Some(4));
    // nothing matches: the list again
    let h = FixedHint { wants: Some(7), places: vec![], blocks: vec![] };
    assert_eq!(select_throwaway_for_location(&inv, &acceptable, Some(&h), 0, 0, 0, 0, false).map(Throwaway::hand_slot), Some(1));
    // the builder wants nothing here (outside the schematic): the list
    let h = FixedHint { wants: None, places: vec![(InvSlot::Hotbar(6), 7)], blocks: vec![] };
    assert_eq!(select_throwaway_for_location(&inv, &acceptable, Some(&h), 0, 0, 0, 0, false).map(Throwaway::hand_slot), Some(1));
    // the off hand counts, with a main hand that does not click
    let mut inv2 = TestInventory::empty();
    inv2.offhand = SlotInfo::of("oak_planks", 64);
    let h = FixedHint { wants: Some(7), places: vec![(InvSlot::Offhand, 7)], blocks: vec![] };
    assert_eq!(select_throwaway_for_location(&inv2, &acceptable, Some(&h), 0, 0, 0, 0, false).map(Throwaway::hand_slot), Some(0));
}

#[test]
fn the_builders_hint_knows_the_schematic_the_hotbar_and_the_item_blocks() {
    let mut h = H::new();
    h.facts.approx = vec![STONE as usize, PLANKS as usize];
    h.facts.blocks = vec![(InvSlot::Hotbar(1), "oak_planks".into()), (InvSlot::Hotbar(0), "stone".into())];
    h.facts.offhand = STONE as usize;
    let mut bp = h.builder(StaticSchematic::new(2, 1, 1, vec![usize::from(PLANKS), 0], 0), Pos::new(8, 1, 8));
    let _ = h.prepare(&mut bp);
    let hint = bp.throwaway_hint(&h.be()).expect("active");
    assert_eq!(hint.place_at(8, 1, 8, 0), Some(PLANKS as usize));
    assert_eq!(hint.place_at(9, 1, 8, 0), None, "air is not placed");
    assert!(hint.slot_places(InvSlot::Hotbar(1), PLANKS as usize));
    assert!(!hint.slot_places(InvSlot::Hotbar(0), PLANKS as usize));
    assert!(hint.slot_places(InvSlot::Offhand, STONE as usize));
    assert!(hint.slot_is_block_of(InvSlot::Hotbar(1), PLANKS as usize));
    assert!(!hint.slot_is_block_of(InvSlot::Hotbar(0), PLANKS as usize));
    assert!(!hint.slot_is_block_of(InvSlot::Hotbar(5), PLANKS as usize));
    // an inactive builder gives no hint
    assert!(BuilderProcess::new().throwaway_hint(&h.be()).is_none());
}

// ---------------------------------------------------------------- allowInventory in the tick

/// A job of planks at (8, 1, 5) and stone at (9, 1, 5), both cells empty and in reach, with nothing on the hotbar that places them: `approx` is the list of
/// states of the 36 slots (the hotbar first). The tool settings have `allowInventory` and a spacing of 0 ticks, so a move is made at once.
fn inventory_job(approx: Vec<StateId>) -> (H, BuilderProcess) {
    let mut h = H::new();
    h.facts.plausible = true;
    h.facts.approx = approx;
    h.facts.ray = Some(BlockHit { pos: Pos::new(8, 0, 5), face: Direction::Up, point: Vec3::new(8.5, 1.0, 5.5) });
    h.exec.tool.allow_inventory = true;
    h.exec.tool.ticks_between_inventory_moves = 0;
    let bp = h.builder(StaticSchematic::new(2, 1, 1, vec![usize::from(PLANKS), usize::from(STONE)], 0), Pos::new(8, 1, 5));
    (h, bp)
}

fn approx_with(slots: &[(usize, StateId)]) -> Vec<StateId> {
    let mut v = vec![0; 36];
    for &(i, st) in slots {
        v[i] = st;
    }
    v
}

/// `BuilderProcess.onTick(boolean,boolean,int)` with `allowInventory`: the wanted block is only in the main inventory, so it is moved to a temporary hotbar slot, and the tick goes on.
#[test]
fn a_wanted_block_in_the_main_inventory_is_moved_to_the_hotbar() {
    let (h, mut bp) = inventory_job(approx_with(&[(20, PLANKS as usize)]));
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    let moves = c.inventory.take_requests();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].in_inventory, 20);
    assert!((1..8).contains(&moves[0].in_hotbar), "slots 0 and 8 are for the pickaxe and the throwaway block");
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath, "after the move it goes on to the goal");
}

/// `BuilderProcess.onTick(boolean,boolean,int)` with `allowInventory` off: nothing is moved, and the tick goes on as before.
#[test]
fn without_the_setting_nothing_is_moved() {
    let (mut h, mut bp) = inventory_job(approx_with(&[(20, PLANKS as usize)]));
    h.exec.tool.allow_inventory = false;
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert!(c.inventory.take_requests().is_empty());
    assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
}

/// `BuilderProcess.onTick(boolean,boolean,int)`: a state that a hotbar slot already holds is not fetched, and that slot is not used as the temporary one for another.
#[test]
fn a_hotbar_slot_that_holds_a_wanted_block_is_kept() {
    // planks are on the hotbar already (slot 3), and stone is in the main inventory: only stone moves, and not onto slot 3
    for _ in 0..30 {
        let (h, mut bp) = inventory_job(approx_with(&[(3, PLANKS as usize), (20, PLANKS as usize), (25, STONE as usize)]));
        let mut c = Controls::default();
        tick(&h, &mut bp, true, &mut c);
        let moves = c.inventory.take_requests();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].in_inventory, 25, "the first slot of 9 to 35 that holds the state with no hotbar option");
        assert_ne!(moves[0].in_hotbar, 3);
        assert!((1..8).contains(&moves[0].in_hotbar));
    }
    // both are on the hotbar: nothing moves
    let (h, mut bp) = inventory_job(approx_with(&[(3, PLANKS as usize), (4, STONE as usize), (20, PLANKS as usize)]));
    let mut c = Controls::default();
    tick(&h, &mut bp, true, &mut c);
    assert!(c.inventory.take_requests().is_empty());
}

/// `BuilderProcess.onTick(boolean,boolean,int)`: only one stack is moved per tick (`break outer`), the first of the main inventory that serves a state with no hotbar option.
#[test]
fn one_stack_is_moved_per_tick() {
    let (h, mut bp) = inventory_job(approx_with(&[(30, STONE as usize), (12, PLANKS as usize), (14, STONE as usize)]));
    let mut c = Controls::default();
    tick(&h, &mut bp, true, &mut c);
    let moves = c.inventory.take_requests();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].in_inventory, 12);
}

/// `BuilderProcess.onTick(boolean,boolean,int)`: when the move cannot be made yet (`ticksBetweenInventoryMoves`), the builder asks for a pause and waits.
#[test]
fn a_delayed_move_pauses_the_path() {
    let (mut h, mut bp) = inventory_job(approx_with(&[(20, PLANKS as usize)]));
    h.exec.tool.ticks_between_inventory_moves = 1; // no tick has counted yet
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd, PathingCommand { goal: None, kind: PathingCommandType::RequestPause });
    assert!(c.inventory.take_requests().is_empty());
    // the behavior's next tick makes the remembered move
    c.inventory.on_tick(&h.inv, &h.exec.tool, false, false, None);
    let moves = c.inventory.take_requests();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].in_inventory, 20);
}

/// `BuilderProcess.onTick(boolean,boolean,int)`: a block that can be placed from the hotbar this tick is placed first, and nothing is fetched on that tick.
#[test]
fn a_placement_comes_before_a_move() {
    let (mut h, mut bp) = inventory_job(approx_with(&[(0, PLANKS as usize), (20, STONE as usize)]));
    h.facts.place = Some((0, PLANKS as usize));
    h.p.looking_at = Some(BlockHit { pos: Pos::new(8, 0, 5), face: Direction::Up, point: Vec3::new(8.5, 1.0, 5.5) });
    h.inv.put(0, SlotInfo::of("oak_planks", 64), &[]);
    let mut c = Controls::default();
    let cmd = tick(&h, &mut bp, true, &mut c).unwrap();
    assert_eq!(cmd.kind, PathingCommandType::CancelAndSetGoal);
    assert_eq!(c.select, Some(0));
    assert!(c.inventory.take_requests().is_empty());
}

// ---------------------------------------------------------------- the size of the job

#[test]
fn a_full_recalc_over_64_cubed_finishes_and_reports_its_time() {
    let mut h = H::new();
    h.world = TestWorld::tall(64, 64, 64);
    h.settings.incorrect_size = i32::MAX;
    let mut bp = h.builder(solid_box(64, 64, 64, PLANKS), Pos::new(0, 0, 0));
    let bcc = h.prepare(&mut bp);
    let ctx = h.ctx();
    let env = h.env(&ctx);
    let start = std::time::Instant::now();
    bp.full_recalc(&env, &bcc, &h.rules(), &h.settings);
    let took = start.elapsed();
    // the floor at y = 0 is stone, so every cell is wrong
    assert_eq!(bp.incorrect_positions().len(), 64 * 64 * 64);
    eprintln!("full_recalc over 64 x 64 x 64: {took:?}");
    assert!(took.as_secs() < 60, "{took:?}");
}

// ---------------------------------------------------------------- review fixes (D50)

#[test]
fn review_the_map_art_line_goes_to_the_console_and_not_to_chat() {
    let h = H::new();
    let mut settings = h.settings.clone();
    settings.map_art_mode = true;
    let mut bp = BuilderProcess::new();
    // a 1 x 1 x 1 map art that is only air: one column with no block
    let parsed: Arc<dyn StaticSchematicView> = Arc::new(solid_box(1, 1, 1, AIR));
    bp.build_parsed("art".into(), parsed, Pos::new(0, 1, 0), &settings, &h.bs).unwrap();
    assert_eq!(bp.take_console_lines(), vec!["1 columns had no block despite being in a map art, letting them be whatever".to_string()]);
    assert!(bp.take_console_lines().is_empty(), "taken once");
    assert!(!bp.take_messages().iter().any(|m| m.contains("columns had no block")));
}

#[test]
fn review_placement_goal_reads_the_live_world_not_the_cache_above() {
    // upstream reads `ctx.world()`: a column that is not loaded is void air, so the block above does not allow the same level, although
    // the cache (which the search context also reads) still holds a stone block there
    let mut h = H::new();
    h.facts.plausible = true;
    h.world.set(3, 2, 8, STONE);
    let goal_of = |h: &H| {
        let mut bp = h.builder(solid_box(1, 1, 1, PLANKS), Pos::new(3, 1, 8));
        let bcc = h.prepare(&mut bp);
        let ctx = h.ctx();
        let env = h.env(&ctx);
        bp.placement_goal(&env, &h.be(), Pos::new(3, 1, 8), &bcc)
    };
    assert_eq!(goal_of(&h), Goal::Adjacent { x: 3, y: 1, z: 8, no: Pos::new(3, 0, 8), allow_same_level: true }, "loaded: stone above");
    h.world.loaded_below_x = 3;
    assert_eq!(goal_of(&h), Goal::Adjacent { x: 3, y: 1, z: 8, no: Pos::new(3, 0, 8), allow_same_level: false }, "not loaded: void air above");
}
