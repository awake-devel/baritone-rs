// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/BlockStateInterface.java (getUncached, isLoaded: the fallback to the cached world).
// LGPL-3.0-or-later, see lib.rs.
//
// The search reads blocks through a `WorldView`. `WithCache` wraps one: a block in a loaded chunk is read from it, a block elsewhere from the cached region
// that holds it, and a block in neither is air, as `BlockStateInterface.get0` does. The search code is not changed: it asks the same trait.
use super::region::CachedRegion;
use super::world::CachedWorld;
use crate::pos::Pos;
use crate::world::{ChunkStatus, WorldView};
use std::cell::RefCell;
use std::sync::{Arc, RwLock};

type Memo = Option<(i32, i32, Arc<RwLock<CachedRegion>>)>;

/// A reader of one cached world. Remembers the region it read last (`prevCached`: "there's great cache locality in block state lookups"), so it is
/// cheap to clone and not shared between threads.
pub struct CacheReader {
    world: Arc<CachedWorld>,
    memo: RefCell<Memo>,
}

impl Clone for CacheReader {
    fn clone(&self) -> CacheReader {
        CacheReader { world: Arc::clone(&self.world), memo: RefCell::new(None) }
    }
}

impl CacheReader {
    pub fn new(world: Arc<CachedWorld>) -> CacheReader {
        CacheReader { world, memo: RefCell::new(None) }
    }

    pub fn world(&self) -> &Arc<CachedWorld> {
        &self.world
    }

    fn region(&self, x: i32, z: i32) -> Option<Arc<RwLock<CachedRegion>>> {
        let (rx, rz) = (x >> 9, z >> 9);
        let mut memo = self.memo.borrow_mut();
        if let Some((mx, mz, r)) = memo.as_ref() {
            if *mx == rx && *mz == rz {
                return Some(Arc::clone(r));
            }
        }
        let r = self.world.get_region(rx, rz)?;
        *memo = Some((rx, rz, Arc::clone(&r)));
        Some(r)
    }

    /// `BlockStateInterface.getUncached`'s cached half: the state of the block in the cache. `None` is air (no region, no chunk, or a height outside the world).
    pub fn state(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let r = self.region(x, z)?;
        let g = r.read().unwrap_or_else(|e| e.into_inner());
        g.get_block(x & 511, y, z & 511, &self.world.dimension, &self.world.env)
    }

    /// The cached half of `BlockStateInterface.isLoaded`.
    pub fn is_cached(&self, x: i32, z: i32) -> bool {
        match self.region(x, z) {
            Some(r) => r.read().unwrap_or_else(|e| e.into_inner()).is_cached(x & 511, z & 511),
            None => false,
        }
    }
}

/// A world with the cache under it.
pub struct WithCache<W: WorldView> {
    pub live: W,
    pub cache: Option<CacheReader>,
    /// `pathThroughCachedOnly`: loaded chunks are not read at all.
    pub cached_only: bool,
}

impl<W: WorldView> WithCache<W> {
    pub fn new(live: W, cache: Option<CacheReader>) -> WithCache<W> {
        WithCache { live, cache, cached_only: false }
    }
}

impl<W: WorldView> WorldView for WithCache<W> {
    /// `BlockStateInterface.getUncached`: upstream's method.
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
        if !self.cached_only {
            if let Some(s) = self.live.loaded_state_id(x, y, z) {
                return s;
            }
        }
        if let Some(c) = &self.cache {
            if let Some(s) = c.state(x, y, z) {
                return s;
            }
        }
        // not loaded and not cached: what the live world says for such a block (air)
        self.live.state_id(x, y, z)
    }
    fn loaded_state_id(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        self.live.loaded_state_id(x, y, z)
    }
    fn contains_loaded_chunk(&self, x: i32, z: i32) -> bool {
        self.live.contains_loaded_chunk(x, z)
    }
    /// `BlockStateInterface.isLoaded`: upstream's method.
    fn is_loaded(&self, x: i32, z: i32) -> bool {
        if !self.cached_only && self.live.is_loaded(x, z) {
            return true;
        }
        self.cache.as_ref().is_some_and(|c| c.is_cached(x, z))
    }
    fn is_open(&self, x: i32, y: i32, z: i32) -> bool {
        self.live.is_open(x, y, z)
    }
    fn falling_block_near(&self, x: i32, y: i32, z: i32) -> bool {
        self.live.falling_block_near(x, y, z)
    }
    fn chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.live.chunk_loaded(chunk_x, chunk_z)
    }
    fn section_has_only_air(&self, chunk_x: i32, chunk_z: i32, section: i32) -> bool {
        self.live.section_has_only_air(chunk_x, chunk_z, section)
    }
    fn min_y(&self) -> i32 {
        self.live.min_y()
    }
    fn height(&self) -> i32 {
        self.live.height()
    }
    /// `ExploreProcess.BaritoneChunkCache.isAlreadyExplored`: upstream's method (it asks the cache only, not the loaded world).
    fn chunk_status(&self, chunk_x: i32, chunk_z: i32) -> ChunkStatus {
        let Some(c) = &self.cache else {
            return self.live.chunk_status(chunk_x, chunk_z);
        };
        let (center_x, center_z) = (chunk_x.wrapping_shl(4), chunk_z.wrapping_shl(4));
        if c.world.is_cached(center_x, center_z) {
            return ChunkStatus::Explored;
        }
        if !c.world.region_loaded(center_x, center_z) {
            c.world.try_load_from_disk_async(center_x >> 9, center_z >> 9);
            return ChunkStatus::Unknown; // we still need to load regions from disk in order to decide properly
        }
        ChunkStatus::NotExplored
    }
    fn cached_locations(&self, block: &str, maximum: i32, center_x: i32, center_z: i32, max_region_distance_sq: i32) -> Vec<Pos> {
        match &self.cache {
            Some(c) => c.world.get_locations_of(block, maximum, center_x, center_z, max_region_distance_sq),
            None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::testkit::{cache_of, temp_dir, with_cache};
    use crate::testing::{TestWorld, STONE};

    /// The search and the scans run on threads of their own: a world with the cache under it has to be sendable.
    #[test]
    fn a_world_with_a_cache_can_go_to_the_search_thread() {
        fn sendable<T: Send + 'static>() {}
        sendable::<WithCache<TestWorld>>();
        let _worker = crate::worker::Worker::<WithCache<TestWorld>>::spawn(crate::worker::monotonic_ms_clock());
    }

    /// `BlockStateInterface.get0` / `isLoaded`: loaded chunks first, then the cache, then air.
    #[test]
    fn a_block_is_read_from_the_loaded_chunk_first_then_the_cache_then_it_is_air() {
        let mut known = TestWorld::flat(64, 16);
        known.set(40, 3, 5, STONE);
        known.set(8, 3, 5, STONE);
        let dir = temp_dir("view");
        let cache = cache_of(&known, &["air", "stone"], dir.clone());
        let mut live = TestWorld::flat(64, 16);
        live.loaded_below_x = 16;
        let mut w = with_cache(live, &cache);
        // unloaded and cached: the cache's block, where the live world would say air
        assert_eq!(w.state_id(40, 3, 5), STONE as usize);
        assert_eq!(w.state_id(40, 4, 5), 0);
        // loaded: the live world, which has no block at (8, 3, 5) although the cache has one
        assert_eq!(w.state_id(8, 3, 5), 0);
        // above and below the world the cache has nothing, so the live world answers
        assert!(w.cache.as_ref().unwrap().state(40, 99, 5).is_none() && w.cache.as_ref().unwrap().state(40, -5, 5).is_none());
        // in neither: air
        assert_eq!(w.state_id(5000, 3, 5), 0);
        assert!(w.is_loaded(8, 5) && w.is_loaded(40, 5) && !w.is_loaded(5000, 5));
        assert!(!w.contains_loaded_chunk(40, 5), "worldContainsLoadedChunk is the live world's");
        // pathThroughCachedOnly: the loaded chunks are not read
        w.cached_only = true;
        assert_eq!(w.state_id(8, 3, 5), STONE as usize);
        // no cache at all: the live world's answers
        let bare = WithCache::new(TestWorld::flat(64, 16), None);
        assert!(!bare.is_loaded(40_000, 5) || bare.live.contains_loaded_chunk(40_000, 5));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `ExploreProcess.BaritoneChunkCache.isAlreadyExplored`: a cached chunk is explored; a chunk in a region that is not in memory is unknown while the
    /// region is read from its file on another thread, and then explored or not; a region without a file comes in empty.
    #[test]
    fn the_chunk_status_reads_a_region_from_disk_in_the_background() {
        use crate::cache::{CacheSettings, CachedWorld};
        let known = TestWorld::flat(64, 64); // 4 x 4 chunks, region (0, 0)
        let dir = temp_dir("status");
        let cache = cache_of(&known, &["air", "stone"], dir.clone());
        assert_eq!(cache.save().saved, 1);
        // a new world over the same folder: nothing is in memory yet
        let fresh = Arc::new(CachedWorld::new(dir.clone(), cache.dimension, Arc::clone(&cache.env), CacheSettings::default(), Box::new(|| 1_000)));
        let w = with_cache(TestWorld::flat(64, 64), &fresh);
        assert!(!fresh.region_loaded(0, 0));
        assert_eq!(w.chunk_status(1, 1), ChunkStatus::Unknown, "the region is on disk: ask again later");
        let mut waited = 0;
        while w.chunk_status(1, 1) == ChunkStatus::Unknown {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
            assert!(waited < 500, "the region never came in");
        }
        assert_eq!(w.chunk_status(1, 1), ChunkStatus::Explored, "{:?}", fresh.take_notes());
        assert_eq!(w.chunk_status(3, 3), ChunkStatus::Explored);
        assert_eq!(w.chunk_status(4, 0), ChunkStatus::NotExplored, "the region is in memory and has no such chunk");
        assert_eq!(w.chunk_status(-1, 0), ChunkStatus::Unknown, "another region: a read is started for it");
        while w.chunk_status(-1, 0) == ChunkStatus::Unknown {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
            assert!(waited < 500, "the empty region never came in");
        }
        assert_eq!(w.chunk_status(-1, 0), ChunkStatus::NotExplored, "no file: an empty region");
        assert_eq!(w.chunk_status(-33, 0), ChunkStatus::Unknown);
        // without a cache the live world decides
        let bare = WithCache::new(TestWorld::flat(64, 64), None);
        assert_eq!(bare.chunk_status(1, 1), ChunkStatus::Explored);
        assert_eq!(bare.chunk_status(40, 1), ChunkStatus::NotExplored);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Asking about every chunk of an unread region every tick starts one read, not one per question (`pendingLoads`).
    #[test]
    fn a_region_is_read_once_however_often_it_is_asked_for() {
        let dir = temp_dir("pending");
        let cache = cache_of(&TestWorld::flat(16, 16), &["air", "stone"], dir.clone());
        assert_eq!(cache.save().saved, 1);
        let fresh = Arc::new(crate::cache::CachedWorld::new(dir.clone(), cache.dimension, Arc::clone(&cache.env), crate::cache::CacheSettings::default(), Box::new(|| 1_000)));
        let started = (0..200).filter(|_| fresh.try_load_from_disk_async(5, 5)).count();
        assert!((1..200).contains(&started), "{started} reads started");
        let mut waited = 0;
        while !fresh.region_loaded(5 * 512, 5 * 512) {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
            assert!(waited < 500);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
