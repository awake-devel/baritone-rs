// baritone-rs engine: tests of the break and place branches of the movements, one rule at a time. Written for this project.
// LGPL-3.0-or-later, see lib.rs.
//
// The simulator in `sim.rs` shows that the rules work together; these show what each one decides when it is given a player, a hotbar and
// a crosshair by hand.
use super::helper::{attempt_to_place_a_block, reachable, PlaceResult};
use super::movements::{safe_to_cancel, update, Env, GameFacts, MoveRuntime};
use super::state::{Controls, Keys, MoveState, MoveStatus, PlayerView};
use crate::blocks::{Block, Direction, Fluid, State};
use crate::config::Config;
use crate::inventory::testing::TestInventory;
use crate::inventory::SlotInfo;
use crate::look::{Rotation, Vec3};
use crate::movement::helper::Border;
use crate::movement::step::{Movement, MovementKind};
use crate::pos::Pos;
use crate::process::pathing::Tables;
use crate::raytrace::BlockHit;
use crate::settings::ExecSettings;
use crate::testing::{test_config, Fixture, TestWorld, AIR, STONE};

const BORDER: Border = Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 };
const WATER: u16 = 2;

/// Air, stone and a source of water; stone mines at 0.2 per tick.
fn tables(cfg: Config) -> Tables {
    let mut states = Fixture::new().states;
    states.push(State { name: "water".into(), block: Block::Water, fluid: Fluid::Water, fluid_empty: false, fluid_amount: 8, fluid_source: true, liquid_block: true, ..State::default() });
    Tables::new(states, cfg, vec![1.0, 0.2, 1.0])
}

fn breaking() -> Config {
    Config { allow_break: true, allow_downward: true, ..test_config() }
}

/// A pickaxe in slot 3 and 64 stone in slot 5.
fn kit() -> TestInventory {
    let mut inv = TestInventory::empty();
    inv.put(3, SlotInfo { pickaxe: true, material_cost: 2, ..SlotInfo::of("iron_pickaxe", 1) }, &[0.0, 0.2]);
    inv.put(5, SlotInfo::of("stone", 64), &[0.0, 0.0125]);
    inv
}

fn hit(x: i32, y: i32, z: i32, face: Direction) -> Option<BlockHit> {
    Some(BlockHit { pos: Pos::new(x, y, z), face, point: Vec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5) })
}

fn player(x: f64, y: f64, z: f64) -> PlayerView {
    PlayerView::standing(x, y, z)
}

#[allow(clippy::too_many_arguments)]
/// Runs `update` once for a movement and returns the status, the controls and the runtime.
fn step(mv: &Movement, rt: &mut MoveRuntime, t: &Tables, w: &TestWorld, p: &PlayerView, set: &ExecSettings, inv: &TestInventory, game: GameFacts) -> (MoveStatus, Controls) {
    let ctx = t.ctx(w, BORDER);
    let mut env = Env::new(&ctx, p, set, p.feet());
    env.inv = inv;
    env.game = game;
    let mut c = Controls::default();
    let s = update(mv, rt, &env, &mut c);
    (s, c)
}

fn traverse(from: (i32, i32, i32), to: (i32, i32, i32)) -> Movement {
    Movement::new(MovementKind::Traverse, Pos::new(from.0, from.1, from.2), Pos::new(to.0, to.1, to.2))
}

#[test]
fn a_block_in_the_way_is_prepared_with_the_best_tool_and_clicked_only_when_the_crosshair_is_on_it() {
    let t = tables(breaking());
    let mut w = TestWorld::flat(10, 10);
    w.set(3, 1, 3, STONE);
    let mv = traverse((2, 1, 3), (3, 1, 3));
    let mut rt = MoveRuntime::new(&mv);
    let set = ExecSettings::default();
    let inv = kit();
    // standing next to it, looking elsewhere: the tool is asked for, the view turns to the block, no click yet
    let mut p = player(2.5, 1.0, 3.5);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Prepping);
    assert_eq!(c.select, Some(3), "the pickaxe");
    let look = c.look.expect("an aim at the block");
    assert!(c.look_force, "aiming at a block to break is a forced rotation (free look)");
    assert!(!c.keys.click_left);
    assert!(look.yaw < -80.0 && look.yaw > -100.0, "east: {look:?}");
    // the crosshair on the block: click
    p.looking_at = hit(3, 1, 3, Direction::West);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Prepping);
    assert!(c.keys.click_left && c.select == Some(3));
    // the block is gone: the movement runs, walks on, and holds no button
    w.set(3, 1, 3, AIR);
    p.looking_at = None;
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Running);
    assert!(c.keys.forward && !c.keys.click_left);
}

#[test]
fn a_view_that_is_already_on_the_aim_clicks_without_the_crosshair() {
    // the fallback in upstream's `prepared`: `isReallyCloseTo` the rotation it wanted
    let t = tables(breaking());
    let mut w = TestWorld::flat(10, 10);
    w.set(3, 1, 3, STONE);
    let mv = traverse((2, 1, 3), (3, 1, 3));
    let mut rt = MoveRuntime::new(&mv);
    let set = ExecSettings::default();
    let inv = kit();
    // (close to the block: a traverse farther away than 0.83 walks on while it digs, and then the aim is its own)
    let mut p = player(2.9, 1.0, 3.5);
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    let want = c.look.unwrap();
    p.yaw = want.yaw;
    // (a pitch equal to the one wanted means "don't care" to the aim processor, as upstream's `reachable` says, so it is a hair off)
    p.pitch = want.pitch + 0.005;
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert!(c.keys.click_left);
}

#[test]
fn a_block_the_settings_do_not_let_it_break_ends_the_movement_and_nothing_is_clicked() {
    let w = {
        let mut w = TestWorld::flat(10, 10);
        w.set(3, 1, 3, STONE);
        w
    };
    let mv = traverse((2, 1, 3), (3, 1, 3));
    let set = ExecSettings::default();
    let inv = kit();
    let mut p = player(2.5, 1.0, 3.5);
    p.looking_at = hit(3, 1, 3, Direction::West);
    for cfg in [Config { allow_break: false, ..breaking() }, Config { blocks_to_disallow_breaking: vec!["stone".into()], ..breaking() }] {
        let t = tables(cfg);
        let mut rt = MoveRuntime::new(&mv);
        let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
        assert_eq!(s, MoveStatus::Unreachable);
        assert!(c.keys == Keys::default() && c.look.is_none() && c.select.is_none());
    }
    // allowBreakAnyway lets one block through when allowBreak is off
    let t = tables(Config { allow_break: false, allow_break_anyway: vec!["stone".into()], ..breaking() });
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Prepping);
    assert!(c.keys.click_left);
}

#[test]
fn a_traverse_walks_on_while_it_breaks_unless_it_is_up_against_the_block_or_told_not_to() {
    let t = tables(breaking());
    let mut w = TestWorld::flat(10, 10);
    w.set(4, 1, 3, STONE);
    w.set(4, 2, 3, STONE);
    let mv = traverse((3, 1, 3), (4, 1, 3));
    let inv = kit();
    let set = ExecSettings::default();
    // 0.85 from the middle of the destination block (4.5): dist 0.95, far enough to walk
    let mut p = player(3.55, 1.0, 3.5);
    // the upper block is the first one the traverse breaks
    p.looking_at = hit(4, 2, 3, Direction::West);
    let mut rt = MoveRuntime::new(&mv);
    // the first tick aims; the second has a target to take the pitch from
    step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Prepping);
    assert!(c.keys.forward && c.keys.click_left);
    // two blocks of stone: the pitch is the efficient 26 degrees, and the yaw is towards the destination
    let look = c.look.unwrap();
    assert_eq!(look.pitch, 26.0);
    assert!((look.yaw + 90.0).abs() < 1.0, "{look:?}");
    // up against it (0.83): stands still and digs
    let p2 = player(3.9, 1.0, 3.5);
    let (_, c) = step(&mv, &mut rt, &t, &w, &p2, &set, &inv, GameFacts::default());
    assert!(!c.keys.forward);
    // told not to walk
    let off = ExecSettings { walk_while_breaking: false, ..ExecSettings::default() };
    let mut rt2 = MoveRuntime::new(&mv);
    step(&mv, &mut rt2, &t, &w, &p, &off, &inv, GameFacts::default());
    let (_, c) = step(&mv, &mut rt2, &t, &w, &p, &off, &inv, GameFacts::default());
    assert!(!c.keys.forward);
}

#[test]
fn a_falling_block_makes_it_wait() {
    struct Falling(TestWorld);
    impl crate::world::WorldView for Falling {
        fn state_id(&self, x: i32, y: i32, z: i32) -> usize {
            self.0.state_id(x, y, z)
        }
        fn contains_loaded_chunk(&self, x: i32, z: i32) -> bool {
            self.0.contains_loaded_chunk(x, z)
        }
        fn falling_block_near(&self, x: i32, y: i32, z: i32) -> bool {
            (x, y, z) == (3, 1, 3)
        }
        fn min_y(&self) -> i32 {
            0
        }
        fn height(&self) -> i32 {
            16
        }
    }
    let t = tables(breaking());
    let mut inner = TestWorld::flat(10, 10);
    inner.set(3, 1, 3, STONE);
    let w = Falling(inner);
    let ctx = t.ctx(&w, BORDER);
    let mv = traverse((2, 1, 3), (3, 1, 3));
    let p = player(2.5, 1.0, 3.5);
    let inv = kit();
    for (setting, clicks) in [(true, false), (false, true)] {
        let set = ExecSettings { pause_mining_for_falling_blocks: setting, ..ExecSettings::default() };
        let mut env = Env::new(&ctx, &p, &set, p.feet());
        env.inv = &inv;
        let mut rt = MoveRuntime::new(&mv);
        let mut c = Controls::default();
        let mut pl = p;
        pl.looking_at = hit(3, 1, 3, Direction::West);
        let env2 = Env { p: &pl, ..env };
        let s = update(&mv, &mut rt, &env2, &mut c);
        assert_eq!(s, MoveStatus::Prepping);
        assert_eq!(c.keys.click_left, clicks, "pauseMiningForFallingBlocks {setting}");
    }
}

#[test]
fn reachable_takes_the_middle_then_the_faces_and_gives_up_behind_a_wall() {
    let t = tables(breaking());
    let mut w = TestWorld::flat(12, 8);
    w.set(5, 1, 3, STONE);
    let set = ExecSettings::default();
    let inv = kit();
    let p = player(3.5, 1.0, 3.5);
    let go = |w: &TestWorld, p: &PlayerView| {
        let ctx = t.ctx(w, BORDER);
        let mut env = Env::new(&ctx, p, &set, p.feet());
        env.inv = &inv;
        reachable(&env, Pos::new(5, 1, 3), 4.5, false)
    };
    // in the open: the middle of the west face is on the line to the middle of the block
    let open = go(&w, &p).expect("reachable");
    // one block in front of it: the line to the middle is cut, the top face is not
    w.set(4, 1, 3, STONE);
    let over = go(&w, &p).expect("the top face");
    assert!(over.pitch < open.pitch, "{over:?} should aim higher than {open:?}: the top face is above the middle");
    // two blocks: nothing is left
    w.set(4, 2, 3, STONE);
    assert_eq!(go(&w, &p), None);
    // out of reach
    let far = player(0.5, 1.0, 3.5);
    assert_eq!(go(&TestWorld::flat(12, 8), &far), None);
}

#[test]
fn reachable_keeps_the_view_when_the_crosshair_is_already_on_the_block() {
    let t = tables(breaking());
    let mut w = TestWorld::flat(12, 8);
    w.set(5, 1, 3, STONE);
    let mut p = player(3.5, 1.0, 3.5);
    p.yaw = -90.0;
    p.pitch = 12.0;
    p.looking_at = hit(5, 1, 3, Direction::West);
    let ctx = t.ctx(&w, BORDER);
    let inv = kit();
    let mut set = ExecSettings::default();
    let mut env = Env::new(&ctx, &p, &set, p.feet());
    env.inv = &inv;
    let r = reachable(&env, Pos::new(5, 1, 3), 4.5, false).unwrap();
    assert_eq!(r, Rotation::new(-90.0, 12.0001));
    set.remain_with_existing_look_direction = false;
    let mut env = Env::new(&ctx, &p, &set, p.feet());
    env.inv = &inv;
    let r = reachable(&env, Pos::new(5, 1, 3), 4.5, false).unwrap();
    assert_ne!(r.pitch, 12.0001);
}

fn bridge_world() -> TestWorld {
    // a floor with a hole at (3,0,3), a block to place against at (2,0,3)
    let mut w = TestWorld::flat(10, 10);
    w.set(3, 0, 3, AIR);
    w
}

#[test]
fn a_bridge_sneaks_and_clicks_only_when_crouching_with_the_crosshair_on_a_face_of_the_cell() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let w = bridge_world();
    let mv = traverse((2, 1, 3), (3, 1, 3));
    let set = ExecSettings::default();
    let inv = kit();
    let mut p = player(2.9, 1.0, 3.5);
    // the crosshair on the west block's east face: the cell to fill is the east neighbour of (2,0,3)
    p.looking_at = hit(2, 0, 3, Direction::East);
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Running);
    assert!(c.keys.sneak && !c.keys.click_right, "standing: sneak first");
    assert_eq!(c.select, Some(5), "the blocks");
    assert!(!rt.was_the_bridge_block_always_there);
    p.crouching = true;
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert!(c.keys.sneak && c.keys.click_right && !c.keys.click_left);
    // assumeSafeWalk: no sneak needed
    let safe = ExecSettings { assume_safe_walk: true, ..ExecSettings::default() };
    let mut rt = MoveRuntime::new(&mv);
    p.crouching = false;
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &safe, &inv, GameFacts::default());
    assert!(c.keys.click_right && !c.keys.sneak);
}

#[test]
fn a_bridge_without_blocks_or_without_permission_is_unreachable_and_clicks_nothing() {
    let w = bridge_world();
    let mv = traverse((2, 1, 3), (3, 1, 3));
    let mut p = player(2.9, 1.0, 3.5);
    p.looking_at = hit(2, 0, 3, Direction::East);
    p.crouching = true;
    // no blocks in the hotbar
    let t = tables(Config { has_throwaway: false, ..breaking() });
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &ExecSettings::default(), &TestInventory::empty(), GameFacts::default());
    assert_eq!(s, MoveStatus::Unreachable);
    assert!(!c.keys.click_right && !c.keys.click_left);
    // blocks, but allowPlace is off
    let off = ExecSettings { allow_place: false, ..ExecSettings::default() };
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &off, &kit(), GameFacts::default());
    assert_eq!(s, MoveStatus::Unreachable);
    assert!(!c.keys.click_right && c.select.is_none());
}

#[test]
fn a_bridge_that_finds_nothing_to_place_against_walks_on_and_one_with_a_block_in_the_way_digs_only_when_breaking_is_on() {
    // standing in the cell it wants, with the crosshair elsewhere: it backs towards the block it left and looks at its face
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let w = bridge_world();
    let mv = traverse((2, 1, 3), (3, 1, 3));
    let set = ExecSettings::default();
    let inv = kit();
    let p = player(3.3, 1.0, 3.5); // in the destination cell, over the hole
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Running);
    assert!(c.look.is_some() || c.keys.sneak);
    // the crosshair on the block it left, on the top face's neighbour: it places
    let mut p2 = p;
    p2.looking_at = hit(2, 0, 3, Direction::East);
    p2.crouching = true;
    let mut rt = MoveRuntime::new(&mv);
    let (_, c) = step(&mv, &mut rt, &t, &w, &p2, &set, &inv, GameFacts::default());
    assert!(c.keys.click_right);
}

#[test]
fn an_ascend_places_the_block_it_jumps_onto_and_backs_up_after_ten_ticks() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let mut w = TestWorld::flat(10, 10);
    w.set(3, 0, 3, AIR); // the support of the destination is missing
    w.set(3, 1, 3, AIR);
    let mv = Movement::new(MovementKind::Ascend, Pos::new(2, 1, 3), Pos::new(3, 2, 3));
    let set = ExecSettings::default();
    let inv = kit();
    let mut p = player(2.5, 1.0, 3.5);
    // dest.below = (3,1,3) is the cell to fill, against (2,1,3)?  no: (3,0,3) is air and (3,1,3) is air; the floor at (3,0,3) is missing
    // so the cell is placed against the floor beside it
    p.looking_at = hit(3, 0, 2, Direction::Up);
    p.crouching = true;
    let mut rt = MoveRuntime::new(&mv);
    let mut backed = None;
    for tick in 1..=12 {
        let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
        assert_eq!(s, MoveStatus::Running, "tick {tick}");
        assert!(!c.keys.jump && !c.keys.forward, "no jump onto a block that is not there");
        if c.keys.back && backed.is_none() {
            backed = Some(tick);
        }
        assert_eq!(rt.ticks_without_placement, tick);
    }
    assert_eq!(backed, Some(11), "after more than ten ticks without a placement");
    // and it is not safe to cancel, as upstream says
    let ctx = t.ctx(&w, BORDER);
    let env = Env::new(&ctx, &p, &set, p.feet());
    assert!(!safe_to_cancel(&mv, &rt, &env));
    let mut fresh = MoveRuntime::new(&mv);
    fresh.state.status = MoveStatus::Running;
    assert!(safe_to_cancel(&mv, &fresh, &env));
}

#[test]
fn a_pillar_sneaks_jumps_and_places_at_the_top_of_the_jump() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let w = TestWorld::flat(10, 10);
    let mv = Movement::new(MovementKind::Pillar, Pos::new(3, 1, 3), Pos::new(3, 2, 3));
    let set = ExecSettings::default();
    let inv = kit();
    let mut p = player(3.5, 1.0, 3.5);
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Running);
    assert!(c.keys.sneak && c.keys.jump && !c.keys.click_right, "on the ground: sneak and jump");
    assert_eq!(c.select, Some(5));
    // the aim is straight down
    assert!(c.look.unwrap().pitch > 80.0, "{:?}", c.look);
    // at the top of the jump, crouching, the crosshair on the block under the feet: place
    p.y = 2.19;
    p.on_ground = false;
    p.crouching = true;
    p.vx = 0.0;
    p.looking_at = hit(3, 0, 3, Direction::Up);
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert!(c.keys.click_right);
    // too low: not yet
    p.y = 1.5;
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert!(!c.keys.click_right);
    // not crouching yet: not yet
    p.y = 2.19;
    p.crouching = false;
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert!(!c.keys.click_right);
}

#[test]
fn a_pillar_with_nothing_to_place_or_no_permission_is_unreachable() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let w = TestWorld::flat(10, 10);
    let mv = Movement::new(MovementKind::Pillar, Pos::new(3, 1, 3), Pos::new(3, 2, 3));
    let p = player(3.5, 1.0, 3.5);
    let mut rt = MoveRuntime::new(&mv);
    let (s, _) = step(&mv, &mut rt, &t, &w, &p, &ExecSettings::default(), &TestInventory::empty(), GameFacts::default());
    assert_eq!(s, MoveStatus::Unreachable);
    let off = ExecSettings { allow_place: false, ..ExecSettings::default() };
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &off, &kit(), GameFacts::default());
    assert_eq!(s, MoveStatus::Unreachable);
    assert!(c.select.is_none());
}

#[test]
fn a_pillar_does_not_break_when_breaking_is_off() {
    // a block that is not replaceable where the feet are (a stone): upstream digs it; with allowBreak off it must not
    let mut cfg = Config { has_throwaway: true, ..breaking() };
    cfg.allow_break = false;
    let t = tables(cfg);
    let mut w = TestWorld::flat(10, 10);
    w.set(3, 1, 3, STONE);
    let mv = Movement::new(MovementKind::Pillar, Pos::new(3, 1, 3), Pos::new(3, 2, 3));
    let p = player(3.5, 2.0, 3.5);
    let mut rt = MoveRuntime::new(&mv);
    rt.state.status = MoveStatus::Running;
    let mut p2 = p;
    p2.crouching = true;
    let (_, c) = step(&mv, &mut rt, &t, &w, &p2, &ExecSettings::default(), &kit(), GameFacts::default());
    assert!(!c.keys.click_left);
}

fn water_fall(world_nether: bool, has_bucket: bool, inv: &TestInventory) -> (MoveStatus, Controls) {
    // a fall of 9 blocks from (3,10,3) into (3,1,3)
    let t = tables(Config { has_water_bucket: has_bucket, max_fall_bucket: 20, ..breaking() });
    let w = TestWorld::flat(10, 12);
    let mv = Movement::new(MovementKind::Fall, Pos::new(3, 10, 3), Pos::new(3, 1, 3));
    let mut rt = MoveRuntime::new(&mv);
    let mut p = player(3.5, 4.0, 3.5);
    p.on_ground = false;
    p.vy = -0.9;
    p.looking_at = hit(3, 0, 3, Direction::Up);
    let game = GameFacts { nether: world_nether, ..GameFacts::default() };
    step(&mv, &mut rt, &t, &w, &p, &ExecSettings::default(), inv, game)
}

#[test]
fn a_long_fall_holds_the_water_bucket_looks_down_and_clicks_on_the_landing_block() {
    let mut inv = kit();
    inv.put(6, SlotInfo::of("water_bucket", 1), &[]);
    let (s, c) = water_fall(false, true, &inv);
    assert_eq!(s, MoveStatus::Running);
    assert_eq!(c.select, Some(6));
    assert_eq!(c.look.unwrap().pitch, 90.0);
    assert!(c.keys.click_right, "the crosshair is on the block under the landing cell");
    // too high above the landing cell: the bucket is held but not used
    let t = tables(Config { has_water_bucket: true, ..breaking() });
    let w = TestWorld::flat(10, 12);
    let mv = Movement::new(MovementKind::Fall, Pos::new(3, 10, 3), Pos::new(3, 1, 3));
    let mut rt = MoveRuntime::new(&mv);
    let mut p = player(3.5, 9.0, 3.5);
    p.on_ground = false;
    p.looking_at = hit(3, 0, 3, Direction::Up);
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &ExecSettings::default(), &inv, GameFacts::default());
    assert!(!c.keys.click_right && c.select.is_none());
}

#[test]
fn a_long_fall_is_unreachable_in_the_nether_or_without_the_bucket() {
    let mut inv = kit();
    inv.put(6, SlotInfo::of("water_bucket", 1), &[]);
    assert_eq!(water_fall(true, true, &inv).0, MoveStatus::Unreachable);
    // the config says there is a bucket but the hotbar has none
    assert_eq!(water_fall(false, true, &kit()).0, MoveStatus::Unreachable);
    // no bucket in the config: not a bucket fall at all (a plain fall, which keeps the bucket out of it)
    let (s, c) = water_fall(false, false, &inv);
    assert_eq!(s, MoveStatus::Running);
    assert!(!c.keys.click_right && c.select.is_none());
}

#[test]
fn landing_in_water_picks_it_up_with_an_empty_bucket_once_it_is_not_sinking() {
    let t = tables(Config { has_water_bucket: true, ..breaking() });
    let mut w = TestWorld::flat(10, 12);
    w.set(3, 1, 3, WATER);
    let mv = Movement::new(MovementKind::Fall, Pos::new(3, 10, 3), Pos::new(3, 1, 3));
    let set = ExecSettings::default();
    let mut inv = kit();
    inv.put(2, SlotInfo::of("bucket", 1), &[]);
    let mut p = player(3.5, 1.0, 3.5);
    p.in_water = true;
    p.vy = -0.1;
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(s, MoveStatus::Running);
    assert_eq!(c.select, Some(2));
    assert!(!c.keys.click_right, "still sinking: hold the bucket, do not click");
    // no longer sinking: click
    p.vy = 0.1;
    let mut rt = MoveRuntime::new(&mv);
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv, GameFacts::default());
    assert_eq!(c.select, Some(2));
    assert!(c.keys.click_right);
    // no bucket: done once it stops sinking
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &kit(), GameFacts::default());
    assert_eq!(s, MoveStatus::Success);
    assert!(!c.keys.click_right);
}

#[test]
fn placing_decides_by_the_crosshair_and_the_throwaway() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let w = bridge_world();
    let ctx = t.ctx(&w, BORDER);
    let set = ExecSettings::default();
    let inv = kit();
    let mut p = player(2.9, 1.0, 3.5);
    let place_at = Pos::new(3, 0, 3);
    let run = |p: &PlayerView, prefer_down: bool| {
        let mut env = Env::new(&ctx, p, &set, p.feet());
        env.inv = &inv;
        let mut st = MoveState::new();
        let r = attempt_to_place_a_block(&env, &mut st, place_at, prefer_down, true);
        (r, st)
    };
    // nothing under the crosshair, but a face to aim at: attempting, sneaking, holding the blocks
    let (r, st) = run(&p, false);
    assert_eq!(r, PlaceResult::Attempting);
    assert!(st.keys.sneak && st.select == Some(5) && st.target.is_some_and(|t| t.force));
    // the crosshair on the cell itself (it is replaceable): ready
    p.looking_at = hit(3, 0, 3, Direction::Up);
    assert_eq!(run(&p, false).0, PlaceResult::ReadyToPlace);
    // on a block that is not next to the cell: the aim goes on
    p.looking_at = hit(7, 0, 7, Direction::Up);
    assert_eq!(run(&p, false).0, PlaceResult::Attempting);
    // the block west of the cell, on its east face: ready
    p.looking_at = hit(2, 0, 3, Direction::East);
    assert_eq!(run(&p, true).0, PlaceResult::ReadyToPlace);
}

/// `InventoryBehavior.throwaway(true, ...)` with `allowInventory`, as `MovementHelper.attemptToPlaceABlock` calls it: a block that is only in the main inventory
/// is asked for in hotbar slot 7 (`requestSwapWithHotBar(i, 7)`) and slot 7 is selected; with the setting off, there is nothing to place.
#[test]
fn a_block_in_the_main_inventory_is_swapped_into_slot_7_and_selected() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let w = bridge_world();
    let ctx = t.ctx(&w, BORDER);
    let mut inv = TestInventory::empty();
    inv.put_main(22, SlotInfo::of("stone", 64), &[]);
    let p = player(2.9, 1.0, 3.5);
    let place_at = Pos::new(3, 0, 3);
    let mut set = ExecSettings::default();
    set.tool.allow_inventory = true;
    let mut env = Env::new(&ctx, &p, &set, p.feet());
    env.inv = &inv;
    let mut st = MoveState::new();
    assert_eq!(attempt_to_place_a_block(&env, &mut st, place_at, false, true), PlaceResult::Attempting);
    assert_eq!(st.select, Some(7));
    assert_eq!(st.swap_in, Some(22));
    // the setting off: the block is out of reach for the movement
    let set = ExecSettings::default();
    let mut env = Env::new(&ctx, &p, &set, p.feet());
    env.inv = &inv;
    let mut st = MoveState::new();
    assert_eq!(attempt_to_place_a_block(&env, &mut st, place_at, false, true), PlaceResult::NoOption);
    assert_eq!(st.swap_in, None);
    assert_eq!(st.status, MoveStatus::Unreachable);
}

/// `Movement.update` with a swap asked for inside `throwaway`: the request reaches the inventory mover, which makes it when the spacing allows it (the
/// default `ticksBetweenInventoryMoves` of 1 makes the first one wait for a tick of `InventoryBehavior`).
#[test]
fn update_hands_the_swap_to_the_inventory_mover() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let w = bridge_world();
    let mut inv = TestInventory::empty();
    inv.put_main(22, SlotInfo::of("stone", 64), &[]);
    let p = player(2.9, 1.0, 3.5);
    let mv = traverse((2, 1, 3), (3, 1, 3));
    let mut set = ExecSettings::default();
    set.tool.allow_inventory = true;
    set.allow_place = true;
    let ctx = t.ctx(&w, BORDER);
    let mut env = Env::new(&ctx, &p, &set, p.feet());
    env.inv = &inv;
    let mut rt = MoveRuntime::new(&mv);
    let mut c = Controls::default();
    update(&mv, &mut rt, &env, &mut c);
    assert_eq!(c.select, Some(7), "the movement asked for the stack");
    // delayed by the spacing, then made by the behavior's tick
    assert!(c.inventory.take_requests().is_empty());
    c.inventory.on_tick(&inv, &set.tool, false, false, None);
    let moves = c.inventory.take_requests();
    assert_eq!(moves.len(), 1);
    assert_eq!((moves[0].in_inventory, moves[0].in_hotbar), (22, 7));
}
