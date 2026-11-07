// baritone-rs benchmark tool, Rust half: times baritone-engine's search on the worlds that java/baritone/pathing/calc/SearchBench.java
// generated and wrote. LGPL-3.0-or-later, see LICENSE.
//
//   search-bench finish DIR [TAG]      times every case of DIR/world-*.txt that DIR/java-finish<TAG>.txt has, with the same
//                                      repetitions; writes DIR/rust-finish<TAG>.txt
//   search-bench limit DIR LIST        times the searches of LIST ("id primary failure reps" per line) under those clock limits;
//                                      writes DIR/rust-limit.txt
//   search-bench check DIR             parses every world file and prints its hash (a self test of the reader)
//
// The lines it writes have the format of the Java side's (T lines), so that run_bench.py compares them field by field.
// This file holds no search code: it builds the engine's context for the file's world and calls create_pathfinder and calculate.
#[allow(dead_code)]
#[path = "../../search-compare/src/parse.rs"]
mod parse;

use baritone_engine::blocks::State;
use baritone_engine::config::{Config, SearchSettings};
use baritone_engine::goals::Goal;
use baritone_engine::movement::helper::{Border, Ctx, Precomputed};
use baritone_engine::movement::step::MovementKind;
use baritone_engine::plan::create_pathfinder;
use baritone_engine::pos::Pos;
use baritone_engine::search::{Avoidance, Path, ResultType, SearchLimits};
use baritone_engine::world::WorldView;
use parse::*;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::time::Instant;

/// The world of the file: a box of `n` x `n` columns at the origin. Every chunk counts as loaded, also outside the box (air there): the
/// world border, which is the box, rejects the moves that leave it. The Java side's `BWorld` does the same.
struct BWorld {
    n: i32,
    h: i32,
    cells: Vec<u16>,
    air: usize,
    border: Border,
}

impl WorldView for BWorld {
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
        if x < 0 || x >= self.n || z < 0 || z >= self.n {
            return self.air;
        }
        self.cells[((x * self.n + z) * self.h + y) as usize] as usize
    }
    fn contains_loaded_chunk(&self, _x: i32, _z: i32) -> bool {
        true
    }
    fn is_loaded(&self, _x: i32, _z: i32) -> bool {
        true
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        self.h
    }
}

fn kv(p: &[&str]) -> HashMap<String, String> {
    p.iter()
        .map(|item| {
            let (k, v) = item.split_once('=').unwrap_or_else(|| panic!("not a key=value item: {item}"));
            (k.to_string(), v.to_string())
        })
        .collect()
}

fn fnv_cells(cells: &[u16]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &c in cells {
        h ^= c as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn fnv_text(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

struct Case {
    id: String,
    goal: Goal,
    start: Pos,
    stress: bool,
}

struct WorldFile {
    seed: u64,
    hash: u64,
    world: BWorld,
    cases: Vec<Case>,
    states: Vec<State>,
    pre: Precomputed,
    cfg: Config,
    tools: Vec<f64>,
}

fn pos3(s: &str) -> Pos {
    let v: Vec<i32> = s.split(',').map(int).collect();
    Pos::new(v[0], v[1], v[2])
}

fn load(path: &std::path::Path) -> WorldFile {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut states: Vec<State> = vec![];
    let (mut str_pick, mut str_hand): (Vec<f64>, Vec<f64>) = (vec![], vec![]);
    let mut cfg: Option<(Config, bool)> = None;
    let mut k: Option<SearchSettings> = None;
    let mut costh: Option<f64> = None;
    let mut world: Option<BWorld> = None;
    let mut cases = vec![];
    let (mut seed, mut hash) = (0u64, 0u64);
    let mut expected_hash = 0u64;
    for line in text.lines() {
        let p: Vec<&str> = line.split(' ').collect();
        match p[0] {
            "S" => {
                let (s, pick, hand) = parse_state(p[2], &p[3..]);
                assert_eq!(int(p[1]) as usize, states.len());
                states.push(s);
                str_pick.push(pick);
                str_hand.push(hand);
            }
            "C" => {
                assert_eq!(int(p[1]), 0, "the benchmark has one configuration");
                cfg = Some(parse_config(&p));
            }
            "K" => {
                assert_eq!(int(p[1]), 0);
                let m = kv(&p[2..]);
                k = Some(SearchSettings {
                    backtrack_cost_favoring_coefficient: dbl(&m["backtrack"]),
                    pathing_max_chunk_border_fetch: int(&m["borderfetch"]),
                    minimum_improvement_repropagation: bit(&m["minimprove"]),
                    path_cutoff_minimum_length: int(&m["cutmin"]),
                    path_cutoff_factor: dbl(&m["cutfactor"]),
                    cutoff_at_load_boundary: bit(&m["loadboundary"]),
                    simplify_unloaded_y_coord: bit(&m["simplify"]),
                    avoidance: bit(&m["avoidance"]),
                    mob_spawner_avoidance_coefficient: dbl(&m["spawnercoeff"]),
                    mob_spawner_avoidance_radius: int(&m["spawnerradius"]),
                    mob_avoidance_coefficient: dbl(&m["mobcoeff"]),
                    mob_avoidance_radius: int(&m["mobradius"]),
                    ..SearchSettings::default()
                });
                costh = if m["costh"] == "-" { None } else { Some(dbl(&m["costh"])) };
            }
            "W" => {
                let m = kv(&p[1..]);
                seed = m["seed"].parse().unwrap();
                expected_hash = u64::from_str_radix(&m["hash"], 16).unwrap();
            }
            "V" => {
                let m = kv(&p[2..]);
                let (n, h, d) = (int(&m["w"]), int(&m["h"]), int(&m["d"]));
                assert_eq!(n, d, "the benchmark worlds are square");
                assert_eq!((int(&m["ox"]), int(&m["oz"]), int(&m["miny"])), (0, 0, 0));
                let b: Vec<f64> = m["border"].split(',').map(dbl_dec).collect();
                let mut cells = Vec::with_capacity((n * h * d) as usize);
                for tok in m["cells"].split(',') {
                    let (idx, run) = match tok.split_once('x') {
                        Some((i, r)) => (i, r.parse::<usize>().expect("run length")),
                        None => (tok, 1),
                    };
                    cells.extend(std::iter::repeat_n(idx.parse::<u16>().expect("palette index"), run));
                }
                assert_eq!(cells.len(), (n * h * d) as usize, "the cells of the world do not fill it");
                hash = fnv_cells(&cells);
                world = Some(BWorld { n, h, cells, air: 0, border: Border { min_x: b[0], max_x: b[1], min_z: b[2], max_z: b[3] } });
            }
            "B" => {
                let m = kv(&p[1..]);
                cases.push(Case { id: m["id"].clone(), goal: parse_goal(&m["goal"]), start: pos3(&m["start"]), stress: m["stress"] == "1" });
            }
            _ => {}
        }
    }
    assert_eq!(hash, expected_hash, "the world read from {} is not the world the Java side hashed", path.display());
    let (mut cfg, has_pick) = cfg.expect("a C line");
    cfg.search = k.expect("a K line");
    if let Some(h) = costh {
        cfg.cost_heuristic = h;
    }
    let pre = Precomputed::new(&states, &cfg);
    let mut world = world.expect("a V line");
    world.air = pre.air();
    let tools = if has_pick { str_pick } else { str_hand };
    WorldFile { seed, hash, world, cases, states, pre, cfg, tools }
}

fn kind_name(k: &MovementKind) -> &'static str {
    match k {
        MovementKind::Downward => "Downward",
        MovementKind::Pillar => "Pillar",
        MovementKind::Traverse => "Traverse",
        MovementKind::Ascend => "Ascend",
        MovementKind::Descend => "Descend",
        MovementKind::Fall => "Fall",
        MovementKind::Diagonal => "Diagonal",
        MovementKind::Parkour { .. } => "Parkour",
    }
}

fn type_name(k: ResultType) -> &'static str {
    match k {
        ResultType::SuccessToGoal => "SUCCESS_TO_GOAL",
        ResultType::SuccessSegment => "SUCCESS_SEGMENT",
        ResultType::Failure => "FAILURE",
        ResultType::Cancellation => "CANCELLATION",
        ResultType::Exception => "EXCEPTION",
    }
}

fn xyz(p: Pos) -> String {
    format!("{},{},{}", p.x, p.y, p.z)
}

/// The text of a path the way the Java side writes it (`SearchGoldenDump.pathText` with movements); its hash tells whether two paths are the same.
fn path_text(p: Option<&Path>) -> String {
    let Some(p) = p else {
        return "-".to_string();
    };
    let mut s = format!("P{}/{}/{}", p.positions().len(), p.num_nodes_considered(), xyz(p.src()));
    for q in p.positions() {
        s.push(' ');
        s.push_str(&xyz(*q));
    }
    s.push_str(" |");
    for m in p.movements().expect("a result path is verified") {
        let l = if m.calculated_while_loaded == Some(true) { 't' } else { 'f' };
        let _ = write!(s, " {}:{}:{:x}:{}", kind_name(&m.kind), xyz(m.dest), m.cost.map(f64::to_bits).unwrap_or(u64::MAX), l);
    }
    s
}

struct Run {
    ns: u128,
    kind: &'static str,
    nodes: i32,
    moves: i32,
    map: usize,
    plen: usize,
    phash: u64,
    best: Pos,
    besth: f64,
}

fn run_once(wf: &WorldFile, c: &Case, primary: i64, failure: i64) -> Run {
    let ctx = Ctx::new(&wf.world, &wf.states, &wf.pre, &wf.cfg, wf.world.border, &wf.tools);
    let gs = wf.cfg.goal_settings();
    let t0 = Instant::now();
    let avoid = Avoidance::create(&wf.cfg.search, &[], &[]);
    let mut finder = create_pathfinder(c.start, &c.goal, None, &ctx, c.start, &avoid);
    let clock_start = Instant::now();
    let mut clock = move || clock_start.elapsed().as_millis() as i64;
    let res = finder.calculate(primary, failure, &SearchLimits::default(), &mut clock);
    let ns = t0.elapsed().as_nanos();
    assert!(res.kind != ResultType::Exception, "the search threw: {:?}", res.error);
    let best = finder.best_path_so_far().expect("a best path").map(|p| p.dest()).unwrap_or(c.start);
    Run { ns, kind: type_name(res.kind), nodes: finder.num_nodes_considered, moves: finder.num_movements_considered, map: finder.map_size(), plen: res.path.as_ref().map(|p| p.positions().len()).unwrap_or(0), phash: fnv_text(&path_text(res.path.as_ref())), best, besth: c.goal.heuristic(best.x, best.y, best.z, &gs) }
}

fn line(c: &Case, mode: &str, primary: i64, failure: i64, runs: &[Run]) -> String {
    let r = &runs[0];
    for o in runs {
        if o.nodes != r.nodes || o.phash != r.phash || o.kind != r.kind {
            return format!("ERR id={} the repeated runs of one search disagree\n", c.id);
        }
    }
    let ns: Vec<String> = runs.iter().map(|r| r.ns.to_string()).collect();
    let gc = vec!["0"; runs.len()].join(",");
    format!("T id={} mode={mode} primary={primary} failure={failure} reps={} ns={} gcms={gc} nodes={} moves={} map={} type={} plen={} phash={:x} best={} besth={}\n", c.id, runs.len(), ns.join(","), r.nodes, r.moves, r.map, r.kind, r.plen, r.phash, xyz(r.best), r.besth)
}

fn world_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir).expect("the directory").filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("world-") && n.ends_with(".txt"))).collect();
    v.sort_by_key(|p| {
        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        n.trim_start_matches("world-").trim_end_matches(".txt").parse::<u64>().unwrap_or(0)
    });
    v
}

/// {id: (reps, type, primary)} from the Java side's finish file.
fn java_finish(dir: &std::path::Path, tag: &str) -> HashMap<String, (usize, String, i64)> {
    let text = std::fs::read_to_string(dir.join(format!("java-finish{tag}.txt"))).expect("java-finish.txt");
    let mut m = HashMap::new();
    for l in text.lines().filter(|l| l.starts_with("T ")) {
        let f = kv(&l.split(' ').skip(1).collect::<Vec<_>>());
        m.insert(f["id"].clone(), (f["reps"].parse().unwrap(), f["type"].clone(), f["primary"].parse().unwrap()));
    }
    m
}

/// Searches of other worlds first, about as much work as the Java side's warm-up (the allocator and the caches; the engine has no JIT).
fn warm_up(wf: &WorldFile) {
    for c in wf.cases.iter().filter(|c| !c.stress).take(2) {
        run_once(wf, c, 1500, 1500);
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(1).expect("mode");
    let dir = std::path::PathBuf::from(a.get(2).expect("directory"));
    match mode.as_str() {
        "check" => {
            for f in world_files(&dir) {
                let wf = load(&f);
                println!("seed {} size {} hash {:x} cases {}", wf.seed, wf.world.n, wf.hash, wf.cases.len());
            }
        }
        "finish" => {
            let tag = a.get(3).map(String::as_str).unwrap_or("");
            let jf = java_finish(&dir, tag);
            let mut out = std::io::BufWriter::new(std::fs::File::create(dir.join(format!("rust-finish{tag}.txt"))).expect("output"));
            let mut warmed = false;
            for f in &world_files(&dir) {
                let wf = load(f);
                writeln!(out, "W seed={} size={} hash={:x}", wf.seed, wf.world.n, wf.hash).unwrap();
                if !warmed {
                    warm_up(&wf);
                    warmed = true;
                }
                for c in wf.cases.iter().filter(|c| !c.stress) {
                    let Some((reps, ty, limit)) = jf.get(&c.id) else { continue };
                    let first = run_once(&wf, c, *limit, *limit);
                    let mut runs = vec![];
                    if ty == "SUCCESS_TO_GOAL" {
                        for _ in 0..*reps {
                            runs.push(run_once(&wf, c, *limit, *limit));
                        }
                    } else {
                        runs.push(first);
                    }
                    out.write_all(line(c, "finish", *limit, *limit, &runs).as_bytes()).unwrap();
                    out.flush().unwrap();
                    let mut ns: Vec<u128> = runs.iter().map(|r| r.ns).collect();
                    ns.sort_unstable();
                    println!("{} {} nodes={} median_ms={}", c.id, runs[0].kind, runs[0].nodes, ns[ns.len() / 2] as f64 / 1e6);
                }
            }
        }
        "limit" => {
            let list = std::fs::read_to_string(a.get(3).expect("list file")).expect("list");
            let mut out = std::io::BufWriter::new(std::fs::File::create(dir.join("rust-limit.txt")).expect("output"));
            let mut cache: Option<(String, WorldFile)> = None;
            for l in list.lines().filter(|l| !l.trim().is_empty()) {
                let f: Vec<&str> = l.split(' ').collect();
                let seed = f[0].split('.').next().unwrap().to_string();
                if cache.as_ref().is_none_or(|(s, _)| *s != seed) {
                    cache = Some((seed.clone(), load(&dir.join(format!("world-{seed}.txt")))));
                    if let Some((_, wf)) = &cache {
                        writeln!(out, "W seed={} size={} hash={:x}", wf.seed, wf.world.n, wf.hash).unwrap();
                    }
                }
                let wf = &cache.as_ref().expect("loaded").1;
                let c = wf.cases.iter().find(|c| c.id == f[0]).expect("a case of the world");
                let (p, fl, reps): (i64, i64, usize) = (f[1].parse().unwrap(), f[2].parse().unwrap(), f[3].parse().unwrap());
                run_once(wf, c, p, fl);
                for _ in 0..reps {
                    let r = run_once(wf, c, p, fl);
                    out.write_all(line(c, "limit", p, fl, std::slice::from_ref(&r)).as_bytes()).unwrap();
                }
                out.flush().unwrap();
                println!("{} limit {p}/{fl}", c.id);
            }
        }
        _ => panic!("mode {mode}"),
    }
}
