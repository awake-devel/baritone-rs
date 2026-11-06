#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Injects one deliberate error at a time into a copy of the engine's search code and checks that the L2 comparison catches it.

Usage: mutate.py REPO GOLDEN_FILE ["name|name|..."]    run the mutations (all, or the named ones)
       mutate.py REPO --where                          check that every pattern occurs exactly once and every name is unique, without building

The parts of the workspace the comparison needs (root Cargo.toml and Cargo.lock, rustfmt.toml, mod/engine, tools/search-compare and
tools/cost-compare's manifest and sources) are copied into a temporary directory, once per worker, each with its own target folder; the
repository is never touched. BARITONE_MUTATE_JOBS sets the number of workers (default as in tools/cost-compare/mutate.py, whose
low_priority and default_jobs are imported: half of the threads, or 2 while a game or another build is running).

A mutation is CAUGHT when the comparison reports mismatches, aborts, reports a coverage gap, or runs over RUN_TIMEOUT seconds (the
unmutated run exits 0 in under a minute). It is MISSED when the comparison reports none. It is 'not applied' when its pattern is not in
the file exactly once, when the mutant does not build, or when the machine killed the run (that says nothing about the error). For a
MISSED mutation the engine's unit tests are run as well and a note says whether they notice it; that never turns MISSED into CAUGHT.
The output lines are the ones tools/gate.py reads ('CAUGHT  wrong=N close=0 name', 'MISSED ...', 'not applied: [...]'), where
'wrong' is the number of mismatches. Missed mutations are listed in known-missed.txt as 'name | no effect: reason' or 'name | hole: reason'.

Each mutation is (file, pattern, replacement, name). Patterns match source text exactly, which is why rustfmt.toml sets a very wide
max_width. Timeouts and cancellation are outside L2 (the clock is fake, the harness never cancels), so mutations of them are expected
to be MISSED and are listed as holes; the unit tests in astar.rs cover them.
"""
import os
import importlib.util
import queue
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor

E = "mod/engine/src/"
ASTAR = E + "search/astar.rs"
NODE = E + "search/node.rs"
HEAP = E + "search/open_set.rs"
PATH = E + "search/path.rs"
FAV = E + "search/favoring.rs"
PLAN = E + "plan.rs"
STEP = E + "movement/step.rs"
MOVES = E + "movement/moves.rs"
POS = E + "pos.rs"

# (file, pattern, replacement, name)
M = [
    (ASTAR, "[1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 10.0]", "[1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 8.0]", "coefficients: last one 8"),
    (ASTAR, "[1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 10.0]", "[2.0, 1.5, 2.5, 3.0, 4.0, 5.0, 10.0]", "coefficients: first two swapped"),
    (ASTAR, "const MIN_DIST_PATH: f64 = 5.0;", "const MIN_DIST_PATH: f64 = 4.0;", "min dist 4"),
    (ASTAR, "const MIN_IMPROVEMENT: f64 = 0.01;", "const MIN_IMPROVEMENT: f64 = 0.02;", "min improvement 0.02"),
    (ASTAR, "COEFFICIENTS.map(|c| 1.0 / c)", "COEFFICIENTS.map(|c| 1.0 / (c * 1.0000001))", "inverse coefficients a little off"),
    (ASTAR, "n.combined_cost = n.estimated_cost_to_goal;", "n.combined_cost = 0.0;", "start node: combined cost 0"),
    (ASTAR, "let mut best_heuristic_so_far = [self.map.nodes[start_node as usize].estimated_cost_to_goal; 7];", "let mut best_heuristic_so_far = [self.map.nodes[start_node as usize].estimated_cost_to_goal + 1.0; 7];", "best heuristic starts high"),
    (ASTAR, "self.popped_hash = pop_hash_step(self.popped_hash, cur_x, cur_y, cur_z);\n            if self.goal.is_in_goal(cur_x, cur_y, cur_z, &gs) {", "self.popped_hash = pop_hash_step(self.popped_hash, cur_x, cur_y, cur_z);\n            if self.goal.is_in_goal(cur_x, cur_y, cur_z, &gs) && num_nodes > 1 {", "goal: not taken at the first node"),
    (ASTAR, "(new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4)", "(new_x >> 4 != cur_x >> 4 && new_z >> 4 != cur_z >> 4)", "chunk change: and instead of or"),
    (ASTAR, "(new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4)", "(new_x / 16 != cur_x / 16 || new_z / 16 != cur_z / 16)", "chunk change: division instead of shift"),
    (ASTAR, "!ctx.world.is_loaded(new_x, new_z)", "!ctx.world.contains_loaded_chunk(new_x, new_z)", "unloaded check: live check instead of isLoaded"),
    (ASTAR, "num_empty_chunk = num_empty_chunk.wrapping_add(1);", "num_empty_chunk = num_empty_chunk.wrapping_add(2);", "empty chunks counted twice"),
    (ASTAR, "if !moves.dynamic_xz() {\n                        // only increment", "if true {\n                        // only increment", "empty chunks: dynamic moves counted too"),
    (ASTAR, "if !moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {", "if !moves.dynamic_xz() && !border.entirely_contains(new_x, cur_z) {", "border: static check on the wrong z"),
    (ASTAR, "if cur_y.wrapping_add(moves.y_offset()) > max_y ||", "if cur_y.wrapping_add(moves.y_offset()) >= max_y ||", "height: max excluded"),
    (ASTAR, "|| cur_y.wrapping_add(moves.y_offset()) < min_y {", "|| cur_y.wrapping_add(moves.y_offset()) <= min_y {", "height: min excluded"),
    (ASTAR, "if action_cost >= COST_INF {", "if action_cost > COST_INF {", "cost inf: greater only"),
    (ASTAR, "if moves.dynamic_xz() && !border.entirely_contains(res.x, res.z) {", "if moves.dynamic_xz() && border.entirely_contains(res.x, res.z) {", "border: dynamic check inverted"),
    (ASTAR, "let hash_code = long_hash(res.x, res.y, res.z);", "let hash_code = long_hash(res.x, res.y, res.z) ^ 1;", "favoring key off by one bit"),
    (ASTAR, "action_cost *= n.favor;", "action_cost *= n.favor * 1.001;", "favor a little off"),
    (ASTAR, "let tentative_cost = self.map.nodes[current as usize].cost + action_cost;", "let tentative_cost = self.map.nodes[current as usize].cost + action_cost + 0.0001;", "tentative cost a little high"),
    (ASTAR, "if improvement > minimum_improvement {", "if improvement >= minimum_improvement {", "improvement: at least"),
    (ASTAR, "n.combined_cost = tentative_cost + n.estimated_cost_to_goal;", "n.combined_cost = tentative_cost;", "combined cost without the heuristic"),
    (ASTAR, "if best_heuristic_so_far[i] - heuristic > minimum_improvement {", "if best_heuristic_so_far[i] - heuristic > 0.0 {", "best node: any improvement"),
    (ASTAR, "let heuristic = n.estimated_cost_to_goal + n.cost * inverse_coefficients[i];", "let heuristic = n.estimated_cost_to_goal + n.cost * inverse_coefficients[0];", "best node: first coefficient only"),
    (ASTAR, "if (num_nodes as u32) >= max {", "if (num_nodes as u32) > max {", "node cap: one more"),
    (ASTAR, "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) as f64", "x_diff.wrapping_mul(x_diff).wrapping_add(z_diff.wrapping_mul(z_diff)) as f64", "distance from start without y"),
    (ASTAR, "if dist > MIN_DIST_PATH * MIN_DIST_PATH {\n                // square the comparison", "if dist >= MIN_DIST_PATH * MIN_DIST_PATH {\n                // square the comparison", "partial path: at least 5 blocks"),
    (ASTAR, "for i in 0..COEFFICIENTS.len() {\n            let Some(best) = self.best_so_far[i] else {", "for i in (0..COEFFICIENTS.len()).rev() {\n            let Some(best) = self.best_so_far[i] else {", "partial path: last coefficient first"),
    (ASTAR, "let path = path.static_cutoff(Some(&self.goal), &gs, s)?;", "let path = path.static_cutoff(None, &gs, s)?;", "static cutoff without the goal"),
    (ASTAR, "let path = path.cutoff_at_loaded_chunks(ctx.world, s)?;", "let path = path.static_cutoff(Some(&self.goal), &gs, s)?.cutoff_at_loaded_chunks(ctx.world, s)?;", "cutoffs in the other order, static one twice"),
    (ASTAR, "if self.goal.is_in_goal(d.x, d.y, d.z, &gs) {\n            Ok(PathCalculationResult { kind: ResultType::SuccessToGoal", "if !self.goal.is_in_goal(d.x, d.y, d.z, &gs) {\n            Ok(PathCalculationResult { kind: ResultType::SuccessToGoal", "result type: goal check inverted"),
    (ASTAR, "return Path::from_nodes(self.real_start, start_node, current, num_nodes, &self.goal, &gs, &self.map.nodes).map(Some);", "return Path::from_nodes(self.real_start, start_node, current, num_nodes.wrapping_sub(1), &self.goal, &gs, &self.map.nodes).map(Some);", "goal path: node count one less"),
    (ASTAR, "return Path::from_nodes(self.real_start, start_node, best, num_nodes, &self.goal, &gs, &self.map.nodes).map(Some);", "return Path::from_nodes(self.real_start, start_node, best, 0, &self.goal, &gs, &self.map.nodes).map(Some);", "partial path: node count 0"),
    (NODE, "(x & ((1 << CELL_BITS_X) - 1))", "(x & ((1 << CELL_BITS_X) - 2))", "cell index: x mask"),
    (NODE, "((z & ((1 << CELL_BITS_Z) - 1)) << CELL_BITS_X)", "((z & ((1 << CELL_BITS_Z) - 2)) << CELL_BITS_X)", "cell index: z mask"),
    (NODE, "(y & ((1 << CELL_BITS_Y) - 1)) << (CELL_BITS_X + CELL_BITS_Z)", "(y & ((1 << CELL_BITS_Y) - 2)) << (CELL_BITS_X + CELL_BITS_Z)", "cell index: y mask"),
    (NODE, "cost: COST_INF, combined_cost: 0.0", "cost: COST_INF - 1.0, combined_cost: 0.0", "node cost starts below infinity"),
    (NODE, "if estimated.is_nan() {", "if false {", "heuristic NaN not checked"),
    (HEAP, "while index > 1 && self.keys[parent_ind as usize] > cost {", "while index > 1 && self.keys[parent_ind as usize] >= cost {", "heap: sift up past equal keys"),
    (HEAP, "if child < self.size && self.keys[child as usize + 1] < child_cost {", "if child < self.size && self.keys[child as usize + 1] <= child_cost {", "heap: right child on a tie"),
    (HEAP, "if cost <= child_cost {\n                break;", "if cost < child_cost {\n                break;", "heap: sift down past equal keys"),
    (FAV, "if coeff != 1.0 {", "if true {", "favoring: coefficient 1 not skipped"),
    (FAV, "favorings.insert(pos.long_hash(), coeff);", "favorings.insert(pos.long_hash() ^ 1, coeff);", "favoring: previous path hash"),
    (FAV, "map.insert(hash, old * self.coefficient);", "map.insert(hash, old + self.coefficient);", "avoidance: sum instead of product"),
    (FAV, "<= radius.wrapping_mul(radius) {\n                        let hash", "< radius.wrapping_mul(radius) {\n                        let hash", "avoidance: sphere excludes its edge"),
    (FAV, "if s.mob_spawner_avoidance_coefficient != 1.0 {", "if true {", "avoidance: spawners with coefficient 1"),
    (FAV, "if !s.avoidance {\n            return res;", "if false {\n            return res;", "avoidance: setting ignored"),
    (PATH, "if real_start != start_node_pos && s.same_position(e) {", "if real_start != start_node_pos {", "fake node: any path"),
    (PATH, "let cost = self.node_costs[i + 1] - self.node_costs[i];", "let cost = self.node_costs[i + 1];", "movement cost: node cost, not the difference"),
    (PATH, "mv.override_cost(java_min(mv.calculate_cost(ctx), cost));", "mv.override_cost(mv.calculate_cost(ctx));", "movement cost: the calculated cost only"),
    (PATH, "mv.override_cost(java_min(mv.calculate_cost(ctx), cost));", "mv.override_cost(cost);", "movement cost: the node difference only"),
    (PATH, "for moves in Moves::ALL {\n            let mut mv", "for moves in Moves::ALL.into_iter().rev() {\n            let mut mv", "runBackwards: reverse move order"),
    (PATH, "let res = Path::cutoff(&self, 0, self.movements.len() as i32)?;", "let res = Path::cutoff(&self, 0, (self.movements.len() as i32).saturating_sub(1))?;", "post process: cut one too early"),
    (PATH, "m.check_loaded_chunk(ctx);", "let _ = &m;", "loaded flag not set"),
    (PATH, "if !world.contains_loaded_chunk(pos.x, pos.z) {\n                return Path::cutoff(&self, 0, i as i32);", "if !world.is_loaded(pos.x, pos.z) {\n                return Path::cutoff(&self, 0, i as i32);", "load boundary: isLoaded instead"),
    (PATH, "return Path::cutoff(&self, 0, i as i32);", "return Path::cutoff(&self, 0, (i as i32).saturating_sub(1));", "load boundary: cut one position early"),
    (PATH, "((self.length().wrapping_sub(min) as f64 * factor) as i32)", "((self.length().wrapping_sub(min) as f64 * factor).round() as i32)", "static cutoff: round instead of truncate"),
    (PATH, ".wrapping_add(min).wrapping_sub(1);\n        Path::cutoff", ".wrapping_add(min);\n        Path::cutoff", "static cutoff: one more position"),
    (PATH, "Some(g) if g.is_in_goal(d.x, d.y, d.z, gs) => return Ok(self),", "Some(g) if g.is_in_goal(d.x, d.y, d.z, gs) && self.length() > min => return Ok(self),", "static cutoff: goal check needs a longer path"),
    (PATH, "for i in 0..first.length() - 1 {", "for i in 0..first.length() {", "splice: overlap search includes the end"),
    (PATH, "if !allow_overlap_cutoff {\n                return Ok(None);", "if allow_overlap_cutoff {\n                return Ok(None);", "splice: overlap rule inverted"),
    (PATH, "num_nodes: first.num_nodes.wrapping_add(second.num_nodes)", "num_nodes: first.num_nodes", "splice: node count of the first"),
    (PATH, "if first.dest() != second.src() {", "if first.dest() == second.src() {", "splice: meeting condition inverted"),
    (PATH, "movements.extend_from_slice(&sm[sub_list(sm.len(), position_in_second, second.length() - 1)?]);", "movements.extend_from_slice(&sm[sub_list(sm.len(), position_in_second + 1, second.length() - 1)?]);", "splice: second movements skip one"),
    (PLAN, "feet.y == start.y && sub.x.wrapping_abs() <= 1 && sub.z.wrapping_abs() <= 1", "feet.y == start.y && sub.x.wrapping_abs() < 1 && sub.z.wrapping_abs() <= 1", "real start: x distance under 1"),
    (PLAN, "feet.y == start.y && sub.x.wrapping_abs() <= 1", "sub.x.wrapping_abs() <= 1", "real start: height not compared"),
    (PLAN, "if !world.contains_loaded_chunk(pos.x, pos.z) {\n                return Goal::Xz", "if !world.is_loaded(pos.x, pos.z) {\n                return Goal::Xz", "simplify goal: isLoaded instead"),
    (PLAN, "| Goal::GetToBlock { x, y, z } | Goal::Near { x, y, z, .. } => Some", "| Goal::GetToBlock { x, y, z } => Some", "goal render pos: no GoalNear"),
    (PLAN, "let favoring = Favoring::new(previous, s, avoidances);", "let favoring = Favoring::new(None, s, avoidances);", "createPathfinder: no previous path"),
    (PLAN, "create_pathfinder(previous.dest(), goal, Some(previous), ctx, feet, avoidances)", "create_pathfinder(previous.dest(), goal, None, ctx, feet, avoidances)", "next segment: no favoring"),
    (STEP, "let ascend = res.y > src.y;", "let ascend = res.y >= src.y;", "parkour movement: ascend at equal height"),
    (STEP, "self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.dest.x, self.dest.z));", "self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.src.x, self.src.z));", "loaded flag: source column"),
    (STEP, "if result.y != d.y {\n                    return COST_INF;\n                }\n                result.cost\n            }\n            MovementKind::Diagonal", "if result.y != d.y + 1 {\n                    return COST_INF;\n                }\n                result.cost\n            }\n            MovementKind::Diagonal", "descend cost: height check off"),
    (MOVES, "let kind = if res.y == src.y.wrapping_sub(1) { MovementKind::Descend } else { MovementKind::Fall };", "let kind = if res.y == src.y.wrapping_sub(2) { MovementKind::Descend } else { MovementKind::Fall };", "apply0: descend or fall"),
    (MOVES, "Moves::DiagonalNortheast => (Direction::North, Direction::East),", "Moves::DiagonalNortheast => (Direction::North, Direction::West),", "apply0: diagonal direction"),
    (MOVES, "let dy = res.y.wrapping_sub(src.y);", "let dy = 0;", "apply0: diagonal height"),
    (POS, "hash = 8734625i64.wrapping_mul(hash).wrapping_add(y as i64);", "hash = 8734625i64.wrapping_mul(hash).wrapping_add(z as i64);", "long hash: y term"),
]

HELPER = E + "movement/helper.rs"
WORLD = E + "world.rs"

M += [
    # --- open set: sift directions, ties, 1-based indices, shifts
    (HEAP, "let result = self.array[1];", "let result = self.array[2];", "heap: lowest taken from slot 2"),
    (HEAP, "let val = self.array[self.size as usize];", "let val = self.array[1];", "heap: last element taken from the root"),
    (HEAP, "let cost = self.keys[self.size as usize];\n        self.array", "let cost = self.keys[1];\n        self.array", "heap: last key taken from the root"),
    (HEAP, "self.array[self.size as usize] = NONE;", "", "heap: removed slot not cleared"),
    (HEAP, "nodes[result as usize].heap_position = -1;", "", "heap: removed node keeps its position"),
    (HEAP, "if self.size == 0 {\n            return Some(result);", "if false {\n            return Some(result);", "heap: single element early return"),
    (HEAP, "let mut child: i32 = 2;", "let mut child: i32 = 3;", "heap: first child is 3"),
    (HEAP, "while child <= self.size {", "while child < self.size {", "heap: sift down stops one early"),
    (HEAP, "if child < self.size && self.keys[child as usize + 1] < child_cost {", "if child <= self.size && self.keys[child as usize + 1] < child_cost {", "heap: right child read past the end"),
    (HEAP, "if child < self.size && self.keys[child as usize + 1] < child_cost {", "if self.keys[child as usize + 1] < child_cost {", "heap: right child without size check"),
    (HEAP, "if child < self.size && self.keys[child as usize + 1] < child_cost {", "if child < self.size && self.keys[child as usize + 1] > child_cost {", "heap: larger right child chosen"),
    (HEAP, "child += 1;\n                child_cost = self.keys[child as usize];", "child += 1;", "heap: right child cost not taken"),
    (HEAP, "if cost <= child_cost {\n                break;", "if cost <= child_cost + 1e-9 {\n                break;", "heap: sift down stops on nearly equal keys"),
    (HEAP, "if cost <= child_cost {\n                break;", "if cost >= child_cost {\n                break;", "heap: sift down comparison inverted"),
    (HEAP, "child = index << 1;", "child = (index << 1) + 1;", "heap: next child index off by one"),
    (HEAP, "self.keys[index as usize] = child_cost;", "", "heap: child key not moved up"),
    (HEAP, "nodes[child_node as usize].heap_position = index;", "", "heap: moved child keeps its position"),
    (HEAP, "nodes[val as usize].heap_position = index;\n        Some(result)", "Some(result)", "heap: reinserted node keeps its position"),
    (HEAP, "let mut parent_ind = index >> 1;", "let mut parent_ind = (index + 1) >> 1;", "heap: first parent index off by one"),
    (HEAP, "index = parent_ind;\n            parent_ind = index >> 1;", "index = parent_ind;\n            parent_ind = (index + 1) >> 1;", "heap: next parent index off by one"),
    (HEAP, "while index > 1 && self.keys[parent_ind as usize] > cost {", "while index > 2 && self.keys[parent_ind as usize] > cost {", "heap: sift up stops below the root"),
    (HEAP, "while index > 1 && self.keys[parent_ind as usize] > cost {", "while index > 1 && self.keys[parent_ind as usize] < cost {", "heap: sift up direction inverted"),
    (HEAP, "self.array[index as usize] = parent_node;", "", "heap: parent not moved down"),
    (HEAP, "self.keys[index as usize] = self.keys[parent_ind as usize];", "", "heap: parent key not moved down"),
    (HEAP, "nodes[parent_node as usize].heap_position = index;", "", "heap: moved parent keeps its position"),
    (HEAP, "self.keys[index as usize] = cost;\n        nodes[val as usize].heap_position = index;\n    }", "self.keys[index as usize] = cost;\n    }", "heap: sifted node keeps its position"),
    (HEAP, "let (pos, cost) = (n.heap_position, n.combined_cost);", "let (pos, cost) = (1, n.combined_cost);", "heap: update sifts from the root"),
    (HEAP, "let (pos, cost) = (n.heap_position, n.combined_cost);", "let (pos, cost) = (n.heap_position, n.cost);", "heap: update uses the cost, not the combined cost"),
    (HEAP, "let cost = nodes[value as usize].combined_cost;", "let cost = nodes[value as usize].cost;", "heap: insert uses the cost, not the combined cost"),
    (HEAP, "if self.size as usize >= self.array.len() - 1 {", "if self.size as usize >= self.array.len() {", "heap: growth one insert late"),
    (HEAP, "let n = self.array.len() << 1;", "let n = self.array.len();", "heap: no growth"),
    # --- node
    (NODE, "(((x >> CELL_BITS_X) & 0xFFFFFFF) as i64) << 36", "(((x >> CELL_BITS_X) & 0xFFFFF) as i64) << 36", "cell key: x mask short"),
    (NODE, "(((z >> CELL_BITS_Z) & 0xFFFFFFF) as i64) << 8", "(((z >> CELL_BITS_Z) & 0xFFFFF) as i64) << 8", "cell key: z mask short"),
    (NODE, "((y >> CELL_BITS_Y) & 0xFF) as i64", "((y >> CELL_BITS_X) & 0xFF) as i64", "cell key: y shift"),
    (NODE, "((y >> CELL_BITS_Y) & 0xFF) as i64", "((y >> CELL_BITS_Y) & 0x7F) as i64", "cell key: y mask short"),
    (NODE, "((y >> CELL_BITS_Y) & 0xFF) as i64", "(y & 0xFF) as i64", "cell key: y without shift"),
    (NODE, "<< (CELL_BITS_X + CELL_BITS_Z)) | ((z &", "<< CELL_BITS_X) | ((z &", "cell index: y shift"),
    (NODE, "if cell[index] == EMPTY {", "if true {", "node map: always a new node"),
    (NODE, "cell[index] = id;", "", "node map: new node not stored"),
    (NODE, "heap_position: -1, favor: 0.0 })", "heap_position: 0, favor: 0.0 })", "node: heap position starts at 0"),
    (NODE, "heap_position: -1, favor: 0.0 })", "heap_position: -1, favor: 1.0 })", "node: favor starts at 1"),
    (NODE, "self.x == other.x && self.y == other.y && self.z == other.z", "self.x == other.x && self.y == other.y", "node: same position ignores z"),
    (NODE, "self.x == other.x && self.y == other.y && self.z == other.z", "self.x == other.x && self.z == other.z", "node: same position ignores y"),
    (NODE, "self.x == other.x && self.y == other.y && self.z == other.z", "self.y == other.y && self.z == other.z", "node: same position ignores x"),
    (NODE, "let estimated = goal.heuristic(x, y, z, gs);", "let estimated = goal.heuristic(x, y, z, gs) + 1.0;", "node: heuristic plus one"),
    # --- astar: the main loop
    (ASTAR, "while !open_set.is_empty() && num_empty_chunk < pathing_max_chunk_border_fetch", "while num_empty_chunk < pathing_max_chunk_border_fetch", "loop: open set emptiness ignored"),
    (ASTAR, "num_empty_chunk < pathing_max_chunk_border_fetch && !is_cancelled(limits) {\n            if", "num_empty_chunk <= pathing_max_chunk_border_fetch && !is_cancelled(limits) {\n            if", "loop: empty chunk limit one more"),
    (ASTAR, "num_empty_chunk < pathing_max_chunk_border_fetch && !is_cancelled(limits) {\n            if", "num_empty_chunk < pathing_max_chunk_border_fetch {\n            if", "loop: cancel flag not read"),
    (ASTAR, "if (num_nodes & (time_check_interval - 1)) == 0 {", "if (num_nodes & (time_check_interval - 1)) == 1 {", "clock: checked at the second node of 64"),
    (ASTAR, "let time_check_interval: i32 = 1 << 6;", "let time_check_interval: i32 = 1 << 5;", "clock: checked every 32 nodes"),
    (ASTAR, "if now.wrapping_sub(failure_timeout_time) >= 0 || (!failing && now.wrapping_sub(primary_timeout_time) >= 0) {", "if now.wrapping_sub(failure_timeout_time) > 0 || (!failing && now.wrapping_sub(primary_timeout_time) >= 0) {", "clock: failure timeout needs to be passed"),
    (ASTAR, "if now.wrapping_sub(failure_timeout_time) >= 0 || (!failing && now.wrapping_sub(primary_timeout_time) >= 0) {", "if now.wrapping_sub(failure_timeout_time) >= 0 && (!failing && now.wrapping_sub(primary_timeout_time) >= 0) {", "clock: both timeouts needed"),
    (ASTAR, "if now.wrapping_sub(failure_timeout_time) >= 0 || (!failing && now.wrapping_sub(primary_timeout_time) >= 0) {", "if now.wrapping_sub(failure_timeout_time) >= 0 || (failing && now.wrapping_sub(primary_timeout_time) >= 0) {", "clock: primary timeout while failing"),
    (ASTAR, "if now.wrapping_sub(failure_timeout_time) >= 0 || (!failing && now.wrapping_sub(primary_timeout_time) >= 0) {", "if now.wrapping_sub(failure_timeout_time) >= 0 || now.wrapping_sub(primary_timeout_time) >= 0 {", "clock: primary timeout ignores failing"),
    (ASTAR, "if now.wrapping_sub(failure_timeout_time) >= 0 || (!failing && now.wrapping_sub(primary_timeout_time) >= 0) {", "if now.wrapping_sub(failure_timeout_time) >= 0 || (!failing && now.wrapping_sub(primary_timeout_time) > 0) {", "clock: primary timeout needs to be passed"),
    (ASTAR, "let failure_timeout_time = start_time.wrapping_add(failure_timeout);", "let failure_timeout_time = start_time.wrapping_add(primary_timeout);", "clock: failure timeout from the primary one"),
    (ASTAR, "let primary_timeout_time = start_time.wrapping_add(primary_timeout);", "let primary_timeout_time = start_time.wrapping_add(failure_timeout);", "clock: primary timeout from the failure one"),
    (ASTAR, "let mut failing = true;", "let mut failing = false;", "failing flag starts false"),
    (ASTAR, "                failing = false;\n", "", "failing flag never cleared"),
    (ASTAR, "if failing && self.get_dist_from_start_sq(neighbor) > MIN_DIST_PATH * MIN_DIST_PATH {", "if failing && self.get_dist_from_start_sq(neighbor) >= MIN_DIST_PATH * MIN_DIST_PATH {", "failing flag: at least 5 blocks"),
    (ASTAR, "if failing && self.get_dist_from_start_sq(neighbor) > MIN_DIST_PATH * MIN_DIST_PATH {", "if failing && self.get_dist_from_start_sq(current) > MIN_DIST_PATH * MIN_DIST_PATH {", "failing flag: distance of the current node"),
    (ASTAR, "self.most_recent_considered = Some(current);", "", "most recent node not kept"),
    (ASTAR, "num_nodes = num_nodes.wrapping_add(1);\n            let (cur_x", "num_nodes = num_nodes.wrapping_add(2);\n            let (cur_x", "node count steps by two"),
    (ASTAR, "if self.goal.is_in_goal(cur_x, cur_y, cur_z, &gs) {\n                self.num_nodes_considered", "if self.goal.is_in_goal(cur_z, cur_y, cur_x, &gs) {\n                self.num_nodes_considered", "goal test: x and z swapped"),
    (ASTAR, "if self.goal.is_in_goal(cur_x, cur_y, cur_z, &gs) {\n                self.num_nodes_considered", "if self.goal.is_in_goal(cur_x, cur_y.wrapping_add(1), cur_z, &gs) {\n                self.num_nodes_considered", "goal test: y one too high"),
    (ASTAR, "self.num_nodes_considered = num_nodes;\n                self.num_movements_considered", "self.num_nodes_considered = 0;\n                self.num_movements_considered", "goal found: node count not kept"),
    (ASTAR, "self.num_movements_considered = num_movements_considered;\n                self.stats.heap_ties", "self.num_movements_considered = 0;\n                self.stats.heap_ties", "goal found: movement count not kept"),
    (ASTAR, "self.num_nodes_considered = num_nodes;\n        self.num_movements_considered", "self.num_nodes_considered = 0;\n        self.num_movements_considered", "search ended: node count not kept"),
    (ASTAR, "self.num_movements_considered = num_movements_considered;\n        self.stats.heap_ties", "self.num_movements_considered = 0;\n        self.stats.heap_ties", "search ended: movement count not kept"),
    (ASTAR, "let new_x = cur_x.wrapping_add(moves.x_offset());", "let new_x = cur_x.wrapping_add(moves.z_offset());", "new x from the z offset"),
    (ASTAR, "let new_z = cur_z.wrapping_add(moves.z_offset());", "let new_z = cur_z.wrapping_add(moves.x_offset());", "new z from the x offset"),
    (ASTAR, "if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(new_x, new_z) {", "if (new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(new_x, new_z) {", "chunk change: x ignored"),
    (ASTAR, "if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(new_x, new_z) {", "if (new_x >> 4 != cur_x >> 4) && !ctx.world.is_loaded(new_x, new_z) {", "chunk change: z ignored"),
    (ASTAR, "if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(new_x, new_z) {", "if (new_x >> 5 != cur_x >> 5 || new_z >> 5 != cur_z >> 5) && !ctx.world.is_loaded(new_x, new_z) {", "chunk change: 32 wide chunks"),
    (ASTAR, "if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(new_x, new_z) {", "if !ctx.world.is_loaded(new_x, new_z) {", "chunk change: always checked"),
    (ASTAR, "if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(new_x, new_z) {", "if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && ctx.world.is_loaded(new_x, new_z) {", "chunk change: loaded check inverted"),
    (ASTAR, "if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(new_x, new_z) {", "if (new_x >> 4 != cur_x >> 4 || new_z >> 4 != cur_z >> 4) && !ctx.world.is_loaded(cur_x, new_z) {", "chunk change: loaded check on the wrong x"),
    (ASTAR, "num_empty_chunk = num_empty_chunk.wrapping_add(1);\n                    }\n                    continue;", "num_empty_chunk = num_empty_chunk.wrapping_add(1);\n                    }", "unloaded move not skipped"),
    (ASTAR, "if !moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {", "if moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {", "border: static check on dynamic moves"),
    (ASTAR, "if !moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {", "if !border.entirely_contains(new_x, new_z) {", "border: static check on every move"),
    (ASTAR, "if !moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {", "if !moves.dynamic_xz() && !border.entirely_contains(cur_x, new_z) {", "border: static check on the wrong x"),
    (ASTAR, "if !moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {\n                    self.stats.border_rejections += 1;\n                    continue;", "if !moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {\n                    self.stats.border_rejections += 1;", "border: static rejection not skipped"),
    (ASTAR, "if moves.dynamic_xz() && !border.entirely_contains(res.x, res.z) {\n                    self.stats.border_rejections += 1;\n                    continue;", "if moves.dynamic_xz() && !border.entirely_contains(res.x, res.z) {\n                    self.stats.border_rejections += 1;", "border: dynamic rejection not skipped"),
    (ASTAR, "if moves.dynamic_xz() && !border.entirely_contains(res.x, res.z) {", "if moves.dynamic_xz() && !border.entirely_contains(res.z, res.x) {", "border: dynamic check x and z swapped"),
    (ASTAR, "if moves.dynamic_xz() && !border.entirely_contains(res.x, res.z) {", "if !border.entirely_contains(res.x, res.z) {", "border: dynamic check on every move"),
    (ASTAR, "if moves.dynamic_xz() && !border.entirely_contains(res.x, res.z) {", "if moves.dynamic_xz() && !border.entirely_contains(new_x, new_z) {", "border: dynamic check on the nominal destination"),
    (ASTAR, "if cur_y.wrapping_add(moves.y_offset()) > max_y ||", "if cur_y.wrapping_sub(moves.y_offset()) > max_y ||", "height: offset sign"),
    (ASTAR, "|| cur_y.wrapping_add(moves.y_offset()) < min_y {\n                    continue;", "&& cur_y.wrapping_add(moves.y_offset()) < min_y {\n                    continue;", "height: bounds need both"),
    (ASTAR, "|| cur_y.wrapping_add(moves.y_offset()) < min_y {\n                    continue;", "|| cur_y.wrapping_add(moves.y_offset()) < min_y {", "height: out of range move not skipped"),
    (ASTAR, "num_movements_considered = num_movements_considered.wrapping_add(1);", "num_movements_considered = num_movements_considered.wrapping_add(2);", "movement count steps by two"),
    (ASTAR, "moves.apply(ctx, cur_x, cur_y, cur_z, &mut res);", "moves.apply(ctx, cur_x, cur_y.wrapping_add(1), cur_z, &mut res);", "apply from one block up"),
    (ASTAR, "if action_cost >= COST_INF {\n                    continue;", "if false {\n                    continue;", "cost inf: move kept"),
    (ASTAR, "if action_cost <= 0.0 || action_cost.is_nan() {", "if action_cost < 0.0 || action_cost.is_nan() {", "cost check: zero allowed"),
    (ASTAR, "if action_cost <= 0.0 || action_cost.is_nan() {", "if action_cost <= 0.0 {", "cost check: NaN allowed"),
    (ASTAR, "if action_cost <= 0.0 || action_cost.is_nan() {", "if false {", "cost check: none"),
    (ASTAR, "if !moves.dynamic_xz() && (res.x != new_x || res.z != new_z) {", "if !moves.dynamic_xz() && (res.x != new_x && res.z != new_z) {", "destination check: both must differ"),
    (ASTAR, "if !moves.dynamic_xz() && (res.x != new_x || res.z != new_z) {", "if false {", "destination check: none"),
    (ASTAR, "if !moves.dynamic_y() && res.y != cur_y.wrapping_add(moves.y_offset()) {", "if false {", "height check: none"),
    (ASTAR, "if !moves.dynamic_y() && res.y != cur_y.wrapping_add(moves.y_offset()) {", "if res.y != cur_y.wrapping_add(moves.y_offset()) {", "height check: on dynamic moves too"),
    (ASTAR, "let hash_code = long_hash(res.x, res.y, res.z);", "let hash_code = long_hash(res.z, res.y, res.x);", "favoring key: x and z swapped"),
    (ASTAR, "let hash_code = long_hash(res.x, res.y, res.z);", "let hash_code = long_hash(cur_x, cur_y, cur_z);", "favoring key: the source"),
    (ASTAR, "let neighbor = self.map.get_node_at_position(res.x, res.y, res.z, &self.goal, &gs)?;", "let neighbor = self.map.get_node_at_position(new_x, res.y, new_z, &self.goal, &gs)?;", "neighbor at the nominal x and z"),
    (ASTAR, "let is_favoring = !self.favoring.is_empty();", "let is_favoring = self.favoring.is_empty();", "favoring: switch inverted"),
    (ASTAR, "let is_favoring = !self.favoring.is_empty();", "let is_favoring = true;", "favoring: always on"),
    (ASTAR, "let is_favoring = !self.favoring.is_empty();", "let is_favoring = false;", "favoring: never on"),
    (ASTAR, "if n.favor == 0.0 {", "if n.favor == 1.0 {", "favor cache: looked up only at 1"),
    (ASTAR, "if n.favor == 0.0 {", "if true {", "favor cache: always looked up"),
    (ASTAR, "n.favor = self.favoring.calculate(hash_code);", "n.favor = 1.0;", "favor: always 1"),
    (ASTAR, "action_cost *= n.favor;", "action_cost += n.favor;", "favor: added, not multiplied"),
    (ASTAR, "let improvement = self.map.nodes[neighbor as usize].cost - tentative_cost;", "let improvement = tentative_cost - self.map.nodes[neighbor as usize].cost;", "improvement: sign"),
    (ASTAR, "let tentative_cost = self.map.nodes[current as usize].cost + action_cost;", "let tentative_cost = self.map.nodes[neighbor as usize].cost + action_cost;", "tentative cost from the neighbor"),
    (ASTAR, "if improvement > minimum_improvement {", "if improvement > 0.0 {", "improvement: any gain"),
    (ASTAR, "if improvement > minimum_improvement {", "if improvement > MIN_IMPROVEMENT {", "improvement: setting ignored"),
    (ASTAR, "let minimum_improvement = if settings.minimum_improvement_repropagation { MIN_IMPROVEMENT } else { 0.0 };", "let minimum_improvement = if settings.minimum_improvement_repropagation { 0.0 } else { MIN_IMPROVEMENT };", "minimum improvement: setting inverted"),
    (ASTAR, "let minimum_improvement = if settings.minimum_improvement_repropagation { MIN_IMPROVEMENT } else { 0.0 };", "let minimum_improvement = MIN_IMPROVEMENT;", "minimum improvement: always on"),
    (ASTAR, "let minimum_improvement = if settings.minimum_improvement_repropagation { MIN_IMPROVEMENT } else { 0.0 };", "let minimum_improvement = 0.0;", "minimum improvement: always off"),
    (ASTAR, "n.previous = Some(current);", "", "previous node not set"),
    (ASTAR, "n.cost = tentative_cost;\n                    n.combined_cost", "n.combined_cost", "cost not stored"),
    (ASTAR, "let was_open = n.is_open();", "let was_open = !n.is_open();", "open set: update and insert swapped"),
    (ASTAR, "if best_heuristic_so_far[i] - heuristic > minimum_improvement {", "if heuristic - best_heuristic_so_far[i] > minimum_improvement {", "best node: comparison direction"),
    (ASTAR, "if best_heuristic_so_far[i] - heuristic > minimum_improvement {", "if best_heuristic_so_far[i] - heuristic >= minimum_improvement {", "best node: at least the improvement"),
    (ASTAR, "if best_heuristic_so_far[i] - heuristic > minimum_improvement {", "if best_heuristic_so_far[i] - heuristic > MIN_IMPROVEMENT {", "best node: setting ignored"),
    (ASTAR, "let heuristic = n.estimated_cost_to_goal + n.cost * inverse_coefficients[i];", "let heuristic = n.estimated_cost_to_goal + n.cost / inverse_coefficients[i];", "best node: cost divided"),
    (ASTAR, "let heuristic = n.estimated_cost_to_goal + n.cost * inverse_coefficients[i];", "let heuristic = n.estimated_cost_to_goal + n.cost * inverse_coefficients[COEFFICIENTS.len() - 1 - i];", "best node: coefficients reversed"),
    (ASTAR, "let heuristic = n.estimated_cost_to_goal + n.cost * inverse_coefficients[i];", "let heuristic = n.estimated_cost_to_goal + n.combined_cost * inverse_coefficients[i];", "best node: combined cost used"),
    (ASTAR, "best_heuristic_so_far[i] = heuristic;", "", "best heuristic not kept"),
    (ASTAR, "self.best_so_far[i] = Some(neighbor);", "self.best_so_far[i] = Some(current);", "best node: the current node kept"),
    (ASTAR, "if (num_nodes as u32) >= max {", "if (num_nodes as u32) >= max.wrapping_add(1) {", "node cap: after one more node"),
    (ASTAR, "stopped_early = true;\n                    break;\n                }\n            }\n        }", "stopped_early = true;\n                }\n            }\n        }", "node cap: loop goes on"),
    (ASTAR, "let start_node = self.map.get_node_at_position(self.start_x, self.start_y, self.start_z, &self.goal, &gs)?;", "let start_node = self.map.get_node_at_position(self.real_start.x, self.real_start.y, self.real_start.z, &self.goal, &gs)?;", "start node at the real start"),
    (ASTAR, "n.cost = 0.0;\n            n.combined_cost", "n.cost = 1.0;\n            n.combined_cost", "start node: cost 1"),
    (ASTAR, "open_set.insert(&mut self.map.nodes, start_node);", "", "start node not in the open set"),
    (ASTAR, "self.best_so_far = [Some(start_node); 7];", "self.best_so_far = [None; 7];", "best nodes start empty"),
    (ASTAR, "let inverse_coefficients: [f64; 7] = COEFFICIENTS.map(|c| 1.0 / c);", "let inverse_coefficients: [f64; 7] = COEFFICIENTS.map(|c| c);", "inverse coefficients not inverted"),
    (ASTAR, "[1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 10.0]", "[1.5, 2.0, 2.5, 3.0, 4.0, 6.0, 10.0]", "coefficients: sixth one 6"),
    (ASTAR, "[1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 10.0]", "[1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 10.5]", "coefficients: last one a little off"),
    (ASTAR, "[1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 10.0]", "[1.25, 2.0, 2.5, 3.0, 4.0, 5.0, 10.0]", "coefficients: first one 1.25"),
    (ASTAR, "const MIN_DIST_PATH: f64 = 5.0;", "const MIN_DIST_PATH: f64 = 6.0;", "min dist 6"),
    (ASTAR, "const MIN_IMPROVEMENT: f64 = 0.01;", "const MIN_IMPROVEMENT: f64 = 0.005;", "min improvement 0.005"),
    (ASTAR, "const MIN_IMPROVEMENT: f64 = 0.01;", "const MIN_IMPROVEMENT: f64 = 0.1;", "min improvement 0.1"),
    (ASTAR, "let x_diff = n.x.wrapping_sub(self.start_x);", "let x_diff = 0i32;", "distance from start without x"),
    (ASTAR, "let z_diff = n.z.wrapping_sub(self.start_z);", "let z_diff = 0i32;", "distance from start without z"),
    (ASTAR, "let y_diff = n.y.wrapping_sub(self.start_y);", "let y_diff = n.y.wrapping_sub(self.start_z);", "distance from start with y from z"),
    (ASTAR, "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) as f64", "(x_diff.abs() + y_diff.abs() + z_diff.abs()) as f64", "distance from start in manhattan"),
    (ASTAR, "if dist > MIN_DIST_PATH * MIN_DIST_PATH {\n                // square the comparison", "if dist > MIN_DIST_PATH {\n                // square the comparison", "partial path: distance not squared"),
    (ASTAR, "if dist > MIN_DIST_PATH * MIN_DIST_PATH {\n                // square the comparison", "if true {\n                // square the comparison", "partial path: any distance"),
    (ASTAR, "self.best_so_far_path(0)", "self.best_so_far_path(1)", "best path so far: node count 1"),
    (ASTAR, "Path::from_nodes(self.real_start, start, recent, 0, &self.goal", "Path::from_nodes(self.real_start, start, recent, 1, &self.goal", "path to the recent node: node count 1"),
    (ASTAR, "Path::from_nodes(self.real_start, start, recent, 0, &self.goal", "Path::from_nodes(self.real_start, start, start, 0, &self.goal", "path to the recent node: the start"),
    (ASTAR, "Some(p) => Some(p.post_process(ctx)?),", "Some(p) => Some(p),", "calculate: path not post processed"),
    (ASTAR, "let Some(path) = path else {\n            return Ok(PathCalculationResult { kind: ResultType::Failure", "let Some(path) = path else {\n            return Ok(PathCalculationResult { kind: ResultType::Cancellation", "calculate: failure as cancellation"),
    (ASTAR, "let path = path.cutoff_at_loaded_chunks(ctx.world, s)?;\n        let path = path.static_cutoff(Some(&self.goal), &gs, s)?;", "let path = path.static_cutoff(Some(&self.goal), &gs, s)?;\n        let path = path.cutoff_at_loaded_chunks(ctx.world, s)?;", "calculate: cutoffs in the other order"),
    (ASTAR, "let path = path.cutoff_at_loaded_chunks(ctx.world, s)?;\n        let path = path.static_cutoff(Some(&self.goal), &gs, s)?;", "let path = path.static_cutoff(Some(&self.goal), &gs, s)?;", "calculate: no loaded cutoff"),
    (ASTAR, "let path = path.cutoff_at_loaded_chunks(ctx.world, s)?;\n        let path = path.static_cutoff(Some(&self.goal), &gs, s)?;", "let path = path.cutoff_at_loaded_chunks(ctx.world, s)?;", "calculate: no static cutoff"),
    (ASTAR, "if is_cancelled(limits) {\n            return Ok(None);", "if false {\n            return Ok(None);", "search ended: cancel flag not read"),
    (ASTAR, "if is_cancelled(limits) {\n            return Ok(PathCalculationResult { kind: ResultType::Cancellation", "if false {\n            return Ok(PathCalculationResult { kind: ResultType::Cancellation", "calculate: cancel flag not read"),
    (ASTAR, "if self.finished {\n            return PathCalculationResult", "if false {\n            return PathCalculationResult", "calculate: a second run allowed"),
    (ASTAR, "        self.finished = true;\n", "", "calculate: finished flag not set"),
    # --- path
    (PATH, "temp_costs.push(n.cost);", "temp_costs.push(n.combined_cost);", "from nodes: combined costs"),
    (PATH, "temp_costs.push(n.cost);", "", "from nodes: costs not collected"),
    (PATH, "current = n.previous;", "current = None;", "from nodes: previous not followed"),
    (PATH, "if temp_path.len() > nodes.len() {", "if temp_path.len() >= nodes.len() {", "from nodes: loop guard one early"),
    (PATH, "if real_start != start_node_pos && s.same_position(e) {", "if real_start == start_node_pos && s.same_position(e) {", "fake node: only without a moved start"),
    (PATH, "if real_start != start_node_pos && s.same_position(e) {", "if real_start != start_node_pos || s.same_position(e) {", "fake node: moved start or same position"),
    (PATH, "if real_start != start_node_pos && s.same_position(e) {", "if s.same_position(e) {", "fake node: start move not tested"),
    (PATH, "src = real_start;\n", "src = start_node_pos;\n", "fake node: source is the node"),
    (PATH, "temp_costs.push(0.0);\n            temp_path.push(real_start);", "temp_costs.push(1.0);\n            temp_path.push(real_start);", "fake node: cost 1"),
    (PATH, "temp_costs.push(0.0);\n            temp_path.push(real_start);", "temp_costs.push(0.0);\n            temp_path.push(start_node_pos);", "fake node: position is the node"),
    (PATH, "PathNode::new(real_start.x, real_start.y, real_start.z, goal, gs)?;", "", "fake node: heuristic not checked"),
    (PATH, "src = start_node_pos;\n        }", "src = real_start;\n        }", "no fake node: source is the real start"),
    (PATH, "temp_path.reverse();\n        temp_costs.reverse();", "temp_path.reverse();", "from nodes: costs not reversed"),
    (PATH, "temp_path.reverse();\n        temp_costs.reverse();", "temp_costs.reverse();", "from nodes: positions not reversed"),
    (PATH, "origin: Origin::Nodes, verified: false, node_costs: temp_costs }", "origin: Origin::Nodes, verified: false, node_costs: Vec::new() }", "from nodes: costs dropped"),
    (PATH, "self.positions[self.positions.len() - 1]", "self.positions[0]", "destination is the first position"),
    (PATH, "if self.positions.is_empty() || !self.movements.is_empty() {", "if self.positions.is_empty() {", "assemble: second run allowed"),
    (PATH, "for i in 0..self.positions.len() - 1 {\n            let cost", "for i in 0..self.positions.len() - 2 {\n            let cost", "assemble: last step not made"),
    (PATH, "let cost = self.node_costs[i + 1] - self.node_costs[i];", "let cost = self.node_costs[i] - self.node_costs[i + 1];", "movement cost: difference reversed"),
    (PATH, "let cost = self.node_costs[i + 1] - self.node_costs[i];", "let cost = self.node_costs[i + 1] - self.node_costs[0];", "movement cost: from the start"),
    (PATH, "None => return Ok(true),", "None => return Ok(false),", "assemble: failure not reported"),
    (PATH, "None => return Ok(true),", "None => continue,", "assemble: unmatched step skipped"),
    (PATH, "if mv.dest == dest {", "if mv.dest.x == dest.x && mv.dest.z == dest.z {", "runBackwards: match ignores y"),
    (PATH, "if mv.dest == dest {", "if mv.dest.y == dest.y && mv.dest.z == dest.z {", "runBackwards: match ignores x"),
    (PATH, "if mv.dest == dest {", "if mv.dest.x == dest.x && mv.dest.y == dest.y {", "runBackwards: match ignores z"),
    (PATH, "mv.override_cost(java_min(mv.calculate_cost(ctx), cost));", "mv.override_cost(crate::costs::java_max(mv.calculate_cost(ctx), cost));", "movement cost: max of the two"),
    (PATH, "mv.override_cost(java_min(mv.calculate_cost(ctx), cost));", "let _ = java_min(mv.calculate_cost(ctx), cost);", "movement cost: not stored"),
    (PATH, "let mut mv = moves.apply0(ctx, src);", "let mut mv = moves.apply0(ctx, dest);", "runBackwards: moves applied from the end"),
    (PATH, "if self.origin != Origin::Nodes || self.verified {", "if self.origin != Origin::Nodes {", "post process: a second run allowed"),
    (PATH, "self.verified = true;\n        let failed", "let failed", "post process: not verified"),
    (PATH, "if failed {\n            let res", "if !failed {\n            let res", "post process: failure test inverted"),
    (PATH, "let res = Path::cutoff(&self, 0, self.movements.len() as i32)?;", "let res = Path::cutoff(&self, 1, self.movements.len() as i32)?;", "post process: cut from 1"),
    (PATH, "let res = Path::cutoff(&self, 0, self.movements.len() as i32)?;", "let res = Path::cutoff(&self, 0, self.movements.len() as i32 + 1)?;", "post process: cut one too late"),
    (PATH, "if res.movements.len() != self.movements.len() {", "if res.movements.len() == self.movements.len() {", "post process: size check inverted"),
    (PATH, "if res.movements.len() != self.movements.len() {", "if false {", "post process: size check dropped"),
    (PATH, "self.sanity_check()?;\n        Ok(self)", "Ok(self)", "post process: no sanity check"),
    (PATH, "for m in self.movements.iter_mut() {\n            m.check_loaded_chunk(ctx);\n        }", "", "post process: loaded flags not set"),
    (PATH, "let pr = sub_list(prev.positions.len(), first, last.wrapping_add(1))?;", "let pr = sub_list(prev.positions.len(), first, last)?;", "cutoff: positions one short"),
    (PATH, "let pr = sub_list(prev.positions.len(), first, last.wrapping_add(1))?;", "let pr = sub_list(prev.positions.len(), first, last.wrapping_add(2))?;", "cutoff: positions one long"),
    (PATH, "let mr = sub_list(movements.len(), first, last)?;", "let mr = sub_list(movements.len(), first, last.wrapping_sub(1))?;", "cutoff: movements one short"),
    (PATH, "origin: Origin::Cutoff, verified: true,", "origin: Origin::Cutoff, verified: false,", "cutoff: not verified"),
    (PATH, "path.src = *path.positions.first().ok_or(SearchError::EmptyPath)?;", "path.src = prev.src;", "cutoff: source of the whole path"),
    (PATH, "num_nodes: prev.num_nodes, goal: prev.goal.clone(), origin: Origin::Cutoff", "num_nodes: 0, goal: prev.goal.clone(), origin: Origin::Cutoff", "cutoff: node count dropped"),
    (PATH, "if from < 0 || to < from || (to as i64) > len as i64 {", "if from <= 0 || to < from || (to as i64) > len as i64 {", "sub list: from 0 refused"),
    (PATH, "if from < 0 || to < from || (to as i64) > len as i64 {", "if from < 0 || to <= from || (to as i64) > len as i64 {", "sub list: empty range refused"),
    (PATH, "if from < 0 || to < from || (to as i64) > len as i64 {", "if from < 0 || to < from || (to as i64) >= len as i64 {", "sub list: full range refused"),
    (PATH, "if from < 0 || to < from || (to as i64) > len as i64 {", "if from < 0 || to < from {", "sub list: range too long allowed"),
    (PATH, "if !s.cutoff_at_load_boundary {\n            return Ok(self);", "if s.cutoff_at_load_boundary {\n            return Ok(self);", "load cutoff: setting inverted"),
    (PATH, "if !s.cutoff_at_load_boundary {\n            return Ok(self);", "if false {\n            return Ok(self);", "load cutoff: setting ignored"),
    (PATH, "for (i, pos) in self.positions.iter().enumerate() {\n            if !world.contains_loaded_chunk(pos.x, pos.z) {", "for (i, pos) in self.positions.iter().enumerate().skip(1) {\n            if !world.contains_loaded_chunk(pos.x, pos.z) {", "load cutoff: first position not tested"),
    (PATH, "if !world.contains_loaded_chunk(pos.x, pos.z) {\n                return Path::cutoff(&self, 0, i as i32);", "if world.contains_loaded_chunk(pos.x, pos.z) {\n                return Path::cutoff(&self, 0, i as i32);", "load cutoff: test inverted"),
    (PATH, "if !world.contains_loaded_chunk(pos.x, pos.z) {\n                return Path::cutoff(&self, 0, i as i32);", "if !world.contains_loaded_chunk(pos.z, pos.x) {\n                return Path::cutoff(&self, 0, i as i32);", "load cutoff: x and z swapped"),
    (PATH, "return Path::cutoff(&self, 0, i as i32);", "return Path::cutoff(&self, 0, (i as i32).wrapping_add(1));", "load cutoff: one position late"),
    (PATH, "if self.length() < min {\n            return Ok(self);", "if self.length() <= min {\n            return Ok(self);", "static cutoff: minimum length itself cut"),
    (PATH, "if self.length() < min {\n            return Ok(self);", "if false {\n            return Ok(self);", "static cutoff: minimum length ignored"),
    (PATH, "None => return Ok(self),\n            Some(g)", "None => {}\n            Some(g)", "static cutoff: no destination goal cuts"),
    (PATH, "Some(g) if g.is_in_goal(d.x, d.y, d.z, gs) => return Ok(self),", "Some(g) if !g.is_in_goal(d.x, d.y, d.z, gs) => return Ok(self),", "static cutoff: goal test inverted"),
    (PATH, "Some(g) if g.is_in_goal(d.x, d.y, d.z, gs) => return Ok(self),", "Some(g) if g.is_in_goal(d.x, d.y.wrapping_add(1), d.z, gs) => return Ok(self),", "static cutoff: goal test one up"),
    (PATH, "((self.length().wrapping_sub(min) as f64 * factor) as i32)", "((self.length().wrapping_sub(min) as f64 / factor) as i32)", "static cutoff: factor divides"),
    (PATH, "((self.length().wrapping_sub(min) as f64 * factor) as i32)", "((self.length().wrapping_sub(min) as f64 * factor).ceil() as i32)", "static cutoff: round up"),
    (PATH, "((self.length().wrapping_sub(min) as f64 * factor) as i32)", "((self.length() as f64 * factor) as i32)", "static cutoff: minimum not taken off"),
    (PATH, ".wrapping_add(min).wrapping_sub(1);\n        Path::cutoff", ".wrapping_add(min).wrapping_sub(2);\n        Path::cutoff", "static cutoff: one position fewer"),
    (PATH, ".wrapping_add(min).wrapping_sub(1);\n        Path::cutoff", ".wrapping_sub(1);\n        Path::cutoff", "static cutoff: minimum not added"),
    (PATH, "Path::cutoff(&self, 0, new_length)", "Path::cutoff(&self, 0, new_length.wrapping_add(1))", "static cutoff: cut one late"),
    (PATH, "Path::cutoff(&self, 0, new_length)", "Path::cutoff(&self, 1, new_length)", "static cutoff: cut from 1"),
    (PATH, "let factor = s.path_cutoff_factor;", "let factor = 0.5;", "static cutoff: factor fixed"),
    (PATH, "let min = s.path_cutoff_minimum_length;", "let min = s.path_cutoff_minimum_length.wrapping_add(1);", "static cutoff: minimum plus one"),
    (PATH, "if path.is_empty() {\n            return Err(SearchError::EmptyPath);\n        }\n        let movements", "if false {\n            return Err(SearchError::EmptyPath);\n        }\n        let movements", "sanity: empty path allowed"),
    (PATH, "if self.src != path[0] {", "if self.src == path[0] {", "sanity: source test inverted"),
    (PATH, "if self.src != path[0] {", "if false {", "sanity: source not tested"),
    (PATH, "if self.dest() != path[path.len() - 1] {", "if false {", "sanity: destination not tested"),
    (PATH, "if path.len() != movements.len() + 1 {", "if path.len() != movements.len() {", "sanity: size test off by one"),
    (PATH, "if path.len() != movements.len() + 1 {", "if false {", "sanity: size not tested"),
    (PATH, "if src != movement.src {", "if false {", "sanity: movement source not tested"),
    (PATH, "if dest != movement.dest {", "if false {", "sanity: movement destination not tested"),
    (PATH, "if seen_so_far.contains(&src) {", "if false {", "sanity: loop not tested"),
    (PATH, "seen_so_far.insert(src);", "", "sanity: positions not remembered"),
    (PATH, "_ => return Ok(None),\n        };\n        if first.dest()", "_ => return Err(SearchError::EmptyPath),\n        };\n        if first.dest()", "splice: missing path is an error"),
    (PATH, "if first.dest() != second.src() {\n            return Ok(None);", "if first.dest() != second.dest() {\n            return Ok(None);", "splice: meets at the destination"),
    (PATH, "if second_pos.contains(&first.positions[i as usize]) {\n                first_position_in_second = i;\n                break;", "if second_pos.contains(&first.positions[i as usize]) {\n                first_position_in_second = i;", "splice: last overlap, not the first"),
    (PATH, "if first_position_in_second != -1 {", "if first_position_in_second != 0 {", "splice: overlap test"),
    (PATH, "first_position_in_second = first.length() - 1;", "first_position_in_second = first.length() - 2;", "splice: no overlap joins one early"),
    (PATH, ".map(|i| i as i32).unwrap_or(-1);", ".map(|i| i as i32).unwrap_or(0);", "splice: missing position is 0"),
    (PATH, "if !allow_overlap_cutoff && position_in_second != 0 {", "if allow_overlap_cutoff && position_in_second != 0 {", "splice: overlap error inverted"),
    (PATH, "if !allow_overlap_cutoff && position_in_second != 0 {", "if false {", "splice: overlap error dropped"),
    (PATH, "positions.extend_from_slice(&first.positions[sub_list(first.positions.len(), 0, first_position_in_second + 1)?]);", "positions.extend_from_slice(&first.positions[sub_list(first.positions.len(), 0, first_position_in_second)?]);", "splice: first positions one short"),
    (PATH, "movements.extend_from_slice(&fm[sub_list(fm.len(), 0, first_position_in_second)?]);", "movements.extend_from_slice(&fm[sub_list(fm.len(), 0, first_position_in_second + 1)?]);", "splice: first movements one long"),
    (PATH, "movements.extend_from_slice(&fm[sub_list(fm.len(), 0, first_position_in_second)?]);", "movements.extend_from_slice(&fm[sub_list(fm.len(), 0, first_position_in_second - 1)?]);", "splice: first movements one short"),
    (PATH, "&second.positions[sub_list(second.positions.len(), position_in_second.wrapping_add(1), second.length())?]", "&second.positions[sub_list(second.positions.len(), position_in_second, second.length())?]", "splice: second positions keep the join"),
    (PATH, "movements.extend_from_slice(&sm[sub_list(sm.len(), position_in_second, second.length() - 1)?]);", "movements.extend_from_slice(&sm[sub_list(sm.len(), position_in_second, second.length() - 2)?]);", "splice: second movements one short"),
    (PATH, "let src = *positions.first().ok_or(SearchError::EmptyPath)?;\n        let path = Path { positions, movements, src, num_nodes: first.num_nodes.wrapping_add(second.num_nodes), goal: first.goal.clone()", "let src = *positions.first().ok_or(SearchError::EmptyPath)?;\n        let path = Path { positions, movements, src, num_nodes: first.num_nodes.wrapping_add(second.num_nodes), goal: second.goal.clone()", "splice: goal of the second path"),
    (PATH, "origin: Origin::Spliced, verified: true,", "origin: Origin::Spliced, verified: false,", "splice: not verified"),
    (PATH, "let target = first.positions[first_position_in_second as usize];", "let target = first.positions[0];", "splice: target is the first position"),
    # --- favoring and avoidance
    (FAV, "let x_diff = x.wrapping_sub(self.center_x);", "let x_diff = 0i32;", "avoidance coefficient: x ignored"),
    (FAV, "let y_diff = y.wrapping_sub(self.center_y);", "let y_diff = 0i32;", "avoidance coefficient: y ignored"),
    (FAV, ".wrapping_add(z_diff.wrapping_mul(z_diff)) <= self.radius_sq {", ".wrapping_add(z_diff.wrapping_mul(z_diff)) < self.radius_sq {", "avoidance coefficient: edge outside"),
    (FAV, "self.coefficient\n        } else {\n            1.0\n        }", "self.coefficient\n        } else {\n            0.0\n        }", "avoidance coefficient: outside is 0"),
    (FAV, "radius_sq: radius.wrapping_mul(radius) }", "radius_sq: radius }", "avoidance: squared radius is the radius"),
    (FAV, "res.push(Avoidance::new(p.x, p.y, p.z, s.mob_spawner_avoidance_coefficient, s.mob_spawner_avoidance_radius));", "res.push(Avoidance::new(p.x, p.y, p.z, s.mob_spawner_avoidance_coefficient, s.mob_avoidance_radius));", "avoidance: spawner radius of the mobs"),
    (FAV, "res.push(Avoidance::new(p.x, p.y, p.z, s.mob_spawner_avoidance_coefficient, s.mob_spawner_avoidance_radius));", "res.push(Avoidance::new(p.x, p.y, p.z, s.mob_avoidance_coefficient, s.mob_spawner_avoidance_radius));", "avoidance: spawner coefficient of the mobs"),
    (FAV, "res.push(Avoidance::new(p.x, p.y, p.z, s.mob_avoidance_coefficient, s.mob_avoidance_radius));", "res.push(Avoidance::new(p.x, p.y, p.z, s.mob_avoidance_coefficient, s.mob_spawner_avoidance_radius));", "avoidance: mob radius of the spawners"),
    (FAV, "res.push(Avoidance::new(p.x, p.y, p.z, s.mob_avoidance_coefficient, s.mob_avoidance_radius));", "res.push(Avoidance::new(p.x, p.y, p.z, s.mob_spawner_avoidance_coefficient, s.mob_avoidance_radius));", "avoidance: mob coefficient of the spawners"),
    (FAV, "if s.mob_avoidance_coefficient != 1.0 {", "if true {", "avoidance: mobs with coefficient 1"),
    (FAV, "if s.mob_avoidance_coefficient != 1.0 {", "if false {", "avoidance: mobs never"),
    (FAV, "if s.mob_spawner_avoidance_coefficient != 1.0 {", "if false {", "avoidance: spawners never"),
    (FAV, "for p in spawners {", "for p in mobs {", "avoidance: spawners from the mob list"),
    (FAV, "for p in mobs {", "for p in spawners {", "avoidance: mobs from the spawner list"),
    (FAV, "let mut x = radius.wrapping_neg();", "let mut x = radius.wrapping_neg().wrapping_add(1);", "sphere: x starts inside"),
    (FAV, "let mut y = radius.wrapping_neg();", "let mut y = radius.wrapping_neg().wrapping_add(1);", "sphere: y starts inside"),
    (FAV, "let mut z = radius.wrapping_neg();", "let mut z = radius.wrapping_neg().wrapping_add(1);", "sphere: z starts inside"),
    (FAV, "while x <= radius {", "while x < radius {", "sphere: x stops early"),
    (FAV, "while y <= radius {", "while y < radius {", "sphere: y stops early"),
    (FAV, "while z <= radius {", "while z < radius {", "sphere: z stops early"),
    (FAV, "x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y)).wrapping_add(z.wrapping_mul(z)) <= radius.wrapping_mul(radius)", "x.wrapping_mul(x).wrapping_add(z.wrapping_mul(z)) <= radius.wrapping_mul(radius)", "sphere: cylinder, y ignored"),
    (FAV, "x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y)).wrapping_add(z.wrapping_mul(z)) <= radius.wrapping_mul(radius)", "x.abs() + y.abs() + z.abs() <= radius", "sphere: diamond"),
    (FAV, "let hash = long_hash(self.center_x.wrapping_add(x), self.center_y.wrapping_add(y), self.center_z.wrapping_add(z));", "let hash = long_hash(self.center_x, self.center_y.wrapping_add(y), self.center_z.wrapping_add(z));", "sphere: x offset ignored"),
    (FAV, "let hash = long_hash(self.center_x.wrapping_add(x), self.center_y.wrapping_add(y), self.center_z.wrapping_add(z));", "let hash = long_hash(self.center_x.wrapping_add(x), self.center_y, self.center_z.wrapping_add(z));", "sphere: y offset ignored"),
    (FAV, "let hash = long_hash(self.center_x.wrapping_add(x), self.center_y.wrapping_add(y), self.center_z.wrapping_add(z));", "let hash = long_hash(self.center_x.wrapping_add(x), self.center_y.wrapping_add(y), self.center_z);", "sphere: z offset ignored"),
    (FAV, "let hash = long_hash(self.center_x.wrapping_add(x), self.center_y.wrapping_add(y), self.center_z.wrapping_add(z));", "let hash = long_hash(self.center_x.wrapping_add(x), self.center_y.wrapping_add(z), self.center_z.wrapping_add(y));", "sphere: y and z swapped"),
    (FAV, "let old = map.get(&hash).copied().unwrap_or(1.0);", "let old = map.get(&hash).copied().unwrap_or(0.0);", "sphere: default 0"),
    (FAV, "map.insert(hash, old * self.coefficient);", "map.insert(hash, self.coefficient);", "sphere: overlap overwrites"),
    (FAV, "if let Some(prev) = previous {\n                for pos in prev.positions() {", "if let Some(prev) = previous {\n                for pos in prev.positions().iter().skip(1) {", "favoring: first position of the previous path left out"),
    (FAV, "if let Some(prev) = previous {\n                for pos in prev.positions() {", "if let Some(prev) = previous {\n                for pos in prev.positions().iter().rev().skip(1) {", "favoring: last position of the previous path left out"),
    (FAV, "favorings.insert(pos.long_hash(), coeff);", "favorings.insert(pos.long_hash(), 1.0 / coeff);", "favoring: coefficient inverted"),
    (FAV, "if coeff != 1.0 {", "if coeff == 1.0 {", "favoring: coefficient test inverted"),
    (FAV, "let coeff = s.backtrack_cost_favoring_coefficient;", "let coeff = s.mob_avoidance_coefficient;", "favoring: wrong coefficient setting"),
    (FAV, "avoid.apply_spherical(&mut f.favorings);", "", "favoring: avoidances not applied"),
    (FAV, "let mut f = Favoring::from_previous(previous, s);", "let mut f = Favoring::from_previous(None, s);", "favoring: previous path left out of the avoidance map"),
    (FAV, "self.favorings.get(&hash).copied().unwrap_or(1.0)", "self.favorings.get(&hash).copied().unwrap_or(0.0)", "favoring: default 0"),
    (FAV, "self.favorings.get(&hash).copied().unwrap_or(1.0)", "self.favorings.get(&hash).copied().unwrap_or(2.0)", "favoring: default 2"),
    (FAV, "self.favorings.get(&hash).copied().unwrap_or(1.0)", "self.favorings.get(&hash).copied().unwrap_or(1.0 + 1e-9)", "favoring: default a little over 1"),
    (FAV, "self.favorings.is_empty()", "false", "favoring: never empty"),
    (FAV, "self.favorings.is_empty()", "true", "favoring: always empty"),
    # --- plan
    (PLAN, "Goal::Block { x, y, z } | Goal::TwoBlocks { x, y, z }", "Goal::TwoBlocks { x, y, z }", "goal render pos: no GoalBlock"),
    (PLAN, "| Goal::TwoBlocks { x, y, z } |", "|", "goal render pos: no GoalTwoBlocks"),
    (PLAN, "| Goal::GetToBlock { x, y, z } |", "|", "goal render pos: no GoalGetToBlock"),
    (PLAN, "Some(Pos::new(*x, *y, *z))", "Some(Pos::new(*x, 0, *z))", "goal render pos: y 0"),
    (PLAN, "Some(Pos::new(*x, *y, *z))", "Some(Pos::new(*z, *y, *x))", "goal render pos: x and z swapped"),
    (PLAN, "if s.simplify_unloaded_y_coord {", "if !s.simplify_unloaded_y_coord {", "simplify goal: setting inverted"),
    (PLAN, "if s.simplify_unloaded_y_coord {", "if true {", "simplify goal: setting ignored"),
    (PLAN, "if !world.contains_loaded_chunk(pos.x, pos.z) {\n                return Goal::Xz", "if world.contains_loaded_chunk(pos.x, pos.z) {\n                return Goal::Xz", "simplify goal: loaded test inverted"),
    (PLAN, "if !world.contains_loaded_chunk(pos.x, pos.z) {\n                return Goal::Xz", "if !world.contains_loaded_chunk(pos.z, pos.x) {\n                return Goal::Xz", "simplify goal: x and z swapped in the test"),
    (PLAN, "return Goal::Xz { x: pos.x, z: pos.z };", "return Goal::Xz { x: pos.z, z: pos.x };", "simplify goal: x and z swapped in the goal"),
    (PLAN, "feet.y == start.y && sub.x.wrapping_abs() <= 1 && sub.z.wrapping_abs() <= 1", "feet.y == start.y && sub.x.wrapping_abs() <= 1 && sub.z.wrapping_abs() < 1", "real start: z distance under 1"),
    (PLAN, "feet.y == start.y && sub.x.wrapping_abs() <= 1 && sub.z.wrapping_abs() <= 1", "feet.y == start.y && sub.x.wrapping_abs() <= 1", "real start: z not compared"),
    (PLAN, "feet.y == start.y && sub.x.wrapping_abs() <= 1 && sub.z.wrapping_abs() <= 1", "feet.y == start.y && sub.x.wrapping_abs() <= 2 && sub.z.wrapping_abs() <= 2", "real start: two blocks away"),
    (PLAN, "feet.y == start.y && sub.x.wrapping_abs() <= 1 && sub.z.wrapping_abs() <= 1", "feet.y == start.y && sub.x.wrapping_abs() <= 1 || sub.z.wrapping_abs() <= 1", "real start: x or z close"),
    (PLAN, "feet.y == start.y && sub.x", "feet.y != start.y && sub.x", "real start: height test inverted"),
    (PLAN, "return feet;\n    }\n    start", "return start;\n    }\n    start", "real start: the start itself"),
    (PLAN, "    start\n}", "    feet\n}", "real start: feet when far"),
    (PLAN, "let transformed = simplify_unloaded_y_coord(goal, ctx.world, s);", "let transformed = goal.clone();", "createPathfinder: goal not simplified"),
    (PLAN, "let real = real_start(start, feet);", "let real = start;", "createPathfinder: real start ignored"),
    (PLAN, "let real = real_start(start, feet);", "let real = feet;", "createPathfinder: feet as real start"),
    (PLAN, "AStarPathFinder::new(real, start.x, start.y, start.z, transformed, favoring, ctx)", "AStarPathFinder::new(real, feet.x, feet.y, feet.z, transformed, favoring, ctx)", "createPathfinder: search from the feet"),
    (PLAN, "AStarPathFinder::new(real, start.x, start.y, start.z, transformed, favoring, ctx)", "AStarPathFinder::new(start, start.x, start.y, start.z, transformed, favoring, ctx)", "createPathfinder: start as real start"),
    (PLAN, "AStarPathFinder::new(real, start.x, start.y, start.z, transformed, favoring, ctx)", "AStarPathFinder::new(real, start.x, start.y, start.z, goal.clone(), favoring, ctx)", "createPathfinder: original goal searched"),
    (PLAN, "create_pathfinder(previous.dest(), goal, Some(previous), ctx, feet, avoidances)", "create_pathfinder(previous.src(), goal, Some(previous), ctx, feet, avoidances)", "next segment: from the previous source"),
    (PLAN, "create_pathfinder(previous.dest(), goal, Some(previous), ctx, feet, avoidances)", "create_pathfinder(previous.dest(), goal, Some(previous), ctx, feet, &[])", "next segment: no avoidances"),
    (PLAN, "let favoring = Favoring::new(previous, s, avoidances);", "let favoring = Favoring::new(previous, s, &[]);", "createPathfinder: no avoidances"),
    (PLAN, "if path_is_being_followed {", "if !path_is_being_followed {", "timeouts: choice inverted"),
    (PLAN, "(s.plan_ahead_primary_timeout_ms, s.plan_ahead_failure_timeout_ms)", "(s.plan_ahead_failure_timeout_ms, s.plan_ahead_primary_timeout_ms)", "timeouts: plan ahead pair swapped"),
    (PLAN, "(s.primary_timeout_ms, s.failure_timeout_ms)", "(s.failure_timeout_ms, s.primary_timeout_ms)", "timeouts: pair swapped"),
    (PLAN, "(s.plan_ahead_primary_timeout_ms, s.plan_ahead_failure_timeout_ms)", "(s.primary_timeout_ms, s.plan_ahead_failure_timeout_ms)", "timeouts: plan ahead primary of the normal search"),
    # --- movement steps
    (STEP, "descend::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if result.y != d.y {", "descend::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if false {", "descend cost: height test dropped"),
    (STEP, "descend::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if result.y != d.y {", "descend::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if result.y == d.y {", "descend cost: height test inverted"),
    (STEP, "diagonal::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if result.y != d.y {", "diagonal::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if false {", "diagonal cost: height test dropped"),
    (STEP, "diagonal::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if result.y != d.y {", "diagonal::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if result.y == d.y {", "diagonal cost: height test inverted"),
    (STEP, "diagonal::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if result.y != d.y {", "diagonal::cost(ctx, s.x, s.y, s.z, d.x, d.z, &mut result);\n                if result.y != s.y {", "diagonal cost: height test against the source"),
    (STEP, "if res.x != d.x || res.y != d.y || res.z != d.z {", "if res.y != d.y || res.z != d.z {", "parkour cost: x not tested"),
    (STEP, "if res.x != d.x || res.y != d.y || res.z != d.z {", "if res.x != d.x || res.z != d.z {", "parkour cost: y not tested"),
    (STEP, "if res.x != d.x || res.y != d.y || res.z != d.z {", "if res.x != d.x || res.y != d.y {", "parkour cost: z not tested"),
    (STEP, "if res.x != d.x || res.y != d.y || res.z != d.z {", "if false {", "parkour cost: destination not tested"),
    (STEP, "if res.x != d.x || res.y != d.y || res.z != d.z {", "if res.x != d.x && res.y != d.y && res.z != d.z {", "parkour cost: destination needs three differences"),
    (STEP, "MovementKind::Downward => downward::cost(ctx, s.x, s.y, s.z),", "MovementKind::Downward => pillar::cost(ctx, s.x, s.y, s.z),", "downward cost: the pillar rule"),
    (STEP, "MovementKind::Pillar => pillar::cost(ctx, s.x, s.y, s.z),", "MovementKind::Pillar => downward::cost(ctx, s.x, s.y, s.z),", "pillar cost: the downward rule"),
    (STEP, "MovementKind::Traverse => traverse::cost(ctx, s.x, s.y, s.z, d.x, d.z),", "MovementKind::Traverse => traverse::cost(ctx, s.x, s.y, s.z, d.z, d.x),", "traverse cost: destination swapped"),
    (STEP, "MovementKind::Ascend => ascend::cost(ctx, s.x, s.y, s.z, d.x, d.z),", "MovementKind::Ascend => traverse::cost(ctx, s.x, s.y, s.z, d.x, d.z),", "ascend cost: the traverse rule"),
    (STEP, "self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.dest.x, self.dest.z));", "self.calculated_while_loaded = Some(!ctx.world.contains_loaded_chunk(self.dest.x, self.dest.z));", "loaded flag: inverted"),
    (STEP, "self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.dest.x, self.dest.z));", "self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.dest.z, self.dest.x));", "loaded flag: x and z swapped"),
    (STEP, "self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.dest.x, self.dest.z));", "self.calculated_while_loaded = Some(ctx.world.is_loaded(self.dest.x, self.dest.z));", "loaded flag: the cached check"),
    (STEP, "self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.dest.x, self.dest.z));", "self.calculated_while_loaded = Some(true);", "loaded flag: always true"),
    (STEP, "self.calculated_while_loaded = Some(ctx.world.contains_loaded_chunk(self.dest.x, self.dest.z));", "self.calculated_while_loaded = None;", "loaded flag: not set"),
    (STEP, "let dist = res.x.wrapping_sub(src.x).wrapping_abs().wrapping_add(res.z.wrapping_sub(src.z).wrapping_abs());", "let dist = res.x.wrapping_sub(src.x).wrapping_abs();", "parkour step: distance in x only"),
    (STEP, "let dist = res.x.wrapping_sub(src.x).wrapping_abs().wrapping_add(res.z.wrapping_sub(src.z).wrapping_abs());", "let dist = res.x.wrapping_sub(src.x).wrapping_abs().wrapping_add(res.z.wrapping_sub(src.z));", "parkour step: distance with a signed z"),
    (STEP, ".above(if ascend { 1 } else { 0 });", ".above(0);", "parkour step: no height"),
    (STEP, ".above(if ascend { 1 } else { 0 });", ".above(if ascend { 0 } else { 1 });", "parkour step: height inverted"),
    (STEP, "let dest = src.relative(direction, dist)", "let dest = src.relative(direction, dist.wrapping_sub(1))", "parkour step: one block short"),
    (STEP, "let ascend = res.y > src.y;", "let ascend = res.y < src.y;", "parkour step: ascend inverted"),
    # --- apply0 and the move table
    (MOVES, "let kind = if res.y == src.y.wrapping_sub(1) { MovementKind::Descend } else { MovementKind::Fall };", "let kind = if res.y != src.y.wrapping_sub(1) { MovementKind::Descend } else { MovementKind::Fall };", "apply0: descend and fall swapped"),
    (MOVES, "let kind = if res.y == src.y.wrapping_sub(1) { MovementKind::Descend } else { MovementKind::Fall };", "let kind = MovementKind::Descend;", "apply0: always a descend"),
    (MOVES, "let kind = if res.y == src.y.wrapping_sub(1) { MovementKind::Descend } else { MovementKind::Fall };", "let kind = MovementKind::Fall;", "apply0: always a fall"),
    (MOVES, "let kind = if res.y == src.y.wrapping_sub(1) { MovementKind::Descend } else { MovementKind::Fall };", "let kind = if res.y <= src.y.wrapping_sub(1) { MovementKind::Descend } else { MovementKind::Fall };", "apply0: descend for any drop"),
    (MOVES, "Movement::new(kind, src, Pos::new(res.x, res.y, res.z))", "Movement::new(kind, src, Pos::new(res.x, res.y, src.z))", "apply0: descend keeps the z"),
    (MOVES, "Movement::new(kind, src, Pos::new(res.x, res.y, res.z))", "Movement::new(kind, src, Pos::new(src.x, res.y, res.z))", "apply0: descend keeps the x"),
    (MOVES, "Movement::new(kind, src, Pos::new(res.x, res.y, res.z))", "Movement::new(kind, src, Pos::new(res.x, src.y.wrapping_sub(1), res.z))", "apply0: descend height fixed"),
    (MOVES, "Moves::Downward => Movement::new(MovementKind::Downward, src, src.below(1)),", "Moves::Downward => Movement::new(MovementKind::Downward, src, src.above(1)),", "apply0: downward goes up"),
    (MOVES, "Moves::Pillar => Movement::new(MovementKind::Pillar, src, src.above(1)),", "Moves::Pillar => Movement::new(MovementKind::Pillar, src, src.below(1)),", "apply0: pillar goes down"),
    (MOVES, "MovementKind::Traverse, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y, src.z.wrapping_add(self.z_offset()))", "MovementKind::Traverse, src, Pos::new(src.x.wrapping_add(self.z_offset()), src.y, src.z.wrapping_add(self.x_offset()))", "apply0: traverse offsets swapped"),
    (MOVES, "MovementKind::Traverse, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y, src.z.wrapping_add(self.z_offset()))", "MovementKind::Traverse, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y.wrapping_add(1), src.z.wrapping_add(self.z_offset()))", "apply0: traverse goes up"),
    (MOVES, "MovementKind::Ascend, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y.wrapping_add(1), src.z.wrapping_add(self.z_offset()))", "MovementKind::Ascend, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y, src.z.wrapping_add(self.z_offset()))", "apply0: ascend stays level"),
    (MOVES, "MovementKind::Ascend, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y.wrapping_add(1), src.z.wrapping_add(self.z_offset()))", "MovementKind::Ascend, src, Pos::new(src.x.wrapping_add(self.x_offset()), src.y.wrapping_sub(1), src.z.wrapping_add(self.z_offset()))", "apply0: ascend goes down"),
    (MOVES, "Moves::DiagonalNorthwest => (Direction::North, Direction::West),", "Moves::DiagonalNorthwest => (Direction::North, Direction::East),", "apply0: northwest goes east"),
    (MOVES, "Moves::DiagonalSoutheast => (Direction::South, Direction::East),", "Moves::DiagonalSoutheast => (Direction::North, Direction::East),", "apply0: southeast goes north"),
    (MOVES, "_ => (Direction::South, Direction::West),", "_ => (Direction::South, Direction::East),", "apply0: southwest goes east"),
    (MOVES, "Movement::new(MovementKind::Diagonal, src, src.relative(dir1, 1).relative(dir2, 1).above(dy))", "Movement::new(MovementKind::Diagonal, src, src.relative(dir1, 1).relative(dir2, 1))", "apply0: diagonal without height"),
    (MOVES, "let dy = res.y.wrapping_sub(src.y);", "let dy = res.y;", "apply0: diagonal height absolute"),
    (MOVES, "Moves::ParkourNorth => Movement::parkour_from(ctx, src, Direction::North),", "Moves::ParkourNorth => Movement::parkour_from(ctx, src, Direction::South),", "apply0: parkour north goes south"),
    (MOVES, "Moves::ParkourSouth => Movement::parkour_from(ctx, src, Direction::South),", "Moves::ParkourSouth => Movement::parkour_from(ctx, src, Direction::North),", "apply0: parkour south goes north"),
    (MOVES, "Moves::ParkourEast => Movement::parkour_from(ctx, src, Direction::East),", "Moves::ParkourEast => Movement::parkour_from(ctx, src, Direction::West),", "apply0: parkour east goes west"),
    (MOVES, "Moves::ParkourWest => Movement::parkour_from(ctx, src, Direction::West),", "Moves::ParkourWest => Movement::parkour_from(ctx, src, Direction::East),", "apply0: parkour west goes east"),
    (MOVES, "Moves::ParkourEast => 4,", "Moves::ParkourEast => 3,", "moves: parkour east offset 3"),
    (MOVES, "Moves::ParkourWest => -4,", "Moves::ParkourWest => -3,", "moves: parkour west offset 3"),
    (MOVES, "Moves::ParkourNorth => -4,", "Moves::ParkourNorth => -5,", "moves: parkour north offset 5"),
    (MOVES, "Moves::ParkourSouth => 4,", "Moves::ParkourSouth => 5,", "moves: parkour south offset 5"),
    (MOVES, "matches!(self, Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "false", "moves: parkour not dynamic in xz"),
    (MOVES, "matches!(self, Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "matches!(self, Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast)", "moves: parkour west not dynamic in xz"),
    (MOVES, "matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | ", "matches!(self, ", "moves: descends not dynamic in y"),
    (MOVES, "| Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth", "| Moves::ParkourNorth", "moves: diagonals not dynamic in y"),
    (MOVES, "Moves::Downward | Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth => -1,", "Moves::Downward | Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth => -1,", "moves: descend south keeps its height"),
    (MOVES, "Moves::Pillar | Moves::AscendNorth | Moves::AscendSouth | Moves::AscendEast | Moves::AscendWest => 1,", "Moves::Pillar | Moves::AscendNorth | Moves::AscendSouth | Moves::AscendEast => 1,", "moves: ascend west keeps its height"),
    (MOVES, "Moves::DescendEast => descend::cost(ctx, x, y, z, x.wrapping_add(1), z, res),", "Moves::DescendEast => descend::cost(ctx, x, y, z, x.wrapping_sub(1), z, res),", "apply: descend east goes west"),
    (MOVES, "Moves::DescendNorth => descend::cost(ctx, x, y, z, x, z.wrapping_sub(1), res),", "Moves::DescendNorth => descend::cost(ctx, x, y, z, x, z.wrapping_add(1), res),", "apply: descend north goes south"),
    (MOVES, "Moves::DiagonalSoutheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_add(1), res),", "Moves::DiagonalSoutheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_sub(1), res),", "apply: southeast goes north"),
    (MOVES, "res.x = x.wrapping_add(self.x_offset());", "res.x = x;", "apply: fixed move keeps x"),
    (MOVES, "res.y = y.wrapping_add(self.y_offset());", "res.y = y;", "apply: fixed move keeps y"),
    (MOVES, "res.z = z.wrapping_add(self.z_offset());", "res.z = z;", "apply: fixed move keeps z"),
    # --- border, world, hash
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "border: min x at the block's own x"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x.wrapping_add(1) as f64) > self.min_x && (x.wrapping_add(1) as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "border: max x at the next x"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x.wrapping_add(1) as f64) >= self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "border: min x edge inside"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) <= self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "border: max x edge inside"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z as f64) > self.min_z && (z as f64) < self.max_z", "border: min z at the block's own z"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z.wrapping_add(1) as f64) < self.max_z", "border: max z at the next z"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) >= self.min_z && (z as f64) < self.max_z", "border: min z edge inside"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) <= self.max_z", "border: max z edge inside"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x", "border: z not tested"),
    (HELPER, "(x.wrapping_add(1) as f64) > self.min_x && (x as f64) < self.max_x && (z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "(z.wrapping_add(1) as f64) > self.min_z && (z as f64) < self.max_z", "border: x not tested"),
    (WORLD, "fn is_loaded(&self, x: i32, z: i32) -> bool {\n        self.contains_loaded_chunk(x, z)", "fn is_loaded(&self, x: i32, z: i32) -> bool {\n        false", "world: default isLoaded false"),
    (POS, "let mut hash: i64 = 3241;", "let mut hash: i64 = 3242;", "long hash: start value"),
    (POS, "hash = 3457689i64.wrapping_mul(hash).wrapping_add(x as i64);", "hash = 3457689i64.wrapping_mul(hash).wrapping_add((x as i64) << 1);", "long hash: x term doubled"),
    (POS, "hash = 3457689i64.wrapping_mul(hash).wrapping_add(x as i64);", "hash = 3457688i64.wrapping_mul(hash).wrapping_add(x as i64);", "long hash: first multiplier"),
    (POS, "hash = 2873465i64.wrapping_mul(hash).wrapping_add(z as i64);", "hash = 2873465i64.wrapping_mul(hash).wrapping_add(x as i64);", "long hash: z term is x"),
    (POS, "hash = 2873465i64.wrapping_mul(hash).wrapping_add(z as i64);", "hash = 2873466i64.wrapping_mul(hash).wrapping_add(z as i64);", "long hash: last multiplier"),
    (POS, "hash = 8734625i64.wrapping_mul(hash).wrapping_add(y as i64);", "hash = 8734625i64.wrapping_mul(hash).wrapping_add(y as i64 & 0xFF);", "long hash: y masked"),
    # --- added after the review of S3 (y = 0, z = 0, far avoidance centres, tall worlds, long corridors, exception causes)
    (NODE, "((y >> CELL_BITS_Y) & 0xFF) as i64", "((y.wrapping_abs() >> CELL_BITS_Y) & 0xFF) as i64", "cell key: y absolute value"),
    (NODE, "((y >> CELL_BITS_Y) & 0xFF) as i64", "((y / 8) & 0xFF) as i64", "cell key: y divided, not shifted"),
    (NODE, "(((z >> CELL_BITS_Z) & 0xFFFFFFF) as i64) << 8", "(((z.wrapping_abs() >> CELL_BITS_Z) & 0xFFFFFFF) as i64) << 8", "cell key: z absolute value"),
    (NODE, "(((x >> CELL_BITS_X) & 0xFFFFFFF) as i64) << 36", "(((x.wrapping_abs() >> CELL_BITS_X) & 0xFFFFFFF) as i64) << 36", "cell key: x absolute value"),
    (FAV, "if s.mob_avoidance_coefficient != 1.0 {", "if s.mob_avoidance_coefficient != 1.0 && s.mob_spawner_avoidance_coefficient != 1.0 {", "avoidance: mobs need the spawner coefficient too"),
    (FAV, "if s.mob_spawner_avoidance_coefficient != 1.0 {", "if s.mob_spawner_avoidance_coefficient != 1.0 && s.mob_avoidance_coefficient != 1.0 {", "avoidance: spawners need the mob coefficient too"),
    (POS, "hash = 3457689i64.wrapping_mul(hash).wrapping_add(x as i64);", "hash = 3457689i64.wrapping_mul(hash).wrapping_add(x as i64 & 0xFFFFFF);", "long hash: x masked to 24 bits"),
    (POS, "hash = 2873465i64.wrapping_mul(hash).wrapping_add(z as i64);", "hash = 2873465i64.wrapping_mul(hash).wrapping_add(z as i64 & 0xFFFFFF);", "long hash: z masked to 24 bits"),
    (POS, "hash = 8734625i64.wrapping_mul(hash).wrapping_add(y as i64);", "hash = 8734625i64.wrapping_mul(hash).wrapping_add(y as i64 & 0xFFFF);", "long hash: y masked to 16 bits"),
    (ASTAR, "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) as f64", "(x_diff as i64 * x_diff as i64 + y_diff as i64 * y_diff as i64 + z_diff as i64 * z_diff as i64) as f64", "distance from start in 64 bits"),
    (NODE, "return Err(SearchError::ImplausibleHeuristic { goal: goal.clone(), x, y, z });", "return Err(SearchError::EmptyPath);", "heuristic error is another error"),
    (PATH, "return Err(SearchError::CutoffOutOfRange { from, to, len });", "return Err(SearchError::EmptyPath);", "cutoff range error is another error"),
]

RUN_TIMEOUT = 300  # seconds; the unmutated comparison takes under a minute
TEST_TIMEOUT = 600
COPIED = ["Cargo.toml", "Cargo.lock", "rustfmt.toml", "mod/engine/Cargo.toml", "mod/engine/src", "tools/search-compare/Cargo.toml", "tools/search-compare/src", "tools/cost-compare/Cargo.toml", "tools/cost-compare/src"]
REPORT = re.compile(r'"mismatches":(\d+)')


def shared():
    """low_priority and default_jobs from the cost comparison's mutate.py (importing it only defines its list)."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "cost-compare", "mutate.py")
    spec = importlib.util.spec_from_file_location("cost_compare_mutate", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.low_priority, mod.default_jobs, mod.ResultCache, mod.run_timeout


def where(repo):
    bad = 0
    for file, a, b, name in M:
        with open(os.path.join(repo, file), encoding="utf-8") as f:
            n = f.read().count(a)
        if n != 1:
            bad += 1
            print(f"{n} matches: {name}")
    names = [m[3] for m in M]
    dup = sorted({n for n in names if names.count(n) > 1})
    if dup:
        print("names used twice:", dup)
    print(f"{len(M)} mutations, {bad} without exactly one match")
    return 1 if bad or dup else 0


def known_missed(repo):
    """{name: reason} from known-missed.txt, the same format and parser as gate.py: lines are 'name | reason'."""
    out = {}
    with open(os.path.join(repo, "tools", "search-compare", "known-missed.txt"), encoding="utf-8") as f:
        for line in f:
            if line.strip() and not line.lstrip().startswith("#"):
                name, sep, reason = line.partition(" | ")
                if sep and reason.strip():
                    out[name.strip()] = reason.strip()
    return out


class Worker:
    """One copy of the workspace with its own target folder, so that workers never see each other's errors."""

    def __init__(self, repo, golden, root, low):
        self.repo, self.golden, self.root = repo, golden, root
        for rel in COPIED:
            src, dst = os.path.join(repo, rel), os.path.join(root, rel)
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            if os.path.isdir(src):
                shutil.copytree(src, dst, ignore=shutil.ignore_patterns("target"))
            else:
                shutil.copy(src, dst)
        self.env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(root, "target"), CARGO_BUILD_JOBS="2")
        self.prefix = ["nice", "-n", "15"] if low else []
        self.exe = os.path.join(root, "target", "release", "search-compare")

    def restore(self, file):
        shutil.copy(os.path.join(self.repo, file), os.path.join(self.root, file))

    def build(self):
        return subprocess.run(self.prefix + ["cargo", "build", "--release", "--quiet", "-p", "search-compare"], cwd=self.root, env=self.env, capture_output=True, text=True)

    def run_golden(self, args=(), timeout=RUN_TIMEOUT):
        """(mismatches, why): the mismatches the comparison reports. Over RUN_TIMEOUT or an abort or a coverage gap give a count of 1 or more
        and a reason, because the unmutated run exits 0. None means the machine killed the run, which says nothing about the error."""
        try:
            r = subprocess.run(self.prefix + [self.exe, self.golden, *args], cwd=self.root, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return 1, "timed out"
        if r.returncode == -signal.SIGKILL:
            return None, "killed from outside"
        m = REPORT.search(r.stdout)
        if r.returncode and not m:
            return 1, "aborted: " + (r.stderr.strip().splitlines() or ["?"])[-1][:80]
        if r.returncode and int(m.group(1)) == 0:
            return 1, "coverage missing"
        return int(m.group(1)) if m else 1, ""

    def unit_tests_fail(self):
        """Whether the engine's own unit tests notice the error (the mutant is still in place). Information only: it never turns MISSED into CAUGHT."""
        try:
            r = subprocess.run(self.prefix + ["cargo", "test", "--quiet", "-p", "baritone-engine"], cwd=self.root, env=self.env, capture_output=True, text=True, timeout=TEST_TIMEOUT)
        except subprocess.TimeoutExpired:
            return True
        return "FAILED" in r.stdout or "panicked" in r.stdout

    def try_mutation(self, mutation, timeout):
        """(verdict, mismatches, note). The verdict is CAUGHT, MISSED or None for 'not applied'."""
        file, a, b, name = mutation
        path = os.path.join(self.root, file)
        self.restore(file)
        with open(path, encoding="utf-8") as f:
            text = f.read()
        if text.count(a) != 1:
            return None, 0, f"the pattern occurs {text.count(a)} times"
        with open(path, "w", encoding="utf-8") as f:
            f.write(text.replace(a, b, 1))
        try:
            built = self.build()
            if built.returncode:
                lines = built.stderr.strip().splitlines()
                return None, 0, "does not build: " + (lines[-1][:100] if lines else "")
            n, why = self.run_golden(("first",), timeout)
            if n is None:
                return None, 0, why
            if n:
                return "CAUGHT", n, why
            return "MISSED", 0, "unit tests catch it" if self.unit_tests_fail() else ""
        finally:
            self.restore(file)


def main():
    repo = os.path.abspath(sys.argv[1])
    if len(sys.argv) > 2 and sys.argv[2] == "--where":
        return where(repo)
    golden = os.path.abspath(sys.argv[2])
    mutations = M
    if len(sys.argv) > 3:
        keep = set(sys.argv[3].split("|"))
        mutations = [m for m in M if m[3] in keep]
    low_priority, default_jobs, ResultCache, run_timeout = shared()
    low = low_priority()
    jobs = int(os.environ.get("BARITONE_MUTATE_JOBS") or (2 if low else default_jobs()))
    root = tempfile.mkdtemp(prefix="search-compare-")
    started = time.time()
    try:
        with ThreadPoolExecutor(jobs) as pool:
            workers = list(pool.map(lambda i: Worker(repo, golden, os.path.join(root, str(i)), low), range(jobs)))

            def baseline(w):
                b = w.build()
                return (None, "no build: " + b.stderr[:200]) if b.returncode else w.run_golden()

            began = time.time()
            bases = list(pool.map(baseline, workers))
            timeout = run_timeout(time.time() - began)
            if any(n != 0 for n, _ in bases):
                print("baseline run failed:", bases)
                print("\n0 caught, 0 missed, 1 not applied")
                print("missed: []")
                print("not applied:", ["baseline"])
                return 1
            print(f"baseline: 0 mismatches, {jobs} workers", flush=True)
            cache = ResultCache(repo, COPIED, golden, "search-compare")
            free = queue.SimpleQueue()
            for w in workers:
                free.put(w)

            def one(mutation):
                done = cache.get(mutation)
                if done:
                    return done
                w = free.get()
                try:
                    verdict, n, why = w.try_mutation(mutation, timeout)
                finally:
                    free.put(w)
                if verdict:
                    cache.put(mutation, [verdict, n, why])
                return verdict, n, why

            caught, missed, bad = [], [], []
            for (file, a, b, name), (verdict, n, why) in zip(mutations, pool.map(one, mutations)):
                if verdict is None:
                    bad.append(name + (" (" + why + ")" if why else ""))
                    print(f"not applied: {name} ({why})", flush=True)
                    continue
                # the line format gate.py reads; the number of mismatches stands in for 'wrong'
                print(f"{verdict:7s} wrong={n:6d} close={0:6d} {name}" + (f"  [{why}]" if why and verdict == "CAUGHT" else ""), flush=True)
                if why and verdict == "MISSED":
                    print(f"  note: {name}: {why}", flush=True)
                (caught if verdict == "CAUGHT" else missed).append(name)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    known = known_missed(repo)
    unlisted = [n for n in missed if n not in known]
    stale = [n for n in known if n in caught]
    print(f"\n{len(caught)} caught, {len(missed)} missed ({len(missed) - len(unlisted)} listed in known-missed.txt with a reason), {len(bad)} not applied")
    print(f"took {time.time() - started:.0f} s with {jobs} workers")
    print("missed:", missed)
    print("missed and not listed:", unlisted)
    print("listed but caught:", stale)
    print("not applied:", bad)
    return 1 if unlisted or stale or bad else 0


if __name__ == "__main__":
    sys.exit(main())
