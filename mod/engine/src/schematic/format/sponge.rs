// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/schematic/format/defaults/SpongeSchematic.java and
// src/main/java/baritone/utils/type/VarInt.java. LGPL-3.0-or-later, see lib.rs.
//
// The Sponge `.schem` format, versions 1 and 2 (`DefaultSchematicFormats` refuses the others before this code runs): a palette from block
// state text to index, and the blocks as an array of varints indexed `(y * length + z) * width + x`.
//
// One thing is changed from upstream (U30, D52) and one is kept:
// - upstream advances through the block data by `VarInt.getSize`, which is the length of `serialize0(value)` and not the number of bytes read
//   (`serialize0` tests bit 7 of the value, so a canonical index of 256, 300, 512 or 16384 counts as one byte). That garbles Sponge files with
//   more than 256 palette entries. This port advances by the bytes it actually read for each varint, so such files, and varints written with
//   extra bytes (0x80 0x00 for 0), load as written.
// - the palette is read with a regular expression whose properties part is `(\w+=\w+,?)+`; the comma between two properties is optional, so
//   `a=bc=d` is accepted, and read as the property `a` with the value `bc`.
use super::nbt::Compound;
use super::{apply_properties, check_size, default_state_or_air, ParseError, ParseLimits};
use crate::schematic::{BlockStates, StateId, StaticSchematic, StaticSchematicView};
use std::collections::HashMap;
use std::sync::Arc;

/// `VarInt.read(byte[],int)`: reads one variable-length int.
/// `VarInt.read(byte[])`: the same from the start.
/// `VarInt.getValue`: the value.
/// `VarInt.getSize` is replaced by the number of bytes read, which is returned with the value (U30, D52).
fn read_var_int(bytes: &[i8], start: usize) -> Result<(i32, usize), ParseError> {
    let mut value: i32 = 0;
    let mut size: u32 = 0;
    let mut index = start;
    loop {
        let b = *bytes.get(index).ok_or_else(|| ParseError(format!("Index {index} out of bounds for length {}", bytes.len())))? as u8;
        index += 1;
        // `(b & 0x7F) << size++ * 7`: the shift count is taken modulo 32, as Java's `int` shift does
        value |= i32::from(b & 0x7F).wrapping_shl(size * 7);
        size += 1;
        if size > 5 {
            return Err(ParseError("VarInt size cannot exceed 5 bytes".to_string()));
        }
        if b & 0x80 == 0 {
            break;
        }
    }
    Ok((value, size as usize))
}

/// `SerializedBlockState`: the block and its properties, parsed from a palette key.
struct SerializedBlockState {
    namespace: String,
    path: String,
    properties: HashMap<String, String>,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The regular expression `(\w+:)?\w+` over the whole of `s`.
fn matches_location(s: &[u8]) -> bool {
    let word = |s: &[u8]| !s.is_empty() && s.iter().all(|&b| is_word(b));
    match s.iter().position(|&b| b == b':') {
        Some(i) => word(&s[..i]) && word(&s[i + 1..]),
        None => word(s),
    }
}

/// The regular expression `(\w+=\w+,?)+` over the whole of `s`, with the backtracking a regex does: after `\w+=` the value `\w+` may stop
/// anywhere inside its run of word characters, because the next item may start there.
fn matches_properties(s: &[u8]) -> bool {
    let n = s.len();
    if n == 0 {
        return false;
    }
    // run[i] = the number of word characters from i on
    let mut run = vec![0usize; n + 1];
    for i in (0..n).rev() {
        run[i] = if is_word(s[i]) { run[i + 1] + 1 } else { 0 };
    }
    // reach[i]: some whole number of items covers s[..i]. The values an item may take end anywhere inside a run of word characters, so every
    // reachable position marks a whole range of later ones: a difference array marks them in one step, and the whole is linear in the length
    // (a loop over the range from every position was quadratic in the longest run, D50).
    let mut reach_flag = vec![false; n + 1];
    reach_flag[0] = true;
    let mut diff = vec![0i32; n + 2];
    let mut cover = 0i32;
    let mut reach_n = false;
    for p in 0..=n {
        cover += diff[p];
        let reachable = reach_flag[p] || cover > 0;
        if p == n {
            reach_n = reachable;
            break;
        }
        if !reachable {
            continue;
        }
        let a = run[p];
        if a == 0 || p + a >= n || s[p + a] != b'=' {
            continue;
        }
        let value_start = p + a + 1;
        let r = run[value_start];
        if r == 0 {
            continue;
        }
        // the value may stop after 1 to `r` characters: positions value_start + 1 ..= value_start + r
        diff[value_start + 1] += 1;
        diff[value_start + r + 1] -= 1;
        // a comma can only follow the last of them (every other one is followed by a word character)
        let end = value_start + r;
        if end < n && s[end] == b',' {
            reach_flag[end + 1] = true;
        }
    }
    reach_n
}

impl SerializedBlockState {
    /// `SpongeSchematic.SerializedBlockState.getFromString`: `None` when the text does not fit the pattern, or when `ResourceLocation.parse` throws, which it does for
    /// an upper-case letter (the pattern allows them, the identifier does not).
    fn from_string(s: &str) -> Option<SerializedBlockState> {
        let bytes = s.as_bytes();
        let (location, properties) = match bytes.iter().position(|&b| b == b'[') {
            None => (bytes, None),
            Some(i) => {
                if i + 1 >= bytes.len() || bytes[bytes.len() - 1] != b']' {
                    return None;
                }
                (&bytes[..i], Some(&bytes[i + 1..bytes.len() - 1]))
            }
        };
        if !matches_location(location) || properties.is_some_and(|p| !matches_properties(p)) {
            return None;
        }
        // everything matched is ASCII
        let location = std::str::from_utf8(location).ok()?;
        let (namespace, path) = super::try_parse_identifier(location)?;
        // `ResourceLocation.parse` has no `:path` shortcut to worry about here: the pattern needs a word before the colon
        let mut map = HashMap::new();
        if let Some(p) = properties {
            let p = std::str::from_utf8(p).ok()?;
            // `String.split(",")` drops the empty strings at the end
            let mut pieces: Vec<&str> = p.split(',').collect();
            while pieces.last().is_some_and(|x| x.is_empty()) {
                pieces.pop();
            }
            for piece in pieces {
                let mut kv = piece.split('=');
                let (k, v) = (kv.next()?, kv.next()?);
                map.insert(k.to_string(), v.to_string());
            }
        }
        Some(SerializedBlockState { namespace, path, properties: map })
    }

    /// `SpongeSchematic.SerializedBlockState.deserialize`: an unknown block is air; the properties go on in sorted key order.
    fn deserialize(&self, states: &BlockStates) -> Result<StateId, ParseError> {
        let base = default_state_or_air(states, &self.namespace, &self.path);
        let mut keys: Vec<&String> = self.properties.keys().collect();
        keys.sort();
        apply_properties(states, base, keys.into_iter().map(|k| (k.as_str(), self.properties[k].as_str())))
    }
}

/// `SpongeSchematic.<init>(CompoundTag)`.
pub(super) fn parse(nbt: &Compound, states: &BlockStates, limits: &ParseLimits) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
    let (x, y, z) = (nbt.get_int("Width"), nbt.get_int("Height"), nbt.get_int("Length"));
    // `new BlockState[x][z][y]`
    check_size(x, y, z, limits)?;

    let mut palette: HashMap<i32, StateId> = HashMap::new();
    let palette_tag = nbt.get_compound("Palette");
    // a later key with the same index wins, so the order of the keys is the order the Java map gives them
    for tag in palette_tag.keys_java_order() {
        let index = palette_tag.get_int(tag);
        let serialized = SerializedBlockState::from_string(tag).ok_or_else(|| ParseError("Unable to parse palette tag".to_string()))?;
        palette.insert(index, serialized.deserialize(states)?);
    }

    // BlockData is stored as an NBT byte[], however, the actual data that is represented is a varint[]
    let raw = nbt.get_byte_array("BlockData");
    // `new int[this.x * this.y * this.z]`: an int product
    let count = x.wrapping_mul(y).wrapping_mul(z);
    if count < 0 {
        return Err(ParseError(format!("negative array size: {count}")));
    }
    let mut block_data = Vec::with_capacity((count as usize).min(raw.len()));
    let mut offset = 0usize;
    for _ in 0..count {
        if offset >= raw.len() {
            return Err(ParseError("No remaining bytes in BlockData for complete schematic".to_string()));
        }
        let (value, size) = read_var_int(raw, offset)?;
        block_data.push(value);
        offset += size;
    }

    let air = states.air();
    let mut out = vec![air; x as usize * y as usize * z as usize];
    for yy in 0..y {
        for zz in 0..z {
            for xx in 0..x {
                let index = yy.wrapping_mul(z).wrapping_add(zz).wrapping_mul(x).wrapping_add(xx);
                let data = usize::try_from(index).ok().and_then(|i| block_data.get(i)).ok_or_else(|| ParseError(format!("Index {index} out of bounds for length {}", block_data.len())))?;
                let state = palette.get(data).ok_or_else(|| ParseError(format!("Invalid Palette Index {index}")))?;
                out[((xx as usize * z as usize) + zz as usize) * y as usize + yy as usize] = *state;
            }
        }
    }
    Ok(Arc::new(StaticSchematic::with_raw_sizes(x, y, z, out, air)))
}

#[cfg(test)]
mod tests {
    use super::super::nbt::testing::*;
    use super::super::nbt::Tag;
    use super::super::SchematicFormat;
    use super::*;

    fn var(v: u32, out: &mut Vec<i8>) {
        let mut v = v;
        while v & !0x7F != 0 {
            out.push((v & 0x7F | 0x80) as u8 as i8);
            v >>= 7;
        }
        out.push(v as u8 as i8);
    }

    fn file(version: i32, w: i32, h: i32, l: i32, palette: &[(&str, i32)], data: Vec<i8>) -> Compound {
        let mut p = Compound::new();
        for (k, v) in palette {
            p.insert(k, Tag::Int(*v));
        }
        comp(vec![("Version", Tag::Int(version)), ("Width", Tag::Short(w as i16)), ("Height", Tag::Short(h as i16)), ("Length", Tag::Short(l as i16)), ("Palette", Tag::Compound(p)), ("BlockData", Tag::ByteArray(data))])
    }

    fn read(c: &Compound, st: &BlockStates) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
        SchematicFormat::Sponge.parse(&gz(c), st, &ParseLimits::default())
    }

    fn name(st: &BlockStates, s: &dyn StaticSchematicView, x: i32, y: i32, z: i32) -> String {
        st.state_to_string(s.get_direct(x, y, z).unwrap())
    }

    #[test]
    fn versions_one_and_two_read_and_three_is_refused() {
        let st = BlockStates::test_states();
        let pal = [("minecraft:stone", 0), ("minecraft:oak_planks", 1), ("minecraft:air", 2)];
        // 2 wide, 2 high, 1 long: index = (y * 1 + z) * 2 + x
        let data = vec![0, 1, 2, 0];
        for v in [1, 2] {
            let s = read(&file(v, 2, 2, 1, &pal, data.clone()), &st).unwrap();
            assert_eq!((s.width_x(), s.height_y(), s.length_z()), (2, 2, 1));
            assert_eq!(st.block(s.get_direct(0, 0, 0).unwrap()), "stone");
            assert_eq!(st.block(s.get_direct(1, 0, 0).unwrap()), "oak_planks");
            assert_eq!(st.block(s.get_direct(0, 1, 0).unwrap()), "air");
            assert_eq!(st.block(s.get_direct(1, 1, 0).unwrap()), "stone");
        }
        assert_eq!(read(&file(3, 2, 2, 1, &pal, data.clone()), &st).err().unwrap().0, "Unsupported Version of a Sponge Schematic");
        assert_eq!(read(&file(0, 2, 2, 1, &pal, data), &st).err().unwrap().0, "Unsupported Version of a Sponge Schematic");
    }

    #[test]
    fn palette_states_properties_and_unknown_blocks() {
        let st = BlockStates::test_states();
        let pal = [("minecraft:oak_stairs[facing=east,half=top]", 0), ("mymod:thing", 1), ("minecraft:stone[nonsense=1]", 2), ("oak_log[axis=z]", 3)];
        let s = read(&file(2, 4, 1, 1, &pal, vec![0, 1, 2, 3]), &st).unwrap();
        assert_eq!(name(&st, &*s, 0, 0, 0), "Block{minecraft:oak_stairs}[facing=east,half=top,shape=straight,waterlogged=false]");
        assert_eq!(st.block(s.get_direct(1, 0, 0).unwrap()), "air", "an unknown block is air");
        assert_eq!(st.block(s.get_direct(2, 0, 0).unwrap()), "stone", "a property the block lacks is skipped");
        assert_eq!(st.prop(s.get_direct(3, 0, 0).unwrap(), "axis"), Some("z"));
    }

    #[test]
    fn palette_errors() {
        let st = BlockStates::test_states();
        let one = |key: &str| read(&file(2, 1, 1, 1, &[(key, 0)], vec![0]), &st);
        // upper case passes the pattern and fails the identifier
        assert_eq!(one("minecraft:Stone").err().unwrap().0, "Unable to parse palette tag");
        assert_eq!(one("Stone").err().unwrap().0, "Unable to parse palette tag");
        // not the pattern at all
        for bad in ["", "a:b:c", "a-b", "stone[", "stone[]", "stone[a]", "stone[a=]", "stone[=b]", "stone[a=b", "stone [a=b]", "a/b", "stone[a=b]x", "stone[a=b,,c=d]", "stone[,a=b]"] {
            assert_eq!(one(bad).err().unwrap().0, "Unable to parse palette tag", "{bad:?}");
        }
        // a value the property never takes
        assert_eq!(one("oak_stairs[facing=up]").err().unwrap().0, "Invalid value for property facing");
        assert_eq!(one("oak_stairs[waterlogged=maybe]").err().unwrap().0, "Invalid value for property waterlogged");
        // accepted: a trailing comma, and the comma left out between two properties (the regex allows both)
        assert!(one("oak_stairs[facing=east,]").is_ok());
        assert_eq!(one("oak_stairs[half=topfacing=east]").err().unwrap().0, "Invalid value for property half");
    }

    #[test]
    fn comma_less_properties_are_one_property() {
        // the regex accepts `a=bc=d`; upstream splits it into the property a with the value bc
        let st = BlockStates::test_states();
        let s = SerializedBlockState::from_string("stone[a=bc=d]").unwrap();
        assert_eq!(s.properties.get("a").map(String::as_str), Some("bc"));
        assert_eq!(s.properties.len(), 1);
        // a duplicate key: the later one wins
        let s = SerializedBlockState::from_string("stone[a=1,a=2]").unwrap();
        assert_eq!(s.properties.get("a").map(String::as_str), Some("2"));
        let _ = st;
    }

    #[test]
    fn the_property_pattern_backtracks_like_a_regex() {
        for ok in ["a=b", "a=b,", "a=b,c=d", "a=bc=d", "a=b,c=d,", "a_1=B_2", "a=bc=de=f"] {
            assert!(matches_properties(ok.as_bytes()), "{ok}");
        }
        for bad in ["", "a", "a=", "=b", "a=b,,c=d", ",a=b", "a=b c=d", "a==b", "a=b=", "a=b,=c"] {
            assert!(!matches_properties(bad.as_bytes()), "{bad}");
        }
        assert!(matches_location(b"a:b") && matches_location(b"ab") && matches_location(b"A_9:b_c"));
        assert!(!matches_location(b"a:") && !matches_location(b":b") && !matches_location(b"a:b:c") && !matches_location(b""));
    }

    #[test]
    fn a_later_duplicate_palette_index_wins_by_hash_map_order() {
        // "stone" (hash 109761319 -> bucket by the spread) and "dirt" (3083005) both claim index 0; whichever the Java map iterates last wins
        let st = BlockStates::test_states();
        let mut p = Compound::new();
        p.insert("stone", Tag::Int(0));
        p.insert("dirt", Tag::Int(0));
        let last = p.keys_java_order()[1].to_string();
        let c = comp(vec![("Version", Tag::Int(2)), ("Width", Tag::Int(1)), ("Height", Tag::Int(1)), ("Length", Tag::Int(1)), ("Palette", Tag::Compound(p)), ("BlockData", Tag::ByteArray(vec![0]))]);
        let s = read(&c, &st).unwrap();
        assert_eq!(st.block(s.get_direct(0, 0, 0).unwrap()), last.as_str());
    }

    #[test]
    fn var_ints() {
        let mut v = vec![];
        // the value reads back and the size is the number of bytes read (D52)
        for (n, size) in [(0u32, 1), (1, 1), (127, 1), (128, 2), (255, 2), (256, 2), (300, 2), (383, 2), (384, 2), (16383, 2), (16384, 3), (2097152, 4), (268435455, 4), (268435456, 5)] {
            v.clear();
            var(n, &mut v);
            assert_eq!(read_var_int(&v, 0).unwrap(), (n as i32, size), "{n}");
        }
        // a negative value takes 5 bytes
        v.clear();
        var(u32::MAX, &mut v);
        assert_eq!(read_var_int(&v, 0).unwrap(), (-1, 5));
        // an over-long form of 0 is read as 0 and counts its bytes; a read can start inside the slice
        assert_eq!(read_var_int(&[-128, 0], 0).unwrap(), (0, 2));
        assert_eq!(read_var_int(&[9, -128, 2], 1).unwrap(), (256, 2));
        // a sixth byte is refused, and so is running off the end
        assert_eq!(read_var_int(&[-1, -1, -1, -1, -1, 1], 0).unwrap_err().0, "VarInt size cannot exceed 5 bytes");
        assert!(read_var_int(&[-1, -1], 0).is_err());
        assert!(read_var_int(&[], 0).is_err());
        // the shift of the fifth byte: 0x0F << 28 keeps its low 4 bits
        assert_eq!(read_var_int(&[-1, -1, -1, -1, 0x7F], 0).unwrap().0, -1);
        assert_eq!(read_var_int(&[0x80u8 as i8, 0x80u8 as i8, 0x80u8 as i8, 0x80u8 as i8, 0x1F], 0).unwrap().0, (0x1Fi32).wrapping_shl(28));
    }

    #[test]
    fn a_non_canonical_varint_is_read_by_its_bytes() {
        // D52: cell 0 is written as 0x80 0x00 (the value 0 in two bytes), then 1, then 1. Upstream advanced by 1 and read the 0x00 as cell 1.
        let st = BlockStates::test_states();
        let pal = [("stone", 0), ("dirt", 1)];
        let s = read(&file(2, 3, 1, 1, &pal, vec![-128, 0, 1, 1]), &st).unwrap();
        assert_eq!(st.block(s.get_direct(0, 0, 0).unwrap()), "stone");
        assert_eq!(st.block(s.get_direct(1, 0, 0).unwrap()), "dirt");
        assert_eq!(st.block(s.get_direct(2, 0, 0).unwrap()), "dirt");
        // the canonical bytes read the same
        let s = read(&file(2, 3, 1, 1, &pal, vec![0, 1, 1]), &st).unwrap();
        assert_eq!(st.block(s.get_direct(1, 0, 0).unwrap()), "dirt");
        // an over-long varint that is cut short is still an error
        assert!(read(&file(2, 2, 1, 1, &pal, vec![0, -128]), &st).is_err());
    }

    #[test]
    fn a_canonical_index_of_256_reads_correctly() {
        // D52: index 256 is 0x80 0x02. Cells 0 to 2 are 256, 0, 2 (sand, stone, dirt); upstream read them as 256, 2, 0.
        let st = BlockStates::test_states();
        let pal = [("stone", 0), ("dirt", 2), ("sand", 256)];
        let s = read(&file(2, 3, 1, 1, &pal, vec![-128, 2, 0, 2]), &st).unwrap();
        assert_eq!(st.block(s.get_direct(0, 0, 0).unwrap()), "sand");
        assert_eq!(st.block(s.get_direct(1, 0, 0).unwrap()), "stone");
        assert_eq!(st.block(s.get_direct(2, 0, 0).unwrap()), "dirt");
    }

    #[test]
    fn a_palette_of_more_than_256_entries_round_trips() {
        // D52: stairs variants stand for the indices (facing, half and waterlogged tell the cells apart); the indices include the ones
        // upstream's size misread (256, 300, 512, 16384) and the ones just beside them
        let st = BlockStates::test_states();
        let picks: [i32; 12] = [0, 256, 300, 255, 512, 383, 384, 1, 599, 16384, 2, 513];
        let mut p = Compound::new();
        let mut data = Vec::new();
        let mut expect = Vec::new();
        for (n, &i) in picks.iter().enumerate() {
            let facing = ["north", "south", "east", "west"][n % 4];
            let half = ["top", "bottom"][(n / 4) % 2];
            let waterlogged = ["true", "false"][(n / 8) % 2];
            p.insert(&format!("minecraft:oak_stairs[facing={facing},half={half},waterlogged={waterlogged}]"), Tag::Int(i));
            var(i as u32, &mut data);
            expect.push((facing, half, waterlogged));
        }
        let c = comp(vec![("Version", Tag::Int(2)), ("Width", Tag::Short(12)), ("Height", Tag::Short(1)), ("Length", Tag::Short(1)), ("Palette", Tag::Compound(p)), ("BlockData", Tag::ByteArray(data))]);
        let s = read(&c, &st).unwrap();
        for (x, (facing, half, waterlogged)) in expect.iter().enumerate() {
            let cell = s.get_direct(x as i32, 0, 0).unwrap();
            assert_eq!(st.prop(cell, "facing"), Some(*facing), "cell {x}");
            assert_eq!(st.prop(cell, "half"), Some(*half), "cell {x}");
            assert_eq!(st.prop(cell, "waterlogged"), Some(*waterlogged), "cell {x}");
        }
    }

    /// The first version of `matches_properties`, which marks every position of the range one at a time: the reference for the linear one.
    fn matches_properties_slow(s: &[u8]) -> bool {
        let n = s.len();
        if n == 0 {
            return false;
        }
        let mut run = vec![0usize; n + 1];
        for i in (0..n).rev() {
            run[i] = if is_word(s[i]) { run[i + 1] + 1 } else { 0 };
        }
        let mut reach = vec![false; n + 1];
        reach[0] = true;
        for p in 0..n {
            if !reach[p] {
                continue;
            }
            let a = run[p];
            if a == 0 || p + a >= n || s[p + a] != b'=' {
                continue;
            }
            let value_start = p + a + 1;
            for k in 1..=run[value_start] {
                let q = value_start + k;
                reach[q] = true;
                if q < n && s[q] == b',' {
                    reach[q + 1] = true;
                }
            }
        }
        reach[n]
    }

    #[test]
    fn review_the_property_pattern_matches_as_before_and_in_linear_time() {
        // every string over a small alphabet up to length 9 agrees with the old loop
        let alphabet = b"a=,_ ";
        let mut total = 0u32;
        for len in 0..=9u32 {
            for code in 0..(alphabet.len() as u32).pow(len) {
                let mut c = code;
                let s: Vec<u8> = (0..len)
                    .map(|_| {
                        let b = alphabet[(c % alphabet.len() as u32) as usize];
                        c /= alphabet.len() as u32;
                        b
                    })
                    .collect();
                assert_eq!(matches_properties(&s), matches_properties_slow(&s), "{:?}", String::from_utf8_lossy(&s));
                total += 1;
            }
        }
        assert!(total > 2_000_000);
        // a key of `a`s run into each other ate quadratic time: 63 KB took a quarter of a second in release, and the 3,900 keys the budget allows
        // a quarter of an hour. Two million bytes in one run now cost nothing noticeable.
        let mut long = vec![b'a'; 700_000];
        long.push(b'=');
        long.extend(std::iter::repeat_n(b'a', 700_000));
        long.push(b'=');
        long.extend(std::iter::repeat_n(b'a', 600_000));
        let began = std::time::Instant::now();
        assert!(matches_properties(&long));
        assert!(began.elapsed() < std::time::Duration::from_secs(5), "{:?}", began.elapsed());
    }

    #[test]
    fn block_data_errors() {
        let st = BlockStates::test_states();
        let pal = [("stone", 0)];
        // too few bytes
        assert_eq!(read(&file(2, 2, 2, 1, &pal, vec![0, 0, 0]), &st).err().unwrap().0, "No remaining bytes in BlockData for complete schematic");
        // an index the palette lacks: the message carries the cell index
        assert_eq!(read(&file(2, 2, 2, 1, &pal, vec![0, 5, 0, 0]), &st).err().unwrap().0, "Invalid Palette Index 1");
        // more bytes than cells are ignored
        assert!(read(&file(2, 1, 1, 1, &pal, vec![0, 0, 0, 0]), &st).is_ok());
        // a negative size, and the limits
        assert!(read(&file(2, -1, 1, 1, &pal, vec![]), &st).is_err());
        let lim = ParseLimits { max_inflated_bytes: 1 << 20, max_cells: 3 };
        assert!(SchematicFormat::Sponge.parse(&gz(&file(2, 2, 2, 1, &pal, vec![0; 4])), &st, &lim).is_err());
        // an empty schematic is fine
        let s = read(&file(2, 0, 3, 3, &pal, vec![]), &st).unwrap();
        assert_eq!((s.width_x(), s.height_y(), s.length_z()), (0, 3, 3), "the other sizes stay, as upstream's reader leaves them");
        // a varint whose bytes run off the end in the middle
        assert!(read(&file(2, 1, 1, 1, &pal, vec![-1]), &st).is_err());
    }

    #[test]
    fn fuzz_never_panics() {
        let st = BlockStates::test_states();
        let lim = ParseLimits { max_inflated_bytes: 1 << 20, max_cells: 1 << 14 };
        let base = root(&file(2, 3, 2, 2, &[("minecraft:stone", 0), ("minecraft:oak_stairs[facing=east,half=top]", 1), ("minecraft:air", 2)], vec![0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2]));
        super::super::tests_support::fuzz(SchematicFormat::Sponge, &base, &st, &lim, 100_000, 0x9e37_79b9_7f4a_7c15);
    }
}
