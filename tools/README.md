# Tools

Test tools for the port. They are not part of the mod.

## cost-compare

Compares upstream's real Java classes with `mod/engine`, bit for bit. The Java side never holds a second implementation of
a rule: it builds worlds and settings, calls upstream's own classes, and writes down what they answer.

1. `java/` is a JUnit test to copy into a checkout of upstream's `src/test/java/` (`run_compare.py` does it). It writes one
   golden file:
   - `S` lines: the palette (about 190 block states, oak doors over facing, hinge and open, copper and iron doors, gates,
     trapdoors, stairs, slabs, snow, fluids of every level, bamboo, a moving piston, and more). Each carries the raw game facts
     that `mod/engine/src/blocks.rs` `State` holds, and the mining strength with a pickaxe and with an empty hand (kept out of
     `State`: the comparison builds the `tool_strength` slice from them). Baritone's rules are never exported as facts.
   - `C` lines: 14 configurations. Each setting of `mod/engine/src/config.rs` takes two different values in at least one of
     them, including the three block lists; `main.rs` checks that and fails if a setting is missing a value.
     Settings are applied to `Baritone.settings()` before each context is built, and `PrecomputedData.forCurrentSettings`
     rebuilds its table by itself when they changed.
   - `G` lines: grids of 13 x 16 x 13 with the start at (6, y, 6), y from 1 to 15 (so that the build limit is reached),
     and whether the column is loaded (about 10% are not). Modes 0 to 6 are random fills (noise, fluids, water, open air).
     Modes 7 to 10 build a structure around the start in all four directions: parkour corridors (gaps of 1 to 4, flat,
     ascending and placed landings, walls, bad take-off and landing blocks, hazards after the landing), staircases and
     slabs, columns of ladders, vines, water, lava and falling blocks, and diagonals with doors, magma, lava and more at
     both corners.
   - `R` lines: for every grid, configuration and `Moves.values()` ordinal, `Moves.apply` after `reset()`: destination and
     cost in hex. Tallied per family: downward, pillar, traverse, ascend, descend, diagonal, parkour.
   - `P` lines: the rules that depend on a position (canWalkThrough, canWalkOn, fullyPassable, canPlaceAgainst,
     mustBeSolidToWalkOn, isReplaceable, canUseFrostWalker, avoidBreaking, avoidWalkingInto, isFlowing, both mining times,
     costOfPlacingAt) at 8 positions near the start. Tallied as `predicates`.
   - `A` lines: every state of `Block.BLOCK_STATE_REGISTRY` (about 27,900), with the same facts and upstream's answers for
     thirteen block-state rules under two configurations (the default, and one with magma, vines, lava walking, bottom slabs
     flipped and another `blocksToAvoid`). Tallied as `states`.
   - `F` line: all 4097 `FALL_N_BLOCKS_COST` entries and `JUMP_ONE_BLOCK_COST`. Tallied as `fall_table`.
   - `Q` lines: instances of every goal class (random and extreme parameters, coordinates near +/-30,000,000 and near the
     int limits, directions, RunAway with and without a fixed Y, nested Composite and Inverted), each at six positions, with
     `isInGoal`, `heuristic(x,y,z)` and `heuristic()`; `heuristic()` is only asked of goals whose box scan is small. Tallied
     per class: `goal_block`, `goal_xz`, `goal_ylevel`, `goal_twoblocks`, `goal_gettoblock`, `goal_near`, `goal_axis`,
     `goal_strict`, `goal_runaway`, `goal_composite`, `goal_inverted`.
   Generation is deterministic (fixed seeds): a second run writes an identical file.
2. `src/main.rs` is the comparison. It reads the golden file, runs the engine (`Moves::ALL[ordinal].apply`, the rule
   functions, the goals) and prints one line per family: evals, finite, distinct, exact, within 1e-9 and WRONG. A family
   with no finite result tested nothing, so `run_compare.py` fails on it. `cost-compare` is a member of the root workspace
   and depends on `mod/engine` by path; it has no `Cargo.lock` or profile of its own.
3. `mutate.py` puts one deliberate error at a time into a copy of the engine and reports whether the comparison notices.
   Use it to check that the data is strong enough, because a clean comparison can also mean a weak test. Each mutation is
   `(file, pattern, replacement, name)`; `mutate.py REPO --where` shows where every pattern matches. To save time, a run stops at its
   first wrong result (the unmutated run never does), so the `wrong=` count of a caught error means little. Verdicts are saved next to
   the golden file and reused only when the compiler, every copied source, the golden file and the mutation are byte-identical;
   `BARITONE_MUTATE_NO_CACHE=1` turns that off. `BARITONE_MUTATE_JOBS` sets the workers, by default half of the threads.

```sh
# From the repository root; JDK 21, an upstream checkout at the pinned commit. The first run may download Minecraft.
python3 tools/run_compare.py --upstream upstream/baritone-1.21.4 --jdk DIR --scratch DIR --grids 300

# By hand, with a golden file already written:
cargo run --release -p cost-compare -- /some/path/golden.txt
python3 tools/cost-compare/mutate.py . /some/path/golden.txt
```

The default is 10000 grids. The golden file is generated, not committed, and stays below about 500 MB at that size.

`java/baritone/api/BaritoneAPI.java` replaces upstream's class on the test classpath only, because the real one needs
a running Minecraft to read its settings file. Both branches use the same file.

### Against upstream's 26.3 branch (`--port 26.3`)

26.3 is the game version Horizon implements, so the whole engine is also compared there. There is still one Java source,
`java/`. `port_26_3.py` turns it into the 26.3 variant on the way into the checkout: a table of text replacements (the
renamed classes, the white glass, carpet, concrete powder and bed, the copper doors, the split chain, the missing
`allowSwimming`, `canPlaceAgainst` and `avoidBreaking` overloads, and the new palette blocks `copper_chest`, `sulfur_spike`
and `oak_shelf`) and two replaced regions. Every replacement must match exactly the number of times it says and every
region once, or the script fails loudly, so a change of `java/` that the table does not follow cannot go unnoticed.
`python3 tools/cost-compare/port_26_3.py DIR` writes the variant to look at it.

What cannot be a replacement is in `port-26.3/baritone/pathing/movement/Compat263.java`, copied next to the test:

- 1.21.4 has constructors made for tests (`BlockStateInterface(border, minY, height)` and a protected
  `CalculationContext` constructor). 26.3 has none. `Compat263` allocates both objects without running a constructor and
  fills their fields by reflection, from the same settings the real constructor reads, and overrides `get0`. The movement
  code under test is untouched. A fake `Level` answers only `getMinY` and `getHeight`. Each context gets a fresh
  `PrecomputedData`, because 26.3 builds that table from the settings of the moment and never rebuilds it.
- 26.3 gives items their tool rules through `DataComponentInitializers`, which a bare `Bootstrap` never runs. `Compat263`
  runs them after the block and fluid tags are bound. Without it every `ItemStack` fails.
- `MovementHelper.isWaterloggedSolidTop` does not exist on 26.3. The registry sweep has a column for it, so `Compat263`
  holds its 1.21.4 body. That column is therefore not an answer of 26.3's code, and no 26.3 rule calls it.

```sh
# JDK 25 and a checkout of upstream's 26.3 branch (commit 25111da). Writes reports/cost-compare-26.3.json.
python3 tools/run_compare.py --upstream upstream/baritone --jdk DIR --scratch DIR --grids 10000 --port 26.3
```

The engine follows 1.21.4's rules, so with `--port 26.3` the differences between the two branches are expected. They are
recorded in the report and are not a failure; every other check still applies (a family with no finite result still fails).
The registry sweep covers all 35,723 block states of 26.3.

`rules-26.3.patch` makes a scratch copy of `mod/engine` behave like 26.3's rules, one group of hunks per rule (each has a
`# rule:` line). Never apply it to the real engine. To use it, copy `Cargo.toml`, `Cargo.lock`, `mod/engine` and
`tools/cost-compare` (without `target` and `java`) to a scratch directory, run `patch -p1 -F0 < rules-26.3.patch` there, and
run `cargo run --release -p cost-compare -- GOLDEN` in it with the golden file of the 26.3 run. At 10,000 grids the
unpatched engine differs from 26.3 in 22,349 results; with the whole patch there are 0 wrong and 0 close in every family.
To measure one rule, apply every hunk group but its own. The counts are in
[Where upstream's two branches disagree](../docs/deviations.md#where-upstreams-two-branches-disagree).

Known limits:

- The facts come from Minecraft 1.21.4 or 26.3 (whichever branch ran), not from Horizon.
- The grids are 13 x 16 x 13 with the start in the middle; movements are only asked from the start. Other starts and
  world heights (a grid never sits at the real minimum or maximum Y of a dimension) are not covered.
- The harness binds the vanilla block and fluid tags itself (it reads `data/minecraft/tags/block/` and `fluid/` from the
  game jar and binds them on the registries), so a diamond pickaxe and an empty hand give different break speeds, and
  `LiquidBlock.isPathfindable` (which asks the fluid tags) no longer calls lava pathfindable. Tool choice is tested for
  those two tools and for no other tool.
- The 26.3 run is not part of the gate, is not repeated three times and has no mutation run. `rules-26.3.patch` was written
  by reading the diff of the two branches, then checked by the comparison; a rule that the generated worlds never reach
  would not show up in it, and the patch cannot tell a missing rule from a world that never exercises it.
- `Moves.apply0`, the executors and everything after the cost functions are not compared here.
- A full mutation run is long: 836 builds, about 39 minutes on 6 workers on the development laptop (104 on one). Named mutations can be
  run alone (`mutate.py REPO GOLDEN "name|name"`).
- Injected errors that stay uncaught are listed with a reason in `cost-compare/known-missed.txt`; those judgements come
  from reading the code and the exported facts, not from a proof.

## search-compare

L2: does the same world, goal and settings give the same search? It compares whole searches of upstream's real classes
(`AStarPathFinder`, `Path`, `CutoffPath`, `SplicedPath`, `Favoring`, `Avoidance`, and `PathingBehavior.createPathfinder` by
reflection) with `mod/engine`, node by node. The Java side never holds a second implementation: it builds worlds and settings, runs
upstream and writes down what it answered.

1. `java/baritone/pathing/calc/SearchGoldenDump.java` is a JUnit test. It sits in upstream's package `baritone.pathing.calc`,
   because `Path` is package-private and the search keeps its state in protected fields. `java/.../SearchBridge.java` is a public
   door into `cost-compare`'s `SpikeGoldenDump` (palette, configurations, test context), so both tools share them without a copy
   and without a change to that file. `run_search.py` copies both Java directories into the checkout and removes them again.
2. **Worlds** (`V` lines) are generated from fixed seeds: 12 kinds (flat, hills, caves, lakes, ladders and vines, village with doors,
   stairs, slabs, trapdoors and gates, walls with gaps and chasms, falls and pits, towers, mazes, random palette noise, terraces),
   32 to 96 blocks on a side and 24 to 64 high, from the 191-state palette of `cost-compare`. Each has an origin (also far from 0,
   negative, next to the world border, and three small ones that put z = 0 inside the world), a minimum Y of 0, -64, -20 or -3 (the last two put
   the terrain across y = 0), chunk states (loaded, only known to the cache, not loaded), and a world border (the default one, or one
   that cuts the world). Cells are run-length coded. Three hand-built worlds follow the generated ones (`specials`): a world 4064 high
   (y from -2032) with a ladder column, a platform at y = 0 and two high platforms, searched under all 16 configurations; a one-wide ladder
   shaft 4064 high, searched up and down with caps of 2047 and 2049 and without; and a one-wide corridor 70000 long along z from a negative
   z, searched at caps of 46000, 46400 and 65540. They exist because the random worlds are at most 64 high and 96 long, so they never reach
   the wrap of the cell key's y part (2048 blocks), a y span of 256, or the wrap of `getDistFromStartSq` (46341 blocks). Both wraps are
   upstream's behaviour, kept (`docs/deviations.md`, U1 and U2).
3. **Searches** (`E` lines): start, the player's feet (the same, a block away, farther, higher; the goal is sometimes the start itself),
   one of 16 configurations, one of 23 sets of search settings (`K` lines: favoring coefficient, `pathingMaxChunkBorderFetch`,
   `minimumImprovementRepropagation`, `pathCutoffMinimumLength` and `Factor`, `cutoffAtLoadBoundary`, `simplifyUnloadedYCoord`,
   avoidance, also with only one of the two coefficients away from 1), a goal of any of the 11 classes (composite and inverted too), and a node cap of 1, 2, 30, 63, 64, 65, 500, 5000 or
   none. Chains of up to four segments search from the end of the previous path with `Favoring` built from it (one in four changes
   the goal), and `SplicedPath.trySplice` joins them with `allowOverlapCutoff` off and on. 15 % of the chains start with a short
   search whose path only serves as a favoring base. Spawners and mobs sit within 10 blocks of the start; in 30 % of the chains some of
   them are moved by an offset that makes upstream's `longHash` of the sphere equal that of the start's surroundings (y + 1 and z - 2873465,
   and two more) or nearly (2^24 apart in x or z, 256 in y), so the collisions of that polynomial are exercised.
4. **Stopping.** A node cap is made on the Java side without changing upstream: the goal is wrapped, and on the N-th `isInGoal`
   call (one per node taken out of the open set) the wrapper calls `cancel()`. The loop ends after that node is expanded. The harness
   then calls `bestSoFar(false, numNodes)` and `postProcess`, `cutoffAtLoadedChunks` and `staticCutoff` itself, in `calculate`'s order,
   so each stage can be compared. A search without a cap goes through `calculate(huge, huge)`. A search that has no cap and would
   run past 20,000 nodes is stopped by the same wrapper and written as a run with that cap. The time limits never fire on either side.
5. **What is compared**, as text, line by line: result type, nodes considered, movements considered, nodes made (`mapSize`), a hash
   of the positions taken out of the open set in order (ties decide that order), the seven `bestSoFar` nodes with their costs (bits),
   `mostRecentConsidered`, `pathToMostRecentNodeConsidered`, `bestPathSoFar()`, the path after each of the three stages and the final
   path, with every movement's kind, destination, cost (bits) and `calculatedWhileLoaded`, the goal after simplification, and the
   splice of each segment. In the Rust run, `calculate()` of a second finder is also checked against the staged run (one search in
   three).
6. `src/main.rs` reads the file, runs the engine and compares. It also counts how often each case was reached, using hooks in the
   engine that only count (`SearchStats`, `popped_hash`). The run fails if a count is zero, unless the case is exempt with a
   reason: coverage includes each result type, each of the seven coefficients, every `Moves` value and `MovementFall` in a final path,
   decrease-key, ties in the heap, `numEmptyChunk` ending a search, border rejection, `MIN_IMPROVEMENT` keeping an update out, `runBackwards`
   failing into a `CutoffPath`, the `realStart` fake node, favoring, both cutoffs, trySplice with overlap, without and refused,
   a simplified goal, and final paths that cross x = 0, y = 0 and z = 0, span 256 in y or pass 46340 blocks. `src/parse.rs` is copied from `cost-compare/src/main.rs` (it is a binary crate).
7. `mutate.py` injects errors one at a time into a copy of the search code (491 of them: open set, nodes, the A* loop, paths, cutoffs,
   splicing, favoring, avoidance, planning, movements and the move table) and checks that the comparison notices. It uses the output format
   and workers of `tools/cost-compare/mutate.py` (`BARITONE_MUTATE_JOBS`), and for a missed one it also runs the engine's unit tests and prints a
   note, which never counts as caught. A full run took 34 minutes on 12 workers. Missed ones are in `known-missed.txt` as `no effect` or `hole`.

```sh
# From the repository root; JDK 21, an upstream checkout at the pinned commit. Writes reports/search-compare-1.21.4.json.
python3 tools/run_search.py --upstream upstream/baritone-1.21.4 --jdk DIR --scratch DIR [--worlds 900 --cases 24 --repeat 3]

# By hand, with a golden file already written:
cargo run --release -p search-compare -- /some/path/search-golden.txt
python3 tools/search-compare/mutate.py . /some/path/search-golden.txt
```

Known limits:

- The timeout logic (`failing`, the 64-node clock check) is not compared: no clock fires on either side. Unit tests in `astar.rs` cover it.
- Cancellation is not compared against Java (`CANCELLATION` is exempt in the report): the harness stops runs by the node cap, which acts as a timeout.
  Upstream resets its flag when a search starts, so there is nothing to compare. For one search in three the harness checks the Rust promises
  instead (a raised flag, also raised during the search, ends with no path; a second `calculate` is refused; `postProcess` twice fails;
  `staticCutoff` without a goal changes nothing) and counts a failure as a mismatch.
- A search that throws is compared as `EXCEPTION` only. Upstream's `calculate` swallows the exception, so the Java side cannot name the cause.
  The Rust side checks it instead: a search may throw only with a NaN heuristic (cause `ImplausibleHeuristic`) or a `pathCutoffFactor` above 1
  (cause `CutoffOutOfRange`), and any other exception or cause counts as a mismatch. 275 of 29,446 searches threw in the committed corpus run.
- The first cell key equal to `Long.MIN_VALUE` throws in upstream and not here (`docs/deviations.md`, D9); it needs a position beyond the world
  border, so the corpus does not contain it.
- `run_search.py` refuses to start Gradle while a game (`horizon-client`) runs; close it or wait.
- A search without a cap and with more than 20,000 nodes is compared as a run with a cap of 20,000.
- The stage lines (after `postProcess`, `cutoffAtLoadedChunks`, `staticCutoff`) exist for runs with a node cap only; a run
  through `calculate` shows the final path.
- `PathingBehavior.createPathfinder` is the real method, called by reflection on an instance made without its constructor, with
  small stand-ins for the player context (feet position, entity list, cached world). The entity list is empty, so the avoidances of
  mobs are added by the harness with upstream's own `Avoidance` class right after; `Avoidance.create` chooses which mobs avoid,
  which is the glue's job (S4). Spawner avoidance goes through `Avoidance.create` itself.
- The block facts and the world are Minecraft 1.21.4's, not Horizon's. The worlds are small boxes; the world outside them is air and
  not loaded. Searches do not reach 26.3's differences (see the cost-compare section).
- The Rust side reads the whole file; the default size is about 100 MB.

## bench

Times upstream's real search against `mod/engine`'s on the same large generated worlds, and compares them by seed. The results and
their limits are in [`docs/BENCHMARK.md`](../docs/BENCHMARK.md). Like the other comparisons it holds no second Java search: it times
`PathingBehavior.createPathfinder` and `AbstractNodeCostSearch.calculate` of the checkout.

1. `bench/java/baritone/pathing/calc/SearchBench.java` (a `main`, in upstream's package like `SearchGoldenDump`) is the only world
   generator. A world is `n` x `n` columns (256 to 1024) and 80 high: noise terrain in four grades of relief, sea and lakes, rivers,
   ravines, caves with lava, trees, and three cobblestone walls with gaps in every third seed. Every chunk counts as loaded and the
   world border is the box, so a search never stops at the box's edge. Each seed has four cases: a `GoalBlock` 100 to 220 blocks
   away (`near`), a `GoalBlock` and a `GoalXZ` far away (`far`, `xz`: 55 to 95 % of the room the world leaves in that direction), and a
   `GoalBlock` 24 blocks under the surface (`deep`, only used for the same-limit runs). Settings are config 0 and search settings 0 of
   the cost and search comparisons: upstream's defaults, with a pickaxe, a throwaway block and sprinting.
2. `bench/src/main.rs` (a crate on its own, `search-bench`, outside the root workspace so that the hashes in the comparison reports
   do not change) reads the files the Java side wrote (`world-N.txt`, in the format of `search-compare`'s golden file, with the
   world's hash checked against the Java side's) and times `create_pathfinder` and `calculate` the same way.
3. `run_bench.py` builds both halves, waits by PID until no game, build or Gradle run is going and the CPUs are idle, pins both sides
   to the same two CPUs (one thread of each of two physical cores), and runs: Java finish, Rust finish (twice), Java same-limit, Rust
   same-limit. The Java process warms up on other worlds (3 million nodes) before it times anything. A search is timed from the
   creation of the finder to the result of `calculate()`, after the context is built; the first run of each case is discarded.
   Both sides write `T` lines with every repetition's nanoseconds, the counters and the hash of the result path (positions and
   movements, as in the L2 comparison's `sf` line); the script compares the counters and the hash.
4. The raw timings, worlds and tables stay in `--scratch` (`results.json`, `tables.md`, several hundred MB). `reports/bench-1.21.4.json`
   holds metadata and counts only.

```sh
# From the repository root; JDK 21, an upstream checkout at the pinned commit; no game, build or Gradle run, and nothing else heavy.
python3 tools/run_bench.py --upstream upstream/baritone-1.21.4 --jdk DIR --scratch DIR [--seeds 20 --finish-ms 10000 --reps 5 --rounds 2 --cpus 2,3]
# Only wait until the machine is idle:
python3 tools/run_bench.py --wait-only
```

The whole run takes about 35 minutes. The time-to-finish phase runs in two rounds (Java, then Rust, twice) so that a drift of the machine shows; `--tables-from SCRATCH` rewrites `tables.md` from an earlier run's `results.json`. The unit tests of its pure parts are in `test_bench.py`; `cd tools/bench && cargo fmt --check &&
cargo clippy -- -D warnings` checks the Rust half, which the root workspace does not cover.

## facts-compare

L1: does Horizon describe each block as Minecraft does? It is not in the root workspace, because it builds against Horizon's
crates in the git-ignored `upstream/horizon-nightly`.

- `java/baritone/FactsDump.java` writes what real Minecraft says about every block state, with the fields of the engine's
  `State`. It binds the vanilla block and fluid tags first. A bare `Bootstrap` has neither, and without the fluid tags
  Java says that lava is pathfindable.
- `src/main.rs` compiles `mod/glue/state_facts.rs`, the mapping the mod will use, and compares field by field. A state's
  identity (its id, name and properties) has to match too.
- `expected-differences.txt` lists the explained differences. It is empty today. An unlisted difference fails, and so does
  an entry that matches nothing. For each difference, the tool also says whether the engine's answers change.
- `mutate.py` injects errors into the mapping. Errors with no possible effect are listed in it, with a reason.

```sh
# JDK 25, upstream's 26.3 branch, and a Horizon version's prepared .local/horizon-client/26.3 folder.
python3 tools/run_facts.py --upstream upstream/baritone --jdk DIR --scratch DIR \
    --horizon upstream/horizon-nightly --resources DIR
# Add --versions-against upstream/baritone-1.21.4 --jdk-old DIR to list what changed between the game versions.
```

Known limits:

- Horizon's facts are exported from Java when a version is prepared, so this mainly tests our mapping and Horizon's loaders.
  It says nothing about Horizon's own behaviour in the game, such as its physics.
- Facts are read over an empty level. A block whose shape depends on its neighbours or its block entity is only checked there.
- Tool strength is checked for a diamond pickaxe and an empty hand, without enchantments. The other tools wait for the glue.
- The tool reads `mob-data.json` directly. In the mod, the same value comes through `MobData`, which isn't compiled here.

## walk-test

A native mod that walks the player along a fixed course from `native_tick`, and writes every tick to a CSV file (the
path in `BARITONE_WALK_LOG`, or `baritone-walk.csv`). It pins a Horizon revision, because it writes private `Game`
fields. It expects a stone platform around (1000, 99, 1000) with a one-block step at (1018, 100, 1024).

Build it for a separate instance, so that no other mods are involved:

```sh
horizon-launcher instance create baritone-test
horizon-launcher native-build tools/walk-test --instance baritone-test --trust-native-code
```

Start the course by typing `/baritonewalk` in the chat box. `horizon-ctl chat` and `command` do not reach it, because
on a connected game they go straight to the server.

## input-probe

A native mod that answers, in the real game, whether a mod can walk, jump, sneak and sprint through the stable
`native_movement_input` hook with the chat or the inventory open ([RESULTS.md](../docs/RESULTS.md#walking-with-the-chat-open)).
It runs by itself: with the environment variable `BARITONE_PROBE_DIR` set to a folder, it waits for the player to be in a
world, sets up a stone platform at (980..1050, 99, 980..1050) through the single-player server's console (the call
`horizon-ctl command` makes), teleports the player to a start point, opens the screen itself (`open_chat_method("")`, or
`Screen::Inventory` set directly), drives the hook for 60 ticks and writes one CSV row per tick (`probe.csv`: position,
yaw, on-ground, sprint flag and speed modifier, speed, the screen, what the hook saw from the game, what it set) and
`probe-console.txt` (every console command and its answer, and the server's own position of the player after each
trial). `runtime.rs` holds the 17 trials. Without the variable it does nothing. It pins a Horizon revision.

```sh
L=~/Applications/horizon/horizon-launcher-linux-x86_64.AppImage
$L native-build tools/input-probe --instance baritone-test --trust-native-code       # about 2 to 3 minutes
mkdir -p OUT
# The game must not pause when the window has no focus: pauseOnLostFocus:false in the instance's options.txt.
BARITONE_PROBE_DIR=OUT $L play --instance baritone-test --world SOME-NEW-WORLD-NAME   # creates the world if it is missing
# Wait for OUT/probe.csv (about two minutes with the world creation), then close the game you started by its PID.
```

No input is sent to the desktop. The world is created by `play --world`, which needs no clicks. While the game window is
focused, a real key pressed by someone sitting at the machine goes into the open chat (this happened once in the first
run, with the probe not yet typing: a single `t` reached the server as a chat message).

## break-place-probe

A native mod that answers, in the real game, whether a mod can switch the hotbar slot, break a block, place a block (against a
floor, and under its own feet while jumping) and use a water bucket, with the chat closed and open
([RESULTS.md](../docs/RESULTS.md#breaking-and-placing)). Like `input-probe` it runs by itself: with `BARITONE_BP_DIR` set to a folder it
waits for the player to be in a world, builds a stone platform, switches to survival, and runs 18 trials (each from the same
start, with an empty inventory and then an iron pickaxe, 64 stone and a water bucket), asking the single-player server about the
result every tick through the server console. It writes `probe.csv` (one row per tick: feet height, slot, held item, buttons, the
three watched blocks as the client sees them), `probe-console.txt` (every command and its answer, with milliseconds since the
trial started) and `summary.txt` (`RESULT` lines with the client and server ticks, `TOOL` lines with the game's own mining
numbers). `runtime.rs` holds the trials and is plain `std` Rust, so `rustc --edition 2024 --crate-type lib runtime.rs` checks it.
It hooks the private `remote_handle_input` (a `HEAD` and a `RETURN`), so it pins the Horizon revision. The trials ending in
`_noswap` are the controls: the same buttons with the chat open and the screen left alone, which must do nothing.

```sh
L=~/Applications/horizon/horizon-launcher-linux-x86_64.AppImage
$L native-build tools/break-place-probe --instance baritone-test --trust-native-code       # 2 to 3.5 minutes
mkdir -p OUT
BARITONE_BP_DIR=OUT $L play --instance baritone-test --world SOME-NEW-WORLD-NAME
# Wait for OUT/summary.txt (about 1.5 minutes after the world opens), then close the game you started by its PID.
```

It replaces the mod built into the instance, so build `mod/` again afterwards. No input goes to the desktop.

## The mod and its scenario trigger

`mod/` is the native mod (id `baritone_rs`): the bundle `horizon-mixin.toml`, `runtime.rs` (state, snapshots, the tick, the commands, the
drawing, and the scenario trigger below), small hook helpers (`tick.rs`, `movement.rs`, `command.rs`, `commands.rs`, `list.rs`,
`primitives.rs`, `unload.rs`, `disconnecting.rs`), `engine/` as a `[[crate]]`, and `glue/state_facts.rs`, which is injected as the module
`baritone_rs_facts` and is also the file `tools/facts-compare` compiles (one file, no copy). It pins Horizon `66f3981`. In the game:

- `/b goto <x> <y> <z>`, `/b goto <x> <z>`, `/b goto <y>` (a `~` makes a number relative to the player; `k` and `m` scale), `/b goal [<x> <y> <z> | <x> <z> | <y> | clear]` (no argument: the player's feet),
  `/b path`, `/b eta`, `/b set [list | modified | reset <name|all> | toggle <name> | save | load | <name> [value]]`, `/b stop`, `/b help`, `/b trace`. `/baritone` is the same, and so are `cancel` for `stop` and `setting`/`settings` for `set`.
  Messages come with the prefix `[baritone-rs]` and, where upstream has a text, are upstream's. It walks, sprints, breaks and places blocks (`allowBreak` and `allowPlace` are on by default, as upstream's);
  parkour and swimming stay off. Settings are upstream's names, types and defaults (about 80), saved at once to `horizon-mods/.horizon-mod-data/baritone_rs/settings.txt` in the game folder. Tab in the chat box completes
  the command names, the settings, their values and, for `goto` and `goal`, `~` and the coordinates of the block the crosshair is on.
- Breaking and placing use the game's own input step, so the mod hooks `remote_handle_input` (a private function: the mod is pinned to a Horizon revision), and completion hooks `Dispatcher::client_suggestions_asking`
  (also private, and in another crate: the mod adds a small module to `horizon-command` to hand its function across). The hotbar is read each tick; the tool in the hand is chosen as upstream's `ToolSet` does and is not put back.
- With no goal given since the game started, the mod does nothing: no thread, no table, no key.
- The first `/b goto` makes the block table from `mob-data.json` and Horizon's block facts (about 0.1 s). If `mob-data.json` is missing or does not
  match the game, the mod says so in chat and does not path.
- The path is drawn in red, the next segment in magenta, a block goal as a green box and an x/z goal as a beam.
- It stops, with the keys the player's again, on `/b stop`, death, a dimension change, the level being dropped, the connection ending, and
  the mod being turned off in Options > Mods.

Build it for the separate instance (about 2 to 3.5 minutes; one build at a time, check `pgrep -a horizon-client; pgrep -af 'cargo|rustc'` first):

```sh
L=~/Applications/horizon/horizon-launcher-linux-x86_64.AppImage
$L native-build mod --instance baritone-test --trust-native-code
```

**Scenario trigger (test only).** With the environment variable `BARITONE_RS_SCENARIO_DIR` set to a folder, the mod runs six fixed walks by
itself once a single-player world named `l3-...` is open (the mod refuses any other world), and writes there. Without the variable the scenario code returns at once. It sends no input to the desktop. It builds a stone platform (980..1050, 99, 980..1050) through the
single-player server's console, trying again until the chunks are loaded, then runs, each from (1000.5, 100, 1000.5):
`smoke` (`b goto 1030 100 1000`), `diagonal` (`b goto ~20 ~ ~20`), `xz` (`b goto 1030 1010`), `stop` (`b stop` after 30 ticks of walking),
`dimension` (a teleport to the Nether after 30 ticks) and `death` (`kill @p` after 30 ticks). The commands go through `on_command`, the code
a typed command reaches. Files: `scenario-result.txt` (PASS or FAIL per run, with the numbers), `scenario-log.txt` (console answers and the
mod's chat lines), and one `<run>.csv` per run with a row for every tick (position, velocity, yaw, on-ground, sprint, whether the keys are
forced, whether a path or a search runs, dead, dimension).

```sh
mkdir -p OUT
# pauseOnLostFocus:false in the instance's options.txt, as for the input probe.
BARITONE_RS_SCENARIO_DIR=OUT $L play --instance baritone-test --world l3-SOME-NEW-NAME
# The last line of OUT/scenario-result.txt is "finished" (about 1.5 minutes after launch). Close the game you started, by its PID.
```

A real typed command, the real keyboard and a connection that really ends are not exercised by this.

**By hand through the control socket.** For a game you started with `play ... --arg --control`, the version's `horizon-ctl` drives it without touching the desktop: `command "give @p minecraft:iron_pickaxe"` and `fill`/`tp` for the setup,
`chat "/b goto 1010 100 1000"` for the mod's commands (this reaches `native_command` at `66f3981`), `key t tap` to open the chat and `type "/b set allowB"` to type in it, `screen` (`chat_input`, `chat_suggestions`) and
`screenshot PATH` to see the suggestion list, `state` and `block X Y Z` to see where the player is and what changed, `inventory`. `key ctrl down`, `key a tap`, `key ctrl up`, `key backspace tap` clears the chat box (`type` has no replace).

## L3 scenarios

`tools/l3/` plays fixed scenarios in the game and judges them (L3 in [PORTING-SAFETY.md](../docs/PORTING-SAFETY.md)). `scenarios.py` builds
each world through the single-player server's console (forceload, fill, setblock; nothing of a world is committed) and says what counts as a
pass. The mod plays the spec (`mod/runtime.rs`, "L3 script player", only when `BARITONE_RS_L3_SPEC` is set), records one row per tick and
judges nothing; `run.py` starts the game of the instance `baritone-test` with a new world, waits for `finished`, closes the game by its PID
(also on failure), and applies the criteria: arrived within 1 block, within a tick limit, blocks changed = 0 in a watched box, never died,
no health lost on drops, plus the checks of each kind (sprinting, a recalculation after a block appears, the stop message and released
keys for an unreachable goal, keys released at `/b stop`). Most scenarios run with the chat closed and open (`chat` in the name).
`m01` to `m05` (R2) cover `/b mine` and `/b goto <block>`: three iron ores at three depths in a small cave (all broken, 3 raw iron, pickaxe
damage equal to the blocks broken, unhurt), a quantity reached through the inventory count (2 raw iron in the hotbar, `mine 3` breaks exactly
one ore), `goto crafting_table` through a wall (ends next to the table, table intact, the table's screen was opened by the right click), and
`/b stop` during `mine` while walking and while the scan runs. For `mine` and `goto <block>` the run is done when the mod has been idle for
60 ticks, and the path to the next ore, made while the last one is broken, is never walked, so the last plan is not required to be done.

The cache and explore scenarios (R2): `c01_goto_chest_cached` stands the player 150 ticks at a trapped chest (the cache packs the chunks within five
chunks of the player every four seconds), moves him 390 blocks away, records with a `probe` event that the chest's chunk is not loaded, and gives
`b goto trapped_chest` (a trapped chest: natural chests lie in the loaded chunks below and are nearer). `c02_goto_across_cached` visits a wall and the goal 400
blocks away the same way, goes back to the start, probes that both are unloaded, and gives a goto whose first path must have more than 290
positions (the loaded world ends about 273 blocks out) round the wall, with `allowBreak` off. `c03_explore` runs `b explore` for 1200 ticks from a random
dry spot (`spreadplayers`), then `b stop`: it walks at least 150 blocks, gets 40 away, goes farther in the later part, loses no health, stops. Each
cache scenario has its own ground, because the cache keeps the chunks of the world for the whole run. `run.py --controls` adds `c01x`/`c02x`, the same
without the visits; they are expected to FAIL (the chest is not found, the first path ends at the loaded edge). `chunkCaching` off is no control: as in
upstream it only stops the files. Every scenario with an area clears the weather again (a half hour run let snow fall in a cold biome).

```sh
# Build the mod into the instance first (see "The mod and its scenario trigger"); no game or build may run.
python3 tools/l3/run.py --out OUTDIR [--only s01 s04] [--no-far] [--report reports/l3-66f3981.json]
python3 tools/l3/run.py --out OUTDIR --judge-only          # judge files from an earlier run
```

Per scenario it writes `<name>.csv` (position, velocity, sprint, health, keys forced, path position, the current movement with its
estimated cost, the mod's tick time), `.segments.txt` (every path installed, with its estimated ticks), `.searches.txt` (the worker's time
for each search), `.snapshots.txt` (the time of each world snapshot and the number of chunks in it), `.notes.txt` (commands and the mod's chat
lines) and `judgement.json`, which also holds the estimated cost against the actual ticks per movement kind. `--report` writes the metadata
only (no paths or names). Scenarios 5 and 10 use natural terrain around the spawn of the new world, so their result depends on the seed
the world got.

Breaking and placing (scenarios `b01` to `b09`, S4): items come from the console (`clear @p`, `give @p ...`), `slot` selects a hotbar slot at the
start, `settings` runs `b set NAME VALUE` before the command and puts the setting back at the end (`after`). The old scenarios whose point is to
walk (`s02`, `s06`, `s07`) set `allowBreak false`, because upstream's default (true) now lets the path dig. Each segment line in `.segments.txt`
carries the `to_break` and `to_place` cells that upstream's `toBreak`/`toPlace` give for the world at that moment, and the result file lists every
changed cell of the watch box. `run.py` checks that every change was planned, that every cell of the last path's plan was done, the exact counts,
that nothing else changed (water only where the scenario allows it), the stack counts and the tool damage, and the estimate against the actual
ticks (0.4x to 2.5x, plus 30). The slot at the end is reported, not judged: upstream never puts it back (docs/deviations.md D32).

The fixes after v0.1.0 (scenarios `b10` to `b12`): `b10_head_sand` sets a sand block in the cell of the player's head at tick 45 (`no_search_after`: the walk goes on, no search that gave a path after it);
`b11_door` and `b12_gate` are a stone corridor whose only way on is a closed oak door or fence gate (`toggled`: the only cells that changed are the door's or gate's own, whose block did not change into another).

## gate.py, check_public_tree.py, run_compare.py

`python3 tools/gate.py` is what a change must pass before a commit; see "The gate" in
[PORTING-SAFETY.md](../docs/PORTING-SAFETY.md). It stops at the first failure and prints steps that could not run as
SKIPPED. Exit status: 0 passed, 1 failed, 2 incomplete. The result goes to `gate-result.json`, which git ignores.

```sh
python3 tools/gate.py --quick                       # no comparison (reported as SKIPPED, so the result is incomplete)
python3 tools/gate.py --upstream DIR --jdk DIR --scratch DIR [--grids N] [--branch NAME] [--no-mutation] [--force]
```

The inputs can also come from `BARITONE_UPSTREAM`, `BARITONE_JDK`, `BARITONE_SCRATCH`, `BARITONE_GRIDS` and
`BARITONE_BRANCH`. The comparison only runs when its source hashes, the upstream commit or the grid count differ from
`reports/cost-compare-<branch>.json`; then it runs three times (the reports must be identical) and `mutate.py` runs.
`cost-compare/known-missed.txt` lists the injected errors that are allowed to go uncaught, each with its reason.

`check_public_tree.py [--history]` checks every tracked file and every untracked file that git does not ignore, so a new
file is checked before `git add`. It fails on game files, large or generated files, Java outside `tools/`, home paths,
email addresses and a private deny list. The deny list is a file named by `BARITONE_RS_DENYLIST` and is never committed.
Without a deny list the step prints SKIPPED and the gate exits 2 (incomplete) instead of 0: this is on purpose, because
nothing in the repository can check account names or other private strings. Set the variable to get a complete run.
The comparison report is written only after the repeat runs and mutation pass; a report on disk means all three passed.

`run_compare.py` runs one comparison and writes the report (metadata and counts only). It refuses to start while a
game or another Gradle client is running. The gate runs `cargo fmt --check`, `clippy -D warnings` and `test` across the
workspace from the repository root.

## ledger.py

Keeps `docs/port-ledger.tsv`, one row per upstream method in scope, as described in "The ledger" in
[PORTING-SAFETY.md](../docs/PORTING-SAFETY.md).

```sh
python3 tools/ledger.py --upstream DIR build              # adds new methods as missing, keeps statuses and notes
python3 tools/ledger.py --upstream DIR check              # checks the rows against upstream and the Rust citations
python3 tools/ledger.py --upstream DIR upstream-diff REV  # methods whose text changed by REV
```
