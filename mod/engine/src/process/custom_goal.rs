// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/process/CustomGoalProcess.java and the types
// PathingCommand / PathingCommandType it answers with. LGPL-3.0-or-later, see lib.rs.
use crate::goals::{Goal, GoalSettings};
use crate::pos::Pos;

/// `PathingCommandType`, the ones `CustomGoalProcess` sends. Pause, defer and the revalidating ones other processes use come with those processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathingCommandType {
    /// Set the goal and cancel the current segment if that is safe.
    CancelAndSetGoal,
    /// Set the goal, and once, cancel the path if it no longer fits the goal; then path.
    ForceRevalidateGoalAndPath,
    /// Set the goal and path to it, starting a search when none runs.
    SetGoalAndPath,
}

/// `PathingCommand`.
#[derive(Clone, Debug, PartialEq)]
pub struct PathingCommand {
    pub goal: Option<Goal>,
    pub kind: PathingCommandType,
}

/// `CustomGoalProcess.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    None,
    GoalSet,
    PathRequested,
    Executing,
}

/// `CustomGoalProcess`: the process behind `goto` and `goal`. A pure state machine: `on_tick` is given what it reads and answers a command.
#[derive(Clone, Debug)]
pub struct CustomGoalProcess {
    goal: Option<Goal>,
    most_recent_goal: Option<Goal>,
    state: State,
}

impl Default for CustomGoalProcess {
    fn default() -> Self {
        CustomGoalProcess::new()
    }
}

impl CustomGoalProcess {
    /// `CustomGoalProcess.<init>`:
    /// `CustomGoalProcess.<init>`.
    pub fn new() -> CustomGoalProcess {
        CustomGoalProcess { goal: None, most_recent_goal: None, state: State::None }
    }

    /// `CustomGoalProcess.setGoal`:
    /// `CustomGoalProcess.setGoal`. (Upstream also forwards the goal to the elytra process when that is active; there is none here.)
    pub fn set_goal(&mut self, goal: Goal) {
        self.goal = Some(goal.clone());
        self.most_recent_goal = Some(goal);
        if self.state == State::None {
            self.state = State::GoalSet;
        }
        if self.state == State::Executing {
            self.state = State::PathRequested;
        }
    }

    /// `CustomGoalProcess.path`:
    /// `CustomGoalProcess.path`.
    pub fn path(&mut self) {
        self.state = State::PathRequested;
    }

    /// Upstream's `ICustomGoalProcess.setGoalAndPath`: `setGoal`, then `path`.
    pub fn set_goal_and_path(&mut self, goal: Goal) {
        self.set_goal(goal);
        self.path();
    }

    /// `CustomGoalProcess.getGoal`:
    /// `CustomGoalProcess.getGoal`.
    pub fn goal(&self) -> Option<&Goal> {
        self.goal.as_ref()
    }

    /// `CustomGoalProcess.mostRecentGoal`: not cleared by `on_lost_control`.
    pub fn most_recent_goal(&self) -> Option<&Goal> {
        self.most_recent_goal.as_ref()
    }

    /// `CustomGoalProcess.isActive`:
    /// `CustomGoalProcess.isActive`.
    pub fn is_active(&self) -> bool {
        self.state != State::None
    }

    /// The state, for tests and the glue's status line.
    pub fn state(&self) -> State {
        self.state
    }

    /// `CustomGoalProcess.onTick`:
    /// `CustomGoalProcess.onTick`. `feet` is the player's feet, `path_start` where a new path would start. The disconnect and
    /// notification settings are not here. Must not be called when the process is not active (upstream throws "Unexpected state").
    pub fn on_tick(&mut self, calc_failed: bool, feet: Pos, path_start: Pos, gs: &GoalSettings) -> PathingCommand {
        match self.state {
            State::GoalSet => PathingCommand { goal: self.goal.clone(), kind: PathingCommandType::CancelAndSetGoal },
            State::PathRequested => {
                // return FORCE_REVALIDATE_GOAL_AND_PATH just once
                let ret = PathingCommand { goal: self.goal.clone(), kind: PathingCommandType::ForceRevalidateGoalAndPath };
                self.state = State::Executing;
                ret
            }
            State::Executing => {
                if calc_failed {
                    self.on_lost_control();
                    // upstream builds the command after losing control, so its goal is null
                    return PathingCommand { goal: self.goal.clone(), kind: PathingCommandType::CancelAndSetGoal };
                }
                let arrived = match &self.goal {
                    None => true,
                    Some(g) => g.is_in_goal(feet.x, feet.y, feet.z, gs) && g.is_in_goal(path_start.x, path_start.y, path_start.z, gs),
                };
                if arrived {
                    self.on_lost_control(); // we're there xd
                    return PathingCommand { goal: self.goal.clone(), kind: PathingCommandType::CancelAndSetGoal };
                }
                PathingCommand { goal: self.goal.clone(), kind: PathingCommandType::SetGoalAndPath }
            }
            // not active: nothing to do. Upstream would throw; the manager never asks an inactive process.
            State::None => PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal },
        }
    }

    /// `CustomGoalProcess.onLostControl`:
    /// `CustomGoalProcess.onLostControl`.
    pub fn on_lost_control(&mut self) {
        self.state = State::None;
        self.goal = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gs() -> GoalSettings {
        GoalSettings { cost_heuristic: 3.563, axis_height: 120 }
    }

    #[test]
    fn a_goal_set_without_path_only_cancels_and_sets() {
        let mut p = CustomGoalProcess::new();
        assert!(!p.is_active());
        p.set_goal(Goal::Block { x: 5, y: 1, z: 5 });
        assert_eq!(p.state(), State::GoalSet);
        for _ in 0..3 {
            let c = p.on_tick(false, Pos::new(0, 1, 0), Pos::new(0, 1, 0), &gs());
            assert_eq!(c.kind, PathingCommandType::CancelAndSetGoal);
            assert_eq!(p.state(), State::GoalSet);
        }
    }

    #[test]
    fn path_requested_revalidates_once_then_sets_goal_and_path() {
        let mut p = CustomGoalProcess::new();
        p.set_goal_and_path(Goal::Block { x: 5, y: 1, z: 5 });
        let a = p.on_tick(false, Pos::new(0, 1, 0), Pos::new(0, 1, 0), &gs());
        assert_eq!(a.kind, PathingCommandType::ForceRevalidateGoalAndPath);
        let b = p.on_tick(false, Pos::new(0, 1, 0), Pos::new(0, 1, 0), &gs());
        assert_eq!(b.kind, PathingCommandType::SetGoalAndPath);
        assert_eq!(b.goal, Some(Goal::Block { x: 5, y: 1, z: 5 }));
    }

    #[test]
    fn arriving_loses_control_with_a_null_goal() {
        let mut p = CustomGoalProcess::new();
        p.set_goal_and_path(Goal::Block { x: 5, y: 1, z: 5 });
        p.on_tick(false, Pos::new(0, 1, 0), Pos::new(0, 1, 0), &gs());
        let c = p.on_tick(false, Pos::new(5, 1, 5), Pos::new(5, 1, 5), &gs());
        assert_eq!(c.kind, PathingCommandType::CancelAndSetGoal);
        assert_eq!(c.goal, None);
        assert!(!p.is_active());
        // the most recent goal stays
        assert_eq!(p.most_recent_goal(), Some(&Goal::Block { x: 5, y: 1, z: 5 }));
        // being in the goal at the feet but not at the path start is not arriving
        let mut p = CustomGoalProcess::new();
        p.set_goal_and_path(Goal::Block { x: 5, y: 1, z: 5 });
        p.on_tick(false, Pos::new(0, 1, 0), Pos::new(0, 1, 0), &gs());
        let c = p.on_tick(false, Pos::new(5, 1, 5), Pos::new(5, 0, 5), &gs());
        assert_eq!(c.kind, PathingCommandType::SetGoalAndPath);
    }

    #[test]
    fn a_failed_calculation_gives_up_with_a_null_goal() {
        let mut p = CustomGoalProcess::new();
        p.set_goal_and_path(Goal::Block { x: 5, y: 1, z: 5 });
        p.on_tick(false, Pos::new(0, 1, 0), Pos::new(0, 1, 0), &gs());
        let c = p.on_tick(true, Pos::new(0, 1, 0), Pos::new(0, 1, 0), &gs());
        assert_eq!((c.kind, c.goal), (PathingCommandType::CancelAndSetGoal, None));
        assert!(!p.is_active());
    }

    #[test]
    fn a_new_goal_while_executing_asks_for_a_new_path() {
        let mut p = CustomGoalProcess::new();
        p.set_goal_and_path(Goal::Block { x: 5, y: 1, z: 5 });
        p.on_tick(false, Pos::new(0, 1, 0), Pos::new(0, 1, 0), &gs());
        assert_eq!(p.state(), State::Executing);
        p.set_goal(Goal::Block { x: 9, y: 1, z: 9 });
        assert_eq!(p.state(), State::PathRequested);
    }
}
