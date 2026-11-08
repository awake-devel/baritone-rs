// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/cache/CachedWorld.java and CacheFiles.java. LGPL-3.0-or-later, see lib.rs.
//
// The cached world of one dimension: the regions in memory (loaded from their files when first asked for), saving, pruning, and the search for special
// blocks. The queue of chunks to pack and the threads are in `worker`.
use super::chunk::{CachedChunk, Dimension};
use super::palette::CacheEnv;
use super::region::{region_file, CachedRegion, LoadOutcome};
#[cfg(test)]
use crate::blocks::State;
use crate::pos::Pos;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

/// Upstream's `REGION_MAX`:
const REGION_MAX: i32 = 30_000_000 / 512 + 1;

/// The settings the cache reads. Defaults are upstream's.
#[derive(Clone, Debug, PartialEq)]
pub struct CacheSettings {
    /// `chunkCaching`, default true.
    pub chunk_caching: bool,
    /// `pruneRegionsFromRAM`, default true.
    pub prune_regions_from_ram: bool,
    /// `chunkPackerQueueMaxSize`, default 2000.
    pub chunk_packer_queue_max_size: i32,
    /// `cachedChunksExpirySeconds`, default -1 (never).
    pub cached_chunks_expiry_seconds: i64,
    /// `repackOnAnyBlockChange`, default true.
    pub repack_on_any_block_change: bool,
}

impl Default for CacheSettings {
    fn default() -> Self {
        CacheSettings { chunk_caching: true, prune_regions_from_ram: true, chunk_packer_queue_max_size: 2000, cached_chunks_expiry_seconds: -1, repack_on_any_block_change: true }
    }
}

/// The counters a test or a log reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveReport {
    pub saved: usize,
    pub failed: usize,
    pub bytes: u64,
}

pub struct CachedWorld {
    directory: PathBuf,
    pub dimension: Dimension,
    pub env: Arc<CacheEnv>,
    regions: RwLock<HashMap<i64, Arc<RwLock<CachedRegion>>>>,
    /// Held while a region is read from disk, so two threads do not load the same one. Not the lock on the map.
    disk_lock: Mutex<()>,
    /// Regions with a background load already on the way (`pendingLoads`).
    pending_loads: Mutex<HashSet<i64>>,
    settings: RwLock<CacheSettings>,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
    /// Messages for the log: failures of loads and saves. Taken by the glue.
    notes: Mutex<Vec<String>>,
    /// Where the player is, in block coordinates, for `prune` (`guessPosition`). `None` when the player is not in this world.
    player: Mutex<Option<(i32, i32)>>,
    /// Held for a whole `save`, so two saves of this world (the timer's and the one when the world is left) do not write the same files at once.
    save_lock: Mutex<()>,
}

impl CachedWorld {
    /// Upstream's `CachedWorld.<init>` without the threads: the folder is made, regions are read on demand.
    pub fn new(directory: PathBuf, dimension: Dimension, env: Arc<CacheEnv>, settings: CacheSettings, clock: Box<dyn Fn() -> i64 + Send + Sync>) -> CachedWorld {
        let _ = std::fs::create_dir_all(&directory);
        CachedWorld { directory, dimension, env, regions: RwLock::new(HashMap::new()), disk_lock: Mutex::new(()), pending_loads: Mutex::new(HashSet::new()), settings: RwLock::new(settings), clock, notes: Mutex::new(vec![]), player: Mutex::new(None), save_lock: Mutex::new(()) }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn now_ms(&self) -> i64 {
        (self.clock)()
    }

    pub fn set_settings(&self, s: CacheSettings) {
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = s;
    }

    pub fn settings(&self) -> CacheSettings {
        self.settings.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Where the player stands (feet, x and z), as `prune` needs it. `None` when the player is not in this world.
    pub fn set_player(&self, at: Option<(i32, i32)>) {
        *self.player.lock().unwrap_or_else(|e| e.into_inner()) = at;
    }

    /// The messages of failures since the last call.
    pub fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut *self.notes.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn note(&self, text: String) {
        let mut n = self.notes.lock().unwrap_or_else(|e| e.into_inner());
        if n.len() < 200 {
            n.push(text);
        }
    }

    /// `CachedWorld.isRegionInWorld`: ported, see the notes below and docs/deviations.md D41.
    /// `CachedWorld.getRegionID`: 0 for a region outside the world.
    fn region_id(region_x: i32, region_z: i32) -> i64 {
        if !Self::region_in_world(region_x, region_z) {
            return 0;
        }
        (i64::from(region_x) & 0xFFFF_FFFF) | (i64::from(region_z) & 0xFFFF_FFFF) << 32
    }

    /// `CachedWorld.isRegionInWorld`: whether the region is inside the world border's reach.
    pub fn region_in_world(region_x: i32, region_z: i32) -> bool {
        (-REGION_MAX..=REGION_MAX).contains(&region_x) && (-REGION_MAX..=REGION_MAX).contains(&region_z)
    }

    /// `CachedWorld.getRegion`: the region if it is in memory. Never reads a file.
    pub fn get_region(&self, region_x: i32, region_z: i32) -> Option<Arc<RwLock<CachedRegion>>> {
        if !Self::region_in_world(region_x, region_z) {
            return None; // its id is region (0, 0)'s
        }
        self.regions.read().unwrap_or_else(|e| e.into_inner()).get(&Self::region_id(region_x, region_z)).cloned()
    }

    /// `CachedWorld.isCached`: upstream's method.
    pub fn is_cached(&self, block_x: i32, block_z: i32) -> bool {
        match self.get_region(block_x >> 9, block_z >> 9) {
            Some(r) => r.read().unwrap_or_else(|e| e.into_inner()).is_cached(block_x & 511, block_z & 511),
            None => false,
        }
    }

    /// `CachedWorld.regionLoaded`: whether the region that holds this block is in memory.
    pub fn region_loaded(&self, block_x: i32, block_z: i32) -> bool {
        self.get_region(block_x >> 9, block_z >> 9).is_some()
    }

    /// `CachedWorld.tryLoadFromDisk`: reads the region if it is not in memory yet. Blocks while the file is read: not for the game thread.
    pub fn try_load_from_disk(&self, region_x: i32, region_z: i32) {
        self.get_or_create_region(region_x, region_z);
    }

    /// `CachedWorld.tryLoadFromDiskAsync`: reads the region on a thread of its own. One read per region at a time, however often it is asked for
    /// (`ExploreProcess` asks about every chunk it considers, every tick). Returns whether a read was started.
    pub fn try_load_from_disk_async(self: &Arc<Self>, region_x: i32, region_z: i32) -> bool {
        let id = Self::region_id(region_x, region_z);
        if !self.pending_loads.lock().unwrap_or_else(|e| e.into_inner()).insert(id) {
            return false;
        }
        let world = Arc::clone(self);
        let spawned = std::thread::Builder::new().name("baritone-rs-region-load".into()).spawn(move || {
            // the entry goes even if the read panics, or the region would never be asked for again
            struct Pending<'a>(&'a CachedWorld, i64);
            impl Drop for Pending<'_> {
                fn drop(&mut self) {
                    self.0.pending_loads.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.1);
                }
            }
            let _pending = Pending(&world, id);
            world.try_load_from_disk(region_x, region_z);
        });
        if spawned.is_err() {
            self.pending_loads.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            return false;
        }
        true
    }

    /// `CachedWorld.getOrCreateRegion`: the region, read from its file the first time.
    pub fn get_or_create_region(&self, region_x: i32, region_z: i32) -> Arc<RwLock<CachedRegion>> {
        if let Some(r) = self.get_region(region_x, region_z) {
            return r;
        }
        if !Self::region_in_world(region_x, region_z) {
            // such a region has the id of region (0, 0) and must not become it: an empty one that is kept nowhere
            return Arc::new(RwLock::new(CachedRegion::new(region_x, region_z)));
        }
        // the file gets its own lock, so a thread that asks about another region is not held up until the gzip is done
        let _disk = self.disk_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = self.get_region(region_x, region_z) {
            return r; // somebody else could have loaded it while we waited
        }
        let mut region = CachedRegion::new(region_x, region_z);
        self.load_region(&mut region);
        let arc = Arc::new(RwLock::new(region));
        self.regions.write().unwrap_or_else(|e| e.into_inner()).insert(Self::region_id(region_x, region_z), Arc::clone(&arc));
        arc
    }

    /// `CachedRegion.load`, with D4: a file that fails to load is kept. Damaged data is moved aside (not deleted, not changed); a file that cannot be read stays
    /// where it is and the region is never saved over it. Either way the failure is a note.
    pub fn load_region(&self, region: &mut CachedRegion) -> LoadOutcome {
        let path = region_file(&self.directory, region.x, region.z);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoadOutcome::Missing,
            Err(e) => {
                region.mark_no_save();
                self.note(format!("cache region {},{} could not be read and is left alone: {e}", region.x, region.z));
                return LoadOutcome::Unreadable(e.to_string());
            }
        };
        let began = std::time::Instant::now();
        let result = super::gzip::decompress(&bytes, super::region::max_region_bytes(&self.dimension)).and_then(|raw| region.deserialize(&raw, &self.dimension, &self.env));
        match result {
            Ok(n) => {
                region.remove_expired(self.settings().cached_chunks_expiry_seconds, self.now_ms());
                self.note(format!("cache region {},{} loaded: {n} chunks in {} ms", region.x, region.z, began.elapsed().as_millis()));
                LoadOutcome::Loaded(n)
            }
            Err(why) => {
                // D4: upstream prints the error, leaves the region empty, and a later save overwrites the file.
                let mut aside = path.with_extension("bcr.corrupt");
                let mut k = 1;
                while aside.exists() {
                    aside = path.with_extension(format!("bcr.corrupt{k}"));
                    k += 1;
                }
                match std::fs::rename(&path, &aside) {
                    Ok(()) => {
                        self.note(format!("cache region {},{} failed to load ({why}); the file was kept as {}", region.x, region.z, aside.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                        LoadOutcome::Corrupt(aside.to_string_lossy().into_owned())
                    }
                    Err(e) => {
                        region.mark_no_save();
                        self.note(format!("cache region {},{} failed to load ({why}) and could not be moved aside ({e}); it is left alone", region.x, region.z));
                        LoadOutcome::Unreadable(why)
                    }
                }
            }
        }
    }

    /// `CachedWorld.updateCachedChunk`: upstream's method.
    pub fn update_cached_chunk(&self, chunk: CachedChunk) {
        if !Self::region_in_world(chunk.x >> 5, chunk.z >> 5) {
            return;
        }
        let region = self.get_or_create_region(chunk.x >> 5, chunk.z >> 5);
        region.write().unwrap_or_else(|e| e.into_inner()).update_cached_chunk(chunk.x & 31, chunk.z & 31, chunk);
    }

    /// `updateCachedChunk` for a chunk that may be the one already held: nothing is stored (and the region is not marked as changed) when it says the same as
    /// the cached one. Returns whether it was stored. Not upstream: it packs when a chunk is queued again, which the glue does for a chunk near the player
    /// because it cannot see which block changed, and an unchanged chunk must not make a region unsaved again.
    pub fn update_cached_chunk_if_changed(&self, chunk: CachedChunk) -> bool {
        if !Self::region_in_world(chunk.x >> 5, chunk.z >> 5) {
            return false;
        }
        let region = self.get_or_create_region(chunk.x >> 5, chunk.z >> 5);
        let mut g = region.write().unwrap_or_else(|e| e.into_inner());
        if g.chunk(chunk.x & 31, chunk.z & 31).is_some_and(|old| old.same_content(&chunk)) {
            // seen again as it was: it is not stale (`cachedChunksExpirySeconds`), and there is nothing new to save
            g.touch_chunk(chunk.x & 31, chunk.z & 31, chunk.cache_timestamp);
            return false;
        }
        g.update_cached_chunk(chunk.x & 31, chunk.z & 31, chunk);
        true
    }

    /// `CachedWorld.getLocationsOf`: the places of a special block in the regions around the one holding `(center_x, center_z)`. `max_region_distance_sq` is
    /// 2 for "adjacent directly or diagonally". The search widens by squared distance and stops at the first ring that reaches `maximum`. The regions it
    /// asks about are loaded from their files.
    pub fn get_locations_of(&self, block: &str, maximum: i32, center_x: i32, center_z: i32, max_region_distance_sq: i32) -> Vec<Pos> {
        let mut res = vec![];
        let (cr_x, cr_z) = (center_x >> 9, center_z >> 9);
        let mut radius = 0;
        while radius <= max_region_distance_sq {
            for xoff in -radius..=radius {
                for zoff in -radius..=radius {
                    if xoff * xoff + zoff * zoff != radius {
                        continue;
                    }
                    let region = self.get_or_create_region(xoff + cr_x, zoff + cr_z);
                    res.extend(region.read().unwrap_or_else(|e| e.into_inner()).get_locations_of(block));
                }
            }
            if res.len() as i64 >= i64::from(maximum) {
                return res;
            }
            radius += 1;
        }
        res
    }

    /// `CachedWorld.allRegions`: ported, see the notes below and docs/deviations.md D41.
    fn all_regions(&self) -> Vec<Arc<RwLock<CachedRegion>>> {
        self.regions.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }

    pub fn region_count(&self) -> usize {
        self.regions.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// `CacheFiles.moveIntoPlace`: ported, see the notes below and docs/deviations.md D41.
    /// `CachedRegion.save`: writes the region to a temporary file and moves it into place (`CacheFiles.moveIntoPlace`), so a save that is cut off leaves
    /// the old file. The region is locked for reading only while its bytes are made, not while they are compressed and written.
    fn save_region(&self, region: &RwLock<CachedRegion>) -> Result<Option<u64>, String> {
        let (raw, version, x, z) = {
            let mut guard = region.write().unwrap_or_else(|e| e.into_inner());
            if !guard.has_unsaved_changes() {
                return Ok(None);
            }
            guard.remove_expired(self.settings().cached_chunks_expiry_seconds, self.now_ms());
            (guard.serialize(&self.dimension, &self.env), guard.version(), guard.x, guard.z)
        };
        std::fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
        let file = region_file(&self.directory, x, z);
        let mut tmp_name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        tmp_name.push(".tmp");
        let tmp = file.with_file_name(tmp_name);
        let zipped = super::gzip::compress(&raw, 6);
        std::fs::write(&tmp, &zipped).map_err(|e| e.to_string())?;
        // moves into place; the rename replaces the old file on every platform std supports for a file in the same folder
        std::fs::rename(&tmp, &file).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
        region.write().unwrap_or_else(|e| e.into_inner()).mark_saved(version);
        Ok(Some(zipped.len() as u64))
    }

    /// `CachedWorld.save`: upstream's method. With `chunkCaching` off nothing is written, but old chunks still leave memory. Then the far regions are pruned.
    pub fn save(&self) -> SaveReport {
        let _one_at_a_time = self.save_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut report = SaveReport::default();
        let settings = self.settings();
        if !settings.chunk_caching {
            for r in self.all_regions() {
                r.write().unwrap_or_else(|e| e.into_inner()).remove_expired(settings.cached_chunks_expiry_seconds, self.now_ms());
            }
            self.prune();
            return report;
        }
        let began = std::time::Instant::now();
        for r in self.all_regions() {
            match self.save_region(&r) {
                Ok(Some(bytes)) => {
                    report.saved += 1;
                    report.bytes += bytes;
                }
                Ok(None) => {}
                Err(e) => {
                    report.failed += 1;
                    let g = r.read().unwrap_or_else(|e| e.into_inner());
                    self.note(format!("cache region {},{} could not be saved: {e}", g.x, g.z));
                }
            }
        }
        self.note(format!("cache saved: {} regions, {} bytes, {} failed, {} ms", report.saved, report.bytes, report.failed, began.elapsed().as_millis()));
        self.prune();
        report
    }

    /// `CachedWorld.prune`: regions more than 1024 blocks from the player leave memory. Regions with unsaved changes are saved first by `save`; one that
    /// still has unsaved changes (a failed save) stays, unless `chunkCaching` is off (nothing is saved then, and upstream prunes all the same).
    pub fn prune(&self) {
        if !self.settings().prune_regions_from_ram {
            return;
        }
        let (px, pz) = self.guess_position();
        // with `chunkCaching` off nothing is ever saved, so "unsaved" would keep every region for the whole session
        let chunk_caching = self.settings().chunk_caching;
        let mut map = self.regions.write().unwrap_or_else(|e| e.into_inner());
        let far: Vec<i64> = map
            .iter()
            .filter_map(|(id, r)| {
                let g = r.read().unwrap_or_else(|e| e.into_inner());
                // doubles, not ints: past ~46k blocks away the square overflowed in upstream, so far away regions could stay in memory forever
                let dist_x = f64::from((g.x << 9) + 256) - f64::from(px);
                let dist_z = f64::from((g.z << 9) + 256) - f64::from(pz);
                let dist = (dist_x * dist_x + dist_z * dist_z).sqrt();
                (dist > 1024.0 && !(chunk_caching && g.has_unsaved_changes())).then_some(*id)
            })
            .collect();
        for id in far {
            map.remove(&id);
        }
    }

    /// `CachedWorld.guessPosition`: the player's feet if the player is in this world, else the most recently modified chunk, else the origin.
    fn guess_position(&self) -> (i32, i32) {
        if let Some(p) = *self.player.lock().unwrap_or_else(|e| e.into_inner()) {
            return p;
        }
        let mut best: Option<(i64, i32, i32)> = None;
        for r in self.all_regions() {
            let g = r.read().unwrap_or_else(|e| e.into_inner());
            if let Some(c) = g.most_recently_modified() {
                if best.is_none_or(|b| b.0 < c.cache_timestamp) {
                    best = Some((c.cache_timestamp, c.x, c.z));
                }
            }
        }
        match best {
            Some((_, cx, cz)) => ((cx << 4) + 8, (cz << 4) + 8),
            None => (0, 0),
        }
    }

    /// `CachedWorld.reloadAllFromDisk`: upstream's method.
    pub fn reload_all_from_disk(&self) {
        let _disk = self.disk_lock.lock().unwrap_or_else(|e| e.into_inner());
        for r in self.all_regions() {
            let mut g = r.write().unwrap_or_else(|e| e.into_inner());
            self.load_region(&mut g);
        }
    }

    /// The regions with unsaved changes.
    pub fn unsaved_regions(&self) -> usize {
        self.all_regions().iter().filter(|r| r.read().unwrap_or_else(|e| e.into_inner()).has_unsaved_changes()).count()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::cache::chunk::pack;
    use crate::cache::chunk::tests::{overworld, Cells};
    use crate::cache::palette::fixture::*;

    pub fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("baritone-rs-cache-{tag}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0)));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    pub fn world_at(dir: &Path) -> CachedWorld {
        CachedWorld::new(dir.to_path_buf(), overworld(), Arc::new(env()), CacheSettings::default(), Box::new(|| 5_000))
    }

    pub fn chunk_with_chest(cx: i32, cz: i32) -> CachedChunk {
        let mut c = Cells::new(384);
        c.set(3, 134, 4, CHEST);
        c.set(3, 135, 4, STONE);
        pack(&c, cx, cz, &overworld(), &env(), false, 4_000)
    }

    #[test]
    fn regions_save_and_load_back() {
        let dir = temp_dir("roundtrip");
        let w = world_at(&dir);
        w.update_cached_chunk(chunk_with_chest(70, -3)); // region (2, -1)
        w.update_cached_chunk(chunk_with_chest(-1, 0)); // region (-1, 0)
        assert_eq!(w.unsaved_regions(), 2);
        let report = w.save();
        assert_eq!((report.saved, report.failed), (2, 0));
        assert_eq!(w.unsaved_regions(), 0);
        assert!(dir.join("r.2.-1.bcr").exists() && dir.join("r.-1.0.bcr").exists());
        assert!(!dir.join("r.2.-1.bcr.tmp").exists(), "the temporary file is gone");
        // nothing changed: nothing is written again
        assert_eq!(w.save().saved, 0);
        // a new world over the same folder reads them
        let w2 = world_at(&dir);
        assert!(!w2.is_cached(70 * 16, -3 * 16), "regions are read when asked for");
        let found = w2.get_locations_of("chest", 10, 70 * 16, -3 * 16, 10);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&Pos::new(70 * 16 + 3, 70, -3 * 16 + 4)));
        assert!(w2.is_cached(70 * 16, -3 * 16));
        assert_eq!(w2.get_region(2, -1).unwrap().read().unwrap().get_block(70 * 16 + 3 - 2 * 512, 70, -3 * 16 + 4 + 512, &overworld(), &env()), Some(CHEST));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// D4: upstream leaves the region empty after a failed load, and the next save overwrites the file with it.
    #[test]
    fn d4_a_file_that_fails_to_load_is_kept() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let garbage = b"this was a region once, now it is not".to_vec();
        std::fs::write(dir.join("r.0.0.bcr"), &garbage).unwrap();
        let w = world_at(&dir);
        let region = w.get_or_create_region(0, 0);
        assert_eq!(region.read().unwrap().chunk_count(), 0);
        // new data goes into the region and is saved
        w.update_cached_chunk(chunk_with_chest(1, 1));
        assert_eq!(w.save().saved, 1);
        // the bad file is still there, byte for byte, under another name; the new region has its own file
        let kept = std::fs::read(dir.join("r.0.0.bcr.corrupt")).unwrap();
        assert_eq!(kept, garbage);
        assert!(w.take_notes().iter().any(|n| n.contains("failed to load") && n.contains("kept")));
        let w2 = world_at(&dir);
        assert_eq!(w2.get_locations_of("chest", 10, 0, 0, 0).len(), 1);
        // a second damaged file does not replace the first kept one
        std::fs::write(dir.join("r.0.0.bcr"), b"second").unwrap();
        let w3 = world_at(&dir);
        w3.get_or_create_region(0, 0);
        assert_eq!(std::fs::read(dir.join("r.0.0.bcr.corrupt")).unwrap(), garbage);
        assert_eq!(std::fs::read(dir.join("r.0.0.bcr.corrupt1")).unwrap(), b"second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_in_another_blocks_table_is_kept_too() {
        // a valid file whose block names this game lacks fails to load as `stringToBlockRequired` throws: kept, not overwritten
        let dir = temp_dir("names");
        let w = world_at(&dir);
        w.update_cached_chunk(chunk_with_chest(0, 0));
        w.save();
        let original = std::fs::read(dir.join("r.0.0.bcr")).unwrap();
        let other = CachedWorld::new(dir.clone(), overworld(), Arc::new(small_env()), CacheSettings::default(), Box::new(|| 1));
        let r = other.get_or_create_region(0, 0);
        assert_eq!(r.read().unwrap().chunk_count(), 0);
        assert_eq!(std::fs::read(dir.join("r.0.0.bcr.corrupt")).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn small_env() -> CacheEnv {
        // no chest, no spawner
        let mut st = states();
        st.truncate(5);
        let d: Vec<usize> = (0..st.len()).collect();
        CacheEnv::new(&st, &d, &vec![false; st.len()], &|s: &State| !s.fluid_empty)
    }

    #[test]
    fn the_search_widens_by_squared_distance_and_stops_at_the_maximum() {
        let dir = temp_dir("widen");
        let w = world_at(&dir);
        // regions (0,0), (1,0) [distance 1], (1,1) [distance 2], (2,0) [distance 4]
        for rx in [0, 1, 2] {
            w.update_cached_chunk(chunk_with_chest(rx * 32, 0));
        }
        w.update_cached_chunk(chunk_with_chest(32, 32));
        assert_eq!(w.get_locations_of("chest", 1, 5, 5, 2).len(), 1, "the first ring that reaches the maximum ends the search");
        assert_eq!(w.get_locations_of("chest", 100, 5, 5, 2).len(), 3, "radius 2 reaches (1,0) and (1,1), not (2,0)");
        assert_eq!(w.get_locations_of("chest", 100, 5, 5, 4).len(), 4);
        assert!(w.get_locations_of("spawner", 100, 5, 5, 4).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_drops_far_saved_regions_and_keeps_unsaved_ones() {
        let dir = temp_dir("prune");
        let w = world_at(&dir);
        w.update_cached_chunk(chunk_with_chest(0, 0)); // region (0,0)
        w.update_cached_chunk(chunk_with_chest(32 * 10, 0)); // region (10, 0): 5000 blocks away
        w.set_player(Some((10, 10)));
        w.prune();
        assert_eq!(w.region_count(), 2, "unsaved regions stay");
        w.save(); // saves, then prunes
        assert_eq!(w.region_count(), 1);
        assert!(w.get_region(10, 0).is_none() && w.get_region(0, 0).is_some());
        // the file is still on disk and the region comes back when asked for
        assert_eq!(w.get_locations_of("chest", 10, 5120, 0, 0).len(), 1);
        // with pruning off nothing leaves
        let mut s = w.settings();
        s.prune_regions_from_ram = false;
        w.set_settings(s);
        w.prune();
        assert_eq!(w.region_count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chunk_caching_off_writes_nothing() {
        let dir = temp_dir("off");
        let w = world_at(&dir);
        let mut s = w.settings();
        s.chunk_caching = false;
        w.set_settings(s);
        w.update_cached_chunk(chunk_with_chest(0, 0));
        assert_eq!(w.save().saved, 0);
        assert!(!dir.join("r.0.0.bcr").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_far_region_does_not_overflow_the_distance() {
        let dir = temp_dir("far");
        let w = world_at(&dir);
        w.update_cached_chunk(chunk_with_chest(32 * 40_000 / 32, 0));
        w.save();
        w.set_player(Some((-29_000_000, 0)));
        w.prune();
        assert_eq!(w.region_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chunk_packed_again_as_it_was_stores_nothing_and_a_changed_one_does() {
        let dir = temp_dir("again");
        let w = world_at(&dir);
        assert!(w.update_cached_chunk_if_changed(chunk_with_chest(1, 1)), "the first pack is stored");
        w.save();
        assert_eq!(w.unsaved_regions(), 0);
        let mut same = chunk_with_chest(1, 1);
        same.cache_timestamp = 9_999;
        assert!(!w.update_cached_chunk_if_changed(same));
        assert_eq!(w.unsaved_regions(), 0, "nothing to save");
        // a chunk with one block more is a change
        let mut c = Cells::new(384);
        c.set(3, 134, 4, CHEST);
        c.set(3, 135, 4, STONE);
        c.set(7, 134, 4, SPAWNER);
        let more = pack(&c, 1, 1, &overworld(), &env(), false, 4_000);
        assert!(w.update_cached_chunk_if_changed(more));
        assert_eq!(w.unsaved_regions(), 1);
        assert_eq!(w.get_locations_of("spawner", 10, 16, 16, 0).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn review_1_with_chunk_caching_off_a_save_still_empties_far_regions() {
        let dir = temp_dir("pruneoff");
        let w = world_at(&dir);
        let mut s = w.settings();
        s.chunk_caching = false;
        w.set_settings(s);
        w.update_cached_chunk(chunk_with_chest(32 * 10, 0)); // 5000 blocks from the player, unsaved for good
        w.set_player(Some((10, 10)));
        assert_eq!(w.region_count(), 1);
        w.save();
        assert_eq!(w.region_count(), 0, "upstream's save prunes with chunkCaching off too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn review_3_saves_of_one_world_do_not_overlap() {
        let dir = temp_dir("saves");
        let w = Arc::new(world_at(&dir));
        for i in 0..8 {
            w.update_cached_chunk(chunk_with_chest(i, 0));
        }
        let saved: usize = std::thread::scope(|s| {
            let hs: Vec<_> = (0..6).map(|_| s.spawn(|| w.save().saved)).collect();
            hs.into_iter().map(|h| h.join().unwrap()).sum()
        });
        assert_eq!(saved, 1, "one save wrote the region, the others found nothing to write");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn review_7_a_chunk_outside_the_world_is_not_region_zero() {
        let dir = temp_dir("outside");
        let w = world_at(&dir);
        w.update_cached_chunk(chunk_with_chest(70_000_000 / 16, 0));
        assert_eq!(w.region_count(), 0);
        assert!(!w.update_cached_chunk_if_changed(chunk_with_chest(0, -70_000_000 / 16)));
        assert_eq!(w.region_count(), 0);
        w.update_cached_chunk(chunk_with_chest(0, 0));
        assert_eq!(w.region_count(), 1);
        assert_eq!(w.get_locations_of("chest", 10, 70_000_000, 0, 0).len(), 0, "asking far outside finds nothing and makes no region");
        assert_eq!(w.region_count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn review_10_a_region_load_that_panics_can_be_asked_for_again() {
        let dir = temp_dir("panicload");
        let first = world_at(&dir);
        first.update_cached_chunk(chunk_with_chest(0, 0));
        first.save();
        // a world whose clock panics: the load thread panics after the file is read
        let boom = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let b2 = Arc::clone(&boom);
        let w = Arc::new(CachedWorld::new(dir.clone(), overworld(), Arc::new(env()), CacheSettings::default(), Box::new(move || if b2.load(std::sync::atomic::Ordering::Relaxed) { panic!("test: the clock fails") } else { 5_000 })));
        assert!(w.try_load_from_disk_async(0, 0));
        let t = std::time::Instant::now();
        while !w.pending_loads.lock().unwrap().is_empty() {
            assert!(t.elapsed() < std::time::Duration::from_secs(5), "the entry of a load that panicked is still there");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        boom.store(false, std::sync::atomic::Ordering::Relaxed);
        assert!(w.try_load_from_disk_async(0, 0), "asking again starts a read");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
