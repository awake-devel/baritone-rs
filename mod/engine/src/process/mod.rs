// baritone-rs engine: the processes and the pathing state machine. LGPL-3.0-or-later, see lib.rs.
//
// `custom_goal` is `CustomGoalProcess` (the process behind `goto`), `mine` is `MineProcess` and `get_to_block` is `GetToBlockProcess`
// (`goto <block>`). `pathing` is `PathingBehavior` with `PathingControlManager`: the process manager that hands control to one process
// at a time, and the search running behind the `SearchRunner` trait.
pub mod custom_goal;
pub mod get_to_block;
pub mod mine;
pub mod pathing;

use crate::pos::Pos;

pub use custom_goal::{CustomGoalProcess, PathingCommand, PathingCommandType};
pub use get_to_block::GetToBlockProcess;
pub use mine::{MineProcess, ScanJob, ScanKind, ScanResult};
pub use pathing::{path_start, PathEvent, Pathing, SearchJob, SearchOutcome, SearchRunner, Tables, TickEnv, TickOutput};

/// A dropped item the player could see: `ItemEntity` with the block its position is in (`Entity.blockPosition`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemEntity {
    /// The stack's item as a registry path, `minecraft:` left off.
    pub item: String,
    pub pos: Pos,
}

/// What the processes read of the game that the world snapshot and the player do not carry. The glue gives it to
/// [`Pathing::set_inputs`] before each tick; the default is what a game without those facts would say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessInputs {
    /// Upstream's `System.currentTimeMillis`: any clock in milliseconds. The mine process holds a mined block as a drop for a few hundred of them.
    pub now_ms: i64,
    /// The dropped items in the loaded world (`ClientLevel.entitiesForRendering`, the `ItemEntity` ones). Empty means none, or that the glue cannot see them.
    pub item_entities: Vec<ItemEntity>,
    /// A container is open: the player's `containerMenu` is not the inventory menu. The get-to-block process uses it to see that its right click worked.
    pub container_open: bool,
}
