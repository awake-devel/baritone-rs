# Design and plan

How the port is built: what is translated and what is written new, the layout, how it runs inside the game, and the hooks it
uses. [Porting safety](PORTING-SAFETY.md) is how the work is checked, and [RESULTS.md](RESULTS.md) is what each stage found.
Horizon facts below were read at revision `66f3981`.

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

## What exists, and what comes next

**First release (v0.1.0):** walking, sprinting, breaking and placing along a computed path, on a pinned Horizon revision, with
the chat open or closed. `goto`, `goal`, `path`, `stop`, `eta`, `set` and `help`, plus the path drawn in the world.

**Since then, on `main` (R2):** `mine`, `goto <block>`, `follow`, the persistent chunk cache and `explore`.

**In progress (R3):** `build` is ported (schematics, masks, the `.schematic`, `.schem` and `.litematic` formats), and so is moving stacks from the main
inventory to the hotbar (`allowInventory`, off by default as upstream; the window click is in the glue, which is not compiled yet, and nothing of it has run in the
game). Elytra flight
comes after it and is written new: it takes the general idea of upstream's elytra flight and none of the code of
nether-pathfinder or of its Java port.

**Later (R4):** `farm`, written new rather than ported.

## Layout

```
Cargo.toml                 workspace, for development and tests only
mod/                       the native mod bundle
  horizon-mixin.toml       hooks, and the crate below
  runtime.rs               module added to the game: holds the state, connects the engine to the game
  tick.rs, command.rs ...  small hook helpers
  glue/                    the block facts table, and the completion module added to the command crate
  engine/                  the engine, a single crate
    src/
      blocks.rs            block table, built from facts given by the glue
      world.rs             WorldView trait, and a mock for tests
      config.rs, settings.rs, set.rs   the settings, their snapshot for a search, and the `set` command
      costs.rs             ActionCosts
      movement/            helper.rs, then one file per upstream movement
      goals.rs
      search/              A*, open set, nodes, partial paths
      plan.rs              segment planning and splicing
      exec.rs, exec/       executor rules; they return intents
      look.rs, freelook.rs, raytrace.rs   the aim
      inventory.rs         tool and throwaway choice
      worker.rs            the search thread
      scan.rs, blockmeta.rs   finding blocks, for `mine` and `goto <block>`
      cache/               the persistent chunk cache, in upstream's region format
      process/             custom goal, get to block, mine, follow, explore, build
      schematic/           schematics, masks and the file formats
      command.rs           the words after `/b`
      sim.rs               a small simulator for tests
tools/                     the comparisons with upstream's Java, the probes, the in-game scenarios, the gate
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

Read at Nightly 2026-11-05, revision `66f3981` (`NATIVE_HOOKS_API` 411). Every row is in use by the mod and runs in the in-game scenarios, except where it says otherwise.

| Need | How | Stability |
|---|---|---|
| Tick, before the player moves | `native_tick`, called at the top of `tick_remote` | Stable |
| Commands | `native_command`, `native_commands` | Stable. Slash commands only. `horizon-ctl chat "/b ..."` reaches `native_command` at `66f3981` |
| Completion of the words after `/b` | A `RETURN` on `horizon_command::Dispatcher::client_suggestions_asking` replaces the tree's list (it has nothing past the name) with the mod's. The mod hands in a function through a small module added to `horizon-command` (`mod/glue/suggest.rs`, `baritone_rs_suggest`), because the chat box (`horizon-client-render`) is below the mod's crate and cannot call it. The block under the crosshair comes from `Registries::target_block`, which the chat fills when it opens | Pinned. Tested in the game (screenshots), [RESULTS.md](RESULTS.md#breaking-and-placing-the-glue). `docs/horizon-suggestions/command-argument-completion.md` describes the stable hook we would rather have |
| Mods list, settings page, HUD | `native_mods`, `native_settings`, `native_hud` | Stable |
| Drawing the path | `native_world_primitives`: lines and boxes, optionally through walls | Stable |
| Moving, with or without the chat open | `native_movement_input`: forward, strafe, jump, sneak. See below | Stable. Tested in single player: `tools/input-probe` and the S4 scenario (`tools/README.md`) |
| Reading blocks | Chunk snapshots from `game.world`, for the worker. `native_level_block_state` and `native_block` exist, but are too slow to copy a window with | Pinned for snapshots |
| Reading the player | `native_player` has position, yaw and pitch. Velocity, ground state and hotbar come from `game.player`, `game.inventory`, `game.selected` | Partly stable |
| Turning | `game.yaw`, `game.pitch`, which the movement packet reads. No stable setter | Pinned. Tested |
| Sprinting | `PlayerMotion::set_sprinting`, set from `native_tick` one tick ahead. `keys.sprint` is not needed: the sprint is a flag on the player and holds while forward is given | Pinned. Tested with the chat closed and open, [RESULTS.md](RESULTS.md#walking-with-the-chat-open) |
| Slot | Set `game.selected`. `remote_send_state` sends `SetCarriedItem` on the next tick whenever it differs from the last sent slot, with any screen open | Pinned (private field). Tested, [RESULTS.md](RESULTS.md#breaking-and-placing) |
| Breaking, placing | `game.left` and `game.right` are read by `remote_handle_input` (private, in `game/remote.rs`), which only runs its input step with `Screen::Playing`. A pinned `HEAD` and `RETURN` on it set both buttons and put `Screen::Playing` in for that one call. Not `tick_mining` or `use_item_as_player`: those serve the integrated server's own player, not the window's | Pinned. Tested with the chat closed and open, [RESULTS.md](RESULTS.md#breaking-and-placing) and [the glue](RESULTS.md#breaking-and-placing-the-glue) |
| Hotbar, tool numbers | `game.inventory[0..9]`, `game.offhand`, `game.items()` (`mining_speed`, `correct_for_drops`, `item_in_tag`), `game.stations().enchantments.attributes(stack)` for efficiency, `game.potions.effects.amplifier("haste")`, `game.res.facts()` for hardness | Pinned (private fields and methods) |
| Moving a stack to the hotbar (`allowInventory`) | `Game::menu_ops` (private, in `game.rs`) in single player and `Game::remote_menu_ops` (`pub(super)`, in `game/remote.rs`) on a server, with `(Some(slot), hotbar, Input::Swap)` and Horizon's own slot numbers (0 to 8 the hotbar, 9 to 35 the rest); `Game::remote_menu()` says whether a server menu is open. The module is injected into `game.rs`, so it reaches both. No new hook | Pinned (private and `pub(super)` methods). Written from the source at `66f3981`, not compiled, not run |
| The block under the crosshair | `game.pick()` (private; the `Hit` has the block, the face as a normal and the point) | Pinned |
| Settings file | `game.native_data_dir(id)` (a `pub(super)` method of `native_hooks.rs`): `horizon-mods/.horizon-mod-data/baritone_rs/settings.txt` in the instance's game folder | Pinned (the folder itself is documented as a mod's own) |
| Tool speed | `game.destroy_progress(pos)` (`pub(super)`, so a module injected into `game.rs` can call it), and the public `Items::mining_speed` and `Items::correct_for_drops` | Pinned. Tested: agrees with the measured break time |
| `isPathfindable` | `StateData.flags & LAND`, in `mobs/data.rs`. Matches Java on every state. If `mob-data.json` is missing, Horizon falls back to guesses, so the mod has to refuse to path then | Pinned |
| `#goto` in chat | No client hook sees plain chat, so this needs a pinned `HEAD` on `handle_chat_input` | Pinned. Not done |

A bundle that touches one private field has to pin `horizon_revision`, even if its other hooks are stable, so the mod stays pinned. Using the stable hooks where they exist still keeps the part that needs rebasing small.

**Horizon version.** The port targets `66f3981`, in the git-ignored checkout `upstream/horizon-nightly`. We stay on one Nightly for the length of a stage and update between stages.

Measurements of the hooks in the game (walking with the chat open, breaking and placing) are in [RESULTS.md](RESULTS.md#walking-with-the-chat-open).

## Testing

Each stage in [Porting safety](PORTING-SAFETY.md#stages) gives the evidence needed. In short:

- **Engine:** `cargo test`, with no game and no Minecraft data.
- **Rules:** bit-exact comparison with upstream's Java, generated locally.
- **Horizon's data:** its block facts against Java's, for every state.
- **Whole searches:** same world and goal give the same path, node by node, except for recorded deviations.
- **In game:** fixed scenarios in a separate instance, including the chat open.
- **Servers:** anarchy servers that allow utility clients, such as 6b6t.org.

Building the mod takes 2 to 5 minutes on the development machine, so most work should need no client build.

## Known risks

| Risk | Consequence | Found out at |
|---|---|---|
| The input hook fails, or servers reject the input pattern | The executor and glue change, the engine doesn't | S4, S5 |
| Snapshots cost too much under block updates | A copied window replaces them, in the world layer only | S4 |
| Horizon's private fields change often | More rebasing work, kept to the glue | Each rebase |
| The physics differ beyond flat walking and sprinting | Some cost constants need new measurements | S4 |
| Effort is known only for direct translation | S1 is measured in [RESULTS.md](RESULTS.md#s1-result), but it doesn't predict the rewritten parts, so there is still no calendar | S3, S4 |

## Decisions

1. **Scope of the first release:** as listed at the top.
2. **Command syntax:** slash commands first (`/b goto ...`), because `native_command` only sees those. Upstream's `#goto`
   would need another hook, not studied yet, and can come later if one exists.
3. **Inputs while a path runs:** keep upstream's behaviour when a player presses keys.

   *Read in S4 (`InputOverrideHandler`, `PlayerMovementInput` and `PathingBehavior` at 1b8e87c; not played):*
   - Baritone is "in control" (`inControl`) when any of forward, back, left, right, sneak or jump is forced, or when a path is being
     followed (`isPathing()`). While it is, the player's keyboard input is replaced completely by `PlayerMovementInput`, which reads only the forced inputs.
     The keys the player holds (movement, jump, sneak, and the sprint key) are not read at all. A player cannot steer, jump or stop a path with keys; `stop` is the way out.
   - Out of control (no path, nothing forced), the keyboard is put back on the next tick, and keys still held count again at once.
   - The mouse is not replaced. With `freeLook` (upstream's default) the executor's rotation is only put in for the player's own tick, so the camera stays the player's own (D35); with it off the glue sets `game.yaw` every tick and the camera is held while a path runs.
   - The sprint state is overridden by `isSprinting()` during a path (`onPlayerSprintState`), `flying` is set to false on every movement tick, and the game's auto jump is off while a path runs.
   - Attack and use are not replaced; only a movement that forces them (placing, breaking, opening a door) presses them.
   - In this port: `Intent::active` says "in control". When it is false the glue must leave the player's keys alone. When it is true, the glue feeds the hook from the intent only,
     and ignores the player's own movement keys. `Intent::cancel_flying` and `disable_auto_jump` carry the last two.
