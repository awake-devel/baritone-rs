/*
 * baritone-rs benchmark tool: times upstream's real search (AStarPathFinder, through PathingBehavior.createPathfinder and
 * AbstractNodeCostSearch.calculate) over large generated worlds, and writes the worlds and the answers for the Rust side to read.
 * Copy it, with the files of tools/cost-compare/java and tools/search-compare/java, into a checkout of upstream (run_bench.py does it).
 * Licensed like the code it exercises: GNU LGPL, version 3 or (at your option) any later version.
 *
 * Rule: this file holds no second implementation of the search. It generates a world (the only world generator of the benchmark;
 * the Rust side reads the file this writes), builds upstream's own context and finder, and times calculate().
 * It is in this package because the finder's counters are protected fields and Path is package-private.
 */
package baritone.pathing.calc;

import baritone.Baritone;
import baritone.api.IBaritone;
import baritone.api.pathing.calc.IPath;
import baritone.api.pathing.goals.Goal;
import baritone.api.pathing.goals.GoalBlock;
import baritone.api.pathing.goals.GoalXZ;
import baritone.api.utils.BetterBlockPos;
import baritone.api.utils.PathCalculationResult;
import baritone.behavior.PathingBehavior;
import baritone.pathing.movement.CalculationContext;
import baritone.pathing.movement.SearchBridge;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.border.WorldBorder;

import java.io.BufferedWriter;
import java.io.OutputStream;
import java.io.PrintStream;
import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Random;

public class SearchBench {
    static final int H = 80;
    static final int SEA = 40;
    static final int[] SIZES = {256, 512, 1024, 384, 768, 640, 896, 320};
    /** The one configuration of the benchmark: config 0 of the cost comparison (upstream's defaults, with a pickaxe, a throwaway block and sprinting) and search settings 0 (upstream's defaults). */
    static final int CFG = 0;
    static final int KSET = 0;

    static int sizeOf(long seed) {
        return SIZES[(int) (seed % SIZES.length)];
    }

    // ------------------------------------------------------------------------------------------------------------
    // The world: noise terrain, sea, rivers, ravines, caves, trees, a few walls
    // ------------------------------------------------------------------------------------------------------------

    static final class Noise2 {
        final float[] v;
        final int nx, nz;
        final double s;

        Noise2(long seed, int w, int d, double scale) {
            s = scale;
            nx = (int) (w / scale) + 3;
            nz = (int) (d / scale) + 3;
            v = new float[nx * nz];
            for (int i = 0; i < nx; i++) {
                for (int j = 0; j < nz; j++) {
                    v[i * nz + j] = (float) SearchGoldenDump.hash01(seed, i, 0, j);
                }
            }
        }

        double at(double x, double z) {
            double fx = x / s, fz = z / s;
            int xi = (int) fx, zi = (int) fz;
            double tx = SearchGoldenDump.smooth(fx - xi), tz = SearchGoldenDump.smooth(fz - zi);
            int i = xi * nz + zi;
            double a = v[i] + (v[i + nz] - v[i]) * tx;
            double b = v[i + 1] + (v[i + nz + 1] - v[i + 1]) * tx;
            return a + (b - a) * tz;
        }
    }

    static final class Noise3 {
        final float[] v;
        final int nx, ny, nz;
        final double s;

        Noise3(long seed, int w, int h, int d, double scale) {
            s = scale;
            nx = (int) (w / scale) + 3;
            ny = (int) (h / scale) + 3;
            nz = (int) (d / scale) + 3;
            v = new float[nx * ny * nz];
            for (int i = 0; i < nx; i++) {
                for (int j = 0; j < ny; j++) {
                    for (int k = 0; k < nz; k++) {
                        v[(i * ny + j) * nz + k] = (float) SearchGoldenDump.hash01(seed, i, j, k);
                    }
                }
            }
        }

        double at(double x, double y, double z) {
            double fx = x / s, fy = y / s, fz = z / s;
            int xi = (int) fx, yi = (int) fy, zi = (int) fz;
            double tx = SearchGoldenDump.smooth(fx - xi), ty = SearchGoldenDump.smooth(fy - yi), tz = SearchGoldenDump.smooth(fz - zi);
            int i = (xi * ny + yi) * nz + zi;
            int dx = ny * nz, dy = nz;
            double c00 = v[i] + (v[i + dx] - v[i]) * tx;
            double c10 = v[i + dy] + (v[i + dx + dy] - v[i + dy]) * tx;
            double c01 = v[i + 1] + (v[i + dx + 1] - v[i + 1]) * tx;
            double c11 = v[i + dy + 1] + (v[i + dx + dy + 1] - v[i + dy + 1]) * tx;
            double a = c00 + (c10 - c00) * ty;
            double b = c01 + (c11 - c01) * ty;
            return a + (b - a) * tz;
        }
    }

    static final class Gen {
        final int size;
        final SearchGoldenDump.Vox vox;
        /** The surface block of each column (x * size + z), before trees. */
        final int[] top;

        Gen(int size, SearchGoldenDump.Vox vox, int[] top) {
            this.size = size;
            this.vox = vox;
            this.top = top;
        }
    }

    static Gen generate(long seed) {
        final int n = sizeOf(seed);
        final long s = seed * 7919L + 17;
        SearchGoldenDump.Vox v = new SearchGoldenDump.Vox(n, H, n);
        int[] top = new int[n * n];
        Noise2 c = new Noise2(s + 1, n, n, 128), m = new Noise2(s + 2, n, n, 48), f = new Noise2(s + 3, n, n, 16), g = new Noise2(s + 4, n, n, 6);
        Noise2 ridge = new Noise2(s + 5, n, n, 90), river = new Noise2(s + 6, n, n, 70), ravine = new Noise2(s + 7, n, n, 55);
        Noise2 patch = new Noise2(s + 8, n, n, 10), entr = new Noise2(s + 9, n, n, 20), lake = new Noise2(s + 15, n, n, 40);
        // how hilly the world is: the four grades repeat over the seeds
        final int rough = (int) (seed % 4);
        final double amp = new double[]{5, 8, 12, 16}[rough];
        Noise3 ca = new Noise3(s + 10, n, H, n, 26), cb = new Noise3(s + 11, n, H, n, 26);
        int[] cells = v.c;
        for (int x = 0; x < n; x++) {
            for (int z = 0; z < n; z++) {
                double hn = 0.50 * (c.at(x, z) - 0.5) * 2 + 0.30 * (m.at(x, z) - 0.5) * 2 + 0.14 * (f.at(x, z) - 0.5) * 2 + 0.06 * (g.at(x, z) - 0.5) * 2;
                double hh = SEA + 5 + amp * hn;
                double rg = ridge.at(x, z);
                if (rough == 3 && rg > 0.68) {
                    hh += (rg - 0.68) / 0.32 * 14;
                }
                if (lake.at(x, z) > 0.86) {
                    hh = Math.min(hh, SEA - 3);
                }
                if (Math.abs(river.at(x, z) - 0.5) < 0.02) {
                    hh = Math.min(hh, SEA - 2);
                }
                if (rough >= 2 && Math.abs(ravine.at(x, z) - 0.5) < 0.006) {
                    hh -= 12;
                }
                int ht = Math.max(6, Math.min(H - 14, (int) Math.round(hh)));
                top[x * n + z] = ht;
                int base = (x * n + z) * H;
                double pt = patch.at(x, z);
                int surface;
                if (ht <= SEA + 1) {
                    surface = SearchGoldenDump.SAND;
                } else if (pt > 0.78) {
                    surface = SearchGoldenDump.GRAVEL;
                } else if (pt < 0.2 && ht > 54) {
                    surface = SearchGoldenDump.STONE;
                } else {
                    surface = SearchGoldenDump.DIRT;
                }
                boolean entrance = entr.at(x, z) > 0.8;
                cells[base] = SearchGoldenDump.BEDROCK;
                for (int y = 1; y <= ht; y++) {
                    int b = y == ht ? surface : (y >= ht - 3 ? (surface == SearchGoldenDump.SAND ? SearchGoldenDump.SAND : SearchGoldenDump.DIRT) : SearchGoldenDump.STONE);
                    if (y >= 3 && y <= (entrance ? ht : ht - 6)) {
                        double na = ca.at(x, y, z), nb = cb.at(x, y, z);
                        if (Math.abs(na - 0.5) < 0.05 && Math.abs(nb - 0.5) < 0.05) {
                            b = y <= 10 ? SearchGoldenDump.LAVA : SearchGoldenDump.AIR;
                        }
                    }
                    cells[base + y] = b;
                }
                for (int y = ht + 1; y <= SEA; y++) {
                    cells[base + y] = SearchGoldenDump.WATER;
                }
                if (ht > SEA + 1 && surface == SearchGoldenDump.DIRT && SearchGoldenDump.hash01(s + 12, x, 0, z) < 0.10) {
                    cells[base + ht + 1] = SearchGoldenDump.GRASS;
                }
            }
        }
        // trees
        for (int x = 3; x < n - 3; x++) {
            for (int z = 3; z < n - 3; z++) {
                int ht = top[x * n + z];
                if (ht > SEA + 1 && cells[(x * n + z) * H + ht] == SearchGoldenDump.DIRT && SearchGoldenDump.hash01(s + 13, x, 0, z) < 0.008) {
                    for (int y = 1; y <= 4; y++) {
                        v.set(x, ht + y, z, SearchGoldenDump.PLANKS);
                    }
                    for (int dx = -2; dx <= 2; dx++) {
                        for (int dz = -2; dz <= 2; dz++) {
                            for (int dy = 3; dy <= 5; dy++) {
                                if ((Math.abs(dx) + Math.abs(dz) <= 3 && dy < 5 || dx == 0 && dz == 0) && v.get(x + dx, ht + dy, z + dz) == SearchGoldenDump.AIR) {
                                    v.set(x + dx, ht + dy, z + dz, SearchGoldenDump.LEAVES);
                                }
                            }
                        }
                    }
                }
            }
        }
        // walls across the land on every third seed: cobblestone, six high, with a gap every 40 to 90 blocks
        if (seed % 3 == 0) {
            Random r = new Random(s + 14);
            for (int k = 0; k < 3; k++) {
                boolean alongX = r.nextBoolean();
                int at = n / 6 + r.nextInt(n * 2 / 3);
                int from = r.nextInt(n / 4), to = n - r.nextInt(n / 4);
                int gap = 40 + r.nextInt(50), gapAt = r.nextInt(gap);
                for (int a = from; a < to; a++) {
                    if ((a + gapAt) % gap < 4) {
                        continue;
                    }
                    int x = alongX ? a : at, z = alongX ? at : a;
                    int ht = top[x * n + z];
                    for (int y = ht + 1; y <= Math.min(H - 1, ht + 6); y++) {
                        v.set(x, y, z, SearchGoldenDump.COBBLE);
                    }
                }
            }
        }
        return new Gen(n, v, top);
    }

    static long fnv(int[] a) {
        long h = 0xcbf29ce484222325L;
        for (int x : a) {
            h ^= (x & 0xFFFFFFFFL);
            h *= 0x100000001b3L;
        }
        return h;
    }

    static long fnvText(String s) {
        long h = 0xcbf29ce484222325L;
        for (int i = 0; i < s.length(); i++) {
            h ^= s.charAt(i);
            h *= 0x100000001b3L;
        }
        return h;
    }

    // ------------------------------------------------------------------------------------------------------------
    // Cases: a start and a goal on the surface
    // ------------------------------------------------------------------------------------------------------------

    static final class Case {
        String id, name;
        long seed;
        int[] start;
        String spec;
        Goal goal;
        boolean stress;
    }

    static boolean standable(Gen g, int x, int z) {
        int n = g.size;
        if (x < 4 || z < 4 || x >= n - 4 || z >= n - 4) {
            return false;
        }
        int ht = g.top[x * n + z];
        int[] c = g.vox.c;
        int base = (x * n + z) * H;
        int b = c[base + ht];
        if (b != SearchGoldenDump.DIRT && b != SearchGoldenDump.SAND && b != SearchGoldenDump.GRAVEL && b != SearchGoldenDump.STONE) {
            return false;
        }
        int a1 = c[base + ht + 1], a2 = c[base + ht + 2];
        return (a1 == SearchGoldenDump.AIR || a1 == SearchGoldenDump.GRASS) && a2 == SearchGoldenDump.AIR;
    }

    /** The nearest standable column to (x, z), searched in growing squares. */
    static int[] nearestStandable(Gen g, int x, int z) {
        for (int r = 0; r < 60; r++) {
            for (int dx = -r; dx <= r; dx++) {
                for (int dz = -r; dz <= r; dz++) {
                    if (Math.max(Math.abs(dx), Math.abs(dz)) == r && standable(g, x + dx, z + dz)) {
                        return new int[]{x + dx, g.top[(x + dx) * g.size + z + dz] + 1, z + dz};
                    }
                }
            }
        }
        throw new IllegalStateException("no standable column near " + x + "," + z);
    }

    static Case makeCase(Gen g, long seed, String name, int[] start, double angle, double dist, boolean xz, boolean stress) {
        int n = g.size;
        int gx = (int) Math.round(start[0] + dist * Math.cos(angle)), gz = (int) Math.round(start[2] + dist * Math.sin(angle));
        gx = Math.max(8, Math.min(n - 9, gx));
        gz = Math.max(8, Math.min(n - 9, gz));
        int[] t = nearestStandable(g, gx, gz);
        Case c = new Case();
        c.seed = seed;
        c.name = name;
        c.id = seed + "." + name;
        c.start = start;
        c.stress = stress;
        if (xz) {
            c.goal = new GoalXZ(t[0], t[2]);
            c.spec = "xz(" + t[0] + "," + t[2] + ")";
        } else {
            int y = t[1];
            if (stress) {
                // inside the rock, 24 blocks under the surface: reachable only by digging
                y = Math.max(8, g.top[t[0] * n + t[2]] - 24);
            }
            c.goal = new GoalBlock(t[0], y, t[2]);
            c.spec = "block(" + t[0] + "," + y + "," + t[2] + ")";
        }
        return c;
    }

    /** The largest distance from (x, z) in a direction that stays inside the world, with a margin. */
    static double room(int n, int x, int z, double angle) {
        double dx = Math.cos(angle), dz = Math.sin(angle), best = 1e9;
        int lo = 12, hi = n - 13;
        if (dx > 1e-9) best = Math.min(best, (hi - x) / dx);
        if (dx < -1e-9) best = Math.min(best, (lo - x) / dx);
        if (dz > 1e-9) best = Math.min(best, (hi - z) / dz);
        if (dz < -1e-9) best = Math.min(best, (lo - z) / dz);
        return best;
    }

    static List<Case> makeCases(Gen g, long seed) {
        int n = g.size;
        Random r = new Random(seed * 31 + 7);
        int[] start = nearestStandable(g, 16 + r.nextInt(Math.max(1, n / 16)), 16 + r.nextInt(Math.max(1, n / 16)));
        List<Case> out = new ArrayList<>();
        double a1 = Math.toRadians(15 + r.nextInt(60)), a2 = Math.toRadians(15 + r.nextInt(60)), a3 = Math.toRadians(15 + r.nextInt(60));
        double near = Math.min(100 + r.nextInt(120), room(n, start[0], start[2], a1) * 0.9);
        out.add(makeCase(g, seed, "near", start, a1, near, false, false));
        out.add(makeCase(g, seed, "far", start, a2, room(n, start[0], start[2], a2) * (0.55 + 0.4 * r.nextDouble()), false, false));
        out.add(makeCase(g, seed, "xz", start, a3, room(n, start[0], start[2], a3) * (0.55 + 0.4 * r.nextDouble()), true, false));
        out.add(makeCase(g, seed, "deep", start, a2, room(n, start[0], start[2], a2) * (0.4 + 0.2 * r.nextDouble()), false, true));
        return out;
    }

    // ------------------------------------------------------------------------------------------------------------
    // World file
    // ------------------------------------------------------------------------------------------------------------

    /**
     * The world as upstream sees it: a box of n x n columns at the origin, H high, with a world border on the box. Every chunk
     * counts as loaded, also outside the box (air there), so a search never stops because it touched the box's edge: the border
     * rejects those moves, as it does at the edge of a real world. (The L2 worlds instead leave chunks unloaded.)
     */
    static final class BWorld extends baritone.utils.BlockStateInterface {
        final int n;
        final int[] cells;
        final WorldBorder wb;

        BWorld(WorldBorder wb, int n, int[] cells) {
            super(new baritone.utils.pathing.BetterWorldBorder(wb), 0, H);
            this.wb = wb;
            this.n = n;
            this.cells = cells;
        }

        @Override
        protected net.minecraft.world.level.block.state.BlockState getUncached(int x, int y, int z) {
            if (x < 0 || x >= n || z < 0 || z >= n) {
                return SearchGoldenDump.AIR_STATE;
            }
            return SearchBridge.state(cells[(x * n + z) * H + y]);
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

    static BWorld world(Gen g) {
        int n = g.size;
        WorldBorder wb = new WorldBorder();
        wb.setCenter(n / 2.0, n / 2.0);
        wb.setSize(n);
        return new BWorld(wb, n, g.vox.c);
    }

    static void writeWorldFile(Path file, Gen g, BWorld wd, List<Case> cases, long seed) throws Exception {
        try (BufferedWriter w = Files.newBufferedWriter(file)) {
            w.write("H search-bench 1\n");
            for (int i = 0; i < SearchBridge.paletteSize(); i++) {
                w.write(SearchBridge.stateLine(i) + "\n");
            }
            w.write(SearchBridge.configLine(CFG) + "\n");
            w.write(SearchGoldenDump.kline(KSET) + "\n");
            w.write("W seed=" + seed + " size=" + g.size + " hash=" + Long.toHexString(fnv(g.vox.c)) + "\n");
            for (Case c : cases) {
                w.write("B id=" + c.id + " name=" + c.name + " goal=" + c.spec + " start=" + c.start[0] + "," + c.start[1] + "," + c.start[2] + " stress=" + (c.stress ? 1 : 0) + "\n");
            }
            StringBuilder cs = new StringBuilder();
            for (int i = 0; i < (g.size >> 4) * (g.size >> 4); i++) {
                cs.append('2');
            }
            w.write("V 0 kind=bench ox=0 oz=0 miny=0 w=" + g.size + " h=" + H + " d=" + g.size + " border=" + wd.wb.getMinX() + "," + wd.wb.getMaxX() + "," + wd.wb.getMinZ() + "," + wd.wb.getMaxZ()
                    + " chunks=" + cs + " cells=" + SearchGoldenDump.rle(g.vox.c) + "\n");
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // Timing one search
    // ------------------------------------------------------------------------------------------------------------

    static final class Run {
        long ns;
        String type;
        int nodes, moves, map, plen;
        String phash;
        int[] best;
        double bestH;
        long gcMs;
    }

    static List<GarbageCollectorMXBean> GCS = ManagementFactory.getGarbageCollectorMXBeans();

    static long gcMs() {
        long t = 0;
        for (GarbageCollectorMXBean b : GCS) {
            t += Math.max(0, b.getCollectionTime());
        }
        return t;
    }

    static Run runOnce(BWorld wd, Case c, long primary, long failure) throws Exception {
        SearchGoldenDump.applyK(SearchGoldenDump.KS.get(KSET));
        BetterBlockPos feet = new BetterBlockPos(c.start[0], c.start[1], c.start[2]);
        IBaritone bar = SearchGoldenDump.baritoneProxy(feet, new ArrayList<>());
        CalculationContext ctx = SearchBridge.context(bar, wd, CFG);
        PathingBehavior pb = SearchGoldenDump.behaviorWith(bar.getPlayerContext());
        if (SearchGoldenDump.createPathfinder == null) {
            SearchGoldenDump.createPathfinder = PathingBehavior.class.getDeclaredMethod("createPathfinder", BlockPos.class, Goal.class, IPath.class, CalculationContext.class);
            SearchGoldenDump.createPathfinder.setAccessible(true);
        }
        BlockPos startPos = new BlockPos(c.start[0], c.start[1], c.start[2]);
        long gc0 = gcMs();
        long t0 = System.nanoTime();
        AStarPathFinder finder = (AStarPathFinder) SearchGoldenDump.createPathfinder.invoke(pb, startPos, c.goal, null, ctx);
        PathCalculationResult res = finder.calculate(primary, failure);
        long t1 = System.nanoTime();
        Run r = new Run();
        r.ns = t1 - t0;
        r.gcMs = gcMs() - gc0;
        r.type = res.getType().name();
        r.nodes = finder.numNodesConsidered;
        r.moves = finder.numMovementsConsidered;
        r.map = finder.mapSize();
        IPath p = res.getPath().orElse(null);
        r.plen = p == null ? 0 : p.positions().size();
        r.phash = Long.toHexString(fnvText(SearchGoldenDump.pathText(p, true)));
        IPath b = finder.bestPathSoFar().orElse(null);
        if (b != null) {
            BetterBlockPos d = b.getDest();
            r.best = new int[]{d.x, d.y, d.z};
            r.bestH = c.goal.heuristic(d.x, d.y, d.z);
        } else {
            r.best = new int[]{c.start[0], c.start[1], c.start[2]};
            r.bestH = c.goal.heuristic(c.start[0], c.start[1], c.start[2]);
        }
        return r;
    }

    static String line(Case c, String mode, long primary, long failure, List<Run> runs) {
        StringBuilder ns = new StringBuilder(), gc = new StringBuilder();
        for (Run r : runs) {
            ns.append(ns.length() == 0 ? "" : ",").append(r.ns);
            gc.append(gc.length() == 0 ? "" : ",").append(r.gcMs);
        }
        Run r = runs.get(0);
        for (Run o : runs) {
            if (o.nodes != r.nodes || !o.phash.equals(r.phash) || !o.type.equals(r.type)) {
                return "ERR id=" + c.id + " the repeated runs of one search disagree\n";
            }
        }
        return "T id=" + c.id + " mode=" + mode + " primary=" + primary + " failure=" + failure + " reps=" + runs.size() + " ns=" + ns + " gcms=" + gc + " nodes=" + r.nodes + " moves=" + r.moves
                + " map=" + r.map + " type=" + r.type + " plen=" + r.plen + " phash=" + r.phash + " best=" + r.best[0] + "," + r.best[1] + "," + r.best[2] + " besth=" + r.bestH + "\n";
    }

    static void warmUp(PrintStream log) throws Exception {
        // searches of other worlds until about three million nodes were taken out of the open set, so that the JIT has seen the hot code
        // before anything is timed
        long t = System.nanoTime();
        long nodes = 0;
        int count = 0;
        outer:
        for (int round = 0; round < 6; round++) {
            for (long seed : new long[]{9003, 9007}) {
                Gen g = generate(seed);
                BWorld wd = world(g);
                for (Case c : makeCases(g, seed)) {
                    if (c.stress) {
                        continue;
                    }
                    Run r = runOnce(wd, c, 1500, 1500);
                    nodes += r.nodes;
                    count++;
                    if (nodes >= 3_000_000L || System.nanoTime() - t > 40_000_000_000L) {
                        break outer;
                    }
                }
            }
        }
        log.println("warm-up: " + count + " searches, " + nodes + " nodes, " + (System.nanoTime() - t) / 1_000_000 + " ms");
    }

    static String arg(String[] a, String key, String dflt) {
        for (String s : a) {
            if (s.startsWith(key + "=")) {
                return s.substring(key.length() + 1);
            }
        }
        if (dflt == null) {
            throw new IllegalArgumentException("missing " + key + "=");
        }
        return dflt;
    }

    /**
     * mode=finish out=DIR seeds=1,2,3 write=1|0 finishms=12000 : times every non-stress case to its end (primary = failure = finishms), writes DIR/java-finish<tag>.txt
     * mode=limit  out=DIR list=FILE : FILE has lines "id primary failure reps"; times those searches under those limits, writes DIR/java-limit.txt
     */
    public static void main(String[] a) throws Exception {
        PrintStream real = System.out;
        PrintStream nul = new PrintStream(OutputStream.nullOutputStream());
        System.setOut(nul);
        System.setErr(nul);
        String mode = arg(a, "mode", null);
        Path dir = Path.of(arg(a, "out", null));
        Files.createDirectories(dir);
        warmUp(real);
        if (mode.equals("finish")) {
            long limit = Long.parseLong(arg(a, "finishms", "12000"));
            boolean write = arg(a, "write", "0").equals("1");
            int maxReps = Integer.parseInt(arg(a, "reps", "5"));
            try (BufferedWriter out = Files.newBufferedWriter(dir.resolve("java-finish" + arg(a, "tag", "") + ".txt"))) {
                for (String sd : arg(a, "seeds", null).split(",")) {
                    long seed = Long.parseLong(sd);
                    Gen g = generate(seed);
                    BWorld wd = world(g);
                    List<Case> cases = makeCases(g, seed);
                    if (write) {
                        writeWorldFile(dir.resolve("world-" + seed + ".txt"), g, wd, cases, seed);
                    }
                    out.write("W seed=" + seed + " size=" + g.size + " hash=" + Long.toHexString(fnv(g.vox.c)) + "\n");
                    for (Case c : cases) {
                        if (c.stress) {
                            continue;
                        }
                        Run first = runOnce(wd, c, limit, limit);
                        List<Run> runs = new ArrayList<>();
                        if (!first.type.equals("SUCCESS_TO_GOAL")) {
                            // did not finish inside the limit: one run, no repetitions
                            runs.add(first);
                        } else {
                            int reps = first.ns < 1_000_000_000L ? maxReps : (first.ns < 6_000_000_000L ? Math.min(maxReps, 3) : Math.min(maxReps, 2));
                            for (int i = 0; i < reps; i++) {
                                runs.add(runOnce(wd, c, limit, limit));
                            }
                        }
                        out.write(line(c, "finish", limit, limit, runs));
                        out.flush();
                        real.println(c.id + " " + c.spec + " " + runs.get(0).type + " nodes=" + runs.get(0).nodes + " median_ms=" + median(runs) / 1e6);
                    }
                }
            }
        } else if (mode.equals("limit")) {
            try (BufferedWriter out = Files.newBufferedWriter(dir.resolve("java-limit.txt"))) {
                Long cur = null;
                Gen g = null;
                BWorld wd = null;
                List<Case> cases = null;
                for (String ln : Files.readAllLines(Path.of(arg(a, "list", null)))) {
                    if (ln.isBlank()) {
                        continue;
                    }
                    String[] f = ln.trim().split(" ");
                    long seed = Long.parseLong(f[0].substring(0, f[0].indexOf('.')));
                    if (cur == null || cur != seed) {
                        g = generate(seed);
                        wd = world(g);
                        cases = makeCases(g, seed);
                        cur = seed;
                        out.write("W seed=" + seed + " size=" + g.size + " hash=" + Long.toHexString(fnv(g.vox.c)) + "\n");
                    }
                    Case c = null;
                    for (Case k : cases) {
                        if (k.id.equals(f[0])) {
                            c = k;
                        }
                    }
                    long p = Long.parseLong(f[1]), fl = Long.parseLong(f[2]);
                    int reps = Integer.parseInt(f[3]);
                    runOnce(wd, c, p, fl);
                    List<Run> runs = new ArrayList<>();
                    for (int i = 0; i < reps; i++) {
                        runs.add(runOnce(wd, c, p, fl));
                    }
                    // under a clock limit the node count and the path differ between repetitions; write each repetition on its own line
                    for (Run r : runs) {
                        List<Run> one = new ArrayList<>();
                        one.add(r);
                        out.write(line(c, "limit", p, fl, one));
                    }
                    out.flush();
                    real.println(c.id + " limit " + p + "/" + fl + " nodes=" + runs.get(0).nodes);
                }
            }
        } else {
            throw new IllegalArgumentException("mode " + mode);
        }
    }

    static long median(List<Run> runs) {
        long[] v = runs.stream().mapToLong(r -> r.ns).sorted().toArray();
        return v[v.length / 2];
    }
}
