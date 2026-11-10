// Ported from cabaletta/baritone at 1b8e87c: the block-state facts that src/api/java/baritone/api/schematic/{SubstituteSchematic,
// RotatedSchematic,MirroredSchematic,MaskSchematic}.java and src/main/java/baritone/utils/schematic/MapArtSchematic.java ask of
// `BlockState` and `Block`. LGPL-3.0-or-later, see lib.rs.
//
// Upstream asks the game's block registry. Here the glue answers once, in a [`BlockStates`]: for every state id the block and the
// properties (the same [`StateEntry`] as `blockmeta`), the default state of every block, which states are air, and what rotating or
// mirroring a state gives (the glue asks the game's `BlockState.rotate` and `BlockState.mirror` for each). Everything else is derived.
use super::{BlockMirror, BlockRotation, StateId};
use crate::blockmeta::StateEntry;
use std::collections::HashMap;

/// `minecraft:` is optional in a block name, as in `BlockUtils.stringToBlockNullable`.
fn norm(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// What the schematics need to know about blocks and block states. Built once by the glue and shared as an `Arc`.
#[derive(Clone, Debug)]
pub struct BlockStates {
    entries: Vec<StateEntry>,
    default_state: HashMap<String, StateId>,
    /// The states of each block, in id order.
    by_block: HashMap<String, Vec<StateId>>,
    /// For each block and property name, the values the block's states take, sorted: what `Property.equals` compares besides the name.
    prop_values: HashMap<String, HashMap<String, Vec<String>>>,
    is_air: Vec<bool>,
    rotate: Vec<[StateId; 3]>,
    mirror: Vec<[StateId; 2]>,
    air: StateId,
}

impl BlockStates {
    /// `entries[id]` is the block and the properties of state `id`. `default_state` maps a block (in `BlockUtils` form) to its
    /// `defaultBlockState`. `is_air[id]` is `block instanceof AirBlock`. `rotate[id]` holds `BlockState.rotate` for CLOCKWISE_90,
    /// CLOCKWISE_180 and COUNTERCLOCKWISE_90, `mirror[id]` holds `BlockState.mirror` for LEFT_RIGHT and FRONT_BACK. `air` is
    /// `Blocks.AIR.defaultBlockState()`. A table that is shorter than `entries` means "unchanged" for the states it lacks.
    pub fn new(entries: Vec<StateEntry>, default_state: HashMap<String, StateId>, is_air: Vec<bool>, rotate: Vec<[StateId; 3]>, mirror: Vec<[StateId; 2]>, air: StateId) -> BlockStates {
        let mut by_block: HashMap<String, Vec<StateId>> = HashMap::new();
        let mut prop_values: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
        for (id, e) in entries.iter().enumerate() {
            by_block.entry(e.block.clone()).or_default().push(id);
            let per_block = prop_values.entry(e.block.clone()).or_default();
            for (k, v) in &e.props {
                let values = per_block.entry(k.clone()).or_default();
                if !values.contains(v) {
                    values.push(v.clone());
                }
            }
        }
        for per_block in prop_values.values_mut() {
            for values in per_block.values_mut() {
                values.sort();
            }
        }
        let default_state = default_state.into_iter().map(|(k, v)| (norm(&k).to_string(), v)).collect();
        BlockStates { entries, default_state, by_block, prop_values, is_air, rotate, mirror, air }
    }

    /// The number of states.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when there are no states.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `Blocks.AIR.defaultBlockState`.
    pub fn air(&self) -> StateId {
        self.air
    }

    /// `state.getBlock() instanceof AirBlock`. A state the table does not know is air only if it is the air state.
    pub fn is_air(&self, id: StateId) -> bool {
        self.is_air.get(id).copied().unwrap_or(id == self.air)
    }

    /// Like `BlockUtils.blockToString(state.getBlock())`: the block's name. Empty for a state the table does not know.
    pub fn block(&self, id: StateId) -> &str {
        self.entries.get(id).map(|e| e.block.as_str()).unwrap_or("")
    }

    /// The properties of a state, name and value as text, in the order the glue gave them.
    pub fn props(&self, id: StateId) -> &[(String, String)] {
        self.entries.get(id).map(|e| e.props.as_slice()).unwrap_or(&[])
    }

    /// The value of one property of a state.
    pub fn prop(&self, id: StateId, name: &str) -> Option<&str> {
        self.props(id).iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Like `block.defaultBlockState()`: `None` when there is no such block. `stone` and `minecraft:stone` are the same block.
    pub fn default_state(&self, block: &str) -> Option<StateId> {
        self.default_state.get(norm(block)).copied()
    }

    /// `state.getBlock().getStateDefinition().getProperty(name) != null`.
    pub fn has_property(&self, id: StateId, name: &str) -> bool {
        self.prop(id, name).is_some()
    }

    /// The state of the same block with every property of `id` as it is, but `name` set to `value`; `None` when the state has no such
    /// property or the block's states never take that value. Upstream's `setValue(p, p.getValue(value))` throws for both.
    pub fn with_property(&self, id: StateId, name: &str, value: &str) -> Option<StateId> {
        let e = self.entries.get(id)?;
        if !e.props.iter().any(|(k, _)| k == name) {
            return None;
        }
        if !self.prop_values.get(&e.block)?.get(name)?.iter().any(|v| v == value) {
            return None;
        }
        let want: Vec<(String, String)> = e.props.iter().map(|(k, v)| if k == name { (k.clone(), value.to_string()) } else { (k.clone(), v.clone()) }).collect();
        self.find_state(&e.block, &want)
    }

    /// `Property.equals` between the property `name` of `a`'s block and the one of `b`'s block: the same name and the same set of
    /// possible values (what the block's states take for it). So `axis` of a log (x, y, z) is not `axis` of a nether portal (x, z),
    /// horizontal `facing` is not six-way `facing`, and `half` (top, bottom) is not `half` (upper, lower). `false` when either block
    /// has no such property.
    pub fn same_property(&self, a: StateId, b: StateId, name: &str) -> bool {
        let values = |id: StateId| self.entries.get(id).and_then(|e| self.prop_values.get(&e.block)).and_then(|m| m.get(name));
        match (values(a), values(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }

    /// `RotatedSchematic.rotate(BlockState,Rotation)`: `BlockState.rotate`, from the game's table.
    /// `BlockState.rotate(rotation)`. A state the table has no row for is unchanged.
    pub fn rotate(&self, id: StateId, r: BlockRotation) -> StateId {
        let index = match r {
            BlockRotation::None => return id,
            BlockRotation::Clockwise90 => 0,
            BlockRotation::Clockwise180 => 1,
            BlockRotation::Counterclockwise90 => 2,
        };
        self.rotate.get(id).map(|row| row[index]).unwrap_or(id)
    }

    /// `MirroredSchematic.mirror(BlockState,Mirror)`: `BlockState.mirror`, from the game's table.
    /// `BlockState.mirror(mirror)`. A state the table has no row for is unchanged.
    pub fn mirror(&self, id: StateId, m: BlockMirror) -> StateId {
        let index = match m {
            BlockMirror::None => return id,
            BlockMirror::LeftRight => 0,
            BlockMirror::FrontBack => 1,
        };
        self.mirror.get(id).map(|row| row[index]).unwrap_or(id)
    }

    /// `SubstituteSchematic.copySingleProp`: one property carried over.
    /// `SubstituteSchematic.withBlock`: the default state of `block`, then every property of `from` that `block` has (the same
    /// property, as [`same_property`](Self::same_property)) copied over. Upstream copies one property at a time and skips the ones the
    /// block lacks; the result does not depend on the order. `None` when there is no such block.
    pub fn with_block(&self, from: StateId, block: &str) -> Option<StateId> {
        let target = self.default_state(block)?;
        let mut want: Vec<(String, String)> = self.props(target).to_vec();
        for (k, v) in self.props(from) {
            if self.same_property(from, target, k) {
                if let Some(slot) = want.iter_mut().find(|(wk, _)| wk == k) {
                    slot.1 = v.clone();
                }
            }
        }
        Some(self.find_state(self.block(target), &want).unwrap_or(target))
    }

    /// Like `BlockState.toString`: `Block{minecraft:oak_stairs}[facing=east,half=bottom]`, the properties sorted by name, and no brackets
    /// for a state without properties. A state the table does not know has an empty block name.
    pub fn state_to_string(&self, id: StateId) -> String {
        let block = self.block(id);
        let mut s = if block.contains(':') { format!("Block{{{block}}}") } else { format!("Block{{minecraft:{block}}}") };
        let mut props: Vec<&(String, String)> = self.props(id).iter().collect();
        if !props.is_empty() {
            props.sort_by(|a, b| a.0.cmp(&b.0));
            s.push('[');
            s.push_str(&props.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","));
            s.push(']');
        }
        s
    }

    /// The state of `block` whose properties are exactly `want` (by name, in any order).
    fn find_state(&self, block: &str, want: &[(String, String)]) -> Option<StateId> {
        self.by_block.get(block)?.iter().copied().find(|&id| {
            let props = self.props(id);
            props.len() == want.len() && want.iter().all(|w| props.iter().any(|p| p == w))
        })
    }

    /// A small table for tests, with this layout of ids:
    ///
    /// - 0 air, 1 cave_air, 2 stone, 3 cobblestone, 4 dirt, 5 oak_planks, 6 glass, 7 sand
    /// - 8, 9, 10 oak_log, axis x, y, z
    /// - 11 to 26 water, level 0 to 15; 27 to 42 lava, level 0 to 15
    /// - 43 to 122 oak_stairs: facing (north, south, west, east) outermost, then half (top, bottom), shape (straight, inner_left,
    ///   inner_right, outer_left, outer_right), waterlogged (true, false) innermost
    /// - 123, 124 nether_portal, axis x, z
    /// - 125 to 130 dispenser, facing north, east, south, west, up, down
    /// - 131 to 134 furnace, facing north, east, south, west
    /// - 135, 136 tall_grass, half lower, upper
    ///
    /// Rotation and mirroring follow the game: `facing` turns clockwise (north, east, south, west), `axis` x and z swap on a quarter turn,
    /// FRONT_BACK flips east and west, LEFT_RIGHT flips north and south, and a stair whose facing flips swaps its left and right shapes.
    pub fn test_states() -> BlockStates {
        let mut entries: Vec<StateEntry> = vec![];
        let mut defaults: HashMap<String, StateId> = HashMap::new();
        let mut add = |entries: &mut Vec<StateEntry>, block: &str, props: &[(&str, &str)], is_default: bool| {
            if is_default {
                defaults.insert(block.to_string(), entries.len());
            }
            entries.push(StateEntry { block: block.to_string(), props: props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() });
        };
        for b in ["air", "cave_air", "stone", "cobblestone", "dirt", "oak_planks", "glass", "sand"] {
            add(&mut entries, b, &[], true);
        }
        for a in ["x", "y", "z"] {
            add(&mut entries, "oak_log", &[("axis", a)], a == "y");
        }
        for b in ["water", "lava"] {
            for level in 0..16 {
                add(&mut entries, b, &[("level", &level.to_string())], level == 0);
            }
        }
        for facing in ["north", "south", "west", "east"] {
            for half in ["top", "bottom"] {
                for shape in ["straight", "inner_left", "inner_right", "outer_left", "outer_right"] {
                    for waterlogged in ["true", "false"] {
                        add(&mut entries, "oak_stairs", &[("facing", facing), ("half", half), ("shape", shape), ("waterlogged", waterlogged)], facing == "north" && half == "bottom" && shape == "straight" && waterlogged == "false");
                    }
                }
            }
        }
        for a in ["x", "z"] {
            add(&mut entries, "nether_portal", &[("axis", a)], a == "x");
        }
        for f in ["north", "east", "south", "west", "up", "down"] {
            add(&mut entries, "dispenser", &[("facing", f)], f == "north");
        }
        for f in ["north", "east", "south", "west"] {
            add(&mut entries, "furnace", &[("facing", f)], f == "north");
        }
        for h in ["lower", "upper"] {
            add(&mut entries, "tall_grass", &[("half", h)], h == "lower");
        }

        let index: HashMap<(String, Vec<(String, String)>), StateId> = entries.iter().enumerate().map(|(i, e)| ((e.block.clone(), e.props.clone()), i)).collect();
        let turn = |facing: &str, quarters: usize| -> String {
            let ring = ["north", "east", "south", "west"];
            match ring.iter().position(|r| *r == facing) {
                Some(i) => ring[(i + quarters) % 4].to_string(),
                None => facing.to_string(), // up and down
            }
        };
        let lookup = |block: &str, props: Vec<(String, String)>| *index.get(&(block.to_string(), props)).expect("test table closed under rotate and mirror");
        let rotate: Vec<[StateId; 3]> = entries
            .iter()
            .map(|e| {
                let mut row = [0; 3];
                for (slot, quarters) in [1usize, 2, 3].into_iter().enumerate() {
                    let props = e
                        .props
                        .iter()
                        .map(|(k, v)| match k.as_str() {
                            "facing" => (k.clone(), turn(v, quarters)),
                            "axis" if quarters % 2 == 1 && v != "y" => (k.clone(), if v == "x" { "z".to_string() } else { "x".to_string() }),
                            _ => (k.clone(), v.clone()),
                        })
                        .collect();
                    row[slot] = lookup(&e.block, props);
                }
                row
            })
            .collect();
        let mirror: Vec<[StateId; 2]> = entries
            .iter()
            .map(|e| {
                let mut row = [0; 2];
                for (slot, flipped) in [("north", "south"), ("east", "west")].into_iter().enumerate() {
                    // slot 0 is LEFT_RIGHT (flips north and south), slot 1 is FRONT_BACK (flips east and west)
                    let facing = e.props.iter().find(|(k, _)| k == "facing").map(|(_, v)| v.as_str());
                    let does_flip = facing == Some(flipped.0) || facing == Some(flipped.1);
                    let props = e
                        .props
                        .iter()
                        .map(|(k, v)| match k.as_str() {
                            "facing" if does_flip => (k.clone(), turn(v, 2)),
                            "shape" if does_flip && e.block == "oak_stairs" => {
                                let swapped = match v.as_str() {
                                    "inner_left" => "inner_right",
                                    "inner_right" => "inner_left",
                                    "outer_left" => "outer_right",
                                    "outer_right" => "outer_left",
                                    other => other,
                                };
                                (k.clone(), swapped.to_string())
                            }
                            _ => (k.clone(), v.clone()),
                        })
                        .collect();
                    row[slot] = lookup(&e.block, props);
                }
                row
            })
            .collect();
        let is_air: Vec<bool> = entries.iter().map(|e| e.block == "air" || e.block == "cave_air").collect();
        BlockStates::new(entries, defaults, is_air, rotate, mirror, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &BlockStates, block: &str, props: &[(&str, &str)]) -> StateId {
        (0..s.len()).find(|&i| s.block(i) == block && props.iter().all(|(k, v)| s.prop(i, k) == Some(*v)) && s.props(i).len() == props.len()).unwrap_or_else(|| panic!("no state {block} {props:?}"))
    }

    #[test]
    fn the_test_table_has_the_documented_layout() {
        let s = BlockStates::test_states();
        assert_eq!(s.len(), 137);
        assert_eq!(s.block(0), "air");
        assert_eq!(s.block(7), "sand");
        assert_eq!(s.block(9), "oak_log");
        assert_eq!(s.prop(10, "axis"), Some("z"));
        assert_eq!(s.prop(11, "level"), Some("0"));
        assert_eq!(s.block(27), "lava");
        assert_eq!(s.block(43), "oak_stairs");
        assert_eq!(s.block(122), "oak_stairs");
        assert_eq!(s.block(123), "nether_portal");
        assert_eq!(s.block(125), "dispenser");
        assert_eq!(s.block(131), "furnace");
        assert_eq!(s.block(136), "tall_grass");
        assert!(s.is_air(0) && s.is_air(1) && !s.is_air(2));
        assert_eq!(s.air(), 0);
    }

    #[test]
    fn default_state_accepts_both_name_forms() {
        let s = BlockStates::test_states();
        assert_eq!(s.default_state("stone"), Some(2));
        assert_eq!(s.default_state("minecraft:stone"), Some(2));
        assert_eq!(s.default_state("nothing"), None);
        // the default of a log is axis=y, of stairs facing north, bottom, straight, not waterlogged
        assert_eq!(s.default_state("oak_log"), Some(9));
        let stairs = s.default_state("oak_stairs").unwrap();
        assert_eq!(s.state_to_string(stairs), "Block{minecraft:oak_stairs}[facing=north,half=bottom,shape=straight,waterlogged=false]");
    }

    #[test]
    fn with_property_changes_one_property_and_refuses_what_does_not_exist() {
        let s = BlockStates::test_states();
        let log_y = id(&s, "oak_log", &[("axis", "y")]);
        assert_eq!(s.with_property(log_y, "axis", "x"), Some(8));
        assert_eq!(s.with_property(log_y, "axis", "w"), None);
        assert_eq!(s.with_property(log_y, "facing", "north"), None);
        assert_eq!(s.with_property(2, "axis", "x"), None);
        let st = id(&s, "oak_stairs", &[("facing", "north"), ("half", "bottom"), ("shape", "straight"), ("waterlogged", "false")]);
        let st2 = s.with_property(st, "facing", "east").unwrap();
        assert_eq!(s.props(st2), &[("facing".into(), "east".into()), ("half".into(), "bottom".into()), ("shape".into(), "straight".into()), ("waterlogged".into(), "false".into())]);
        // a value that exists for the name but only on another block is refused
        assert_eq!(s.with_property(log_y, "axis", "q"), None);
        assert_eq!(s.with_property(10_000, "axis", "x"), None);
        // the nether portal takes x and z only
        assert_eq!(s.with_property(123, "axis", "z"), Some(124));
        assert_eq!(s.with_property(123, "axis", "y"), None);
    }

    #[test]
    fn same_property_is_java_property_equality() {
        let s = BlockStates::test_states();
        let log = 9;
        let portal = 123;
        let log2 = 8;
        assert!(s.same_property(log, log2, "axis"));
        // axis {x, y, z} against axis {x, z}
        assert!(!s.same_property(log, portal, "axis"));
        // facing of stairs (4 values) against facing of a dispenser (6 values), and against a furnace (the same 4)
        let stairs = 43;
        assert!(!s.same_property(stairs, 125, "facing"));
        assert!(s.same_property(stairs, 131, "facing"));
        // half {top, bottom} against half {lower, upper}
        assert!(!s.same_property(stairs, 135, "half"));
        // a property one side lacks
        assert!(!s.same_property(log, stairs, "axis"));
        assert!(!s.same_property(2, 3, "axis"));
        // level is the same for water and lava
        assert!(s.same_property(11, 27, "level"));
    }

    #[test]
    fn with_block_copies_the_properties_the_target_has() {
        let s = BlockStates::test_states();
        // stairs to planks: planks have no properties, so it is the default planks
        let stairs = id(&s, "oak_stairs", &[("facing", "east"), ("half", "top"), ("shape", "inner_left"), ("waterlogged", "true")]);
        assert_eq!(s.with_block(stairs, "oak_planks"), Some(5));
        assert_eq!(s.with_block(stairs, "minecraft:oak_planks"), Some(5));
        // stairs to stairs: all four properties kept
        assert_eq!(s.with_block(stairs, "oak_stairs"), Some(stairs));
        // planks to stairs: the default stairs
        let default_stairs = s.default_state("oak_stairs").unwrap();
        assert_eq!(s.with_block(5, "oak_stairs"), Some(default_stairs));
        // stairs to a furnace: facing is the same property, so it is copied; half is not a furnace property
        let furnace_east = id(&s, "furnace", &[("facing", "east")]);
        assert_eq!(s.with_block(stairs, "furnace"), Some(furnace_east));
        // stairs to a dispenser: facing differs (four values against six), so the default stays
        assert_eq!(s.with_block(stairs, "dispenser"), Some(125));
        // log (axis x, y, z) to a nether portal (axis x, z): not the same property
        let log_z = 10;
        assert_eq!(s.with_block(log_z, "nether_portal"), Some(123));
        // lava to water keeps the level
        assert_eq!(s.with_block(27 + 5, "water"), Some(11 + 5));
        assert_eq!(s.with_block(stairs, "nothing"), None);
    }

    #[test]
    fn state_to_string_sorts_the_properties() {
        let s = BlockStates::test_states();
        assert_eq!(s.state_to_string(0), "Block{minecraft:air}");
        let st = id(&s, "oak_stairs", &[("waterlogged", "false"), ("facing", "east"), ("half", "bottom"), ("shape", "straight")]);
        assert_eq!(s.state_to_string(st), "Block{minecraft:oak_stairs}[facing=east,half=bottom,shape=straight,waterlogged=false]");
        assert_eq!(s.state_to_string(8), "Block{minecraft:oak_log}[axis=x]");
        assert_eq!(s.state_to_string(99_999), "Block{minecraft:}");
    }

    #[test]
    fn rotate_and_mirror_follow_the_tables() {
        let s = BlockStates::test_states();
        let east = id(&s, "furnace", &[("facing", "east")]);
        let south = id(&s, "furnace", &[("facing", "south")]);
        let north = id(&s, "furnace", &[("facing", "north")]);
        let west = id(&s, "furnace", &[("facing", "west")]);
        assert_eq!(s.rotate(north, BlockRotation::Clockwise90), east);
        assert_eq!(s.rotate(north, BlockRotation::Clockwise180), south);
        assert_eq!(s.rotate(north, BlockRotation::Counterclockwise90), west);
        assert_eq!(s.rotate(north, BlockRotation::None), north);
        assert_eq!(s.mirror(east, BlockMirror::FrontBack), west);
        assert_eq!(s.mirror(east, BlockMirror::LeftRight), east);
        assert_eq!(s.mirror(north, BlockMirror::LeftRight), south);
        assert_eq!(s.mirror(north, BlockMirror::None), north);
        // a short table means unchanged
        let short = BlockStates::new(vec![StateEntry::default()], HashMap::new(), vec![], vec![], vec![], 0);
        assert_eq!(short.rotate(0, BlockRotation::Clockwise90), 0);
        assert_eq!(short.mirror(5, BlockMirror::FrontBack), 5);
        assert!(short.is_air(0) && !short.is_air(1));
    }
}
