// baritone-rs engine: free look. Ported from cabaletta/baritone at 1b8e87c: the target modes and the restore of the player's rotation in
// src/main/java/baritone/behavior/LookBehavior.java, and the settings freeLook, blockFreeLook, antiCheatCompatibility, smoothLook and
// smoothLookTicks of Settings.java. LGPL-3.0-or-later, see lib.rs.
//
// Upstream sets the player's rotation after the player's own update (PRE), so the movement packet carries it, and puts the player's
// rotation back after the entities were updated (POST). Which of the two happens, and whether the player's view is put back, depends on
// the "mode" of the target. This file holds that decision and the memory it needs (the rotation the server was sent, the yaw buffer of
// `smoothLook`) as plain data. The glue puts the rotation in during the game's player tick and takes it out again (docs/deviations.md D29, D35).
// Not ported: the elytra and swimming branches of the mode (those movements do not exist here).
use crate::look::Rotation;
use std::collections::VecDeque;

/// The settings free look reads, with upstream's defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct LookSettings {
    /// `freeLook`, default true: move without forcing the client's rotation.
    pub free_look: bool,
    /// `blockFreeLook`, default false: break and place without forcing the client's rotation. Needs `freeLook`.
    pub block_free_look: bool,
    /// `antiCheatCompatibility`, default true: with `freeLook`, the rotation of a plain move is still sent to the server.
    pub anti_cheat_compatibility: bool,
    /// `smoothLook`, default false: the client's yaw is the average of the last `smooth_look_ticks` target yaws.
    pub smooth_look: bool,
    /// `smoothLookTicks`, default 5.
    pub smooth_look_ticks: i32,
}

impl Default for LookSettings {
    fn default() -> Self {
        LookSettings { free_look: true, block_free_look: false, anti_cheat_compatibility: true, smooth_look: false, smooth_look_ticks: 5 }
    }
}

/// `LookBehavior.Target.Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The rotation is set on the client and the player sees it.
    Client,
    /// The rotation is set for the player's tick only, so the server and the movement see it and the player's view does not change.
    Server,
    /// Nothing changes, on the client or on the server.
    None,
}

/// `LookBehavior.Target.Mode.resolve`:
/// Upstream's `resolve`, without the elytra and swimming branches. `block_interact` is the target's `hasToForceRotations` (breaking and placing).
pub fn resolve_mode(s: &LookSettings, block_interact: bool) -> Mode {
    if s.free_look {
        // A break or a place needs the rotation set somehow, or the crosshair is on whatever the player looks at.
        if block_interact {
            return if s.block_free_look { Mode::Server } else { Mode::Client };
        }
        return if s.anti_cheat_compatibility { Mode::Server } else { Mode::None };
    }
    // all free look settings are off, so the angles are set
    Mode::Client
}

/// What happens to the player's view after the tick that carried the rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum After {
    /// The view stays at the rotation (mode `Client`).
    Keep,
    /// The view the player had is put back (mode `Server`).
    Player,
    /// Mode `Client` with `smoothLook`: the yaw becomes this average (`None`: the player's yaw from before), the pitch stays.
    SmoothYaw(Option<f32>),
}

/// The rotation for the next player tick and what to do after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub rotation: Rotation,
    pub after: After,
}

/// Upstream's `serverRotation` and the buffer of `smoothLook`.
#[derive(Clone, Debug, Default)]
pub struct LookState {
    server_rotation: Option<Rotation>,
    smooth_yaw: VecDeque<f32>,
}

impl LookState {
    pub fn new() -> LookState {
        LookState::default()
    }

    /// `LookBehavior.getEffectiveRotation`:
    /// Also `BaritonePlayerContext.playerRotations`. With `freeLook`, the rotation the server was last
    /// sent while a path turns the player; otherwise (and before the first one) the player's own.
    pub fn effective(&self, s: &LookSettings, player: Rotation) -> Rotation {
        if s.free_look {
            self.server_rotation.unwrap_or(player)
        } else {
            player
        }
    }

    /// The rotation the server was last sent, if a path set one.
    pub fn server_rotation(&self) -> Option<Rotation> {
        self.server_rotation
    }

    /// `LookBehavior.onWorldEvent`:
    /// Forgets the rotation the server saw: the path ended, or the level is gone.
    pub fn clear(&mut self) {
        self.server_rotation = None;
    }

    /// `LookBehavior.onPlayerUpdate`:
    /// `LookBehavior.onSendPacket`:
    /// Decides what the next player tick carries (and so what the server is sent). `rotation` is the aim after the aim processor and `target`
    /// the rotation the movement asked for (what `smoothLook` averages); `block_interact` is whether the target must be forced.
    /// `None` when the tick leaves the player's rotation alone (no target, or mode `None`).
    pub fn plan(&mut self, s: &LookSettings, rotation: Option<Rotation>, target: Option<Rotation>, block_interact: bool) -> Option<Plan> {
        let Some(rotation) = rotation else {
            self.server_rotation = None;
            return None;
        };
        let mode = resolve_mode(s, block_interact);
        if mode == Mode::None {
            // the packet carries the player's own rotation
            self.server_rotation = None;
            return None;
        }
        self.smooth_yaw.push_back(target.unwrap_or(rotation).yaw);
        while self.smooth_yaw.len() as i64 > i64::from(s.smooth_look_ticks) {
            self.smooth_yaw.pop_front();
        }
        self.server_rotation = Some(rotation);
        let after = if mode == Mode::Server {
            After::Player
        } else if s.smooth_look {
            let n = self.smooth_yaw.len();
            // the mean of doubles, as `DoubleStream.average`
            let avg = (n > 0).then(|| (self.smooth_yaw.iter().map(|y| f64::from(*y)).sum::<f64>() / n as f64) as f32);
            After::SmoothYaw(avg)
        } else {
            After::Keep
        };
        Some(Plan { rotation, after })
    }
}

/// The view after the tick, given the view the player had before the rotation was put in. `None` for `Keep`: nothing to do.
pub fn restored_view(plan: &Plan, before: Rotation) -> Option<Rotation> {
    match plan.after {
        After::Keep => None,
        After::Player => Some(before),
        After::SmoothYaw(avg) => Some(Rotation::new(avg.unwrap_or(before.yaw), plan.rotation.pitch)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rot(y: f32, p: f32) -> Rotation {
        Rotation::new(y, p)
    }

    #[test]
    fn the_modes_follow_upstream_s_table() {
        let mut s = LookSettings::default();
        // the defaults: a move is silent, a break or place turns the view
        assert_eq!(resolve_mode(&s, false), Mode::Server);
        assert_eq!(resolve_mode(&s, true), Mode::Client);
        s.block_free_look = true;
        assert_eq!(resolve_mode(&s, true), Mode::Server);
        s.anti_cheat_compatibility = false;
        assert_eq!(resolve_mode(&s, false), Mode::None);
        assert_eq!(resolve_mode(&s, true), Mode::Server);
        // freeLook off: everything turns the view, whatever the other settings say
        s.free_look = false;
        assert_eq!(resolve_mode(&s, false), Mode::Client);
        assert_eq!(resolve_mode(&s, true), Mode::Client);
    }

    #[test]
    fn a_silent_move_puts_the_players_view_back_and_remembers_what_the_server_saw() {
        let s = LookSettings::default();
        let mut st = LookState::new();
        let player = rot(10.0, 5.0);
        assert_eq!(st.effective(&s, player), player);
        let plan = st.plan(&s, Some(rot(90.0, 0.0)), Some(rot(90.0, 0.0)), false).expect("a plan");
        assert_eq!(plan.after, After::Player);
        assert_eq!(restored_view(&plan, player), Some(player));
        // the next aim is relative to what the server saw, not to the player's view
        assert_eq!(st.effective(&s, rot(33.0, 0.0)), rot(90.0, 0.0));
        // the path ends
        assert!(st.plan(&s, None, None, false).is_none());
        assert_eq!(st.effective(&s, rot(33.0, 0.0)), rot(33.0, 0.0));
    }

    #[test]
    fn with_free_look_off_the_view_is_the_aim_and_the_effective_rotation_is_the_players() {
        let s = LookSettings { free_look: false, ..LookSettings::default() };
        let mut st = LookState::new();
        let plan = st.plan(&s, Some(rot(90.0, 3.0)), Some(rot(90.0, 3.0)), false).unwrap();
        assert_eq!(plan, Plan { rotation: rot(90.0, 3.0), after: After::Keep });
        assert_eq!(restored_view(&plan, rot(0.0, 0.0)), None);
        assert_eq!(st.effective(&s, rot(1.0, 2.0)), rot(1.0, 2.0));
    }

    #[test]
    fn mode_none_changes_nothing() {
        let s = LookSettings { anti_cheat_compatibility: false, ..LookSettings::default() };
        let mut st = LookState::new();
        assert!(st.plan(&s, Some(rot(90.0, 0.0)), Some(rot(90.0, 0.0)), false).is_none());
        assert_eq!(st.server_rotation(), None);
    }

    #[test]
    fn smooth_look_averages_the_last_targets_and_keeps_the_pitch() {
        let s = LookSettings { free_look: false, smooth_look: true, smooth_look_ticks: 2, ..LookSettings::default() };
        let mut st = LookState::new();
        let p1 = st.plan(&s, Some(rot(10.0, 4.0)), Some(rot(10.0, 4.0)), false).unwrap();
        assert_eq!(restored_view(&p1, rot(0.0, 0.0)), Some(rot(10.0, 4.0)));
        let p2 = st.plan(&s, Some(rot(30.0, 6.0)), Some(rot(30.0, 6.0)), false).unwrap();
        assert_eq!(restored_view(&p2, rot(0.0, 0.0)), Some(rot(20.0, 6.0)));
        // the window is two ticks long
        let p3 = st.plan(&s, Some(rot(50.0, 6.0)), Some(rot(50.0, 6.0)), false).unwrap();
        assert_eq!(restored_view(&p3, rot(0.0, 0.0)), Some(rot(40.0, 6.0)));
        // no ticks to average: the player's yaw is kept
        let s0 = LookSettings { smooth_look_ticks: 0, ..s };
        let p = st.plan(&s0, Some(rot(70.0, 1.0)), Some(rot(70.0, 1.0)), false).unwrap();
        assert_eq!(restored_view(&p, rot(7.0, 0.0)), Some(rot(7.0, 1.0)));
    }

    #[test]
    fn breaking_with_block_free_look_is_silent() {
        let s = LookSettings { block_free_look: true, ..LookSettings::default() };
        let mut st = LookState::new();
        let plan = st.plan(&s, Some(rot(-90.0, 40.0)), Some(rot(-90.0, 40.0)), true).unwrap();
        assert_eq!(plan.after, After::Player);
    }
}
