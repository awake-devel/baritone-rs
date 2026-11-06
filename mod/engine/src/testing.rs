// baritone-rs engine: fixtures for the unit tests of the search. Written for this project. LGPL-3.0-or-later, see lib.rs.
use crate::blocks::{Block, State};
use crate::config::Config;
use crate::goals::Goal;
use crate::movement::helper::{Border, Ctx, Precomputed};
use crate::pos::Pos;
use crate::search::node::NodeMap;
use crate::search::{Path, SearchError};
use crate::world::WorldView;

pub const AIR: u16 = 0;
pub const STONE: u16 = 1;

/// A settings snapshot with the settings the tests rely on written out: sprinting on, nothing is broken or placed, no parkour.
pub fn test_config() -> Config {
    Config {
        has_throwaway: false,
        has_water_bucket: false,
        can_sprint: true,
        frost_walker: 0,
        water_mult: 1.0,
        allow_break: false,
        allow_break_anyway: vec![],
        blocks_to_avoid: vec![],
        blocks_to_disallow_breaking: vec![],
        assume_walk_on_water: false,
        assume_walk_on_lava: false,
        allow_walk_on_magma: false,
        allow_vines: false,
        allow_walk_on_bottom_slab: true,
        strict_liquid_check: false,
        avoid_updating_falling: true,
        allow_place_source: false,
        allow_place_flow: false,
        allow_swimming: false,
        allow_parkour: false,
        allow_parkour_place: false,
        allow_parkour_ascend: true,
        allow_jump_at_build_limit: false,
        allow_diagonal_descend: false,
        allow_diagonal_ascend: false,
        allow_downward: false,
        max_fall_no_water: 3,
        max_fall_bucket: 20,
        place_penalty: 20.0,
        break_penalty: 2.0,
        jump_penalty: 2.0,
        walk_on_water_penalty: 3.0,
        cost_heuristic: 3.563,
        axis_height: 120,
        search: Default::default(),
    }
}

/// Two block states, air and stone, and what a `Ctx` needs besides the world.
pub struct Fixture {
    pub states: Vec<State>,
    pub pre: Precomputed,
    pub cfg: Config,
    pub tool: Vec<f64>,
}

impl Fixture {
    pub fn new() -> Fixture {
        Fixture::with_config(test_config())
    }

    pub fn with_config(cfg: Config) -> Fixture {
        let air = State { name: "air".into(), block: Block::Air, air: true, pathfindable: true, fluid_empty: true, replaceable: true, ..State::default() };
        let stone = State { name: "stone".into(), shape_full: true, top_sturdy: true, fluid_empty: true, ..State::default() };
        let states = vec![air, stone];
        let pre = Precomputed::new(&states, &cfg);
        Fixture { states, pre, cfg, tool: vec![1.0, 1.0] }
    }

    pub fn ctx<'a, W: WorldView>(&'a self, world: &'a W) -> Ctx<'a, W> {
        let border = Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 };
        Ctx::new(world, &self.states, &self.pre, &self.cfg, border, &self.tool)
    }
}

/// A world of `w * d` columns, 16 blocks high, from the origin. Everything outside the columns is air. Columns with `x >= loaded_below_x` are not loaded.
pub struct TestWorld {
    pub w: i32,
    pub d: i32,
    cells: Vec<u16>,
    pub loaded_below_x: i32,
}

impl TestWorld {
    /// A stone floor at y = 0, and air above it.
    pub fn flat(w: i32, d: i32) -> TestWorld {
        let mut t = TestWorld { w, d, cells: vec![AIR; (w * d * 16) as usize], loaded_below_x: i32::MAX };
        for x in 0..w {
            for z in 0..d {
                t.set(x, 0, z, STONE);
            }
        }
        t
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        let i = ((y * self.d + z) * self.w + x) as usize;
        self.cells[i] = id;
    }
}

impl WorldView for TestWorld {
    fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
        if x < 0 || x >= self.w || z < 0 || z >= self.d {
            return AIR as usize;
        }
        self.cells[((y * self.d + z) * self.w + x) as usize] as usize
    }
    fn contains_loaded_chunk(&self, x: i32, _z: i32) -> bool {
        x < self.loaded_below_x
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        16
    }
}

/// A verified path through the given positions, made from hand-built nodes of cost 0, 10, 20, ... Nothing checks that the steps are possible, only that each one ends where a move of `Moves::ALL` ends.
pub fn chain_path<W: WorldView>(ctx: &Ctx<'_, W>, pts: &[Pos]) -> Result<Path, SearchError> {
    let goal = Goal::Block { x: 0, y: 0, z: 0 };
    let gs = ctx.cfg.goal_settings();
    let mut map = NodeMap::default();
    let mut ids = vec![];
    for (i, p) in pts.iter().enumerate() {
        let id = map.get_node_at_position(p.x, p.y, p.z, &goal, &gs)?;
        map.nodes[id as usize].cost = i as f64 * 10.0;
        if i > 0 {
            map.nodes[id as usize].previous = Some(ids[i - 1]);
        }
        ids.push(id);
    }
    let raw = Path::from_nodes(pts[0], ids[0], *ids.last().unwrap(), 7, &goal, &gs, &map.nodes)?;
    raw.post_process(ctx)
}
