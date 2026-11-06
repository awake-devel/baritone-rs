#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Runs the L2 comparison (whole searches, node by node) and writes reports/search-compare-<branch>.json on success.

Steps: delete the earlier report; copy the Java tests (tools/cost-compare/java, which holds the palette and the configurations,
and tools/search-compare/java) into the upstream checkout; run SearchGoldenDump with Gradle to write the golden file; remove the
copied files (always); run the Rust comparison (tools/search-compare, a member of the root workspace) on the golden file; check its
counts; write the report. With --repeat N the whole run is done N times and the reports must be identical (the golden file too).

Inputs (argument, or environment variable):
  --upstream / BARITONE_UPSTREAM   checkout of upstream's 1.21.4 branch
  --jdk      / BARITONE_JDK        JDK home
  --scratch  / BARITONE_SCRATCH    directory for the golden file
  --worlds   / BARITONE_WORLDS     number of generated worlds (default 900)
  --cases    / BARITONE_CASES      search chains per world (default 24)
  --branch   / BARITONE_BRANCH     branch name for the report (default: the Minecraft version in gradle.properties)

The report holds metadata only: commits, versions, hashes and counts. No paths, no game data, no timings (those are printed).
"""
import argparse
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run_compare as rc  # noqa: E402

REPO = rc.REPO
CRATE = os.path.join(REPO, "tools", "search-compare")
JAVA_DIRS = [os.path.join(REPO, "tools", "cost-compare", "java"), os.path.join(CRATE, "java")]
JAVA_TEST = "baritone.pathing.calc.SearchGoldenDump"
DEFAULT_WORLDS = 900
DEFAULT_CASES = 24
Fail = rc.Fail


def java_files():
    """{path under src/test/java: file} of every Java file to copy in."""
    out = {}
    for base in JAVA_DIRS:
        for d, _, names in os.walk(base):
            for n in names:
                p = os.path.join(d, n)
                rel = os.path.relpath(p, base).replace(os.sep, "/")
                if rel in out:
                    raise Fail("two Java sources for " + rel)
                out[rel] = p
    return out


def source_hashes():
    out = {}
    for rel, p in sorted(java_files().items()):
        out["java/" + rel] = rc.sha256_file(p)
    files = ["Cargo.toml", "Cargo.lock", "rustfmt.toml", "tools/search-compare/Cargo.toml", "mod/engine/Cargo.toml", "tools/run_search.py"]
    for top in ("tools/search-compare/src", "mod/engine/src"):
        for d, _, names in os.walk(os.path.join(REPO, *top.split("/"))):
            for n in names:
                if n.endswith(".rs"):
                    files.append(os.path.relpath(os.path.join(d, n), REPO).replace(os.sep, "/"))
    for rel in sorted(files):
        out[rel] = rc.sha256_file(os.path.join(REPO, *rel.split("/")))
    return out


def copy_java(upstream):
    dest_root = os.path.join(upstream, "src", "test", "java")
    files, dirs = [], []
    for rel, src in sorted(java_files().items()):
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
        with open(src, encoding="utf-8") as f:
            text = f.read()
        with open(dest, "w", encoding="utf-8") as f:
            f.write(text)
        files.append(dest)
    return files, dirs


def cargo_run(golden):
    cmd = ["cargo", "run", "--release", "--quiet", "-p", "search-compare", "--", golden]
    env = dict(os.environ)
    if rc.pgrep("-x", "rustc"):
        cmd = ["nice", "-n", "15"] + cmd
        env["CARGO_BUILD_JOBS"] = "4"
    return rc.run(cmd, cwd=REPO, env=env)


def one_run(upstream, jdk, scratch, worlds, cases, branch, tag):
    """One full run. Returns (report dict, timings dict). Raises Fail."""
    if rc.pgrep("-x", "horizon-client"):
        raise Fail("a game (horizon-client) is running; not starting Gradle")
    if rc.gradle_client_running():
        raise Fail("another Gradle run is in progress; only one at a time")
    os.makedirs(scratch, exist_ok=True)
    golden = os.path.join(scratch, f"search-golden-{branch}-{tag}.txt")
    if os.path.exists(golden):
        os.remove(golden)
    files, dirs = [], []
    t0 = time.time()
    try:
        files, dirs = copy_java(upstream)
        env = dict(os.environ, JAVA_HOME=jdk, SEARCH_OUT=golden, SEARCH_WORLDS=str(worlds), SEARCH_CASES=str(cases))
        g = rc.run(
            ["./gradlew", "--no-daemon", "--console=plain", "cleanTest", "test", "--tests", JAVA_TEST, f"-Porg.gradle.java.installations.paths={jdk}"],
            cwd=upstream, env=env,
        )
    finally:
        rc.remove_copied(files, dirs)
    t_java = time.time() - t0
    if g.returncode or not os.path.exists(golden):
        raise Fail("Gradle failed or wrote no golden file:\n" + (g.stdout + g.stderr)[-3000:])
    t1 = time.time()
    r = cargo_run(golden)
    t_rust = time.time() - t1
    if r.returncode not in (0, 1):
        raise Fail("the Rust comparison failed to run:\n" + (r.stdout + r.stderr)[-3000:])
    try:
        line = next(ln for ln in r.stdout.splitlines() if ln.startswith("REPORT "))
        res = json.loads(line[len("REPORT "):])
    except (StopIteration, ValueError):
        raise Fail("no report from the comparison:\n" + (r.stdout + r.stderr)[-3000:])
    print(r.stdout.replace(line, "").rstrip(), flush=True)
    problems = []
    if res["mismatches"]:
        problems.append(f"{res['mismatches']} mismatches")
    zero = [k for k, v in res["coverage"].items() if v == 0 and k not in res["exempt"]]
    if zero:
        problems.append("coverage stayed at zero for: " + ", ".join(zero))
    if res["searches"] == 0:
        problems.append("no searches")
    if problems:
        print(r.stdout[-6000:], file=sys.stderr)
        raise Fail("; ".join(problems))
    jv = rc.run([os.path.join(jdk, "bin", "java"), "-version"])
    report = {
        "tool": "search-compare",
        "upstream_branch": branch,
        "upstream_commit": rc.upstream_commit(upstream),
        "minecraft_version": rc.read_minecraft_version(upstream),
        "java": (jv.stderr or jv.stdout).splitlines()[0].strip() if (jv.stderr or jv.stdout) else "unknown",
        "rustc": rc.run(["rustc", "--version"]).stdout.strip(),
        "worlds": int(worlds),
        "cases_per_world": int(cases),
        "sources": dict(source_hashes(), golden=rc.sha256_file(golden)),
        "results": res,
    }
    return report, {"java_s": round(t_java, 1), "rust_s": round(t_rust, 1), "golden_mb": round(os.path.getsize(golden) / 1e6, 1)}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    for n in ("upstream", "jdk", "scratch", "worlds", "cases", "branch"):
        ap.add_argument("--" + n)
    ap.add_argument("--repeat", type=int, default=1, help="run N times and require identical reports")
    ap.add_argument("--report", help="write the report here instead of reports/search-compare-<branch>.json")
    a = ap.parse_args(argv)
    get = lambda v, env, d=None: v or os.environ.get(env) or d
    try:
        upstream, jdk, scratch = (get(getattr(a, n), "BARITONE_" + n.upper()) for n in ("upstream", "jdk", "scratch"))
        for n, v in (("upstream", upstream), ("jdk", jdk), ("scratch", scratch)):
            if not v:
                raise Fail(f"missing --{n} (or the BARITONE_{n.upper()} environment variable)")
        upstream, jdk, scratch = map(os.path.abspath, (upstream, jdk, scratch))
        worlds = int(get(a.worlds, "BARITONE_WORLDS", DEFAULT_WORLDS))
        cases = int(get(a.cases, "BARITONE_CASES", DEFAULT_CASES))
        branch = get(a.branch, "BARITONE_BRANCH") or rc.read_minecraft_version(upstream)
        path = a.report or os.path.join(REPO, "reports", f"search-compare-{branch}.json")
        if os.path.exists(path):
            os.remove(path)
        reports = []
        for i in range(a.repeat):
            report, timing = one_run(upstream, jdk, scratch, worlds, cases, branch, f"run{i + 1}")
            print(f"run {i + 1}/{a.repeat}: java {timing['java_s']} s (Gradle included), rust {timing['rust_s']} s, golden file {timing['golden_mb']} MB", flush=True)
            reports.append(report)
        for i, rep in enumerate(reports[1:], 2):
            if rep != reports[0]:
                raise Fail(f"run {i} gave another report than run 1")
    except Fail as e:
        print("FAIL:", e, file=sys.stderr)
        return 1
    rc.write_report(reports[0], path)
    res = reports[0]["results"]
    print(f"OK: {res['searches']} searches, {res['mismatches']} mismatches, {a.repeat} identical run(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
