#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Runs one cost-compare comparison end to end and writes reports/cost-compare-<branch>.json on success.

Steps: delete the earlier report; copy the Java test into the upstream checkout; run it with Gradle to write the
golden file; remove the copied files (always); run the Rust comparison (tools/cost-compare, a member of the root
workspace) on the golden file; parse; write the report.

Inputs (argument, or environment variable):
  --upstream / BARITONE_UPSTREAM   checkout of upstream's branch
  --jdk      / BARITONE_JDK        JDK home
  --scratch  / BARITONE_SCRATCH    directory for the golden file
  --grids    / BARITONE_GRIDS      number of random grids (default 10000)
  --branch   / BARITONE_BRANCH     branch name for the report (default: the Minecraft version in gradle.properties)
  --port 26.3                      run the harness on upstream's 26.3 branch: tools/cost-compare/port_26_3.py turns the Java
                                   source into the 26.3 variant on the way in. The engine follows 1.21.4's rules, so the two
                                   branches' own differences are expected: they are recorded in the report, not a failure. The
                                   same golden file must then show none with rules-26.3.patch applied to a scratch copy.

The golden file stays in the scratch directory as golden-<branch>.txt.
The report holds metadata only: commits, versions, hashes and counts. No paths, no game data.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "cost-compare"))
import port_26_3  # noqa: E402

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
CRATE = os.path.join(REPO, "tools", "cost-compare")
PATCH_26_3 = os.path.join(CRATE, "rules-26.3.patch")
JAVA_TEST = "baritone.pathing.movement.SpikeGoldenDump"
# Every family the comparison prints a line for. A movement or goal family with no finite result tested nothing.
MOVEMENTS = ("downward", "pillar", "traverse", "ascend", "descend", "diagonal", "parkour")
GOALS = tuple("goal_" + n for n in ("block", "xz", "ylevel", "twoblocks", "gettoblock", "near", "axis", "strict", "runaway", "composite", "inverted"))
OTHERS = ("predicates", "states", "fall_table")
FUNCS = MOVEMENTS + GOALS + OTHERS
DEFAULT_GRIDS = 10000
LINE = re.compile(
    r"^(\w+): (\d+) evals, (\d+) finite, (\d+) distinct, exact (\d+), within 1e-9 (\d+), WRONG (\d+)\s*$", re.M
)


class Fail(Exception):
    pass


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def java_files(root=CRATE):
    """Relative paths (with /) of the Java test files under tools/cost-compare/java."""
    base = os.path.join(root, "java")
    out = []
    for d, _, names in os.walk(base):
        for n in names:
            out.append(os.path.relpath(os.path.join(d, n), base).replace(os.sep, "/"))
    return sorted(out)


def source_hashes(repo=REPO, port=None):
    """sha256 of every source that takes part in a comparison, keyed by a repository-relative name: the Java test (as
    'java/...'), the comparison's own Rust sources, the engine's sources, and the Cargo and format files of the workspace.
    With a port, also the files that make the Java test into that variant."""
    crate = os.path.join(repo, "tools", "cost-compare")
    out = {}
    if port:
        for rel in port_26_3.source_files() + ["rules-26.3.patch"]:
            out["port/" + rel] = sha256_file(os.path.join(crate, *rel.split("/")))
    for rel in java_files(crate):
        out["java/" + rel] = sha256_file(os.path.join(crate, "java", *rel.split("/")))
    files = ["Cargo.toml", "Cargo.lock", "rustfmt.toml", "tools/cost-compare/Cargo.toml", "mod/engine/Cargo.toml"]
    for top in ("tools/cost-compare/src", "mod/engine/src"):
        for d, _, names in os.walk(os.path.join(repo, *top.split("/"))):
            for n in names:
                if n.endswith(".rs"):
                    files.append(os.path.relpath(os.path.join(d, n), repo).replace(os.sep, "/"))
    for rel in sorted(files):
        out[rel] = sha256_file(os.path.join(repo, *rel.split("/")))
    return out


def parse_output(text):
    """Parses the stdout of the Rust comparison. Returns {function: counts}. Raises Fail if a function is missing."""
    res = {}
    for m in LINE.finditer(text):
        name, evals, finite, distinct, exact, close, wrong = m.group(1), *map(int, m.groups()[1:])
        res[name] = {
            "evals": evals,
            "finite": finite,
            "distinct": distinct,
            "exact": exact,
            "close": close,
            "wrong": wrong,
        }
    for f in FUNCS:
        if f not in res:
            raise Fail(f"no result line for '{f}' in the comparison output")
        if res[f]["evals"] == 0:
            raise Fail(f"'{f}' has no evaluations")
        if f in MOVEMENTS + GOALS and res[f]["finite"] == 0:
            raise Fail(f"'{f}' has no finite result: the inputs never reach it, so it tested nothing")
    return res


def check_results(res):
    bad = {f: r["wrong"] for f, r in res.items() if r["wrong"]}
    if bad:
        raise Fail("wrong results: " + ", ".join(f"{f}={n}" for f, n in sorted(bad.items())))


def rules_explain_every_difference(golden, scratch):
    """Applies rules-26.3.patch to a scratch copy of the engine and compares again. Anything left is a difference between the
    branches that nobody has listed, or a difference in the game, so it fails. So does a patch that no longer applies."""
    copy = os.path.join(scratch, "engine-with-26.3-rules")
    shutil.rmtree(copy, ignore_errors=True)
    os.makedirs(os.path.join(copy, "tools"))
    for name in ("Cargo.toml", "Cargo.lock", "rustfmt.toml"):
        shutil.copy(os.path.join(REPO, name), copy)
    shutil.copytree(os.path.join(REPO, "mod", "engine"), os.path.join(copy, "mod", "engine"), ignore=shutil.ignore_patterns("target"))
    shutil.copytree(CRATE, os.path.join(copy, "tools", "cost-compare"), ignore=shutil.ignore_patterns("target", "__pycache__", "java", "port-26.3"))
    # The root workspace lists it as a member, so Cargo needs it there, although this run does not build it.
    shutil.copytree(os.path.join(REPO, "tools", "search-compare"), os.path.join(copy, "tools", "search-compare"), ignore=shutil.ignore_patterns("target", "__pycache__", "java"))
    with open(PATCH_26_3, encoding="utf-8") as f:
        p = run(["patch", "-p1", "-F0"], cwd=copy, input=f.read())
    if p.returncode:
        raise Fail("rules-26.3.patch no longer applies:\n" + (p.stdout + p.stderr)[-2000:])
    r = cargo_run(golden, cwd=copy)
    if r.returncode:
        raise Fail("the comparison with the 26.3 rules failed to run:\n" + (r.stdout + r.stderr)[-2000:])
    res = parse_output(r.stdout)
    left = {f: n["wrong"] + n["close"] for f, n in res.items() if n["wrong"] or n["close"]}
    if left:
        raise Fail("differences from 26.3 that no listed rule explains: " + ", ".join(f"{f}={n}" for f, n in sorted(left.items())))


def read_minecraft_version(upstream):
    with open(os.path.join(upstream, "gradle.properties"), encoding="utf-8") as f:
        for ln in f:
            k, _, v = ln.partition("=")
            if k.strip() == "minecraft_version":
                return v.strip()
    raise Fail("no minecraft_version in gradle.properties")


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def cargo_run(golden, cwd=REPO):
    """The Rust comparison. At low priority and with few jobs while another build is running (it only builds a small crate)."""
    cmd = ["cargo", "run", "--release", "--quiet", "-p", "cost-compare", "--", golden]
    env = dict(os.environ)
    if pgrep("-x", "rustc"):
        cmd = ["nice", "-n", "15"] + cmd
        env["CARGO_BUILD_JOBS"] = "4"
    return run(cmd, cwd=cwd, env=env)


def upstream_commit(upstream):
    r = run(["git", "-C", upstream, "rev-parse", "HEAD"])
    if r.returncode:
        raise Fail("upstream is not a git checkout")
    return r.stdout.strip()


def pgrep(*args):
    r = subprocess.run(["pgrep", *args], capture_output=True, text=True)
    return r.returncode == 0


def gradle_client_running():
    """True if a Gradle wrapper client (a java process) is running. Matches the process name, not any shell mentioning it."""
    try:
        pids = [p for p in os.listdir("/proc") if p.isdigit()]
    except OSError:
        return False
    for pid in pids:
        try:
            with open(f"/proc/{pid}/comm") as f:
                if f.read().strip() != "java":
                    continue
            with open(f"/proc/{pid}/cmdline", "rb") as f:
                if b"GradleWrapperMain" in f.read():
                    return True
        except OSError:
            continue
    return False


def java_texts(port):
    """{path under src/test/java: text} of the test to copy in: the source as it is, or its variant for another branch."""
    if port == "26.3":
        try:
            return port_26_3.ported_files()
        except port_26_3.PortError as e:
            raise Fail("cannot port the harness to 26.3: " + str(e))
    return {rel: open(os.path.join(CRATE, "java", *rel.split("/")), encoding="utf-8").read() for rel in java_files()}


def copy_java(upstream, port=None):
    """Copies the Java test in. Returns the files created and directories created, to remove afterwards."""
    texts = java_texts(port)
    dest_root = os.path.join(upstream, "src", "test", "java")
    files, dirs = [], []
    for rel in sorted(texts):
        dest = os.path.join(dest_root, *rel.split("/"))
        if os.path.exists(dest):
            raise Fail("refusing to overwrite an existing upstream file: " + rel)
        d = os.path.dirname(dest)
        missing = []
        while not os.path.isdir(d):
            missing.append(d)
            d = os.path.dirname(d)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        dirs += reversed(missing)
        with open(dest, "w", encoding="utf-8") as f:
            f.write(texts[rel])
        files.append(dest)
    return files, dirs


def remove_copied(files, dirs):
    for f in files:
        try:
            os.remove(f)
        except FileNotFoundError:
            pass
    for d in reversed(dirs):
        try:
            os.rmdir(d)
        except OSError:
            pass


def execute(upstream, jdk, scratch, grids, branch, report_path, port=None):
    """Runs one comparison. Returns the report dict. Raises Fail."""
    upstream, jdk, scratch = map(os.path.abspath, (upstream, jdk, scratch))
    if os.path.exists(report_path):
        os.remove(report_path)
    if pgrep("-x", "horizon-client"):
        raise Fail("a game (horizon-client) is running; not starting Gradle")
    if gradle_client_running():
        raise Fail("another Gradle run is in progress; only one at a time")
    os.makedirs(scratch, exist_ok=True)
    golden = os.path.join(scratch, f"golden-{branch}.txt")
    if os.path.exists(golden):
        os.remove(golden)
    commit = upstream_commit(upstream)
    mc = read_minecraft_version(upstream)
    files, dirs = [], []
    try:
        files, dirs = copy_java(upstream, port)
        env = dict(os.environ, JAVA_HOME=jdk, SPIKE_OUT=golden, SPIKE_GRIDS=str(grids))
        # cleanTest first: Gradle would otherwise skip an up-to-date test and write no golden file.
        g = run(
            [
                "./gradlew", "--no-daemon", "--console=plain", "cleanTest", "test", "--tests", JAVA_TEST,
                f"-Porg.gradle.java.installations.paths={jdk}",
            ],
            cwd=upstream, env=env,
        )
    finally:
        remove_copied(files, dirs)
    if g.returncode or not os.path.exists(golden):
        raise Fail("Gradle failed or wrote no golden file:\n" + (g.stdout + g.stderr)[-2000:])
    r = cargo_run(golden)
    if r.returncode:
        raise Fail("the Rust comparison failed to run:\n" + (r.stdout + r.stderr)[-2000:])
    res = parse_output(r.stdout)
    if not port:
        check_results(res)
    else:
        rules_explain_every_difference(golden, scratch)
    jv = run([os.path.join(jdk, "bin", "java"), "-version"])
    report = {
        "tool": "cost-compare",
        "upstream_branch": branch,
        "upstream_commit": commit,
        "minecraft_version": mc,
        "java": (jv.stderr or jv.stdout).splitlines()[0].strip() if (jv.stderr or jv.stdout) else "unknown",
        "rustc": run(["rustc", "--version"]).stdout.strip(),
        "grids": int(grids),
        "sources": dict(source_hashes(port=port), golden=sha256_file(golden)),
        "results": {f: {k: v for k, v in res[f].items()} for f in FUNCS},
    }
    if port:
        report["port"] = port
        report["note"] = "engine follows 1.21.4's rules; WRONG counts are the two branches' own differences, see docs/deviations.md"
    return report


def write_report(report, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")


def report_path_for(branch):
    return os.path.join(REPO, "reports", f"cost-compare-{branch}.json")


def resolve(args):
    """Fills missing arguments from the environment. Returns (upstream, jdk, scratch, grids, branch) or raises Fail."""
    get = lambda v, env, d=None: v or os.environ.get(env) or d
    upstream = get(args.upstream, "BARITONE_UPSTREAM")
    jdk = get(args.jdk, "BARITONE_JDK")
    scratch = get(args.scratch, "BARITONE_SCRATCH")
    for name, v in (("upstream", upstream), ("jdk", jdk), ("scratch", scratch)):
        if not v:
            raise Fail(f"missing --{name} (or the BARITONE_{name.upper()} environment variable)")
    grids = int(get(args.grids, "BARITONE_GRIDS", DEFAULT_GRIDS))
    branch = get(args.branch, "BARITONE_BRANCH") or read_minecraft_version(upstream)
    return upstream, jdk, scratch, grids, branch


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    for n in ("upstream", "jdk", "scratch", "grids", "branch"):
        ap.add_argument("--" + n)
    ap.add_argument("--port", choices=["26.3"], help="run on upstream's 26.3 branch (see the top of this file)")
    ap.add_argument("--report", help="write the report here instead of reports/cost-compare-<branch>.json")
    a = ap.parse_args(argv)
    try:
        upstream, jdk, scratch, grids, branch = resolve(a)
        path = a.report or report_path_for(branch)
        report = execute(upstream, jdk, scratch, grids, branch, path, a.port)
    except Fail as e:
        print("FAIL:", e, file=sys.stderr)
        return 1
    write_report(report, path)
    print("OK:", ", ".join(f"{f}: {r['evals']} evals, {r['finite']} finite, exact {r['exact']}, close {r['close']}" for f, r in report["results"].items()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
