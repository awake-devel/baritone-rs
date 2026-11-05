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

Known limits:

- It covers `MovementTraverse.cost` and `MovementDescend.cost` only, over 85 hand-picked block states.
- The facts come from Minecraft 1.21.4, not from Horizon.
- A bare `Bootstrap` has no block tags, so a pickaxe and an empty hand give the same break speed. Tool choice is not
  tested.

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
