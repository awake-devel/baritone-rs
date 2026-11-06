// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/pathing/calc/openset/BinaryHeapOpenSet.java
// LGPL-3.0-or-later, see lib.rs.
//
// The same 1-based array and the same sift order. A node's place in the heap lives in the node, so the methods take the node arena.
use super::node::{NodeId, PathNode};

const NONE: NodeId = NodeId::MAX;

/// `BinaryHeapOpenSet`: a binary min-heap on `combinedCost`. Ties are decided by where the sifting stops, so the comparisons are upstream's.
pub struct BinaryHeapOpenSet {
    /// `array`: node ids, index 0 unused.
    array: Vec<NodeId>,
    /// `keys[i]` is always the `combinedCost` of `array[i]`.
    keys: Vec<f64>,
    size: i32,
    /// Test hook: how many sifting comparisons met two equal keys. Only counted, never read by the heap.
    pub ties: u32,
}

impl Default for BinaryHeapOpenSet {
    fn default() -> Self {
        BinaryHeapOpenSet::new()
    }
}

impl BinaryHeapOpenSet {
    /// `BinaryHeapOpenSet.<init>()`: the default capacity, 1024.
    pub fn new() -> Self {
        BinaryHeapOpenSet::with_capacity(1024)
    }

    /// `BinaryHeapOpenSet.<init>(int)`.
    pub fn with_capacity(size: usize) -> Self {
        BinaryHeapOpenSet { array: vec![NONE; size], keys: vec![0.0; size], size: 0, ties: 0 }
    }

    /// `BinaryHeapOpenSet.size`: how many nodes are in the heap.
    pub fn size(&self) -> i32 {
        self.size
    }

    /// `BinaryHeapOpenSet.insert`: the arrays double when full.
    pub fn insert(&mut self, nodes: &mut [PathNode], value: NodeId) {
        if self.size as usize >= self.array.len() - 1 {
            let n = self.array.len() << 1;
            self.array.resize(n, NONE);
            self.keys.resize(n, 0.0);
        }
        self.size += 1;
        let cost = nodes[value as usize].combined_cost;
        self.sift_up(nodes, self.size, value, cost);
    }

    /// `BinaryHeapOpenSet.update`: the cost only ever goes down, so the node only ever moves up.
    pub fn update(&mut self, nodes: &mut [PathNode], val: NodeId) {
        let n = &nodes[val as usize];
        let (pos, cost) = (n.heap_position, n.combined_cost);
        self.sift_up(nodes, pos, val, cost);
    }

    /// `BinaryHeapOpenSet.siftUp`: puts a node at an index and moves it up while the parent's key is larger.
    fn sift_up(&mut self, nodes: &mut [PathNode], index: i32, val: NodeId, cost: f64) {
        let mut index = index;
        let mut parent_ind = index >> 1;
        while index > 1 && self.keys[parent_ind as usize] > cost {
            let parent_node = self.array[parent_ind as usize];
            self.array[index as usize] = parent_node;
            self.keys[index as usize] = self.keys[parent_ind as usize];
            nodes[parent_node as usize].heap_position = index;
            index = parent_ind;
            parent_ind = index >> 1;
        }
        if index > 1 && self.keys[parent_ind as usize] == cost {
            self.ties += 1;
        }
        self.array[index as usize] = val;
        self.keys[index as usize] = cost;
        nodes[val as usize].heap_position = index;
    }

    /// `BinaryHeapOpenSet.isEmpty`: no node is in the heap.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// `BinaryHeapOpenSet.removeLowest`: `None` where upstream throws "Cannot remove from empty heap".
    pub fn remove_lowest(&mut self, nodes: &mut [PathNode]) -> Option<NodeId> {
        if self.size == 0 {
            return None;
        }
        let result = self.array[1];
        nodes[result as usize].heap_position = -1;
        let val = self.array[self.size as usize];
        let cost = self.keys[self.size as usize];
        self.array[self.size as usize] = NONE;
        self.size -= 1;
        if self.size == 0 {
            return Some(result);
        }
        let mut index: i32 = 1;
        let mut child: i32 = 2;
        while child <= self.size {
            let mut child_cost = self.keys[child as usize];
            if child < self.size && self.keys[child as usize + 1] < child_cost {
                child += 1;
                child_cost = self.keys[child as usize];
            } else if child < self.size && self.keys[child as usize + 1] == child_cost {
                self.ties += 1;
            }
            if cost == child_cost {
                self.ties += 1;
            }
            if cost <= child_cost {
                break;
            }
            let child_node = self.array[child as usize];
            self.array[index as usize] = child_node;
            self.keys[index as usize] = child_cost;
            nodes[child_node as usize].heap_position = index;
            index = child;
            child = index << 1;
        }
        self.array[index as usize] = val;
        self.keys[index as usize] = cost;
        nodes[val as usize].heap_position = index;
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::{Goal, GoalSettings};

    fn nodes_with_costs(costs: &[f64]) -> Vec<PathNode> {
        let g = Goal::Block { x: 0, y: 0, z: 0 };
        let gs = GoalSettings { cost_heuristic: 1.0, axis_height: 120 };
        costs
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut n = PathNode::new(i as i32, 0, 0, &g, &gs).unwrap();
                n.combined_cost = *c;
                n
            })
            .collect()
    }

    fn drain(h: &mut BinaryHeapOpenSet, nodes: &mut [PathNode]) -> Vec<NodeId> {
        let mut out = vec![];
        while !h.is_empty() {
            out.push(h.remove_lowest(nodes).unwrap());
        }
        out
    }

    #[test]
    fn ties_pop_in_the_order_the_sifting_gives() {
        // Traced by hand through upstream's siftUp and removeLowest: the nodes with cost 1 come out as 1, 3 and the nodes
        // with cost 2 as 4, 2, which is not their insertion order.
        let mut nodes = nodes_with_costs(&[3.0, 1.0, 2.0, 1.0, 2.0]);
        let mut h = BinaryHeapOpenSet::new();
        for id in 0..5 {
            h.insert(&mut nodes, id);
        }
        assert_eq!(h.size(), 5);
        assert_eq!(nodes[1].heap_position, 1);
        assert_eq!(nodes[3].heap_position, 2);
        assert_eq!(nodes[4].heap_position, 5);
        assert_eq!(drain(&mut h, &mut nodes), vec![1, 3, 4, 2, 0]);
        assert!(nodes.iter().all(|n| !n.is_open()));
        assert_eq!(h.remove_lowest(&mut nodes), None);
    }

    #[test]
    fn a_tie_does_not_move_a_node_above_its_parent() {
        // siftUp stops at an equal key: the second node with cost 5 stays below the first.
        let mut nodes = nodes_with_costs(&[5.0, 5.0]);
        let mut h = BinaryHeapOpenSet::new();
        h.insert(&mut nodes, 0);
        h.insert(&mut nodes, 1);
        assert_eq!((nodes[0].heap_position, nodes[1].heap_position), (1, 2));
    }

    #[test]
    fn update_moves_a_node_up_and_tracks_positions() {
        let mut nodes = nodes_with_costs(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let mut h = BinaryHeapOpenSet::new();
        for id in 0..6 {
            h.insert(&mut nodes, id);
        }
        nodes[5].combined_cost = 0.5;
        h.update(&mut nodes, 5);
        assert_eq!(nodes[5].heap_position, 1);
        assert_eq!(h.remove_lowest(&mut nodes), Some(5));
        // every node still in the heap knows its place
        assert!(nodes.iter().take(5).all(|n| n.is_open()));
        assert_eq!(drain(&mut h, &mut nodes), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn grows_past_its_capacity() {
        let costs: Vec<f64> = (0..50).map(|i| (50 - i) as f64).collect();
        let mut nodes = nodes_with_costs(&costs);
        let mut h = BinaryHeapOpenSet::with_capacity(4);
        for id in 0..50 {
            h.insert(&mut nodes, id);
        }
        let order = drain(&mut h, &mut nodes);
        assert_eq!(order, (0..50).rev().collect::<Vec<_>>());
    }

    #[test]
    fn nan_keys_compare_false_both_ways() {
        // `keys[parent] > cost` and `cost <= childCost` are false for NaN, so a NaN node neither rises nor sinks.
        let mut nodes = nodes_with_costs(&[1.0, f64::NAN]);
        let mut h = BinaryHeapOpenSet::new();
        h.insert(&mut nodes, 0);
        h.insert(&mut nodes, 1);
        assert_eq!(nodes[1].heap_position, 2);
    }
}
