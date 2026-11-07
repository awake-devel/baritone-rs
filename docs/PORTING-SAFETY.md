# Porting safety

**Status: approved on 2026-11-05. Not built yet, apart from the two tools named below.**

How baritone-rs shows that the port behaves like Baritone, and where it knowingly doesn't. The ideas come from how
Horizon checks itself against Minecraft ([parity workflow](https://github.com/horizon-rust/horizon/blob/main/docs/parity-workflow.md),
[port ledger](https://github.com/horizon-rust/horizon/blob/main/docs/port-ledger.md)), scaled down to a small project
on one laptop. Only the ideas: our tools are written here, without Horizon's code.

## The rules

1. **Upstream's code is the specification, not our memory of it.** A rule is translated from the pinned commit: same
   order of operations, same constants, same conditions.
2. **Evidence means running the real thing.** A comparison feeds the same inputs to upstream's Java and to our Rust.
   The Java side runs upstream's real classes. It is never a second implementation of the algorithm.
3. **Every difference is written down.** A place where we differ from upstream, on purpose, has a deviation entry. A
   difference with no entry fails the check.
4. **Report the tested scope, never a percentage.** "1,920,000 comparisons match" describes those inputs only. No finite
   set of inputs proves equivalence, so this project never claims "100% parity".
5. **A clean result has to prove it could have failed.** Each comparison comes with injected errors that it must catch.
6. **A test that did not run is not a pass.** Skipped cases are recorded as skipped.

## Levels of evidence

Each level answers a different question. Passing one says nothing about the next.

| Level | Question | How | Needs the game |
|---|---|---|---|
| **L0 Rules** | Does each translated function give upstream's answer? | Bit-exact comparison per function, over generated grids | No |
| **L1 Facts** | Does Horizon describe each block as Minecraft does? | Horizon's facts against Java's, for every block state | No |
| **L2 Search** | Does the same world and goal give the same path? | Whole searches compared node by node | No |
| **L3 Scenarios** | Does the player actually get there in Horizon? | Fixed worlds played in the client, with pass criteria | Yes |
| **L4 Servers** | Does it hold on a real server? | Allowed servers only, reported per server | Yes |

- **L0** covers the seven movement costs, the predicates and the goals: [`tools/cost-compare`](../tools/README.md).
- **L1** covers every block state of `26.3`: [`tools/facts-compare`](../tools/README.md). It compares the table the mod would
  build from Horizon's data with what Java says.
- **L2** needs a node cap in place of upstream's time limits, which would otherwise make runs differ: [`tools/search-compare`](../tools/README.md).
- **L3** exists as a first step: [`tools/walk-test`](../tools/README.md) drives a fixed course. A scenario checks arrival,
  time and blocks changed. It does not claim tick-for-tick equality with Java Minecraft, because nothing compares them
  yet.
- **L4** runs on anarchy servers that allow utility clients, such as 6b6t.org, as often as useful, at any stage. Each
  result names its server.

Layers we redesign (world access, threads, cache, processes) have no upstream behaviour to match line by line. They get
ordinary tests for the properties that matter, such as "a cancelled search never installs a path", and then L2 and L3
cover them from outside.

## Two upstream branches, one game

Rules come from upstream's `1.21.4` branch, which has the fixes. Horizon is Minecraft 26.3. So a comparison against
`1.21.4` alone tests rules on the wrong game.

Comparisons therefore run against both when a rule depends on game data:

- **`1.21.4` (JDK 21):** the source of the rules.
- **`26.3` (JDK 25):** upstream on the same game as Horizon, and the reference for L1. The harness is made from the `1.21.4`
  source at run time (`tools/cost-compare/port_26_3.py`). See [Deviations](deviations.md#where-upstreams-two-branches-disagree).

Where the two disagree, the disagreement gets a deviation entry that says which one we follow.

## The ledger

`docs/port-ledger.tsv` has one row per upstream method in scope:

| Status | Meaning |
|---|---|
| `port` | Translated. The Rust function's doc comment opens with the method: `` /// `MovementTraverse.cost`: ... `` |
| `redesign` | Same behaviour on purpose, new structure. Says what replaces it. |
| `drop` | Not needed. Says why. |
| `later` | In scope for a later stage. |
| `missing` | Not decided yet. The default. |

A row also records the upstream commit and a hash of the method's text, and which comparison covers it, if any.

A script checks the ledger against both sides:

- A cited method must exist at the pinned commit.
- A `port` row must have a Rust function citing it.
- A method whose text changed in a newer upstream is listed. That list is the work needed to take a later fix.

This is also how the license's provenance requirement gets met: every file and function says where it came from.

## Deviations

[`docs/deviations.md`](deviations.md) lists each intentional difference: the upstream behaviour, ours, why, and how the comparison is
told to expect it. An exception must be as narrow as the behaviour. It names the function and the condition, never
"ignore this function".

Candidates from the audit are fixes such as the cache's wrong Y key and the inverted water flow test. Fixes with no effect
on which paths get chosen need no decision: fix them and record them. Fixes that change which paths get chosen need the
owner's decision. Two examples are making spawner avoidance work, which is dead upstream, and the Y heuristic, which is
kept as is for now.

A bug that also exists upstream is fixed here, in the port, and recorded as a deviation. This project doesn't maintain
upstream, so nothing is sent there.

The owner has decided to take three such fixes, although they change which paths get chosen:

- Special blocks in the cache being looked up 64 blocks too low.
- The bottom layer caution that never applies.
- The inverted flow test for water at chunk edges.

## Java traps

Each one is a way for a correct-looking translation to give a different answer. The first two have already been seen in
this project.

| Trap | Rule |
|---|---|
| `1 - float` stays a float in Java, then widens | Keep the exact width of each step. Found by injected error. |
| `Math.pow` and friends round differently per library | One small module holds these calls. See below. |
| `int` overflow wraps in Java, panics or wraps in Rust by build | Use explicit `wrapping_*` wherever Java could overflow. |
| `>>`, `/`, `%`, and casts on negative numbers | Translate each case. Never assume. |
| `HashMap` iteration order, tie-breaks in the open set | Reproduce order wherever it can change a result. L2 catches the rest. |
| Null as a meaningful value | Use `Option`. Note each place where this removes an upstream bug. |
| Unsynchronized data shared between threads | Never translate it. The design replaces it. |

### Floating point

Horizon's `jmath` module exists because Rust's float functions don't always round like HotSpot's. A mod's own crate
can't depend on `horizon-core`, so we can't use it from there.

The exposure is small in the pathing, behavior and pathing API code: `Math.pow` is called twice, and the rest are `abs`,
`min`, `max`, `sqrt`, `ceil`, `round` and `toRadians`. So:

- All such calls go through one module.
- The fall-cost table is compared in full, all 4,097 entries, against Java.
- The `pow` result has only been checked on x86-64 Linux. Other platforms stay unverified until they are compared.

## The gate

`tools/gate.py` is what a change must pass before it is committed. It stops at the first failure:

1. Public-tree check: no game files, generated data or private details in the repository.
2. `cargo fmt --check`, then `cargo clippy -D warnings`.
3. Unit tests. These include small hand-written fixtures with no Minecraft data, so they can run anywhere.
4. Ledger check.
5. The comparisons whose inputs changed, at low priority if another build or game is running: Horizon's facts, the rules on
   `26.3` (every difference must be explained by `rules-26.3.patch`), and the rules on `1.21.4`.
6. For a new or changed comparison: three identical runs, and the injected errors.

Injected errors that go uncaught must each be classified, as either a real hole in the test or an error with no possible
effect. A hole stays listed until it is closed.

Known holes today: five mutations remain listed as holes in `tools/cost-compare/known-missed.txt`.

- `diagonal: a/b low mining with falling`: needs an instantly breakable block that is not walkable at a corner's feet, a
  falling block over it, and `breakPenalty` 0 (config 10), all at once.
- `goals: strict direction without step allowed` and `goals: run away from nothing allowed`: upstream throws in the
  constructor, which a golden file cannot record. The unit tests in `goals.rs` cover them; `mutate.py` does not run unit tests.
- `goals: inside strictly below outside`: needs an inside and an outside cell with bit-equal heuristics.

The seven `normal cube` entries were holes until S2. They are now listed as no effect: no state of the six excluded classes
has `shape_full`, in Java or in the table built from Horizon, and `facts-compare` holds that for every state. A shulker box
is full over Horizon's empty level, which is why the table reads `shape_full` as false for blocks with a dynamic shape.

Two were closed in S0: tool choice (the harness now binds the vanilla block tags, so a pickaxe and an empty hand differ)
and the open top trapdoor (added to the block list, also waterlogged, and to the fluid-heavy grids).

### Injected errors on the whole engine (S1)

`mutate.py` now holds 836 injected errors, at least one per branch and early return of `ascend`, `pillar`, `downward`,
`diagonal`, `parkour` and `moves` (wrong offset, swapped arguments, a direction swap), each goal's rules and arithmetic
(wrapping, `java_max`, `java_min`), and the helper predicates (the terms of `fullyPassableBlockState`,
`avoidWalkingInto` and `isBlockNormalCube`, the lava and loaded-chunk rules, the settings lists, `allowBreakAnyway`).
Last full run, against the 10000-grid golden file: 773 caught, 63 missed, 0 not applied. A run of the comparison that
aborts (a failed assertion on the move table) or runs over 300 seconds counts as caught.

Strengthening the harness was tried first for every miss. It added: the offsets and dynamic flags of `Moves` in the `M`
lines, a world border per grid (`W` lines, so placing meets the border), configurations 14 and 15 (frost walker 1 with
sprint, with and without walking on water, magma, both diagonals), clean-diagonal, pond, door-above and falling-stack
scenarios, waterlogged take-off blocks, and fractional `GoalRunAway` distances.

The remaining 63 are listed with a reason in `known-missed.txt`: 58 "no effect" (each with the code or sweep fact that
makes the error unobservable) and 5 holes (above).

The eight errors that were missed on `MovementTraverse` and `MovementDescend` alone were tried again with the other
movements and the stronger grids:

- Still missed, still no effect (reasons now taken from the code and the registry sweep, see the file):
  `through: skull passable`, `walkon: honey ok`, `frost: level ignored`, `replace: tall grass override`,
  `fall: void check`, `fall: flowing water`.
- Now caught: `through: partial fluid maybe` and `water pos: xor to or`.

An independent check used a separate script, not `mutate.py`: 100 operator changes picked at random across the engine
(seed 20261106). One did not build. Of the other 99, 95 were caught (2 of them by an abort and 1 by the time limit) and 4
were missed. All 4 were read and have no effect: an unused return value in `descend`, the case already listed as
`near inside boundary`, and two sign flips in `traverse` that only change the order of the same five directions. So this
sample found no gap in the listed errors. It is a sample, not a proof.

### S1 result

The gate passed on the final engine at upstream `1.21.4` commit `1b8e87c`: 22,312,446 results in 21 families, all
bit-identical, none close, none wrong, with 3 runs giving identical reports. The grids use 191 block states under 16
configurations, and the registry sweep covers all 27,866 states under 2 configurations.

Limits of this result:

- It is L0. The block facts come from Java `1.21.4`, not from Horizon. S2 tests those.
- Twelve injected errors were listed as holes at the time. S2 settled seven of them, and the other five stay open.
- State flags come from the harness. The glue that will set them on Horizon doesn't exist yet.
- `GoalThreeBlocks` is not ported (`later`), because it belongs to `MineProcess`.

Measured effort for S1: about 9 hours of wall-clock time, of which about 6 h 50 min was a workflow of 9 agents using
1.53 million tokens. It produced 2407 lines of engine, and the methods marked `port` in the ledger went from 35 to 114 of
671. Most of the time went to the 836 injected errors (a full run takes about 104 minutes at low priority), not to the
translation. No mismatch was found at any stage, and the one review finding (`f64::min` at the water speed cap, practically
unreachable) was fixed.

This doesn't give a calendar for the rest. S1 was direct translation with an exact oracle, and S3 to S5 are largely
rewrites, with no such oracle from S4 on.

### S2 result

The gate passed at Horizon Nightly `66f3981`, upstream `26.3` commit `25111da` and upstream `1.21.4` commit `1b8e87c`.

**Horizon's facts against Java's (L1).** All 1,786,150 facts of all 35,723 block states of `26.3` are equal, and each state's
id, name and properties match. No exception is listed. The table comes from `mod/glue/state_facts.rs`, which the mod will
use. Of 103 errors injected into that file, 97 are caught, and the other 6 cannot show in vanilla's data (the reasons are in
`tools/facts-compare/mutate.py`).

Two things turned up on the way:

- A bare `Bootstrap` binds no fluid tags, so the harness said that lava is pathfindable. Both harnesses now bind them. No
  engine answer changed, and the `1.21.4` comparison stayed at 0 wrong.
- Over Horizon's empty level a shulker box is full, while upstream's null level throws and counts it as not full. The table
  reads `shape_full` as false for every block with a dynamic shape, which is exactly the set that throws, plus scaffolding
  and powder snow, which are not full anyway.

**Where the facts come from.** Horizon exports them from Java 26.3 when a version is prepared: `gameplay-facts.json`,
`block-behaviours.json`, `block-tags.json` and `mob-data.json`, which holds `isPathfindable`. The three versions checked
(Nightly 2026-11-01, Nightly 2026-11-05 and the test build) have identical exports. If `mob-data.json` is missing or has the
wrong length, `MobData` falls back to guesses, so the mod has to refuse to path in that case.

**Between the game versions.** All facts of the 27,848 states that both versions share are equal. `chain` is gone (18 states)
and `iron_chain` appears, and 192 blocks are new in `26.3`. This list comes from one run of `run_facts.py --versions-against`,
which the gate does not repeat.

**The new blocks.** No rule was tuned on them. The rules treat them as upstream's `26.3` does, and the registry sweep on `26.3`
covers them. The ones the rules let a player walk through are plants, torches, buttons, a pressure plate and openable
doors and gates, and `sulfur_spike` is not passable. Some are refused as a floor although a player could stand on them,
such as copper chests and shelves, so paths avoid them. This was read from the sweep's groups, not played.

**The rules on `26.3`.** With the engine as it is, 22,349 of 22,516,728 results differ from upstream's `26.3`. With
`tools/cost-compare/rules-26.3.patch` applied to a scratch copy, none do, and the gate holds that. So the two branches' rule
differences explain all of them on these worlds. The rules are listed in [deviations.md](deviations.md#where-upstreams-two-branches-disagree).
The `26.3` run has no injected errors and no repeat runs of its own.

Limits of this result:

- It tests our mapping and Horizon's loaders against Java. It says nothing about how Horizon behaves in play, such as its
  physics or its server's rules. L3 and L4 test those.
- Facts are read over an empty level, so a shape that depends on neighbours or a block entity is only checked there.
- Tool strength is checked for a diamond pickaxe and an empty hand, without enchantments.
- `MobData` is not compiled in the tool, which reads the same column of `mob-data.json` itself.
- The walk test still has not been played on `66f3981`.

Measured effort for S2: about 3.5 hours of work, plus a pause. One Sonnet agent ported the harness (15 minutes, 275,000
tokens), and most of the waiting was on the injected errors. Running them on 6 workers took about 39 minutes against 104
one at a time, so 2.7 times faster, with the same counts. A hard-killed run now counts as not applied rather than caught.

### S3 result

The gate passed on upstream `1.21.4` commit `1b8e87c`, with nothing skipped.

**Whole searches against Java (L2).** All 29,446 searches over 903 worlds match, node by node: 236,240 compared lines, 8,609
splice lines and 0 mismatches, and three runs give identical reports. The Java side runs upstream's own classes, and a hash of
the positions taken out of the open set catches a different order of ties. The worlds cover 12 generated kinds and three built by
hand, including a 4064-high shaft and a 70,000-long corridor, under 16 configurations, 11 goal classes and many search settings.

**Injected errors.** Of 491 errors in the search code, 415 are caught and 76 are missed, each listed with a reason in
`tools/search-compare/known-missed.txt`: 57 cannot change an answer, and 19 are real holes. The reasons were read from the source,
not proved. The holes are:

- The timeouts and the failing flag (17), because no clock fires in the comparison. Unit tests catch most of them, but not the
  equality edges of the failure timeout and of the 5-block failing distance.
- The heap's early return for a single element, and the default `isLoaded` of a world without a cache (a unit test catches the
  second).

**Reviews.** Two independent reviews found no difference that changes a path. They found one overflow that could not wrap
(fixed), differences missing from the deviations file (added), and test gaps such as no search crossing y = 0 (closed). The
gate then found what the workflow never built: `facts-compare` and two scratch copies of the workspace needed changes for the new
crate and setting. No engine answer changed.

**Upstream behaviour kept.** `getDistFromStartSq` wraps at about 46,341 blocks, and the cell key aliases at 2048 blocks in y. Both
are now reached by the corpus, and the Rust matches them. See [deviations.md](deviations.md).

Limits of this result:

- Block facts and rules are `1.21.4`'s in this comparison, not Horizon's. S2 links them, but no whole search has run on `26.3`.
- Timeouts and cancellation are not compared with Java, only unit-tested.
- The worlds are small boxes with unloaded air around them, and nothing has been played.
- Rust copies `Movement` values where Java shares objects between paths. That has no effect yet, and S4 must check it once the
  executor keeps state on them.
- The harness calls upstream's `createPathfinder` by reflection, with a small stand-in for the player.

Measured effort for S3: about 6 hours of work. A workflow of 6 agents took 4 h 40 min and 1.77 million tokens, and the last 85 minutes
went to the tool and the gate. Stopping an injected error's run at its first wrong result, and using 12 workers, took the search
errors from 1 h 51 min (479 errors, 6 workers) to 34 minutes, and the cost errors from 39 minutes to about 19, with the same counts
on the cost errors. Verdicts are saved next to the golden file and reused only on byte-identical inputs, so a gate that repeats
them on unchanged code takes about a minute for that step.

### S4 result (walking, sprinting, breaking and placing goto)

S4 as the owner cut it for the first release: `/b goto`, `/b goal`, `/b path`, `/b stop`, `/b eta`, `/b set` (about 80 settings with upstream's names, types and defaults, saved at once to the mod's
folder), `/b help`, `/b trace`, and tab completion of the subcommands and their arguments. The path walks, sprints, **breaks and places blocks**, with the chat closed or open. Parkour and swimming stay
off, as upstream's defaults (`allowParkour`, `allowSwimming`), and their branches fail by name (D16). Every other setting keeps upstream's default, among them `allowBreak` and `allowPlace` (true),
`allowWaterBucketFall` (true) and `maxFallHeightBucket` (20). The work came in two parts: walking and sprinting, then breaking, placing and the commands.

**The gate.** The gate passed for the first part (walking only), with nothing skipped: 29,446 searches and 22,312,446 rule results matched Java, all 1,786,150 block facts matched, and the injected-error counts were
415 of 491 in the search and 773 of 836 in the rules. It then passed again on the finished S4 (2026-11-07): 29,446 searches with 0 mismatches and
identical repeats, the rule comparison with identical repeats, 773 of 836 injected rule errors caught (63 listed) and 415 of 491 search errors caught (76 listed),
nothing not applied. The block facts and the 26.3 rule reports were reused, because their inputs had not changed. Its first run on S4 stopped at one injected
error whose text anchor no longer matched (a new function sat between the fluid check and `avoidBreaking`); only the anchor in `mutate.py` was changed. The ledger
(`tools/ledger.py --upstream upstream/baritone-1.21.4 check`) passes with 763 rows: port 350, redesign 140, drop 82, later 156 and missing 35.

**What breaking and placing do.** A movement breaks what is in its `toBreak` and places what is in its `toPlace`, in upstream's order, as `Movement.prepared` and the movement subclasses do.

- *Breaking.* The tool is chosen as upstream's `ToolSet` does (the fastest hotbar slot for the block, silk touch and `itemSaver` as it reads them, an empty-handed fallback) and is not put back
  when the path ends (D32). The aim is `RotationUtils.reachable` (D25). The left button is held in the game's own input step (`mod/input_head.rs`, `input_return.rs`; with the chat open the screen
  is swapped for `Playing` for that one call, since the step otherwise only aborts a dig), and let go of exactly once when the executor no longer wants it. The break time is the game's: `destroy_progress`, plus
  the delay of about five ticks between two blocks (docs/PLAN.md).
- *Placing.* `attemptToPlaceABlock` picks the face (north, south, east, west, down), aims at the point of that face, selects a throwaway block from `acceptableThrowawayItems`, and presses the right
  button once when the crosshair is on the face. Bridging, the backing-up bridge, the pillar and the ascend over a placed block use it. A water-bucket fall (`MovementFall`) holds the bucket, looks straight down,
  places the water when the crosshair is on the landing block, and picks it up again.
- *Settings that forbid.* Nothing is broken or placed that the settings forbid, also while a movement is not safe to cancel (D28). `allowBreak`, `allowPlace` and the block lists are honoured.
- *Commands.* `goto` (coordinates with `~`, `k`/`m`), `goal` (set, `clear`, `reset`), `path` (start from the goal), `stop`, `eta` (upstream's texts and rounding), `set` (list, modified, reset, toggle, save, load, view,
  set), `help`, `trace`. Tab completes the command names, the settings and their values, and `~` and the crosshair block's coordinates for `goto` and `goal` (D30).

**Walking, in the game (L3).** Single player on Horizon `66f3981`, in a new world named `l3-...`, chat closed and open. Ticks are the sum over the movements executed. The "longest search" of
this run is longer than in the first part's run (the flat walk's one search took 8 ms where the first run had 0.6 ms, and the 1500-block run's longest took 1.9 s against 136 ms). No search or cost code
changed, but the settings did: the first part searched with `allowBreak` off, so every movement that needed a block removed cost `COST_INF` at once; now
`allowBreak` is on, as upstream's default, and the search prices digging through the terrain too, which means far more nodes and a tool lookup per block. That
reading is from the diff, not measured (the load on the machine during the run was not recorded either), and these numbers say nothing about speed against Java.

| Scenario | Chat closed / open | Estimated / actual ticks | Longest search |
|---|---|---|---|
| Flat, 50 blocks | pass / pass | 178.2 / 178 | 8.6 ms |
| Around an L wall | pass / pass | 119.2 / 119 | 7.8 ms |
| 8 steps up, then 8 diagonal blocks | pass / pass | 119.2 / 137 | 8.4 ms |
| Drops of 1, 2 and 3 | pass / pass | 79.9 / 75 | 3.0 ms |
| Natural terrain, 300 blocks | pass / not run | 1344.7 / 2076 | 500.8 ms (2 searches) |
| Sealed room | pass / pass | 71.3 / 71, gave up | 57.9 ms (72 and 73 searches) |
| Wall placed in the way | pass / pass | 162.1 / 161 | 2.5 ms |
| `/b stop` | pass / pass | still after 7 ticks | 2.7 ms |
| Ladder and 1-deep water | pass / pass | 144.2 / 142 | 7.1 ms |
| Floor of bottom slabs | pass / pass | 115.8 / 115 | 6.4 ms |
| 1500 blocks, unloaded chunks | pass / not run | 6567.2 / 9986 | 1.9 s (10 searches) |

Estimates against the game, as in the first part: on land the costs hold (Traverse 1.05 and Diagonal 0.95 on the natural walk), and moves touching deep water take about 2.2 times their estimate
(130 Traverse moves: 602 estimated, 1331 actual), which is U8 and the owner's call. Ascend ran 1.16 times its estimate in the first part's natural walk and the steps scenario took 137 ticks against 119; this is
not explained yet.

**Breaking and placing, in the game (L3 `b01` to `b09`).** Each scenario was run with the chat closed and open and passed both times. The checks per run: every changed cell of the watch box was in a
segment's `toBreak` or `toPlace`, every cell of the last path's plan was done, nothing else changed; the broken and placed counts, the stack counts and the tool's durability are exact; no death and no health
loss; actual ticks within 0.4 to 2.5 times the estimate (plus 30), a bound of my choosing. Estimated / actual movement ticks, chat closed / chat open:

| Scenario | What happened | Estimated / actual ticks |
|---|---|---|
| `b01` stone wall, iron pickaxe | 1 block broken (the top block of the 2-high wall), 1 durability lost, then an ascend | 61.1 / 62 and 67 |
| `b02` dirt wall, bare hand | 1 block broken | 65.0 / 75, 68.6 / 74 |
| `b03` 3-wide bridge | 3 cobblestone placed | 80.0 / 63 and 63 |
| `b04` pillar, ledge variant | 5 placed, then an ascend | 51.0 / 58 and 58 |
| `b04` pillar, shaft variant | 6 placed | 49.0 / 50 and 50 |
| `b05` dig down 5 | 5 broken, 5 durability lost | 75.6 / 76 and 76 |
| `b06` wall, `allowBreak` on | 2 broken | 62.8 / 71 and 71 |
| `b06` wall, `allowBreak` off | routed around, 0 changes | 86.0 / 85 and 85 |
| `b07` 12-block drop with a water bucket | full health, bucket back in the hotbar, no water left | 47.9 / 49 and 49 |
| `b08` `/b stop` at tick 70 of a 150-tick hand dig | the block was still there 230 ticks later | not applicable |
| `b09` stone by hand (the control of `b08`) | 1 block broken | 189.3 / 189 and 189 |

Notes. Every judged run in the final suite passed: 42 of 42 (20 walking, 22 breaking and placing), in one run in a new world, with one game started and closed by its PID; the report is
`reports/l3-66f3981.json`. The first guesses for `b01`, `b02` and the pillar were wrong, because upstream's plan breaks only the top block of a 2-high wall and ends the pillar with an ascend; the scenarios were
corrected, not the checks loosened, and `b04` has two variants for it. `s02`, `s06` and `s07` set `allowBreak false` for their run, because under upstream's default the path would dig through their walls and they exist to test
walking; the setting is put back afterwards. The bucket fall is allowed by upstream's defaults. `b08` infers that the dig stopped from the block surviving; it does not read the server's dig state.
The hotbar slot is not restored after the path (upstream never does either, D32), so `run.py` reports start and end slot without judging them (`b01` starts on slot 5 and ends on slot 0, where the pickaxe is).

**The probe (`tools/break-place-probe`).** Before the glue, a separate test mod measured in the real game what a mod can do: switch the slot, break (dirt by hand 16 ticks, stone with an iron pickaxe 14, stone by hand
157), place against a floor and under the feet while jumping, and use a water bucket, with the chat closed and open. It found that with the chat open the game's input step does nothing but abort a dig, so the mod swaps the screen
for that one call; the controls (the same buttons without the swap) did nothing. Details and the break-time formula are in docs/PLAN.md ("Breaking and placing").

**Reviews and fixes.** In the first part, two independent reviews (the executor against upstream, and safety) found no difference that changes a path; fixed after them: the mouse sensitivity was never read (D17), a panicking search
retried silently (D23), a block id missing from the table could panic, `tools/l3/run.py` could close a game it did not start, `/b` typed while the mod was off went on to the server, a failed thread start would have crashed the game.
In the second part, an independent review of the break and place code against upstream (with scratch tests that fail where the port differs) and a safety review of the glue found the following. Fixed, each with a test:

- `MovementFall.willPlaceBucket` was approximated by `hasWaterBucket && drop > maxFallHeightNoWater`; on a ladder shaft the executor placed water where the search had planned none. It now runs `dynamicFallCost` as upstream
  (`exec/review_tests.rs` R1).
- The executor's `canPlaceAgainst` had no world-border check, where upstream's `BlockStateInterface` has one (R2).
- The incidental left clicks (the traverse's "something is in the way", the pillar's block at the feet) ignored `blocksToDisallowBreaking`; they are now not pressed while the crosshair is on a block that must never be broken (D28, R4).
  Upstream presses them whatever is under the crosshair (U11).
- `pauseMiningForFallingBlocks` had no effect in the game, because the glue never answered `falling_block_near`; it now does, from the game's falling blocks (D31; unit-tested box test, not tried with sand in the game).
- A mouse button could stay held after `/b stop` when the stop ran twice before the next input step, or when the step did not run (inventory screen): the let-go now stays pending until a step delivered it (`engine/src/buttons.rs`, five tests).
  The input hook's `RETURN` now puts the chat back only if the step left the screen as `Playing`.
- `set reset` alone now prints upstream's "Not enough arguments" error after the three explanation lines, and a line of `settings.txt` is judged by upstream's pattern (D33).
- A test run could wipe an existing world: `tools/l3/run.py` now refuses a world name that does not start with `l3-` or already exists, and the mod's test triggers (`BARITONE_RS_L3_SPEC`, `BARITONE_RS_SCENARIO_DIR`) refuse any single-player
  world whose name does not start with `l3-` (`tools/test_l3.py` tests the name check; the mod's refusal was read and built, and was not provoked in a game).

Recorded, not changed: the pillar digs the block at its feet with the throwaway block in hand (U14, `exec/review_tests.rs` R3, upstream's behaviour), a falling block landing in the head cell ends the movement as `InsideWall` instead of
mining out (D31), `/b` and `/baritone` are taken from servers, `/b trace` logs exact coordinates whatever `censorCoordinates` says, mod chat lines (with the goal coordinates) go to the game log, `set load` completes no file names, and settings are
not range-checked (a huge `primaryTimeoutMS` on an unreachable goal makes the search run as long as asked, as upstream; not run). Open from the first part: the logs of searches and snapshots grow slowly outside the test code, and "Arrived." can print while the player is still falling.

**Final checks of this work.** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (200 engine tests), `python3 -m unittest discover -s tools -q` (95 tests), the
ledger check and `tools/check_public_tree.py` with the deny list all pass. The Java comparisons (L0 to L2) and the gate were not rerun by that agent; the gate was run afterwards and passed (see "The gate" above).

**Limits of this result.**

- Single player only, so no anticheat, no real server lag and no packet sequencing under latency. That is S5. A server's own break timing was not measured.
- Commands went straight to the mod's command function, not through the chat box, so a typed `/b goto` was not run by the tests; the tab completion and the chat box were smoke-tested by hand through `horizon-ctl` in the
  glue step, not by L3. Real keys during a path have not been run, nor a real disconnect.
- Not covered: creative mode, sneaking, efficiency, haste or fatigue, a tool that breaks mid-dig, falling blocks (sand and gravel) in the game, picking up drops, the Nether beyond stopping when the dimension changes, water with Depth Strider
  or Frost Walker (D22), and a closed door on the only route (a loop of failing and searching, D15).
- The natural walks ran with the chat closed only, each on the terrain of its own new world. The executor has not been compared with Java tick by tick; the reviews compared it by reading.
- The timing bounds (0.4 to 2.5 times the estimate) are my own choice; the actual ratios in the breaking and placing scenarios ran 0.79 to 1.15.
- The unit and property tests use fixed-seed random runs and a simple simulator, which is not evidence of real physics.

**Left out on purpose.** Parkour and swimming (off by upstream's defaults, branches fail by name, D16); `goto <block>` and the other processes (mine, follow, explore, build, farm, elytra); using doors, buttons and levers beyond what D15 says (`allowInventory`); Depth Strider and Frost Walker from the boots (D22); the stored settings that nothing reads (`renderPath`, `allowInventory`, D29).

Measured effort: the first part took a workflow of 7 agents, 3 h 39 min and 1.96 million tokens, and its last agent ended before writing this section. The second part took a workflow of 7 agents (probe, engine, glue, scenarios, two reviews, fixes), 2 h 47 min and 2.04 million tokens, then two gate runs by hand.

### S5, first session (6b6t.org, 2026-11-07)

L4 on **6b6t.org** only, with the owner's account, the S4 build (`3218596`) and Horizon `66f3981`. Commands were typed through the control
socket (`horizon-ctl chat`), with the chat closed. Each run sampled the player's position every 0.1 s and counted any jump of more than 3
blocks between samples as a server setback. Gear was never at risk: the account had an empty inventory and no armour.

**Main world: no movement possible, as upstream.** Every spawn, and the account's saved position, is on an obsidian floor at y 319, the
top of the Overworld, with the ground about 250 blocks below. `/b goto` 50 blocks away on that floor answered "No path" at once,
because upstream's search refuses any move whose feet would be above `maxY` (U15). The floor's few holes are 250-block drops, which
upstream never takes without a water bucket. The mod was set to never break ender chests (`blocksToDisallowBreaking`) before any run there.

**Lobby (adventure mode): walking works on the real server.** 10 runs, about 584 blocks walked over 1,551 samples, with **no setback**:
- `/b goto` into the lobby's portal arrived both times it was used, and moved the account to the main world.
- 4 runs arrived: 41.6 blocks in 7.7 s (sprinting, about 5.4 blocks/s), 73.7 blocks in 14.4 s, 85.5 blocks in 16.7 s, and the portal walk. They
  included jumps up steps, diagonals and slab floors.
- 2 runs stood still in U16's loop (an Ascend through dirt that adventure mode can't break, retried every 148 ticks) until `/b set allowBreak false`.
- With `allowBreak` off, 3 runs ended with "No path" after walking 0, 29 and 189 blocks: a goal outside the lobby's walkable area and a
  pit at y 57 with no way out without breaking. These are expected answers, not faults found.

**Port bug found and fixed:** a new `/b goto` announced the previous goal's "the path was cancelled: TimedOut" again, because the engine
kept the last cancel reason across goals. `Pathing::goto` and `path` now clear it (test `a_new_goal_forgets_the_last_goals_cancel`, which
fails without the fix). The fix is not yet in a build that ran on the server.

**Not tested yet on a server:** breaking and placing (adventure mode and the empty inventory allowed neither), the chat open (`horizon-ctl
key t` does not open it, so all runs had it closed), small setbacks under 3 blocks (the method can't see them), any anticheat kick
(none happened, but the runs were short), and another server.

**After the owner's decision on U15:** `allowWalkOnBuildLimit` (D34, on by default) was built (`native-66f3981-08329dda4b30`) and run on
6b6t's roof: `/b goto` 50 blocks away arrived, 50.7 blocks in 9.2 s, with no setback. The repeated-cancel fix was in that build too.

**Free look (`freeLook`, D35, 2026-11-07).** `freeLook`, `blockFreeLook`, `antiCheatCompatibility`, `smoothLook` and `smoothLookTicks` are read now, with upstream's
defaults (so by default a walk no longer turns the camera, and a dig or place still does). The mod puts the executor's rotation in at the start of the game's input
step (the existing pinned hook; no hook was added) and puts the player's view back right after the player's tick, so the picks for the clicks, the movement and the
packet carry the aim and no frame shows it. Ran, single player, one game window (build `native-66f3981-f4c75a4a5b49`): with the camera set to 150/10, `freeLook` on, a
50-block walk east left the client view at 150/10 for the whole walk while the server's rotation was -90 and the player arrived (1030.5, 100, 1000.5); `freeLook` off turned the
view to 270 as before. With `blockFreeLook` on, digging 8 stone blocks through a wall and bridging a 4-block gap with cobblestone both finished with the view still at
150/10 and the server looking at the blocks (yaw -90/90, pitch 26/80); with it off (upstream's default) the view turned to the block (270/26). L3 on that build: 42 of 42
scenarios passed. Limits: single player only; the 20 Hz server rotation was sampled through the console at about 8 Hz, not read from packets; `smoothLook`, `antiCheatCompatibility`
off and the elytra and swimming branches were not run in the game (the first two only in unit tests); a real mouse turn during a path was not tried here (it ran later on 6b6t, below); the third-person model sees the aim for one tick; the aim
still takes effect one tick after the mod decides it, as before.

**S5, second session (6b6t.org, 2026-11-07, build with free look, `d0b08a4`).** All runs on the main world's roof at y 319, `freeLook` on
(the default), commands typed through the control socket, the position sampled every 0.1 s:
- **500 blocks:** arrived in 89.7 s (5.6 blocks/s, sprinting), no setback. The owner moved the real mouse during this run: the view stayed
  theirs and the walk went on, so a real mouse turn during a path has now run, on a server.
- **1500 blocks:** arrived in 333.3 s, no setback. Sprinting stopped when the hunger bar reached 6 (the game forbids sprinting below 7):
  5.59 blocks/s, then 4.32 blocks/s walking for the rest, as the game requires.
- **Chat open:** the chat screen stayed open for the whole of a 100-block run (207 of 208 samples), and the player walked 97.9 blocks
  with no setback before another player killed it 2 blocks from the goal (health 20 to 5 to 0 in half a second, in the air). Moving with
  the chat open now has a server result too.
- No kick, no anticheat message, and no setback in any run. Deaths on 6b6t in this and the owner's sessions were other players and
  the owner's own `/kill`, not the mod.
- **Still not run on a server:** breaking and placing. Every account starts on the roof with an empty inventory, and `/kit` answers
  nothing on 6b6t, so there was nothing to place and no tool.

**S5 closed (owner's decision, 2026-11-07)** with one server only (6b6t.org) and with breaking and placing on a server **not run**: they
rest on the single-player scenarios (L3) alone. Both stay open for a later L4 session.

## Reports

A comparison writes a small JSON report: the upstream commit, the game version, the Java and Rust versions, hashes of the
sources it ran, and its counts. A previous success report is deleted before a new run starts, so that a failed run can't
leave an old pass behind.

Reports hold metadata only and may be committed. Generated data stays local.

## What must never enter the repository

- Minecraft code, data or generated golden files.
- Anything from Sunset, closed clients or Eris.
- The elytra pathfinder's terrain generator files, which upstream describes as coming from Minecraft.
- Tokens, account names, private hostnames or home paths.

### Keeping Horizon clean

Horizon's contribution rules put LGPL code off limits, even to read. So this project's code must never be offered to
Horizon. When we need a hook there, we ask for it by describing the need, and we don't send code from here.

In the other direction, we read Horizon to learn its API and take ideas. We don't copy its code, although its MIT license
would allow that.

## Stages

Each stage ends with a report to the owner, who decides whether to continue.

| Stage | Work | Done when |
|---|---|---|
| **S0** | Ledger, deviations, gate and reports. Close the tool and trapdoor holes. Try the `26.3` comparison. | The gate runs on today's two functions |
| **S1** | The other five movement costs, predicates and goals | L0 on all of them, on all block states of the corpus |
| **S2** | First move to the latest Horizon Nightly, then compare Horizon's facts against Java's | L1 on all block states, with each difference explained |
| **S3** | A*, partial paths and segments | L2 on a corpus of generated worlds |
| **S4** | World access, threads, executor and glue | Property tests, then L3 scenarios |
| **S5** | Full server runs. Earlier stages can also test on servers. | L4 reports, per server |

S2 comes before S3 on purpose. If Horizon's facts can't support the rules, that needs to be known before building more on
top of them.

## Testing rules in the game

- Use a separate instance and world, never one with other mods or the owner's own worlds.
- Run one full client build at a time, and check that no game or build is already running.
- Use a normal window on the desktop. Never nest a compositor or add outputs.
- Close the game when done.

## Open questions

- Whether the checks in `tools/gate.py` should later also run on public CI. Without Minecraft there, CI could only run
  steps 1 to 4.
- Whether Horizon's pinned revision stays at `4aa6854`, or moves to one with `native_movement_input`.
- How to keep walking with the chat open, which is a requirement: players use Baritone while they talk or type commands.
  At `4aa6854`, the game ignores movement keys while a screen is open, so `tools/walk-test` stops when the chat opens. L3
  needs a scenario for this.
