# Deviations

Each place where this port differs from upstream on purpose. The rules are in
[PORTING-SAFETY.md](PORTING-SAFETY.md#deviations). Upstream lines are cited at the `1.21.4` commit `1b8e87c`.

Entries 1 to 4 are decided and wait for the code they apply to. Entry 5 is active and has no effect on any real search. Entries 6 to 10 are active (S3) and change no chosen path that a game can reach. Entries 11 to 23 are active (S4, executor, state machine and the mod): they change when a search is cancelled (12) and what `/b stop` waits for (13), and nothing a search returns. Entries 24 to 32 are active (S4, breaking, placing, settings, commands and the glue): none changes a search; 28 changes what the executor does when the settings forbid an action that a path in progress still asks for. Entry 35 is active (free look): it changes what the player's camera does while a path runs, and nothing a search returns. Entries 36 and 37 are active (R2, engine side of `mine` and `goto <block>`): they change which process asks for a search, and no search.

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
- **Upstream:** `MovementHelper.openDoors` aims at the middle of the door's collision shape (`VecUtils.calculateBlockCenter`) with a forced target and presses
  the right button. The gate branch of `MovementTraverse` asks `RotationUtils.reachable` for a rotation that a ray trace confirms, and presses the right button only when it finds one.
- **Ours:** the same, always (since the fix after v0.1.0; `ExecSettings::can_use_blocks` and `MoveFailure::NeedsUseBlock` are gone). A door is aimed at the middle of the *cell*, not of its
  collision shape (there are no shapes in the engine), and the game's own use then acts on whatever its crosshair is on. A gate uses `exec::helper::reachable`, whose ray treats a closed gate as a whole cube
  (`raytrace::blocks_ray`: not pathfindable), D25; when no rotation reaches it nothing is clicked and the movement goes on, as upstream. The right button is the glue's `game.right`, pressed through the pinned
  input hook (`Intent::use_block` counts as a press like `use_item`, D32). Whether a door or gate is open is `WorldView::is_open`, not a block fact (the default says open). Iron doors are not wooden
  (`wooden_door` is false), so `openDoors` leaves them alone as upstream does and the search treats them as walls.
- **Tested:** the engine's unit tests (door and gate, open and closed) and, in the game, L3 `b11_door` and `b12_gate` (chat closed and open: a corridor whose only exit is a closed oak door or fence gate; arrived,
  and only the door's own cells changed state). Not run in the game: iron doors, doors that open towards the player, trapdoors, hinge right, other wood types.
- **Changes chosen paths:** no.
- **Decided by:** the owner (the fix after v0.1.0). Before, the default was off and a closed door on the only route ended the movement and looped.

### D16. Branches that swim, jump a gap or mine from inside a block fail by name

- **Status:** active, in `exec::movements`. Each has a row marked `later` in the ledger. (Until the breaking and placing step, the branches that break, place and use a bucket failed by name too; they are carried out now, see D24 to D28.)
- **Upstream:** a movement swims along the surface with `allowSwimming` (`holdWaterline`, `preTurnIntoTheBend`), runs the parkour jump, and
  mines whatever the crosshair is on when the head is in a block.
- **Ours:** the movement ends as `Failed` with a `MoveFailure` (`Swimming`, `Parkour`). The executor cancels the path and the pathing
  searches again. With `allowSwimming` and `allowParkour` off (upstream's defaults) a search never returns a movement that needs the first or the third.
  The third branch (the head in a block) is ported since the fix after v0.1.0: `Movement.update` switches to the best tool for the block the crosshair is on (if the crosshair is on one) and
  presses the left button, whatever the movement is doing, and the movement goes on (`MoveFailure::InsideWall` is gone). It aims nothing of its own, as upstream.
- **Why:** these are outside the first release (docs/PLAN.md).
- **Changes chosen paths:** no.
- **Decided by:** the owner (the S4 cut, and the task for the breaking and placing step).

### D17. The aim

- **Status:** active, in `look`.
- **Upstream:** `Mth.atan2` (Minecraft's own approximation), random offsets on the aim (`randomLooking` 0.01 and `randomLooking113`),
  free look with a server-only rotation, and `smoothLook`.
- **Ours:** `f64::atan2` and no random offsets. Free look and `smoothLook` are ported since D35: `Intent::rotation` is the rotation of the player's
  tick, and `freelook.rs` decides whether the view keeps it. The mouse grid (`peekRotation`, `nudgeToLevel`) is ported. The sensitivity is the player's own option, as upstream
  (`LookBehavior`, `options.sensitivity()`): the glue copies Horizon's `settings.sensitivity` into `ExecSettings::mouse_sensitivity` every tick and
  `Pathing` hands it to the aim processor each time (a test in `sim.rs` pins it; before the S4 review the setting was never read and the grid was fixed at 0.5).
  `Rotation::new` turns a NaN or an infinity into 0 where upstream throws.
- **Why:** the first two differ by far less than the mouse grid (0.15 degrees).
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
  `ExecSettings::default` for all). Not carried: the settings of processes that do not exist (the mine and get-to-block settings are carried since R2, D37), the chat prefix and notifications. `renderPath` and `allowInventory` are stored
  but read by nobody in the engine; `freeLook`, `blockFreeLook`, `antiCheatCompatibility`, `smoothLook` and `smoothLookTicks` are read through `look_settings` (D35). Names are matched without regard to case. Values are printed as Java prints them (`2.0`, `1.0E7`, a block without `minecraft:`, an item with it) and read as
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
- **Fixed after v0.1.0:** when a falling block lands in the cell of the player's head while a movement runs, the executor mines it as upstream does (D16): best tool for the block under the crosshair, left button, the
  movement goes on. Before, the movement ended as `InsideWall` and the path was searched again. The glue's `in_wall` is "the block at the eye is a full cube" (`shape_full`); upstream's `isInWall` is the game's
  suffocation test, so a full cube that does not suffocate may also count here (not checked against the block table). Not run in the game: mining the sand out (by hand it takes longer than the sand stays a block); L3 `b10_head_sand` only shows that the walk goes on.
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
  The setting `renderPath` is stored; the path is always drawn. The camera follows `freeLook` (D35).
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

### D34. Walking on the world's top layer (`allowWalkOnBuildLimit`, fixes U15)

- **Status:** active, in `search/astar.rs` (the height check), `config.rs` (`SearchSettings::walk_on_build_limit`) and `set.rs`.
- **Upstream:** the search drops any node whose feet are above `maxY`, so nothing moves from the top layer (U15).
- **Ours:** a new setting, `allowWalkOnBuildLimit`, lets the feet be at `maxY + 1` and no higher. The mod's settings default it to
  **true** (owner's choice, 2026-11-07, for 6b6t's spawn floor at y 319); `SearchSettings::default()` keeps it false, so the Java
  comparisons (L2) still run upstream's search. Test: `walking_on_the_top_layer_needs_the_build_limit_setting`.
- **Changes chosen paths:** yes, but only for a player or a path on the top layer.

### D35. Free look (`freeLook`, `blockFreeLook`, `antiCheatCompatibility`, `smoothLook`; replaces the camera hold of D17 and D29)

- **Status:** active, in `freelook.rs` (the decision), `exec/state.rs` (`Intent::look_force`, `look_target`) and `mod/runtime.rs` (the swap).
- **Upstream:** `LookBehavior` sets the player's rotation after the player's own update, so the movement packet carries it, and puts the
  player's rotation back after the entities were updated. Per target, `Target.Mode.resolve` picks `CLIENT` (the view keeps it), `SERVER` (the view is put
  back) or `NONE`. With `freeLook` (default **true**): a plain move is `SERVER` (`antiCheatCompatibility`, default true; `NONE` without it), a break or place
  (`hasToForceRotations`) is `CLIENT` unless `blockFreeLook` (default **false**) is on. With `freeLook` off everything is `CLIENT`. `smoothLook` (default off)
  replaces the view's yaw, in mode `CLIENT`, with the mean of the last `smoothLookTicks` (5) target yaws. The aim is made against `getEffectiveRotation`, the rotation the server was
  last sent, and `objectMouseOver` is a ray along it. The movement of the tick uses the rotation through `RotationMoveEvent`.
- **Ours:** the same decision and the same defaults (`freeLook` true, `blockFreeLook` false, `antiCheatCompatibility` true), so by default a walk leaves the
  camera alone and a dig or a place turns it, as upstream. `blockFreeLook true` keeps the camera for those too. The mod's tick (`native_tick`, after the player's tick) plans the
  rotation for the **next** player tick, as it did before (one tick of delay, as every other input of the mod). A mode `CLIENT` rotation is written to the view at once. A `SERVER`
  one (and a smoothed one) is put in at the start of the game's input step (the pinned hook of `remote_handle_input`, which is before the clicks' picks, the movement and
  `remote_send_state`) and the view is put back at the start of the next `native_tick`, after the whole player tick. So the picks of that tick (the block a click acts on), the movement, and the packet
  all see the aim, and no frame is drawn in between. The effective rotation (`LookState::server_rotation`) is the aim last put in, or the player's own view when none is; `looking_at` is picked from it.
  The `smoothLook` buffer averages the movement's target yaw before the aim processor, as upstream.
- **Differs from upstream:** (1) The elytra and swimming branches of `Mode.resolve` (`elytraFreeLook`, `allowSwimming`) are not ported, as those movements are not. (2) Upstream's
  `onPlayerRotationMove` (the movement uses the rotation by an event) is the swap itself here: the player's body takes `game.yaw` when the tick starts. (3) `serverRotation` is not read from the
  outgoing packets but set to the aim the tick carried; when the mode is `NONE` or there is no target it is the player's own view. (4) The player's third-person model and the lead of a held lead (`rope_rot_old`) are ticked
  inside the swap, so for that one tick they see the aim; the first-person view does not. (5) If the player's tick is skipped (the chunk is not loaded, an error), the swap is not made; if it ends early,
  the view is put back at the next input step. (6) Smooth look does not average the pitch (upstream does only while gliding).
- **Why:** the owner's wish that the camera stays the player's own while the server and the movement follow the path (task of 2026-11-07).
- **Changes chosen paths:** no (the search never reads the view). It changes what the executor sees as its own rotation while a path runs (the aim is made against the rotation the server was sent, as upstream).
- **Decided by:** the owner.

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
- **U15. No path at all from the top of the world.** `AStarPathFinder` drops a neighbour whose feet would be above `maxY`
  (`minY + height - 1`, 319 in the Overworld), and a player standing on the highest block layer has feet at `maxY + 1`. So every
  move from there, even a flat step, is refused and the search ends with no path. Found on 6b6t.org (2026-11-07), whose main-world
  spawn is an obsidian floor at y 319: `/b goto` 50 blocks away on that floor answered "No path to the goal was found." at once. The
  port kept this until the owner chose to change it: D34 (`allowWalkOnBuildLimit`, on by default in the mod).
- **U16. Adventure mode is not considered.** Nothing in upstream reads the game mode or `mayBuild`, so with `allowBreak` on (the
  default) the search still plans to dig. The game refuses the break, the movement times out after its cost plus
  `movementTimeoutTicks`, the new search finds the same path, and the player stands still in that loop until stopped. Found in
  6b6t.org's lobby (2026-11-07): an Ascend through dirt and grass (cost 43.6) repeated every 148 ticks. `/b set allowBreak false`
  avoids it. The port does the same; the owner chose to keep it (2026-11-07).

### D36. Three processes, and the scans are jobs

- **Status:** active, in `process/pathing.rs` (the manager), `process/mine.rs`, `process/get_to_block.rs`. Engine only so far: the glue calls it in the next step.
- **Upstream:** `PathingControlManager` keeps the processes in a hash set, puts a newly active one at the front of a list, sorts by `priority()` (all 0 here) and asks them in turn; the first answer that is
  not `DEFER` is in control, and every process behind it that is not temporary loses control. `MineProcess.mine` and `GetToBlockProcess.getToBlock` run their first scan inside the call, on the game thread; the
  later scans (`rescan`) run on an executor thread every `mineGoalUpdateInterval` ticks and write the answer into the process's fields when they are done, whatever the process is doing by then (`GetToBlockProcess`
  checks that it is still going to the same block, `MineProcess` does not).
- **Ours:** the same rules for three fixed processes (`CustomGoalProcess`, `MineProcess`, `GetToBlockProcess`, registered in that order, so two that become active in the same tick would be ordered by
  that, which the commands never do). `RequestPause`, `RevalidateGoalAndPath` and `Defer` are ported with the pause rule of `tickPath` (`pausedThisTick`, the keys cleared on the first paused tick) and the
  `cancelOnGoalInvalidation` rule of `postTick`. A process that answers `null` while still active is stopped (upstream throws `IllegalStateException`). The first scan is a job too (D39): `Pathing::tick` queues it before the processes
  are asked and the process asks for a pause until it is answered, so no process sees "nothing known yet". Every scan is a `ScanJob` that owns what it reads: the glue takes it (`Pathing::take_scan_jobs`), runs it on any thread over a snapshot
  (`ScanJob::run`) and hands the answer back (`Pathing::deliver_scan`). Each answer carries the process's session number, and an answer from an earlier session (the process was stopped or started again) is dropped.
  A process starts no new scan while one is out; upstream starts one every interval, whether or not the last has finished.
- **Why:** the engine has no threads and no shared fields, and a slow scan of the whole loaded world would otherwise queue up. The session number is what `GetToBlockProcess.rescan` does with its `gettingTo != target` check, for both.
- **Not carried:** `SET_GOAL_AND_PAUSE` (nothing that exists sends it), priorities other than 0, `isTemporary` (no process is), the check that throws when a process stays active after `cancelEverything`, `mostRecentInControl`
  and `mostRecentCommand` (`Pathing::in_control` answers the first). The `safeToCancel` field starts false as upstream's does; nothing reads it with no path, except that a pause request is not honoured then, which is what
  upstream does too.
- **Changes chosen paths:** no. A path is searched as before; what changes is which process asks for it.
- **Decided by:** the task for this step.

### D37. `mine` and `goto <block>`

- **Status:** active, engine only: `blockmeta.rs`, `scan.rs`, `process/mine.rs`, `process/get_to_block.rs`, the mine settings in `set.rs` and `settings.rs`, the commands in `command.rs`.
- **Ours, the same as upstream:** the order and the rules of `MineProcess.onTick` (quantity check, the blacklist after a failed path, the Louca hold on a mined block, the shaft in the player's own column mined from where he
  stands with a pause request, `updateGoal`, `coalesce` with `GoalThreeBlocks`), `prune` and its filters, `searchWorld`, legit mine, `GetToBlockProcess` (goals per block, the nether portal, the chests with a block on top, the
  right click on arrival, `blacklistClosest`), and `FasterWorldScanner.scanChunkRadius` (D39: the square spiral of chunks, the sections from the player's outwards, never more than `max` hits). Defaults are upstream's.
- **Differs from upstream:**
  1. **No cached world.** Every block is scanned straight from the loaded chunks, as upstream does for the blocks it does not keep in its cache. Upstream finds the blocks of `CachedChunk.BLOCKS_TO_KEEP_TRACK_OF` (chests, furnaces,
     ender chests, spawners, portals, shulker boxes, hoppers, beacons, heads, ...) only through the cache, so `goto chest` there finds chests remembered from chunks that are no longer loaded and this does not. Ores
     are not in that list, so `mine` finds the same places. `maxCachedWorldScanCount`, `extendCacheOnThreshold`, `repack` and `scanChunk` are not ported.
  2. **The scanner.** The first version of this port copied `WorldScanner`, which `MineProcess` does not use: `BaritoneProvider.getWorldScanner()` is `FasterWorldScanner`. Corrected in R2 (D39).
  3. **Blocks and drops come from the glue.** `BlockOptionalMeta` matches by state id and item path from a `BlockIndex` the glue builds once (the block and properties of each state, and the items each block drops, from
     the loot tables). Upstream reads the registries and runs the loot tables with a netherite pickaxe and a fixed random seed. The glue should list every item the block can drop for the best case; an item the
     table lacks is not counted towards `mine <n>`, and a dropped item of it is not walked to. An unknown property is a message (`"x" is not a property of Block{minecraft:y}`), where upstream throws a `NullPointerException`.
  4. **Dropped items and the inventory come from the glue too.** `ProcessInputs::item_entities` (empty: none seen), `Inventory::main_slots` (default: the hotbar only; upstream counts the 36 main slots), `ProcessInputs::now_ms`
     for the Louca hold, `ProcessInputs::container_open` for the right click. Without them `mine <n>` counts the hotbar, and dropped items are not walked to.
  5. **The first scan is queued in the first tick** and not run inside the command (D36, D39).
  6. **Texts.** The lines the processes say (`logDirect`) are upstream's, returned by `Pathing::take_messages` for the glue to print. The desktop notification on a failed mine (`notificationOnMineFail`) is the glue's. `GetToBlockProcess.rightClick`'s `System.out.println` is dropped.
  7. **`GetToBlockCalculationContext`** (the context that ignores `allowBreak` when places are picked) is the same scan run with `allowBreak` on. The walk itself still obeys `allowBreak`.
  8. **The new goals print with `Debug`** (`goal_to_string`), as the other goals the commands do not make. No command prints them.
- **Changes chosen paths:** no. The search and the costs are untouched; the goals the processes send are upstream's.
- **Decided by:** the task for this step (the owner's R2: `mine` first).

### D38. The glue of `mine` and `goto <block>` (mod only, found by running it)

What the mod feeds the engine (`mod/runtime.rs`), where it differs from upstream:

1. **Scans run on their own thread.** Every scan (the first of a command too, which upstream runs inline; see D39 for how the process waits for it) is a `ScanJob` over a `Snapshot` of the
   loaded chunks (the same pointer copy the search uses: 50 to 150 microseconds on the game thread in the smoke test). The scan
   itself runs on a thread of its own and the answer reaches the process on a later tick. In the smoke test a scan took 15 to 25 ms with ore
   near, and 2.3 to 2.6 s when fewer than 64 ores were near and the scan went through every loaded chunk (render distance 12);
   that time is off the game thread.
2. **What a block drops** comes from Horizon's loot tables (`blocks/<name>`): every item of every pool and entry that does not need a tool in
   hand (no `match_tool` condition without `inverted`), whatever the chance. Upstream rolls the table with an empty hand and keeps what came
   out. So a block with a chance drop (leaves: saplings) lists it, and silk-touch and shears items are left out. `mine` counts the listed
   items in the 36 main slots.
3. **Item entities** are the game's dropped items (`ItemEntity`), with the block their position is in. They are given only while a process is
   active.
4. **Tab completion** offers the registry's block paths after `mine` and, as the only argument, after `goto`. The names are read once,
   with the first tick, so completion works before any `/b` command has run (about one pass over the registry). Properties inside `[...]` are not completed.
5. **`Arrived.`** is said only for `/b goto <x y z>` and `/b path`. Upstream says nothing when a walk of `mine` or `goto <block>` ends; the
   `AtGoal` event that ends each of them is not announced.
6. **The buttons and the held slot are applied while the keys are not forced** (the intent's `active` is false). Upstream's `mine` pauses the
   path and clicks while it stands still (an ore above the player's head), and `select_slot` comes with it. Before this change the mod
   ignored both outside a walk.

### D39. The first scan is a job, and the scanner is `FasterWorldScanner` (R2 review)

- **Status:** active, in `scan.rs`, `process/pathing.rs`, `process/mine.rs`, `process/get_to_block.rs`. Two things the R2 reviews found.
- **Upstream:** `MineProcess.mine` and `GetToBlockProcess.getToBlock` scan inside the command, on the game thread. The scanner is `FasterWorldScanner` (`BaritoneProvider.getWorldScanner()`).
  `scanChunkRadius` takes the chunks of `getChunkRange(playerChunk, maxSearchRadius)` (a square spiral of `maxSearchRadius - 1` rings; the 32 of `MineProcess` is 31 rings, not a distance), scans them in parallel, and keeps the first `max` hits in
  that order (`limit`); `yLevelThreshold` is not read. Inside a chunk the sections are the player's, the one below, the one above the player's, and so on; inside a section the packed order is `y`, `z`, `x`; a section with
  one block state everywhere (`SingleValuePalette`) is walked `x`, `y`, `z`.
- **Ours:**
  1. `scan::scan_chunk_radius` is that scan, walked in order and stopped once `max` hits are in (the same first `max`; upstream's parallelism is only speed). The earlier `WorldScanner` port (squared-distance rings, a whole chunk kept past
     `max`, a stop at an all-unloaded ring) is gone; it chose different ores whenever more than 64 matched.
  2. The engine cannot see a palette, so "one state everywhere" is read off the 4096 cells, and only for a section whose first cell the filter matches. A section with a bigger palette that holds one state would be walked `y`, `z`, `x`
     upstream and `x`, `y`, `z` here. That needs a section edited down to one state after it was sent; the order matters only for which of the 64 are kept of such a section.
  3. The first scan is queued as a job in the first tick (`Pathing::tick`), like every later one, and the process answers `RequestPause` until the answer is delivered, then goes on with its normal tick 0. The player stands still for the
     length of the scan (15 to 25 ms with ore near; seconds when fewer than 64 matches are near and the scan covers every loaded chunk) instead of the game freezing. Before this change the first scan ran on the game thread and
     measured 0.17 s to 4.6 s of stall in the L3 run (`mod_tick_micros`).
- **Changes chosen paths:** (1) yes, when more than 64 matches are loaded, toward upstream's choice; (3) no.
- **Decided by:** the R2 review findings; (1) is a port bug (upstream's behaviour was copied from the wrong class), (3) keeps the game responsive.

### D40. `follow` (R2)

- **Status:** active, in `process/follow.rs`, `command.rs`, `mth.rs`, `goals.rs`, `process/pathing.rs` and the glue (`mod/runtime.rs`).
- **Upstream:** `FollowProcess` holds a `Predicate<Entity>` and a `List<Entity>` cache and reads `ctx.entitiesStream()`; `FollowCommand` makes the predicate from an enum and from `EntityClassById` / `NearbyPlayer`. `GoalXZ.fromDirection` uses `Mth.sin` and `Mth.cos`.
- **Ours:**
  1. **Entities are plain data.** The glue lists the entities each tick as `EntityInfo` (id, type id with namespace, player name, dropped item, position, living, alive) in `ProcessInputs::entities`, and only while the follow process holds a filter, because
     `isActive` depends on them. The player itself is never listed (upstream's `followable` refuses it). The list is the mobs the game simulates (other players included, through the multiplayer player table) and the dropped items, capped at 4096. Other entity
     kinds Horizon keeps in lists of their own (arrows and other projectiles, boats, minecarts, item frames, armor stands, experience orbs) are not listed, so `follow entity arrow` finds nothing.
  2. **The filter is data.** `FollowFilter` is `Living`, `Players`, `Types`, `Entities` (by id, as `Entity.equals`) or `Items` (`pickup`); these are the only predicates the commands make.
  3. **The cache is not a field.** `scanWorld` is a pure function called where upstream reads `cache`, with the inputs of the tick; `isActive` and `onTick` of one tick see the same list. `isActive` reads the player's position and the follow settings that
     `Pathing::tick` stored at the start of its tick (the manager asks `isActive` outside any tick too).
  4. **`distinct()` is by entity id**, the first occurrence kept, after the filters, as upstream's order. It uses a hash set, as upstream's stream does (a first version searched the answer for each entity: 8 ms per scan at 4096 entities, found in review and fixed, with a test).
     The scan is still worked out at each read (`isActive` of the manager, `onTick`, the display name), as upstream's `scanWorld` runs in `onTick` only; it is linear now (the test scans 40000 entities in well under 500 ms, debug build) and not cached.
  5. **`Mth.sin` / `Mth.cos`** are the game's table (`(i / 10430.378350470453).sin()` as `f32`, indexed through a `long`), in `mth.rs`. `GoalXZ.fromDirection` goes through it. Minecraft 1.21.4 (the branch of the pinned upstream) indexes with a float product; the game
     this runs in (26.3) uses the double one. The two differ only for an angle that lands on the edge of a table step, which can move the goal by one block. The table is built here as `sin(i / 10430.378...)` as `f32`, while the game computes `i * pi * 2 / 65536`; the two can
     differ by one unit in the last place of the `f32`. Not measured (the review read it; no game source at hand to compare). Only `followOffsetDistance` other than 0 reaches it.
  6. **`Entity.toString`** (printed by `follow player <name>` and `displayName0`) cannot use the game's class names or display names. A player prints as `RemotePlayer['Name'/id, l='ClientLevel', x=0.00, y=0.00, z=0.00]` (`LocalPlayer` for the player itself, exact
     but for the position); any other entity prints its registry path in camel case as the class and in title case as the name (`ZombieVillager['Zombie Villager'/12, ...]`). `%.2f` is Rust's rounding, which differs from Java's on a tie.
  7. **The entity types are Horizon's.** `follow entity <type>` knows the ids of the mob types Horizon loads, `minecraft:player` and `minecraft:item`. Upstream knows every entry of `ENTITY_TYPE`. A type that exists in the game but not here answers
     "Expected EntityClassById", as an unknown id does upstream.
  8. **Tab completion of the types offers ids.** Upstream appends `Object::toString` of each `EntityType`, which in vanilla is the translation key (`entity.minecraft.pig`), not an id that `EntityClassById` accepts, then filters with
     `filterPrefixNamespaced`, which assumes fully qualified ids. Read, not run (no vanilla source here to check `EntityType.toString`). Here the completion offers `minecraft:pig`. The rest of `tabComplete` (the groups, the lists, a done word that names nothing gives nothing)
     is as upstream, and so is NearbyPlayer's (names of the players of the world, the player itself included).
  9. **Error texts** carry the position upstream's exception gives (`Error at argument #2: Expected EntityClassById`), including its quirk: the argument that fails has been consumed, so the exception names the next one when there is one. The other
     commands of this port leave the position out.
  10. `follow player <name>` of a name nobody has sets an empty filter and then says "No valid entities in range!", as upstream (it sets the filter before it throws). The process is then inactive and stays so; a later `/b stop` clears the filter.
  11. **The cap of 4096 entities drops the rest silently, mobs first.** The glue lists mobs, then other players, then dropped items, and stops at 4096. In a world with more than 4096 mobs no item is listed, and `follow entity item` finds nothing. Upstream has no cap.
     The listing is O(entities) with one string per entity, which is why the cap exists. Changing the order or the cap is not a path change.
- **Changes chosen paths:** no. The goals are upstream's (`GoalNear` in a `GoalComposite`, revalidated each tick); (5) can move an offset goal by one block in rare cases.
- **Decided by:** the entities come from the game, which has no `Entity` class; the rest follows from keeping the process pure.

### Upstream behaviour kept, found in the R2 reviews (reported, not fixed)

- **U17. A head stuck in a block is mined with no `allowBreak` check.** `Movement.update` presses the left button with the best tool for whatever the crosshair is on when `isInWall`, whatever `allowBreak`,
  `blocksToDisallowBreaking` or `avoidBreaking` say. With `allowBreak false` the player still digs out of a block that lands on his head (sand, gravel) while a walk runs. Kept; a guard would not change a path, but it is the owner's call.
- **U18. The shaft click of `mine` re-checks only `air` and `avoidBreaking`.** A dropped item resting in a cell of the player's own column (a torch, a flower) is a known place (`mineScanDroppedItems`), and `MineProcess.onTick` clicks
  the block of the shaft without asking whether the filter or `allowBreak` would let the search break it. With `allowBreak false` and an `allowBreakAnyway` filter the mod can click such a cell. Kept.
- **U19. Some numeric settings have no bound.** `allowOnlyExposedOresDistance` loops over `(2r+1)^3` cells per candidate and `mineMaxOreLocationsCount` can make the scan keep every match; a huge value set by the user costs time or
  memory. Kept as upstream.
- **U20. A button can stay pressed for one extra tick** when `MineProcess.onTick` returns before `clearAllKeys` (quantity reached, or a failed path with nothing to blacklist) while a paused path exists. Read, not measured. Kept.
- **U21. A `GoalComposite` of N goals costs N for every search node** (`isInGoal` and `heuristic` ask each goal). `follow entities` with `followTargetMaxDistance 0` in a world with hundreds of mobs multiplies the search time by that number. Upstream's shape, kept; changing the heuristic would change which paths are found.
  The default `followTargetMaxDistance 45` bounds the count in practice. Read and reasoned, not measured in the game.

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
