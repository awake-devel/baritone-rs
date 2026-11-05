# Credits

baritone-rs is a port. The design and the algorithms it carries over come from the people below, not from
this repository.

## Baritone

[Baritone](https://github.com/cabaletta/baritone) was started by Leijurv in August 2018. By commit count on
the `26.3` branch, its largest contributors are Leijurv and Brady, followed by ZacSharp and wagyourtail.

## rfresh2

rfresh2 maintains a [fork of Baritone](https://github.com/rfresh2/baritone), contributes upstream, and wrote
the [change that brought upstream to Minecraft 26.3](https://github.com/cabaletta/baritone/pull/5130), the
version Horizon targets.

## All Baritone authors

These are the commit author names on upstream's `26.3` branch at commit
`25111daedf1d59e6a8dfb5a3e61885cdb8d953df`, sorted without regard to case, exactly as git records them. Names
that differ only by case are listed once. Other names may belong to the same person.

0x22, ads123ads, Argentoz, aUniqueUser, ave4224, aver, Azzar-Aixel, Babbaj, Baddeveloper, Bella, Bella Who,
Biscuit, BitForge, BlueBlazeTech, Brady, Brady Hahn, Bram Avontuur, ByteZ1337, cattyn, CDAGaming,
CompileRider, Conner, Conner Vercellino, CorruptedSeal, Dennis, DifferentPerson, Dominika, Echocage, ehylo,
elongated muskrat, Entropy, Entropy5, EvilCodeZ, EvilSourcerer, flurr, Friedolin2000, FTC55, fw4hre0xxq,
GameCenterJerry, Graicc, Greg Depoire--Ferrer, Harry, Howard Stark, Ic3Tank, Imeguras, IronException, Jack,
jayc331, Joerez, John, John200410, krzys-h, kuthy, Kyle Koivuneva, Larson, Leijurv, LHM, Logan Darklock, Lucas
Arden, Lyzev, mankool, mankool0, Mike, millennIumAMbiguity, Moondarker, Murat65536, Murat Bayraktar, Nachiketa
Gargi, odisseus, Orinion, orsondmc, Patrick Huber, PhlegethonAcheron, RealIndrit, reb0rt, rfresh2, rycbar0, Sam
Corbett, schmar03, scorbett123, Scribblefoxx, Skylandia, StijnSimons, StylexTV, SuperOP535, SylveonBottle,
Tabbs, Todarac, underscore-zi, unrootexe, Wagyourtail, Warpten, William Gray, Xennex, Xiaodown, Your Name,
ZacSharp, ZakME

The live list is on [GitHub](https://github.com/cabaletta/baritone/graphs/contributors). When a fix is ported
from a later upstream commit, its authors are added here.

If your name is missing or you want it changed, please open an issue.

## Upstream licensing

- Upstream's `LICENSE` file is the unmodified text of the GNU LGPL, version 3. Upstream source files state
  "either version 3 of the License, or (at your option) any later version".
- Upstream source files name no individual copyright holders, so none are named here. Their copyright remains
  with Baritone's authors.
- Upstream's README describes the license as "LGPL-3.0 with anime exception", and its repository includes an
  image, [`LICENSE-Part-2.jpg`](https://github.com/cabaletta/baritone/blob/25111daedf1d59e6a8dfb5a3e61885cdb8d953df/LICENSE-Part-2.jpg),
  showing a penguin with the words "No anime". Upstream gives no further text. The image is not copied here.

## Ported files

Each ported file keeps upstream's notice and states where it came from and that it was changed:

```rust
// This file is part of baritone-rs, a Rust port of Baritone.
// Ported to Rust on YYYY-MM-DD from <upstream path> in cabaletta/baritone at commit <full commit hash>.
//
// This file is part of Baritone.
//
// Baritone is free software: you can redistribute it and/or modify
// it under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// Baritone is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Lesser General Public License for more details.
//
// You should have received a copy of the GNU Lesser General Public License
// along with Baritone.  If not, see <https://www.gnu.org/licenses/>.
```

## Elytra pathfinder

Elytra flight in this port will come from the Java pathfinder in upstream's `1.21.4` branch
(`src/main/java/baritone/process/elytra/pathfinder/`), added by
[pull request 5117](https://github.com/cabaletta/baritone/pull/5117) and merged as commit
`3d3da10b0cb0fb71a80ffb2fe673fac6927924d2`.

- **Java port:** Mattias Rask ([0Mattias](https://github.com/0Mattias)), with later changes by aver. Reviewed
  by babbaj and ZacSharp.
- **Original native library:** [nether-pathfinder](https://github.com/babbaj/nether-pathfinder), by babbaj, with
  contributions from leijurv, ZeroMemes, underscore-zi, 0-x-2-2 and 0Mattias.

**Licensing assumption.** The Java port's files carry Baritone's LGPL notice, and this project treats them as
licensed on the same terms as the rest of Baritone. That is an assumption, not an explicit grant from the
authors of nether-pathfinder: no license file was found in that repository on 2026-11-05. Its native code is not
used here. If you hold rights in nether-pathfinder and object, please open an issue.

**Excluded files.** `ChunkGeneratorHell` and `NoiseGeneratorImproved` describe themselves as coming from
Minecraft, and `NoiseGeneratorOctaves` belongs with them. The assumption above does not cover them, and they will
not be ported.
