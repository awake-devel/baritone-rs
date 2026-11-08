// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/utils/BlockOptionalMeta.java and BlockOptionalMetaLookup.java, and
// the name lookup of BlockUtils.java. LGPL-3.0-or-later, see lib.rs.
//
// Upstream reads the block registry, the states of a block and the loot tables from the game. Here the glue answers once, in a
// [`BlockIndex`]: for every state id the block it belongs to and its properties, and for every block the items it drops. The engine
// then matches by state id and by item path, and never touches the game. What upstream compares by hash (`BlockState.hashCode`,
// `ItemStack.getBaritoneHash` minus the damage) is compared here by identity: a state id, an item path.
use std::collections::{HashMap, HashSet};

/// One block state, as the matcher needs it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateEntry {
    /// The block's registry name as `BlockUtils.blockToString` writes it: the path for `minecraft:`, the whole name otherwise.
    pub block: String,
    /// The state's properties, name and value as text (`Property.getName`, `Property.getName(value)`).
    pub props: Vec<(String, String)>,
}

/// What the glue tells the engine about the blocks: built once per game session, indexed by state id.
#[derive(Clone, Debug, Default)]
pub struct BlockIndex {
    states: Vec<StateEntry>,
    by_block: HashMap<String, Vec<usize>>,
    drops: HashMap<String, Vec<String>>,
}

impl BlockIndex {
    /// `states[id]` is the state with that id. `drops` maps a block to the item paths its loot table gives (`minecraft:` left off,
    /// as for blocks); a block with no entry drops nothing, as `BlockOptionalMeta.drops` does for a block without a loot table.
    pub fn new(states: Vec<StateEntry>, drops: HashMap<String, Vec<String>>) -> BlockIndex {
        let mut by_block: HashMap<String, Vec<usize>> = HashMap::new();
        for (id, s) in states.iter().enumerate() {
            by_block.entry(s.block.clone()).or_default().push(id);
        }
        BlockIndex { states, by_block, drops }
    }

    /// The block's states, in id order. `None` when there is no such block.
    pub fn states_of(&self, block: &str) -> Option<&[usize]> {
        self.by_block.get(block).map(|v| v.as_slice())
    }

    /// The block of a state id.
    pub fn block_of(&self, id: usize) -> Option<&str> {
        self.states.get(id).map(|s| s.block.as_str())
    }

    /// Every block's name, sorted: for tab completion.
    pub fn block_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.by_block.keys().map(|s| s.as_str()).collect();
        v.sort_unstable();
        v
    }

    /// Upstream's `BlockUtils.stringToBlockNullable`: the block with this name (`minecraft:` is optional), as the name it is kept under.
    pub fn block_name(&self, name: &str) -> Option<String> {
        let path = name.strip_prefix("minecraft:").unwrap_or(name);
        // another namespace is kept whole; a bare name means `minecraft:`
        self.by_block.contains_key(path).then(|| path.to_string())
    }

    fn drops_of(&self, block: &str) -> &[String] {
        self.drops.get(block).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// How upstream writes a block in a message: `Block.toString`, `Block{minecraft:stone}`.
fn block_to_string(block: &str) -> String {
    if block.contains(':') {
        format!("Block{{{block}}}")
    } else {
        format!("Block{{minecraft:{block}}}")
    }
}

/// `BlockOptionalMeta`: a block, and optionally the values of some of its properties. Matches the block's states that have them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockOptionalMeta {
    block: String,
    properties_description: String,
    /// The matching state ids, in id order.
    states: Vec<usize>,
    state_set: HashSet<usize>,
    /// The paths of the items the block drops (`stackHashes`).
    drops: Vec<String>,
}

/// The pattern `^(?<id>.+?)(?:\[(?<properties>.+?)?\])?$` of upstream: the id is the shortest start (one character at least) after which
/// there is nothing, or square brackets that close the text. The properties are what lies between them; none when it is empty.
fn split_selector(selector: &str) -> Option<(&str, Option<&str>)> {
    if selector.is_empty() {
        return None; // the id is one character at least
    }
    let ends: Vec<usize> = selector.char_indices().map(|(i, _)| i).skip(1).chain(std::iter::once(selector.len())).collect();
    for i in ends {
        let rest = &selector[i..];
        if rest.is_empty() {
            return Some((selector, None));
        }
        if rest.len() >= 2 && rest.starts_with('[') && rest.ends_with(']') {
            let props = &rest[1..rest.len() - 1];
            return Some((&selector[..i], if props.is_empty() { None } else { Some(props) }));
        }
    }
    None
}

impl BlockOptionalMeta {
    /// `BlockOptionalMeta.<init>(Block)`: every state of the block. `Err` when the block does not exist.
    pub fn from_block(block: &str, index: &BlockIndex) -> Result<BlockOptionalMeta, String> {
        let name = index.block_name(block).ok_or_else(|| format!("Invalid block name {block}"))?;
        Ok(BlockOptionalMeta::build(name, "{}".to_string(), &[], index))
    }

    /// `BlockOptionalMeta.<init>(String)`: `id`, `id[]` or `id[property=value,...]`. `Err` carries upstream's message where it has one
    /// (an invalid selector or block name, a pair that is not `name=value`); an unknown property or value is a different text, because upstream throws a
    /// `NullPointerException` for a property the block lacks.
    pub fn parse(selector: &str, index: &BlockIndex) -> Result<BlockOptionalMeta, String> {
        let (id, props) = split_selector(selector).ok_or_else(|| "invalid block selector".to_string())?;
        let name = index.block_name(id).ok_or_else(|| format!("Invalid block name {id}"))?;
        let mut wanted: Vec<(String, String)> = vec![];
        if let Some(raw) = props {
            for pair in raw.split(',') {
                // Java's `split("=")` drops trailing empty strings: `a=` is one part, `a==b` has an empty middle one
                let mut parts: Vec<&str> = pair.split('=').collect();
                while parts.last().is_some_and(|p| p.is_empty()) {
                    parts.pop();
                }
                if parts.len() != 2 {
                    return Err(format!("\"{pair}\" is not a valid property-value pair"));
                }
                let (key, value) = (parts[0], parts[1]);
                let states = index.states_of(&name).unwrap_or(&[]);
                let has_key = states.iter().any(|&id| index.states[id].props.iter().any(|(k, _)| k == key));
                if !has_key {
                    return Err(format!("\"{key}\" is not a property of {}", block_to_string(&name)));
                }
                let has_value = states.iter().any(|&id| index.states[id].props.iter().any(|(k, v)| k == key && v == value));
                if !has_value {
                    return Err(format!("\"{value}\" is not a valid value for {key} on {}", block_to_string(&name)));
                }
                wanted.push((key.to_string(), value.to_string()));
            }
        }
        let description = match props {
            None => "{}".to_string(),
            Some(p) => format!("{{{}}}", p.replace('=', ":")),
        };
        Ok(BlockOptionalMeta::build(name, description, &wanted, index))
    }

    fn build(block: String, properties_description: String, wanted: &[(String, String)], index: &BlockIndex) -> BlockOptionalMeta {
        // `getStates`: the block's states for which every wanted property has the wanted value
        let states: Vec<usize> = index.states_of(&block).unwrap_or(&[]).iter().copied().filter(|&id| wanted.iter().all(|(k, v)| index.states[id].props.iter().any(|(pk, pv)| pk == k && pv == v))).collect();
        let state_set = states.iter().copied().collect();
        // `getStackHashes`: the drops of the block of each matching state; all the states share their block
        let drops = if states.is_empty() { vec![] } else { index.drops_of(&block).to_vec() };
        BlockOptionalMeta { block, properties_description, states, state_set, drops }
    }

    /// `BlockOptionalMeta.getBlock`: the block's name (`minecraft:` left off).
    pub fn block(&self) -> &str {
        &self.block
    }

    /// `BlockOptionalMeta.matches(Block)`: the same block.
    pub fn matches_block(&self, block: &str) -> bool {
        block == self.block
    }

    /// `BlockOptionalMeta.matches(BlockState)`: the state is one of this block's states with the wanted properties.
    pub fn matches_state(&self, state: usize) -> bool {
        self.state_set.contains(&state)
    }

    /// `BlockOptionalMeta.matches(ItemStack)`: the item is one the block drops. The damage does not count, as upstream's hash minus the damage.
    pub fn matches_item(&self, item: &str) -> bool {
        self.drops.iter().any(|d| d == item)
    }

    /// `BlockOptionalMeta.getAnyBlockState`: a state that matches, or none. Upstream takes the first of a hash set, which is any; the lowest id here.
    pub fn any_block_state(&self) -> Option<usize> {
        self.states.first().copied()
    }

    /// `BlockOptionalMeta.getAllBlockStates`: in id order.
    pub fn all_block_states(&self) -> &[usize] {
        &self.states
    }

    /// `BlockOptionalMeta.stackHashes`: the paths of the items the block drops.
    pub fn stack_items(&self) -> &[String] {
        &self.drops
    }
}

impl std::fmt::Display for BlockOptionalMeta {
    /// `BlockOptionalMeta.toString`: the text of a selector.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BlockOptionalMeta{{block={},properties={}}}", block_to_string(&self.block), self.properties_description)
    }
}

/// `BlockOptionalMetaLookup`: several selectors, asked together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockOptionalMetaLookup {
    block_set: HashSet<String>,
    state_set: HashSet<usize>,
    item_set: HashSet<String>,
    boms: Vec<BlockOptionalMeta>,
}

impl BlockOptionalMetaLookup {
    /// `BlockOptionalMetaLookup.<init>(BlockOptionalMeta...)`.
    pub fn new(boms: Vec<BlockOptionalMeta>) -> BlockOptionalMetaLookup {
        let mut block_set = HashSet::new();
        let mut state_set = HashSet::new();
        let mut item_set = HashSet::new();
        for bom in &boms {
            block_set.insert(bom.block.clone());
            state_set.extend(bom.states.iter().copied());
            item_set.extend(bom.drops.iter().cloned());
        }
        BlockOptionalMetaLookup { block_set, state_set, item_set, boms }
    }

    /// `BlockOptionalMetaLookup.<init>(String...)`: `Err` is the first selector that does not parse.
    pub fn from_names(names: &[&str], index: &BlockIndex) -> Result<BlockOptionalMetaLookup, String> {
        let boms: Result<Vec<_>, _> = names.iter().map(|n| BlockOptionalMeta::parse(n, index)).collect();
        Ok(BlockOptionalMetaLookup::new(boms?))
    }

    /// `BlockOptionalMetaLookup.has(Block)`: one of the selectors is for this block.
    pub fn has_block(&self, block: &str) -> bool {
        self.block_set.contains(block)
    }

    /// `BlockOptionalMetaLookup.has(BlockState)`: one of the selectors matches this state.
    pub fn has(&self, state: usize) -> bool {
        self.state_set.contains(&state)
    }

    /// `BlockOptionalMetaLookup.has(ItemStack)`: by the item's path.
    pub fn has_item(&self, item: &str) -> bool {
        self.item_set.contains(item)
    }

    /// `BlockOptionalMetaLookup.blocks`: the selectors.
    pub fn blocks(&self) -> &[BlockOptionalMeta] {
        &self.boms
    }
}

impl std::fmt::Display for BlockOptionalMetaLookup {
    /// `BlockOptionalMetaLookup.toString`: the list as `Arrays.toString` writes it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let items: Vec<String> = self.boms.iter().map(|b| b.to_string()).collect();
        write!(f, "BlockOptionalMetaLookup{{[{}]}}", items.join(", "))
    }
}

/// A small block index for tests: a few blocks with properties and drops.
#[cfg(test)]
pub fn test_index() -> BlockIndex {
    let e = |block: &str, props: &[(&str, &str)]| StateEntry { block: block.to_string(), props: props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
    let states = vec![
        e("air", &[]),
        e("stone", &[]),
        e("diamond_ore", &[]),
        e("deepslate_diamond_ore", &[]),
        e("oak_log", &[("axis", "x")]),
        e("oak_log", &[("axis", "y")]),
        e("oak_log", &[("axis", "z")]),
        e("wheat", &[("age", "0")]),
        e("wheat", &[("age", "1")]),
        e("chest", &[("facing", "north"), ("waterlogged", "false")]),
        e("chest", &[("facing", "north"), ("waterlogged", "true")]),
        e("create:brass_block", &[]),
        e("nether_portal", &[("axis", "x")]),
        e("furnace", &[("facing", "north"), ("lit", "false")]),
    ];
    let mut drops = HashMap::new();
    drops.insert("diamond_ore".to_string(), vec!["diamond".to_string()]);
    drops.insert("deepslate_diamond_ore".to_string(), vec!["diamond".to_string()]);
    drops.insert("stone".to_string(), vec!["cobblestone".to_string()]);
    drops.insert("oak_log".to_string(), vec!["oak_log".to_string()]);
    BlockIndex::new(states, drops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_matches_every_state_of_the_block() {
        let idx = test_index();
        let b = BlockOptionalMeta::parse("oak_log", &idx).unwrap();
        assert_eq!(b.all_block_states(), &[4, 5, 6]);
        assert!(b.matches_state(5) && !b.matches_state(1));
        assert!(b.matches_block("oak_log") && !b.matches_block("stone"));
        // the namespace is optional
        assert_eq!(BlockOptionalMeta::parse("minecraft:oak_log", &idx).unwrap().all_block_states(), &[4, 5, 6]);
        assert_eq!(BlockOptionalMeta::parse("oak_log[]", &idx).unwrap().all_block_states(), &[4, 5, 6]);
        assert_eq!(b.to_string(), "BlockOptionalMeta{block=Block{minecraft:oak_log},properties={}}");
    }

    #[test]
    fn properties_narrow_the_states() {
        let idx = test_index();
        let b = BlockOptionalMeta::parse("oak_log[axis=y]", &idx).unwrap();
        assert_eq!(b.all_block_states(), &[5]);
        assert_eq!(b.any_block_state(), Some(5));
        assert_eq!(b.to_string(), "BlockOptionalMeta{block=Block{minecraft:oak_log},properties={axis:y}}");
        let c = BlockOptionalMeta::parse("chest[facing=north,waterlogged=true]", &idx).unwrap();
        assert_eq!(c.all_block_states(), &[10]);
        assert_eq!(c.to_string(), "BlockOptionalMeta{block=Block{minecraft:chest},properties={facing:north,waterlogged:true}}");
        assert_eq!(BlockOptionalMeta::parse("chest[waterlogged=false]", &idx).unwrap().all_block_states(), &[9]);
    }

    #[test]
    fn a_bad_selector_is_refused() {
        let idx = test_index();
        assert_eq!(BlockOptionalMeta::parse("nothing", &idx).unwrap_err(), "Invalid block name nothing");
        assert_eq!(BlockOptionalMeta::parse("", &idx).unwrap_err(), "invalid block selector");
        assert_eq!(BlockOptionalMeta::parse("oak_log[axis]", &idx).unwrap_err(), "\"axis\" is not a valid property-value pair");
        assert_eq!(BlockOptionalMeta::parse("oak_log[axis=]", &idx).unwrap_err(), "\"axis=\" is not a valid property-value pair");
        assert_eq!(BlockOptionalMeta::parse("oak_log[a=b=c]", &idx).unwrap_err(), "\"a=b=c\" is not a valid property-value pair");
        assert!(BlockOptionalMeta::parse("oak_log[axis=w]", &idx).unwrap_err().contains("not a valid value for axis"));
        assert!(BlockOptionalMeta::parse("oak_log[age=1]", &idx).unwrap_err().contains("not a property of"));
        // another namespace is kept whole
        assert!(BlockOptionalMeta::parse("create:brass_block", &idx).is_ok());
        assert!(BlockOptionalMeta::parse("brass_block", &idx).is_err());
    }

    #[test]
    fn the_selector_pattern_splits_as_the_regex_does() {
        assert_eq!(split_selector("a"), Some(("a", None)));
        assert_eq!(split_selector("a[b=c]"), Some(("a", Some("b=c"))));
        assert_eq!(split_selector("a[]"), Some(("a", None)));
        assert_eq!(split_selector("[b]"), Some(("[b]", None)));
        // lazy id: the first bracket that closes the text starts the properties
        assert_eq!(split_selector("a[b][c]"), Some(("a", Some("b][c"))));
        assert_eq!(split_selector(""), None);
    }

    #[test]
    fn items_are_matched_by_what_the_block_drops() {
        let idx = test_index();
        let b = BlockOptionalMeta::parse("diamond_ore", &idx).unwrap();
        assert!(b.matches_item("diamond") && !b.matches_item("stone"));
        let l = BlockOptionalMetaLookup::from_names(&["diamond_ore", "stone"], &idx).unwrap();
        assert!(l.has_item("diamond") && l.has_item("cobblestone") && !l.has_item("dirt"));
        assert!(l.has(1) && l.has(2) && !l.has(3) && !l.has(0));
        assert!(l.has_block("stone") && !l.has_block("air"));
        assert_eq!(l.blocks().len(), 2);
        assert_eq!(l.to_string(), "BlockOptionalMetaLookup{[BlockOptionalMeta{block=Block{minecraft:diamond_ore},properties={}}, BlockOptionalMeta{block=Block{minecraft:stone},properties={}}]}");
        // a block without a loot table drops nothing
        assert!(BlockOptionalMeta::parse("wheat", &idx).unwrap().stack_items().is_empty());
        assert!(BlockOptionalMetaLookup::from_names(&["stone", "nothing"], &idx).is_err());
    }
}
