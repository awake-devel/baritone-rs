// baritone-rs test tool: compares upstream's real Java classes with baritone-engine, bit for bit.
// LGPL-3.0-or-later, see LICENSE.
//
// Input: the golden file that java/baritone/pathing/movement/SpikeGoldenDump.java writes. Line types:
//   B  world border and the size of the grids, start position
//   M  the ordinals of Moves.values(), with xOffset, yOffset, zOffset, dynamicXZ and dynamicY
//   S  one palette state: raw game facts, and the mining strength with a pickaxe and with a hand
//   C  one configuration: every setting that the engine's Config holds
//   F  the fall table and JUMP_ONE_BLOCK_COST
//   Z  the two configurations of the registry sweep
//   A  one state of the game's whole block state registry, with upstream's answers under those two configurations
//   Q  one goal, with upstream's answers at several positions
//   W  the world border (min x, max x, min z, max z) for the grids that follow
//   L  whether the column of the next grid is loaded (worldContainsLoadedChunk)
//   G  one grid (mode, the start height, the palette indexes of its cells, two hex digits each)
//   R  one movement result from the start of the grid under one configuration
//   P  the position-bound rules (canWalkThrough, getMiningDurationTicks...) at one position under one configuration
// Nothing in the file is a Baritone rule except the answers of upstream's own classes, which are what is compared.
use baritone_engine::blocks::*;
use baritone_engine::config::Config;
use baritone_engine::costs::*;
use baritone_engine::goals::*;
use baritone_engine::movement::helper::*;
use baritone_engine::movement::moves::Moves;
use baritone_engine::world::GridWorld;
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

const GRID_W: i32 = 13;
const GRID_H: i32 = 16;
const GRID_D: i32 = 13;

fn bit(v: &str) -> bool {
    match v {
        "1" => true,
        "0" => false,
        _ => panic!("not a flag: {v}"),
    }
}

fn dbl(v: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(v, 16).unwrap_or_else(|_| panic!("not a hex double: {v}")))
}

fn int(v: &str) -> i32 {
    v.parse().unwrap_or_else(|_| panic!("not an int: {v}"))
}

fn list(v: &str) -> Vec<String> {
    if v == "-" {
        vec![]
    } else {
        v.split(',').map(|s| s.to_string()).collect()
    }
}

fn parse_fluid(v: &str) -> Fluid {
    match v {
        "none" => Fluid::None,
        "water" => Fluid::Water,
        "flowing_water" => Fluid::FlowingWater,
        "lava" => Fluid::Lava,
        "flowing_lava" => Fluid::FlowingLava,
        _ => panic!("unknown fluid {v}"),
    }
}

fn parse_slab_type(v: &str) -> SlabType {
    match v {
        "BOTTOM" | "-" => SlabType::Bottom,
        "TOP" => SlabType::Top,
        "DOUBLE" => SlabType::Double,
        _ => panic!("unknown slab type {v}"),
    }
}

fn parse_half(v: &str) -> Half {
    match v {
        "BOTTOM" | "-" => Half::Bottom,
        "TOP" => Half::Top,
        _ => panic!("unknown half {v}"),
    }
}

fn parse_shape(v: &str) -> StairShape {
    match v {
        "STRAIGHT" | "-" => StairShape::Straight,
        "INNER_LEFT" => StairShape::InnerLeft,
        "INNER_RIGHT" => StairShape::InnerRight,
        "OUTER_LEFT" => StairShape::OuterLeft,
        "OUTER_RIGHT" => StairShape::OuterRight,
        _ => panic!("unknown stair shape {v}"),
    }
}

fn parse_direction(v: &str) -> Direction {
    match v {
        "NORTH" | "-" => Direction::North,
        "SOUTH" => Direction::South,
        "EAST" => Direction::East,
        "WEST" => Direction::West,
        "UP" => Direction::Up,
        "DOWN" => Direction::Down,
        _ => panic!("unknown direction {v}"),
    }
}

/// A state from its name and `key=value` facts. Returns the mining strengths too (0 when the line has none).
fn parse_state(name: &str, kvs: &[&str]) -> (State, f64, f64) {
    let mut s = State { name: name.to_string(), block: Block::from_name(name), ..Default::default() };
    let (mut pick, mut hand) = (0.0, 0.0);
    for kv in kvs {
        let (k, v) = kv.split_once('=').unwrap_or_else(|| panic!("not a fact: {kv}"));
        match k {
            "air" => s.air = bit(v),
            "amethyst_cluster" => s.amethyst_cluster = bit(v),
            "azalea" => s.azalea = bit(v),
            "bamboo_stalk" => s.bamboo_stalk = bit(v),
            "base_fire" => s.base_fire = bit(v),
            "carpet" => s.carpet = bit(v),
            "cauldron" => s.cauldron = bit(v),
            "door" => s.door = bit(v),
            "falling" => s.falling = bit(v),
            "gate" => s.gate = bit(v),
            "infested" => s.infested = bit(v),
            "leaves" => s.leaves = bit(v),
            "lily" => s.lily = bit(v),
            "liquid_block" => s.liquid_block = bit(v),
            "moving_piston" => s.moving_piston = bit(v),
            "pointed_dripstone" => s.pointed_dripstone = bit(v),
            "scaffolding" => s.scaffolding = bit(v),
            "shulker" => s.shulker = bit(v),
            "skull_any" => s.skull_any = bit(v),
            "skull_floor" => s.skull_floor = bit(v),
            "slab" => s.slab = bit(v),
            "snow" => s.snow = bit(v),
            "stair" => s.stair = bit(v),
            "stained_glass" => s.stained_glass = bit(v),
            "trapdoor" => s.trapdoor = bit(v),
            "end_portal_block" => s.end_portal_block = bit(v),
            "wooden_door" => s.wooden_door = bit(v),
            "pathfindable" => s.pathfindable = bit(v),
            "shape_full" => s.shape_full = bit(v),
            "waterlogged" => s.waterlogged = bit(v),
            "top_sturdy" => s.top_sturdy = bit(v),
            "replaceable" => s.replaceable = bit(v),
            "falling_free" => s.falling_free = bit(v),
            "frost_melt" => s.frost_melt = bit(v),
            "fluid" => s.fluid = parse_fluid(v),
            "fluid_empty" => s.fluid_empty = bit(v),
            "fluid_amount" => s.fluid_amount = int(v),
            "fluid_source" => s.fluid_source = bit(v),
            "fluid_flowing_kind" => s.fluid_flowing_kind = bit(v),
            "level" => s.level = int(v),
            "slab_type" => s.slab_type = parse_slab_type(v),
            "stair_half" => s.stair_half = parse_half(v),
            "stair_shape" => s.stair_shape = parse_shape(v),
            "trap_open" => s.trap_open = bit(v),
            "trap_half" => s.trap_half = parse_half(v),
            "layers" => s.layers = int(v),
            "door_facing" => s.door_facing = parse_direction(v),
            "door_hinge_left" => s.door_hinge_left = bit(v),
            "str_pick" => pick = dbl(v),
            "str_hand" => hand = dbl(v),
            _ => panic!("unknown fact {k}"),
        }
    }
    (s, pick, hand)
}

/// A configuration line: the engine's Config, and whether the player holds a pickaxe (what picks the tool strengths).
fn parse_config(p: &[&str]) -> (Config, bool) {
    let mut kv: BTreeMap<&str, &str> = BTreeMap::new();
    for item in &p[2..] {
        let (k, v) = item.split_once('=').unwrap_or_else(|| panic!("not a setting: {item}"));
        assert!(kv.insert(k, v).is_none(), "setting twice: {k}");
    }
    fn take<'a>(kv: &mut BTreeMap<&str, &'a str>, k: &str) -> &'a str {
        kv.remove(k).unwrap_or_else(|| panic!("config line without {k}"))
    }
    let mut get = |k: &str| take(&mut kv, k);
    let pick = bit(get("pick"));
    let c = Config {
        has_throwaway: bit(get("throwaway")),
        has_water_bucket: bit(get("bucket")),
        can_sprint: bit(get("sprint")),
        frost_walker: int(get("frost")),
        water_mult: f32::from_bits(u32::from_str_radix(get("watermult"), 16).expect("hex float")),
        allow_break: bit(get("allowbreak")),
        allow_break_anyway: list(get("anyway")),
        blocks_to_avoid: list(get("avoid")),
        blocks_to_disallow_breaking: list(get("disallow")),
        assume_walk_on_water: bit(get("walkwater")),
        assume_walk_on_lava: bit(get("walklava")),
        allow_walk_on_magma: bit(get("magma")),
        allow_vines: bit(get("vines")),
        allow_walk_on_bottom_slab: bit(get("bottomslab")),
        strict_liquid_check: bit(get("strictliquid")),
        avoid_updating_falling: bit(get("avoidfalling")),
        allow_place_source: bit(get("placesource")),
        allow_place_flow: bit(get("placeflow")),
        allow_swimming: bit(get("swim")),
        allow_parkour: bit(get("parkour")),
        allow_parkour_place: bit(get("parkourplace")),
        allow_parkour_ascend: bit(get("parkourascend")),
        allow_jump_at_build_limit: bit(get("buildlimit")),
        allow_diagonal_descend: bit(get("diagdescend")),
        allow_diagonal_ascend: bit(get("diagascend")),
        allow_downward: bit(get("downward")),
        max_fall_no_water: int(get("maxfall")),
        max_fall_bucket: int(get("maxfallbucket")),
        place_penalty: dbl(get("place")),
        break_penalty: dbl(get("break")),
        jump_penalty: dbl(get("jump")),
        walk_on_water_penalty: dbl(get("waterwalk")),
        cost_heuristic: dbl(get("costheuristic")),
        axis_height: int(get("axisheight")),
        search: Default::default(),
    };
    assert!(kv.is_empty(), "config line with settings the engine does not have: {:?}", kv.keys().collect::<Vec<_>>());
    (c, pick)
}

/// The settings that no configuration of the file gives both values to. Each of them would leave half of a rule untested.
fn config_gaps(cfgs: &[Config]) -> Vec<&'static str> {
    let mut gaps = vec![];
    macro_rules! flag {
        ($($f:ident),*) => { $( if !(cfgs.iter().any(|c| c.$f) && cfgs.iter().any(|c| !c.$f)) { gaps.push(stringify!($f)); } )* };
    }
    macro_rules! varies {
        ($($f:ident),*) => { $( if !cfgs.iter().any(|c| c.$f != cfgs[0].$f) { gaps.push(stringify!($f)); } )* };
    }
    macro_rules! lists {
        ($($f:ident),*) => { $( if !(cfgs.iter().any(|c| c.$f.is_empty()) && cfgs.iter().any(|c| !c.$f.is_empty())) { gaps.push(stringify!($f)); } )* };
    }
    if cfgs.is_empty() {
        return vec!["no configuration at all"];
    }
    flag!(
        has_throwaway,
        has_water_bucket,
        can_sprint,
        allow_break,
        assume_walk_on_water,
        assume_walk_on_lava,
        allow_walk_on_magma,
        allow_vines,
        allow_walk_on_bottom_slab,
        strict_liquid_check,
        avoid_updating_falling,
        allow_place_source,
        allow_place_flow,
        allow_swimming,
        allow_parkour,
        allow_parkour_place,
        allow_parkour_ascend,
        allow_jump_at_build_limit,
        allow_diagonal_descend,
        allow_diagonal_ascend,
        allow_downward
    );
    varies!(frost_walker, water_mult, max_fall_no_water, max_fall_bucket, place_penalty, break_penalty, jump_penalty, walk_on_water_penalty, cost_heuristic, axis_height);
    lists!(allow_break_anyway, blocks_to_avoid, blocks_to_disallow_breaking);
    gaps
}

#[derive(Default)]
struct Tally {
    evals: u64,
    exact: u64,
    close: u64,
    wrong: u64,
    finite: u64,
    distinct: BTreeSet<u64>,
}

impl Tally {
    /// One comparison. `finite` says whether upstream's answer counts as a real, non-trivial one (a finite cost, a true flag).
    fn add(&mut self, java: f64, rust: f64, same_dest: bool, finite: bool) -> bool {
        self.evals += 1;
        if finite {
            self.finite += 1;
        }
        self.distinct.insert(java.to_bits());
        if same_dest && (java.to_bits() == rust.to_bits() || (java.is_nan() && rust.is_nan())) {
            self.exact += 1;
            true
        } else if same_dest && !(java == 0.0 && rust == 0.0) && (java - rust).abs() <= 1e-9 * java.abs().max(1.0) {
            self.close += 1;
            true
        } else {
            self.wrong += 1;
            if STOP_AT_FIRST.load(std::sync::atomic::Ordering::Relaxed) {
                // Injected-error runs only need to know that something differs. The normal run never does this.
                println!("stopped at the first wrong result");
                std::process::exit(1);
            }
            false
        }
    }
    fn add_bool(&mut self, java: bool, rust: bool) -> bool {
        self.add(if java { 1.0 } else { 0.0 }, if rust { 1.0 } else { 0.0 }, true, java)
    }
    fn print(&self, name: &str) {
        println!("{name}: {} evals, {} finite, {} distinct, exact {}, within 1e-9 {}, WRONG {}", self.evals, self.finite, self.distinct.len(), self.exact, self.close, self.wrong);
    }
}

/// The family of a movement, from its ordinal in `Moves.values()`.
fn family(ordinal: usize) -> &'static str {
    match ordinal {
        0 => "downward",
        1 => "pillar",
        2..=5 => "traverse",
        6..=9 => "ascend",
        10..=13 => "descend",
        14..=17 => "diagonal",
        18..=21 => "parkour",
        _ => panic!("no such move {ordinal}"),
    }
}

static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, ()> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| ())
}

struct Run {
    quiet: bool,
    tallies: BTreeMap<String, Tally>,
    shown: BTreeMap<String, u32>,
    mismatch_by_config: BTreeMap<usize, u64>,
    state_wrong_by_fn: [u64; 13],
    in_goal_true: BTreeMap<String, u64>,
    rust_panics: u64,
    /// Settings that no configuration of the file gives two values to.
    config_gaps: Vec<&'static str>,
}

impl Run {
    fn new(quiet: bool) -> Run {
        Run { quiet, tallies: BTreeMap::new(), shown: BTreeMap::new(), mismatch_by_config: BTreeMap::new(), state_wrong_by_fn: [0; 13], in_goal_true: BTreeMap::new(), rust_panics: 0, config_gaps: vec![] }
    }
    fn tally(&mut self, family: &str) -> &mut Tally {
        self.tallies.entry(family.to_string()).or_default()
    }
    /// Prints the first few mismatches of a family.
    fn show(&mut self, family: &str, msg: impl FnOnce() -> String) {
        let n = self.shown.entry(family.to_string()).or_default();
        if !self.quiet && *n < 6 {
            *n += 1;
            println!("MISMATCH {family}: {}", msg());
        }
    }
}

const PREDICATES: [&str; 13] = ["canWalkThrough", "canWalkOn", "fullyPassable", "canPlaceAgainst", "mustBeSolidToWalkOn", "isReplaceable", "canUseFrostWalker", "avoidBreaking", "avoidWalkingInto", "isFlowing", "miningTicks", "miningTicksFalling", "costOfPlacingAt"];
const STATE_FNS: [&str; 13] = ["canWalkThroughBlockState", "canWalkOnBlockState", "fullyPassableBlockState", "avoidWalkingInto", "isBlockNormalCube", "isWaterloggedSolidTop", "isClimbable", "isBottomSlab", "isWater", "isLava", "isLiquid", "possiblyFlowing", "isTransparent"];

fn ternary_char(t: Ternary) -> u8 {
    match t {
        Ternary::Yes => b'Y',
        Ternary::Maybe => b'M',
        Ternary::No => b'N',
    }
}

/// The engine's answers for a state, in the order and encoding of the Java side's `answers`.
fn rust_answers(s: &State, c: &Config) -> Vec<u8> {
    let f = |v: bool| if v { b'1' } else { b'0' };
    vec![ternary_char(can_walk_through_state(s, c)), ternary_char(can_walk_on_state(s, c)), ternary_char(fully_passable_state(s)), f(avoid_walking_into(s, c)), f(is_block_normal_cube(s)), f(is_waterlogged_solid_top(s)), f(is_climbable(s)), f(is_bottom_slab(s)), f(is_water(s)), f(is_lava(s)), f(is_liquid(s)), f(possibly_flowing(s)), f(is_transparent(s))]
}

/// A goal from the spec that the Java side writes: `name(args)`, ints separated by commas, goals nested in `composite` and `inverted`.
struct SpecParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl SpecParser<'_> {
    fn ident(&mut self) -> String {
        let st = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_lowercase() {
            self.i += 1;
        }
        String::from_utf8(self.s[st..self.i].to_vec()).unwrap()
    }
    fn expect(&mut self, c: u8) {
        assert_eq!(self.s.get(self.i), Some(&c), "goal spec: expected {} at {}", c as char, self.i);
        self.i += 1;
    }
    fn token(&mut self) -> String {
        let st = self.i;
        while self.i < self.s.len() && self.s[self.i] != b',' && self.s[self.i] != b')' {
            self.i += 1;
        }
        String::from_utf8(self.s[st..self.i].to_vec()).unwrap()
    }
    fn ints(&mut self, n: usize) -> Vec<i32> {
        let mut v = vec![];
        for k in 0..n {
            if k > 0 {
                self.expect(b',');
            }
            v.push(int(&self.token()));
        }
        v
    }
    fn goal(&mut self) -> Goal {
        let name = self.ident();
        self.expect(b'(');
        let g = match name.as_str() {
            "block" => {
                let a = self.ints(3);
                Goal::Block { x: a[0], y: a[1], z: a[2] }
            }
            "xz" => {
                let a = self.ints(2);
                Goal::Xz { x: a[0], z: a[1] }
            }
            "ylevel" => Goal::YLevel { level: self.ints(1)[0] },
            "twoblocks" => {
                let a = self.ints(3);
                Goal::TwoBlocks { x: a[0], y: a[1], z: a[2] }
            }
            "gettoblock" => {
                let a = self.ints(3);
                Goal::GetToBlock { x: a[0], y: a[1], z: a[2] }
            }
            "near" => {
                let a = self.ints(4);
                goal_near(a[0], a[1], a[2], a[3])
            }
            "axis" => Goal::Axis,
            "strict" => {
                let a = self.ints(5);
                goal_strict_direction(a[0], a[1], a[2], a[3], a[4])
            }
            "runaway" => {
                let dist = dbl(&self.token());
                self.expect(b',');
                let m = self.token();
                let maintain = if m == "n" { None } else { Some(int(&m)) };
                let mut from = vec![];
                while self.s[self.i] == b',' {
                    self.i += 1;
                    let x = int(&self.token());
                    self.expect(b',');
                    let y = int(&self.token());
                    self.expect(b',');
                    let z = int(&self.token());
                    from.push((x, y, z));
                }
                goal_run_away(dist, maintain, from)
            }
            "composite" => {
                let mut goals = vec![];
                while self.s[self.i] != b')' {
                    if !goals.is_empty() {
                        self.expect(b',');
                    }
                    goals.push(self.goal());
                }
                Goal::Composite(goals)
            }
            "inverted" => Goal::Inverted(Box::new(self.goal())),
            _ => panic!("unknown goal {name}"),
        };
        self.expect(b')');
        g
    }
}

fn parse_goal(spec: &str) -> Goal {
    let mut p = SpecParser { s: spec.as_bytes(), i: 0 };
    let g = p.goal();
    assert_eq!(p.i, spec.len(), "goal spec: trailing text");
    g
}

fn goal_family(spec: &str) -> String {
    format!("goal_{}", spec.split('(').next().unwrap())
}

fn goal_finite(v: f64) -> bool {
    v.is_finite() && v.abs() < f64::MAX
}

/// Reads a golden file and compares every line with the engine.
fn run<R: BufRead>(input: R, quiet: bool) -> Run {
    let mut run = Run::new(quiet);
    let mut border = Border { min_x: 0.0, max_x: 0.0, min_z: 0.0, max_z: 0.0 };
    let (mut w, mut h, mut d, mut sx, mut sz) = (0, 0, 0, 0, 0);
    let mut states: Vec<State> = vec![];
    let (mut str_pick, mut str_hand): (Vec<f64>, Vec<f64>) = (vec![], vec![]);
    let mut cfgs: Vec<Config> = vec![];
    let mut has_pick: Vec<bool> = vec![];
    let mut pres: Vec<Precomputed> = vec![];
    let mut air_id = 0;
    let mut reg = (0, 0);
    let (mut cells, mut loaded, mut y, mut grid_no) = (Vec::<u16>::new(), true, 0, 0u64);
    let (mut registry_states, mut goals, mut palette_checked) = (0u64, 0u64, false);
    let t0 = std::time::Instant::now();
    for line in input.lines() {
        let line = line.expect("read error");
        let p: Vec<&str> = line.split(' ').collect();
        match p[0] {
            "B" => {
                border = Border { min_x: dbl_dec(p[1]), max_x: dbl_dec(p[2]), min_z: dbl_dec(p[3]), max_z: dbl_dec(p[4]) };
                (w, h, d, sx, sz) = (int(p[5]), int(p[6]), int(p[7]), int(p[8]), int(p[9]));
                assert_eq!((w, h, d), (GRID_W, GRID_H, GRID_D), "the grid size changed; the Rust side has to follow");
            }
            "M" => {
                let ord: usize = p[1].parse().unwrap();
                assert_eq!(Moves::ALL[ord].name(), p[2], "Moves.values() and Moves::ALL disagree at {ord}");
                let m = Moves::ALL[ord];
                let java = (int(p[3]), int(p[4]), int(p[5]), bit(p[6]), bit(p[7]));
                assert_eq!((m.x_offset(), m.y_offset(), m.z_offset(), m.dynamic_xz(), m.dynamic_y()), java, "the offsets or dynamic flags of {} differ from upstream's", p[2]);
            }
            "S" => {
                let (s, pick, hand) = parse_state(p[2], &p[3..]);
                assert_eq!(int(p[1]) as usize, states.len());
                states.push(s);
                str_pick.push(pick);
                str_hand.push(hand);
            }
            "C" => {
                let (c, pick) = parse_config(&p);
                assert_eq!(int(p[1]) as usize, cfgs.len());
                cfgs.push(c);
                has_pick.push(pick);
            }
            "F" => {
                let table = fall_n_blocks_cost();
                assert_eq!(p.len(), table.len() + 2, "the fall table has another length");
                for (i, v) in p[1..=table.len()].iter().enumerate() {
                    run.tally("fall_table").add(dbl(v), table[i], true, true);
                }
                run.tally("fall_table").add(dbl(p[table.len() + 1]), jump_one_block_cost(), true, true);
            }
            "Z" => reg = (p[1].parse::<usize>().unwrap(), p[2].parse::<usize>().unwrap()),
            "A" => {
                let (kvs, answers): (Vec<&str>, Vec<&str>) = p[3..].iter().copied().partition(|kv| !kv.starts_with("r0=") && !kv.starts_with("r1="));
                let (s, _, _) = parse_state(p[2], &kvs);
                registry_states += 1;
                for (k, a) in answers.iter().enumerate() {
                    assert_eq!(&a[..3], format!("r{k}=").as_str());
                    let java = &a.as_bytes()[3..];
                    let c = &cfgs[if k == 0 { reg.0 } else { reg.1 }];
                    let rust = rust_answers(&s, c);
                    for f in 0..13 {
                        let (jv, rv) = (java[f], rust[f]);
                        let val = |v: u8| {
                            if v == b'Y' || v == b'1' {
                                2.0
                            } else if v == b'M' {
                                1.0
                            } else {
                                0.0
                            }
                        };
                        let ok = run.tally("states").add(val(jv), val(rv), jv == rv, jv == b'Y' || jv == b'1' || jv == b'M');
                        if !ok {
                            run.state_wrong_by_fn[f] += 1;
                            run.show("states", || format!("state {} {} under config {}: {} java={} rust={}", p[1], p[2], k, STATE_FNS[f], jv as char, rv as char));
                        }
                    }
                }
            }
            "Q" => {
                goals += 1;
                let ci: usize = p[1].parse().unwrap();
                let fam = goal_family(p[2]);
                let gs = cfgs[ci].goal_settings();
                let parsed = guarded(|| parse_goal(p[2]));
                let g = parsed.unwrap_or_else(|_| panic!("could not parse the goal {}", p[2]));
                if p[3] != "-" {
                    let jv = dbl(p[3]);
                    let rv = guarded(|| g.heuristic_inside(&gs));
                    let ok = match rv {
                        Ok(v) => run.tally(&fam).add(jv, v, true, goal_finite(jv)),
                        Err(()) => {
                            run.rust_panics += 1;
                            run.tally(&fam).add(jv, f64::from_bits(0x7ff0_dead_0000_0001), true, goal_finite(jv));
                            false
                        }
                    };
                    if !ok {
                        run.show(&fam, || format!("{} heuristic() java={jv} rust={rv:?}", p[2]));
                    }
                }
                for pos in p[4..].chunks(5) {
                    let (x, yy, z) = (int(pos[0]), int(pos[1]), int(pos[2]));
                    let in_java = bit(pos[3]);
                    let hj = dbl(pos[4]);
                    let r_in = guarded(|| g.is_in_goal(x, yy, z, &gs));
                    let r_h = guarded(|| g.heuristic(x, yy, z, &gs));
                    if r_in.is_err() || r_h.is_err() {
                        run.rust_panics += 1;
                    }
                    let in_ok = run.tally(&fam).add_bool(in_java, r_in.unwrap_or(!in_java));
                    if in_java {
                        *run.in_goal_true.entry(fam.clone()).or_default() += 1;
                    }
                    let h_ok = run.tally(&fam).add(hj, r_h.unwrap_or(f64::from_bits(0x7ff0_dead_0000_0001)), true, goal_finite(hj));
                    if !in_ok || !h_ok {
                        run.show(&fam, || format!("{} at ({x},{yy},{z}) isInGoal java={in_java} rust={r_in:?}; heuristic java={hj} rust={r_h:?}", p[2]));
                    }
                }
            }
            "W" => border = Border { min_x: dbl_dec(p[1]), max_x: dbl_dec(p[2]), min_z: dbl_dec(p[3]), max_z: dbl_dec(p[4]) },
            "L" => loaded = bit(p[1]),
            "G" => {
                if pres.is_empty() {
                    pres = cfgs.iter().map(|c| Precomputed::new(&states, c)).collect();
                    air_id = pres[0].air();
                }
                if !palette_checked {
                    palette_checked = true;
                    run.config_gaps = config_gaps(&cfgs);
                }
                grid_no += 1;
                y = int(p[2]);
                assert_eq!(p[3].len() as i32, 2 * w * h * d, "grid of the wrong size");
                cells = p[3].as_bytes().chunks(2).map(|c| u16::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap()).collect();
            }
            "R" => {
                let ci: usize = p[1].parse().unwrap();
                let ord: usize = p[2].parse().unwrap();
                let (jx, jy, jz, jc) = (int(p[3]), int(p[4]), int(p[5]), dbl(p[6]));
                let world = GridWorld { w, h, d, cells: &cells, air: air_id, loaded };
                let tools = if has_pick[ci] { &str_pick } else { &str_hand };
                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);
                let mut res = MoveResult::default();
                let m = Moves::ALL[ord];
                let fam = family(ord);
                match guarded(|| m.apply(&ctx, sx, y, sz, &mut res)) {
                    Ok(()) => {
                        let same = (jx, jy, jz) == (res.x, res.y, res.z);
                        if !run.tally(fam).add(jc, res.cost, same, jc < COST_INF) {
                            *run.mismatch_by_config.entry(ci).or_default() += 1;
                            run.show(fam, || format!("grid {grid_no} y={y} config {ci} {}: java=({jx},{jy},{jz}) {jc} rust=({},{},{}) {}", m.name(), res.x, res.y, res.z, res.cost));
                        }
                    }
                    Err(()) => {
                        run.rust_panics += 1;
                        run.tally(fam).add(jc, f64::from_bits(0x7ff0_dead_0000_0001), false, jc < COST_INF);
                        *run.mismatch_by_config.entry(ci).or_default() += 1;
                        run.show(fam, || format!("grid {grid_no} y={y} config {ci} {}: the engine panicked", m.name()));
                    }
                }
            }
            "P" => {
                let ci: usize = p[1].parse().unwrap();
                let (x, yy, z) = (int(p[2]), int(p[3]), int(p[4]));
                let bits = p[5].as_bytes();
                let (m0, m1, plc) = (dbl(p[6]), dbl(p[7]), dbl(p[8]));
                let world = GridWorld { w, h, d, cells: &cells, air: air_id, loaded };
                let tools = if has_pick[ci] { &str_pick } else { &str_hand };
                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);
                let got = guarded(|| {
                    let b = ctx.get(x, yy, z);
                    let flags = [ctx.can_walk_through_at(x, yy, z), ctx.can_walk_on_at(x, yy, z), ctx.fully_passable_at(x, yy, z), ctx.can_place_against(x, yy, z), ctx.must_be_solid_to_walk_on(x, yy, z, &b), ctx.is_replaceable(x, z, &b), ctx.can_use_frost_walker(&b), ctx.avoid_breaking(x, yy, z, b), avoid_walking_into(&b, &cfgs[ci]), ctx.is_flowing(x, yy, z, &b)];
                    (flags, ctx.mining_duration_at(x, yy, z, false), ctx.mining_duration_at(x, yy, z, true), ctx.cost_of_placing_at(x, yy, z, b))
                });
                match got {
                    Ok((flags, r0, r1, rp)) => {
                        for (k, f) in flags.iter().enumerate() {
                            if !run.tally("predicates").add_bool(bit(&p[5][k..k + 1]), *f) {
                                run.show("predicates", || format!("grid {grid_no} config {ci} at ({x},{yy},{z}): {} java={} rust={f}", PREDICATES[k], bits[k] as char));
                            }
                        }
                        for (k, (jv, rv)) in [(m0, r0), (m1, r1), (plc, rp)].into_iter().enumerate() {
                            if !run.tally("predicates").add(jv, rv, true, jv < COST_INF) {
                                run.show("predicates", || format!("grid {grid_no} config {ci} at ({x},{yy},{z}): {} java={jv} rust={rv}", PREDICATES[10 + k]));
                            }
                        }
                    }
                    Err(()) => {
                        run.rust_panics += 1;
                        for _ in 0..13 {
                            run.tally("predicates").add(0.0, f64::from_bits(0x7ff0_dead_0000_0001), false, false);
                        }
                        run.show("predicates", || format!("grid {grid_no} config {ci} at ({x},{yy},{z}): the engine panicked"));
                    }
                }
            }
            other => panic!("bad line type {other}"),
        }
    }
    if !quiet {
        println!("mismatches in movements by config {:?}", run.mismatch_by_config);
        println!("registry sweep wrong by function {:?}", STATE_FNS.iter().zip(run.state_wrong_by_fn.iter()).filter(|(_, n)| **n > 0).collect::<Vec<_>>());
        println!("isInGoal true by goal {:?}", run.in_goal_true);
        println!("engine panics {} {:?}", run.rust_panics, PANICS.lock().unwrap().iter().take(5).collect::<Vec<_>>());
        println!("palette {} states, {} configs, {} grids, {} registry states, {} goals, {:?}", states.len(), cfgs.len(), grid_no, registry_states, goals, t0.elapsed());
    }
    run
}

/// The world border is written with Java's `double` text form, such as `-2.9999984E7`.
fn dbl_dec(v: &str) -> f64 {
    v.parse().unwrap_or_else(|_| panic!("not a number: {v}"))
}

static STOP_AT_FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let mut l = PANICS.lock().unwrap();
        if l.len() < 20 {
            l.push(info.to_string());
        }
    }));
    let path = std::env::args().nth(1).expect("usage: cost-compare GOLDEN_FILE [quiet [first]]");
    let quiet = std::env::args().nth(2).as_deref() == Some("quiet");
    STOP_AT_FIRST.store(std::env::args().nth(3).as_deref() == Some("first"), std::sync::atomic::Ordering::Relaxed);
    let file = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path).expect("cannot open the golden file"));
    let run = run(file, quiet);
    for (name, t) in &run.tallies {
        t.print(name);
    }
    if !run.config_gaps.is_empty() {
        println!("CONFIG COVERAGE MISSING: no two different values for {:?}", run.config_gaps);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_exact_close_wrong() {
        let mut t = Tally::default();
        assert!(t.add(1.5, 1.5, true, true));
        assert!(t.add(f64::NAN, f64::from_bits(0xfff8_0000_0000_0000), true, false));
        assert!(t.add(100.0, 100.0 + 1e-8, true, true));
        assert!(!t.add(1.0, 1.0, false, true));
        assert!(!t.add(0.0, -0.0, true, false));
        assert_eq!((t.evals, t.exact, t.close, t.wrong, t.finite), (5, 2, 1, 2, 3));
        assert_eq!(t.distinct.len(), 5);
    }

    #[test]
    fn families_cover_every_move() {
        let names: BTreeSet<&str> = (0..Moves::ALL.len()).map(family).collect();
        assert_eq!(names.len(), 7);
        assert_eq!(family(Moves::ParkourWest as usize), "parkour");
        assert_eq!(family(Moves::DiagonalNortheast as usize), "diagonal");
    }

    #[test]
    fn goal_specs() {
        assert_eq!(parse_goal("block(1,-2,3)"), Goal::Block { x: 1, y: -2, z: 3 });
        assert_eq!(parse_goal("axis()"), Goal::Axis);
        assert_eq!(parse_goal("near(0,0,0,65536)"), Goal::Near { x: 0, y: 0, z: 0, range_sq: 0 });
        assert_eq!(parse_goal("strict(1,2,3,0,-1)"), Goal::StrictDirection { x: 1, y: 2, z: 3, dx: 0, dz: -1 });
        let run_away = parse_goal("runaway(4010000000000000,n,1,2,3,4,5,6)");
        assert_eq!(run_away, Goal::RunAway { from: vec![(1, 2, 3), (4, 5, 6)], distance_sq: 16, maintain_y: None });
        assert_eq!(parse_goal("runaway(4008000000000000,7,0,0,0)"), Goal::RunAway { from: vec![(0, 0, 0)], distance_sq: 9, maintain_y: Some(7) });
        let nested = parse_goal("composite(block(0,0,0),inverted(composite()),xz(1,2))");
        assert_eq!(nested, Goal::Composite(vec![Goal::Block { x: 0, y: 0, z: 0 }, Goal::Inverted(Box::new(Goal::Composite(vec![]))), Goal::Xz { x: 1, z: 2 }]));
        assert_eq!(goal_family("composite(block(0,0,0))"), "goal_composite");
    }

    #[test]
    #[should_panic(expected = "trailing text")]
    fn goal_spec_trailing_text() {
        parse_goal("xz(1,2)x");
    }

    fn config_line(extra: &str) -> Vec<String> {
        let base = "C 0 pick=1 throwaway=1 bucket=0 sprint=1 frost=0 watermult=3f800000 allowbreak=1 anyway=- avoid=tripwire disallow=- walkwater=0 walklava=0 magma=0 vines=0 bottomslab=1 strictliquid=0 avoidfalling=1 placesource=1 placeflow=1 swim=0 parkour=0 parkourplace=0 parkourascend=1 buildlimit=0 diagdescend=0 diagascend=0 downward=1 maxfall=3 maxfallbucket=20 place=4034000000000000 break=4000000000000000 jump=4000000000000000 waterwalk=4008000000000000 costheuristic=400c40c49ba5e354 axisheight=120";
        format!("{base} {extra}").split(' ').map(|s| s.to_string()).collect()
    }

    #[test]
    fn config_line_parses_and_unknown_settings_fail() {
        let v = config_line("");
        let p: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
        let (c, pick) = parse_config(&p[..p.len() - 1]);
        assert!(pick && c.has_throwaway && !c.allow_parkour && c.allow_parkour_ascend);
        assert_eq!((c.max_fall_no_water, c.max_fall_bucket, c.axis_height), (3, 20, 120));
        assert_eq!(c.blocks_to_avoid, vec!["tripwire".to_string()]);
        assert_eq!(c.place_penalty, 20.0);
        let caught = std::panic::catch_unwind(|| {
            let v = config_line("surprise=1");
            let p: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
            parse_config(&p);
        });
        assert!(caught.is_err());
    }

    #[test]
    fn config_gaps_are_found() {
        let v = config_line("");
        let p: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
        let (a, _) = parse_config(&p[..p.len() - 1]);
        let b = a.clone();
        let gaps = config_gaps(&[a.clone(), b]);
        assert!(gaps.contains(&"allow_parkour") && gaps.contains(&"jump_penalty") && gaps.contains(&"blocks_to_avoid"));
        let mut c = a.clone();
        c.allow_parkour = !a.allow_parkour;
        let gaps = config_gaps(&[a, c]);
        assert!(!gaps.contains(&"allow_parkour"));
    }

    #[test]
    fn state_line_parses() {
        let (s, pick, hand) = parse_state("oak_slab", &["slab=1", "slab_type=TOP", "fluid=water", "level=-1", "door_facing=-", "str_pick=3ff0000000000000", "str_hand=0"]);
        assert!(s.slab && s.slab_type == SlabType::Top && s.fluid == Fluid::Water);
        assert_eq!((pick, hand), (1.0, 0.0));
        assert_eq!(s.block, Block::Other);
        assert_eq!(parse_state("soul_sand", &[]).0.block, Block::SoulSand);
    }

    /// The whole reading path on a tiny file: an all-air grid, where moving down from the start is impossible.
    #[test]
    fn plumbing_on_a_tiny_file() {
        let v = config_line("");
        let c_line = v[..v.len() - 1].join(" ");
        let air = "S 0 air air=1 fluid=none fluid_empty=1 pathfindable=1 replaceable=1 level=-1 str_pick=0 str_hand=0";
        let cells = "00".repeat((GRID_W * GRID_H * GRID_D) as usize);
        let inf = format!("{:x}", COST_INF.to_bits());
        let one = format!("{:x}", 1.0f64.to_bits());
        let text = format!("B -29999984 29999984 -29999984 29999984 13 16 13 6 6\nM 0 DOWNWARD 0 -1 0 0 0\n{air}\n{c_line}\nL 1\nG 0 5 {cells}\nR 0 0 6 4 6 {inf}\nR 0 0 6 4 6 {one}\n");
        let run = run(text.as_bytes(), true);
        let t = &run.tallies["downward"];
        assert_eq!((t.evals, t.exact, t.wrong), (2, 1, 1));
    }
}
