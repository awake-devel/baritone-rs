#!/usr/bin/env python3
# Tests for ledger.py. Written for baritone-rs. The Java in the tests is invented, not taken from Baritone.
#
# This file is part of baritone-rs.
#
# baritone-rs is free software: you can redistribute it and/or modify
# it under the terms of the GNU Lesser General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# baritone-rs is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU Lesser General Public License for more details.
#
# You should have received a copy of the GNU Lesser General Public License
# along with baritone-rs.  If not, see <https://www.gnu.org/licenses/>.
import contextlib
import io
import os
import subprocess
import tempfile
import unittest

import ledger


def keys(src):
    return [(m.cls, m.key) for m in ledger.parse_java(src)]


class LexerTest(unittest.TestCase):
    def test_comments_and_strings_are_masked(self):
        src = 'a { "x}" /* } */ // }\n \'}\' }'
        code, mask = ledger.lex(src)
        self.assertEqual(len(code), len(src))
        self.assertEqual(mask.count("}"), 1)
        self.assertEqual(mask.count("{"), 1)
        self.assertIn('"x}"', code)  # strings stay in the hashed text
        self.assertNotIn("/*", code)  # comments do not

    def test_escapes(self):
        src = '"a\\"}" + \'\\\\\' + "}" {'
        _, mask = ledger.lex(src)
        self.assertEqual(mask.count("}"), 0)
        self.assertEqual(mask.count("{"), 1)

    def test_text_block(self):
        src = 'String s = """\n  } { "quoted" \\"""\n  """; }'
        _, mask = ledger.lex(src)
        self.assertEqual(mask.count("{"), 0)
        self.assertEqual(mask.count("}"), 1)


class ParserTest(unittest.TestCase):
    def test_braces_in_strings_chars_and_comments(self):
        src = '''
        class A {
            String s = "}}}";
            void a() { String t = "{"; char c = '}'; /* } */ // {
            }
            int b() { return '{'; }
        }'''
        self.assertEqual(keys(src), [("A", "a"), ("A", "b")])

    def test_nested_classes(self):
        src = '''
        public class Outer {
            void one() {}
            static class In {
                void two() {}
                class Deep { Deep() {} void three() {} }
            }
            interface I { void four(); default void five() { } static int six() { return 1; } }
            void seven() {}
        }'''
        self.assertEqual(keys(src), [
            ("Outer", "one"), ("Outer.In", "two"), ("Outer.In.Deep", "<init>"), ("Outer.In.Deep", "three"),
            ("Outer.I", "four"), ("Outer.I", "five"), ("Outer.I", "six"), ("Outer", "seven")])

    def test_lambdas_and_anonymous_classes_belong_to_the_method(self):
        src = '''
        class A {
            Runnable r = () -> { int x = 1; };
            Object o = new Object() { public String toString() { return "x"; } };
            void m() {
                Runnable q = () -> { foo(); };
                Object p = new Object() { void hidden() { } };
                class Local { void alsoHidden() {} }
                java.util.function.Function<Integer, Integer> f = x -> { return x; };
            }
            void after() {}
        }'''
        self.assertEqual(keys(src), [("A", "m"), ("A", "after")])

    def test_interface_default_static_and_abstract(self):
        src = '''
        public interface Costs {
            double WALK = 20 / 4.317;
            double[] TABLE = make();
            static double[] make() { return new double[]{1, 2}; }
            double apply(int x);
            default double twice(int x) { return apply(x) * 2; }
            @Deprecated(since = "1")
            static double old(double t) { return t; }
        }'''
        self.assertEqual(keys(src), [("Costs", "make"), ("Costs", "apply"), ("Costs", "twice"), ("Costs", "old")])

    def test_enum_with_constant_bodies(self):
        src = '''
        public enum Mv {
            UP(0, 1) {
                @Override double cost(int x) { return x; }
            },
            DOWN(0, -1) {
                @Override double cost(int x) { return -x; }
            },
            FLAT(0, 0);
            final int dx, dy;
            Mv(int dx, int dy) { this.dx = dx; this.dy = dy; }
            abstract double cost(int x);
        }'''
        # FLAT has no body but Mv.cost is abstract: that is a compile error in Java, only the parser matters here
        self.assertEqual(keys(src), [("Mv.UP", "cost"), ("Mv.DOWN", "cost"), ("Mv", "<init>"), ("Mv", "cost")])

    def test_enum_without_constants_bodies_or_with_annotations(self):
        src = 'enum E { @Deprecated A, B; void f() {} }'
        self.assertEqual(keys(src), [("E", "f")])

    def test_overloads_get_signatures(self):
        src = '''
        class A {
            void f(int a) {}
            void f(int a, java.util.List<String> b) {}
            void f(final String... xs) {}
            void g(int[] a, Map<String, List<Integer>> m) {}
            A() {}
            A(int x) {}
        }'''
        self.assertEqual(keys(src), [
            ("A", "f(int)"), ("A", "f(int,List)"), ("A", "f(String...)"), ("A", "g"),
            ("A", "<init>()"), ("A", "<init>(int)")])

    def test_modifiers_generics_annotations_throws(self):
        src = '''
        class A<T extends Comparable<T>> {
            @SuppressWarnings({"a", "b{"}) public static <U> List<U> make(@Nullable U u) throws Exception, Error { return null; }
            private final int[] arr[] = {1, 2};
            static { int k = 1; }
            { int j = 2; }
            protected abstract T pick(T a, T b);
            public <V> A(V v) {}
            private int[] grid() { return new int[]{1}; }
        }'''
        self.assertEqual(keys(src), [("A", "make"), ("A", "pick"), ("A", "<init>"), ("A", "grid")])

    def test_method_named_like_the_class_with_a_return_type_is_a_method(self):
        src = "class A { void A() {} }"
        self.assertEqual(keys(src), [("A", "A")])

    def test_hash_ignores_whitespace_and_comments_but_not_code(self):
        a = ledger.parse_java("class A { int f() { return 1; } }")[0].digest()
        b = ledger.parse_java("class A {\n  /** doc */\n  int f() {\n    // note\n    return  1;\n  }\n}")[0].digest()
        c = ledger.parse_java("class A { int f() { return 2; } }")[0].digest()
        d = ledger.parse_java('class A { String f() { return "a b"; } }')[0].digest()
        e = ledger.parse_java('class A { String f() { return "a  b"; } }')[0].digest()
        self.assertEqual(a, b)
        self.assertNotEqual(a, c)
        self.assertNotEqual(d, e)  # a change inside a string counts

    def test_unbalanced_source_raises(self):
        with self.assertRaises(ValueError):
            ledger.parse_java("class A { void f() { }")


def sh(*a, cwd=None):
    subprocess.run(a, cwd=cwd, check=True, capture_output=True, text=True)


class CommandTest(unittest.TestCase):
    """Runs the commands against a throwaway git repository with a layout like upstream's."""

    PATH = "src/api/java/baritone/api/pathing/movement/ActionCosts.java"

    JAVA1 = """package x;
public interface ActionCosts {
    double ONE = 1;
    static double velocity(int ticks) { return ticks * 2; }
    static double dist(double d) { return d; }
    static double dist(double d, int n) { return d * n; }
}
"""
    JAVA2 = JAVA1.replace("ticks * 2", "ticks * 3")

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = self.tmp.name
        self.up = os.path.join(t, "up")
        self.repo = os.path.join(t, "repo")
        self.ledger = os.path.join(self.repo, "docs", "port-ledger.tsv")
        os.makedirs(os.path.join(self.up, os.path.dirname(self.PATH)))
        os.makedirs(os.path.join(self.repo, "mod", "engine", "src"))
        sh("git", "init", "-q", cwd=self.up)
        sh("git", "config", "user.email", "test.invalid", cwd=self.up)
        sh("git", "config", "user.name", "t", cwd=self.up)
        self.rev1 = self.commit(self.JAVA1)
        ledger._cache.clear()
        self.saved_scope = ledger.SCOPE
        ledger.SCOPE = [self.PATH]

    def tearDown(self):
        ledger.SCOPE = self.saved_scope
        self.tmp.cleanup()

    def commit(self, text):
        with open(os.path.join(self.up, self.PATH), "w") as fh:
            fh.write(text)
        sh("git", "add", "-A", cwd=self.up)
        sh("git", "commit", "-q", "-m", "c", cwd=self.up)
        return subprocess.run(["git", "-C", self.up, "rev-parse", "--short=7", "HEAD"], capture_output=True,
                              text=True).stdout.strip()

    def run_cmd(self, *args):
        ledger._cache.clear()
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = ledger.main(["--upstream", self.up, "--ledger", self.ledger, "--repo", self.repo, *args])
        return code, buf.getvalue()

    def rust(self, text):
        with open(os.path.join(self.repo, "mod", "engine", "src", "lib.rs"), "w") as fh:
            fh.write(text)

    def set_status(self, cls, method, status, note=""):
        rows = ledger.read_ledger(self.ledger)
        for r in rows:
            if (r["class"], r["method"]) == (cls, method):
                r["status"] = status
                r["note"] = note
        ledger.write_ledger(self.ledger, rows)

    def test_build_adds_missing_rows_and_keeps_edits(self):
        code, out = self.run_cmd("build", "--rev", self.rev1)
        self.assertEqual(code, 0, out)
        rows = {(r["class"], r["method"]): r for r in ledger.read_ledger(self.ledger)}
        self.assertEqual(sorted(rows), [("ActionCosts", "dist(double)"), ("ActionCosts", "dist(double,int)"),
                                        ("ActionCosts", "velocity")])
        self.assertTrue(all(r["status"] == "missing" and r["commit"] == self.rev1 for r in rows.values()))
        self.set_status("ActionCosts", "velocity", "drop", "tab\tand\nnewline")
        rev2 = self.commit(self.JAVA2)
        code, out = self.run_cmd("build", "--rev", rev2)
        rows = {(r["class"], r["method"]): r for r in ledger.read_ledger(self.ledger)}
        self.assertEqual(rows[("ActionCosts", "velocity")]["status"], "drop")
        self.assertEqual(rows[("ActionCosts", "velocity")]["note"], "tab and newline")
        self.assertEqual(rows[("ActionCosts", "velocity")]["commit"], self.rev1)  # nothing is refreshed
        self.assertIn("0 rows added", out)

    def test_build_reports_vanished_methods_and_keeps_the_row(self):
        self.run_cmd("build", "--rev", self.rev1)
        rev2 = self.commit(self.JAVA1.replace("static double dist(double d, int n) { return d * n; }", ""))
        code, out = self.run_cmd("build", "--rev", rev2)
        self.assertIn("NOT FOUND", out)
        self.assertIn("dist(double,int)", out)
        # dist is no longer overloaded, so its key changes from dist(double) to dist: the old row is reported
        # too and stays, and a new row is added. Nothing is deleted silently.
        self.assertIn("NOT FOUND at %s (row kept): ActionCosts.dist(double)" % ledger.short_rev(self.up, "HEAD"), out)
        self.assertEqual(len(ledger.read_ledger(self.ledger)), 4)

    def test_check_passes_on_a_consistent_ledger(self):
        self.run_cmd("build", "--rev", self.rev1)
        self.set_status("ActionCosts", "velocity", "port")
        self.rust("/// `ActionCosts.velocity`: the fall speed.\nfn velocity(t: i32) -> f64 { t as f64 }\n")
        code, out = self.run_cmd("check")
        self.assertEqual(code, 0, out)
        self.assertIn("check passed", out)

    def test_check_fails_when_a_port_row_has_no_citation(self):
        self.run_cmd("build", "--rev", self.rev1)
        self.set_status("ActionCosts", "velocity", "port")
        self.rust("fn velocity() {}\n")
        code, out = self.run_cmd("check")
        self.assertEqual(code, 1)
        self.assertIn("status port, but no Rust function cites it", out)

    def test_check_fails_on_a_citation_with_no_row(self):
        self.run_cmd("build", "--rev", self.rev1)
        self.rust("/// `ActionCosts.nothing`: made up.\nfn f() {}\n")
        code, out = self.run_cmd("check")
        self.assertEqual(code, 1)
        self.assertIn("ActionCosts.nothing: no row", out)

    def test_check_fails_when_a_citation_is_on_a_missing_row_or_not_on_a_function(self):
        self.run_cmd("build", "--rev", self.rev1)
        self.rust("/// `ActionCosts.velocity`: cited but the row is still missing.\nfn f() {}\n")
        code, out = self.run_cmd("check")
        self.assertEqual(code, 1)
        self.assertIn("row has status missing", out)
        self.set_status("ActionCosts", "velocity", "port")
        self.rust("/// `ActionCosts.velocity`: on a constant.\npub const X: f64 = 1.0;\n")
        code, out = self.run_cmd("check")
        self.assertIn("not on a function", out)
        self.assertIn("no Rust function cites it", out)

    def test_overloaded_methods_need_a_signature_in_the_citation(self):
        self.run_cmd("build", "--rev", self.rev1)
        self.set_status("ActionCosts", "dist(double)", "port")
        self.rust("/// `ActionCosts.dist`: which one?\nfn f() {}\n")
        code, out = self.run_cmd("check")
        self.assertIn("ambiguous", out)
        self.rust("/// `ActionCosts.dist(double)`: this one.\n#[inline]\nfn f() {}\n")
        code, out = self.run_cmd("check")
        self.assertEqual(code, 0, out)

    def test_check_fails_on_a_hash_mismatch(self):
        self.run_cmd("build", "--rev", self.rev1)
        rows = ledger.read_ledger(self.ledger)
        for r in rows:
            if r["method"] == "velocity":
                r["hash"] = "0" * 16
        ledger.write_ledger(self.ledger, rows)
        code, out = self.run_cmd("check")
        self.assertEqual(code, 1)
        self.assertIn("hash mismatch", out)

    def test_check_fails_when_the_method_is_not_at_the_recorded_commit(self):
        self.run_cmd("build", "--rev", self.rev1)
        rows = ledger.read_ledger(self.ledger)
        rows.append(dict(rows[0], method="ghost"))
        ledger.write_ledger(self.ledger, rows)
        code, out = self.run_cmd("check")
        self.assertEqual(code, 1)
        self.assertIn("ActionCosts.ghost: method no longer exists", out)

    def test_check_rejects_bad_status_and_duplicates(self):
        self.run_cmd("build", "--rev", self.rev1)
        rows = ledger.read_ledger(self.ledger)
        rows[0]["status"] = "done"
        rows.append(dict(rows[1]))
        ledger.write_ledger(self.ledger, rows)
        code, out = self.run_cmd("check")
        self.assertIn("unknown status", out)
        self.assertIn("duplicate row", out)

    def test_upstream_diff_lists_changed_removed_and_new(self):
        self.run_cmd("build", "--rev", self.rev1)
        text = self.JAVA2.replace("static double dist(double d, int n) { return d * n; }",
                                  "static double extra() { return 0; }")
        rev2 = self.commit(text)
        code, out = self.run_cmd("upstream-diff", rev2)
        self.assertEqual(code, 0)
        self.assertIn("CHANGED  ActionCosts.velocity", out)
        self.assertIn("REMOVED  ActionCosts.dist(double,int)", out)
        self.assertIn("NEW      ActionCosts.extra", out)
        self.assertNotIn("CHANGED  ActionCosts.dist(double)", out)

    def test_header_is_checked(self):
        os.makedirs(os.path.dirname(self.ledger))
        with open(self.ledger, "w") as fh:
            fh.write("wrong\n")
        with self.assertRaises(ValueError):
            ledger.read_ledger(self.ledger)


if __name__ == "__main__":
    unittest.main()
