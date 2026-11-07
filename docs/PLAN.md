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
| Commands | `native_command`, `native_commands` | Stable. Slash commands only. `horizon-ctl chat "/b ..."` reaches `native_command` at `66f3981` (earlier notes said it did not) |
| Completion of the words after `/b` | A `RETURN` on `horizon_command::Dispatcher::client_suggestions_asking` replaces the tree's list (it has nothing past the name) with the mod's. The mod hands in a function through a small module added to `horizon-command` (`mod/glue/suggest.rs`, `baritone_rs_suggest`), because the chat box (`horizon-client-render`) is below the mod's crate and cannot call it. The block under the crosshair comes from `Registries::target_block`, which the chat fills when it opens | Pinned. Tested in the game (screenshots), [below](#breaking-and-placing-the-glue). `docs/horizon-suggestions/command-argument-completion.md` describes the stable hook we would rather have |
| Mods list, settings page, HUD | `native_mods`, `native_settings`, `native_hud` | Stable |
| Drawing the path | `native_world_primitives`: lines and boxes, optionally through walls | Stable |
| Moving, with or without the chat open | `native_movement_input`: forward, strafe, jump, sneak. See below | Stable. Tested in single player: `tools/input-probe` and the S4 scenario (`tools/README.md`) |
| Reading blocks | Chunk snapshots from `game.world`, for the worker. `native_level_block_state` and `native_block` exist, but are too slow to copy a window with | Pinned for snapshots |
| Reading the player | `native_player` has position, yaw and pitch. Velocity, ground state and hotbar come from `game.player`, `game.inventory`, `game.selected` | Partly stable |
| Turning | `game.yaw`, `game.pitch`, which the movement packet reads. No stable setter | Pinned. Yaw tested at `4aa6854` |
| Sprinting | `PlayerMotion::set_sprinting`. `keys.sprint` comes from the keyboard and needs `Screen::Playing`, and `run_sprint_tick` may undo a direct set, so this needs checking again | Pinned. Tested by `tools/walk-test` at `4aa6854` only |
| Slot | Set `game.selected`. `remote_send_state` sends `SetCarriedItem` on the next tick whenever it differs from the last sent slot, with any screen open | Pinned (private field). Tested, [below](#breaking-and-placing) |
| Breaking, placing | `game.left` and `game.right` are read by `remote_handle_input` (private, in `game/remote.rs`), which only runs its input step with `Screen::Playing`. A pinned `HEAD` and `RETURN` on it set both buttons and put `Screen::Playing` in for that one call. Not `tick_mining` or `use_item_as_player`: those serve the integrated server's own player, not the window's | Pinned. Tested with the chat closed and open, [below](#breaking-and-placing) and [the glue](#breaking-and-placing-the-glue) |
| Hotbar, tool numbers | `game.inventory[0..9]`, `game.offhand`, `game.items()` (`mining_speed`, `correct_for_drops`, `item_in_tag`), `game.stations().enchantments.attributes(stack)` for efficiency, `game.potions.effects.amplifier("haste")`, `game.res.facts()` for hardness | Pinned (private fields and methods) |
| The block under the crosshair | `game.pick()` (private; the `Hit` has the block, the face as a normal and the point) | Pinned |
| Settings file | `game.native_data_dir(id)` (a `pub(super)` method of `native_hooks.rs`): `horizon-mods/.horizon-mod-data/baritone_rs/settings.txt` in the instance's game folder | Pinned (the folder itself is documented as a mod's own) |
| Tool speed | `game.destroy_progress(pos)` (`pub(super)`, so a module injected into `game.rs` can call it), and the public `Items::mining_speed` and `Items::correct_for_drops` | Pinned. Tested: agrees with the measured break time |
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

### Breaking and placing

Measured in the game on 2026-11-07 at `66f3981`, in a new single-player world in survival, by `tools/break-place-probe` (see [its section](../tools/README.md#break-place-probe)). Two full runs gave the same results to within one tick; the table is the second. The probe gives the player an iron pickaxe, 64 stone and a water bucket through owner console commands, sets up the blocks the same way, and asks the server about the result (`execute if block`, `data get entity @p SelectedItem`) every tick. "Chat" means the chat screen was open the whole time. Server times are one tick coarser than client times because the server is asked once per tick.

**Why the chat needs a trick.** `tick_remote_level` calls `remote_handle_input` every tick before the player moves. It turns `game.left` and `game.right` into mining and use, but only when `screen == Screen::Playing`; with any other screen it aborts a running dig (`stop_remote_mining`, which sends the abort packet) and does nothing else. `native_tick` is too late and `remote_mining` is private to `game/remote.rs`. So the probe puts a `HEAD` and a `RETURN` hook on `remote_handle_input` (`input_head.rs`, `input_return.rs`). At the head it sets `game.left` and `game.right` from the mod's intent and, with the chat open, sets `game.screen = Screen::Playing`; at the return it puts `Screen::Chat` back. The game's own code then does everything a real held mouse button does, for the player, the server and the prediction. Aim is `game.yaw` and `game.pitch`, set from `native_tick`; they act in the next tick's input step, and the ray starts from the eye position at the end of the previous tick (the input step runs before the player moves). Opening the chat clears `left` and `right`, so the hook sets them every tick.

The control: the same buttons with the chat open and **no** screen change did nothing in all three kinds. The iron pickaxe held on stone for 60 ticks broke nothing, the use button at the floor placed nothing, and six use presses at the top of a jump in the pillar trial placed nothing. So the screen swap is what makes it work, not a side effect.

| What | Screen | Result |
|---|---|---|
| Slot 1, 2, 8 (empty), 0 set through `game.selected` | none / chat with swap / chat without swap | The server's `SelectedItemSlot` and `SelectedItem` were right 4 ticks later in every case, including the empty hand (`Found no elements matching SelectedItem`). No swap is needed for the slot |
| Dirt, empty hand | none / chat | Client saw air after 16 / 20 ticks, server 17 / 21. Drop: one dirt item |
| Stone, iron pickaxe | none / chat | 14 / 13 ticks, server 15 / 14. Drop: one cobblestone. The pickaxe took 1 damage on the server |
| Stone, empty hand | none / chat | 157 / 156 ticks (7.8 s), server 158 / 157. Drop: nothing, as in vanilla |
| Stone, iron pickaxe, chat, no swap (control) | chat | Nothing in 60 ticks |
| Stone from slot 1, aimed at the floor in front (yaw 0, pitch 39) | none / chat | Block at the target after 4 ticks, server 1 tick later, stack 64 to 63. Control without swap: nothing |
| Pillar: jump (through `native_movement_input`), look down (pitch 90), press use once when the feet pass y = 101.02 | none / chat | Placed on the first press (feet at y = 101.166), the server had it a tick later, and the player landed on it (server `Pos` y = 101.0). Control without swap: six presses, nothing |
| Water bucket onto the floor in front, then the same click again | none / chat | Water at the target after 4 ticks, server 1 tick later; the server's hand became an empty bucket. The second click turned the hand back into a water bucket on the server, and the client showed air at the target 1 tick after the click. The server still had water there until tick 39, which is the flowing water around the source decaying (the source and its neighbours had spread), not a failed pick-up |

**Break time.** The game's own `destroy_progress(pos)` was read for the target on the first tick of each trial and compared with the ticks the game then took:

| Held item, block | hardness | `mining_speed` | `correct_for_drops` | `destroy_progress` per tick | ceil(1 / progress) | Vanilla | Measured (client, chat closed) |
|---|---|---|---|---|---|---|---|
| none, dirt | 0.5 | 1 | false | 0.066667 | 15 | 0.75 s | 16 (first break of the run) |
| iron pickaxe, stone | 1.5 | 6 | true | 0.133333 | 8 | 0.4 s | 14 |
| none, stone | 1.5 | 1 | false | 0.006667 | 150 | 7.5 s | 157 |

`destroy_progress` is exactly `mining_speed / hardness / (30 if the tool is right for the drops, else 100)`, times the break-speed attribute (1 here), with the factors for water, not standing on the ground (1/5) and effects added when they apply. The three values above come out of that formula by hand, so a mod can compute it for any stack from `Items::mining_speed`, `Items::correct_for_drops` and the block's hardness without touching a private function. Only the held stack was measured; evaluating another hotbar slot (upstream's `ToolSet` does) by setting `game.selected` for a moment and calling `destroy_progress` is not tested. The break takes the ticks the number says, plus one for the tick that sends the start packet. The first break of the run took exactly that (1 + 15). The others took 4 to 6 ticks more: the game waits `destroy_delay` (5) ticks after a break, and it only counts them down while the attack button is held (`remote_mining(false)`). That reading of the code fits the numbers but was not isolated. A path that breaks blocks one after another therefore pays about 5 ticks between blocks, as vanilla does.

**Not covered.** A remote server (its own break timing and anticheat, packet sequence numbers under latency, lag compensation): this was the integrated server only. Creative mode, sneaking, placing against a side face or a block that needs a different hand, falling blocks, a tool that breaks mid-dig, haste, efficiency and underwater mining (the code handles them in `destroy_progress`; no trial used them), breaking while the player moves, and picking up the drops. What stops a running dig when the mod stops (`/b stop`): not tested; with the chat closed the game's own `left = false` ends it, and with the chat open the input step has to run once more with `left` off, or `request_remote_mining_stop` has to be called.

**What the next stage can rely on.** Write `game.selected`, `game.yaw`, `game.pitch` and the buttons from `native_tick` (they act in the next tick's input step); set `game.left` or `game.right` for as long as the action should run (a place needs one press, not a hold: a held use repeats every 4 ticks); watch the client's world for the result; and give the pinned hooks on `remote_handle_input` a single owner in the glue. The hooks make the mod pinned to a Horizon revision, as `game.yaw` already does.

### Breaking and placing: the engine side

Done in the engine on 2026-11-07, with no client build, and **not connected to the glue yet**. It runs on the simulator in `mod/engine/src/sim.rs` (a path that breaks a wall, bridges a
gap with and without a floor beside it, pillars up, digs down; the same rules with the settings off), which is not evidence of the game. What the glue has to do:

- **Each tick, before `Pathing::tick`:** fill `PlayerView` (new field `looking_at`: the block the game's crosshair is on, with the face; `None` if unknown, but then bridging never places),
  pass the hotbar as `TickEnv::inv` (an `Inventory`: the slot in hand, the nine stacks as `SlotInfo`, the off hand, and `speed_vs_state`, which is `inventory::calculate_speed_vs_block` with
  the game's numbers) and `TickEnv::game` (`GameFacts`: block reach, the Nether, the aim processor). Rebuild `Tables` when the settings or the hotbar change: `Settings::apply_to_config`,
  `Settings::apply_inventory` (sets `has_throwaway` and `has_water_bucket`), the strengths from `inventory::get_str_vs_block`, and `Settings::apply_to_exec` for `ExecSettings`.
- **From the `Intent`, in `native_tick`:** `select_slot` (set `game.selected`), `rotation` as before, `attack` (hold `game.left` for the input step, with the `remote_handle_input` hooks of
  `tools/break-place-probe`), `use_item` (one press of `game.right`), `sneak` and `jump` as before. Let go of `left` when `attack` goes true to false and when the mod lets go of the player.
  `use_block` is still the door click.
- **Commands:** `command::goal_command`, `path_command`, `eta_command` (with `Pathing::in_control`, `ticks_remaining_in_segment` and `estimated_ticks_to_goal`), `Settings::run_set`, and
  `command::complete` for the words after `/b`. `Pathing::clear_goal` is `goal clear`.

### Breaking and placing: the glue

Done on 2026-11-07 (`mod/runtime.rs`, `mod/input_head.rs`, `mod/input_return.rs`, `mod/suggest.rs`, `mod/glue/suggest.rs`). It builds on `66f3981` into `baritone-test` (130 to 230 s)
and was smoke-tested in single-player worlds through `horizon-ctl` (`chat "/b ..."`, `command` for the setup, `key t tap` and `type` to open the chat and type into it, `screenshot`
to see the suggestions). Raw logs are in the scratchpad of the task, not in the repository. What each tick does:

- **Hotbar.** `read_hotbar` clones the nine stacks and the off hand, and the efficiency each one gives; `GameInv` answers the engine's `Inventory` from them (`ToolSet.calculateSpeedVsBlock` over
  `Items::mining_speed`, `correct_for_drops` and the block facts). The table of tool strengths is made per block, from the default state as upstream reads it, and again only when a
  `Sig` changes: the settings, the item and enchantments of each hotbar slot, whether a throwaway block and a water bucket are at hand, whether food allows sprinting (more than 6), the haste or
  fatigue amplifier. It took 4 to 36 ms in the game. Counts are not part of it, so placing blocks does not rebuild the table until a stack runs out.
- **Crosshair.** `PlayerView::looking_at` is `game.pick()`.
- **Intent.** `select_slot` sets `game.selected` (nothing is put back when the path ends, as upstream: only `MovementHelper`, `InventoryBehavior` and `MovementFall` set the slot). `attack` and `use_item` go to `BUTTONS`, which
  `on_input_head` copies to `game.left` and `game.right` at the start of the game's input step, with the chat swapped for `Screen::Playing` for that one call and put back in `on_input_return`. A button the mod held and no longer
  wants is let go of once, so a dig stops, and then the player's own mouse is theirs again. Only the chat is swapped; any other screen (the inventory) leaves the buttons without effect.
- **Commands.** `/b goto`, `goal`, `path`, `eta`, `set` (also `setting`, `settings`), `stop` (also `cancel`), `help`, `trace`. Texts are upstream's (`goal_command`, `path_command`, `eta_command`, `run_set`). Every change of a setting is saved
  at once, as upstream does, and the file is read the first time anything asks.
- **Completion.** `suggest_provider` answers the pinned hook with the text up to the cursor: the command names (and `trace`), the words of `goal`, the settings, `set`'s keywords and the values of a boolean, and `~` plus the coordinates of the block the crosshair is
  on (one, two or three of them, as vanilla offers them for `/tp`).

Results in the game (integrated server, iron pickaxe in slot 0, cobblestone in slot 1, `peaceful`, a stone platform with walls and gaps, the chat closed and open): see the report of the task for the numbers. Summary:

| Case | Chat closed | Chat open |
|---|---|---|
| `goto` through a 2-high stone wall, selected slot 1 (cobblestone) | Broke one block, switched to the pickaxe (slot 0), walked over the 1-high rest, arrived. The pickaxe lost 1 durability and one cobblestone was picked up | Same |
| `goto` across a 1-wide gap with parkour off | Placed one cobblestone, stack 64 to 63, arrived | Same |
| `goto` through a 3-high wall by hand, `allowPlace` off | not run | Dug (took about 12 s per two blocks) |
| `/b stop` in the middle of that dig | not run | The block was not broken in 14 s more (the control run broke it after about 12 s). The state's `destroying` list was empty in both runs, so this is not a view of the dig itself |
| `goto` 10 blocks down off an edge with a water bucket | not run | Landed with full health, and the bucket came back (the table's `has_water_bucket` went true, false, true) |
| `goto` over a 2-high obsidian wall with `allowBreak` on | Placed one cobblestone and climbed instead of mining obsidian by hand | not run |
| Tab completion: commands, `set allowB`, `set allowBreak `, `goto ` while looking at a block, `goto 1000 ` | not applicable | Right, by screenshot (the list is the chat box's own) |

Not run: creative mode, sneaking, picking up drops while pathing, `goto` with a falling block (`falling_block_near` is not answered), swimming and parkour (off), a remote server, any other screen than the chat, a pickaxe with
efficiency or a silk touch tool, haste or mining fatigue, `/b set load`, a second tool in the hotbar for the same block. `will_place_bucket` is still the earlier approximation (D16 section of `deviations.md`).

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
   - The mouse is not replaced. With `freeLook` (upstream's default) the executor's rotation is only put in for the player's own tick, so the camera stays the player's own (D35); with it off the glue sets `game.yaw` every tick and the camera is held while a path runs.
   - The sprint state is overridden by `isSprinting()` during a path (`onPlayerSprintState`), `flying` is set to false on every movement tick, and the game's auto jump is off while a path runs.
   - Attack and use are not replaced; only a movement that forces them (placing, breaking, opening a door) presses them.
   - In this port: `Intent::active` says "in control". When it is false the glue must leave the player's keys alone. When it is true, the glue feeds the hook from the intent only,
     and ignores the player's own movement keys. `Intent::cancel_flying` and `disable_auto_jump` carry the last two.
