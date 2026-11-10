// baritone-rs engine: the arguments of `goto`. Ported from cabaletta/baritone at 1b8e87c: RelativeCoordinate.java, RelativeGoal.java,
// RelativeGoalBlock.java, RelativeGoalXZ.java, RelativeGoalYLevel.java and the coordinate branch of GotoCommand.java.
// LGPL-3.0-or-later, see lib.rs.
//
// Upstream reads these from a command argument consumer that can peek, consume and tab complete. Here the arguments are a slice of
// strings and each datatype answers `Option`: upstream's `peekDatatypePostOrNull` is `None` when the datatype throws.
use crate::blockmeta::{BlockIndex, BlockOptionalMeta, BlockOptionalMetaLookup};
use crate::goals::Goal;
use crate::mth::floor as mth_floor;
use crate::pos::Pos;
use crate::process::follow::{entity_string, FollowFilter};

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
    /// The first argument is not a coordinate. Upstream reads it as a block to find: the glue then calls [`goto_block`].
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

// ---------------------------------------------------------------- the block commands

/// `GotoCommand.execute`, the block branch: one argument, a block selector. `Err` is the text upstream's exception gives.
pub fn goto_block(args: &[&str], index: &BlockIndex) -> Result<BlockOptionalMeta, CommandError> {
    require_max(args, 1)?;
    let Some(first) = args.first() else {
        return Err("Not enough arguments (expected at least 1)".to_string());
    };
    BlockOptionalMeta::parse(first, index)
}

/// What `mine` asks for: how many drops to have in the inventory (0: no end), and the blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MineRequest {
    pub quantity: i32,
    pub filter: BlockOptionalMetaLookup,
    /// The selectors as `BlockOptionalMeta.toString` writes them, in a list: what `Mining %s` prints.
    pub text: String,
}

impl MineRequest {
    /// The line `MineCommand` prints before it starts: `Mining [BlockOptionalMeta{...}, ...]`.
    pub fn message(&self) -> String {
        format!("Mining {}", self.text)
    }
}

/// Upstream's `MineCommand.execute`: an optional whole number first (`Integer.class` with a default of 0), then at least one block selector.
pub fn mine_command(args: &[&str], index: &BlockIndex) -> Result<MineRequest, CommandError> {
    let (quantity, rest) = match args.first().and_then(|a| a.parse::<i32>().ok()) {
        Some(q) => (q, &args[1..]),
        None => (0, args),
    };
    if rest.is_empty() {
        return Err("Not enough arguments (expected at least 1)".to_string());
    }
    let mut boms = vec![];
    for a in rest {
        boms.push(BlockOptionalMeta::parse(a, index)?);
    }
    let text = format!("[{}]", boms.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", "));
    Ok(MineRequest { quantity, filter: BlockOptionalMetaLookup::new(boms), text })
}

/// The blocks to offer for the word being typed after `mine` or `goto`: the names that start with it (`minecraft:` may be left off). Upstream's
/// completion also offers the properties inside `[]`; that is left out.
pub fn complete_blocks(current: &str, index: &BlockIndex) -> Vec<String> {
    let cur = current.to_lowercase();
    let bare = cur.strip_prefix("minecraft:").unwrap_or(&cur);
    index.block_names().into_iter().filter(|n| n.to_lowercase().starts_with(bare)).map(|n| n.to_string()).collect()
}

// ---------------------------------------------------------------- follow

/// A player of the world, as `NearbyPlayer` finds them (`world().players()`): the player itself is one of them, though it is never followed.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerRef {
    /// The entity id. The player itself has none the glue knows: it gives `u32::MAX`, which prints as -1 and is never in the entity list.
    pub id: u32,
    pub name: String,
    pub local: bool,
    pub position: [f64; 3],
}

/// What `FollowCommand.execute` did. Upstream sets the filter and then prints, and `follow player` of nobody sets an empty filter and then throws
/// `NoEntitiesException`: so the filter and the error can both be there. `filter` is `None` when nothing was set.
#[derive(Clone, Debug, PartialEq)]
pub struct FollowOutcome {
    pub filter: Option<FollowFilter>,
    /// The `logDirect` lines.
    pub lines: Vec<String>,
    pub error: Option<CommandError>,
}

/// `ResourceLocation.tryParse`, as the id it makes (`minecraft:` when no namespace is given), or `None` when a character is not allowed.
fn parse_resource_location(text: &str) -> Option<String> {
    let (namespace, path) = match text.find(':') {
        Some(0) => ("minecraft", &text[1..]),
        Some(i) => (&text[..i], &text[i + 1..]),
        None => ("minecraft", text),
    };
    let ns_ok = namespace.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase() || matches!(c, '_' | '-' | '.'));
    let path_ok = path.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase() || matches!(c, '_' | '-' | '.' | '/'));
    (ns_ok && path_ok).then(|| format!("{namespace}:{path}"))
}

/// Upstream's `EntityClassById.get`: the entity type with that id, or `None` when the text is not an id or no type has it.
fn entity_type_by_id(text: &str, types: &[String]) -> Option<String> {
    let id = parse_resource_location(text)?;
    types.contains(&id).then_some(id)
}

/// Upstream's `NearbyPlayer.get`: the player with that name, ignoring case.
fn player_by_name<'a>(name: &str, players: &'a [PlayerRef]) -> Option<&'a PlayerRef> {
    let name = name.to_lowercase();
    players.iter().find(|p| p.name.to_lowercase() == name)
}

/// `CommandInvalidTypeException`'s text for argument `index` (0-based, counted from the first word after `follow`).
fn invalid_type(index: usize, expected: &str) -> CommandError {
    format!("Error at argument #{}: Expected {expected}", index + 1)
}

/// `FollowCommand.execute`. `players` are the players of the world and `entity_types` the registry ids of the entity types.
pub fn follow_command(args: &[&str], players: &[PlayerRef], entity_types: &[String]) -> FollowOutcome {
    let fail = |error: CommandError| FollowOutcome { filter: None, lines: vec![], error: Some(error) };
    if args.is_empty() {
        return fail("Not enough arguments (expected at least 1)".to_string());
    }
    if args.len() == 1 {
        let (filter, name) = match args[0].to_lowercase().as_str() {
            "entities" => (FollowFilter::Living, "entities"),
            "players" => (FollowFilter::Players, "players"),
            _ => return fail(invalid_type(0, "FollowGroup")),
        };
        return FollowOutcome { filter: Some(filter), lines: vec![format!("Following all {name}")], error: None };
    }
    let is_entity = match args[0].to_lowercase().as_str() {
        "entity" => true,
        "player" => false,
        _ => return fail(invalid_type(0, "FollowList")),
    };
    let mut classes: Vec<String> = vec![];
    let mut found: Vec<&PlayerRef> = vec![];
    for (i, word) in args.iter().enumerate().skip(1) {
        if is_entity {
            match entity_type_by_id(word, entity_types) {
                Some(t) => classes.push(t),
                // the argument is consumed when the datatype throws: the exception names the next one, if there is one
                None => return fail(invalid_type(if i + 1 < args.len() { i + 1 } else { i }, "EntityClassById")),
            }
        } else if let Some(p) = player_by_name(word, players) {
            found.push(p);
        }
    }
    if is_entity {
        let mut lines = vec!["Following these types of entities:".to_string()];
        lines.extend(classes.iter().cloned());
        return FollowOutcome { filter: Some(FollowFilter::Types(classes)), lines, error: None };
    }
    let filter = FollowFilter::Entities(found.iter().map(|p| p.id).collect());
    if found.is_empty() {
        return FollowOutcome { filter: Some(filter), lines: vec![], error: Some("No valid entities in range!".to_string()) };
    }
    let mut lines = vec!["Following these entities:".to_string()];
    lines.extend(found.iter().map(|p| entity_string(if p.local { "LocalPlayer" } else { "RemotePlayer" }, &p.name, p.id as i32, p.position)));
    FollowOutcome { filter: Some(filter), lines, error: None }
}

/// Upstream's `TabCompleteHelper.filterPrefixNamespaced`: the ids that start with the text, or whose path does when no namespace is typed; an explicit namespace is
/// matched as given. Text that is not an id gives nothing.
fn filter_prefix_namespaced(prefix: &str, ids: impl Iterator<Item = String>) -> Vec<String> {
    let Some(loc) = parse_resource_location(prefix) else {
        return vec![];
    };
    if prefix.contains(':') {
        let want = loc.to_lowercase();
        return ids.filter(|i| i.to_lowercase().starts_with(&want)).collect();
    }
    let search = prefix.to_lowercase();
    ids.filter(|id| {
        let c = id.to_lowercase();
        c.starts_with(&search) || c.split_once(':').is_some_and(|(_, path)| path.starts_with(&search))
    })
    .collect()
}

/// Upstream's `FollowCommand.tabComplete`: `before` are the words after `follow` that are done, `current` the one being typed. With one word: the groups and the
/// lists. Then the entity types, or the names of the players. A word that is done and names nothing gives no completion at all.
///
/// Upstream completes the types with `Object::toString` of each `EntityType`, which is its description id, not the id its filter wants (docs/deviations.md D40).
pub fn complete_follow(before: &[&str], current: &str, players: &[PlayerRef], entity_types: &[String]) -> Vec<String> {
    let cur = current.to_lowercase();
    let Some((list, done)) = before.split_first() else {
        return ["entities", "players", "entity", "player"].iter().filter(|w| w.starts_with(&cur)).map(|w| w.to_string()).collect();
    };
    let entity = match list.to_lowercase().as_str() {
        "entity" => true,
        "player" => false,
        _ => return vec![],
    };
    for word in done {
        let resolves = if entity { entity_type_by_id(word, entity_types).is_some() } else { player_by_name(word, players).is_some() };
        if !resolves {
            return vec![];
        }
    }
    if entity {
        let mut v = filter_prefix_namespaced(current, entity_types.iter().cloned());
        v.sort_by_key(|s| s.to_lowercase());
        v
    } else {
        let mut v: Vec<String> = players.iter().map(|p| p.name.clone()).filter(|n| n.to_lowercase().starts_with(&cur)).collect();
        v.sort_by_key(|s| s.to_lowercase());
        v
    }
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
        // the anonymous `GoalXZ` of the explore process prints as its superclass does
        Goal::Xz { x, z } | Goal::XzMaintainY { x, z, .. } => format!("GoalXZ{{x={},z={}}}", c(*x), c(*z)),
        Goal::YLevel { level } => format!("GoalYLevel{{y={}}}", c(*level)),
        // `BuilderProcess.JankyGoalComposite.toString`
        Goal::Janky { primary, fallback } => format!("JankyComposite Primary: {} Fallback: {}", goal_to_string(primary, censor), goal_to_string(fallback, censor)),
        // `BuilderProcess.GoalBreak.toString`, `GoalAdjacent.toString`, `GoalPlace.toString`
        Goal::Break { x, y, z } => format!("GoalBreak{{x={},y={},z={}}}", c(*x), c(*y), c(*z)),
        Goal::Adjacent { x, y, z, .. } => format!("GoalAdjacent{{x={},y={},z={}}}", c(*x), c(*y), c(*z)),
        Goal::Place { x, y, z } => format!("GoalPlace{{x={},y={},z={}}}", c(*x), c(*y), c(*z)),
        // the anonymous `GoalGetToBlock` of `breakGoal` prints as its superclass does
        Goal::BreakFromAbove { x, y, z } => format!("GoalGetToBlock{{x={},y={},z={}}}", c(*x), c(*y), c(*z)),
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

/// What `explore` asks for: the block column to explore around, and the line it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExploreRequest {
    pub x: i32,
    pub z: i32,
}

impl ExploreRequest {
    /// `ExploreCommand`'s `Exploring from GoalXZ{...}`; `censor` is `censorCoordinates`.
    pub fn message(&self, censor: bool) -> String {
        format!("Exploring from {}", goal_to_string(&Goal::Xz { x: self.x, z: self.z }, censor))
    }
}

/// Upstream's `ExploreCommand.execute`: no arguments explores from the player's column; two (`RelativeGoalXZ`, `~` for the player's) explore from there.
/// Any other number of arguments is refused as `requireExactly(2)` does.
pub fn explore_command(args: &[&str], feet: Pos) -> Result<ExploreRequest, CommandError> {
    if args.is_empty() {
        return Ok(ExploreRequest { x: feet.x, z: feet.z });
    }
    if args.len() < 2 {
        return Err("Not enough arguments (expected at least 2)".to_string());
    }
    require_max(args, 2)?;
    // `RelativeGoalXZ` fails on the first coordinate that is not one. That argument is consumed by then, and `getDatatypePost` names the
    // next argument if there is one (`hasAny() ? peek() : consumed()`), else the failing one (D50)
    for (i, a) in args.iter().enumerate() {
        if relative_coordinate(a, 0.0).is_none() {
            return Err(invalid_type(if i + 1 < args.len() { i + 1 } else { i }, "RelativeGoalXZ"));
        }
    }
    match relative_goal_xz(args, feet) {
        Some(Goal::Xz { x, z }) => Ok(ExploreRequest { x, z }),
        _ => Err(invalid_type(0, "RelativeGoalXZ")),
    }
}

/// Upstream's `ExploreFilterCommand.execute`, up to the reading of the file: the path as typed and whether the list is inverted. The word after the path must be
/// `invert` (any case) or nothing.
pub fn explore_filter_command(args: &[&str]) -> Result<(String, bool), CommandError> {
    require_max(args, 2)?;
    let Some(path) = args.first() else {
        return Err("Not enough arguments (expected at least 1)".to_string());
    };
    let invert = match args.get(1) {
        None => false,
        Some(w) if w.eq_ignore_ascii_case("invert") => true,
        Some(_) => return Err(invalid_type(1, "either \"invert\" or nothing")),
    };
    Ok(((*path).to_string(), invert))
}

// ---------------------------------------------------------------- the builder commands

/// What `build` asks for: the file as typed (`RelativeFile` resolves it against the schematics folder, which is the glue's) and the origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildRequest {
    pub file: String,
    pub origin: Pos,
}

/// `BuildCommand.execute`: the arguments.
/// `RelativeBlockPos.apply`: three coordinates, `~` for the feet.
/// `BuildCommand.execute`, the arguments: a file, then nothing or three coordinates (`RelativeBlockPos`, `~` for the feet, floored as `Mth.floor` does). Upstream checks the
/// file before it reads the coordinates, so that `build nosuch x` says "Cannot find" first; here the coordinates are read first (docs/deviations.md D48).
pub fn build_command(args: &[&str], feet: Pos) -> Result<BuildRequest, CommandError> {
    let Some(file) = args.first() else {
        return Err("Not enough arguments (expected at least 1)".to_string());
    };
    let coords = &args[1..];
    let origin = if coords.is_empty() {
        feet
    } else {
        require_max(coords, 3)?;
        let mut v = [0i32; 3];
        let base = [feet.x, feet.y, feet.z];
        for (j, slot) in v.iter_mut().enumerate() {
            let arg = coords.get(j).ok_or_else(|| invalid_type(coords.len(), "RelativeBlockPos"))?;
            // the failing coordinate is argument `j + 1` (the file is 0); `RelativeCoordinate` has consumed it, and `getDatatypePost` names the
            // next argument if there is one (`hasAny() ? peek() : consumed()`), else the failing one (D50)
            let c = relative_coordinate(arg, f64::from(base[j])).ok_or_else(|| invalid_type(if j + 2 < args.len() { j + 2 } else { j + 1 }, "RelativeBlockPos"))?;
            *slot = mth_floor(c);
        }
        Pos::new(v[0], v[1], v[2])
    };
    Ok(BuildRequest { file: (*file).to_string(), origin })
}

/// `ExecutionControlCommands`' `resume`, the builder half: `requireMax(0)`, then "Not paused" unless the builder is paused. `Ok` is the line it prints.
pub fn resume_command(args: &[&str], builder_paused: bool) -> Result<&'static str, CommandError> {
    require_max(args, 0)?;
    if !builder_paused {
        return Err("Not paused".to_string());
    }
    Ok("Resumed")
}

/// `java.nio.file.Path.relativize` for two absolute, normalised paths: the way from `base` to `target`.
fn relativize(base: &std::path::Path, target: &std::path::Path) -> std::path::PathBuf {
    let b: Vec<_> = base.components().collect();
    let t: Vec<_> = target.components().collect();
    let common = b.iter().zip(&t).take_while(|(x, y)| x == y).count();
    let mut out = std::path::PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c.as_os_str());
    }
    out
}

/// A folder as the glue lists it: the canonical folder, and its entries as (name, is a folder).
pub type Listing = (std::path::PathBuf, Vec<(String, bool)>);

/// `BuildCommand.tabComplete`: the file names for the first word.
/// `RelativeFile.tabComplete(IArgConsumer,File)`: the files of the folder the typed text points into, as the text would be typed (relative to `base`, or whole when it is an
/// absolute path), with a separator after folders, kept when they start with the text (ignoring case) and hold no space. `base` is the canonical schematics
/// folder. `list_dir` is the glue's: given a folder it gives the canonical folder and its entries (name, is a folder), or `None` when it cannot be listed.
pub fn complete_relative_file(current: &str, base: &std::path::Path, list_dir: &dyn Fn(&std::path::Path) -> Option<Listing>) -> Vec<String> {
    use std::path::{Path, MAIN_SEPARATOR};
    let current_path = Path::new(current);
    let absolute = current_path.is_absolute();
    let use_parent = !current.is_empty() && !current.ends_with(MAIN_SEPARATOR);
    // `components` drops a trailing separator, as `new File(base, text)` does
    let current_file: std::path::PathBuf = if absolute { current_path.components().collect() } else { base.join(current).components().collect() };
    let dir = if use_parent { current_file.parent().map(Path::to_path_buf).unwrap_or(current_file) } else { current_file };
    let Some((dir, entries)) = list_dir(&dir) else {
        return vec![];
    };
    let cur = current.to_lowercase();
    entries
        .into_iter()
        .map(|(name, is_dir)| {
            let full = dir.join(&name);
            let shown = if absolute { full.to_string_lossy().into_owned() } else { relativize(base, &full).to_string_lossy().into_owned() };
            if is_dir {
                format!("{shown}{MAIN_SEPARATOR}")
            } else {
                shown
            }
        })
        .filter(|s| s.to_lowercase().starts_with(&cur))
        .filter(|s| !s.contains(' '))
        .collect()
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
pub const COMMANDS: &[(&[&str], &str)] = &[
    (&["goal"], "Set or clear the goal"),
    (&["goto"], "Go to a coordinate or block"),
    (&["mine"], "Mine some blocks"),
    (&["follow"], "Follow entity things"),
    (&["explore"], "Explore things"),
    (&["explorefilter"], "Explore chunks from a json"),
    (&["build"], "Build a schematic"),
    (&["resume", "r", "unpause", "unpaws"], "Resumes Baritone after a pause"),
    (&["path"], "Start heading towards the goal"),
    (&["cancel", "stop"], "Cancel what Baritone is currently doing"),
    (&["eta"], "View the current ETA"),
    (&["set", "setting", "settings"], "View or change settings"),
];

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
        "build" => {
            // upstream: the file name for the first word (the glue lists the files), then `~` for each coordinate that follows a coordinate, up to three
            let mut v: Vec<String> = vec![];
            if (1..=3).contains(&rest.len()) && rest[1..].iter().all(|a| relative_coordinate(a, 0.0).is_some()) {
                v.push("~".to_string());
            }
            v.retain(|s| starts(s));
            v
        }
        "explorefilter" => {
            // upstream completes a file name for the first word (the glue does not list files) and nothing for the second
            let mut v: Vec<String> = vec![];
            if rest.len() == 1 {
                v.push("invert".to_string());
            }
            v.retain(|s| starts(s));
            v
        }
        "explore" => {
            // upstream: with at most two words typed, a `~` for the coordinate being typed
            let mut v: Vec<String> = vec![];
            if rest.len() < 2 && rest.iter().all(|a| relative_coordinate(a, 0.0).is_some()) {
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

    #[test]
    fn mine_takes_an_optional_quantity_and_block_selectors() {
        let idx = crate::blockmeta::test_index();
        let r = mine_command(&["diamond_ore", "deepslate_diamond_ore"], &idx).unwrap();
        assert_eq!(r.quantity, 0);
        assert_eq!(r.filter.blocks().len(), 2);
        assert_eq!(r.message(), "Mining [BlockOptionalMeta{block=Block{minecraft:diamond_ore},properties={}}, BlockOptionalMeta{block=Block{minecraft:deepslate_diamond_ore},properties={}}]");
        let r = mine_command(&["64", "oak_log[axis=y]"], &idx).unwrap();
        assert_eq!(r.quantity, 64);
        assert_eq!(r.filter.blocks()[0].all_block_states(), &[5]);
        assert_eq!(mine_command(&[], &idx).unwrap_err(), "Not enough arguments (expected at least 1)");
        assert_eq!(mine_command(&["5"], &idx).unwrap_err(), "Not enough arguments (expected at least 1)");
        assert_eq!(mine_command(&["stone", "nothing"], &idx).unwrap_err(), "Invalid block name nothing");
        // a number later on is a block name, which does not exist
        assert!(mine_command(&["stone", "5"], &idx).is_err());
        assert_eq!(mine_command(&["-3", "stone"], &idx).unwrap().quantity, -3);
    }

    #[test]
    fn goto_a_block_takes_one_selector() {
        let idx = crate::blockmeta::test_index();
        assert_eq!(goto_block(&["diamond_ore"], &idx).unwrap().block(), "diamond_ore");
        assert_eq!(goto_block(&["diamond_ore", "stone"], &idx).unwrap_err(), "Too many arguments (expected at most 1)");
        assert!(goto_block(&["nothing"], &idx).is_err());
        assert_eq!(goto_block(&[], &idx).unwrap_err(), "Not enough arguments (expected at least 1)");
    }

    #[test]
    fn blocks_complete_by_prefix() {
        let idx = crate::blockmeta::test_index();
        assert_eq!(complete_blocks("diam", &idx), vec!["diamond_ore"]);
        assert_eq!(complete_blocks("minecraft:oak", &idx), vec!["oak_log"]);
        assert!(complete_blocks("zzz", &idx).is_empty());
        assert!(complete_blocks("", &idx).len() >= 5);
    }

    fn players() -> Vec<PlayerRef> {
        vec![PlayerRef { id: 11, name: "Alice".to_string(), local: false, position: [1.0, 64.0, 2.0] }, PlayerRef { id: 12, name: "bob_2".to_string(), local: false, position: [0.0, 70.5, -3.0] }, PlayerRef { id: u32::MAX, name: "Me".to_string(), local: true, position: [5.0, 64.0, 5.0] }]
    }

    fn types() -> Vec<String> {
        ["minecraft:pig", "minecraft:cow", "minecraft:player", "minecraft:zombie", "minecraft:zombie_villager", "mymod:pig"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn follow_groups_set_a_filter_and_say_so() {
        let o = follow_command(&["entities"], &players(), &types());
        assert_eq!((o.filter, o.lines, o.error), (Some(FollowFilter::Living), vec!["Following all entities".to_string()], None));
        let o = follow_command(&["PLAYERS"], &players(), &types());
        assert_eq!((o.filter, o.lines), (Some(FollowFilter::Players), vec!["Following all players".to_string()]));
    }

    #[test]
    fn follow_entity_takes_types_by_id_with_or_without_the_namespace() {
        let o = follow_command(&["entity", "pig", "minecraft:zombie", "mymod:pig"], &players(), &types());
        let want = vec!["minecraft:pig".to_string(), "minecraft:zombie".to_string(), "mymod:pig".to_string()];
        assert_eq!(o.filter, Some(FollowFilter::Types(want.clone())));
        assert_eq!(o.lines[0], "Following these types of entities:");
        assert_eq!(o.lines[1..], want[..]);
        assert_eq!(o.error, None);
    }

    #[test]
    fn follow_player_finds_names_ignoring_case_and_complains_about_nobody() {
        let o = follow_command(&["player", "alice", "ME", "nobody"], &players(), &types());
        assert_eq!(o.filter, Some(FollowFilter::Entities(vec![11, u32::MAX])));
        assert_eq!(o.lines[0], "Following these entities:");
        assert_eq!(o.lines[1], "RemotePlayer['Alice'/11, l='ClientLevel', x=1.00, y=64.00, z=2.00]");
        assert_eq!(o.lines[2], "LocalPlayer['Me'/-1, l='ClientLevel', x=5.00, y=64.00, z=5.00]", "the player's own id is not known here: -1");
        assert_eq!(o.error, None);
        // upstream sets the filter first and throws after: an empty filter is in place with the error
        let o = follow_command(&["player", "nobody"], &players(), &types());
        assert_eq!(o.filter, Some(FollowFilter::Entities(vec![])));
        assert_eq!(o.error.as_deref(), Some("No valid entities in range!"));
    }

    #[test]
    fn follow_errors_are_upstream_s_texts() {
        let f = |a: &[&str]| follow_command(a, &players(), &types());
        assert_eq!(f(&[]).error.as_deref(), Some("Not enough arguments (expected at least 1)"));
        assert_eq!(f(&["pig"]).error.as_deref(), Some("Error at argument #1: Expected FollowGroup"));
        assert_eq!(f(&["entity"]).error.as_deref(), Some("Error at argument #1: Expected FollowGroup"));
        assert_eq!(f(&["pigs", "cow"]).error.as_deref(), Some("Error at argument #1: Expected FollowList"));
        // the argument that fails is consumed, so the exception names the next one when there is one
        assert_eq!(f(&["entity", "nothing"]).error.as_deref(), Some("Error at argument #2: Expected EntityClassById"));
        assert_eq!(f(&["entity", "nothing", "pig"]).error.as_deref(), Some("Error at argument #3: Expected EntityClassById"));
        assert_eq!(f(&["entity", "pig", "Pig"]).error.as_deref(), Some("Error at argument #3: Expected EntityClassById"));
        assert_eq!(f(&["entity", "a:b:c", "pig"]).error.as_deref(), Some("Error at argument #3: Expected EntityClassById"));
        assert_eq!(f(&["entity", "nothing"]).filter, None, "a failed command sets nothing");
    }

    #[test]
    fn follow_completes_the_groups_the_types_and_the_players() {
        let (p, t) = (players(), types());
        assert_eq!(complete_follow(&[], "", &p, &t), ["entities", "players", "entity", "player"]);
        assert_eq!(complete_follow(&[], "PL", &p, &t), ["players", "player"]);
        assert_eq!(complete_follow(&[], "entity", &p, &t), ["entity"]);
        // types: by id, or by the path when no namespace is typed; sorted
        assert_eq!(complete_follow(&["entity"], "zomb", &p, &t), ["minecraft:zombie", "minecraft:zombie_villager"]);
        assert_eq!(complete_follow(&["ENTITY"], "pi", &p, &t), ["minecraft:pig", "mymod:pig"]);
        assert_eq!(complete_follow(&["entity"], "minecraft:p", &p, &t), ["minecraft:pig", "minecraft:player"]);
        assert_eq!(complete_follow(&["entity"], "mymod:", &p, &t), ["mymod:pig"]);
        assert_eq!(complete_follow(&["entity"], "Bad Id", &p, &t), Vec::<String>::new());
        // players: names, the player itself included
        assert_eq!(complete_follow(&["player"], "", &p, &t), ["Alice", "bob_2", "Me"]);
        assert_eq!(complete_follow(&["player"], "B", &p, &t), ["bob_2"]);
        // earlier words must resolve, or nothing is offered
        assert_eq!(complete_follow(&["player", "alice"], "b", &p, &t), ["bob_2"]);
        assert!(complete_follow(&["player", "nobody"], "", &p, &t).is_empty());
        assert!(complete_follow(&["entity", "pig"], "co", &p, &t).contains(&"minecraft:cow".to_string()));
        assert!(complete_follow(&["entity", "nothing"], "co", &p, &t).is_empty());
        assert!(complete_follow(&["entities"], "", &p, &t).is_empty(), "a group takes no more words");
    }

    #[test]
    fn explore_takes_no_arguments_or_two_coordinates() {
        assert_eq!(explore_command(&[], feet()), Ok(ExploreRequest { x: 100, z: -20 }));
        assert_eq!(explore_command(&["10", "-3"], feet()), Ok(ExploreRequest { x: 10, z: -3 }));
        assert_eq!(explore_command(&["~30", "~"], feet()), Ok(ExploreRequest { x: 130, z: -20 }));
        assert_eq!(explore_command(&["10.9", "-0.1"], feet()), Ok(ExploreRequest { x: 10, z: -1 }), "floored, as GoalXZ's coordinates are");
        assert_eq!(explore_command(&["1k", "2m"], feet()), Ok(ExploreRequest { x: 1000, z: 2_000_000 }));
        assert_eq!(explore_command(&["10"], feet()), Err("Not enough arguments (expected at least 2)".to_string()));
        assert_eq!(explore_command(&["1", "2", "3"], feet()), Err("Too many arguments (expected at most 2)".to_string()));
        assert_eq!(explore_command(&["1", "x"], feet()), Err("Error at argument #2: Expected RelativeGoalXZ".to_string()));
        assert_eq!(explore_command(&["x", "1"], feet()), Err("Error at argument #2: Expected RelativeGoalXZ".to_string()));
        assert_eq!(ExploreRequest { x: 5, z: -7 }.message(false), "Exploring from GoalXZ{x=5,z=-7}");
        assert_eq!(ExploreRequest { x: 5, z: -7 }.message(true), "Exploring from GoalXZ{x=<censored>,z=<censored>}");
    }

    #[test]
    fn explorefilter_takes_a_path_and_optionally_invert() {
        assert_eq!(explore_filter_command(&["chunks.json"]), Ok(("chunks.json".to_string(), false)));
        assert_eq!(explore_filter_command(&["chunks.json", "invert"]), Ok(("chunks.json".to_string(), true)));
        assert_eq!(explore_filter_command(&["chunks.json", "INVERT"]), Ok(("chunks.json".to_string(), true)));
        assert_eq!(explore_filter_command(&[]), Err("Not enough arguments (expected at least 1)".to_string()));
        assert_eq!(explore_filter_command(&["a", "b"]), Err("Error at argument #2: Expected either \"invert\" or nothing".to_string()));
        assert_eq!(explore_filter_command(&["a", "invert", "x"]), Err("Too many arguments (expected at most 2)".to_string()));
    }

    #[test]
    fn explore_commands_complete_and_are_listed() {
        let s = crate::set::Settings::new();
        assert_eq!(complete(&["explore"], "", &s), ["~"]);
        assert_eq!(complete(&["explore", "5"], "", &s), ["~"]);
        assert!(complete(&["explore", "5", "6"], "", &s).is_empty());
        assert_eq!(complete(&["explorefilter", "f.json"], "", &s), ["invert"]);
        assert_eq!(complete(&["explorefilter", "f.json"], "in", &s), ["invert"]);
        assert!(complete(&["explorefilter"], "", &s).is_empty());
        let names = command_names();
        assert!(names.contains(&"explore") && names.contains(&"explorefilter"));
        assert!(complete(&[], "explore", &s).contains(&"explorefilter".to_string()));
    }

    // ------------------------------------------------------------ build and resume

    #[test]
    fn build_takes_a_file_and_the_feet_or_three_coordinates() {
        assert_eq!(build_command(&["box"], feet()), Ok(BuildRequest { file: "box".to_string(), origin: feet() }));
        assert_eq!(build_command(&["sub/box.schem", "1", "2", "3"], feet()), Ok(BuildRequest { file: "sub/box.schem".to_string(), origin: Pos::new(1, 2, 3) }));
        assert_eq!(build_command(&["box", "~", "~5", "~-2"], feet()).map(|r| r.origin), Ok(Pos::new(100, 69, -22)));
        assert_eq!(build_command(&["box", "10.9", "-0.1", "1k"], feet()).map(|r| r.origin), Ok(Pos::new(10, -1, 1000)));
    }

    #[test]
    fn build_refuses_what_upstream_refuses() {
        assert_eq!(build_command(&[], feet()), Err("Not enough arguments (expected at least 1)".to_string()));
        assert_eq!(build_command(&["box", "1", "2", "3", "4"], feet()), Err("Too many arguments (expected at most 3)".to_string()));
        // a bad middle coordinate is consumed, and the error names the next argument (`hasAny() ? peek() : consumed()`, D50)
        assert_eq!(build_command(&["box", "1", "x", "3"], feet()), Err("Error at argument #4: Expected RelativeBlockPos".to_string()));
        assert_eq!(build_command(&["box", "x", "2", "3"], feet()), Err("Error at argument #3: Expected RelativeBlockPos".to_string()));
        // a bad last coordinate has no next argument: it names itself
        assert_eq!(build_command(&["box", "1", "2", "x"], feet()), Err("Error at argument #4: Expected RelativeBlockPos".to_string()));
        // one or two coordinates are not a block position
        assert_eq!(build_command(&["box", "1"], feet()), Err("Error at argument #2: Expected RelativeBlockPos".to_string()));
        assert_eq!(build_command(&["box", "1", "2"], feet()), Err("Error at argument #3: Expected RelativeBlockPos".to_string()));
    }

    #[test]
    fn resume_needs_a_paused_builder() {
        assert_eq!(resume_command(&[], true), Ok("Resumed"));
        assert_eq!(resume_command(&[], false), Err("Not paused".to_string()));
        assert_eq!(resume_command(&["x"], true), Err("Too many arguments (expected at most 0)".to_string()));
    }

    #[test]
    fn the_build_and_resume_commands_are_listed_and_completed() {
        let names = command_names();
        for n in ["build", "resume", "r", "unpause", "unpaws"] {
            assert!(names.contains(&n), "{n}");
        }
        let s = crate::set::Settings::new();
        assert_eq!(complete(&["build", "box"], "", &s), vec!["~".to_string()]);
        assert_eq!(complete(&["build", "box", "1", "2"], "", &s), vec!["~".to_string()]);
        assert!(complete(&["build", "box", "1", "2", "3"], "", &s).is_empty());
        assert!(complete(&["build", "box", "x"], "", &s).is_empty());
        assert!(complete(&["build"], "", &s).is_empty(), "the glue lists the files");
        assert!(complete(&[], "bu", &s).contains(&"build".to_string()));
        assert!(complete(&[], "res", &s).contains(&"resume".to_string()));
    }

    fn listing(dir: &std::path::Path) -> Option<Listing> {
        // what the glue's canonicalisation does to the folder
        let mut norm = std::path::PathBuf::new();
        for c in dir.components() {
            match c {
                std::path::Component::ParentDir => {
                    norm.pop();
                }
                c => norm.push(c.as_os_str()),
            }
        }
        match norm.to_str()? {
            "/s" => Some(("/s".into(), vec![("Box.schem".into(), false), ("House".into(), true), ("my plan.schem".into(), false), ("a.litematic".into(), false)])),
            "/s/House" => Some(("/s/House".into(), vec![("wall.schem".into(), false)])),
            "/other" => Some(("/other".into(), vec![("x.schem".into(), false), ("Sub".into(), true)])),
            _ => None,
        }
    }

    #[test]
    fn file_completion_matches_relative_file_tab_complete() {
        if !cfg!(unix) {
            return;
        }
        let base = std::path::Path::new("/s");
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        // nothing typed: everything in the folder, folders with a separator, names with a space dropped
        assert_eq!(sorted(complete_relative_file("", base, &listing)), vec!["Box.schem", "House/", "a.litematic"]);
        // a prefix, in any case, lists the parent
        assert_eq!(sorted(complete_relative_file("b", base, &listing)), vec!["Box.schem"]);
        assert_eq!(sorted(complete_relative_file("H", base, &listing)), vec!["House/"]);
        // into a folder
        assert_eq!(complete_relative_file("House/", base, &listing), vec!["House/wall.schem"]);
        assert_eq!(complete_relative_file("House/w", base, &listing), vec!["House/wall.schem"]);
        // up and out: the way back is written with ..
        assert_eq!(sorted(complete_relative_file("../other/", base, &listing)), vec!["../other/Sub/", "../other/x.schem"]);
        // an absolute path is listed and shown whole
        assert_eq!(sorted(complete_relative_file("/other/", base, &listing)), vec!["/other/Sub/", "/other/x.schem"]);
        // a folder that cannot be listed gives nothing
        assert!(complete_relative_file("nowhere/", base, &listing).is_empty());
    }
}
