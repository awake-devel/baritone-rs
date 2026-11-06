// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/calc/PathNode.java, and the cell map of
// src/main/java/baritone/pathing/calc/AbstractNodeCostSearch.java (getNodeAtPosition). LGPL-3.0-or-later, see lib.rs.
use super::SearchError;
use crate::costs::COST_INF;
use crate::goals::{Goal, GoalSettings};
use std::collections::HashMap;

/// Index of a node in the arena of one search.
pub type NodeId = u32;

/// `AbstractNodeCostSearch.CELL_BITS_X`, `CELL_BITS_Y`, `CELL_BITS_Z`: nodes live in 16 x 8 x 16 cells.
const CELL_BITS_X: i32 = 4;
const CELL_BITS_Y: i32 = 3;
const CELL_BITS_Z: i32 = 4;
const CELL_SIZE: usize = 1 << (CELL_BITS_X + CELL_BITS_Y + CELL_BITS_Z);
const EMPTY: NodeId = NodeId::MAX;

/// `PathNode`: a node of the search. `previous` is an index into the arena.
#[derive(Clone, Debug)]
pub struct PathNode {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Cached, always `goal.heuristic(x, y, z)`.
    pub estimated_cost_to_goal: f64,
    /// Total cost of getting here from the start.
    pub cost: f64,
    /// Always `estimated_cost_to_goal + cost`.
    pub combined_cost: f64,
    pub previous: Option<NodeId>,
    /// Where the node is in the heap array, -1 when it is not in the heap.
    pub heap_position: i32,
    /// The favoring multiplier for the moves into this node. 0 means not looked up yet.
    pub favor: f64,
}

impl PathNode {
    /// `PathNode.<init>`: the cost starts at `COST_INF`. Upstream throws when the heuristic is NaN, this returns the error.
    pub fn new(x: i32, y: i32, z: i32, goal: &Goal, gs: &GoalSettings) -> Result<PathNode, SearchError> {
        let estimated = goal.heuristic(x, y, z, gs);
        if estimated.is_nan() {
            return Err(SearchError::ImplausibleHeuristic { goal: goal.clone(), x, y, z });
        }
        Ok(PathNode { x, y, z, estimated_cost_to_goal: estimated, cost: COST_INF, combined_cost: 0.0, previous: None, heap_position: -1, favor: 0.0 })
    }

    /// `PathNode.isOpen`: in the heap.
    pub fn is_open(&self) -> bool {
        self.heap_position != -1
    }

    /// `PathNode.equals`: the same position.
    pub fn same_position(&self, other: &PathNode) -> bool {
        self.x == other.x && self.y == other.y && self.z == other.z
    }
}

/// The nodes of one search: the arena, and the position lookup through cells. Upstream never iterates the cell map, so a plain `HashMap` does.
#[derive(Default)]
pub struct NodeMap {
    pub nodes: Vec<PathNode>,
    cells: HashMap<i64, Box<[NodeId; CELL_SIZE]>>,
}

impl NodeMap {
    /// `AbstractNodeCostSearch.getNodeAtPosition`: the node at a position, made on first use. The key and index arithmetic is upstream's, with its arithmetic shifts.
    pub fn get_node_at_position(&mut self, x: i32, y: i32, z: i32, goal: &Goal, gs: &GoalSettings) -> Result<NodeId, SearchError> {
        let cell_key: i64 = (((x >> CELL_BITS_X) & 0xFFFFFFF) as i64) << 36 | (((z >> CELL_BITS_Z) & 0xFFFFFFF) as i64) << 8 | ((y >> CELL_BITS_Y) & 0xFF) as i64;
        let cell = self.cells.entry(cell_key).or_insert_with(|| Box::new([EMPTY; CELL_SIZE]));
        let index = (((y & ((1 << CELL_BITS_Y) - 1)) << (CELL_BITS_X + CELL_BITS_Z)) | ((z & ((1 << CELL_BITS_Z) - 1)) << CELL_BITS_X) | (x & ((1 << CELL_BITS_X) - 1))) as usize;
        if cell[index] == EMPTY {
            let node = PathNode::new(x, y, z, goal, gs)?;
            let id = self.nodes.len() as NodeId;
            self.nodes.push(node);
            cell[index] = id;
            return Ok(id);
        }
        Ok(cell[index])
    }

    /// How many nodes were made, which is what `AbstractNodeCostSearch.mapSize` reports.
    pub fn num_nodes_created(&self) -> usize {
        self.nodes.len()
    }
}
