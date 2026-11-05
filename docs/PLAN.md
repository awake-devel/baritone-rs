# Plan

**Status: approved on 2026-11-05.** How [Porting safety](PORTING-SAFETY.md) checks this work is a separate
document. Horizon facts below were read at revision `4aa6854`. None of this design has been built, except where a test
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
- **Blocks:** the engine sees `u32` state ids and a table indexed by them. The glue builds it from Horizon's `StateFacts`
  on the game thread, and it's shared read-only. `isPathfindable` has no known source yet: Horizon has a trait declaration
  for it, and S2 has to settle this.
- **Settings:** a search gets one immutable snapshot and reads nothing global.
- **Worker:** one thread, with new jobs replacing waiting ones. Results carry a generation number, so a stale result is
  dropped. A search has a node cap as well as a time limit.
- **Executor:** it reads a view of the world and returns an intent, and never touches the game itself. A separate context
  serves its cost rechecks, so nothing is shared with the worker.

## Hooks

| Need | How | Stability |
|---|---|---|
| Tick, commands, mods list, settings page, HUD | `native_tick`, `native_command`, `native_commands`, `native_mods`, `native_settings`, `native_hud` | Stable |
| Drawing the path | A native render hook, not chosen yet | Stable |
| Reading the player and chunks | `game.player`, `game.world` | Private fields, pinned |
| Moving and turning | `game.keys`, `game.yaw` | Private fields, pinned. Tested by `tools/walk-test` |
| Sprinting | `PlayerMotion::set_sprinting`. The sprint key can't work: `heal_hold_latches` releases it while the keyboard reports Control up | Pinned. Tested by `tools/walk-test` |
| Walking with the chat open | See below | Pinned. **Not tested** |
| Breaking, placing, slot | Not studied yet | Unknown |

Private fields mean a rebase on each new Horizon revision, so the glue stays small and all in one place. Current main also
has `native_movement_input`, which has no sprint or look parameter, so it wouldn't remove all of this. We develop on the
pin and evaluate a move at S4.

### Walking with the chat open

This is a requirement: players use Baritone while they talk or type commands.

`tick_remote` builds `Keys` with `playing && self.keys.contains(..)`, so an open screen clears all movement. It then
computes the input packet from those keys, so patching only the later call to `controlled_simulation_tick_with_sprint_window`
would make the player move while telling the server no keys are held.

So the hook has to replace `keys` right after it's built, with `MODIFY_VARIABLE` or `AFTER_ASSIGN`, while a path is running.
What a real player's keys should do in that case still has to be decided, as does what happens with other screens, such as
an inventory or a container. S4 starts with this test, because the executor depends on it.

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
| `StateFacts` doesn't cover what the rules need | The predicate layer costs much more | S2 |
| The input hook fails, or servers reject the input pattern | The executor and glue change, the engine doesn't | S4, S5 |
| Snapshots cost too much under block updates | A copied window replaces them, in the world layer only | S4 |
| Horizon's private fields change often | More rebasing work, kept to the glue | Each rebase |
| The physics differ beyond flat walking and sprinting | Some cost constants need new measurements | S4 |
| Effort is unknown | No calendar is given until S1 has been measured | S1 |

## Decisions

1. **Scope of the first release:** as listed above.
2. **Command syntax:** slash commands first (`/b goto ...`), because `native_command` only sees those. Upstream's `#goto`
   would need another hook, not studied yet, and can come later if one exists.
3. **Inputs while a path runs:** keep upstream's behaviour when a player presses keys. The owner expects it to be somewhat
   buggy, so it needs checking in upstream's code and will take work.
