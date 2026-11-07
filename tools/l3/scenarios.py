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


def new_scenarios():
    out = []
    for chat in (False, True):
        out += [b01_break_stone(chat), b02_break_dirt_by_hand(chat), b03_bridge(chat), b04_pillar(chat), b04_pillar(chat, x0=6600, ledge=False), b05_dig_down(chat),
                b06_wall_break_allowed_vs_not(chat, True), b06_wall_break_allowed_vs_not(chat, False), b07_bucket_fall(chat), b08_stop_dig(chat), b09_dig_stone_by_hand(chat)]
    return out


def all_scenarios(include_far=True):
    # The natural-terrain ones come first, at the spawn, before any world building moves the player away.
    out = [s05_natural_300(False)]
    if include_far:
        out.append(s10_far(False))
    for chat in (False, True):
        out += [s01_flat(chat), s02_wall(chat), s03_steps_and_diagonals(chat), s04_drops(chat)]
    for chat in (False, True):
        out += [s07_block_in_the_way(chat), s09_ladder_and_water(chat), s08_stop(chat), s11_slab_floor(chat)]
    out += [s06_sealed(False), s06_sealed(True)]
    out += new_scenarios()
    return out
