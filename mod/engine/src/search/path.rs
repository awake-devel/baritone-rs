// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/calc/Path.java, src/main/java/baritone/pathing/path/CutoffPath.java,
// src/main/java/baritone/pathing/path/SplicedPath.java, src/main/java/baritone/utils/pathing/PathBase.java and
// src/api/java/baritone/api/pathing/calc/IPath.java. LGPL-3.0-or-later, see lib.rs.
//
// Upstream has `Path`, `CutoffPath` and `SplicedPath` behind `IPath`. They differ only in how they are made, so here they are one struct.
use super::node::{NodeId, PathNode};
use super::SearchError;
use crate::config::SearchSettings;
use crate::costs::java_min;
use crate::goals::{Goal, GoalSettings};
use crate::movement::helper::Ctx;
use crate::movement::moves::Moves;
use crate::movement::step::Movement;
use crate::pos::Pos;
use crate::world::WorldView;
use std::collections::HashSet;
use std::ops::Range;

/// How a path was made: only the first kind can be post-processed, as only upstream's `Path` implements `postProcess`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Origin {
    Nodes,
    Cutoff,
    Spliced,
}

/// `IPath` and its three implementations. The movements exist once `post_process` has run, or for a cut or spliced path, which are made from verified paths.
#[derive(Clone, Debug)]
pub struct Path {
    positions: Vec<Pos>,
    movements: Vec<Movement>,
    /// `getSrc`: the start of a `Path` (which can be the real start, not the first node), the first position of the others.
    src: Pos,
    num_nodes: i32,
    goal: Goal,
    origin: Origin,
    verified: bool,
    /// The cost of each node along the path, in order. Only a `Path` made from nodes has them.
    node_costs: Vec<f64>,
}

/// `List.subList(from, to)` with Java's checks.
fn sub_list(len: usize, from: i32, to: i32) -> Result<Range<usize>, SearchError> {
    if from < 0 || to < from || (to as i64) > len as i64 {
        return Err(SearchError::CutoffOutOfRange { from, to, len });
    }
    Ok(from as usize..to as usize)
}

impl Path {
    /// `Path.<init>`: walks `previous` from the end node back to the start. When the real position differs from the start node and the search found no movement, a node at the real position is added, so that one movement to the single position can be made. That node has cost 0, and its heuristic is checked like any node's.
    pub fn from_nodes(real_start: Pos, start: NodeId, end: NodeId, num_nodes: i32, goal: &Goal, gs: &GoalSettings, nodes: &[PathNode]) -> Result<Path, SearchError> {
        let mut current = Some(end);
        let mut temp_path: Vec<Pos> = Vec::new();
        let mut temp_costs: Vec<f64> = Vec::new();
        while let Some(c) = current {
            let n = &nodes[c as usize];
            temp_costs.push(n.cost);
            temp_path.push(Pos::new(n.x, n.y, n.z));
            current = n.previous;
            // upstream would loop forever on a cycle of `previous`
            if temp_path.len() > nodes.len() {
                return Err(SearchError::SanityCheck("Path doubles back on itself, making a loop"));
            }
        }
        let (s, e) = (&nodes[start as usize], &nodes[end as usize]);
        let start_node_pos = Pos::new(s.x, s.y, s.z);
        let src;
        if real_start != start_node_pos && s.same_position(e) {
            src = real_start;
            // the node itself is only a cost of 0 in a list; making it still checks the heuristic, as upstream does
            PathNode::new(real_start.x, real_start.y, real_start.z, goal, gs)?;
            temp_costs.push(0.0);
            temp_path.push(real_start);
        } else {
            src = start_node_pos;
        }
        temp_path.reverse();
        temp_costs.reverse();
        Ok(Path { positions: temp_path, movements: Vec::new(), src, num_nodes, goal: goal.clone(), origin: Origin::Nodes, verified: false, node_costs: temp_costs })
    }

    /// `Path.getGoal`: the goal the path was calculated towards.
    /// `CutoffPath.getGoal`: the same.
    /// `SplicedPath.getGoal`: the same.
    pub fn goal(&self) -> &Goal {
        &self.goal
    }

    /// `Path.positions`: every position along the path.
    /// `CutoffPath.positions`: the same.
    /// `SplicedPath.positions`: the same.
    pub fn positions(&self) -> &[Pos] {
        &self.positions
    }

    /// `Path.movements`: upstream throws "Path not yet verified" before `postProcess`; here that is an error.
    /// `CutoffPath.movements`: the same, and a cut path is always verified.
    /// `SplicedPath.movements`: the same, and a spliced path is always verified.
    pub fn movements(&self) -> Result<&[Movement], SearchError> {
        if !self.verified {
            return Err(SearchError::NotVerified);
        }
        Ok(&self.movements)
    }

    /// `Path.getNumNodesConsidered`: how many nodes the search had taken when the path was made.
    /// `CutoffPath.getNumNodesConsidered`: the same.
    /// `SplicedPath.getNumNodesConsidered`: the same.
    pub fn num_nodes_considered(&self) -> i32 {
        self.num_nodes
    }

    /// `Path.getSrc`: the start position (see the field). For a cut or spliced path it is the first position, as `IPath.getSrc` says.
    pub fn src(&self) -> Pos {
        self.src
    }

    /// `Path.getDest`: the end position, which is the last position.
    pub fn dest(&self) -> Pos {
        self.positions[self.positions.len() - 1]
    }

    /// `SplicedPath.length`: the number of positions, as `IPath.length` says for every path.
    pub fn length(&self) -> i32 {
        self.positions.len() as i32
    }

    /// `Path.assembleMovements`: makes the movements. Returns true when one became impossible during the calculation, with the movements before it kept.
    fn assemble_movements<W: WorldView>(&mut self, ctx: &Ctx<'_, W>) -> Result<bool, SearchError> {
        if self.positions.is_empty() || !self.movements.is_empty() {
            return Err(SearchError::EmptyPath);
        }
        for i in 0..self.positions.len() - 1 {
            let cost = self.node_costs[i + 1] - self.node_costs[i];
            match Path::run_backwards(ctx, self.positions[i], self.positions[i + 1], cost) {
                None => return Ok(true),
                Some(m) => self.movements.push(m),
            }
        }
        Ok(false)
    }

    /// `Path.runBackwards`: the first move, in `Moves.values()` order, whose movement from `src` ends at `dest`. Nothing checks that the movement is possible, only where it ends. Its cost is the smaller of the cost it has now and the difference of the node costs, which can be skewed by favoring.
    fn run_backwards<W: WorldView>(ctx: &Ctx<'_, W>, src: Pos, dest: Pos, cost: f64) -> Option<Movement> {
        for moves in Moves::ALL {
            let mut mv = moves.apply0(ctx, src);
            if mv.dest == dest {
                mv.override_cost(java_min(mv.calculate_cost(ctx), cost));
                return Some(mv);
            }
        }
        None
    }

    /// `Path.postProcess`: makes the movements, notes for each whether its destination was loaded, and checks the result. When a movement became impossible, the path is cut before it. Upstream throws on a second call, and `postProcess` on a cut or spliced path is `IPath`'s default, which throws too.
    pub fn post_process<W: WorldView>(mut self, ctx: &Ctx<'_, W>) -> Result<Path, SearchError> {
        if self.origin != Origin::Nodes || self.verified {
            return Err(SearchError::CannotPostProcess);
        }
        self.verified = true;
        let failed = self.assemble_movements(ctx)?;
        for m in self.movements.iter_mut() {
            m.check_loaded_chunk(ctx);
        }
        if failed {
            let res = Path::cutoff(&self, 0, self.movements.len() as i32)?;
            if res.movements.len() != self.movements.len() {
                return Err(SearchError::WrongSizeAfterCutoff);
            }
            return Ok(res);
        }
        self.sanity_check()?;
        Ok(self)
    }

    /// `CutoffPath.<init>(IPath,int,int)`: the positions `first` to `last` and the movements `first` to `last - 1` of a verified path. `CutoffPath.<init>(IPath,int)` is `first` 0. The result is checked.
    pub fn cutoff(prev: &Path, first: i32, last: i32) -> Result<Path, SearchError> {
        let pr = sub_list(prev.positions.len(), first, last.wrapping_add(1))?;
        let movements = prev.movements()?;
        let mr = sub_list(movements.len(), first, last)?;
        let path = Path { positions: prev.positions[pr].to_vec(), movements: movements[mr].to_vec(), src: Pos::new(0, 0, 0), num_nodes: prev.num_nodes, goal: prev.goal.clone(), origin: Origin::Cutoff, verified: true, node_costs: Vec::new() };
        let mut path = path;
        path.src = *path.positions.first().ok_or(SearchError::EmptyPath)?;
        path.sanity_check()?;
        Ok(path)
    }

    /// `PathBase.cutoffAtLoadedChunks`: cuts the path before the first position whose column is not loaded. Does nothing unless `cutoffAtLoadBoundary` is on.
    pub fn cutoff_at_loaded_chunks<W: WorldView>(self, world: &W, s: &SearchSettings) -> Result<Path, SearchError> {
        if !s.cutoff_at_load_boundary {
            return Ok(self);
        }
        for (i, pos) in self.positions.iter().enumerate() {
            if !world.contains_loaded_chunk(pos.x, pos.z) {
                return Path::cutoff(&self, 0, i as i32);
            }
        }
        Ok(self)
    }

    /// `PathBase.staticCutoff`: a path of `pathCutoffMinimumLength` positions or more that does not reach the goal is cut to that length plus `pathCutoffFactor` of what is above it, less one. `None` for the goal means no cut.
    pub fn static_cutoff(self, destination: Option<&Goal>, gs: &GoalSettings, s: &SearchSettings) -> Result<Path, SearchError> {
        let min = s.path_cutoff_minimum_length;
        if self.length() < min {
            return Ok(self);
        }
        let d = self.dest();
        match destination {
            None => return Ok(self),
            Some(g) if g.is_in_goal(d.x, d.y, d.z, gs) => return Ok(self),
            _ => {}
        }
        let factor = s.path_cutoff_factor;
        let new_length = ((self.length().wrapping_sub(min) as f64 * factor) as i32).wrapping_add(min).wrapping_sub(1);
        Path::cutoff(&self, 0, new_length)
    }

    /// `IPath.sanityCheck`: the path starts and ends where its positions do, has one movement fewer than positions, each movement joins its two positions, and no position is left twice.
    pub fn sanity_check(&self) -> Result<(), SearchError> {
        let path = &self.positions;
        if path.is_empty() {
            return Err(SearchError::EmptyPath);
        }
        let movements = self.movements()?;
        if self.src != path[0] {
            return Err(SearchError::SanityCheck("Start node does not equal first path element"));
        }
        if self.dest() != path[path.len() - 1] {
            return Err(SearchError::SanityCheck("End node does not equal last path element"));
        }
        if path.len() != movements.len() + 1 {
            return Err(SearchError::SanityCheck("Size of path array is unexpected"));
        }
        let mut seen_so_far: HashSet<Pos> = HashSet::new();
        for i in 0..path.len() - 1 {
            let (src, dest, movement) = (path[i], path[i + 1], &movements[i]);
            if src != movement.src {
                return Err(SearchError::SanityCheck("Path source is not equal to the movement source"));
            }
            if dest != movement.dest {
                return Err(SearchError::SanityCheck("Path destination is not equal to the movement destination"));
            }
            if seen_so_far.contains(&src) {
                return Err(SearchError::SanityCheck("Path doubles back on itself, making a loop"));
            }
            seen_so_far.insert(src);
        }
        Ok(())
    }

    /// `SplicedPath.trySplice`: joins `second` onto `first`. They must meet: the end of `first` is the start of `second`. With `allow_overlap_cutoff` off, any other shared position (the end of `first` apart) means no splice. With it on, `first` is cut at its first position that `second` also holds, and `second` continues from there. `None` for either path means no splice, as upstream's null.
    pub fn try_splice(first: Option<&Path>, second: Option<&Path>, allow_overlap_cutoff: bool) -> Result<Option<Path>, SearchError> {
        let (first, second) = match (first, second) {
            (Some(f), Some(s)) => (f, s),
            _ => return Ok(None),
        };
        if first.dest() != second.src() {
            return Ok(None);
        }
        let second_pos: HashSet<Pos> = second.positions.iter().copied().collect();
        let mut first_position_in_second: i32 = -1;
        // overlap in the very last element is fine (and required) so only go up to first.length() - 1
        for i in 0..first.length() - 1 {
            if second_pos.contains(&first.positions[i as usize]) {
                first_position_in_second = i;
                break;
            }
        }
        if first_position_in_second != -1 {
            if !allow_overlap_cutoff {
                return Ok(None);
            }
        } else {
            first_position_in_second = first.length() - 1;
        }
        let target = first.positions[first_position_in_second as usize];
        let position_in_second = second.positions.iter().position(|p| *p == target).map(|i| i as i32).unwrap_or(-1);
        if !allow_overlap_cutoff && position_in_second != 0 {
            return Err(SearchError::SpliceOverlap);
        }
        let (fm, sm) = (first.movements()?, second.movements()?);
        let mut positions: Vec<Pos> = Vec::new();
        let mut movements: Vec<Movement> = Vec::new();
        positions.extend_from_slice(&first.positions[sub_list(first.positions.len(), 0, first_position_in_second + 1)?]);
        movements.extend_from_slice(&fm[sub_list(fm.len(), 0, first_position_in_second)?]);
        positions.extend_from_slice(&second.positions[sub_list(second.positions.len(), position_in_second.wrapping_add(1), second.length())?]);
        movements.extend_from_slice(&sm[sub_list(sm.len(), position_in_second, second.length() - 1)?]);
        let src = *positions.first().ok_or(SearchError::EmptyPath)?;
        let path = Path { positions, movements, src, num_nodes: first.num_nodes.wrapping_add(second.num_nodes), goal: first.goal.clone(), origin: Origin::Spliced, verified: true, node_costs: Vec::new() };
        path.sanity_check()?;
        Ok(Some(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::step::MovementKind;
    use crate::search::node::NodeMap;
    use crate::testing::*;

    fn line(x0: i32, x1: i32, z: i32) -> Vec<Pos> {
        if x0 <= x1 {
            (x0..=x1).map(|x| Pos::new(x, 1, z)).collect()
        } else {
            (x1..=x0).rev().map(|x| Pos::new(x, 1, z)).collect()
        }
    }

    fn p(x: i32, z: i32) -> Pos {
        Pos::new(x, 1, z)
    }

    #[test]
    fn post_process_matches_moves_by_destination_in_move_order() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        // a path built by hand through an east step, a south step and a north-east diagonal
        let path = chain_path(&ctx, &[p(1, 1), p(2, 1), p(2, 2), p(3, 1)]).unwrap();
        let kinds: Vec<_> = path.movements().unwrap().iter().map(|m| m.kind).collect();
        assert_eq!(kinds, vec![MovementKind::Traverse, MovementKind::Traverse, MovementKind::Diagonal]);
        // each cost is the smaller of the calculated cost and the node cost difference (10)
        let costs: Vec<f64> = path.movements().unwrap().iter().map(|m| m.cost.unwrap()).collect();
        assert!((costs[0] - crate::costs::SPRINT_ONE_BLOCK_COST).abs() < 1e-9);
        assert!(costs[2] < 10.0);
        assert_eq!(path.num_nodes_considered(), 7);
    }

    #[test]
    fn the_node_cost_difference_caps_the_cost() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let goal = Goal::Block { x: 0, y: 0, z: 0 };
        let gs = ctx.cfg.goal_settings();
        let mut map = NodeMap::default();
        let a = map.get_node_at_position(1, 1, 1, &goal, &gs).unwrap();
        let b = map.get_node_at_position(2, 1, 1, &goal, &gs).unwrap();
        map.nodes[a as usize].cost = 0.0;
        map.nodes[b as usize].cost = 1.5;
        map.nodes[b as usize].previous = Some(a);
        let path = Path::from_nodes(p(1, 1), a, b, 2, &goal, &gs, &map.nodes).unwrap().post_process(&ctx).unwrap();
        assert_eq!(path.movements().unwrap()[0].cost, Some(1.5));
    }

    #[test]
    fn an_unverified_path_has_no_movements_and_is_verified_once() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let goal = Goal::Block { x: 0, y: 0, z: 0 };
        let gs = ctx.cfg.goal_settings();
        let mut map = NodeMap::default();
        let a = map.get_node_at_position(1, 1, 1, &goal, &gs).unwrap();
        map.nodes[a as usize].cost = 0.0;
        let raw = Path::from_nodes(p(1, 1), a, a, 1, &goal, &gs, &map.nodes).unwrap();
        assert_eq!(raw.movements().unwrap_err(), SearchError::NotVerified);
        assert_eq!(raw.length(), 1);
        let done = raw.post_process(&ctx).unwrap();
        assert!(done.movements().unwrap().is_empty());
        assert_eq!(done.clone().post_process(&ctx).unwrap_err(), SearchError::CannotPostProcess);
    }

    #[test]
    fn a_movement_that_became_impossible_cuts_the_path_before_it() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        // the second step is two blocks long: no move ends there
        let path = chain_path(&ctx, &[p(1, 1), p(2, 1), p(4, 1), p(5, 1)]).unwrap();
        assert_eq!(path.positions(), &[p(1, 1), p(2, 1)]);
        assert_eq!(path.movements().unwrap().len(), 1);
        assert_eq!(path.num_nodes_considered(), 7);
        // and when the very first step is impossible only the start is left
        let path = chain_path(&ctx, &[p(1, 1), p(4, 1)]).unwrap();
        assert_eq!(path.positions(), &[p(1, 1)]);
        assert!(path.movements().unwrap().is_empty());
    }

    #[test]
    fn an_impossible_step_can_match_the_movement_of_a_move_that_failed() {
        // Upstream quirk, kept: `Moves.apply0` does not check that its move is possible. A parkour move that fails leaves its
        // result at the reset values (0, 0, 0), so its distance is |x| + |z| of the source and it ends 2 blocks east of (1,1,1).
        // A step to that position is matched by it, with parkour off, and gets the node cost difference as its cost
        // because the calculated cost is COST_INF.
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        assert!(!ctx.cfg.allow_parkour);
        let path = chain_path(&ctx, &[p(1, 1), p(3, 1)]).unwrap();
        assert_eq!(path.positions(), &[p(1, 1), p(3, 1)]);
        let m = &path.movements().unwrap()[0];
        assert!(matches!(m.kind, MovementKind::Parkour { dist: 2, .. }));
        assert_eq!(m.cost, Some(10.0));
    }

    #[test]
    fn checks_the_loaded_chunk_of_each_destination() {
        let fx = Fixture::new();
        let mut world = TestWorld::flat(20, 20);
        world.loaded_below_x = 3;
        let ctx = fx.ctx(&world);
        let path = chain_path(&ctx, &line(1, 4, 1)).unwrap();
        let loaded: Vec<_> = path.movements().unwrap().iter().map(|m| m.calculated_while_loaded).collect();
        assert_eq!(loaded, vec![Some(true), Some(false), Some(false)]);
    }

    #[test]
    fn cutoff_at_loaded_chunks_cuts_at_the_first_unloaded_position() {
        let fx = Fixture::new();
        let mut world = TestWorld::flat(20, 20);
        world.loaded_below_x = 4;
        let ctx = fx.ctx(&world);
        let path = chain_path(&ctx, &line(1, 7, 1)).unwrap();
        let mut s = SearchSettings::default();
        // off by default: unchanged
        let same = path.clone().cutoff_at_loaded_chunks(&world, &s).unwrap();
        assert_eq!(same.length(), 7);
        s.cutoff_at_load_boundary = true;
        let cut = path.clone().cutoff_at_loaded_chunks(&world, &s).unwrap();
        // x = 4 is the first position that is not loaded: it is index 3, which is the last one kept
        assert_eq!(cut.positions(), &line(1, 4, 1)[..]);
        assert_eq!(cut.movements().unwrap().len(), 3);
        assert_eq!(cut.num_nodes_considered(), 7);
        // nothing unloaded: unchanged
        world.loaded_below_x = 100;
        assert_eq!(path.cutoff_at_loaded_chunks(&world, &s).unwrap().length(), 7);
    }

    #[test]
    fn cutoff_at_a_first_position_that_is_unloaded_leaves_the_start() {
        let fx = Fixture::new();
        let mut world = TestWorld::flat(20, 20);
        let ctx_path = {
            let ctx = fx.ctx(&world);
            chain_path(&ctx, &line(5, 8, 1)).unwrap()
        };
        world.loaded_below_x = 0;
        let s = SearchSettings { cutoff_at_load_boundary: true, ..Default::default() };
        let cut = ctx_path.cutoff_at_loaded_chunks(&world, &s).unwrap();
        assert_eq!(cut.positions(), &[p(5, 1)]);
        assert!(cut.movements().unwrap().is_empty());
    }

    #[test]
    fn static_cutoff_arithmetic() {
        let fx = Fixture::new();
        let world = TestWorld::flat(80, 20);
        let ctx = fx.ctx(&world);
        let gs = ctx.cfg.goal_settings();
        let s = SearchSettings::default();
        let elsewhere = Goal::Block { x: 500, y: 1, z: 1 };
        let make = |n: i32| chain_path(&ctx, &line(1, n, 1)).unwrap();
        // shorter than the minimum length (30): unchanged
        assert_eq!(make(29).static_cutoff(Some(&elsewhere), &gs, &s).unwrap().length(), 29);
        // exactly the minimum: (30 - 30) * 0.9 = 0, + 30 - 1 = 29 is the last index, so 30 positions: the same path
        assert_eq!(make(30).static_cutoff(Some(&elsewhere), &gs, &s).unwrap().length(), 30);
        // 41 positions: (int) (11 * 0.9) = 9, + 29 = 38 is the last index, so 39 positions
        let cut = make(41).static_cutoff(Some(&elsewhere), &gs, &s).unwrap();
        assert_eq!(cut.length(), 39);
        assert_eq!(cut.movements().unwrap().len(), 38);
        // 70 positions: (int) (40 * 0.9) = 36, + 29 = 65, so 66 positions
        assert_eq!(make(70).static_cutoff(Some(&elsewhere), &gs, &s).unwrap().length(), 66);
        // the goal is the end: unchanged
        let at_end = Goal::Block { x: 41, y: 1, z: 1 };
        assert_eq!(make(41).static_cutoff(Some(&at_end), &gs, &s).unwrap().length(), 41);
        // no goal: unchanged
        assert_eq!(make(41).static_cutoff(None, &gs, &s).unwrap().length(), 41);
        // another factor and minimum
        let s2 = SearchSettings { path_cutoff_factor: 0.5, path_cutoff_minimum_length: 10, ..Default::default() };
        // (int) (20 * 0.5) = 10, + 10 - 1 = 19, so 20 positions
        assert_eq!(make(30).static_cutoff(Some(&elsewhere), &gs, &s2).unwrap().length(), 20);
        // the double is cut to an int by truncation: (int) (3 * 0.7) = 2, + 10 - 1 = 11, so 12 positions
        let s3 = SearchSettings { path_cutoff_factor: 0.7, path_cutoff_minimum_length: 10, ..Default::default() };
        assert_eq!(make(13).static_cutoff(Some(&elsewhere), &gs, &s3).unwrap().length(), 12);
    }

    #[test]
    fn static_cutoff_with_a_factor_above_one_runs_out_of_the_path() {
        // upstream: subList throws an IndexOutOfBoundsException
        let fx = Fixture::new();
        let world = TestWorld::flat(80, 20);
        let ctx = fx.ctx(&world);
        let gs = ctx.cfg.goal_settings();
        let s = SearchSettings { path_cutoff_factor: 2.0, ..Default::default() };
        let r = chain_path(&ctx, &line(1, 40, 1)).unwrap().static_cutoff(Some(&Goal::Block { x: 500, y: 1, z: 1 }), &gs, &s);
        assert!(matches!(r, Err(SearchError::CutoffOutOfRange { .. })));
    }

    #[test]
    fn splice_without_overlap_joins_the_two() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let first = chain_path(&ctx, &line(1, 4, 1)).unwrap();
        let second = chain_path(&ctx, &[p(4, 1), p(4, 2), p(4, 3)]).unwrap();
        for allow in [false, true] {
            let s = Path::try_splice(Some(&first), Some(&second), allow).unwrap().unwrap();
            assert_eq!(s.positions(), &[p(1, 1), p(2, 1), p(3, 1), p(4, 1), p(4, 2), p(4, 3)]);
            assert_eq!(s.movements().unwrap().len(), 5);
            assert_eq!(s.num_nodes_considered(), 14);
            assert_eq!(s.src(), p(1, 1));
            assert_eq!(s.dest(), p(4, 3));
            assert_eq!(s.goal(), first.goal());
            s.sanity_check().unwrap();
        }
    }

    #[test]
    fn splice_needs_the_paths_to_meet() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let first = chain_path(&ctx, &line(1, 4, 1)).unwrap();
        let apart = chain_path(&ctx, &[p(5, 1), p(5, 2)]).unwrap();
        assert!(Path::try_splice(Some(&first), Some(&apart), true).unwrap().is_none());
        assert!(Path::try_splice(Some(&first), None, true).unwrap().is_none());
        assert!(Path::try_splice(None, Some(&apart), true).unwrap().is_none());
    }

    #[test]
    fn splice_with_overlap_is_refused_or_cut_at_the_first_shared_position() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        // first: (1,1) (2,1) (3,1) (3,2); second starts at the end of first and walks back through (2,1)... no: through (2,2), (2,1), (2,0)
        let first = chain_path(&ctx, &[p(1, 1), p(2, 1), p(3, 1), p(3, 2)]).unwrap();
        let second = chain_path(&ctx, &[p(3, 2), p(2, 2), p(2, 1), p(2, 0), p(1, 0)]).unwrap();
        // not allowed: no splice
        assert!(Path::try_splice(Some(&first), Some(&second), false).unwrap().is_none());
        // allowed: first is cut at its first position that second holds, (2,1) at index 1, and second goes on after (2,1), which is its index 2
        let s = Path::try_splice(Some(&first), Some(&second), true).unwrap().unwrap();
        assert_eq!(s.positions(), &[p(1, 1), p(2, 1), p(2, 0), p(1, 0)]);
        let m = s.movements().unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!((m[0].src, m[0].dest), (p(1, 1), p(2, 1)));
        assert_eq!((m[1].src, m[1].dest), (p(2, 1), p(2, 0)));
        assert_eq!(s.num_nodes_considered(), 14);
        s.sanity_check().unwrap();
    }

    #[test]
    fn splice_overlap_at_the_first_position() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let first = chain_path(&ctx, &[p(1, 1), p(2, 1), p(2, 2)]).unwrap();
        let second = chain_path(&ctx, &[p(2, 2), p(1, 2), p(1, 1), p(0, 1)]).unwrap();
        assert!(Path::try_splice(Some(&first), Some(&second), false).unwrap().is_none());
        let s = Path::try_splice(Some(&first), Some(&second), true).unwrap().unwrap();
        // first is cut to its first position (1,1), and second continues after (1,1), its index 2
        assert_eq!(s.positions(), &[p(1, 1), p(0, 1)]);
        assert_eq!(s.movements().unwrap().len(), 1);
    }

    #[test]
    fn splice_uses_the_real_start_of_the_second_path() {
        // second.getSrc() is the real start, which is the fake first position: it must equal the end of first
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let first = chain_path(&ctx, &line(1, 3, 1)).unwrap();
        let second = chain_path(&ctx, &line(3, 5, 1)).unwrap();
        let s = Path::try_splice(Some(&first), Some(&second), false).unwrap().unwrap();
        assert_eq!(s.length(), 5);
    }

    #[test]
    fn a_cut_or_spliced_path_cannot_be_post_processed() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let path = chain_path(&ctx, &line(1, 4, 1)).unwrap();
        let cut = Path::cutoff(&path, 0, 2).unwrap();
        assert_eq!(cut.length(), 3);
        assert_eq!(cut.post_process(&ctx).unwrap_err(), SearchError::CannotPostProcess);
        assert!(matches!(Path::cutoff(&path, 0, 9), Err(SearchError::CutoffOutOfRange { .. })));
        assert!(matches!(Path::cutoff(&path, -1, 2), Err(SearchError::CutoffOutOfRange { .. })));
        // a cut can start after the first position
        let mid = Path::cutoff(&path, 1, 3).unwrap();
        assert_eq!(mid.positions(), &line(2, 4, 1)[..]);
        assert_eq!(mid.src(), p(2, 1));
    }

    #[test]
    fn sanity_check_finds_each_fault() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        let good = chain_path(&ctx, &line(1, 4, 1)).unwrap();
        good.sanity_check().unwrap();
        let mut bad = good.clone();
        bad.movements.pop();
        assert_eq!(bad.sanity_check(), Err(SearchError::SanityCheck("Size of path array is unexpected")));
        let mut bad = good.clone();
        bad.movements[1].src = p(9, 9);
        assert_eq!(bad.sanity_check(), Err(SearchError::SanityCheck("Path source is not equal to the movement source")));
        let mut bad = good.clone();
        bad.movements[1].dest = p(9, 9);
        assert_eq!(bad.sanity_check(), Err(SearchError::SanityCheck("Path destination is not equal to the movement destination")));
        let mut bad = good.clone();
        bad.src = p(9, 9);
        assert_eq!(bad.sanity_check(), Err(SearchError::SanityCheck("Start node does not equal first path element")));
        // a loop: the start is visited again
        let mut bad = good.clone();
        bad.positions = vec![p(1, 1), p(2, 1), p(1, 1), p(2, 1)];
        bad.movements = vec![Movement::new(MovementKind::Traverse, p(1, 1), p(2, 1)), Movement::new(MovementKind::Traverse, p(2, 1), p(1, 1)), Movement::new(MovementKind::Traverse, p(1, 1), p(2, 1))];
        assert_eq!(bad.sanity_check(), Err(SearchError::SanityCheck("Path doubles back on itself, making a loop")));
    }
}
