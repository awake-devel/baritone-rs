// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/schematic/format/defaults/MCEditSchematic.java. LGPL-3.0-or-later, see lib.rs.
//
// The MCEdit `.schematic` format: numeric block ids in a byte array, with the high bits of the id in `AddBlocks`. The metadata (`Data`) is ignored,
// as upstream ignores it (its code for it is commented out). An id goes through `ItemIdFix` to a pre-1.13 name, and a name the registry does not
// have is air (U31): `minecraft:grass`, `planks`, `log`, `stonebrick` and the like are all air here.
use super::legacy_ids::item_name;
use super::nbt::Compound;
use super::{check_size, default_state_or_air, try_parse_identifier, ParseError, ParseLimits};
use crate::schematic::{BlockStates, StateId, StaticSchematic, StaticSchematicView};
use std::sync::Arc;

/// `MCEditSchematic.<init>(CompoundTag)`.
pub(super) fn parse(schematic: &Compound, states: &BlockStates, limits: &ParseLimits) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
    let ty = schematic.get_string("Materials");
    if ty != "Alpha" {
        return Err(ParseError(format!("bad schematic {ty}")));
    }
    let (x, y, z) = (schematic.get_int("Width"), schematic.get_int("Height"), schematic.get_int("Length"));
    let blocks = schematic.get_byte_array("Blocks");

    // `new BlockState[x][z][y]`: the size is checked before anything is made from the file (D50)
    check_size(x, y, z, limits)?;
    // `AddBlocks` upstream is expanded to one nibble per entry: the first (index 2k) is the high nibble of byte k, the second the low one (the
    // upstream comments say the opposite). Here the nibble is read from the byte when it is wanted, so no copy of twice the size is made.
    // The length upstream would have given the expanded array is `2 * len`.
    let additional: Option<&[i8]> = if schematic.contains("AddBlocks") { Some(schematic.get_byte_array("AddBlocks")) } else { None };
    // one lookup per distinct id, as the registry lookup would give the same state every time
    let mut by_id: std::collections::HashMap<i32, StateId> = std::collections::HashMap::new();
    let air = states.air();
    let mut out = vec![air; x as usize * y as usize * z as usize];
    for yy in 0..y {
        for zz in 0..z {
            for xx in 0..x {
                let block_ind = yy.wrapping_mul(z).wrapping_add(zz).wrapping_mul(x).wrapping_add(xx);
                let at = |len: usize| usize::try_from(block_ind).ok().filter(|&i| i < len).ok_or_else(|| ParseError(format!("Index {block_ind} out of bounds for length {len}")));
                let mut block_id = i32::from(blocks[at(blocks.len())?] as u8);
                if let Some(add) = additional {
                    let i = at(add.len().saturating_mul(2))?;
                    let b = add[i / 2] as u8;
                    block_id |= i32::from(if i % 2 == 0 { b >> 4 } else { b & 0xF }) << 8;
                }
                let state = *by_id.entry(block_id).or_insert_with(|| match try_parse_identifier(item_name(block_id)) {
                    None => air,
                    Some((ns, path)) => default_state_or_air(states, &ns, &path),
                });
                out[((xx as usize * z as usize) + zz as usize) * y as usize + yy as usize] = state;
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

    fn file(w: i32, h: i32, l: i32, blocks: Vec<u8>, add: Option<Vec<u8>>) -> Compound {
        let mut c = comp(vec![("Materials", s("Alpha")), ("Width", Tag::Short(w as i16)), ("Height", Tag::Short(h as i16)), ("Length", Tag::Short(l as i16)), ("Blocks", Tag::ByteArray(blocks.iter().map(|&b| b as i8).collect())), ("Data", Tag::ByteArray(vec![0; blocks.len()]))]);
        if let Some(a) = add {
            c.insert("AddBlocks", Tag::ByteArray(a.iter().map(|&b| b as i8).collect()));
        }
        c
    }

    fn read(c: &Compound, st: &BlockStates) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
        SchematicFormat::McEdit.parse(&gz(c), st, &ParseLimits::default())
    }

    #[test]
    fn legacy_ids_become_states_and_renamed_ones_become_air() {
        let st = BlockStates::test_states();
        // x runs fastest, then z, then y: ids 1 (stone), 4 (cobblestone), 2 (grass: renamed, so air), 3 (dirt), 12 (sand), 5 (planks: renamed), 20 (glass), 0
        let c = file(2, 2, 2, vec![1, 4, 2, 3, 12, 5, 20, 0], None);
        let sch = read(&c, &st).unwrap();
        assert_eq!((sch.width_x(), sch.height_y(), sch.length_z()), (2, 2, 2));
        let g = |x, y, z| st.block(sch.get_direct(x, y, z).unwrap()).to_string();
        // index = (y * length + z) * width + x
        assert_eq!(g(0, 0, 0), "stone");
        assert_eq!(g(1, 0, 0), "cobblestone");
        assert_eq!(g(0, 0, 1), "air", "id 2 is minecraft:grass, which 26.3 does not have");
        assert_eq!(g(1, 0, 1), "dirt");
        assert_eq!(g(0, 1, 0), "sand");
        assert_eq!(g(1, 1, 0), "air", "id 5 is minecraft:planks");
        assert_eq!(g(0, 1, 1), "glass");
        assert_eq!(g(1, 1, 1), "air");
    }

    #[test]
    fn add_blocks_nibbles_high_first() {
        let st = BlockStates::test_states();
        // two cells: one AddBlocks byte 0x10 gives cell 0 the high bits 1 and cell 1 the high bits 0.
        // cell 0: id = 1 | 1 << 8 = 257 (not in the table: air). cell 1: id 1 = stone.
        let sch = read(&file(2, 1, 1, vec![1, 1], Some(vec![0x10])), &st).unwrap();
        assert_eq!(st.block(sch.get_direct(0, 0, 0).unwrap()), "air");
        assert_eq!(st.block(sch.get_direct(1, 0, 0).unwrap()), "stone");
        // and the other way round: 0x01 puts the high bits on the second cell
        let sch = read(&file(2, 1, 1, vec![1, 1], Some(vec![0x01])), &st).unwrap();
        assert_eq!(st.block(sch.get_direct(0, 0, 0).unwrap()), "stone");
        assert_eq!(st.block(sch.get_direct(1, 0, 0).unwrap()), "air");
        // a negative AddBlocks byte masks cleanly: 0xF1 is nibbles 15 and 1
        assert!(read(&file(2, 1, 1, vec![1, 1], Some(vec![0xF1])), &st).is_ok());
        // AddBlocks shorter than the cells is the array bounds exception
        assert!(read(&file(4, 1, 1, vec![1; 4], Some(vec![0])), &st).is_err());
    }

    #[test]
    fn materials_and_sizes() {
        let st = BlockStates::test_states();
        let mut c = file(1, 1, 1, vec![1], None);
        c.insert("Materials", s("Classic"));
        assert_eq!(read(&c, &st).err().unwrap().0, "bad schematic Classic");
        let mut c = file(1, 1, 1, vec![1], None);
        c.insert("Materials", Tag::Int(1));
        assert_eq!(read(&c, &st).err().unwrap().0, "bad schematic ");
        // Blocks shorter than the box
        assert!(read(&file(2, 2, 2, vec![1; 4], None), &st).is_err());
        // longer is fine
        assert!(read(&file(1, 1, 1, vec![1; 4], None), &st).is_ok());
        // negative size
        assert!(read(&file(-1, 2, 2, vec![], None), &st).is_err());
        // an empty box keeps the sizes of the file: upstream's reader sets them directly and never zeroes them
        let e = read(&file(0, 5, 5, vec![], None), &st).unwrap();
        assert_eq!((e.width_x(), e.height_y(), e.length_z()), (0, 5, 5));
        assert_eq!(e.get_direct(0, 0, 0), None);
        assert_eq!(e.get_column(0, 0), None);
        // the limits
        let lim = ParseLimits { max_inflated_bytes: 1 << 20, max_cells: 7 };
        assert!(SchematicFormat::McEdit.parse(&gz(&file(2, 2, 2, vec![1; 8], None)), &st, &lim).is_err());
        assert!(SchematicFormat::McEdit.parse(&gz(&file(7, 1, 1, vec![1; 7], None)), &st, &lim).is_ok());
    }

    #[test]
    fn size_tags_of_other_types_follow_get_int() {
        let st = BlockStates::test_states();
        let mut c = file(2, 1, 1, vec![1, 1], None);
        c.insert("Width", Tag::Float(2.9));
        assert_eq!(read(&c, &st).unwrap().width_x(), 2);
        c.insert("Width", s("2"));
        assert_eq!(read(&c, &st).unwrap().width_x(), 0, "a string width is 0");
    }

    #[test]
    fn fuzz_never_panics() {
        let st = BlockStates::test_states();
        let lim = ParseLimits { max_inflated_bytes: 1 << 20, max_cells: 1 << 14 };
        let base = root(&file(3, 2, 2, vec![1, 4, 2, 3, 12, 5, 20, 0, 1, 1, 1, 1], Some(vec![0x10, 0, 0, 0, 0, 0])));
        super::super::tests_support::fuzz(SchematicFormat::McEdit, &base, &st, &lim, 100_000, 0x9e37_79b9_7f4a_7c15);
    }
}
