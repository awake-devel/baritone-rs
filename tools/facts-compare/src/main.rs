// baritone-rs test tool: compares what Horizon says about every block state with what Java Minecraft says (L1).
// LGPL-3.0-or-later, see ../../../mod/engine/src/lib.rs.
//
//   facts-compare RESOURCES JAVA_DUMP EXPECTED [REPORT]
//
// RESOURCES is a Horizon version's prepared folder (.local/horizon-client/26.3). JAVA_DUMP comes from java/baritone/FactsDump.java.
// EXPECTED lists the differences that are explained. Every other difference fails, and so does an entry that matches nothing.
// A difference counts as harmless only if the engine's predicates still agree, and the tool checks that.

#[path = "../../../mod/glue/state_facts.rs"]
mod state_facts;

use baritone_engine::blocks::{Block, Direction, Fluid, Half, SlabType, StairShape, State};
use baritone_engine::config::Config;
use baritone_engine::movement::helper::*;
use horizon_items::blocks::BlockFacts;
use horizon_world::level::data::load_registry;
use horizon_world::registry::BlockStateId;
use state_facts::{state_from_horizon, tool_strength, Sources, Tool};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// `MobData::load` uses the export only if it has one row per state, and otherwise guesses. The mod has to refuse that.
fn land_from_mob_data(root: &Path, states: usize) -> Vec<bool> {
    let text = std::fs::read(root.join("mob-data.json")).expect("mob-data.json");
    let v: serde_json::Value = serde_json::from_slice(&text).expect("mob-data.json parses");
    let rows = v["states"].as_array().expect("states");
    assert_eq!(rows.len(), states, "mob-data.json does not cover every state: Horizon would fall back to its guesses");
    // `mobs::data::LAND`, the bit of the second column.
    rows.iter().map(|r| r[1].as_u64().expect("flags") & 1 != 0).collect()
}

fn bit(v: bool) -> String {
    (if v { "1" } else { "0" }).to_string()
}

fn half(v: Half) -> &'static str {
    match v {
        Half::Bottom => "BOTTOM",
        Half::Top => "TOP",
    }
}

/// The State as `key=value` text, in the Java dump's encoding. It names every field, so a new one doesn't compile until it's compared.
fn fields(s: &State) -> Vec<(&'static str, String)> {
    let State {
        name: _,
        block: _,
        air,
        amethyst_cluster,
        azalea,
        bamboo_stalk,
        base_fire,
        carpet,
        cauldron,
        door,
        falling,
        gate,
        infested,
        leaves,
        lily,
        liquid_block,
        moving_piston,
        pointed_dripstone,
        scaffolding,
        shulker,
        skull_any,
        skull_floor,
        slab,
        snow,
        stair,
        stained_glass,
        trapdoor,
        end_portal_block,
        wooden_door,
        pathfindable,
        shape_full,
        waterlogged,
        blocks_fluid_flow,
        top_sturdy,
        replaceable,
        falling_free,
        frost_melt,
        fluid,
        fluid_empty,
        fluid_amount,
        fluid_source,
        fluid_flowing_kind,
        level,
        slab_type,
        stair_half,
        stair_shape,
        trap_open,
        trap_half,
        layers,
        door_facing,
        door_hinge_left,
    } = s;
    vec![
        ("air", bit(*air)),
        ("amethyst_cluster", bit(*amethyst_cluster)),
        ("azalea", bit(*azalea)),
        ("bamboo_stalk", bit(*bamboo_stalk)),
        ("base_fire", bit(*base_fire)),
        ("carpet", bit(*carpet)),
        ("cauldron", bit(*cauldron)),
        ("door", bit(*door)),
        ("falling", bit(*falling)),
        ("gate", bit(*gate)),
        ("infested", bit(*infested)),
        ("leaves", bit(*leaves)),
        ("lily", bit(*lily)),
        ("liquid_block", bit(*liquid_block)),
        ("moving_piston", bit(*moving_piston)),
        ("pointed_dripstone", bit(*pointed_dripstone)),
        ("scaffolding", bit(*scaffolding)),
        ("shulker", bit(*shulker)),
        ("skull_any", bit(*skull_any)),
        ("skull_floor", bit(*skull_floor)),
        ("slab", bit(*slab)),
        ("snow", bit(*snow)),
        ("stair", bit(*stair)),
        ("stained_glass", bit(*stained_glass)),
        ("trapdoor", bit(*trapdoor)),
        ("end_portal_block", bit(*end_portal_block)),
        ("wooden_door", bit(*wooden_door)),
        ("pathfindable", bit(*pathfindable)),
        ("shape_full", bit(*shape_full)),
        ("waterlogged", bit(*waterlogged)),
        ("blocks_fluid_flow", bit(*blocks_fluid_flow)),
        ("top_sturdy", bit(*top_sturdy)),
        ("replaceable", bit(*replaceable)),
        ("falling_free", bit(*falling_free)),
        ("frost_melt", bit(*frost_melt)),
        (
            "fluid",
            match fluid {
                Fluid::None => "none",
                Fluid::Water => "water",
                Fluid::FlowingWater => "flowing_water",
                Fluid::Lava => "lava",
                Fluid::FlowingLava => "flowing_lava",
            }
            .to_string(),
        ),
        ("fluid_empty", bit(*fluid_empty)),
        ("fluid_amount", fluid_amount.to_string()),
        ("fluid_source", bit(*fluid_source)),
        ("fluid_flowing_kind", bit(*fluid_flowing_kind)),
        ("level", if *liquid_block { level.to_string() } else { "-1".to_string() }),
        (
            "slab_type",
            if *slab {
                match slab_type {
                    SlabType::Bottom => "BOTTOM",
                    SlabType::Top => "TOP",
                    SlabType::Double => "DOUBLE",
                }
            } else {
                "-"
            }
            .to_string(),
        ),
        ("stair_half", if *stair { half(*stair_half) } else { "-" }.to_string()),
        (
            "stair_shape",
            if *stair {
                match stair_shape {
                    StairShape::Straight => "STRAIGHT",
                    StairShape::InnerLeft => "INNER_LEFT",
                    StairShape::InnerRight => "INNER_RIGHT",
                    StairShape::OuterLeft => "OUTER_LEFT",
                    StairShape::OuterRight => "OUTER_RIGHT",
                }
            } else {
                "-"
            }
            .to_string(),
        ),
        ("trap_open", bit(*trap_open)),
        ("trap_half", if *trapdoor { half(*trap_half) } else { "-" }.to_string()),
        ("layers", if *snow { layers.to_string() } else { "0".to_string() }),
        (
            "door_facing",
            if *door {
                match door_facing {
                    Direction::North => "NORTH",
                    Direction::South => "SOUTH",
                    Direction::East => "EAST",
                    Direction::West => "WEST",
                    Direction::Up => "UP",
                    Direction::Down => "DOWN",
                }
            } else {
                "-"
            }
            .to_string(),
        ),
        ("door_hinge_left", bit(*door_hinge_left)),
    ]
}

/// Puts one Java value into a state, to see what the rules would say with Java's fact.
fn set_field(s: &mut State, key: &str, v: &str) {
    let on = v == "1";
    match key {
        "air" => s.air = on,
        "amethyst_cluster" => s.amethyst_cluster = on,
        "azalea" => s.azalea = on,
        "bamboo_stalk" => s.bamboo_stalk = on,
        "base_fire" => s.base_fire = on,
        "carpet" => s.carpet = on,
        "cauldron" => s.cauldron = on,
        "door" => s.door = on,
        "falling" => s.falling = on,
        "gate" => s.gate = on,
        "infested" => s.infested = on,
        "leaves" => s.leaves = on,
        "lily" => s.lily = on,
        "liquid_block" => s.liquid_block = on,
        "moving_piston" => s.moving_piston = on,
        "pointed_dripstone" => s.pointed_dripstone = on,
        "scaffolding" => s.scaffolding = on,
        "shulker" => s.shulker = on,
        "skull_any" => s.skull_any = on,
        "skull_floor" => s.skull_floor = on,
        "slab" => s.slab = on,
        "snow" => s.snow = on,
        "stair" => s.stair = on,
        "stained_glass" => s.stained_glass = on,
        "trapdoor" => s.trapdoor = on,
        "end_portal_block" => s.end_portal_block = on,
        "wooden_door" => s.wooden_door = on,
        "pathfindable" => s.pathfindable = on,
        "shape_full" => s.shape_full = on,
        "waterlogged" => s.waterlogged = on,
        "blocks_fluid_flow" => s.blocks_fluid_flow = on,
        "top_sturdy" => s.top_sturdy = on,
        "replaceable" => s.replaceable = on,
        "falling_free" => s.falling_free = on,
        "frost_melt" => s.frost_melt = on,
        "fluid_empty" => s.fluid_empty = on,
        "fluid_source" => s.fluid_source = on,
        "fluid_flowing_kind" => s.fluid_flowing_kind = on,
        "fluid_amount" => s.fluid_amount = v.parse().unwrap(),
        "level" => s.level = v.parse().unwrap(),
        "layers" => s.layers = v.parse().unwrap(),
        "trap_open" => s.trap_open = on,
        "door_hinge_left" => s.door_hinge_left = on,
        "fluid" => {
            s.fluid = match v {
                "water" => Fluid::Water,
                "flowing_water" => Fluid::FlowingWater,
                "lava" => Fluid::Lava,
                "flowing_lava" => Fluid::FlowingLava,
                _ => Fluid::None,
            }
        }
        "slab_type" | "stair_half" | "stair_shape" | "trap_half" | "door_facing" | "str_pick" | "str_hand" => {
            // A difference in these has no plain "harmless" answer, so it is never judged by the predicates below.
        }
        other => panic!("unknown field {other}"),
    }
}

/// The settings that decide the predicates, at upstream's defaults, or with the other choices that change an answer.
fn config(flipped: bool) -> Config {
    Config {
        search: Default::default(),
        has_throwaway: true,
        has_water_bucket: false,
        can_sprint: true,
        frost_walker: 0,
        water_mult: 1.0,
        allow_break: true,
        allow_break_anyway: vec![],
        blocks_to_avoid: vec!["tripwire".into()],
        blocks_to_disallow_breaking: vec![],
        assume_walk_on_water: false,
        assume_walk_on_lava: flipped,
        allow_walk_on_magma: flipped,
        allow_vines: flipped,
        allow_walk_on_bottom_slab: !flipped,
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
        allow_downward: true,
        max_fall_no_water: 3,
        max_fall_bucket: 20,
        place_penalty: 20.0,
        break_penalty: 1.0,
        jump_penalty: 2.0,
        walk_on_water_penalty: 3.0,
        cost_heuristic: 3.563,
        axis_height: 120,
    }
}

/// What the rules conclude about a state. Equal answers mean that a differing fact cannot change a cost through this state.
fn answers(s: &State, c: &Config) -> String {
    format!("{:?}{:?}{:?}{}{}{}{}{}{}{}{}{}{}", can_walk_through_state(s, c), can_walk_on_state(s, c), fully_passable_state(s), avoid_walking_into(s, c), is_block_normal_cube(s), is_waterlogged_solid_top(s), is_climbable(s), is_bottom_slab(s), is_water(s), is_lava(s), is_liquid(s), possibly_flowing(s), is_transparent(s),)
}

fn block_of(desc: &str) -> &str {
    let d = desc.split('[').next().unwrap();
    d.strip_prefix("minecraft:").unwrap_or(d)
}

struct Entry {
    field: String,
    block: String,
    used: usize,
}

fn read_expected(path: &str) -> Vec<Entry> {
    let text = std::fs::read_to_string(path).expect("expected differences file");
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let p: Vec<&str> = l.split('\t').collect();
            assert_eq!(p.len(), 3, "field, block, reason, separated by tabs: {l}");
            assert!(p[2].len() > 10, "a reason is needed: {l}");
            Entry { field: p[0].to_string(), block: p[1].to_string(), used: 0 }
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: facts-compare RESOURCES JAVA_DUMP EXPECTED [REPORT]");
        std::process::exit(2);
    }
    let root = Path::new(&args[1]);
    let registry = load_registry(root).expect("registry");
    let (facts, behaviours) = BlockFacts::load_pair(root, &registry).expect("block facts");
    let land = land_from_mob_data(root, registry.len());
    let src = Sources { registry: &registry, facts: &facts, behaviours: &behaviours };
    let mut expected = read_expected(&args[3]);

    let dump = std::fs::read_to_string(&args[2]).expect("java dump");
    let (mut compared, mut same_fields, mut lines) = (0u64, 0u64, 0usize);
    let mut seen = BTreeSet::new();
    // (field, block) -> (states, java, horizon, states where the rules' answers change)
    let mut diffs: BTreeMap<(String, String), (usize, String, String, usize)> = BTreeMap::new();
    let mut identity_problems = vec![];
    let cfgs = [config(false), config(true)];

    for line in dump.lines() {
        let p: Vec<&str> = line.split(' ').collect();
        assert_eq!(p[0], "S", "bad line");
        lines += 1;
        let id: u32 = p[1].parse().unwrap();
        let desc = p[2];
        seen.insert(id);
        let sid = BlockStateId(id);
        let Some(h) = registry.state(sid) else {
            identity_problems.push(format!("{id}: {desc} is not in Horizon's registry"));
            continue;
        };
        let mut props: Vec<String> = desc.split('[').nth(1).unwrap().trim_end_matches(']').split(',').filter(|s| !s.is_empty()).map(String::from).collect();
        props.sort();
        let mut hprops: Vec<String> = h.properties.iter().map(|(k, v)| format!("{k}={v}")).collect();
        hprops.sort();
        if h.name != desc.split('[').next().unwrap() || props != hprops {
            identity_problems.push(format!("{id}: java {desc}, horizon {} {hprops:?}", h.name));
            continue;
        }
        let state = state_from_horizon(&src, sid, land[id as usize]);
        assert_eq!(state.block, Block::from_name(block_of(desc)));
        let mut ours: BTreeMap<&str, String> = fields(&state).into_iter().collect();
        ours.insert("str_pick", format!("{:x}", tool_strength(&src, sid, Tool::DiamondPickaxe).to_bits()));
        ours.insert("str_hand", format!("{:x}", tool_strength(&src, sid, Tool::EmptyHand).to_bits()));
        for kv in &p[3..] {
            let (k, v) = kv.split_once('=').unwrap();
            if k.starts_with("info_") {
                continue;
            }
            let ours_v = ours.remove(k).unwrap_or_else(|| panic!("the State has no field {k}"));
            compared += 1;
            if ours_v == v {
                same_fields += 1;
                continue;
            }
            let mut with_java = state.clone();
            set_field(&mut with_java, k, v);
            let structural = matches!(k, "slab_type" | "stair_half" | "stair_shape" | "trap_half" | "door_facing" | "str_pick" | "str_hand");
            let changes = structural || cfgs.iter().any(|c| answers(&state, c) != answers(&with_java, c));
            let e = diffs.entry((k.to_string(), block_of(desc).to_string())).or_insert((0, v.to_string(), ours_v, 0));
            e.0 += 1;
            if changes {
                e.3 += 1;
            }
        }
        assert!(ours.is_empty(), "fields the Java dump does not have: {:?}", ours.keys());
    }
    for id in 0..registry.len() as u32 {
        if !seen.contains(&id) {
            identity_problems.push(format!("{id}: only in Horizon's registry"));
        }
    }

    let mut unexplained = 0usize;
    let mut explained = 0usize;
    let mut harmful_listed = 0usize;
    for ((field, block), (n, j, h, changes)) in &diffs {
        let entry = expected.iter_mut().find(|e| e.field == *field && (e.block == *block || e.block == "*"));
        let effect = if *changes > 0 { format!("the rules' answers change on {changes}") } else { "the rules' answers don't change".to_string() };
        match entry {
            Some(e) => {
                e.used += 1;
                explained += n;
                if *changes > 0 {
                    harmful_listed += changes;
                }
                println!("explained  {field} {block}: {n} states, java={j} horizon={h}; {effect}");
            }
            None => {
                unexplained += n;
                println!("UNEXPLAINED {field} {block}: {n} states, java={j} horizon={h}; {effect}");
            }
        }
    }
    let unused: Vec<String> = expected.iter().filter(|e| e.used == 0).map(|e| format!("{} {}", e.field, e.block)).collect();
    for p in identity_problems.iter().take(40) {
        println!("IDENTITY {p}");
    }
    for u in &unused {
        println!("UNUSED ENTRY {u}");
    }
    println!("{lines} states, {compared} facts compared, {same_fields} equal, {} different ({explained} explained, {unexplained} not), {} identity problems, {harmful_listed} explained differences change an answer", compared - same_fields, identity_problems.len());

    let ok = unexplained == 0 && identity_problems.is_empty() && unused.is_empty() && lines == registry.len();
    if let Some(report) = args.get(4) {
        let json = serde_json::json!({
            "states": lines, "facts_compared": compared, "facts_equal": same_fields,
            "different": compared - same_fields, "explained": explained, "unexplained": unexplained,
            "identity_problems": identity_problems.len(), "explained_changing_an_answer": harmful_listed,
        });
        std::fs::write(report, serde_json::to_string_pretty(&json).unwrap() + "\n").expect("report");
    }
    std::process::exit(if ok { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_can_be_set_from_its_text() {
        let s = State::default();
        for (k, v) in fields(&s) {
            let mut t = s.clone();
            set_field(&mut t, k, &v);
        }
    }

    #[test]
    fn a_changed_fact_can_change_the_answers() {
        let c = config(false);
        let solid = State { shape_full: true, top_sturdy: true, ..State::default() };
        let mut other = solid.clone();
        set_field(&mut other, "shape_full", "0");
        assert_ne!(answers(&solid, &c), answers(&other, &c));
    }
}
