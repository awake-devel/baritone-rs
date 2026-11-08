// baritone-rs engine: helpers for the tests that put a cache under a `TestWorld`. Written for this project. LGPL-3.0-or-later, see lib.rs.
use super::chunk::{pack, Dimension, PackSource};
use super::palette::CacheEnv;
use super::view::{CacheReader, WithCache};
use super::world::{CacheSettings, CachedWorld};
use crate::blocks::{Block, State};
use crate::testing::TestWorld;
use std::path::PathBuf;
use std::sync::Arc;

/// A chunk of a `TestWorld` for the packer.
pub struct TestChunk<'a> {
    pub world: &'a TestWorld,
    pub cx: i32,
    pub cz: i32,
}

impl PackSource for TestChunk<'_> {
    fn state(&self, x: i32, y: i32, z: i32) -> usize {
        use crate::world::WorldView;
        self.world.state_id(self.cx * 16 + x, y, self.cz * 16 + z)
    }
    fn section_has_only_air(&self, section: usize) -> bool {
        (0..16).all(|y| (0..16).all(|z| (0..16).all(|x| self.state(x, section as i32 * 16 + y, z) == 0)))
    }
}

/// The block table of a test world, by the name of each state id.
pub fn env_of(names: &[&str]) -> CacheEnv {
    let states: Vec<State> = names.iter().map(|n| State { name: (*n).to_string(), block: Block::from_name(n), air: *n == "air", fluid_empty: true, ..State::default() }).collect();
    let default_of: Vec<usize> = (0..states.len()).collect();
    CacheEnv::new(&states, &default_of, &vec![false; states.len()], &|s: &State| !s.fluid_empty)
}

pub fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("baritone-rs-cache-{tag}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0)));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A cached world with every whole chunk of `w` packed into it. The folder is made fresh in the temp directory and is the caller's to remove.
pub fn cache_of(w: &TestWorld, names: &[&str], dir: PathBuf) -> Arc<CachedWorld> {
    let dim = Dimension { min_y: 0, height: w.h, logical_height: w.h, natural: true, ultra_warm: false, has_ceiling: false, end_effects: false };
    let env = Arc::new(env_of(names));
    let world = Arc::new(CachedWorld::new(dir, dim, Arc::clone(&env), CacheSettings::default(), Box::new(|| 1_000)));
    for cx in 0..(w.w + 15) / 16 {
        for cz in 0..(w.d + 15) / 16 {
            world.update_cached_chunk(pack(&TestChunk { world: w, cx, cz }, cx, cz, &dim, &env, false, 1_000));
        }
    }
    world
}

/// The live world with the cache under it.
pub fn with_cache(live: TestWorld, cache: &Arc<CachedWorld>) -> WithCache<TestWorld> {
    WithCache::new(live, Some(CacheReader::new(Arc::clone(cache))))
}
