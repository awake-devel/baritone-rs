/*
 * baritone-rs test tool. LGPL-3.0-or-later. Copied into a checkout of upstream to use its Minecraft classpath, and removed
 * afterwards. Writes what real Minecraft says about every block state, with the fields of the engine's State.
 * Nothing here is game code or game data; the output stays local.
 *
 * Written against 26.3. tools/run_facts.py applies the renames for 1.21.4.
 */
package baritone;

import baritone.utils.ToolSet;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.tags.TagKey;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.piston.MovingPistonBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.DoorHingeSide;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.level.pathfinder.PathComputationType;
import org.junit.Test;

import java.io.BufferedWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

public class FactsDump {
    static int b(boolean v) {
        return v ? 1 : 0;
    }

    static String hx(double d) {
        return Long.toHexString(Double.doubleToRawLongBits(d));
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

    static boolean shapeThrows(BlockState s) {
        try {
            s.getCollisionShape(null, null);
            return false;
        } catch (Exception e) {
            return true;
        }
    }

    static boolean topSturdy(BlockState s) {
        try {
            return s.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.UP);
        } catch (Exception e) {
            return false;
        }
    }

    static String describe(BlockState s) {
        String props = s.getProperties().stream()
                .map(p -> p.getName() + "=" + name(s, p))
                .sorted()
                .collect(Collectors.joining(","));
        return BuiltInRegistries.BLOCK.getKey(s.getBlock()) + "[" + props + "]";
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    static String name(BlockState s, net.minecraft.world.level.block.state.properties.Property p) {
        return p.getName(s.getValue(p));
    }

    static String line(int id, BlockState s, ItemStack pick, ItemStack hand) {
        Block k = s.getBlock();
        FluidState f = s.getFluidState();
        BlockState def = k.defaultBlockState();
        String[][] props = {
                {"air", "" + b(k instanceof AirBlock)}, {"amethyst_cluster", "" + b(k instanceof AmethystClusterBlock)}, {"azalea", "" + b(k instanceof AzaleaBlock)},
                {"bamboo_stalk", "" + b(k instanceof BambooStalkBlock)}, {"base_fire", "" + b(k instanceof BaseFireBlock)}, {"carpet", "" + b(k instanceof CarpetBlock)},
                {"cauldron", "" + b(k instanceof CauldronBlock)}, {"door", "" + b(k instanceof DoorBlock)}, {"falling", "" + b(k instanceof FallingBlock)},
                {"gate", "" + b(k instanceof FenceGateBlock)}, {"infested", "" + b(k instanceof InfestedBlock)}, {"leaves", "" + b(k instanceof LeavesBlock)},
                {"lily", "" + b(k instanceof LilyPadBlock)}, {"liquid_block", "" + b(k instanceof LiquidBlock)}, {"moving_piston", "" + b(k instanceof MovingPistonBlock)},
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
                {"str_pick", hx(ToolSet.calculateSpeedVsBlock(pick, def))},
                {"str_hand", hx(ToolSet.calculateSpeedVsBlock(hand, def))},
        };
        StringBuilder o = new StringBuilder("S ").append(id).append(' ').append(describe(s));
        for (String[] kv : props) {
            o.append(' ').append(kv[0]).append('=').append(kv[1]);
        }
        // Not engine fields: they tell why a difference happens.
        o.append(" info_shape_throws=").append(b(shapeThrows(s)));
        o.append(" info_dynamic_shape=").append(b(k.hasDynamicShape()));
        return o.toString();
    }

    /** A bare Bootstrap binds no tags. This reads a registry's vanilla tags from the game jar and binds them. */
    static <T> int bindTags(net.minecraft.core.Registry<T> registry, net.minecraft.resources.ResourceKey<? extends net.minecraft.core.Registry<T>> key, String folder) throws Exception {
        Path jar = Path.of(Bootstrap.class.getProtectionDomain().getCodeSource().getLocation().toURI());
        Map<String, List<String>> raw = new LinkedHashMap<>();
        String prefix = "data/minecraft/tags/" + folder + "/";
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
        if (raw.isEmpty()) throw new IllegalStateException("no " + folder + " tags found in " + jar);
        Map<TagKey<T>, List<Holder<T>>> bound = new HashMap<>();
        for (String id : raw.keySet()) {
            List<Holder<T>> out = new ArrayList<>();
            expand(registry, id, raw, out, new HashSet<>());
            bound.put(TagKey.create(key, Identifier.withDefaultNamespace(id)), out);
        }
        var pending = registry.prepareTagReload(new TagLoader.LoadResult<>(key, bound));
        pending.apply();
        return pending.size();
    }

    static <T> void expand(net.minecraft.core.Registry<T> registry, String id, Map<String, List<String>> raw, List<Holder<T>> out, Set<String> seen) {
        if (!seen.add(id)) return;
        for (String v : raw.getOrDefault(id, List.of())) {
            if (v.startsWith("#")) {
                expand(registry, v.substring(1).replace("minecraft:", ""), raw, out, seen);
            } else {
                registry.get(Identifier.parse(v)).ifPresent(h -> {
                    if (!out.contains(h)) out.add(h);
                });
            }
        }
    }

    //@26.3-begin
    /** 26.3 items get their tool rules from DataComponentInitializers, which a bare Bootstrap never runs. */
    @SuppressWarnings({"rawtypes", "unchecked"})
    static void bindItemComponents() {
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
    }
    //@26.3-end

    @Test
    public void dump() throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        // Blocks read them for tools and for FallingBlock.isFree, and LiquidBlock.isPathfindable reads the fluid ones.
        bindTags(BuiltInRegistries.BLOCK, Registries.BLOCK, "block");
        bindTags(BuiltInRegistries.FLUID, Registries.FLUID, "fluid");
        //@26.3-begin
        bindItemComponents();
        //@26.3-end
        ItemStack pick = new ItemStack(Items.DIAMOND_PICKAXE);
        ItemStack hand = ItemStack.EMPTY;
        try (BufferedWriter w = Files.newBufferedWriter(Path.of(System.getenv("FACTS_OUT")))) {
            for (BlockState s : Block.BLOCK_STATE_REGISTRY) {
                w.write(line(Block.BLOCK_STATE_REGISTRY.getId(s), s, pick, hand));
                w.write('\n');
            }
        }
    }
}
