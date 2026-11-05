# baritone-rs

A Rust port of [Baritone](https://github.com/cabaletta/baritone), the Minecraft pathfinder, for
[Horizon](https://github.com/horizon-rust/horizon), the Rust rewrite of Minecraft: Java Edition 26.3.

## Status

**Nothing is ported yet, and there is no mod to use.** This repository holds the license, the credits, two test tools
described in [tools/README.md](tools/README.md), the [plan](docs/PLAN.md) and the way the port gets checked,
[Porting safety](docs/PORTING-SAFETY.md).

## Upstream

The port is based on [`cabaletta/baritone`](https://github.com/cabaletta/baritone), the original project:

| Upstream branch | Commit | Role |
|---|---|---|
| `1.21.4` | `1b8e87cac675d72d8094e2bea1ec7207cdbc971b` | Default upstream branch, source of the pathing rules, which have the latest fixes |
| `26.3` | `25111daedf1d59e6a8dfb5a3e61885cdb8d953df` | Reference for game behavior and block names, same Minecraft version as Horizon |

Commits were recorded on 2026-11-05. Each ported file will name the upstream commit it came from, as
described in [CREDITS.md](CREDITS.md), and that commit may differ from one file to another.

## Credits

Baritone is the work of Leijurv, Brady and the many other people listed in [CREDITS.md](CREDITS.md). rfresh2
wrote the upstream change that brought Baritone to Minecraft 26.3, the version Horizon targets. The elytra
pathfinder comes from babbaj's nether-pathfinder, through the Java port by Mattias Rask.

Bugs in this port belong to this port. Please do not report them to the Baritone project.

## License

baritone-rs is a derivative work of Baritone and is licensed under the **GNU Lesser General Public License,
version 3 or (at your option) any later version** (`LGPL-3.0-or-later`), the same terms under which upstream
files are published. See [LICENSE](LICENSE). The LGPL builds on the GNU General Public License, version 3,
included as [LICENSE-GPL-3.0](LICENSE-GPL-3.0).

[CREDITS.md](CREDITS.md) has notes on upstream's licensing.

**Elytra pathfinder.** Elytra flight will be ported from the Java port of
[nether-pathfinder](https://github.com/babbaj/nether-pathfinder) in upstream's `1.21.4` branch. Those files
carry Baritone's LGPL notice, and this project assumes the same license applies to them. The nether-pathfinder
repository itself had no license file on 2026-11-05, so this is an assumption rather than an explicit grant.
That assumption does not extend to its nether terrain generator, which upstream describes as coming from
Minecraft: those files will not be ported. See [CREDITS.md](CREDITS.md#elytra-pathfinder).

## Disclaimer

baritone-rs is an independent project, not affiliated with or endorsed by the Baritone project, Horizon,
Mojang or Microsoft. Minecraft is a trademark of Mojang Synergies AB.

This program comes with no warranty, to the extent permitted by law. See the license for details.
