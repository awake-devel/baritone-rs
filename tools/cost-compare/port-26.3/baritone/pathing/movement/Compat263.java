/*
 * baritone-rs test tool: the parts of the cost-compare harness that cannot be a text replacement when the harness runs
 * on upstream's 26.3 branch. tools/cost-compare/port_26_3.py copies this file next to the ported SpikeGoldenDump.java.
 * Licensed like the code it exercises: GNU LGPL, version 3 or (at your option) any later version.
 *
 * Why it exists: 1.21.4 has constructors made for tests (BlockStateInterface(border, minY, height) and a protected
 * CalculationContext constructor). 26.3 has none. Here both objects are allocated without running a constructor, and
 * their fields are filled in by reflection from the same settings the real constructor reads. The movement code under
 * test is untouched. Nothing in this file holds a rule of Baritone: only plumbing and the one stand-in below.
 */
package baritone.pathing.movement;

import baritone.Baritone;
import baritone.api.Settings;
import baritone.api.pathing.movement.ActionCosts;
import baritone.utils.BlockStateInterface;
import baritone.utils.ToolSet;
import baritone.utils.pathing.BetterWorldBorder;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

final class Compat263 {
    private Compat263() {
    }

    static final sun.misc.Unsafe UNSAFE;

    static {
        try {
            java.lang.reflect.Field f = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
            f.setAccessible(true);
            UNSAFE = (sun.misc.Unsafe) f.get(null);
        } catch (ReflectiveOperationException e) {
            throw new ExceptionInInitializerError(e);
        }
    }

    static void set(Object o, Class<?> c, String name, Object v) {
        try {
            java.lang.reflect.Field f = c.getDeclaredField(name);
            f.setAccessible(true);
            f.set(o, v);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(name, e);
        }
    }

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> c) {
        try {
            return (T) UNSAFE.allocateInstance(c);
        } catch (InstantiationException e) {
            throw new IllegalStateException(e);
        }
    }

    /** Only getMinY and getHeight are ever called on it (MovementDescend, MovementParkour). */
    static final class FakeLevel extends net.minecraft.client.multiplayer.ClientLevel {
        FakeLevel() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        @Override
        public int getMinY() {
            return 0;
        }

        @Override
        public int getHeight() {
            return SpikeGoldenDump.H;
        }
    }

    static final Level LEVEL = alloc(FakeLevel.class);

    static final class Grid extends BlockStateInterface {
        BlockState[] cells;
        boolean loaded;

        Grid() {
            super((baritone.api.utils.IPlayerContext) null);
        }

        @Override
        public BlockState get0(int x, int y, int z) {
            if (y < 0 || y >= SpikeGoldenDump.H || x < 0 || x >= SpikeGoldenDump.W || z < 0 || z >= SpikeGoldenDump.D) {
                return SpikeGoldenDump.air;
            }
            return cells[(y * SpikeGoldenDump.D + z) * SpikeGoldenDump.W + x];
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

    static BlockStateInterface grid(BetterWorldBorder border, BlockState[] cells, boolean loaded) {
        Grid g = alloc(Grid.class);
        g.cells = cells;
        g.loaded = loaded;
        set(g, BlockStateInterface.class, "world", LEVEL);
        set(g, BlockStateInterface.class, "worldBorder", border);
        set(g, BlockStateInterface.class, "isPassableBlockPos", new BlockPos.MutableBlockPos());
        try {
            var ctor = Class.forName("baritone.utils.BlockStateInterfaceAccessWrapper").getDeclaredConstructor(BlockStateInterface.class);
            ctor.setAccessible(true);
            set(g, BlockStateInterface.class, "access", ctor.newInstance(g));
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
        return g;
    }

    /**
     * Builds a context the way the 26.3 constructor would, from the current settings (so apply(cfg) must run first).
     * Every field is set from the same setting as in CalculationContext's constructor.
     */
    static CalculationContext ctx(BlockStateInterface bsi, ToolSet tools, boolean throwaway, boolean bucket, boolean sprint, int frost, float waterMult) {
        Settings st = Baritone.settings();
        CalculationContext c = alloc(CalculationContext.class);
        Class<?> k = CalculationContext.class;
        set(c, k, "safeForThreadedUse", false);
        set(c, k, "world", LEVEL);
        set(c, k, "bsi", bsi);
        set(c, k, "toolSet", tools);
        set(c, k, "hasThrowaway", throwaway);
        set(c, k, "hasWaterBucket", bucket);
        set(c, k, "canSprint", sprint);
        set(c, k, "placeBlockCost", st.blockPlacementPenalty.value);
        set(c, k, "allowBreak", st.allowBreak.value);
        set(c, k, "allowBreakAnyway", new ArrayList<>(st.allowBreakAnyway.value));
        set(c, k, "allowParkour", st.allowParkour.value);
        set(c, k, "allowParkourPlace", st.allowParkourPlace.value);
        set(c, k, "allowJumpAtBuildLimit", st.allowJumpAtBuildLimit.value);
        set(c, k, "allowParkourAscend", st.allowParkourAscend.value);
        set(c, k, "assumeWalkOnWater", st.assumeWalkOnWater.value);
        set(c, k, "allowFallIntoLava", false);
        set(c, k, "frostWalker", frost);
        set(c, k, "allowDiagonalDescend", st.allowDiagonalDescend.value);
        set(c, k, "allowDiagonalAscend", st.allowDiagonalAscend.value);
        set(c, k, "allowDownward", st.allowDownward.value);
        set(c, k, "minFallHeight", 3);
        set(c, k, "maxFallHeightNoWater", st.maxFallHeightNoWater.value);
        set(c, k, "maxFallHeightBucket", st.maxFallHeightBucket.value);
        set(c, k, "waterWalkSpeed", ActionCosts.WALK_ONE_IN_WATER_COST * (1 - waterMult) + ActionCosts.WALK_ONE_BLOCK_COST * waterMult);
        set(c, k, "breakBlockAdditionalCost", st.blockBreakAdditionalPenalty.value);
        set(c, k, "backtrackCostFavoringCoefficient", st.backtrackCostFavoringCoefficient.value);
        set(c, k, "jumpPenalty", st.jumpPenalty.value);
        set(c, k, "walkOnWaterOnePenalty", st.walkOnWaterOnePenalty.value);
        set(c, k, "allowWalkOnMagmaBlocks", st.allowWalkOnMagmaBlocks.value);
        set(c, k, "worldBorder", bsi.worldBorder);
        // 26.3 builds this table from the settings of the moment and never rebuilds it, so each context needs its own.
        set(c, k, "precomputedData", new baritone.pathing.precompute.PrecomputedData());
        return c;
    }

    /**
     * 26.3 items get their tool rules from DataComponentInitializers, which a bare Bootstrap never runs (the game runs
     * them after it has loaded the registries and tags). Without this every ItemStack fails. Run it after the tags are bound.
     */
    @SuppressWarnings({"rawtypes", "unchecked"})
    static int bindItemComponents() {
        net.minecraft.core.HolderLookup.Provider world = net.minecraft.data.registries.VanillaRegistries.createWorldLookup();
        List<net.minecraft.core.HolderLookup.RegistryLookup<?>> all = new ArrayList<>();
        BuiltInRegistries.REGISTRY.forEach(r -> all.add((net.minecraft.core.HolderLookup.RegistryLookup<?>) r));
        Set<Object> have = new HashSet<>();
        for (var l : all) have.add(l.key());
        world.listRegistryKeys().forEach(k -> {
            if (have.add(k)) all.add(world.lookupOrThrow((net.minecraft.resources.ResourceKey) k));
        });
        var provider = net.minecraft.core.HolderLookup.Provider.create(all.stream());
        var pending = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(provider);
        pending.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        return pending.size();
    }

    /**
     * MovementHelper.isWaterloggedSolidTop does not exist on 26.3. The registry sweep still has a column for it, so this
     * is its 1.21.4 body: it keeps that column the same on both branches. It is not an answer of upstream's 26.3 code,
     * and no 26.3 rule calls it.
     */
    static boolean isWaterloggedSolidTop(BlockState state) {
        return state.hasProperty(BlockStateProperties.WATERLOGGED) && state.getValue(BlockStateProperties.WATERLOGGED)
                && state.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.UP);
    }
}
