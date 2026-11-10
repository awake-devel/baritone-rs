// baritone-rs engine: the settings by name, and the `set` command. Settings.java and the parts of SettingsUtil.java and SetCommand.java at
// cabaletta/baritone 1b8e87c that the settings of this port need, in new structure. LGPL-3.0-or-later, see lib.rs.
//
// Upstream has one object with a field per setting, read from anywhere. Here every setting has its upstream name, type and default in one
// table (`Settings`), the glue changes values by name (`set`, a file) and turns the table into the snapshots the engine reads: `Config`
// (the search and the costs), `ExecSettings` (the executor) and `ToolSettings` (the hotbar). Nothing in a tick reads this table.
//
// Names are matched without regard to case, as `byLowerName` does. Values are parsed and printed as `SettingsUtil` does, so a line of
// upstream's `settings.txt` reads the same here.
//
// Left out on purpose (docs/deviations.md D29): the settings of the processes that do not exist here (the builder's are in, D49; `buildOnlySelection`
// is not: there are no selections), and the ones that are the game's (the chat prefix, notifications, `censorCoordinates` is kept for the goal text). `freeLook` and its four
// companions (`look_settings`) are read by the glue through `freelook.rs`; `renderPath` is stored and printed but read by nobody
// in the engine (it belongs to the glue). `allowInventory` and its two companions are read through `tool_settings` (D53).
use crate::config::Config;
use crate::inventory::{Inventory, ToolSettings};
use crate::schematic::{BlockMirror, BlockRotation};
use crate::settings::{BuildSettings, ExecSettings};

/// The value of a setting. The variants are the Java types of upstream's settings: `Boolean`, `Integer`, `Long`, `Double`, `Float`,
/// `String`, `List<Block>`, `List<Item>`, `List<String>`, `Map<Block, List<Block>>`, `Vec3i`, `Rotation` and `Mirror`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Long(i64),
    Double(f64),
    Float(f32),
    Text(String),
    /// Registry paths of blocks, `minecraft:` left off (`BlockUtils.blockToString`).
    Blocks(Vec<String>),
    /// Registry names of items, with their namespace (`BuiltInRegistries.ITEM.getKey(item).toString()`).
    Items(Vec<String>),
    /// `List<String>`.
    Strings(Vec<String>),
    /// `Map<Block, List<Block>>`, in the order it was written (upstream's `HashMap` prints in its own order: docs/deviations.md D49). Names as in `Blocks`.
    BlockMap(Vec<(String, Vec<String>)>),
    /// `Vec3i`: x, y, z.
    Vec3i(i32, i32, i32),
    /// `net.minecraft.world.level.block.Rotation`.
    Rotation(BlockRotation),
    /// `net.minecraft.world.level.block.Mirror`.
    Mirror(BlockMirror),
}

impl Value {
    /// Upstream's `SettingsUtil.settingTypeToString`: the Java type, without package names.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "Boolean",
            Value::Int(_) => "Integer",
            Value::Long(_) => "Long",
            Value::Double(_) => "Double",
            Value::Float(_) => "Float",
            Value::Text(_) => "String",
            Value::Blocks(_) => "List<Block>",
            Value::Items(_) => "List<Item>",
            Value::Strings(_) => "List<String>",
            // `Type.getTypeName` is `java.util.Map<...Block, java.util.List<...Block>>`; the regex of `settingTypeToString` strips the packages
            Value::BlockMap(_) => "Map<Block, List<Block>>",
            Value::Vec3i(..) => "Vec3i",
            Value::Rotation(_) => "Rotation",
            Value::Mirror(_) => "Mirror",
        }
    }
}

/// What the engine can ask the game about names. A setting that holds blocks or items is only accepted when every name exists
/// (upstream looks the names up in the registries and throws on a missing one).
pub trait Registry {
    fn has_block(&self, path: &str) -> bool;
    fn has_item(&self, name: &str) -> bool;
}

/// A registry that accepts every well-formed name: for tests, and for a glue that cannot look names up.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnyName;

impl Registry for AnyName {
    fn has_block(&self, path: &str) -> bool {
        !path.is_empty()
    }
    fn has_item(&self, name: &str) -> bool {
        !name.is_empty()
    }
}

/// One setting: `Settings.Setting`.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub name: &'static str,
    pub default: Value,
    pub value: Value,
}

// ---------------------------------------------------------------- Java's text for numbers

/// Java's `Double.toString`: the shortest digits that read back as the same double, `1.0E7` style at and above 10^7 and below 10^-3, and
/// a `.0` on a whole number.
pub fn java_double_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    java_shortest(&format!("{v:e}"), v.abs() >= 1.0e-3 && v.abs() < 1.0e7)
}

/// Java's `Float.toString`: the same for a `float`.
pub fn java_float_to_string(v: f32) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    java_shortest(&format!("{v:e}"), v.abs() >= 1.0e-3 && v.abs() < 1.0e7)
}

/// Turns Rust's `1.2345e2` (shortest digits) into Java's text: plain with a point for a medium size, `1.2345E2` otherwise.
fn java_shortest(sci: &str, plain: bool) -> String {
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (neg, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if plain {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                out.push_str(&digits);
                out.push_str(&"0".repeat(int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.push_str(&"0".repeat((-exp - 1) as usize));
            out.push_str(&digits);
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        out.push('E');
        out.push_str(&exp.to_string());
    }
    out
}

/// `Double.parseDouble`, for the texts people type: trims, takes `NaN` and `Infinity`, and a trailing `d` or `f`. (Java also reads hexadecimal
/// floating point; that is not accepted here.)
fn parse_java_double(raw: &str) -> Option<f64> {
    let t = raw.trim();
    let t = t.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(t);
    match t {
        "NaN" | "+NaN" | "-NaN" => Some(f64::NAN),
        "Infinity" | "+Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        _ => {
            if t.is_empty() || t.chars().any(|c| !(c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))) {
                return None;
            }
            t.parse::<f64>().ok()
        }
    }
}

/// `Integer.parseInt` and `Long.parseLong`: an optional sign and decimal digits, nothing else.
fn parse_java_integer(raw: &str) -> Option<i64> {
    let digits = raw.strip_prefix(['+', '-']).unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse::<i64>().ok().or_else(|| raw.strip_prefix('+').and_then(|r| r.parse::<i64>().ok()))
}

/// The block name as a setting holds it: the path for `minecraft:`, the whole name otherwise (`BlockUtils.blockToString`).
fn block_name(raw: &str) -> String {
    let t = raw.trim();
    t.strip_prefix("minecraft:").unwrap_or(t).to_string()
}

/// The item name as a setting holds it: always with a namespace (`ResourceLocation.parse`).
fn item_name(raw: &str) -> String {
    let t = raw.trim();
    if t.contains(':') {
        t.to_string()
    } else {
        format!("minecraft:{t}")
    }
}

/// The registry path of an item name: the part after `minecraft:`.
fn item_path(name: &str) -> String {
    name.strip_prefix("minecraft:").unwrap_or(name).to_string()
}

/// `SettingsUtil.settingValueToString` for a value.
pub fn value_to_string(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Long(l) => l.to_string(),
        Value::Double(d) => java_double_to_string(*d),
        Value::Float(f) => java_float_to_string(*f),
        Value::Text(t) => t.clone(),
        Value::Blocks(l) | Value::Items(l) | Value::Strings(l) => l.join(","),
        Value::BlockMap(m) => m.iter().map(|(k, v)| format!("{k}->{}", v.join(","))).collect::<Vec<_>>().join(","),
        Value::Vec3i(x, y, z) => format!("{x},{y},{z}"),
        Value::Rotation(r) => r.name().to_string(),
        Value::Mirror(m) => m.name().to_string(),
    }
}

/// `SettingsUtil.parseAndApply`'s parsing: the text as a value of the same type as `like`. `None` is a parse error (or a name the registry
/// does not know).
fn parse_value(like: &Value, raw: &str, reg: &dyn Registry) -> Option<Value> {
    Some(match like {
        Value::Bool(_) => Value::Bool(raw.eq_ignore_ascii_case("true")), // Boolean::parseBoolean: anything but "true" is false
        Value::Int(_) => Value::Int(i32::try_from(parse_java_integer(raw)?).ok()?),
        Value::Long(_) => Value::Long(parse_java_integer(raw)?),
        Value::Double(_) => Value::Double(parse_java_double(raw)?),
        Value::Float(_) => Value::Float(parse_java_double(raw)? as f32),
        Value::Text(_) => Value::Text(raw.to_string()),
        Value::Blocks(_) => {
            let mut out = vec![];
            for part in split_java(raw) {
                let n = block_name(&part);
                if !reg.has_block(&n) {
                    return None;
                }
                out.push(n);
            }
            Value::Blocks(out)
        }
        Value::Items(_) => {
            let mut out = vec![];
            for part in split_java(raw) {
                let n = item_name(&part);
                if !reg.has_item(&n) {
                    return None;
                }
                out.push(n);
            }
            Value::Items(out)
        }
        Value::Strings(_) => Value::Strings(split_java(raw)),
        Value::BlockMap(_) => {
            // `raw.split(",(?=[^,]*->)")`, then each piece at "->" (`String.split`, so trailing empty strings go): a piece with no value is an
            // `ArrayIndexOutOfBoundsException`, a name that is no block is a parse error, and a repeated key is `Collectors.toMap`'s `IllegalStateException`
            let mut out: Vec<(String, Vec<String>)> = vec![];
            for piece in split_map_pieces(raw) {
                let parts = split_java_by(&piece, "->");
                let (key, values) = (parts.first()?, parts.get(1)?);
                let key = block_name(key);
                if !reg.has_block(&key) || out.iter().any(|(k, _)| *k == key) {
                    return None;
                }
                let mut list = vec![];
                for v in split_java(values) {
                    let n = block_name(&v);
                    if !reg.has_block(&n) {
                        return None;
                    }
                    list.push(n);
                }
                out.push((key, list));
            }
            Value::BlockMap(out)
        }
        Value::Vec3i(..) => {
            // `Integer.parseInt(str.split(",")[i])` for i in 0..3, with no trimming; more parts than three are ignored
            let parts = split_java(raw);
            let n = |i: usize| parts.get(i).and_then(|p| parse_java_integer(p)).and_then(|v| i32::try_from(v).ok());
            Value::Vec3i(n(0)?, n(1)?, n(2)?)
        }
        // `Rotation.valueOf` and `Mirror.valueOf`: the exact upper-case name
        Value::Rotation(_) => Value::Rotation(BlockRotation::from_name(raw)?),
        Value::Mirror(_) => Value::Mirror(BlockMirror::from_name(raw)?),
    })
}

/// `String.split(sep)` for a literal separator: trailing empty strings are dropped, and an empty text gives one empty string.
fn split_java_by(raw: &str, sep: &str) -> Vec<String> {
    let mut parts: Vec<String> = raw.split(sep).map(str::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// Like `raw.split(",(?=[^,]*->)")`: a comma splits only when the text after it, up to the next comma, holds `->`.
fn split_map_pieces(raw: &str) -> Vec<String> {
    let mut parts: Vec<String> = vec![];
    let mut cur = String::new();
    let chars: Vec<char> = raw.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c == ',' {
            let rest: String = chars[i + 1..].iter().take_while(|c| **c != ',').collect();
            if rest.contains("->") {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
        }
        cur.push(*c);
    }
    parts.push(cur);
    while parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// Java's `String.split(",")`: trailing empty strings are dropped, and an empty text gives one empty string.
fn split_java(raw: &str) -> Vec<String> {
    let mut parts: Vec<String> = raw.split(',').map(str::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

// ---------------------------------------------------------------- the table

/// Every setting of this port, with the name, type and default of upstream's `Settings.java` at 1b8e87c.
fn table() -> Vec<Setting> {
    use Value::*;
    let s = |name: &'static str, v: Value| Setting { name, default: v.clone(), value: v };
    let blocks = |l: &[&str]| Blocks(l.iter().map(|b| b.to_string()).collect());
    let items = |l: &[&str]| Items(l.iter().map(|b| b.to_string()).collect());
    vec![
        // movement, the world and the costs: `Config`
        s("allowBreak", Bool(true)),
        s("allowBreakAnyway", blocks(&[])),
        s("allowPlace", Bool(true)),
        s("allowSprint", Bool(true)),
        s("allowParkour", Bool(false)),
        s("allowParkourPlace", Bool(false)),
        s("allowParkourAscend", Bool(true)),
        s("allowJumpAtBuildLimit", Bool(false)),
        s("allowWalkOnBuildLimit", Bool(true)),
        s("allowWaterBucketFall", Bool(true)),
        s("allowDiagonalDescend", Bool(false)),
        s("allowDiagonalAscend", Bool(false)),
        s("allowDownward", Bool(true)),
        s("allowSwimming", Bool(false)),
        s("allowVines", Bool(false)),
        s("allowWalkOnBottomSlab", Bool(true)),
        s("allowWalkOnMagmaBlocks", Bool(false)),
        s("allowPlaceInFluidsSource", Bool(true)),
        s("allowPlaceInFluidsFlow", Bool(true)),
        s("assumeWalkOnWater", Bool(false)),
        s("assumeWalkOnLava", Bool(false)),
        s("strictLiquidCheck", Bool(false)),
        s("avoidUpdatingFallingBlocks", Bool(true)),
        s("blocksToAvoid", blocks(&["tripwire"])),
        s("blocksToDisallowBreaking", blocks(&[])),
        s("maxFallHeightNoWater", Int(3)),
        s("maxFallHeightBucket", Int(20)),
        s("blockPlacementPenalty", Double(20.0)),
        s("blockBreakAdditionalPenalty", Double(2.0)),
        s("jumpPenalty", Double(2.0)),
        s("walkOnWaterOnePenalty", Double(3.0)),
        s("costHeuristic", Double(3.563)),
        s("axisHeight", Int(120)),
        // the search
        s("backtrackCostFavoringCoefficient", Double(0.5)),
        s("pathingMaxChunkBorderFetch", Int(50)),
        s("minimumImprovementRepropagation", Bool(true)),
        s("pathCutoffMinimumLength", Int(30)),
        s("pathCutoffFactor", Double(0.9)),
        s("cutoffAtLoadBoundary", Bool(false)),
        s("simplifyUnloadedYCoord", Bool(true)),
        s("primaryTimeoutMS", Long(500)),
        s("failureTimeoutMS", Long(2000)),
        s("planAheadPrimaryTimeoutMS", Long(4000)),
        s("planAheadFailureTimeoutMS", Long(5000)),
        s("avoidance", Bool(false)),
        s("mobSpawnerAvoidanceCoefficient", Double(2.0)),
        s("mobSpawnerAvoidanceRadius", Int(16)),
        s("mobAvoidanceCoefficient", Double(1.5)),
        s("mobAvoidanceRadius", Int(8)),
        // the executor: `ExecSettings`
        s("sprintAscends", Bool(true)),
        s("costVerificationLookahead", Int(5)),
        s("maxCostIncrease", Double(10.0)),
        s("movementTimeoutTicks", Int(100)),
        s("maxPathHistoryLength", Int(300)),
        s("pathHistoryCutoffAmount", Int(50)),
        s("splicePath", Bool(true)),
        s("planningTickLookahead", Int(150)),
        s("overshootTraverse", Bool(true)),
        s("allowOvershootDiagonalDescend", Bool(true)),
        s("assumeStep", Bool(false)),
        s("sprintInWater", Bool(true)),
        s("cancelOnGoalInvalidation", Bool(true)),
        s("assumeSafeWalk", Bool(false)),
        s("walkWhileBreaking", Bool(true)),
        s("pauseMiningForFallingBlocks", Bool(true)),
        s("remainWithExistingLookDirection", Bool(true)),
        // the mine and get-to-block processes: `MineSettings`
        s("mineGoalUpdateInterval", Int(5)),
        s("mineMaxOreLocationsCount", Int(64)),
        s("maxCachedWorldScanCount", Int(10)),
        s("extendCacheOnThreshold", Bool(false)),
        s("minYLevelWhileMining", Int(0)),
        s("maxYLevelWhileMining", Int(2031)),
        s("allowOnlyExposedOres", Bool(false)),
        s("allowOnlyExposedOresDistance", Int(1)),
        s("exploreForBlocks", Bool(true)),
        s("mineScanDroppedItems", Bool(true)),
        s("mineDropLoiterDurationMSThanksLouca", Long(250)),
        s("legitMine", Bool(false)),
        s("legitMineYLevel", Int(-59)),
        s("legitMineIncludeDiagonals", Bool(false)),
        s("forceInternalMining", Bool(true)),
        s("internalMiningAirException", Bool(true)),
        s("blacklistClosestOnFailure", Bool(true)),
        s("rightClickContainerOnArrival", Bool(true)),
        s("enterPortal", Bool(true)),
        // the persistent chunk cache: `CacheSettings`
        s("chunkCaching", Bool(true)),
        s("pruneRegionsFromRAM", Bool(true)),
        s("chunkPackerQueueMaxSize", Int(2000)),
        s("cachedChunksExpirySeconds", Long(-1)),
        s("repackOnAnyBlockChange", Bool(true)),
        // the follow process: `FollowSettings`
        s("followOffsetDistance", Double(0.0)),
        s("followOffsetDirection", Float(0.0)),
        s("followRadius", Int(3)),
        s("followTargetMaxDistance", Int(0)),
        // the explore process: `ExploreSettings`
        s("exploreChunkSetMinimumSize", Int(10)),
        s("worldExploringChunkOffset", Int(0)),
        s("exploreMaintainY", Int(64)),
        s("disableCompletionCheck", Bool(false)),
        // the hotbar: `ToolSettings`
        s("autoTool", Bool(true)),
        s("assumeExternalAutoTool", Bool(false)),
        s("preferSilkTouch", Bool(false)),
        s("useSwordToMine", Bool(true)),
        s("itemSaver", Bool(false)),
        s("itemSaverThreshold", Int(10)),
        s("considerPotionEffects", Bool(true)),
        s("blocksToAvoidBreaking", blocks(&["crafting_table", "furnace", "chest", "trapped_chest"])),
        s("avoidBreakingMultiplier", Double(0.1)),
        s("acceptableThrowawayItems", items(&["minecraft:dirt", "minecraft:cobblestone", "minecraft:netherrack", "minecraft:stone"])),
        s("allowInventory", Bool(false)),
        s("ticksBetweenInventoryMoves", Int(1)),
        s("inventoryMoveOnlyIfStationary", Bool(false)),
        // the builder: `BuildSettings`
        s("buildInLayers", Bool(false)),
        s("layerOrder", Bool(false)),
        s("layerHeight", Int(1)),
        s("startAtLayer", Int(0)),
        s("skipFailedLayers", Bool(false)),
        s("buildRepeat", Vec3i(0, 0, 0)),
        s("buildRepeatCount", Int(-1)),
        s("buildRepeatSneaky", Bool(true)),
        s("breakFromAbove", Bool(false)),
        s("goalBreakFromAbove", Bool(false)),
        s("mapArtMode", Bool(false)),
        s("okIfWater", Bool(false)),
        s("incorrectSize", Int(100)),
        s("breakCorrectBlockPenaltyMultiplier", Double(10.0)),
        s("placeIncorrectBlockPenaltyMultiplier", Double(2.0)),
        s("schematicOrientationX", Bool(false)),
        s("schematicOrientationY", Bool(false)),
        s("schematicOrientationZ", Bool(false)),
        s("buildSchematicRotation", Rotation(BlockRotation::None)),
        s("buildSchematicMirror", Mirror(BlockMirror::None)),
        s("builderTickScanRadius", Int(5)),
        s("distanceTrim", Bool(true)),
        s("buildIgnoreBlocks", blocks(&[])),
        s("buildSkipBlocks", blocks(&[])),
        s("okIfAir", blocks(&[])),
        s("buildValidSubstitutes", BlockMap(vec![])),
        s("buildSubstitutes", BlockMap(vec![])),
        s("buildIgnoreExisting", Bool(false)),
        s("buildIgnoreDirection", Bool(false)),
        s("buildIgnoreProperties", Strings(vec![])),
        // the glue's: the chat line and the file extension of `build` (`notificationOnBuildFinished` is a desktop notification upstream, D37.6)
        s("notificationOnBuildFinished", Bool(true)),
        s("schematicFallbackExtension", Text("schematic".to_string())),
        s("renderPath", Bool(true)),
        s("freeLook", Bool(true)),
        s("blockFreeLook", Bool(false)),
        s("antiCheatCompatibility", Bool(true)),
        s("smoothLook", Bool(false)),
        s("smoothLookTicks", Int(5)),
        s("censorCoordinates", Bool(false)),
    ]
}

/// All settings, with their values. `Settings.allSettings`.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    list: Vec<Setting>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::new()
    }
}

/// What `set` asks the caller to do besides printing: files are the glue's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetEffect {
    None,
    /// `set save`: write the modified settings to `settings.txt`.
    Save,
    /// `set load [file]`: reset the modified settings, then read the file (default `settings.txt`).
    Load(String),
    /// A setting changed: the glue rebuilds `Config`, `ExecSettings` and `ToolSettings` from `Settings`.
    Changed,
}

/// What `set` answers: lines for the chat, and what to do. `error` is a command error (upstream throws a `CommandException`).
#[derive(Clone, Debug, PartialEq)]
pub struct SetOutcome {
    pub lines: Vec<String>,
    pub effect: SetEffect,
    pub error: Option<String>,
}

impl SetOutcome {
    fn lines(lines: Vec<String>, effect: SetEffect) -> SetOutcome {
        SetOutcome { lines, effect, error: None }
    }
    fn err(msg: String) -> SetOutcome {
        SetOutcome { lines: vec![], effect: SetEffect::None, error: Some(msg) }
    }
}

impl Settings {
    /// Every setting at its default.
    pub fn new() -> Settings {
        Settings { list: table() }
    }

    /// `Settings.allSettings`.
    pub fn all(&self) -> &[Setting] {
        &self.list
    }

    /// Upstream's `Settings.byLowerName`: a setting by its name, in any case.
    pub fn find(&self, name: &str) -> Option<&Setting> {
        self.list.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    fn find_mut(&mut self, name: &str) -> Option<&mut Setting> {
        self.list.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Upstream's `SettingsUtil.modifiedSettings`: the settings whose value is not the default.
    pub fn modified(&self) -> Vec<&Setting> {
        self.list.iter().filter(|s| s.value != s.default).collect()
    }

    /// The value of a setting; panics on a name that is not in the table, which is a mistake in this file.
    pub fn get(&self, name: &str) -> &Value {
        &self.find(name).unwrap_or_else(|| panic!("no setting named {name}")).value
    }

    pub fn get_bool(&self, name: &str) -> bool {
        match self.get(name) {
            Value::Bool(b) => *b,
            v => panic!("{name} is a {}", v.type_name()),
        }
    }

    pub fn get_int(&self, name: &str) -> i32 {
        match self.get(name) {
            Value::Int(i) => *i,
            v => panic!("{name} is a {}", v.type_name()),
        }
    }

    pub fn get_long(&self, name: &str) -> i64 {
        match self.get(name) {
            Value::Long(i) => *i,
            v => panic!("{name} is a {}", v.type_name()),
        }
    }

    pub fn get_double(&self, name: &str) -> f64 {
        match self.get(name) {
            Value::Double(d) => *d,
            v => panic!("{name} is a {}", v.type_name()),
        }
    }

    pub fn get_float(&self, name: &str) -> f32 {
        match self.get(name) {
            Value::Float(f) => *f,
            v => panic!("{name} is a {}", v.type_name()),
        }
    }

    pub fn get_list(&self, name: &str) -> &[String] {
        match self.get(name) {
            Value::Blocks(l) | Value::Items(l) | Value::Strings(l) => l,
            v => panic!("{name} is a {}", v.type_name()),
        }
    }

    /// Sets a value by name with the text people type (`SettingsUtil.parseAndApply`). `Err` says why not.
    pub fn parse_and_apply(&mut self, name: &str, raw: &str, reg: &dyn Registry) -> Result<(), String> {
        let Some(setting) = self.find_mut(name) else {
            return Err("No setting by that name".to_string());
        };
        match parse_value(&setting.value, raw, reg) {
            Some(v) => {
                setting.value = v;
                Ok(())
            }
            None => Err(format!("{raw:?} is not a valid {}", setting.value.type_name())),
        }
    }

    /// Puts a value in a setting by name; the type must be the setting's. For the glue and for tests.
    pub fn set(&mut self, name: &str, value: Value) -> Result<(), String> {
        let Some(setting) = self.find_mut(name) else {
            return Err("No setting by that name".to_string());
        };
        if std::mem::discriminant(&setting.value) != std::mem::discriminant(&value) {
            return Err(format!("{} is a {}", setting.name, setting.value.type_name()));
        }
        setting.value = value;
        Ok(())
    }

    /// `Setting.reset`.
    pub fn reset(&mut self, name: &str) -> bool {
        match self.find_mut(name) {
            Some(s) => {
                s.value = s.default.clone();
                true
            }
            None => false,
        }
    }

    /// Resets the modified settings to their defaults (`set reset all`, and the first step of `set load`).
    pub fn reset_all(&mut self) {
        for s in &mut self.list {
            s.value = s.default.clone();
        }
    }

    /// The text of `settings.txt` for the modified settings: a line `name value` each (`SettingsUtil.save`).
    pub fn to_file_text(&self) -> String {
        self.modified().iter().map(|s| format!("{} {}\n", s.name, value_to_string(&s.value))).collect()
    }

    /// `SettingsUtil.readAndApply`'s lines: a line is `name value` split at the first space; empty lines and lines starting with `#` or `//`
    /// are skipped. Returns the lines that could not be applied, with the reason.
    pub fn apply_file_text(&mut self, text: &str, reg: &dyn Registry) -> Vec<String> {
        let mut problems = vec![];
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }
            // upstream's SETTING_PATTERN `^(?<setting>[^ ]+) +(?<value>.+)`, whole line: a name that does not start with a space, then spaces,
            // then at least one character (when only spaces follow, the regex gives the last one back as the value)
            let rest = match line.find(' ') {
                Some(i) if i > 0 => &line[i..],
                _ => "",
            };
            let name = &line[..line.len() - rest.len()];
            let trimmed = rest.trim_start_matches(' ');
            let value = if !trimmed.is_empty() {
                trimmed
            } else if rest.len() >= 2 {
                " "
            } else {
                problems.push(format!("Invalid syntax in setting file: {line}"));
                continue;
            };
            let name = if name.eq_ignore_ascii_case("allowjumpat256") { "allowJumpAtBuildLimit" } else { name };
            if self.parse_and_apply(name, value, reg).is_err() {
                problems.push(format!("Unable to parse line {line}"));
            }
        }
        problems
    }

    // ------------------------------------------------------------ the snapshots

    /// Writes the settings into a `Config`: the search, the costs and the movement rules. The fields the player decides (`can_sprint`, `frost_walker`,
    /// `water_mult`, `has_throwaway`, `has_water_bucket`) are left alone: the glue sets them each tick, with [`Settings::apply_inventory`].
    pub fn apply_to_config(&self, c: &mut Config) {
        c.allow_break = self.get_bool("allowBreak");
        c.allow_break_anyway = self.get_list("allowBreakAnyway").to_vec();
        c.blocks_to_avoid = self.get_list("blocksToAvoid").to_vec();
        c.blocks_to_disallow_breaking = self.get_list("blocksToDisallowBreaking").to_vec();
        c.assume_walk_on_water = self.get_bool("assumeWalkOnWater");
        c.assume_walk_on_lava = self.get_bool("assumeWalkOnLava");
        c.allow_walk_on_magma = self.get_bool("allowWalkOnMagmaBlocks");
        c.allow_vines = self.get_bool("allowVines");
        c.allow_walk_on_bottom_slab = self.get_bool("allowWalkOnBottomSlab");
        c.strict_liquid_check = self.get_bool("strictLiquidCheck");
        c.avoid_updating_falling = self.get_bool("avoidUpdatingFallingBlocks");
        c.allow_place_source = self.get_bool("allowPlaceInFluidsSource");
        c.allow_place_flow = self.get_bool("allowPlaceInFluidsFlow");
        c.allow_swimming = self.get_bool("allowSwimming");
        c.allow_parkour = self.get_bool("allowParkour");
        c.allow_parkour_place = self.get_bool("allowParkourPlace");
        c.allow_parkour_ascend = self.get_bool("allowParkourAscend");
        c.allow_jump_at_build_limit = self.get_bool("allowJumpAtBuildLimit");
        c.allow_diagonal_descend = self.get_bool("allowDiagonalDescend");
        c.allow_diagonal_ascend = self.get_bool("allowDiagonalAscend");
        c.allow_downward = self.get_bool("allowDownward");
        c.max_fall_no_water = self.get_int("maxFallHeightNoWater");
        c.max_fall_bucket = self.get_int("maxFallHeightBucket");
        c.place_penalty = self.get_double("blockPlacementPenalty");
        c.break_penalty = self.get_double("blockBreakAdditionalPenalty");
        c.jump_penalty = self.get_double("jumpPenalty");
        c.walk_on_water_penalty = self.get_double("walkOnWaterOnePenalty");
        c.cost_heuristic = self.get_double("costHeuristic");
        c.axis_height = self.get_int("axisHeight");
        let q = &mut c.search;
        q.backtrack_cost_favoring_coefficient = self.get_double("backtrackCostFavoringCoefficient");
        q.pathing_max_chunk_border_fetch = self.get_int("pathingMaxChunkBorderFetch");
        q.minimum_improvement_repropagation = self.get_bool("minimumImprovementRepropagation");
        q.path_cutoff_minimum_length = self.get_int("pathCutoffMinimumLength");
        q.path_cutoff_factor = self.get_double("pathCutoffFactor");
        q.cutoff_at_load_boundary = self.get_bool("cutoffAtLoadBoundary");
        q.simplify_unloaded_y_coord = self.get_bool("simplifyUnloadedYCoord");
        q.primary_timeout_ms = self.get_long("primaryTimeoutMS");
        q.failure_timeout_ms = self.get_long("failureTimeoutMS");
        q.plan_ahead_primary_timeout_ms = self.get_long("planAheadPrimaryTimeoutMS");
        q.plan_ahead_failure_timeout_ms = self.get_long("planAheadFailureTimeoutMS");
        q.avoidance = self.get_bool("avoidance");
        q.walk_on_build_limit = self.get_bool("allowWalkOnBuildLimit");
        q.mob_spawner_avoidance_coefficient = self.get_double("mobSpawnerAvoidanceCoefficient");
        q.mob_spawner_avoidance_radius = self.get_int("mobSpawnerAvoidanceRadius");
        q.mob_avoidance_coefficient = self.get_double("mobAvoidanceCoefficient");
        q.mob_avoidance_radius = self.get_int("mobAvoidanceRadius");
    }

    /// The tool and throwaway settings.
    pub fn tool_settings(&self) -> ToolSettings {
        ToolSettings {
            auto_tool: self.get_bool("autoTool"),
            assume_external_auto_tool: self.get_bool("assumeExternalAutoTool"),
            prefer_silk_touch: self.get_bool("preferSilkTouch"),
            use_sword_to_mine: self.get_bool("useSwordToMine"),
            item_saver: self.get_bool("itemSaver"),
            item_saver_threshold: self.get_int("itemSaverThreshold"),
            consider_potion_effects: self.get_bool("considerPotionEffects"),
            blocks_to_avoid_breaking: self.get_list("blocksToAvoidBreaking").to_vec(),
            avoid_breaking_multiplier: self.get_double("avoidBreakingMultiplier"),
            acceptable_throwaway_items: self.get_list("acceptableThrowawayItems").iter().map(|n| item_path(n)).collect(),
            allow_inventory: self.get_bool("allowInventory"),
            ticks_between_inventory_moves: self.get_int("ticksBetweenInventoryMoves"),
            inventory_move_only_if_stationary: self.get_bool("inventoryMoveOnlyIfStationary"),
        }
    }

    /// The settings of free look (`freelook.rs`): the glue reads them each tick.
    pub fn look_settings(&self) -> crate::freelook::LookSettings {
        crate::freelook::LookSettings { free_look: self.get_bool("freeLook"), block_free_look: self.get_bool("blockFreeLook"), anti_cheat_compatibility: self.get_bool("antiCheatCompatibility"), smooth_look: self.get_bool("smoothLook"), smooth_look_ticks: self.get_int("smoothLookTicks") }
    }

    /// Writes the settings into an `ExecSettings`. `mouse_sensitivity` is not an upstream setting and is left alone.
    pub fn apply_to_exec(&self, e: &mut ExecSettings) {
        e.allow_sprint = self.get_bool("allowSprint");
        e.sprint_ascends = self.get_bool("sprintAscends");
        e.cost_verification_lookahead = self.get_int("costVerificationLookahead");
        e.max_cost_increase = self.get_double("maxCostIncrease");
        e.movement_timeout_ticks = self.get_int("movementTimeoutTicks");
        e.max_path_history_length = self.get_int("maxPathHistoryLength");
        e.path_history_cutoff_amount = self.get_int("pathHistoryCutoffAmount");
        e.splice_path = self.get_bool("splicePath");
        e.planning_tick_lookahead = self.get_int("planningTickLookahead");
        e.overshoot_traverse = self.get_bool("overshootTraverse");
        e.allow_overshoot_diagonal_descend = self.get_bool("allowOvershootDiagonalDescend");
        e.assume_step = self.get_bool("assumeStep");
        e.sprint_in_water = self.get_bool("sprintInWater");
        e.cancel_on_goal_invalidation = self.get_bool("cancelOnGoalInvalidation");
        e.allow_place = self.get_bool("allowPlace");
        e.assume_safe_walk = self.get_bool("assumeSafeWalk");
        e.walk_while_breaking = self.get_bool("walkWhileBreaking");
        e.pause_mining_for_falling_blocks = self.get_bool("pauseMiningForFallingBlocks");
        e.remain_with_existing_look_direction = self.get_bool("remainWithExistingLookDirection");
        e.tool = self.tool_settings();
        e.mine = self.mine_settings();
        e.follow = self.follow_settings();
        e.explore = self.explore_settings();
    }

    pub fn get_text(&self, name: &str) -> &str {
        match self.get(name) {
            Value::Text(t) => t,
            v => panic!("{name} is a {}", v.type_name()),
        }
    }

    fn get_block_map(&self, name: &str) -> Vec<(String, Vec<String>)> {
        match self.get(name) {
            Value::BlockMap(m) => m.clone(),
            v => panic!("{name} is a {}", v.type_name()),
        }
    }

    /// The builder's settings, as one snapshot for a tick (`BuildSettings`).
    pub fn build_settings(&self) -> BuildSettings {
        let (rx, ry, rz) = match self.get("buildRepeat") {
            Value::Vec3i(x, y, z) => (*x, *y, *z),
            v => panic!("buildRepeat is a {}", v.type_name()),
        };
        let rotation = match self.get("buildSchematicRotation") {
            Value::Rotation(r) => *r,
            v => panic!("buildSchematicRotation is a {}", v.type_name()),
        };
        let mirror = match self.get("buildSchematicMirror") {
            Value::Mirror(m) => *m,
            v => panic!("buildSchematicMirror is a {}", v.type_name()),
        };
        BuildSettings {
            build_in_layers: self.get_bool("buildInLayers"),
            layer_order: self.get_bool("layerOrder"),
            layer_height: self.get_int("layerHeight"),
            start_at_layer: self.get_int("startAtLayer"),
            skip_failed_layers: self.get_bool("skipFailedLayers"),
            build_repeat: (rx, ry, rz),
            build_repeat_count: self.get_int("buildRepeatCount"),
            build_repeat_sneaky: self.get_bool("buildRepeatSneaky"),
            break_from_above: self.get_bool("breakFromAbove"),
            goal_break_from_above: self.get_bool("goalBreakFromAbove"),
            map_art_mode: self.get_bool("mapArtMode"),
            ok_if_water: self.get_bool("okIfWater"),
            incorrect_size: self.get_int("incorrectSize"),
            break_correct_block_penalty_multiplier: self.get_double("breakCorrectBlockPenaltyMultiplier"),
            place_incorrect_block_penalty_multiplier: self.get_double("placeIncorrectBlockPenaltyMultiplier"),
            schematic_orientation_x: self.get_bool("schematicOrientationX"),
            schematic_orientation_y: self.get_bool("schematicOrientationY"),
            schematic_orientation_z: self.get_bool("schematicOrientationZ"),
            build_schematic_rotation: rotation,
            build_schematic_mirror: mirror,
            builder_tick_scan_radius: self.get_int("builderTickScanRadius"),
            distance_trim: self.get_bool("distanceTrim"),
            build_ignore_blocks: self.get_list("buildIgnoreBlocks").to_vec(),
            build_skip_blocks: self.get_list("buildSkipBlocks").to_vec(),
            ok_if_air: self.get_list("okIfAir").to_vec(),
            build_valid_substitutes: self.get_block_map("buildValidSubstitutes"),
            build_substitutes: self.get_block_map("buildSubstitutes"),
            build_ignore_existing: self.get_bool("buildIgnoreExisting"),
            build_ignore_direction: self.get_bool("buildIgnoreDirection"),
            build_ignore_properties: self.get_list("buildIgnoreProperties").to_vec(),
        }
    }

    /// The settings of the explore process.
    pub fn explore_settings(&self) -> crate::settings::ExploreSettings {
        crate::settings::ExploreSettings { explore_chunk_set_minimum_size: self.get_int("exploreChunkSetMinimumSize"), world_exploring_chunk_offset: self.get_int("worldExploringChunkOffset"), explore_maintain_y: self.get_int("exploreMaintainY"), disable_completion_check: self.get_bool("disableCompletionCheck") }
    }

    /// The settings of the follow process.
    pub fn follow_settings(&self) -> crate::settings::FollowSettings {
        crate::settings::FollowSettings { follow_radius: self.get_int("followRadius"), follow_offset_distance: self.get_double("followOffsetDistance"), follow_offset_direction: self.get_float("followOffsetDirection"), follow_target_max_distance: self.get_int("followTargetMaxDistance") }
    }

    /// The settings of the persistent chunk cache.
    pub fn cache_settings(&self) -> crate::cache::CacheSettings {
        crate::cache::CacheSettings { chunk_caching: self.get_bool("chunkCaching"), prune_regions_from_ram: self.get_bool("pruneRegionsFromRAM"), chunk_packer_queue_max_size: self.get_int("chunkPackerQueueMaxSize"), cached_chunks_expiry_seconds: self.get_long("cachedChunksExpirySeconds"), repack_on_any_block_change: self.get_bool("repackOnAnyBlockChange") }
    }

    /// The settings of the mine and get-to-block processes.
    pub fn mine_settings(&self) -> crate::settings::MineSettings {
        crate::settings::MineSettings {
            mine_goal_update_interval: self.get_int("mineGoalUpdateInterval"),
            mine_max_ore_locations_count: self.get_int("mineMaxOreLocationsCount"),
            max_cached_world_scan_count: self.get_int("maxCachedWorldScanCount"),
            extend_cache_on_threshold: self.get_bool("extendCacheOnThreshold"),
            min_y_level_while_mining: self.get_int("minYLevelWhileMining"),
            max_y_level_while_mining: self.get_int("maxYLevelWhileMining"),
            allow_only_exposed_ores: self.get_bool("allowOnlyExposedOres"),
            allow_only_exposed_ores_distance: self.get_int("allowOnlyExposedOresDistance"),
            explore_for_blocks: self.get_bool("exploreForBlocks"),
            mine_scan_dropped_items: self.get_bool("mineScanDroppedItems"),
            mine_drop_loiter_duration_ms: self.get_long("mineDropLoiterDurationMSThanksLouca"),
            legit_mine: self.get_bool("legitMine"),
            legit_mine_y_level: self.get_int("legitMineYLevel"),
            legit_mine_include_diagonals: self.get_bool("legitMineIncludeDiagonals"),
            force_internal_mining: self.get_bool("forceInternalMining"),
            internal_mining_air_exception: self.get_bool("internalMiningAirException"),
            blacklist_closest_on_failure: self.get_bool("blacklistClosestOnFailure"),
            right_click_container_on_arrival: self.get_bool("rightClickContainerOnArrival"),
            enter_portal: self.get_bool("enterPortal"),
        }
    }

    /// `CalculationContext.<init>(IBaritone,boolean)`: what the player's inventory and the world make of two fields of the context.
    /// `hasThrowaway` is `allowPlace` and a block in the list somewhere in reach (`hasGenericThrowaway`); `hasWaterBucket` is
    /// `allowWaterBucketFall`, a water bucket in the hotbar and a dimension that is not the Nether. The glue calls this each tick, on a copy of
    /// the `Config` of the tables, before the tables are used.
    pub fn apply_inventory(&self, c: &mut Config, inv: &dyn Inventory, nether: bool) {
        let ts = self.tool_settings();
        c.has_throwaway = self.get_bool("allowPlace") && crate::inventory::has_generic_throwaway(inv, &ts.acceptable_throwaway_items, ts.allow_inventory);
        c.has_water_bucket = self.get_bool("allowWaterBucketFall") && crate::inventory::find_hotbar_slot(inv, "water_bucket").is_some() && !nether;
    }

    // ------------------------------------------------------------ the command

    /// `SetCommand.execute`. `args` are the words after `set` (or `setting`, `settings`). The text of the answer is upstream's; a
    /// `CommandException` is `error`.
    pub fn run_set(&mut self, args: &[&str], reg: &dyn Registry) -> SetOutcome {
        let arg = args.first().map(|a| a.to_lowercase()).unwrap_or_else(|| "list".to_string());
        let rest = if args.is_empty() { &[][..] } else { &args[1..] };
        if matches!(arg.as_str(), "s" | "save") {
            return SetOutcome::lines(vec!["Settings saved".to_string()], SetEffect::Save);
        }
        if matches!(arg.as_str(), "load" | "ld") {
            let file = rest.first().map(|f| f.to_string()).unwrap_or_else(|| "settings.txt".to_string());
            return SetOutcome::lines(vec![format!("Settings reloaded from {file}")], SetEffect::Load(file));
        }
        let view_modified = matches!(arg.as_str(), "m" | "mod" | "modified");
        let view_all = matches!(arg.as_str(), "all" | "l" | "list");
        if view_modified || view_all {
            let search = match rest.first() {
                Some(a) if a.parse::<i32>().is_err() => a.to_string(),
                _ => String::new(),
            };
            if rest.len() > 1 {
                return SetOutcome::err("Too many arguments (expected at most 1)".to_string());
            }
            let mut entries: Vec<&Setting> = if view_modified { self.modified() } else { self.list.iter().collect() };
            entries.retain(|s| s.name.to_lowercase().contains(&search.to_lowercase()));
            entries.sort_by_key(|s| s.name.to_lowercase());
            let modified_word = if view_modified { "modified " } else { "" };
            let head = if search.is_empty() { format!("All {modified_word}settings:") } else { format!("All {modified_word}settings containing the string '{search}':") };
            // Paginator: pages of 8, the page is the argument when it is a number
            let page_size = 8usize;
            let max_page = (entries.len().max(1) - 1) / page_size + 1;
            let page = rest.first().and_then(|a| a.parse::<i32>().ok()).unwrap_or(1);
            if page < 1 || page as usize > max_page {
                return SetOutcome::err(format!("Expected a valid page (1-{max_page}), but got {page} instead"));
            }
            let mut lines = vec![head];
            let offset = (page as usize - 1) * page_size;
            for i in offset..offset + page_size {
                match entries.get(i) {
                    Some(s) => lines.push(format!("{} ({})", s.name, s.value.type_name())),
                    None => lines.push("--".to_string()),
                }
            }
            lines.push(format!("<< | >> {page}/{max_page}"));
            return SetOutcome::lines(lines, SetEffect::None);
        }
        if rest.len() > 1 {
            return SetOutcome::err("Too many arguments (expected at most 1)".to_string());
        }
        let resetting = arg == "reset";
        let toggling = arg == "toggle";
        let doing_something = resetting || toggling;
        let mut lines = vec![];
        if resetting {
            match rest.first() {
                None => {
                    lines.push("Please specify 'all' as an argument to reset to confirm you'd really like to do this".to_string());
                    lines.push("ALL settings will be reset. Use the 'set modified' or 'modified' commands to see what will be reset.".to_string());
                    lines.push("Specify a setting name instead of 'all' to only reset one setting".to_string());
                }
                Some(a) if a.eq_ignore_ascii_case("all") => {
                    self.reset_all();
                    return SetOutcome::lines(vec!["All settings have been reset to their default values".to_string()], SetEffect::Save);
                }
                Some(_) => {}
            }
        }
        if toggling && rest.is_empty() {
            return SetOutcome::err("Not enough arguments (expected at least 1)".to_string());
        }
        let setting_name = if doing_something {
            match rest.first() {
                Some(n) => n.to_string(),
                // `set reset` alone: the explanation, then upstream throws not enough arguments
                None => return SetOutcome { lines, effect: SetEffect::None, error: Some("Not enough arguments (expected at least 1)".to_string()) },
            }
        } else {
            args[0].to_string()
        };
        let Some(setting) = self.find(&setting_name) else {
            return SetOutcome::err("Expected a valid setting".to_string());
        };
        let name = setting.name;
        let value_args = if doing_something { &rest[1.min(rest.len())..] } else { rest };
        if !doing_something && value_args.is_empty() {
            lines.push(format!("Value of setting {name}:"));
            lines.push(value_to_string(&setting.value));
            return SetOutcome::lines(lines, SetEffect::None);
        }
        let old = value_to_string(&setting.value);
        if resetting {
            self.reset(name);
        } else if toggling {
            let Value::Bool(b) = setting.value else {
                return SetOutcome::err("Expected a toggleable setting, but got some other setting instead".to_string());
            };
            let nb = !b;
            let _ = self.set(name, Value::Bool(nb));
            lines.push(format!("Toggled setting {name} to {nb}"));
        } else {
            let Some(new_value) = value_args.first() else {
                return SetOutcome::err("Not enough arguments (expected at least 1)".to_string());
            };
            if let Err(e) = self.parse_and_apply(name, new_value, reg) {
                return SetOutcome::err(format!("Expected a valid value ({e})"));
            }
        }
        if !toggling {
            let now = value_to_string(self.get(name));
            lines.push(format!("Successfully {} {name} to {now}", if resetting { "reset" } else { "set" }));
        }
        lines.push(format!("Old value: {old}"));
        if (name == "chatControl" || name == "chatControlAnyway" || name == "prefixControl") && matches!(self.get(name), Value::Bool(false)) {
            lines.push("Warning: some ways of entering commands no longer work.".to_string());
        }
        SetOutcome::lines(lines, SetEffect::Changed)
    }

    /// `SetCommand.tabComplete`. `before` are the words typed after `set`, without the one being completed, and `current` is that word.
    pub fn complete(&self, before: &[&str], current: &str) -> Vec<String> {
        let cur = current.to_lowercase();
        let filter = |mut v: Vec<String>| {
            v.retain(|s| s.to_lowercase().starts_with(&cur));
            v
        };
        match before {
            [] => {
                let mut names: Vec<String> = self.list.iter().map(|s| s.name.to_string()).collect();
                names.sort_by_key(|n| n.to_lowercase());
                let mut v: Vec<String> = ["list", "modified", "reset", "toggle", "save", "load"].iter().map(|s| s.to_string()).collect();
                v.extend(names);
                filter(v)
            }
            [first] => {
                let first_l = first.to_lowercase();
                if first_l == "reset" {
                    let mut v: Vec<String> = vec!["all".to_string()];
                    let mut m: Vec<String> = self.modified().iter().map(|s| s.name.to_string()).collect();
                    m.sort_by_key(|n| n.to_lowercase());
                    v.extend(m);
                    filter(v)
                } else if first_l == "toggle" {
                    let mut v: Vec<String> = self.list.iter().filter(|s| matches!(s.value, Value::Bool(_))).map(|s| s.name.to_string()).collect();
                    v.sort_by_key(|n| n.to_lowercase());
                    filter(v)
                } else if matches!(first_l.as_str(), "ld" | "load" | "s" | "save") {
                    vec![]
                } else if let Some(s) = self.find(first) {
                    match &s.value {
                        Value::Bool(b) => filter(if *b { vec!["true".to_string(), "false".to_string()] } else { vec!["false".to_string(), "true".to_string()] }),
                        v => vec![value_to_string(v)],
                    }
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut Settings, line: &str) -> SetOutcome {
        let words: Vec<&str> = line.split_whitespace().collect();
        s.run_set(&words, &AnyName)
    }

    #[test]
    fn the_defaults_are_the_snapshots_the_engine_started_with() {
        let s = Settings::new();
        let mut c = Config::upstream_defaults();
        // scramble every field the table owns, then write the defaults back
        c.allow_break = false;
        c.blocks_to_avoid = vec![];
        c.search.primary_timeout_ms = 1;
        c.cost_heuristic = 0.0;
        s.apply_to_config(&mut c);
        let mut want = Config::upstream_defaults();
        // the one port setting that differs from upstream's search: walking on the top layer (U15)
        want.search.walk_on_build_limit = true;
        assert_eq!(format!("{c:?}"), format!("{want:?}"));
        let mut e = ExecSettings { allow_sprint: false, max_cost_increase: 0.0, ..ExecSettings::default() };
        s.apply_to_exec(&mut e);
        assert_eq!(e, ExecSettings::default());
        assert_eq!(s.tool_settings(), ToolSettings::default());
        assert!(s.modified().is_empty());
    }

    #[test]
    fn names_defaults_and_types_follow_settings_java() {
        let s = Settings::new();
        // spot checks against Settings.java at 1b8e87c
        for (name, v) in [
            ("allowBreak", "true"),
            ("allowPlace", "true"),
            ("allowSprint", "true"),
            ("allowParkour", "false"),
            ("allowWaterBucketFall", "true"),
            ("allowDiagonalDescend", "false"),
            ("allowDiagonalAscend", "false"),
            ("allowDownward", "true"),
            ("maxFallHeightNoWater", "3"),
            ("maxFallHeightBucket", "20"),
            ("blocksToAvoid", "tripwire"),
            ("acceptableThrowawayItems", "minecraft:dirt,minecraft:cobblestone,minecraft:netherrack,minecraft:stone"),
            ("primaryTimeoutMS", "500"),
            ("failureTimeoutMS", "2000"),
            ("planAheadPrimaryTimeoutMS", "4000"),
            ("planAheadFailureTimeoutMS", "5000"),
            ("costHeuristic", "3.563"),
            ("renderPath", "true"),
            ("freeLook", "true"),
            ("blockFreeLook", "false"),
            ("antiCheatCompatibility", "true"),
            ("smoothLook", "false"),
            ("smoothLookTicks", "5"),
            ("maxCostIncrease", "10.0"),
            ("pathCutoffFactor", "0.9"),
            ("avoidBreakingMultiplier", "0.1"),
            ("blockPlacementPenalty", "20.0"),
            ("followRadius", "3"),
            ("followOffsetDistance", "0.0"),
            ("followOffsetDirection", "0.0"),
            ("followTargetMaxDistance", "0"),
            ("exploreChunkSetMinimumSize", "10"),
            ("worldExploringChunkOffset", "0"),
            ("exploreMaintainY", "64"),
            ("disableCompletionCheck", "false"),
        ] {
            assert_eq!(value_to_string(s.get(name)), v, "{name}");
        }
        assert_eq!(s.get("primaryTimeoutMS").type_name(), "Long");
        assert_eq!(s.get("blocksToAvoid").type_name(), "List<Block>");
        assert_eq!(s.get("acceptableThrowawayItems").type_name(), "List<Item>");
        // names are unique, ignoring case
        let mut names: Vec<String> = s.all().iter().map(|x| x.name.to_lowercase()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
    }

    #[test]
    fn java_prints_doubles_its_own_way() {
        for (v, want) in [(2.0, "2.0"), (0.1, "0.1"), (3.563, "3.563"), (10.0, "10.0"), (120.0, "120.0"), (1.0e7, "1.0E7"), (1.5e-4, "1.5E-4"), (0.001, "0.001"), (123456.789, "123456.789"), (-0.5, "-0.5"), (0.0, "0.0"), (12345678.0, "1.2345678E7"), (1.0e21, "1.0E21")] {
            assert_eq!(java_double_to_string(v), want, "{v}");
        }
        assert_eq!(java_double_to_string(f64::NAN), "NaN");
        assert_eq!(java_double_to_string(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(java_float_to_string(4.5), "4.5");
        assert_eq!(java_float_to_string(1.0e-5), "1.0E-5");
    }

    #[test]
    fn numbers_are_read_as_java_reads_them() {
        let mut s = Settings::new();
        assert!(s.parse_and_apply("costHeuristic", "2", &AnyName).is_ok());
        assert_eq!(s.get_double("costHeuristic"), 2.0);
        assert!(s.parse_and_apply("costHeuristic", "1e2", &AnyName).is_ok());
        assert_eq!(s.get_double("costHeuristic"), 100.0);
        assert!(s.parse_and_apply("costHeuristic", " 2.5d ", &AnyName).is_ok());
        assert_eq!(s.get_double("costHeuristic"), 2.5);
        assert!(s.parse_and_apply("costHeuristic", "abc", &AnyName).is_err());
        assert!(s.parse_and_apply("maxFallHeightNoWater", "5", &AnyName).is_ok());
        assert!(s.parse_and_apply("maxFallHeightNoWater", "5.5", &AnyName).is_err());
        assert!(s.parse_and_apply("maxFallHeightNoWater", "99999999999", &AnyName).is_err());
        assert!(s.parse_and_apply("maxFallHeightNoWater", "+7", &AnyName).is_ok());
        assert_eq!(s.get_int("maxFallHeightNoWater"), 7);
        assert!(s.parse_and_apply("primaryTimeoutMS", "99999999999", &AnyName).is_ok());
        assert_eq!(s.get_long("primaryTimeoutMS"), 99_999_999_999);
        // a boolean that is not "true" is false, as Boolean.parseBoolean says
        assert!(s.parse_and_apply("allowBreak", "TRUE", &AnyName).is_ok());
        assert!(s.get_bool("allowBreak"));
        assert!(s.parse_and_apply("allowBreak", "yes", &AnyName).is_ok());
        assert!(!s.get_bool("allowBreak"));
    }

    #[test]
    fn lists_are_comma_separated_and_checked_against_the_registry() {
        struct Only(&'static [&'static str]);
        impl Registry for Only {
            fn has_block(&self, p: &str) -> bool {
                self.0.contains(&p)
            }
            fn has_item(&self, n: &str) -> bool {
                self.0.contains(&n)
            }
        }
        let mut s = Settings::new();
        let reg = Only(&["stone", "dirt", "minecraft:dirt", "minecraft:gravel"]);
        assert!(s.parse_and_apply("blocksToAvoid", "minecraft:stone,dirt", &reg).is_ok());
        assert_eq!(s.get_list("blocksToAvoid"), ["stone", "dirt"]);
        assert!(s.parse_and_apply("blocksToAvoid", "stone,nonsense", &reg).is_err());
        assert_eq!(s.get_list("blocksToAvoid"), ["stone", "dirt"], "a failed parse changes nothing");
        assert!(s.parse_and_apply("acceptableThrowawayItems", "dirt,minecraft:gravel", &reg).is_ok());
        assert_eq!(s.get_list("acceptableThrowawayItems"), ["minecraft:dirt", "minecraft:gravel"]);
        assert_eq!(s.tool_settings().acceptable_throwaway_items, ["dirt", "gravel"]);
        // an empty list cannot be typed: the text is one empty name
        assert!(s.parse_and_apply("blocksToAvoid", "", &reg).is_err());
    }

    /// `allowInventory`, `ticksBetweenInventoryMoves` and `inventoryMoveOnlyIfStationary`: upstream's names and defaults, and `ToolSettings` reads them.
    #[test]
    fn the_inventory_move_settings_have_upstreams_defaults_and_reach_the_tool_settings() {
        let mut s = Settings::new();
        let ts = s.tool_settings();
        assert!(!ts.allow_inventory);
        assert_eq!(ts.ticks_between_inventory_moves, 1);
        assert!(!ts.inventory_move_only_if_stationary);
        assert_eq!(run(&mut s, "allowinventory").lines, ["Value of setting allowInventory:", "false"]);
        assert_eq!(run(&mut s, "allowInventory true").lines, ["Successfully set allowInventory to true", "Old value: false"]);
        assert_eq!(run(&mut s, "ticksBetweenInventoryMoves 4").lines, ["Successfully set ticksBetweenInventoryMoves to 4", "Old value: 1"]);
        assert_eq!(run(&mut s, "inventoryMoveOnlyIfStationary true").lines, ["Successfully set inventoryMoveOnlyIfStationary to true", "Old value: false"]);
        let ts = s.tool_settings();
        assert!(ts.allow_inventory && ts.inventory_move_only_if_stationary);
        assert_eq!(ts.ticks_between_inventory_moves, 4);
        // `CalculationContext.<init>`: `hasThrowaway` sees the main inventory only with the setting
        let mut inv = crate::inventory::testing::TestInventory::empty();
        inv.put_main(15, crate::inventory::SlotInfo::of("dirt", 3), &[]);
        let mut c = crate::testing::test_config();
        s.apply_inventory(&mut c, &inv, false);
        assert!(c.has_throwaway, "allowPlace is on by default, and allowInventory now");
        run(&mut s, "allowInventory false");
        s.apply_inventory(&mut c, &inv, false);
        assert!(!c.has_throwaway);
    }

    #[test]
    fn set_shows_sets_toggles_and_resets_like_upstream() {
        let mut s = Settings::new();
        let o = run(&mut s, "allowbreak");
        assert_eq!(o.lines, ["Value of setting allowBreak:", "true"]);
        let o = run(&mut s, "ALLOWBREAK false");
        assert_eq!(o.lines, ["Successfully set allowBreak to false", "Old value: true"]);
        assert_eq!(o.effect, SetEffect::Changed);
        assert!(!s.get_bool("allowBreak"));
        let o = run(&mut s, "toggle allowBreak");
        assert_eq!(o.lines, ["Toggled setting allowBreak to true", "Old value: false"]);
        assert!(s.get_bool("allowBreak"));
        run(&mut s, "costHeuristic 4");
        let o = run(&mut s, "reset costHeuristic");
        assert_eq!(o.lines, ["Successfully reset costHeuristic to 3.563", "Old value: 4.0"]);
        // reset with nothing: the warning, and no change
        let o = run(&mut s, "reset");
        assert_eq!(o.lines.len(), 3);
        assert_eq!(o.error.as_deref(), Some("Not enough arguments (expected at least 1)"));
        run(&mut s, "primaryTimeoutMS 10");
        run(&mut s, "allowParkour true");
        let o = run(&mut s, "reset all");
        assert_eq!(o.lines, ["All settings have been reset to their default values"]);
        assert!(s.modified().is_empty());
        // errors
        assert_eq!(run(&mut s, "nonsense").error.as_deref(), Some("Expected a valid setting"));
        assert!(run(&mut s, "toggle costHeuristic").error.as_deref().unwrap().starts_with("Expected a toggleable setting"));
        assert!(run(&mut s, "toggle").error.is_some());
        assert!(run(&mut s, "costHeuristic abc").error.is_some());
        assert!(run(&mut s, "allowBreak true extra").error.is_some());
        assert_eq!(s.get_double("costHeuristic"), 3.563, "a failed set changes nothing");
    }

    #[test]
    fn set_lists_pages_of_eight_and_filters() {
        let mut s = Settings::new();
        let o = run(&mut s, "list");
        assert_eq!(o.lines[0], "All settings:");
        assert_eq!(o.lines.len(), 1 + 8 + 1);
        assert!(o.lines[1].ends_with("(Boolean)") || o.lines[1].contains('('));
        assert!(o.lines.last().unwrap().ends_with(&format!("1/{}", (s.all().len() - 1) / 8 + 1)));
        let o = run(&mut s, "list allowParkour");
        assert_eq!(o.lines[0], "All settings containing the string 'allowParkour':");
        assert_eq!(&o.lines[1..4], ["allowParkour (Boolean)", "allowParkourAscend (Boolean)", "allowParkourPlace (Boolean)"]);
        assert_eq!(o.lines[4], "--");
        assert!(run(&mut s, "list 999").error.is_some());
        // modified shows only what changed
        run(&mut s, "allowParkour true");
        let o = run(&mut s, "modified");
        assert_eq!(o.lines[0], "All modified settings:");
        assert_eq!(o.lines[1], "allowParkour (Boolean)");
        assert_eq!(o.lines[2], "--");
        // no argument is the list
        assert_eq!(run(&mut s, "").lines[0], "All settings:");
    }

    #[test]
    fn save_and_load_go_through_the_glue_as_text() {
        let mut s = Settings::new();
        run(&mut s, "allowParkour true");
        run(&mut s, "costHeuristic 2");
        run(&mut s, "blocksToAvoid tripwire,cactus");
        let text = s.to_file_text();
        assert_eq!(text, "allowParkour true\nblocksToAvoid tripwire,cactus\ncostHeuristic 2.0\n");
        assert_eq!(run(&mut s, "save").effect, SetEffect::Save);
        assert_eq!(run(&mut s, "load other.txt").effect, SetEffect::Load("other.txt".to_string()));
        let mut t = Settings::new();
        let problems = t.apply_file_text(&format!("# a comment\n// another\n\n{text}allowjumpat256 true\nbroken\nnoSuchSetting 1\ncostHeuristic nope\n"), &AnyName);
        assert!(t.get_bool("allowParkour"));
        assert!(t.get_bool("allowJumpAtBuildLimit"));
        assert_eq!(t.get_list("blocksToAvoid"), ["tripwire", "cactus"]);
        assert_eq!(problems, ["Invalid syntax in setting file: broken", "Unable to parse line noSuchSetting 1", "Unable to parse line costHeuristic nope"]);
        assert_eq!(t.get_double("costHeuristic"), 2.0);
        // upstream's pattern: a leading space or a name with no value is invalid syntax; two spaces after a name give a value of one space
        let problems = t.apply_file_text(" costHeuristic 3\ncostHeuristic \nnoSuchSetting  \n", &AnyName);
        assert_eq!(problems, ["Invalid syntax in setting file:  costHeuristic 3", "Invalid syntax in setting file: costHeuristic ", "Unable to parse line noSuchSetting  "]);
    }

    #[test]
    fn the_inventory_decides_throwaway_and_the_bucket() {
        use crate::inventory::testing::TestInventory;
        use crate::inventory::SlotInfo;
        let mut s = Settings::new();
        let mut c = Config::upstream_defaults();
        let mut inv = TestInventory::empty();
        s.apply_inventory(&mut c, &inv, false);
        assert!(!c.has_throwaway && !c.has_water_bucket);
        inv.put(2, SlotInfo::of("cobblestone", 8), &[]);
        inv.put(7, SlotInfo::of("water_bucket", 1), &[]);
        s.apply_inventory(&mut c, &inv, false);
        assert!(c.has_throwaway && c.has_water_bucket);
        // the Nether takes the bucket away, allowPlace the blocks, allowWaterBucketFall the bucket
        s.apply_inventory(&mut c, &inv, true);
        assert!(c.has_throwaway && !c.has_water_bucket);
        run(&mut s, "allowPlace false");
        run(&mut s, "allowWaterBucketFall false");
        s.apply_inventory(&mut c, &inv, false);
        assert!(!c.has_throwaway && !c.has_water_bucket);
    }

    #[test]
    fn tab_completion_follows_upstream() {
        let mut s = Settings::new();
        let first = s.complete(&[], "");
        assert_eq!(&first[..6], ["list", "modified", "reset", "toggle", "save", "load"]);
        assert!(first.contains(&"allowBreak".to_string()));
        assert_eq!(s.complete(&[], "allowParkourA"), ["allowParkourAscend"]);
        assert_eq!(s.complete(&[], "RES"), ["reset"]);
        // a boolean offers the other value last
        assert_eq!(s.complete(&["allowBreak"], ""), ["true", "false"]);
        assert_eq!(s.complete(&["allowParkour"], ""), ["false", "true"]);
        assert_eq!(s.complete(&["allowParkour"], "t"), ["true"]);
        // other types show the value
        assert_eq!(s.complete(&["costHeuristic"], ""), ["3.563"]);
        // toggle offers booleans; reset offers all and the modified ones
        let t = s.complete(&["toggle"], "allowDi");
        assert_eq!(t, ["allowDiagonalAscend", "allowDiagonalDescend"]);
        run(&mut s, "costHeuristic 4");
        assert_eq!(s.complete(&["reset"], ""), ["all", "costHeuristic"]);
        assert!(s.complete(&["nonsense"], "").is_empty());
        assert!(s.complete(&["allowBreak", "x"], "").is_empty());
    }

    // ------------------------------------------------------------ the builder's settings (D49)

    struct Known(&'static [&'static str]);
    impl Registry for Known {
        fn has_block(&self, path: &str) -> bool {
            self.0.contains(&path)
        }
        fn has_item(&self, _name: &str) -> bool {
            true
        }
    }
    const BLOCKS: Known = Known(&["stone", "cobblestone", "dirt", "oak_planks", "glass", "mod:thing"]);

    #[test]
    fn the_builder_settings_have_upstreams_names_types_and_defaults() {
        let s = Settings::new();
        for (name, ty, v) in [
            ("buildInLayers", "Boolean", "false"),
            ("layerOrder", "Boolean", "false"),
            ("layerHeight", "Integer", "1"),
            ("startAtLayer", "Integer", "0"),
            ("skipFailedLayers", "Boolean", "false"),
            ("buildRepeat", "Vec3i", "0,0,0"),
            ("buildRepeatCount", "Integer", "-1"),
            ("buildRepeatSneaky", "Boolean", "true"),
            ("breakFromAbove", "Boolean", "false"),
            ("goalBreakFromAbove", "Boolean", "false"),
            ("mapArtMode", "Boolean", "false"),
            ("okIfWater", "Boolean", "false"),
            ("incorrectSize", "Integer", "100"),
            ("breakCorrectBlockPenaltyMultiplier", "Double", "10.0"),
            ("placeIncorrectBlockPenaltyMultiplier", "Double", "2.0"),
            ("schematicOrientationX", "Boolean", "false"),
            ("schematicOrientationY", "Boolean", "false"),
            ("schematicOrientationZ", "Boolean", "false"),
            ("buildSchematicRotation", "Rotation", "NONE"),
            ("buildSchematicMirror", "Mirror", "NONE"),
            ("builderTickScanRadius", "Integer", "5"),
            ("distanceTrim", "Boolean", "true"),
            ("buildIgnoreBlocks", "List<Block>", ""),
            ("buildSkipBlocks", "List<Block>", ""),
            ("okIfAir", "List<Block>", ""),
            ("buildValidSubstitutes", "Map<Block, List<Block>>", ""),
            ("buildSubstitutes", "Map<Block, List<Block>>", ""),
            ("buildIgnoreExisting", "Boolean", "false"),
            ("buildIgnoreDirection", "Boolean", "false"),
            ("buildIgnoreProperties", "List<String>", ""),
            ("notificationOnBuildFinished", "Boolean", "true"),
            ("schematicFallbackExtension", "String", "schematic"),
        ] {
            let st = s.find(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(st.value.type_name(), ty, "{name}");
            assert_eq!(value_to_string(&st.value), v, "{name}");
        }
        assert_eq!(s.build_settings(), BuildSettings::default());
        assert!(s.modified().is_empty());
    }

    #[test]
    fn the_builder_settings_round_trip_through_text_and_the_settings_file() {
        let mut s = Settings::new();
        for (name, text) in [
            ("buildRepeat", "6,0,-2"),
            ("buildSchematicRotation", "CLOCKWISE_90"),
            ("buildSchematicMirror", "FRONT_BACK"),
            ("buildSubstitutes", "stone->cobblestone,dirt,oak_planks->glass"),
            ("buildValidSubstitutes", "stone->dirt"),
            ("buildIgnoreProperties", "waterlogged,facing"),
            ("buildSkipBlocks", "stone,mod:thing"),
            ("schematicFallbackExtension", "schem"),
            ("buildInLayers", "true"),
            ("layerHeight", "2"),
            ("breakCorrectBlockPenaltyMultiplier", "7.5"),
        ] {
            s.parse_and_apply(name, text, &BLOCKS).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(value_to_string(s.get(name)), text, "{name}");
        }
        let bs = s.build_settings();
        assert_eq!(bs.build_repeat, (6, 0, -2));
        assert_eq!(bs.build_schematic_rotation, BlockRotation::Clockwise90);
        assert_eq!(bs.build_schematic_mirror, BlockMirror::FrontBack);
        assert_eq!(bs.build_substitutes, vec![("stone".to_string(), vec!["cobblestone".to_string(), "dirt".to_string()]), ("oak_planks".to_string(), vec!["glass".to_string()])]);
        assert_eq!(bs.build_ignore_properties, vec!["waterlogged".to_string(), "facing".to_string()]);
        assert_eq!(bs.build_skip_blocks, vec!["stone".to_string(), "mod:thing".to_string()]);
        assert!(bs.build_in_layers);
        assert_eq!(bs.layer_height, 2);
        // the file the glue saves reads back to the same values
        let text = s.to_file_text();
        let mut t = Settings::new();
        assert!(t.apply_file_text(&text, &BLOCKS).is_empty());
        assert_eq!(t, s);
        assert_eq!(t.build_settings(), bs);
    }

    #[test]
    fn a_namespaced_minecraft_block_in_a_map_is_stored_by_its_path() {
        let mut s = Settings::new();
        s.parse_and_apply("buildSubstitutes", "minecraft:stone->minecraft:dirt,cobblestone", &BLOCKS).unwrap();
        assert_eq!(value_to_string(s.get("buildSubstitutes")), "stone->dirt,cobblestone");
    }

    #[test]
    fn bad_builder_values_are_refused() {
        let mut s = Settings::new();
        for (name, text) in [
            ("buildSubstitutes", "stone->nosuchblock"),
            ("buildSubstitutes", "nosuchblock->stone"),
            ("buildSubstitutes", "stone->dirt,stone->glass"), // a repeated key: Collectors.toMap throws
            ("buildSubstitutes", "stone"),                    // no "->": ArrayIndexOutOfBounds
            ("buildSubstitutes", "stone->"),                  // no value after the arrow
            ("buildSkipBlocks", "stone,nosuchblock"),
            ("buildRepeat", "1,2"),
            ("buildRepeat", "1,2,x"),
            ("buildRepeat", "1, 2, 3"), // parseInt does not trim
            ("buildRepeat", "1,2,99999999999"),
            ("buildSchematicRotation", "clockwise_90"),
            ("buildSchematicRotation", "CLOCKWISE_45"),
            ("buildSchematicMirror", "left_right"),
            ("buildSchematicMirror", "UP_DOWN"),
            ("layerHeight", "two"),
        ] {
            assert!(s.parse_and_apply(name, text, &BLOCKS).is_err(), "{name} {text}");
        }
        assert!(s.modified().is_empty());
        // through the command: a clear error, the value unchanged
        let out = s.run_set(&["buildSchematicRotation", "clockwise_90"], &BLOCKS);
        assert!(out.error.is_some_and(|e| e.starts_with("Expected a valid value")));
        // extra parts of a Vec3i are ignored, as `split(",")[i]` does
        s.parse_and_apply("buildRepeat", "1,2,3,4", &BLOCKS).unwrap();
        assert_eq!(value_to_string(s.get("buildRepeat")), "1,2,3");
    }

    #[test]
    fn map_pieces_split_where_java_splits() {
        assert_eq!(split_map_pieces("a->b,c->d"), vec!["a->b", "c->d"]);
        assert_eq!(split_map_pieces("a->b,c,d->e"), vec!["a->b,c", "d->e"]);
        assert_eq!(split_map_pieces("a->b,"), vec!["a->b,"]);
        assert_eq!(split_map_pieces(""), vec![""]);
        assert_eq!(split_map_pieces("a,b->c"), vec!["a", "b->c"]);
    }

    #[test]
    fn set_prints_the_new_types_and_toggles_nothing_else() {
        let mut s = Settings::new();
        let out = run(&mut s, "buildRepeat");
        assert_eq!(out.lines, vec!["Value of setting buildRepeat:".to_string(), "0,0,0".to_string()]);
        let out = s.run_set(&["buildRepeat", "6,0,0"], &BLOCKS);
        assert!(out.error.is_none());
        assert!(out.lines.contains(&"Successfully set buildRepeat to 6,0,0".to_string()));
        assert!(out.lines.contains(&"Old value: 0,0,0".to_string()));
        let out = s.run_set(&["toggle", "buildRepeat"], &BLOCKS);
        assert_eq!(out.error.as_deref(), Some("Expected a toggleable setting, but got some other setting instead"));
        let out = run(&mut s, "list buildschematic");
        assert!(out.lines.iter().any(|l| l == "buildSchematicMirror (Mirror)"));
        assert!(out.lines.iter().any(|l| l == "buildSchematicRotation (Rotation)"));
        let out = run(&mut s, "list buildsub");
        assert!(out.lines.iter().any(|l| l == "buildSubstitutes (Map<Block, List<Block>>)"));
    }
}
