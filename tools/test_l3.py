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
            m = re.match(r"forceload add (-?\d+) (-?\d+) (-?\d+) (-?\d+)", s["prep"][1])
            self.assertTrue(m, s["name"])
            fx0, fz0, fx1, fz1 = map(int, m.groups())
            self.assertLessEqual(((fx1 >> 4) - (fx0 >> 4) + 1) * ((fz1 >> 4) - (fz0 >> 4) + 1), 256)
            for line in s["prep"][2:]:
                nums = list(map(int, re.findall(r"(?<![\w.])-?\d+(?![\w.])", line.split("minecraft:")[0])[:6]))
                if line.startswith("fill"):
                    self.assertTrue(fx0 <= nums[0] and nums[3] <= fx1 and fz0 <= nums[2] and nums[5] <= fz1, f"{s['name']}: {line}")

    def test_the_far_scenario_can_be_left_out(self):
        self.assertFalse(any(s["name"].startswith("s10") for s in scenarios.all_scenarios(include_far=False)))


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


if __name__ == "__main__":
    unittest.main()
