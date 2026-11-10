// Written for this project from the public list of Minecraft 1.12 string ids (the Minecraft Wiki's "Java Edition data values" page): facts, not
// code. Neither Horizon's table nor anyone's source was looked at. LGPL-3.0-or-later, see lib.rs.
//
// What `net.minecraft.util.datafix.fixes.ItemIdFix.getItem(int)` gives `MCEditSchematic` for a numeric block id: the pre-1.13 name of the item
// with that id, such as `minecraft:grass` or `minecraft:stonebrick`. The id is an item id, and ids 1 to 255 are the items of blocks, so a block
// that has no item of its own (water, lava, fire, wheat, a sign, a bed, a door, a repeater, a double slab...) is not in the table. An id that is not
// in it gives `minecraft:air`: upstream's `getItem` falls back to the name at id 0 (this is from memory of the game's source, and is the one fact
// here that could not be checked against it; see D47).
//
// Most of these names no longer exist in the 1.13 registry (`grass`, `planks`, `log`, `stonebrick`, `wool`...), so `MCEditSchematic` turns the cell
// into air: that is upstream's behaviour and U31 keeps it. Only the names that survived the flattening place a block: stone, dirt, cobblestone,
// bedrock, sand, gravel, the ores, glass, dispenser, and so on.

/// The air name, for id 0 and for an id with no entry.
const AIR: &str = "minecraft:air";

/// Sorted by id.
static TABLE: &[(u16, &str)] = &[
    (1, "minecraft:stone"),
    (2, "minecraft:grass"),
    (3, "minecraft:dirt"),
    (4, "minecraft:cobblestone"),
    (5, "minecraft:planks"),
    (6, "minecraft:sapling"),
    (7, "minecraft:bedrock"),
    (12, "minecraft:sand"),
    (13, "minecraft:gravel"),
    (14, "minecraft:gold_ore"),
    (15, "minecraft:iron_ore"),
    (16, "minecraft:coal_ore"),
    (17, "minecraft:log"),
    (18, "minecraft:leaves"),
    (19, "minecraft:sponge"),
    (20, "minecraft:glass"),
    (21, "minecraft:lapis_ore"),
    (22, "minecraft:lapis_block"),
    (23, "minecraft:dispenser"),
    (24, "minecraft:sandstone"),
    (25, "minecraft:noteblock"),
    (27, "minecraft:golden_rail"),
    (28, "minecraft:detector_rail"),
    (29, "minecraft:sticky_piston"),
    (30, "minecraft:web"),
    (31, "minecraft:tallgrass"),
    (32, "minecraft:deadbush"),
    (33, "minecraft:piston"),
    (35, "minecraft:wool"),
    (37, "minecraft:yellow_flower"),
    (38, "minecraft:red_flower"),
    (39, "minecraft:brown_mushroom"),
    (40, "minecraft:red_mushroom"),
    (41, "minecraft:gold_block"),
    (42, "minecraft:iron_block"),
    (44, "minecraft:stone_slab"),
    (45, "minecraft:brick_block"),
    (46, "minecraft:tnt"),
    (47, "minecraft:bookshelf"),
    (48, "minecraft:mossy_cobblestone"),
    (49, "minecraft:obsidian"),
    (50, "minecraft:torch"),
    (52, "minecraft:mob_spawner"),
    (53, "minecraft:oak_stairs"),
    (54, "minecraft:chest"),
    (56, "minecraft:diamond_ore"),
    (57, "minecraft:diamond_block"),
    (58, "minecraft:crafting_table"),
    (60, "minecraft:farmland"),
    (61, "minecraft:furnace"),
    (65, "minecraft:ladder"),
    (66, "minecraft:rail"),
    (67, "minecraft:stone_stairs"),
    (69, "minecraft:lever"),
    (70, "minecraft:stone_pressure_plate"),
    (72, "minecraft:wooden_pressure_plate"),
    (73, "minecraft:redstone_ore"),
    (76, "minecraft:redstone_torch"),
    (77, "minecraft:stone_button"),
    (78, "minecraft:snow_layer"),
    (79, "minecraft:ice"),
    (80, "minecraft:snow"),
    (81, "minecraft:cactus"),
    (82, "minecraft:clay"),
    (84, "minecraft:jukebox"),
    (85, "minecraft:fence"),
    (86, "minecraft:pumpkin"),
    (87, "minecraft:netherrack"),
    (88, "minecraft:soul_sand"),
    (89, "minecraft:glowstone"),
    (91, "minecraft:lit_pumpkin"),
    (95, "minecraft:stained_glass"),
    (96, "minecraft:trapdoor"),
    (97, "minecraft:monster_egg"),
    (98, "minecraft:stonebrick"),
    (99, "minecraft:brown_mushroom_block"),
    (100, "minecraft:red_mushroom_block"),
    (101, "minecraft:iron_bars"),
    (102, "minecraft:glass_pane"),
    (103, "minecraft:melon_block"),
    (106, "minecraft:vine"),
    (107, "minecraft:fence_gate"),
    (108, "minecraft:brick_stairs"),
    (109, "minecraft:stone_brick_stairs"),
    (110, "minecraft:mycelium"),
    (111, "minecraft:waterlily"),
    (112, "minecraft:nether_brick"),
    (113, "minecraft:nether_brick_fence"),
    (114, "minecraft:nether_brick_stairs"),
    (116, "minecraft:enchanting_table"),
    (120, "minecraft:end_portal_frame"),
    (121, "minecraft:end_stone"),
    (122, "minecraft:dragon_egg"),
    (123, "minecraft:redstone_lamp"),
    (126, "minecraft:wooden_slab"),
    (128, "minecraft:sandstone_stairs"),
    (129, "minecraft:emerald_ore"),
    (130, "minecraft:ender_chest"),
    (131, "minecraft:tripwire_hook"),
    (133, "minecraft:emerald_block"),
    (134, "minecraft:spruce_stairs"),
    (135, "minecraft:birch_stairs"),
    (136, "minecraft:jungle_stairs"),
    (137, "minecraft:command_block"),
    (138, "minecraft:beacon"),
    (139, "minecraft:cobblestone_wall"),
    (143, "minecraft:wooden_button"),
    (145, "minecraft:anvil"),
    (146, "minecraft:trapped_chest"),
    (147, "minecraft:light_weighted_pressure_plate"),
    (148, "minecraft:heavy_weighted_pressure_plate"),
    (151, "minecraft:daylight_detector"),
    (152, "minecraft:redstone_block"),
    (153, "minecraft:quartz_ore"),
    (154, "minecraft:hopper"),
    (155, "minecraft:quartz_block"),
    (156, "minecraft:quartz_stairs"),
    (157, "minecraft:activator_rail"),
    (158, "minecraft:dropper"),
    (159, "minecraft:stained_hardened_clay"),
    (160, "minecraft:stained_glass_pane"),
    (161, "minecraft:leaves2"),
    (162, "minecraft:log2"),
    (163, "minecraft:acacia_stairs"),
    (164, "minecraft:dark_oak_stairs"),
    (165, "minecraft:slime"),
    (166, "minecraft:barrier"),
    (167, "minecraft:iron_trapdoor"),
    (168, "minecraft:prismarine"),
    (169, "minecraft:sea_lantern"),
    (170, "minecraft:hay_block"),
    (171, "minecraft:carpet"),
    (172, "minecraft:hardened_clay"),
    (173, "minecraft:coal_block"),
    (174, "minecraft:packed_ice"),
    (175, "minecraft:double_plant"),
    (179, "minecraft:red_sandstone"),
    (180, "minecraft:red_sandstone_stairs"),
    (182, "minecraft:stone_slab2"),
    (183, "minecraft:spruce_fence_gate"),
    (184, "minecraft:birch_fence_gate"),
    (185, "minecraft:jungle_fence_gate"),
    (186, "minecraft:dark_oak_fence_gate"),
    (187, "minecraft:acacia_fence_gate"),
    (188, "minecraft:spruce_fence"),
    (189, "minecraft:birch_fence"),
    (190, "minecraft:jungle_fence"),
    (191, "minecraft:dark_oak_fence"),
    (192, "minecraft:acacia_fence"),
    (198, "minecraft:end_rod"),
    (199, "minecraft:chorus_plant"),
    (200, "minecraft:chorus_flower"),
    (201, "minecraft:purpur_block"),
    (202, "minecraft:purpur_pillar"),
    (203, "minecraft:purpur_stairs"),
    (205, "minecraft:purpur_slab"),
    (206, "minecraft:end_bricks"),
    (208, "minecraft:grass_path"),
    (210, "minecraft:repeating_command_block"),
    (211, "minecraft:chain_command_block"),
    (212, "minecraft:frosted_ice"),
    (213, "minecraft:magma"),
    (214, "minecraft:nether_wart_block"),
    (215, "minecraft:red_nether_brick"),
    (216, "minecraft:bone_block"),
    (217, "minecraft:structure_void"),
    (218, "minecraft:observer"),
    (219, "minecraft:white_shulker_box"),
    (220, "minecraft:orange_shulker_box"),
    (221, "minecraft:magenta_shulker_box"),
    (222, "minecraft:light_blue_shulker_box"),
    (223, "minecraft:yellow_shulker_box"),
    (224, "minecraft:lime_shulker_box"),
    (225, "minecraft:pink_shulker_box"),
    (226, "minecraft:gray_shulker_box"),
    (227, "minecraft:silver_shulker_box"),
    (228, "minecraft:cyan_shulker_box"),
    (229, "minecraft:purple_shulker_box"),
    (230, "minecraft:blue_shulker_box"),
    (231, "minecraft:brown_shulker_box"),
    (232, "minecraft:green_shulker_box"),
    (233, "minecraft:red_shulker_box"),
    (234, "minecraft:black_shulker_box"),
    (235, "minecraft:white_glazed_terracotta"),
    (236, "minecraft:orange_glazed_terracotta"),
    (237, "minecraft:magenta_glazed_terracotta"),
    (238, "minecraft:light_blue_glazed_terracotta"),
    (239, "minecraft:yellow_glazed_terracotta"),
    (240, "minecraft:lime_glazed_terracotta"),
    (241, "minecraft:pink_glazed_terracotta"),
    (242, "minecraft:gray_glazed_terracotta"),
    (243, "minecraft:silver_glazed_terracotta"),
    (244, "minecraft:cyan_glazed_terracotta"),
    (245, "minecraft:purple_glazed_terracotta"),
    (246, "minecraft:blue_glazed_terracotta"),
    (247, "minecraft:brown_glazed_terracotta"),
    (248, "minecraft:green_glazed_terracotta"),
    (249, "minecraft:red_glazed_terracotta"),
    (250, "minecraft:black_glazed_terracotta"),
    (251, "minecraft:concrete"),
    (252, "minecraft:concrete_powder"),
    (255, "minecraft:structure_block"),
];

/// Like `ItemIdFix.getItem`: the pre-1.13 item name of an id, or `minecraft:air` when the table has none.
pub fn item_name(id: i32) -> &'static str {
    match u16::try_from(id).ok().and_then(|id| TABLE.binary_search_by_key(&id, |&(i, _)| i).ok()) {
        Some(at) => TABLE[at].1,
        None => AIR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_has_no_duplicates() {
        assert!(TABLE.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn known_ids() {
        assert_eq!(item_name(0), "minecraft:air");
        assert_eq!(item_name(1), "minecraft:stone");
        assert_eq!(item_name(2), "minecraft:grass");
        assert_eq!(item_name(4), "minecraft:cobblestone");
        assert_eq!(item_name(98), "minecraft:stonebrick");
        assert_eq!(item_name(219), "minecraft:white_shulker_box");
        assert_eq!(item_name(234), "minecraft:black_shulker_box");
        assert_eq!(item_name(235), "minecraft:white_glazed_terracotta");
        assert_eq!(item_name(250), "minecraft:black_glazed_terracotta");
        assert_eq!(item_name(255), "minecraft:structure_block");
    }

    #[test]
    fn missing_and_out_of_range_ids_are_air() {
        for id in [8, 9, 11, 26, 36, 43, 59, 253, 254, 256, 4095, -1, 70000] {
            assert_eq!(item_name(id), "minecraft:air", "{id}");
        }
    }
}
