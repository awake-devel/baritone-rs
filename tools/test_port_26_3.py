# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Tests for cost-compare/port_26_3.py: the port must work on the real source and fail loudly when the source moves."""
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "cost-compare"))
sys.path.insert(0, HERE)
import port_26_3 as pt  # noqa: E402
import run_compare as rc  # noqa: E402


class PortTest(unittest.TestCase):
    def test_real_source_ports(self):
        files = pt.ported_files()
        dump = files[pt.DUMP]
        self.assertNotIn("ResourceLocation", dump)
        self.assertNotIn("class Grid", dump)
        self.assertIn("Compat263.ctx(", dump)
        self.assertIn("baritone/pathing/movement/Compat263.java", files)

    def test_text_that_moved_fails(self):
        with open(os.path.join(pt.JAVA, *pt.DUMP.split("/")), encoding="utf-8") as f:
            text = f.read()
        with self.assertRaises(pt.PortError):
            pt.port_text(text.replace("WaterlilyBlock", "SomethingElse"))
        with self.assertRaises(pt.PortError):
            pt.port_text(text.replace("static final class Tools extends ToolSet {", "static final class Tools2 extends ToolSet {"))
        with self.assertRaises(pt.PortError):
            pt.port_text(text + "\n// Blocks.WHITE_CARPET\n")

    def test_copy_ported_and_remove(self):
        with tempfile.TemporaryDirectory() as up:
            files, dirs = rc.copy_java(up, "26.3")
            try:
                self.assertTrue(any(f.endswith("Compat263.java") for f in files))
            finally:
                rc.remove_copied(files, dirs)
            self.assertFalse(os.path.exists(os.path.join(up, "src", "test", "java", "baritone", "pathing")))

    def test_hashes_include_the_port(self):
        h = rc.source_hashes(port="26.3")
        self.assertIn("port/port_26_3.py", h)
        self.assertIn("port/port-26.3/baritone/pathing/movement/Compat263.java", h)
        self.assertFalse(any(k.startswith("port/") for k in rc.source_hashes()))


if __name__ == "__main__":
    unittest.main()
