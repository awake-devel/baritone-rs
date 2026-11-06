/*
 * baritone-rs test tool: a public door into the test classes of cost-compare (SpikeGoldenDump), so that the search comparison
 * uses the same palette, the same configurations and the same test context, without copying them or changing that file.
 * Licensed like the code it exercises: GNU LGPL, version 3 or (at your option) any later version.
 */
package baritone.pathing.movement;

import baritone.Baritone;
import baritone.api.IBaritone;
import baritone.utils.BlockStateInterface;
import baritone.utils.ToolSet;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;

public final class SearchBridge {
    private SearchBridge() {
    }

    private static boolean inited;
    private static ItemStack pick, hand;

    /** Binds the vanilla tags and sets what the test classes set before they run. Call once, first. */
    public static synchronized void init() throws Exception {
        if (inited) {
            return;
        }
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        SpikeGoldenDump.bindTags(BuiltInRegistries.BLOCK, Registries.BLOCK, "block");
        SpikeGoldenDump.bindTags(BuiltInRegistries.FLUID, Registries.FLUID, "fluid");
        Baritone.settings().considerPotionEffects.value = false;
        SpikeGoldenDump.air = Blocks.AIR.defaultBlockState();
        pick = new ItemStack(Items.DIAMOND_PICKAXE);
        hand = ItemStack.EMPTY;
        inited = true;
    }

    public static int paletteSize() {
        return SpikeGoldenDump.PAL.size();
    }

    public static BlockState state(int i) {
        return SpikeGoldenDump.PAL.get(i);
    }

    public static int ix(Block b) {
        return SpikeGoldenDump.ix(b);
    }

    public static String stateLine(int i) {
        return SpikeGoldenDump.facts("S", i, SpikeGoldenDump.PAL.get(i), pick, hand);
    }

    public static int configCount() {
        return SpikeGoldenDump.CONFIGS.size();
    }

    public static String configLine(int i) {
        return SpikeGoldenDump.cline(i, SpikeGoldenDump.CONFIGS.get(i));
    }

    public static String hx(double d) {
        return SpikeGoldenDump.hx(d);
    }

    /** The palette indexes of a named role of the cost comparison's block generators. */
    public static int[] role(String name) {
        return switch (name) {
            case "SOLIDS" -> SpikeGoldenDump.SOLIDS;
            case "FLUIDS" -> SpikeGoldenDump.FLUIDS;
            case "WATERS" -> SpikeGoldenDump.WATERS;
            case "LAVAS" -> SpikeGoldenDump.LAVAS;
            case "WATERLOGGED" -> SpikeGoldenDump.WATERLOGGED;
            case "CLIMBS" -> SpikeGoldenDump.CLIMBS;
            case "FALLERS" -> SpikeGoldenDump.FALLERS;
            case "DOORS" -> SpikeGoldenDump.DOORS;
            case "GATES" -> SpikeGoldenDump.GATES;
            case "TRAPS" -> SpikeGoldenDump.TRAPS;
            case "STAIRS" -> SpikeGoldenDump.STAIRS;
            case "SLABS" -> SpikeGoldenDump.SLABS;
            case "SNOWS" -> SpikeGoldenDump.SNOWS;
            case "CARPETS" -> SpikeGoldenDump.CARPETS;
            case "HAZARDS" -> SpikeGoldenDump.HAZARDS;
            case "SPECIAL_FLOORS" -> SpikeGoldenDump.SPECIAL_FLOORS;
            case "KEY_FLOORS" -> SpikeGoldenDump.KEY_FLOORS;
            case "STEP_BLOCKS" -> SpikeGoldenDump.STEP_BLOCKS;
            case "OBSTACLES" -> SpikeGoldenDump.OBSTACLES;
            case "CLUTTER" -> SpikeGoldenDump.CLUTTER;
            case "NON_AIR" -> SpikeGoldenDump.NON_AIR;
            default -> throw new IllegalArgumentException(name);
        };
    }

    static final class BCtx extends CalculationContext {
        BCtx(IBaritone baritone, BlockStateInterface bsi, ToolSet tools, SpikeGoldenDump.Cfg c) {
            super(baritone, false, null, null, bsi, tools, c.throwaway, c.bucket, c.sprint, c.frost, c.waterMult);
        }
    }

    /** Applies configuration i to the settings and builds a context for it. The search settings are set by the caller before this call: the context snapshots some of them. */
    public static CalculationContext context(IBaritone baritone, BlockStateInterface bsi, int cfg) {
        SpikeGoldenDump.Cfg c = SpikeGoldenDump.CONFIGS.get(cfg);
        SpikeGoldenDump.apply(c);
        return new BCtx(baritone, bsi, new SpikeGoldenDump.Tools(c.pick ? pick : hand), c);
    }
}
