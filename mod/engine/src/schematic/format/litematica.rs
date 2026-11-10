// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/schematic/format/defaults/LitematicaSchematic.java. LGPL-3.0-or-later, see lib.rs.
//
// The Litematica `.litematic` format, version 7 (`DefaultSchematicFormats` refuses the others before this code runs): named regions, each with a
// position, a size (which may be negative), a palette and its blocks packed into a long array with a number of bits per entry that depends on
// the size of the palette. The regions become one `CompositeSchematic`.
//
// Credit: `LitematicaBitArray` is maruohon's, from the Litematica mod (https://github.com/maruohon/litematica), used by Baritone under LGPLv3 with
// the permission of the author. The rest of the reader is rycbar's, written after EmersonDove's work, as upstream credits it.
//
// Quirks that are upstream's and are kept (U32 to U34):
// - the bits per entry come from `Math.log(n) / Math.log(2)` in floating point, rounded up (see `bits_per_block`);
// - `getMinOfSubregion` is `min(position, position + size + 1)`: for a negative size that is one block past the true corner;
// - a region's volume is the product of its three sizes in a Java `int`, which wraps, and a region whose size is not a multiple of anything the
//   bit array was built for reads out of range;
// - the regions are put into the composite in the iteration order of a `java.util.HashMap`, and the composite asks the first entry that has the
//   position, so which of two overlapping regions wins depends on their names' hashes (`Compound::keys_java_order`).
use super::nbt::Compound;
use super::{apply_properties, default_state_or_air, try_parse_identifier, ParseError, ParseLimits};
use crate::schematic::{BlockStates, CompositeSchematic, Schematic, StateId, StaticSchematic, StaticSchematicView};
use std::sync::Arc;

/// The most regions this port reads from one file (D50).
pub const MAX_REGIONS: usize = 4096;

/// `LitematicaSchematic.getRegions`: the regions in `HashMap` order.
/// The regions, in the order `getRegions` gives them: the iteration order of the Java map.
fn get_regions(nbt: &Compound) -> Vec<&Compound> {
    let regions = nbt.get_compound("Regions");
    regions.keys_java_order().into_iter().map(|k| regions.get_compound(k)).collect()
}

/// `LitematicaSchematic.getMinOfSubregion`: the corner on one axis.
/// `getMinOfSubregion`: `Math.min(a, a + b + 1)` for the position `a` and the size `b` on one axis (an int sum, which wraps).
fn get_min_of_subregion(sub_reg: &Compound, axis: &str) -> i32 {
    let a = sub_reg.get_compound("Position").get_int(axis);
    let b = sub_reg.get_compound("Size").get_int(axis);
    a.min(a.wrapping_add(b).wrapping_add(1))
}

/// `LitematicaSchematic.getBlockList`: the palette as states.
/// `LitematicaSchematic.setPropertyValue`: one property of a palette entry.
/// `getBlockList`: the palette as states.
fn get_block_list(palette: &[&Compound], states: &BlockStates) -> Result<Vec<StateId>, ParseError> {
    let mut list = Vec::with_capacity(palette.len());
    for tag in palette {
        // an unparsable name is air, as is a block the registry lacks
        let block = match try_parse_identifier(tag.get_string("Name")) {
            None => states.air(),
            Some((ns, path)) => default_state_or_air(states, &ns, &path),
        };
        let properties = tag.get_compound("Properties");
        let keys = properties.keys_java_order();
        list.push(apply_properties(states, block, keys.into_iter().map(|k| (k, properties.get_string(k))))?);
    }
    Ok(list)
}

/// `LitematicaSchematic.getBitsPerBlock`: the bits of one entry.
/// `getBitsPerBlock`: `(int) Math.max(2, Math.ceil(Math.log(n) / Math.log(2)))`.
///
/// The `ln` calls are here and not in `mth.rs` (which this unit does not own); they are the platform's `f64::ln`. Java's `Math.log` gave the exact
/// `ceil(log2(n))` for every n from 1 to 65536 on OpenJDK 27 on x86-64 (checked 2026-11-09), as this does. It is allowed one unit in the last place of
/// error, so another JVM could differ for an `n` where the quotient lands within an ulp of an integer; the test `bits_per_block_against_exact_log2`
/// lists where the quotient is not exact here.
pub fn bits_per_block(amount_of_block_types: usize) -> i32 {
    let n = amount_of_block_types as f64;
    (n.ln() / 2f64.ln()).ceil().max(2.0) as i32
}

/// `LitematicaBitArray`: `bitsPerEntry` bits per entry, packed little-endian into longs, an entry running over into the next long.
struct LitematicaBitArray<'a> {
    long_array: &'a [i64],
    bits_per_entry: i32,
    max_entry_value: u64,
    array_size: i64,
}

impl<'a> LitematicaBitArray<'a> {
    /// `LitematicaSchematic.LitematicaBitArray.roundUp`: the long count is given by the file.
    /// `LitematicaSchematic.LitematicaBitArray.size`: the entry count.
    /// `LitematicaBitArray.<init>`: `Validate.inclusiveBetween(1, 32, bitsPerEntry)`. The array is the one the file has (upstream allocates a
    /// new one only for null, which `getLongArray` never gives).
    fn new(bits_per_entry: i32, array_size: i64, long_array: &'a [i64]) -> Result<LitematicaBitArray<'a>, ParseError> {
        if !(1..=32).contains(&bits_per_entry) {
            return Err(ParseError(format!("The value {bits_per_entry} is not in the specified inclusive range of 1 to 32")));
        }
        Ok(LitematicaBitArray { long_array, bits_per_entry, max_entry_value: (1u64 << bits_per_entry) - 1, array_size })
    }

    /// `LitematicaSchematic.LitematicaBitArray.getAt`: one entry.
    /// `LitematicaBitArray.getAt`, with `Validate.inclusiveBetween(0, arraySize - 1, index)`. An index into a long the file does not have is
    /// Java's `ArrayIndexOutOfBoundsException`. The result is a Java `int`: with 32 bits per entry it can be negative.
    fn get_at(&self, index: i64) -> Result<i32, ParseError> {
        if index < 0 || index > self.array_size - 1 {
            return Err(ParseError(format!("The value {index} is not in the specified inclusive range of 0 to {}", self.array_size - 1)));
        }
        let bits = i64::from(self.bits_per_entry);
        let start_offset = index * bits;
        let start_arr_index = (start_offset >> 6) as i32;
        let end_arr_index = (((index + 1) * bits - 1) >> 6) as i32;
        let start_bit_offset = (start_offset & 0x3F) as u32;
        let at = |i: i32| -> Result<u64, ParseError> { usize::try_from(i).ok().and_then(|i| self.long_array.get(i)).map(|&l| l as u64).ok_or_else(|| ParseError(format!("Index {i} out of bounds for length {}", self.long_array.len()))) };
        let value = if start_arr_index == end_arr_index {
            at(start_arr_index)? >> start_bit_offset & self.max_entry_value
        } else {
            let end_offset = 64 - start_bit_offset;
            (at(start_arr_index)? >> start_bit_offset | at(end_arr_index)?.wrapping_shl(end_offset)) & self.max_entry_value
        };
        Ok(value as i32)
    }
}

/// `LitematicaSchematic.fillInSchematic`: the regions into a composite.
/// `LitematicaSchematic.writeSubregionIntoSchematic`: one region's cells.
/// `LitematicaSchematic.getBlockState`: a cell of a region.
/// `LitematicaSchematic.getDirect`: the composite's read.
/// `LitematicaSchematic.getMinOfSchematic`: the smallest corner of all regions.
/// `LitematicaSchematic.getVolume`: the cell count of a region.
/// `LitematicaSchematic.<init>(CompoundTag)`: `fillInSchematic` then the composite.
pub(super) fn parse(nbt: &Compound, states: &BlockStates, limits: &ParseLimits) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
    let regions = get_regions(nbt);
    // the composite looks a position up by walking its entries, so every lookup costs this many steps (U41); real files have a handful
    if regions.len() > MAX_REGIONS {
        return Err(ParseError(format!("schematic is too large for this port: {} regions (limit {MAX_REGIONS})", regions.len())));
    }
    let air = states.air();
    let mut composite = CompositeSchematic::new(0, 0, 0, air);

    // `getMinOfSchematic` for each axis
    let min_of = |axis: &str| regions.iter().fold(i32::MAX, |n, r| n.min(get_min_of_subregion(r, axis)));
    let (min_x, min_y, min_z) = (min_of("x"), min_of("y"), min_of("z"));

    let mut cells_in_all: u64 = 0;
    for sub_reg in regions {
        let used_block_types = sub_reg.get_compound_list("BlockStatePalette");
        let block_list = get_block_list(&used_block_types, states)?;

        let bits_per_block = bits_per_block(used_block_types.len());
        // `getVolume`: `Math.abs` of an int product, widened to a long afterwards
        let size = sub_reg.get_compound("Size");
        let region_volume = i64::from(size.get_int("x").wrapping_mul(size.get_int("y")).wrapping_mul(size.get_int("z")).wrapping_abs());
        let bit_array = LitematicaBitArray::new(bits_per_block, region_volume, sub_reg.get_long_array("BlockStates"))?;

        // `writeSubregionIntoSchematic`
        let offset_x = get_min_of_subregion(sub_reg, "x").wrapping_sub(min_x);
        let offset_y = get_min_of_subregion(sub_reg, "y").wrapping_sub(min_y);
        let offset_z = get_min_of_subregion(sub_reg, "z").wrapping_sub(min_z);
        let (size_x, size_y, size_z) = (size.get_int("x").wrapping_abs(), size.get_int("y").wrapping_abs(), size.get_int("z").wrapping_abs());
        // `new BlockState[sizeX][sizeZ][sizeY]`
        if size_x < 0 || size_y < 0 || size_z < 0 {
            return Err(ParseError(format!("negative array size: {}", size_x.min(size_y).min(size_z))));
        }
        let cells = size_x as u64 * size_y as u64 * size_z as u64;
        cells_in_all = cells_in_all.saturating_add(cells);
        if cells_in_all > limits.max_cells {
            return Err(ParseError(format!("schematic is too large for this port: {cells_in_all} blocks (limit {})", limits.max_cells)));
        }
        let mut out = vec![air; cells as usize];
        let mut index: i64 = 0;
        for y in 0..size_y as usize {
            for z in 0..size_z as usize {
                for x in 0..size_x as usize {
                    let at = bit_array.get_at(index)?;
                    let state = usize::try_from(at).ok().and_then(|i| block_list.get(i)).ok_or_else(|| ParseError(format!("Index {at} out of bounds for length {}", block_list.len())))?;
                    out[((x * size_z as usize) + z) * size_y as usize + y] = *state;
                    index += 1;
                }
            }
        }
        composite.put(Arc::new(StaticSchematic::new(size_x, size_y, size_z, out, air)), offset_x, offset_y, offset_z);
    }
    // The cells the regions hold are limited above, but the box that holds the regions is not: two regions of one block a billion blocks apart make
    // a box the builder would walk cell by cell on the game thread (U40). The box is limited like a single schematic is.
    let box_cells = u64::try_from(composite.width_x()).unwrap_or(0).saturating_mul(u64::try_from(composite.height_y()).unwrap_or(0)).saturating_mul(u64::try_from(composite.length_z()).unwrap_or(0));
    if box_cells > limits.max_cells {
        return Err(ParseError(format!("schematic is too large for this port: its regions span a box of {} x {} x {} blocks (limit {})", composite.width_x(), composite.height_y(), composite.length_z(), limits.max_cells)));
    }
    Ok(Arc::new(composite))
}

#[cfg(test)]
mod tests {
    use super::super::nbt::testing::*;
    use super::super::nbt::Tag;
    use super::super::SchematicFormat;
    use super::*;

    /// Packs entries the way Litematica does: `bits` each, little-endian, running over into the next long.
    fn pack(values: &[u32], bits: u32) -> Vec<i64> {
        let total = (values.len() as u64 * u64::from(bits)).div_ceil(64) as usize;
        let mut longs = vec![0u64; total];
        for (i, &v) in values.iter().enumerate() {
            let start = i as u64 * u64::from(bits);
            let (word, off) = ((start >> 6) as usize, (start & 63) as u32);
            longs[word] |= u64::from(v) << off;
            if off + bits > 64 {
                longs[word + 1] |= u64::from(v) >> (64 - off);
            }
        }
        longs.into_iter().map(|l| l as i64).collect()
    }

    fn vec3(x: i32, y: i32, z: i32) -> Tag {
        Tag::Compound(comp(vec![("x", Tag::Int(x)), ("y", Tag::Int(y)), ("z", Tag::Int(z))]))
    }

    fn entry(name: &str, props: &[(&str, &str)]) -> Tag {
        let mut c = comp(vec![("Name", s(name))]);
        if !props.is_empty() {
            c.insert("Properties", Tag::Compound(comp(props.iter().map(|(k, v)| (*k, s(v))).collect())));
        }
        Tag::Compound(c)
    }

    fn region(pos: (i32, i32, i32), size: (i32, i32, i32), palette: Vec<Tag>, values: &[u32]) -> Tag {
        let bits = bits_per_block(palette.len()) as u32;
        Tag::Compound(comp(vec![("Position", vec3(pos.0, pos.1, pos.2)), ("Size", vec3(size.0, size.1, size.2)), ("BlockStatePalette", Tag::List(10, palette)), ("BlockStates", Tag::LongArray(pack(values, bits)))]))
    }

    fn file(version: i32, regions: Vec<(&str, Tag)>) -> Compound {
        comp(vec![("Version", Tag::Int(version)), ("Regions", Tag::Compound(comp(regions)))])
    }

    fn read(c: &Compound, st: &BlockStates) -> Result<Arc<dyn StaticSchematicView>, ParseError> {
        SchematicFormat::Litematica.parse(&gz(c), st, &ParseLimits::default())
    }

    fn pal() -> Vec<Tag> {
        vec![entry("minecraft:air", &[]), entry("minecraft:stone", &[]), entry("minecraft:oak_planks", &[]), entry("minecraft:oak_stairs", &[("facing", "east"), ("half", "top")])]
    }

    #[test]
    fn one_region_reads_in_x_then_z_then_y_order() {
        let st = BlockStates::test_states();
        // 2 x 2 x 2, values 0..8 mod 4: index = (y * 2 + z) * 2 + x
        let values: Vec<u32> = (0..8).map(|i| i % 4).collect();
        let s = read(&file(7, vec![("r", region((0, 0, 0), (2, 2, 2), pal(), &values))]), &st).unwrap();
        assert_eq!((s.width_x(), s.height_y(), s.length_z()), (2, 2, 2));
        let b = |x, y, z| st.block(s.get_direct(x, y, z).unwrap()).to_string();
        assert_eq!(b(0, 0, 0), "air");
        assert_eq!(b(1, 0, 0), "stone");
        assert_eq!(b(0, 0, 1), "oak_planks");
        assert_eq!(st.prop(s.get_direct(1, 0, 1).unwrap(), "facing"), Some("east"));
        assert_eq!(st.prop(s.get_direct(1, 0, 1).unwrap(), "half"), Some("top"));
        assert_eq!(b(0, 1, 0), "air");
        assert_eq!(b(1, 1, 1), "oak_stairs");
    }

    #[test]
    fn two_regions_with_a_negative_size() {
        let st = BlockStates::test_states();
        // A: position 0, size 2x1x2 -> min x = min(0, 0+2+1) = 0.
        // B: position 3, size -2x1x1 -> min x = min(3, 3-2+1) = 2, and the size is 2 wide.
        let a = region((0, 0, 0), (2, 1, 2), pal(), &[1, 1, 1, 1]);
        let b = region((3, 0, 0), (-2, 1, 1), pal(), &[2, 2]);
        let s = read(&file(7, vec![("a", a), ("b", b)]), &st).unwrap();
        // B sits at offset 2 - 0 = 2, so the box is 4 wide
        assert_eq!((s.width_x(), s.height_y(), s.length_z()), (4, 1, 2));
        let n = |x, z| st.block(s.get_direct(x, 0, z).unwrap()).to_string();
        assert_eq!(n(0, 0), "stone");
        assert_eq!(n(1, 1), "stone");
        assert_eq!(n(2, 0), "oak_planks");
        assert_eq!(n(3, 0), "oak_planks");
        // a gap between the entries: z = 1 at x = 2 is in no entry
        assert_eq!(s.get_direct(2, 0, 1), None);
        assert!(!s.in_schematic(2, 0, 1, None));
        assert!(s.in_schematic(3, 0, 0, None));
    }

    #[test]
    fn the_minimum_corner_moves_every_region() {
        let st = BlockStates::test_states();
        // two regions far from the origin: offsets are relative to the smallest min corner
        let a = region((100, 64, -50), (1, 1, 1), pal(), &[1]);
        let b = region((102, 65, -48), (1, 1, 1), pal(), &[2]);
        let s = read(&file(7, vec![("a", a), ("b", b)]), &st).unwrap();
        assert_eq!((s.width_x(), s.height_y(), s.length_z()), (3, 2, 3));
        assert_eq!(st.block(s.get_direct(0, 0, 0).unwrap()), "stone");
        assert_eq!(st.block(s.get_direct(2, 1, 2).unwrap()), "oak_planks");
    }

    #[test]
    fn entries_span_two_longs() {
        let st = BlockStates::test_states();
        // 5 palette entries need 3 bits; entry 21 covers bits 63 to 65, across two longs
        let palette: Vec<Tag> = ["air", "stone", "dirt", "sand", "glass"].iter().map(|n| entry(&format!("minecraft:{n}"), &[])).collect();
        assert_eq!(bits_per_block(5), 3);
        let values: Vec<u32> = (0..40).map(|i| (i * 7 + 3) % 5).collect();
        let longs = pack(&values, 3);
        assert_eq!(longs.len(), 2, "120 bits");
        let s = read(&file(7, vec![("r", region((0, 0, 0), (40, 1, 1), palette.clone(), &values))]), &st).unwrap();
        let names = ["air", "stone", "dirt", "sand", "glass"];
        for (x, v) in values.iter().enumerate() {
            assert_eq!(st.block(s.get_direct(x as i32, 0, 0).unwrap()), names[*v as usize], "x {x}");
        }
        // 5 bits per entry (17 to 32 types are 5 bits): 64 / 5 leaves a remainder of 4 bits, so entry 12 spans longs too
        let big: Vec<Tag> = (0..20).map(|i| if i == 3 { entry("minecraft:dirt", &[]) } else { entry("minecraft:stone", &[]) }).collect();
        assert_eq!(bits_per_block(20), 5);
        let values: Vec<u32> = (0..30).map(|i| if i % 4 == 0 { 3 } else { 19 }).collect();
        let s = read(&file(7, vec![("r", region((0, 0, 0), (30, 1, 1), big, &values))]), &st).unwrap();
        for (x, v) in values.iter().enumerate() {
            assert_eq!(st.block(s.get_direct(x as i32, 0, 0).unwrap()), if *v == 3 { "dirt" } else { "stone" });
        }
    }

    #[test]
    fn an_overlap_is_decided_by_hash_map_order() {
        let st = BlockStates::test_states();
        // the file lists "b" first, but the Java map iterates "a" (bucket 1) before "b" (bucket 2), so "a" is put first and wins
        let a = region((0, 0, 0), (1, 1, 1), pal(), &[1]);
        let b = region((0, 0, 0), (1, 1, 1), pal(), &[2]);
        let s = read(&file(7, vec![("b", b.clone()), ("a", a.clone())]), &st).unwrap();
        assert_eq!(st.block(s.get_direct(0, 0, 0).unwrap()), "stone");
        // and with the names swapped the other region wins
        let s = read(&file(7, vec![("a", b), ("b", a)]), &st).unwrap();
        assert_eq!(st.block(s.get_direct(0, 0, 0).unwrap()), "oak_planks");
    }

    #[test]
    fn palette_names_properties_and_errors() {
        let st = BlockStates::test_states();
        let r = |palette: Vec<Tag>, values: &[u32]| read(&file(7, vec![("r", region((0, 0, 0), (values.len() as i32, 1, 1), palette, values))]), &st);
        // an unknown block, an invalid name and an unknown property are all harmless
        let s = r(vec![entry("minecraft:nope", &[]), entry("Not Valid", &[]), entry("minecraft:stone", &[("bogus", "1")])], &[0, 1, 2]).unwrap();
        assert_eq!(st.block(s.get_direct(0, 0, 0).unwrap()), "air");
        assert_eq!(st.block(s.get_direct(1, 0, 0).unwrap()), "air");
        assert_eq!(st.block(s.get_direct(2, 0, 0).unwrap()), "stone");
        // a value the property never takes is an error
        assert_eq!(r(vec![entry("minecraft:oak_stairs", &[("facing", "up")])], &[0]).err().unwrap().0, "Invalid value for property facing");
        // a palette index past the palette
        assert!(r(vec![entry("minecraft:stone", &[])], &[1]).is_err(), "two bits can say 1 with a one-entry palette");
        // an entry that is not a compound makes the list empty (upstream's getList(key, 10) on another type)
        let mut reg = region((0, 0, 0), (1, 1, 1), pal(), &[0]);
        if let Tag::Compound(c) = &mut reg {
            c.insert("BlockStatePalette", Tag::List(3, vec![Tag::Int(1)]));
        }
        assert!(read(&file(7, vec![("r", reg)]), &st).is_err(), "an empty palette has no state for entry 0");
        // the block states array is too short for the size
        let mut reg = region((0, 0, 0), (100, 1, 1), pal(), &[0]);
        if let Tag::Compound(c) = &mut reg {
            c.insert("BlockStates", Tag::LongArray(vec![0]));
        }
        assert!(read(&file(7, vec![("r", reg)]), &st).is_err());
    }

    #[test]
    fn sizes_volumes_and_limits() {
        let st = BlockStates::test_states();
        // no regions: an empty composite
        let s = read(&file(7, vec![]), &st).unwrap();
        assert_eq!((s.width_x(), s.height_y(), s.length_z()), (0, 0, 0));
        // a zero side: the region is empty and the box stays 0 (the region still counts as an entry)
        let s = read(&file(7, vec![("r", region((0, 0, 0), (0, 3, 3), pal(), &[]))]), &st).unwrap();
        assert_eq!(s.width_x(), 0);
        // the volume is an int product: 65536 * 65536 * 1 wraps to 0, so the bit array allows no entry and the first read fails
        let big = Tag::Compound(comp(vec![("Position", vec3(0, 0, 0)), ("Size", vec3(65536, 65536, 1)), ("BlockStatePalette", Tag::List(10, pal())), ("BlockStates", Tag::LongArray(vec![]))]));
        assert!(read(&file(7, vec![("r", big)]), &st).is_err());
        // i32::MIN as a size: abs() stays negative
        let neg = Tag::Compound(comp(vec![("Position", vec3(0, 0, 0)), ("Size", vec3(i32::MIN, 1, 1)), ("BlockStatePalette", Tag::List(10, pal())), ("BlockStates", Tag::LongArray(vec![]))]));
        assert!(read(&file(7, vec![("r", neg)]), &st).is_err());
        // limits are summed over the regions
        let lim = ParseLimits { max_inflated_bytes: 1 << 20, max_cells: 5 };
        let two = file(7, vec![("a", region((0, 0, 0), (3, 1, 1), pal(), &[1, 1, 1])), ("b", region((5, 0, 0), (3, 1, 1), pal(), &[1, 1, 1]))]);
        assert!(SchematicFormat::Litematica.parse(&gz(&two), &st, &lim).is_err());
        let one = file(7, vec![("a", region((0, 0, 0), (3, 1, 1), pal(), &[1, 1, 1]))]);
        assert!(SchematicFormat::Litematica.parse(&gz(&one), &st, &lim).is_ok());
        // a region that is not a compound counts as an empty one
        let odd = comp(vec![("Version", Tag::Int(7)), ("Regions", Tag::Compound(comp(vec![("x", Tag::Int(1))])))]);
        assert_eq!(read(&odd, &st).unwrap().width_x(), 0);
    }

    #[test]
    fn review_regions_far_apart_make_a_box_that_is_refused() {
        // two one-block regions a billion blocks apart: each is inside the cell limit, the box between them is not (U40)
        let st = BlockStates::test_states();
        let a = region((0, 0, 0), (1, 1, 1), pal(), &[1]);
        let b = region((1_000_000_000, 1_000_000, 1_000_000_000), (1, 1, 1), pal(), &[1]);
        let err = read(&file(7, vec![("a", a.clone()), ("b", b)]), &st).err().unwrap();
        assert!(err.0.contains("span a box"), "{}", err.0);
        // close together is fine, and the box is limited by the same number as the cells
        let c = region((3, 0, 0), (1, 1, 1), pal(), &[1]);
        assert_eq!(read(&file(7, vec![("a", a.clone()), ("c", c.clone())]), &st).unwrap().width_x(), 4);
        let lim = ParseLimits { max_inflated_bytes: 1 << 20, max_cells: 3 };
        assert!(SchematicFormat::Litematica.parse(&gz(&file(7, vec![("a", a), ("c", c)])), &st, &lim).is_err(), "4 x 1 x 1 is more than 3");
    }

    #[test]
    fn review_a_file_of_very_many_regions_is_refused() {
        let st = BlockStates::test_states();
        let many = |n: usize| -> Compound {
            let names: Vec<String> = (0..n).map(|i| format!("r{i}")).collect();
            file(7, names.iter().map(|k| (k.as_str(), region((0, 0, 0), (0, 0, 0), pal(), &[]))).collect())
        };
        assert!(read(&many(MAX_REGIONS), &st).is_ok());
        let err = read(&many(MAX_REGIONS + 1), &st).err().unwrap();
        assert!(err.0.contains("regions"), "{}", err.0);
    }

    #[test]
    fn the_bit_array_reads_and_validates() {
        let longs = pack(&[5, 6, 7, 1, 2, 3, 4, 0, 1], 3);
        let a = LitematicaBitArray::new(3, 9, &longs).unwrap();
        for (i, v) in [5, 6, 7, 1, 2, 3, 4, 0, 1].iter().enumerate() {
            assert_eq!(a.get_at(i as i64).unwrap(), *v);
        }
        assert!(a.get_at(-1).is_err() && a.get_at(9).is_err());
        assert!(LitematicaBitArray::new(0, 1, &[]).is_err() && LitematicaBitArray::new(33, 1, &[]).is_err());
        // 32 bits per entry can give a negative int
        let l = pack(&[0xFFFF_FFFF, 7], 32);
        let a = LitematicaBitArray::new(32, 2, &l).unwrap();
        assert_eq!(a.get_at(0).unwrap(), -1);
        assert_eq!(a.get_at(1).unwrap(), 7);
        // too short a long array
        let a = LitematicaBitArray::new(3, 100, &longs).unwrap();
        assert!(a.get_at(99).is_err());
    }

    /// `ceil(log2(n))` in integers.
    fn exact_ceil_log2(n: usize) -> u32 {
        if n <= 1 {
            0
        } else {
            usize::BITS - (n - 1).leading_zeros()
        }
    }

    #[test]
    fn bits_per_block_against_exact_log2() {
        assert_eq!(bits_per_block(0), 2);
        assert_eq!(bits_per_block(1), 2);
        assert_eq!(bits_per_block(2), 2);
        assert_eq!(bits_per_block(5), 3);
        let mut different = vec![];
        for n in 1..=65536usize {
            let exact = exact_ceil_log2(n).max(2) as i32;
            if bits_per_block(n) != exact {
                different.push((n, bits_per_block(n), exact));
            }
        }
        // on this platform's `ln`: the floating-point formula and the integer one agree for every palette size from 1 to 65536
        assert_eq!(different, vec![], "n, float formula, exact: the places where upstream's bits per entry is not the true minimum");
    }

    #[test]
    fn fuzz_never_panics() {
        let st = BlockStates::test_states();
        let lim = ParseLimits { max_inflated_bytes: 1 << 20, max_cells: 1 << 14 };
        let base = root(&file(7, vec![("a", region((0, 0, 0), (3, 2, 2), pal(), &[0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3])), ("b", region((5, 0, 0), (-2, 1, 1), pal(), &[2, 2]))]));
        super::super::tests_support::fuzz(SchematicFormat::Litematica, &base, &st, &lim, 100_000, 0x1234_5678_9abc_def1);
    }
}
