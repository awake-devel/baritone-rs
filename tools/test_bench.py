#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Tests of the pure parts of run_bench.py: reading result lines, pairing the two sides, the summaries and the choice of cases."""
import math
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_bench as rb  # noqa: E402


def t_line(cid, ns, nodes=1000, typ="SUCCESS_TO_GOAL", phash="ab", best="1,2,3", besth="5.5", primary=100, failure=100, mode="finish"):
    return (f"T id={cid} mode={mode} primary={primary} failure={failure} reps={len(ns)} ns={','.join(map(str, ns))} gcms={','.join('0' for _ in ns)} "
            f"nodes={nodes} moves={nodes * 20} map={nodes * 2} type={typ} plen=10 phash={phash} best={best} besth={besth}")


CASES = {
    "1.near": {"seed": 1, "name": "near", "goal_kind": "block", "goal": "block(100,50,0)", "gx": 100, "gz": 0, "start": (0, 50, 0), "stress": False, "dist": 100.0},
    "1.far": {"seed": 1, "name": "far", "goal_kind": "block", "goal": "block(400,50,0)", "gx": 400, "gz": 0, "start": (0, 50, 0), "stress": False, "dist": 400.0},
    "2.near": {"seed": 2, "name": "near", "goal_kind": "xz", "goal": "xz(0,100)", "gx": 0, "gz": 100, "start": (0, 50, 0), "stress": False, "dist": 100.0},
    "2.deep": {"seed": 2, "name": "deep", "goal_kind": "block", "goal": "block(0,10,300)", "gx": 0, "gz": 300, "start": (0, 50, 0), "stress": True, "dist": 300.0},
}


class TestParse(unittest.TestCase):
    def test_parse_t(self):
        t = rb.parse_t(t_line("1.near", [3, 1, 2], nodes=77))
        self.assertEqual(t["ns"], [3, 1, 2])
        self.assertEqual(t["nodes"], 77)
        self.assertEqual(t["best"], (1, 2, 3))
        self.assertEqual(t["besth"], 5.5)

    def test_read_results_refuses_err(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "r.txt")
            with open(p, "w") as f:
                f.write("W seed=1 size=256 hash=ff\n" + t_line("1.near", [1]) + "\nERR id=1.far the repeated runs of one search disagree\n")
            with self.assertRaises(rb.Fail):
                rb.read_results(p)

    def test_read_cases(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "world-1.txt"), "w") as f:
                f.write("H x\nB id=1.near name=near goal=block(100,50,0) start=0,50,0 stress=0\nB id=1.xz name=xz goal=xz(3,4) start=0,50,0 stress=0\nV 0 cells=1\n")
            c = rb.read_cases(d)
            self.assertEqual(c["1.near"]["gx"], 100)
            self.assertEqual((c["1.xz"]["gx"], c["1.xz"]["gz"]), (3, 4))
            self.assertAlmostEqual(c["1.xz"]["dist"], 5.0)


class TestStats(unittest.TestCase):
    def test_quantile_and_summary(self):
        self.assertEqual(rb.quantile([1, 2, 3, 4, 5], 0.5), 3)
        self.assertEqual(rb.quantile([1, 2, 3, 4], 0.25), 1.75)
        s = rb.summarize([0.5, 1.0, 2.0, 4.0])
        self.assertEqual((s["n"], s["min"], s["max"], s["rust_slower"]), (4, 0.5, 4.0, 1))
        self.assertEqual(s["median"], 1.5)
        self.assertIsNone(rb.summarize([]))

    def test_spread(self):
        self.assertAlmostEqual(rb.spread([90, 100, 110]), 0.2)

    def test_compare_finish(self):
        j = {"1.near": rb.parse_t(t_line("1.near", [200_000_000] * 3)), "1.far": rb.parse_t(t_line("1.far", [10, 10], typ="SUCCESS_SEGMENT"))}
        r = {"1.near": rb.parse_t(t_line("1.near", [100_000_000] * 3)), "1.far": rb.parse_t(t_line("1.far", [10, 10], typ="SUCCESS_SEGMENT"))}
        rows = rb.compare_finish(j, r, CASES)
        self.assertEqual(len(rows), 2)
        by = {r["id"]: r for r in rows}
        self.assertAlmostEqual(by["1.near"]["ratio"], 2.0)
        self.assertTrue(by["1.near"]["finished"] and by["1.near"]["same_path"])
        self.assertFalse(by["1.far"]["finished"])

    def test_merge_rounds_keeps_the_rounds_apart(self):
        a = {"1.near": rb.parse_t(t_line("1.near", [100, 100]))}
        b = {"1.near": rb.parse_t(t_line("1.near", [300, 300]))}
        m = rb.merge_rounds([a, b])["1.near"]
        self.assertEqual(m["ns"], [100, 100, 300, 300])
        self.assertEqual(m["round_ns"], [[100, 100], [300, 300]])
        r = {"1.near": rb.parse_t(t_line("1.near", [50, 50]))}
        row = rb.compare_finish(rb.merge_rounds([a, b]), rb.merge_rounds([r, r]), CASES)[0]
        self.assertEqual(row["round_ratios"], [2.0, 6.0])

    def test_merge_rounds_refuses_another_answer(self):
        a = {"1.near": rb.parse_t(t_line("1.near", [1], nodes=5))}
        b = {"1.near": rb.parse_t(t_line("1.near", [1], nodes=6))}
        with self.assertRaises(rb.Fail):
            rb.merge_rounds([a, b])

    def test_different_path_is_noticed(self):
        j = {"1.near": rb.parse_t(t_line("1.near", [5], phash="aa"))}
        r = {"1.near": rb.parse_t(t_line("1.near", [5], phash="bb"))}
        row = rb.compare_finish(j, r, CASES)[0]
        self.assertTrue(row["same_search"])
        self.assertFalse(row["same_path"])

    def test_seed_ratios_add_up_within_a_seed(self):
        rows = [
            {"seed": 1, "finished": True, "java_ms": 30.0, "rust_ms": 10.0},
            {"seed": 1, "finished": True, "java_ms": 10.0, "rust_ms": 10.0},
            {"seed": 2, "finished": False, "java_ms": 99.0, "rust_ms": 1.0},
        ]
        self.assertEqual(rb.seed_ratios(rows), {1: 2.0})

    def test_pick_limit_cases(self):
        rows = [
            {"id": "1.near", "finished": True, "java_ms": 10.0, "rust_ms": 5.0},
            {"id": "1.far", "finished": True, "java_ms": 900.0, "rust_ms": 400.0},
            {"id": "2.near", "finished": False, "java_ms": 12000.0, "rust_ms": 12000.0},
        ]
        # 1.far: the faster side needs 400 ms, under 1.25 x 500, so it can finish inside 500 ms and is not picked
        self.assertEqual(rb.pick_limit_cases(rows, CASES, 500, 5, 12000), ["2.near"])
        self.assertEqual(rb.pick_limit_cases(rows, CASES, 50, 5, 12000), ["1.far", "2.near"])
        self.assertEqual(rb.pick_limit_cases(rows, CASES, 50, 5, 12000, taken=("1.far",)), ["2.near"])

    def test_compare_limit_progress(self):
        j = [rb.parse_t(t_line("1.far", [500_000_000], nodes=1000, typ="SUCCESS_SEGMENT", best="100,50,0", besth="900", primary=500, failure=500, mode="limit"))]
        r = [rb.parse_t(t_line("1.far", [500_000_000], nodes=2000, typ="SUCCESS_SEGMENT", best="250,50,0", besth="500", primary=500, failure=500, mode="limit"))]
        row = rb.compare_limit(j, r, CASES)[0]
        self.assertEqual(row["node_ratio"], 2.0)
        self.assertEqual(row["java"]["rem"], 300.0)
        self.assertEqual(row["rust"]["closer"], 250.0)
        self.assertEqual(row["closer_ratio"], 250.0 / 100.0)
        self.assertFalse(math.isnan(row["closer_ratio"]))


if __name__ == "__main__":
    unittest.main()
