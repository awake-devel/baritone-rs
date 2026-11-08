// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/cache/CachedRegion.java. LGPL-3.0-or-later, see lib.rs.
//
// A region is 32 x 32 chunks (512 x 512 blocks). Its file, `r.<x>.<z>.bcr`, is the format upstream writes: a gzip stream of `DataOutputStream` data,
// big-endian: the magic, one present byte per chunk, the packed bits of each present chunk, 256 `writeUTF` block names of each (the overview), the special
// blocks of each, and the timestamp of each. A cache folder written by Java Baritone can be read, and the other way round, as long as the block names
// are the same (docs/deviations.md D41).
use super::chunk::{size, size_in_bytes, CachedChunk, Dimension, SpecialBlocks};
use super::palette::CacheEnv;
use crate::pos::Pos;
use std::sync::Arc;

const CHUNK_NOT_PRESENT: u8 = 0;
const CHUNK_PRESENT: u8 = 1;

/// Upstream's `CACHED_REGION_MAGIC`: detects files that are not a cache, or that an old version wrote.
pub const CACHED_REGION_MAGIC: i32 = 456_022_911;

/// What a load found (D4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// There is no file: a new region.
    Missing,
    /// The file was read.
    Loaded(usize),
    /// The file is damaged or is not this cache's format. It was moved aside, untouched, to the path in the text.
    Corrupt(String),
    /// The file could not be read at all (permissions, a failing disk). It was left where it is and this region is never saved over it.
    Unreadable(String),
}

#[derive(Debug)]
pub struct CachedRegion {
    pub x: i32,
    pub z: i32,
    /// `chunks[x][z]` of upstream, at `x * 32 + z`.
    chunks: Vec<Option<CachedChunk>>,
    has_unsaved_changes: bool,
    /// Counts changes, so a save can tell whether the region changed while the file was being written.
    version: u64,
    /// D4: the file could not be read, so nothing is written over it.
    no_save: bool,
}

impl CachedRegion {
    /// Upstream's `CachedRegion.<init>`:
    pub fn new(x: i32, z: i32) -> CachedRegion {
        CachedRegion { x, z, chunks: vec![None; 1024], has_unsaved_changes: false, version: 0, no_save: false }
    }

    /// `CachedRegion.getBlock`: `x` and `z` are inside the region (0..512), `y` is a world Y.
    pub fn get_block(&self, x: i32, y: i32, z: i32, dim: &Dimension, env: &CacheEnv) -> Option<usize> {
        let adj_y = y - dim.min_y;
        if adj_y < 0 || adj_y >= dim.height {
            return None;
        }
        self.chunks[((x >> 4) * 32 + (z >> 4)) as usize].as_ref()?.get_block(x & 15, adj_y, z & 15, dim, env)
    }

    /// `CachedRegion.isCached`: upstream's method.
    pub fn is_cached(&self, x: i32, z: i32) -> bool {
        self.chunks[((x >> 4) * 32 + (z >> 4)) as usize].is_some()
    }

    /// `CachedRegion.getLocationsOf`: the places of a special block in the region, chunk by chunk.
    pub fn get_locations_of(&self, block: &str) -> Vec<Pos> {
        let mut res = vec![];
        for chunk in self.chunks.iter().flatten() {
            if let Some(locs) = chunk.absolute_blocks(block) {
                res.extend(locs);
            }
        }
        res
    }

    /// `CachedRegion.updateCachedChunk`: upstream's method.
    pub fn update_cached_chunk(&mut self, chunk_x: i32, chunk_z: i32, chunk: CachedChunk) {
        self.chunks[(chunk_x * 32 + chunk_z) as usize] = Some(chunk);
        self.has_unsaved_changes = true;
        self.version += 1;
    }

    /// The chunk at these coordinates inside the region (0..32), if it is cached.
    pub fn chunk(&self, chunk_x: i32, chunk_z: i32) -> Option<&CachedChunk> {
        self.chunks[(chunk_x * 32 + chunk_z) as usize].as_ref()
    }

    /// Sets the time a cached chunk was last seen, without marking the region as changed.
    pub fn touch_chunk(&mut self, chunk_x: i32, chunk_z: i32, timestamp: i64) {
        if let Some(c) = self.chunks[(chunk_x * 32 + chunk_z) as usize].as_mut() {
            c.cache_timestamp = timestamp;
        }
    }

    /// `CachedRegion.removeExpired`: chunks cached more than `expiry_seconds` ago are dropped. A negative expiry keeps everything.
    pub fn remove_expired(&mut self, expiry_seconds: i64, now_ms: i64) {
        if expiry_seconds < 0 {
            return;
        }
        let oldest_acceptable = now_ms.saturating_sub(expiry_seconds.saturating_mul(1000));
        for slot in self.chunks.iter_mut() {
            if slot.as_ref().is_some_and(|c| c.cache_timestamp < oldest_acceptable) {
                *slot = None;
                self.has_unsaved_changes = true;
                self.version += 1;
            }
        }
    }

    /// `CachedRegion.mostRecentlyModified`: upstream's method.
    pub fn most_recently_modified(&self) -> Option<&CachedChunk> {
        let mut recent: Option<&CachedChunk> = None;
        for c in self.chunks.iter().flatten() {
            if recent.is_none_or(|r| c.cache_timestamp > r.cache_timestamp) {
                recent = Some(c);
            }
        }
        recent
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.iter().flatten().count()
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.has_unsaved_changes && !self.no_save
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// A save of this version reached the disk: the region has no unsaved changes, unless it changed since.
    pub fn mark_saved(&mut self, version: u64) {
        if self.version == version {
            self.has_unsaved_changes = false;
        }
    }

    pub fn mark_no_save(&mut self) {
        self.no_save = true;
    }

    /// `CachedRegion.save`: ported, see the notes below and docs/deviations.md D41.
    /// The part of `CachedRegion.save` that writes the stream (before gzip).
    pub fn serialize(&self, dim: &Dimension, env: &CacheEnv) -> Vec<u8> {
        let chunk_bytes = size_in_bytes(size(dim.height));
        let present = self.chunks.iter().flatten().count();
        let mut out = Vec::with_capacity(4 + 1024 + present * (chunk_bytes + 256 * 12 + 16));
        out.extend_from_slice(&CACHED_REGION_MAGIC.to_be_bytes());
        for slot in &self.chunks {
            match slot {
                None => out.push(CHUNK_NOT_PRESENT),
                Some(c) => {
                    out.push(CHUNK_PRESENT);
                    out.extend_from_slice(c.to_byte_array());
                    // fills the empty 0s that should be trailing to fill up the space
                    out.resize(out.len() + chunk_bytes - c.to_byte_array().len(), 0);
                }
            }
        }
        for c in self.chunks.iter().flatten() {
            for state in c.overview() {
                let name = env.palette.name_of(*state).map(|n| &**n).unwrap_or("air");
                write_utf(&mut out, name);
            }
        }
        for c in self.chunks.iter().flatten() {
            let locs = c.relative_blocks();
            out.extend_from_slice(&(locs.len() as u16).to_be_bytes());
            for (name, list) in locs {
                write_utf(&mut out, name);
                // a chunk can hold more of one block than a short counts (a chunk of cobweb); upstream writes the count modulo 65536 and all the entries, which no
                // reader can follow. 65536 is the count 0 stands for, so that many are kept (D43)
                let list = &list[..list.len().min(65536)];
                out.extend_from_slice(&(list.len() as u16).to_be_bytes());
                for p in list {
                    out.push(((p[2] << 4) | p[0]) as u8);
                    out.extend_from_slice(&(p[1] - dim.min_y).to_be_bytes());
                }
            }
        }
        for c in self.chunks.iter().flatten() {
            out.extend_from_slice(&c.cache_timestamp.to_be_bytes());
        }
        out
    }

    /// `CachedRegion.load`: ported, see the notes below and docs/deviations.md D41.
    /// The part of `CachedRegion.load` that reads the stream (after gzip). Nothing is changed unless the whole stream was read: "only if the entire file
    /// was uncorrupted do we actually set the chunks".
    pub fn deserialize(&mut self, bytes: &[u8], dim: &Dimension, env: &CacheEnv) -> Result<usize, String> {
        let mut r = Reader { buf: bytes, pos: 0 };
        let magic = r.i32()?;
        if magic != CACHED_REGION_MAGIC {
            return Err(format!("Bad magic value {magic}"));
        }
        let chunk_bytes = size_in_bytes(size(dim.height));
        let mut present = vec![false; 1024];
        let mut bits: Vec<Option<Vec<u8>>> = vec![None; 1024];
        for i in 0..1024 {
            match r.u8()? {
                CHUNK_PRESENT => {
                    bits[i] = Some(r.take(chunk_bytes)?.to_vec());
                    present[i] = true;
                }
                CHUNK_NOT_PRESENT => {}
                _ => return Err("Malformed stream".to_string()),
            }
        }
        let mut overviews: Vec<Vec<usize>> = vec![vec![]; 1024];
        for i in 0..1024 {
            if present[i] {
                let mut ov = Vec::with_capacity(256);
                for _ in 0..256 {
                    let name = r.utf()?;
                    ov.push(env.palette.default_state(&name).ok_or_else(|| format!("Unknown block {name}"))?);
                }
                overviews[i] = ov;
            }
        }
        let mut locations: Vec<SpecialBlocks> = vec![vec![]; 1024];
        let mut total = 0usize;
        for i in 0..1024 {
            if present[i] {
                let kinds = usize::from(r.u16()?);
                let (mut entries, cells) = (0usize, 256 * dim.height.max(0) as usize);
                for _ in 0..kinds {
                    let name: Arc<str> = Arc::from(r.utf()?.as_str());
                    if env.palette.default_state(&name).is_none() {
                        return Err(format!("Unknown block {name}"));
                    }
                    let mut n = usize::from(r.u16()?);
                    if n == 0 {
                        // an entire chunk full of air can happen in the end
                        n = 65536;
                    }
                    entries += n;
                    total += n;
                    if entries > cells || total > MAX_REGION_SPECIALS {
                        return Err("More special blocks than the chunk or the region can hold".to_string());
                    }
                    let mut list = Vec::with_capacity(n.min(4096));
                    for _ in 0..n {
                        let xz = r.u8()?;
                        let y = r.i32()?;
                        // the Y is stored from the lowest one: anything outside the dimension is damage (D43)
                        if y < 0 || y >= dim.height {
                            return Err(format!("Special block at Y {y} outside the dimension"));
                        }
                        list.push([i32::from(xz & 0x0f), y + dim.min_y, i32::from((xz >> 4) & 0x0f)]);
                    }
                    locations[i].push((name, list));
                }
            }
        }
        let mut stamps = vec![0i64; 1024];
        for i in 0..1024 {
            if present[i] {
                stamps[i] = r.i64()?;
            }
        }
        let mut chunks: Vec<Option<CachedChunk>> = vec![None; 1024];
        let mut count = 0;
        for i in 0..1024 {
            if present[i] {
                let (cx, cz) = ((i / 32) as i32, (i % 32) as i32);
                let chunk = CachedChunk::new(cx + 32 * self.x, cz + 32 * self.z, dim, bits[i].take().unwrap_or_default(), std::mem::take(&mut overviews[i]), std::mem::take(&mut locations[i]), stamps[i], env)?;
                chunks[i] = Some(chunk);
                count += 1;
            }
        }
        self.chunks = chunks;
        Ok(count)
    }
}

/// Java's `DataOutputStream.writeUTF`: two bytes of length, then the text (modified UTF-8, which is UTF-8 for names that have no NUL and no emoji).
fn write_utf(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    out.extend_from_slice(&(b.len().min(65535) as u16).to_be_bytes());
    out.extend_from_slice(&b[..b.len().min(65535)]);
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos.checked_add(n).is_none_or(|e| e > self.buf.len()) {
            return Err("unexpected end of data".to_string());
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Result<i32, String> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i64(&mut self) -> Result<i64, String> {
        let b = self.take(8)?;
        Ok(i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
    fn utf(&mut self) -> Result<String, String> {
        let n = usize::from(self.u16()?);
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "bad UTF in a block name".to_string())
    }
}

/// The most special blocks (D43) one region file may list in all: 4 Mi, 20 MB of stream. A real region holds a few thousand.
const MAX_REGION_SPECIALS: usize = 4 << 20;

/// The most a region file can unpack to: every chunk full, its overview, a generous list of names and counts, and [`MAX_REGION_SPECIALS`] entries of 5 bytes. Upstream
/// reads up to whatever the gzip holds; a few kilobytes of zeros made a gigabyte.
pub fn max_region_bytes(dim: &Dimension) -> usize {
    let chunk = 1 + size_in_bytes(size(dim.height)) + 256 * 70 + 8 + 2 + 128 * 70;
    4 + 1024 * chunk + 5 * MAX_REGION_SPECIALS
}

/// `CachedRegion.getRegionFile`: upstream's method.
pub fn region_file(dir: &std::path::Path, region_x: i32, region_z: i32) -> std::path::PathBuf {
    dir.join(format!("r.{region_x}.{region_z}.bcr"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::chunk::pack;
    use crate::cache::chunk::tests::{overworld, Cells};
    use crate::cache::palette::fixture::*;

    fn region_with_two_chunks() -> CachedRegion {
        let env = env();
        let dim = overworld();
        let mut a = Cells::new(384);
        for y in 0..80 {
            a.set(3, y, 3, STONE);
        }
        a.set(4, 144, 4, CHEST);
        a.set(5, 145, 4, SPAWNER);
        a.set(5, 130, 5, SPAWNER);
        let mut b = Cells::new(384);
        b.set(0, 10, 0, WATER);
        let mut r = CachedRegion::new(-1, 2);
        r.update_cached_chunk(31, 0, pack(&a, -1, 64, &dim, &env, false, 1000));
        r.update_cached_chunk(0, 5, pack(&b, -32, 69, &dim, &env, false, 2000));
        r
    }

    #[test]
    fn a_region_round_trips_through_its_bytes_and_gzip() {
        let env = env();
        let dim = overworld();
        let r = region_with_two_chunks();
        let raw = r.serialize(&dim, &env);
        let z = crate::cache::gzip::compress(&raw, 6);
        assert!(z.len() * 10 < raw.len(), "mostly zeros: {} of {}", z.len(), raw.len());
        let back = crate::cache::gzip::decompress(&z, 1 << 28).unwrap();
        assert_eq!(back, raw);
        let mut s = CachedRegion::new(-1, 2);
        assert_eq!(s.deserialize(&back, &dim, &env).unwrap(), 2);
        assert_eq!(s.serialize(&dim, &env), raw);
        assert_eq!(s.get_locations_of("spawner"), r.get_locations_of("spawner"));
        assert_eq!(s.get_locations_of("spawner").len(), 2);
        assert_eq!(s.get_locations_of("chest"), vec![Pos::new(-12, 80, 1028)]);
        // the same answers from every block
        for (x, y, z) in [(496 + 3, 40, 3), (496 + 4, 80, 4), (0, 10, 5 * 16), (100, 100, 100), (3, 3, 3)] {
            assert_eq!(r.get_block(x, y, z, &dim, &env), s.get_block(x, y, z, &dim, &env));
        }
        assert_eq!(s.get_block(496 + 4, 80, 4, &dim, &env), Some(CHEST), "D1 survives the file");
        assert!(s.is_cached(496, 0) && s.is_cached(0, 80) && !s.is_cached(256, 256));
    }

    #[test]
    fn the_stream_is_upstreams_layout() {
        let env = env();
        let dim = overworld();
        let raw = region_with_two_chunks().serialize(&dim, &env);
        assert_eq!(&raw[..4], &456_022_911i32.to_be_bytes());
        // the flags and the data are interleaved: chunk (0, 5) is the first present one, at index 5, and its bits follow its flag
        assert_eq!(raw[4 + 5], 1);
        assert_eq!(raw[4 + 4], 0);
        // chunk (31, 0), index 992, comes after the 24576 bytes of the first
        assert_eq!(raw[4 + 992 + 24576], 1);
        assert_eq!(raw[4 + 993 + 24576], 0);
        let after = 4 + 1024 + 2 * 24576;
        // then the overview: writeUTF("water") = 00 05 'w' 'a' 't' 'e' 'r'
        assert_eq!(&raw[after..after + 7], &[0, 5, b'w', b'a', b't', b'e', b'r'], "the first column of the first present chunk holds water");
        // and the last 16 bytes are the two timestamps, in chunk order: (0, 5) at 2000 and (31, 0) at 1000
        let n = raw.len();
        assert_eq!(&raw[n - 16..n - 8], &2000i64.to_be_bytes());
        assert_eq!(&raw[n - 8..], &1000i64.to_be_bytes());
    }

    #[test]
    fn damage_is_an_error_and_changes_nothing() {
        let env = env();
        let dim = overworld();
        let raw = region_with_two_chunks().serialize(&dim, &env);
        let mut r = CachedRegion::new(-1, 2);
        let mut bad_magic = raw.clone();
        bad_magic[0] ^= 1;
        assert!(r.deserialize(&bad_magic, &dim, &env).unwrap_err().starts_with("Bad magic"));
        assert!(r.deserialize(&raw[..raw.len() - 5], &dim, &env).is_err());
        let mut malformed = raw.clone();
        malformed[9] = 7;
        assert_eq!(r.deserialize(&malformed, &dim, &env).unwrap_err(), "Malformed stream");
        assert_eq!(r.chunk_count(), 0);
        // a block name this game does not know is a failure too, as `stringToBlockRequired` throws
        let mut named = raw.clone();
        let at = 4 + 1024 + 2 * 24576 + 2;
        named[at] = b'q';
        assert!(r.deserialize(&named, &dim, &env).unwrap_err().starts_with("Unknown block"));
        assert_eq!(r.chunk_count(), 0);
    }

    #[test]
    fn expiry_drops_old_chunks_only_when_asked() {
        let mut r = region_with_two_chunks();
        r.remove_expired(-1, 10_000_000);
        assert_eq!(r.chunk_count(), 2);
        r.remove_expired(5, 6_500); // oldest acceptable: 1500
        assert_eq!(r.chunk_count(), 1);
        assert_eq!(r.most_recently_modified().unwrap().cache_timestamp, 2000);
    }

    #[test]
    fn a_chunk_full_of_one_special_block_is_read_back() {
        // upstream writes a 65536 count as 0 (a short): the reader reads 0 as 65536
        let env = env();
        let dim = overworld();
        let mut raw = vec![];
        raw.extend_from_slice(&CACHED_REGION_MAGIC.to_be_bytes());
        raw.push(1);
        raw.extend_from_slice(&vec![0u8; 24576]);
        raw.extend(std::iter::repeat_n(0u8, 1023));
        for _ in 0..256 {
            raw.extend_from_slice(&[0, 3, b'a', b'i', b'r']);
        }
        raw.extend_from_slice(&1u16.to_be_bytes());
        raw.extend_from_slice(&[0, 7, b's', b'p', b'a', b'w', b'n', b'e', b'r']);
        raw.extend_from_slice(&0u16.to_be_bytes());
        for i in 0..65536u32 {
            raw.push(((i >> 4) & 0xf0 | (i & 0xf)) as u8);
            raw.extend_from_slice(&((i >> 8) as i32).to_be_bytes());
        }
        raw.extend_from_slice(&5i64.to_be_bytes());
        let mut r = CachedRegion::new(0, 0);
        assert_eq!(r.deserialize(&raw, &dim, &env).unwrap(), 1);
        assert_eq!(r.get_locations_of("spawner").len(), 65536);
    }

    /// The stream of one chunk up to its section of special blocks.
    fn prefix() -> Vec<u8> {
        let mut raw = vec![];
        raw.extend_from_slice(&CACHED_REGION_MAGIC.to_be_bytes());
        raw.push(1);
        raw.extend_from_slice(&vec![0u8; 24576]);
        raw.extend(std::iter::repeat_n(0u8, 1023));
        for _ in 0..256 {
            raw.extend_from_slice(&[0, 3, b'a', b'i', b'r']);
        }
        raw
    }

    /// A region stream of one chunk with one list of `count` spawner entries, each at the given Y.
    fn stream_with_spawners(ys: impl Fn(u32) -> i32, count: u32) -> Vec<u8> {
        let mut raw = prefix();
        raw.extend_from_slice(&1u16.to_be_bytes());
        raw.extend_from_slice(&[0, 7, b's', b'p', b'a', b'w', b'n', b'e', b'r']);
        raw.extend_from_slice(&(count as u16).to_be_bytes());
        for i in 0..count {
            raw.push((i & 0xff) as u8);
            raw.extend_from_slice(&ys(i).to_be_bytes());
        }
        raw.extend_from_slice(&5i64.to_be_bytes());
        raw
    }

    #[test]
    fn review_8_a_special_block_outside_the_dimension_is_damage() {
        let (env, dim) = (env(), overworld());
        for bad in [-1, 384, i32::MAX] {
            let mut r = CachedRegion::new(0, 0);
            let err = r.deserialize(&stream_with_spawners(|_| bad, 1), &dim, &env).unwrap_err();
            assert!(err.contains("outside the dimension"), "{err}");
            assert_eq!(r.chunk_count(), 0, "nothing is set");
        }
        let mut r = CachedRegion::new(0, 0);
        assert_eq!(r.deserialize(&stream_with_spawners(|_| 383, 1), &dim, &env).unwrap(), 1);
        assert_eq!(r.get_locations_of("spawner")[0].y, 383 + dim.min_y);
    }

    #[test]
    fn review_4_a_stream_of_endless_special_blocks_is_refused_and_the_unpack_has_a_bound() {
        let (env, dim) = (env(), overworld());
        // two lists of 65536 are 131072 entries in a chunk of 98304 cells
        let mut raw = prefix();
        raw.extend_from_slice(&2u16.to_be_bytes());
        for _ in 0..2 {
            raw.extend_from_slice(&[0, 7, b's', b'p', b'a', b'w', b'n', b'e', b'r']);
            raw.extend_from_slice(&0u16.to_be_bytes());
            for i in 0..65536u32 {
                raw.push(0);
                raw.extend_from_slice(&((i % 384) as i32).to_be_bytes());
            }
        }
        raw.extend_from_slice(&5i64.to_be_bytes());
        let mut r = CachedRegion::new(0, 0);
        let err = r.deserialize(&raw, &dim, &env).unwrap_err();
        assert!(err.contains("More special blocks"), "{err}");
        // the unpack of a gzip is bounded by the dimension, not by a gigabyte
        let bound = max_region_bytes(&dim);
        assert!(bound < 128 << 20 && bound > 25 << 20, "{bound}");
        let bomb = crate::cache::gzip::compress(&vec![0u8; bound + 1], 1);
        assert!(crate::cache::gzip::decompress(&bomb, bound).is_err());
    }

    #[test]
    fn review_6_more_than_65536_special_blocks_of_a_kind_are_kept_as_65536_and_read_back() {
        let (env, dim) = (env(), overworld());
        let mut c = Cells::new(384);
        for y in 0..275 {
            for x in 0..16 {
                for z in 0..16 {
                    c.set(x, y, z, SPAWNER);
                }
            }
        }
        let chunk = pack(&c, 0, 0, &dim, &env, false, 1_000);
        let mut r = CachedRegion::new(0, 0);
        r.update_cached_chunk(0, 0, chunk);
        let raw = r.serialize(&dim, &env);
        let mut back = CachedRegion::new(0, 0);
        assert_eq!(back.deserialize(&raw, &dim, &env).unwrap(), 1, "upstream wrote count % 65536 and all the entries: no reader follows that");
        assert_eq!(back.get_locations_of("spawner").len(), 65536);
        assert_eq!(back.chunk(0, 0).unwrap().cache_timestamp, 1_000, "the stamp after the lists is where it belongs");
    }
}
