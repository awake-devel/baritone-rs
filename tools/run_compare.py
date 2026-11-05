#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Runs one cost-compare comparison end to end and writes reports/cost-compare-<branch>.json on success.

Steps: delete the earlier report; copy the Java test into the upstream checkout; run it with Gradle to write the
golden file; remove the copied files (always); run the Rust comparison on the golden file; parse; write the report.

Inputs (argument, or environment variable):
  --upstream / BARITONE_UPSTREAM   checkout of upstream's branch
  --jdk      / BARITONE_JDK        JDK home
  --scratch  / BARITONE_SCRATCH    directory for the golden file
  --grids    / BARITONE_GRIDS      number of random grids (default 40000)
  --branch   / BARITONE_BRANCH     branch name for the report (default: the Minecraft version in gradle.properties)

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

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
CRATE = os.path.join(REPO, "tools", "cost-compare")
JAVA_TEST = "baritone.pathing.movement.SpikeGoldenDump"
FUNCS = ("traverse", "descend")
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


def source_hashes(root=CRATE):
    """sha256 of every source that takes part in a comparison, keyed by a repository-relative name."""
    out = {}
    for rel in java_files(root):
        out["java/" + rel] = sha256_file(os.path.join(root, "java", *rel.split("/")))
    for rel in ("src/lib.rs", "src/main.rs", "Cargo.toml"):
        out[rel] = sha256_file(os.path.join(root, *rel.split("/")))
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
    return res


def check_results(res):
    bad = {f: r["wrong"] for f, r in res.items() if r["wrong"]}
    if bad:
        raise Fail("wrong results: " + ", ".join(f"{f}={n}" for f, n in sorted(bad.items())))


def read_minecraft_version(upstream):
    with open(os.path.join(upstream, "gradle.properties"), encoding="utf-8") as f:
        for ln in f:
            k, _, v = ln.partition("=")
            if k.strip() == "minecraft_version":
                return v.strip()
    raise Fail("no minecraft_version in gradle.properties")


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def upstream_commit(upstream):
    r = run(["git", "-C", upstream, "rev-parse", "HEAD"])
    if r.returncode:
        raise Fail("upstream is not a git checkout")
    return r.stdout.strip()


def pgrep(*args):
    r = run(["pgrep", *args])
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


def copy_java(upstream):
    """Copies the Java test in. Returns the files created and directories created, to remove afterwards."""
    base = os.path.join(CRATE, "java")
    dest_root = os.path.join(upstream, "src", "test", "java")
    files, dirs = [], []
    for rel in java_files():
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
        shutil.copyfile(os.path.join(base, *rel.split("/")), dest)
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


def execute(upstream, jdk, scratch, grids, branch, report_path):
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
        files, dirs = copy_java(upstream)
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
    r = run(["cargo", "run", "--release", "--quiet", "--", golden], cwd=CRATE)
    if r.returncode:
        raise Fail("the Rust comparison failed to run:\n" + (r.stdout + r.stderr)[-2000:])
    res = parse_output(r.stdout)
    check_results(res)
    jv = run([os.path.join(jdk, "bin", "java"), "-version"])
    report = {
        "tool": "cost-compare",
        "upstream_branch": branch,
        "upstream_commit": commit,
        "minecraft_version": mc,
        "java": (jv.stderr or jv.stdout).splitlines()[0].strip() if (jv.stderr or jv.stdout) else "unknown",
        "rustc": run(["rustc", "--version"]).stdout.strip(),
        "grids": int(grids),
        "sources": dict(source_hashes(), golden=sha256_file(golden)),
        "results": {f: {k: v for k, v in res[f].items()} for f in FUNCS},
    }
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
    grids = int(get(args.grids, "BARITONE_GRIDS", 40000))
    branch = get(args.branch, "BARITONE_BRANCH") or read_minecraft_version(upstream)
    return upstream, jdk, scratch, grids, branch


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    for n in ("upstream", "jdk", "scratch", "grids", "branch"):
        ap.add_argument("--" + n)
    ap.add_argument("--report", help="write the report here instead of reports/cost-compare-<branch>.json")
    a = ap.parse_args(argv)
    try:
        upstream, jdk, scratch, grids, branch = resolve(a)
        path = a.report or report_path_for(branch)
        report = execute(upstream, jdk, scratch, grids, branch, path)
    except Fail as e:
        print("FAIL:", e, file=sys.stderr)
        return 1
    write_report(report, path)
    print("OK:", ", ".join(f"{f}: {r['evals']} evals, exact {r['exact']}, close {r['close']}" for f, r in report["results"].items()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
