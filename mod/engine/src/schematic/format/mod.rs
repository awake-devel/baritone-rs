// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/schematic/format/DefaultSchematicFormats.java, src/main/java/baritone/utils/schematic/SchematicSystem.java
// and src/api/java/baritone/api/schematic/format/ISchematicFormat.java. LGPL-3.0-or-later, see lib.rs.
//
// The schematic file formats. Parsing takes the bytes of a file, never a path: reading the file is the glue's job. Upstream's `ISchematicFormat`
// interface and the `Registry` of formats become the closed enum [`SchematicFormat`] (D47). The block names a file holds are looked up in the
// shared [`BlockStates`] table, as upstream looks them up in the block registry: a name the table does not know is air.
use super::{BlockStates, StateId, StaticSchematicView};
use std::sync::Arc;

pub mod legacy_ids;
pub mod litematica;
pub mod mcedit;
pub mod nbt;
pub mod sponge;

/// Why a file could not be read: the message of upstream's exception where it has one, or a short plain text for the limits of this port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// What this port refuses to read (D47): upstream reads anything and runs out of memory on a hostile file.
#[derive(Clone, Copy, Debug)]
pub struct ParseLimits {
    /// The most bytes the gzip layer may inflate to, and the memory the NBT tags may be estimated to use.
    pub max_inflated_bytes: usize,
    /// The most blocks (width times height times length) of one schematic: summed over the regions of a Litematica file, and also the box that holds them.
    pub max_cells: u64,
}

impl Default for ParseLimits {
    fn default() -> ParseLimits {
        ParseLimits { max_inflated_bytes: 256 << 20, max_cells: 33_554_432 }
    }
}

/// `DefaultSchematicFormats`, in the order the registry holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchematicFormat {
    McEdit,
    Sponge,
    Litematica,
}

impl SchematicFormat {
    pub const ALL: [SchematicFormat; 3] = [SchematicFormat::McEdit, SchematicFormat::Sponge, SchematicFormat::Litematica];

    /// `DefaultSchematicFormats.extension`, which `getFileExtensions` lists.
    pub fn extension(self) -> &'static str {
        match self {
            SchematicFormat::McEdit => "schematic",
            SchematicFormat::Sponge => "schem",
            SchematicFormat::Litematica => "litematic",
        }
    }

    /// `DefaultSchematicFormats.isFileType`: the extension test.
    /// `ISchematicFormat.isFileType`: the extension of the name, compared with `String.equalsIgnoreCase`.
    pub fn is_file_type(self, name: &str) -> bool {
        equals_ignore_case(self.extension(), extension_of(name))
    }

    /// `SchematicSystem.getByFile`: the first format in registry order whose extension the name has.
    pub fn for_file_name(name: &str) -> Option<SchematicFormat> {
        SchematicFormat::ALL.into_iter().find(|f| f.is_file_type(name))
    }

    /// `DefaultSchematicFormats.getFileExtensions`: the extensions of the three formats.
    /// `ISchematicFormat.getFileExtensions`: the extension of one format.
    /// `SchematicSystem.getFileExtensions`: every format's extensions, in registry order.
    pub fn file_extensions() -> Vec<&'static str> {
        SchematicFormat::ALL.iter().map(|f| f.extension()).collect()
    }

    /// `ISchematicFormat.parse`: the schematic a file holds. The Sponge and Litematica formats look at the file's `Version` first, as upstream
    /// does, and refuse the ones it does not read.
    pub fn parse(self, bytes: &[u8], states: &BlockStates, limits: &ParseLimits) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
        let root = nbt::read_compressed(bytes, limits.max_inflated_bytes)?;
        self.build(&root, states, limits)
    }

    /// [`SchematicFormat::parse`] that takes the file's bytes, so that they are freed as soon as the NBT is read and are not held through the
    /// build of the cells (up to 128 MiB of them in the glue, D50).
    pub fn parse_owned(self, bytes: Vec<u8>, states: &BlockStates, limits: &ParseLimits) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
        let root = nbt::read_compressed(&bytes, limits.max_inflated_bytes);
        drop(bytes);
        self.build(&root?, states, limits)
    }

    /// `DefaultSchematicFormats.MCEDIT.parse`: the MCEdit reader.
    /// `DefaultSchematicFormats.SPONGE.parse`: the Sponge reader, versions 1 and 2.
    /// `DefaultSchematicFormats.LITEMATICA.parse`: the Litematica reader, version 7.
    /// The part of `parse` after the NBT reader.
    pub(crate) fn build(self, root: &nbt::Compound, states: &BlockStates, limits: &ParseLimits) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
        match self {
            SchematicFormat::McEdit => mcedit::parse(root, states, limits),
            SchematicFormat::Sponge => match root.get_int("Version") {
                1 | 2 => sponge::parse(root, states, limits),
                _ => Err(ParseError("Unsupported Version of a Sponge Schematic".to_string())),
            },
            SchematicFormat::Litematica => match root.get_int("Version") {
                4..=6 => Err(ParseError("This litematic Version is too old.".to_string())),
                // the typo is upstream's
                7 => litematica::parse(root, states, limits),
                _ => Err(ParseError("Unsuported Version of a Litematica Schematic".to_string())),
            },
        }
    }
}

/// Like `FilenameUtils.getExtension`: what follows the last `.` of the last path segment (both `/` and `\` separate segments), or "" when there is no dot.
pub fn extension_of(path: &str) -> &str {
    let sep = path.rfind(['/', '\\']).map_or(0, |i| i + 1);
    match path[sep..].rfind('.') {
        Some(i) => &path[sep + i + 1..],
        None => "",
    }
}

/// `String.equalsIgnoreCase`, char by char: equal, or equal after upper-casing, or after lower-casing that (`Character.toUpperCase` is one
/// char to one char, so a character whose upper case is several keeps itself).
fn equals_ignore_case(a: &str, b: &str) -> bool {
    fn upper(c: char) -> char {
        let mut it = c.to_uppercase();
        match (it.next(), it.next()) {
            (Some(u), None) => u,
            _ => c,
        }
    }
    fn lower(c: char) -> char {
        let mut it = c.to_lowercase();
        match (it.next(), it.next()) {
            (Some(l), None) => l,
            _ => c,
        }
    }
    let mut x = a.chars();
    let mut y = b.chars();
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(c), Some(d)) => {
                if c != d && upper(c) != upper(d) && lower(upper(c)) != lower(upper(d)) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

fn is_namespace_char(c: char) -> bool {
    matches!(c, '_' | '-' | '.' | '0'..='9' | 'a'..='z')
}

fn is_path_char(c: char) -> bool {
    is_namespace_char(c) || c == '/'
}

/// Like `ResourceLocation.tryParse`: `namespace:path`, or a path in `minecraft`; `None` for a character that is not allowed (upper case, a space...).
/// A `:` at the start means the default namespace. An empty path is valid, as in the game.
pub(crate) fn try_parse_identifier(s: &str) -> Option<(String, String)> {
    match s.find(':') {
        Some(i) => {
            let (ns, path) = (&s[..i], &s[i + 1..]);
            if !path.chars().all(is_path_char) {
                return None;
            }
            if i == 0 {
                return Some(("minecraft".to_string(), path.to_string()));
            }
            ns.chars().all(is_namespace_char).then(|| (ns.to_string(), path.to_string()))
        }
        None => s.chars().all(is_path_char).then(|| ("minecraft".to_string(), s.to_string())),
    }
}

/// `BuiltInRegistries.BLOCK.get(location)` and `defaultBlockState()`, with `Blocks.AIR` for a block the table does not have.
pub(crate) fn default_state_or_air(states: &BlockStates, namespace: &str, path: &str) -> StateId {
    let name = if namespace == "minecraft" { path.to_string() } else { format!("{namespace}:{path}") };
    states.default_state(&name).unwrap_or_else(|| states.air())
}

/// `setPropertyValue` for each property, as `SpongeSchematic` and `LitematicaSchematic` do: a property the block does not have is skipped, and a
/// value it never takes is an error (`IllegalArgumentException: Invalid value for property`). `props` are applied in the order given.
pub(crate) fn apply_properties<'a>(states: &BlockStates, mut state: StateId, props: impl Iterator<Item = (&'a str, &'a str)>) -> Result<StateId, ParseError> {
    for (key, value) in props {
        if states.has_property(state, key) {
            match states.with_property(state, key, value) {
                Some(next) => state = next,
                None => return Err(ParseError(format!("Invalid value for property {key}"))),
            }
        }
    }
    Ok(state)
}

/// The size check every format makes before it allocates: `Width * Height * Length` against the cell limit. A negative size is the
/// `NegativeArraySizeException` upstream's `new BlockState[x][z][y]` throws.
pub(crate) fn check_size(x: i32, y: i32, z: i32, limits: &ParseLimits) -> Result<u64, ParseError> {
    if x < 0 || y < 0 || z < 0 {
        return Err(ParseError(format!(
            "negative array size: {}",
            if x < 0 {
                x
            } else if z < 0 {
                z
            } else {
                y
            }
        )));
    }
    let cells = x as u64 * y as u64 * z as u64;
    if cells > limits.max_cells {
        return Err(ParseError(format!("schematic is too large for this port: {cells} blocks (limit {})", limits.max_cells)));
    }
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_and_registry_order() {
        assert_eq!(SchematicFormat::file_extensions(), vec!["schematic", "schem", "litematic"]);
        assert_eq!(SchematicFormat::McEdit.extension(), "schematic");
        assert_eq!(SchematicFormat::for_file_name("/a/b/House.SCHEM"), Some(SchematicFormat::Sponge));
        assert_eq!(SchematicFormat::for_file_name("x.litematic"), Some(SchematicFormat::Litematica));
        assert_eq!(SchematicFormat::for_file_name("x.schematic"), Some(SchematicFormat::McEdit));
        assert_eq!(SchematicFormat::for_file_name("x.nbt"), None);
        assert_eq!(SchematicFormat::for_file_name("schem"), None, "no dot, no extension");
        // the extension is after the last dot of the last segment
        assert_eq!(SchematicFormat::for_file_name("dir.schem/file"), None);
        assert_eq!(SchematicFormat::for_file_name("a.schem.bak"), None);
        assert_eq!(SchematicFormat::for_file_name("a.b.schem"), Some(SchematicFormat::Sponge));
        // Character.toUpperCase maps the long s to S
        assert_eq!(SchematicFormat::for_file_name("x.\u{17F}chem"), Some(SchematicFormat::Sponge));
    }

    #[test]
    fn extension_of_follows_commons_io() {
        assert_eq!(extension_of("a.txt"), "txt");
        assert_eq!(extension_of("a"), "");
        assert_eq!(extension_of("a."), "");
        assert_eq!(extension_of(".hidden"), "hidden");
        assert_eq!(extension_of("d.x/f"), "");
        assert_eq!(extension_of("d.x\\f.y"), "y");
        assert_eq!(extension_of(""), "");
    }

    #[test]
    fn identifiers_follow_resource_location() {
        assert_eq!(try_parse_identifier("stone"), Some(("minecraft".into(), "stone".into())));
        assert_eq!(try_parse_identifier("minecraft:oak_stairs"), Some(("minecraft".into(), "oak_stairs".into())));
        assert_eq!(try_parse_identifier("mod:a/b.c-d"), Some(("mod".into(), "a/b.c-d".into())));
        assert_eq!(try_parse_identifier(":stone"), Some(("minecraft".into(), "stone".into())));
        assert_eq!(try_parse_identifier("Stone"), None);
        assert_eq!(try_parse_identifier("minecraft:Stone"), None);
        assert_eq!(try_parse_identifier("my mod:stone"), None);
        assert_eq!(try_parse_identifier("a/b:stone"), None, "a slash is not allowed in a namespace");
        assert_eq!(try_parse_identifier(""), Some(("minecraft".into(), "".into())));
    }

    #[test]
    fn the_version_decides_before_the_reader() {
        let st = BlockStates::test_states();
        let lim = ParseLimits::default();
        let c = nbt::testing::comp(vec![("Version", nbt::Tag::Int(3))]);
        assert_eq!(SchematicFormat::Sponge.build(&c, &st, &lim).err().unwrap().0, "Unsupported Version of a Sponge Schematic");
        assert_eq!(SchematicFormat::Sponge.build(&nbt::Compound::new(), &st, &lim).err().unwrap().0, "Unsupported Version of a Sponge Schematic");
        for v in [4, 5, 6] {
            let c = nbt::testing::comp(vec![("Version", nbt::Tag::Int(v))]);
            assert_eq!(SchematicFormat::Litematica.build(&c, &st, &lim).err().unwrap().0, "This litematic Version is too old.");
        }
        for v in [0, 3, 8] {
            let c = nbt::testing::comp(vec![("Version", nbt::Tag::Int(v))]);
            assert_eq!(SchematicFormat::Litematica.build(&c, &st, &lim).err().unwrap().0, "Unsuported Version of a Litematica Schematic");
        }
        assert!(SchematicFormat::Litematica.parse(b"nope", &st, &lim).is_err());
    }
}

/// The fuzz every format's tests share.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::cache::gzip;

    /// A deterministic xorshift.
    pub struct Rng(pub u64);
    impl Rng {
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        pub fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    /// One mutation of `base`: bytes flipped or set to edge values, a cut, an insertion, a copied chunk, or noise.
    pub fn mutate(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
        let mut m = base.to_vec();
        match rng.below(7) {
            0 => {
                for _ in 0..1 + rng.below(4) {
                    let at = rng.below(m.len());
                    m[at] = rng.next() as u8;
                }
            }
            1 => m.truncate(rng.below(m.len() + 1)),
            2 => {
                let at = rng.below(m.len() + 1);
                m.insert(at, rng.next() as u8);
            }
            3 => {
                let at = rng.below(m.len().saturating_sub(4).max(1));
                let v = [0u8, 0xFF, 0x7F, 0x80][rng.below(4)];
                for b in m.iter_mut().skip(at).take(1 + rng.below(4)) {
                    *b = v;
                }
            }
            4 => {
                let n = rng.below(200);
                m = (0..n).map(|_| rng.next() as u8).collect();
            }
            5 => {
                let (from, to, len) = (rng.below(m.len()), rng.below(m.len()), rng.below(32));
                let chunk: Vec<u8> = m.iter().skip(from).take(len).copied().collect();
                for (i, b) in chunk.into_iter().enumerate() {
                    if to + i < m.len() {
                        m[to + i] = b;
                    }
                }
            }
            _ => {
                // two byte changes near the numbers at the front of the file
                for _ in 0..2 {
                    let at = rng.below(m.len().min(80));
                    m[at] = rng.next() as u8;
                }
            }
        }
        m
    }

    /// Reads `iterations` mutated inputs: the NBT of `base` (uncompressed) mutated and read, and now and then the whole gzip path on mutated
    /// compressed bytes. Nothing may panic, and some must read (so that the fuzz reaches the format).
    pub fn fuzz(format: SchematicFormat, base: &[u8], states: &BlockStates, limits: &ParseLimits, iterations: usize, seed: u64) {
        let mut rng = Rng(seed);
        let (mut nbt_ok, mut built) = (0usize, 0usize);
        let compressed = gzip::compress(base, 1);
        for i in 0..iterations {
            let m = mutate(&mut rng, base);
            if let Ok(c) = nbt::read_uncompressed(&m, limits.max_inflated_bytes) {
                nbt_ok += 1;
                if format.build(&c, states, limits).is_ok() {
                    built += 1;
                }
            }
            if i % 50 == 0 {
                let _ = format.parse(&mutate(&mut rng, &compressed), states, limits);
                let _ = format.parse(&gzip::compress(&m, 1), states, limits);
            }
        }
        assert!(nbt_ok * 20 > iterations, "the fuzz should reach the format: {nbt_ok} of {iterations} read as NBT");
        assert!(built > 0, "no mutation of the base was ever read as a schematic");
    }
}
