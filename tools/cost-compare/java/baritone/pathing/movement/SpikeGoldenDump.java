/*
 * baritone-rs test tool: dumps game facts and upstream's own answers. Copy it into a checkout of upstream.
 * Licensed like the code it exercises: GNU LGPL, version 3 or (at your option) any later version.
 *
 * Rule: this file exports raw game facts (what a block state is) and the results of upstream's real classes. It never
 * exports one of Baritone's rules as a fact; the Rust side has to derive those itself.
 */
package baritone.pathing.movement;

import baritone.Baritone;
import baritone.api.Settings;
import baritone.api.pathing.goals.*;
import baritone.api.pathing.movement.ActionCosts;
import baritone.pathing.precompute.Ternary;
import baritone.utils.BlockStateInterface;
import baritone.utils.ToolSet;
import baritone.utils.pathing.BetterWorldBorder;
import baritone.utils.pathing.MutableMoveResult;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.Bootstrap;
import net.minecraft.tags.TagKey;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.piston.MovingPistonBlock;
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
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.function.Consumer;
import java.util.function.Predicate;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

public class SpikeGoldenDump {
    static final int W = 13, H = 16, D = 13, SX = 6, SZ = 6;
    static BlockState air;

    // The palette, the roles and the configurations below read the block registry while the class loads, so the game's
    // bootstrap has to be the first static initializer.
    static {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    // ------------------------------------------------------------------------------------------------------------
    // The world: a box of blocks. Out of range in X or Z is air; out of range in Y is air through get0.
    // ------------------------------------------------------------------------------------------------------------

    static final class Grid extends BlockStateInterface {
        final BlockState[] cells;
        final boolean loaded;

        Grid(BetterWorldBorder border, BlockState[] cells, boolean loaded) {
            super(border, 0, H);
            this.cells = cells;
            this.loaded = loaded;
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
            return loaded;
        }

        @Override
        public boolean worldContainsLoadedChunk(int x, int z) {
            return loaded;
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

    // ------------------------------------------------------------------------------------------------------------
    // Configurations: every setting that mod/engine/src/config.rs holds, written to the file in full.
    // ------------------------------------------------------------------------------------------------------------

    static final class Cfg {
        boolean pick = true, throwaway = true, bucket = false, sprint = true;
        int frost = 0;
        float waterMult = 1f;
        boolean allowBreak = true, walkOnWater = false, walkOnLava = false, magma = false, vines = false, bottomSlab = true,
                strictLiquid = false, avoidFalling = true, placeSource = true, placeFlow = true, swim = false;
        boolean parkour = false, parkourPlace = false, parkourAscend = true, jumpAtBuildLimit = false,
                diagDescend = false, diagAscend = false, downward = true;
        int maxFall = 3, maxFallBucket = 20;
        double placePenalty = 20, breakPenalty = 2, jumpPenalty = 2, waterWalkPenalty = 3, costHeuristic = 3.563;
        int axisHeight = 120;
        List<Block> avoid = List.of(Blocks.TRIPWIRE), disallow = List.of(), anyway = List.of();
    }

    static Cfg cfg(Consumer<Cfg> f) {
        Cfg c = new Cfg();
        f.accept(c);
        return c;
    }

    static final List<Cfg> CONFIGS = new ArrayList<>();
    /** The configuration whose costHeuristic and axisHeight differ from the defaults, used by half of the goal checks. */
    static final int GOAL_CFG = 10;
    /** The registry sweep answers under config 0 and under this one (magma, vines, lava, bottom slab flipped, other blocksToAvoid). */
    static final int REG_ALT = 13;

    static {
        // 0: upstream's defaults, with a pickaxe, a throwaway block and sprinting
        CONFIGS.add(cfg(c -> {
        }));
        // 1: bare hand, nothing to place, no sprint, no water speed
        CONFIGS.add(cfg(c -> {
            c.pick = false;
            c.throwaway = false;
            c.sprint = false;
            c.waterMult = 0f;
        }));
        // 2: water bucket, frost walker, depth strider; parkour on
        CONFIGS.add(cfg(c -> {
            c.bucket = true;
            c.frost = 2;
            c.waterMult = 0.33333334f;
            c.parkour = true;
        }));
        // 3: breaking off, except for a list; diagonals on
        CONFIGS.add(cfg(c -> {
            c.allowBreak = false;
            c.anyway = List.of(Blocks.STONE, Blocks.DIRT, Blocks.GRAVEL, Blocks.OAK_PLANKS, Blocks.ICE);
            c.diagDescend = true;
            c.diagAscend = true;
        }));
        // 4: walk on water, lava, magma and vines
        CONFIGS.add(cfg(c -> {
            c.walkOnWater = true;
            c.walkOnLava = true;
            c.magma = true;
            c.vines = true;
            c.parkour = true;
        }));
        // 5: the strict, no-place side of every fluid and slab setting, swimming, a long fall
        CONFIGS.add(cfg(c -> {
            c.bucket = true;
            c.waterMult = 0f;
            c.bottomSlab = false;
            c.strictLiquid = true;
            c.avoidFalling = false;
            c.placeSource = false;
            c.placeFlow = false;
            c.swim = true;
            c.maxFall = 6;
            c.maxFallBucket = 8;
            c.parkour = true;
            c.parkourPlace = true;
        }));
        // 6: parkour with placing, ascending parkour on
        CONFIGS.add(cfg(c -> {
            c.parkour = true;
            c.parkourPlace = true;
        }));
        // 7: parkour without sprint and without ascending, at the build limit; big jump penalty, magma, frost
        CONFIGS.add(cfg(c -> {
            c.parkour = true;
            c.parkourAscend = false;
            c.jumpAtBuildLimit = true;
            c.sprint = false;
            c.jumpPenalty = 5;
            c.magma = true;
            c.frost = 1;
        }));
        // 8: diagonals, no downward, other blocksToAvoid
        CONFIGS.add(cfg(c -> {
            c.diagDescend = true;
            c.diagAscend = true;
            c.downward = false;
            c.parkour = true;
            c.avoid = List.of(Blocks.OAK_DOOR, Blocks.WHITE_CARPET, Blocks.SNOW, Blocks.COBWEB);
        }));
        // 9: empty blocksToAvoid, blocks that must not be broken, diagonal descend only, cheap placing
        CONFIGS.add(cfg(c -> {
            c.avoid = List.of();
            c.disallow = List.of(Blocks.DIRT, Blocks.GRAVEL, Blocks.OAK_LEAVES, Blocks.COBBLESTONE);
            c.diagDescend = true;
            c.parkour = true;
            c.parkourPlace = true;
            c.placePenalty = 3.5;
        }));
        // 10: the goals' other settings, cheap everything
        CONFIGS.add(cfg(c -> {
            c.walkOnWater = true;
            c.parkour = true;
            c.sprint = false;
            c.breakPenalty = 0;
            c.jumpPenalty = 0.25;
            c.waterWalkPenalty = 0.5;
            c.maxFallBucket = 12;
            c.costHeuristic = 1.0;
            c.axisHeight = 7;
        }));
        // 11: nothing to place, bare hand, parkour with place on (so the cost of placing is infinite)
        CONFIGS.add(cfg(c -> {
            c.pick = false;
            c.throwaway = false;
            c.parkour = true;
            c.parkourPlace = true;
            c.diagAscend = true;
        }));
        // 12: bucket, frost, swimming with sprint, lava walking
        CONFIGS.add(cfg(c -> {
            c.bucket = true;
            c.frost = 2;
            c.swim = true;
            c.walkOnLava = true;
            c.parkour = true;
            c.diagDescend = true;
            c.diagAscend = true;
            c.waterMult = 0.5f;
        }));
        // 13: the registry sweep's second configuration: magma, vines, lava, bottom slab flipped, other blocksToAvoid
        CONFIGS.add(cfg(c -> {
            c.magma = true;
            c.vines = true;
            c.walkOnLava = true;
            c.bottomSlab = false;
            c.avoid = List.of(Blocks.TRIPWIRE, Blocks.OAK_DOOR, Blocks.ICE, Blocks.SWEET_BERRY_BUSH, Blocks.WHITE_CARPET);
            c.parkour = true;
            c.parkourAscend = false;
            c.placePenalty = 1.5;
        }));
        // 14: frost walker 1 with sprinting, walking on water and magma, both diagonals, parkour with placing
        CONFIGS.add(cfg(c -> {
            c.frost = 1;
            c.walkOnWater = true;
            c.diagDescend = true;
            c.diagAscend = true;
            c.parkour = true;
            c.parkourPlace = true;
            c.magma = true;
        }));
        // 15: frost walker 1 with sprinting and no walking on water, both diagonals, parkour with placing
        CONFIGS.add(cfg(c -> {
            c.frost = 1;
            c.diagDescend = true;
            c.diagAscend = true;
            c.parkour = true;
            c.parkourPlace = true;
        }));
    }

    static void apply(Cfg c) {
        Settings s = Baritone.settings();
        s.allowBreak.value = c.allowBreak;
        s.allowBreakAnyway.value = new ArrayList<>(c.anyway);
        s.blocksToAvoid.value = new ArrayList<>(c.avoid);
        s.blocksToDisallowBreaking.value = new ArrayList<>(c.disallow);
        s.assumeWalkOnWater.value = c.walkOnWater;
        s.assumeWalkOnLava.value = c.walkOnLava;
        s.allowWalkOnMagmaBlocks.value = c.magma;
        s.allowVines.value = c.vines;
        s.allowWalkOnBottomSlab.value = c.bottomSlab;
        s.strictLiquidCheck.value = c.strictLiquid;
        s.avoidUpdatingFallingBlocks.value = c.avoidFalling;
        s.allowPlaceInFluidsSource.value = c.placeSource;
        s.allowPlaceInFluidsFlow.value = c.placeFlow;
        s.allowSwimming.value = c.swim;
        s.allowParkour.value = c.parkour;
        s.allowParkourPlace.value = c.parkourPlace;
        s.allowParkourAscend.value = c.parkourAscend;
        s.allowJumpAtBuildLimit.value = c.jumpAtBuildLimit;
        s.allowDiagonalDescend.value = c.diagDescend;
        s.allowDiagonalAscend.value = c.diagAscend;
        s.allowDownward.value = c.downward;
        s.maxFallHeightNoWater.value = c.maxFall;
        s.maxFallHeightBucket.value = c.maxFallBucket;
        s.blockPlacementPenalty.value = c.placePenalty;
        s.blockBreakAdditionalPenalty.value = c.breakPenalty;
        s.jumpPenalty.value = c.jumpPenalty;
        s.walkOnWaterOnePenalty.value = c.waterWalkPenalty;
        s.costHeuristic.value = c.costHeuristic;
        s.axisHeight.value = c.axisHeight;
        // PrecomputedData.forCurrentSettings notices a change of the settings it was built with and builds a new table.
        // It is not reset here, so that the harness also exercises that check.
    }

    static String names(List<Block> l) {
        if (l.isEmpty()) {
            return "-";
        }
        StringBuilder o = new StringBuilder();
        for (Block b : l) {
            if (o.length() > 0) {
                o.append(',');
            }
            o.append(BuiltInRegistries.BLOCK.getKey(b).getPath());
        }
        return o.toString();
    }

    static String cline(int i, Cfg c) {
        return "C " + i + " pick=" + b(c.pick) + " throwaway=" + b(c.throwaway) + " bucket=" + b(c.bucket) + " sprint=" + b(c.sprint)
                + " frost=" + c.frost + " watermult=" + Integer.toHexString(Float.floatToIntBits(c.waterMult))
                + " allowbreak=" + b(c.allowBreak) + " anyway=" + names(c.anyway) + " avoid=" + names(c.avoid) + " disallow=" + names(c.disallow)
                + " walkwater=" + b(c.walkOnWater) + " walklava=" + b(c.walkOnLava) + " magma=" + b(c.magma) + " vines=" + b(c.vines)
                + " bottomslab=" + b(c.bottomSlab) + " strictliquid=" + b(c.strictLiquid) + " avoidfalling=" + b(c.avoidFalling)
                + " placesource=" + b(c.placeSource) + " placeflow=" + b(c.placeFlow) + " swim=" + b(c.swim)
                + " parkour=" + b(c.parkour) + " parkourplace=" + b(c.parkourPlace) + " parkourascend=" + b(c.parkourAscend)
                + " buildlimit=" + b(c.jumpAtBuildLimit) + " diagdescend=" + b(c.diagDescend) + " diagascend=" + b(c.diagAscend)
                + " downward=" + b(c.downward) + " maxfall=" + c.maxFall + " maxfallbucket=" + c.maxFallBucket
                + " place=" + hx(c.placePenalty) + " break=" + hx(c.breakPenalty) + " jump=" + hx(c.jumpPenalty)
                + " waterwalk=" + hx(c.waterWalkPenalty) + " costheuristic=" + hx(c.costHeuristic) + " axisheight=" + c.axisHeight;
    }

    // ------------------------------------------------------------------------------------------------------------
    // The palette: the block states that grids are made of
    // ------------------------------------------------------------------------------------------------------------

    static <T extends Comparable<T>> BlockState with(BlockState s, Property<T> p, T v) {
        return s.hasProperty(p) ? s.setValue(p, v) : s;
    }

    static BlockState wl(BlockState s) {
        return with(s, BlockStateProperties.WATERLOGGED, true);
    }

    static List<BlockState> palette() {
        LinkedHashSet<BlockState> p = new LinkedHashSet<>();
        for (Block b : new Block[]{Blocks.AIR, Blocks.CAVE_AIR, Blocks.STONE, Blocks.DIRT, Blocks.COBBLESTONE, Blocks.GRAVEL, Blocks.SAND, Blocks.RED_SAND,
                Blocks.BEDROCK, Blocks.OBSIDIAN, Blocks.OAK_PLANKS, Blocks.GLASS, Blocks.WHITE_STAINED_GLASS, Blocks.GLASS_PANE, Blocks.OAK_LEAVES,
                Blocks.SOUL_SAND, Blocks.SOUL_SOIL, Blocks.MAGMA_BLOCK, Blocks.ICE, Blocks.PACKED_ICE, Blocks.FROSTED_ICE, Blocks.INFESTED_STONE,
                Blocks.CHEST, Blocks.TRAPPED_CHEST, Blocks.ENDER_CHEST, Blocks.FURNACE, Blocks.CRAFTING_TABLE, Blocks.FARMLAND, Blocks.DIRT_PATH,
                Blocks.LADDER, Blocks.VINE, Blocks.WEEPING_VINES, Blocks.WEEPING_VINES_PLANT, Blocks.TWISTING_VINES, Blocks.TWISTING_VINES_PLANT,
                Blocks.COBWEB, Blocks.FIRE, Blocks.SOUL_FIRE, Blocks.OAK_FENCE_GATE, Blocks.OAK_FENCE, Blocks.COBBLESTONE_WALL, Blocks.IRON_BARS,
                Blocks.WHITE_CARPET, Blocks.MOSS_CARPET, Blocks.LILY_PAD, Blocks.TALL_GRASS, Blocks.LARGE_FERN, Blocks.SHORT_GRASS, Blocks.CACTUS,
                Blocks.HONEY_BLOCK, Blocks.SLIME_BLOCK, Blocks.BUBBLE_COLUMN, Blocks.POWDER_SNOW, Blocks.BIG_DRIPLEAF, Blocks.SMALL_DRIPLEAF,
                Blocks.END_ROD, Blocks.SWEET_BERRY_BUSH, Blocks.POINTED_DRIPSTONE, Blocks.SHULKER_BOX, Blocks.SCAFFOLDING, Blocks.BAMBOO,
                Blocks.BAMBOO_SAPLING, Blocks.SKELETON_SKULL, Blocks.SKELETON_WALL_SKULL, Blocks.CAULDRON, Blocks.WATER_CAULDRON, Blocks.LAVA_CAULDRON,
                Blocks.AZALEA, Blocks.FLOWERING_AZALEA, Blocks.AMETHYST_CLUSTER, Blocks.KELP, Blocks.KELP_PLANT, Blocks.SEAGRASS, Blocks.TALL_SEAGRASS,
                Blocks.END_PORTAL, Blocks.END_PORTAL_FRAME, Blocks.NETHER_PORTAL, Blocks.TORCH, Blocks.REDSTONE_WIRE, Blocks.TRIPWIRE,
                Blocks.TRIPWIRE_HOOK, Blocks.COCOA, Blocks.MOVING_PISTON, Blocks.PISTON_HEAD, Blocks.ANVIL, Blocks.HOPPER, Blocks.CAKE,
                Blocks.ENCHANTING_TABLE, Blocks.FLOWER_POT, Blocks.SNOW_BLOCK, Blocks.WHITE_CONCRETE_POWDER, Blocks.DRAGON_EGG, Blocks.BARRIER,
                Blocks.GLOW_LICHEN, Blocks.CHAIN, Blocks.LANTERN, Blocks.CAMPFIRE, Blocks.COMPOSTER, Blocks.SEA_PICKLE, Blocks.REPEATER, Blocks.RAIL,
                Blocks.STONE_PRESSURE_PLATE, Blocks.LEVER, Blocks.BELL, Blocks.MUD, Blocks.SPAWNER, Blocks.OAK_SIGN, Blocks.WHITE_BED}) {
            p.add(b.defaultBlockState());
        }
        BlockState stair = Blocks.OAK_STAIRS.defaultBlockState();
        for (Half h : Half.values()) {
            for (StairsShape sh : StairsShape.values()) {
                p.add(stair.setValue(StairBlock.HALF, h).setValue(StairBlock.SHAPE, sh));
            }
        }
        p.add(wl(stair));
        p.add(wl(stair.setValue(StairBlock.HALF, Half.TOP)));
        p.add(wl(stair.setValue(StairBlock.SHAPE, StairsShape.INNER_LEFT)));
        p.add(wl(stair.setValue(StairBlock.SHAPE, StairsShape.OUTER_RIGHT)));
        p.add(stair.setValue(StairBlock.FACING, Direction.EAST));
        BlockState slab = Blocks.OAK_SLAB.defaultBlockState();
        for (SlabType t : SlabType.values()) {
            p.add(slab.setValue(SlabBlock.TYPE, t));
            if (t != SlabType.DOUBLE) {
                p.add(wl(slab.setValue(SlabBlock.TYPE, t)));
            }
        }
        BlockState trap = Blocks.OAK_TRAPDOOR.defaultBlockState();
        for (Half h : Half.values()) {
            for (boolean open : new boolean[]{false, true}) {
                p.add(trap.setValue(TrapDoorBlock.HALF, h).setValue(TrapDoorBlock.OPEN, open));
                p.add(wl(trap.setValue(TrapDoorBlock.HALF, h).setValue(TrapDoorBlock.OPEN, open)));
            }
        }
        p.add(Blocks.IRON_TRAPDOOR.defaultBlockState());
        p.add(Blocks.IRON_TRAPDOOR.defaultBlockState().setValue(TrapDoorBlock.OPEN, true));
        p.add(Blocks.COPPER_TRAPDOOR.defaultBlockState());
        for (Direction f : new Direction[]{Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST}) {
            for (DoorHingeSide hinge : DoorHingeSide.values()) {
                for (boolean open : new boolean[]{false, true}) {
                    p.add(Blocks.OAK_DOOR.defaultBlockState().setValue(DoorBlock.FACING, f).setValue(DoorBlock.HINGE, hinge).setValue(DoorBlock.OPEN, open));
                }
            }
        }
        p.add(Blocks.OAK_DOOR.defaultBlockState().setValue(DoorBlock.HALF, DoubleBlockHalf.UPPER));
        p.add(Blocks.OAK_DOOR.defaultBlockState().setValue(DoorBlock.HALF, DoubleBlockHalf.UPPER).setValue(DoorBlock.OPEN, true));
        for (Block door : new Block[]{Blocks.IRON_DOOR, Blocks.COPPER_DOOR}) {
            p.add(door.defaultBlockState());
            p.add(door.defaultBlockState().setValue(DoorBlock.OPEN, true));
        }
        BlockState gate = Blocks.OAK_FENCE_GATE.defaultBlockState();
        p.add(gate.setValue(FenceGateBlock.OPEN, true));
        p.add(gate.setValue(FenceGateBlock.FACING, Direction.EAST));
        p.add(gate.setValue(FenceGateBlock.FACING, Direction.EAST).setValue(FenceGateBlock.OPEN, true));
        p.add(gate.setValue(FenceGateBlock.IN_WALL, true));
        for (int lv : new int[]{0, 1, 2, 4, 7, 8}) {
            p.add(Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, lv));
            p.add(Blocks.LAVA.defaultBlockState().setValue(LiquidBlock.LEVEL, lv));
        }
        for (int l = 1; l <= 8; l++) {
            p.add(Blocks.SNOW.defaultBlockState().setValue(SnowLayerBlock.LAYERS, l));
        }
        p.add(wl(Blocks.OAK_LEAVES.defaultBlockState()));
        p.add(wl(Blocks.SCAFFOLDING.defaultBlockState()));
        p.add(wl(Blocks.LADDER.defaultBlockState()));
        p.add(Blocks.VINE.defaultBlockState().setValue(VineBlock.NORTH, true));
        p.add(Blocks.POINTED_DRIPSTONE.defaultBlockState().setValue(PointedDripstoneBlock.TIP_DIRECTION, Direction.DOWN));
        p.add(Blocks.BUBBLE_COLUMN.defaultBlockState().setValue(BubbleColumnBlock.DRAG_DOWN, false));
        return new ArrayList<>(p);
    }

    static final List<BlockState> PAL = palette();
    static final Map<BlockState, Integer> PIDX = new HashMap<>();

    static {
        if (PAL.size() > 256) {
            throw new IllegalStateException("palette has " + PAL.size() + " states; the grid encoding holds 256");
        }
        if (PAL.get(0) != Blocks.AIR.defaultBlockState()) {
            throw new IllegalStateException("palette index 0 must be air");
        }
        for (int i = 0; i < PAL.size(); i++) {
            PIDX.put(PAL.get(i), i);
        }
    }

    /** The palette index of a state; fails if the palette lacks it, so that a grid mode can never silently place something else. */
    static int ix(BlockState s) {
        Integer i = PIDX.get(s);
        if (i == null) {
            throw new IllegalStateException("not in the palette: " + s);
        }
        return i;
    }

    static int ix(Block b) {
        return ix(b.defaultBlockState());
    }

    static int[] where(Predicate<BlockState> f) {
        List<Integer> l = new ArrayList<>();
        for (int i = 0; i < PAL.size(); i++) {
            if (f.test(PAL.get(i))) {
                l.add(i);
            }
        }
        if (l.isEmpty()) {
            throw new IllegalStateException("a role of the palette is empty");
        }
        return l.stream().mapToInt(Integer::intValue).toArray();
    }

    static int[] ofBlocks(Block... bs) {
        List<Block> l = List.of(bs);
        return where(s -> l.contains(s.getBlock()));
    }

    // roles, computed from the palette so that no index is written by hand
    static final int AIR = 0;
    static final int STONE = ix(Blocks.STONE), DIRT = ix(Blocks.DIRT), GRAVEL = ix(Blocks.GRAVEL), COBBLE = ix(Blocks.COBBLESTONE);
    static final int[] SOLIDS = ofBlocks(Blocks.STONE, Blocks.DIRT, Blocks.COBBLESTONE, Blocks.OAK_PLANKS, Blocks.BEDROCK, Blocks.OBSIDIAN, Blocks.INFESTED_STONE, Blocks.ICE, Blocks.FURNACE);
    static final int[] FLUIDS = where(s -> !s.getFluidState().isEmpty());
    static final int[] WATERS = where(s -> MovementHelper.isWater(s));
    static final int[] LAVAS = where(s -> MovementHelper.isLava(s));
    static final int[] WATERLOGGED = where(s -> !s.getFluidState().isEmpty() && !(s.getBlock() instanceof LiquidBlock));
    static final int[] CLIMBS = where(s -> MovementHelper.isClimbable(s.getBlock()));
    static final int[] FALLERS = where(s -> s.getBlock() instanceof FallingBlock);
    static final int[] DOORS = where(s -> s.getBlock() instanceof DoorBlock);
    static final int[] GATES = where(s -> s.getBlock() instanceof FenceGateBlock);
    static final int[] TRAPS = where(s -> s.getBlock() instanceof TrapDoorBlock);
    static final int[] STAIRS = where(s -> s.getBlock() instanceof StairBlock);
    static final int[] SLABS = where(s -> s.getBlock() instanceof SlabBlock);
    static final int[] SNOWS = where(s -> s.getBlock() instanceof SnowLayerBlock);
    static final int[] CARPETS = where(s -> s.getBlock() instanceof CarpetBlock);
    static final int[] HAZARDS = ofBlocks(Blocks.MAGMA_BLOCK, Blocks.CACTUS, Blocks.SWEET_BERRY_BUSH, Blocks.FIRE, Blocks.SOUL_FIRE, Blocks.COBWEB, Blocks.BUBBLE_COLUMN, Blocks.END_PORTAL, Blocks.POWDER_SNOW, Blocks.WATER, Blocks.LAVA);
    static final int[] SPECIAL_FLOORS = where(s -> {
        Block k = s.getBlock();
        return k == Blocks.MAGMA_BLOCK || k == Blocks.SOUL_SAND || k == Blocks.FARMLAND || k == Blocks.ICE || k == Blocks.PACKED_ICE || k == Blocks.HONEY_BLOCK
                || k == Blocks.GLASS || k == Blocks.DIRT_PATH || k == Blocks.CHEST || k == Blocks.LADDER || k == Blocks.CACTUS || k instanceof SlabBlock
                || k instanceof StairBlock || k instanceof TrapDoorBlock || k == Blocks.SLIME_BLOCK || k == Blocks.OAK_LEAVES || k == Blocks.SCAFFOLDING
                || k == Blocks.WATER || k == Blocks.LAVA || k == Blocks.SOUL_SOIL || k == Blocks.CAULDRON || k == Blocks.OAK_FENCE;
    });
    static final int[] KEY_FLOORS = ofBlocks(Blocks.MAGMA_BLOCK, Blocks.SOUL_SAND, Blocks.FARMLAND, Blocks.ICE, Blocks.HONEY_BLOCK, Blocks.GLASS, Blocks.DIRT_PATH, Blocks.WATER);
    static final int[] STEP_BLOCKS = where(s -> {
        Block k = s.getBlock();
        return k instanceof SlabBlock || k instanceof StairBlock || k instanceof SnowLayerBlock || k instanceof CarpetBlock || k instanceof TrapDoorBlock
                || k == Blocks.FARMLAND || k == Blocks.SOUL_SAND || k == Blocks.DIRT_PATH || k == Blocks.CHEST || k == Blocks.LILY_PAD || k == Blocks.OAK_FENCE
                || k == Blocks.HONEY_BLOCK || k == Blocks.ICE || k == Blocks.GLASS || k == Blocks.STONE || k == Blocks.OAK_PLANKS || k == Blocks.COBBLESTONE_WALL
                || k == Blocks.SCAFFOLDING || k == Blocks.WATER;
    });
    static final int[] OBSTACLES = where(s -> {
        Block k = s.getBlock();
        return k == Blocks.STONE || k == Blocks.DIRT || k == Blocks.GLASS || k == Blocks.ICE || k == Blocks.INFESTED_STONE || k == Blocks.CHEST || k == Blocks.OBSIDIAN
                || k instanceof FallingBlock || k instanceof DoorBlock || k instanceof FenceGateBlock || k instanceof TrapDoorBlock || k == Blocks.COBWEB
                || k == Blocks.OAK_FENCE || k == Blocks.COBBLESTONE_WALL || k == Blocks.VINE || k == Blocks.LADDER || k == Blocks.CRAFTING_TABLE || k == Blocks.FURNACE
                || k == Blocks.BEDROCK || k instanceof LeavesBlock || k == Blocks.SNOW || k == Blocks.WHITE_CARPET;
    });
    /** Anything that may sit at the feet or head: air most of the time, then plants, doors, fluids, hazards. */
    static final int[] CLUTTER = where(s -> {
        Block k = s.getBlock();
        return k instanceof AirBlock || k == Blocks.SHORT_GRASS || k == Blocks.TALL_GRASS || k == Blocks.TORCH || k == Blocks.COBWEB || k == Blocks.VINE || k == Blocks.LADDER
                || k instanceof DoorBlock || k instanceof FenceGateBlock || k instanceof TrapDoorBlock || k instanceof SnowLayerBlock || k instanceof CarpetBlock
                || !s.getFluidState().isEmpty() || k == Blocks.SWEET_BERRY_BUSH || k == Blocks.FIRE || k == Blocks.REDSTONE_WIRE || k == Blocks.RAIL || k == Blocks.CACTUS
                || k == Blocks.TRIPWIRE || k == Blocks.LILY_PAD || k == Blocks.OAK_FENCE || k == Blocks.BUBBLE_COLUMN || k == Blocks.POWDER_SNOW;
    });
    static final int[] NON_AIR = where(s -> !(s.getBlock() instanceof AirBlock));

    static int pick(Random r, int[] a) {
        return a[r.nextInt(a.length)];
    }

    static int b(boolean v) {
        return v ? 1 : 0;
    }

    static String hx(double d) {
        return Long.toHexString(Double.doubleToLongBits(d));
    }

    static String fluid(FluidState f) {
        Fluid t = f.getType();
        if (t == Fluids.WATER) return "water";
        if (t == Fluids.FLOWING_WATER) return "flowing_water";
        if (t == Fluids.LAVA) return "lava";
        if (t == Fluids.FLOWING_LAVA) return "flowing_lava";
        return "none";
    }

    static boolean shapeFull(BlockState s) {
        try {
            return Block.isShapeFullBlock(s.getCollisionShape(null, null));
        } catch (Exception e) {
            return false;
        }
    }

    static boolean topSturdy(BlockState s) {
        try {
            return s.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.UP);
        } catch (Exception e) {
            return false;
        }
    }

    /** Raw game facts only, exactly the fields of the Rust State. Tool strengths come last and only for palette states. */
    static String facts(String tag, int i, BlockState s, ItemStack pick, ItemStack hand) {
        Block k = s.getBlock();
        FluidState f = s.getFluidState();
        StringBuilder o = new StringBuilder(tag).append(' ').append(i).append(' ').append(BuiltInRegistries.BLOCK.getKey(k).getPath());
        String[][] props = {
                {"air", "" + b(k instanceof AirBlock)}, {"amethyst_cluster", "" + b(k instanceof AmethystClusterBlock)}, {"azalea", "" + b(k instanceof AzaleaBlock)},
                {"bamboo_stalk", "" + b(k instanceof BambooStalkBlock)}, {"base_fire", "" + b(k instanceof BaseFireBlock)}, {"carpet", "" + b(k instanceof CarpetBlock)},
                {"cauldron", "" + b(k instanceof CauldronBlock)}, {"door", "" + b(k instanceof DoorBlock)}, {"falling", "" + b(k instanceof FallingBlock)},
                {"gate", "" + b(k instanceof FenceGateBlock)}, {"infested", "" + b(k instanceof InfestedBlock)}, {"leaves", "" + b(k instanceof LeavesBlock)},
                {"lily", "" + b(k instanceof WaterlilyBlock)}, {"liquid_block", "" + b(k instanceof LiquidBlock)}, {"moving_piston", "" + b(k instanceof MovingPistonBlock)},
                {"pointed_dripstone", "" + b(k instanceof PointedDripstoneBlock)}, {"scaffolding", "" + b(k instanceof ScaffoldingBlock)}, {"shulker", "" + b(k instanceof ShulkerBoxBlock)},
                {"skull_any", "" + b(k instanceof AbstractSkullBlock)}, {"skull_floor", "" + b(k instanceof SkullBlock)}, {"slab", "" + b(k instanceof SlabBlock)},
                {"snow", "" + b(k instanceof SnowLayerBlock)}, {"stair", "" + b(k instanceof StairBlock)}, {"stained_glass", "" + b(k instanceof StainedGlassBlock)},
                {"trapdoor", "" + b(k instanceof TrapDoorBlock)}, {"end_portal_block", "" + b(k instanceof EndPortalBlock)},
                {"wooden_door", "" + b(k instanceof DoorBlock && DoorBlock.isWoodenDoor(s))}, {"pathfindable", "" + b(s.isPathfindable(PathComputationType.LAND))},
                {"shape_full", "" + b(shapeFull(s))},
                {"waterlogged", "" + b(s.hasProperty(BlockStateProperties.WATERLOGGED) && s.getValue(BlockStateProperties.WATERLOGGED))},
                {"top_sturdy", "" + b(topSturdy(s))},
                {"replaceable", "" + b(s.canBeReplaced())}, {"falling_free", "" + b(FallingBlock.isFree(s))}, {"frost_melt", "" + b(s == FrostedIceBlock.meltsInto())},
                {"fluid", fluid(f)}, {"fluid_empty", "" + b(f.isEmpty())}, {"fluid_amount", "" + f.getType().getAmount(f)}, {"fluid_source", "" + b(f.isSource())},
                {"fluid_flowing_kind", "" + b(f.getType() instanceof FlowingFluid)},
                {"level", k instanceof LiquidBlock ? "" + s.getValue(LiquidBlock.LEVEL) : "-1"},
                {"slab_type", k instanceof SlabBlock ? s.getValue(SlabBlock.TYPE).name() : "-"},
                {"stair_half", k instanceof StairBlock ? s.getValue(StairBlock.HALF).name() : "-"},
                {"stair_shape", k instanceof StairBlock ? s.getValue(StairBlock.SHAPE).name() : "-"},
                {"trap_open", k instanceof TrapDoorBlock ? "" + b(s.getValue(TrapDoorBlock.OPEN)) : "0"},
                {"trap_half", k instanceof TrapDoorBlock ? s.getValue(TrapDoorBlock.HALF).name() : "-"},
                {"layers", k instanceof SnowLayerBlock ? "" + s.getValue(SnowLayerBlock.LAYERS) : "0"},
                {"door_facing", k instanceof DoorBlock ? s.getValue(DoorBlock.FACING).name() : "-"},
                {"door_hinge_left", k instanceof DoorBlock ? "" + b(s.getValue(DoorBlock.HINGE) == DoorHingeSide.LEFT) : "0"},
        };
        for (String[] kv : props) {
            o.append(' ').append(kv[0]).append('=').append(kv[1]);
        }
        if (pick != null) {
            o.append(" str_pick=").append(hx(strength(pick, s))).append(" str_hand=").append(hx(strength(hand, s)));
        }
        return o.toString();
    }

    static String t(Ternary t) {
        return t.name().substring(0, 1);
    }

    /** Upstream's answers for one state under the settings applied at the moment. 13 characters. */
    static String answers(BlockState s) {
        Block k = s.getBlock();
        return t(MovementHelper.canWalkThroughBlockState(s)) + t(MovementHelper.canWalkOnBlockState(s)) + t(MovementHelper.fullyPassableBlockState(s))
                + b(MovementHelper.avoidWalkingInto(s)) + b(MovementHelper.isBlockNormalCube(s)) + b(MovementHelper.isWaterloggedSolidTop(s))
                + b(MovementHelper.isClimbable(k)) + b(MovementHelper.isBottomSlab(s)) + b(MovementHelper.isWater(s)) + b(MovementHelper.isLava(s))
                + b(MovementHelper.isLiquid(s)) + b(MovementHelper.possiblyFlowing(s)) + b(MovementHelper.isTransparent(k));
    }

    // ------------------------------------------------------------------------------------------------------------
    // Grid generation. Modes 0 to 6 are random fills. Modes 7 to 10 build a structure around the start in all four
    // directions at once, so that every movement from the start has something to decide.
    // ------------------------------------------------------------------------------------------------------------

    static final int MODES = 11;
    static final int[] DX = {0, 0, 1, -1}, DZ = {-1, 1, 0, 0};

    static final class G {
        final int[] c = new int[W * H * D];

        void set(int x, int y, int z, int v) {
            if (x >= 0 && x < W && y >= 0 && y < H && z >= 0 && z < D) {
                c[(y * D + z) * W + x] = v;
            }
        }

        /** Arm d, k blocks out from the start, s blocks to the side. */
        void arm(int d, int k, int s, int y, int v) {
            int px = DZ[d] != 0 ? 1 : 0, pz = DX[d] != 0 ? 1 : 0;
            set(SX + DX[d] * k + px * s, y, SZ + DZ[d] * k + pz * s, v);
        }
    }

    static int[] grid(Random r, int mode, int y) {
        G g = new G();
        int n = PAL.size();
        switch (mode) {
            case 0, 1, 2, 3, 4, 5, 6 -> {
                for (int x = 0; x < W; x++) {
                    for (int z = 0; z < D; z++) {
                        int floor = 3 + r.nextInt(mode == 0 ? 2 : 6);
                        for (int yy = 0; yy < H; yy++) {
                            int v;
                            boolean noise = mode == 3 ? r.nextInt(100) < 70 : r.nextInt(100) < 12;
                            if (mode == 4 && r.nextInt(100) < 60) {
                                v = pick(r, FLUIDS);
                            } else if (mode == 5) {
                                // mostly water: sources above sources, and flowing water, for the walk-on-water rules
                                v = r.nextInt(100) < 85 ? pick(r, WATERS) : (yy < 2 ? STONE : pick(r, new int[]{AIR, AIR, ix(Blocks.LILY_PAD), ix(Blocks.WHITE_CARPET)}));
                            } else if (mode == 6) {
                                // mostly open air down to the bottom of the grid, for falls that reach the world's lowest layer
                                v = r.nextInt(100) < 4 ? r.nextInt(n) : AIR;
                            } else if (noise) {
                                v = r.nextInt(n);
                            } else if (yy < floor) {
                                v = r.nextBoolean() ? STONE : DIRT;
                            } else {
                                v = AIR;
                            }
                            g.set(x, yy, z, v);
                        }
                    }
                }
            }
            case 7 -> parkour(r, g, y);
            case 8 -> stairs(r, g, y);
            case 9 -> vertical(r, g, y);
            case 10 -> diagonals(r, g, y);
            default -> throw new IllegalStateException();
        }
        if (mode >= 7 && r.nextInt(100) < 35) {
            // a little noise on top, away from the start itself
            int density = 1 + r.nextInt(3);
            for (int i = 0; i < W * H * D; i++) {
                if (r.nextInt(100) < density) {
                    g.c[i] = r.nextInt(n);
                }
            }
        }
        return g.c;
    }

    static int floorBlock(Random r) {
        int k = r.nextInt(100);
        return k < 35 ? STONE : (k < 55 ? pick(r, KEY_FLOORS) : (k < 85 ? pick(r, SPECIAL_FLOORS) : pick(r, NON_AIR)));
    }

    /** Flat corridors with gaps, for the parkour movement: landings, ascending landings, placing, walls, bad take-off and landing blocks. */
    static void parkour(Random r, G g, int y) {
        g.set(SX, y - 1, SZ, r.nextInt(100) < 15 ? pick(r, WATERLOGGED) : floorBlock(r));
        if (r.nextInt(100) < 10) {
            g.set(SX, y, SZ, pick(r, CLUTTER));
        }
        if (r.nextInt(100) < 10) {
            g.set(SX, y + 2, SZ, pick(r, OBSTACLES));
        }
        for (int d = 0; d < 4; d++) {
            int gap = r.nextInt(5);
            int kind = r.nextInt(8);
            int land = floorBlock(r);
            int lk = gap + 1;
            for (int k = 1; k <= gap; k++) {
                g.arm(d, k, 0, y - 1, r.nextInt(100) < 85 ? AIR : pick(r, HAZARDS));
            }
            switch (kind) {
                case 0, 1, 5, 6 -> g.arm(d, lk, 0, y - 1, land);
                case 2 -> {
                    g.arm(d, lk, 0, y, land);
                    g.arm(d, lk, 0, y - 1, STONE);
                }
                case 3 -> {
                    // placing: the landing floor is missing, with support at the side, below, or nowhere
                    int s = r.nextInt(4);
                    if (s == 0) g.arm(d, lk, 1, y - 1, STONE);
                    if (s == 1) g.arm(d, lk, 0, y - 2, STONE);
                    if (s == 2) g.arm(d, lk, -1, y - 1, pick(r, SOLIDS));
                    if (r.nextInt(100) < 30) g.arm(d, lk, 0, y - 1, pick(r, new int[]{ix(Blocks.SHORT_GRASS), ix(Blocks.TALL_GRASS), ix(Blocks.LARGE_FERN), ix(Blocks.WATER), ix(Blocks.LAVA), SNOWS[0], SNOWS[1]}));
                }
                case 4 -> g.arm(d, lk, 0, y - 2, land);
                default -> g.arm(d, lk, 0, y - 1, land);
            }
            if (kind == 5 || r.nextInt(100) < 20) {
                // a wall somewhere on the way, at feet, head or above
                g.arm(d, 1 + r.nextInt(lk), 0, y + r.nextInt(4), pick(r, OBSTACLES));
            }
            if (kind == 6 || r.nextInt(100) < 15) {
                g.arm(d, lk + 1, 0, y + r.nextInt(2), pick(r, HAZARDS));
            }
            if (r.nextInt(100) < 15) {
                g.arm(d, lk, 0, y + 1 + r.nextInt(3), pick(r, CLUTTER));
            }
            for (int k = lk + 1; k <= 5; k++) {
                if (r.nextInt(100) < 70) g.arm(d, k, 0, y - 1, STONE);
            }
        }
    }

    /** Staircases and slabs, for ascend: steps of every kind of block, headroom, blocks to break, blocks to place. */
    static void stairs(Random r, G g, int y) {
        g.set(SX, y - 1, SZ, r.nextInt(100) < 55 ? STONE : (r.nextInt(100) < 40 ? pick(r, SLABS) : pick(r, SPECIAL_FLOORS)));
        if (r.nextInt(100) < 15) {
            // a stack of falling blocks over the start
            g.set(SX, y + 1, SZ, pick(r, FALLERS));
            g.set(SX, y + 2, SZ, pick(r, FALLERS));
            g.set(SX, y + 3, SZ, pick(r, FALLERS));
        }
        if (r.nextInt(100) < 25) {
            g.set(SX, y + 2, SZ, pick(r, OBSTACLES));
        }
        if (r.nextInt(100) < 10) {
            g.set(SX, y, SZ, pick(r, CLUTTER));
        }
        if (r.nextInt(100) < 10) {
            // a pond under the start, with a lily pad or a carpet on it
            g.set(SX, y - 1, SZ, pick(r, WATERS));
            g.set(SX, y, SZ, pick(r, new int[]{ix(Blocks.LILY_PAD), ix(Blocks.WHITE_CARPET), AIR}));
        }
        for (int d = 0; d < 4; d++) {
            int up = r.nextInt(4);
            int h = 0;
            for (int k = 1; k <= 5; k++) {
                if (k <= up) {
                    h++;
                } else if (r.nextInt(100) < 35) {
                    h--;
                }
                int bt = r.nextInt(100) < 35 ? STONE : pick(r, STEP_BLOCKS);
                int roll = r.nextInt(100);
                if (roll < 12) {
                    // placing: the step is missing, with support at the side, below, or nowhere
                    int s = r.nextInt(3);
                    if (s == 0) g.arm(d, k, 1, y + h - 1, STONE);
                    if (s == 1) g.arm(d, k, 0, y + h - 2, STONE);
                    if (r.nextInt(100) < 25) g.arm(d, k, 0, y + h - 1, pick(r, new int[]{ix(Blocks.SHORT_GRASS), ix(Blocks.TALL_GRASS), ix(Blocks.LARGE_FERN), SNOWS[0], ix(Blocks.WATER)}));
                } else {
                    g.arm(d, k, 0, y + h - 1, bt);
                }
                if (roll >= 12 && roll < 28) {
                    // something in the way at the feet or head: to break, or to refuse
                    g.arm(d, k, 0, y + h + r.nextInt(2), pick(r, OBSTACLES));
                    if (r.nextInt(100) < 30) g.arm(d, k, 0, y + h + 1 + r.nextInt(2), pick(r, FALLERS));
                } else if (roll >= 28 && roll < 36) {
                    g.arm(d, k, 0, y + h, pick(r, CLUTTER));
                }
            }
            if (r.nextInt(100) < 25) {
                pit(r, g, d, y);
            }
        }
    }

    /** A pit or a fall one block out from the start: a drop of 1 to 5 blocks onto something, with things in the way above. */
    static void pit(Random r, G g, int d, int y) {
        int depth = r.nextInt(5);
        for (int hh = y - depth - 2; hh <= y + 3; hh++) {
            g.arm(d, 1, 0, hh, AIR);
        }
        int pf = r.nextInt(6);
        g.arm(d, 1, 0, y - depth - 2, pf == 0 ? STONE : pf == 1 ? pick(r, WATERS) : pf == 2 ? pick(r, CLIMBS) : pf == 3 ? pick(r, SLABS) : pf == 4 ? pick(r, LAVAS) : floorBlock(r));
        if (r.nextInt(100) < 40) g.arm(d, 1, 0, y + r.nextInt(2), pick(r, OBSTACLES));
        if (r.nextInt(100) < 40) g.arm(d, 1, 0, y + 2, pick(r, FALLERS));
        if (r.nextInt(100) < 25) g.arm(d, 1, 0, y - 1 - r.nextInt(depth + 1), pick(r, r.nextBoolean() ? CLIMBS : WATERS));
    }

    /** Ladders, vines, water and falling blocks in a column through the start, for pillar and downward. */
    static void vertical(Random r, G g, int y) {
        int ck = r.nextInt(6);
        int fallFrom = 1 + r.nextInt(2);
        for (int h = -4; h <= 4; h++) {
            int v;
            switch (ck) {
                case 0 -> v = r.nextInt(100) < 80 ? ix(Blocks.LADDER) : (r.nextBoolean() ? AIR : pick(r, CLIMBS));
                case 1 -> v = r.nextInt(100) < 80 ? pick(r, CLIMBS) : AIR;
                case 2 -> v = r.nextInt(100) < 80 ? pick(r, WATERS) : (r.nextBoolean() ? AIR : ix(Blocks.LILY_PAD));
                case 3 -> v = h >= fallFrom ? pick(r, FALLERS) : (r.nextInt(100) < 60 ? AIR : pick(r, OBSTACLES));
                case 4 -> v = pick(r, CLUTTER);
                default -> v = r.nextInt(100) < 50 ? pick(r, LAVAS) : (r.nextBoolean() ? pick(r, WATERS) : pick(r, SOLIDS));
            }
            g.set(SX, y + h, SZ, v);
        }
        if (r.nextInt(100) < 35) {
            g.set(SX, y + 2, SZ, pick(r, OBSTACLES));
        }
        // the floor below the start, and the blocks two and three below it
        int below = r.nextInt(8);
        g.set(SX, y - 1, SZ, switch (below) {
            case 0 -> ix(Blocks.LADDER);
            case 1 -> pick(r, SLABS);
            case 2 -> pick(r, STAIRS);
            case 3 -> AIR;
            case 4 -> pick(r, WATERS);
            default -> STONE;
        });
        g.set(SX, y - 2, SZ, r.nextInt(100) < 70 ? pick(r, SOLIDS) : pick(r, NON_AIR));
        if (r.nextInt(100) < 50) g.set(SX, y - 3, SZ, pick(r, SOLIDS));
        for (int d = 0; d < 4; d++) {
            for (int h = -1; h <= 2; h++) {
                if (r.nextInt(100) < 50) {
                    g.arm(d, 1, 0, y + h, r.nextInt(100) < 85 ? STONE : pick(r, new int[]{ix(Blocks.GLASS), ix(Blocks.WHITE_STAINED_GLASS), ix(Blocks.ICE), ix(Blocks.OAK_LEAVES)}));
                }
            }
        }
    }

    /** The four diagonals with their two corners each: doors, magma, lava, water and more, at the feet and head and under the corners. */
    static void diagonals(Random r, G g, int y) {
        g.set(SX, y - 1, SZ, r.nextInt(100) < 55 ? STONE : (r.nextBoolean() ? pick(r, KEY_FLOORS) : pick(r, SPECIAL_FLOORS)));
        if (r.nextInt(100) < 15) {
            g.set(SX, y + 2, SZ, pick(r, OBSTACLES));
        }
        int[] corner = concat(DOORS, DOORS, GATES, TRAPS, HAZARDS, CLIMBS, FALLERS, FALLERS, new int[]{AIR, AIR, AIR, AIR, AIR, AIR, STONE, STONE, ix(Blocks.GLASS)});
        for (int q = 0; q < 4; q++) {
            int dx = (q & 1) == 0 ? 1 : -1, dz = (q & 2) == 0 ? 1 : -1;
            int[][] cs = {{SX + dx, SZ}, {SX, SZ + dz}};
            for (int[] c : cs) {
                int k = r.nextInt(100);
                g.set(c[0], y - 1, c[1], k < 60 ? STONE : (k < 85 ? pick(r, SPECIAL_FLOORS) : AIR));
                if (r.nextInt(100) < 45) g.set(c[0], y, c[1], pick(r, corner));
                if (r.nextInt(100) < 30) g.set(c[0], y + 1, c[1], pick(r, corner));
                if (r.nextInt(100) < 20) g.set(c[0], y + 2, c[1], r.nextBoolean() ? pick(r, FALLERS) : pick(r, OBSTACLES));
                int fall = r.nextInt(100);
                if (fall < 20) {
                    // something to break at the feet with a falling block over it
                    g.set(c[0], y, c[1], r.nextBoolean() ? pick(r, OBSTACLES) : ix(Blocks.SWEET_BERRY_BUSH));
                    g.set(c[0], y + 1, c[1], pick(r, FALLERS));
                    g.set(c[0], y + 2, c[1], pick(r, FALLERS));
                } else if (fall < 40) {
                    // free feet, something to break at the head with a falling block over it
                    g.set(c[0], y, c[1], AIR);
                    g.set(c[0], y + 1, c[1], r.nextBoolean() ? pick(r, OBSTACLES) : ix(Blocks.SWEET_BERRY_BUSH));
                    g.set(c[0], y + 2, c[1], pick(r, FALLERS));
                }
            }
            int dxp = SX + dx, dzp = SZ + dz;
            int kind = r.nextInt(5);
            // flat, ascending, descending, placing, nothing
            switch (kind) {
                case 0 -> g.set(dxp, y - 1, dzp, floorBlock(r));
                case 1 -> {
                    g.set(dxp, y, dzp, floorBlock(r));
                    g.set(dxp, y - 1, dzp, STONE);
                    if (r.nextInt(100) < 35) g.set(dxp, y + 1, dzp, pick(r, DOORS));
                }
                case 2 -> g.set(dxp, y - 2, dzp, floorBlock(r));
                case 3 -> {
                    g.set(dxp + dx, y - 1, dzp, STONE);
                }
                default -> {
                }
            }
            if (r.nextInt(100) < 25) g.set(dxp, y + r.nextInt(3), dzp, pick(r, corner));
            if (r.nextInt(100) < 15) g.set(dxp, y - 2 - r.nextInt(2), dzp, pick(r, LAVAS));
        }
        if (r.nextInt(100) < 25) {
            // a clean start: everything round the diagonals is clear, only the floors vary
            for (int q = 0; q < 4; q++) {
                int dx = (q & 1) == 0 ? 1 : -1, dz = (q & 2) == 0 ? 1 : -1;
                int[][] cs = {{SX + dx, SZ}, {SX, SZ + dz}, {SX + dx, SZ + dz}};
                for (int[] c : cs) {
                    for (int h = 0; h <= 2; h++) g.set(c[0], y + h, c[1], AIR);
                    g.set(c[0], y - 1, c[1], r.nextInt(100) < 70 ? STONE : pick(r, KEY_FLOORS));
                }
                if (r.nextInt(100) < 25) {
                    // an ascending diagonal with a door in its head cell
                    g.set(SX + dx, y, SZ + dz, STONE);
                    g.set(SX + dx, y + 1, SZ + dz, pick(r, DOORS));
                }
            }
            for (int h = 0; h <= 2; h++) g.set(SX, y + h, SZ, AIR);
            g.set(SX, y - 1, SZ, pick(r, KEY_FLOORS));
        }
        if (r.nextInt(100) < 20) {
            // a pond: the start in the water, with water sources in the floor of the diagonals
            g.set(SX, y - 1, SZ, pick(r, WATERS));
            g.set(SX, y, SZ, pick(r, WATERS));
            for (int q = 0; q < 4; q++) {
                int dx = (q & 1) == 0 ? 1 : -1, dz = (q & 2) == 0 ? 1 : -1;
                if (r.nextInt(100) < 70) g.set(SX + dx, y - 1, SZ + dz, pick(r, WATERS));
                if (r.nextInt(100) < 50) g.set(SX + dx, y, SZ + dz, AIR);
            }
        }
    }

    static int[] concat(int[]... as) {
        int n = 0;
        for (int[] a : as) n += a.length;
        int[] o = new int[n];
        int i = 0;
        for (int[] a : as) {
            System.arraycopy(a, 0, o, i, a.length);
            i += a.length;
        }
        return o;
    }

    // ------------------------------------------------------------------------------------------------------------
    // Goals
    // ------------------------------------------------------------------------------------------------------------

    static final class Gen {
        Goal goal;
        String spec;
        /** False when evaluating heuristic() would scan a box too big for the harness (or whose loop would not end). */
        boolean h0 = true;
        final List<int[]> anchors = new ArrayList<>();
    }

    static int rc(Random r, boolean extreme) {
        int k = r.nextInt(20);
        if (k < 9) return r.nextInt(41) - 20;
        if (k < 12) return r.nextInt(2001) - 1000;
        if (k < 16 || !extreme) return (r.nextBoolean() ? 1 : -1) * (29_999_000 + r.nextInt(2000)) + (k < 16 ? 0 : r.nextInt(5));
        switch (r.nextInt(6)) {
            case 0:
                return Integer.MAX_VALUE;
            case 1:
                return Integer.MAX_VALUE - r.nextInt(5);
            case 2:
                return Integer.MIN_VALUE;
            case 3:
                return Integer.MIN_VALUE + r.nextInt(5);
            case 4:
                return (1 << 30) + r.nextInt(3) - 1;
            default:
                return -(1 << 30) - r.nextInt(3);
        }
    }

    static final String[] CLASSES = {"block", "xz", "ylevel", "twoblocks", "gettoblock", "near", "axis", "strict", "runaway", "composite", "inverted"};

    static Gen gen(Random r, int cls, int depth) {
        Gen o = new Gen();
        int x = rc(r, true), y = rc(r, true), z = rc(r, true);
        switch (cls) {
            case 0 -> {
                o.goal = new GoalBlock(x, y, z);
                o.spec = "block(" + x + "," + y + "," + z + ")";
                o.anchors.add(new int[]{x, y, z});
            }
            case 1 -> {
                o.goal = new GoalXZ(x, z);
                o.spec = "xz(" + x + "," + z + ")";
                o.anchors.add(new int[]{x, y, z});
            }
            case 2 -> {
                o.goal = new GoalYLevel(y);
                o.spec = "ylevel(" + y + ")";
                o.anchors.add(new int[]{x, y, z});
            }
            case 3 -> {
                o.goal = new GoalTwoBlocks(new BlockPos(x, y, z));
                o.spec = "twoblocks(" + x + "," + y + "," + z + ")";
                o.anchors.add(new int[]{x, y, z});
            }
            case 4 -> {
                o.goal = new GoalGetToBlock(new BlockPos(x, y, z));
                o.spec = "gettoblock(" + x + "," + y + "," + z + ")";
                o.anchors.add(new int[]{x, y, z});
            }
            case 5 -> {
                // small ranges, except in the overflow flavour (no heuristic() there, it would scan a huge box)
                int range;
                if (r.nextInt(100) < 25) {
                    int[] big = {46340, 46341, 65535, 65536, 100000, -46341, 1 << 20, Integer.MAX_VALUE, Integer.MIN_VALUE};
                    range = big[r.nextInt(big.length)];
                    o.h0 = false;
                } else {
                    range = r.nextInt(8) - 1;
                }
                o.goal = new GoalNear(new BlockPos(x, y, z), range);
                o.spec = "near(" + x + "," + y + "," + z + "," + range + ")";
                o.anchors.add(new int[]{x, y, z});
            }
            case 6 -> {
                o.goal = new GoalAxis();
                o.spec = "axis()";
                o.anchors.add(new int[]{0, 120, 0});
                o.anchors.add(new int[]{r.nextInt(9) - 4, 7, r.nextInt(9) - 4});
            }
            case 7 -> {
                Direction[] ds = {Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST};
                Direction dir = ds[r.nextInt(4)];
                o.goal = new GoalStrictDirection(new BlockPos(x, y, z), dir);
                o.spec = "strict(" + x + "," + y + "," + z + "," + dir.getStepX() + "," + dir.getStepZ() + ")";
                o.anchors.add(new int[]{x, y, z});
            }
            case 8 -> {
                int count = 1 + r.nextInt(3);
                boolean far = r.nextInt(100) < 20;
                if (!far) {
                    // heuristic() loops from the lowest to the highest corner of a box round the points, so keep them clear of the int limits
                    x = rc(r, false);
                    y = rc(r, false);
                    z = rc(r, false);
                }
                double dist;
                if (far) {
                    double[] big = {1e5, 46341.0, 1e10, Double.NaN, -3.0, Double.POSITIVE_INFINITY, 65536.5};
                    dist = big[r.nextInt(big.length)];
                    o.h0 = false;
                } else {
                    dist = r.nextInt(7) + new double[]{0.0, 0.5, 0.8, 0.9}[r.nextInt(4)];
                }
                Integer maintain = r.nextBoolean() ? null : Integer.valueOf(rc(r, false));
                BlockPos[] from = new BlockPos[count];
                StringBuilder sb = new StringBuilder("runaway(").append(hx(dist)).append(',').append(maintain == null ? "n" : "" + maintain);
                for (int i = 0; i < count; i++) {
                    // close together: the box that heuristic() scans spans all of them
                    int fx = far ? rc(r, true) : x + r.nextInt(7) - 3, fy = far ? rc(r, true) : y + r.nextInt(7) - 3, fz = far ? rc(r, true) : z + r.nextInt(7) - 3;
                    from[i] = new BlockPos(fx, fy, fz);
                    sb.append(',').append(fx).append(',').append(fy).append(',').append(fz);
                    o.anchors.add(new int[]{fx, fy, fz});
                }
                o.goal = new GoalRunAway(dist, maintain, from);
                o.spec = sb.append(')').toString();
            }
            case 9, 10 -> {
                if (depth >= 3) {
                    return gen(r, r.nextInt(9), depth);
                }
                if (cls == 10) {
                    Gen in = gen(r, r.nextInt(MODES_GOALS), depth + 1);
                    o.goal = new GoalInverted(in.goal);
                    o.spec = "inverted(" + in.spec + ")";
                    o.h0 = in.h0;
                    o.anchors.addAll(in.anchors);
                } else {
                    int count = r.nextInt(5) == 0 ? r.nextInt(2) : 2 + r.nextInt(2);
                    Goal[] gs = new Goal[count];
                    StringBuilder sb = new StringBuilder("composite(");
                    for (int i = 0; i < count; i++) {
                        Gen in = gen(r, r.nextInt(MODES_GOALS), depth + 1);
                        gs[i] = in.goal;
                        o.h0 &= in.h0;
                        o.anchors.addAll(in.anchors);
                        if (i > 0) sb.append(',');
                        sb.append(in.spec);
                    }
                    o.goal = new GoalComposite(gs);
                    o.spec = sb.append(')').toString();
                }
            }
            default -> throw new IllegalStateException();
        }
        return o;
    }

    static final int MODES_GOALS = 11;

    // ------------------------------------------------------------------------------------------------------------
    // Block tags: a bare Bootstrap binds none, so tools would see no mining rules.
    // ------------------------------------------------------------------------------------------------------------

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

    // ------------------------------------------------------------------------------------------------------------
    // The dump
    // ------------------------------------------------------------------------------------------------------------

    static String preds(CalculationContext ctx, int x, int y, int z) {
        BlockState st = ctx.get(x, y, z);
        StringBuilder o = new StringBuilder();
        o.append(b(MovementHelper.canWalkThrough(ctx, x, y, z)));
        o.append(b(MovementHelper.canWalkOn(ctx, x, y, z)));
        o.append(b(MovementHelper.fullyPassable(ctx, x, y, z)));
        o.append(b(MovementHelper.canPlaceAgainst(ctx, x, y, z)));
        o.append(b(MovementHelper.mustBeSolidToWalkOn(ctx, x, y, z, st)));
        o.append(b(MovementHelper.isReplaceable(x, y, z, st, ctx.bsi)));
        o.append(b(MovementHelper.canUseFrostWalker(ctx, st)));
        o.append(b(MovementHelper.avoidBreaking(ctx, x, y, z, st)));
        o.append(b(MovementHelper.avoidWalkingInto(st)));
        o.append(b(MovementHelper.isFlowing(x, y, z, st, ctx.bsi)));
        o.append(' ').append(hx(MovementHelper.getMiningDurationTicks(ctx, x, y, z, false)));
        o.append(' ').append(hx(MovementHelper.getMiningDurationTicks(ctx, x, y, z, true)));
        o.append(' ').append(hx(ctx.costOfPlacingAt(x, y, z, st)));
        return o.toString();
    }

    @Test
    public void dump() throws Exception {
        System.out.println("bound block tags: " + bindBlockTags());
        Settings s = Baritone.settings();
        s.considerPotionEffects.value = false;
        air = Blocks.AIR.defaultBlockState();
        int count = Integer.parseInt(System.getenv().getOrDefault("SPIKE_GRIDS", "5000"));
        Path out = Path.of(System.getenv("SPIKE_OUT"));
        ItemStack pick = new ItemStack(Items.DIAMOND_PICKAXE);
        ItemStack hand = ItemStack.EMPTY;
        WorldBorder wb = new WorldBorder();
        BetterWorldBorder border = new BetterWorldBorder(wb);
        Random r = new Random(0x5EEDL);
        Moves[] moves = Moves.values();
        try (BufferedWriter w = Files.newBufferedWriter(out)) {
            w.write("B " + wb.getMinX() + " " + wb.getMaxX() + " " + wb.getMinZ() + " " + wb.getMaxZ() + " " + W + " " + H + " " + D + " " + SX + " " + SZ + "\n");
            for (Moves m : moves) {
                w.write("M " + m.ordinal() + " " + m.name() + " " + m.xOffset + " " + m.yOffset + " " + m.zOffset + " " + (m.dynamicXZ ? 1 : 0) + " " + (m.dynamicY ? 1 : 0) + "\n");
            }
            for (int i = 0; i < PAL.size(); i++) {
                w.write(facts("S", i, PAL.get(i), pick, hand) + "\n");
            }
            for (int i = 0; i < CONFIGS.size(); i++) {
                w.write(cline(i, CONFIGS.get(i)) + "\n");
            }

            // the fall table
            StringBuilder f = new StringBuilder("F");
            for (double v : ActionCosts.FALL_N_BLOCKS_COST) {
                f.append(' ').append(hx(v));
            }
            f.append(' ').append(hx(ActionCosts.JUMP_ONE_BLOCK_COST));
            w.write(f + "\n");

            // every block state of the game, and upstream's answers under two configurations
            w.write("Z 0 " + REG_ALT + "\n");
            int total = Block.BLOCK_STATE_REGISTRY.size();
            for (int id = 0; id < total; id++) {
                BlockState st = Block.BLOCK_STATE_REGISTRY.byId(id);
                apply(CONFIGS.get(0));
                String a0 = answers(st);
                apply(CONFIGS.get(REG_ALT));
                String a1 = answers(st);
                w.write(facts("A", id, st, null, null) + " r0=" + a0 + " r1=" + a1 + "\n");
            }

            // goals
            int goalCount = Math.max(300, count);
            Random gr = new Random(0x60A15L);
            int[] cfgs = {0, GOAL_CFG};
            for (int cls = 0; cls < CLASSES.length; cls++) {
                for (int i = 0; i < goalCount; i++) {
                    Gen gen = gen(gr, cls, 0);
                    int gc = cfgs[gr.nextInt(2)];
                    apply(CONFIGS.get(gc));
                    StringBuilder q = new StringBuilder("Q ").append(gc).append(' ').append(gen.spec).append(' ');
                    q.append(gen.h0 ? hx(gen.goal.heuristic()) : "-");
                    for (int p = 0; p < 6; p++) {
                        int px, py, pz;
                        if (p < 3 && !gen.anchors.isEmpty()) {
                            int[] a = gen.anchors.get(gr.nextInt(gen.anchors.size()));
                            int spread = p == 0 ? 0 : 3;
                            px = a[0] + (spread == 0 ? 0 : gr.nextInt(2 * spread + 1) - spread);
                            py = a[1] + (spread == 0 ? 0 : gr.nextInt(2 * spread + 1) - spread);
                            pz = a[2] + (spread == 0 ? 0 : gr.nextInt(2 * spread + 1) - spread);
                        } else {
                            px = rc(gr, true);
                            py = rc(gr, true);
                            pz = rc(gr, true);
                        }
                        q.append(' ').append(px).append(' ').append(py).append(' ').append(pz).append(' ').append(b(gen.goal.isInGoal(px, py, pz))).append(' ').append(hx(gen.goal.heuristic(px, py, pz)));
                    }
                    w.write(q + "\n");
                }
            }

            // grids
            BlockState[] states = new BlockState[W * H * D];
            MutableMoveResult res = new MutableMoveResult();
            StringBuilder line = new StringBuilder();
            for (int gi = 0; gi < count; gi++) {
                BetterWorldBorder gridBorder = border;
                if (r.nextInt(100) < 30) {
                    // a small world border round the start, so that placing runs into it
                    WorldBorder small = new WorldBorder();
                    small.setCenter(SX + r.nextInt(9) - 4 + (r.nextBoolean() ? 0.5 : 0.0), SZ + r.nextInt(9) - 4 + (r.nextBoolean() ? 0.5 : 0.0));
                    small.setSize(new int[]{3, 4, 5, 6, 8, 10, 14}[r.nextInt(7)]);
                    gridBorder = new BetterWorldBorder(small);
                    w.write("W " + small.getMinX() + " " + small.getMaxX() + " " + small.getMinZ() + " " + small.getMaxZ() + "\n");
                } else {
                    w.write("W " + wb.getMinX() + " " + wb.getMaxX() + " " + wb.getMinZ() + " " + wb.getMaxZ() + "\n");
                }
                int mode = r.nextInt(MODES);
                int y = 1 + r.nextInt(H - 1);
                if (mode == 10 && r.nextInt(100) < 50) {
                    y = 3 + r.nextInt(7);
                }
                int[] cells = grid(r, mode, y);
                boolean loaded = r.nextInt(10) != 0;
                int[][] ppos = new int[8][];
                for (int i = 0; i < ppos.length; i++) {
                    ppos[i] = new int[]{SX - 3 + r.nextInt(7), y - 2 + r.nextInt(6), SZ - 3 + r.nextInt(7)};
                }
                line.setLength(0);
                line.append("L ").append(b(loaded)).append('\n').append("G ").append(mode).append(' ').append(y).append(' ');
                for (int i = 0; i < cells.length; i++) {
                    states[i] = PAL.get(cells[i]);
                    int v = cells[i];
                    line.append(Character.forDigit(v >> 4, 16)).append(Character.forDigit(v & 15, 16));
                }
                w.write(line + "\n");
                for (int ci = 0; ci < CONFIGS.size(); ci++) {
                    Cfg c = CONFIGS.get(ci);
                    apply(c);
                    Ctx ctx = new Ctx(new Grid(gridBorder, states, loaded), new Tools(c.pick ? pick : hand), c.throwaway, c.bucket, c.sprint, c.frost, c.waterMult);
                    line.setLength(0);
                    for (Moves m : moves) {
                        res.reset();
                        m.apply(ctx, SX, y, SZ, res);
                        line.append("R ").append(ci).append(' ').append(m.ordinal()).append(' ').append(res.x).append(' ').append(res.y).append(' ').append(res.z).append(' ').append(hx(res.cost)).append('\n');
                    }
                    for (int[] p : ppos) {
                        line.append("P ").append(ci).append(' ').append(p[0]).append(' ').append(p[1]).append(' ').append(p[2]).append(' ').append(preds(ctx, p[0], p[1], p[2])).append('\n');
                    }
                    w.write(line.toString());
                }
            }
        }
    }
}
