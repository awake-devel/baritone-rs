// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/calc/AStarPathFinder.java and
// src/main/java/baritone/pathing/calc/AbstractNodeCostSearch.java (with PathCalculationResult from the API). LGPL-3.0-or-later, see lib.rs.
//
// Differences that are the design, not the rules (see docs/deviations.md):
// - Upstream stops on wall-clock time. The caller gives the clock, so a test can fake it, and a node cap, so two runs can be compared.
// - `cancel` is a flag the caller owns, and `calculate` does not reset it. Upstream resets `cancelRequested` at the start of `calculate`,
//   which loses a cancel that arrived first.
// - Upstream throws on an implausible cost, a wrong destination and a NaN heuristic. Here they are `SearchError` values.
// - Logging, the `slowPath` setting and the printing at the end are dropped.
use super::favoring::Favoring;
use super::node::{NodeId, NodeMap};
use super::open_set::BinaryHeapOpenSet;
use super::path::Path;
use super::SearchError;
use crate::costs::COST_INF;
use crate::goals::Goal;
use crate::movement::helper::{Ctx, MoveResult};
use crate::movement::moves::Moves;
use crate::pos::{long_hash, Pos};
use crate::world::WorldView;
use std::sync::atomic::{AtomicBool, Ordering};

/// The cost coefficients of the partial path search, upstream's `COEFFICIENTS`.
const COEFFICIENTS: [f64; 7] = [1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 10.0];
/// Upstream's `MIN_DIST_PATH`: a path that goes less far than this and does not reach its goal is not worth considering.
const MIN_DIST_PATH: f64 = 5.0;
/// Upstream's `MIN_IMPROVEMENT`: floating point errors make costs differ by about 1e-16 on flat ground, and that is not worth an update.
const MIN_IMPROVEMENT: f64 = 0.01;

/// The types of upstream's `PathCalculationResult`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResultType {
    SuccessToGoal,
    SuccessSegment,
    Failure,
    Cancellation,
    Exception,
}

/// `PathCalculationResult`: the type, the path when there is one, and for `Exception` what upstream would have thrown.
#[derive(Clone, Debug)]
pub struct PathCalculationResult {
    pub kind: ResultType,
    pub path: Option<Path>,
    pub error: Option<SearchError>,
}

/// What stops a search besides the time limits.
#[derive(Clone, Copy, Default)]
pub struct SearchLimits<'a> {
    /// The N-th node is taken out and expanded in full, and then the loop ends the way a timeout ends it, with `bestSoFar`. `None` is no cap. A cap of 0 acts as 1: the first node is always expanded.
    pub max_nodes: Option<u32>,
    /// Set from outside to stop the search. It is read once per loop turn and never reset here.
    pub cancel: Option<&'a AtomicBool>,
}

/// Test hooks: how often the search met each of the cases a comparison has to reach. Only counted, never read by the search.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct SearchStats {
    /// Nodes already in the open set that got a cheaper way (`openSet.update`, a decrease-key).
    pub heap_updates: u32,
    /// Comparisons in the open set that met two equal keys.
    pub heap_ties: u32,
    /// The loop ended because `numEmptyChunk` reached `pathingMaxChunkBorderFetch`.
    pub empty_chunk_stops: u32,
    /// Moves dropped because their destination is outside the world border.
    pub border_rejections: u32,
    /// Better costs that `MIN_IMPROVEMENT` kept out: positive improvements of at most 0.01.
    pub min_improvement_blocks: u32,
    /// Costs multiplied by a favoring other than 1.
    pub favored_moves: u32,
    /// When the search ended without taking the goal out of the open set: the index of the coefficient whose best node `bestSoFar` took, if any.
    pub coefficient_chosen: Option<u8>,
}

/// `AStarPathFinder` and `AbstractNodeCostSearch`. One finder runs one search.
pub struct AStarPathFinder<'c, 'a, W: WorldView> {
    ctx: &'c Ctx<'a, W>,
    real_start: Pos,
    start_x: i32,
    start_y: i32,
    start_z: i32,
    goal: Goal,
    favoring: Favoring,
    map: NodeMap,
    start_node: Option<NodeId>,
    most_recent_considered: Option<NodeId>,
    best_so_far: [Option<NodeId>; 7],
    finished: bool,
    /// How many nodes the last `calculate0` took out of the open set.
    pub num_nodes_considered: i32,
    /// How many moves it asked for a cost.
    pub num_movements_considered: i32,
    /// Test hook: a hash of the positions taken out of the open set, in order. FNV-1a over the three coordinates as unsigned 32-bit words, from the offset basis.
    pub popped_hash: u64,
    /// Test hook, see [`SearchStats`].
    pub stats: SearchStats,
}

/// The start of `popped_hash`.
pub const POP_HASH_START: u64 = 0xcbf2_9ce4_8422_2325;

/// One step of `popped_hash`.
pub fn pop_hash_step(mut h: u64, x: i32, y: i32, z: i32) -> u64 {
    for v in [x, y, z] {
        h ^= v as u32 as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl<'c, 'a, W: WorldView> AStarPathFinder<'c, 'a, W> {
    /// `AStarPathFinder.<init>`: the search starts from `start`; `real_start` is where the player is, which differs when the start was moved to a nearby block.
    pub fn new(real_start: Pos, start_x: i32, start_y: i32, start_z: i32, goal: Goal, favoring: Favoring, ctx: &'c Ctx<'a, W>) -> Self {
        AStarPathFinder { ctx, real_start, start_x, start_y, start_z, goal, favoring, map: NodeMap::default(), start_node: None, most_recent_considered: None, best_so_far: [None; 7], finished: false, num_nodes_considered: 0, num_movements_considered: 0, popped_hash: POP_HASH_START, stats: SearchStats::default() }
    }

    /// `AbstractNodeCostSearch.getGoal`: the goal the search works towards, after any simplification.
    pub fn goal(&self) -> &Goal {
        &self.goal
    }

    /// `AbstractNodeCostSearch.getStart`: the position the search starts from.
    pub fn start(&self) -> Pos {
        Pos::new(self.start_x, self.start_y, self.start_z)
    }

    /// `AbstractNodeCostSearch.isFinished`: `calculate` has run.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// `AbstractNodeCostSearch.mapSize`: how many nodes were made.
    pub fn map_size(&self) -> usize {
        self.map.num_nodes_created()
    }

    /// `AbstractNodeCostSearch.getDistFromStartSq`: the squared distance, in `int` arithmetic, which wraps for nodes more than about 46,000 blocks away.
    fn get_dist_from_start_sq(&self, id: NodeId) -> f64 {
        let n = &self.map.nodes[id as usize];
        let x_diff = n.x.wrapping_sub(self.start_x);
        let y_diff = n.y.wrapping_sub(self.start_y);
        let z_diff = n.z.wrapping_sub(self.start_z);
        x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) as f64
    }

    /// `AbstractNodeCostSearch.calculate`: runs the search, then post-processes the path and cuts it at the loaded chunks and by length. A finder runs once; a second call answers `Exception` with `SearchError::Reused`. `clock` returns milliseconds and is called the way upstream calls `System.currentTimeMillis`.
    pub fn calculate(&mut self, primary_timeout: i64, failure_timeout: i64, limits: &SearchLimits<'_>, clock: &mut dyn FnMut() -> i64) -> PathCalculationResult {
        if self.finished {
            return PathCalculationResult { kind: ResultType::Exception, path: None, error: Some(SearchError::Reused) };
        }
        let r = self.calculate_inner(primary_timeout, failure_timeout, limits, clock);
        self.finished = true;
        match r {
            Ok(res) => res,
            Err(e) => PathCalculationResult { kind: ResultType::Exception, path: None, error: Some(e) },
        }
    }

    fn calculate_inner(&mut self, primary_timeout: i64, failure_timeout: i64, limits: &SearchLimits<'_>, clock: &mut dyn FnMut() -> i64) -> Result<PathCalculationResult, SearchError> {
        let ctx = self.ctx;
        let found = self.calculate0(primary_timeout, failure_timeout, limits, clock)?;
        let path = match found {
            Some(p) => Some(p.post_process(ctx)?),
            None => None,
        };
        if is_cancelled(limits) {
            return Ok(PathCalculationResult { kind: ResultType::Cancellation, path: None, error: None });
        }
        let Some(path) = path else {
            return Ok(PathCalculationResult { kind: ResultType::Failure, path: None, error: None });
        };
        let gs = ctx.cfg.goal_settings();
        let s = &ctx.cfg.search;
        let path = path.cutoff_at_loaded_chunks(ctx.world, s)?;
        let path = path.static_cutoff(Some(&self.goal), &gs, s)?;
        let d = path.dest();
        if self.goal.is_in_goal(d.x, d.y, d.z, &gs) {
            Ok(PathCalculationResult { kind: ResultType::SuccessToGoal, path: Some(path), error: None })
        } else {
            Ok(PathCalculationResult { kind: ResultType::SuccessSegment, path: Some(path), error: None })
        }
    }

    /// `AStarPathFinder.calculate0`: the search itself. Returns the path to the goal, the best partial path when the limits end the search, or nothing when it was cancelled or there is no path.
    pub fn calculate0(&mut self, primary_timeout: i64, failure_timeout: i64, limits: &SearchLimits<'_>, clock: &mut dyn FnMut() -> i64) -> Result<Option<Path>, SearchError> {
        let ctx = self.ctx;
        let gs = ctx.cfg.goal_settings();
        let settings = &ctx.cfg.search;
        let min_y = ctx.min_y;
        // U15: with `walk_on_build_limit`, feet may stand on the top layer, one above `maxY`.
        let max_y = if settings.walk_on_build_limit { ctx.max_y + 1 } else { ctx.max_y };
        let start_node = self.map.get_node_at_position(self.start_x, self.start_y, self.start_z, &self.goal, &gs)?;
        self.start_node = Some(start_node);
        {
            let n = &mut self.map.nodes[start_node as usize];
            n.cost = 0.0;
            n.combined_cost = n.estimated_cost_to_goal;
        }
        let mut open_set = BinaryHeapOpenSet::new();
        open_set.insert(&mut self.map.nodes, start_node);
        let inverse_coefficients: [f64; 7] = COEFFICIENTS.map(|c| 1.0 / c);
        // the best node by the metric of (estimatedCostToGoal + cost / COEFFICIENTS[i])
        let mut best_heuristic_so_far = [self.map.nodes[start_node as usize].estimated_cost_to_goal; 7];
        self.best_so_far = [Some(start_node); 7];
        let border = ctx.border;
        let start_time = clock();
        let primary_timeout_time = start_time.wrapping_add(primary_timeout);
        let failure_timeout_time = start_time.wrapping_add(failure_timeout);
        let mut failing = true;
        let mut num_nodes: i32 = 0;
        let mut num_movements_considered: i32 = 0;
        let mut num_empty_chunk: i32 = 0;
        let mut stopped_early = false;
        let is_favoring = !self.favoring.is_empty();
        let time_check_interval: i32 = 1 << 6;
        let pathing_max_chunk_border_fetch = settings.pathing_max_chunk_border_fetch;
        let minimum_improvement = if settings.minimum_improvement_repropagation { MIN_IMPROVEMENT } else { 0.0 };
        while !open_set.is_empty() && num_empty_chunk < pathing_max_chunk_border_fetch && !is_cancelled(limits) {
            if (num_nodes & (time_check_interval - 1)) == 0 {
                // only call this once every 64 nodes
                let now = clock();
                if now.wrapping_sub(failure_timeout_time) >= 0 || (!failing && now.wrapping_sub(primary_timeout_time) >= 0) {
                    stopped_early = true;
                    break;
                }
            }
            let current = open_set.remove_lowest(&mut self.map.nodes).expect("the open set is not empty");
            self.most_recent_considered = Some(current);
            num_nodes = num_nodes.wrapping_add(1);
            let (cur_x, cur_y, cur_z) = {
                let n = &self.map.nodes[current as usize];
                (n.x, n.y, n.z)
            };
            self.popped_hash = pop_hash_step(self.popped_hash, cur_x, cur_y, cur_z);
            if self.goal.is_in_goal(cur_x, cur_y, cur_z, &gs) {
                self.num_nodes_considered = num_nodes;
                self.num_movements_considered = num_movements_considered;
                self.stats.heap_ties = open_set.ties;
                return Path::from_nodes(self.real_start, start_node, current, num_nodes, &self.goal, &gs, &self.map.nodes).map(Some);
            }
            for moves in Moves::ALL {
                let new_x = cur_x.wrapping_add(moves.x_offset());
                let new_z = cur_z.wrapping_add(moves.z_offset());
                if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(new_x, new_z) {
                    // only need to check if the destination is a loaded chunk if it's in a different chunk than the start of the movement
                    if !moves.dynamic_xz() {
                        // only increment the counter if the movement would have gone out of bounds guaranteed
                        num_empty_chunk = num_empty_chunk.wrapping_add(1);
                    }
                    continue;
                }
                if !moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {
                    self.stats.border_rejections += 1;
                    continue;
                }
                if cur_y.wrapping_add(moves.y_offset()) > max_y || cur_y.wrapping_add(moves.y_offset()) < min_y {
                    continue;
                }
                let mut res = MoveResult::default();
                moves.apply(ctx, cur_x, cur_y, cur_z, &mut res);
                num_movements_considered = num_movements_considered.wrapping_add(1);
                let mut action_cost = res.cost;
                if action_cost >= COST_INF {
                    continue;
                }
                if action_cost <= 0.0 || action_cost.is_nan() {
                    return Err(SearchError::ImplausibleCost { moves, x: cur_x, y: cur_y, z: cur_z, cost: action_cost });
                }
                // check destination after verifying it's not COST_INF -- some movements return COST_INF without adjusting the destination
                if moves.dynamic_xz() && !border.entirely_contains(res.x, res.z) {
                    self.stats.border_rejections += 1;
                    continue;
                }
                if !moves.dynamic_xz() && (res.x != new_x || res.z != new_z) {
                    return Err(SearchError::WrongDestination { moves, x: cur_x, y: cur_y, z: cur_z, to_x: res.x, to_z: res.z, expected_x: new_x, expected_z: new_z });
                }
                if !moves.dynamic_y() && res.y != cur_y.wrapping_add(moves.y_offset()) {
                    return Err(SearchError::WrongHeight { moves, x: cur_x, y: cur_y, z: cur_z, to_y: res.y, expected_y: cur_y.wrapping_add(moves.y_offset()) });
                }
                let hash_code = long_hash(res.x, res.y, res.z);
                let neighbor = self.map.get_node_at_position(res.x, res.y, res.z, &self.goal, &gs)?;
                if is_favoring {
                    // favoring only cares where you end up, not where you came from, so look it up once per node
                    let n = &mut self.map.nodes[neighbor as usize];
                    if n.favor == 0.0 {
                        n.favor = self.favoring.calculate(hash_code);
                    }
                    action_cost *= n.favor;
                    if n.favor != 1.0 {
                        self.stats.favored_moves += 1;
                    }
                }
                let tentative_cost = self.map.nodes[current as usize].cost + action_cost;
                let improvement = self.map.nodes[neighbor as usize].cost - tentative_cost;
                if improvement > 0.0 && improvement <= minimum_improvement {
                    self.stats.min_improvement_blocks += 1;
                }
                if improvement > minimum_improvement {
                    let n = &mut self.map.nodes[neighbor as usize];
                    n.previous = Some(current);
                    n.cost = tentative_cost;
                    n.combined_cost = tentative_cost + n.estimated_cost_to_goal;
                    let was_open = n.is_open();
                    if was_open {
                        self.stats.heap_updates += 1;
                        open_set.update(&mut self.map.nodes, neighbor);
                    } else {
                        open_set.insert(&mut self.map.nodes, neighbor);
                    }
                    for i in 0..COEFFICIENTS.len() {
                        let n = &self.map.nodes[neighbor as usize];
                        let heuristic = n.estimated_cost_to_goal + n.cost * inverse_coefficients[i];
                        if best_heuristic_so_far[i] - heuristic > minimum_improvement {
                            best_heuristic_so_far[i] = heuristic;
                            self.best_so_far[i] = Some(neighbor);
                            if failing && self.get_dist_from_start_sq(neighbor) > MIN_DIST_PATH * MIN_DIST_PATH {
                                failing = false;
                            }
                        }
                    }
                }
            }
            if let Some(max) = limits.max_nodes {
                if (num_nodes as u32) >= max {
                    // the node cap ends the loop the way a timeout does
                    stopped_early = true;
                    break;
                }
            }
        }
        self.num_nodes_considered = num_nodes;
        self.num_movements_considered = num_movements_considered;
        self.stats.heap_ties = open_set.ties;
        if !open_set.is_empty() && num_empty_chunk >= pathing_max_chunk_border_fetch && !is_cancelled(limits) && !stopped_early {
            self.stats.empty_chunk_stops = 1;
        }
        if is_cancelled(limits) {
            return Ok(None);
        }
        self.stats.coefficient_chosen = self.start_node.and_then(|_| (0..COEFFICIENTS.len()).find(|i| self.best_so_far[*i].is_some_and(|b| self.get_dist_from_start_sq(b) > MIN_DIST_PATH * MIN_DIST_PATH)).map(|i| i as u8));
        self.best_so_far_path(num_nodes)
    }

    /// `AbstractNodeCostSearch.bestSoFar`: the path to the first node, in the order of the coefficients, that is more than `MIN_DIST_PATH` from the start. Nothing when there is none, which is "no path". The printing of the outcome is dropped.
    fn best_so_far_path(&self, num_nodes: i32) -> Result<Option<Path>, SearchError> {
        let Some(start_node) = self.start_node else {
            return Ok(None);
        };
        let gs = self.ctx.cfg.goal_settings();
        for i in 0..COEFFICIENTS.len() {
            let Some(best) = self.best_so_far[i] else {
                continue;
            };
            let dist = self.get_dist_from_start_sq(best);
            if dist > MIN_DIST_PATH * MIN_DIST_PATH {
                // square the comparison since distFromStartSq is squared
                return Path::from_nodes(self.real_start, start_node, best, num_nodes, &self.goal, &gs, &self.map.nodes).map(Some);
            }
        }
        // instead of returning bestSoFar[0], be less misleading
        Ok(None)
    }

    /// Test hook: the position and cost of each of the seven `bestSoFar` nodes, which is what a comparison reads. Empty slots are `None`.
    pub fn best_so_far_nodes(&self) -> [Option<(Pos, f64)>; 7] {
        self.best_so_far.map(|b| {
            b.map(|id| {
                let n = &self.map.nodes[id as usize];
                (Pos::new(n.x, n.y, n.z), n.cost)
            })
        })
    }

    /// Test hook: the position of `mostRecentConsidered`.
    pub fn most_recent_considered(&self) -> Option<Pos> {
        self.most_recent_considered.map(|id| {
            let n = &self.map.nodes[id as usize];
            Pos::new(n.x, n.y, n.z)
        })
    }

    /// `AbstractNodeCostSearch.bestPathSoFar`: the best partial path now, as `bestSoFar` with no node count.
    pub fn best_path_so_far(&self) -> Result<Option<Path>, SearchError> {
        self.best_so_far_path(0)
    }

    /// `AbstractNodeCostSearch.pathToMostRecentNodeConsidered`: the path to the node taken out last, with a node count of 0.
    pub fn path_to_most_recent_node_considered(&self) -> Result<Option<Path>, SearchError> {
        let (Some(start), Some(recent)) = (self.start_node, self.most_recent_considered) else {
            return Ok(None);
        };
        Path::from_nodes(self.real_start, start, recent, 0, &self.goal, &self.ctx.cfg.goal_settings(), &self.map.nodes).map(Some)
    }
}

fn is_cancelled(limits: &SearchLimits<'_>) -> bool {
    limits.cancel.is_some_and(|c| c.load(Ordering::Acquire))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::step::MovementKind;
    use crate::testing::*;

    fn finder<'c, 'a, W: crate::world::WorldView>(ctx: &'c Ctx<'a, W>, from: Pos, goal: Goal) -> AStarPathFinder<'c, 'a, W> {
        AStarPathFinder::new(from, from.x, from.y, from.z, goal, Favoring::from_previous(None, &ctx.cfg.search), ctx)
    }

    /// A clock that gives 0 the first time and `later` every time after.
    fn jump_clock(later: i64) -> impl FnMut() -> i64 {
        let mut first = true;
        move || {
            if first {
                first = false;
                0
            } else {
                later
            }
        }
    }

    #[test]
    fn walks_a_straight_line_to_the_goal() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let mut f = finder(&ctx, Pos::new(1, 1, 1), Goal::Block { x: 8, y: 1, z: 1 });
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.kind, ResultType::SuccessToGoal, "{:?}", r.error);
        let p = r.path.unwrap();
        let expect: Vec<Pos> = (1..=8).map(|x| Pos::new(x, 1, 1)).collect();
        assert_eq!(p.positions(), &expect[..]);
        let moves = p.movements().unwrap();
        assert_eq!(moves.len(), 7);
        assert!(moves.iter().all(|m| m.kind == MovementKind::Traverse && m.calculated_while_loaded == Some(true)));
        // a sprinting traverse costs WALK_ONE_BLOCK_COST * SPRINT_MULTIPLIER
        let step = crate::costs::SPRINT_ONE_BLOCK_COST;
        for m in moves {
            assert!((m.cost.unwrap() - step).abs() < 1e-9);
        }
        assert!(p.num_nodes_considered() >= 8);
        assert!(f.is_finished());
        assert_eq!(f.num_nodes_considered, p.num_nodes_considered());
        assert!(f.map_size() >= 8);
        // a finder runs once
        let again = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!((again.kind, again.error), (ResultType::Exception, Some(SearchError::Reused)));
    }

    #[test]
    fn walking_on_the_top_layer_needs_the_build_limit_setting() {
        // U15, found on 6b6t: a floor on the highest layer (y 15 of a 16-high world) puts the feet at maxY + 1.
        let mut world = TestWorld::flat(20, 20);
        for x in 0..20 {
            for z in 0..20 {
                world.set(x, 15, z, STONE);
            }
        }
        let run = |on: bool| {
            let mut cfg = test_config();
            cfg.search.walk_on_build_limit = on;
            let fx = Fixture::with_config(cfg);
            let ctx = fx.ctx(&world);
            let mut f = finder(&ctx, Pos::new(1, 16, 1), Goal::Block { x: 8, y: 16, z: 1 });
            let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
            (r.kind, r.path.map(|p| p.positions().to_vec()))
        };
        // upstream's search: no move at all from the top layer
        assert_ne!(run(false).0, ResultType::SuccessToGoal);
        let (kind, path) = run(true);
        assert_eq!(kind, ResultType::SuccessToGoal);
        let expect: Vec<Pos> = (1..=8).map(|x| Pos::new(x, 16, 1)).collect();
        assert_eq!(path.unwrap(), expect);
        // the setting allows one layer above maxY, no more: the default is upstream's
        assert!(!crate::config::SearchSettings::default().walk_on_build_limit);
    }

    #[test]
    fn a_goal_at_the_start_is_a_path_of_one_position() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let mut f = finder(&ctx, Pos::new(3, 1, 3), Goal::Block { x: 3, y: 1, z: 3 });
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.kind, ResultType::SuccessToGoal);
        let p = r.path.unwrap();
        assert_eq!(p.positions(), &[Pos::new(3, 1, 3)]);
        assert!(p.movements().unwrap().is_empty());
        assert_eq!(p.num_nodes_considered(), 1);
    }

    #[test]
    fn real_start_adds_a_fake_node_and_one_movement() {
        // The search starts at (1,1,1) but the player is at (2,1,1), and the start is already the goal.
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let mut f = AStarPathFinder::new(Pos::new(2, 1, 1), 1, 1, 1, Goal::Block { x: 1, y: 1, z: 1 }, Favoring::from_previous(None, &ctx.cfg.search), &ctx);
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.kind, ResultType::SuccessToGoal, "{:?}", r.error);
        let p = r.path.unwrap();
        assert_eq!(p.positions(), &[Pos::new(2, 1, 1), Pos::new(1, 1, 1)]);
        assert_eq!(p.src(), Pos::new(2, 1, 1));
        assert_eq!(p.dest(), Pos::new(1, 1, 1));
        let m = &p.movements().unwrap()[0];
        assert_eq!(m.kind, MovementKind::Traverse);
        // the node cost difference is 0 (the start and the fake node both cost 0), and the cost is the smaller of that and the real cost
        assert_eq!(m.cost, Some(0.0));
    }

    #[test]
    fn the_node_cap_ends_the_search_with_the_best_partial_path() {
        let fx = Fixture::new();
        let world = TestWorld::flat(100, 100);
        let ctx = fx.ctx(&world);
        let mut f = finder(&ctx, Pos::new(1, 1, 1), Goal::Block { x: 90, y: 1, z: 1 });
        let limits = SearchLimits { max_nodes: Some(25), cancel: None };
        let r = f.calculate(500, 2000, &limits, &mut || 0);
        assert_eq!(r.kind, ResultType::SuccessSegment, "{:?}", r.error);
        assert_eq!(f.num_nodes_considered, 25);
        let p = r.path.unwrap();
        assert_eq!(p.num_nodes_considered(), 25);
        assert!(p.length() < 30, "the test expects a path short enough to skip the static cutoff, got {}", p.length());
        // the path ends at the node of the first coefficient whose best node is more than 5 blocks from the start
        let i = (0..7).find(|i| f.get_dist_from_start_sq(f.best_so_far[*i].unwrap()) > 25.0).unwrap();
        let n = &f.map.nodes[f.best_so_far[i].unwrap() as usize];
        assert_eq!(p.dest(), Pos::new(n.x, n.y, n.z));
        assert_eq!(p.src(), Pos::new(1, 1, 1));
    }

    #[test]
    fn the_node_cap_against_a_goal_on_a_line() {
        // The goal is the 8th node taken out. The cap ends the loop after the N-th node has been expanded, and bestSoFar then
        // takes the first coefficient with a node more than 5 blocks from the start.
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let run = |cap: u32| {
            let mut f = finder(&ctx, Pos::new(1, 1, 1), Goal::Block { x: 8, y: 1, z: 1 });
            let r = f.calculate(500, 2000, &SearchLimits { max_nodes: Some(cap), cancel: None }, &mut || 0);
            (r, f.num_nodes_considered)
        };
        // the goal is popped as the 8th node: found whole
        let (r, n) = run(8);
        assert_eq!((r.kind, n, r.path.unwrap().num_nodes_considered()), (ResultType::SuccessToGoal, 8, 8));
        // 7 nodes expanded: the goal is a neighbor, not popped, but it is the best node, so the path is whole and counts 7 nodes
        let (r, n) = run(7);
        assert_eq!((r.kind, n), (ResultType::SuccessToGoal, 7));
        assert_eq!(r.path.unwrap().num_nodes_considered(), 7);
        // 6 expanded: the farthest node is 6 blocks away (x = 7), a segment
        let (r, n) = run(6);
        assert_eq!((r.kind, n), (ResultType::SuccessSegment, 6));
        assert_eq!(r.path.unwrap().dest(), Pos::new(7, 1, 1));
        // 5 expanded: the farthest node is exactly 5 blocks away, and 25 > 25 is false, so there is no path
        let (r, n) = run(5);
        assert_eq!((r.kind, n), (ResultType::Failure, 5));
        // a cap of 0 acts as 1
        let (_, n) = run(0);
        assert_eq!(n, 1);
    }

    #[test]
    fn partial_path_selection_goes_through_the_coefficients_in_order() {
        let fx = Fixture::new();
        let world = TestWorld::flat(40, 40);
        let ctx = fx.ctx(&world);
        let goal = Goal::Block { x: 30, y: 1, z: 1 };
        let gs = ctx.cfg.goal_settings();
        let mut f = finder(&ctx, Pos::new(1, 1, 1), goal.clone());
        let start = f.map.get_node_at_position(1, 1, 1, &goal, &gs).unwrap();
        f.start_node = Some(start);
        let at = |f: &mut AStarPathFinder<'_, '_, TestWorld>, x: i32, z: i32| {
            let id = f.map.get_node_at_position(x, 1, z, &goal, &gs).unwrap();
            f.map.nodes[id as usize].previous = Some(start);
            f.map.nodes[id as usize].cost = 1.0;
            id
        };
        // no path when nothing is farther than 5 blocks: 4 away, then exactly 5 away (25 > 25 is false)
        let (a, b) = (at(&mut f, 5, 1), at(&mut f, 6, 1));
        f.best_so_far = [Some(a), Some(b), None, None, None, None, None];
        assert!(f.best_path_so_far().unwrap().is_none());
        // 5 blocks in x and 1 in z: 26 > 25 and it is taken, and it is taken from the first coefficient that has one
        let c = at(&mut f, 6, 2);
        let d = at(&mut f, 20, 1);
        f.best_so_far = [Some(a), Some(b), Some(c), Some(d), None, None, None];
        let p = f.best_path_so_far().unwrap().unwrap();
        assert_eq!(p.dest(), Pos::new(6, 1, 2));
        assert_eq!(p.num_nodes_considered(), 0);
        // an empty slot is skipped
        f.best_so_far = [None, None, Some(a), None, None, None, Some(d)];
        assert_eq!(f.best_path_so_far().unwrap().unwrap().dest(), Pos::new(20, 1, 1));
        // no start node, no path
        f.start_node = None;
        assert!(f.best_path_so_far().unwrap().is_none());
    }

    #[test]
    fn a_failing_search_ignores_the_primary_timeout_until_it_got_somewhere() {
        let fx = Fixture::new();
        let world = TestWorld::flat(400, 20);
        let ctx = fx.ctx(&world);
        let goal = Goal::Block { x: 390, y: 1, z: 1 };
        // the clock says 1000 ms after the start from the first check on: past the primary timeout (500), short of the failure timeout (2000).
        // At node 0 the search is still failing (nothing is 5 blocks away), so it goes on; at node 64 it is not, so it stops there.
        let mut f = finder(&ctx, Pos::new(1, 1, 1), goal.clone());
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut jump_clock(1000));
        assert_eq!(f.num_nodes_considered, 64);
        assert_eq!(r.kind, ResultType::SuccessSegment);
        // past the failure timeout the search stops at once, before the first node, with no path
        let mut f = finder(&ctx, Pos::new(1, 1, 1), goal);
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut jump_clock(3000));
        assert_eq!(f.num_nodes_considered, 0);
        assert_eq!(r.kind, ResultType::Failure);
        assert!(r.path.is_none());
    }

    #[test]
    fn the_clock_is_read_every_64_nodes() {
        let fx = Fixture::new();
        let world = TestWorld::flat(400, 20);
        let ctx = fx.ctx(&world);
        let mut f = finder(&ctx, Pos::new(1, 1, 1), Goal::Block { x: 390, y: 1, z: 1 });
        // 100 ms per reading; the start is reading 0 and the check at node 0 is reading 1. Primary timeout 500: reading 5 (at node 256) is the first to reach it.
        let mut t = -100;
        let mut calls = 0;
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || {
            t += 100;
            calls += 1;
            t
        });
        assert_eq!(f.num_nodes_considered, 256);
        assert_eq!(calls, 6);
        assert_eq!(r.kind, ResultType::SuccessSegment);
    }

    #[test]
    fn cancel_before_the_search_gives_a_cancellation() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let flag = AtomicBool::new(true);
        let mut f = finder(&ctx, Pos::new(1, 1, 1), Goal::Block { x: 8, y: 1, z: 1 });
        let r = f.calculate(500, 2000, &SearchLimits { max_nodes: None, cancel: Some(&flag) }, &mut || 0);
        assert_eq!(r.kind, ResultType::Cancellation);
        assert!(r.path.is_none() && r.error.is_none());
        assert_eq!(f.num_nodes_considered, 0);
        assert!(f.is_finished());
        // the flag is the caller's and is left set
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn cancel_during_the_search_stops_it_and_never_installs_a_path() {
        let fx = Fixture::new();
        let world = TestWorld::flat(400, 20);
        let ctx = fx.ctx(&world);
        let flag = AtomicBool::new(false);
        let mut f = finder(&ctx, Pos::new(1, 1, 1), Goal::Block { x: 390, y: 1, z: 1 });
        let mut calls = 0;
        // the third reading is the check at node 64: the flag goes up there, the loop body still takes node 65, and the next turn sees the flag
        let r = f.calculate(500, 2000, &SearchLimits { max_nodes: None, cancel: Some(&flag) }, &mut || {
            calls += 1;
            if calls == 3 {
                flag.store(true, Ordering::SeqCst);
            }
            0
        });
        assert_eq!(r.kind, ResultType::Cancellation);
        assert!(r.path.is_none());
        assert_eq!(f.num_nodes_considered, 65);
    }

    #[test]
    fn a_walled_in_start_has_no_path() {
        let fx = Fixture::new();
        let mut world = TestWorld::flat(20, 20);
        for (x, z) in [(4, 5), (6, 5), (5, 4), (5, 6)] {
            world.set(x, 1, z, STONE);
            world.set(x, 2, z, STONE);
        }
        // the diagonals between the walls are blocked by the corner blocks of the walls
        for (x, z) in [(4, 4), (6, 4), (4, 6), (6, 6)] {
            world.set(x, 1, z, STONE);
            world.set(x, 2, z, STONE);
        }
        let ctx = fx.ctx(&world);
        let mut f = finder(&ctx, Pos::new(5, 1, 5), Goal::Block { x: 15, y: 1, z: 15 });
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.kind, ResultType::Failure);
        assert_eq!(f.num_nodes_considered, 1);
        assert_eq!(f.map_size(), 1);
    }

    #[test]
    fn unloaded_chunks_stop_the_search_after_the_border_fetch_limit() {
        let fx = Fixture::new();
        let mut world = TestWorld::flat(100, 100);
        // everything from x = 16 on is not loaded, and the goal is beyond it
        world.loaded_below_x = 16;
        let ctx = fx.ctx(&world);
        let mut f = finder(&ctx, Pos::new(2, 1, 50), Goal::Block { x: 60, y: 1, z: 50 });
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        // the search reaches the edge of the loaded part and gives a segment that ends there
        assert_eq!(r.kind, ResultType::SuccessSegment, "{:?}", r.error);
        let p = r.path.unwrap();
        assert!(p.dest().x < 16);
        assert!(f.num_movements_considered > 0);
    }

    #[test]
    fn a_nan_heuristic_is_an_error_value() {
        let mut cfg = test_config();
        cfg.cost_heuristic = f64::NAN;
        let fx = Fixture::with_config(cfg);
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let mut f = finder(&ctx, Pos::new(1, 1, 1), Goal::Xz { x: 8, z: 1 });
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.kind, ResultType::Exception);
        assert!(matches!(r.error, Some(SearchError::ImplausibleHeuristic { .. })));
        assert!(f.is_finished());
    }

    #[test]
    fn favoring_the_previous_path_changes_the_costs_not_the_route_on_open_ground() {
        let fx = Fixture::new();
        let world = TestWorld::flat(30, 30);
        let ctx = fx.ctx(&world);
        let mut f = finder(&ctx, Pos::new(1, 1, 1), Goal::Block { x: 8, y: 1, z: 1 });
        let first = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0).path.unwrap();
        let fav = Favoring::from_previous(Some(&first), &ctx.cfg.search);
        assert!(!fav.is_empty());
        let mut g = AStarPathFinder::new(Pos::new(1, 1, 1), 1, 1, 1, Goal::Block { x: 8, y: 1, z: 1 }, fav, &ctx);
        let second = g.calculate(500, 2000, &SearchLimits::default(), &mut || 0).path.unwrap();
        assert_eq!(second.positions(), first.positions());
        // the node costs were halved by the favoring, and the movement cost is the smaller of the two
        let half = crate::costs::SPRINT_ONE_BLOCK_COST * 0.5;
        assert!(second.movements().unwrap().iter().all(|m| (m.cost.unwrap() - half).abs() < 1e-9));
    }

    /// `BlockStateInterface.getUncached`: a block in a chunk that is not loaded is read from the cached region. A wall that only the cache knows is
    /// planned around, where the same search over the loaded chunks alone ends at their border.
    #[test]
    fn a_cached_region_beyond_the_loaded_chunks_is_planned_through() {
        use crate::cache::testkit::{cache_of, temp_dir, with_cache};
        let fx = Fixture::new();
        // what the cache was told once: a wall 3 high across x = 40 with a gap at z = 70
        let mut known = TestWorld::flat(100, 100);
        for z in 0..100 {
            if z != 70 {
                for y in 1..4 {
                    known.set(40, y, z, STONE);
                }
            }
        }
        let dir = temp_dir("astar");
        let cache = cache_of(&known, &["air", "stone"], dir.clone());
        // what the game shows now: nothing from x = 16 on is loaded
        let mut live = TestWorld::flat(100, 100);
        live.loaded_below_x = 16;
        let goal = Goal::Block { x: 60, y: 1, z: 50 };
        // without the cache: a segment that ends at the border
        let ctx = fx.ctx(&live);
        let r = finder(&ctx, Pos::new(2, 1, 50), goal.clone()).calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.kind, ResultType::SuccessSegment);
        // with it: the whole way, through the gap
        let world = with_cache(live, &cache);
        let ctx = fx.ctx(&world);
        let r = finder(&ctx, Pos::new(2, 1, 50), goal).calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.kind, ResultType::SuccessToGoal, "{:?}", r.error);
        let path = r.path.unwrap();
        let positions = path.positions();
        assert_eq!(*positions.last().unwrap(), Pos::new(60, 1, 50));
        assert!(positions.iter().any(|p| p.x == 40 && p.z == 70), "it goes through the gap");
        assert!(positions.iter().all(|p| p.x != 40 || p.z == 70), "and never through the wall");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
