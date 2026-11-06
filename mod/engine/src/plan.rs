// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/behavior/PathingBehavior.java (createPathfinder, and the choices of
// findPathInNewThread that do not need a thread). LGPL-3.0-or-later, see lib.rs.
//
// Planning the next segment is a search that starts where the previous path ends and favors its positions. The executor, the thread
// that runs the search and the events are S4.
use crate::config::SearchSettings;
use crate::goals::Goal;
use crate::movement::helper::Ctx;
use crate::pos::Pos;
use crate::search::{AStarPathFinder, Avoidance, Favoring, Path};
use crate::world::WorldView;

/// The position of a goal that has one to draw, which is upstream's `IGoalRenderPos`: the goals with a single block as target.
pub fn goal_render_pos(goal: &Goal) -> Option<Pos> {
    match goal {
        Goal::Block { x, y, z } | Goal::TwoBlocks { x, y, z } | Goal::GetToBlock { x, y, z } | Goal::Near { x, y, z, .. } => Some(Pos::new(*x, *y, *z)),
        _ => None,
    }
}

/// The first part of `PathingBehavior.createPathfinder`: with `simplifyUnloadedYCoord` on, a goal that has a position, and whose column is not loaded, becomes a `GoalXZ`, because the height of a block that far away is unknown. Any other goal is returned as it is.
pub fn simplify_unloaded_y_coord<W: WorldView>(goal: &Goal, world: &W, s: &SearchSettings) -> Goal {
    if s.simplify_unloaded_y_coord {
        if let Some(pos) = goal_render_pos(goal) {
            if !world.contains_loaded_chunk(pos.x, pos.z) {
                return Goal::Xz { x: pos.x, z: pos.z };
            }
        }
    }
    goal.clone()
}

/// The `realStart` of `PathingBehavior.createPathfinder`: the player's feet when they are at the height of the start and within one block of it in x and z, else the start. The search starts from the start either way; `realStart` decides the first position of a path that has no movement.
pub fn real_start(start: Pos, feet: Pos) -> Pos {
    let sub = feet.subtract(start);
    if feet.y == start.y && sub.x.wrapping_abs() <= 1 && sub.z.wrapping_abs() <= 1 {
        return feet;
    }
    start
}

/// `PathingBehavior.createPathfinder`: the search from `start` to `goal`. `previous` is the path being followed, whose positions are favored, or none. `feet` is the player's position. `avoidances` come from `Avoidance::create`.
pub fn create_pathfinder<'c, 'a, W: WorldView>(start: Pos, goal: &Goal, previous: Option<&Path>, ctx: &'c Ctx<'a, W>, feet: Pos, avoidances: &[Avoidance]) -> AStarPathFinder<'c, 'a, W> {
    let s = &ctx.cfg.search;
    let transformed = simplify_unloaded_y_coord(goal, ctx.world, s);
    let favoring = Favoring::new(previous, s, avoidances);
    let real = real_start(start, feet);
    AStarPathFinder::new(real, start.x, start.y, start.z, transformed, favoring, ctx)
}

/// The search for the segment after `previous`: it starts at the end of the path being followed and favors its positions, as `findPathInNewThread` does with the current path. Use `timeouts(true, ..)` for its limits.
pub fn next_segment_search<'c, 'a, W: WorldView>(previous: &Path, goal: &Goal, ctx: &'c Ctx<'a, W>, feet: Pos, avoidances: &[Avoidance]) -> AStarPathFinder<'c, 'a, W> {
    create_pathfinder(previous.dest(), goal, Some(previous), ctx, feet, avoidances)
}

/// The timeouts `findPathInNewThread` picks: the plan-ahead ones while a path is being followed, else the primary and failure ones. Returns (primary, failure) in milliseconds.
pub fn timeouts(path_is_being_followed: bool, s: &SearchSettings) -> (i64, i64) {
    if path_is_being_followed {
        (s.plan_ahead_primary_timeout_ms, s.plan_ahead_failure_timeout_ms)
    } else {
        (s.primary_timeout_ms, s.failure_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{ResultType, SearchLimits};
    use crate::testing::*;

    #[test]
    fn real_start_is_the_feet_only_when_close_and_level() {
        let start = Pos::new(10, 5, 10);
        assert_eq!(real_start(start, Pos::new(10, 5, 10)), Pos::new(10, 5, 10));
        assert_eq!(real_start(start, Pos::new(11, 5, 9)), Pos::new(11, 5, 9));
        assert_eq!(real_start(start, Pos::new(9, 5, 11)), Pos::new(9, 5, 11));
        // two blocks away in x or z
        assert_eq!(real_start(start, Pos::new(12, 5, 10)), start);
        assert_eq!(real_start(start, Pos::new(10, 5, 8)), start);
        // another height
        assert_eq!(real_start(start, Pos::new(10, 6, 10)), start);
        assert_eq!(real_start(start, Pos::new(10, 4, 10)), start);
        // int wraps: the difference between the extremes is 1 in Java, so the feet are taken
        assert_eq!(real_start(Pos::new(i32::MAX, 5, 0), Pos::new(i32::MIN, 5, 0)), Pos::new(i32::MIN, 5, 0));
    }

    #[test]
    fn an_unloaded_goal_becomes_a_goal_xz() {
        let mut world = TestWorld::flat(40, 40);
        world.loaded_below_x = 16;
        let s = SearchSettings::default();
        let far = Goal::Block { x: 30, y: 70, z: 5 };
        assert_eq!(simplify_unloaded_y_coord(&far, &world, &s), Goal::Xz { x: 30, z: 5 });
        for g in [Goal::TwoBlocks { x: 30, y: 70, z: 5 }, Goal::GetToBlock { x: 30, y: 70, z: 5 }, crate::goals::goal_near(30, 70, 5, 3)] {
            assert_eq!(simplify_unloaded_y_coord(&g, &world, &s), Goal::Xz { x: 30, z: 5 });
        }
        // loaded: unchanged
        let near = Goal::Block { x: 3, y: 70, z: 5 };
        assert_eq!(simplify_unloaded_y_coord(&near, &world, &s), near);
        // goals without a position are unchanged
        let y = Goal::YLevel { level: 3 };
        assert_eq!(simplify_unloaded_y_coord(&y, &world, &s), y);
        assert_eq!(simplify_unloaded_y_coord(&Goal::Xz { x: 30, z: 5 }, &world, &s), Goal::Xz { x: 30, z: 5 });
        // the setting off: unchanged
        let off = SearchSettings { simplify_unloaded_y_coord: false, ..Default::default() };
        assert_eq!(simplify_unloaded_y_coord(&far, &world, &off), far);
    }

    #[test]
    fn timeouts_depend_on_whether_a_path_is_followed() {
        let s = SearchSettings::default();
        assert_eq!(timeouts(false, &s), (500, 2000));
        assert_eq!(timeouts(true, &s), (4000, 5000));
    }

    #[test]
    fn create_pathfinder_uses_the_feet_as_real_start() {
        let fx = Fixture::new();
        let world = TestWorld::flat(20, 20);
        let ctx = fx.ctx(&world);
        // the start is the goal; the feet are one block away, so the path has the feet as its first position
        let mut f = create_pathfinder(Pos::new(1, 1, 1), &Goal::Block { x: 1, y: 1, z: 1 }, None, &ctx, Pos::new(2, 1, 1), &[]);
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.kind, ResultType::SuccessToGoal);
        assert_eq!(r.path.unwrap().positions(), &[Pos::new(2, 1, 1), Pos::new(1, 1, 1)]);
        // feet two blocks away: the path is the start alone
        let mut f = create_pathfinder(Pos::new(1, 1, 1), &Goal::Block { x: 1, y: 1, z: 1 }, None, &ctx, Pos::new(3, 1, 1), &[]);
        let r = f.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(r.path.unwrap().positions(), &[Pos::new(1, 1, 1)]);
    }

    #[test]
    fn the_next_segment_starts_where_the_previous_ends_and_favors_it() {
        let fx = Fixture::new();
        let world = TestWorld::flat(60, 20);
        let ctx = fx.ctx(&world);
        let goal = Goal::Block { x: 50, y: 1, z: 1 };
        let mut f = create_pathfinder(Pos::new(1, 1, 1), &goal, None, &ctx, Pos::new(1, 1, 1), &[]);
        let first = f.calculate(500, 2000, &SearchLimits { max_nodes: Some(20), cancel: None }, &mut || 0);
        assert_eq!(first.kind, ResultType::SuccessSegment);
        let first = first.path.unwrap();
        let mut g = next_segment_search(&first, &goal, &ctx, Pos::new(1, 1, 1), &[]);
        assert_eq!(g.start(), first.dest());
        let second = g.calculate(500, 2000, &SearchLimits::default(), &mut || 0);
        assert_eq!(second.kind, ResultType::SuccessToGoal, "{:?}", second.error);
        let second = second.path.unwrap();
        assert_eq!(second.src(), first.dest());
        // the two join
        let both = crate::search::Path::try_splice(Some(&first), Some(&second), false).unwrap().unwrap();
        assert_eq!(both.dest(), Pos::new(50, 1, 1));
        assert_eq!(both.length(), 50);
    }
}
