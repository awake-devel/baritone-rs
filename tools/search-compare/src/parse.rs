// baritone-rs test tool: the parts of tools/cost-compare/src/main.rs that read the golden file's palette, configuration and goal lines.
// LGPL-3.0-or-later, see LICENSE. These functions are copied from there unchanged (the format of the S and C lines and the goal
// specs is the same file format); cost-compare is a binary crate, so they cannot be shared without changing it.
use baritone_engine::blocks::*;
use baritone_engine::config::Config;
use baritone_engine::goals::*;
use std::collections::BTreeMap;

pub fn bit(v: &str) -> bool {
    match v {
        "1" => true,
        "0" => false,
        _ => panic!("not a flag: {v}"),
    }
}

pub fn dbl(v: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(v, 16).unwrap_or_else(|_| panic!("not a hex double: {v}")))
}

pub fn int(v: &str) -> i32 {
    v.parse().unwrap_or_else(|_| panic!("not an int: {v}"))
}

pub fn list(v: &str) -> Vec<String> {
    if v == "-" {
        vec![]
    } else {
        v.split(',').map(|s| s.to_string()).collect()
    }
}

pub fn parse_fluid(v: &str) -> Fluid {
    match v {
        "none" => Fluid::None,
        "water" => Fluid::Water,
        "flowing_water" => Fluid::FlowingWater,
        "lava" => Fluid::Lava,
        "flowing_lava" => Fluid::FlowingLava,
        _ => panic!("unknown fluid {v}"),
    }
}

pub fn parse_slab_type(v: &str) -> SlabType {
    match v {
        "BOTTOM" | "-" => SlabType::Bottom,
        "TOP" => SlabType::Top,
        "DOUBLE" => SlabType::Double,
        _ => panic!("unknown slab type {v}"),
    }
}

pub fn parse_half(v: &str) -> Half {
    match v {
        "BOTTOM" | "-" => Half::Bottom,
        "TOP" => Half::Top,
        _ => panic!("unknown half {v}"),
    }
}

pub fn parse_shape(v: &str) -> StairShape {
    match v {
        "STRAIGHT" | "-" => StairShape::Straight,
        "INNER_LEFT" => StairShape::InnerLeft,
        "INNER_RIGHT" => StairShape::InnerRight,
        "OUTER_LEFT" => StairShape::OuterLeft,
        "OUTER_RIGHT" => StairShape::OuterRight,
        _ => panic!("unknown stair shape {v}"),
    }
}

pub fn parse_direction(v: &str) -> Direction {
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
pub fn parse_state(name: &str, kvs: &[&str]) -> (State, f64, f64) {
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
pub fn parse_config(p: &[&str]) -> (Config, bool) {
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

pub fn parse_goal(spec: &str) -> Goal {
    let mut p = SpecParser { s: spec.as_bytes(), i: 0 };
    let g = p.goal();
    assert_eq!(p.i, spec.len(), "goal spec: trailing text");
    g
}

pub fn dbl_dec(v: &str) -> f64 {
    v.parse().unwrap_or_else(|_| panic!("not a number: {v}"))
}
