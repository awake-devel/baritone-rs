// baritone-rs engine: the arguments of `goto`. Ported from cabaletta/baritone at 1b8e87c: RelativeCoordinate.java, RelativeGoal.java,
// RelativeGoalBlock.java, RelativeGoalXZ.java, RelativeGoalYLevel.java and the coordinate branch of GotoCommand.java.
// LGPL-3.0-or-later, see lib.rs.
//
// Upstream reads these from a command argument consumer that can peek, consume and tab complete. Here the arguments are a slice of
// strings and each datatype answers `Option`: upstream's `peekDatatypePostOrNull` is `None` when the datatype throws.
use crate::goals::Goal;
use crate::pos::Pos;

/// Minecraft's `Mth.floor(double)` is `(int) d`, one less when that rounded up. A Java `int` cast saturates, and so does `as`. The subtraction wraps.
fn mth_floor(d: f64) -> i32 {
    let i = d as i32;
    if d < f64::from(i) {
        i.wrapping_sub(1)
    } else {
        i
    }
}

/// The pattern of `RelativeCoordinate` is `^(~?)([+-]?(?:\d+(?:\.\d*)?|\.\d+)([kKmM]?)|)$`. Answers whether the number part (group 2) is
/// well formed, and what it is. The empty group is allowed and is 0.
fn number_part(text: &str) -> Option<&str> {
    if text.is_empty() {
        return Some(text);
    }
    let b = text.as_bytes();
    let mut i = 0;
    if matches!(b[i], b'+' | b'-') {
        i += 1;
    }
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let int_len = digits(i);
    if int_len > 0 {
        i += int_len;
        if i < b.len() && b[i] == b'.' {
            i += 1;
            i += digits(i);
        }
    } else if i < b.len() && b[i] == b'.' && digits(i + 1) > 0 {
        i += 1 + digits(i + 1);
    } else {
        return None;
    }
    if i < b.len() && matches!(b[i], b'k' | b'K' | b'm' | b'M') {
        i += 1;
    }
    (i == b.len()).then_some(text)
}

/// `RelativeCoordinate.apply`: a number, optionally with a `~` in front to make it relative to `origin`, and optionally scaled
/// by `k` (1000) or `m` (1000000). `None` where upstream throws `IllegalArgumentException`.
///
/// Upstream's scales are the letters `kKmM`; `group(2).toLowerCase().contains("k")` and `"m"`, both applied if both were there,
/// which the pattern does not allow.
pub fn relative_coordinate(arg: &str, origin: f64) -> Option<f64> {
    let (relative, rest) = match arg.strip_prefix('~') {
        Some(r) => (true, r),
        None => (false, arg),
    };
    let group2 = number_part(rest)?;
    let numeric: String = group2.chars().filter(|c| !matches!(c, 'k' | 'K' | 'm' | 'M')).collect();
    let mut offset = if group2.is_empty() { 0.0 } else { numeric.parse::<f64>().ok()? };
    let lower = group2.to_lowercase();
    if lower.contains('k') {
        offset *= 1000.0;
    }
    if lower.contains('m') {
        offset *= 1_000_000.0;
    }
    Some(if relative { origin + offset } else { offset })
}

/// `RelativeGoalBlock.apply`: three coordinates, from the first three arguments.
pub fn relative_goal_block(args: &[&str], origin: Pos) -> Option<Goal> {
    let x = relative_coordinate(args.first()?, f64::from(origin.x))?;
    let y = relative_coordinate(args.get(1)?, f64::from(origin.y))?;
    let z = relative_coordinate(args.get(2)?, f64::from(origin.z))?;
    Some(Goal::Block { x: mth_floor(x), y: mth_floor(y), z: mth_floor(z) })
}

/// `RelativeGoalXZ.apply`: two coordinates.
pub fn relative_goal_xz(args: &[&str], origin: Pos) -> Option<Goal> {
    let x = relative_coordinate(args.first()?, f64::from(origin.x))?;
    let z = relative_coordinate(args.get(1)?, f64::from(origin.z))?;
    Some(Goal::Xz { x: mth_floor(x), z: mth_floor(z) })
}

/// `RelativeGoalYLevel.apply`: one coordinate, relative to the origin's Y.
pub fn relative_goal_y_level(args: &[&str], origin: Pos) -> Option<Goal> {
    let y = relative_coordinate(args.first()?, f64::from(origin.y))?;
    Some(Goal::YLevel { level: mth_floor(y) })
}

/// `RelativeGoal.apply`: tries three coordinates, then two, then one, and is a block goal at the origin when none fits. Arguments
/// past the ones the goal took are left alone.
pub fn relative_goal(args: &[&str], origin: Pos) -> Goal {
    relative_goal_block(args, origin).or_else(|| relative_goal_xz(args, origin)).or_else(|| relative_goal_y_level(args, origin)).unwrap_or(Goal::Block { x: origin.x, y: origin.y, z: origin.z })
}

/// Why `goto` did not make a goal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GotoError {
    /// No argument. Upstream: `args.getDatatypeFor` throws not enough arguments.
    NoArguments,
    /// More than three arguments with a coordinate first: `requireMax(3)`.
    TooManyArguments,
    /// The first argument is not a coordinate. Upstream reads it as a block to find; that process is not ported.
    BlockName,
}

/// `GotoCommand.execute`: the coordinate branch. If the first argument is a coordinate, at most three arguments are allowed and the
/// goal is read from them relative to `feet`.
pub fn goto_goal(args: &[&str], feet: Pos) -> Result<Goal, GotoError> {
    let Some(first) = args.first() else {
        return Err(GotoError::NoArguments);
    };
    if relative_coordinate(first, 0.0).is_none() {
        return Err(GotoError::BlockName);
    }
    if args.len() > 3 {
        return Err(GotoError::TooManyArguments);
    }
    Ok(relative_goal(args, feet))
}

// ---------------------------------------------------------------- the other commands

/// What a command refuses with: upstream's `CommandException` text, without the position of the argument.
pub type CommandError = String;

/// `ArgConsumer.requireMax`.
fn require_max(args: &[&str], max: usize) -> Result<(), CommandError> {
    if args.len() > max {
        Err(format!("Too many arguments (expected at most {max})"))
    } else {
        Ok(())
    }
}

/// The text of a goal as upstream's `toString` writes it, for the three goals a command makes; `censor` is `censorCoordinates`.
/// The other goals are written with their `Debug` form, which is not upstream's.
pub fn goal_to_string(goal: &Goal, censor: bool) -> String {
    let c = |v: i32| if censor { "<censored>".to_string() } else { v.to_string() };
    match goal {
        Goal::Block { x, y, z } => format!("GoalBlock{{x={},y={},z={}}}", c(*x), c(*y), c(*z)),
        Goal::Xz { x, z } => format!("GoalXZ{{x={},z={}}}", c(*x), c(*z)),
        Goal::YLevel { level } => format!("GoalYLevel{{y={}}}", c(*level)),
        other => format!("{other:?}"),
    }
}

/// What `goal` did, for the glue to carry out.
#[derive(Clone, Debug, PartialEq)]
pub enum GoalAction {
    /// `goal clear`: the goal is erased (`ICustomGoalProcess.setGoal(null)`). `had_goal` is whether there was one.
    Clear { had_goal: bool },
    /// `goal <coordinates>`: the goal is set, without a path.
    Set(Goal),
}

impl GoalAction {
    /// The line `GoalCommand` prints.
    pub fn message(&self, censor: bool) -> String {
        match self {
            GoalAction::Clear { had_goal: true } => "Cleared goal".to_string(),
            GoalAction::Clear { had_goal: false } => "There was no goal to clear".to_string(),
            GoalAction::Set(g) => format!("Goal: {}", goal_to_string(g, censor)),
        }
    }
}

/// Upstream's `GoalCommand.execute`: `goal`, `goal reset`, `clear` or `none`, and `goal <y>`, `<x> <z>`, `<x> <y> <z>` with `~` for the player's position.
/// `current` is the goal the goal process holds now.
pub fn goal_command(args: &[&str], origin: Pos, current: Option<&Goal>) -> Result<GoalAction, CommandError> {
    if let Some(first) = args.first() {
        if matches!(*first, "reset" | "clear" | "none") {
            require_max(args, 1)?;
            return Ok(GoalAction::Clear { had_goal: current.is_some() });
        }
    }
    require_max(args, 3)?;
    Ok(GoalAction::Set(relative_goal(args, origin)))
}

/// What `path` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathAction {
    /// There is no goal: "No goal set".
    NoGoal,
    /// Head for the goal: "Now pathing".
    Path,
}

impl PathAction {
    pub fn message(self) -> &'static str {
        match self {
            PathAction::NoGoal => "No goal set",
            PathAction::Path => "Now pathing",
        }
    }
}

/// Upstream's `PathCommand.execute`: no arguments; with a goal, start heading for it. (Upstream also repacks the world scanner's cache first.)
pub fn path_command(args: &[&str], has_goal: bool) -> Result<PathAction, CommandError> {
    require_max(args, 0)?;
    Ok(if has_goal { PathAction::Path } else { PathAction::NoGoal })
}

const ETA_UNITS: [&str; 5] = ["seconds", "minutes", "hours", "days", "weeks"];
const ETA_TICKS_PER_UNIT: [i32; 5] = [20, 1200, 72000, 1728000, 12096000];

/// Java's `String.format("%.0f", x)`: the nearest whole number, halves away from zero.
fn java_format_0f(x: f64) -> String {
    let r = (x.abs() + 0.5).floor();
    let r = if x.is_sign_negative() && r != 0.0 { -r } else { r };
    format!("{r:.0}")
}

/// Upstream's `ETACommand.formatTime`: ticks as the largest unit of which there are at least two, with the ticks in brackets. We just assume tps is 20.
pub fn eta_format_time(ticks: f64) -> String {
    if !ticks.is_finite() {
        return crate::set::java_double_to_string(ticks); // NaN, +Infinity and -Infinity don't need units
    }
    for i in (0..ETA_UNITS.len()).rev() {
        let value = (ticks as i32) / ETA_TICKS_PER_UNIT[i];
        if value >= 2 {
            return format!("{} {} ({} ticks)", value, ETA_UNITS[i], java_format_0f(ticks));
        }
    }
    format!("{} ticks", java_format_0f(ticks))
}

/// Upstream's `ETACommand.execute`, from the numbers it asks `PathingBehavior` for: `segment` is `ticksRemainingInSegment` and `goal` is
/// `estimatedTicksToGoal`; `None` is upstream's `NaN`. `in_control` is whether a process is in control; without one the command refuses.
/// The answer is the lines the command prints: the two times in one message, and the plea when it will take weeks.
pub fn eta_command(args: &[&str], in_control: bool, segment: Option<f64>, goal: Option<f64>) -> Result<Vec<String>, CommandError> {
    require_max(args, 0)?;
    if !in_control {
        return Err("No process in control".to_string());
    }
    let ticks_remaining_in_segment = segment.unwrap_or(f64::NAN);
    let ticks_remaining_in_goal = goal.unwrap_or(f64::NAN);
    let mut lines = vec![format!("Next segment: {}\n        Goal: {}", eta_format_time(ticks_remaining_in_segment), eta_format_time(ticks_remaining_in_goal))];
    if ticks_remaining_in_goal.is_finite() && ticks_remaining_in_goal >= 24_192_000.0 {
        lines.push("Please don't do this to me.".to_string());
    }
    Ok(lines)
}

/// The commands of this port, with the names they answer to and the short description `HelpCommand` lists.
pub const COMMANDS: &[(&[&str], &str)] = &[(&["goal"], "Set or clear the goal"), (&["goto"], "Go to a coordinate or block"), (&["path"], "Start heading towards the goal"), (&["cancel", "stop"], "Cancel what Baritone is currently doing"), (&["eta"], "View the current ETA"), (&["set", "setting", "settings"], "View or change settings")];

/// Every name a command answers to, in the order of [`COMMANDS`].
pub fn command_names() -> Vec<&'static str> {
    COMMANDS.iter().flat_map(|(names, _)| names.iter().copied()).collect()
}

/// Completion of what is typed after `/b`: the words before the one being completed, and that word. The names of the commands, then what
/// each takes: `~` for a coordinate, `reset`, `clear` and `none` for `goal`, and the settings for `set`. Matching ignores case.
pub fn complete(before: &[&str], current: &str, settings: &crate::set::Settings) -> Vec<String> {
    let cur = current.to_lowercase();
    let starts = |s: &str| s.to_lowercase().starts_with(&cur);
    let Some((name, rest)) = before.split_first() else {
        let mut names: Vec<String> = command_names().iter().map(|s| s.to_string()).collect();
        names.push("help".to_string());
        names.retain(|n| starts(n));
        return names;
    };
    match name.to_lowercase().as_str() {
        "set" | "setting" | "settings" => settings.complete(rest, current),
        "goal" => {
            // upstream: with one argument typed, the words of `goal` and `~`; with up to three, a `~` after each coordinate that parses
            let mut v: Vec<String> = vec![];
            if rest.is_empty() {
                v.extend(["reset", "clear", "none", "~"].iter().map(|s| s.to_string()));
            } else if rest.len() < 3 && rest.iter().all(|a| relative_coordinate(a, 0.0).is_some()) {
                v.push("~".to_string());
            }
            v.retain(|s| starts(s));
            v
        }
        "goto" => {
            let mut v: Vec<String> = vec![];
            if rest.len() < 3 && rest.iter().all(|a| relative_coordinate(a, 0.0).is_some()) {
                v.push("~".to_string());
            }
            v.retain(|s| starts(s));
            v
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feet() -> Pos {
        Pos::new(100, 64, -20)
    }

    #[test]
    fn three_numbers_make_a_block_goal() {
        assert_eq!(goto_goal(&["10", "70", "-3"], feet()), Ok(Goal::Block { x: 10, y: 70, z: -3 }));
    }

    #[test]
    fn two_numbers_make_an_xz_goal() {
        assert_eq!(goto_goal(&["10", "-3"], feet()), Ok(Goal::Xz { x: 10, z: -3 }));
    }

    #[test]
    fn one_number_is_a_y_level() {
        assert_eq!(goto_goal(&["12"], feet()), Ok(Goal::YLevel { level: 12 }));
    }

    #[test]
    fn tilde_is_relative_to_the_feet() {
        assert_eq!(goto_goal(&["~", "~5", "~-2"], feet()), Ok(Goal::Block { x: 100, y: 69, z: -22 }));
        assert_eq!(goto_goal(&["~30", "~"], feet()), Ok(Goal::Xz { x: 130, z: -20 }));
        assert_eq!(goto_goal(&["~-4"], feet()), Ok(Goal::YLevel { level: 60 }));
    }

    #[test]
    fn decimals_are_floored_toward_minus_infinity() {
        assert_eq!(goto_goal(&["10.9", "-0.1"], feet()), Ok(Goal::Xz { x: 10, z: -1 }));
        assert_eq!(goto_goal(&["-3.5"], feet()), Ok(Goal::YLevel { level: -4 }));
        assert_eq!(goto_goal(&["5.", ".5"], feet()), Ok(Goal::Xz { x: 5, z: 0 }));
    }

    #[test]
    fn scales_k_and_m() {
        assert_eq!(goto_goal(&["1k", "2m"], feet()), Ok(Goal::Xz { x: 1000, z: 2_000_000 }));
        assert_eq!(goto_goal(&["1.5K"], feet()), Ok(Goal::YLevel { level: 1500 }));
        assert_eq!(relative_coordinate("~1k", 5.0), Some(1005.0));
    }

    #[test]
    fn a_plus_sign_is_allowed() {
        assert_eq!(relative_coordinate("+7", 0.0), Some(7.0));
    }

    #[test]
    fn the_pattern_is_strict() {
        for bad in ["abc", "1e5", "1x", "--1", "1.2.3", "~~1", "k", ".", "1km", "~a", "0x10"] {
            assert_eq!(relative_coordinate(bad, 0.0), None, "{bad}");
        }
    }

    #[test]
    fn only_the_first_three_are_read_but_four_are_refused() {
        assert_eq!(goto_goal(&["1", "2", "3", "4"], feet()), Err(GotoError::TooManyArguments));
    }

    #[test]
    fn a_second_argument_that_is_not_a_number_falls_back_to_one_coordinate() {
        // `RelativeGoal` tries three, two, then one; the extra argument is not read.
        assert_eq!(goto_goal(&["10", "x"], feet()), Ok(Goal::YLevel { level: 10 }));
        assert_eq!(goto_goal(&["10", "20", "x"], feet()), Ok(Goal::Xz { x: 10, z: 20 }));
    }

    #[test]
    fn a_block_name_and_no_arguments_are_errors() {
        assert_eq!(goto_goal(&["diamond_ore"], feet()), Err(GotoError::BlockName));
        assert_eq!(goto_goal(&[], feet()), Err(GotoError::NoArguments));
    }

    #[test]
    fn floor_saturates_like_java() {
        assert_eq!(mth_floor(3.0e10), i32::MAX);
        assert_eq!(mth_floor(-3.0e10), i32::MAX);
        assert_eq!(mth_floor(f64::NAN), 0);
    }

    // ------------------------------------------------------------ goal, path, eta, completion

    #[test]
    fn goal_sets_clears_and_prints_like_upstream() {
        let here = feet();
        assert_eq!(goal_command(&[], here, None), Ok(GoalAction::Set(Goal::Block { x: 100, y: 64, z: -20 })));
        assert_eq!(goal_command(&["1", "2", "3"], here, None), Ok(GoalAction::Set(Goal::Block { x: 1, y: 2, z: 3 })));
        assert_eq!(goal_command(&["~", "~5"], here, None), Ok(GoalAction::Set(Goal::Xz { x: 100, z: -15 })));
        assert_eq!(goal_command(&["7"], here, None), Ok(GoalAction::Set(Goal::YLevel { level: 7 })));
        assert_eq!(goal_command(&["1", "2", "3", "4"], here, None), Err("Too many arguments (expected at most 3)".to_string()));
        let g = Goal::Block { x: 1, y: 2, z: 3 };
        for word in ["reset", "clear", "none"] {
            assert_eq!(goal_command(&[word], here, Some(&g)), Ok(GoalAction::Clear { had_goal: true }));
        }
        assert_eq!(goal_command(&["clear"], here, None), Ok(GoalAction::Clear { had_goal: false }));
        assert_eq!(goal_command(&["clear", "x"], here, Some(&g)), Err("Too many arguments (expected at most 1)".to_string()));
        // the words are matched as typed, as upstream's `contains` does
        assert!(matches!(goal_command(&["CLEAR"], here, Some(&g)), Ok(GoalAction::Set(_))));
        // the text
        assert_eq!(GoalAction::Set(g.clone()).message(false), "Goal: GoalBlock{x=1,y=2,z=3}");
        assert_eq!(GoalAction::Set(Goal::Xz { x: -4, z: 9 }).message(false), "Goal: GoalXZ{x=-4,z=9}");
        assert_eq!(GoalAction::Set(Goal::YLevel { level: 70 }).message(false), "Goal: GoalYLevel{y=70}");
        assert_eq!(GoalAction::Set(g).message(true), "Goal: GoalBlock{x=<censored>,y=<censored>,z=<censored>}");
        assert_eq!(GoalAction::Clear { had_goal: true }.message(false), "Cleared goal");
        assert_eq!(GoalAction::Clear { had_goal: false }.message(false), "There was no goal to clear");
    }

    #[test]
    fn path_needs_a_goal_and_no_arguments() {
        assert_eq!(path_command(&[], false), Ok(PathAction::NoGoal));
        assert_eq!(path_command(&[], true), Ok(PathAction::Path));
        assert_eq!(PathAction::NoGoal.message(), "No goal set");
        assert_eq!(PathAction::Path.message(), "Now pathing");
        assert_eq!(path_command(&["x"], true), Err("Too many arguments (expected at most 0)".to_string()));
    }

    #[test]
    fn eta_times_are_written_in_the_largest_unit_with_at_least_two() {
        for (ticks, want) in [(0.0, "0 ticks"), (39.0, "39 ticks"), (40.0, "2 seconds (40 ticks)"), (2399.0, "119 seconds (2399 ticks)"), (2400.0, "2 minutes (2400 ticks)"), (144_000.0, "2 hours (144000 ticks)"), (3_456_000.0, "2 days (3456000 ticks)"), (24_192_000.0, "2 weeks (24192000 ticks)"), (59.5, "2 seconds (60 ticks)"), (0.5, "1 ticks"), (1.5, "2 ticks"), (2.5, "3 ticks")] {
            assert_eq!(eta_format_time(ticks), want, "{ticks}");
        }
        assert_eq!(eta_format_time(f64::NAN), "NaN");
        assert_eq!(eta_format_time(f64::INFINITY), "Infinity");
        assert_eq!(eta_format_time(f64::NEG_INFINITY), "-Infinity");
        // 40.4 ticks: two seconds, written with the ticks rounded
        assert_eq!(eta_format_time(40.4), "2 seconds (40 ticks)");
    }

    #[test]
    fn eta_prints_both_times_and_refuses_without_a_process() {
        let lines = eta_command(&[], true, Some(120.0), Some(5000.0)).unwrap();
        assert_eq!(lines, ["Next segment: 6 seconds (120 ticks)\n        Goal: 4 minutes (5000 ticks)"]);
        let lines = eta_command(&[], true, None, None).unwrap();
        assert_eq!(lines, ["Next segment: NaN\n        Goal: NaN"]);
        let lines = eta_command(&[], true, Some(1.0), Some(30_000_000.0)).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], "Please don't do this to me.");
        assert_eq!(eta_command(&[], false, None, None), Err("No process in control".to_string()));
        assert!(eta_command(&["x"], true, None, None).is_err());
    }

    #[test]
    fn completion_offers_the_commands_then_what_each_takes() {
        let s = crate::set::Settings::new();
        let all = complete(&[], "", &s);
        for name in ["goto", "goal", "path", "stop", "cancel", "eta", "set", "settings", "help"] {
            assert!(all.contains(&name.to_string()), "{name}");
        }
        assert_eq!(complete(&[], "go", &s), ["goal", "goto"]);
        assert_eq!(complete(&[], "ET", &s), ["eta"]);
        assert_eq!(complete(&["goal"], "", &s), ["reset", "clear", "none", "~"]);
        assert_eq!(complete(&["goal"], "c", &s), ["clear"]);
        assert_eq!(complete(&["goal", "10"], "", &s), ["~"]);
        assert!(complete(&["goal", "10", "20", "30"], "", &s).is_empty());
        assert!(complete(&["goal", "x"], "", &s).is_empty());
        assert_eq!(complete(&["goto"], "~", &s), ["~"]);
        assert_eq!(complete(&["goto", "~", "5"], "", &s), ["~"]);
        assert!(complete(&["goto", "1", "2", "3"], "", &s).is_empty());
        assert!(complete(&["path"], "", &s).is_empty());
        // set goes to the settings
        assert_eq!(complete(&["set"], "allowParkourA", &s), ["allowParkourAscend"]);
        assert_eq!(complete(&["settings", "allowBreak"], "", &s), ["true", "false"]);
        assert_eq!(complete(&["set", "toggle"], "allowDiagonalD", &s), ["allowDiagonalDescend"]);
        assert!(complete(&["nonsense"], "", &s).is_empty());
    }
}
