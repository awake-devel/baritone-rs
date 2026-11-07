# Deviations

Each place where this port differs from upstream on purpose. The rules are in
[PORTING-SAFETY.md](PORTING-SAFETY.md#deviations). Upstream lines are cited at the `1.21.4` commit `1b8e87c`.

Entries 1 to 4 are decided and wait for the code they apply to. Entry 5 is active and has no effect on any real search. Entries 6 to 10 are active (S3) and change no chosen path that a game can reach. Entries 11 to 23 are active (S4, executor, state machine and the mod): they change when a search is cancelled (12) and what `/b stop` waits for (13), and nothing a search returns. Entries 24 to 32 are active (S4, breaking, placing, settings, commands and the glue): none changes a search; 28 changes what the executor does when the settings forbid an action that a path in progress still asks for.

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

### D11. The executor owns its movements, and splices in place

- **Status:** active, in `exec`.
- **Upstream:** `PathExecutor` holds the `IPath`, whose `Movement` objects carry their own state (`currentState`, `numTicks`,
  `forceSafeMode`, cached costs). A splice or a cut makes a new executor over movement objects shared with the old one.
- **Ours:** `PathExecutor` copies the movements out of the path and keeps their runtime state in a parallel list. `try_splice`
  changes the executor in place and appends the next executor's movements and state. It keeps what upstream's new executor is
  given (position, cost estimate and its index, ticks on the current movement) and resets what it resets (`ticksAway`, `sprintNextTick`).
- **Why:** a path is plain data that a search hands over; nothing is shared between threads.
- **Changes chosen paths:** no.
- **Decided by:** nobody needed.

### D12. The best path so far of a running search cannot be read

- **Status:** active, in `exec` and `process::pathing`.
- **Upstream:** `PathExecutor.shouldPause` and the "are we calculating the right thing" check in `tickPath` read
  `inProgress.bestPathSoFar()`, which the running search computes on demand from another thread.
- **Ours:** the search runs in one call on the worker, and its partial path is not visible before it ends. The executor gets
  `Progress { best_positions: None }`. `shouldPause` then never pauses (the player does not wait for a search that is about to
  replace the path with a backtrack). `tickPath` takes "there is no best path so far", so it cancels a search whose start is
  neither the end of the current path, nor the player's feet, nor the expected start, without asking whether the best path so far
  passes through the player.
- **Why:** reading it would mean running the search in slices, which changes the search code and its comparison with Java.
- **Changes chosen paths:** it changes when a search is cancelled and restarted, not which path a search returns.
- **Decided by:** nobody yet; reported. Review if the player is seen to walk back and forth while a long search runs.

### D13. `/b stop` is `forceCancel`

- **Status:** active, in `process::pathing` (`Pathing::stop`).
- **Upstream:** `#stop` is `cancelEverything`: the processes lose control at once, but the current segment is cut only when
  the movement says it is safe (`safeToCancel`). A fall in the air, or a diagonal cornering over a gap, goes on until then.
- **Ours:** `stop` is `forceCancel`: it cancels the search, drops the current and next paths, clears the keys and returns a neutral
  `Intent`. `cancel_everything` is also there, with upstream's behaviour.
- **Why:** with walking only, nothing needs a safe moment, and a stop that leaves keys held for some ticks is a surprise.
- **Changes chosen paths:** no.
- **Decided by:** the task for S4 ("stop always gives a neutral Intent").

### D14. A cancelled or replaced search is dropped by its generation, and does not block the next one

- **Status:** active, in `process::pathing` and `worker`.
- **Upstream:** `inProgress` stays set until the search thread returns, even after `cancel()`, so no second search starts
  until the cancelled one has ended; its result is dropped by the thread itself (`Cancellation` installs nothing).
- **Ours:** `cancel` forgets the search at once, tells the runner to stop it, and a result of any other generation than the one
  in progress is dropped when it arrives. A new search can start in the same tick. The worker runs one search at a time: a new job replaces a job that waits, and
  stops the one that runs, and a cancelled search delivers nothing.
- **Why:** the plan (docs/PLAN.md): "results carry a generation number, so a stale result is dropped".
- **Changes chosen paths:** no.
- **Decided by:** the plan.

### D15. Closed doors and fence gates

- **Status:** active, in `exec::movements`.
- **Upstream:** `MovementHelper.openDoors` and the gate branch of `MovementTraverse` aim at the middle of the block's collision shape
  (`VecUtils.calculateBlockCenter`, `RotationUtils.reachable` with a ray trace) and press the right button.
- **Ours:** the aim is the middle of the block, and there is no ray trace, so a gate is always "reachable". The click is
  `Intent::use_block`, set only when `ExecSettings::can_use_blocks` is true (the default is false). When it is false, the movement
  ends as `Failed` with `NeedsUseBlock` and the path is cancelled and searched again. Whether a door or gate is open is
  `WorldView::is_open`, not a block fact (the default says open).
- **Why:** the glue may not have a simple way to press the button yet. Not adding fields to the block table keeps `facts-compare` as it is.
- **Changes chosen paths:** no. But a closed door on the only route is searched again at once, so the walk loops until the door is opened by hand, or the glue sets `can_use_blocks`.
- **Decided by:** the task for S4.

### D16. Branches that swim, jump a gap or mine from inside a block fail by name

- **Status:** active, in `exec::movements`. Each has a row marked `later` in the ledger. (Until the breaking and placing step, the branches that break, place and use a bucket failed by name too; they are carried out now, see D24 to D28.)
- **Upstream:** a movement swims along the surface with `allowSwimming` (`holdWaterline`, `preTurnIntoTheBend`), runs the parkour jump, and
  mines whatever the crosshair is on when the head is in a block.
- **Ours:** the movement ends as `Failed` with a `MoveFailure` (`Swimming`, `InsideWall`, `Parkour`). The executor cancels the path and the pathing
  searches again. With `allowSwimming` and `allowParkour` off (upstream's defaults) a search never returns a movement that needs the first or the third;
  `InsideWall` happens when the player's head is in a block.
- **Why:** these are outside the first release (docs/PLAN.md).
- **Changes chosen paths:** no.
- **Decided by:** the owner (the S4 cut, and the task for the breaking and placing step).

### D17. The aim

- **Status:** active, in `look`.
- **Upstream:** `Mth.atan2` (Minecraft's own approximation), random offsets on the aim (`randomLooking` 0.01 and `randomLooking113`),
  free look with a server-only rotation, and `smoothLook`.
- **Ours:** `f64::atan2`, no random offsets, no free look and no smoothing: `Intent::rotation` is what the glue sets as the player's yaw and
  pitch. The mouse grid (`peekRotation`, `nudgeToLevel`) is ported. The sensitivity is the player's own option, as upstream
  (`LookBehavior`, `options.sensitivity()`): the glue copies Horizon's `settings.sensitivity` into `ExecSettings::mouse_sensitivity` every tick and
  `Pathing` hands it to the aim processor each time (a test in `sim.rs` pins it; before the S4 review the setting was never read and the grid was fixed at 0.5).
  `Rotation::new` turns a NaN or an infinity into 0 where upstream throws.
- **Why:** the first two differ by far less than the mouse grid (0.15 degrees). The glue has no server-only rotation.
- **Changes chosen paths:** no.
- **Decided by:** the task for S4.

### D18. Exceptions that upstream throws are values

- **Status:** active, in `exec` and `process`.
- **Upstream:** `openDoors` throws `NoSuchElementException` when the two positions are not next to each other. `onTick` throws `IllegalStateException`
  when `overrideFall` returns a position not on the path. `trySplice` throws when the spliced path ends elsewhere.
  `CustomGoalProcess.onTick` throws in an unknown state. `Movement.calculatedWhileLoaded` unboxes a null.
- **Ours:** `open_doors` answers true. The fall override cancels the path (`CancelReason::IllegalState`). A splice that cannot be done drops the
  planned next path. A process that is not active answers a command that does nothing. An unset `calculated_while_loaded` counts as loaded.
- **Why:** the executor must not panic. None of these is reached by a path from the search.
- **Changes chosen paths:** no.
- **Decided by:** nobody needed.

### D19. Smaller differences

- `MovementFall.willPlaceBucket` runs `dynamicFallCost` again, as upstream (an earlier version of the port used `hasWaterBucket && drop > maxFallHeightNoWater`, which placed water on a fall past a ladder where the search had planned no bucket; found by the review of the breaking and placing step and fixed, `exec/review_tests.rs` R1).
- `Movement.prepared` waits for falling-block entities (`pauseMiningForFallingBlocks`) through `WorldView::falling_block_near`, which says none unless the glue answers it (D31).
- `Movement.update` sets `flying = false` and the auto jump off: these are `Intent::cancel_flying` and `disable_auto_jump`.
- `PathingBehavior.pathStart` and the rest are as upstream. The search's `previous` path for favoring is a clone of the current path.
- The order of a tick differs from Baritone's: Baritone ticks before the player moves; Horizon's `native_tick` runs after the move,
  and the hook that sets the keys runs before the next one. An `Intent` therefore acts on the next movement of the player (`docs/PLAN.md`, "Walking with the chat open").

### D20. Arriving is recognised by the mod

- **Status:** active, in `mod/runtime.rs`.
- **Upstream:** `PathingBehavior.tickPath` queues `AT_GOAL` when the path ends in the goal. But `CustomGoalProcess` gives up control as soon as the
  feet are in the goal, and that runs in `preTick`, before `tickPath`. Read from the code, the path is then cut first and `AT_GOAL` is not
  queued (not traced in the game). Upstream says nothing at that moment in its default settings, so it would not show there.
  That holds only when the previous tick said the movement was safe to cancel. After a fall or a descend it is not, and then the engine does queue
  `AtGoal` (test `at_goal_is_queued_when_a_fall_ends_in_the_goal`, same as upstream). The process has lost control as soon as the feet are in the goal's
  cell, so "Arrived." can be printed while the player is still falling.
- **Ours:** the mod remembers the goal of `/b goto`. When the goal process is done and the feet are in that goal, it says "Arrived." The
  engine is unchanged.
- **Why:** a first version should say that it arrived. In the game on 2026-11-06 a 30-block walk ended in the goal and the mod received no `AtGoal` event.
- **Changes chosen paths:** no.
- **Decided by:** nobody needed.

### D21. The mod reads the world from two places

- **Status:** active, in `mod/runtime.rs`.
- **Upstream:** one `BlockStateInterface` over the live world, for the search thread and the game thread alike.
- **Ours:** the search gets a `Snapshot`: clones of the `Arc<Chunk>` of every loaded chunk, taken on the game thread when a search starts,
  indexed by the block table. The executor's rechecks on the game thread read the live world (`Live`). They share only the block table
  (`Tables`, read only). The snapshot covers every loaded chunk, not a window around the player.
- **Why:** docs/PLAN.md, "How it runs".
- **Changes chosen paths:** no.
- **Decided by:** the plan.

### D22. What the first version leaves out of the player's side

- **Status:** active, in `mod/runtime.rs`.
- **Ours:** `Config.can_sprint` is true for the whole session (the executor still reads the food level and stops a sprint at 6 or less); the
  tool strength is the empty hand's, because nothing is broken; `water_mult` and `frost_walker` are the defaults of a player without boots. There
  is no setting yet, so the executor's `ExecSettings` are upstream's defaults. `goto <block>` is refused with a message (the get-to-block process is not
  ported). A path to a goal in another dimension is not a thing: the goal is dropped when the dimension changes.
- **Changes chosen paths:** `can_sprint` could, for a player who starts a walk hungry. Not seen.
- **Decided by:** the task for S4.

### D23. A search that panics stops the goto

- **Status:** active, in `process/pathing.rs` (`take_search_failure`) and `mod/runtime.rs`.
- **Upstream:** an exception inside a search ends it with `Exception`; no `CALC_FAILED` is queued, the goal process stays in its executing
  state and starts the same search on the next tick, again and again.
- **Ours:** the engine keeps that behaviour and only remembers the failure. The glue reads it, stops the goto and says so in the chat, once.
- **Why:** upstream's loop would take a chunk snapshot every tick and log an error at 20 Hz while the player stands still and says nothing.
- **Changes chosen paths:** no. It only applies when no path was found at all.
- **Decided by:** nobody needed. A search now also cannot index past the block table: the glue reads an id the table lacks as air
  (`world::known_state`, not upstream), which removes the one panic source found.

### D24. The hotbar is an interface, and tools are chosen by state

- **Status:** active, in `inventory.rs`.
- **Upstream:** `ToolSet` reads `player.getInventory()` and picks the best slot for a `Block` (`b.defaultBlockState()`), with a cache by block.
  `InventoryBehavior` also moves stacks from the main inventory to the hotbar (`allowInventory`) and has a branch for the builder
  (`selectThrowawayForLocation` with `placeAt` from the builder process).
- **Ours:** the glue answers `Inventory` (the slot in hand, the nine stacks and the off hand as `SlotInfo`, and `speed_vs_state`, which is
  `ToolSet.calculateSpeedVsBlock` for one stack and one state). The choice (`get_best_slot`, ties, swords, `itemSaver`, silk touch, the
  material cost) is upstream's, over a state id read from the world; upstream reads the default state of the block, which has the same hardness and
  tool rules but could differ for a block whose destroy speed depends on a property (none in vanilla). `allowInventory` is stored and printed but not
  carried out, there is no cache (the glue keeps the table of strengths), and there is no builder, so `selectThrowawayForLocation` is the loop over
  `acceptableThrowawayItems` only.
- **Changes chosen paths:** no. The costs read `Tables::tool_strength`, which the glue makes with `get_str_vs_block`.
- **Decided by:** the task for this step.

### D25. Aiming works on whole cubes

- **Status:** active, in `raytrace.rs` and `exec/helper.rs`.
- **Upstream:** `RayTraceUtils.rayTraceTowards` asks `Level.clip` for the first outline shape a ray meets, `RotationUtils.reachable` tries the middle of the block's
  collision shape and then the middle of each face of it (`VecUtils.calculateBlockCenter`), and `Mth.sin` and `Mth.cos` read tables.
- **Ours:** the engine has no shapes, only the flags of a state. A ray stops at a block that is not air, not a fluid block, and either a full cube or not walkable;
  it meets the whole cube. The middle of a block is the middle of the cube, and the faces are the faces of the cube. The look vector uses `f32::sin` and `f32::cos`.
  The first face tried by upstream's `reachable` is the top: its table of side offsets is named down, up, north, south, west, east but is applied as `1 - offset`,
  so "down" is the top. That order is kept. The ray starts at the eye of a sneaking player (1.27) when the movement will sneak.
- **Why:** shapes would need a new column of the block table, and `facts-compare` checks that table state by state.
- **Changes what the game does:** an aim at a slab, a stair or a flower can be judged reachable when the real shape is not hit, or the other way round. The game
  decides what is hit: the glue reports it in `PlayerView::looking_at` and the movements press the button only when it is the block they want, or when the view is
  already really close to the aim they asked for (upstream's own fallback). A face that is wrongly judged costs a few ticks, not a wrong block.
- **Changes chosen paths:** no.
- **Decided by:** the task for this step.

### D26. Clicks are intent fields, and wait for the slot

- **Status:** active, in `process/pathing.rs` (`build_intent`) and `exec/state.rs`.
- **Upstream:** `InputOverrideHandler` forces `CLICK_LEFT` and `CLICK_RIGHT` and `BlockBreakHelper` and `BlockPlaceHelper` act on `objectMouseOver` in
  the same tick that `selected` changes. Left wins over right.
- **Ours:** `Intent::attack` (hold the left button), `Intent::use_item` (press the right button) and `Intent::select_slot`. Left wins over right. A
  click is not sent on a tick in which the slot to hold is not the slot in hand: it waits one tick, so the game has the new slot when it clicks (the server
  is told of the slot in `remote_send_state`, which may run after the input step; see docs/PLAN.md). The glue stops a dig when `attack` goes from
  true to false and when the mod lets go of the player.
- **Changes chosen paths:** no. A change of tool costs one tick.
- **Decided by:** the task for this step.

### D27. A branch of `Movement.prepared` that cannot be reached

- **Status:** active, in `exec::movements::prepared_base`.
- **Upstream:** after the loop, `if (somethingInTheWay)` sets the status to unreachable. Inside the loop, both ways out of the `if (!canWalkThrough)` return, so
  the flag is never true after the loop.
- **Ours:** the flag is not kept. The unreachable status is used for the case of D28.
- **Changes chosen paths:** no.

### D28. Nothing is broken or placed that the settings forbid

- **Status:** active, in `exec::movements` (`prepared_base`, the traverse, the pillar) and `exec::helper::attempt_to_place_a_block`.
- **Upstream:** the movement breaks or places whatever it needs. The settings act through the cost, which `PathExecutor.onTick` checks every tick before it
  updates the movement, and cancels the path when it is infinite and `safeToCancel`. While the movement is not safe to cancel (a bridge block half placed, a fall) the
  check does not cancel and the movement goes on to break or place all the same. `attemptToPlaceABlock` also goes on to click when there is no throwaway block, with
  whatever is in the hand, and the clicks "something is in the way" of the traverse and the pillar break whatever the crosshair is on.
- **Ours:** a block in the way of `prepared` is broken only if the movement's own cost would pay for it (`Ctx::mining_duration_at` is finite: `allowBreak` or
  `allowBreakAnyway`, not `blocksToDisallowBreaking`, no liquid beside it, a tool that digs it). Otherwise the movement ends as `Unreachable` and nothing is clicked.
  `attemptToPlaceABlock` and the pillar end the movement as `Unreachable` when `allowPlace` is off (`ExecSettings::allow_place`), and a placement with no throwaway block
  in the hotbar is `Unreachable` too. The incidental left clicks (the traverse's "something is in the way", the pillar's block at the feet) need `allowBreak` and are not pressed while the crosshair is on a block that
  must never be broken (`blocksToDisallowBreaking`, ice, an infested block; `may_click_left`, review finding R4). When the crosshair is on nothing or on another block they are pressed as upstream.
  The pick-up of the water after a bucket fall is not placing and is done whenever an empty bucket is in the hotbar, as upstream.
- **Why:** the owner's task: the executor never attacks or places when the settings forbid it. Unit tests pin each guard (`exec/break_place_tests.rs`); a fixed-seed test in `sim.rs` over random worlds with edits pins that the right button is never pressed with `allowPlace` off (it fails if the guard is removed). The same test did not fail with the break guard removed, because the cost check cancels first in those worlds, so the break guard rests on its unit test.
- **Changes chosen paths:** no. It changes what a running path does in the few ticks in which upstream would have broken a block that the settings forbid.
- **Decided by:** the task for this step.

### D29. The settings and the `set` command

- **Status:** active, in `set.rs`.
- **Ours:** one table of the settings of this port, with upstream's names, types and defaults (checked against `Settings.java` for each and against `Config::upstream_defaults` and
  `ExecSettings::default` for all). Not carried: the settings of processes that do not exist, the chat prefix and notifications. `freeLook`, `renderPath` and `allowInventory` are stored
  but read by nobody in the engine. Names are matched without regard to case. Values are printed as Java prints them (`2.0`, `1.0E7`, a block without `minecraft:`, an item with it) and read as
  `SettingsUtil` reads them, except: a number is read with Rust's parser after trimming and a trailing `d` or `f` (no hexadecimal floats), "modified" means "not equal to the default" where
  upstream compares object identity, and the command's errors read `Expected a valid setting` without the position of the argument. `set save` and `set load` ask the glue for the file
  (`SetEffect`); `Settings::to_file_text` and `apply_file_text` are the file's text.
- **Changes chosen paths:** only as the user's settings do.
- **Decided by:** the task for this step.

### D30. The commands `goal`, `path`, `eta` and the completion

- **Status:** active, in `command.rs` and `process/pathing.rs`.
- **Ours:** `goal` (a goal, or `reset`, `clear`, `none`), `path` and `eta` are upstream's with its texts; `goal` with arguments that are not numbers falls back to the player's own position
  as `RelativeGoal` does. `eta` refuses with "No process in control" unless the goal process was asked in the last tick (`Pathing::in_control`); like upstream's `mostRecentInControl`
  that is still true on the tick in which the player arrived, so `eta` then prints its times. `PathingBehavior.estimatedTicksToGoal` is `Pathing::estimated_ticks_to_goal` (it resets as upstream
  when the player is in the goal). `%.0f` is rounded half away from zero as Java does. Completion offers the command names, `~`, the words of `goal`, and for `set` what `SetCommand.tabComplete` offers;
  `goto` does not offer block names, because `goto <block>` is not ported.
- **Changes chosen paths:** no.
- **Decided by:** the task for this step.

### D31. Falling blocks need the glue

- **Status:** active, in `world.rs` (the box test `falling_box_hits`) and `mod/runtime.rs` (`Live::falling_block_near`).
- **Ours:** `Movement.prepared` asks `WorldView::falling_block_near` before it mines, as upstream asks the entities (`getEntitiesOfClass(FallingBlockEntity, AABB(0,0,0,1,1.1,1).move(pos))`).
  The default says none. The game thread's `Live` world answers it from the falling blocks the game holds (`game.falling`: a box 0.98 wide and tall, tested strictly on all three axes). Before the fix of the
  review (S4b), `pauseMiningForFallingBlocks` had no effect in the game. It is a unit-tested box test; no sand or gravel was mined in the game.
- **Still different:** when a falling block lands in the cell of the player's head while a movement runs, the movement ends as `InsideWall` (D16) and the path is cancelled and searched again, where upstream
  mines the block out of the way.
- **Changes chosen paths:** no.

### D32. The glue: buttons, slot, settings file, completion

- **Status:** active, in `mod/runtime.rs` and the hooks it uses.
- **Upstream:** `InputOverrideHandler` forces `CLICK_LEFT` and `CLICK_RIGHT` and Minecraft's own tick acts on them; the settings are one object saved to `settings.txt` in the game folder; `#`-commands complete through
  `TabCompleteEvent`.
- **Ours:** the buttons are set in the start of the game's input step (a pinned `HEAD`) and the chat is swapped for `Playing` for that call (`RETURN` puts it back), because that step does nothing with the chat open. A button the mod held and
  no longer wants is let go of once, and the let-go stays pending until an input step on the `Playing` screen (or the chat swapped for it) has delivered it (`buttons.rs`; an earlier version lost it on a second
  stop before the next step or on the inventory screen). `RETURN` puts the chat back only if the step left the screen as `Playing`. The slot is set with `game.selected` and is not put back when the path ends, as upstream. `settings.txt` is in the mod's own folder (`horizon-mods/.horizon-mod-data/baritone_rs/`) of the game folder, read
  on first use and written after each change. Completion is a pinned `RETURN` on the command tree's suggestions, with a module added to `horizon-command` to hand the function across crates. The table of block facts is made again when a `Sig`
  changes (settings, hotbar items and enchantments, throwaway, bucket, food above 6, potion), not each tick; upstream builds a new `CalculationContext` per search and a `ToolSet` per use, and the table here is the same snapshot taken
  when something changed. `canSprint` is `allowSprint` and food above 6, as upstream. `frostWalker` and `waterSpeedMultiplier` stay 0 and 1.0 (not carried from the boots, as in the walking-only version; U8 explains the second).
  The settings `freeLook` and `renderPath` are stored; the path is always drawn and the camera is always the mod's while a path runs.
  The `trace` command and the word `trace` in completion are not upstream's. `/b trace` prints the exact position of the player every tick, whatever `censorCoordinates` says; it is opt-in and off at each start.
  `/b` and `/baritone` are claimed from servers: a server's own command of that name is not sent while the mod is loaded, also when the mod is switched off, and the chat box completes them from the mod. Other commands pass untouched.
  The test triggers (`BARITONE_RS_L3_SPEC`, `BARITONE_RS_SCENARIO_DIR`) run only in a single-player world whose name starts with `l3-`; `tools/l3/run.py` refuses a name that does not, and a name that already exists.
  Mod chat lines are also written to the game log, so coordinates in "Going to" are logged unless `censorCoordinates` is on. The note buffer for the test triggers is filled only while a trigger variable is set.
- **Changes chosen paths:** no. A boot with Depth Strider or Frost Walker is not seen, so a player wearing them gets the costs of one who does not.
- **Decided by:** the task for this step.

### D33. Smaller differences in `set` and `eta`

- **Status:** active, in `set.rs` and `command.rs`.
- **Ours:** `set load <file>` completes no file names (upstream lists the files of the settings folder). `set reset <letters>` completes the setting names that start with them; upstream completes nothing there, because
  its provider looks at the word being typed. `%.0f` in `eta` rounds with `floor(x + 0.5)`, which differs from Java for the one double just below 0.5 and cannot show in a time. A line of `settings.txt` is judged by upstream's
  pattern `^([^ ]+) +(.+)` (see `apply_file_text`): a leading space or a name with no value is "Invalid syntax in setting file", anything else that does not parse is "Unable to parse line".
- **Changes chosen paths:** no.

### Upstream behaviour kept, found in S3 (reported, not fixed)

The port does what upstream does here; a change would change paths, so it is the owner's call.

- **U1. `getDistFromStartSq` is an `int` and wraps.** A best node 46,341 to 65,535 blocks from the start has a squared
  distance that wraps to a value below 25 or negative, so the partial path is refused as "no path". In a one-wide corridor
  both sides answered a segment at a cap of 46,000 nodes and a failure at caps from 46,343 to 65,536. The committed corpus has a
  corridor of 70,000 blocks along z (caps 46,000, 46,400 and 65,540) for this.
- **U2. The y part of the cell key repeats every 2048 blocks** (`(y >> 3) & 0xFF`, bug 4 in `tools/search-compare/known-missed.txt`). Two nodes
  2048 blocks apart in y, with the same x and z, share a slot, so a search in a column higher than 2048 stops at 2048 nodes and
  returns a segment. The committed corpus has a one-wide shaft 4064 high for this. A vanilla world is at most 384 high.

### Upstream behaviour kept, found in S4 (reported, not fixed)

- **U3. A new `goto` while a search runs does not stop that search.** `ForceRevalidate` only cancels when there is a current path, so
  the search for the old goal finishes and its path is followed to its end; the new goal is searched from there.
- **U4. A descend is never sprinted through into a traverse.** `canSprintFromDescendInto` compares the traverse's direction `(1, 0, 0)` with the
  descend's `(1, -1, 0)`; older Baritone compared `next.getDirection().down()`. At 1b8e87c the descend into a traverse slows to a stop; a
  descend into a descend, and into a diagonal, still sprints. Fixing it changes how fast stairs down are walked, so it is the owner's call.
- **U5. When the goal is reached, `CustomGoalProcess` answers `CANCEL_AND_SET_GOAL` with a null goal** (it builds the command after `onLostControl`), and
  `PathingBehavior` sets its goal to null. Harmless.
- **U6. `CANCELED` is queued on every tick without a process** (`cancelSegmentIfSafe` runs each idle tick). The events are only for the chat.
- **U7. While a path is followed, the player's own keys are ignored.** See `docs/PLAN.md`, decision 3.
- **U8. Without Depth Strider, water costs the same as land.** `CalculationContext.waterSpeedMultiplier` ends in `return 1.0f` at 1b8e87c, and the
  26.3 branch starts from the same `1.0f`. The formula `WALK_ONE_IN_WATER_COST * (1 - m) + WALK_ONE_BLOCK_COST * m` then gives 4.633 ticks a block, not 9.091,
  so a player without boots is priced as faster in water than one with Depth Strider I (m = 0.33). The search takes water it should avoid. In the game (S4
  scenarios 5 and 10) moves through deep water took about 2.2 times their estimate. A value of 0 would match the 9.091 constant. Fixing it changes
  paths, so it is the owner's call.

### Upstream behaviour kept, found in the breaking and placing step (reported, not fixed)

- **U9. `attemptToPlaceABlock` aims at the lower quarter of the face.** `faceY = (placeAt.y + against.y + 0.5) * 0.5` is the point a quarter of a block above the lower edge of the
  two blocks, not the middle of the face. It works for the faces it is used with. Kept.
- **U10. `RotationUtils.reachable` tries the top of the block first of the faces.** See D25.
- **U11. The traverse's "something is in the way" click breaks whatever the crosshair is on,** and the place and break buttons are decided by the crosshair of the previous frame in
  Baritone and of the previous tick here. Kept, with the guard of D28 (`allowBreak`, and no click while the crosshair is on a block that must never be broken).
- **U12. Diagonals never break.** `MovementDiagonal.prepared` is always true, and the cost lets a diagonal through only if one corner is free and the destination can be walked through,
  so `toBreak` of a diagonal is empty for any path a search returns.
- **U13. `ToolSet.getMaterialCost` always answers -1 for a tool.** It tests the stack against `ItemTags.WOODEN_TOOL_MATERIALS` and the other five, which hold the repair materials (`wooden_tool_materials` is `#minecraft:planks`, `iron_tool_materials` is the iron ingot; read in the 26.3 tag files, and 1.21.4's tags have the same names), not the tools.
  A stone pickaxe and a diamond pickaxe are never told apart by material, so a tie in speed goes to the first slot (and to silk touch). The glue computes it the same way, with the same tags, so it is -1 for a tool here too. Fixing it
  would change which of two equal tools is used, so it is the owner's call.
- **U14. The pillar breaks the block at its feet with the throwaway block in hand.** `MovementPillar` selects the throwaway block (`selectThrowawayForLocation`) and then clicks the block at the feet
  when it is neither air nor replaceable, with no `switchToBestToolFor`. It matters only for a block that the cost lets through but is neither walkable nor replaceable (a torch, a carpet, a sign): it is dug by hand.
  Kept (`exec/review_tests.rs` R3).

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
