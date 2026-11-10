# Porting safety

How baritone-rs shows that the port behaves like Baritone, and where it knowingly doesn't. The ideas come from how
Horizon checks itself against Minecraft ([parity workflow](https://github.com/horizon-rust/horizon/blob/main/docs/parity-workflow.md),
[port ledger](https://github.com/horizon-rust/horizon/blob/main/docs/port-ledger.md)), scaled down to a small project
on one machine. Only the ideas: our tools are written here, without Horizon's code.

This file is the method. What each stage ran and found is in [RESULTS.md](RESULTS.md), and every difference from upstream is in
[deviations.md](deviations.md).

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
- **L3** is a suite of fixed scenarios played in the client: [`tools/l3`](../tools/README.md#l3-scenarios). A scenario checks arrival,
  time and blocks changed, with the chat closed and open. It does not claim tick-for-tick equality with Java Minecraft, because
  nothing compares them.
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
on which paths get chosen need no decision: fix them and record them. Fixes that change which paths get chosen are decided one by one, and the entry records the date and the reason. Two examples are making spawner avoidance work, which is dead upstream, and the Y heuristic, which is
kept as is for now.

A bug that also exists upstream is either fixed here, in the port, and recorded as a deviation (a `D` entry), or kept and
recorded as upstream behaviour (a `U` entry). This project doesn't maintain upstream, so nothing is sent there.

Three such fixes are taken, although they change which paths get chosen (D1 to D3):

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

Results of each stage, with what ran and what did not, are in [RESULTS.md](RESULTS.md).

## Reports

A comparison writes a small JSON report: the upstream commit, the game version, the Java and Rust versions, hashes of the
sources it ran, and its counts. A previous success report is deleted before a new run starts, so that a failed run can't
leave an old pass behind.

Reports hold metadata only and may be committed. Generated data stays local.

## What must never enter the repository

- Minecraft code, data or generated golden files.
- Anything from Sunset, closed clients or other private projects.
- Code from nether-pathfinder or from its Java port in upstream. Elytra flight is written new here (see [CREDITS.md](../CREDITS.md#elytra)).
- Tokens, account names, private hostnames or home paths.

### Keeping Horizon clean

Horizon's contribution rules put LGPL code off limits, even to read. So this project's code must never be offered to
Horizon. When we need a hook there, we ask for it by describing the need, and we don't send code from here.

In the other direction, we read Horizon to learn its API and take ideas. We don't copy its code, although its MIT license
would allow that.

## Stages

`S` stages built the port up to the first release; `R` stages add upstream's other features. A stage is done when its evidence is in
[RESULTS.md](RESULTS.md).

| Stage | Work | Done when |
|---|---|---|
| **S0** | Ledger, deviations, gate and reports. Close the tool and trapdoor holes. Try the `26.3` comparison. | The gate runs on today's two functions |
| **S1** | The other five movement costs, predicates and goals | L0 on all of them, on all block states of the corpus |
| **S2** | First move to the latest Horizon Nightly, then compare Horizon's facts against Java's | L1 on all block states, with each difference explained |
| **S3** | A*, partial paths and segments | L2 on a corpus of generated worlds |
| **S4** | World access, threads, executor and glue | Property tests, then L3 scenarios |
| **S5** | Full server runs. Earlier stages can also test on servers. | L4 reports, per server |
| **R2** | `mine`, `goto <block>`, `follow`, the persistent chunk cache, `explore` | Unit tests, L3 scenarios, the gate |
| **R3** | `build` (ported), then elytra flight (written new) | The same, plus flight traces from the game for elytra |
| **R4** | `farm` (written new: upstream's farmer is weak) | The same |

S0 to S5 and R2 are done. S5 ran on one server (6b6t.org), without breaking or placing there. R3 is in progress: `build` is ported, elytra is not started.

S2 comes before S3 on purpose. If Horizon's facts can't support the rules, that needs to be known before building more on
top of them.

## Testing rules in the game

- Use a separate instance and world, never one with other mods or a player's own worlds.
- Run one full client build at a time, and check that no game or build is already running.
- Use a normal window on the desktop. Never nest a compositor or add outputs.
- Close the game when done.

## Open questions

- Whether the checks in `tools/gate.py` should later also run on public CI. Without Minecraft there, CI could only run
  steps 1 to 4.
