# Deviations

Each place where this port differs from upstream on purpose. The rules are in
[PORTING-SAFETY.md](PORTING-SAFETY.md#deviations). Upstream lines are cited at the `1.21.4` commit `1b8e87c`.

No deviation is active yet: there is no engine. Entries 1 to 4 are decided and wait for the code they apply to.

## How exceptions work today

They don't. `tools/cost-compare` has no exception mechanism. Its comparison against `1.21.4` is exact and no deviation
touches `MovementTraverse.cost` or `MovementDescend.cost`, so nothing needs to be excused.

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

## Where upstream's two branches disagree

Measured with `tools/cost-compare` on `MovementTraverse.cost` and `MovementDescend.cost`: 87 block states, 5000 generated
grids, one run, against upstream `26.3` (commit `25111da`). The Rust translation matches `1.21.4` on the same grids, so
every difference below comes from the two Java branches. The `26.3` run uses a separate harness copy
(`tools/cost-compare/java-26.3`), which builds its context by reflection.

For all 87 states, every fact the harness exports is identical on both versions. No game difference showed up in what
was exported.

With both rules below switched off in a scratch copy of the Rust code, no mismatch is left. So these two explain all of
it.

| Rule | Where | Results that differ | We follow | Status |
|---|---|---|---|---|
| Swimming | `1.21.4` adds `allowSwimming` and `SWIM_ONE_BLOCK_COST`, and caps the water walk speed with it. `26.3` has neither. | 1009 traverse results, all in the one config with swimming and sprint. Descend is not affected. `26.3` comes out dearer. | `1.21.4` | Decided by the owner |
| `isWaterloggedSolidTop` | Only in `1.21.4`. Used in `canWalkOnBlockState` and `MovementDescend.cost`. `26.3` treats waterlogged top slabs, top stairs, top trapdoors and scaffolding as plain water. | 553 traverse and 524 descend results, in all configs. | `1.21.4` | Decided by the owner |

The two classes overlap in 31 traverse results.

Reasoning:

- Both are rules that `1.21.4` has and `26.3` lacks, which fits `26.3` being the branch that lags behind.
- For `isWaterloggedSolidTop`, the fact it relies on (whether the face is sturdy) is the same on both versions for all 87
  states. So the rule is not a game difference.
- Swimming is a setting rather than a fact about blocks, so the data cannot settle it.

The owner chose `1.21.4` for both on 2026-11-05. Both rules are a **risk for S2**: if
Horizon's own facts differ from Java's for these blocks, the rules would give wrong answers on Horizon. S2 has to check
exactly these blocks.

Not tested:

- Doors. `1.21.4` accepts only wooden doors, and `26.3` accepts every door except the iron one. They differ for copper
  doors. The palette has only oak and iron doors, so this gave zero results and is untested.
- `jumpPenalty` has the same default on both and is not used by the two functions compared.
- The magma corner rule is in `MovementDiagonal`, which the harness does not cover.
- Other differences in how the two branches are written (`canPlaceAgainst`, `minY`, mining cache, precomputed flags)
  showed no change in results here.
