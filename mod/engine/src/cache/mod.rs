// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/cache/ (CachedChunk, CachedRegion, CachedWorld, ChunkPacker, CacheFiles,
// WorldData and the part of WorldProvider that names the folders). LGPL-3.0-or-later, see lib.rs.
//
// The persistent chunk cache: every chunk the client loads is packed to two bits per block (air, water, avoid, solid), plus the list of the blocks
// worth remembering (chests, spawners, portals, ...) and the top block of each column. Packed chunks live in regions of 32 x 32 chunks, are saved to
// gzip files in upstream's format, and are read back by the search for the chunks that are not loaded (`BlockStateInterface.getUncached`) and by
// `mine` / `goto <block>` for the blocks of [`BLOCKS_TO_KEEP_TRACK_OF`].
//
// What is new structure: the engine has no game, so a loaded chunk reaches [`chunk::pack`] through the trait [`chunk::PackSource`], block names and
// block states meet in a [`palette::CacheEnv`] the glue builds from its block table, and the threads upstream starts in its constructors are in
// [`worker`]. The decided fixes D1 to D4 (docs/deviations.md) are in [`chunk`] and [`region`].
pub mod chunk;
pub mod gzip;
pub mod palette;
pub mod region;
#[cfg(test)]
pub mod testkit;
pub mod view;
pub mod worker;
pub mod world;

pub use chunk::{CachedChunk, Dimension, PackSource, PathingBlockType};
pub use palette::CacheEnv;
pub use region::CachedRegion;
pub use view::{CacheReader, WithCache};
pub use world::{CacheSettings, CachedWorld};

/// `CachedChunk.BLOCKS_TO_KEEP_TRACK_OF`, by registry path (`BlockUtils.blockToString`).
pub const BLOCKS_TO_KEEP_TRACK_OF: &[&str] = &[
    "ender_chest",
    "furnace",
    "chest",
    "trapped_chest",
    "end_portal",
    "end_portal_frame",
    "spawner",
    "barrier",
    "observer",
    "white_shulker_box",
    "orange_shulker_box",
    "magenta_shulker_box",
    "light_blue_shulker_box",
    "yellow_shulker_box",
    "lime_shulker_box",
    "pink_shulker_box",
    "gray_shulker_box",
    "light_gray_shulker_box",
    "cyan_shulker_box",
    "purple_shulker_box",
    "blue_shulker_box",
    "brown_shulker_box",
    "green_shulker_box",
    "red_shulker_box",
    "black_shulker_box",
    "nether_portal",
    "hopper",
    "beacon",
    "brewing_stand",
    "creeper_head",
    "creeper_wall_head",
    "dragon_head",
    "dragon_wall_head",
    "player_head",
    "player_wall_head",
    "zombie_head",
    "zombie_wall_head",
    "skeleton_skull",
    "skeleton_wall_skull",
    "wither_skeleton_skull",
    "wither_skeleton_wall_skull",
    "enchanting_table",
    "anvil",
    "white_bed",
    "orange_bed",
    "magenta_bed",
    "light_blue_bed",
    "yellow_bed",
    "lime_bed",
    "pink_bed",
    "gray_bed",
    "light_gray_bed",
    "cyan_bed",
    "purple_bed",
    "blue_bed",
    "brown_bed",
    "green_bed",
    "red_bed",
    "black_bed",
    "dragon_egg",
    "jukebox",
    "end_gateway",
    "cobweb",
    "nether_wart",
    "ladder",
    "vine",
];

/// Whether `BLOCKS_TO_KEEP_TRACK_OF` holds the block of this registry path.
pub fn keeps_track_of(name: &str) -> bool {
    BLOCKS_TO_KEEP_TRACK_OF.contains(&name)
}

/// A name for logs that does not leak an address: the cache folder of a server is named by this hash of the address, not by the address.
pub fn server_key(address: &str) -> String {
    // FNV-1a over the lower-cased address, twice with different seeds: 128 bits of a hash that is not secret, only not readable.
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x8422_2325_cbf2_9ce4;
    for byte in address.trim().to_lowercase().bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(7);
    }
    format!("srv-{a:016x}{b:016x}")
}

/// A folder name made of what the text holds that is safe on every file system: letters, digits, `-`, `_` and `.` stay, anything else becomes `_`.
/// A name that is empty, or only dots, becomes `_`. A path separator can never survive, so a name cannot leave its folder.
pub fn sanitize_component(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    if s.is_empty() || s.chars().all(|c| c == '.') {
        "_".to_string()
    } else {
        s.chars().take(100).collect()
    }
}

/// [`sanitize_component`], but two names that it maps to the same text stay two folders: a name that lost anything (a space, a non-ASCII letter, a cut at 100
/// characters) gets `.h` and 16 hex digits of a hash of the raw name; a name that survived whole is unchanged, so upstream's folder names are kept (D43).
pub fn unique_component(name: &str) -> String {
    let s = sanitize_component(name);
    if s == name {
        return s;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        h = (h ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{s}.h{h:016x}")
}

/// The folder of a single player world, by its name: `sp-` and [`unique_component`].
pub fn singleplayer_key(world_name: &str) -> String {
    format!("sp-{}", unique_component(world_name))
}

/// `WorldProvider.getWorldDataDirectory`: `<namespace>/<path>_<logical height>` under the folder of the world or server.
pub fn dimension_folder(dimension_id: &str, logical_height: i32) -> std::path::PathBuf {
    let (ns, path) = dimension_id.split_once(':').unwrap_or(("minecraft", dimension_id));
    std::path::PathBuf::from(unique_component(ns)).join(format!("{}_{logical_height}", unique_component(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_names_cannot_leave_their_folder() {
        assert_eq!(sanitize_component("../../etc"), ".._.._etc");
        assert_eq!(sanitize_component(".."), "_");
        assert_eq!(sanitize_component(""), "_");
        assert_eq!(sanitize_component("a/b\\c:d"), "a_b_c_d");
        for part in dimension_folder("minecraft:../../x", 384).components() {
            assert!(matches!(part, std::path::Component::Normal(_)), "{part:?}");
        }
        assert_eq!(dimension_folder("minecraft:overworld", 384), std::path::PathBuf::from("minecraft/overworld_384"));
    }

    #[test]
    fn names_that_sanitise_alike_get_folders_of_their_own() {
        // review finding 1: "My World" and "My_World" shared a folder, and so did any two non-ASCII names of the same length
        assert_ne!(singleplayer_key("My World"), singleplayer_key("My_World"));
        assert_ne!(singleplayer_key("\u{4e16}\u{754c}"), singleplayer_key("\u{4e2d}\u{6587}"));
        assert_eq!(singleplayer_key("My_World"), "sp-My_World", "a name that survives whole keeps its plain folder");
        assert_eq!(singleplayer_key("My World"), singleplayer_key("My World"));
        assert_ne!(dimension_folder("mod:a b", 384), dimension_folder("mod:a_b", 384));
        let long = "x".repeat(150);
        assert_ne!(unique_component(&long), unique_component(&format!("{long}y")));
        for part in std::path::Path::new(&singleplayer_key("../../x y")).components() {
            assert!(matches!(part, std::path::Component::Normal(_)));
        }
    }

    #[test]
    fn a_server_key_hides_the_address_and_ignores_case() {
        let k = server_key("Example.Org:25565");
        assert_eq!(k, server_key("example.org:25565 "));
        assert_ne!(k, server_key("example.org:25566"));
        assert!(!k.contains("example") && k.starts_with("srv-") && k.len() == 4 + 32);
    }

    #[test]
    fn the_list_is_upstreams() {
        assert_eq!(BLOCKS_TO_KEEP_TRACK_OF.len(), 66);
        assert!(keeps_track_of("chest") && keeps_track_of("vine") && !keeps_track_of("stone"));
    }
}
