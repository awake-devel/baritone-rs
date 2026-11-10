#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""L3: plays the scenarios of scenarios.py in the game and judges what the mod recorded.

  python3 tools/l3/run.py --out DIR [--only NAME_PREFIX ...] [--no-far] [--world NAME]    play the scenarios, then judge
  python3 tools/l3/run.py --out DIR --judge-only                                          judge files from an earlier run

The mod (mod/runtime.rs, "L3 script player") builds each world through the single-player server's console, teleports the player,
opens the chat when the scenario says so, gives the commands, and writes one row per tick. This script writes the spec, starts the
game of the instance baritone-test with BARITONE_RS_L3_SPEC set, waits for "finished", closes the game by its PID, and applies the
pass criteria. No input goes to the desktop. The world is created by `play --world` and is new each time; nothing of it is committed.

The mod must be built into the instance first (tools/README.md, "L3 scenarios"). One game or build at a time: this refuses to start
while a horizon-client, cargo or rustc process runs.

Writes <out>/judgement.json (everything) and, with --report FILE, a metadata-only summary for reports/.
"""
import argparse
import csv
import datetime
import json
import math
import os
import signal
import statistics
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import scenarios  # noqa: E402

LAUNCHER = os.path.expanduser("~/Applications/horizon/horizon-launcher-linux-x86_64.AppImage")
INSTANCE = "baritone-test"
SAVES = os.path.expanduser(f"~/.local/share/horizon/instances/{INSTANCE}/saves")
HORIZON_REVISION = "66f3981"


# ---------------------------------------------------------------- running the game

def running(name):
    return subprocess.run(["pgrep", "-x", name], capture_output=True, text=True).stdout.split()


def started_by(spec_path):
    """The `horizon-client` PIDs that carry this run's spec path in their environment, i.e. the game this script started.

    The game inherits the environment of `play`. A game that somebody else opened has no such variable, and is never listed."""
    mine = []
    for pid in running("horizon-client"):
        try:
            with open(f"/proc/{pid}/environ", "rb") as f:
                env = f.read().split(b"\0")
        except OSError:
            continue
        if f"BARITONE_RS_L3_SPEC={spec_path}".encode() in env:
            mine.append(pid)
    return mine


def check_world_name(world):
    """The scenarios clear the inventory and fill large boxes, so they may only run in a world made for them: a name starting with `l3-` that
    does not exist yet (the mod refuses any other name too). Returns an error text, or None."""
    if not world.startswith("l3-") or "/" in world or world in (".", ".."):
        return f"the world name must start with l3- and hold no slash (got {world!r}); nothing was started"
    if os.path.exists(os.path.join(SAVES, world)):
        return f"the world {world!r} already exists; the scenarios wipe what is around them, so give a new name; nothing was started"
    return None


# The game folder of the instance is the working directory of the game, and `b build` reads its `schematics` folder (docs/deviations.md D48). The build
# scenarios write their files here before the game starts and remove them afterwards. Scenario r10 shows in the chat where the mod looks.
SCHEMATICS = os.path.expanduser(f"~/.local/share/horizon/instances/{INSTANCE}/schematics")


def write_schematics(files, names):
    """Writes the schematic files of the selected scenarios. Refuses to replace a file that is not byte for byte what would be written.
    Returns what to remove afterwards: (the files made here, whether the folder was made here)."""
    made, made_dir = [], not os.path.isdir(SCHEMATICS)
    os.makedirs(SCHEMATICS, exist_ok=True)
    for n in sorted(names):
        path = os.path.join(SCHEMATICS, n)
        if os.path.exists(path):
            with open(path, "rb") as existing:
                same = existing.read() == files[n]
            if not same:
                sys.exit(f"{path} exists and differs from the test file; nothing was started")
            continue
        with open(path, "wb") as f:
            f.write(files[n])
        made.append(path)
    return made, made_dir


def remove_schematics(made, made_dir):
    for path in made:
        try:
            os.remove(path)
        except OSError:
            pass
    if made_dir:
        try:
            os.rmdir(SCHEMATICS)
        except OSError:
            pass


SETTINGS_FILE = os.path.expanduser(f"~/.local/share/horizon/instances/{INSTANCE}/horizon-mods/.horizon-mod-data/baritone_rs/settings.txt")


def preflight():
    # `/b set` saves at once, so a setting changed by hand in an earlier session outlives it and silently changes what the scenarios test
    # (allowPlace false made every bridge and pillar fail with "No path", 2026-11-08).
    try:
        bad = [l.strip() for l in open(SETTINGS_FILE) if l.split()[:1] and l.split()[0].startswith("allow") and l.split()[1:] == ["false"]]
    except OSError:
        bad = []
    if bad:
        sys.exit(f"blocked: {SETTINGS_FILE} turns settings off that the scenarios need ({bad}); put them back with `/b set NAME true`; nothing was started")
    busy = {n: running(n) for n in ("horizon-client", "cargo", "rustc")}
    busy = {n: p for n, p in busy.items() if p}
    if busy:
        sys.exit(f"blocked: a game or a build is running ({busy}); nothing was started")


def play(spec_path, out, world, timeout_s):
    env = dict(os.environ, BARITONE_RS_L3_SPEC=spec_path)
    log = open(os.path.join(out, "play.log"), "w")
    proc = subprocess.Popen([LAUNCHER, "play", "--instance", INSTANCE, "--world", world], env=env, stdout=log, stderr=subprocess.STDOUT,
                            start_new_session=True)
    started = time.time()
    result = os.path.join(out, "l3-result.txt")
    mine = set()
    status = "timeout"
    try:
        while time.time() - started < timeout_s:
            time.sleep(5)
            mine |= set(started_by(spec_path))
            if os.path.exists(result) and open(result).read().rstrip().endswith("finished"):
                status = "finished"
                break
            if proc.poll() is not None and not started_by(spec_path):
                status = "the game exited early"
                break
            done = open(result).read().count("\n") if os.path.exists(result) else 0
            print(f"  {int(time.time() - started)} s, {done} scenarios recorded", flush=True)
    finally:
        # Close what was started here, by PID, also when something failed.
        mine |= set(started_by(spec_path))
        for pid in mine:
            try:
                os.kill(int(pid), signal.SIGTERM)
            except ProcessLookupError:
                pass
        for _ in range(30):
            if not any(os.path.exists(f"/proc/{pid}") for pid in mine):
                break
            time.sleep(1)
        for pid in started_by(spec_path):
            try:
                os.kill(int(pid), signal.SIGKILL)
            except ProcessLookupError:
                pass
        if proc.poll() is None:
            proc.terminate()
        log.close()
    return status


# ---------------------------------------------------------------- reading what the mod wrote

def read_rows(path):
    with open(path) as f:
        return list(csv.DictReader(f))


def fnum(r, k):
    return float(r[k])


def read_meta(out):
    metas = {}
    with open(os.path.join(out, "l3-result.txt")) as f:
        for line in f:
            line = line.strip()
            if line.startswith("{"):
                m = json.loads(line)
                metas[m["name"]] = m
    return metas


def read_kv_lines(path):
    rows = []
    if not os.path.exists(path):
        return rows
    for line in open(path):
        d = {}
        for part in line.split():
            if "=" in part:
                k, v = part.split("=", 1)
                d[k] = v
        if d:
            rows.append(d)
    return rows


def read_notes(path):
    notes = []
    if os.path.exists(path):
        for line in open(path):
            parts = line.rstrip("\n").split("\t", 2)
            if len(parts) == 3:
                notes.append((int(parts[0]), parts[1], parts[2]))
    return notes


def speed(a, b):
    return math.hypot(fnum(b, "x") - fnum(a, "x"), fnum(b, "z") - fnum(a, "z"))


# ---------------------------------------------------------------- measurements

def movement_groups(rows):
    """Consecutive rows of one movement (kind, source, destination): actual ticks against the estimated cost."""
    groups = []
    for r in rows:
        if r["mk"] in ("0", ""):
            continue
        key = (r["mk"], r["msx"], r["msy"], r["msz"], r["mdx"], r["mdy"], r["mdz"])
        if groups and groups[-1]["key"] == key:
            groups[-1]["ticks"] += 1
        else:
            groups.append({"key": key, "kind": r["mk"], "est": float(r["mcost"]), "ticks": 1, "first": int(r["t"]), "water": False})
        if r.get("water") == "1":
            groups[-1]["water"] = True
    return groups


def per_movement(rows):
    groups = movement_groups(rows)
    kinds = {}
    for g in groups:
        k = kinds.setdefault(g["kind"] + (" (touching water)" if g["water"] else ""), {"count": 0, "estimated_ticks": 0.0, "actual_ticks": 0})
        k["count"] += 1
        k["estimated_ticks"] += g["est"]
        k["actual_ticks"] += g["ticks"]
    for k in kinds.values():
        k["estimated_ticks"] = round(k["estimated_ticks"], 2)
        k["ratio_actual_over_estimated"] = round(k["actual_ticks"] / k["estimated_ticks"], 3) if k["estimated_ticks"] else None
    est = sum(g["est"] for g in groups)
    act = sum(g["ticks"] for g in groups)
    return {
        "movements": len(groups),
        "estimated_ticks_total": round(est, 2),
        "actual_ticks_total": act,
        "ratio_actual_over_estimated": round(act / est, 3) if est else None,
        "by_kind": kinds,
    }


def stats(values):
    if not values:
        return None
    v = sorted(values)
    return {"n": len(v), "mean": round(statistics.fmean(v), 1), "median": round(statistics.median(v), 1),
            "p95": round(v[min(len(v) - 1, int(len(v) * 0.95))], 1), "max": round(v[-1], 1)}


def timing(out, name, rows, command_tick):
    searches = [s for s in read_kv_lines(os.path.join(out, name + ".searches.txt"))]
    snaps = read_kv_lines(os.path.join(out, name + ".snapshots.txt"))
    tick_us = [float(r["tick_us"]) for r in rows if command_tick is None or int(r["t"]) > command_tick]
    return {
        "searches": len(searches),
        "search_micros": stats([float(s["micros"]) for s in searches]),
        "search_results": sorted({s["kind"] for s in searches}),
        "snapshots": len(snaps),
        "snapshot_micros": stats([float(s["micros"]) for s in snaps]),
        "snapshot_chunks": stats([float(s["chunks"]) for s in snaps]),
        "mod_tick_micros": stats(tick_us),
    }



# ---------------------------------------------------------------- blocks: the plan against what was done

AIR = ("minecraft:air", "minecraft:cave_air", "minecraft:void_air")


def is_air(n):
    return n in AIR


def is_water(n):
    return n in ("minecraft:water", "minecraft:flowing_water")


def parse_cells(text):
    return {tuple(int(v) for v in c.split(",")) for c in text.split(";") if c}


def read_plan(path):
    """Per segment line of <name>.segments.txt: the cells to break and to place that the path meant when it was made."""
    plans = []
    if os.path.exists(path):
        for line in open(path):
            d = {}
            for part in line.split():
                if "=" in part and not part.startswith("movements_list"):
                    k, v = part.split("=", 1)
                    d[k] = v
            if "to_break" in d or "to_place" in d:
                plans.append({"break": parse_cells(d.get("to_break", "")), "place": parse_cells(d.get("to_place", ""))})
    return plans


def read_changes(meta):
    """The changed cells of the watch box: (x, y, z) -> (before, after)."""
    out = {}
    for line in meta.get("all_changes") or []:
        pos, change = line.split(": ", 1)
        before, after = change.split(" -> ")
        out[tuple(int(v) for v in pos.split(","))] = (before, after)
    return out


def hotbar_items(text):
    """'0:iron_pickaxe x1 d3; 1:cobblestone x64 d0' -> {item: (count, damage)} (the first stack of each item)."""
    out = {}
    for part in (text or "").split("; "):
        if not part:
            continue
        slot, rest = part.split(":", 1)
        item, count, dmg = rest.split(" ")
        out.setdefault(item, (int(count[1:]), int(dmg[1:])))
    return out


def hotbar_total(text, item):
    """The count of an item over all the stacks of a hotbar text."""
    total = 0
    for part in (text or "").split("; "):
        if not part:
            continue
        _, rest = part.split(":", 1)
        name, count, _ = rest.split(" ")
        if name == item:
            total += int(count[1:])
    return total


def judge_blocks(res, chk, meta, plans):
    """Checks of a scenario that is allowed to change blocks (chk has "plan"): every change was planned, every planned cell was
    done, the counts are the expected ones, nothing else changed. Water cells are transient (a bucket's water), counted apart."""
    ok = res["checks"]
    changes = read_changes(meta)
    broken = {c for c, (b, a) in changes.items() if not is_air(b) and not is_water(b) and (is_air(a) or is_water(a))}
    placed = {c for c, (b, a) in changes.items() if (is_air(b) or is_water(b)) and not is_air(a) and not is_water(a)}
    water = {c for c, (b, a) in changes.items() if c not in broken and c not in placed and (is_water(b) or is_water(a))}
    other = set(changes) - broken - placed - water
    res["blocks"] = {"broken": len(broken), "placed": len(placed), "water_cells": len(water), "other": sorted(other)[:8], "segments": len(plans)}
    union_break = set().union(*[p["break"] for p in plans]) if plans else set()
    union_place = set().union(*[p["place"] for p in plans]) if plans else set()
    ok["every broken block was in a plan's toBreak"] = broken <= union_break
    ok["every placed block was in a plan's toPlace"] = placed <= union_place
    if plans and not chk.get("last_path_may_be_unused"):
        # The last path was made from what was left to do, so all of it was done. An earlier one may have been replaced.
        # (Not for `mine`: the path to the next ore is made while the last ore is being broken, and the process ends before it is walked.)
        ok["every toBreak cell of the last path was broken"] = plans[-1]["break"] <= broken
        ok["every toPlace cell of the last path was placed"] = plans[-1]["place"] <= placed
    want = chk["plan"]
    if isinstance(want["break"], list):
        lo, hi = want["break"]
        ok[f"broke {lo} to {hi} blocks"] = lo <= len(broken) <= hi
    else:
        ok[f"broke exactly {want['break']} blocks"] = len(broken) == want["break"]
    if isinstance(want["place"], list):
        lo, hi = want["place"]
        ok[f"placed {lo} to {hi} blocks"] = lo <= len(placed) <= hi
    else:
        ok[f"placed exactly {want['place']} blocks"] = len(placed) == want["place"]
    ok["no other block changed" + (" (but water)" if chk.get("water_ok") else "")] = not other and (chk.get("water_ok") or not water)
    if "water_ok" in chk and water:
        res["notes"].append(f"{len(water)} water cells differ from the start at the end (a bucket's water, still flowing or settled)")
    start, end = hotbar_items(meta.get("hotbar_start")), hotbar_items(meta.get("hotbar_end"))
    res["hotbar_start"], res["hotbar_end"] = meta.get("hotbar_start"), meta.get("hotbar_end")
    for item, delta in (chk.get("stack_delta") or {}).items():
        ok[f"{item} count changed by {delta}"] = item in start and item in end and end[item][0] - start[item][0] == delta
    for item, dmg in (chk.get("tool_damage") or {}).items():
        ok[f"{item} lost {dmg} durability"] = item in start and item in end and end[item][1] - start[item][1] == dmg
    if chk.get("tool_damage_equals_broken"):
        item = chk["tool_damage_equals_broken"]
        ok[f"{item} lost one durability per block broken ({len(broken)})"] = item in start and item in end and end[item][1] - start[item][1] == len(broken)
    for item in chk.get("hotbar_end_has") or []:
        ok[f"{item} in the hotbar at the end"] = item in end
    # The slot: upstream never puts it back (docs/deviations.md D32), so this is reported, not judged.
    res["slot_start"], res["slot_end"] = meta.get("slot_start"), meta.get("slot_end")
    res["slot_restored_after_the_path"] = meta.get("slot_start") == meta.get("slot_end")


# ---------------------------------------------------------------- build: the box against the schematic

def norm_block(n):
    return "minecraft:air" if is_air(n) else n


def read_timeline(path):
    """<name>.timeline.txt: one line per changed watch cell and tick, `t x,y,z before after`."""
    out = []
    if os.path.exists(path):
        with open(path) as f:
            for line in f:
                t, pos, before, after = line.split()
                out.append((int(t), tuple(int(v) for v in pos.split(",")), before, after))
    return out


def judge_build(res, chk, meta, rows, notes, out, name, cmd):
    """`b build`: every cell of the schematic's box as the schematic says at the end, the messages, the time, what changed outside the box, and the
    optional checks of the scenario: the layers, the pause, the stop, the rotated stairs, the path in the message. The expectations are the box's own;
    the settings that loosen what counts as the same block (buildIgnoreProperties and the like) are not used by any scenario, so the names are compared."""
    ok = res["checks"]
    bx, by, bz, w, h, l = chk["box"]
    changes = read_changes(meta)
    initial = {tuple(c[:3]): c[3] for c in chk["initial"]}
    expect = {tuple(c[:3]): c[3] for c in chk["expect"]}
    cells = [(x, y, z) for x in range(bx, bx + w) for y in range(by, by + h) for z in range(bz, bz + l)]
    inside = set(cells)

    def final(c):
        return changes[c][1] if c in changes else initial.get(c, "minecraft:air")

    placed = {c for c, (b, a) in changes.items() if is_air(b) and not is_air(a)}
    broken = {c for c, (b, a) in changes.items() if not is_air(b) and is_air(a)}
    outside = sorted(c for c in changes if c not in inside)
    res["build"] = {"cells": len(cells), "expected_solid": len(expect), "placed": len(placed), "broken": len(broken), "outside_changed": outside[:8]}
    chat = res["chat_lines"]
    if not chk.get("partial"):
        wrong = [c for c in cells if norm_block(final(c)) != norm_block(expect.get(c, "minecraft:air"))]
        res["build"]["wrong_cells"] = wrong[:8]
        ok["every cell of the box is as the schematic says"] = not wrong
        done = [t for t, kind, text in notes if kind == "chat" and "Done building" in text]
        if "done_within" in chk:
            took = (done[0] - cmd) if done and cmd is not None else None
            res["ticks_from_command_to_done"] = took
            ok[f"'Done building' within {chk['done_within']} ticks of the command"] = took is not None and took <= chk["done_within"]
    ok[f"at most {chk['outside_max']} cells outside the box changed"] = len(outside) <= chk["outside_max"]
    for m in chk.get("messages") or []:
        ok[f"said {m!r}"] = any(m in line for line in chat)
    plan = chk.get("plan")
    if plan:
        for what, got in (("place", len(placed)), ("break", len(broken))):
            lo, hi = plan[what]
            ok[f"{what} {lo} to {hi} cells (did {got})"] = lo <= got <= hi
    tl = read_timeline(os.path.join(out, name + ".timeline.txt"))
    placed_at = {}
    for t, c, before, after in tl:
        if is_air(before) and not is_air(after):
            placed_at.setdefault(c, t)
    if "layers" in chk:
        lay = chk["layers"]
        layer_cells = [[c for c in expect if c[1] == by + n] for n in range(h)]
        problems = []
        for n in range(h - 1):
            lower = [placed_at[c] for c in layer_cells[n] if c in placed_at]
            upper = [placed_at[c] for c in layer_cells[n + 1] if c in placed_at]
            if lower and upper and len(lower) == len(layer_cells[n]) and min(upper) < max(lower):
                problems.append(f"layer {n + 1} began at tick {min(upper)} before layer {n} was complete at {max(lower)}")
        res["build"]["layer_problems"] = problems
        ok["no cell of a layer was placed before the layer below was complete"] = not problems and all(c in placed_at for c in expect)
    if "paused_still" in chk:
        t0, t1 = chk["paused_still"]
        window = [r for r in rows if t0 <= int(r["t"]) <= t1]
        moved = max((speed(window[i], window[i + 1]) for i in range(len(window) - 1)), default=1.0)
        res["build"]["max_speed_while_paused"] = round(moved, 4)
        ok[f"the player stood still between ticks {t0} and {t1}"] = bool(window) and moved < 0.02
        ok[f"nothing was built between ticks {t0} and {t1}"] = not any(t0 <= t <= t1 for t, c, b, a in tl)
    if "stop_tick" in chk:
        ts = chk["stop_tick"]
        ok["keys released and no path from the stop tick on"] = all(r["forced"] == "0" and r["pathing"] == "0" for r in rows if int(r["t"]) >= ts + 2)
        late = [t for t, c, b, a in tl if t > ts + chk["quiet_after"]]
        ok[f"no cell changed later than {chk['quiet_after']} ticks after the stop"] = not late
        before = [t for t, c, b, a in tl if t <= ts]
        ok["it was building before the stop"] = len(before) > 0
        ok["it was not finished at the stop"] = len(placed) < len(expect)
    if "sample_facing" in chk:
        sf = chk["sample_facing"]
        texts = [text for t, kind, text in notes if kind == "sample" and t == sf["tick"]]
        res["build"]["sample"] = texts
        ok[f"every sampled stair has {sf['has']}"] = bool(texts) and all(sf["has"] in part for part in texts[0].split(" ") if part)
    if "cannot_find" in chk:
        want = f"Cannot find {os.path.join(SCHEMATICS, chk['cannot_find'])}"
        ok["'Cannot find' names the file in the schematics folder of the instance"] = any(line == want for line in chat)


# ---------------------------------------------------------------- the pass criteria

def judge(out, sc, meta):
    name = sc["name"]
    chk = sc["check"]
    res = {"scenario": name, "chat": sc["chat"], "kind": chk["kind"], "checks": {}, "notes": []}
    if meta is None:
        res.update(status="not run", notes=["the mod recorded nothing for this scenario"])
        return res
    if meta.get("setup_error"):
        res.update(status="setup failed", notes=[meta["setup_error"]])
        return res
    csv_path = os.path.join(out, name + ".csv")
    rows = read_rows(csv_path) if os.path.exists(csv_path) else []
    if not rows:
        res.update(status="not run", notes=["no rows"])
        return res
    ok = res["checks"]
    cmd = meta.get("command_tick")
    first, last = rows[0], rows[-1]
    res["ticks"] = meta["ticks"]
    res["command_tick"] = cmd
    res["arrived_tick"] = meta.get("arrived_tick")
    notes = read_notes(os.path.join(out, name + ".notes.txt"))
    res["chat_lines"] = [n[2] for n in notes if n[1] == "chat"]
    res["chat_open_in_rows"] = sum(int(r["chat"]) for r in rows)
    if sc["chat"]:
        # For `mine` and `goto <block>` the run goes on 60 ticks after the process ended (and a right click on a table replaces the chat screen).
        n_rows = len(rows) - 60 if chk["kind"] in ("mine", "goto_block") else len(rows)
        ok["chat was open during the walk"] = res["chat_open_in_rows"] >= n_rows * 0.9
    ok["never died"] = not meta["died"]
    if chk.get("probe_unloaded"):
        # The chunk cache scenarios: the chunks of the target must have been out of the game when the command was given.
        probes = [n[2] for n in notes if n[1] == "probe"]
        res["probes"] = probes
        ok["the target's chunks were not loaded when the command was given"] = bool(probes) and all(p.endswith("loaded=false") for p in probes)
    if "min_distance_walked" in chk:
        walked = sum(speed(rows[i], rows[i + 1]) for i in range(len(rows) - 1) if cmd is None or int(rows[i]["t"]) >= cmd and abs(fnum(rows[i + 1], "x") - fnum(rows[i], "x")) < 5 and abs(fnum(rows[i + 1], "z") - fnum(rows[i], "z")) < 5)
        res["distance_walked_after_command"] = round(walked, 1)
        ok[f"walked at least {chk['min_distance_walked']} blocks after the command"] = walked >= chk["min_distance_walked"]
    if chk.get("blocks_changed") == 0 and meta.get("blocks_changed") is not None:
        ok["blocks changed = 0"] = meta["blocks_changed"] == 0
        res["watch_cells"] = meta.get("watch_cells")
    if chk.get("no_health_loss"):
        ok["no fall damage (health never below its start)"] = meta["min_health"] is not None and meta["min_health"] >= float(first["health"]) - 0.01
    res["min_health"] = meta.get("min_health")
    if meta.get("changed_cells"):
        res["changed_cells"] = meta["changed_cells"]
    kind = chk["kind"]
    fp = (fnum(last, "x"), fnum(last, "y"), fnum(last, "z"))
    res["final_position"] = [round(v, 2) for v in fp]

    if kind in ("arrive", "arrive_relative"):
        if kind == "arrive":
            gx, gy, gz = chk["goal"]
            near = abs(fp[0] - (gx + 0.5)) <= 1.0 and abs(fp[2] - (gz + 0.5)) <= 1.0 and abs(fp[1] - gy) <= 1.0
        else:
            sx, sz = fnum(first, "x"), fnum(first, "z")
            gx, gz = math.floor(sx) + chk["dx"], math.floor(sz) + chk["dz"]
            near = abs(fp[0] - (gx + 0.5)) <= 1.0 and abs(fp[2] - (gz + 0.5)) <= 1.0
        res["distance_to_goal_xz"] = round(math.hypot(fp[0] - (gx + 0.5), fp[2] - (gz + 0.5)), 2)
        arrived = meta.get("arrived_tick") is not None
        took = (meta["arrived_tick"] - cmd) if arrived and cmd is not None else None
        res["ticks_from_command_to_arrival"] = took
        ok["arrived (the mod said so)"] = arrived
        ok["within 1 block of the goal"] = near
        ok[f"within {chk['limit_ticks']} ticks"] = took is not None and took <= chk["limit_ticks"]
        if "sprint_min_fraction" in chk:
            moving = [r for r in rows if r["forced"] == "1" and int(r["t"]) > (cmd or 0) + 3]
            frac = sum(1 for r in moving if r["sprinting"] == "1") / max(1, len(moving))
            res["sprint_fraction_of_driven_ticks"] = round(frac, 2)
            ok["sprinting most of the way"] = frac >= chk["sprint_min_fraction"]
        searches = read_kv_lines(os.path.join(out, name + ".searches.txt"))
        good = [s for s in searches if s["cancelled"] == "false" and s["kind"] in ("SuccessToGoal", "SuccessSegment")]
        res["successful_searches"] = len(good)
        if "first_search_min_positions" in chk:
            after = [x for x in searches if int(x["t"].split("=")[-1]) >= (cmd or 0) and x["cancelled"] == "false" and x["kind"] in ("SuccessToGoal", "SuccessSegment")]
            first_n = int(after[0]["positions"]) if after else 0
            res["first_search_positions"] = first_n
            res["first_search_kind"] = after[0]["kind"] if after else None
            ok[f"the first path made after the command has more than {chk['first_search_min_positions']} positions (planned beyond the loaded chunks)"] = first_n > chk["first_search_min_positions"]
        if "min_segments" in chk:
            ok[f"more than one segment (>= {chk['min_segments']} searches that gave a path)"] = len(good) >= chk["min_segments"]
        if "min_searches_after" in chk:
            t0, n = chk["min_searches_after"]
            late = [s for s in good if int(s["t"].split("=")[-1]) >= t0]
            res["searches_after_the_block_appeared"] = len(late)
            ok["recalculated after the block appeared"] = len(late) >= 1
        res["per_movement"] = per_movement(rows)
        if "no_search_after" in chk:
            t0 = chk["no_search_after"]
            later = [s for s in good if int(s["t"].split("=")[-1]) >= t0]
            res["searches_after_the_event"] = len(later)
            ok["no search that gave a path after the event (the walk went on)"] = not later
        if "toggled" in chk:
            changes = read_changes(meta)
            want = {tuple(c) for c in chk["toggled"]}
            res["door_cells_changed"] = sorted(changes)
            ok["the door or gate was used (its cells changed state)"] = set(changes) == want
            ok["its block did not change into another"] = all(b == a for b, a in changes.values())
        if "plan" in chk:
            judge_blocks(res, chk, meta, read_plan(os.path.join(out, name + ".segments.txt")))
            pm = res["per_movement"]
            res["estimated_vs_actual_ticks"] = {"estimated": pm["estimated_ticks_total"], "actual": pm["actual_ticks_total"], "ratio": pm["ratio_actual_over_estimated"]}
            ok["actual ticks within 0.4x to 2.5x of the estimate (+30)"] = pm["estimated_ticks_total"] > 0 and 0.4 * pm["estimated_ticks_total"] - 30 <= pm["actual_ticks_total"] <= 2.5 * pm["estimated_ticks_total"] + 30
        if cmd is not None:
            res["first_driven_tick_after_command"] = next((int(r["t"]) - cmd for r in rows if r["forced"] == "1"), None)
        if kind == "arrive_relative" and not all(ok.values()) and chk.get("reported_ok"):
            stopped_with_message = (not meta["busy_at_end"]) and any("No path" in c or "Stopped" in c or "cancel" in c.lower() for c in res["chat_lines"])
            if stopped_with_message:
                res["notes"].append("did not arrive but stopped with a message: " + " | ".join(res["chat_lines"][-3:]))
    elif kind in ("mine", "goto_block"):
        # `b mine` and `b goto <block>`: no goal cell; the walk ends when the process is done (the mod is idle for 60 ticks).
        done_t = meta["ticks"] - 60
        took = done_t - cmd if cmd is not None else None
        res["ticks_from_command_to_done"] = took
        ok["stopped on its own (path off, keys released)"] = (not meta["busy_at_end"]) and (not meta["forced_at_end"])
        ok[f"done within {chk['limit_ticks']} ticks of the command"] = took is not None and took <= chk["limit_ticks"]
        changes = read_changes(meta)
        broken_cells = {c for c, (b, a) in changes.items() if not is_air(b) and not is_water(b) and (is_air(a) or is_water(a))}
        for c in chk.get("ores_broken") or []:
            ok[f"ore {c} was broken"] = tuple(c) in broken_cells
        for c in chk.get("ores_intact") or []:
            ok[f"ore {c} is still there"] = tuple(c) not in changes
        if "ores_broken_count" in chk:
            n = sum(1 for c in chk["ores_all"] if tuple(c) in broken_cells)
            res["ores_broken"] = n
            ok[f"exactly {chk['ores_broken_count']} of the {len(chk['ores_all'])} ores were broken"] = n == chk["ores_broken_count"]
        for c in chk.get("intact") or []:
            ok[f"{c} was not touched"] = tuple(c) not in changes
        res["items_end"] = {}
        for item, want in (chk.get("items_end") or {}).items():
            have = hotbar_total(meta.get("hotbar_end"), item)
            res["items_end"][item] = have
            ok[f"{item} in the hotbar: {want}"] = have == want
        res["hotbar_start"], res["hotbar_end"] = meta.get("hotbar_start"), meta.get("hotbar_end")
        res["container_open_ticks"] = meta.get("container_open_ticks")
        if chk.get("opens_container"):
            ok["the block was right clicked open (a container or station screen was open)"] = bool(meta.get("container_open_ticks"))
        if "have_message" in chk:
            ok[f"said 'Have {chk['have_message']} valid items'"] = any(f"Have {chk['have_message']} valid items" in m for m in res["chat_lines"])
        if kind == "goto_block":
            bx, by, bz = chk["block"]
            d = math.hypot(fp[0] - (bx + 0.5), fp[2] - (bz + 0.5))
            res["distance_to_block_xz"] = round(d, 2)
            ok[f"ended within {chk['radius']} blocks of the block"] = d <= chk["radius"] and abs(fp[1] - by) <= 1.5
        searches = read_kv_lines(os.path.join(out, name + ".searches.txt"))
        res["successful_searches"] = len([s for s in searches if s["cancelled"] == "false" and s["kind"] in ("SuccessToGoal", "SuccessSegment")])
        res["per_movement"] = per_movement(rows)
        judge_blocks(res, chk, meta, read_plan(os.path.join(out, name + ".segments.txt")))
        pm = res["per_movement"]
        res["estimated_vs_actual_ticks"] = {"estimated": pm["estimated_ticks_total"], "actual": pm["actual_ticks_total"], "ratio": pm["ratio_actual_over_estimated"]}
        ok["no 'Unable to find any path' in the chat"] = not any("Unable to find any path" in m for m in res["chat_lines"])
    elif kind == "follow":
        # `b follow entity ...` after NoAI mobs that the scenario moves at known ticks (positions are the scenario's own, not read back).
        # Per step: the player gets within radius + tolerance within `catch_up_ticks` of the step (or of the command) and stays for `hold_ticks`
        # until the next step. Overall: never farther from the target than the longest jump plus that radius. With a decoy of another type:
        # far from it at the end of every step.
        reach = chk["radius"] + chk["tolerance"]
        byt = {int(r["t"]): r for r in rows}
        target = chk["target"]
        end_t = max(byt)

        def target_at(t):
            cur = target[0]
            for st in target:
                if st[0] <= t:
                    cur = st
            return cur[1], cur[2]

        def dist_target(t):
            tx, tz = target_at(t)
            return math.hypot(fnum(byt[t], "x") - tx, fnum(byt[t], "z") - tz)

        starts = [max(st[0], cmd or 0) for st in target]
        ends = [target[i + 1][0] - 1 if i + 1 < len(target) else end_t for i in range(len(target))]
        hops = [math.hypot(target[i + 1][1] - target[i][1], target[i + 1][2] - target[i][2]) for i in range(len(target) - 1)]
        first_t = min(byt)
        hops.append(dist_target(max(starts[0], first_t)))
        steps_out = []
        for i, (a, b) in enumerate(zip(starts, ends)):
            caught = next((t for t in range(a, b + 1) if t in byt and dist_target(t) <= reach), None)
            hold = [dist_target(t) for t in range(max(a, b - chk["hold_ticks"] + 1), b + 1) if t in byt]
            steps_out.append({"from_tick": a, "to_tick": b, "catch_up_ticks": None if caught is None else caught - a,
                              "distance_at_end": round(hold[-1], 2) if hold else None, "hold_min": round(min(hold), 2) if hold else None, "hold_max": round(max(hold), 2) if hold else None})
            ok[f"step {i + 1}: within {reach} blocks of the target within {chk['catch_up_ticks']} ticks"] = caught is not None and caught - a <= chk["catch_up_ticks"]
            ok[f"step {i + 1}: stayed within {reach} blocks for the last {chk['hold_ticks']} ticks"] = bool(hold) and max(hold) <= reach
        res["follow"] = {"reach": reach, "steps": steps_out}
        walked = [dist_target(t) for t in range(max(cmd or 0, first_t), end_t + 1) if t in byt]
        res["follow"]["max_distance"] = round(max(walked), 2)
        res["follow"]["longest_jump"] = round(max(hops), 2)
        ok[f"never farther than the longest jump ({max(hops):.1f}) plus {reach}"] = max(walked) <= max(hops) + reach
        ok["stayed at the height of the floor"] = all(abs(fnum(r, "y") - (scenarios.SKY + 1)) <= 1.5 for r in rows)
        if chk.get("decoy"):
            far = []
            for i, e in enumerate(chk["decoy"]):
                tick = ends[i] if i < len(ends) else end_t
                if e[1] == "rel":
                    sample = byt[e[0]]
                    dx, dz = fnum(sample, "x") + e[2], fnum(sample, "z") + e[3]
                else:
                    dx, dz = e[1], e[2]
                far.append(round(math.hypot(fnum(byt[tick], "x") - dx, fnum(byt[tick], "z") - dz), 2))
                ok[f"step {i + 1}: at least {chk['decoy_min_distance']} blocks from the other kind of mob"] = far[-1] >= chk["decoy_min_distance"]
            res["follow"]["distance_to_decoy_at_step_ends"] = far
        sp = [speed(rows[i], rows[i + 1]) for i in range(len(rows) - 1) if int(rows[i]["t"]) > (cmd or 0)]
        res["follow"]["max_speed_blocks_per_tick"] = round(max(sp), 3) if sp else None
    elif kind == "unreachable":
        msgs = res["chat_lines"]
        ok["stopped on its own (path off, keys released)"] = (not meta["busy_at_end"]) and (not meta["forced_at_end"])
        ok["said why in the chat"] = any("No path" in m for m in msgs)
        ok[f"within {chk['limit_ticks']} ticks of the command"] = cmd is not None and meta["ticks"] - cmd <= chk["limit_ticks"]
        # Walking in circles: how far did it travel in total, against the distance between start and end.
        total = sum(speed(rows[i], rows[i + 1]) for i in range(len(rows) - 1))
        res["distance_walked"] = round(total, 1)
        res["per_movement"] = per_movement(rows)
    elif kind == "stop_dig":
        ts = chk["stop_tick"]
        byt = {int(r["t"]): r for r in rows}
        ok["keys released at the stop tick"] = byt[ts]["forced"] == "0"
        ok["no keys and no path from the stop tick on"] = all(r["forced"] == "0" and r["pathing"] == "0" for r in rows if int(r["t"]) >= ts)
        # A dig that went on would have finished 150 ticks after it began: the run lasts well beyond that. blocks_changed == 0 is checked above.
        res["ticks_after_the_stop"] = len(rows) - ts
        ok["the run went on at least 200 ticks past the stop"] = len(rows) - ts >= 200
        ok["the mod did not move the player after the stop"] = math.hypot(fnum(byt[ts], "x") - fnum(rows[-1], "x"), fnum(byt[ts], "z") - fnum(rows[-1], "z")) < 0.5
    elif kind == "explore":
        ts = chk["stop_tick"]
        byt = {int(r["t"]): r for r in rows}
        sx, sz = fnum(rows[0], "x"), fnum(rows[0], "z")
        dist = {t: math.hypot(fnum(r, "x") - sx, fnum(r, "z") - sz) for t, r in byt.items()}
        cmd0 = cmd or 0
        early = max([d for t, d in dist.items() if t <= cmd0 + 400] or [0.0])
        late = max([d for t, d in dist.items() if cmd0 + 400 < t <= ts] or [0.0])
        total = sum(speed(rows[i], rows[i + 1]) for i in range(len(rows) - 1) if int(rows[i]["t"]) <= ts)
        res["explore"] = {"walked": round(total, 1), "max_distance_first_400_ticks": round(early, 1), "max_distance_after": round(late, 1),
                          "distance_at_stop": round(dist.get(ts, 0.0), 1), "distance_every_200_ticks": [round(dist[t], 1) for t in sorted(dist) if t % 200 == 0]}
        ok[f"walked at least {chk['min_distance_walked']} blocks"] = total >= chk["min_distance_walked"]
        ok[f"got at least {chk['min_far']} blocks away from the start"] = max(early, late) >= chk["min_far"]
        ok["went farther in the later part than in the first 400 ticks"] = late > early
        ok["keys released at the stop tick"] = byt[ts]["forced"] == "0"
        ok["no keys and no path from the stop tick on"] = all(r["forced"] == "0" and r["pathing"] == "0" for r in rows if int(r["t"]) >= ts)
        ok["said 'ok canceled' or similar at the stop"] = any(n[0] >= ts and n[1] == "chat" and "cancel" in n[2].lower() for n in notes)
        a, b = byt[min(ts + 30, len(rows) - 1)], rows[-1]
        ok["stood still from 30 ticks after the stop"] = math.hypot(fnum(a, "x") - fnum(b, "x"), fnum(a, "z") - fnum(b, "z")) < 0.05
    elif kind == "stop":
        ts = chk["stop_tick"]
        byt = {int(r["t"]): r for r in rows}
        ok["keys released at the stop tick"] = byt[ts]["forced"] == "0"
        ok["no keys and no path from the stop tick on"] = all(r["forced"] == "0" and r["pathing"] == "0" for r in rows if int(r["t"]) >= ts)
        before = byt[ts - 1]
        res["speed_the_tick_before_the_stop"] = round(speed(byt[ts - 2], before), 3)
        if chk.get("not_walking_before"):
            ok["was not walking at the stop (the scan was still running)"] = res["speed_the_tick_before_the_stop"] < 0.05
            ok["the walk never started afterwards (the player did not move)"] = math.hypot(fnum(byt[ts], "x") - fnum(rows[-1], "x"), fnum(byt[ts], "z") - fnum(rows[-1], "z")) < 0.01
            ok[f"the run went on at least {chk['still_for_ticks']} ticks past the stop (the scan's result had arrived)"] = len(rows) - ts >= chk["still_for_ticks"]
        else:
            ok["was walking before the stop"] = res["speed_the_tick_before_the_stop"] > 0.2
        settle = None
        still = 0
        for t in range(ts + 1, len(rows)):
            if t in byt and t - 1 in byt:
                still = still + 1 if speed(byt[t - 1], byt[t]) < 0.005 else 0
                if still >= 5 and settle is None:
                    settle = t - 4
        res["ticks_until_still"] = None if settle is None else settle - ts
        res["coasting_distance_blocks"] = round(math.hypot(fnum(byt[ts], "x") - fnum(rows[-1], "x"), fnum(byt[ts], "z") - fnum(rows[-1], "z")), 3)
        ok["stood still within 30 ticks of the stop"] = settle is not None and settle - ts <= 30
        if settle is not None:
            a, b = byt[settle], rows[-1]
            ok["no movement after that"] = math.hypot(fnum(a, "x") - fnum(b, "x"), fnum(a, "z") - fnum(b, "z")) < 0.01
    elif kind == "build":
        judge_build(res, chk, meta, rows, notes, out, name, cmd)
    res["timing"] = timing(out, name, rows, cmd)
    res["status"] = "pass" if all(ok.values()) else "fail"
    if kind == "arrive_relative" and res["status"] == "fail" and chk.get("reported_ok") and res["notes"]:
        res["status"] = "reported"
    return res


def summarize(results):
    allm = {}
    for r in results:
        t = r.get("timing")
        if not t:
            continue
        for k in ("search_micros", "snapshot_micros", "snapshot_chunks", "mod_tick_micros"):
            if t[k]:
                allm.setdefault(k, []).append(t[k])
    out = {}
    for k, lst in allm.items():
        out[k] = {"max": max(x["max"] for x in lst), "median_of_medians": round(statistics.median(x["median"] for x in lst), 1), "runs": len(lst)}
    return out


# ---------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, help="folder for the recorded files")
    ap.add_argument("--only", nargs="*", help="run only the scenarios whose name starts with one of these")
    ap.add_argument("--no-far", action="store_true", help="leave out the 1500-block scenario")
    ap.add_argument("--world", help="name of the new world: must start with l3- and must not exist (default: l3-<time>)")
    ap.add_argument("--judge-only", action="store_true")
    ap.add_argument("--controls", action="store_true", help="also play the cache scenarios with chunkCaching off; they are expected to fail")
    ap.add_argument("--timeout", type=int, default=3600, help="seconds to wait for the whole run")
    ap.add_argument("--report", help="write the metadata-only summary here")
    args = ap.parse_args()
    out = os.path.abspath(args.out)
    scs = scenarios.all_scenarios(include_far=not args.no_far, controls=args.controls)
    if args.only:
        scs = [s for s in scs if any(s["name"].startswith(p) for p in args.only)]
    status = "judged earlier files"
    if not args.judge_only:
        preflight()
        if os.path.exists(out) and os.listdir(out):
            sys.exit(f"{out} is not empty; use a new folder")
        os.makedirs(out, exist_ok=True)
        spec = {"out": out, "scenarios": [{k: v for k, v in s.items() if k != "check"} for s in scs]}
        spec_path = os.path.join(out, "spec.json")
        with open(spec_path, "w") as f:
            json.dump(spec, f, indent=1)
        world = args.world or "l3-" + datetime.datetime.now().strftime("%m%d-%H%M%S")
        problem = check_world_name(world)
        if problem:
            sys.exit(problem)
        print(f"playing {len(scs)} scenarios in a new world", flush=True)
        files = scenarios.build_files()
        wanted = {n for s in scs for n in s.get("files", [])}
        made, made_dir = write_schematics(files, wanted) if wanted else ([], False)
        try:
            status = play(spec_path, out, world, args.timeout)
        finally:
            remove_schematics(made, made_dir)
        print("game:", status, flush=True)
    metas = read_meta(out) if os.path.exists(os.path.join(out, "l3-result.txt")) else {}
    results = [judge(out, s, metas.get(s["name"])) for s in scs]
    summary = {"horizon_revision": HORIZON_REVISION, "run": status, "scenarios": results, "overall_timing": summarize(results)}
    with open(os.path.join(out, "judgement.json"), "w") as f:
        json.dump(summary, f, indent=1)
    for r in results:
        failed = [k for k, v in r["checks"].items() if not v]
        print(f"{r['status'].upper():8} {r['scenario']:34} ticks={r.get('ticks')} {('failed: ' + '; '.join(failed)) if failed else ''} {' '.join(r['notes'])[:160]}")
    if args.report:
        keep = ("scenario", "chat", "status", "ticks", "blocks", "estimated_vs_actual_ticks", "slot_start", "slot_end", "slot_restored_after_the_path", "hotbar_start", "hotbar_end", "ticks_from_command_to_arrival", "command_tick", "arrived_tick", "distance_to_goal_xz", "min_health", "successful_searches", "items_end", "container_open_ticks", "ores_broken", "distance_to_block_xz", "ticks_from_command_to_done", "probes", "distance_walked_after_command", "first_search_positions", "first_search_kind", "explore", "chat_lines", "speed_the_tick_before_the_stop", "ticks_until_still", "coasting_distance_blocks", "follow")
        slim = [{k: r[k] for k in keep if k in r} | {"failed_checks": [k for k, v in r["checks"].items() if not v]} for r in results]
        rep = {"horizon_revision": HORIZON_REVISION, "instance": INSTANCE, "run": status, "scenarios": slim, "overall_timing": summary["overall_timing"]}
        with open(args.report, "w") as f:
            json.dump(rep, f, indent=1)
            f.write("\n")
    bad = [r for r in results if r["status"] not in ("pass", "reported")]
    sys.exit(1 if bad or status not in ("finished", "judged earlier files") else 0)


if __name__ == "__main__":
    main()
