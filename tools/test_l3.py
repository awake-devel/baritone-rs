# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Checks the L3 scenario definitions without the game: the console lines are within the server's limits, names are unique, and the
pass criteria read a recorded run the way run.py says."""
import csv
import json
import os
import re
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "l3"))
import run  # noqa: E402
import scenarios  # noqa: E402


class SpecTests(unittest.TestCase):
    def test_names_are_unique_and_json_clean(self):
        scs = scenarios.all_scenarios()
        names = [s["name"] for s in scs]
        self.assertEqual(len(names), len(set(names)))
        json.dumps(scs)

    def test_fills_stay_within_the_servers_limit(self):
        for s in scenarios.all_scenarios():
            for line in s["prep"]:
                m = re.match(r"fill (-?\d+) (-?\d+) (-?\d+) (-?\d+) (-?\d+) (-?\d+) ", line)
                if m:
                    x0, y0, z0, x1, y1, z1 = map(int, m.groups())
                    volume = (x1 - x0 + 1) * (y1 - y0 + 1) * (z1 - z0 + 1)
                    self.assertGreater(volume, 0, line)
                    self.assertLessEqual(volume, 32768, f"{s['name']}: {line}")

    def test_every_scenario_that_builds_forceloads_what_it_builds_in(self):
        for s in scenarios.all_scenarios():
            if not s["prep"]:
                continue
            m = re.match(r"forceload add (-?\d+) (-?\d+) (-?\d+) (-?\d+)", s["prep"][2])
            self.assertTrue(m, s["name"])
            fx0, fz0, fx1, fz1 = map(int, m.groups())
            self.assertLessEqual(((fx1 >> 4) - (fx0 >> 4) + 1) * ((fz1 >> 4) - (fz0 >> 4) + 1), 256)
            for line in s["prep"][3:]:
                nums = list(map(int, re.findall(r"(?<![\w.])-?\d+(?![\w.])", line.split("minecraft:")[0])[:6]))
                if line.startswith("fill"):
                    self.assertTrue(fx0 <= nums[0] and nums[3] <= fx1 and fz0 <= nums[2] and nums[5] <= fz1, f"{s['name']}: {line}")

    def test_the_new_scenarios_give_items_and_start_with_an_empty_hotbar(self):
        for s in scenarios.new_scenarios():
            if s["name"].startswith(("b08", "b09", "b02", "b10", "b11", "b12")):
                continue
            self.assertEqual(s["prep"][0], "clear @p", s["name"])
            self.assertTrue(any(l.startswith("give @p") for l in s["prep"]), s["name"])

    def test_the_door_gate_and_sand_scenarios_name_their_checks(self):
        by = {s["name"]: s for s in scenarios.new_scenarios()}
        for chat in (0, 1):
            self.assertEqual(by[f"b10_head_sand_chat{chat}"]["check"]["no_search_after"], 45)
            self.assertEqual(len(by[f"b11_door_chat{chat}"]["check"]["toggled"]), 2)
            self.assertEqual(len(by[f"b12_gate_chat{chat}"]["check"]["toggled"]), 1)
            self.assertTrue(any("sand" in e.get("console", "") for e in by[f"b10_head_sand_chat{chat}"]["events"]))

    def test_a_setting_is_set_before_the_command_and_put_back_after(self):
        off = [s for s in scenarios.new_scenarios() if s["name"] == "b06_wall_allowbreakoff_chat0"][0]
        self.assertIn({"t": 3, "set": "b set allowBreak false"}, off["events"])
        self.assertEqual(off["after"], ["b set allowBreak true"])

    def test_the_mine_scenarios_name_their_ores_and_counts(self):
        by = {s["name"]: s for s in scenarios.mine_scenarios()}
        self.assertEqual(len(by), 10)
        for chat in (0, 1):
            m1 = by[f"m01_mine_cave3_chat{chat}"]["check"]
            self.assertEqual(len(m1["ores_broken"]), 3)
            self.assertEqual(len({o[1] for o in m1["ores_broken"]}), 3, "three different depths")
            self.assertEqual(m1["items_end"], {"raw_iron": 3})
            m2 = by[f"m02_mine_quantity_chat{chat}"]
            self.assertEqual(m2["check"]["ores_broken_count"], 1)
            self.assertEqual(len(m2["check"]["ores_all"]), 4)
            self.assertTrue(any("raw_iron 2" in l for l in m2["prep"]))
            self.assertTrue(by[f"m03_goto_table_wall_chat{chat}"]["check"]["opens_container"])
            self.assertEqual(by[f"m05_stop_mine_scanning_chat{chat}"]["check"]["stop_tick"], 14)

    def test_the_far_scenario_can_be_left_out(self):
        self.assertFalse(any(s["name"].startswith("s10") for s in scenarios.all_scenarios(include_far=False)))


class BlockJudgeTests(unittest.TestCase):
    def test_a_range_of_broken_blocks_and_the_items_of_a_hotbar(self):
        self.assertEqual(run.hotbar_total("0:iron_pickaxe x1 d3; 1:raw_iron x2 d0; 2:raw_iron x1 d0", "raw_iron"), 3)
        self.assertEqual(run.hotbar_total("", "raw_iron"), 0)
        plans = [{"break": {(1, 200, 0), (2, 200, 0)}, "place": set()}]
        ch = [f"{x},200,0: minecraft:iron_ore -> minecraft:air" for x in (1, 2)]
        r = {"checks": {}, "notes": []}
        run.judge_blocks(r, {"plan": {"break": [1, 3], "place": [0, 2]}, "tool_damage_equals_broken": "iron_pickaxe"}, {"all_changes": ch, "hotbar_start": "0:iron_pickaxe x1 d0", "hotbar_end": "0:iron_pickaxe x1 d2"}, plans)
        self.assertTrue(all(r["checks"].values()), r["checks"])
        r = {"checks": {}, "notes": []}
        run.judge_blocks(r, {"plan": {"break": [3, 4], "place": 0}, "tool_damage_equals_broken": "iron_pickaxe", "last_path_may_be_unused": True},
                         {"all_changes": ch, "hotbar_start": "0:iron_pickaxe x1 d0", "hotbar_end": "0:iron_pickaxe x1 d1"}, plans)
        failed = [k for k, v in r["checks"].items() if not v]
        self.assertEqual(sorted(failed), ["broke 3 to 4 blocks", "iron_pickaxe lost one durability per block broken (2)"])

    def meta(self, changes, start="0:cobblestone x64 d0", end="0:cobblestone x61 d0"):
        return {"all_changes": changes, "hotbar_start": start, "hotbar_end": end, "slot_start": 6, "slot_end": 0}

    def judged(self, changes, plans, chk, **kw):
        res = {"checks": {}, "notes": []}
        run.judge_blocks(res, chk, self.meta(changes, **kw), plans)
        return res

    CHK = {"plan": {"break": 0, "place": 2}, "stack_delta": {"cobblestone": -3}}

    def test_a_planned_placement_passes(self):
        plans = [{"break": set(), "place": {(1, 200, 0), (2, 200, 0), (3, 200, 0)}}]
        ch = [f"{x},200,0: minecraft:air -> minecraft:cobblestone" for x in (1, 2, 3)]
        r = self.judged(ch, plans, {"plan": {"break": 0, "place": 3}, "stack_delta": {"cobblestone": -3}})
        self.assertTrue(all(r["checks"].values()), r["checks"])
        self.assertFalse(r["slot_restored_after_the_path"])

    def test_a_block_nobody_planned_and_a_wrong_count_fail(self):
        plans = [{"break": set(), "place": {(1, 200, 0)}}]
        ch = ["1,200,0: minecraft:air -> minecraft:cobblestone", "5,200,0: minecraft:stone -> minecraft:air"]
        r = self.judged(ch, plans, self.CHK, end="0:cobblestone x63 d0")
        failed = [k for k, v in r["checks"].items() if not v]
        self.assertIn("every broken block was in a plan's toBreak", failed)
        self.assertIn("placed exactly 2 blocks", failed)
        self.assertIn("cobblestone count changed by -3", failed)

    def test_a_planned_cell_that_was_not_done_fails(self):
        plans = [{"break": {(4, 201, 0)}, "place": set()}]
        r = self.judged([], plans, {"plan": {"break": 1, "place": 0}})
        self.assertFalse(r["checks"]["every toBreak cell of the last path was broken"])

    def test_water_is_only_allowed_when_the_scenario_says_so(self):
        ch = ["1,200,0: minecraft:air -> minecraft:water"]
        self.assertFalse(all(self.judged(ch, [], {"plan": {"break": 0, "place": 0}})["checks"].values()))
        self.assertTrue(all(self.judged(ch, [], {"plan": {"break": 0, "place": 0}, "water_ok": True})["checks"].values()))


class JudgeTests(unittest.TestCase):
    HEADER = "t,x,y,z,vx,vy,vz,yaw,on_ground,sprinting,forced,forward,health,dead,chat,pathing,calculating,seg,pos,tick_us,water,mk,mcost,msx,msy,msz,mdx,mdy,mdz"

    def write(self, d, name, rows):
        with open(os.path.join(d, name + ".csv"), "w") as f:
            f.write(self.HEADER + "\n")
            for r in rows:
                f.write(r + "\n")

    def row(self, t, x, forced=1, pathing=1, health=20, water=0):
        return f"{t},{x},201,0.5,0,0,0,0,1,1,{forced},{forced},{health},0,0,{pathing},0,0,{t},5,{water},Traverse,3.564,{t},201,0,{t + 1},201,0"

    def test_a_stop_is_judged_by_the_keys_at_the_stop_tick(self):
        sc = {"name": "x", "chat": False, "check": {"kind": "stop", "stop_tick": 5, "blocks_changed": 0}}
        meta = {"name": "x", "ticks": 12, "command_tick": 1, "arrived_tick": None, "blocks_changed": 0, "min_health": 20.0, "died": False,
                "busy_at_end": False, "forced_at_end": False, "setup_error": None, "watch_cells": 4}
        with tempfile.TemporaryDirectory() as d:
            walking = [self.row(t, 1 + 0.28 * t) for t in range(0, 5)]
            still = [self.row(t, 2.4, forced=0, pathing=0) for t in range(5, 12)]
            self.write(d, "x", walking + still)
            self.assertEqual(run.judge(d, sc, meta)["status"], "pass")
            # keys still forced at the stop tick: a fail
            late = [self.row(t, 1 + 0.28 * t) for t in range(0, 7)] + [self.row(t, 3, forced=0, pathing=0) for t in range(7, 12)]
            self.write(d, "x", late)
            res = run.judge(d, sc, meta)
            self.assertEqual(res["status"], "fail")
            self.assertFalse(res["checks"]["keys released at the stop tick"])

    def test_a_death_or_a_changed_block_fails(self):
        sc = {"name": "y", "chat": False, "check": {"kind": "arrive", "goal": [3, 201, 0], "limit_ticks": 50, "blocks_changed": 0}}
        base = {"name": "y", "ticks": 8, "command_tick": 1, "arrived_tick": 7, "blocks_changed": 0, "min_health": 20.0, "died": False,
                "busy_at_end": False, "forced_at_end": False, "setup_error": None, "watch_cells": 4}
        with tempfile.TemporaryDirectory() as d:
            self.write(d, "y", [self.row(t, 0.5 + 0.5 * t) for t in range(8)])
            self.assertEqual(run.judge(d, sc, base)["status"], "pass")
            self.assertEqual(run.judge(d, sc, dict(base, blocks_changed=1))["status"], "fail")
            self.assertEqual(run.judge(d, sc, dict(base, died=True))["status"], "fail")
            self.assertEqual(run.judge(d, sc, dict(base, arrived_tick=None))["status"], "fail")


class BuildScenarioTests(unittest.TestCase):
    def test_the_ten_build_scenarios_are_there_and_use_files_that_exist(self):
        scs = scenarios.build_scenarios()
        names = {s["name"] for s in scs}
        for base in ("r01_build_schem_box", "r02_build_clear", "r05_missing_then_resume", "r06_stop_build"):
            self.assertIn(base + "_chat0", names)
            self.assertIn(base + "_chat1", names)
        for base in ("r03_build_litematic_rotated", "r04_build_layers", "r07_mcedit_legacy", "r08_substitutes", "r09_repeat", "r10_fallback_extension"):
            self.assertIn(base + "_chat0", names)
        files = scenarios.build_files()
        for s in scs:
            self.assertEqual(s["check"]["kind"], "build")
            self.assertTrue(s["files"] and all(f in files for f in s["files"]), s["name"])
            self.assertTrue(any(e.get("command", "").startswith("b build") or e.get("set", "").startswith("b build") for e in s["events"]), s["name"])

    def test_the_files_are_deterministic_and_gzip_nbt(self):
        import gzip
        a, b = scenarios.build_files(), scenarios.build_files()
        self.assertEqual(a, b)
        for name, data in a.items():
            raw = gzip.decompress(data)
            self.assertEqual(raw[0], 10, name)  # a compound
            self.assertEqual(raw[1:3], b"\0\0", name)  # with an empty name
            self.assertEqual(raw[-1], 0, name)  # closed by an End tag
        self.assertEqual({n.rsplit(".", 1)[1] for n in a}, {"schem", "litematic", "schematic"})

    def test_expected_cells_lie_in_the_box_and_the_areas_do_not_overlap(self):
        spans = []
        for s in scenarios.build_scenarios():
            bx, by, bz, w, h, l = s["check"]["box"]
            for c in s["check"]["expect"]:
                self.assertTrue(bx <= c[0] < bx + w and by <= c[1] < by + h and bz <= c[2] < bz + l, (s["name"], c))
            fx = re.match(r"forceload add (-?\d+) (-?\d+) (-?\d+) (-?\d+)", s["prep"][2]).groups()
            spans.append((s["name"], int(fx[0]), int(fx[1]), int(fx[2]), int(fx[3])))
        for i, a in enumerate(spans):
            for b in spans[i + 1:]:
                self.assertTrue(a[3] < b[1] or b[3] < a[1] or a[4] < b[2] or b[4] < a[2], (a, b))

    def test_the_settings_of_a_build_scenario_are_reset_after_it(self):
        by = {s["name"]: s for s in scenarios.build_scenarios()}
        r09 = by["r09_repeat_chat0"]
        self.assertIn({"t": 3, "set": "b set buildRepeat 6,0,0"}, r09["events"])
        self.assertIn("b set reset buildRepeatCount", r09["after"])
        self.assertIn({"t": 3, "set": "b set buildSubstitutes stone->cobblestone"}, by["r08_substitutes_chat0"]["events"])

    def test_the_rotated_scenario_expects_the_turned_cells(self):
        s = [s for s in scenarios.build_scenarios() if s["name"].startswith("r03")][0]
        ox, oy, oz = s["check"]["box"][:3]
        exp = {tuple(c[:3]): c[3] for c in s["check"]["expect"]}
        self.assertEqual(exp[(ox, oy, oz + 1)], "minecraft:oak_stairs")
        self.assertEqual(exp[(ox + 2, oy, oz + 2)], "minecraft:oak_planks")
        self.assertNotIn((ox, oy, oz), exp)
        self.assertEqual(s["check"]["sample_facing"]["has"], "facing=south")


class BuildJudgeTests(unittest.TestCase):
    BOX = [100, 201, 100, 2, 1, 2]

    def judged(self, changes, chk_extra=None, notes=(), timeline="", rows=()):
        chk = {"kind": "build", "box": self.BOX, "expect": [[100 + x, 201, 100 + z, "minecraft:oak_planks"] for x in range(2) for z in range(2)], "initial": [],
               "outside_max": 0, "done_within": 100, "messages": ["Done building"], **(chk_extra or {})}
        res = {"checks": {}, "notes": [], "chat_lines": [n[2] for n in notes if n[1] == "chat"]}
        meta = {"all_changes": changes}
        with tempfile.TemporaryDirectory() as d:
            if timeline:
                with open(os.path.join(d, "n.timeline.txt"), "w") as f:
                    f.write(timeline)
            run.judge_build(res, chk, meta, list(rows), list(notes), d, "n", 10)
        return res

    FULL = [f"{100 + x},201,{100 + z}: minecraft:air -> minecraft:oak_planks" for x in range(2) for z in range(2)]

    def test_a_finished_build_passes(self):
        r = self.judged(self.FULL, notes=[(60, "chat", "Done building")])
        self.assertTrue(all(r["checks"].values()), r["checks"])

    def test_a_wrong_cell_a_late_finish_and_a_stray_block_fail(self):
        r = self.judged(self.FULL[:3], notes=[(60, "chat", "Done building")])
        self.assertFalse(r["checks"]["every cell of the box is as the schematic says"])
        r = self.judged(self.FULL, notes=[(200, "chat", "Done building")])
        self.assertFalse(r["checks"]["'Done building' within 100 ticks of the command"])
        r = self.judged(self.FULL + ["5,201,5: minecraft:air -> minecraft:oak_planks"], notes=[(60, "chat", "Done building")])
        self.assertFalse(r["checks"]["at most 0 cells outside the box changed"])
        r = self.judged(self.FULL, notes=[])
        self.assertFalse(r["checks"]["said 'Done building'"])

    def test_air_and_cave_air_are_the_same_and_stone_that_stays_is_a_failure(self):
        chk = {"initial": [[100, 201, 100, "minecraft:stone"]], "expect": [[100, 201, 101, "minecraft:oak_planks"]]}
        ch = ["100,201,100: minecraft:stone -> minecraft:cave_air", "100,201,101: minecraft:air -> minecraft:oak_planks"]
        r = self.judged(ch, chk, notes=[(60, "chat", "Done building")])
        self.assertTrue(all(r["checks"].values()), r["checks"])
        r = self.judged(ch[1:], chk, notes=[(60, "chat", "Done building")])
        self.assertFalse(r["checks"]["every cell of the box is as the schematic says"])

    def test_layers_must_be_built_bottom_up(self):
        box = [100, 201, 100, 1, 2, 1]
        chk = {"box": box, "expect": [[100, 201, 100, "minecraft:oak_planks"], [100, 202, 100, "minecraft:oak_planks"]], "layers": {"height": 1}, "messages": [], "done_within": 100}
        ch = ["100,201,100: minecraft:air -> minecraft:oak_planks", "100,202,100: minecraft:air -> minecraft:oak_planks"]
        good = "20 100,201,100 minecraft:air minecraft:oak_planks\n30 100,202,100 minecraft:air minecraft:oak_planks\n"
        bad = "20 100,202,100 minecraft:air minecraft:oak_planks\n30 100,201,100 minecraft:air minecraft:oak_planks\n"
        notes = [(60, "chat", "Done building")]
        self.assertTrue(all(self.judged(ch, chk, notes, good)["checks"].values()))
        self.assertFalse(self.judged(ch, chk, notes, bad)["checks"]["no cell of a layer was placed before the layer below was complete"])

    def test_a_stop_needs_quiet_after_it(self):
        chk = {"partial": True, "stop_tick": 30, "quiet_after": 10, "messages": [], "expect": [[100, 201, 100, "minecraft:oak_planks"], [101, 201, 100, "minecraft:oak_planks"]]}
        ch = ["100,201,100: minecraft:air -> minecraft:oak_planks"]
        rows = [{"t": str(t), "forced": "0", "pathing": "0", "x": "1", "z": "1"} for t in range(0, 80)]
        quiet = "20 100,201,100 minecraft:air minecraft:oak_planks\n"
        self.assertTrue(all(self.judged(ch, chk, [], quiet, rows)["checks"].values()))
        late = quiet + "55 101,201,100 minecraft:air minecraft:oak_planks\n"
        self.assertFalse(self.judged(ch, chk, [], late, rows)["checks"]["no cell changed later than 10 ticks after the stop"])

    def test_a_paused_build_keeps_the_player_still(self):
        chk = {"paused_still": [20, 40], "messages": [], "partial": True}
        rows = [{"t": str(t), "x": "5.0", "z": "5.0", "forced": "0", "pathing": "0"} for t in range(0, 60)]
        self.assertTrue(self.judged([], chk, [], "", rows)["checks"]["the player stood still between ticks 20 and 40"])
        rows[30]["x"] = "5.5"
        self.assertFalse(self.judged([], chk, [], "", rows)["checks"]["the player stood still between ticks 20 and 40"])

    def test_the_facing_and_the_missing_file_message_are_read_from_the_notes(self):
        chk = {"messages": [], "partial": True, "sample_facing": {"tick": 90, "cells": [], "has": "facing=south"}}
        ok_note = [(90, "sample", "1,2,3=minecraft:oak_stairs[facing=south,half=bottom] 1,2,4=minecraft:oak_stairs[facing=south,half=bottom]")]
        self.assertTrue(all(self.judged([], chk, ok_note)["checks"].values()))
        bad_note = [(90, "sample", "1,2,3=minecraft:oak_stairs[facing=east,half=bottom]")]
        self.assertFalse(all(self.judged([], chk, bad_note)["checks"].values()))
        want = f"Cannot find {os.path.join(run.SCHEMATICS, 'nosuch.schematic')}"
        r = self.judged([], {"messages": [], "partial": True, "cannot_find": "nosuch.schematic"}, [(5, "chat", want)])
        self.assertTrue(all(r["checks"].values()), r["checks"])

    def test_the_files_are_written_once_and_removed_and_a_different_file_is_refused(self):
        saved = run.SCHEMATICS
        with tempfile.TemporaryDirectory() as d:
            run.SCHEMATICS = os.path.join(d, "schematics")
            try:
                made, made_dir = run.write_schematics({"a.schem": b"1", "b.schem": b"2"}, {"a.schem"})
                self.assertTrue(made_dir)
                self.assertEqual(sorted(os.listdir(run.SCHEMATICS)), ["a.schem"])
                made2, _ = run.write_schematics({"a.schem": b"1"}, {"a.schem"})
                self.assertEqual(made2, [])
                with self.assertRaises(SystemExit):
                    run.write_schematics({"a.schem": b"other"}, {"a.schem"})
                run.remove_schematics(made, made_dir)
                self.assertFalse(os.path.exists(run.SCHEMATICS))
            finally:
                run.SCHEMATICS = saved


class OwnershipTests(unittest.TestCase):
    def test_only_the_game_started_by_this_run_is_listed(self):
        import subprocess
        spec = "/tmp/some-spec.json"
        ours = subprocess.Popen(["sleep", "30"], env=dict(os.environ, BARITONE_RS_L3_SPEC=spec))
        theirs = subprocess.Popen(["sleep", "30"], env={k: v for k, v in os.environ.items() if k != "BARITONE_RS_L3_SPEC"})
        other_spec = subprocess.Popen(["sleep", "30"], env=dict(os.environ, BARITONE_RS_L3_SPEC="/tmp/other.json"))
        procs = (ours, theirs, other_spec)
        saved = run.running
        try:
            run.running = lambda name: [str(p.pid) for p in procs]
            self.assertEqual(run.started_by(spec), [str(ours.pid)])
        finally:
            run.running = saved
            for p in procs:
                p.kill()
                p.wait()


class WorldNameTests(unittest.TestCase):
    def test_only_a_new_l3_world_is_accepted(self):
        with tempfile.TemporaryDirectory() as d:
            saved = run.SAVES
            run.SAVES = d
            try:
                os.makedirs(os.path.join(d, "l3-old"))
                os.makedirs(os.path.join(d, "New World"))
                self.assertIsNone(run.check_world_name("l3-fresh"))
                self.assertIn("already exists", run.check_world_name("l3-old"))
                self.assertIn("must start with l3-", run.check_world_name("New World"))
                self.assertIn("must start with l3-", run.check_world_name("l3-../x"))
                self.assertIn("must start with l3-", run.check_world_name(""))
            finally:
                run.SAVES = saved


if __name__ == "__main__":
    unittest.main()
