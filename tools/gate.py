#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""The gate: what a change must pass before it is committed. Stops at the first failure.

Steps, in order:
  1 public tree        tools/check_public_tree.py
  2 cargo              fmt --check, clippy -D warnings, tests, in tools/cost-compare
  3 python tests       unittest discover under tools/
  4 ledger             tools/ledger.py --upstream DIR check, if tools/ledger.py exists
  5 comparison         tools/run_compare.py, only when its inputs differ from the report. The report is written
                       only after steps 5 to 7 all pass, so a report on disk means all three passed.
  6 repeat             two more runs that must give identical results
  7 mutation           tools/cost-compare/mutate.py; fails on 'not applied' and on a missed mutation
                       that is not listed in tools/cost-compare/known-missed.txt

Steps that cannot run are printed as SKIPPED and listed at the end. Exit status: 0 passed, 1 failed,
2 incomplete (nothing failed, but something was skipped). A skipped step is never a pass.

Options: --quick (no comparison, repeat or mutation), --no-mutation, --force (run the comparison even if
the report matches). Comparison inputs: --upstream, --jdk, --scratch, --grids, --branch, or the BARITONE_*
environment variables (see run_compare.py). The result goes to gate-result.json, which git ignores.
"""
import argparse
import ast
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run_compare as rc  # noqa: E402

REPO = rc.REPO
CRATE = rc.CRATE
RESULT = os.path.join(REPO, "gate-result.json")
KNOWN_MISSED = os.path.join(CRATE, "known-missed.txt")
PASS, FAIL, SKIPPED, CURRENT = "PASSED", "FAILED", "SKIPPED", "UP TO DATE"


def parse_known_missed(text):
    """Returns {mutation name: reason}. Lines are 'name | reason'; # starts a comment."""
    out = {}
    for ln in text.splitlines():
        if not ln.strip() or ln.lstrip().startswith("#"):
            continue
        name, sep, reason = ln.partition(" | ")
        if not sep or not reason.strip():
            raise ValueError(f"known-missed.txt: a reason is required: {ln!r}")
        out[name.strip()] = reason.strip()
    return out


def parse_mutation_output(text):
    """Returns (caught names, missed names, not-applied entries) from mutate.py's stdout."""
    caught, missed, not_applied = [], [], None
    for ln in text.splitlines():
        m = re.match(r"^(CAUGHT|MISSED)\s+wrong=\s*\d+ close=\s*\d+ (.*)$", ln)
        if m:
            (caught if m.group(1) == "CAUGHT" else missed).append(m.group(2))
        elif ln.startswith("not applied:"):
            not_applied = ast.literal_eval(ln[len("not applied:"):].strip())
    if not_applied is None:
        raise ValueError("mutate.py printed no 'not applied' line; it did not finish")
    return caught, missed, not_applied


def judge_mutation(caught, missed, not_applied, known):
    """Returns (ok, messages)."""
    msgs = []
    for n in not_applied:
        msgs.append(f"not applied (pattern no longer matches or build broke): {n}")
    unlisted = [n for n in missed if n not in known]
    for n in unlisted:
        msgs.append(f"missed and not listed in known-missed.txt: {n}")
    for n in known:
        if n in caught:
            msgs.append(f"note: '{n}' is listed as missed but is now caught; remove it from the list")
        elif n not in missed and n not in not_applied:
            msgs.append(f"note: '{n}' is listed but mutate.py has no such mutation")
    return not (not_applied or unlisted), msgs


def mutation_inputs_hash():
    """sha256 over mutate.py and known-missed.txt: a change to either means the mutation step must run again."""
    h = hashlib.sha256()
    for path in (os.path.join(CRATE, "mutate.py"), KNOWN_MISSED):
        h.update(rc.sha256_file(path).encode())
    return h.hexdigest()


def comparison_current(report, current_sources, commit, grids, mutation_inputs):
    """True when the report was made from exactly the inputs we have now, and the gate recorded
    that repeat runs and mutation passed on them (the report is only written after they pass)."""
    if not report:
        return False
    old = {k: v for k, v in report.get("sources", {}).items() if k != "golden"}
    return (
        old == current_sources
        and report.get("upstream_commit") == commit
        and report.get("grids") == grids
        and report.get("gate", {}).get("mutation_inputs") == mutation_inputs
    )


def load_report(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def busy():
    """True when a game or another build is running, so we should build at low priority."""
    return rc.gradle_client_running() or any(
        subprocess.run(["pgrep", *a], capture_output=True).returncode == 0 for a in (["-x", "horizon-client"], ["-x", "rustc"])
    )


class Gate:
    def __init__(self, args):
        self.a = args
        self.steps = []  # (name, status, detail)
        self.failed = False

    def record(self, name, status, detail=""):
        self.steps.append({"step": name, "status": status, "detail": detail})
        print(f"[{status}] {name}" + (f": {detail}" if detail else ""), flush=True)
        if status == FAIL:
            self.failed = True

    def sh(self, cmd, cwd, env=None, low=False):
        if low:
            cmd = ["nice", "-n", "15"] + cmd
        r = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
        return r.returncode, (r.stdout + r.stderr)

    # -- steps ---------------------------------------------------------------------------------

    def public_tree(self):
        rcode, out = self.sh([sys.executable, os.path.join(HERE, "check_public_tree.py")], REPO)
        if rcode == 0:
            self.record("public tree", PASS)
        elif rcode == 3:
            self.record("public tree", SKIPPED, "deny list not configured (BARITONE_RS_DENYLIST); the rest passed")
        else:
            self.record("public tree", FAIL, "\n" + out)

    def cargo(self):
        low = busy()
        env = dict(os.environ, CARGO_TERM_COLOR="never")
        if low:
            env["CARGO_BUILD_JOBS"] = "4"
        for label, cmd in (
            ("cargo fmt --check", ["cargo", "fmt", "--check", "--", "--color", "never"]),
            ("cargo clippy", ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"]),
            ("cargo test", ["cargo", "test"]),
        ):
            rcode, out = self.sh(cmd, CRATE, env, low)
            if rcode:
                self.record(label, FAIL, "\n" + out[:3000])
                return
            self.record(label, PASS)

    def python_tests(self):
        rcode, out = self.sh([sys.executable, "-m", "unittest", "discover", "-s", HERE, "-p", "test_*.py"], REPO)
        tail = out.strip().splitlines()[-1] if out.strip() else ""
        self.record("python tests", PASS if rcode == 0 else FAIL, tail if rcode == 0 else "\n" + out[-3000:])

    def ledger(self):
        path = os.path.join(HERE, "ledger.py")
        if not os.path.exists(path):
            self.record("ledger check", SKIPPED, "tools/ledger.py does not exist yet")
            return
        upstream = self.a.upstream or os.environ.get("BARITONE_UPSTREAM")
        if not upstream:
            self.record("ledger check", SKIPPED, "no upstream checkout given (--upstream or BARITONE_UPSTREAM)")
            return
        rcode, out = self.sh([sys.executable, path, "--upstream", upstream, "check"], REPO)
        self.record("ledger check", PASS if rcode == 0 else FAIL, "" if rcode == 0 else "\n" + out[-3000:])

    def comparison(self):
        names = ("comparison", "repeat runs", "mutation")
        a = self.a
        if a.quick:
            for n in names:
                self.record(n, SKIPPED, "--quick")
            return
        try:
            upstream, jdk, scratch, grids, branch = rc.resolve(a)
        except rc.Fail as e:
            for n in names:
                self.record(n, SKIPPED, str(e))
            return
        path = rc.report_path_for(branch)
        report = load_report(path)
        try:
            commit = rc.upstream_commit(upstream)
        except rc.Fail as e:
            for n in names:
                self.record(n, SKIPPED, str(e))
            return
        minputs = mutation_inputs_hash()
        changed = a.force or not comparison_current(report, rc.source_hashes(), commit, grids, minputs)
        if not changed:
            self.record("comparison", CURRENT, f"report for {branch} matches the inputs; not re-run")
            self.record("repeat runs", CURRENT, "not needed")
            self.record("mutation", CURRENT, "not needed")
            return
        try:
            first = rc.execute(upstream, jdk, scratch, grids, branch, path)
        except rc.Fail as e:
            self.record("comparison", FAIL, "\n" + str(e))
            return
        # The report is written only after the repeat runs and the mutation step pass. A report on disk
        # means all three passed, so a later run may call them UP TO DATE.
        self.record("comparison", PASS, f"{branch}: " + ", ".join(f"{f} {r['evals']} evals" for f, r in first["results"].items()))
        with tempfile.TemporaryDirectory() as tmp:
            for i in (2, 3):
                try:
                    again = rc.execute(upstream, jdk, scratch, grids, branch, os.path.join(tmp, f"run{i}.json"))
                except rc.Fail as e:
                    os.remove(path)
                    self.record("repeat runs", FAIL, f"run {i}: {e}")
                    return
                if again != first:
                    os.remove(path)
                    self.record("repeat runs", FAIL, f"run {i} differs from run 1 (the report was deleted)")
                    return
        self.record("repeat runs", PASS, "3 runs, identical reports")
        # The last run left the golden file in scratch; mutation uses it.
        if a.no_mutation:
            self.record("mutation", SKIPPED, "--no-mutation; no report is written, so the next full run starts again")
            return
        if self.mutation(os.path.join(scratch, f"golden-{branch}.txt")):
            first["gate"] = {"mutation_inputs": minputs}
            rc.write_report(first, path)

    def mutation(self, golden):
        rcode, out = self.sh([sys.executable, os.path.join(CRATE, "mutate.py"), CRATE, golden], REPO, low=busy())
        try:
            with open(KNOWN_MISSED, encoding="utf-8") as f:
                known = parse_known_missed(f.read())
            caught, missed, not_applied = parse_mutation_output(out)
        except (OSError, ValueError, SyntaxError) as e:
            self.record("mutation", FAIL, str(e) + "\n" + out[-1500:])
            return False
        ok, msgs = judge_mutation(caught, missed, not_applied, known)
        summary = f"{len(caught)} caught, {len(missed)} missed ({len([m for m in missed if m in known])} listed), {len(not_applied)} not applied"
        self.record("mutation", PASS if ok else FAIL, summary + ("\n" + "\n".join(msgs) if msgs else ""))
        return ok

    # -- driver --------------------------------------------------------------------------------

    def run(self):
        for step in (self.public_tree, self.cargo, self.python_tests, self.ledger, self.comparison):
            step()
            if self.failed:
                break
        skipped = [s["step"] for s in self.steps if s["status"] == SKIPPED]
        status = "failed" if self.failed else ("incomplete" if skipped else "passed")
        print()
        if self.failed:
            print("GATE FAILED (stopped at the first failure)")
        elif skipped:
            print("GATE INCOMPLETE: nothing failed, but these steps were SKIPPED: " + ", ".join(skipped))
        else:
            print("GATE PASSED")
        self.write(status, skipped)
        return {"passed": 0, "failed": 1, "incomplete": 2}[status]

    def write(self, status, skipped):
        head = subprocess.run(["git", "-C", REPO, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        dirty = bool(subprocess.run(["git", "-C", REPO, "status", "--porcelain"], capture_output=True, text=True).stdout.strip())
        data = {
            "time": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
            "status": status,
            "head": head,
            "uncommitted_changes": dirty,
            "skipped": skipped,
            "steps": self.steps,
        }
        with open(RESULT, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
            f.write("\n")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--quick", action="store_true", help="skip the comparison, repeat runs and mutation")
    ap.add_argument("--no-mutation", action="store_true")
    ap.add_argument("--force", action="store_true", help="run the comparison even if the report is current")
    for n in ("upstream", "jdk", "scratch", "grids", "branch"):
        ap.add_argument("--" + n)
    return Gate(ap.parse_args(argv)).run()


if __name__ == "__main__":
    sys.exit(main())
