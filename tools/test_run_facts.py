#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
import contextlib
import io
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_facts as rf  # noqa: E402


class ForOldGame(unittest.TestCase):
    def test_removes_the_new_blocks_and_renames(self):
        text = "a\n    //@26.3-begin\n    new();\n    //@26.3-end\nIdentifier.parse LilyPadBlock\n"
        out = rf.for_old_game(text)
        self.assertNotIn("new()", out)
        self.assertIn("ResourceLocation.parse WaterlilyBlock", out)

    def test_the_real_file_keeps_nothing_of_the_new_game(self):
        with open(rf.JAVA, encoding="utf-8") as f:
            out = rf.for_old_game(f.read())
        for word in ("Identifier", "LilyPadBlock", "DataComponentInitializers", "bindItemComponents"):
            self.assertNotIn(word, out)


class CompareVersions(unittest.TestCase):
    def test_finds_a_changed_fact_and_ignores_info_fields(self):
        with tempfile.TemporaryDirectory() as d:
            old, new = os.path.join(d, "o"), os.path.join(d, "n")
            with open(old, "w") as f:
                f.write("S 0 minecraft:a[] x=1 info_y=0\nS 1 minecraft:gone[] x=1 info_y=0\n")
            with open(new, "w") as f:
                f.write("S 0 minecraft:a[] x=2 info_y=9\nS 1 minecraft:new[] x=1 info_y=0\n")
            with contextlib.redirect_stdout(io.StringIO()):
                r = rf.compare_versions(old, new)
        self.assertEqual(r, {"common_states": 1, "fact_changes": 1, "blocks_gone": 1, "blocks_added": 1})


if __name__ == "__main__":
    unittest.main()
