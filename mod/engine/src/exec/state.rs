// baritone-rs engine: what the executor reads from the player and what it answers. New structure, ported from
// cabaletta/baritone at 1b8e87c: MovementState.java, MovementStatus.java, InputOverrideHandler.java (the forced inputs) and
// PlayerMovementInput.java. LGPL-3.0-or-later, see lib.rs.
use crate::look::{Rotation, Vec3};
use crate::pos::Pos;

/// The player, as the executor needs to see it. The glue fills it before each tick. Positions are the player's feet, in blocks.
/// Nothing here is a Horizon type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerView {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The velocity per tick (`getDeltaMovement`).
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub in_water: bool,
    pub swimming: bool,
    pub crouching: bool,
    /// `horizontalCollision`.
    pub horizontal_collision: bool,
    /// Upstream's `isInWall`: the head is inside a block.
    pub in_wall: bool,
    /// `getFoodData().getFoodLevel()`; sprinting needs more than 6.
    pub food_level: i32,
    /// Upstream's `getEyeHeight`: 1.62 standing, 1.27 sneaking.
    pub eye_height: f64,
    /// The block at the raw feet position (the height plus 0.1251) is a slab of any type: `playerFeet` then answers the block above.
    pub feet_in_slab: bool,
}

impl PlayerView {
    /// A player standing still on the ground at a position, looking south, with a full stomach.
    pub fn standing(x: f64, y: f64, z: f64) -> PlayerView {
        PlayerView { x, y, z, vx: 0.0, vy: 0.0, vz: 0.0, yaw: 0.0, pitch: 0.0, on_ground: true, in_water: false, swimming: false, crouching: false, horizontal_collision: false, in_wall: false, food_level: 20, eye_height: 1.62, feet_in_slab: false }
    }

    /// Upstream's `IPlayerContext.playerFeet`: the block the feet are in. 0.1251 is added to the height so soul sand and farmland count as the block above them,
    /// and a slab there answers the block above it (`feet_in_slab`, read by the glue from the world, as upstream's `instanceof SlabBlock`).
    pub fn feet(&self) -> Pos {
        Pos::new(self.x.floor() as i32, (self.y + 0.1251).floor() as i32 + i32::from(self.feet_in_slab), self.z.floor() as i32)
    }

    /// The block at the raw feet position, before the slab rule of `feet`. The glue reads the world there to set `feet_in_slab`.
    pub fn raw_feet(&self) -> Pos {
        Pos::new(self.x.floor() as i32, (self.y + 0.1251).floor() as i32, self.z.floor() as i32)
    }

    /// `IPlayerContext.playerHead`.
    pub fn head(&self) -> Vec3 {
        Vec3::new(self.x, self.y + self.eye_height, self.z)
    }

    /// `IPlayerContext.playerRotations`.
    pub fn rotations(&self) -> Rotation {
        Rotation::new(self.yaw, self.pitch)
    }

    /// Upstream's `Entity.blockPosition`: the block that holds the position, without the 0.1251.
    pub fn block_position(&self) -> Pos {
        Pos::new(self.x.floor() as i32, self.y.floor() as i32, self.z.floor() as i32)
    }
}

/// `MovementStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveStatus {
    Prepping,
    Waiting,
    Running,
    Success,
    Unreachable,
    Failed,
    Canceled,
}

impl MoveStatus {
    /// `MovementStatus.isComplete`.
    pub fn is_complete(self) -> bool {
        matches!(self, MoveStatus::Success | MoveStatus::Unreachable | MoveStatus::Failed | MoveStatus::Canceled)
    }
}

/// Why a movement ended as `Failed` because it needs something this version does not do. Each of these is a branch of
/// upstream that this version does not carry out; the movement stops, the path is cancelled and a new one is searched.
/// Each has a row marked `later` in docs/port-ledger.tsv.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveFailure {
    /// A block in the way would have to be broken (`prepared` mines it upstream). `allowBreak` is off.
    NeedsBreaking(Pos),
    /// A block would have to be placed: a bridge block, a support to jump onto, or a pillar. Placing is off.
    NeedsPlacing(Pos),
    /// A fall that upstream lands with a water bucket.
    NeedsBucket,
    /// A closed wooden door or fence gate must be opened, and `ExecSettings::can_use_blocks` is false.
    NeedsUseBlock(Pos),
    /// `allowSwimming` is on: the swimming rules (`holdWaterline`, `preTurnIntoTheBend`) are not ported.
    Swimming,
    /// The head is inside a block: upstream mines whatever is selected.
    InsideWall,
    /// A parkour movement: `allowParkour` is off by default and its executor is not ported.
    Parkour,
}

/// Why the executor cancelled its path. `PathExecutor.cancel` has no reason upstream, only a debug log line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CancelReason {
    /// More than `MAX_MAX_DIST_FROM_PATH` (3 blocks) from the path.
    TooFarFromPath,
    /// More than `MAX_DIST_FROM_PATH` (2 blocks) away for more than `MAX_TICKS_AWAY` (200) ticks.
    AwayTooLong,
    /// A movement a few steps ahead has become impossible.
    LookaheadImpossible,
    /// This movement has become impossible.
    MovementImpossible,
    /// This movement costs more than it did when the path was made, by more than `maxCostIncrease`.
    CostIncreased,
    /// The movement ended as `Unreachable` or `Failed`.
    Status(MoveStatus),
    /// The movement needs something this version does not do.
    Unsupported(MoveFailure),
    /// The movement took longer than its cost estimate plus `movementTimeoutTicks`.
    TimedOut,
    /// The path ended in a state upstream throws on (`overrideFall` returned a destination not on the path).
    IllegalState,
}

/// Upstream's `MovementState.MovementTarget`: a rotation the movement wants, and whether it must be applied (placing and mining) or is only a look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub rotation: Rotation,
    pub force: bool,
}

/// The forced inputs of `InputOverrideHandler`: the keys the executor holds down. `click_left` is never set in this version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
    pub click_right: bool,
}

/// Upstream's `MovementState`: the status, the rotation target and the inputs of one movement. Upstream's input map (`setInput`) and
/// the forced inputs are the same flags here, since a movement's inputs start empty on every tick.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveState {
    pub status: MoveStatus,
    /// Kept between ticks, as upstream's `currentState`: a movement that sets no target uses the last one.
    pub target: Option<Target>,
    pub keys: Keys,
    /// The block a `click_right` is for.
    pub use_pos: Option<Pos>,
    pub failure: Option<MoveFailure>,
}

impl MoveState {
    pub fn new() -> MoveState {
        MoveState { status: MoveStatus::Prepping, target: None, keys: Keys::default(), use_pos: None, failure: None }
    }
}

impl Default for MoveState {
    fn default() -> Self {
        MoveState::new()
    }
}

/// `InputOverrideHandler` and the rotation `LookBehavior` was asked for: shared by every executor. The pathing state machine owns one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Controls {
    pub keys: Keys,
    /// `LookBehavior.target`, set by a movement and consumed by `Pathing::tick` when it builds the intent.
    pub look: Option<Rotation>,
    pub use_pos: Option<Pos>,
}

impl Controls {
    /// `InputOverrideHandler.clearAllKeys`:
    /// `InputOverrideHandler.clearAllKeys`.
    pub fn clear_all_keys(&mut self) {
        self.keys = Keys::default();
        self.use_pos = None;
    }
}

/// What the glue does with the player this tick.
///
/// While `active` is true, upstream's `PlayerMovementInput` replaces the keyboard completely (`InputOverrideHandler.inControl`):
/// the player's own movement keys, jump and sneak are not read at all, and the fields below are the only input. When `active`
/// is false nothing is overridden and the player's own keys apply.
///
/// An intent never asks to break or place. `use_block` is the one use of the right click, on a door or fence gate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Intent {
    /// A path is being followed (upstream's `isPathing`, or a forced movement key).
    pub active: bool,
    /// The yaw and pitch to set, already passed through the aim processor. `None`: leave the view as it is.
    pub rotation: Option<Rotation>,
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sneak: bool,
    /// Upstream's `PathExecutor.isSprinting`: whether the player should be sprinting from the next movement. Only meaningful while `active`; when
    /// it is false and `active` is true, the glue stops the sprint (upstream: `setSprinting(false)`).
    pub sprint: bool,
    /// Right click on this block: a closed door or fence gate. Only when `ExecSettings::can_use_blocks` is on.
    pub use_block: Option<Pos>,
    /// `Movement.update` sets `abilities.flying = false` while a path runs.
    pub cancel_flying: bool,
    /// `PathingBehavior.onPlayerUpdate` turns the game's auto jump off while a path runs.
    pub disable_auto_jump: bool,
}

impl Intent {
    /// The intent when no path is followed: nothing is overridden.
    pub fn neutral() -> Intent {
        Intent::default()
    }

    /// Whether the intent overrides nothing and presses nothing.
    pub fn is_neutral(&self) -> bool {
        *self == Intent::neutral()
    }

    /// `PlayerMovementInput.tick`: the forward impulse, +1 for forward and -1 for back, so both cancel.
    pub fn forward_impulse(&self) -> f32 {
        (self.forward as i32 - self.back as i32) as f32
    }

    /// `PlayerMovementInput.tick`: the left impulse, +1 for left and -1 for right.
    pub fn strafe_impulse(&self) -> f32 {
        (self.left as i32 - self.right as i32) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feet_on_a_bottom_slab_are_the_block_above_it() {
        // standing on a bottom slab at y = 61 puts the raw feet in the slab; upstream answers y = 62
        let mut p = PlayerView::standing(255.98, 61.5, 229.5);
        assert_eq!(p.feet(), Pos::new(255, 61, 229));
        p.feet_in_slab = true;
        assert_eq!(p.feet(), Pos::new(255, 62, 229));
        assert_eq!(p.raw_feet(), Pos::new(255, 61, 229));
    }

    #[test]
    fn feet_add_the_soul_sand_offset() {
        let p = PlayerView::standing(10.5, 4.0 - 0.125, -0.5);
        assert_eq!(p.feet(), Pos::new(10, 4, -1));
        assert_eq!(p.block_position(), Pos::new(10, 3, -1));
    }

    #[test]
    fn impulses() {
        let mut i = Intent::neutral();
        assert!(i.is_neutral());
        i.forward = true;
        assert_eq!(i.forward_impulse(), 1.0);
        i.back = true;
        assert_eq!(i.forward_impulse(), 0.0);
        i.left = true;
        assert_eq!(i.strafe_impulse(), 1.0);
        assert!(!i.is_neutral());
    }

    #[test]
    fn only_four_statuses_are_complete() {
        for s in [MoveStatus::Prepping, MoveStatus::Waiting, MoveStatus::Running] {
            assert!(!s.is_complete());
        }
        for s in [MoveStatus::Success, MoveStatus::Unreachable, MoveStatus::Failed, MoveStatus::Canceled] {
            assert!(s.is_complete());
        }
    }
}
