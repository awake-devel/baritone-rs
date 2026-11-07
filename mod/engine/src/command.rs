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
}
