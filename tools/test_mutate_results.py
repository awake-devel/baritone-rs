#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Saved injected-error verdicts may only be reused on byte-identical inputs."""
import importlib.util
import os
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("cost_compare_mutate", os.path.join(HERE, "cost-compare", "mutate.py"))
mutate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mutate)

MUTATION = ("src/a.rs", "1 + 1", "1 + 2", "adds one more")


class Results(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.repo = os.path.join(tmp.name, "repo")
        self.scratch = os.path.join(tmp.name, "scratch")
        os.makedirs(os.path.join(self.repo, "src"))
        os.makedirs(self.scratch)
        self.source = os.path.join(self.repo, "src", "a.rs")
        self.golden = os.path.join(self.scratch, "golden.txt")
        self.write(self.source, "fn a() {}")
        self.write(self.golden, "line")

    @staticmethod
    def write(path, text):
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)

    def cache(self):
        return mutate.ResultCache(self.repo, ["src"], self.golden, "tool")

    def test_same_inputs_reuse_a_verdict(self):
        self.cache().put(MUTATION, ["CAUGHT", 1, ""])
        self.assertEqual(self.cache().get(MUTATION), ["CAUGHT", 1, ""])

    def test_another_mutation_is_not_reused(self):
        self.cache().put(MUTATION, ["CAUGHT", 1, ""])
        self.assertIsNone(self.cache().get(("src/a.rs", "1 + 1", "1 + 3", "adds one more")))

    def test_an_engine_edit_discards_everything(self):
        self.cache().put(MUTATION, ["CAUGHT", 1, ""])
        self.write(self.source, "fn a() { }")
        self.assertIsNone(self.cache().get(MUTATION))

    def test_another_golden_file_discards_everything(self):
        self.cache().put(MUTATION, ["CAUGHT", 1, ""])
        self.write(self.golden, "other line")
        self.assertIsNone(self.cache().get(MUTATION))
        self.assertEqual(len(os.listdir(self.scratch)), 1, "the old results file is removed when the new one is opened")

    def test_a_torn_last_line_is_ignored(self):
        cache = self.cache()
        cache.put(MUTATION, ["MISSED", 0, ""])
        with open(cache.path, "a", encoding="utf-8") as f:
            f.write('["abc", ["CAU')
        self.assertEqual(self.cache().get(MUTATION), ["MISSED", 0, ""])

    def test_it_can_be_turned_off(self):
        self.cache().put(MUTATION, ["CAUGHT", 1, ""])
        with mock.patch.dict(os.environ, {"BARITONE_MUTATE_NO_CACHE": "1"}):
            cache = self.cache()
            self.assertIsNone(cache.get(MUTATION))
            cache.put(MUTATION, ["MISSED", 0, ""])
        self.assertEqual(self.cache().get(MUTATION), ["CAUGHT", 1, ""])

    def test_the_timeout_follows_the_baseline_within_limits(self):
        self.assertEqual(mutate.run_timeout(1), 60)
        self.assertEqual(mutate.run_timeout(40), 160)
        self.assertEqual(mutate.run_timeout(1000), mutate.RUN_TIMEOUT)


if __name__ == "__main__":
    unittest.main()
