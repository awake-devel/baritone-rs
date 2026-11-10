// baritone-rs engine: the processes and the pathing state machine. LGPL-3.0-or-later, see lib.rs.
//
// `custom_goal` is `CustomGoalProcess` (the process behind `goto`), `mine` is `MineProcess`, `get_to_block` is `GetToBlockProcess`
// (`goto <block>`) and `follow` is `FollowProcess` and `explore` is `ExploreProcess`. `pathing` is `PathingBehavior` with `PathingControlManager`: the process manager that hands control to one process
// at a time, and the search running behind the `SearchRunner` trait.
pub mod build;
pub mod custom_goal;
pub mod explore;
pub mod follow;
pub mod get_to_block;
pub mod mine;
pub mod pathing;

use crate::pos::Pos;

pub use build::{Aabb, BuildEnv, BuildFacts, BuilderProcess, InvSlot, NoBuildFacts, ValidRules};
pub use custom_goal::{CustomGoalProcess, PathingCommand, PathingCommandType};
pub use explore::{ExploreProcess, JsonChunkFilter};
pub use follow::{FollowFilter, FollowProcess};
pub use get_to_block::GetToBlockProcess;
pub use mine::{MineProcess, ScanJob, ScanKind, ScanResult};
pub use pathing::{path_start, PathEvent, Pathing, ProcessContext, SearchJob, SearchOutcome, SearchRunner, Tables, TickEnv, TickOutput};

/// A dropped item the player could see: `ItemEntity` with the block its position is in (`Entity.blockPosition`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemEntity {
    /// The stack's item as a registry path, `minecraft:` left off.
    pub item: String,
    pub pos: Pos,
}

/// An entity the follow process can see: what `FollowProcess` and `FollowCommand` read of an `Entity`, as plain data. The glue lists the
/// entities of the loaded world each tick, without the player itself (upstream's `followable` refuses it).
#[derive(Clone, Debug, PartialEq)]
pub struct EntityInfo {
    /// `Entity.getId`. Two entities are the same when their ids are (`Entity.equals`).
    pub id: u32,
    /// The entity type's registry id with its namespace, `minecraft:pig` (`BuiltInRegistries.ENTITY_TYPE.getKey`).
    pub kind: String,
    /// The player's name when this is a player (`minecraft:player`).
    pub player_name: Option<String>,
    /// The stack's item with its namespace when this is a dropped item (`minecraft:item`).
    pub item: Option<String>,
    /// Upstream's `Entity.position`: the feet, not the block they are in.
    pub position: [f64; 3],
    /// A `LivingEntity` (what `follow entities` follows).
    pub living: bool,
    /// Upstream's `Entity.isAlive`: not removed, and for a living entity more than 0 health. The glue may leave the dead out.
    pub alive: bool,
}

/// What the processes read of the game that the world snapshot and the player do not carry. The glue gives it to
/// [`Pathing::set_inputs`] before each tick; the default is what a game without those facts would say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessInputs {
    /// Upstream's `System.currentTimeMillis`: any clock in milliseconds. The mine process holds a mined block as a drop for a few hundred of them.
    pub now_ms: i64,
    /// The dropped items in the loaded world (`ClientLevel.entitiesForRendering`, the `ItemEntity` ones). Empty means none, or that the glue cannot see them.
    pub item_entities: Vec<ItemEntity>,
    /// A container is open: the player's `containerMenu` is not the inventory menu. The get-to-block process uses it to see that its right click worked.
    pub container_open: bool,
    /// The entities of the loaded world, never the player (`ctx.entitiesStream`). Empty means none, or that the glue does not list them: it does only
    /// while the follow process has a filter.
    pub entities: Vec<EntityInfo>,
}
