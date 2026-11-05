// baritone-rs test tool: compares the translation against costs computed by upstream. Same license as lib.rs.
use baritone_spike::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};

fn bit(v: &str) -> bool {
    v == "1"
}
fn dbl(v: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(v, 16).unwrap())
}

fn parse_state(p: &[&str]) -> State {
    let mut s = State { block: p[2].to_string(), ..Default::default() };
    for kv in &p[3..] {
        let (k, v) = kv.split_once('=').unwrap();
        match k {
            "air" => s.air = bit(v),
            "base_fire" => s.base_fire = bit(v),
            "skull" => s.skull = bit(v),
            "skull_block" => s.skull_block = bit(v),
            "shulker" => s.shulker = bit(v),
            "slab" => s.slab = bit(v),
            "trapdoor" => s.trapdoor = bit(v),
            "amethyst" => s.amethyst = bit(v),
            "azalea" => s.azalea = bit(v),
            "door" => s.door = bit(v),
            "wooden_door" => s.wooden_door = bit(v),
            "gate" => s.gate = bit(v),
            "carpet" => s.carpet = bit(v),
            "snow" => s.snow = bit(v),
            "cauldron" => s.cauldron = bit(v),
            "stair" => s.stair = bit(v),
            "stained_glass" => s.stained_glass = bit(v),
            "liquid_block" => s.liquid_block = bit(v),
            "infested" => s.infested = bit(v),
            "lily" => s.lily = bit(v),
            "falling" => s.falling = bit(v),
            "leaves" => s.leaves = bit(v),
            "pathfindable" => s.pathfindable = bit(v),
            "normal_cube" => s.normal_cube = bit(v),
            "waterlogged_solid_top" => s.waterlogged_solid_top = bit(v),
            "replaceable" => s.replaceable = bit(v),
            "falling_free" => s.falling_free = bit(v),
            "frost_melt" => s.frost_melt = bit(v),
            "fluid" => s.fluid = v.to_string(),
            "fluid_empty" => s.fluid_empty = bit(v),
            "fluid_amount" => s.fluid_amount = v.parse().unwrap(),
            "fluid_source" => s.fluid_source = bit(v),
            "fluid_flowing_kind" => s.fluid_flowing_kind = bit(v),
            "level" => s.level = v.parse().unwrap(),
            "slab_type" => s.slab_type = v.to_string(),
            "stair_half" => s.stair_half = v.to_string(),
            "stair_shape" => s.stair_shape = v.to_string(),
            "trap_open" => s.trap_open = bit(v),
            "trap_half" => s.trap_half = v.to_string(),
            "layers" => s.layers = v.parse().unwrap(),
            "str_pick" => s.str_pick = dbl(v),
            "str_hand" => s.str_hand = dbl(v),
            _ => panic!("unknown fact {k}"),
        }
    }
    s
}

#[derive(Default)]
struct Tally {
    evals: u64,
    exact: u64,
    close: u64,
    wrong: u64,
    finite: u64,
    distinct: BTreeMap<u64, u64>,
}

impl Tally {
    fn add(&mut self, java: f64, rust: f64, same_dest: bool) -> bool {
        self.evals += 1;
        if java < COST_INF {
            self.finite += 1;
        }
        *self.distinct.entry(java.to_bits()).or_default() += 1;
        if java.to_bits() == rust.to_bits() && same_dest {
            self.exact += 1;
            true
        } else if same_dest && (java - rust).abs() <= 1e-9 * java.abs().max(1.0) {
            self.close += 1;
            true
        } else {
            self.wrong += 1;
            false
        }
    }
    fn print(&self, n: &str) {
        println!("{n}: {} evals, {} finite, {} distinct, exact {}, within 1e-9 {}, WRONG {}", self.evals, self.finite, self.distinct.len(), self.exact, self.close, self.wrong);
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("golden file");
    let quiet = std::env::args().nth(2).as_deref() == Some("quiet");
    let f = BufReader::new(std::fs::File::open(path).unwrap());
    let mut border = Border { min_x: 0.0, max_x: 0.0, min_z: 0.0, max_z: 0.0 };
    let (mut w, mut h, mut d) = (0, 0, 0);
    let (mut place, mut brk, mut waterwalk, mut bucket) = (0.0, 0.0, 0.0, 0);
    let mut states: Vec<State> = vec![];
    let mut configs: Vec<Config> = vec![];
    let mut cells: Vec<u16> = vec![];
    let mut y = 0;
    let (mut trav, mut desc) = (Tally::default(), Tally::default());
    let mut shown = 0;
    let mut by_cfg: BTreeMap<usize, u64> = BTreeMap::new();
    let t0 = std::time::Instant::now();
    for line in f.lines() {
        let line = line.unwrap();
        let p: Vec<&str> = line.split(' ').collect();
        match p[0] {
            "B" => {
                border = Border { min_x: p[1].parse().unwrap(), max_x: p[2].parse().unwrap(), min_z: p[3].parse().unwrap(), max_z: p[4].parse().unwrap() };
                w = p[5].parse().unwrap();
                h = p[6].parse().unwrap();
                d = p[7].parse().unwrap();
            }
            "X" => {
                for kv in &p[1..] {
                    let (k, v) = kv.split_once('=').unwrap();
                    match k {
                        "place" => place = v.parse().unwrap(),
                        "break" => brk = v.parse().unwrap(),
                        "waterwalk" => waterwalk = v.parse().unwrap(),
                        "bucket" => bucket = v.parse().unwrap(),
                        _ => panic!(),
                    }
                }
            }
            "S" => states.push(parse_state(&p)),
            "C" => configs.push(Config {
                pick: bit(p[2]),
                has_throwaway: bit(p[3]),
                has_water_bucket: bit(p[4]),
                can_sprint: bit(p[5]),
                frost_walker: p[6].parse().unwrap(),
                water_mult: f32::from_bits(u32::from_str_radix(p[7], 16).unwrap()),
                allow_break: bit(p[8]),
                assume_walk_on_water: bit(p[9]),
                allow_walk_on_magma: bit(p[10]),
                allow_vines: bit(p[11]),
                allow_walk_on_bottom_slab: bit(p[12]),
                strict_liquid_check: bit(p[13]),
                avoid_updating_falling: bit(p[14]),
                allow_place_source: bit(p[15]),
                allow_place_flow: bit(p[16]),
                allow_swimming: bit(p[17]),
                max_fall_no_water: p[18].parse().unwrap(),
                max_fall_bucket: bucket,
                place_penalty: place,
                break_penalty: brk,
                walk_on_water_penalty: waterwalk,
            }),
            "G" => cells = p[1..].iter().map(|v| v.parse().unwrap()).collect(),
            "Y" => y = p[1].parse().unwrap(),
            "R" => {
                let ci: usize = p[1].parse().unwrap();
                let dx: i32 = p[2].parse().unwrap();
                let dz: i32 = p[3].parse().unwrap();
                let tj = dbl(p[4]);
                let (rx, ry, rz): (i32, i32, i32) = (p[5].parse().unwrap(), p[6].parse().unwrap(), p[7].parse().unwrap());
                let cj = dbl(p[8]);
                let ctx = Ctx::new(World { w, h, d, cells: &cells, min_y: 0 }, &states, &configs[ci], border);
                let tr = ctx.traverse_cost(2, y, 2, dx, dz);
                let mut res = MoveResult::default();
                ctx.descend_cost(2, y, 2, dx, dz, &mut res);
                let t_ok = trav.add(tj, tr, true);
                let d_same = (rx, ry, rz) == (res.x, res.y, res.z);
                let d_ok = desc.add(cj, res.cost, d_same);
                if !t_ok || !d_ok {
                    *by_cfg.entry(ci).or_default() += 1;
                    if shown < 12 && !quiet {
                        shown += 1;
                        println!("MISMATCH cfg={ci} y={y} dest=({dx},{dz}) traverse java={tj} rust={tr} | descend java=({rx},{ry},{rz},{cj}) rust=({},{},{},{})", res.x, res.y, res.z, res.cost);
                    }
                }
            }
            _ => panic!("bad line {}", p[0]),
        }
    }
    trav.print("traverse");
    desc.print("descend");
    println!("mismatches by config {by_cfg:?}");
    println!("palette {} states, {} configs, {:?}", states.len(), configs.len(), t0.elapsed());
}
