// baritone-rs engine: tests from the independent review of the break and place code against upstream. Written for this project.
// LGPL-3.0-or-later, see lib.rs.
//
// Each test names the finding it pins. R1 and R2 were port mistakes (fixed); R3 pins what upstream does, R4 the guard added to what upstream does (docs/deviations.md U14, U11 and D28).
use super::helper::{attempt_to_place_a_block, PlaceResult};
use super::movements::{update, Env, GameFacts, MoveRuntime};
use super::state::{Controls, MoveState, MoveStatus, PlayerView};
use crate::blocks::{Block, Direction, Fluid, State};
use crate::config::Config;
use crate::inventory::testing::TestInventory;
use crate::inventory::SlotInfo;
use crate::look::Vec3;
use crate::movement::descend::dynamic_fall_cost;
use crate::movement::helper::{Border, MoveResult};
use crate::movement::step::{Movement, MovementKind};
use crate::pos::Pos;
use crate::process::pathing::Tables;
use crate::raytrace::BlockHit;
use crate::settings::ExecSettings;
use crate::testing::{test_config, Fixture, TestWorld, AIR, STONE};

const BORDER: Border = Border { min_x: -3.0e7, max_x: 3.0e7, min_z: -3.0e7, max_z: 3.0e7 };
const LADDER: u16 = 3;
const TORCH: u16 = 4;

fn tables(cfg: Config) -> Tables {
    let mut states = Fixture::new().states;
    states.push(State { name: "water".into(), block: Block::Water, fluid: Fluid::Water, fluid_empty: false, fluid_amount: 8, fluid_source: true, liquid_block: true, ..State::default() });
    states.push(State { name: "ladder".into(), block: Block::Ladder, fluid_empty: true, pathfindable: true, ..State::default() });
    // a block that is not air, not replaceable, not walkable on: a torch or a sign
    states.push(State { name: "torch".into(), fluid_empty: true, pathfindable: true, ..State::default() });
    Tables::new(states, cfg, vec![1.0, 0.2, 1.0, 1.0, 0.5])
}

fn breaking() -> Config {
    Config { allow_break: true, allow_downward: true, ..test_config() }
}

fn kit() -> TestInventory {
    let mut inv = TestInventory::empty();
    inv.put(3, SlotInfo { pickaxe: true, material_cost: 2, ..SlotInfo::of("iron_pickaxe", 1) }, &[0.0, 0.2, 0.0, 0.0, 0.9]);
    inv.put(5, SlotInfo::of("stone", 64), &[0.0, 0.0125]);
    inv
}

fn hit(x: i32, y: i32, z: i32, face: Direction) -> Option<BlockHit> {
    Some(BlockHit { pos: Pos::new(x, y, z), face, point: Vec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5) })
}

fn step(mv: &Movement, rt: &mut MoveRuntime, t: &Tables, w: &TestWorld, p: &PlayerView, set: &ExecSettings, inv: &TestInventory) -> (MoveStatus, Controls) {
    let ctx = t.ctx(w, BORDER);
    let mut env = Env::new(&ctx, p, set, p.feet());
    env.inv = inv;
    env.game = GameFacts::default();
    let mut c = Controls::default();
    let s = update(mv, rt, &env, &mut c);
    (s, c)
}

/// R1 (fixed): a fall down a ladder shaft that the search prices as a plain fall (no bucket) is no bucket fall in the executor (`willPlaceBucket` runs `dynamicFallCost`).
#[test]
fn r1_a_fall_past_a_ladder_is_not_a_bucket_fall_in_the_executor_either() {
    let t = tables(Config { has_water_bucket: true, ..breaking() });
    let mut w = TestWorld::flat(10, 16);
    for y in 3..=7 {
        w.set(3, y, 3, LADDER);
    }
    // search side: upstream's dynamicFallCost from the top of the shaft
    let ctx = t.ctx(&w, BORDER);
    let mut res = MoveResult::default();
    let below = ctx.get(3, 10, 3);
    let bucket = dynamic_fall_cost(&ctx, 12, 3, 3, 0.0, below, &mut res);
    assert_eq!((res.x, res.y, res.z), (3, 1, 3), "the search lands on the floor at y = 1");
    assert!(!bucket, "the search says: no bucket needed (the ladder resets the fall)");
    // executor side
    let mv = Movement::new(MovementKind::Fall, Pos::new(3, 12, 3), Pos::new(3, 1, 3));
    let mut inv = kit();
    inv.put(6, SlotInfo::of("water_bucket", 1), &[]);
    let mut p = PlayerView::standing(3.5, 4.0, 3.5);
    p.on_ground = false;
    p.vy = -0.2;
    p.looking_at = hit(3, 0, 3, Direction::Up);
    let mut rt = MoveRuntime::new(&mv);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &ExecSettings::default(), &inv);
    println!("status {s:?} select {:?} click_right {} look {:?}", c.select, c.keys.click_right, c.look);
    // upstream: willPlaceBucket() is false here, so no bucket is selected and nothing is placed
    assert!(c.select.is_none() && !c.keys.click_right, "the executor must not take the bucket where the search planned none");
}

/// R2 (fixed): the executor's placement test checks the world border, as upstream's `BlockStateInterface` does.
#[test]
fn r2_place_against_a_block_in_the_border_margin() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let mut w = TestWorld::flat(10, 10);
    for x in 0..10 {
        for z in 0..10 {
            w.set(x, 0, z, AIR);
        }
    }
    w.set(3, 0, 3, STONE); // the only block to place against: east of the cell (2,0,3)
    let border = Border { min_x: -3.0e7, max_x: 4.0, min_z: -3.0e7, max_z: 3.0e7 };
    let ctx = t.ctx(&w, border);
    println!("can_place_against(3,0,3) = {}, executor variant = {}", ctx.can_place_against(3, 0, 3), ctx.can_place_against_bsi(3, 0, 3));
    let set = ExecSettings::default();
    let inv = kit();
    let p = PlayerView::standing(1.5, 1.0, 3.5);
    let mut env = Env::new(&ctx, &p, &set, p.feet());
    env.inv = &inv;
    let mut st = MoveState::new();
    let r = attempt_to_place_a_block(&env, &mut st, Pos::new(2, 0, 3), false, true);
    println!("result {r:?}");
    // upstream MovementHelper.canPlaceAgainst(IPlayerContext) -> BlockStateInterface, which checks worldBorder.canPlaceAt: false for x = 3, border 4
    assert_eq!(r, PlaceResult::NoOption);
}

/// R3 (upstream): the pillar breaks the block at its feet with the throwaway block in the hand, not with the best tool.
#[test]
fn r3_pillar_breaks_with_the_throwaway_in_hand() {
    let t = tables(Config { has_throwaway: true, ..breaking() });
    let mut w = TestWorld::flat(10, 10);
    w.set(3, 1, 3, TORCH);
    let mv = Movement::new(MovementKind::Pillar, Pos::new(3, 1, 3), Pos::new(3, 2, 3));
    let p = PlayerView::standing(3.5, 1.0, 3.5);
    let mut rt = MoveRuntime::new(&mv);
    rt.state.status = MoveStatus::Running;
    let (_, c) = step(&mv, &mut rt, &t, &w, &p, &ExecSettings::default(), &kit());
    println!("select {:?} click_left {}", c.select, c.keys.click_left);
    // upstream selects the throwaway block for the pillar and clicks the block at the feet without `switchToBestToolFor` (docs/deviations.md U14)
    assert_eq!(c.select, Some(5), "upstream digs the block at the pillar's feet with the throwaway block in hand");
    assert!(c.keys.click_left);
}

/// R4 (U11, guarded by D28): the incidental left clicks do not look at what is under the crosshair, so blocksToDisallowBreaking does not stop them.
#[test]
fn r4_bridge_click_left_skips_disallowed_blocks() {
    let cfg = Config { has_throwaway: true, blocks_to_disallow_breaking: vec!["stone".into()], ..breaking() };
    let t = tables(cfg);
    let mut w = TestWorld::flat(10, 10);
    w.set(3, 0, 3, AIR);
    let mv = Movement::new(MovementKind::Traverse, Pos::new(2, 1, 3), Pos::new(3, 1, 3));
    let inv = kit();
    let set = ExecSettings::default();
    let mut p = PlayerView::standing(2.9, 1.0, 3.5);
    let mut rt = MoveRuntime::new(&mv);
    step(&mv, &mut rt, &t, &w, &p, &set, &inv);
    let want = rt.state.target.expect("an aim").rotation;
    p.yaw = want.yaw;
    p.pitch = want.pitch + 0.005;
    // the crosshair is on a stone block that is not the face to place against
    p.looking_at = hit(2, 0, 2, Direction::West);
    let (s, c) = step(&mv, &mut rt, &t, &w, &p, &set, &inv);
    println!("status {s:?} click_left {} click_right {}", c.keys.click_left, c.keys.click_right);
    // upstream presses the button here whatever the crosshair is on (U11); here a block in blocksToDisallowBreaking is not clicked (D28)
    assert!(!c.keys.click_left, "a disallowed block (stone) would be broken");
    // the same crosshair on a block that may be broken is clicked, as upstream
    let t2 = tables(Config { has_throwaway: true, ..breaking() });
    let mut p2 = p;
    p2.looking_at = hit(2, 0, 2, Direction::West);
    let mut rt2 = MoveRuntime::new(&mv);
    step(&mv, &mut rt2, &t2, &w, &p2, &set, &inv);
    let (_, c2) = step(&mv, &mut rt2, &t2, &w, &p2, &set, &inv);
    assert!(c2.keys.click_left, "an allowed block is clicked");
}
