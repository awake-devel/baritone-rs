# Deviations

Each place where this port differs from upstream on purpose. The rules are in
[PORTING-SAFETY.md](PORTING-SAFETY.md#deviations). Upstream lines are cited at the `1.21.4` commit `1b8e87c`.

Entries 1 to 4 are decided and wait for the code they apply to. Entry 5 is active and has no effect on any real search.

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
