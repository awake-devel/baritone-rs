# baritone-rs

A Rust port of [Baritone](https://github.com/cabaletta/baritone), the Minecraft pathfinder, for
[Horizon](https://github.com/horizon-rust/horizon), the Rust rewrite of Minecraft: Java Edition 26.3.

## Status

**The first pre-release, v0.1.0, has `goto`.** Everything else below is on `main` and not in a release yet. Ported, and run in the game:

- **Pathing:** walking, sprinting, jumping up steps, dropping down, ladders and vines, breaking blocks in the way, bridging,
  pillaring and digging down, with tool choice, throwaway blocks and water bucket falls. Parkour and swimming are off, as in
  upstream's defaults, and not ported yet.
- **Commands:** `/b goto`, `goal`, `path`, `stop`, `eta`, `set` and `help`, with tab completion of subcommands, settings and values.
  Settings follow upstream's names and defaults, and `freeLook` keeps the camera yours while Baritone steers.
- **Chunk cache:** the chunks the client loads are remembered (packed to 2 bits per block, saved to region files in upstream's format), so a path can be planned through
  terrain that is not loaded any more and `goto <chest, furnace, spawner, ...>` finds blocks seen earlier.
- **Processes:** `goto <block>`, `mine`, `follow` and `explore` (`explorefilter` takes a JSON list of chunks), as upstream's.
- **Builder:** `build <file> [x y z]` builds a `.schematic`, `.schem` (Sponge v1 and v2) or `.litematic` (v7) file from the `schematics`
  folder of the game folder, and `resume` carries on after a pause. Upstream's builder settings are in `set` (`buildInLayers`,
  `buildSubstitutes`, `buildRepeat`, `buildSchematicRotation`, ...). Selections (`sel`) are not ported. Moving blocks from the main inventory to the
  hotbar is (`allowInventory`, off by default as upstream, with `ticksBetweenInventoryMoves` and `inventoryMoveOnlyIfStationary`), but it has not been run in
  the game yet: with it off, a build uses what is on the hotbar.
- **Not yet:** farm, elytra.

**How it is checked.** Every movement rule gives upstream's answer bit for bit on 22 million cases, whole searches match upstream's
node by node on 29,446 searches, Horizon's block facts match Java's on all 35,723 block states, and 86 fixed scenarios pass in the
game in single player. It has also run on a real server (6b6t.org). The method and every result, with their limits, are in
[Porting safety](docs/PORTING-SAFETY.md) and [docs/RESULTS.md](docs/RESULTS.md); every difference from upstream is listed in [docs/deviations.md](docs/deviations.md).

**Speed.** On the same worlds, starts and goals, the port finds the same paths as upstream's Java search about **1.4 times faster**
(median over 51 searches on 20 seeds; per seed 1.05 to 1.84 times), and within the same time limit it explores 1.2 to 1.6 times
more nodes. One machine and synthetic worlds: see [docs/BENCHMARK.md](docs/BENCHMARK.md) for the method and the limits.

**Using it.** The mod is a Horizon native mod built with horizon-mixins. It follows Horizon Nightly `66f3981`, and it uses a few
pinned hooks, so a newer nightly may need an update first. Build it into a launcher instance with
`horizon-launcher native-build mod --instance <instance> --trust-native-code`, then type `/b help` in the game.

This repository also holds the [design and plan](docs/PLAN.md) and the test tools described in [tools/README.md](tools/README.md).

## Upstream

The port is based on [`cabaletta/baritone`](https://github.com/cabaletta/baritone), the original project:

| Upstream branch | Commit | Role |
|---|---|---|
| `1.21.4` | `1b8e87cac675d72d8094e2bea1ec7207cdbc971b` | Default upstream branch, source of the pathing rules, which have the latest fixes |
| `26.3` | `25111daedf1d59e6a8dfb5a3e61885cdb8d953df` | Reference for game behavior and block names, same Minecraft version as Horizon |

Commits were recorded on 2026-11-05. Each ported file names the upstream commit it came from, as
described in [CREDITS.md](CREDITS.md), and that commit may differ from one file to another.

## Credits

Baritone is the work of Leijurv, Brady and the many other people listed in [CREDITS.md](CREDITS.md). rfresh2
wrote the upstream change that brought Baritone to Minecraft 26.3, the version Horizon targets.


Bugs in this port belong to this port. Please do not report them to the Baritone project.

## License

baritone-rs is a derivative work of Baritone and is licensed under the **GNU Lesser General Public License,
version 3 or (at your option) any later version** (`LGPL-3.0-or-later`), the same terms under which upstream
files are published. See [LICENSE](LICENSE). The LGPL builds on the GNU General Public License, version 3,
included as [LICENSE-GPL-3.0](LICENSE-GPL-3.0).

[CREDITS.md](CREDITS.md) has notes on upstream's licensing.

**Elytra.** Elytra flight is not ported yet. It will be written new for this port: it takes the general idea of how upstream's
elytra flight behaves, and none of the code of [nether-pathfinder](https://github.com/babbaj/nether-pathfinder), the library
upstream plans its routes with, or of its Java port. See [CREDITS.md](CREDITS.md#elytra).

## Disclaimer

baritone-rs is an independent project, not affiliated with or endorsed by the Baritone project, Horizon,
Mojang or Microsoft. Minecraft is a trademark of Mojang Synergies AB.

This program comes with no warranty, to the extent permitted by law. See the license for details.
