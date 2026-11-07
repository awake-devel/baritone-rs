# Benchmark: search speed against upstream Baritone

How fast is the port's search (`mod/engine`) next to upstream Baritone's real search, on the same world, start, goal and
settings? The question comes from the owner: compare by seed (a comparison across different seeds means nothing; several seeds give
more comparisons), and show how much further Rust gets in the same time limit. Every number below is a pair measured on one world.

The tool is [`tools/run_bench.py`](../tools/README.md#bench). The run reported here took 33 minutes on 2026-11-07 and wrote
`reports/bench-1.21.4.json` (metadata and counts only). The worlds, the raw timings and the tables stayed out of the repository.

## Summary

- **20 seeds, 60 searches** (three per seed: a `GoalBlock` 100 to 220 blocks away, a far `GoalBlock`, a far `GoalXZ`). 51 of them finished on
  both sides inside the 10 s limit. For all 51 the two sides made the same search (nodes taken out of the open set, movements considered, nodes made) and
  returned the same path, movement by movement and cost bit by bit. The largest has 1.94 million nodes. (L2 compared searches up to 20,000 nodes; this is a
  wider check of the same thing, not a proof.)
- **Time to finish.** Java time divided by Rust time, above 1 meaning Rust was faster: **median 1.44** over the 51 searches (quartiles 1.34 to 1.61,
  range 0.66 to 1.88). Per seed, adding up that seed's finished searches: **median 1.57, range 1.05 to 1.84** over 20 seeds. In nodes per second:
  Java 261 thousand (median; range 168 to 389 thousand), Rust 377 thousand (range 186 to 484 thousand).
- **Rust was slower in 3 of the 51 searches** (seed 19 near 0.66, seed 19 far 0.92, seed 20 near 0.99). All three come from the first round of the Rust run on
  seeds 19 and 20, when the pinned CPUs' clock dipped to 2.5 GHz and the repetitions of those searches were scattered (for seed 19 near, 18 to 59 ms). In the second round Rust is
  faster in all 51 (range 1.01 to 2.05). A later rerun of seeds 19 and 20 gave 1.2 to 1.6 on the five searches that finished on both sides. See "Reading the numbers".
- **Nine searches did not finish in 10 s** on one or both sides. Where Rust finished and Java did not (3 searches), Java's time is at least 1.05, 1.20 and 1.44 times Rust's.
- **The same time limit.** Given the same clock limit, Rust takes **1.17 times** (50 ms), **1.47 times** (500 ms), **1.45 times** (2000 ms) as many nodes out of the open set as Java
  (medians over 14, 14 and 10 searches; ranges 1.06 to 1.41, 1.26 to 1.63, 1.36 to 1.74). Its best partial path ends nearer the goal in 1 of 14 searches at 50 ms,
  8 of 14 at 500 ms, 6 of 10 at 2000 ms, and never farther. The gain is often small and sometimes large: at 500 ms Rust's path ended a median of
  21 blocks (horizontally) nearer the goal, at most 118 blocks.

None of this says what a player would see. It times the search alone, on this machine and this set of worlds (see "Limits").

## Method

### What is timed

Upstream's own classes: `PathingBehavior.createPathfinder` and `AbstractNodeCostSearch.calculate(primary, failure)` of the pinned checkout (`upstream/baritone-1.21.4`, commit
`1b8e87c`), through the same harness as L2 (`SearchGoldenDump`'s palette, configuration and context helpers). The Rust side calls `create_pathfinder` and `calculate` of
`mod/engine`. The timed span is the same on both sides: from the creation of the finder to the returned result, which includes
`postProcess`, the two cutoffs and the result type. It leaves out the world, the settings and the context (block tables, tool strengths), and the JVM start.

### Worlds, one generator

`SearchBench.java` is the only world generator. It writes each world to a file in the format of the L2 golden file; the Rust side reads that file
(it checks a hash of all cells against the hash the Java side computed from its own array before it times anything). A world is `n` x `n` columns
(`n` from 256 to 1024 by seed) and 80 high:

- height from noise at four scales, in four grades of relief by seed (amplitude 5, 8, 12 and 16 blocks), with mountain ridges in the roughest grade;
- sea level at y = 40 with beaches, lakes, narrow rivers, ravines (the two rougher grades), caves with lava below y = 10, trees, gravel and stone patches,
  short grass, and, in every third seed, three cobblestone walls six high with gaps every 40 to 90 blocks.

Every chunk counts as loaded, also outside the box, and the world border is the box. A search therefore ends only by reaching the goal, by running out of
open nodes, or by the clock. (L2's worlds instead leave chunks unloaded, which stops a search after 50 contacts with the edge.)

Seeds are 1 to 20. Seed `s` has size `[256, 512, 1024, 384, 768, 640, 896, 320][s mod 8]`. The start is a standable column near the world's corner. The goals are
standable columns found by a spiral search from a point in a random direction:

| Case | Goal | Distance |
|---|---|---|
| `near` | `GoalBlock` | 100 to 220 blocks |
| `far` | `GoalBlock` | 55 to 95 % of the room the world leaves in that direction (up to 1200 blocks) |
| `xz` | `GoalXZ` | the same, another direction |

(The generator also writes a `deep` case, a block 24 under the surface, as a spare for the same-limit runs. The final run did not need it.)

### Settings

Upstream's defaults, from the L2 tools: configuration 0 of `cost-compare` (allowBreak on, allowPlace on with a throwaway block, sprint on, parkour off, diagonal
descend and ascend off, max fall 3, break penalty 2, place penalty 20, jump penalty 2, cost heuristic 3.563, a pickaxe in the hand) and search settings 0
(favoring coefficient 0.5, `pathingMaxChunkBorderFetch` 50, minimum improvement repropagation on, cutoff length 30 and factor 0.9, avoidance off).
No previous path, no mobs, no spawners, no potion effects.

### Measurements

**A. Time to finish.** primary = failure = 10,000 ms, so the clock only matters for searches that cannot finish. For each case: warm-up, one run that is thrown away, then
5 runs (3 if the discarded run took over 1 s, 2 if over 6 s). A search that does not finish in the limit is run once. The whole phase is run twice (two rounds), Java then Rust
each time; both rounds' repetitions go into one list per case, and each round's own ratio is also reported. Java's warm-up is about three million nodes on other worlds
(seeds 9003 and 9007) in the same JVM before anything is timed, then the discarded run of each case. The Rust side has no JIT; it runs two searches first.

**B. Same time limit.** Searches that neither side can finish inside the limit: the faster side's median time to finish is over 1.25 times the limit, or the search did
not finish in A. Chosen spread over the seeds (14 at 50 ms, 14 at 500 ms, 10 at 2000 ms, 8 at upstream's own pair of primary 500 ms and failure 2000 ms), one discarded run
and 5 runs (3 at 2000 ms and for the default pair). Reported: nodes taken out of the open set (median over the runs), and the progress of the best partial
path (`bestPathSoFar()`): how far its end is from the goal horizontally, how many blocks closer than the start that is, and the goal's heuristic there (smaller is closer).
Under a clock limit the stopping point depends on the clock, so paths are not compared here. (With the default pair every search ran 500 ms: once a path of 5 blocks exists,
the primary limit applies and the failure limit never does.)

### Fairness, and what differs besides the search

- Same machine, one side at a time, never at the same time. Both pinned with `taskset` to the same two logical CPUs (2 and 3, two different physical cores), one search thread each. The script
  waited until no game, build or Gradle run was going and the CPUs were idle; a monitor noted any such process during each timed phase (none appeared). Run order: Java, Rust, Java, Rust, then the
  same-limit runs Java then Rust.
- JVM: OpenJDK 21.0.13, `-Xms6g -Xmx6g -XX:+UseSerialGC`, default JIT. Rust: rustc 1.99.1, `cargo build --release` of a crate in `tools/bench`, which has the default release profile (opt-level 3, no LTO, no
  `target-cpu`).
- World access differs, and cannot be made the same. Java reads `BlockState` objects through upstream's `BlockStateInterface` (which, during `calculate`, keeps a 64k-entry position cache) from an `int[]` and a palette list;
  Rust reads a `u16[]` state id and a table of precomputed facts. Both are flat arrays, with no chunk lookup. A search in the game goes through chunks on both sides, which this does not model.
- Memory: Java allocates a `PathNode` object per node in a hash of cell arrays, and a young-generation collector runs. The measured GC time per search was 0 ms in most cases and at most 55 ms. Rust grows
  vectors. Memory use was not measured.
- Java prints a few lines when a search ends early (silenced here); Rust does not. Java's `calculate` is `synchronized` and claims the search caches (two 64k arrays) inside the timed span. Rust has no such caches.
- Clocks: Java `System.nanoTime` for the span and `currentTimeMillis` every 64 nodes inside the search; Rust `Instant` for both.

### The machine

Intel Core i7-8750H (6 cores, 12 threads, 2.2 GHz base, up to 4.1 GHz), governor `performance`, intel_pstate with turbo on, Linux 7.3.1-cachyos. It is a laptop and it runs hot: the package
temperature was between 68 and 100 degrees C during every phase, and the pinned CPUs' clock was between 2.5 and 4.1 GHz (median 3.5 to 3.9). Per-phase numbers are in the monitor part of the raw results (not in the repository).

## Results

### Time to finish


| Seed | Case | Goal | Dist | Java ms (min-max) | Rust ms (min-max) | Java/Rust | By round | Nodes | Nodes/s Java | Nodes/s Rust | Same path |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | far | `block(232,44,322)` | 336 | 542.8 (493.1-618.4) | 361.8 (334.1-440.0) | 1.50 | 1.42 / 1.60 | 141061 | 260k | 390k | yes |
| 1 | near | `block(121,50,141)` | 125 | 183.9 (164.0-199.4) | 128.7 (117.9-149.6) | 1.43 | 1.33 / 1.48 | 56284 | 306k | 437k | yes |
| 1 | xz | `xz(259,329)` | 358 | 1095.1 (946.8-1208.7) | 630.8 (594.1-746.8) | 1.74 | 1.30 / 1.85 | 245602 | 224k | 389k | yes |
| 2 | near | `block(118,45,139)` | 113 | 6.4 (5.9-7.0) | 4.6 (4.1-5.5) | 1.37 | 1.22 / 1.67 | 1959 | 308k | 423k | yes |
| 3 | far | `block(186,48,323)` | 333 | 2650.3 (2437.4-2895.1) | 1532.8 (1446.3-1628.2) | 1.73 | 1.55 / 1.95 | 528248 | 199k | 345k | yes |
| 3 | near | `block(144,49,151)` | 166 | 72.4 (61.5-77.4) | 48.7 (43.8-53.5) | 1.48 | 1.25 / 1.65 | 21287 | 294k | 437k | yes |
| 3 | xz | `xz(301,301)` | 383 | 7859.4 (7453.0-8422.2) | 4174.4 (4007.9-4379.2) | 1.88 | 1.74 / 2.05 | 1322017 | 168k | 317k | yes |
| 4 | far | `block(318,44,614)` | 648 | 1321.5 (1259.2-1391.9) | 925.7 (885.5-1018.7) | 1.43 | 1.31 / 1.51 | 371742 | 281k | 402k | yes |
| 4 | near | `block(93,47,127)` | 112 | 63.4 (62.5-67.1) | 53.8 (48.5-57.7) | 1.18 | 1.11 / 1.29 | 24461 | 386k | 455k | yes |
| 4 | xz | `xz(673,203)` | 661 | 9277.0 (9173.2-9502.0) | 5350.1 (5317.0-5829.0) | 1.73 | 1.69 / 1.72 | 1942529 | 209k | 363k | yes |
| 5 | far | `block(570,47,366)` | 634 | 1477.1 (1414.8-1552.2) | 1053.8 (981.3-1155.4) | 1.40 | 1.30 / 1.50 | 407696 | 276k | 387k | yes |
| 5 | near | `block(123,48,164)` | 162 | 14.4 (13.0-17.2) | 10.4 (9.4-11.6) | 1.38 | 1.41 / 1.43 | 4148 | 288k | 398k | yes |
| 5 | xz | `xz(435,177)` | 429 | 5678.6 (5562.2-6259.4) | 3483.3 (3295.8-3693.7) | 1.63 | 1.55 / 1.72 | 1221643 | 215k | 351k | yes |
| 6 | far | `block(258,51,598)` | 577 | 1012.0 (982.9-1047.1) | 706.3 (637.8-775.7) | 1.43 | 1.35 / 1.60 | 264937 | 262k | 375k | yes |
| 6 | near | `block(149,48,184)` | 153 | 69.6 (65.5-83.2) | 55.4 (50.1-86.3) | 1.26 | 1.10 / 1.38 | 24806 | 356k | 448k | yes |
| 6 | xz | `xz(280,601)` | 587 | 1596.0 (1549.0-1654.8) | 1022.1 (949.1-1062.0) | 1.56 | 1.47 / 1.68 | 372714 | 234k | 365k | yes |
| 7 | far | `block(203,46,238)` | 277 | 1151.6 (1126.4-1242.5) | 734.6 (655.3-738.7) | 1.57 | 1.53 / 1.75 | 263567 | 229k | 359k | yes |
| 7 | near | `block(132,52,200)` | 203 | 534.7 (512.3-580.8) | 360.4 (322.8-427.6) | 1.48 | 1.42 / 1.65 | 143677 | 269k | 399k | yes |
| 7 | xz | `xz(122,264)` | 254 | 1059.2 (1038.2-1126.4) | 660.7 (619.3-730.3) | 1.60 | 1.49 / 1.73 | 249698 | 236k | 378k | yes |
| 8 | far | `block(163,46,199)` | 218 | 261.6 (245.8-281.0) | 187.3 (172.2-216.3) | 1.40 | 1.25 / 1.59 | 80205 | 307k | 428k | yes |
| 8 | near | `block(145,45,101)` | 139 | 190.8 (183.3-200.8) | 154.1 (137.1-174.2) | 1.24 | 1.18 / 1.38 | 64211 | 337k | 417k | yes |
| 8 | xz | `xz(223,162)` | 238 | 25.8 (21.7-29.2) | 20.0 (17.5-23.6) | 1.29 | 1.21 / 1.35 | 7566 | 293k | 378k | yes |
| 9 | far | `block(247,44,390)` | 418 | 3884.8 (3843.7-4130.3) | 2446.9 (2314.4-2605.7) | 1.59 | 1.51 / 1.66 | 868424 | 224k | 355k | yes |
| 9 | near | `block(134,46,128)` | 138 | 7.8 (7.6-8.8) | 7.0 (6.2-8.0) | 1.11 | 1.02 / 1.23 | 2622 | 336k | 373k | yes |
| 9 | xz | `xz(301,400)` | 455 | 5811.8 (5674.1-6572.0) | 3305.6 (3287.8-3333.3) | 1.76 | 1.77 / 1.74 | 1197287 | 206k | 362k | yes |
| 10 | near | `block(142,45,157)` | 121 | 57.8 (50.8-63.7) | 40.1 (39.1-41.0) | 1.44 | 1.31 / 1.57 | 19380 | 335k | 484k | yes |
| 11 | far | `block(90,41,256)` | 244 | 324.7 (319.5-359.8) | 249.7 (226.0-294.6) | 1.30 | 1.18 / 1.42 | 99342 | 306k | 398k | yes |
| 11 | near | `block(155,50,137)` | 179 | 2270.3 (2222.6-2366.2) | 1393.8 (1257.8-1547.3) | 1.63 | 1.49 / 1.76 | 466347 | 205k | 335k | yes |
| 11 | xz | `xz(186,220)` | 259 | 3691.9 (3554.0-3754.4) | 2234.9 (1953.6-2310.6) | 1.65 | 1.62 / 1.69 | 700330 | 190k | 313k | yes |
| 12 | far | `block(530,47,316)` | 557 | 718.8 (647.6-848.9) | 535.2 (527.7-569.9) | 1.34 | 1.19 / 1.53 | 206022 | 287k | 385k | yes |
| 12 | near | `block(189,46,150)` | 178 | 575.3 (501.6-650.4) | 420.0 (406.4-469.8) | 1.37 | 1.23 / 1.45 | 159504 | 277k | 380k | yes |
| 12 | xz | `xz(317,452)` | 481 | 737.7 (664.9-808.2) | 551.9 (539.9-585.0) | 1.34 | 1.22 / 1.43 | 208139 | 282k | 377k | yes |
| 13 | near | `block(153,48,152)` | 175 | 626.9 (564.8-709.3) | 400.8 (389.9-443.0) | 1.56 | 1.40 / 1.62 | 147481 | 235k | 368k | yes |
| 13 | xz | `xz(534,242)` | 548 | 3681.9 (3461.3-4071.7) | 2282.2 (2220.3-2399.5) | 1.61 | 1.51 / 1.79 | 746085 | 203k | 327k | yes |
| 14 | far | `block(743,42,257)` | 744 | 8729.1 (7963.5-9033.5) | 4917.8 (4682.5-5137.9) | 1.78 | 1.77 / 1.74 | 1646880 | 189k | 335k | yes |
| 14 | near | `block(139,42,146)` | 158 | 42.2 (37.7-46.4) | 35.2 (34.1-36.6) | 1.20 | 1.07 / 1.31 | 13421 | 318k | 381k | yes |
| 15 | far | `block(226,41,274)` | 309 | 2731.0 (2602.1-2944.5) | 1471.5 (1380.0-1594.5) | 1.86 | 2.01 / 1.75 | 490099 | 179k | 333k | yes |
| 15 | near | `block(185,50,186)` | 216 | 622.9 (578.0-727.0) | 351.6 (322.0-400.3) | 1.77 | 1.91 / 1.63 | 131921 | 212k | 375k | yes |
| 15 | xz | `xz(101,200)` | 181 | 641.5 (586.8-757.0) | 409.7 (372.7-472.3) | 1.57 | 1.83 / 1.43 | 150990 | 235k | 369k | yes |
| 16 | far | `block(68,48,153)` | 132 | 55.1 (47.9-72.5) | 43.1 (36.9-50.8) | 1.28 | 1.84 / 1.01 | 17757 | 322k | 412k | yes |
| 16 | near | `block(214,46,132)` | 214 | 419.4 (352.4-455.1) | 254.8 (236.2-316.1) | 1.65 | 1.82 / 1.29 | 102544 | 244k | 402k | yes |
| 16 | xz | `xz(160,79)` | 142 | 57.7 (50.5-74.8) | 40.7 (36.2-46.2) | 1.42 | 1.72 / 1.21 | 16308 | 283k | 400k | yes |
| 17 | far | `block(314,45,195)` | 317 | 1549.6 (1507.6-1791.4) | 1048.2 (960.6-1205.7) | 1.48 | 1.80 / 1.39 | 369646 | 239k | 353k | yes |
| 17 | near | `block(225,45,121)` | 204 | 117.6 (103.0-231.5) | 87.8 (80.4-99.9) | 1.34 | 1.57 / 1.11 | 36930 | 314k | 421k | yes |
| 17 | xz | `xz(228,469)` | 469 | 4062.4 (3747.6-5182.0) | 2751.5 (2571.7-2886.6) | 1.48 | 1.59 / 1.40 | 813804 | 200k | 296k | yes |
| 18 | near | `block(230,48,111)` | 189 | 374.5 (336.9-479.3) | 270.7 (225.7-333.2) | 1.38 | 1.39 / 1.46 | 97809 | 261k | 361k | yes |
| 19 | far | `block(320,47,238)` | 368 | 3888.7 (3825.1-4092.4) | 4233.8 (2209.7-7800.8) | 0.92 | 0.59 / 1.72 | 787061 | 202k | 186k | yes |
| 19 | near | `block(142,41,65)` | 126 | 21.3 (19.8-23.5) | 32.2 (18.3-59.2) | 0.66 | 0.46 / 1.10 | 7366 | 345k | 229k | yes |
| 19 | xz | `xz(223,268)` | 320 | 2845.9 (2770.3-2985.0) | 2151.3 (1645.6-3128.0) | 1.32 | 1.03 / 1.72 | 591874 | 208k | 275k | yes |
| 20 | near | `block(99,46,138)` | 123 | 44.3 (42.5-49.5) | 44.8 (34.8-78.1) | 0.99 | 0.77 / 1.22 | 17229 | 389k | 385k | yes |
| 20 | xz | `xz(300,541)` | 573 | 5101.0 (4803.7-5528.1) | 3275.4 (3226.9-3381.3) | 1.56 | 1.62 / 1.48 | 1076275 | 211k | 329k | yes |

Not finished on both sides inside the limit (the same search, stopped by the clock; one run per side). A search that did not finish has run for the whole limit; where only Rust finished, Java/Rust is at least the limit over Rust's time:

| Seed | Case | Goal | Dist | Java | Rust | Java/Rust |
|---|---|---|---|---|---|---|
| 2 | far | `block(784,43,547)` | 882 | not finished, 10.0 s, 2049600 nodes | finished, 9.5 s, 3048650 nodes | at least 1.05 |
| 2 | xz | `xz(574,630)` | 780 | not finished, 10.0 s, 1901824 nodes | finished, 6.9 s, 2226074 nodes | at least 1.44 |
| 10 | far | `block(953,41,865)` | 1197 | not finished, 10.0 s, 1796544 nodes | not finished, 10.0 s, 3181440 nodes | - |
| 10 | xz | `xz(797,797)` | 1037 | not finished, 10.0 s, 1785344 nodes | not finished, 10.0 s, 2935360 nodes | - |
| 13 | far | `block(410,49,438)` | 560 | not finished, 10.0 s, 1884032 nodes | not finished, 10.0 s, 2988928 nodes | - |
| 14 | xz | `xz(265,735)` | 744 | not finished, 10.0 s, 1702208 nodes | not finished, 10.0 s, 3315584 nodes | - |
| 18 | far | `block(342,45,768)` | 790 | not finished, 10.0 s, 1985728 nodes | not finished, 10.0 s, 2640768 nodes | - |
| 18 | xz | `xz(690,930)` | 1099 | not finished, 10.0 s, 1919360 nodes | not finished, 10.0 s, 1707840 nodes | - |
| 20 | far | `block(525,46,682)` | 811 | not finished, 10.0 s, 2096448 nodes | finished, 8.4 s, 2344407 nodes | at least 1.20 |

#### Summary (Java time / Rust time, above 1 means Rust was faster)

| Group | Cases | Median | Quartiles | Min | Max | Rust slower |
|---|---|---|---|---|---|---|
| all | 51 | 1.44 | 1.34-1.61 | 0.66 | 1.88 | 3 |
| near | 20 | 1.38 | 1.23-1.48 | 0.66 | 1.77 | 2 |
| far | 15 | 1.43 | 1.37-1.58 | 0.92 | 1.86 | 1 |
| xz | 16 | 1.58 | 1.46-1.67 | 1.29 | 1.88 | 0 |
| round 1 only | 51 | 1.41 | 1.22-1.60 | 0.46 | 2.01 | 3 |
| round 2 only | 51 | 1.57 | 1.39-1.72 | 1.01 | 2.05 | 0 |
| per seed (finished cases of a seed added up) | 20 | 1.57 | 1.43-1.64 | 1.05 | 1.84 | 0 |

Nodes per second, median over the finished cases: Java 261k (range 168k-389k), Rust 377k (range 186k-484k).



### Same time limit



#### Limit 50 ms (primary = failure)

| Seed | Case | Dist | Java nodes | Rust nodes | Rust/Java nodes | Java closer by | Rust closer by | Java left | Rust left | Java h | Rust h |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | far | 336 | 14912 | 21056 | 1.41 | 142 | 142 | 194 | 194 | 706 | 706 |
| 2 | far | 882 | 17216 | 19584 | 1.14 | 193 | 214 | 689 | 667 | 2478 | 2418 |
| 3 | xz | 383 | 16576 | 17856 | 1.08 | 117 | 117 | 266 | 266 | 947 | 947 |
| 5 | xz | 429 | 17472 | 19008 | 1.09 | 286 | 286 | 143 | 143 | 547 | 547 |
| 7 | far | 277 | 17344 | 20032 | 1.15 | 79 | 79 | 198 | 198 | 709 | 709 |
| 8 | far | 218 | 16192 | 20480 | 1.26 | 196 | 196 | 22 | 22 | 78 | 78 |
| 10 | far | 1197 | 16896 | 19840 | 1.17 | 153 | 153 | 1044 | 1044 | 3722 | 3722 |
| 11 | near | 179 | 17856 | 20032 | 1.12 | 137 | 137 | 42 | 42 | 159 | 159 |
| 12 | near | 178 | 19840 | 21120 | 1.06 | 168 | 168 | 10 | 10 | 36 | 36 |
| 13 | xz | 548 | 15168 | 19136 | 1.26 | 38 | 38 | 511 | 511 | 1970 | 1970 |
| 15 | far | 309 | 14592 | 19264 | 1.32 | 69 | 69 | 240 | 240 | 866 | 866 |
| 16 | near | 214 | 15104 | 19136 | 1.27 | 162 | 162 | 52 | 52 | 189 | 189 |
| 18 | far | 790 | 16192 | 18816 | 1.16 | 156 | 156 | 635 | 635 | 2446 | 2446 |
| 19 | far | 368 | 17280 | 20480 | 1.19 | 161 | 161 | 208 | 208 | 741 | 741 |

Rust/Java nodes: median 1.17, range 1.06-1.41 over 14 searches. Rust/Java blocks closer to the goal: median 1.00, range 1.00-1.11. Rust's best partial path ended nearer the goal in 1, farther in 0, equal (within half a block) in 13.

#### Limit 500 ms (primary = failure)

| Seed | Case | Dist | Java nodes | Rust nodes | Rust/Java nodes | Java closer by | Rust closer by | Java left | Rust left | Java h | Rust h |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | xz | 358 | 138048 | 181760 | 1.32 | 274 | 298 | 83 | 59 | 297 | 212 |
| 2 | xz | 780 | 142976 | 180032 | 1.26 | 234 | 234 | 547 | 547 | 1949 | 1949 |
| 3 | xz | 383 | 115712 | 172160 | 1.49 | 210 | 264 | 173 | 119 | 617 | 425 |
| 4 | xz | 661 | 139008 | 190976 | 1.37 | 237 | 237 | 425 | 425 | 1628 | 1628 |
| 6 | far | 577 | 128832 | 187840 | 1.46 | 300 | 357 | 277 | 220 | 1055 | 826 |
| 7 | far | 277 | 112128 | 181056 | 1.61 | 150 | 150 | 127 | 127 | 456 | 456 |
| 9 | far | 418 | 137408 | 189952 | 1.38 | 244 | 262 | 174 | 156 | 679 | 584 |
| 10 | xz | 1037 | 116160 | 181056 | 1.56 | 257 | 285 | 780 | 752 | 2788 | 2688 |
| 11 | xz | 259 | 118720 | 184960 | 1.56 | 159 | 159 | 100 | 100 | 358 | 358 |
| 13 | xz | 548 | 110016 | 179712 | 1.63 | 178 | 245 | 370 | 303 | 1429 | 1154 |
| 14 | xz | 744 | 114944 | 180160 | 1.57 | 225 | 272 | 519 | 472 | 1987 | 1810 |
| 17 | xz | 469 | 122176 | 181568 | 1.49 | 148 | 148 | 320 | 320 | 1235 | 1235 |
| 18 | xz | 1099 | 133504 | 181952 | 1.36 | 165 | 283 | 934 | 816 | 3525 | 3026 |
| 19 | xz | 320 | 134720 | 177920 | 1.32 | 273 | 273 | 47 | 47 | 166 | 166 |

Rust/Java nodes: median 1.47, range 1.26-1.63 over 14 searches. Rust/Java blocks closer to the goal: median 1.08, range 1.00-1.71. Rust's best partial path ended nearer the goal in 8, farther in 0, equal (within half a block) in 6.

#### Limit 2000 ms (primary = failure)

| Seed | Case | Dist | Java nodes | Rust nodes | Rust/Java nodes | Java closer by | Rust closer by | Java left | Rust left | Java h | Rust h |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 2 | far | 882 | 465088 | 666496 | 1.43 | 545 | 578 | 337 | 303 | 1208 | 1088 |
| 2 | xz | 780 | 445120 | 651392 | 1.46 | 420 | 420 | 361 | 361 | 1295 | 1295 |
| 4 | xz | 661 | 510144 | 694336 | 1.36 | 524 | 524 | 137 | 137 | 504 | 504 |
| 9 | xz | 455 | 480000 | 670016 | 1.40 | 347 | 358 | 108 | 98 | 389 | 350 |
| 10 | far | 1197 | 439424 | 628608 | 1.43 | 427 | 427 | 769 | 769 | 2744 | 2744 |
| 13 | far | 560 | 460800 | 642688 | 1.39 | 306 | 366 | 254 | 194 | 933 | 739 |
| 14 | xz | 744 | 447168 | 660160 | 1.48 | 408 | 458 | 337 | 287 | 1298 | 1053 |
| 17 | xz | 469 | 439680 | 649024 | 1.48 | 310 | 310 | 158 | 158 | 575 | 575 |
| 18 | xz | 1099 | 413376 | 672192 | 1.63 | 357 | 365 | 742 | 734 | 2644 | 2615 |
| 20 | far | 811 | 432704 | 751296 | 1.74 | 326 | 502 | 484 | 308 | 1727 | 1104 |

Rust/Java nodes: median 1.45, range 1.36-1.74 over 10 searches. Rust/Java blocks closer to the goal: median 1.03, range 1.00-1.54. Rust's best partial path ended nearer the goal in 6, farther in 0, equal (within half a block) in 4.

#### Limit upstream's default limits (primary 500 ms, failure 2000 ms)

| Seed | Case | Dist | Java nodes | Rust nodes | Rust/Java nodes | Java closer by | Rust closer by | Java left | Rust left | Java h | Rust h |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 2 | far | 882 | 110848 | 205376 | 1.85 | 378 | 464 | 504 | 417 | 1829 | 1495 |
| 3 | xz | 383 | 108800 | 193216 | 1.78 | 210 | 284 | 173 | 99 | 617 | 359 |
| 5 | xz | 429 | 137152 | 213824 | 1.56 | 290 | 394 | 139 | 35 | 534 | 133 |
| 10 | far | 1197 | 119616 | 189504 | 1.58 | 313 | 349 | 884 | 848 | 3152 | 3030 |
| 13 | far | 560 | 120576 | 193408 | 1.60 | 23 | 23 | 537 | 537 | 1913 | 1913 |
| 14 | xz | 744 | 123840 | 192832 | 1.56 | 235 | 276 | 509 | 468 | 1948 | 1793 |
| 18 | far | 790 | 122752 | 199552 | 1.63 | 156 | 283 | 635 | 508 | 2446 | 1970 |
| 19 | far | 368 | 122944 | 177280 | 1.44 | 264 | 305 | 105 | 63 | 421 | 256 |

Rust/Java nodes: median 1.59, range 1.44-1.85 over 8 searches. Rust/Java blocks closer to the goal: median 1.20, range 1.00-1.82. Rust's best partial path ended nearer the goal in 7, farther in 0, equal (within half a block) in 1.


## Reading the numbers

- **The ratio is stable in kind and loose in size.** Between the two rounds the median over the cases moved from 1.41 to 1.57, and a single case's ratio moved by a median of 0.24 between the rounds (34 of 51 moved by 0.3 or less, the largest by 1.13). Do not read a
  difference below about 0.2 as real. The columns of minimum and maximum are over all repetitions of both rounds.
- **Short searches show less gain.** By Rust time: under 100 ms, 14 searches, median 1.28 (0.66 to 1.48); 100 ms to 1 s, 19 searches, 1.43 (1.24 to 1.77); 1 to 5 s, 17 searches, 1.61 (0.92 to 1.88); one over 5 s, 1.73.
  Java has been warmed up, so this is not the JIT. A fixed cost per search on one side, or the cache effects of a small search, are possible causes; this was not investigated.
- **The three slow Rust cases** (seeds 19 and 20, round 1) had the Rust clock dip: `rust_finish_1` shows a minimum of 2.5 GHz on the pinned CPUs against 2.9 to 3.2 GHz in the other phases, and repetitions of
  `19.near` ran 18 to 59 ms. In round 2 the same searches gave 1.10 and 1.72. A separate rerun of seeds 19 and 20 afterwards (Java then Rust, on CPUs 2 and 4, not part of the tables)
  gave Java/Rust of 1.28 (19 near), 1.54 (19 far), 1.59 (19 xz), 1.23 (20 near) and 1.41 (20 xz); 20 far finished for Rust in 8.2 s and not for Java in 10 s. These numbers are reported here as a check; they
  do not replace the table, which keeps the slow values.
- **Nodes per second** is nodes taken out of the open set per second of the timed span. Both sides do the same work (same nodes, same movements), so it compares speed per node exactly.
- **Same limit, progress.** A search that has more nodes does not always get farther: the best partial path depends on the order of the open set, which is identical on both sides, so up to the point where
  Java stopped, Rust's search is the same search. At 50 ms the two best nodes are usually the same, within half a block (13 of 14 searches), because both stopped before the frontier moved on. The difference grows with the limit:
  at 500 ms and 2000 ms, Rust ended nearer the goal in 8 of 14 and 6 of 10 searches, by a median of 21 and 9 blocks (at most 118 and 176). At the default limits Rust was nearer in 7 of 8 (median 58 blocks).
- **Unfinished searches in A** (the table of searches that did not finish) are single runs, 10 s each. They fall in the same range: for example Rust took out 3.18 million nodes and Java 1.80 million in 10 s on seed 10, but 1.71
  million against Java's 1.92 million on `18.xz`, which came late in the Rust round 1, in the slow period.

## Limits

- One machine, one thermally limited laptop, one JVM configuration (Serial GC, 6 GB). G1, Minecraft's own JVM flags, and the competing threads of a running game were not tried. Java's JVM start and warm-up are not counted
  (a bot that searches once after joining would pay for them).
- One synthetic generator. The worlds look like terrain, but they are not Minecraft terrain: no biomes, no village or mineshaft structures, no doors or fluids other than still water and lava, no snow, vines or ladders. Real worlds
  with other block mixes could favour either side.
- One configuration (upstream's defaults), no previous path (so no favoring), no avoidance, no mobs, and no segments. Other settings (parkour, diagonals, swimming, a water bucket) change which moves are tried and were not timed.
- Rust built without LTO or `target-cpu=native`; the mod will be built with Horizon's profile, which may differ.
- The world is a flat array on both sides. In the game, Java reads through the client's chunk cache and the port reads through its own snapshots; the cost of that was not measured here.
- 51 searches finished on both sides. The unfinished 9 are single runs. Of the same-limit tables, the searches were picked from the cases of A, so they are the long ones.
- The same path was checked through a hash of the result path's text. It covers positions, movement kinds, destinations, cost bits and the loaded flag, and the counters. Only finished searches are compared; a search that the clock stops depends on the clock.
- Memory use, start-up time and the cost of building the world view were not measured.
- Per-seed numbers add up that seed's finished searches (1 to 3 of them, only 1 on seeds 2, 10 and 18). There are no pooled means across seeds anywhere in this document.

## How to rerun

```sh
python3 tools/run_bench.py --upstream upstream/baritone-1.21.4 --jdk DIR --scratch DIR     # about 35 minutes, no game, build or Gradle run meanwhile
python3 tools/run_bench.py --tables-from DIR                                              # rewrite the tables from the raw results of an earlier run
```

See [the tools README](../tools/README.md#bench). The tables above are the file `tables.md` of that run with the text around them added by hand.
