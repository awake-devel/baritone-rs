# Plan

**Status: approved on 2026-11-05.** How [Porting safety](PORTING-SAFETY.md) checks this work is a separate
document. Horizon facts below were read at revision `66f3981`. None of this design has been built, except where a test
is named.

## The approach

Port the rules, and write the rest new.

| Part | Approach |
|---|---|
| Movement costs, block predicates, goals | Translated closely: same function names and condition order |
| A*, partial paths, segment planning, executor rules | Same behaviour and rule order, new data structures and ownership |
| Mine, Follow, Explore and other process decisions, aim model | Same decisions, new structure |
| World access, threads, cache, settings, commands, Horizon glue | New |
| Java API layer, event bus, mixins, multi-instance, Schematica glue | Dropped |

An audit of upstream's code led to this split. A straight translation can't work, because upstream reads live chunks from
other threads without locks and shares one context between threads. A rewrite from zero would lose the movement rules,
which are written down nowhere else.

## First release

**In scope:** walking, sprinting, breaking and placing along a computed path, on a pinned Horizon revision, with the chat
open or closed. `goto`, `goal`, `path`, `stop` and a basic set of settings, plus the path drawn in the world.

**Next, in this order:** `mine`, `follow`, `explore`, then a persistent cache.

**Later:** builder, farm, elytra. Elytra would start from upstream's Java pathfinder, without the terrain generator files.

## Layout

```
Cargo.toml                 workspace, for development and tests only
mod/                       the native mod bundle
  horizon-mixin.toml       hooks, and the crate below
  runtime.rs               module added to the game: holds the state, connects the engine to the game
  tick.rs, command.rs ...  small hook helpers
  engine/                  the engine, a single crate
    Cargo.toml
    src/
      blocks.rs            block table, built from facts given by the glue
      world.rs             WorldView trait, and a mock for tests
      config.rs            settings snapshot, the only source of settings for a search
      costs.rs             ActionCosts
      movement/            helper.rs, then one file per upstream movement
      goals/
      search/              A*, open set, nodes, partial paths
      plan.rs              segment planning and splicing
      exec.rs              executor rules, returns intents
      process/             custom goal first, others later
      look.rs              aim model
tools/                     cost-compare, walk-test, and later comparisons
```

### Why one crate

A mod's crate can't depend on any path or git crate, and that includes Horizon's own crates. So the engine can't be split
into several local crates, and it can't name Horizon's types.

This helps us. The engine has to stay plain Rust, which the project rules want anyway, and its tests need no client
build. Editing a helper recompiles `horizon-client` but not the engine. The cost is that all game access goes through the
glue. Registry dependencies must already be in Horizon's `Cargo.lock`, so the engine starts with none.

The engine uses `#![forbid(unsafe_code)]`.

## How it runs

```
             game thread                                   worker thread

native_tick  read player and hotbar
             clone Arc<Chunk> around the player  -------->  search over a snapshot
             check for a finished search         <--------  best path so far, finished path
             exec.rs: rules -> Intent
             apply the Intent: yaw, keys, sprint, slot
```

- **World:** chunks are `Arc<Chunk>` holding `Arc<Section>` with copy on write, and the mesher already reads them from
  another thread. A search owns its clones, so it needs no locks. That the types are `Send` and `Sync` is inferred. The
  cost of copy on write under block updates is not measured.
- **Blocks:** the engine sees state ids and a table indexed by them. `mod/glue/state_facts.rs` builds it from Horizon's block
  data on the game thread, and it's shared read-only. `tools/facts-compare` compiles the same file and checks every field
  for every state. `isPathfindable` comes from `mob-data.json`, which Horizon exports from Java: `StateData.flags & LAND`.
- **Settings:** a search gets one immutable snapshot and reads nothing global.
- **Worker:** one thread, with new jobs replacing waiting ones. Results carry a generation number, so a stale result is
  dropped. A search has a node cap as well as a time limit.
- **Executor:** it reads a view of the world and returns an intent, and never touches the game itself. A separate context
  serves its cost rechecks, so nothing is shared with the worker.

## Hooks

Read at Nightly 2026-11-05, revision `66f3981` (`NATIVE_HOOKS_API` 411). `tools/walk-test` compiles against this revision but has not been run on it; the rest has been read, not run.

| Need | How | Stability |
|---|---|---|
| Tick, before the player moves | `native_tick`, called at the top of `tick_remote` | Stable |
| Commands | `native_command`, `native_commands` | Stable. Slash commands only |
| Mods list, settings page, HUD | `native_mods`, `native_settings`, `native_hud` | Stable |
| Drawing the path | `native_world_primitives`: lines and boxes, optionally through walls | Stable |
| Moving, with or without the chat open | `native_movement_input`: forward, strafe, jump, sneak. See below | Stable. Tested in single player: `tools/input-probe` and the S4 scenario (`tools/README.md`) |
| Reading blocks | Chunk snapshots from `game.world`, for the worker. `native_level_block_state` and `native_block` exist, but are too slow to copy a window with | Pinned for snapshots |
| Reading the player | `native_player` has position, yaw and pitch. Velocity, ground state and hotbar come from `game.player`, `game.inventory`, `game.selected` | Partly stable |
| Turning | `game.yaw`, `game.pitch`, which the movement packet reads. No stable setter | Pinned. Yaw tested at `4aa6854` |
| Sprinting | `PlayerMotion::set_sprinting`. `keys.sprint` comes from the keyboard and needs `Screen::Playing`, and `run_sprint_tick` may undo a direct set, so this needs checking again | Pinned. Tested by `tools/walk-test` at `4aa6854` only |
| Breaking, placing, slot | Through `game.left` and `tick_mining`, `game.right` and `use_item_as_player`, `game.selected`. Not one stable hook | Pinned. Not tested |
| Tool speed | `destroy_progress` and `block_break_speed`. Neither is public to a mod, so the glue needs its own access | Pinned. Not tested |
| `isPathfindable` | `StateData.flags & LAND`, in `mobs/data.rs`. Matches Java on every state. If `mob-data.json` is missing, Horizon falls back to guesses, so the mod has to refuse to path then | Pinned |
| `#goto` in chat | No client hook sees plain chat, so this needs a pinned `HEAD` on `handle_chat_input` | Pinned. Later |

A bundle that touches one private field has to pin `horizon_revision`, even if its other hooks are stable, so the mod stays pinned. Using the stable hooks where they exist still keeps the part that needs rebasing small.

**Horizon version.** The port targets `66f3981`, in the git-ignored checkout `upstream/horizon-nightly`. `../horizon/` stays at `4aa6854` for Eris. We stay on one Nightly for the length of a stage and update between stages.

### Walking with the chat open

This is a requirement: players use Baritone while they talk or type commands.

At `66f3981`, `tick_remote` still gives zero movement keys while a screen is open, because `playing` requires `Screen::Playing`. But it then calls `native_movement_input`, with no such condition, and rewrites the keys from what the hook returns before it builds the `PlayerInput` byte for the server. So from reading the code, a mod can walk with the chat open using a stable hook, and the server sees the same keys as the movement. This replaces the `MODIFY_VARIABLE` idea from `4aa6854`.

Measured in the game on 2026-11-06 at `66f3981`, in a single-player world, by `tools/input-probe` (see [its section](../tools/README.md#input-probe)). The mod drove the player from `native_movement_input` (forward, strafe, jump, sneak; yaw through `game.yaw`; sprint through `PlayerMotion::set_sprinting` from `native_tick`), with 60 ticks per trial, and logged every tick. Each row below was measured with the screen named; "game read" is what the game itself gave the hook before it was overwritten, and it was zero in every trial.

| Screen | Forward | Jump | Sneak | Strafe | Yaw | Sprint | Speed (blocks/s) |
|---|---|---|---|---|---|---|---|
| none, driven through the same hook (the control) | works | not run | not run | not run | not run | holds | walk 4.32, sprint 5.61 |
| chat | works | works (jump 1.25 blocks high) | works | works | works (west at yaw 90) | holds | walk 4.32, sprint 5.61, sneak 1.29 |
| inventory | works | not run | not run | not run | not run | holds | walk 4.32, sprint 5.61 |

- **The hook runs with a screen open.** `hook_calls` is 1 per tick in all 1,649 rows, with the chat or the inventory open, and the game's own movement is 0 there (the idle controls do not move at all).
- **Sprint holds with a screen open, and `keys.sprint` is not needed.** In `sprint.rs`, `keys.sprint` only starts a sprint; the sprint is a flag on the player, and it stops by itself when forward is released, on a real collision, or when it becomes impossible (food, water). With the hook giving forward every tick, one `set_sprinting(true)` kept the flag and the speed modifier for all 60 ticks, in the chat and in the inventory, the same as setting it every tick. Releasing forward ended it on the next tick, with the screen still open. Sprint-jumping works and is not a rubber band: the one-tick burst at take-off (about 12 blocks/s) comes from the game's own jump boost.
- **The integrated server accepts it.** After each trial the server's position of the player (`data get entity @p Pos`, run as the owner) was exactly one tick of movement behind the client's, in every trial, with a screen open or not (0.216 blocks at walking speed, 0.281 at sprinting speed, 0.065 sneaking), and the client never jumped back: the speed between two ticks after the first ticks changes by at most 0.07 blocks/s, except in the jump trials, where it is the jump itself. The log has no warning. This is the integrated server only; a remote server's anticheat is a different test (S5).
- **Order inside a tick.** The hook runs inside `tick_remote_level`, before the player moves; `native_tick` runs after it. So a `set_sprinting(true)` from `native_tick` takes effect on the next tick's movement, and the first driven tick walks. The executor should set sprint on the tick before it wants it.
- **Real keys during a screen (from the code).** `held(k)` is `playing && keys.contains(k)`, and `playing` needs `Screen::Playing`, so a key the player holds while the chat or the inventory is open reaches the hook as zero. The key is still recorded (`keys.press`), and `capture(true)` calls `restore_held_keys` when the screen closes, so a key still held counts again from the next tick (if the window has focus; `capture(true)` returns early without it). Opening the chat clears `game.keys`. With no screen open, the hook gets the player's real keys in its arguments first, so the executor can decide there whether to keep them, combine them or override them. Nothing in the hook tells which case it is, so the glue needs `game.screen` for that.
- **Other screens.** Only the chat and the inventory were run. `Screen::Inventory` was set directly, like the E key does in survival, but not with the key. A container opened by the server, the pause screen or a dialog were not run; the pause screen stops the game in single player anyway, and a dialog (`dialog_open()`) also clears `playing`, so by the code they behave like the inventory, which is not a measurement.
- **Not covered.** Creative mode (E opens `Screen::Creative`), flying, water, a remote server, and the real keyboard. The probe did not use the chat's own input path (typing, sending), so it says nothing about a command typed while a path runs.

The consequences for S4 and the executor: write the input through `native_movement_input` only, set `game.yaw` from the tick, set sprint from `native_tick` one tick ahead, and do not depend on `keys.sprint`. The open questions are closed, and the `MODIFY_VARIABLE` idea is not needed.

## Testing

Each stage in [Porting safety](PORTING-SAFETY.md#stages) gives the evidence needed. In short:

- **Engine:** `cargo test`, with no game and no Minecraft data.
- **Rules:** bit-exact comparison with upstream's Java, generated locally.
- **Horizon's data:** its block facts against Java's, for every state.
- **Whole searches:** same world and goal give the same path, node by node, except for recorded deviations.
- **In game:** fixed scenarios in a separate instance, including the chat open.
- **Servers:** anarchy servers that allow utility clients, such as 6b6t.org.

Building the mod takes 2 to 5 minutes on this laptop, so most work should need no client build.

## Known risks

| Risk | Consequence | Found out at |
|---|---|---|
| The input hook fails, or servers reject the input pattern | The executor and glue change, the engine doesn't | S4, S5 |
| Snapshots cost too much under block updates | A copied window replaces them, in the world layer only | S4 |
| Horizon's private fields change often | More rebasing work, kept to the glue | Each rebase |
| The physics differ beyond flat walking and sprinting | Some cost constants need new measurements | S4 |
| Effort is known only for direct translation | S1 is measured in [PORTING-SAFETY.md](PORTING-SAFETY.md#s1-result), but it doesn't predict the rewritten parts, so there is still no calendar | S3, S4 |

**L3 result at `66f3981` (2026-11-06, `reports/l3-66f3981.json`, details in [tools/README.md](../tools/README.md#l3-scenarios)).** 18 runs passed, in new
single-player worlds, with the chat closed and open. Walking, sprinting, steps, diagonals and drops of 1 to 3 blocks took about the ticks the
costs say (0.94 to 1.0 of the estimate); a step up took 1.34 times its estimate. Swimming did not: movements that touch deep water took
about 2.2 times their estimate in the natural-terrain walks (a shallow one-block wade did not show this). That is a cost constant to
measure, not changed here. The search worker took 0.2 to 4 ms for a short path and up to 160 ms for a 160-block segment; a snapshot of
637 chunks took 60 to 160 microseconds. Not measured: the cost of copy-on-write while a snapshot is held.

## Decisions

1. **Scope of the first release:** as listed above.
2. **Command syntax:** slash commands first (`/b goto ...`), because `native_command` only sees those. Upstream's `#goto`
   would need another hook, not studied yet, and can come later if one exists.
3. **Inputs while a path runs:** keep upstream's behaviour when a player presses keys. The owner expects it to be somewhat
   buggy, so it needs checking in upstream's code and will take work.

   *Read in S4 (`InputOverrideHandler`, `PlayerMovementInput` and `PathingBehavior` at 1b8e87c; not played):*
   - Baritone is "in control" (`inControl`) when any of forward, back, left, right, sneak or jump is forced, or when a path is being
     followed (`isPathing()`). While it is, the player's keyboard input is replaced completely by `PlayerMovementInput`, which reads only the forced inputs.
     The keys the player holds (movement, jump, sneak, and the sprint key) are not read at all. A player cannot steer, jump or stop a path with keys; `stop` is the way out.
   - Out of control (no path, nothing forced), the keyboard is put back on the next tick, and keys still held count again at once.
   - The mouse is not replaced, but the executor sets the yaw it wants every tick (upstream does that server-side with free look; the glue sets `game.yaw`, so the camera is held while a path runs).
   - The sprint state is overridden by `isSprinting()` during a path (`onPlayerSprintState`), `flying` is set to false on every movement tick, and the game's auto jump is off while a path runs.
   - Attack and use are not replaced; only a movement that forces them (placing, breaking, opening a door) presses them.
   - In this port: `Intent::active` says "in control". When it is false the glue must leave the player's keys alone. When it is true, the glue feeds the hook from the intent only,
     and ignores the player's own movement keys. `Intent::cancel_flying` and `disable_auto_jump` carry the last two.
