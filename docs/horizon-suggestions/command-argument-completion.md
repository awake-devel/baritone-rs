# Suggestion for Horizon: completion of a mod's command arguments

Status: draft, not sent. Horizon takes ideas through its suggestion queue (playhorizon.dev/hub, or `/suggest` on Discord),
not through GitHub issues. Following our rule in `docs/PORTING-SAFETY.md` ("Keeping Horizon clean"), the request describes the need
and sends no code from this repository.

---

**Title:** Stable hook for completing the arguments of a mod's client commands

**What a native mod can do today.** `native_commands` (API 35) lets a mod name the client commands it owns, and the chat box
then colours, completes and lists those names like the game's own. Completion stops at the name: once the player has typed
`/b ` (`b` being the command), the chat box has nothing to offer for what comes next.

**What we'd like.** A stable hook that the chat box asks while the player types after a command the mod owns. It would get
the text typed so far and the cursor position, and could answer with suggestions. Each suggestion would have its text, the part
of the input it replaces, and optionally a short tooltip. The chat box would show them the way it shows vanilla's Brigadier
suggestions: the same list, the same Tab behaviour, and grey hint text for an argument that hasn't been started yet.

**Why.** Mods with several subcommands are tiring to use without it. Our case is a pathfinding mod with `/b goto <x> <y> <z>`,
`/b stop` and, soon, settings by name (`/b set <setting> <value>`). Completion would offer the subcommands, then setting names,
then valid values, and for coordinates the block the player is looking at or `~`, as vanilla does for `/tp`.

**Details that would help:**

- Suggestions computed on the game thread, so the mod can read the player's position and the world.
- Asked again as the text changes, as vanilla does for server commands.
- Kept apart from server commands: a mod-owned name is never sent to the server, so its completion shouldn't be either.

**What we use meanwhile.** A hook pinned to the current internal chat-suggestion code, which we update by hand when a nightly
changes it.
