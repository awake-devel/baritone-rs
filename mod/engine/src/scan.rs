// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/cache/FasterWorldScanner.java (scanChunkRadius, getChunkRange, scanChunksInternal,
// scanChunkInternal, collectChunkSections, visitSection). LGPL-3.0-or-later, see lib.rs.
//
// Finds the blocks a filter matches in the chunks that are loaded: `BaritoneProvider.getWorldScanner()` is `FasterWorldScanner`, and that is the
// scanner `MineProcess` and `MineCommand` call. The answer is upstream's: a square spiral of chunks (`getChunkRange`), inside a chunk the sections
// from the player's outwards, inside a section `y`, `z`, `x`, and never more than `max` hits in all. `MineProcess.prune` sorts it afterwards.
use crate::blockmeta::BlockOptionalMetaLookup;
use crate::pos::Pos;
use crate::world::WorldView;

/// `FasterWorldScanner.getChunkRange`: the chunks around `(center_x, center_z)` for rings 0 up to `chunk_radius` (exclusive), as a square spiral
/// with the closest first. The order of a ring is upstream's: north row from the west side to the middle and back out, and so on.
pub fn get_chunk_range(center_x: i32, center_z: i32, chunk_radius: i32) -> Vec<(i32, i32)> {
    let mut chunks = vec![(center_x, center_z)];
    let mut i = 1;
    while i < chunk_radius {
        for j in 0..=i {
            chunks.push((center_x.wrapping_sub(j), center_z.wrapping_sub(i)));
            if j != 0 {
                chunks.push((center_x.wrapping_add(j), center_z.wrapping_sub(i)));
                chunks.push((center_x.wrapping_sub(j), center_z.wrapping_add(i)));
            }
            chunks.push((center_x.wrapping_add(j), center_z.wrapping_add(i)));
            if j != i {
                chunks.push((center_x.wrapping_sub(i), center_z.wrapping_sub(j)));
                chunks.push((center_x.wrapping_add(i), center_z.wrapping_sub(j)));
                if j != 0 {
                    chunks.push((center_x.wrapping_sub(i), center_z.wrapping_add(j)));
                    chunks.push((center_x.wrapping_add(i), center_z.wrapping_add(j)));
                }
            }
        }
        i += 1;
    }
    chunks
}

/// `FasterWorldScanner.visitSection`: adds the matches of one section. A section that holds one block state everywhere (upstream: a
/// `SingleValuePalette`) is walked `x`, `y`, `z`, with `max` checked only in the innermost loop; any other section `y`, `z`, `x`, which is the order of the
/// packed storage. The engine cannot see the palette, so "one state everywhere" is read off the cells (docs/deviations.md D39).
#[allow(clippy::too_many_arguments)]
fn visit_section<W: WorldView>(world: &W, lookup: &BlockOptionalMetaLookup, blocks: &mut Vec<Pos>, chunk_x: i32, section_y: i32, chunk_z: i32, max: usize) {
    let first = world.state_id(chunk_x, section_y, chunk_z);
    let mut uniform = false;
    if lookup.has(first) {
        uniform = (0..16).all(|y| (0..16).all(|z| (0..16).all(|x| world.state_id(chunk_x + x, section_y + y, chunk_z + z) == first)));
    }
    if uniform {
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    if blocks.len() >= max {
                        break;
                    }
                    blocks.push(Pos::new(chunk_x + x, section_y + y, chunk_z + z));
                }
            }
        }
        return;
    }
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                if lookup.has(world.state_id(chunk_x + x, section_y + y, chunk_z + z)) {
                    if blocks.len() >= max {
                        return;
                    }
                    blocks.push(Pos::new(chunk_x + x, section_y + y, chunk_z + z));
                }
            }
        }
    }
}

/// `FasterWorldScanner.collectChunkSections`: (with `scanChunkInternal`, whose unloaded-chunk case is here too) the matches of one chunk, at most `max`, the sections from the player's
/// outwards: the player's, the one below, the one above that, and so on. Nothing when the chunk is not loaded.
fn scan_chunk_internal<W: WorldView>(world: &W, lookup: &BlockOptionalMetaLookup, chunk: (i32, i32), player_section: i32, max: usize) -> Vec<Pos> {
    let mut blocks = vec![];
    if !world.chunk_loaded(chunk.0, chunk.1) {
        return blocks;
    }
    let chunk_x = chunk.0.wrapping_shl(4);
    let chunk_z = chunk.1.wrapping_shl(4);
    let min_y = world.min_y();
    let l = world.height() / 16;
    let mut i = player_section.wrapping_sub(1);
    let mut j = player_section;
    while (i >= 0 || j < l) && blocks.len() < max {
        if (0..l).contains(&j) && !world.section_has_only_air(chunk.0, chunk.1, j) {
            visit_section(world, lookup, &mut blocks, chunk_x, min_y + j * 16, chunk_z, max);
        }
        if (0..l).contains(&i) && blocks.len() < max && !world.section_has_only_air(chunk.0, chunk.1, i) {
            visit_section(world, lookup, &mut blocks, chunk_x, min_y + i * 16, chunk_z, max);
        }
        j += 1;
        i -= 1;
    }
    blocks
}

/// `FasterWorldScanner.scanChunkRadius`: the blocks the filter matches in the loaded chunks of the `max_search_radius` rings around `feet`
/// (`getChunkRange`), the first `max` of them in that order (a negative `max` is no limit). `yLevelThreshold` is not used, as upstream.
/// Upstream scans the chunks in parallel and keeps the order; this walks them in order and stops once `max` hits are in.
pub fn scan_chunk_radius<W: WorldView>(world: &W, feet: Pos, filter: &BlockOptionalMetaLookup, max: i32, max_search_radius: i32) -> Vec<Pos> {
    let mut res: Vec<Pos> = vec![];
    if filter.blocks().is_empty() || max_search_radius < 0 {
        return res;
    }
    let limit = if max < 0 { usize::MAX } else { max as usize };
    let player_section = feet.y.wrapping_sub(world.min_y()) >> 4;
    for chunk in get_chunk_range(feet.x >> 4, feet.z >> 4, max_search_radius) {
        if res.len() >= limit {
            break;
        }
        // one chunk contributes at most what is still missing: the first `limit` of the whole list are the same
        res.extend(scan_chunk_internal(world, filter, chunk, player_section, limit - res.len()));
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockmeta::{test_index, BlockOptionalMetaLookup};
    use crate::testing::{TestWorld, ORE, STONE};

    fn ore_filter() -> BlockOptionalMetaLookup {
        // state 2 is `diamond_ore` in the test index, and `ORE` in the test world
        BlockOptionalMetaLookup::from_names(&["diamond_ore"], &test_index()).unwrap()
    }

    #[test]
    fn the_chunk_range_is_the_square_spiral() {
        assert_eq!(get_chunk_range(0, 0, 1), vec![(0, 0)]);
        // ring 1: j = 0 gives (0,-1), (0,1), then (-1,0), (1,0); j = 1 gives (-1,-1), (1,-1), (-1,1), (1,1)
        assert_eq!(get_chunk_range(0, 0, 2), vec![(0, 0), (0, -1), (0, 1), (-1, 0), (1, 0), (-1, -1), (1, -1), (-1, 1), (1, 1)]);
        // radius 32 is 31 rings: a 63 x 63 square, every chunk once
        let r = get_chunk_range(5, -7, 32);
        assert_eq!(r.len(), 63 * 63);
        let mut s = r.clone();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), r.len());
        assert!(r.iter().all(|(x, z)| (x - 5).abs() < 32 && (z + 7).abs() < 32));
    }

    #[test]
    fn it_returns_at_most_max_and_the_north_chunk_comes_before_the_west() {
        let mut w = TestWorld::flat(48, 48);
        let f = ore_filter();
        let feet = Pos::new(24, 1, 24);
        w.set(5, 5, 24, ORE); // chunk (0, 1), west
        w.set(24, 5, 5, ORE); // chunk (1, 0), north
        w.set(24, 6, 6, ORE);
        w.set(24, 7, 7, ORE);
        let got = scan_chunk_radius(&w, feet, &f, 1, 32);
        assert_eq!(got, vec![Pos::new(24, 5, 5)]);
        assert_eq!(scan_chunk_radius(&w, feet, &f, 3, 32).len(), 3);
        assert_eq!(scan_chunk_radius(&w, feet, &f, -1, 32).len(), 4);
        assert!(scan_chunk_radius(&w, feet, &f, 0, 32).is_empty());
    }

    #[test]
    fn inside_a_chunk_it_is_y_then_z_then_x_and_the_players_section_first() {
        let mut w = TestWorld::tall(16, 16, 48);
        let f = ore_filter();
        w.set(14, 5, 4, ORE);
        w.set(2, 5, 3, ORE);
        w.set(9, 2, 9, ORE);
        w.set(1, 20, 1, ORE); // section 1
        w.set(1, 40, 1, ORE); // section 2
        w.set(1, 3, 1, ORE); // section 0
                             // the player in section 1: 1, then 0 (below), then 2 (above that)
        let got = scan_chunk_radius(&w, Pos::new(8, 20, 8), &f, 100, 32);
        assert_eq!(got, vec![Pos::new(1, 20, 1), Pos::new(9, 2, 9), Pos::new(1, 3, 1), Pos::new(2, 5, 3), Pos::new(14, 5, 4), Pos::new(1, 40, 1)]);
    }

    #[test]
    fn a_section_of_one_state_is_walked_x_then_y_then_z() {
        let mut w = TestWorld::flat(16, 16);
        let f = ore_filter();
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    w.set(x, y, z, ORE);
                }
            }
        }
        let got = scan_chunk_radius(&w, Pos::new(8, 1, 8), &f, 20, 32);
        assert_eq!(got.len(), 20);
        assert_eq!(got[0], Pos::new(0, 0, 0));
        assert_eq!(got[15], Pos::new(0, 0, 15));
        assert_eq!(got[16], Pos::new(0, 1, 0));
    }

    #[test]
    fn an_unloaded_chunk_gives_nothing_but_the_scan_goes_on_past_it() {
        let mut w = TestWorld::flat(64, 16);
        w.set(40, 2, 3, ORE);
        w.loaded_below_x = 32;
        let f = ore_filter();
        // chunk x = 2 (40 >> 4) is not loaded: the ore is not found
        assert!(scan_chunk_radius(&w, Pos::new(8, 1, 8), &f, 10, 32).is_empty());
        // a chunk further out than an unloaded one is still read (upstream has no "ring of unloaded chunks" stop)
        w.loaded_below_x = i32::MAX;
        w.set(56, 2, 3, ORE);
        assert_eq!(scan_chunk_radius(&w, Pos::new(8, 1, 8), &f, 10, 32).len(), 2);
    }

    #[test]
    fn an_empty_filter_scans_nothing() {
        let w = TestWorld::flat(32, 32);
        assert!(scan_chunk_radius(&w, Pos::new(8, 1, 8), &BlockOptionalMetaLookup::new(vec![]), 10, 32).is_empty());
    }

    #[test]
    fn other_blocks_do_not_match() {
        let mut w = TestWorld::flat(16, 16);
        w.set(3, 3, 3, STONE);
        assert!(scan_chunk_radius(&w, Pos::new(8, 1, 8), &ore_filter(), 10, 32).is_empty());
    }
}
