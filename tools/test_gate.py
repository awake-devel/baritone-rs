# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
import json
import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import check_public_tree as cpt  # noqa: E402
import gate  # noqa: E402
import run_compare as rc  # noqa: E402

# Private-looking strings are built from pieces so that this file passes its own check.
HOME = "/" + "home/someone/x"
SAMPLE = """traverse: 100 evals, 60 finite, 7 distinct, exact 98, within 1e-9 2, WRONG 0
descend: 90 evals, 50 finite, 9 distinct, exact 90, within 1e-9 0, WRONG 0
mismatches by config {}
palette 85 states, 4 configs, 1.2s
"""


class ParseCompare(unittest.TestCase):
    def test_parse(self):
        r = rc.parse_output(SAMPLE)
        self.assertEqual(r["traverse"], {"evals": 100, "finite": 60, "distinct": 7, "exact": 98, "close": 2, "wrong": 0})
        self.assertEqual(r["descend"]["exact"], 90)
        rc.check_results(r)

    def test_wrong_fails(self):
        r = rc.parse_output(SAMPLE.replace("WRONG 0", "WRONG 3", 1))
        with self.assertRaises(rc.Fail):
            rc.check_results(r)

    def test_missing_function(self):
        with self.assertRaises(rc.Fail):
            rc.parse_output(SAMPLE.split("\n")[0])

    def test_zero_evals(self):
        with self.assertRaises(rc.Fail):
            rc.parse_output(SAMPLE.replace("100 evals", "0 evals"))

    def test_garbage(self):
        with self.assertRaises(rc.Fail):
            rc.parse_output("thread 'main' panicked")

    def test_source_hashes_cover_inputs(self):
        h = rc.source_hashes()
        self.assertIn("src/lib.rs", h)
        self.assertTrue(any(k.startswith("java/") for k in h))

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
            open(report, "w").write("{}")
            with self.assertRaises(Exception):
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

    def test_known_missed_format(self):
        k = gate.parse_known_missed("# c\n\nx: y | no effect: because\n")
        self.assertEqual(k, {"x: y": "no effect: because"})
        with self.assertRaises(ValueError):
            gate.parse_known_missed("x: y\n")
        with self.assertRaises(ValueError):
            gate.parse_known_missed("x: y | \n")

    def test_shipped_known_missed_parses_and_is_listed_in_mutate(self):
        with open(gate.KNOWN_MISSED, encoding="utf-8") as f:
            known = gate.parse_known_missed(f.read())
        with open(os.path.join(gate.CRATE, "mutate.py"), encoding="utf-8") as f:
            src = f.read()
        for name in known:
            self.assertIn(f'("{name}"', src)
        self.assertNotIn("break: wrong tool table", known)
        self.assertNotIn("solid: trapdoor", known)


class CurrentReport(unittest.TestCase):
    def setUp(self):
        self.src = {"src/lib.rs": "a", "src/main.rs": "b"}
        self.rep = {"sources": dict(self.src, golden="g"), "upstream_commit": "c1", "grids": 5, "gate": {"mutation_inputs": "m1"}}

    def test_same(self):
        self.assertTrue(gate.comparison_current(self.rep, self.src, "c1", 5, "m1"))

    def test_changed_source(self):
        self.assertFalse(gate.comparison_current(self.rep, dict(self.src, **{"src/lib.rs": "z"}), "c1", 5, "m1"))

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


class FailedMutationIsNotForgotten(unittest.TestCase):
    """Regression: a failed mutation step used to leave the report behind, and the next run said UP TO DATE."""

    def run_gate(self, mutation_ok):
        class A:
            quick = False
            no_mutation = False
            force = False
            upstream = jdk = scratch = "x"
            grids = "5"
            branch = "b"

        report = {"upstream_commit": "c1", "grids": 5, "sources": {}, "results": {"traverse": {"evals": 1}, "descend": {"evals": 1}}}
        d = tempfile.mkdtemp()
        path = os.path.join(d, "r.json")
        saved = (rc.resolve, rc.report_path_for, rc.upstream_commit, rc.source_hashes, rc.execute)
        rc.resolve = lambda a: ("u", "j", d, 5, "b")
        rc.report_path_for = lambda b: path
        rc.upstream_commit = lambda u: "c1"
        rc.source_hashes = lambda root=None: {}
        rc.execute = lambda *a: json.loads(json.dumps(report))
        try:
            g = gate.Gate(A())
            g.mutation = lambda golden: (g.record("mutation", gate.PASS if mutation_ok else gate.FAIL) or mutation_ok)
            g.comparison()
        finally:
            rc.resolve, rc.report_path_for, rc.upstream_commit, rc.source_hashes, rc.execute = saved
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
            g.comparison()
        finally:
            os.environ.update(saved)
        self.assertEqual([s["status"] for s in g.steps], [gate.SKIPPED] * 3)
        self.assertFalse(g.failed)

    def test_quick_skips(self):
        class A:
            quick = True

        g = gate.Gate(A())
        g.comparison()
        self.assertEqual([s["status"] for s in g.steps], [gate.SKIPPED] * 3)


def make_repo(files):
    d = tempfile.mkdtemp()
    subprocess.run(["git", "init", "-q", d], check=True)
    for name, data in files.items():
        p = os.path.join(d, name)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "wb") as f:
            f.write(data)
    subprocess.run(["git", "-C", d, "add", "-A"], check=True)
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@users.noreply.github.com",
               GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@users.noreply.github.com")
    subprocess.run(["git", "-C", d, "commit", "-q", "-m", "init"], check=True, env=env)
    return d


class PublicTree(unittest.TestCase):
    def run_check(self, files, deny=None, history=False):
        d = make_repo(files)
        env_old = os.environ.pop(cpt.ENV_DENY, None)
        tmp = None
        try:
            if deny is not None:
                fd, tmp = tempfile.mkstemp()
                os.write(fd, deny.encode())
                os.close(fd)
                os.environ[cpt.ENV_DENY] = tmp
            findings, _ = cpt.check_tree(d, cpt.load_denylist(os.environ.get(cpt.ENV_DENY)))
            if history:
                findings += cpt.check_history(d, cpt.load_denylist(os.environ.get(cpt.ENV_DENY)))[0]
            return findings
        finally:
            os.environ.pop(cpt.ENV_DENY, None)
            if env_old:
                os.environ[cpt.ENV_DENY] = env_old
            if tmp:
                os.remove(tmp)

    def test_clean(self):
        self.assertEqual(self.run_check({"a.md": b"hello\n", "tools/T.java": b"class T {}"}), [])

    def test_untracked_files_are_checked(self):
        d = make_repo({"a.md": b"hello\n"})
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
        for text in (HOME, "/" + "Users/x", "/" + "tmp/claude-1", "/" + "run/user/1000"):
            f = self.run_check({"n.md": text.encode()})
            self.assertEqual(len(f), 1, text)

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
        d = make_repo({"n.md": HOME.encode()})
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@users.noreply.github.com",
                   GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@users.noreply.github.com")
        subprocess.run(["git", "-C", d, "rm", "-q", "n.md"], check=True)
        subprocess.run(["git", "-C", d, "commit", "-q", "-m", "rm"], check=True, env=env)
        self.assertEqual(cpt.check_tree(d, None)[0], [])
        self.assertEqual(len(cpt.check_history(d, None)[0]), 1)

    def test_history_commit_message(self):
        d = make_repo({"a.md": b"x"})
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@users.noreply.github.com",
                   GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@users.noreply.github.com")
        subprocess.run(["git", "-C", d, "commit", "-q", "--allow-empty", "-m", "worked in " + HOME], check=True, env=env)
        self.assertEqual(len(cpt.check_history(d, None)[0]), 1)


if __name__ == "__main__":
    unittest.main()
