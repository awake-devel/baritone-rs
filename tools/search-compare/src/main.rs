// baritone-rs test tool: L2, whole searches compared with upstream's real Java classes, node by node. LGPL-3.0-or-later, see LICENSE.
//
// Input: the golden file that java/baritone/pathing/calc/SearchGoldenDump.java writes (and, for the palette and the configurations,
// the classes of tools/cost-compare/java). Line types:
//   H  format marker
//   S  one palette state: raw game facts (the same S lines as cost-compare)
//   C  one configuration (the same C lines as cost-compare)
//   K  one set of search settings: the settings of the search, the path cutoffs, the favoring and the avoidance
//   V  one generated world: kind, origin, size, world border, chunk states and the cells, run-length coded
//   E  one search: all its inputs. Followed by what upstream answered, one line per item:
//        r  result type, nodes considered, movements considered, nodes made, hash of the positions taken out of the open set,
//           and the simplified goal
//        b  the seven bestSoFar nodes: position and cost bits
//        n  mostRecentConsidered
//        q  pathToMostRecentNodeConsidered     u  bestPathSoFar()
//        s1 s2 s3  the path after postProcess, cutoffAtLoadedChunks and staticCutoff (runs with a node cap)
//        sf the final path
//      A path is P<positions>/<nodes considered>/<src> then the positions, then after a bar the movements: kind, destination,
//      cost bits, calculatedWhileLoaded.
//   t  the path after SplicedPath.trySplice of a segment onto what came before, for allowOverlapCutoff 0 and 1
//   J  what the Java side counted
// The Rust side runs the same search from the same inputs, writes the same lines, and compares them as text. Nothing in the file is a
// Baritone rule except upstream's answers, which are what is compared.
mod parse;

use baritone_engine::blocks::State;
use baritone_engine::config::{Config, SearchSettings};
use baritone_engine::goals::Goal;
use baritone_engine::movement::helper::{Border, Ctx, Precomputed};
use baritone_engine::movement::moves::Moves;
use baritone_engine::movement::step::MovementKind;
use baritone_engine::plan::{create_pathfinder, next_segment_search, real_start};
use baritone_engine::pos::Pos;
use baritone_engine::search::{Avoidance, Favoring, Path, ResultType, SearchError, SearchLimits};
use baritone_engine::world::WorldView;
use parse::*;
use std::collections::{BTreeMap, HashMap};

/// The time limits are far away: a run never ends because of the clock.
const FAR: i64 = 1 << 40;

struct SWorld {
    kind: String,
    ox: i32,
    oz: i32,
    min_y: i32,
    w: i32,
    h: i32,
    d: i32,
    cxn: i32,
    czn: i32,
    cells: Vec<u16>,
    chunks: Vec<u8>,
    air: usize,
    border: Border,
}

impl SWorld {
    fn chunk(&self, x: i32, z: i32) -> u8 {
        let (cx, cz) = ((x >> 4) - (self.ox >> 4), (z >> 4) - (self.oz >> 4));
        if cx < 0 || cx >= self.cxn || cz < 0 || cz >= self.czn {
            return 0;
        }
        self.chunks[(cx * self.czn + cz) as usize]
    }
}

impl WorldView for SWorld {
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
        let (lx, lz) = (x.wrapping_sub(self.ox), z.wrapping_sub(self.oz));
        if lx < 0 || lx >= self.w || lz < 0 || lz >= self.d {
            return self.air;
        }
        self.cells[((lx * self.d + lz) * self.h + (y - self.min_y)) as usize] as usize
    }
    fn contains_loaded_chunk(&self, x: i32, z: i32) -> bool {
        self.chunk(x, z) == 2
    }
    fn is_loaded(&self, x: i32, z: i32) -> bool {
        self.chunk(x, z) >= 1
    }
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.h
    }
}

fn kv_map<'a>(p: &[&'a str]) -> HashMap<&'a str, &'a str> {
    let mut m = HashMap::new();
    for item in p {
        let (k, v) = item.split_once('=').unwrap_or_else(|| panic!("not a key=value item: {item}"));
        assert!(m.insert(k, v).is_none(), "key twice: {k}");
    }
    m
}

fn parse_world(p: &[&str], air: usize) -> SWorld {
    let m = kv_map(&p[2..]);
    let g = |k: &str| int(m[k]);
    let (w, h, d) = (g("w"), g("h"), g("d"));
    let b: Vec<f64> = m["border"].split(',').map(dbl_dec).collect();
    let mut cells = Vec::with_capacity((w * h * d) as usize);
    for tok in m["cells"].split(',') {
        let (idx, n) = match tok.split_once('x') {
            Some((i, n)) => (i, n.parse::<usize>().expect("run length")),
            None => (tok, 1),
        };
        let idx: u16 = idx.parse().expect("palette index");
        cells.extend(std::iter::repeat_n(idx, n));
    }
    assert_eq!(cells.len(), (w * h * d) as usize, "the cells of a world do not fill it");
    let chunks: Vec<u8> = m["chunks"].bytes().map(|c| c - b'0').collect();
    assert_eq!(chunks.len() as i32, (w >> 4) * (d >> 4));
    SWorld { kind: m["kind"].to_string(), ox: g("ox"), oz: g("oz"), min_y: g("miny"), w, h, d, cxn: w >> 4, czn: d >> 4, cells, chunks, air, border: Border { min_x: b[0], max_x: b[1], min_z: b[2], max_z: b[3] } }
}

fn parse_k(p: &[&str]) -> SearchSettings {
    let m = kv_map(&p[2..]);
    SearchSettings {
        backtrack_cost_favoring_coefficient: dbl(m["backtrack"]),
        pathing_max_chunk_border_fetch: int(m["borderfetch"]),
        minimum_improvement_repropagation: bit(m["minimprove"]),
        path_cutoff_minimum_length: int(m["cutmin"]),
        path_cutoff_factor: dbl(m["cutfactor"]),
        cutoff_at_load_boundary: bit(m["loadboundary"]),
        simplify_unloaded_y_coord: bit(m["simplify"]),
        avoidance: bit(m["avoidance"]),
        mob_spawner_avoidance_coefficient: dbl(m["spawnercoeff"]),
        mob_spawner_avoidance_radius: int(m["spawnerradius"]),
        mob_avoidance_coefficient: dbl(m["mobcoeff"]),
        mob_avoidance_radius: int(m["mobradius"]),
        ..SearchSettings::default()
    }
}

fn pos3(s: &str) -> Pos {
    let v: Vec<i32> = s.split(',').map(int).collect();
    Pos::new(v[0], v[1], v[2])
}

fn pos_list(s: &str) -> Vec<Pos> {
    if s == "-" {
        vec![]
    } else {
        s.split(';').map(pos3).collect()
    }
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

/// The text of a path, the way the Java side writes it. An unverified path has positions only.
fn path_text(p: Option<&Path>, moves: bool) -> String {
    let Some(p) = p else {
        return "-".to_string();
    };
    let mut s = format!("P{}/{}/{}", p.positions().len(), p.num_nodes_considered(), xyz(p.src()));
    for q in p.positions() {
        s.push(' ');
        s.push_str(&xyz(*q));
    }
    if moves {
        s.push_str(" |");
        for m in p.movements().expect("a path to write with movements is verified") {
            let l = match m.calculated_while_loaded {
                Some(true) => 't',
                Some(false) => 'f',
                None => 'n',
            };
            s.push_str(&format!(" {}:{}:{:x}:{}", kind_name(&m.kind), xyz(m.dest), m.cost.map(f64::to_bits).unwrap_or(u64::MAX), l));
        }
    }
    s
}

#[derive(Default)]
struct Cover {
    c: BTreeMap<String, u64>,
}

impl Cover {
    fn hit(&mut self, k: &str) {
        *self.c.entry(k.to_string()).or_default() += 1;
    }
    fn get(&self, k: &str) -> u64 {
        self.c.get(k).copied().unwrap_or(0)
    }
}

#[derive(Default)]
struct Chain {
    first: Option<Path>,
    acc: [Option<Path>; 2],
    dead: [bool; 2],
}

struct Mismatches {
    n: u64,
    by_tag: BTreeMap<String, u64>,
    /// Injected-error runs only need to know that something differs, so they stop here. The normal run never does.
    stop_at_first: bool,
}

/// What a run with `first` prints instead of the full report.
fn stop_at_first_mismatch() -> ! {
    println!("REPORT {{\"mismatches\":1,\"stopped_at_first\":true}}");
    std::process::exit(1);
}

impl Mismatches {
    fn report(&mut self, id: &str, tag: &str, java: &str, rust: &str) {
        self.n += 1;
        if self.stop_at_first {
            stop_at_first_mismatch();
        }
        *self.by_tag.entry(tag.to_string()).or_default() += 1;
        if self.n <= 25 {
            let at = java.bytes().zip(rust.bytes()).position(|(a, b)| a != b).unwrap_or(java.len().min(rust.len()));
            let from = at.saturating_sub(60);
            let cut = |s: &str| s.chars().skip(from).take(160).collect::<String>();
            println!("MISMATCH {id} {tag} (first difference at byte {at})\n   java: ...{}\n   rust: ...{}", cut(java), cut(rust));
        }
    }
}

/// What one search gave.
struct Out {
    lines: Vec<(&'static str, String)>,
    result: Option<Path>,
}

struct Env<'a> {
    /// The settings of each `K` line, and the heuristic cost it overrides the configuration's with.
    heuristics: &'a [Option<f64>],
    states: &'a [State],
    pres: &'a [Precomputed],
    cfgs: &'a [Config],
    tools: &'a [&'a [f64]],
    ks: &'a [SearchSettings],
}

struct Input<'a> {
    w: &'a SWorld,
    cfg: usize,
    set: usize,
    cap: Option<u32>,
    goal: Goal,
    start: Pos,
    feet: Pos,
    previous: Option<&'a Path>,
    /// The search is the next segment after `previous` (the search starts where it ends), so it is made the way the planner makes it.
    next: bool,
    spawners: Vec<Pos>,
    mobs: Vec<Pos>,
}

/// What the engine threw. Upstream's `calculate` swallows the exception into `Type.EXCEPTION`, so the Java side cannot say why; the cause is checked on this side against the settings that are meant to make a search throw (see `exception`).
fn cause(e: Option<&SearchError>) -> &'static str {
    match e {
        Some(SearchError::ImplausibleHeuristic { .. }) => "heuristic",
        Some(SearchError::CutoffOutOfRange { .. }) => "range",
        _ => "other",
    }
}

/// The answer for a search that threw. Only two settings make a search throw: a heuristic that is NaN, and a `pathCutoffFactor` above 1 (the cut then asks for a `subList` past the end). Any other exception, or one with another cause, is counted as a failed check.
fn exception(env: &Env<'_>, inp: &Input<'_>, cover: &mut Cover, e: Option<&SearchError>) -> Out {
    let nan_heuristic = env.heuristics[inp.set].is_some_and(f64::is_nan);
    let expected = if nan_heuristic {
        "heuristic"
    } else if env.ks[inp.set].path_cutoff_factor > 1.0 {
        "range"
    } else {
        "none"
    };
    if cause(e) != expected {
        cover.hit("exception_cause_unexpected");
    }
    Out { lines: vec![("r", "EXCEPTION 0 0 0 0 -".to_string())], result: None }
}

fn run_search(env: &Env<'_>, inp: &Input<'_>, cover: &mut Cover, id: u64) -> Out {
    let mut cfg = env.cfgs[inp.cfg].clone();
    cfg.search = env.ks[inp.set].clone();
    if let Some(h) = env.heuristics[inp.set] {
        cfg.cost_heuristic = h;
    }
    let ctx = Ctx::new(inp.w, env.states, &env.pres[inp.cfg], &cfg, inp.w.border, env.tools[inp.cfg]);
    let avoid = Avoidance::create(&cfg.search, &inp.spawners, &inp.mobs);
    let make = || match inp.previous {
        Some(prev) if inp.next => next_segment_search(prev, &inp.goal, &ctx, inp.feet, &avoid),
        _ => create_pathfinder(inp.start, &inp.goal, inp.previous, &ctx, inp.feet, &avoid),
    };
    let mut finder = make();
    let simp = if finder.goal() != &inp.goal {
        match finder.goal() {
            Goal::Xz { x, z } => {
                cover.hit("simplified_goal");
                format!("{x},{z}")
            }
            other => panic!("a goal was simplified to something else than a GoalXZ: {other:?}"),
        }
    } else {
        "-".to_string()
    };
    if !Favoring::new(inp.previous, &cfg.search, &avoid).is_empty() {
        cover.hit("favoring_active");
    }
    let gs = cfg.goal_settings();
    let mut raw_len = None;
    let (s1, s2, s3, kind, fin): (Option<Path>, Option<Path>, Option<Path>, ResultType, Option<Path>) = match inp.cap {
        None => {
            let r = finder.calculate(FAR, FAR, &SearchLimits::default(), &mut || 0);
            if r.kind == ResultType::Exception {
                return exception(env, inp, cover, r.error.as_ref());
            }
            (None, None, None, r.kind, r.path)
        }
        Some(cap) => {
            let limits = SearchLimits { max_nodes: Some(cap), cancel: None };
            let staged = (|| -> Result<_, SearchError> {
                let found = finder.calculate0(FAR, FAR, &limits, &mut || 0)?;
                raw_len = found.as_ref().map(|p| p.positions().len());
                let s1 = match found {
                    Some(p) => Some(p.post_process(&ctx)?),
                    None => None,
                };
                let (s2, s3, kind) = match &s1 {
                    None => (None, None, ResultType::Failure),
                    Some(p) => {
                        let s2 = p.clone().cutoff_at_loaded_chunks(ctx.world, &cfg.search)?;
                        let s3 = s2.clone().static_cutoff(Some(finder.goal()), &gs, &cfg.search)?;
                        let d = s3.dest();
                        let kind = if finder.goal().is_in_goal(d.x, d.y, d.z, &gs) { ResultType::SuccessToGoal } else { ResultType::SuccessSegment };
                        (Some(s2), Some(s3), kind)
                    }
                };
                Ok((s1, s2, s3, kind))
            })();
            let (s1, s2, s3, kind) = match staged {
                Ok(x) => x,
                Err(e) => return exception(env, inp, cover, Some(&e)),
            };
            let fin = s3.clone();
            // calculate() on a fresh finder has to give what the stages gave, for one search in three
            if id.is_multiple_of(3) {
                let mut again = make();
                let r = again.calculate(FAR, FAR, &limits, &mut || 0);
                if r.kind != kind || path_text(r.path.as_ref(), true) != path_text(fin.as_ref(), true) {
                    cover.hit("calculate_disagrees_with_stages");
                }
                // Properties of the engine that upstream's Java does not have (docs/deviations.md D6, D7), checked here by what they promise:
                // a finder runs once, a raised cancel flag ends a search with a cancellation and no work, and no destination goal means no static cutoff.
                if !matches!(again.calculate(FAR, FAR, &limits, &mut || 0).error, Some(SearchError::Reused)) {
                    cover.hit("selfcheck_failures");
                }
                let raised = std::sync::atomic::AtomicBool::new(true);
                let cancelled = SearchLimits { max_nodes: limits.max_nodes, cancel: Some(&raised) };
                let mut c = make();
                let r = c.calculate(FAR, FAR, &cancelled, &mut || 0);
                if r.kind != ResultType::Cancellation || r.path.is_some() || c.num_nodes_considered != 0 {
                    cover.hit("selfcheck_failures");
                }
                if !matches!(make().calculate0(FAR, FAR, &cancelled, &mut || 0), Ok(None)) {
                    cover.hit("selfcheck_failures");
                }
                // a flag raised during the search (from the clock, which is read every 64 nodes): calculate() reports a cancellation, and calculate0()
                // gives no partial path, only the path to the goal when the goal was taken out in the turn the flag was raised
                let mid = std::sync::atomic::AtomicBool::new(false);
                let mid_limits = SearchLimits { max_nodes: None, cancel: Some(&mid) };
                let mut calls = 0;
                let mut c = make();
                let r = c.calculate(FAR, FAR, &mid_limits, &mut || {
                    calls += 1;
                    if calls == 2 {
                        mid.store(true, std::sync::atomic::Ordering::Release);
                    }
                    0
                });
                if calls >= 2 && (r.kind != ResultType::Cancellation || r.path.is_some()) {
                    cover.hit("selfcheck_failures");
                }
                mid.store(false, std::sync::atomic::Ordering::Release);
                let mut calls = 0;
                let mut c = make();
                let r = c.calculate0(FAR, FAR, &mid_limits, &mut || {
                    calls += 1;
                    if calls == 2 {
                        mid.store(true, std::sync::atomic::Ordering::Release);
                    }
                    0
                });
                if calls >= 2 {
                    match r {
                        Ok(None) => {}
                        Ok(Some(p)) => {
                            let d = p.dest();
                            if !c.goal().is_in_goal(d.x, d.y, d.z, &gs) {
                                cover.hit("selfcheck_failures");
                            }
                        }
                        Err(_) => cover.hit("selfcheck_failures"),
                    }
                }
                // Path.postProcess throws when a path is verified twice
                if let Some(p) = &s1 {
                    if p.clone().post_process(&ctx).is_ok() {
                        cover.hit("selfcheck_failures");
                    }
                }
                if let Some(p) = &s2 {
                    if path_text(p.clone().static_cutoff(None, &gs, &cfg.search).ok().as_ref(), true) != path_text(Some(p), true) {
                        cover.hit("selfcheck_failures");
                    }
                }
            }
            (s1, s2, s3, kind, fin)
        }
    };
    let mut lines: Vec<(&'static str, String)> = vec![];
    lines.push(("r", format!("{} {} {} {} {:x} {}", type_name(kind), finder.num_nodes_considered, finder.num_movements_considered, finder.map_size(), finder.popped_hash, simp)));
    let mut b = String::new();
    for node in finder.best_so_far_nodes() {
        let (p, c) = node.expect("the seven slots are filled from the start node on");
        b.push_str(&format!(" {},{:x}", xyz(p), c.to_bits()));
    }
    lines.push(("b", b.trim_start().to_string()));
    lines.push(("n", finder.most_recent_considered().map(xyz).unwrap_or_else(|| "-".to_string())));
    lines.push(("q", path_text(finder.path_to_most_recent_node_considered().expect("a path to the last node").as_ref(), false)));
    lines.push(("u", path_text(finder.best_path_so_far().expect("a best path so far").as_ref(), false)));
    if inp.cap.is_some() {
        lines.push(("s1", path_text(s1.as_ref(), true)));
        lines.push(("s2", path_text(s2.as_ref(), true)));
        lines.push(("s3", path_text(s3.as_ref(), true)));
    }
    lines.push(("sf", path_text(fin.as_ref(), true)));

    // coverage
    let st = finder.stats;
    for (name, n) in [("heap_update", st.heap_updates), ("heap_tie", st.heap_ties), ("empty_chunk_stop", st.empty_chunk_stops), ("border_rejection", st.border_rejections), ("min_improvement_block", st.min_improvement_blocks), ("favored_move", st.favored_moves)] {
        if n > 0 {
            cover.hit(name);
        }
    }
    if let Some(i) = st.coefficient_chosen {
        cover.hit(&format!("coefficient_{i}"));
    }
    cover.hit(&format!("result_{}", type_name(kind).to_lowercase()));
    if let Some(rl) = raw_len {
        if let Some(p) = &s1 {
            if p.positions().len() < rl {
                cover.hit("run_backwards_cut");
            }
        }
    }
    if let (Some(a), Some(b)) = (&s1, &s2) {
        if b.length() < a.length() {
            cover.hit("load_boundary_cut");
        }
    }
    if let (Some(a), Some(b)) = (&s2, &s3) {
        if b.length() < a.length() {
            cover.hit("static_cutoff_cut");
        }
    }
    let rs = real_start(inp.start, inp.feet);
    if rs != inp.start {
        for p in [s1.as_ref(), fin.as_ref()].into_iter().flatten() {
            if p.src() != inp.start {
                cover.hit("real_start_fake_node");
                break;
            }
        }
    }
    if let Some(p) = &fin {
        let ps = p.positions();
        let (lo, hi) = ps.iter().fold((Pos { x: i32::MAX, y: i32::MAX, z: i32::MAX }, Pos { x: i32::MIN, y: i32::MIN, z: i32::MIN }), |(lo, hi), q| (Pos { x: lo.x.min(q.x), y: lo.y.min(q.y), z: lo.z.min(q.z) }, Pos { x: hi.x.max(q.x), y: hi.y.max(q.y), z: hi.z.max(q.z) }));
        if lo.x < 0 && hi.x >= 0 {
            cover.hit("final_path_crosses_x0");
        }
        if lo.y < 0 && hi.y >= 0 {
            cover.hit("final_path_crosses_y0");
        }
        if lo.z < 0 && hi.z >= 0 {
            cover.hit("final_path_crosses_z0");
        }
        if hi.y.wrapping_sub(lo.y) >= 256 {
            cover.hit("final_path_y_span_256");
        }
        if hi.x.wrapping_sub(lo.x).abs().max(hi.z.wrapping_sub(lo.z).abs()) >= 46341 {
            cover.hit("final_path_over_46340_blocks");
        }
        for m in p.movements().expect("verified") {
            if matches!(m.kind, MovementKind::Fall) {
                cover.hit("kind_fall");
            }
            if let Some(mv) = Moves::ALL.iter().find(|mv| mv.apply0(&ctx, m.src).dest == m.dest) {
                cover.hit(&format!("move_{}", mv.name()));
            }
        }
    }
    Out { lines, result: fin }
}

/// Counters that the end of the run adds to the mismatches.
const STOP_COUNTERS: [&str; 3] = ["calculate_disagrees_with_stages", "exception_cause_unexpected", "selfcheck_failures"];

fn main() {
    let path = std::env::args().nth(1).expect("usage: search-compare GOLDEN_FILE [first]");
    let stop_at_first = std::env::args().nth(2).as_deref() == Some("first");
    let t0 = std::time::Instant::now();
    let text = std::fs::read_to_string(path).expect("cannot read the golden file");
    let lines: Vec<&str> = text.lines().collect();
    let mut states: Vec<State> = vec![];
    let (mut str_pick, mut str_hand): (Vec<f64>, Vec<f64>) = (vec![], vec![]);
    let mut cfgs: Vec<Config> = vec![];
    let mut has_pick: Vec<bool> = vec![];
    let mut ks: Vec<SearchSettings> = vec![];
    let mut heuristics: Vec<Option<f64>> = vec![];
    let mut pres: Vec<Precomputed> = vec![];
    let mut worlds: Vec<SWorld> = vec![];
    let mut results: HashMap<u64, Option<Path>> = HashMap::new();
    let mut chains: HashMap<u64, Chain> = HashMap::new();
    let mut cover = Cover::default();
    let mut mm = Mismatches { n: 0, by_tag: BTreeMap::new(), stop_at_first };
    let (mut searches, mut splice_lines, mut compared_lines) = (0u64, 0u64, 0u64);
    let mut java_counts = String::new();
    let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_cap: BTreeMap<String, u64> = BTreeMap::new();
    let mut i = 0;
    while i < lines.len() {
        if stop_at_first && STOP_COUNTERS.iter().any(|k| cover.get(k) > 0) {
            stop_at_first_mismatch();
        }
        let p: Vec<&str> = lines[i].split(' ').collect();
        i += 1;
        match p[0] {
            "H" => {}
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
            "K" => {
                assert_eq!(int(p[1]) as usize, ks.len());
                ks.push(parse_k(&p));
                let costh = p.iter().find_map(|x| x.strip_prefix("costh=")).expect("costh");
                heuristics.push(if costh == "-" { None } else { Some(dbl(costh)) });
            }
            "V" => {
                if pres.is_empty() {
                    pres = cfgs.iter().map(|c| Precomputed::new(&states, c)).collect();
                }
                assert_eq!(int(p[1]) as usize, worlds.len());
                let w = parse_world(&p, pres[0].air());
                worlds.push(w);
            }
            "J" => java_counts = p[1..].join(" "),
            "E" => {
                let h = kv_map(&p[1..]);
                let id: u64 = h["id"].parse().unwrap();
                // what Java answered
                let mut java: Vec<(&str, &str)> = vec![];
                while i < lines.len() {
                    let tag = lines[i].split(' ').next().unwrap();
                    if matches!(tag, "r" | "b" | "n" | "q" | "u" | "s1" | "s2" | "s3" | "sf") {
                        java.push((tag, lines[i].split_once(' ').map(|x| x.1).unwrap_or("")));
                        i += 1;
                    } else {
                        break;
                    }
                }
                let mut tlines: Vec<&str> = vec![];
                while i < lines.len() && lines[i].starts_with("t ") {
                    tlines.push(lines[i]);
                    i += 1;
                }
                let tools: Vec<&[f64]> = has_pick.iter().map(|p| if *p { &str_pick[..] } else { &str_hand[..] }).collect();
                let env = Env { heuristics: &heuristics, states: &states, pres: &pres, cfgs: &cfgs, tools: &tools, ks: &ks };
                let wi: usize = h["w"].parse().unwrap();
                let cap = if h["cap"] == "u" { None } else { Some(h["cap"].parse::<u32>().unwrap()) };
                let prev_id: i64 = h["prev"].parse().unwrap();
                let previous = if prev_id >= 0 { results.get(&(prev_id as u64)).expect("the earlier search is before this one").clone() } else { None };
                let goal = parse_goal(h["goal"]);
                let spec_class = h["goal"].split('(').next().unwrap().to_string();
                let inp = Input { w: &worlds[wi], cfg: h["cfg"].parse().unwrap(), set: h["set"].parse().unwrap(), cap, goal, start: pos3(h["start"]), feet: pos3(h["feet"]), previous: previous.as_ref(), next: h["role"] == "next", spawners: pos_list(h["sp"]), mobs: pos_list(h["mob"]) };
                let out = run_search(&env, &inp, &mut cover, id);
                if out.lines[0].1.starts_with("EXCEPTION") {
                    cover.hit("result_exception");
                }
                searches += 1;
                *by_kind.entry(worlds[wi].kind.clone()).or_default() += 1;
                *by_cap.entry(h["cap"].to_string()).or_default() += 1;
                cover.hit(&format!("goal_{spec_class}"));
                cover.hit(&format!("config_{}", inp.cfg));
                cover.hit(&format!("settings_{}", inp.set));
                cover.hit(if cap.is_some() { "flow_capped" } else { "flow_unlimited" });
                if inp.previous.is_some() {
                    cover.hit("previous_path_given");
                }
                // compare line by line, as text
                let id_s = format!("E{id}");
                let rust: Vec<(&str, &str)> = out.lines.iter().map(|(t, s)| (*t, s.as_str())).collect();
                if rust.len() != java.len() || rust.iter().zip(java.iter()).any(|(a, b)| a.0 != b.0) {
                    mm.report(&id_s, "lines", &format!("{:?}", java.iter().map(|x| x.0).collect::<Vec<_>>()), &format!("{:?}", rust.iter().map(|x| x.0).collect::<Vec<_>>()));
                } else {
                    for (a, b) in rust.iter().zip(java.iter()) {
                        compared_lines += 1;
                        if a.1 != b.1 {
                            mm.report(&id_s, a.0, b.1, a.1);
                        }
                    }
                }
                // splicing
                let chain_id: u64 = h["chain"].parse().unwrap();
                let seg: u64 = h["seg"].parse().unwrap();
                let role = h["role"];
                let mut expect_t: Vec<String> = vec![];
                if role == "plain" {
                    chains.entry(chain_id).or_default().first = out.result.clone();
                } else if role == "next" {
                    let ch = chains.entry(chain_id).or_default();
                    for a in 0..2usize {
                        if ch.dead[a] {
                            continue;
                        }
                        let first = ch.acc[a].as_ref().or(ch.first.as_ref());
                        let text = match Path::try_splice(first, out.result.as_ref(), a == 1) {
                            Ok(Some(sp)) => {
                                let (f, s) = (first.unwrap(), out.result.as_ref().unwrap());
                                if a == 1 {
                                    cover.hit(if sp.length() < f.length() + s.length() - 1 { "splice_overlap_cut" } else { "splice_no_overlap" });
                                } else {
                                    cover.hit("splice_no_overlap");
                                }
                                let t = path_text(Some(&sp), true);
                                ch.acc[a] = Some(sp);
                                t
                            }
                            Ok(None) => {
                                if first.is_some() && out.result.is_some() {
                                    cover.hit("splice_refused");
                                }
                                ch.dead[a] = true;
                                "-".to_string()
                            }
                            Err(_) => {
                                cover.hit("splice_error");
                                ch.dead[a] = true;
                                "ERR".to_string()
                            }
                        };
                        expect_t.push(format!("t {chain_id} {seg} {a} {text}"));
                    }
                }
                if expect_t.len() != tlines.len() {
                    mm.report(&id_s, "t-count", &tlines.len().to_string(), &expect_t.len().to_string());
                } else {
                    for (a, b) in expect_t.iter().zip(tlines.iter()) {
                        splice_lines += 1;
                        if a != b {
                            mm.report(&id_s, "t", b, a);
                        }
                    }
                }
                results.insert(id, out.result);
            }
            other => panic!("bad line type {other}"),
        }
    }
    // the coverage that has to be reached, and what is exempt with its reason
    let mut required: Vec<String> = [
        "result_success_to_goal",
        "result_success_segment",
        "result_failure",
        "heap_update",
        "heap_tie",
        "empty_chunk_stop",
        "border_rejection",
        "min_improvement_block",
        "favored_move",
        "favoring_active",
        "run_backwards_cut",
        "real_start_fake_node",
        "load_boundary_cut",
        "static_cutoff_cut",
        "splice_overlap_cut",
        "splice_no_overlap",
        "splice_refused",
        "simplified_goal",
        "kind_fall",
        "previous_path_given",
        "flow_capped",
        "flow_unlimited",
        "result_exception",
        "final_path_crosses_x0",
        "final_path_crosses_y0",
        "final_path_crosses_z0",
        "final_path_y_span_256",
        "final_path_over_46340_blocks",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    required.extend((0..7).map(|i| format!("coefficient_{i}")));
    required.extend(Moves::ALL.iter().map(|m| format!("move_{}", m.name())));
    required.extend(["block", "xz", "ylevel", "twoblocks", "gettoblock", "near", "axis", "strict", "runaway", "composite", "inverted"].iter().map(|g| format!("goal_{g}")));
    required.extend((0..cfgs.len()).map(|c| format!("config_{c}")));
    required.extend((0..ks.len()).map(|k| format!("settings_{k}")));
    let exempt: BTreeMap<&str, &str> = BTreeMap::from([
        ("result_cancellation", "only the caller's cancel flag ends a search with a cancellation; the harness ends runs with a node cap, which acts as a timeout, so no run is cancelled. The flag is unit-tested in astar.rs"),
        ("splice_error", "SplicedPath.trySplice throws only when overlapping paths are spliced without allowOverlapCutoff, and a path's own first position is always its first index, so no input reaches it"),
        ("calculate_disagrees_with_stages", "this counts disagreements between calculate() and the staged run: it has to stay at zero, and is reported as a mismatch"),
    ]);
    let mut coverage: BTreeMap<String, u64> = BTreeMap::new();
    for k in &required {
        coverage.insert(k.clone(), cover.get(k));
    }
    for k in ["result_cancellation", "splice_error"] {
        coverage.insert(k.to_string(), cover.get(k));
    }
    let calc_dis = cover.get("calculate_disagrees_with_stages");
    if calc_dis > 0 {
        mm.n += calc_dis;
        println!("MISMATCH calculate() disagrees with the staged run in {calc_dis} searches");
    }
    let bad_causes = cover.get("exception_cause_unexpected");
    if bad_causes > 0 {
        mm.n += bad_causes;
        println!("MISMATCH {bad_causes} searches threw for a reason the settings do not explain");
    }
    let selfchecks = cover.get("selfcheck_failures");
    if selfchecks > 0 {
        mm.n += selfchecks;
        println!("MISMATCH {selfchecks} Rust-only property checks failed (single use, cancel flag, post processing twice, static cutoff without a goal)");
    }
    println!("searches {searches}, lines compared {compared_lines}, splice lines {splice_lines}, worlds {}, palette {}, configs {}, settings {}", worlds.len(), states.len(), cfgs.len(), ks.len());
    println!("java counted: {java_counts}");
    println!("searches by world kind {by_kind:?}");
    println!("searches by cap {by_cap:?}");
    println!("mismatches by line {:?}", mm.by_tag);
    let mut gaps = vec![];
    for (k, v) in &coverage {
        println!("coverage {k}: {v}");
        if *v == 0 && !exempt.contains_key(k.as_str()) {
            gaps.push(k.clone());
        }
    }
    eprintln!("compared in {:?}", t0.elapsed());
    let json = format!("{{\"searches\":{searches},\"lines_compared\":{compared_lines},\"splice_lines\":{splice_lines},\"mismatches\":{},\"coverage\":{},\"exempt\":{},\"by_world_kind\":{},\"by_cap\":{},\"java_counted\":\"{java_counts}\"}}", mm.n, map_json(&coverage), map_json_str(&exempt), map_json(&by_kind), map_json(&by_cap));
    println!("REPORT {json}");
    if mm.n > 0 || !gaps.is_empty() {
        if !gaps.is_empty() {
            println!("COVERAGE MISSING: {gaps:?}");
        }
        std::process::exit(1);
    }
}

fn map_json(m: &BTreeMap<String, u64>) -> String {
    format!("{{{}}}", m.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect::<Vec<_>>().join(","))
}

fn map_json_str(m: &BTreeMap<&str, &str>) -> String {
    format!("{{{}}}", m.iter().map(|(k, v)| format!("\"{k}\":\"{}\"", v.replace('"', "'"))).collect::<Vec<_>>().join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_cells_are_run_length_decoded() {
        let w = parse_world(&["V", "0", "kind=x", "ox=16", "oz=-16", "miny=-64", "w=16", "h=2", "d=16", "border=-1.0E3,1000.0,-1000.0,1.0E3", "chunks=2", &format!("cells=1x{},0x{}", 16 * 16, 16 * 16)], 0);
        assert_eq!(w.cells.len(), 512);
        assert_eq!(w.cells[0], 1);
        assert_eq!(w.cells[511], 0);
        assert!(w.contains_loaded_chunk(16, -16) && !w.contains_loaded_chunk(0, -16));
        assert!(!w.is_loaded(32, -16));
        assert_eq!(w.border.min_x, -1000.0);
    }

    #[test]
    fn an_absent_path_is_a_dash() {
        assert_eq!(path_text(None, true), "-");
        assert_eq!(xyz(Pos::new(1, -2, 3)), "1,-2,3");
    }
}
