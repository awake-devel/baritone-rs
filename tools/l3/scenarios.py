# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""The L3 scenarios: how to build each world, what to do in it, and what counts as a pass.

A scenario is generated, never committed as world data. Each one builds its own area high in the sky (stone at y=200, so the
player stands at y=201), far from the spawn, through the single-player server's console (forceload, fill, setblock). Anything
the path steps off falls into the void or far down: a path that misjudges an edge ends in a death, which the checks catch.

Scenario 5 and 10 are the exception: natural terrain around the spawn.

Run by run.py. The mod plays the spec it is given (mod/runtime.rs, "L3 script player") and records; the judging is in run.py.
"""

SKY = 200          # y of the stone floors; the player's feet are at SKY + 1
CLEAR_FROM = 194   # air is made from here up, so natural terrain in the area is gone
CLEAR_TO = 235


def clear(x0, y0, z0, x1, y1, z1):
    """fill ... air in pieces of at most 32768 blocks."""
    lines = []
    nx = x1 - x0 + 1
    per_x = (y1 - y0 + 1) * (z1 - z0 + 1)
    step = max(1, 32000 // per_x)
    x = x0
    while x <= x1:
        xe = min(x1, x + step - 1)
        lines.append(f"fill {x} {y0} {z0} {xe} {y1} {z1} minecraft:air")
        x = xe + 1
    assert nx > 0
    return lines


def floor(x0, z0, x1, z1, y=SKY, block="minecraft:stone"):
    return [f"fill {x0} {y} {z0} {x1} {y} {z1} {block}"]


class Area:
    """A box of the world that a scenario builds in. Everything is in absolute block coordinates. The inventory is emptied first."""

    def __init__(self, x0, z0, x1, z1, y0=CLEAR_FROM, y1=CLEAR_TO):
        self.box = (x0, y0, z0, x1, y1, z1)
        self.lines = [
            "clear @p",
            "forceload remove all",
            f"forceload add {x0 - 16} {z0 - 16} {x1 + 16} {z1 + 16}",
            # The run lasts half an hour. The clear weather from its start ran out and snow fell in a cold biome (2026-11-08, 11 scenarios failed
            # on snow layers and nothing else), so every scenario clears the weather again.
            "weather clear 1000000",
        ]
        self.lines += clear(*self.box)

    def add(self, *lines):
        for l in lines:
            self.lines.append(l)
        return self


def tp(x, y, z):
    return f"execute in minecraft:overworld run tp @p {x} {y} {z} 0 0"


def setting_commands(settings):
    """`b set NAME VALUE` lines for a dict of settings, and the lines that put them back (every setting used here defaults to true)."""
    on = [f"b set {k} {str(v).lower()}" for k, v in settings.items()]
    off = [f"b set {k} true" for k in settings]
    return on, off


def scenario(name, chat, area, start, events, end, stop_when, check, watch=None, wait_loaded=None, slot=None, settings=None):
    """`slot`: the hotbar slot selected when the scenario starts. `settings`: {name: bool} set by `b set` before the command and put back at the end."""
    on, off = setting_commands(settings or {})
    events = [{"t": 3, "set": c} for c in on] + list(events)
    return {
        "slot": slot,
        "after": off,
        "name": name,
        "chat": chat,
        "prep": area.lines if area else [],
        "tp": tp(*start) if start else None,
        "wait_loaded": wait_loaded if wait_loaded is not None else ([[int(start[0]), int(start[2])]] if start else []),
        "watch": list(watch) if watch else None,
        "events": events,
        "end": end,
        "stop_when": stop_when,
        "check": check,
    }


def with_chat(chat, events):
    """The chat is opened before the command when the scenario is a chat one (tick 2, the command at tick 10)."""
    return ([{"t": 2, "chat": "open"}] if chat else []) + events


def around(box, pad=2):
    x0, y0, z0, x1, y1, z1 = box
    return (x0 - pad, y0, z0 - pad, x1 + pad, y1, z1 + pad)


def kit(*items):
    """Console lines that empty the inventory and give items (they land in the hotbar slots 0, 1, 2, ... in this order)."""
    return ["clear @p"] + [f"give @p minecraft:{i}" + (" 64" if i == "cobblestone" else "") for i in items]


def s01_flat(chat, x0=3000, z0=3000):
    a = Area(x0, z0, x0 + 60, z0 + 8)
    a.add(*floor(x0, z0, x0 + 60, z0 + 8))
    goal = (x0 + 52, SKY + 1, z0 + 4)
    return scenario(
        f"s01_flat_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=600, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 400, "sprint_min_fraction": 0.7, "blocks_changed": 0, "blocks_walked": 50},
        watch=(x0, SKY - 4, z0, x0 + 60, SKY + 12, z0 + 8))


def s02_wall(chat, x0=3200, z0=3000):
    a = Area(x0, z0 - 12, x0 + 50, z0 + 24)
    a.add(*floor(x0, z0 - 12, x0 + 50, z0 + 24))
    # Wall A runs along z, wall B along x from its south end: an L, three blocks high. Its pocket is open to the north and the east.
    a.add(f"fill {x0 + 20} {SKY + 1} {z0 - 8} {x0 + 20} {SKY + 3} {z0 + 10} minecraft:stone")
    a.add(f"fill {x0 + 20} {SKY + 1} {z0 + 10} {x0 + 36} {SKY + 3} {z0 + 10} minecraft:stone")
    goal = (x0 + 30, SKY + 1, z0 + 3)
    return scenario(
        f"s02_wall_chat{int(chat)}", chat, a, (x0 + 10.5, SKY + 1, z0 + 3.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=700, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 450, "blocks_changed": 0},
        watch=(x0, SKY - 2, z0 - 12, x0 + 50, SKY + 8, z0 + 24), settings={"allowBreak": False})


def s03_steps_and_diagonals(chat, x0=3400, z0=3000):
    a = Area(x0, z0, x0 + 28, z0 + 16)
    a.add(*floor(x0, z0, x0 + 3, z0 + 4))
    for k in range(1, 9):  # eight single steps up, one block each, one wide
        a.add(f"setblock {x0 + 3 + k} {SKY + k} {z0 + 2} minecraft:stone")
    top = SKY + 8
    a.add(*floor(x0 + 12, z0 + 1, x0 + 14, z0 + 3, y=top))
    for j in range(8):  # a chain of single blocks that touch only at their corners
        a.add(f"setblock {x0 + 15 + j} {top} {z0 + 4 + j} minecraft:stone")
    a.add(*floor(x0 + 23, z0 + 12, x0 + 25, z0 + 14, y=top))
    goal = (x0 + 24, top + 1, z0 + 13)
    return scenario(
        f"s03_steps_diagonals_chat{int(chat)}", chat, a, (x0 + 1.5, SKY + 1, z0 + 2.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=700, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 450, "blocks_changed": 0},
        watch=(x0 - 1, SKY - 2, z0 - 1, x0 + 27, top + 4, z0 + 16))


def s04_drops(chat, x0=3600, z0=3000):
    tiers = [(0, 5, SKY + 10), (6, 11, SKY + 9), (12, 17, SKY + 7), (18, 23, SKY + 4)]  # drops of 1, 2 and 3 between them
    a = Area(x0, z0, x0 + 24, z0 + 4)
    for xa, xb, y in tiers:
        a.add(*floor(x0 + xa, z0, x0 + xb, z0 + 4, y=y))
    goal = (x0 + 21, SKY + 5, z0 + 2)
    return scenario(
        f"s04_drops_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 11, z0 + 2.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=600, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 400, "blocks_changed": 0, "no_health_loss": True},
        watch=(x0 - 1, SKY, z0 - 1, x0 + 25, SKY + 14, z0 + 5))


def s05_natural_300(chat):
    # No world building: the spawn. `~300 ~` is two coordinates, x and z, so the goal is the column 300 blocks east.
    return scenario(
        f"s05_natural300_chat{int(chat)}", chat, None, None,
        with_chat(chat, [{"t": 10, "command": "b goto ~300 ~"}]),
        end=5000, stop_when="arrived",
        check={"kind": "arrive_relative", "dx": 300, "dz": 0, "limit_ticks": 4000, "min_segments": 2},
        wait_loaded=[])


def s06_sealed(chat, x0=3800, z0=3000):
    a = Area(x0, z0, x0 + 40, z0 + 12)
    a.add(*floor(x0, z0, x0 + 40, z0 + 12))
    a.add(f"fill {x0 + 24} {SKY + 1} {z0 + 3} {x0 + 28} {SKY + 5} {z0 + 7} minecraft:stone")
    a.add(f"fill {x0 + 25} {SKY + 1} {z0 + 4} {x0 + 27} {SKY + 3} {z0 + 6} minecraft:air")
    goal = (x0 + 26, SKY + 1, z0 + 5)
    return scenario(
        f"s06_sealed_chat{int(chat)}", chat, a, (x0 + 3.5, SKY + 1, z0 + 5.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=1800, stop_when="idle",
        check={"kind": "unreachable", "limit_ticks": 1500, "blocks_changed": 0},
        watch=(x0, SKY - 2, z0, x0 + 40, SKY + 8, z0 + 12), settings={"allowBreak": False})


def s07_block_in_the_way(chat, x0=4000, z0=3000):
    a = Area(x0, z0, x0 + 50, z0 + 8)
    a.add(*floor(x0, z0, x0 + 50, z0 + 8))
    goal = (x0 + 45, SKY + 1, z0 + 4)
    # At tick 60 the player is about 12 blocks along; a wall of 2 x 5 blocks appears across the line, 10 blocks ahead.
    return scenario(
        f"s07_block_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [
            {"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"},
            {"t": 60, "console": f"fill {x0 + 25} {SKY + 1} {z0 + 2} {x0 + 25} {SKY + 2} {z0 + 6} minecraft:stone"},
            {"t": 70, "rebase": True},
        ]),
        end=700, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 450, "blocks_changed": 0, "min_searches_after": [60, 1]},
        watch=(x0, SKY - 2, z0, x0 + 50, SKY + 8, z0 + 8), settings={"allowBreak": False})


def s08_stop(chat, x0=4200, z0=3000):
    a = Area(x0, z0, x0 + 90, z0 + 8)
    a.add(*floor(x0, z0, x0 + 90, z0 + 8))
    goal = (x0 + 85, SKY + 1, z0 + 4)
    return scenario(
        f"s08_stop_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}, {"t": 70, "command": "b stop"}]),
        end=170, stop_when="never",
        check={"kind": "stop", "stop_tick": 70, "blocks_changed": 0},
        watch=(x0, SKY - 2, z0, x0 + 90, SKY + 8, z0 + 8))


def s09_ladder_and_water(chat, x0=4400, z0=3000):
    a = Area(x0, z0 - 1, x0 + 24, z0 + 6)
    a.add(*floor(x0, z0, x0 + 5, z0 + 4))
    # A pit one block deep across the whole width, walled on both sides, filled with water that cannot spread.
    a.add(*floor(x0 + 6, z0 - 1, x0 + 11, z0 + 5, y=SKY - 1))
    a.add(f"fill {x0 + 6} {SKY} {z0 - 1} {x0 + 11} {SKY} {z0 - 1} minecraft:stone")
    a.add(f"fill {x0 + 6} {SKY} {z0 + 5} {x0 + 11} {SKY} {z0 + 5} minecraft:stone")
    a.add(f"fill {x0 + 6} {SKY} {z0} {x0 + 11} {SKY} {z0 + 4} minecraft:water")
    a.add(*floor(x0 + 12, z0, x0 + 16, z0 + 4))
    top = SKY + 9
    a.add(f"fill {x0 + 17} {SKY} {z0 + 2} {x0 + 17} {top} {z0 + 2} minecraft:stone")
    a.add(*floor(x0 + 17, z0 + 1, x0 + 21, z0 + 3, y=top))
    for y in range(SKY + 1, top + 1):
        a.add(f"setblock {x0 + 16} {y} {z0 + 2} minecraft:ladder[facing=west]")
    goal = (x0 + 20, top + 1, z0 + 2)
    return scenario(
        f"s09_ladder_water_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 2.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=1200, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 900, "blocks_changed": 0},
        # Not the water pit (x0 + 6 .. x0 + 11): at this height the water freezes by itself in a cold biome, which a first run showed.
        watch=(x0 + 12, SKY - 3, z0 - 2, x0 + 23, top + 4, z0 + 7))


def s10_far(chat):
    # From wherever scenario 5 left the player: 1500 blocks south, through chunks nobody has loaded.
    return scenario(
        f"s10_far1500_chat{int(chat)}", chat, None, None,
        with_chat(chat, [{"t": 10, "command": "b goto ~ ~1500"}]),
        end=14000, stop_when="arrived",
        check={"kind": "arrive_relative", "dx": 0, "dz": 1500, "limit_ticks": 12000, "min_segments": 2, "reported_ok": True},
        wait_loaded=[])


def s11_slab_floor(chat, x0=4600, z0=3000):
    """A floor of bottom slabs over stone, like 6b6t's lobby: the player stands at y + 0.5, and upstream's playerFeet answers the
    block above the slab. Without that rule every step was planned as an ascend that never jumped (found on 6b6t, 2026-11-07)."""
    a = Area(x0, z0, x0 + 40, z0 + 12)
    a.add(*floor(x0, z0, x0 + 40, z0 + 12))
    a.add(*floor(x0, z0, x0 + 40, z0 + 12, y=SKY + 1, block="minecraft:stone_slab[type=bottom]"))
    goal = (x0 + 32, SKY + 2, z0 + 9)
    return scenario(
        f"s11_slab_floor_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1.5, z0 + 3.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=600, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 400, "blocks_changed": 0},
        watch=(x0, SKY - 4, z0, x0 + 40, SKY + 12, z0 + 12))


# ---------------------------------------------------------------- breaking and placing (S4)

def plan_check(**kw):
    """The check of a scenario that changes blocks: what the plan said, what was done. See run.py `judge`."""
    return dict(kind="arrive", **kw)


def b01_break_stone(chat, x0=4800, z0=3000):
    """A wall of stone, two high, across the whole floor: the only way is through. Iron pickaxe in slot 0, slot 5 (empty) selected.
    The plan breaks the top block only and goes over the lower one (an ascend and a descend), which is cheaper than two breaks."""
    a = Area(x0, z0, x0 + 40, z0 + 8)
    a.add(*floor(x0, z0, x0 + 40, z0 + 8))
    a.add(f"fill {x0 + 20} {SKY + 1} {z0} {x0 + 20} {SKY + 2} {z0 + 8} minecraft:stone")
    a.add(*kit("iron_pickaxe"))
    goal = (x0 + 26, SKY + 1, z0 + 4)
    return scenario(
        f"b01_break_stone_chat{int(chat)}", chat, a, (x0 + 14.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=700, stop_when="arrived",
        check=plan_check(goal=goal, limit_ticks=450, plan={"break": 1, "place": 0}, tool_damage={"iron_pickaxe": 1}, no_health_loss=True),
        watch=(x0 + 12, SKY - 2, z0 - 1, x0 + 30, SKY + 6, z0 + 9), slot=5)


def b02_break_dirt_by_hand(chat, x0=5000, z0=3000):
    """The same with dirt and an empty inventory: about 16 ticks a block by hand. One block to break, as above."""
    a = Area(x0, z0, x0 + 40, z0 + 8)
    a.add(*floor(x0, z0, x0 + 40, z0 + 8, block="minecraft:dirt"))
    a.add(f"fill {x0 + 20} {SKY + 1} {z0} {x0 + 20} {SKY + 2} {z0 + 8} minecraft:dirt")
    goal = (x0 + 26, SKY + 1, z0 + 4)
    return scenario(
        f"b02_break_dirt_hand_chat{int(chat)}", chat, a, (x0 + 14.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=900, stop_when="arrived",
        check=plan_check(goal=goal, limit_ticks=600, plan={"break": 1, "place": 0}, no_health_loss=True),
        watch=(x0 + 12, SKY - 2, z0 - 1, x0 + 30, SKY + 6, z0 + 9), slot=3)


def b03_bridge(chat, x0=5200, z0=3000):
    """A gap three wide in a floor five wide, over the void: three cobblestone blocks are the only way across."""
    a = Area(x0, z0, x0 + 30, z0 + 4)
    a.add(*floor(x0, z0, x0 + 9, z0 + 4))
    a.add(*floor(x0 + 13, z0, x0 + 30, z0 + 4))
    a.add(*kit("cobblestone"))
    goal = (x0 + 18, SKY + 1, z0 + 2)
    return scenario(
        f"b03_bridge_chat{int(chat)}", chat, a, (x0 + 5.5, SKY + 1, z0 + 2.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=800, stop_when="arrived",
        check=plan_check(goal=goal, limit_ticks=500, plan={"break": 0, "place": 3}, stack_delta={"cobblestone": -3}, no_health_loss=True),
        watch=(x0 + 3, SKY - 3, z0 - 1, x0 + 22, SKY + 6, z0 + 5), slot=6)


def b04_pillar(chat, x0=5400, z0=3000, ledge=True):
    """A shaft one block wide in a block of stone. ledge: the stone is six high and the goal on top, so five blocks go in the
    shaft and the sixth step up is an ascend onto the stone (a rise of six). Otherwise the stone is seven high and the goal is inside the
    shaft at the top: six blocks go in, a pillar of six."""
    a = Area(x0, z0, x0 + 14, z0 + 14)
    a.add(*floor(x0, z0, x0 + 14, z0 + 14))
    cx, cz = x0 + 6, z0 + 6
    top = SKY + (6 if ledge else 7)
    a.add(f"fill {cx - 2} {SKY + 1} {cz - 2} {cx + 2} {top} {cz + 2} minecraft:stone")
    a.add(f"fill {cx} {SKY + 1} {cz} {cx} {top} {cz} minecraft:air")
    a.add(*kit("cobblestone"))
    goal = (cx + 2, SKY + 7, cz) if ledge else (cx, SKY + 7, cz)
    n = 5 if ledge else 6
    return scenario(
        f"b04_pillar_{'ledge' if ledge else 'shaft'}_chat{int(chat)}", chat, a, (cx + 0.5, SKY + 1, cz + 0.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=900, stop_when="arrived",
        check=plan_check(goal=goal, limit_ticks=600, plan={"break": 0, "place": n}, stack_delta={"cobblestone": -n}, no_health_loss=True),
        watch=(cx - 3, SKY, cz - 3, cx + 3, SKY + 10, cz + 3), slot=4)


def b05_dig_down(chat, x0=5600, z0=3000):
    """A mass of stone nine wide and ten deep, the goal five blocks below the surface inside it: five blocks to break, straight down."""
    a = Area(x0, z0, x0 + 8, z0 + 8)
    a.add(f"fill {x0} {SKY - 10} {z0} {x0 + 8} {SKY} {z0 + 8} minecraft:stone")
    a.add(*kit("iron_pickaxe"))
    goal = (x0 + 4, SKY + 1 - 5, z0 + 4)
    return scenario(
        f"b05_dig_down5_chat{int(chat)}", chat, a, (x0 + 4.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=900, stop_when="arrived",
        check=plan_check(goal=goal, limit_ticks=600, plan={"break": 5, "place": 0}, tool_damage={"iron_pickaxe": 5}, no_health_loss=True),
        watch=(x0, SKY - 10, z0, x0 + 8, SKY + 3, z0 + 8), slot=7)


def b06_wall_break_allowed_vs_not(chat, allow, x0=5800, z0=3000):
    """A wall three high that ends 4 blocks before the edge of the floor, goal behind it. With allowBreak on the plan goes through
    (two blocks); with it off, around and nothing changes. The pickaxe is in the hotbar in both."""
    a = Area(x0, z0, x0 + 40, z0 + 16)
    a.add(*floor(x0, z0, x0 + 40, z0 + 16))
    a.add(f"fill {x0 + 20} {SKY + 1} {z0} {x0 + 20} {SKY + 3} {z0 + 12} minecraft:stone")
    a.add(*kit("iron_pickaxe"))
    goal = (x0 + 26, SKY + 1, z0 + 4)
    plan = {"break": 2, "place": 0} if allow else {"break": 0, "place": 0}
    return scenario(
        f"b06_wall_allowbreak{'on' if allow else 'off'}_chat{int(chat)}", chat, a, (x0 + 14.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=900, stop_when="arrived",
        check=plan_check(goal=goal, limit_ticks=600, plan=plan, no_health_loss=True),
        watch=(x0 + 12, SKY - 2, z0 - 1, x0 + 32, SKY + 6, z0 + 17), slot=2, settings=None if allow else {"allowBreak": False})


def b07_bucket_fall(chat, x0=6000, z0=3000):
    """A drop of twelve blocks, more than maxFallHeightNoWater (3) and less than maxFallHeightBucket (20): only with the water bucket."""
    a = Area(x0, z0, x0 + 24, z0 + 12)
    a.add(*floor(x0, z0, x0 + 24, z0 + 12))
    a.add(*floor(x0, z0, x0 + 5, z0 + 12, y=SKY + 12))
    a.add(*kit("water_bucket"))
    goal = (x0 + 9, SKY + 1, z0 + 6)
    return scenario(
        f"b07_bucket_fall12_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 13, z0 + 6.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=700, stop_when="arrived",
        check=plan_check(goal=goal, limit_ticks=400, plan={"break": 0, "place": 0}, water_ok=True, hotbar_end_has=["water_bucket"], no_health_loss=True),
        watch=(x0, SKY - 2, z0, x0 + 24, SKY + 14, z0 + 12), slot=3)


def b08_stop_dig(chat, x0=6200, z0=3000):
    """Stone by hand takes 150 ticks a block. `b stop` at tick 70 (the dig is under way): the block must still be there 230 ticks later."""
    a = Area(x0, z0, x0 + 40, z0 + 8)
    a.add(*floor(x0, z0, x0 + 40, z0 + 8))
    a.add(f"fill {x0 + 20} {SKY + 1} {z0} {x0 + 20} {SKY + 2} {z0 + 8} minecraft:stone")
    goal = (x0 + 26, SKY + 1, z0 + 4)
    return scenario(
        f"b08_stop_dig_chat{int(chat)}", chat, a, (x0 + 18.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}, {"t": 70, "command": "b stop"}]),
        end=300, stop_when="never",
        check={"kind": "stop_dig", "stop_tick": 70, "blocks_changed": 0},
        watch=(x0 + 16, SKY - 2, z0 - 1, x0 + 30, SKY + 6, z0 + 9), slot=0)


def b09_dig_stone_by_hand(chat, x0=6400, z0=3000):
    """The control of b08: the same wall and no `b stop`. It must break the block by hand (about 150 ticks) and arrive."""
    a = Area(x0, z0, x0 + 40, z0 + 8)
    a.add(*floor(x0, z0, x0 + 40, z0 + 8))
    a.add(f"fill {x0 + 20} {SKY + 1} {z0} {x0 + 20} {SKY + 2} {z0 + 8} minecraft:stone")
    goal = (x0 + 26, SKY + 1, z0 + 4)
    return scenario(
        f"b09_dig_stone_hand_chat{int(chat)}", chat, a, (x0 + 18.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=700, stop_when="arrived",
        check=plan_check(goal=goal, limit_ticks=450, plan={"break": 1, "place": 0}, no_health_loss=True),
        watch=(x0 + 16, SKY - 2, z0 - 1, x0 + 30, SKY + 6, z0 + 9), slot=0)


def b10_head_sand(chat, x0=6800, z0=3000):
    """Sand lands where the head is, in the middle of a walk (S4 follow-up, docs/deviations.md D16 and D31). At tick 45 the console sets a sand
    block in the cell of the player's head. Upstream `Movement.update` then switches to the best tool and presses the left button on what the
    crosshair is on, and the walk goes on; before the fix the movement failed and the path was searched again. The sand then falls on and
    may land in the cell of the feet, so blocks are not judged; what is judged is that the walk reached the goal and that no search that gave
    a path was made after the sand came."""
    a = Area(x0, z0, x0 + 70, z0 + 8)
    a.add(*floor(x0, z0, x0 + 70, z0 + 8))
    goal = (x0 + 62, SKY + 1, z0 + 4)
    return scenario(
        f"b10_head_sand_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [
            {"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"},
            {"t": 45, "console": "execute as @p at @s run setblock ~ ~1 ~ minecraft:sand"},
        ]),
        end=700, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 500, "no_search_after": 45},
        watch=(x0, SKY - 2, z0, x0 + 70, SKY + 8, z0 + 8), settings={"allowBreak": False}, slot=0)


def closed_corridor(x0, z0, length=24):
    """A corridor one block wide along x at z0 + 4, two high, walled and roofed in stone, closed at both ends. Returns the Area."""
    a = Area(x0, z0, x0 + length, z0 + 8)
    a.add(*floor(x0, z0, x0 + length, z0 + 8))
    a.add(f"fill {x0} {SKY + 1} {z0 + 3} {x0 + length} {SKY + 3} {z0 + 5} minecraft:stone")
    a.add(f"fill {x0 + 1} {SKY + 1} {z0 + 4} {x0 + length - 1} {SKY + 2} {z0 + 4} minecraft:air")
    return a


def b11_door(chat, x0=7000, z0=3000):
    """A corridor whose only way on is a closed oak door (the door faces along the corridor, so it blocks). Upstream always opens it
    (`MovementHelper.openDoors`). Judged: arrival, and that the only blocks that changed are the two halves of the door (opened)."""
    a = closed_corridor(x0, z0)
    dx = x0 + 12
    a.add(f"setblock {dx} {SKY + 1} {z0 + 4} minecraft:oak_door[facing=east,half=lower,hinge=left,open=false]")
    a.add(f"setblock {dx} {SKY + 2} {z0 + 4} minecraft:oak_door[facing=east,half=upper,hinge=left,open=false]")
    goal = (x0 + 20, SKY + 1, z0 + 4)
    return scenario(
        f"b11_door_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=600, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 350, "toggled": [(dx, SKY + 1, z0 + 4), (dx, SKY + 2, z0 + 4)]},
        watch=(x0, SKY - 2, z0, x0 + 24, SKY + 6, z0 + 8), settings={"allowBreak": False}, slot=0)


def b12_gate(chat, x0=7200, z0=3000):
    """The same with a closed oak fence gate."""
    a = closed_corridor(x0, z0)
    gx = x0 + 12
    a.add(f"setblock {gx} {SKY + 1} {z0 + 4} minecraft:oak_fence_gate[facing=east,open=false]")
    goal = (x0 + 20, SKY + 1, z0 + 4)
    return scenario(
        f"b12_gate_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]),
        end=600, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 350, "toggled": [(gx, SKY + 1, z0 + 4)]},
        watch=(x0, SKY - 2, z0, x0 + 24, SKY + 6, z0 + 8), settings={"allowBreak": False}, slot=0)


def new_scenarios():
    out = []
    for chat in (False, True):
        out += [b01_break_stone(chat), b02_break_dirt_by_hand(chat), b03_bridge(chat), b04_pillar(chat), b04_pillar(chat, x0=6600, ledge=False), b05_dig_down(chat),
                b06_wall_break_allowed_vs_not(chat, True), b06_wall_break_allowed_vs_not(chat, False), b07_bucket_fall(chat), b08_stop_dig(chat), b09_dig_stone_by_hand(chat),
                b10_head_sand(chat), b11_door(chat), b12_gate(chat)]
    return out


# ---------------------------------------------------------------- mine and goto <block> (R2)

def m01_mine_cave(chat, x0=7400, z0=3000):
    """Three iron ores in a small cave at three depths: one in the east wall at the height of the feet, one in the north wall three blocks up (the
    head height of a person standing on the floor), one in the wall of a pit three blocks below the floor. `b mine 3 iron_ore` must break all three
    and pick up their drops (3 raw iron), the whole walk must be planned (every block broken was in a plan's toBreak), and the player is unhurt."""
    a = Area(x0, z0, x0 + 20, z0 + 20)
    a.add(f"fill {x0} {SKY - 6} {z0} {x0 + 20} {SKY + 6} {z0 + 20} minecraft:stone")
    a.add(f"fill {x0 + 4} {SKY + 1} {z0 + 4} {x0 + 16} {SKY + 4} {z0 + 16} minecraft:air")
    a.add(f"fill {x0 + 10} {SKY - 2} {z0 + 15} {x0 + 13} {SKY} {z0 + 18} minecraft:air")
    ores = [(x0 + 17, SKY + 1, z0 + 10), (x0 + 10, SKY + 4, z0 + 3), (x0 + 9, SKY - 2, z0 + 16)]
    for o in ores:
        a.add(f"setblock {o[0]} {o[1]} {o[2]} minecraft:iron_ore")
    a.add(*kit("iron_pickaxe"))
    return scenario(
        f"m01_mine_cave3_chat{int(chat)}", chat, a, (x0 + 10.5, SKY + 1, z0 + 10.5),
        with_chat(chat, [{"t": 10, "command": "b mine 3 iron_ore"}]),
        end=2400, stop_when="idle",
        check={"kind": "mine", "ores_broken": ores, "ores_intact": [], "items_end": {"raw_iron": 3}, "limit_ticks": 1800,
               "plan": {"break": [3, 40], "place": [0, 6]}, "have_message": 3, "no_health_loss": True,
               "last_path_may_be_unused": True, "tool_damage_equals_broken": "iron_pickaxe"},
        watch=(x0, SKY - 6, z0, x0 + 20, SKY + 6, z0 + 20), slot=5)


def m02_mine_quantity(chat, x0=7600, z0=3000):
    """The quantity counts what is in the inventory: two raw iron are already in the hotbar, four ores lie on the floor, `b mine 3 iron_ore` must
    break exactly one (the count was 2) and stop with 3 raw iron. The three others stay."""
    a = Area(x0, z0, x0 + 30, z0 + 10)
    a.add(*floor(x0, z0, x0 + 30, z0 + 10))
    ores = [(x0 + 10, SKY + 1, z0 + 5), (x0 + 14, SKY + 1, z0 + 3), (x0 + 18, SKY + 1, z0 + 7), (x0 + 22, SKY + 1, z0 + 5)]
    for o in ores:
        a.add(f"setblock {o[0]} {o[1]} {o[2]} minecraft:iron_ore")
    a.add(*kit("iron_pickaxe"))
    a.add("give @p minecraft:raw_iron 2")
    return scenario(
        f"m02_mine_quantity_chat{int(chat)}", chat, a, (x0 + 4.5, SKY + 1, z0 + 5.5),
        with_chat(chat, [{"t": 10, "command": "b mine 3 iron_ore"}]),
        end=1500, stop_when="idle",
        check={"kind": "mine", "ores_broken": [], "ores_broken_count": 1, "ores_all": ores, "items_end": {"raw_iron": 3}, "limit_ticks": 900,
               "plan": {"break": 1, "place": 0}, "have_message": 3, "no_health_loss": True,
               "last_path_may_be_unused": True, "tool_damage_equals_broken": "iron_pickaxe"},
        watch=(x0, SKY - 2, z0, x0 + 30, SKY + 6, z0 + 10), slot=5)


def m03_goto_table_behind_wall(chat, x0=7800, z0=3000):
    """`b goto crafting_table`: the table stands behind a wall of stone two high across the whole floor, so the way is through (the pickaxe is in the
    hotbar). The player ends next to the table, the table is still there, and what was broken was in a plan's toBreak."""
    a = Area(x0, z0, x0 + 40, z0 + 8)
    a.add(*floor(x0, z0, x0 + 40, z0 + 8))
    a.add(f"fill {x0 + 20} {SKY + 1} {z0} {x0 + 20} {SKY + 2} {z0 + 8} minecraft:stone")
    table = (x0 + 28, SKY + 1, z0 + 4)
    a.add(f"setblock {table[0]} {table[1]} {table[2]} minecraft:crafting_table")
    a.add(*kit("iron_pickaxe"))
    return scenario(
        f"m03_goto_table_wall_chat{int(chat)}", chat, a, (x0 + 14.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": "b goto crafting_table"}]),
        end=1500, stop_when="idle",
        check={"kind": "goto_block", "block": table, "radius": 2.2, "limit_ticks": 900, "plan": {"break": [1, 2], "place": 0},
               "no_health_loss": True, "intact": [table], "opens_container": True, "last_path_may_be_unused": True,
               "tool_damage_equals_broken": "iron_pickaxe"},
        watch=(x0 + 12, SKY - 2, z0 - 1, x0 + 34, SKY + 6, z0 + 9), slot=5)


def m04_stop_mine_walking(chat, x0=8000, z0=3000):
    """`b mine gold_block` toward a block 100 blocks away; `b stop` at tick 160, when the scan is long over and the player is running. Same
    checks as `b stop` for a goto: keys released at once, no path, stands still, the block is untouched. Gold blocks, not iron ore: the scan
    keeps the first 64 hits of a spiral of chunks (FasterWorldScanner), and the natural iron ore far below the platform would fill them before
    a block six chunks away is seen (docs/deviations.md D39)."""
    a = Area(x0, z0, x0 + 120, z0 + 8)
    a.add(*floor(x0, z0, x0 + 120, z0 + 8))
    a.add(f"setblock {x0 + 105} {SKY + 1} {z0 + 4} minecraft:gold_block")
    a.add(*kit("iron_pickaxe"))
    return scenario(
        f"m04_stop_mine_walking_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": "b mine gold_block"}, {"t": 160, "command": "b stop"}]),
        end=300, stop_when="never",
        check={"kind": "stop", "stop_tick": 160, "blocks_changed": 0},
        watch=(x0, SKY - 2, z0, x0 + 120, SKY + 8, z0 + 8), slot=0)


def m05_stop_mine_scanning(chat, x0=8200, z0=3000):
    """`b stop` four ticks after `b mine`, while the scan runs on its own thread (it takes about 2 s when fewer than 64 ores are near). The scan's
    result arrives after the stop and must not start the walk: no keys, no path, the player does not move, the ore is untouched."""
    a = Area(x0, z0, x0 + 60, z0 + 8)
    a.add(*floor(x0, z0, x0 + 60, z0 + 8))
    a.add(f"setblock {x0 + 40} {SKY + 1} {z0 + 4} minecraft:iron_ore")
    a.add(*kit("iron_pickaxe"))
    return scenario(
        f"m05_stop_mine_scanning_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, [{"t": 10, "command": "b mine iron_ore"}, {"t": 14, "command": "b stop"}]),
        end=260, stop_when="never",
        check={"kind": "stop", "stop_tick": 14, "blocks_changed": 0, "not_walking_before": True, "still_for_ticks": 200},
        watch=(x0, SKY - 2, z0, x0 + 60, SKY + 8, z0 + 8), slot=0)


def mine_scenarios():
    out = []
    for chat in (False, True):
        out += [m01_mine_cave(chat), m02_mine_quantity(chat), m03_goto_table_behind_wall(chat), m04_stop_mine_walking(chat), m05_stop_mine_scanning(chat)]
    return out


# ---------------------------------------------------------------- follow (R2)

def summon(kind, tag, x, z):
    """A mob that stands still (NoAI), cannot be hurt, and carries a tag so that the scenario can move it with `tp`."""
    return f'summon minecraft:{kind} {x} {SKY + 1} {z} {{NoAI:1b,Silent:1b,Invulnerable:1b,PersistenceRequired:1b,Tags:["{tag}"]}}'


def mob_tp(tag, x, z):
    return f"tp @e[tag={tag},limit=1] {x} {SKY + 1} {z}"


def follow_area(x0, z0, length, width):
    a = Area(x0, z0, x0 + length, z0 + width)
    a.add(*floor(x0, z0, x0 + length, z0 + width))
    # No snow layers on the floor during the run (they showed up once in a cold biome and count as changed blocks).
    a.add("kill @e[tag=l3chosen]", "kill @e[tag=l3decoy]", "weather clear 1000000")
    return a


def f01_follow_route(chat, x0=8400, z0=3000):
    """`b follow entity villager` after a NoAI villager that the scenario moves along a route in three steps of about 35 blocks (the last one
    back past the player). Each time the player must be within followRadius (3) plus 1.5 blocks of the villager within 180 ticks and stay there, and
    must never be farther from it than the longest jump plus that radius."""
    a = follow_area(x0, z0, 120, 16)
    start = (x0 + 20.5, z0 + 8.5)
    steps = [(0, *start), (200, x0 + 55.5, z0 + 4.5), (400, x0 + 90.5, z0 + 12.5), (600, x0 + 62.5, z0 + 6.5)]
    a.add(summon("villager", "l3chosen", *start))
    events = [{"t": 10, "command": "b follow entity villager"}]
    events += [{"t": t, "console": mob_tp("l3chosen", x, z)} for t, x, z in steps[1:]]
    events += [{"t": 796, "command": "b stop"}, {"t": 797, "console": "kill @e[tag=l3chosen]"}]
    return scenario(
        f"f01_follow_route_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 8.5),
        with_chat(chat, events),
        end=800, stop_when="never",
        check={"kind": "follow", "radius": 3, "tolerance": 1.5, "target": steps, "decoy": None, "catch_up_ticks": 180, "hold_ticks": 20, "blocks_changed": 0, "no_health_loss": True},
        watch=(x0, SKY - 2, z0, x0 + 120, SKY + 8, z0 + 16))


def f02_follow_type_filter(chat, chosen="pig", decoy="villager", x0=8600, z0=3000):
    """Two kinds of NoAI mobs: `b follow entity CHOSEN`. The decoy stands 10 blocks from the player, the chosen one 35 blocks away on the other
    side. The player must go to the chosen one and ignore the decoy; at tick 260 the decoy is moved next to the player and the chosen one 32 blocks
    away, and the player must leave the decoy and go to the chosen one again (never nearer than 25 blocks to the decoy at the end of a step)."""
    if chosen == "villager":
        x0 += 200
    a = follow_area(x0, z0, 120, 16)
    p = (x0 + 50.5, z0 + 8.5)
    target = [(0, x0 + 85.5, z0 + 8.5), (260, x0 + 50.5, z0 + 4.5)]
    dec = [(0, x0 + 40.5, z0 + 8.5), (260, "rel", 2, 0)]
    a.add(summon(chosen, "l3chosen", *target[0][1:]), summon(decoy, "l3decoy", *dec[0][1:]))
    events = [
        {"t": 10, "command": f"b follow entity {chosen}"},
        {"t": 260, "console": "execute at @p run tp @e[tag=l3decoy,limit=1] ~2 ~ ~"},
        {"t": 261, "console": mob_tp("l3chosen", *target[1][1:])},
        {"t": 556, "command": "b stop"},
        {"t": 557, "console": "kill @e[tag=l3chosen]"},
        {"t": 557, "console": "kill @e[tag=l3decoy]"},
    ]
    return scenario(
        f"f02_follow_only_{chosen}_chat{int(chat)}", chat, a, (*p[:1], SKY + 1, p[1]),
        with_chat(chat, events),
        end=560, stop_when="never",
        check={"kind": "follow", "radius": 3, "tolerance": 1.5, "target": [(0, *target[0][1:]), (261, *target[1][1:])], "decoy": dec, "decoy_min_distance": 25,
               "catch_up_ticks": 200, "hold_ticks": 20, "blocks_changed": 0, "no_health_loss": True},
        watch=(x0, SKY - 2, z0, x0 + 120, SKY + 8, z0 + 16))


def f04_stop_follow(chat, x0=9200, z0=3000):
    """`b stop` while the player runs after a villager 88 blocks away (tick 90); at tick 130 the villager is moved. Same checks as `b stop` for a
    goto: keys released at once, no path afterwards, the player stands still and does not move again, nothing in the world changed."""
    a = follow_area(x0, z0, 120, 8)
    a.add(summon("villager", "l3chosen", x0 + 92.5, z0 + 4.5))
    events = [{"t": 10, "command": "b follow entity villager"}, {"t": 90, "command": "b stop"}, {"t": 130, "console": mob_tp("l3chosen", x0 + 20.5, z0 + 2.5)},
              {"t": 295, "console": "kill @e[tag=l3chosen]"}]
    return scenario(
        f"f04_stop_follow_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 4.5),
        with_chat(chat, events),
        end=300, stop_when="never",
        check={"kind": "stop", "stop_tick": 90, "blocks_changed": 0},
        watch=(x0, SKY - 2, z0, x0 + 120, SKY + 8, z0 + 8))


def follow_scenarios():
    out = []
    for chat in (False, True):
        out += [f01_follow_route(chat), f02_follow_type_filter(chat, "pig", "villager"), f02_follow_type_filter(chat, "villager", "pig"), f04_stop_follow(chat)]
    for sc in out:  # a follow filter outlives the scenario that set it: clear it before the next one
        sc["after"] = ["b stop"] + sc["after"]
    return out


# ---------------------------------------------------------------- the chunk cache and explore (R2)

def cache_visits(x0, z0, visits):
    """Console `tp` events that stand the player in a place for a while, so that the cache (which packs the chunks within five chunks of the
    player every four seconds) holds the chunks there, then move on. `visits` is [(tick, x, z)]."""
    return [{"t": t, "console": tp(x0 + dx, SKY + 1, z0 + dz)} for t, dx, dz in visits]


def c01_goto_chest_cached(chat, x0=12000, z0=3000, seen=True):
    """`b goto trapped_chest` to a chest whose chunk the game has unloaded. A 400 long stone strip with a chest 6 blocks from the start; the player stays
    there 150 ticks (the cache packs the chunks), is moved 390 blocks away (beyond the render distance of 16 chunks, so the client drops the
    chunk), and a probe at tick 290 records that the chest's chunk is not loaded. Then `b goto trapped_chest`: the only place that still knows the chest is
    the cache. The player must walk back (about 390 blocks) to within 2.2 blocks of it, open it, and not break it. With `seen` False (the
    control) the player starts at the far end and never goes near the chest, so the cache cannot know it and the goto must fail.
    (`chunkCaching` off is no control: it only stops the files, as upstream.)"""
    a = Area(x0, z0, x0 + 400, z0 + 8)
    a.add(*floor(x0, z0, x0 + 400, z0 + 8))
    chest = (x0 + 6, SKY + 1, z0 + 4)
    a.add(f"setblock {chest[0]} {chest[1]} {chest[2]} minecraft:trapped_chest")
    a.add(*kit("iron_pickaxe"))
    events = cache_visits(x0, z0, [(150, 395.5, 4.5)]) if seen else []
    events += [{"t": 290, "probe": [chest[0], chest[2]]}, {"t": 300, "command": "b goto trapped_chest"}]
    name = "c01_goto_chest_cached" if seen else "c01x_control_not_seen"
    start = (x0 + 2.5, SKY + 1, z0 + 4.5) if seen else (x0 + 395.5, SKY + 1, z0 + 4.5)
    return scenario(
        f"{name}_chat{int(chat)}", chat, a, start,
        with_chat(chat, events),
        end=4200, stop_when="idle",
        check={"kind": "goto_block", "block": chest, "radius": 2.2, "limit_ticks": 2600, "plan": {"break": 0, "place": 0}, "no_health_loss": True,
               "intact": [chest], "opens_container": True, "last_path_may_be_unused": True, "tool_damage_equals_broken": "iron_pickaxe",
               "probe_unloaded": True, "min_distance_walked": 350},
        watch=(x0, SKY - 2, z0, x0 + 12, SKY + 6, z0 + 8), slot=5)


def c02_goto_across_cached(chat, x0=14000, z0=3000, seen=True):
    """A `b goto X Y Z` whose path crosses chunks that are only in the cache. A 410 long strip, 17 wide, with a wall across it at 320 (z0 to
    z0+14, the gap is at z0+15 and z0+16). `allowBreak` is off, so the way is round the wall. The player visits the wall and the goal (140 ticks
    each), so the cache holds them, and goes back to the start; at tick 590 the wall's and the goal's chunks are probed (not loaded). The goal
    is 400 blocks away. The first search must give a path longer than 290 positions: the loaded world ends about 273 blocks from the start, so
    only the cache can let it plan past that. Arrive within a block, wall untouched."""
    a = Area(x0, z0, x0 + 410, z0 + 16)
    a.add(*floor(x0, z0, x0 + 410, z0 + 16))
    a.add(f"fill {x0 + 320} {SKY + 1} {z0} {x0 + 320} {SKY + 3} {z0 + 14} minecraft:stone")
    goal = (x0 + 400, SKY + 1, z0 + 1)
    events = cache_visits(x0, z0, [(150, 316.5, 8.5), (300, 400.5, 1.5), (450, 2.5, 1.5)]) if seen else []
    events += ([{"t": 290, "rebase": True}] if seen else []) + [{"t": 590, "probe": [x0 + 320, z0 + 4]}, {"t": 591, "probe": [goal[0], goal[2]]},
               {"t": 600, "command": f"b goto {goal[0]} {goal[1]} {goal[2]}"}]
    name = "c02_goto_across_cached" if seen else "c02x_control_not_seen"
    return scenario(
        f"{name}_chat{int(chat)}", chat, a, (x0 + 2.5, SKY + 1, z0 + 1.5),
        with_chat(chat, events),
        end=4600, stop_when="arrived",
        check={"kind": "arrive", "goal": goal, "limit_ticks": 2800, **({"blocks_changed": 0} if seen else {}), "no_health_loss": True, "probe_unloaded": True,
               "first_search_min_positions": 290, "min_distance_walked": 400},
        watch=(x0 + 310, SKY - 2, z0, x0 + 330, SKY + 6, z0 + 16) if seen else None, settings={"allowBreak": False})


def c03_explore(chat):
    """`b explore` for 1200 ticks on natural terrain (where the previous scenario left the player), then `b stop`. The player must walk
    (at least 150 blocks in all), get away from the start (the farthest point at least 40 blocks off, and farther in the second half than in the
    first 400 ticks), lose no health, and stand still after the stop with the keys released."""
    sc = scenario(
        f"c03_explore_chat{int(chat)}", chat, None, None,
        with_chat(chat, [{"t": 10, "command": "b explore"}, {"t": 1210, "command": "b stop"}]),
        end=1450, stop_when="never",
        check={"kind": "explore", "stop_tick": 1210, "no_health_loss": True, "min_distance_walked": 150, "min_far": 40},
        wait_loaded=[])
    # spreadplayers puts the player on dry ground at a random spot (it avoids water and lava): the first run ended its explore in a lake, and the
    # next scenario began on the lake bed and drowned a little, which says nothing about explore.
    sc["tp"] = "execute in minecraft:overworld run spreadplayers 0 0 100 400 false @p"
    return sc


def cache_scenarios(controls=False):
    out = []
    for chat in (False, True):
        d = 600 if chat else 0
        out += [c01_goto_chest_cached(chat, x0=12000 + d), c02_goto_across_cached(chat, x0=14000 + d)]
    if controls:
        out += [c01_goto_chest_cached(False, x0=13200, seen=False), c02_goto_across_cached(False, x0=15200, seen=False)]
    return out


# ---------------------------------------------------------------- build (R3)
#
# `b build <file> [x y z]`. The schematic files are generated by tools/l3/schematics.py (never committed) and written by run.py into the schematics
# folder of the game folder before the game starts (`build_files()`). Every scenario builds on the stone floor at SKY, in its own area from
# x0 = 16000, and gives the command with absolute coordinates, so the box of the schematic is known: its corner is `origin`.

import schematics  # noqa: E402  (tools/l3 is on the path of run.py)

PLANKS = "minecraft:oak_planks"


def box_cells(size, name, gap=()):
    """A hollow box of `name`: floor, roof and the four walls; the cells in `gap` are air."""
    w, h, l = size
    cells = {}
    for x in range(w):
        for y in range(h):
            for z in range(l):
                if (x in (0, w - 1) or z in (0, l - 1) or y in (0, h - 1)) and (x, y, z) not in gap:
                    cells[(x, y, z)] = name
    return cells


def give(*stacks):
    """Console lines that empty the inventory and give stacks, (item, count) each, in order (hotbar slots 0, 1, 2, ...)."""
    return ["clear @p"] + [f"give @p minecraft:{item} {count}" for item, count in stacks]


def at(origin, cells):
    return {(origin[0] + x, origin[1] + y, origin[2] + z): n for (x, y, z), n in cells.items()}


def with_build_settings(sc, settings):
    """`settings`: {name: value text}. Set at tick 3 and put back (`b set reset NAME`) when the scenario is over."""
    sc["events"] = [{"t": 3, "set": f"b set {k} {v}"} for k, v in settings.items()] + sc["events"]
    sc["after"] = sc["after"] + [f"b set reset {k}" for k in settings]
    return sc


_FILES = {}  # file name -> bytes, of every build scenario made so far


def build_scenario(name, chat, x0, z0, files, size, cells, commands, hotbar, extra_prep=(), initial=None, settings=None, end=3000, stop_when="idle",
                   timeline=False, margin=3, console=(), **check):
    """A build on a stone floor. The box is at origin (x0 + 8, SKY + 1, z0 + 8). `files`: {file name: bytes}, written before the game starts.
    `commands`: [(tick, command text)] given at those ticks (the first starts the clock). `cells`: the expected non-air cells (box coordinates)."""
    origin = (x0 + 8, SKY + 1, z0 + 8)
    a = Area(x0, z0, x0 + 30, z0 + 30)
    a.add(*floor(x0, z0, x0 + 30, z0 + 30))
    a.add(*extra_prep)
    a.add(*hotbar)
    w, h, l = size
    watch = (origin[0] - margin, SKY, origin[2] - margin, origin[0] + w + margin, SKY + h + 4, origin[2] + l + margin)
    events = []
    for t, c in commands:
        events.append({"t": t, "set" if c.startswith("set:") else "command": c[4:] if c.startswith("set:") else c})
    events += [{"t": t, "console": c} for t, c in console]
    events.sort(key=lambda e: e["t"])
    _FILES.update(files)
    sc = scenario(name, chat, a, (x0 + 2.5, SKY + 1, z0 + 12.5), with_chat(chat, events), end=end, stop_when=stop_when,
                  check={"kind": "build", "box": [*origin, w, h, l], "expect": [[*c, n] for c, n in at(origin, cells).items()],
                         "initial": [[*c, n] for c, n in (initial or {}).items()], "no_health_loss": True, **check},
                  watch=watch)
    if timeline:
        sc["timeline"] = True
    sc["files"] = sorted(files)
    if settings:
        with_build_settings(sc, settings)
    return sc


def box_origin(x0, z0):
    return (x0 + 8, SKY + 1, z0 + 8)


def build_command(file, x0, z0, dx=0):
    o = box_origin(x0, z0)
    return f"b build {file} {o[0] + dx} {o[1]} {o[2]}"


def r01_build_schem_box(chat, x0=16000, z0=3000):
    """A hollow box of planks, 5 x 4 x 5 with a door gap (two cells high) in the west wall. From a stack of 128 planks, with cobblestone to raise itself
    on (planks are not in `acceptableThrowawayItems`, so without a generic throwaway block the roof cannot be reached: the first run of this scenario
    stood in the box for ever, U42) and a pickaxe to take the scaffolding away again. Every cell of the box must be as the schematic says, it says
    "Done building" within the limit, no health is lost, and the cells that end up changed inside the box are the planks of the schematic. The
    builder only looks inside the schematic's box (`inSchematic`), so scaffolding it put down outside the box stays (the first good run left 3
    cobblestone cells); at most 6 cells outside may change."""
    size = (5, 4, 5)
    cells = box_cells(size, PLANKS, gap={(0, 1, 2), (0, 2, 2)})
    f = {"r01_box.schem": schematics.sponge(size, cells)}
    return build_scenario(f"r01_build_schem_box_chat{int(chat)}", chat, x0, z0, f, size, cells, [(10, build_command("r01_box.schem", x0, z0))],
                          give(("oak_planks", 64), ("oak_planks", 64), ("iron_axe", 1), ("cobblestone", 64), ("iron_pickaxe", 1)), done_within=2400, outside_max=6,
                          plan={"place": [len(cells), len(cells) + 12], "break": [0, 0]}, messages=["Successfully loaded schematic for building", "Done building"])


def r02_build_clear(chat, x0=16600, z0=3000):
    """A schematic of a planks floor with air above it, where the world has stone in the air cells (placed by the scenario): the stone is broken with the
    pickaxe, the floor is placed, and it is done."""
    size = (5, 3, 5)
    cells = {(x, 0, z): PLANKS for x in range(5) for z in range(5)}
    stone = {(x, y, z): "minecraft:stone" for x in range(1, 4) for y in (1, 2) for z in range(1, 4)}
    f = {"r02_clear.schem": schematics.sponge(size, cells)}
    o = box_origin(x0, z0)
    prep = [f"setblock {o[0] + x} {o[1] + y} {o[2] + z} minecraft:stone" for (x, y, z) in sorted(stone)]
    return build_scenario(f"r02_build_clear_chat{int(chat)}", chat, x0, z0, f, size, cells, [(10, build_command("r02_clear.schem", x0, z0))],
                          give(("iron_pickaxe", 1), ("oak_planks", 64)), extra_prep=prep, initial=stone, done_within=2400, outside_max=2,
                          plan={"place": [len(cells), len(cells) + 6], "break": [len(stone), len(stone)]}, messages=["Done building"])


def r03_build_litematic_rotated(chat=False, x0=17200, z0=3000):
    """A .litematic of version 7 with two regions, the second with a negative size on X: three by two planks, and two stairs facing east one row
    beyond them. With buildSchematicRotation CLOCKWISE_90 the box turns a quarter: the planks are at x 1 to 2, z 0 to 2 and the stairs at x 0, z 1 to 2,
    facing south (a state turned with the game's own table). The facing is read from the world at the end."""
    stairs = "minecraft:oak_stairs[facing=east,half=bottom,shape=straight,waterlogged=false]"
    regions = {
        "planks": {"position": (0, 0, 0), "size": (3, 1, 2), "cells": {(x, 0, z): PLANKS for x in range(3) for z in range(2)}},
        "stairs": {"position": (2, 0, 2), "size": (-2, 1, 1), "cells": {(0, 0, 0): stairs, (1, 0, 0): stairs}},
    }
    f = {"r03_rotated.litematic": schematics.litematica(regions)}
    cells = {(x, 0, z): PLANKS for x in (1, 2) for z in range(3)}
    cells.update({(0, 0, 1): "minecraft:oak_stairs", (0, 0, 2): "minecraft:oak_stairs"})
    o = box_origin(x0, z0)
    sample_at = 2900
    sc = build_scenario(f"r03_build_litematic_rotated_chat{int(chat)}", chat, x0, z0, f, (3, 1, 3), cells, [(10, build_command("r03_rotated.litematic", x0, z0))],
                        give(("oak_planks", 64), ("oak_stairs", 64)), settings={"buildSchematicRotation": "CLOCKWISE_90"}, end=3000, stop_when="never",
                        done_within=2400, outside_max=2, messages=["Done building"],
                        sample_facing={"tick": sample_at, "cells": [[o[0], o[1], o[2] + 1], [o[0], o[1], o[2] + 2]], "has": "facing=south"})
    sc["events"].append({"t": sample_at, "sample": [[o[0], o[1], o[2] + 1], [o[0], o[1], o[2] + 2]]})
    return sc


def r04_build_layers(chat=False, x0=17800, z0=3000):
    """buildInLayers with a layer height of 1 and a 3 x 3 x 3 cube of planks. From the timeline of the watched cells: no cell of a layer was placed
    while a cell of the layer below was still missing. "Starting layer" is said for each layer."""
    size = (3, 3, 3)
    cells = {(x, y, z): PLANKS for x in range(3) for y in range(3) for z in range(3)}
    f = {"r04_cube.schem": schematics.sponge(size, cells)}
    return build_scenario(f"r04_build_layers_chat{int(chat)}", chat, x0, z0, f, size, cells, [(10, build_command("r04_cube.schem", x0, z0))],
                          give(("oak_planks", 64), ("iron_axe", 1)), settings={"buildInLayers": "true", "layerHeight": "1"}, done_within=2400, outside_max=2,
                          timeline=True, layers={"height": 1, "bottom_up": True}, messages=["Starting layer", "Done building"])


def r05_missing_then_resume(chat, x0=18400, z0=3000):
    """A glass floor, 3 x 1 x 3, with only planks in the hotbar. The builder says what is missing and pauses ("Unable to do it. Pausing."); the
    player stands still; at tick 400 glass is given, at 420 `/b resume` is typed, and the build is completed."""
    size = (3, 1, 3)
    cells = {(x, 0, z): "minecraft:glass" for x in range(3) for z in range(3)}
    f = {"r05_glass.schem": schematics.sponge(size, cells)}
    return build_scenario(f"r05_missing_then_resume_chat{int(chat)}", chat, x0, z0, f, size, cells,
                          [(10, build_command("r05_glass.schem", x0, z0)), (420, "b resume")], give(("oak_planks", 64)),
                          end=3000, stop_when="idle", done_within=2600, outside_max=2, timeline=True,
                          messages=["Missing materials for at least:", "Unable to do it. Pausing. resume to resume, cancel to cancel", "Resumed", "Done building"],
                          paused_still=[200, 400], console=[(400, "give @p minecraft:glass 64")])


def r06_build_stop(chat, x0=19000, z0=3000):
    """`b stop` at tick 130 (120 ticks into a big build, a 12 x 1 x 12 floor of planks): no keys and no path afterwards, and no cell changes in the 10
    ticks that follow (or later)."""
    size = (12, 1, 12)
    cells = {(x, 0, z): PLANKS for x in range(12) for z in range(12)}
    f = {"r06_floor.schem": schematics.sponge(size, cells)}
    return build_scenario(f"r06_stop_build_chat{int(chat)}", chat, x0, z0, f, size, cells,
                          [(10, build_command("r06_floor.schem", x0, z0)), (130, "b stop")], give(("oak_planks", 64), ("oak_planks", 64)),
                          end=420, stop_when="never", timeline=True, stop_tick=130, quiet_after=10, partial=True, outside_max=2)


def r07_mcedit_legacy(chat=False, x0=19600, z0=3000):
    """An MCEdit .schematic (Alpha) of ids 1 (stone) and 4 (cobblestone) and one cell of id 2, which was grass: it is not a block of this version, so
    it reads as air and the cell stays empty (docs/deviations.md U31)."""
    size = (3, 1, 3)
    ids = {(x, 0, 0): 1 for x in range(3)}
    ids.update({(x, 0, 1): 4 for x in range(3)})
    ids.update({(0, 0, 2): 1, (1, 0, 2): 2, (2, 0, 2): 4})
    cells = {c: ("minecraft:stone" if i == 1 else "minecraft:cobblestone") for c, i in ids.items() if i in (1, 4)}
    f = {"r07_legacy.schematic": schematics.mcedit(size, ids)}
    return build_scenario(f"r07_mcedit_legacy_chat{int(chat)}", chat, x0, z0, f, size, cells, [(10, build_command("r07_legacy.schematic", x0, z0))],
                          give(("stone", 64), ("cobblestone", 64)), done_within=1800, outside_max=2, messages=["Done building"])


def r08_substitutes(chat=False, x0=20200, z0=3000):
    """buildSubstitutes stone->cobblestone with a schematic of stone and a hotbar of cobblestone only: every cell ends as cobblestone."""
    size = (3, 1, 3)
    cells = {(x, 0, z): "minecraft:cobblestone" for x in range(3) for z in range(3)}
    f = {"r08_stone.schem": schematics.sponge(size, {c: "minecraft:stone" for c in cells})}
    return build_scenario(f"r08_substitutes_chat{int(chat)}", chat, x0, z0, f, size, cells, [(10, build_command("r08_stone.schem", x0, z0))],
                          give(("cobblestone", 64)), settings={"buildSubstitutes": "stone->cobblestone"}, done_within=1800, outside_max=2,
                          messages=["Done building"])


def r09_repeat(chat=False, x0=20800, z0=3000):
    """buildRepeat 6,0,0 with buildRepeatCount 2: the 3 x 1 x 3 floor is built, then again 6 blocks to the east, and it is done."""
    size = (3, 1, 3)
    one = {(x, 0, z): PLANKS for x in range(3) for z in range(3)}
    cells = dict(one)
    cells.update({(x + 6, y, z): n for (x, y, z), n in one.items()})
    f = {"r09_floor.schem": schematics.sponge(size, one)}
    sc = build_scenario(f"r09_repeat_chat{int(chat)}", chat, x0, z0, f, (9, 1, 3), cells, [(10, build_command("r09_floor.schem", x0, z0))],
                        give(("oak_planks", 64)), settings={"buildRepeat": "6,0,0", "buildRepeatCount": "2"}, done_within=3000, outside_max=2,
                        messages=["Repeating build in vector", "Done building"], end=3400)
    return sc


def r10_fallback_extension(chat=False, x0=21400, z0=3000):
    """`b build r10_box` has no extension, so schematicFallbackExtension ("schematic") is added and the file r10_box.schematic is found and built.
    `b build nosuch` says "Cannot find" with the whole path, which also shows where the schematics folder is."""
    size = (3, 1, 3)
    ids = {(x, 0, z): 1 for x in range(3) for z in range(3)}
    cells = {c: "minecraft:stone" for c in ids}
    f = {"r10_box.schematic": schematics.mcedit(size, ids)}
    o = box_origin(x0, z0)
    return build_scenario(f"r10_fallback_extension_chat{int(chat)}", chat, x0, z0, f, size, cells,
                          [(5, "set:b build nosuch"), (40, f"b build r10_box {o[0]} {o[1]} {o[2]}")], give(("stone", 64)), done_within=1800, outside_max=2,
                          messages=["Successfully loaded schematic for building", "Done building"], cannot_find="nosuch.schematic")


def full_hotbar_then(*stacks):
    """Console lines that empty the inventory, fill the nine hotbar slots with things that are no building block (slot 4 and 5 hold the pickaxe and the
    axe), and then give `stacks`: with the hotbar full they land in the main inventory (slot 9 on), where only `allowInventory` finds them."""
    filler = [("iron_pickaxe", 1), ("iron_axe", 1), ("stick", 8), ("apple", 4), ("bread", 4), ("coal", 8), ("torch", 8), ("arrow", 8), ("string", 8)]
    return give(*filler, *stacks)


def r11_build_from_main_inventory(chat=False, x0=22000, z0=3000):
    """NOT RUN YET (written with the port of `allowInventory`, 2026-11-10). A 3 x 1 x 3 floor of planks, with the nine hotbar slots full and 64 planks
    in the main inventory, `allowInventory` on: the builder asks for the planks to be swapped onto the hotbar (a window click, `ClickType.SWAP`) and builds
    the floor. Every cell as the schematic says, "Done building", no health lost."""
    size = (3, 1, 3)
    cells = {(x, 0, z): PLANKS for x in range(3) for z in range(3)}
    f = {"r11_floor.schem": schematics.sponge(size, cells)}
    return build_scenario(f"r11_build_from_main_inventory_chat{int(chat)}", chat, x0, z0, f, size, cells, [(10, build_command("r11_floor.schem", x0, z0))],
                          full_hotbar_then(("oak_planks", 64)), settings={"allowInventory": "true"}, done_within=2400, outside_max=2,
                          messages=["Successfully loaded schematic for building", "Done building"])


def r12_main_inventory_only_if_stationary(chat=False, x0=22600, z0=3000):
    """NOT RUN YET. The same job as r11 with `inventoryMoveOnlyIfStationary` on and `ticksBetweenInventoryMoves` 5: the builder still finishes (the move waits for the
    pauser to stop the path and for the player to stand still)."""
    size = (3, 1, 3)
    cells = {(x, 0, z): PLANKS for x in range(3) for z in range(3)}
    f = {"r12_floor.schem": schematics.sponge(size, cells)}
    return build_scenario(f"r12_main_inventory_only_if_stationary_chat{int(chat)}", chat, x0, z0, f, size, cells, [(10, build_command("r12_floor.schem", x0, z0))],
                          full_hotbar_then(("oak_planks", 64)),
                          settings={"allowInventory": "true", "inventoryMoveOnlyIfStationary": "true", "ticksBetweenInventoryMoves": "5"}, done_within=2600,
                          outside_max=2, messages=["Successfully loaded schematic for building", "Done building"])


def build_scenarios():
    out = []
    for chat in (False, True):
        d = 100 if chat else 0
        out += [r01_build_schem_box(chat, x0=16000 + d), r02_build_clear(chat, x0=16600 + d)]
    out += [r03_build_litematic_rotated(), r04_build_layers()]
    for chat in (False, True):
        d = 100 if chat else 0
        out += [r05_missing_then_resume(chat, x0=18400 + d), r06_build_stop(chat, x0=19000 + d)]
    out += [r07_mcedit_legacy(), r08_substitutes(), r09_repeat(), r10_fallback_extension()]
    out += [r11_build_from_main_inventory(), r12_main_inventory_only_if_stationary()]
    return out


def build_files():
    """{file name: bytes} for every build scenario: the files run.py writes into the schematics folder before the game starts. Deterministic."""
    build_scenarios()
    return dict(_FILES)


def all_scenarios(include_far=True, controls=False, with_build=True):
    # The natural-terrain ones come first, at the spawn, before any world building moves the player away.
    out = [s05_natural_300(False), c03_explore(False), c03_explore(True)]
    if include_far:
        out.append(s10_far(False))
    for chat in (False, True):
        out += [s01_flat(chat), s02_wall(chat), s03_steps_and_diagonals(chat), s04_drops(chat)]
    for chat in (False, True):
        out += [s07_block_in_the_way(chat), s09_ladder_and_water(chat), s08_stop(chat), s11_slab_floor(chat)]
    out += [s06_sealed(False), s06_sealed(True)]
    out += new_scenarios()
    out += mine_scenarios()
    out += cache_scenarios(controls)
    out += follow_scenarios()
    out += build_scenarios() if with_build else []
    return out
