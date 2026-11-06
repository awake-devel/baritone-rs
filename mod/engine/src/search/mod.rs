// baritone-rs engine: the search. LGPL-3.0-or-later, see lib.rs.
//
// A* (`astar`), its nodes and open set, the path it returns and the favoring of the previous path.
pub mod astar;
pub mod favoring;
pub mod node;
pub mod open_set;
pub mod path;

pub use astar::{AStarPathFinder, PathCalculationResult, ResultType, SearchLimits};
pub use favoring::{Avoidance, Favoring};
pub use node::{NodeId, PathNode};
pub use open_set::BinaryHeapOpenSet;
pub use path::Path;

use crate::goals::Goal;
use crate::movement::moves::Moves;

/// What upstream signals with an exception during a search. The search returns it where Java throws.
#[derive(Clone, Debug, PartialEq)]
pub enum SearchError {
    /// A move calculated a cost that is zero, negative or NaN.
    ImplausibleCost { moves: Moves, x: i32, y: i32, z: i32, cost: f64 },
    /// A fixed-offset move ended at another x or z than its offset says.
    WrongDestination { moves: Moves, x: i32, y: i32, z: i32, to_x: i32, to_z: i32, expected_x: i32, expected_z: i32 },
    /// A move without a dynamic height ended at another y than its offset says.
    WrongHeight { moves: Moves, x: i32, y: i32, z: i32, to_y: i32, expected_y: i32 },
    /// The goal's heuristic is NaN.
    ImplausibleHeuristic { goal: Goal, x: i32, y: i32, z: i32 },
    /// The finder has been used before (upstream's `calculate` throws).
    Reused,
    /// There are no positions, or there are movements already (upstream's `assembleMovements` throws).
    EmptyPath,
    /// The movements of a path that `postProcess` has not run on.
    NotVerified,
    /// `postProcess` a second time, or on a path that is not a raw `Path`.
    CannotPostProcess,
    /// After `postProcess` cut the path, it has another size than the movements.
    WrongSizeAfterCutoff,
    /// A `subList` of a cut is out of range.
    CutoffOutOfRange { from: i32, to: i32, len: usize },
    /// A fault found by the sanity check; the text is upstream's message.
    SanityCheck(&'static str),
    /// "Paths to be spliced are overlapping incorrectly".
    SpliceOverlap,
}
