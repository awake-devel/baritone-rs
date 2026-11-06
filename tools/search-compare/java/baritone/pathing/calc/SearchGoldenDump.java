/*
 * baritone-rs test tool: runs upstream's real searches over generated worlds and writes everything they answer. Copy it, with the
 * files of tools/cost-compare/java and tools/search-compare/java, into a checkout of upstream (run_search.py does it).
 * Licensed like the code it exercises: GNU LGPL, version 3 or (at your option) any later version.
 *
 * Rule: this file builds worlds, settings and goals and calls upstream's own classes (AStarPathFinder, Path, CutoffPath, SplicedPath,
 * Favoring, Avoidance, PathingBehavior.createPathfinder). It holds no second implementation of the algorithm. What it adds is
 * only the control of how a run stops (see Counting) and the writing of the answers.
 *
 * It is in this package because Path is package-private and the search keeps its state in protected fields.
 */
package baritone.pathing.calc;

import baritone.Baritone;
import baritone.api.IBaritone;
import baritone.api.cache.ICachedWorld;
import baritone.api.pathing.calc.IPath;
import baritone.api.pathing.goals.*;
import baritone.api.pathing.movement.IMovement;
import baritone.api.utils.BetterBlockPos;
import baritone.api.utils.IPlayerContext;
import baritone.api.utils.PathCalculationResult;
import baritone.api.cache.IWorldData;
import baritone.behavior.PathingBehavior;
import baritone.pathing.movement.CalculationContext;
import baritone.pathing.movement.Movement;
import baritone.pathing.movement.SearchBridge;
import baritone.pathing.path.SplicedPath;
import baritone.utils.BlockStateInterface;
import baritone.utils.pathing.Avoidance;
import baritone.utils.pathing.BetterWorldBorder;
import it.unimi.dsi.fastutil.longs.Long2DoubleOpenHashMap;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import org.junit.Test;

import java.io.BufferedWriter;
import java.io.OutputStream;
import java.io.PrintStream;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.Random;
import java.util.stream.Stream;

public class SearchGoldenDump {
    /** A search that is meant to run to its end stops here instead, and is then written as a run with this node cap. */
    static final int HARD = 20000;
    /** The time limits are far away: a run never ends because of the clock. */
    static final long FAR = 1L << 40;
    static final int[] CAPS = {1, 2, 63, 64, 65, 500, 5000, -1};

    static {
        SearchBridgeInit.run();
    }

    static final class SearchBridgeInit {
        static void run() {
            try {
                SearchBridge.init();
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        }
    }

    static final BlockState AIR_STATE = Blocks.AIR.defaultBlockState();
    static final int AIR = 0;
    static final int CAVE_AIR = SearchBridge.ix(Blocks.CAVE_AIR);
    static final int STONE = SearchBridge.ix(Blocks.STONE), DIRT = SearchBridge.ix(Blocks.DIRT), GRAVEL = SearchBridge.ix(Blocks.GRAVEL),
            COBBLE = SearchBridge.ix(Blocks.COBBLESTONE), SAND = SearchBridge.ix(Blocks.SAND), WATER = SearchBridge.ix(Blocks.WATER),
            LAVA = SearchBridge.ix(Blocks.LAVA), LADDER = SearchBridge.ix(Blocks.LADDER), VINE = SearchBridge.ix(Blocks.VINE),
            PLANKS = SearchBridge.ix(Blocks.OAK_PLANKS), GLASS = SearchBridge.ix(Blocks.GLASS), ICE = SearchBridge.ix(Blocks.ICE),
            OBSIDIAN = SearchBridge.ix(Blocks.OBSIDIAN), BEDROCK = SearchBridge.ix(Blocks.BEDROCK), LEAVES = SearchBridge.ix(Blocks.OAK_LEAVES),
            COBWEB = SearchBridge.ix(Blocks.COBWEB), FENCE = SearchBridge.ix(Blocks.OAK_FENCE), PATH = SearchBridge.ix(Blocks.DIRT_PATH),
            GRASS = SearchBridge.ix(Blocks.SHORT_GRASS), SOUL_SAND = SearchBridge.ix(Blocks.SOUL_SAND), MAGMA = SearchBridge.ix(Blocks.MAGMA_BLOCK),
            LILY = SearchBridge.ix(Blocks.LILY_PAD);
    static final int[] SOLIDS = SearchBridge.role("SOLIDS"), WATERS = SearchBridge.role("WATERS"), LAVAS = SearchBridge.role("LAVAS"),
            CLIMBS = SearchBridge.role("CLIMBS"), DOORS = SearchBridge.role("DOORS"), GATES = SearchBridge.role("GATES"),
            TRAPS = SearchBridge.role("TRAPS"), STAIRS = SearchBridge.role("STAIRS"), SLABS = SearchBridge.role("SLABS"),
            OBSTACLES = SearchBridge.role("OBSTACLES"), CLUTTER = SearchBridge.role("CLUTTER"), FALLERS = SearchBridge.role("FALLERS"),
            KEY_FLOORS = SearchBridge.role("KEY_FLOORS"), SPECIAL_FLOORS = SearchBridge.role("SPECIAL_FLOORS"), HAZARDS = SearchBridge.role("HAZARDS"),
            STEP_BLOCKS = SearchBridge.role("STEP_BLOCKS"), NON_AIR = SearchBridge.role("NON_AIR"), FLUIDS = SearchBridge.role("FLUIDS");

    static int pick(Random r, int[] a) {
        return a[r.nextInt(a.length)];
    }

    static boolean isAir(int c) {
        return c == AIR || c == CAVE_AIR;
    }

    static String hx(double d) {
        return SearchBridge.hx(d);
    }

    // ------------------------------------------------------------------------------------------------------------
    // The world
    // ------------------------------------------------------------------------------------------------------------

    /** A box of blocks at (ox, minY, oz). Cells are indexed (x * d + z) * h + y. Outside the box in x and z it is air and not loaded. */
    static final class World extends BlockStateInterface {
        final int ox, oz, w, h, d, cxn, czn;
        final int[] cells;
        /** Per chunk of the box, x major: 0 not loaded, 1 only known to the cache (isLoaded, not worldContainsLoadedChunk), 2 loaded. */
        final byte[] chunks;
        final String kind;
        final WorldBorder wb;

        World(String kind, WorldBorder wb, int minY, int ox, int oz, int w, int h, int d, int[] cells, byte[] chunks) {
            super(new BetterWorldBorder(wb), minY, h);
            this.kind = kind;
            this.wb = wb;
            this.ox = ox;
            this.oz = oz;
            this.w = w;
            this.h = h;
            this.d = d;
            this.cells = cells;
            this.chunks = chunks;
            this.cxn = w >> 4;
            this.czn = d >> 4;
        }

        int minYv() {
            return minY;
        }

        @Override
        protected BlockState getUncached(int x, int y, int z) {
            int lx = x - ox, lz = z - oz;
            if (lx < 0 || lx >= w || lz < 0 || lz >= d) {
                return AIR_STATE;
            }
            return SearchBridge.state(cells[(lx * d + lz) * h + y]);
        }

        int chunk(int x, int z) {
            int cx = (x >> 4) - (ox >> 4), cz = (z >> 4) - (oz >> 4);
            if (cx < 0 || cx >= cxn || cz < 0 || cz >= czn) {
                return 0;
            }
            return chunks[cx * czn + cz];
        }

        @Override
        public boolean isLoaded(int x, int z) {
            return chunk(x, z) >= 1;
        }

        @Override
        public boolean worldContainsLoadedChunk(int x, int z) {
            return chunk(x, z) == 2;
        }

        int cell(int lx, int ly, int lz) {
            return cells[(lx * d + lz) * h + ly];
        }
    }

    static final class Vox {
        final int w, h, d;
        final int[] c;

        Vox(int w, int h, int d) {
            this.w = w;
            this.h = h;
            this.d = d;
            this.c = new int[w * h * d];
        }

        boolean in(int x, int y, int z) {
            return x >= 0 && x < w && y >= 0 && y < h && z >= 0 && z < d;
        }

        int get(int x, int y, int z) {
            return in(x, y, z) ? c[(x * d + z) * h + y] : STONE;
        }

        void set(int x, int y, int z, int v) {
            if (in(x, y, z)) {
                c[(x * d + z) * h + y] = v;
            }
        }

        void fill(int x0, int y0, int z0, int x1, int y1, int z1, int v) {
            for (int x = Math.max(0, Math.min(x0, x1)); x <= Math.min(w - 1, Math.max(x0, x1)); x++) {
                for (int z = Math.max(0, Math.min(z0, z1)); z <= Math.min(d - 1, Math.max(z0, z1)); z++) {
                    for (int y = Math.max(0, Math.min(y0, y1)); y <= Math.min(h - 1, Math.max(y0, y1)); y++) {
                        c[(x * d + z) * h + y] = v;
                    }
                }
            }
        }

        /** The highest non-air cell of a column, or -1. */
        int top(int x, int z) {
            for (int y = h - 1; y >= 0; y--) {
                if (!isAir(c[(x * d + z) * h + y])) {
                    return y;
                }
            }
            return -1;
        }
    }

    static double hash01(long seed, int x, int y, int z) {
        long h = seed * 0x9E3779B97F4A7C15L + x * 0xC2B2AE3D27D4EB4FL + y * 0x165667B19E3779F9L + z * 0x27D4EB2F165667C5L;
        h ^= h >>> 29;
        h *= 0xBF58476D1CE4E5B9L;
        h ^= h >>> 32;
        h *= 0x94D049BB133111EBL;
        h ^= h >>> 29;
        return (h >>> 11) * (1.0 / (1L << 53));
    }

    static double smooth(double t) {
        return t * t * (3 - 2 * t);
    }

    static double vnoise(long seed, double x, double y, double z) {
        int xi = (int) Math.floor(x), yi = (int) Math.floor(y), zi = (int) Math.floor(z);
        double fx = smooth(x - xi), fy = smooth(y - yi), fz = smooth(z - zi);
        double v = 0;
        for (int a = 0; a < 2; a++) {
            for (int b = 0; b < 2; b++) {
                for (int c = 0; c < 2; c++) {
                    double wgt = (a == 0 ? 1 - fx : fx) * (b == 0 ? 1 - fy : fy) * (c == 0 ? 1 - fz : fz);
                    v += wgt * hash01(seed, xi + a, yi + b, zi + c);
                }
            }
        }
        return v;
    }

    static double fbm(long seed, double x, double z) {
        return 0.55 * vnoise(seed, x / 16.0, 0, z / 16.0) + 0.3 * vnoise(seed + 1, x / 8.0, 0, z / 8.0) + 0.15 * vnoise(seed + 2, x / 4.0, 0, z / 4.0);
    }

    static final String[] KINDS = {"flat", "hills", "caves", "lakes", "ladders", "village", "gapwalls", "falls", "towers", "maze", "noise", "mesa"};

    /** Terrain from noise: stone, a dirt skin, sand and gravel patches, water up to the sea level. Returns the heights. */
    static int[][] terrain(Vox v, Random r, long seed, int base, double amp, int sea) {
        int[][] top = new int[v.w][v.d];
        for (int x = 0; x < v.w; x++) {
            for (int z = 0; z < v.d; z++) {
                int t = Math.max(2, Math.min(v.h - 6, base + (int) Math.round(amp * (fbm(seed, x, z) - 0.5) * 2)));
                top[x][z] = t;
                double patch = vnoise(seed + 7, x / 6.0, 0, z / 6.0);
                for (int y = 0; y <= t; y++) {
                    int b;
                    if (y == 0) {
                        b = r.nextInt(8) == 0 ? BEDROCK : STONE;
                    } else if (y < t - 2) {
                        b = r.nextInt(40) == 0 ? GRAVEL : STONE;
                    } else if (y == t) {
                        b = patch > 0.7 ? SAND : (patch < 0.2 ? GRAVEL : DIRT);
                    } else {
                        b = DIRT;
                    }
                    v.set(x, y, z, b);
                }
                for (int y = t + 1; y <= sea; y++) {
                    v.set(x, y, z, WATER);
                }
            }
        }
        return top;
    }

    static void scatter(Vox v, Random r, int[][] top, int count, int[] what, int dy) {
        for (int i = 0; i < count; i++) {
            int x = r.nextInt(v.w), z = r.nextInt(v.d);
            int t = top != null ? top[x][z] : 3;
            v.set(x, t + 1 + dy, z, pick(r, what));
        }
    }

    static Vox generate(int kind, int w, int h, int d, Random r, long seed) {
        Vox v = new Vox(w, h, d);
        int fy = 2 + r.nextInt(Math.max(1, Math.min(6, h / 4)));
        switch (KINDS[kind]) {
            case "flat" -> {
                v.fill(0, 0, 0, w - 1, fy - 1, d - 1, STONE);
                for (int x = 0; x < w; x++) {
                    for (int z = 0; z < d; z++) {
                        int k = r.nextInt(100);
                        v.set(x, fy - 1, z, k < 60 ? STONE : (k < 85 ? DIRT : (k < 95 ? GRAVEL : pick(r, KEY_FLOORS))));
                    }
                }
                int[][] top = new int[w][d];
                for (int[] row : top) java.util.Arrays.fill(row, fy - 1);
                scatter(v, r, top, w * d * (1 + r.nextInt(3)) / 100, OBSTACLES, 0);
                scatter(v, r, top, w * d / 150, OBSTACLES, 1);
                scatter(v, r, top, w * d / 100, CLUTTER, 0);
                scatter(v, r, top, w * d / 120, SLABS, 0);
                scatter(v, r, top, w * d / 150, STAIRS, 0);
                for (int i = r.nextInt(4); i > 0; i--) {
                    int x = r.nextInt(w - 4), z = r.nextInt(d - 4), sx = 2 + r.nextInt(8), sz = 2 + r.nextInt(8);
                    v.fill(x, fy - 2, z, x + sx, fy - 1, z + sz, r.nextInt(3) == 0 ? LAVA : pick(r, WATERS));
                }
                for (int i = 1 + r.nextInt(3); i > 0; i--) {
                    int x = r.nextInt(w), z = r.nextInt(d);
                    int len = 4 + r.nextInt(20), depth = 1 + r.nextInt(4);
                    if (r.nextBoolean()) v.fill(x, fy - depth, z, x + len, fy, z, AIR);
                    else v.fill(x, fy - depth, z, x, fy, z + len, AIR);
                }
            }
            case "hills" -> {
                int[][] top = terrain(v, r, seed, h / 3, 6 + r.nextInt(8), r.nextInt(3) == 0 ? h / 3 : 0);
                scatter(v, r, top, w * d / 60, new int[]{LEAVES, LEAVES, COBWEB, GRASS, STONE, DIRT}, 0);
                scatter(v, r, top, w * d / 100, OBSTACLES, 0);
            }
            case "caves" -> {
                int[][] top = terrain(v, r, seed, h * 2 / 3, 4 + r.nextInt(5), 0);
                double thr = 0.52 + r.nextInt(8) * 0.01;
                for (int x = 0; x < w; x++) {
                    for (int z = 0; z < d; z++) {
                        for (int y = 2; y < top[x][z] - 1; y++) {
                            if (vnoise(seed + 11, x / 7.0, y / 5.0, z / 7.0) * 0.7 + vnoise(seed + 12, x / 3.5, y / 3.0, z / 3.5) * 0.3 > thr) {
                                int b = y <= 4 && r.nextInt(100) < 70 ? LAVA : (y <= 6 && r.nextInt(100) < 10 ? pick(r, WATERS) : (r.nextInt(300) == 0 ? COBWEB : AIR));
                                v.set(x, y, z, b);
                            }
                        }
                    }
                }
                for (int i = 0; i < 4 + r.nextInt(6); i++) {
                    int x = r.nextInt(w), z = r.nextInt(d), t = top[x][z];
                    int[] col = r.nextBoolean() ? new int[]{LADDER} : CLIMBS;
                    for (int y = Math.max(2, t - 12 - r.nextInt(10)); y <= t; y++) v.set(x, y, z, pick(r, col));
                }
            }
            case "lakes" -> {
                int sea = h / 3 + 2;
                int[][] top = terrain(v, r, seed, h / 3, 5 + r.nextInt(5), sea);
                for (int i = 0; i < 2 + r.nextInt(3); i++) {
                    int cx = r.nextInt(w), cz = r.nextInt(d), rad = 3 + r.nextInt(7);
                    for (int x = Math.max(0, cx - rad); x < Math.min(w, cx + rad); x++) {
                        for (int z = Math.max(0, cz - rad); z < Math.min(d, cz + rad); z++) {
                            if ((x - cx) * (x - cx) + (z - cz) * (z - cz) <= rad * rad) {
                                int t = top[x][z];
                                for (int y = Math.max(1, t - 2); y <= t + 3; y++) v.set(x, y, z, y <= t ? STONE : AIR);
                                v.set(x, Math.max(1, t - 2), z, STONE);
                                for (int y = Math.max(1, t - 1); y <= t; y++) v.set(x, y, z, LAVA);
                            }
                        }
                    }
                }
                for (int x = 0; x < w; x++) {
                    for (int z = 0; z < d; z++) {
                        if (v.get(x, sea, z) == WATER && r.nextInt(4) == 0) v.set(x, sea, z, pick(r, WATERS));
                    }
                }
            }
            case "ladders" -> {
                v.fill(0, 0, 0, w - 1, fy - 1, d - 1, STONE);
                for (int i = 0; i < 6 + r.nextInt(7); i++) {
                    int tx = 2 + r.nextInt(w - 5), tz = 2 + r.nextInt(d - 5), th = 5 + r.nextInt(Math.max(1, h - fy - 9));
                    int fat = r.nextInt(3) == 0 ? 1 : 0;
                    v.fill(tx, fy, tz, tx + fat, fy + th, tz + fat, r.nextInt(5) == 0 ? COBBLE : STONE);
                    int[] col = r.nextInt(3) == 0 ? CLIMBS : (r.nextInt(5) == 0 ? WATERS : new int[]{LADDER});
                    int side = r.nextInt(4);
                    int lx = tx + (side == 0 ? -1 : side == 1 ? 1 + fat : 0), lz = tz + (side == 2 ? -1 : side == 3 ? 1 + fat : 0);
                    for (int y = fy; y <= fy + th; y++) {
                        if (r.nextInt(25) != 0) v.set(lx, y, lz, pick(r, col));
                    }
                    v.fill(tx - 1, fy + th + 1, tz - 1, tx + 1 + fat, fy + th + 1, tz + 1 + fat, STONE);
                }
                scatter(v, r, null, w * d / 120, OBSTACLES, fy - 3);
            }
            case "village" -> {
                v.fill(0, 0, 0, w - 1, fy - 1, d - 1, DIRT);
                for (int x = 0; x < w; x++) for (int z = 0; z < d; z++) if (r.nextInt(5) == 0) v.set(x, fy - 1, z, r.nextBoolean() ? PATH : GRAVEL);
                for (int i = 0; i < 5 + r.nextInt(6); i++) {
                    int x0 = 1 + r.nextInt(w - 10), z0 = 1 + r.nextInt(d - 10), sx = 4 + r.nextInt(5), sz = 4 + r.nextInt(5), wh = 2 + r.nextInt(2);
                    v.fill(x0, fy, z0, x0 + sx, fy + wh, z0 + sz, PLANKS);
                    v.fill(x0 + 1, fy, z0 + 1, x0 + sx - 1, fy + wh - 1, z0 + sz - 1, AIR);
                    v.fill(x0, fy + wh, z0, x0 + sx, fy + wh, z0 + sz, r.nextBoolean() ? PLANKS : pick(r, SLABS));
                    int dz = z0 + 1 + r.nextInt(sz - 1);
                    int door = pick(r, DOORS);
                    v.set(x0, fy, dz, door);
                    v.set(x0, fy + 1, dz, pick(r, DOORS));
                    if (r.nextInt(3) == 0) v.set(x0 + sx, fy, z0 + 1 + r.nextInt(sz - 1), pick(r, TRAPS));
                    for (int k = 0; k < 3; k++) v.set(x0 + 1 + r.nextInt(sx - 1), fy + 1, z0, r.nextInt(3) == 0 ? pick(r, TRAPS) : GLASS);
                    for (int k = 0; k < 3; k++) v.set(x0 - 1, fy + k - 1, dz, k == 0 ? pick(r, STAIRS) : AIR);
                }
                for (int i = 0; i < 6; i++) {
                    int x = r.nextInt(w - 10), z = r.nextInt(d);
                    for (int k = 0; k < 8; k++) v.set(x + k, fy, z, r.nextInt(6) == 0 ? pick(r, GATES) : FENCE);
                }
                scatter(v, r, null, w * d / 80, CLUTTER, fy - 3);
            }
            case "gapwalls" -> {
                v.fill(0, 0, 0, w - 1, fy - 1, d - 1, STONE);
                for (int x = 0; x < w; x++) for (int z = 0; z < d; z++) if (r.nextInt(8) == 0) v.set(x, fy - 1, z, pick(r, KEY_FLOORS));
                for (int i = 0; i < 3 + r.nextInt(4); i++) {
                    boolean alongX = r.nextBoolean();
                    int pos = 6 + r.nextInt((alongX ? w : d) - 12), thick = 1 + r.nextInt(2), ht = 1 + r.nextInt(5);
                    int kind2 = r.nextInt(3);
                    int span = alongX ? d : w;
                    for (int k = 0; k < span; k++) {
                        for (int t = 0; t < thick; t++) {
                            for (int y = fy; y < fy + ht; y++) {
                                if (alongX) v.set(pos + t, y, k, kind2 == 0 ? STONE : (kind2 == 1 ? pick(r, OBSTACLES) : DIRT));
                                else v.set(k, y, pos + t, kind2 == 0 ? STONE : (kind2 == 1 ? pick(r, OBSTACLES) : DIRT));
                            }
                        }
                    }
                    for (int g = 1 + r.nextInt(3); g > 0; g--) {
                        int at = r.nextInt(span - 2), width = 1 + r.nextInt(3);
                        for (int k = at; k < at + width; k++) {
                            for (int t = 0; t < thick; t++) {
                                for (int y = fy; y < h; y++) {
                                    if (alongX) v.set(pos + t, y, k, AIR);
                                    else v.set(k, y, pos + t, AIR);
                                }
                            }
                        }
                    }
                }
                for (int i = 0; i < 2 + r.nextInt(4); i++) {
                    boolean alongX = r.nextBoolean();
                    int pos = 4 + r.nextInt((alongX ? w : d) - 10), wd = 1 + r.nextInt(5), depth = 1 + r.nextInt(fy);
                    if (alongX) v.fill(pos, fy - depth, 0, pos + wd, fy, d - 1, AIR);
                    else v.fill(0, fy - depth, pos, w - 1, fy, pos + wd, AIR);
                    if (r.nextInt(3) == 0) {
                        int[] fill = r.nextBoolean() ? LAVAS : WATERS;
                        if (alongX) v.fill(pos, fy - depth, 0, pos + wd, fy - depth, d - 1, pick(r, fill));
                        else v.fill(0, fy - depth, pos, w - 1, fy - depth, pos + wd, pick(r, fill));
                    }
                }
            }
            case "falls" -> {
                int level = h - 6 - r.nextInt(4);
                int x = 0;
                while (x < w) {
                    int len = 3 + r.nextInt(6);
                    for (int k = 0; k < len && x < w; k++, x++) {
                        for (int z = 0; z < d; z++) {
                            for (int y = 0; y <= level; y++) v.set(x, y, z, y == level ? (r.nextInt(6) == 0 ? pick(r, STEP_BLOCKS) : STONE) : STONE);
                        }
                    }
                    int drop = r.nextInt(4) == 0 ? 4 + r.nextInt(9) : 1 + r.nextInt(3);
                    level = Math.max(1, level - drop);
                }
                for (int i = 0; i < 3 + r.nextInt(5); i++) {
                    int px = r.nextInt(w - 3), pz = r.nextInt(d - 3), pd = 3 + r.nextInt(12);
                    int t = v.top(px, pz);
                    v.fill(px, Math.max(1, t - pd), pz, px + 1, h - 1, pz + 1, AIR);
                    int pf = r.nextInt(6);
                    v.fill(px, Math.max(1, t - pd) - 1, pz, px + 1, Math.max(1, t - pd) - 1, pz + 1, pf == 0 ? STONE : pf == 1 ? pick(r, WATERS) : pf == 2 ? LADDER : pf == 3 ? pick(r, SLABS) : pf == 4 ? pick(r, LAVAS) : COBBLE);
                }
            }
            case "towers" -> {
                v.fill(0, 0, 0, w - 1, fy - 1, d - 1, STONE);
                for (int i = 0; i < 12 + r.nextInt(12); i++) {
                    int tx = r.nextInt(w - 2), tz = r.nextInt(d - 2), th = 2 + r.nextInt(h - fy - 4);
                    int fat = r.nextInt(3);
                    v.fill(tx, fy, tz, tx + fat, fy + th, tz + fat, r.nextInt(4) == 0 ? pick(r, SOLIDS) : STONE);
                    if (r.nextInt(3) == 0) v.fill(tx - 1, fy + th, tz - 1, tx + 1 + fat, fy + th, tz + 1 + fat, STONE);
                }
                int cx = r.nextInt(w - 12) + 6, cz = r.nextInt(d - 12) + 6;
                for (int k = 0; k < 6; k++) v.fill(cx - 6 + k, fy, cz - 6 + k, cx + 6 - k, fy + k, cz + 6 - k, STONE);
            }
            case "maze" -> {
                v.fill(0, 0, 0, w - 1, fy - 1, d - 1, STONE);
                int mw = w / 2, md = d / 2;
                boolean[][] seen = new boolean[mw][md];
                int[][] wallX = new int[mw][md];
                v.fill(0, fy, 0, w - 1, fy + 2, d - 1, r.nextInt(3) == 0 ? COBBLE : STONE);
                java.util.ArrayDeque<int[]> st = new java.util.ArrayDeque<>();
                st.push(new int[]{0, 0});
                seen[0][0] = true;
                v.fill(0, fy, 0, 1, fy + 2, 1, AIR);
                int[][] dirs = {{1, 0}, {-1, 0}, {0, 1}, {0, -1}};
                while (!st.isEmpty()) {
                    int[] cur = st.peek();
                    List<int[]> opts = new ArrayList<>();
                    for (int[] dd : dirs) {
                        int nx = cur[0] + dd[0], nz = cur[1] + dd[1];
                        if (nx >= 0 && nx < mw && nz >= 0 && nz < md && !seen[nx][nz]) opts.add(new int[]{nx, nz, dd[0], dd[1]});
                    }
                    if (opts.isEmpty()) {
                        st.pop();
                        continue;
                    }
                    int[] o = opts.get(r.nextInt(opts.size()));
                    seen[o[0]][o[1]] = true;
                    v.fill(o[0] * 2, fy, o[1] * 2, o[0] * 2, fy + 2, o[1] * 2, AIR);
                    v.fill(cur[0] * 2 + o[2], fy, cur[1] * 2 + o[3], cur[0] * 2 + o[2], fy + 2, cur[1] * 2 + o[3], AIR);
                    st.push(new int[]{o[0], o[1]});
                }
                for (int i = 0; i < w * d / 100; i++) v.set(r.nextInt(w), fy + r.nextInt(3), r.nextInt(d), pick(r, new int[]{AIR, GLASS, pick(r, DOORS)}));
            }
            case "noise" -> {
                v.fill(0, 0, 0, w - 1, fy - 1, d - 1, STONE);
                for (int x = 0; x < w; x++) {
                    for (int z = 0; z < d; z++) {
                        for (int y = fy; y < fy + 4; y++) {
                            int k = r.nextInt(100);
                            if (k < 6) v.set(x, y, z, r.nextInt(SearchBridge.paletteSize()));
                            else if (k < 8) v.set(x, y, z, pick(r, FLUIDS));
                            else if (k < 11) v.set(x, y, z, pick(r, CLUTTER));
                        }
                        if (r.nextInt(100) < 15) v.set(x, fy - 1, z, pick(r, SPECIAL_FLOORS));
                    }
                }
            }
            default -> {
                v.fill(0, 0, 0, w - 1, fy - 1, d - 1, STONE);
                int steps = Math.min(h - fy - 3, 3 + r.nextInt(8));
                for (int k = 0; k < steps; k++) {
                    int m = k * (Math.min(w, d) / (2 * steps + 2) + 1);
                    int blk = r.nextInt(4) == 0 ? pick(r, STEP_BLOCKS) : STONE;
                    v.fill(m, fy, m, w - 1 - m, fy + k, d - 1 - m, STONE);
                    if (r.nextBoolean()) v.fill(m, fy + k + 1, m, w - 1 - m, fy + k + 1, d - 1 - m, AIR);
                    for (int x = m; x < w - m; x += 1 + r.nextInt(3)) v.set(x, fy + k, m, blk);
                }
                scatter(v, r, null, w * d / 100, SLABS, fy);
                scatter(v, r, null, w * d / 120, STAIRS, fy);
            }
        }
        return v;
    }

    // ------------------------------------------------------------------------------------------------------------
    // Settings of the search
    // ------------------------------------------------------------------------------------------------------------

    static final class K {
        double backtrack = 0.5;
        int borderFetch = 50;
        boolean minImprove = true;
        int cutMin = 30;
        double cutFactor = 0.9;
        /** Overrides the configuration's costHeuristic when set. */
        Double costH = null;
        boolean loadBoundary = false, simplify = true, avoidance = false;
        double spawnerCoeff = 2.0, mobCoeff = 1.5;
        int spawnerRadius = 16, mobRadius = 8;
    }

    static final List<K> KS = new ArrayList<>();
    static final int[] K_WEIGHT = new int[32];

    static void k(int weight, java.util.function.Consumer<K> f) {
        K x = new K();
        f.accept(x);
        K_WEIGHT[KS.size()] = weight;
        KS.add(x);
    }

    static {
        k(34, x -> {
        });
        k(8, x -> x.loadBoundary = true);
        k(4, x -> x.backtrack = 1.0);
        k(5, x -> x.backtrack = 0.25);
        k(3, x -> x.backtrack = 2.0);
        k(5, x -> x.borderFetch = 5);
        k(3, x -> x.borderFetch = 1);
        k(2, x -> x.borderFetch = 300);
        k(4, x -> x.minImprove = false);
        k(5, x -> {
            x.cutMin = 10;
            x.cutFactor = 0.5;
        });
        k(4, x -> {
            x.cutMin = 5;
            x.cutFactor = 0.99;
            x.loadBoundary = true;
        });
        k(2, x -> x.cutMin = 100);
        k(4, x -> x.simplify = false);
        k(4, x -> {
            x.avoidance = true;
            x.spawnerRadius = 4;
            x.mobRadius = 3;
        });
        k(3, x -> {
            x.avoidance = true;
            x.spawnerCoeff = 0.5;
            x.spawnerRadius = 6;
            x.mobCoeff = 3.0;
            x.mobRadius = 2;
        });
        k(3, x -> {
            x.cutFactor = 0.0;
            x.cutMin = 20;
        });
        k(1, x -> {
            x.cutFactor = 1.5;
            x.cutMin = 8;
        });
        k(4, x -> {
            x.loadBoundary = true;
            x.borderFetch = 20;
            x.cutMin = 15;
        });
        // no heuristic at all: a search that is Dijkstra's, with ties everywhere
        k(3, x -> x.costH = 0.0);
        k(2, x -> x.costH = 12.0);
        // a heuristic that is not a number: the first node cannot be made
        k(1, x -> x.costH = Double.NaN);
        // avoidance with only one of the two coefficients away from 1: the other kind of sphere must not switch the first one off
        k(3, x -> {
            x.avoidance = true;
            x.spawnerCoeff = 1.0;
            x.spawnerRadius = 5;
            x.mobCoeff = 2.5;
            x.mobRadius = 5;
        });
        k(3, x -> {
            x.avoidance = true;
            x.spawnerCoeff = 3.0;
            x.spawnerRadius = 5;
            x.mobCoeff = 1.0;
            x.mobRadius = 5;
        });
    }

    static String kline(int i) {
        K x = KS.get(i);
        return "K " + i + " backtrack=" + hx(x.backtrack) + " borderfetch=" + x.borderFetch + " minimprove=" + (x.minImprove ? 1 : 0) + " cutmin=" + x.cutMin + " cutfactor=" + hx(x.cutFactor)
                + " loadboundary=" + (x.loadBoundary ? 1 : 0) + " simplify=" + (x.simplify ? 1 : 0) + " avoidance=" + (x.avoidance ? 1 : 0) + " spawnercoeff=" + hx(x.spawnerCoeff)
                + " spawnerradius=" + x.spawnerRadius + " mobcoeff=" + hx(x.mobCoeff) + " mobradius=" + x.mobRadius + " costh=" + (x.costH == null ? "-" : hx(x.costH));
    }

    static void applyK(K x) {
        var s = Baritone.settings();
        s.backtrackCostFavoringCoefficient.value = x.backtrack;
        s.pathingMaxChunkBorderFetch.value = x.borderFetch;
        s.minimumImprovementRepropagation.value = x.minImprove;
        s.pathCutoffMinimumLength.value = x.cutMin;
        s.pathCutoffFactor.value = x.cutFactor;
        s.cutoffAtLoadBoundary.value = x.loadBoundary;
        s.simplifyUnloadedYCoord.value = x.simplify;
        s.avoidance.value = x.avoidance;
        s.mobSpawnerAvoidanceCoefficient.value = x.spawnerCoeff;
        s.mobSpawnerAvoidanceRadius.value = x.spawnerRadius;
        s.mobAvoidanceCoefficient.value = x.mobCoeff;
        s.mobAvoidanceRadius.value = x.mobRadius;
        s.slowPath.value = false;
        s.chatDebug.value = false;
        s.desktopNotifications.value = false;
    }

    // ------------------------------------------------------------------------------------------------------------
    // Goals
    // ------------------------------------------------------------------------------------------------------------

    static final class GG {
        Goal goal;
        String spec;
    }

    static final class Sampler {
        final World wd;
        final Random r;

        Sampler(World wd, Random r) {
            this.wd = wd;
            this.r = r;
        }

        /** A standing position: the cell above the top block of a random column, in world coordinates. */
        int[] surface() {
            for (int tries = 0; tries < 60; tries++) {
                int lx = r.nextInt(wd.w), lz = r.nextInt(wd.d);
                int ly = -1;
                for (int y = wd.h - 3; y >= 1; y--) {
                    if (!isAir(wd.cell(lx, y - 1, lz)) && isAir(wd.cell(lx, y, lz)) && isAir(wd.cell(lx, y + 1, lz))) {
                        ly = y;
                        if (r.nextInt(3) != 0) break;
                    }
                }
                if (ly >= 0) {
                    return new int[]{wd.ox + lx, wd.minYv() + ly, wd.oz + lz};
                }
            }
            return new int[]{wd.ox + wd.w / 2, wd.minYv() + 2, wd.oz + wd.d / 2};
        }

        int[] anywhere() {
            return new int[]{wd.ox + r.nextInt(wd.w), wd.minYv() + r.nextInt(wd.h), wd.oz + r.nextInt(wd.d)};
        }

        int[] outside() {
            int[] p = surface();
            int dir = r.nextInt(4);
            int far = 8 + r.nextInt(100);
            return new int[]{p[0] + (dir == 0 ? far : dir == 1 ? -far : 0), p[1] + r.nextInt(9) - 4, p[2] + (dir == 2 ? far : dir == 3 ? -far : 0)};
        }

        int[] near(int[] c, int maxDist) {
            for (int tries = 0; tries < 80; tries++) {
                int[] p = surface();
                int dx = p[0] - c[0], dz = p[2] - c[2];
                if (Math.abs(dx) <= maxDist && Math.abs(dz) <= maxDist) {
                    return p;
                }
            }
            return new int[]{c[0] + r.nextInt(7) - 3, c[1], c[2] + r.nextInt(7) - 3};
        }

        int[] pos(int[] start, int mode) {
            return switch (mode) {
                case 0, 1, 2 -> near(start, 12 + r.nextInt(30));
                case 3 -> surface();
                case 4 -> anywhere();
                case 5 -> outside();
                default -> new int[]{start[0] + r.nextInt(9) - 4, start[1] + r.nextInt(5) - 2, start[2] + r.nextInt(9) - 4};
            };
        }
    }

    static GG gen(Sampler s, Random r, int cls, int depth, int[] start, boolean reachable) {
        GG o = new GG();
        int pm = reachable ? r.nextInt(3) : (r.nextInt(100) < 55 ? r.nextInt(3) : 3 + r.nextInt(4));
        int[] p = s.pos(start, pm);
        int x = p[0], y = p[1], z = p[2];
        switch (cls) {
            case 0 -> {
                o.goal = new GoalBlock(x, y, z);
                o.spec = "block(" + x + "," + y + "," + z + ")";
            }
            case 1 -> {
                o.goal = new GoalXZ(x, z);
                o.spec = "xz(" + x + "," + z + ")";
            }
            case 2 -> {
                int lv = r.nextInt(4) == 0 ? y + r.nextInt(30) - 10 : y;
                o.goal = new GoalYLevel(lv);
                o.spec = "ylevel(" + lv + ")";
            }
            case 3 -> {
                o.goal = new GoalTwoBlocks(new BlockPos(x, y, z));
                o.spec = "twoblocks(" + x + "," + y + "," + z + ")";
            }
            case 4 -> {
                o.goal = new GoalGetToBlock(new BlockPos(x, y, z));
                o.spec = "gettoblock(" + x + "," + y + "," + z + ")";
            }
            case 5 -> {
                int range = r.nextInt(7);
                o.goal = new GoalNear(new BlockPos(x, y, z), range);
                o.spec = "near(" + x + "," + y + "," + z + "," + range + ")";
            }
            case 6 -> {
                o.goal = new GoalAxis();
                o.spec = "axis()";
            }
            case 7 -> {
                Direction[] ds = {Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST};
                Direction dir = ds[r.nextInt(4)];
                BlockPos origin = new BlockPos(start[0] + r.nextInt(5) - 2, start[1], start[2] + r.nextInt(5) - 2);
                o.goal = new GoalStrictDirection(origin, dir);
                o.spec = "strict(" + origin.getX() + "," + origin.getY() + "," + origin.getZ() + "," + dir.getStepX() + "," + dir.getStepZ() + ")";
            }
            case 8 -> {
                int count = 1 + r.nextInt(3);
                double dist = r.nextInt(25) + new double[]{0.0, 0.5, 0.8, 0.9}[r.nextInt(4)];
                Integer maintain = r.nextInt(100) < 60 ? null : Integer.valueOf(start[1] + r.nextInt(3) - 1);
                BlockPos[] from = new BlockPos[count];
                StringBuilder sb = new StringBuilder("runaway(").append(hx(dist)).append(',').append(maintain == null ? "n" : "" + maintain);
                for (int i = 0; i < count; i++) {
                    int fx = start[0] + r.nextInt(9) - 4, fy = start[1] + r.nextInt(5) - 2, fz = start[2] + r.nextInt(9) - 4;
                    from[i] = new BlockPos(fx, fy, fz);
                    sb.append(',').append(fx).append(',').append(fy).append(',').append(fz);
                }
                o.goal = new GoalRunAway(dist, maintain, from);
                o.spec = sb.append(')').toString();
            }
            case 9, 10 -> {
                if (depth >= 2) {
                    return gen(s, r, r.nextInt(9), depth, start, reachable);
                }
                if (cls == 10) {
                    GG in = gen(s, r, r.nextInt(11), depth + 1, start, reachable);
                    o.goal = new GoalInverted(in.goal);
                    o.spec = "inverted(" + in.spec + ")";
                } else {
                    int count = r.nextInt(8) == 0 ? r.nextInt(2) : 2 + r.nextInt(2);
                    Goal[] gs = new Goal[count];
                    StringBuilder sb = new StringBuilder("composite(");
                    for (int i = 0; i < count; i++) {
                        GG in = gen(s, r, r.nextInt(11), depth + 1, start, reachable);
                        gs[i] = in.goal;
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

    static final int[] CLASS_WEIGHT = {22, 12, 6, 6, 8, 8, 3, 5, 6, 12, 6};

    static int pickClass(Random r) {
        int t = r.nextInt(100), acc = 0;
        for (int i = 0; i < CLASS_WEIGHT.length; i++) {
            acc += CLASS_WEIGHT[i];
            if (t < acc) return i;
        }
        return 0;
    }

    /** Counts the goal's isInGoal calls (one per node taken out of the open set), hashes the positions, and cancels the search at the limit. */
    static final class Counting implements Goal {
        final Goal inner;
        AbstractNodeCostSearch finder;
        long[] hist = new long[1024];
        int n;
        long h = 0xcbf29ce484222325L;
        boolean armed;
        boolean fired;
        int limit = Integer.MAX_VALUE;

        Counting(Goal inner) {
            this.inner = inner;
            hist[0] = h;
        }

        static long step(long h, int v) {
            h ^= (v & 0xFFFFFFFFL);
            return h * 0x100000001b3L;
        }

        @Override
        public boolean isInGoal(int x, int y, int z) {
            if (armed) {
                h = step(step(step(h, x), y), z);
                if (n + 2 >= hist.length) {
                    hist = java.util.Arrays.copyOf(hist, hist.length * 2);
                }
                hist[++n] = h;
                if (n >= limit) {
                    fired = true;
                    finder.cancel();
                }
            }
            return inner.isInGoal(x, y, z);
        }

        @Override
        public double heuristic(int x, int y, int z) {
            return inner.heuristic(x, y, z);
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // Running one search with upstream's classes
    // ------------------------------------------------------------------------------------------------------------

    static Object unsafe;
    static Method allocate;

    static PathingBehavior behaviorWith(IPlayerContext ctx) throws Exception {
        if (allocate == null) {
            Field f = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
            f.setAccessible(true);
            unsafe = f.get(null);
            allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        }
        PathingBehavior pb = (PathingBehavior) allocate.invoke(unsafe, PathingBehavior.class);
        Field cf = baritone.behavior.Behavior.class.getDeclaredField("ctx");
        cf.setAccessible(true);
        cf.set(pb, ctx);
        return pb;
    }

    static Method createPathfinder;

    static IBaritone baritoneProxy(BetterBlockPos feet, List<BlockPos> spawners) {
        ICachedWorld cached = (ICachedWorld) Proxy.newProxyInstance(ICachedWorld.class.getClassLoader(), new Class<?>[]{ICachedWorld.class}, (p, m, a) -> {
            if (m.getName().equals("getLocationsOf")) return new ArrayList<>(spawners);
            throw new UnsupportedOperationException(m.getName());
        });
        IWorldData wdata = (IWorldData) Proxy.newProxyInstance(IWorldData.class.getClassLoader(), new Class<?>[]{IWorldData.class}, (p, m, a) -> {
            if (m.getName().equals("getCachedWorld")) return cached;
            throw new UnsupportedOperationException(m.getName());
        });
        IPlayerContext pc = (IPlayerContext) Proxy.newProxyInstance(IPlayerContext.class.getClassLoader(), new Class<?>[]{IPlayerContext.class}, (p, m, a) -> {
            switch (m.getName()) {
                case "playerFeet":
                    return feet;
                case "worldData":
                    return wdata;
                case "entitiesStream":
                    return Stream.empty();
                default:
                    throw new UnsupportedOperationException(m.getName());
            }
        });
        return (IBaritone) Proxy.newProxyInstance(IBaritone.class.getClassLoader(), new Class<?>[]{IBaritone.class}, (p, m, a) -> {
            if (m.getName().equals("getPlayerContext")) return pc;
            throw new UnsupportedOperationException(m.getName());
        });
    }

    static final class Rec {
        String type;
        IPath result;
        StringBuilder text = new StringBuilder();
        int nodes;
        boolean hardHit;
        int capUsed;
    }

    static String xyz(int x, int y, int z) {
        return x + "," + y + "," + z;
    }

    static String bpos(BlockPos p) {
        return xyz(p.getX(), p.getY(), p.getZ());
    }

    static String pathText(IPath p, boolean moves) {
        if (p == null) {
            return "-";
        }
        StringBuilder sb = new StringBuilder("P").append(p.positions().size()).append('/').append(p.getNumNodesConsidered()).append('/').append(bpos(p.getSrc()));
        for (BetterBlockPos q : p.positions()) {
            sb.append(' ').append(q.x).append(',').append(q.y).append(',').append(q.z);
        }
        if (moves) {
            sb.append(" |");
            for (IMovement m : p.movements()) {
                Movement mm = (Movement) m;
                sb.append(' ').append(mm.getClass().getSimpleName().replace("Movement", "")).append(':').append(bpos(mm.getDest())).append(':').append(hx(mm.getCost())).append(':').append(mm.calculatedWhileLoaded() ? 't' : 'f');
            }
        }
        return sb.toString();
    }

    static Field fGoal, fFavoring, fFavorings;

    /** Everything about one search. cap -1 is "to its end" through calculate(); anything else is the manual run with a node cap. */
    static Rec search(World wd, int cfg, int kIdx, GG g, int[] start, int[] feet, List<BlockPos> spawners, List<BlockPos> mobs, IPath previous, int cap) throws Exception {
        PrintStream out = System.out, err = System.err;
        PrintStream nul = new PrintStream(OutputStream.nullOutputStream());
        System.setOut(nul);
        System.setErr(nul);
        try {
            Rec rec = searchInner(wd, cfg, kIdx, g, start, feet, spawners, mobs, previous, cap);
            rec.capUsed = cap;
            if (rec.hardHit) {
                rec = searchInner(wd, cfg, kIdx, g, start, feet, spawners, mobs, previous, HARD);
                rec.capUsed = HARD;
            }
            return rec;
        } finally {
            System.setOut(out);
            System.setErr(err);
        }
    }

    static Rec searchInner(World wd, int cfg, int kIdx, GG g, int[] start, int[] feet, List<BlockPos> spawners, List<BlockPos> mobs, IPath previous, int cap) throws Exception {
        K k = KS.get(kIdx);
        applyK(k);
        BetterBlockPos feetPos = new BetterBlockPos(feet[0], feet[1], feet[2]);
        IBaritone bar = baritoneProxy(feetPos, spawners);
        CalculationContext ctx = SearchBridge.context(bar, wd, cfg);
        if (k.costH != null) {
            Baritone.settings().costHeuristic.value = k.costH;
        }
        if (createPathfinder == null) {
            createPathfinder = PathingBehavior.class.getDeclaredMethod("createPathfinder", BlockPos.class, Goal.class, IPath.class, CalculationContext.class);
            createPathfinder.setAccessible(true);
            fGoal = AbstractNodeCostSearch.class.getDeclaredField("goal");
            fGoal.setAccessible(true);
            fFavoring = AStarPathFinder.class.getDeclaredField("favoring");
            fFavoring.setAccessible(true);
            fFavorings = Class.forName("baritone.utils.pathing.Favoring").getDeclaredField("favorings");
            fFavorings.setAccessible(true);
        }
        PathingBehavior pb = behaviorWith(bar.getPlayerContext());
        AStarPathFinder finder = (AStarPathFinder) createPathfinder.invoke(pb, new BlockPos(start[0], start[1], start[2]), g.goal, previous, ctx);
        // the mobs: Avoidance.create asks the entity list, which the harness does not have, so the same avoidances are applied here, after the spawners
        if (k.avoidance && k.mobCoeff != 1.0D) {
            Long2DoubleOpenHashMap map = (Long2DoubleOpenHashMap) fFavorings.get(fFavoring.get(finder));
            for (BlockPos m : mobs) {
                new Avoidance(m, k.mobCoeff, k.mobRadius).applySpherical(map);
            }
        }
        Goal simplified = finder.getGoal();
        Counting cg = new Counting(simplified);
        cg.finder = finder;
        fGoal.set(finder, cg);

        Rec rec = new Rec();
        String simp = "-";
        if (simplified != g.goal) {
            GoalXZ xz = (GoalXZ) simplified;
            simp = xz.getX() + "," + xz.getZ();
        }
        boolean manual = cap > 0;
        cg.armed = true;
        cg.limit = manual ? cap : HARD;
        IPath s1 = null, s2 = null, s3 = null, fin = null;
        String type;
        try {
            if (!manual) {
                PathCalculationResult res = finder.calculate(FAR, FAR);
                if (cg.fired) {
                    rec.hardHit = true;
                    return rec;
                }
                type = res.getType().name();
                fin = res.getPath().orElse(null);
            } else {
                ctx.claimSearchCaches();
                try {
                    Optional<IPath> found = finder.calculate0(FAR, FAR);
                    cg.armed = false;
                    if (found.isEmpty() && cg.fired) {
                        found = finder.bestSoFar(false, finder.numNodesConsidered);
                    }
                    s1 = found.map(IPath::postProcess).orElse(null);
                    if (s1 == null) {
                        type = "FAILURE";
                    } else {
                        s2 = s1.cutoffAtLoadedChunks(ctx.bsi);
                        s3 = s2.staticCutoff(finder.getGoal());
                        fin = s3;
                        type = finder.getGoal().isInGoal(fin.getDest()) ? "SUCCESS_TO_GOAL" : "SUCCESS_SEGMENT";
                    }
                } finally {
                    ctx.releaseSearchCaches();
                }
            }
        } catch (Exception e) {
            type = "EXCEPTION";
        }
        cg.armed = false;
        rec.type = type;
        rec.result = fin;
        StringBuilder t = rec.text;
        if (type.equals("EXCEPTION")) {
            t.append("r EXCEPTION 0 0 0 0 -\n");
            return rec;
        }
        rec.nodes = finder.numNodesConsidered;
        t.append("r ").append(type).append(' ').append(finder.numNodesConsidered).append(' ').append(finder.numMovementsConsidered).append(' ').append(finder.mapSize()).append(' ')
                .append(Long.toHexString(cg.hist[finder.numNodesConsidered])).append(' ').append(simp).append('\n');
        t.append("b");
        for (int i = 0; i < finder.bestSoFar.length; i++) {
            PathNode b = finder.bestSoFar[i];
            t.append(' ').append(b.x).append(',').append(b.y).append(',').append(b.z).append(',').append(hx(b.cost));
        }
        t.append('\n');
        PathNode mr = finder.mostRecentConsidered;
        t.append("n ").append(mr == null ? "-" : xyz(mr.x, mr.y, mr.z)).append('\n');
        t.append("q ").append(pathText(finder.pathToMostRecentNodeConsidered().orElse(null), false)).append('\n');
        t.append("u ").append(pathText(finder.bestPathSoFar().orElse(null), false)).append('\n');
        if (manual) {
            t.append("s1 ").append(pathText(s1, true)).append('\n');
            t.append("s2 ").append(pathText(s2, true)).append('\n');
            t.append("s3 ").append(pathText(s3, true)).append('\n');
        }
        t.append("sf ").append(pathText(fin, true)).append('\n');
        return rec;
    }

    // ------------------------------------------------------------------------------------------------------------
    // The dump
    // ------------------------------------------------------------------------------------------------------------

    static String pointList(List<BlockPos> l) {
        if (l.isEmpty()) {
            return "-";
        }
        StringBuilder sb = new StringBuilder();
        for (BlockPos p : l) {
            if (sb.length() > 0) sb.append(';');
            sb.append(bpos(p));
        }
        return sb.toString();
    }

    static String rle(int[] cells) {
        StringBuilder sb = new StringBuilder();
        int i = 0;
        while (i < cells.length) {
            int j = i;
            while (j < cells.length && cells[j] == cells[i]) j++;
            if (sb.length() > 0) sb.append(',');
            sb.append(cells[i]);
            if (j - i > 1) sb.append('x').append(j - i);
            i = j;
        }
        return sb.toString();
    }

    static int pickWeighted(Random r, int[] weights, int n) {
        int total = 0;
        for (int i = 0; i < n; i++) total += weights[i];
        int t = r.nextInt(total), acc = 0;
        for (int i = 0; i < n; i++) {
            acc += weights[i];
            if (t < acc) return i;
        }
        return 0;
    }

    static int pickCap(Random r) {
        int t = r.nextInt(100);
        if (t < 6) return 1;
        if (t < 10) return 2;
        if (t < 15) return 63;
        if (t < 21) return 64;
        if (t < 26) return 65;
        if (t < 44) return 500;
        if (t < 66) return 5000;
        return -1;
    }

    static int eid;
    static int chainId;
    static long searches, unlimited, hardHits;

    @Test
    public void dump() throws Exception {
        int worldCount = Integer.parseInt(System.getenv().getOrDefault("SEARCH_WORLDS", "60"));
        int casesPerWorld = Integer.parseInt(System.getenv().getOrDefault("SEARCH_CASES", "24"));
        Path outPath = Path.of(System.getenv("SEARCH_OUT"));
        Random master = new Random(0x5EA2C4L);
        int[][] sizes = {{48, 32, 48}, {64, 48, 64}, {32, 40, 96}, {96, 24, 32}, {64, 64, 64}, {48, 48, 80}, {80, 36, 48}};
        int[][] origins = {{0, 0}, {0, 0}, {-48, -80}, {-16, 32}, {1_000_000, -2_000_000}, {29_999_936, 12_000}, {-29_999_984, 0}, {32, -30_000_000}, {16, -30}, {-30, -20}, {-8, -40}};
        try (BufferedWriter w = Files.newBufferedWriter(outPath)) {
            w.write("H search-compare 1\n");
            for (int i = 0; i < SearchBridge.paletteSize(); i++) {
                w.write(SearchBridge.stateLine(i) + "\n");
            }
            for (int i = 0; i < SearchBridge.configCount(); i++) {
                w.write(SearchBridge.configLine(i) + "\n");
            }
            for (int i = 0; i < KS.size(); i++) {
                w.write(kline(i) + "\n");
            }
            for (int wi = 0; wi < worldCount; wi++) {
                long seed = master.nextLong();
                Random r = new Random(seed);
                int kind = wi % KINDS.length;
                int[] sz = sizes[r.nextInt(sizes.length)];
                int[] org = origins[r.nextInt(origins.length)];
                // 0 and -64 as before; -20 and -3 put the surface or the towers across y = 0, as in a real Overworld
                int minYPick = r.nextInt(8);
                int minY = minYPick < 4 ? 0 : (minYPick < 6 ? -64 : (minYPick == 6 ? -20 : -3));
                Vox vox = generate(kind, sz[0], sz[1], sz[2], r, seed);
                // chunks: mostly loaded, some only cached, some not loaded
                int cxn = sz[0] >> 4, czn = sz[2] >> 4;
                byte[] chunks = new byte[cxn * czn];
                int unloadPct = new int[]{0, 0, 10, 25, 40}[r.nextInt(5)];
                for (int c = 0; c < chunks.length; c++) {
                    int t = r.nextInt(100);
                    chunks[c] = (byte) (t < unloadPct ? 0 : (t < unloadPct + unloadPct / 2 + (r.nextInt(5) == 0 ? 10 : 0) ? 1 : 2));
                }
                WorldBorder wb = new WorldBorder();
                int bt = r.nextInt(10);
                if (bt >= 6) {
                    double cx = org[0] + sz[0] / 2.0 + r.nextInt(17) - 8, cz = org[1] + sz[2] / 2.0 + r.nextInt(17) - 8;
                    wb.setCenter(cx, cz);
                    wb.setSize(Math.max(sz[0], sz[2]) * (0.5 + r.nextInt(12) * 0.1));
                }
                World wd = new World(KINDS[kind], wb, minY, org[0], org[1], sz[0], sz[1], sz[2], vox.c, chunks);
                writeWorld(w, wi, wd, wb, chunks, vox);
                Sampler sampler = new Sampler(wd, r);
                for (int ci = 0; ci < casesPerWorld; ci++) {
                    runChain(w, wd, wi, r, sampler);
                }
            }
            specials(w, worldCount);
            w.write("J searches=" + searches + " unlimited=" + unlimited + " hardhits=" + hardHits + "\n");
        }
    }

    static GG blockGoal(int x, int y, int z) {
        GG g = new GG();
        g.goal = new GoalBlock(x, y, z);
        g.spec = "block(" + x + "," + y + "," + z + ")";
        return g;
    }

    static GG levelGoal(int y) {
        GG g = new GG();
        g.goal = new GoalYLevel(y);
        g.spec = "ylevel(" + y + ")";
        return g;
    }

    /** One search of the default settings, alone in its chain, from a position where the player stands. */
    static void one(BufferedWriter w, int wi, World wd, int cfg, GG g, int[] start, int cap) throws Exception {
        writeSearch(w, chainId++, 0, "plain", wi, cfg, 0, g, start, start, new ArrayList<>(), new ArrayList<>(), null, -1, cap, wd);
    }

    static World bigWorld(String kind, int minY, int ox, int oz, Vox v) {
        byte[] chunks = new byte[(v.w >> 4) * (v.d >> 4)];
        java.util.Arrays.fill(chunks, (byte) 2);
        return new World(kind, new WorldBorder(), minY, ox, oz, v.w, v.h, v.d, v.c, chunks);
    }

    /**
     * Worlds the generators cannot make because of their size, hand built. They reach what the random worlds cannot: a search across y = 0 over a
     * 4064-high column (the y part of the cell key wraps at 2048 blocks, and the cell keys of nodes 256 apart must not tie), and a one-wide corridor
     * long enough for upstream's squared distance from the start (an int) to wrap past 46340 blocks.
     */
    static void specials(BufferedWriter w, int wi) throws Exception {
        // 1. a tall world from y = -2032 to 2031: ladder column, a platform at y = 0, two high ones, a pool and a hole
        {
            Vox v = new Vox(32, 4064, 32);
            int minY = -2032, ox = -16, oz = -16;
            v.fill(0, 0, 0, 31, 0, 31, STONE);
            for (int y = 1; y < 4060; y++) {
                v.set(8, y, 8, LADDER);
                v.set(9, y, 8, STONE);
            }
            v.fill(12, 2031, 12, 18, 2031, 18, STONE);
            v.fill(20, 3000, 20, 28, 3000, 28, STONE);
            v.fill(4, 3400, 4, 6, 3400, 6, STONE);
            v.set(24, 1, 24, WATER);
            v.set(24, 3000, 24, 0);
            World wd = bigWorld("tall", minY, ox, oz, v);
            writeWorld(w, wi, wd, wd.wb, wd.chunks, v);
            for (int cfg = 0; cfg < SearchBridge.configCount(); cfg++) {
                int[] s = {ox + 7, minY + 1, oz + 8};
                for (int gy : new int[]{200, 2040, 2060, 3050, 4050}) {
                    one(w, wi, wd, cfg, blockGoal(ox + 8, minY + gy, oz + 8), s, -1);
                }
                one(w, wi, wd, cfg, levelGoal(minY + 2500), s, 5000);
                one(w, wi, wd, cfg, levelGoal(minY + 2500), s, 2100);
                int[] top = {ox + 22, minY + 3001, oz + 22};
                one(w, wi, wd, cfg, blockGoal(ox + 24, minY + 1, oz + 24), top, -1);
                one(w, wi, wd, cfg, levelGoal(minY + 1), top, 5000);
                int[] lad = {ox + 8, minY + 2100, oz + 8};
                one(w, wi, wd, cfg, blockGoal(ox + 8, minY + 30, oz + 8), lad, 5000);
                int[] hi = {ox + 5, minY + 3401, oz + 5};
                one(w, wi, wd, cfg, blockGoal(ox + 15, minY + 1, oz + 15), hi, 5000);
                int[] mid = {ox + 15, minY + 2032, oz + 15};
                one(w, wi, wd, cfg, blockGoal(ox + 8, minY + 2000, oz + 8), mid, -1);
                one(w, wi, wd, cfg, blockGoal(ox + 8, minY + 2100, oz + 8), mid, -1);
            }
            wi++;
        }
        // 2. a one-wide ladder shaft 4064 high: a search that goes 4000 nodes straight up or down, and is cut at 2047 and 2049
        {
            int h = 4064, minY = -2032, ox = -16, oz = -32;
            Vox v = new Vox(16, h, 16);
            v.fill(0, 0, 0, 15, h - 1, 15, BEDROCK);
            for (int y = 1; y < h - 3; y++) v.set(8, y, 8, LADDER);
            World wd = bigWorld("shaft", minY, ox, oz, v);
            writeWorld(w, wi, wd, wd.wb, wd.chunks, v);
            int x = ox + 8, z = oz + 8;
            for (int cfg = 0; cfg < SearchBridge.configCount(); cfg++) {
                int[] bot = {x, minY + 1, z}, top = {x, minY + 4000, z};
                one(w, wi, wd, cfg, blockGoal(x, minY + 4000, z), bot, -1);
                one(w, wi, wd, cfg, blockGoal(x, minY + 1, z), top, -1);
                one(w, wi, wd, cfg, blockGoal(x, minY + 4000, z), bot, 2047);
                one(w, wi, wd, cfg, blockGoal(x, minY + 4000, z), bot, 2049);
            }
            wi++;
        }
        // 3. a one-wide corridor 70000 long along z from a negative z: the squared distance of a node from the start wraps in an int at 46341 blocks
        {
            Vox v = new Vox(16, 5, 70000);
            int ox = 32, oz = -35000;
            v.fill(0, 0, 0, 15, 4, 69999, BEDROCK);
            v.fill(8, 1, 0, 8, 2, 69999, 0);
            World wd = bigWorld("corridor", 0, ox, oz, v);
            writeWorld(w, wi, wd, wd.wb, wd.chunks, v);
            int[] s = {ox + 8, 1, oz + 2};
            for (int cap : new int[]{46000, 46400, 65540}) {
                one(w, wi, wd, 0, blockGoal(ox + 8, 1, oz + 69000), s, cap);
            }
            wi++;
        }
    }

    static void writeWorld(BufferedWriter w, int wi, World wd, WorldBorder wb, byte[] chunks, Vox vox) throws Exception {
        StringBuilder cs = new StringBuilder();
        for (byte c : chunks) cs.append((char) ('0' + c));
        w.write("V " + wi + " kind=" + wd.kind + " ox=" + wd.ox + " oz=" + wd.oz + " miny=" + wd.minYv() + " w=" + wd.w + " h=" + wd.h + " d=" + wd.d + " border=" + wb.getMinX() + "," + wb.getMaxX() + "," + wb.getMinZ() + "," + wb.getMaxZ()
                + " chunks=" + cs + " cells=" + rle(vox.c) + "\n");
    }

    /** Offsets that move an avoidance centre where upstream's long hash of the positions around it either equals the hash of the positions around the start, or nearly does. */
    static final int[][] HASH_SHIFTS = {
            // longHash(x, y + k, z - k * 2873465) == longHash(x, y, z): the sphere lands on the start's cells (bug 7 of known-missed)
            {0, 1, -2873465}, {0, 2, -2 * 2873465}, {0, -1, 2873465},
            // 2^24 apart in x or z, 256 apart in y: equal if the hash masked the coordinate, different in upstream
            {1 << 24, 0, 0}, {-(1 << 24), 0, 0}, {0, 0, 1 << 24}, {0, 0, -(1 << 24)}, {0, 256, 0}, {0, -256, 0}};

    static BlockPos shifted(BlockPos p, int[] d) {
        return new BlockPos(p.getX() + d[0], p.getY() + d[1], p.getZ() + d[2]);
    }

    static void runChain(BufferedWriter w, World wd, int wi, Random r, Sampler sampler) throws Exception {
        int chain = chainId++;
        int cfg = r.nextInt(SearchBridge.configCount());
        int kIdx = pickWeighted(r, K_WEIGHT, KS.size());
        K k = KS.get(kIdx);
        int[] start = sampler.surface();
        if (r.nextInt(40) == 0) {
            start = sampler.anywhere();
        }
        int[] feet = start.clone();
        int fr = r.nextInt(100);
        boolean fakeNode = false;
        if (fr < 12) {
            feet[0] += r.nextInt(3) - 1;
            feet[2] += r.nextInt(3) - 1;
        } else if (fr < 16) {
            feet[0] += 2 + r.nextInt(3);
        } else if (fr < 19) {
            feet[1] += 1;
        }
        boolean reachable = r.nextInt(100) < 70;
        GG g;
        if (r.nextInt(100) < 5) {
            // a goal that is the start itself, with the player a block away: the path has one position and a fake first node
            g = new GG();
            g.goal = new GoalBlock(start[0], start[1], start[2]);
            g.spec = "block(" + xyz(start[0], start[1], start[2]) + ")";
            feet[0] = start[0] + (r.nextBoolean() ? 1 : -1);
            feet[1] = start[1];
            feet[2] = start[2] + r.nextInt(3) - 1;
        } else {
            g = gen(sampler, r, pickClass(r), 0, start, reachable);
        }
        List<BlockPos> spawners = new ArrayList<>(), mobs = new ArrayList<>();
        if (k.avoidance || r.nextInt(100) < 25) {
            // also when the setting is off: the lists are then ignored, and a comparison has to show that
            for (int i = r.nextInt(3); i > 0; i--) spawners.add(new BlockPos(start[0] + r.nextInt(21) - 10, start[1] + r.nextInt(7) - 3, start[2] + r.nextInt(21) - 10));
            for (int i = 1 + r.nextInt(4); i > 0; i--) mobs.add(new BlockPos(start[0] + r.nextInt(21) - 10, start[1] + r.nextInt(7) - 3, start[2] + r.nextInt(21) - 10));
        }
        if (r.nextInt(100) < 30) {
            // some of the centres far away, where only upstream's hash can tie them to the search area
            for (List<BlockPos> l : List.of(spawners, mobs)) {
                for (int i = 0; i < l.size(); i++) {
                    if (r.nextBoolean()) l.set(i, shifted(l.get(i), HASH_SHIFTS[r.nextInt(HASH_SHIFTS.length)]));
                }
            }
        }
        IPath prev = null;
        int prevId = -1;
        if (r.nextInt(100) < 15) {
            GG og = gen(sampler, r, pickClass(r), 0, start, true);
            prevId = writeSearch(w, chain, 0, "oracle", wi, cfg, kIdx, og, start, feet, spawners, mobs, null, -1, 30, wd);
            prev = lastResult;
        }
        int segs = r.nextInt(100) < 55 ? 1 : 2 + r.nextInt(3);
        IPath[] acc = new IPath[2];
        boolean[] dead = new boolean[2];
        IPath firstSeg = null;
        int[] curStart = start;
        for (int seg = 0; seg < segs; seg++) {
            int cap = pickCap(r);
            GG sg = g;
            if (seg > 0 && r.nextInt(100) < 25) {
                sg = gen(sampler, r, pickClass(r), 0, start, true);
            }
            String role = seg == 0 ? "plain" : "next";
            int id = writeSearch(w, chain, seg, role, wi, cfg, kIdx, sg, curStart, feet, spawners, mobs, prev, prevId, cap, wd);
            IPath res = lastResult;
            String type = lastType;
            if (seg == 0) {
                firstSeg = res;
            } else {
                for (int a = 0; a < 2; a++) {
                    if (dead[a]) continue;
                    boolean allow = a == 1;
                    IPath first = acc[a] != null ? acc[a] : firstSeg;
                    String text;
                    try {
                        Optional<SplicedPath> sp = SplicedPath.trySplice(first, res, allow);
                        if (sp.isPresent()) {
                            acc[a] = sp.get();
                            text = pathText(acc[a], true);
                        } else {
                            text = "-";
                            dead[a] = true;
                        }
                    } catch (IllegalStateException e) {
                        text = "ERR";
                        dead[a] = true;
                    }
                    w.write("t " + chain + " " + seg + " " + (allow ? 1 : 0) + " " + text + "\n");
                }
            }
            if (!type.equals("SUCCESS_SEGMENT") || res == null) {
                break;
            }
            prev = res;
            prevId = id;
            curStart = new int[]{res.getDest().x, res.getDest().y, res.getDest().z};
            g = sg;
        }
    }

    static IPath lastResult;
    static String lastType;

    static int writeSearch(BufferedWriter w, int chain, int seg, String role, int wi, int cfg, int kIdx, GG g, int[] start, int[] feet, List<BlockPos> spawners, List<BlockPos> mobs, IPath prev, int prevId, int cap, World wd) throws Exception {
        Rec rec = search(wd, cfg, kIdx, g, start, feet, spawners, mobs, prev, cap);
        int id = eid++;
        searches++;
        if (cap < 0) unlimited++;
        int writtenCap = rec.capUsed;
        if (cap < 0 && rec.capUsed == HARD) hardHits++;
        w.write("E id=" + id + " chain=" + chain + " seg=" + seg + " role=" + role + " w=" + wi + " cfg=" + cfg + " set=" + kIdx + " cap=" + (writtenCap < 0 ? "u" : "" + writtenCap) + " goal=" + g.spec
                + " start=" + xyz(start[0], start[1], start[2]) + " feet=" + xyz(feet[0], feet[1], feet[2]) + " prev=" + prevId + " sp=" + pointList(spawners) + " mob=" + pointList(mobs) + "\n");
        w.write(rec.text.toString());
        lastResult = rec.result;
        lastType = rec.type;
        return id;
    }
}
