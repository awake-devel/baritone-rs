// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/process/ExploreProcess.java (and the data type MyChunkPos it reads a filter into).
// LGPL-3.0-or-later, see lib.rs.
//
// `explore`: the goal is a `GoalComposite` of the centres of the closest chunks the cache does not hold, asked again every tick (`FORCE_REVALIDATE_GOAL_AND_PATH`),
// so that it moves on as the chunks the player walks to get cached. A filter (`explorefilter`) lists chunks that count as explored, or, inverted, the only
// chunks that count as not explored. The process reads the cache through [`WorldView::chunk_status`], which also starts the read of a region file when the
// region is not in memory yet; while that read is out the process pauses (upstream: `REQUEST_PAUSE`).
use super::custom_goal::{PathingCommand, PathingCommandType};
use crate::goals::Goal;
use crate::settings::ExploreSettings;
use crate::world::{ChunkStatus, WorldView};
use std::collections::HashSet;

/// What `explorefilter` says when its file is not a list of chunks.
pub const INVALID_JSON: &str = "Invalid JSON syntax";

/// Upstream's `ExploreProcess.JsonChunkFilter`: a list of chunks from a file, `[{"x":0,"z":0},...]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonChunkFilter {
    /// If true, the list is the chunks that are NOT explored; if false, the chunks that ARE explored.
    invert: bool,
    in_filter: HashSet<(i32, i32)>,
    /// The list as it was read, with repeats (`countRemain` walks it, not the set).
    positions: Vec<(i32, i32)>,
}

impl JsonChunkFilter {
    /// `ExploreProcess.JsonChunkFilter.<init>`, from the text of the file. (Upstream opens the file itself and lets Gson read a `MyChunkPos[]`: the glue
    /// reads the file, and a text that is not a list of objects is an error here where upstream throws.) A chunk without `x` or `z` has 0 there, as Gson leaves a field it
    /// did not read. Other members are ignored.
    pub fn parse(text: &str, invert: bool) -> Result<JsonChunkFilter, String> {
        let positions = chunk_list(text)?;
        let in_filter = positions.iter().copied().collect();
        Ok(JsonChunkFilter { invert, in_filter, positions })
    }

    /// How many positions were read, for the line `Loaded N positions` (`logDirect`).
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    pub fn invert(&self) -> bool {
        self.invert
    }

    /// `ExploreProcess.JsonChunkFilter.isAlreadyExplored`: explored when it is on the list of explored chunks, or not on the list of unexplored ones; otherwise it
    /// depends on the cache, which is [`ChunkStatus::Unknown`] here and left to [`ExploreProcess`] (upstream has the same answer and `EitherChunk` falls back).
    fn is_already_explored(&self, chunk_x: i32, chunk_z: i32) -> ChunkStatus {
        if self.in_filter.contains(&(chunk_x, chunk_z)) ^ self.invert {
            ChunkStatus::Explored
        } else {
            ChunkStatus::Unknown
        }
    }

    /// `ExploreProcess.JsonChunkFilter.countRemain`: with the list of explored chunks, anything not on it is unexplored (`i32::MAX`); inverted, everything not on the
    /// list is assumed cached, so the count is how many chunks of the list the cache lacks, stopping at `minimum`.
    fn count_remain<W: WorldView>(&self, world: &W, minimum: i32) -> i32 {
        if !self.invert {
            return i32::MAX;
        }
        let mut count_remain = 0;
        for &(x, z) in &self.positions {
            if world.chunk_status(x, z) != ChunkStatus::Explored {
                // either waiting for it or dont have it at all
                count_remain += 1;
                if count_remain >= minimum {
                    return count_remain;
                }
            }
        }
        count_remain
    }
}

/// `ExploreProcess`.
#[derive(Clone, Debug, Default)]
pub struct ExploreProcess {
    /// The block position of the centre, y 0 (`explorationOrigin`); `None` when not exploring.
    origin: Option<(i32, i32)>,
    filter: Option<JsonChunkFilter>,
    distance_completed: i32,
    messages: Vec<String>,
}

impl ExploreProcess {
    /// `ExploreProcess.<init>`: `ExploreProcess::new`; not exploring, no filter.
    pub fn new() -> ExploreProcess {
        ExploreProcess::default()
    }

    /// `ExploreProcess.isActive`: there is a centre.
    pub fn is_active(&self) -> bool {
        self.origin.is_some()
    }

    /// `ExploreProcess.explore`: explore around this block column, from the nearest chunks.
    pub fn explore(&mut self, center_x: i32, center_z: i32) {
        self.origin = Some((center_x, center_z));
        self.distance_completed = 0;
    }

    /// `ExploreProcess.applyJsonFilter`: the filter replaces the one before. It is kept when exploring ends (only the centre is forgotten).
    pub fn apply_json_filter(&mut self, filter: JsonChunkFilter) {
        self.messages.push(format!("Loaded {} positions", filter.len()));
        self.filter = Some(filter);
    }

    /// The filter, if one is applied.
    pub fn filter(&self) -> Option<&JsonChunkFilter> {
        self.filter.as_ref()
    }

    /// The centre being explored around, as the block position (x, z).
    pub fn origin(&self) -> Option<(i32, i32)> {
        self.origin
    }

    /// The chunk distance (Manhattan) from the centre up to which every chunk was found explored.
    pub fn distance_completed(&self) -> i32 {
        self.distance_completed
    }

    /// The chat lines the process has said since the last call (`logDirect`).
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    /// `ExploreProcess.EitherChunk.isAlreadyExplored`: the filter's answer if it says explored, else the cache's (`BaritoneChunkCache.isAlreadyExplored`, which is
    /// [`WorldView::chunk_status`]).
    /// `ExploreProcess.calcFilter`: builds that either-filter when a filter is applied, and the cache alone when not; here both ask `self.filter`.
    fn is_already_explored<W: WorldView>(&self, world: &W, chunk_x: i32, chunk_z: i32) -> ChunkStatus {
        if let Some(f) = &self.filter {
            if f.is_already_explored(chunk_x, chunk_z) == ChunkStatus::Explored {
                return ChunkStatus::Explored;
            }
        }
        world.chunk_status(chunk_x, chunk_z)
    }

    /// `ExploreProcess.EitherChunk.countRemain`: the smaller of the two counts.
    /// `ExploreProcess.BaritoneChunkCache.countRemain`: `Integer.MAX_VALUE`, which is the count when no filter is applied.
    fn count_remain<W: WorldView>(&self, world: &W, s: &ExploreSettings) -> i32 {
        match &self.filter {
            Some(f) => f.count_remain(world, s.explore_chunk_set_minimum_size), // min with the cache's `i32::MAX` is the filter's own count
            None => i32::MAX,
        }
    }

    /// `ExploreProcess.onTick`: `None` is upstream's `null` (the process is done, or failed). `world` is the world the cache sits under.
    pub fn on_tick<W: WorldView>(&mut self, calc_failed: bool, world: &W, s: &ExploreSettings) -> Option<PathingCommand> {
        if calc_failed {
            self.messages.push("Failed".to_string());
            self.on_lost_control();
            return None;
        }
        if !s.disable_completion_check && self.count_remain(world, s) == 0 {
            self.messages.push("Explored all chunks".to_string());
            self.on_lost_control();
            return None;
        }
        let origin = self.origin?;
        let Some(closest) = self.closest_uncached_chunks(origin, world, s) else {
            // awaiting region load from disk
            return Some(PathingCommand { goal: None, kind: PathingCommandType::RequestPause });
        };
        Some(PathingCommand { goal: Some(Goal::Composite(closest)), kind: PathingCommandType::ForceRevalidateGoalAndPath })
    }

    /// `ExploreProcess.closestUncachedChunks`: the goals for the closest chunks that are not explored, by the Manhattan distance of the chunk from the one the centre
    /// is in, ring by ring, at least `exploreChunkSetMinimumSize` of them (or all that remain, if fewer). `None` is upstream's `null`: a region is being read.
    fn closest_uncached_chunks<W: WorldView>(&mut self, center: (i32, i32), world: &W, s: &ExploreSettings) -> Option<Vec<Goal>> {
        let chunk_x = center.0 >> 4;
        let chunk_z = center.1 >> 4;
        let mut count = self.count_remain(world, s).min(s.explore_chunk_set_minimum_size);
        let mut centers: Vec<(i32, i32)> = vec![];
        let render_distance = s.world_exploring_chunk_offset;
        let mut dist = self.distance_completed;
        loop {
            for dx in -dist..=dist {
                let zval = dist - dx.abs();
                for mult in 0..2 {
                    let dz = (mult * 2 - 1) * zval; // dz can be either -zval or zval
                    match self.is_already_explored(world, chunk_x.wrapping_add(dx), chunk_z.wrapping_add(dz)) {
                        ChunkStatus::Unknown => return None, // awaiting load
                        ChunkStatus::NotExplored => {}
                        ChunkStatus::Explored => continue,
                    }
                    let mut center_x = chunk_x.wrapping_add(dx).wrapping_shl(4).wrapping_add(8);
                    let mut center_z = chunk_z.wrapping_add(dz).wrapping_shl(4).wrapping_add(8);
                    let offset = render_distance.wrapping_shl(4);
                    center_x = if dx < 0 { center_x.wrapping_sub(offset) } else { center_x.wrapping_add(offset) };
                    center_z = if dz < 0 { center_z.wrapping_sub(offset) } else { center_z.wrapping_add(offset) };
                    centers.push((center_x, center_z));
                }
            }
            if dist % 10 == 0 {
                count = self.count_remain(world, s).min(s.explore_chunk_set_minimum_size);
            }
            if centers.len() as i64 >= i64::from(count) {
                return Some(centers.into_iter().map(|(x, z)| create_goal(x, z, s)).collect());
            }
            if centers.is_empty() {
                // we have explored everything from 0 to dist inclusive
                // next time we should start our check at dist+1
                self.distance_completed = dist + 1;
            }
            dist = dist.wrapping_add(1);
        }
    }

    /// `ExploreProcess.onLostControl`: forgets the centre. The filter stays.
    pub fn on_lost_control(&mut self) {
        self.origin = None;
    }

    /// `ExploreProcess.displayName0`: upstream ends with the goals of `closestUncachedChunks`, and throws if a region is being read; here that case says `null`.
    pub fn display_name<W: WorldView>(&mut self, world: &W, s: &ExploreSettings) -> String {
        let Some(origin) = self.origin else {
            return "Exploring".to_string();
        };
        let going = match self.closest_uncached_chunks(origin, world, s) {
            Some(goals) => format!("GoalComposite[{}]", goals.iter().map(|g| crate::command::goal_to_string(g, false)).collect::<Vec<_>>().join(", ")),
            None => "null".to_string(),
        };
        format!("Exploring around BlockPos{{x={}, y=0, z={}}}, distance completed {}, currently going to {going}", origin.0, origin.1, self.distance_completed)
    }
}

/// `ExploreProcess.createGoal`: a `GoalXZ`; with `exploreMaintainY` not -1, one whose heuristic also pulls towards that height. It is not a `GoalBlock` because
/// `isInGoal` should be true whatever the height, as long as X and Z are right.
fn create_goal(x: i32, z: i32, s: &ExploreSettings) -> Goal {
    if s.explore_maintain_y == -1 {
        return Goal::Xz { x, z };
    }
    Goal::XzMaintainY { x, z, level: s.explore_maintain_y }
}

// ---------------------------------------------------------------- the file

/// The deepest nesting the reader follows. A list of chunks is two deep; anything deeper is not one, and a hostile file cannot make the reader recurse far.
const MAX_DEPTH: usize = 32;

/// Reads `[{"x":int,"z":int},...]` as Gson would read a `MyChunkPos[]` (members of other names are skipped; a missing `x` or `z` is 0).
fn chunk_list(text: &str) -> Result<Vec<(i32, i32)>, String> {
    let mut r = Reader { b: text.as_bytes(), i: 0 };
    r.skip_ws();
    // a byte order mark is not part of the text (Gson's reader skips it too)
    if r.b[r.i..].starts_with(&[0xEF, 0xBB, 0xBF]) {
        r.i += 3;
        r.skip_ws();
    }
    r.expect(b'[')?;
    let mut out = vec![];
    r.skip_ws();
    if r.peek() == Some(b']') {
        r.i += 1;
    } else {
        loop {
            r.skip_ws();
            out.push(r.chunk_object()?);
            r.skip_ws();
            match r.next() {
                Some(b',') => {}
                Some(b']') => break,
                _ => return Err(INVALID_JSON.to_string()),
            }
        }
    }
    r.skip_ws();
    if r.i != r.b.len() {
        return Err(INVALID_JSON.to_string()); // text after the list
    }
    Ok(out)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.next() == Some(c) {
            Ok(())
        } else {
            Err(INVALID_JSON.to_string())
        }
    }

    /// `{...}` with the members `x` and `z` as integers; the last one of a name wins (Gson sets the field again).
    fn chunk_object(&mut self) -> Result<(i32, i32), String> {
        self.expect(b'{')?;
        let (mut x, mut z) = (0, 0);
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok((x, z));
        }
        loop {
            self.skip_ws();
            let name = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            match name.as_str() {
                "x" => x = self.int()?,
                "z" => z = self.int()?,
                _ => self.skip_value(1)?,
            }
            self.skip_ws();
            match self.next() {
                Some(b',') => {}
                Some(b'}') => return Ok((x, z)),
                _ => return Err(INVALID_JSON.to_string()),
            }
        }
    }

    /// A string, with its escapes. Only the member names are looked at, so a bad escape is an error but the text is not kept past that.
    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out: Vec<u8> = vec![];
        loop {
            match self.next() {
                None => return Err(INVALID_JSON.to_string()),
                Some(b'"') => break,
                Some(b'\\') => match self.next() {
                    Some(c @ (b'"' | b'\\' | b'/')) => out.push(c),
                    Some(b'b') => out.push(8),
                    Some(b'f') => out.push(12),
                    Some(b'n') => out.push(b'\n'),
                    Some(b'r') => out.push(b'\r'),
                    Some(b't') => out.push(b'\t'),
                    Some(b'u') => {
                        let hex = self.b.get(self.i..self.i + 4).ok_or_else(|| INVALID_JSON.to_string())?;
                        let hex = std::str::from_utf8(hex).map_err(|_| INVALID_JSON.to_string())?;
                        let unit = u32::from_str_radix(hex, 16).map_err(|_| INVALID_JSON.to_string())?;
                        self.i += 4;
                        // a name that is not "x" or "z" never matters; a lone surrogate is kept as a replacement character
                        let c = char::from_u32(unit).unwrap_or('\u{FFFD}');
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                    _ => return Err(INVALID_JSON.to_string()),
                },
                Some(c) if c < 0x20 => return Err(INVALID_JSON.to_string()),
                Some(c) => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| INVALID_JSON.to_string())
    }

    /// An integer: a JSON number whose value is whole and fits an `int` (Gson's `nextInt` takes `3.0` and `3e0` too).
    fn int(&mut self) -> Result<i32, String> {
        let start = self.i;
        while matches!(self.peek(), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
            self.i += 1;
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| INVALID_JSON.to_string())?;
        if let Ok(n) = text.parse::<i32>() {
            // `+5` and `05` are not JSON numbers
            if text.starts_with('+') || (text.trim_start_matches('-').len() > 1 && text.trim_start_matches('-').starts_with('0')) {
                return Err(INVALID_JSON.to_string());
            }
            return Ok(n);
        }
        let d: f64 = text.parse().map_err(|_| INVALID_JSON.to_string())?;
        if text.starts_with('+') || !d.is_finite() || d.fract() != 0.0 || d < f64::from(i32::MIN) || d > f64::from(i32::MAX) {
            return Err(INVALID_JSON.to_string());
        }
        Ok(d as i32)
    }

    /// Skips one value of any kind.
    fn skip_value(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(INVALID_JSON.to_string());
        }
        self.skip_ws();
        match self.peek() {
            Some(b'"') => {
                self.string()?;
            }
            Some(b'{') => {
                self.i += 1;
                self.skip_ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    self.skip_ws();
                    self.string()?;
                    self.skip_ws();
                    self.expect(b':')?;
                    self.skip_value(depth + 1)?;
                    self.skip_ws();
                    match self.next() {
                        Some(b',') => {}
                        Some(b'}') => return Ok(()),
                        _ => return Err(INVALID_JSON.to_string()),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                self.skip_ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    self.skip_value(depth + 1)?;
                    self.skip_ws();
                    match self.next() {
                        Some(b',') => {}
                        Some(b']') => return Ok(()),
                        _ => return Err(INVALID_JSON.to_string()),
                    }
                }
            }
            Some(b't') => self.word(b"true")?,
            Some(b'f') => self.word(b"false")?,
            Some(b'n') => self.word(b"null")?,
            Some(b'-' | b'0'..=b'9') => {
                let start = self.i;
                while matches!(self.peek(), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| INVALID_JSON.to_string())?;
                t.parse::<f64>().map_err(|_| INVALID_JSON.to_string())?;
            }
            _ => return Err(INVALID_JSON.to_string()),
        }
        Ok(())
    }

    fn word(&mut self, w: &[u8]) -> Result<(), String> {
        if self.b[self.i..].starts_with(w) {
            self.i += w.len();
            Ok(())
        } else {
            Err(INVALID_JSON.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A cache for the tests: the chunks it holds, and the chunks whose region is "on disk" (they answer `Unknown` until `load` is called for them).
    struct Cache {
        held: HashSet<(i32, i32)>,
        unloaded: RefCell<HashSet<(i32, i32)>>,
        asked: RefCell<Vec<(i32, i32)>>,
    }

    impl Cache {
        fn new(held: impl IntoIterator<Item = (i32, i32)>) -> Cache {
            Cache { held: held.into_iter().collect(), unloaded: RefCell::new(HashSet::new()), asked: RefCell::new(vec![]) }
        }
    }

    impl WorldView for Cache {
        fn state_id(&self, _x: i32, _y: i32, _z: i32) -> usize {
            0
        }
        fn contains_loaded_chunk(&self, _x: i32, _z: i32) -> bool {
            false
        }
        fn chunk_status(&self, cx: i32, cz: i32) -> ChunkStatus {
            self.asked.borrow_mut().push((cx, cz));
            if self.held.contains(&(cx, cz)) {
                ChunkStatus::Explored
            } else if self.unloaded.borrow().contains(&(cx, cz)) {
                ChunkStatus::Unknown
            } else {
                ChunkStatus::NotExplored
            }
        }
        fn min_y(&self) -> i32 {
            0
        }
        fn height(&self) -> i32 {
            16
        }
    }

    fn square(r: i32) -> impl Iterator<Item = (i32, i32)> {
        (-r..=r).flat_map(move |x| (-r..=r).map(move |z| (x, z)))
    }

    fn settings() -> ExploreSettings {
        ExploreSettings::default()
    }

    fn xz(x: i32, z: i32) -> Goal {
        Goal::XzMaintainY { x, z, level: 64 }
    }

    fn goals(cmd: &PathingCommand) -> Vec<Goal> {
        match cmd.goal.as_ref().expect("a goal") {
            Goal::Composite(g) => g.clone(),
            other => panic!("not a composite: {other:?}"),
        }
    }

    fn chunk_of(g: &Goal) -> (i32, i32) {
        match g {
            Goal::XzMaintainY { x, z, .. } | Goal::Xz { x, z } => (x >> 4, z >> 4),
            other => panic!("not a column goal: {other:?}"),
        }
    }

    #[test]
    fn an_empty_world_gives_the_ten_closest_chunks_nearest_first() {
        let mut p = ExploreProcess::new();
        p.explore(8, 8); // the middle of chunk (0, 0)
        let cache = Cache::new([]);
        let cmd = p.on_tick(false, &cache, &settings()).expect("a command");
        assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
        let g = goals(&cmd);
        // Upstream walks each ring with `dz = -zval` and then `+zval`, so a chunk with `zval == 0` (the ones on the x axis) is added twice: ring 0 gives two
        // entries, ring 1 six (4 chunks), ring 2 ten (8 chunks). The count (10) is first reached by the end of ring 2, which is taken whole: 18 goals.
        let chunks: Vec<(i32, i32)> = g.iter().map(chunk_of).collect();
        assert_eq!(&chunks[..8], &[(0, 0), (0, 0), (-1, 0), (-1, 0), (0, -1), (0, 1), (1, 0), (1, 0)]);
        assert_eq!(chunks.len(), 18);
        assert!(chunks.iter().all(|(x, z)| x.abs() + z.abs() <= 2));
        // the goals are at the middle of the chunks and keep the height
        assert_eq!(g[0], xz(8, 8));
    }

    #[test]
    fn the_chunks_the_cache_holds_are_skipped_and_the_search_moves_outward() {
        let mut p = ExploreProcess::new();
        p.explore(8, 8);
        let cache = Cache::new(square(2)); // the 5 x 5 chunks around
        let cmd = p.on_tick(false, &cache, &settings()).unwrap();
        let chunks: Vec<(i32, i32)> = goals(&cmd).iter().map(chunk_of).collect();
        // the closest chunks that are not held have Manhattan distance 3 (the corners of the square are at 4, and the sides at 3)
        assert!(chunks.iter().all(|(x, z)| !cache.held.contains(&(*x, *z))), "{chunks:?}");
        let dists: Vec<i32> = chunks.iter().map(|(x, z)| x.abs() + z.abs()).collect();
        assert_eq!(*dists.iter().min().unwrap(), 3);
        assert!(chunks.len() >= 10);
        // nothing was explored at distance 0 and 1 (all held): the next tick starts further out
        assert_eq!(p.distance_completed(), 3, "rings 0, 1 and 2 were all held; the first with an unheld chunk is 3");
    }

    #[test]
    fn distance_completed_only_moves_over_rings_with_nothing_to_explore() {
        let mut p = ExploreProcess::new();
        p.explore(0, 0);
        let mut held: HashSet<(i32, i32)> = square(1).collect();
        // the ring of distance 2 has (0, 2) not held, but all else of rings 0 and 1 is
        let cache = Cache::new(held.iter().copied());
        p.on_tick(false, &cache, &settings());
        assert_eq!(p.distance_completed(), 2, "rings 0 and 1 are held in full (the corners of the square are in ring 2)");
        // the chunks of ring 2 and 3 get held except one far one: the process starts at the first ring that has something
        held.extend(square(3).filter(|(x, z)| x.abs() + z.abs() <= 3));
        let cache = Cache::new(held.iter().copied());
        p.on_tick(false, &cache, &settings());
        assert_eq!(p.distance_completed(), 4);
        // a new call to explore starts again at 0
        p.explore(0, 0);
        assert_eq!(p.distance_completed(), 0);
    }

    #[test]
    fn the_goals_move_on_as_chunks_get_cached() {
        let mut p = ExploreProcess::new();
        p.explore(8, 8);
        let mut held: HashSet<(i32, i32)> = HashSet::new();
        let mut last: Vec<(i32, i32)> = vec![];
        for round in 0..4 {
            let cache = Cache::new(held.iter().copied());
            let chunks: Vec<(i32, i32)> = goals(&p.on_tick(false, &cache, &settings()).unwrap()).iter().map(chunk_of).collect();
            assert!(chunks.iter().all(|c| !held.contains(c)), "round {round}: a held chunk is a goal");
            assert_ne!(chunks, last, "round {round}: the goals did not change");
            // the player walks to the nearest ones and the game caches the chunks around where it got
            let nearest = chunks[0];
            held.extend(square(1).map(|(x, z)| (nearest.0 + x, nearest.1 + z)));
            last = chunks;
        }
    }

    #[test]
    fn a_region_not_read_yet_pauses_the_process_until_it_is() {
        let mut p = ExploreProcess::new();
        p.explore(8, 8);
        let cache = Cache::new([]);
        cache.unloaded.borrow_mut().insert((1, 0));
        let cmd = p.on_tick(false, &cache, &settings()).unwrap();
        assert_eq!(cmd, PathingCommand { goal: None, kind: PathingCommandType::RequestPause });
        assert!(p.is_active(), "waiting is not the end");
        // the region came in: the chunk is not held, so it is a goal now
        cache.unloaded.borrow_mut().clear();
        let cmd = p.on_tick(false, &cache, &settings()).unwrap();
        assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
        assert!(goals(&cmd).iter().any(|g| chunk_of(g) == (1, 0)));
    }

    #[test]
    fn the_chunk_offset_and_the_set_size_shape_the_goals() {
        let mut p = ExploreProcess::new();
        p.explore(8, 8);
        let cache = Cache::new([]);
        let s = ExploreSettings { world_exploring_chunk_offset: 2, explore_chunk_set_minimum_size: 1, ..settings() };
        let g = goals(&p.on_tick(false, &cache, &s).unwrap());
        // size 1: the first ring that reaches it is ring 0: the chunk itself (twice, see above), pushed out 2 chunks = 32 blocks: dx = 0 is not < 0, so the positive way
        assert_eq!(g, vec![xz(8 + 32, 8 + 32), xz(8 + 32, 8 + 32)]);
        // ring 1: the offset pushes each goal outwards on its own side (negative dx or dz goes the other way)
        let cache = Cache::new([(0, 0)]);
        let g = goals(&p.on_tick(false, &cache, &s).unwrap());
        // the whole ring is taken (6 entries, the two chunks on the x axis twice): (-1, 0) goes down by the offset in x and up in z (dz = 0 is not < 0), (0, -1) up in x and
        // down in z, (0, 1) up in both, (1, 0) up in both
        assert_eq!(g, vec![xz(-40, 40), xz(-40, 40), xz(40, -40), xz(40, 56), xz(56, 40), xz(56, 40)]);
    }

    #[test]
    fn maintain_y_minus_one_makes_a_plain_goal() {
        let mut p = ExploreProcess::new();
        p.explore(0, 0);
        let s = ExploreSettings { explore_maintain_y: -1, explore_chunk_set_minimum_size: 1, ..settings() };
        assert_eq!(goals(&p.on_tick(false, &Cache::new([]), &s).unwrap()), vec![Goal::Xz { x: 8, z: 8 }, Goal::Xz { x: 8, z: 8 }]);
        let s = ExploreSettings { explore_maintain_y: 70, explore_chunk_set_minimum_size: 1, ..settings() };
        assert_eq!(goals(&p.on_tick(false, &Cache::new([]), &s).unwrap()), vec![Goal::XzMaintainY { x: 8, z: 8, level: 70 }, Goal::XzMaintainY { x: 8, z: 8, level: 70 }]);
    }

    #[test]
    fn a_failed_path_ends_the_process_with_failed() {
        let mut p = ExploreProcess::new();
        p.explore(0, 0);
        assert!(p.on_tick(true, &Cache::new([]), &settings()).is_none());
        assert!(!p.is_active());
        assert_eq!(p.take_messages(), vec!["Failed"]);
    }

    #[test]
    fn losing_control_forgets_the_centre_and_keeps_the_filter() {
        let mut p = ExploreProcess::new();
        p.apply_json_filter(JsonChunkFilter::parse("[{\"x\":0,\"z\":0}]", false).unwrap());
        p.explore(0, 0);
        p.on_lost_control();
        assert!(!p.is_active());
        assert!(p.filter().is_some());
        assert_eq!(p.take_messages(), vec!["Loaded 1 positions"]);
    }

    // ------------------------------------------------------------ the filter

    #[test]
    fn a_filter_of_explored_chunks_is_added_to_the_cache() {
        let mut p = ExploreProcess::new();
        p.explore(8, 8);
        // the filter says (0, 0), (1, 0) and (0, 1) are explored; the cache has (-1, 0)
        p.apply_json_filter(JsonChunkFilter::parse(r#"[{"x":0,"z":0},{"x":1,"z":0},{"x":0,"z":1}]"#, false).unwrap());
        let cache = Cache::new([(-1, 0)]);
        let chunks: Vec<(i32, i32)> = goals(&p.on_tick(false, &cache, &settings()).unwrap()).iter().map(chunk_of).collect();
        for c in [(0, 0), (1, 0), (0, 1), (-1, 0)] {
            assert!(!chunks.contains(&c), "{c:?} counts as explored");
        }
        assert!(chunks.contains(&(0, -1)));
        // the answer of the filter alone for a chunk not on its list is Unknown: the cache decides
        assert_eq!(p.filter().unwrap().is_already_explored(5, 5), ChunkStatus::Unknown);
        assert_eq!(p.filter().unwrap().is_already_explored(1, 0), ChunkStatus::Explored);
    }

    #[test]
    fn an_inverted_filter_lists_the_chunks_not_explored_and_everything_else_is_explored() {
        let mut p = ExploreProcess::new();
        p.explore(8, 8);
        p.apply_json_filter(JsonChunkFilter::parse(r#"[{"x":3,"z":0},{"x":-4,"z":2}]"#, true).unwrap());
        let cache = Cache::new([]);
        let cmd = p.on_tick(false, &cache, &settings()).unwrap();
        // only listed chunks are goals: the count (2) is smaller than the set size (10), so the search goes on until it has 2 entries. Chunk (3, 0) is on the x
        // axis, so it is added twice (see above) and the 2 are reached before the ring of (-4, 2)
        let chunks: Vec<(i32, i32)> = goals(&cmd).iter().map(chunk_of).collect();
        assert_eq!(chunks, vec![(3, 0), (3, 0)]);
        // when the cache holds both, nothing remains: "Explored all chunks" and the process ends
        let cache = Cache::new([(3, 0), (-4, 2)]);
        assert!(p.on_tick(false, &cache, &settings()).is_none());
        assert!(!p.is_active());
        assert_eq!(p.take_messages().last().map(String::as_str), Some("Explored all chunks"));
    }

    #[test]
    fn disable_completion_check_keeps_an_inverted_filter_going_with_nothing_left() {
        let mut p = ExploreProcess::new();
        p.explore(8, 8);
        p.apply_json_filter(JsonChunkFilter::parse(r#"[{"x":3,"z":0}]"#, true).unwrap());
        let cache = Cache::new([(3, 0)]);
        let s = ExploreSettings { disable_completion_check: true, ..settings() };
        // count = min(0, 10) = 0 chunks wanted: the first ring ends with 0 >= 0 and an empty set of goals
        let cmd = p.on_tick(false, &cache, &s).unwrap();
        assert_eq!(cmd.kind, PathingCommandType::ForceRevalidateGoalAndPath);
        assert!(p.is_active());
    }

    #[test]
    fn the_inverted_count_stops_at_the_set_size_and_asks_the_cache_about_the_list_only() {
        let pos: Vec<String> = (0..50).map(|i| format!("{{\"x\":{i},\"z\":0}}")).collect();
        let f = JsonChunkFilter::parse(&format!("[{}]", pos.join(",")), true).unwrap();
        let cache = Cache::new([]);
        assert_eq!(f.count_remain(&cache, 10), 10);
        assert_eq!(cache.asked.borrow().len(), 10, "it stops counting at the minimum");
        let cache = Cache::new((0..50).map(|i| (i, 0)));
        assert_eq!(f.count_remain(&cache, 10), 0);
        // not inverted: always the maximum
        let f = JsonChunkFilter::parse("[]", false).unwrap();
        assert_eq!(f.count_remain(&cache, 10), i32::MAX);
    }

    #[test]
    fn a_chunk_listed_twice_counts_twice() {
        let f = JsonChunkFilter::parse(r#"[{"x":1,"z":1},{"x":1,"z":1},{"x":1,"z":1}]"#, true).unwrap();
        assert_eq!(f.len(), 3);
        assert_eq!(f.count_remain(&Cache::new([]), 10), 3);
    }

    // ------------------------------------------------------------ the file

    #[test]
    fn the_file_is_a_list_of_chunks() {
        let f = JsonChunkFilter::parse("  [ {\"x\": 1, \"z\": -2} ,\n {\"z\":7,\"x\":-3,\"other\":[1,{\"a\":null}],\"y\":\"s\\\"t\"} ] ", false).unwrap();
        assert_eq!(f.positions, vec![(1, -2), (-3, 7)]);
        assert_eq!(JsonChunkFilter::parse("[]", false).unwrap().len(), 0);
        assert_eq!(JsonChunkFilter::parse("[{}]", true).unwrap().positions, vec![(0, 0)], "a missing field stays 0");
        assert_eq!(JsonChunkFilter::parse("[{\"x\":1.0,\"z\":2e0}]", false).unwrap().positions, vec![(1, 2)], "whole numbers in other spellings are fine");
        assert_eq!(JsonChunkFilter::parse("\u{FEFF}[{\"x\":1,\"z\":2}]", false).unwrap().positions, vec![(1, 2)]);
        assert_eq!(JsonChunkFilter::parse("[{\"x\":1,\"x\":9,\"z\":2}]", false).unwrap().positions, vec![(9, 2)], "the last one wins");
    }

    #[test]
    fn bad_text_is_an_error_and_never_a_panic() {
        for bad in [
            "",
            " ",
            "null",
            "{}",
            "[",
            "]",
            "[{\"x\":1,\"z\":2}",
            "[{\"x\":1,\"z\":2}] x",
            "[{\"x\":1,\"z\":2},]",
            "[,]",
            "[1]",
            "[null]",
            "[\"x\"]",
            "[{\"x\":1.5,\"z\":2}]",
            "[{\"x\":99999999999,\"z\":2}]",
            "[{\"x\":\"1\",\"z\":2}]",
            "[{\"x\":+1,\"z\":2}]",
            "[{\"x\":01,\"z\":2}]",
            "[{\"x\":NaN,\"z\":2}]",
            "[{\"x\":1 \"z\":2}]",
            "[{x:1,z:2}]",
            "[{\"x\":1,\"z\":2,\"bad\":tru}]",
            "[{\"x\":1,\"z\":2,\"bad\":\"unclosed}]",
            "[{\"x\":1,\"z\":2,\"bad\":\"\\q\"}]",
            "[{\"x\":1,\"z\":2,\"bad\":\"\\u12\"}]",
            "[{\"x\":1,\"z\":2,\"bad\":\"a\u{1}b\"}]",
            "\u{0}",
        ] {
            assert_eq!(JsonChunkFilter::parse(bad, false), Err(INVALID_JSON.to_string()), "{bad:?}");
        }
    }

    #[test]
    fn deep_nesting_is_refused_without_recursing_far() {
        let deep = format!("[{{\"x\":1,\"z\":2,\"a\":{}{}}}]", "[".repeat(100_000), "]".repeat(100_000));
        assert_eq!(JsonChunkFilter::parse(&deep, false), Err(INVALID_JSON.to_string()));
        let ok = format!("[{{\"x\":1,\"z\":2,\"a\":{}{}}}]", "[".repeat(10), "]".repeat(10));
        assert!(JsonChunkFilter::parse(&ok, false).is_ok());
    }

    #[test]
    fn a_long_list_reads_quickly() {
        let body: Vec<String> = (0..200_000).map(|i| format!("{{\"x\":{},\"z\":{}}}", i % 1000, i / 1000)).collect();
        let text = format!("[{}]", body.join(","));
        let t = std::time::Instant::now();
        let f = JsonChunkFilter::parse(&text, false).unwrap();
        assert_eq!(f.len(), 200_000);
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn display_name_tells_the_centre_the_distance_and_the_goals() {
        let mut p = ExploreProcess::new();
        p.explore(-20, 40);
        let s = ExploreSettings { explore_chunk_set_minimum_size: 1, ..settings() };
        let name = p.display_name(&Cache::new([]), &s);
        assert_eq!(name, "Exploring around BlockPos{x=-20, y=0, z=40}, distance completed 0, currently going to GoalComposite[GoalXZ{x=-24,z=40}, GoalXZ{x=-24,z=40}]");
    }
}
