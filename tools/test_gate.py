# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import check_public_tree as cpt  # noqa: E402
import gate  # noqa: E402
import run_compare as rc  # noqa: E402

# Private-looking strings are built from pieces so that this file passes its own check.
HOME = "/" + "home/someone/x"
# The test repositories must not depend on the developer's git configuration (signing, hooks, templates).
GIT_ENV = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull,
               GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@users.noreply.github.com",
               GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@users.noreply.github.com")


def quiet(fn, *args):
    """Runs a gate step without its progress lines in the test output."""
    with contextlib.redirect_stdout(io.StringIO()):
        return fn(*args)


def sample_lines(**override):
    """A comparison output with a line for every family; `override` replaces a family's line with the given text."""
    out = []
    for f in rc.FUNCS:
        line = override.get(f, f"{f}: 100 evals, 60 finite, 7 distinct, exact 98, within 1e-9 2, WRONG 0")
        out.append(line)
    return "\n".join(out) + "\nmismatches in movements by config {}\npalette 85 states, 4 configs, 1.2s\n"


SAMPLE = sample_lines(descend="descend: 90 evals, 50 finite, 9 distinct, exact 90, within 1e-9 0, WRONG 0")


class ParseCompare(unittest.TestCase):
    def test_parse(self):
        r = rc.parse_output(SAMPLE)
        self.assertEqual(r["traverse"], {"evals": 100, "finite": 60, "distinct": 7, "exact": 98, "close": 2, "wrong": 0})
        self.assertEqual(sorted(r), sorted(rc.FUNCS))
        self.assertEqual(r["descend"]["exact"], 90)
        rc.check_results(r)

    def test_wrong_fails(self):
        r = rc.parse_output(SAMPLE.replace("WRONG 0", "WRONG 3", 1))
        with self.assertRaises(rc.Fail):
            rc.check_results(r)

    def test_missing_function(self):
        with self.assertRaises(rc.Fail):
            rc.parse_output(SAMPLE.split("\n")[0])
        with self.assertRaises(rc.Fail):
            rc.parse_output(SAMPLE.replace("goal_near:", "goal_nearly:"))

    def test_movement_or_goal_without_a_finite_result_fails(self):
        for fam in ("parkour", "goal_runaway"):
            with self.assertRaises(rc.Fail, msg=fam):
                rc.parse_output(sample_lines(**{fam: f"{fam}: 100 evals, 0 finite, 7 distinct, exact 100, within 1e-9 0, WRONG 0"}))
        # families that are not movements or goals may have no finite result
        rc.parse_output(sample_lines(states="states: 100 evals, 0 finite, 7 distinct, exact 100, within 1e-9 0, WRONG 0"))

    def test_zero_evals(self):
        with self.assertRaises(rc.Fail):
            rc.parse_output(SAMPLE.replace("100 evals", "0 evals"))

    def test_garbage(self):
        with self.assertRaises(rc.Fail):
            rc.parse_output("thread 'main' panicked")

    def test_source_hashes_cover_inputs(self):
        h = rc.source_hashes()
        for k in ("tools/cost-compare/src/main.rs", "mod/engine/src/movement/helper.rs", "mod/engine/src/goals.rs", "Cargo.toml", "Cargo.lock", "rustfmt.toml", "tools/cost-compare/Cargo.toml", "mod/engine/Cargo.toml"):
            self.assertIn(k, h)
        self.assertNotIn("tools/cost-compare/src/lib.rs", h)
        self.assertTrue(any(k.startswith("java/") for k in h))
        self.assertFalse(any(k.startswith("java-26.3/") for k in h))

    def test_copy_and_remove(self):
        with tempfile.TemporaryDirectory() as up:
            os.makedirs(os.path.join(up, "src", "test", "java", "baritone"))
            files, dirs = rc.copy_java(up)
            self.assertTrue(files and all(os.path.exists(f) for f in files))
            rc.remove_copied(files, dirs)
            self.assertFalse(any(os.path.exists(f) for f in files))
            self.assertFalse(os.path.exists(os.path.join(up, "src", "test", "java", "baritone", "pathing")))
            self.assertTrue(os.path.isdir(os.path.join(up, "src", "test", "java", "baritone")))

    def test_copy_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as up:
            files, dirs = rc.copy_java(up)
            try:
                with self.assertRaises(rc.Fail):
                    rc.copy_java(up)
            finally:
                rc.remove_copied(files, dirs)

    def test_stale_report_deleted_on_failure(self):
        with tempfile.TemporaryDirectory() as d:
            report = os.path.join(d, "r.json")
            with open(report, "w") as f:
                f.write("{}")
            with self.assertRaises(rc.Fail):
                rc.execute(os.path.join(d, "no-such-upstream"), d, d, 1, "x", report)
            self.assertFalse(os.path.exists(report))


class Mutation(unittest.TestCase):
    OUT = (
        "CAUGHT  wrong=    12 close=     0 a: one\n"
        "MISSED  wrong=     0 close=     0 b: two\n"
        "MISSED  wrong=     0 close=     0 c: three\n"
        "\n1 caught, 2 missed, 1 not applied\n"
        "missed: ['b: two', 'c: three']\n"
        "not applied: ['d: four']\n"
    )

    def test_parse(self):
        self.assertEqual(gate.parse_mutation_output(self.OUT), (["a: one"], ["b: two", "c: three"], ["d: four"]))

    def test_unfinished(self):
        with self.assertRaises(ValueError):
            gate.parse_mutation_output("CAUGHT  wrong=1 close=0 a\n")

    def test_judge(self):
        ok, msgs = gate.judge_mutation(["a: one"], ["b: two"], [], {"b: two": "no effect"})
        self.assertTrue(ok)
        self.assertEqual(msgs, [])

    def test_unlisted_miss_fails(self):
        ok, msgs = gate.judge_mutation([], ["b: two"], [], {})
        self.assertFalse(ok)
        self.assertIn("b: two", msgs[0])

    def test_not_applied_fails(self):
        ok, _ = gate.judge_mutation([], [], ["d: four"], {})
        self.assertFalse(ok)

    def test_stale_entry_is_a_note_only(self):
        ok, msgs = gate.judge_mutation(["a: one"], [], [], {"a: one": "hole"})
        self.assertTrue(ok)
        self.assertIn("now caught", msgs[0])

    def test_listed_but_unknown_is_a_note_only(self):
        ok, msgs = gate.judge_mutation(["a: one"], [], [], {"z: gone": "hole"})
        self.assertTrue(ok)
        self.assertIn("no such mutation", msgs[0])

    def test_known_missed_format(self):
        k = gate.parse_known_missed("# c\n\nx: y | no effect: because\n")
        self.assertEqual(k, {"x: y": "no effect: because"})
        with self.assertRaises(ValueError):
            gate.parse_known_missed("x: y\n")
        with self.assertRaises(ValueError):
            gate.parse_known_missed("x: y | \n")

    def test_shipped_known_missed_parses_and_is_listed_in_mutate(self):
        for path in (gate.KNOWN_MISSED, gate.SEARCH_KNOWN_MISSED):
            with open(path, encoding="utf-8") as f:
                listed = gate.parse_known_missed(f.read())
            with open(os.path.join(os.path.dirname(path), "mutate.py"), encoding="utf-8") as f:
                src = f.read()
            for name in listed:
                self.assertIn(f'"{name}"),', src, path)
        with open(gate.KNOWN_MISSED, encoding="utf-8") as f:
            known = gate.parse_known_missed(f.read())
        self.assertNotIn("break: wrong tool table", known)
        self.assertNotIn("solid: trapdoor", known)


class CurrentReport(unittest.TestCase):
    def setUp(self):
        self.src = {"mod/engine/src/lib.rs": "a", "tools/cost-compare/src/main.rs": "b"}
        self.rep = {"sources": dict(self.src, golden="g"), "upstream_commit": "c1", "grids": 5, "gate": {"mutation_inputs": "m1"}}

    def test_same(self):
        self.assertTrue(gate.comparison_current(self.rep, self.src, "c1", 5, "m1"))

    def test_changed_source(self):
        self.assertFalse(gate.comparison_current(self.rep, dict(self.src, **{"mod/engine/src/lib.rs": "z"}), "c1", 5, "m1"))

    def test_changed_commit_or_grids(self):
        self.assertFalse(gate.comparison_current(self.rep, self.src, "c2", 5, "m1"))
        self.assertFalse(gate.comparison_current(self.rep, self.src, "c1", 6, "m1"))

    def test_changed_mutation_inputs(self):
        self.assertFalse(gate.comparison_current(self.rep, self.src, "c1", 5, "m2"))

    def test_report_without_gate_record_is_not_current(self):
        rep = {k: v for k, v in self.rep.items() if k != "gate"}
        self.assertFalse(gate.comparison_current(rep, self.src, "c1", 5, "m1"))

    def test_no_report(self):
        self.assertFalse(gate.comparison_current(None, self.src, "c1", 5, "m1"))


class CurrentSearchReport(unittest.TestCase):
    def setUp(self):
        self.src = {"tools/search-compare/src/main.rs": "a"}
        self.rep = {"sources": dict(self.src, golden="g"), "upstream_commit": "c1", "worlds": 4, "cases_per_world": 9, "gate": {"mutation_inputs": "m1"}}

    def test_same(self):
        self.assertTrue(gate.search_current(self.rep, self.src, "c1", 4, 9, "m1"))

    def test_any_changed_input(self):
        for args in (({}, "c1", 4, 9, "m1"), (self.src, "c2", 4, 9, "m1"), (self.src, "c1", 5, 9, "m1"), (self.src, "c1", 4, 8, "m1"), (self.src, "c1", 4, 9, "m2")):
            self.assertFalse(gate.search_current(self.rep, *args), args)

    def test_no_report_or_no_gate_record(self):
        self.assertFalse(gate.search_current(None, self.src, "c1", 4, 9, "m1"))
        self.assertFalse(gate.search_current({k: v for k, v in self.rep.items() if k != "gate"}, self.src, "c1", 4, 9, "m1"))


class FailedMutationIsNotForgotten(unittest.TestCase):
    """Regression: a failed mutation step used to leave the report behind, and the next run said UP TO DATE."""

    def run_gate(self, mutation_ok, results=None, stale=True):
        """Runs the comparison step on a fake comparison. `results` are the three runs' results, the same by default."""
        class A:
            quick = False
            no_mutation = False
            force = False
            upstream = jdk = scratch = "x"
            grids = "5"
            branch = "b"

        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d)
        path = os.path.join(d, "r.json")
        if stale:  # an earlier run's report, which no longer matches the inputs
            with open(path, "w") as f:
                f.write("{}")
        runs = iter(results or [{"traverse": {"evals": 1}}] * 3)

        def execute(upstream, jdk, scratch, grids, branch, report_path, port=None):
            if os.path.exists(report_path):  # as the real one does
                os.remove(report_path)
            return {"upstream_commit": "c1", "grids": 5, "sources": {}, "results": next(runs)}

        with mock.patch.multiple(rc, resolve=lambda a: ("u", "j", d, 5, "b"), report_path_for=lambda b: path,
                                 upstream_commit=lambda u: "c1", source_hashes=lambda root=None: {}, execute=execute):
            g = gate.Gate(A())
            g.mutation = lambda golden: (g.record("mutation", gate.PASS if mutation_ok else gate.FAIL) or mutation_ok)
            quiet(g.comparison)
        return g, path

    def test_failed_mutation_leaves_no_report(self):
        g, path = self.run_gate(False)
        self.assertTrue(g.failed)
        self.assertFalse(os.path.exists(path))

    def test_passed_mutation_writes_report(self):
        g, path = self.run_gate(True)
        self.assertFalse(g.failed)
        self.assertTrue(os.path.exists(path))
        with open(path) as f:
            self.assertIn("mutation_inputs", json.load(f)["gate"])

    def test_differing_repeat_run_fails_and_leaves_no_report(self):
        """Regression: this path removed a report that was already gone, and the gate died without a result."""
        for stale in (True, False):
            g, path = self.run_gate(True, [{"traverse": {"evals": 1}}, {"traverse": {"evals": 1}}, {"traverse": {"evals": 2}}], stale)
            self.assertEqual([s["status"] for s in g.steps], [gate.PASS, gate.FAIL])
            self.assertIn("run 3", g.steps[1]["detail"])
            self.assertFalse(os.path.exists(path))


class ExitStatus(unittest.TestCase):
    """0 passed, 1 failed, 2 incomplete; the gate stops at the first failure and always writes its result."""

    def run_gate(self, statuses):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d)
        result = os.path.join(d, "gate-result.json")
        g = gate.Gate(None)
        names = ("public_tree", "cargo", "python_tests", "ledger", "facts", "rules_26_3", "comparison", "search")
        for name, status in zip(names, statuses, strict=True):
            setattr(g, name, lambda name=name, status=status: g.record(name, status))
        with mock.patch.object(gate, "RESULT", result):
            code = quiet(g.run)
        with open(result) as f:
            return code, json.load(f)

    def test_passed(self):
        code, res = self.run_gate([gate.PASS] * 7 + [gate.CURRENT])
        self.assertEqual((code, res["status"], res["skipped"]), (0, "passed", []))

    def test_skipped_is_never_a_pass(self):
        code, res = self.run_gate([gate.PASS, gate.SKIPPED] + [gate.PASS] * 6)
        self.assertEqual((code, res["status"], res["skipped"]), (2, "incomplete", ["cargo"]))
        self.assertEqual(len(res["steps"]), 8)

    def test_stops_at_the_first_failure(self):
        code, res = self.run_gate([gate.SKIPPED, gate.PASS, gate.FAIL] + [gate.PASS] * 5)
        self.assertEqual((code, res["status"]), (1, "failed"))
        self.assertEqual([s["step"] for s in res["steps"]], ["public_tree", "cargo", "python_tests"])


class GateSkips(unittest.TestCase):
    def test_comparison_without_inputs_is_skipped(self):
        class A:
            quick = False
            no_mutation = False
            force = False
            upstream = jdk = scratch = grids = branch = None

        saved = {k: os.environ.pop(k) for k in list(os.environ) if k.startswith("BARITONE_")}
        try:
            g = gate.Gate(A())
            quiet(g.comparison)
        finally:
            os.environ.update(saved)
        self.assertEqual([s["status"] for s in g.steps], [gate.SKIPPED] * 3)
        self.assertFalse(g.failed)

    def test_quick_skips(self):
        class A:
            quick = True

        g = gate.Gate(A())
        quiet(g.comparison)
        self.assertEqual([s["status"] for s in g.steps], [gate.SKIPPED] * 3)


def git(d, *args):
    subprocess.run(["git", "-C", d, *args], check=True, env=GIT_ENV)


class PublicTree(unittest.TestCase):
    def make_repo(self, files):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d)
        git(d, "init", "-q")
        for name, data in files.items():
            p = os.path.join(d, name)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as f:
                f.write(data)
        git(d, "add", "-A")
        git(d, "commit", "-q", "-m", "init")
        return d

    def run_check(self, files, deny=None):
        """The findings on a repository of `files`; `deny` is the text of a deny list file."""
        d = self.make_repo(files)
        if deny is not None:
            path = os.path.join(d, ".git", "denylist.txt")
            with open(path, "w", encoding="utf-8") as f:
                f.write(deny)
            deny = cpt.load_denylist(path)
        return cpt.check_tree(d, deny)[0]

    def test_clean(self):
        self.assertEqual(self.run_check({"a.md": b"hello\n", "tools/T.java": b"class T {}"}), [])

    def test_untracked_files_are_checked(self):
        d = self.make_repo({"a.md": b"hello\n"})
        for name, data in (("fake.jar", b"PK"), ("tools/new.py", ("x = '" + HOME + "'\n").encode())):
            p = os.path.join(d, name)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as f:
                f.write(data)
        with open(os.path.join(d, ".gitignore"), "w") as f:
            f.write("ignored.jar\n")
        with open(os.path.join(d, "ignored.jar"), "wb") as f:
            f.write(b"PK")
        findings, _ = cpt.check_tree(d, None)
        self.assertEqual(len(findings), 2, findings)

    def test_jar_and_class(self):
        f = self.run_check({"x.jar": b"PK", "y.class": b"\xca\xfe"})
        self.assertEqual(len(f), 2)

    def test_golden_data_but_not_golden_source(self):
        f = self.run_check({"golden.txt.out": b"1", "tools/SpikeGoldenDump.java": b"class X {}"})
        self.assertEqual(len(f), 1)
        self.assertIn("golden.txt.out", f[0])

    def test_big_file_but_not_big_license(self):
        big = b"x" * (cpt.MAX_BYTES + 1)
        f = self.run_check({"data.bin": big, "LICENSE-GPL": big})
        self.assertEqual(len(f), 1)
        self.assertIn("data.bin", f[0])

    def test_java_outside_tools(self):
        f = self.run_check({"src/A.java": b"class A {}"})
        self.assertIn("outside tools/", f[0])

    def test_private_paths(self):
        texts = (HOME, "/" + "Users/x", "/" + "tmp/claude-1", "/" + "run/user/1000")
        f = self.run_check({f"n{i}.md": t.encode() for i, t in enumerate(texts)})
        self.assertEqual([m.split(":")[0] for m in f], [f"n{i}.md" for i in range(len(texts))], f)

    def test_email(self):
        self.assertEqual(len(self.run_check({"n.md": ("mail me: someone" + "@" + "private.example.net").encode()})), 1)
        self.assertEqual(self.run_check({"n.md": b"1+x@users.noreply.github.com"}), [])

    def test_binary_content_not_scanned(self):
        self.assertEqual(self.run_check({"n.bin": b"\0" + HOME.encode()}), [])

    def test_deny_list(self):
        f = self.run_check({"n.md": b"my secretword here", "secretword.md": b"x"}, deny="# c\nSecretWord\n")
        self.assertEqual(len(f), 2)

    def test_deny_list_not_set(self):
        self.assertIsNone(cpt.load_denylist(None))

    def test_history_finds_removed_file(self):
        d = self.make_repo({"n.md": HOME.encode()})
        git(d, "rm", "-q", "n.md")
        git(d, "commit", "-q", "-m", "rm")
        self.assertEqual(cpt.check_tree(d, None)[0], [])
        self.assertEqual(len(cpt.check_history(d, None)[0]), 1)

    def test_history_commit_message(self):
        d = self.make_repo({"a.md": b"x"})
        git(d, "commit", "-q", "--allow-empty", "-m", "worked in " + HOME)
        self.assertEqual(len(cpt.check_history(d, None)[0]), 1)


if __name__ == "__main__":
    unittest.main()
