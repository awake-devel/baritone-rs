# Deviations

Each place where this port differs from upstream on purpose. The rules are in
[PORTING-SAFETY.md](PORTING-SAFETY.md#deviations). Upstream lines are cited at the `1.21.4` commit `1b8e87c`.

Entries 1 to 4 are decided and wait for the code they apply to. Entry 5 is active and has no effect on any real search. Entries 6 to 10 are active (S3) and change no chosen path that a game can reach.

## How exceptions work today

They don't. `tools/cost-compare` has no exception mechanism. Its comparison against `1.21.4` is exact and no deviation
touches a compared result, so nothing needs to be excused.

When a deviation does change a compared function, an exception has to name the function and the exact condition. It
never says "ignore this function". A difference with no entry here must still fail the check.

## Entries

Fields: upstream, ours, why, changes chosen paths, decided by, expectation in the comparison.

### D1. Cached special blocks looked up 64 blocks too low

- **Status:** decided, not yet ported.
- **Upstream:** `ChunkPacker.pack` stores each special block with its world Y (`y + minY`, line 92).
  `CachedChunk.setSpecial` turns that into a key with `getPositionIndex` (line 191). `CachedRegion.getBlock` passes a
  Y relative to the dimension minimum (`y - dimension.minY()`, line 80), and `CachedChunk.getBlock` looks the key up
  with it (lines 197 and 210 to 215). In the overworld, where `minY` is -64, the lookup is 64 blocks too low, so special
  blocks are found at the wrong height or not at all.
- **Ours:** store and look up with the same Y.
- **Why:** the lookup is wrong. Special blocks (chests, spawners, and so on) come back as their plain pathing type.
- **Changes chosen paths:** yes.
- **Decided by:** the owner.
- **Comparison:** none today. It will need a test with a special block at a known height.

### D2. The bottom layer caution never applies

- **Status:** decided, not yet ported.
- **Upstream:** `CachedChunk.getBlock` returns obsidian instead of stone for solid blocks with `y < -59` in a natural
  dimension (line 222). That `y` is already relative to the dimension minimum, so it is never below -59.
  The intended caution near bedrock never happens.
- **Ours:** compare the world Y.
- **Why:** the check was meant to discourage breaking blocks near the bottom, where bedrock may be.
- **Changes chosen paths:** yes, near the bottom layers.
- **Decided by:** the owner.
- **Comparison:** none today.

### D3. Inverted flow test for water at chunk edges

- **Status:** decided, not yet ported.
- **Upstream:** `ChunkPacker.getPathingBlockType` handles water on the chunk border (lines 141 to 147). If the water
  has a horizontal flow it returns `WATER`, otherwise `AVOID`. The comment above the function says flowing water should
  be avoided, so the test is the wrong way round.
- **Ours:** flowing water at the edge is `AVOID`, still water is `WATER`, as for water inside the chunk.
- **Why:** water at the edge of a chunk should be treated like water elsewhere.
- **Changes chosen paths:** yes, around water at chunk edges.
- **Decided by:** the owner.
- **Comparison:** none today.

### D4. Keep region files that fail to load

- **Status:** decided, not yet ported.
- **Upstream:** `CachedRegion.load` catches any exception, prints it and goes on (lines 192 and 307). The region stays
  empty and a later save overwrites the file that could not be read.
- **Ours:** a file that fails to load is kept as is, and the failure is reported.
- **Why:** one bad read should not destroy cached data.
- **Changes chosen paths:** no. It needs no decision.
- **Decided by:** nobody needed. Fixes with no effect on paths are made and recorded.
- **Comparison:** none. Ordinary tests for the cache layer.

### D5. Goal search box that ends at the largest integer

- **Status:** active, in `Goal::heuristic_inside`.
- **Upstream:** `GoalNear.heuristic()` and `GoalRunAway.heuristic()` loop with `x++` on an `int`. If the box ends at
  `Integer.MAX_VALUE`, the counter wraps and the loop never ends.
- **Ours:** inclusive ranges, which stop at the end of the box.
- **Why:** a hang helps nobody, and Rust's ranges do this without extra code.
- **Changes chosen paths:** no. It needs coordinates about 70 times beyond the world border.
- **Decided by:** nobody needed. Fixes with no effect on paths are made and recorded.
- **Comparison:** none. The comparison does not call `heuristic()` on such goals, because upstream would not return.

### D6. The search stops on a node cap, a given clock and a flag

- **Status:** active, in `search::astar`.
- **Upstream:** `AStarPathFinder.calculate0` reads `System.currentTimeMillis` and stops at the primary or failure timeout,
  and at `cancelRequested`. `AbstractNodeCostSearch.calculate` sets `cancelRequested = false` first, so a `cancel()`
  that arrives before the call is lost.
- **Ours:** the caller gives a clock (a closure with milliseconds) and the same checks run every 64 nodes with the same
  `failing` flag. A node cap is added: the N-th node is taken out and fully expanded, then the loop ends the way a
  timeout ends it, in `bestSoFar`. The cancel flag is the caller's `AtomicBool`; `calculate` does not reset it.
- **Why:** runs have to be comparable (L2), and tests need a fake clock. A cancel must not get lost.
- **Changes chosen paths:** no. With no cap and a real clock the loop is upstream's. A search that ends on the cap ends at a
  node count, where upstream ends at a time.
- **Decided by:** the stage plan.
- **Comparison:** L2 (`tools/search-compare`) runs with a node cap and compares what both sides have after it. Java gets the same cap
  without a change to upstream: its goal is wrapped, and on the N-th `isInGoal` call (one per node taken out of the open set) the wrapper
  calls `cancel()`. The loop then ends after that node has been expanded; the harness takes `bestSoFar(false, numNodes)` and runs
  `postProcess`, `cutoffAtLoadedChunks` and `staticCutoff` itself, as `calculate` does. Searches without a cap go through `calculate`.
  The time limits never fire on either side, so the timeout logic itself is not part of L2 (unit tests cover it).

### D7. Failures are values, and one path type stands for three

- **Status:** active, in `search`.
- **Upstream:** an implausible cost, a wrong destination, a NaN heuristic, a path checked twice, a bad `subList` and a failed
  sanity check throw. `calculate` turns the exception into `Type.EXCEPTION`. `Path`, `CutoffPath` and `SplicedPath` are three
  classes. `Path.<init>` loops forever if the `previous` links form a cycle.
- **Ours:** `SearchError` values, with the same conditions and upstream's messages for the sanity check. One `Path` struct.
  A cycle of `previous` is an error. A second `calculate` on a finder returns `Exception` (`SearchError::Reused`) as a
  value, where upstream throws `IllegalStateException("Path finder cannot be reused!")` before its try block
  (`AbstractNodeCostSearch.java:122-124`), so the caller of upstream sees the exception and never a result.
- **Why:** the engine does not throw, and the three classes differ only in how they are made.
- **Changes chosen paths:** no.
- **Decided by:** nobody needed.
- **Comparison:** none for the causes of the errors in upstream, since `calculate` swallows them (it logs and answers
  `EXCEPTION`). `tools/search-compare` compares that a search threw, and checks on the Rust side that the cause is the one the
  settings explain (a NaN heuristic, or a `pathCutoffFactor` above 1). The unit tests make each error.

### D8. Costs of a path are copied when it is made

- **Status:** active, in `Path::from_nodes`.
- **Upstream:** `Path` keeps the `PathNode` objects and reads their `cost` in `assembleMovements`, later. A search that
  went on after `bestPathSoFar` would change them.
- **Ours:** the costs are copied when the path is made.
- **Why:** nodes are in an arena that the path does not borrow.
- **Changes chosen paths:** no. `postProcess` is only called on the finished search.
- **Decided by:** nobody needed.
- **Comparison:** none.

### D9. The first cell key may equal the key that means "no cell yet"

- **Status:** active, in `search::node`. No effect in the game.
- **Upstream:** `AbstractNodeCostSearch` starts with `prevCellKey = Long.MIN_VALUE` and `prevCell = null`
  (line 57). If the first lookup of a search has the key `Long.MIN_VALUE`, `cellKey == prevCellKey` is true and
  `cell = prevCell` is null, so `cell[index]` throws a `NullPointerException` (lines 199-200) and `calculate` answers `EXCEPTION`.
  The key is `Long.MIN_VALUE` for x from -2147483648 to -2147483633, z from 0 to 15 and y from 0 to 7 (the cell
  of `x >> 4 = -134217728`, whose 28 low bits are `0x8000000`, shifted into the sign bit).
- **Ours:** the cells are a map with no sentinel, so the search runs normally there.
- **Why:** the position is 70 times beyond the world border (30 million blocks), so no game reaches it. The same position
  at x + 16 or y = 9 gives another key and matched in a trial run.
- **Changes chosen paths:** no, for every position a game can have.
- **Decided by:** nobody needed.
- **Comparison:** none. A trial run through upstream's classes, outside the committed corpus, with a start at `(-2147483647, 1, 1)`
  found exactly this difference in 2 of 2,825 searches and no other.

### D10. `slowPath` is not read

- **Status:** active, in `search::astar`.
- **Upstream:** `calculate0` reads the debug setting `slowPath` and, when it is on, uses `slowPathTimeoutMS` for both
  timeouts and logs a line.
- **Ours:** the caller gives both timeouts, so a caller that wants the slow path passes `slowPathTimeoutMS` itself. The
  setting is not read.
- **Why:** it is a debugging aid; the engine has no settings object to read at that point and no log.
- **Changes chosen paths:** no. A timeout acts only on a search that runs out of time.
- **Decided by:** nobody needed.
- **Comparison:** none (the time limits never fire in L2).

### Upstream behaviour kept, found in S3 (reported, not fixed)

The port does what upstream does here; a change would change paths, so it is the owner's call.

- **U1. `getDistFromStartSq` is an `int` and wraps.** A best node 46,341 to 65,535 blocks from the start has a squared
  distance that wraps to a value below 25 or negative, so the partial path is refused as "no path". In a one-wide corridor
  both sides answered a segment at a cap of 46,000 nodes and a failure at caps from 46,343 to 65,536. The committed corpus has a
  corridor of 70,000 blocks along z (caps 46,000, 46,400 and 65,540) for this.
- **U2. The y part of the cell key repeats every 2048 blocks** (`(y >> 3) & 0xFF`, bug 4 in `tools/search-compare/known-missed.txt`). Two nodes
  2048 blocks apart in y, with the same x and z, share a slot, so a search in a column higher than 2048 stops at 2048 nodes and
  returns a segment. The committed corpus has a one-wide shaft 4064 high for this. A vanilla world is at most 384 high.

## Where upstream's two branches disagree

Measured with `tools/cost-compare --port 26.3` on every movement family, the goals, the position predicates and the whole
block-state registry (35,723 states of 26.3): 10,000 generated grids, one run, against upstream `26.3` (commit `25111da`).
The Rust translation matches `1.21.4` on the same grids with 0 wrong and 0 close, so every difference below comes from
the two Java branches (or from the game, see "Game differences").

Unpatched, the engine differs from `26.3` in 22,349 results: downward 1,137, pillar 399, traverse 2,924, ascend 1,134,
descend 1,228, diagonal 4,401, parkour 323, predicates 10,181, registry sweep 586 (all in `canWalkOnBlockState`), and in
the goals `GoalRunAway` 34 and `GoalComposite` 2 (a composite holding a run-away goal). Every other goal and the fall
table are identical.

`tools/cost-compare/rules-26.3.patch` makes a scratch copy of the engine behave like `26.3`, one hunk group per rule below.
With the whole patch every family is back to 0 wrong and 0 close, so these rules explain every difference found. Nothing
is left over. To measure a rule alone, the table has the results that differ when only that rule's hunks are left out of the
patch (the engine then differs from `26.3` in that rule alone). The counts overlap, so they do not add up to 22,349: one
result can need two rules.

| Rule | Where | Results that differ when it alone is left out | We follow | Status |
|---|---|---|---|---|
| Swimming | `1.21.4` adds `allowSwimming` and `SWIM_ONE_BLOCK_COST`, and caps the water walk speed with it (`CalculationContext`). `26.3` has neither. | 1,435: traverse 1,284, diagonal 151. Needs the swimming and sprint configs. | `1.21.4` | Decided by the owner |
| `isWaterloggedSolidTop` in `canWalkOnBlockState` | Only in `1.21.4`: waterlogged top slabs, top stairs, top trapdoors and scaffolding are not water to walk on. `26.3` treats any water state as maybe. | 15,746: predicates 10,181, registry sweep 586, traverse 1,673, ascend 1,134, downward 1,137, descend 471, diagonal 317, parkour 235, pillar 12. Most of the other `isWaterloggedSolidTop` results ride on this one. | `1.21.4` | Decided by the owner |
| `isWaterloggedSolidTop` in `MovementDescend` | The water landing branch skips a waterlogged solid top (`1.21.4`). | 471 descend | `1.21.4` | Decided by the owner |
| `isWaterloggedSolidTop` in `MovementParkour` | The take-off test with `assumeWalkOnWater` skips a waterlogged solid top (`1.21.4`). | 88 parkour | `1.21.4` | Decided by the owner |
| `isWaterloggedSolidTop` in `MovementPillar` | The last condition: `isLiquid(fromDown) && !isWaterloggedSolidTop(fromDown) && assumeWalkOnWater` in `1.21.4`, without the middle term in `26.3`. | 387 pillar | `1.21.4` | Decided by the owner |
| `jumpPenalty` on the diagonal ascend | `1.21.4` adds `context.jumpPenalty` to a diagonal ascend. `26.3` does not. | 3,351 diagonal | `1.21.4` | Decided by the owner |
| `isBlockingDoor` in `MovementDiagonal` | Only in `1.21.4`: an early return, the `ALow`/`BLow` test, and `+1` on the mining time of the two side cells. | 565 diagonal | `1.21.4` | Decided by the owner |
| Magma corner in `MovementDiagonal` | `1.21.4` tests `cuttingOver2` for magma at the second corner. `26.3` tests `cuttingOver1` twice, so a magma block at that corner is not refused. This looks like a bug in `26.3`. | 108 diagonal | `1.21.4` | Decided by the owner |
| `GoalRunAway.heuristic()` box | `26.3` has `maxX = Math.max(minX, ...)` (the same for Y and Z), so with several points its scan box is too small and the heuristic is wrong. `1.21.4` has `Math.max(maxX, ...)`. This is a bug in `26.3`. | 36: `GoalRunAway` 34 (14 wrong, 20 within 1e-9), `GoalComposite` 2 | `1.21.4` | Decided by the owner |
| Doors in `canWalkThroughBlockState` | `1.21.4` asks `DoorBlock.isWoodenDoor`. `26.3` accepts every door and gate except `Blocks.IRON_DOOR`. | 0 (see below) | `1.21.4` | Decided by the owner |

Reasoning:

- All the rules but the door rule are rules that `1.21.4` has and `26.3` lacks, which fits `26.3` being the branch that
  lags behind. The magma corner and `GoalRunAway` are bugs fixed in `1.21.4`.
- For `isWaterloggedSolidTop`, the fact it relies on (whether the face is sturdy) is the same on both versions, so the rule is
  not a game difference. The four use sites are separate lines of upstream's code and are measured separately.
- Swimming is a setting rather than a fact about blocks, so the data cannot settle it.
- Doors: in the game data of `26.3`, `DoorBlock.isWoodenDoor` is false for the iron door only (copper doors can be opened by
  hand, and the 1,344 other door states are wooden or copper), so the two rules give the same answer for every one of the
  1,408 door states in the registry, and the palette has oak, iron and copper doors. The earlier note that they differ for
  copper doors was wrong. A modded door with another block set type is the only case where they would differ, and the
  harness cannot test that.
- `GoalAxis.equals` (a null check) and `GoalRunAway.hashCode` (`26.3` unboxes a null `maintainY` and throws) also differ.
  Neither changes a cost or a heuristic, and the harness calls neither, so they have no hunk and no count.
- The other differences read in the two branches (`canPlaceAgainst` and `avoidBreaking` through precomputed flags, `minY`
  and `maxY`, the mining cache, `ToolSet` internals, `openDoors` and the other executor code) showed no change in results.

The owner chose `1.21.4` for the swimming and `isWaterloggedSolidTop` rules on 2026-11-05, and for the other rules in the table on 2026-11-06. The risk for the first two was that Horizon's facts
might differ from Java's for these blocks. S2 checked that: `tools/facts-compare` finds no difference on any state, including
the 2,344 waterlogged states with a sturdy top. Swimming is a setting, so it has no block fact to check.

Game differences: none showed. The engine reads the facts of the game version that ran (here the `26.3` jar), and the
patched engine matches all 35,723 registry states and every generated grid, so no `26.3` fact (new classes, new blocks,
changed tool speeds) is missing from the engine's set of facts.

Not tested:

- Horizon's facts in this comparison. They are Java's, from the `26.3` jar. `tools/facts-compare` compares them with Horizon's separately.
- The parts of the movements after the cost functions, and positions other than the grid's centre (see
  `tools/README.md`).
- A rule that the generated worlds never reach. The patch was written from the diff of the two branches and then
  confirmed, so a difference that neither the diff reading nor the worlds showed would be missed. The `26.3` run has no
  mutation run behind it.
- Other values of `jumpPenalty` than the three the configurations use (2, 5 and 0.25).
