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
   `(file, pattern, replacement, name)`; `mutate.py REPO --where` shows where every pattern matches.

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
