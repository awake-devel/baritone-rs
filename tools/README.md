# Tools

Test tools for the port. They are not part of the mod.

## cost-compare

Checks a Rust translation of Baritone's movement costs against the Java original, bit for bit.

1. `java/` is a JUnit test to copy into a checkout of upstream's `src/test/java/`. It builds random block grids and
   writes three things to a file: raw game facts per block state, the settings, and the costs that upstream's own
   Java computes. Baritone's rules are never exported, so the Rust side has to derive them.
2. `src/` holds the translation (`lib.rs`) and the comparison (`main.rs`).
3. `mutate.py` puts one deliberate error at a time into the translation and reports whether the comparison notices.
   Use it to check that the data is strong enough, because a clean comparison can also mean a weak test.

```sh
# In a checkout of upstream's 1.21.4 branch, with JDK 21. The first run downloads Minecraft and takes several minutes.
cp -r <this repository>/tools/cost-compare/java/* src/test/java/
SPIKE_OUT=/some/path/golden.txt SPIKE_GRIDS=40000 ./gradlew test --tests 'baritone.pathing.movement.SpikeGoldenDump'

# Here, in tools/cost-compare/
cargo run --release -- /some/path/golden.txt
python3 mutate.py . /some/path/golden.txt
```

The golden file is about 80 MB and is generated, so it is not committed.

`java/baritone/api/BaritoneAPI.java` replaces upstream's class on the test classpath only, because the real one needs
a running Minecraft to read its settings file.

### Against upstream's 26.3 branch (`java-26.3/`)

`java-26.3/` is a separate copy of the harness for upstream's `26.3` branch (the game version Horizon implements). It
needs JDK 25 and the same Gradle command; Gradle 8.14 runs on JDK 25 and downloads Minecraft 26.3 on first use. The
1.21.4 copy in `java/` does not work there, for these reasons:

- 1.21.4 has constructors made for tests (`BlockStateInterface(border, minY, height)` and a protected
  `CalculationContext` constructor). 26.3 has none. The copy allocates both objects without running a constructor
  and fills their fields by reflection, from the same settings the real constructor reads, and overrides `get0`.
  The movement code under test is untouched. A fake `Level` answers only `getMinY` and `getHeight`.
- 26.3 gives items their tool rules through `DataComponentInitializers`, which a bare `Bootstrap` never runs. The copy
  runs them (`bindItemComponents`) after binding the block tags. Without it every `ItemStack` fails.
- Renamed or removed: `ResourceLocation` is `Identifier`, `WaterlilyBlock` is `LilyPadBlock`, the white glass and carpet
  are `Blocks.STAINED_GLASS.white()` and `Blocks.CARPET.white()`. `allowSwimming` does not exist, so the config line still
  writes the flag but nothing applies it. `MovementHelper.isWaterloggedSolidTop` does not exist, so the copy holds its
  1.21.4 body and exports the same fact.
- The `PrecomputedData` table is built from the settings of the moment, so each context gets a fresh one.

The comparison on a 26.3 golden file is expected to show differences. They are rule differences between the branches
(the translation follows 1.21.4), and game differences would show as differing `S` lines.

Known limits:

- It covers `MovementTraverse.cost` and `MovementDescend.cost` only, over 87 hand-picked block states.
- The facts come from Minecraft 1.21.4, not from Horizon.
- The harness binds the vanilla block tags itself (it reads `data/minecraft/tags/block/` from the game jar and binds
  them on the block registry), so a diamond pickaxe and an empty hand now give different break speeds. Tool choice
  is tested for the two configurations in the file (pickaxe, empty hand) and for no other tool.
- Eight injected errors are still missed. Each one has been read and judged to have no effect on these two functions
  in 1.21.4 (redundant with another rule, or only reachable from other movements). Those judgements come from
  reading the code and the exported facts, not from a proof. They are listed in `docs/PORTING-SAFETY.md`.

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
game or another Gradle client is running.

## ledger.py

Keeps `docs/port-ledger.tsv`, one row per upstream method in scope, as described in "The ledger" in
[PORTING-SAFETY.md](../docs/PORTING-SAFETY.md).

```sh
python3 tools/ledger.py --upstream DIR build              # adds new methods as missing, keeps statuses and notes
python3 tools/ledger.py --upstream DIR check              # checks the rows against upstream and the Rust citations
python3 tools/ledger.py --upstream DIR upstream-diff REV  # methods whose text changed by REV
```
