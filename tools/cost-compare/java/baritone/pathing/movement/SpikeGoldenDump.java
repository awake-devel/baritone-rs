/*
 * baritone-rs test tool: dumps game facts and upstream's own costs. Copy it into a checkout of upstream.
 * Licensed like the code it exercises: GNU LGPL, version 3 or (at your option) any later version.
 */
package baritone.pathing.movement;

import baritone.Baritone;
import baritone.api.Settings;
import baritone.pathing.movement.movements.MovementDescend;
import baritone.pathing.movement.movements.MovementTraverse;
import baritone.utils.BlockStateInterface;
import baritone.utils.ToolSet;
import baritone.utils.pathing.BetterWorldBorder;
import baritone.utils.pathing.MutableMoveResult;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import net.minecraft.SharedConstants;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.tags.TagKey;
import net.minecraft.tags.TagLoader;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.*;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.material.*;
import net.minecraft.world.level.pathfinder.PathComputationType;
import org.junit.Test;

import java.io.BufferedWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

public class SpikeGoldenDump {
    static final int W = 5, H = 16, D = 5;
    static BlockState air;

    static final class Grid extends BlockStateInterface {
        final BlockState[] cells;

        Grid(BetterWorldBorder border, BlockState[] cells) {
            super(border, 0, H);
            this.cells = cells;
        }

        @Override
        protected BlockState getUncached(int x, int y, int z) {
            if (x < 0 || x >= W || z < 0 || z >= D) {
                return air;
            }
            return cells[(y * D + z) * W + x];
        }

        @Override
        public boolean isLoaded(int x, int z) {
            return true;
        }

        @Override
        public boolean worldContainsLoadedChunk(int x, int z) {
            return true;
        }
    }

    static final class Tools extends ToolSet {
        final ItemStack hand;

        Tools(ItemStack hand) {
            super(null);
            this.hand = hand;
        }

        @Override
        public double getStrVsBlock(BlockState state) {
            return strength(hand, state);
        }
    }

    static double strength(ItemStack hand, BlockState state) {
        Block b = state.getBlock();
        double mult = Baritone.settings().blocksToAvoidBreaking.value.contains(b) ? Baritone.settings().avoidBreakingMultiplier.value : 1;
        return ToolSet.calculateSpeedVsBlock(hand, b.defaultBlockState()) * mult;
    }

    static final class Ctx extends CalculationContext {
        Ctx(BlockStateInterface bsi, ToolSet tools, boolean throwaway, boolean bucket, boolean sprint, int frost, float waterMult) {
            super(null, false, null, null, bsi, tools, throwaway, bucket, sprint, frost, waterMult);
        }
    }

    record Cfg(boolean pick, boolean throwaway, boolean bucket, boolean sprint, int frost, float waterMult,
               boolean allowBreak, boolean walkOnWater, boolean magma, boolean vines, boolean bottomSlab,
               boolean strictLiquid, boolean avoidFalling, boolean placeSource, boolean placeFlow, boolean swim, int maxFall) {
    }

    static final Cfg[] CONFIGS = {
            new Cfg(true, true, false, true, 0, 1f, true, false, false, false, true, false, true, true, true, false, 3),
            new Cfg(false, false, false, false, 0, 0f, true, false, false, false, true, false, true, true, true, false, 3),
            new Cfg(true, true, true, true, 2, 0.33333334f, true, false, false, false, true, false, true, true, true, false, 3),
            new Cfg(true, true, false, true, 0, 1f, false, false, false, false, true, false, true, true, true, false, 3),
            new Cfg(true, true, false, true, 0, 1f, true, true, true, true, true, false, true, true, true, false, 3),
            new Cfg(true, true, true, true, 0, 0f, true, false, false, false, false, true, false, false, false, true, 6),
    };

    static void apply(Cfg c) {
        Settings s = Baritone.settings();
        s.allowBreak.value = c.allowBreak;
        s.assumeWalkOnWater.value = c.walkOnWater;
        s.allowWalkOnMagmaBlocks.value = c.magma;
        s.allowVines.value = c.vines;
        s.allowWalkOnBottomSlab.value = c.bottomSlab;
        s.strictLiquidCheck.value = c.strictLiquid;
        s.avoidUpdatingFallingBlocks.value = c.avoidFalling;
        s.allowPlaceInFluidsSource.value = c.placeSource;
        s.allowPlaceInFluidsFlow.value = c.placeFlow;
        s.allowSwimming.value = c.swim;
        s.maxFallHeightNoWater.value = c.maxFall;
    }

    static List<BlockState> palette() {
        List<BlockState> p = new ArrayList<>();
        for (Block b : new Block[]{Blocks.AIR, Blocks.CAVE_AIR, Blocks.STONE, Blocks.DIRT, Blocks.GRAVEL, Blocks.SAND, Blocks.BEDROCK, Blocks.OBSIDIAN,
                Blocks.GLASS, Blocks.WHITE_STAINED_GLASS, Blocks.OAK_LEAVES, Blocks.SOUL_SAND, Blocks.MAGMA_BLOCK, Blocks.ICE, Blocks.PACKED_ICE,
                Blocks.INFESTED_STONE, Blocks.CHEST, Blocks.TRAPPED_CHEST, Blocks.ENDER_CHEST, Blocks.FURNACE, Blocks.FARMLAND, Blocks.DIRT_PATH,
                Blocks.LADDER, Blocks.VINE, Blocks.WEEPING_VINES, Blocks.TWISTING_VINES_PLANT, Blocks.COBWEB, Blocks.FIRE, Blocks.SOUL_FIRE,
                Blocks.OAK_DOOR, Blocks.IRON_DOOR, Blocks.OAK_FENCE_GATE, Blocks.OAK_FENCE, Blocks.COBBLESTONE_WALL, Blocks.WHITE_CARPET, Blocks.LILY_PAD,
                Blocks.TALL_GRASS, Blocks.LARGE_FERN, Blocks.SHORT_GRASS, Blocks.CACTUS, Blocks.HONEY_BLOCK, Blocks.BUBBLE_COLUMN, Blocks.POWDER_SNOW,
                Blocks.BIG_DRIPLEAF, Blocks.END_ROD, Blocks.SWEET_BERRY_BUSH, Blocks.POINTED_DRIPSTONE, Blocks.SHULKER_BOX, Blocks.SCAFFOLDING, Blocks.BAMBOO,
                Blocks.SKELETON_SKULL, Blocks.CAULDRON, Blocks.WATER_CAULDRON, Blocks.AZALEA, Blocks.AMETHYST_CLUSTER, Blocks.KELP, Blocks.SEAGRASS,
                Blocks.END_PORTAL, Blocks.TORCH, Blocks.REDSTONE_WIRE, Blocks.TRIPWIRE, Blocks.COCOA, Blocks.CRAFTING_TABLE}) {
            p.add(b.defaultBlockState());
        }
        BlockState stair = Blocks.OAK_STAIRS.defaultBlockState();
        p.add(stair);
        p.add(stair.setValue(StairBlock.HALF, Half.TOP));
        p.add(stair.setValue(StairBlock.WATERLOGGED, true));
        p.add(stair.setValue(StairBlock.WATERLOGGED, true).setValue(StairBlock.SHAPE, StairsShape.INNER_LEFT));
        p.add(stair.setValue(StairBlock.WATERLOGGED, true).setValue(StairBlock.HALF, Half.TOP));
        BlockState slab = Blocks.OAK_SLAB.defaultBlockState();
        p.add(slab);
        p.add(slab.setValue(SlabBlock.TYPE, SlabType.TOP));
        p.add(slab.setValue(SlabBlock.TYPE, SlabType.DOUBLE));
        p.add(slab.setValue(SlabBlock.WATERLOGGED, true));
        p.add(slab.setValue(SlabBlock.TYPE, SlabType.TOP).setValue(SlabBlock.WATERLOGGED, true));
        BlockState trap = Blocks.OAK_TRAPDOOR.defaultBlockState();
        p.add(trap);
        p.add(trap.setValue(TrapDoorBlock.HALF, Half.TOP).setValue(TrapDoorBlock.WATERLOGGED, true));
        p.add(trap.setValue(TrapDoorBlock.OPEN, true).setValue(TrapDoorBlock.WATERLOGGED, true));
        p.add(Blocks.WATER.defaultBlockState());
        p.add(Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, 1));
        p.add(Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, 8));
        p.add(Blocks.LAVA.defaultBlockState());
        p.add(Blocks.LAVA.defaultBlockState().setValue(LiquidBlock.LEVEL, 2));
        p.add(Blocks.SNOW.defaultBlockState());
        p.add(Blocks.SNOW.defaultBlockState().setValue(SnowLayerBlock.LAYERS, 3));
        p.add(Blocks.OAK_LEAVES.defaultBlockState().setValue(LeavesBlock.WATERLOGGED, true));
        p.add(Blocks.SCAFFOLDING.defaultBlockState().setValue(ScaffoldingBlock.WATERLOGGED, true));
        // Added at the end so that earlier indexes do not move (the fluid list in grid() uses fixed indexes).
        p.add(trap.setValue(TrapDoorBlock.HALF, Half.TOP));
        p.add(trap.setValue(TrapDoorBlock.OPEN, true).setValue(TrapDoorBlock.HALF, Half.TOP).setValue(TrapDoorBlock.WATERLOGGED, true));
        return p;
    }

    static int b(boolean v) {
        return v ? 1 : 0;
    }

    static String fluid(FluidState f) {
        Fluid t = f.getType();
        if (t == Fluids.WATER) return "water";
        if (t == Fluids.FLOWING_WATER) return "flowing_water";
        if (t == Fluids.LAVA) return "lava";
        if (t == Fluids.FLOWING_LAVA) return "flowing_lava";
        return "none";
    }

    /** Raw game facts only. Baritone's own rules are never exported: the Rust side has to derive them. */
    static String facts(int i, BlockState s, ItemStack pick, ItemStack hand) {
        Block k = s.getBlock();
        FluidState f = s.getFluidState();
        StringBuilder o = new StringBuilder("S ").append(i).append(' ').append(BuiltInRegistries.BLOCK.getKey(k).getPath());
        String[][] props = {
                {"air", "" + b(k instanceof AirBlock)}, {"base_fire", "" + b(k instanceof BaseFireBlock)}, {"skull", "" + b(k instanceof AbstractSkullBlock)},
                {"skull_block", "" + b(k instanceof SkullBlock)}, {"shulker", "" + b(k instanceof ShulkerBoxBlock)}, {"slab", "" + b(k instanceof SlabBlock)},
                {"trapdoor", "" + b(k instanceof TrapDoorBlock)}, {"amethyst", "" + b(k instanceof AmethystClusterBlock)}, {"azalea", "" + b(k instanceof AzaleaBlock)},
                {"door", "" + b(k instanceof DoorBlock)}, {"wooden_door", "" + b(k instanceof DoorBlock && DoorBlock.isWoodenDoor(s))}, {"gate", "" + b(k instanceof FenceGateBlock)},
                {"carpet", "" + b(k instanceof CarpetBlock)}, {"snow", "" + b(k instanceof SnowLayerBlock)}, {"cauldron", "" + b(k instanceof CauldronBlock)},
                {"stair", "" + b(k instanceof StairBlock)}, {"stained_glass", "" + b(k instanceof StainedGlassBlock)}, {"liquid_block", "" + b(k instanceof LiquidBlock)},
                {"infested", "" + b(k instanceof InfestedBlock)}, {"lily", "" + b(k instanceof WaterlilyBlock)}, {"falling", "" + b(k instanceof FallingBlock)},
                {"leaves", "" + b(k instanceof LeavesBlock)}, {"pathfindable", "" + b(s.isPathfindable(PathComputationType.LAND))},
                {"normal_cube", "" + b(MovementHelper.isBlockNormalCube(s))}, {"waterlogged_solid_top", "" + b(MovementHelper.isWaterloggedSolidTop(s))},
                {"replaceable", "" + b(s.canBeReplaced())}, {"falling_free", "" + b(FallingBlock.isFree(s))},
                {"frost_melt", "" + b(s == FrostedIceBlock.meltsInto())},
                {"fluid", fluid(f)}, {"fluid_empty", "" + b(f.isEmpty())}, {"fluid_amount", "" + f.getType().getAmount(f)}, {"fluid_source", "" + b(f.isSource())},
                {"fluid_flowing_kind", "" + b(f.getType() instanceof FlowingFluid)},
                {"level", k instanceof LiquidBlock ? "" + s.getValue(LiquidBlock.LEVEL) : "-1"},
                {"slab_type", k instanceof SlabBlock ? s.getValue(SlabBlock.TYPE).name() : "-"},
                {"stair_half", k instanceof StairBlock ? s.getValue(StairBlock.HALF).name() : "-"},
                {"stair_shape", k instanceof StairBlock ? s.getValue(StairBlock.SHAPE).name() : "-"},
                {"trap_open", k instanceof TrapDoorBlock ? "" + b(s.getValue(TrapDoorBlock.OPEN)) : "0"},
                {"trap_half", k instanceof TrapDoorBlock ? s.getValue(TrapDoorBlock.HALF).name() : "-"},
                {"layers", k instanceof SnowLayerBlock ? "" + s.getValue(SnowLayerBlock.LAYERS) : "0"},
                {"str_pick", Long.toHexString(Double.doubleToLongBits(strength(pick, s)))},
                {"str_hand", Long.toHexString(Double.doubleToLongBits(strength(hand, s)))},
        };
        for (String[] kv : props) {
            o.append(' ').append(kv[0]).append('=').append(kv[1]);
        }
        return o.toString();
    }

    static int[] grid(Random r, int n) {
        int[] c = new int[W * H * D];
        int mode = r.nextInt(7);
        int[] fluids = {65, 66, 67, 71, 72, 74, 75, 76, 76, 76, 77, 78, 83, 84, 85, 86};
        for (int x = 0; x < W; x++) {
            for (int z = 0; z < D; z++) {
                int floor = 3 + r.nextInt(mode == 0 ? 2 : 6);
                for (int y = 0; y < H; y++) {
                    int v;
                    boolean noise = mode == 3 ? r.nextInt(100) < 70 : r.nextInt(100) < 12;
                    if (mode == 4 && r.nextInt(100) < 60) {
                        v = fluids[r.nextInt(fluids.length)];
                    } else if (mode == 5) {
                        // mostly water: sources above sources, and flowing water, for the walk-on-water rules
                        v = r.nextInt(100) < 85 ? 76 + r.nextInt(3) : (y < 2 ? 2 : 0);
                    } else if (mode == 6) {
                        // mostly open air down to the bottom of the grid, for falls that reach the world's lowest layer
                        v = r.nextInt(100) < 4 ? r.nextInt(n) : 0;
                    } else if (noise) {
                        v = r.nextInt(n);
                    } else if (y < floor) {
                        v = 2 + r.nextInt(2);
                    } else {
                        v = 0;
                    }
                    c[(y * D + z) * W + x] = v;
                }
            }
        }
        return c;
    }

    /**
     * A bare Bootstrap binds no tags, so tools would see no mining rules. This reads the vanilla block tags from the
     * game jar on the classpath and binds them on the block registry, resolving nested "#tag" entries.
     */
    static int bindBlockTags() throws Exception {
        Path jar = Path.of(Bootstrap.class.getProtectionDomain().getCodeSource().getLocation().toURI());
        Map<String, List<String>> raw = new LinkedHashMap<>();
        String prefix = "data/minecraft/tags/block/";
        try (ZipFile z = new ZipFile(jar.toFile())) {
            for (java.util.Enumeration<? extends ZipEntry> e = z.entries(); e.hasMoreElements(); ) {
                ZipEntry en = e.nextElement();
                String n = en.getName();
                if (!n.startsWith(prefix) || !n.endsWith(".json")) continue;
                String id = n.substring(prefix.length(), n.length() - 5);
                JsonObject o = JsonParser.parseReader(new java.io.InputStreamReader(z.getInputStream(en))).getAsJsonObject();
                List<String> vals = new ArrayList<>();
                for (JsonElement v : o.getAsJsonArray("values")) {
                    vals.add(v.isJsonObject() ? v.getAsJsonObject().get("id").getAsString() : v.getAsString());
                }
                raw.put(id, vals);
            }
        }
        if (raw.isEmpty()) throw new IllegalStateException("no block tags found in " + jar);
        Map<TagKey<Block>, List<Holder<Block>>> bound = new HashMap<>();
        for (String id : raw.keySet()) {
            List<Holder<Block>> out = new ArrayList<>();
            expand(id, raw, out, new java.util.HashSet<>());
            bound.put(TagKey.create(Registries.BLOCK, ResourceLocation.withDefaultNamespace(id)), out);
        }
        var pending = BuiltInRegistries.BLOCK.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK, bound));
        pending.apply();
        return pending.size();
    }

    static void expand(String id, Map<String, List<String>> raw, List<Holder<Block>> out, java.util.Set<String> seen) {
        if (!seen.add(id)) return;
        for (String v : raw.getOrDefault(id, List.of())) {
            if (v.startsWith("#")) {
                expand(v.substring(1).replace("minecraft:", ""), raw, out, seen);
            } else {
                BuiltInRegistries.BLOCK.get(ResourceLocation.parse(v)).ifPresent(h -> {
                    if (!out.contains(h)) out.add(h);
                });
            }
        }
    }

    @Test
    public void dump() throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        System.out.println("bound block tags: " + bindBlockTags());
        Settings s = Baritone.settings();
        s.considerPotionEffects.value = false;
        air = Blocks.AIR.defaultBlockState();
        int count = Integer.parseInt(System.getenv().getOrDefault("SPIKE_GRIDS", "5000"));
        Path out = Path.of(System.getenv("SPIKE_OUT"));
        List<BlockState> pal = palette();
        ItemStack pick = new ItemStack(Items.DIAMOND_PICKAXE);
        ItemStack hand = ItemStack.EMPTY;
        WorldBorder wb = new WorldBorder();
        BetterWorldBorder border = new BetterWorldBorder(wb);
        int[][] dirs = {{0, -1}, {0, 1}, {1, 0}, {-1, 0}};
        Random r = new Random(0x5EEDL);
        try (BufferedWriter w = Files.newBufferedWriter(out)) {
            w.write("B " + wb.getMinX() + " " + wb.getMaxX() + " " + wb.getMinZ() + " " + wb.getMaxZ() + " " + W + " " + H + " " + D + "\n");
            w.write("X place=" + s.blockPlacementPenalty.value + " break=" + s.blockBreakAdditionalPenalty.value + " waterwalk=" + s.walkOnWaterOnePenalty.value + " bucket=" + s.maxFallHeightBucket.value + "\n");
            for (int i = 0; i < pal.size(); i++) {
                w.write(facts(i, pal.get(i), pick, hand) + "\n");
            }
            for (int i = 0; i < CONFIGS.length; i++) {
                Cfg c = CONFIGS[i];
                w.write("C " + i + " " + b(c.pick) + " " + b(c.throwaway) + " " + b(c.bucket) + " " + b(c.sprint) + " " + c.frost + " " + Integer.toHexString(Float.floatToIntBits(c.waterMult)) + " " + b(c.allowBreak) + " " + b(c.walkOnWater) + " " + b(c.magma) + " " + b(c.vines) + " " + b(c.bottomSlab) + " " + b(c.strictLiquid) + " " + b(c.avoidFalling) + " " + b(c.placeSource) + " " + b(c.placeFlow) + " " + b(c.swim) + " " + c.maxFall + "\n");
            }
            MutableMoveResult res = new MutableMoveResult();
            for (int g = 0; g < count; g++) {
                int[] cells = grid(r, pal.size());
                BlockState[] states = new BlockState[cells.length];
                StringBuilder line = new StringBuilder("G");
                for (int i = 0; i < cells.length; i++) {
                    states[i] = pal.get(cells[i]);
                    line.append(' ').append(cells[i]);
                }
                w.write(line + "\n");
                int y = 1 + r.nextInt(H - 3);
                w.write("Y " + y + "\n");
                for (int ci = 0; ci < CONFIGS.length; ci++) {
                    Cfg c = CONFIGS[ci];
                    apply(c);
                    Ctx ctx = new Ctx(new Grid(border, states), new Tools(c.pick ? pick : hand), c.throwaway, c.bucket, c.sprint, c.frost, c.waterMult);
                    for (int[] d : dirs) {
                        int dx = 2 + d[0], dz = 2 + d[1];
                        double t = MovementTraverse.cost(ctx, 2, y, 2, dx, dz);
                        res.reset();
                        MovementDescend.cost(ctx, 2, y, 2, dx, dz, res);
                        w.write("R " + ci + " " + dx + " " + dz + " " + Long.toHexString(Double.doubleToLongBits(t)) + " " + res.x + " " + res.y + " " + res.z + " " + Long.toHexString(Double.doubleToLongBits(res.cost)) + "\n");
                    }
                }
            }
        }
    }
}
