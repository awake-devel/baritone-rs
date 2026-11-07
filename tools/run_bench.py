#!/usr/bin/env python3
# baritone-rs benchmark tool. LGPL-3.0-or-later, see LICENSE.
"""Times upstream Baritone's real search against baritone-engine's on the same generated worlds, and writes reports/bench-<branch>.json.

Steps: build tools/bench (the Rust half); copy the Java files (tools/cost-compare/java, tools/search-compare/java and tools/bench/java)
into the upstream checkout, have Gradle compile them and print the test classpath, remove the copied files (always); wait until no
game, build or Gradle run is going on this machine; then, one side after the other and pinned to the same CPUs:

  A. finish: Java generates N worlds from fixed seeds, writes them, and times every case to its end (warm-up first, one discarded
     run, then repetitions); Rust reads the same files and does the same.
  B. limit: the cases that neither side finishes inside a clock limit (50 ms, 500 ms, 2000 ms, and upstream's own 500/2000 pair) are
     run again under that limit on both sides; nodes and the progress of the best partial path are compared.

The raw timings, the worlds and the tables stay in the scratch directory (results.json, tables.md). The report in reports/ holds
metadata and counts only: versions, machine, parameters, source hashes, how many searches, how many gave the same path.

Inputs (argument, or environment variable):
  --upstream / BARITONE_UPSTREAM   checkout of upstream's 1.21.4 branch
  --jdk      / BARITONE_JDK        JDK home
  --scratch  / BARITONE_SCRATCH    directory for the worlds, the raw results and the tables (several hundred MB)
  --seeds N                        number of world seeds, 1..N (default 20)
  --finish-ms MS                   clock limit of the time-to-finish measurement (default 10000)
  --reps N                         repetitions of a short search (default 5; longer ones get fewer, see SearchBench.java)
  --rounds N                       the time-to-finish phase is run N times, Java then Rust each time (default 2), to show drift
  --cpus A,B                       CPUs both sides are pinned to (default: one thread of each of two physical cores)
  --no-report                      do not write reports/bench-<branch>.json
  --wait-only                      only wait until the machine is idle, then exit 0
"""
import argparse
import json
import math
import os
import platform
import shutil
import statistics
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run_compare as rc  # noqa: E402
import run_search as rs  # noqa: E402

REPO = rc.REPO
BENCH = os.path.join(REPO, "tools", "bench")
BENCH_JAVA = os.path.join(BENCH, "java")
JAVA_MAIN = "baritone.pathing.calc.SearchBench"
HEAP = "6g"
JAVA_FLAGS = [f"-Xms{HEAP}", f"-Xmx{HEAP}", "-XX:+UseSerialGC"]
LIMITS = (50, 500, 2000)
DEFAULT_PAIR = (500, 2000)
# how many cases are timed under each limit, and how many repetitions
LIMIT_CASES = {"50": 14, "500": 14, "2000": 10, "default": 8}
LIMIT_REPS = {"50": 5, "500": 5, "2000": 3, "default": 3}
Fail = rc.Fail
INIT_GRADLE = """allprojects {
    if (it == rootProject) {
        afterEvaluate {
            tasks.register('benchClasspath') {
                dependsOn 'testClasses'
                doLast {
                    new File(System.getenv('BENCH_CP_OUT')).text = sourceSets.test.runtimeClasspath.asPath
                }
            }
        }
    }
}
"""


# ----------------------------------------------------------------------------------------------------------------------
# Reading and summarising results (pure functions, tested in test_bench.py)
# ----------------------------------------------------------------------------------------------------------------------

def parse_t(line):
    """The fields of a T line as a dict; ns and gcms become lists of ints."""
    f = dict(item.split("=", 1) for item in line.split(" ")[1:])
    f["ns"] = [int(x) for x in f["ns"].split(",")]
    f["gcms"] = [int(x) for x in f["gcms"].split(",")]
    for k in ("primary", "failure", "reps", "nodes", "moves", "map", "plen"):
        f[k] = int(f[k])
    f["best"] = tuple(int(x) for x in f["best"].split(","))
    f["besth"] = float(f["besth"])
    return f


def read_results(path):
    """{world seed: hash} and the T lines (ERR lines raise) of a result file."""
    worlds, runs = {}, []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("W "):
                kv = dict(x.split("=", 1) for x in line.split(" ")[1:])
                worlds[int(kv["seed"])] = kv["hash"]
            elif line.startswith("T "):
                runs.append(parse_t(line))
            elif line.startswith("ERR"):
                raise Fail(f"{path}: {line}")
    return worlds, runs


def read_cases(world_dir):
    """{case id: {seed, name, kind, goal, start, stress}} from the B lines of the world files."""
    cases = {}
    for name in sorted(os.listdir(world_dir)):
        if not (name.startswith("world-") and name.endswith(".txt")):
            continue
        with open(os.path.join(world_dir, name), encoding="utf-8") as f:
            for line in f:
                if line.startswith("V "):
                    break
                if line.startswith("B "):
                    kv = dict(x.split("=", 1) for x in line.split(" ")[1:])
                    spec = kv["goal"]
                    kind, args = spec.rstrip(")\n").split("(")
                    nums = [int(x) for x in args.split(",")]
                    start = tuple(int(x) for x in kv["start"].split(","))
                    gx, gz = (nums[0], nums[2]) if kind == "block" else (nums[0], nums[1])
                    cases[kv["id"]] = {
                        "seed": int(kv["id"].split(".")[0]), "name": kv["name"], "goal_kind": kind, "goal": spec,
                        "gx": gx, "gz": gz, "start": start, "stress": kv["stress"] == "1",
                        "dist": math.hypot(gx - start[0], gz - start[2]),
                    }
    return cases


def merge_rounds(rounds):
    """One T dict per case from the results of several rounds: the repetitions of all rounds in one list (round_ns keeps them apart). The
    counters and the path of a search do not depend on the clock when it finishes, so they must agree between rounds."""
    out = {}
    for rnd in rounds:
        for cid, t in rnd.items():
            if cid not in out:
                out[cid] = dict(t, round_ns=[list(t["ns"])])
                continue
            m = out[cid]
            if t["type"] == "SUCCESS_TO_GOAL" and (m["nodes"], m["phash"], m["type"]) != (t["nodes"], t["phash"], t["type"]):
                raise Fail(f"{cid}: a finished search gave another answer in another round")
            m["ns"] = m["ns"] + t["ns"]
            m["gcms"] = m["gcms"] + t["gcms"]
            m["round_ns"].append(list(t["ns"]))
    return out


def med(xs):
    return statistics.median(xs)


def quantile(xs, q):
    s = sorted(xs)
    if not s:
        return float("nan")
    k = (len(s) - 1) * q
    lo, hi = math.floor(k), math.ceil(k)
    return s[lo] + (s[hi] - s[lo]) * (k - lo)


def spread(xs):
    """(max - min) / median, as a fraction."""
    m = med(xs)
    return (max(xs) - min(xs)) / m if m else float("nan")


def compare_finish(java, rust, cases):
    """One row per case that both sides ran. Ratio = Java time / Rust time, so above 1 means Rust was faster."""
    rows = []
    for cid in sorted(java, key=lambda c: (cases[c]["seed"], c)):
        if cid not in rust:
            continue
        j, r = java[cid], rust[cid]
        same_search = (j["nodes"], j["moves"], j["map"], j["type"]) == (r["nodes"], r["moves"], r["map"], r["type"])
        same_path = same_search and j["phash"] == r["phash"] and j["plen"] == r["plen"]
        jm, rm = med(j["ns"]), med(r["ns"])
        finished = j["type"] == "SUCCESS_TO_GOAL" and r["type"] == "SUCCESS_TO_GOAL"
        rows.append({
            "id": cid, "seed": cases[cid]["seed"], "name": cases[cid]["name"], "goal": cases[cid]["goal"], "dist": cases[cid]["dist"],
            "java_type": j["type"], "rust_type": r["type"], "finished": finished,
            "java_ms": jm / 1e6, "rust_ms": rm / 1e6, "ratio": jm / rm,
            "java_spread": spread(j["ns"]), "rust_spread": spread(r["ns"]), "java_reps": len(j["ns"]), "rust_reps": len(r["ns"]),
            "java_min_ms": min(j["ns"]) / 1e6, "java_max_ms": max(j["ns"]) / 1e6, "rust_min_ms": min(r["ns"]) / 1e6, "rust_max_ms": max(r["ns"]) / 1e6,
            "nodes_java": j["nodes"], "nodes_rust": r["nodes"], "moves_java": j["moves"], "moves_rust": r["moves"],
            "plen": j["plen"], "same_search": same_search, "same_path": same_path,
            "java_nps": j["nodes"] / (jm / 1e9), "rust_nps": r["nodes"] / (rm / 1e9),
            "java_gc_ms": med(j["gcms"]),
            "round_ratios": [med(a) / med(b) for a, b in zip(j.get("round_ns", [j["ns"]]), r.get("round_ns", [r["ns"]]))],
        })
    return rows


def summarize(values):
    """Median, quartiles, range of a list of ratios."""
    if not values:
        return None
    return {"n": len(values), "median": med(values), "q1": quantile(values, 0.25), "q3": quantile(values, 0.75), "min": min(values), "max": max(values),
            "rust_slower": sum(1 for v in values if v < 1.0)}


def seed_ratios(rows):
    """Per seed: the Java time over the Rust time of the seed's finished cases added up (one world, so one paired comparison)."""
    by = {}
    for r in rows:
        if r["finished"]:
            by.setdefault(r["seed"], []).append(r)
    return {s: sum(r["java_ms"] for r in rs_) / sum(r["rust_ms"] for r in rs_) for s, rs_ in sorted(by.items())}


def group_limit(runs):
    """{(id, primary, failure): [T dicts]}"""
    out = {}
    for t in runs:
        out.setdefault((t["id"], t["primary"], t["failure"]), []).append(t)
    return out


def hdist(best, case):
    return math.hypot(best[0] - case["gx"], best[2] - case["gz"])


def compare_limit(java, rust, cases):
    """One row per (case, limits) that both sides ran. Progress is measured on the best partial path: how far (horizontally) its end is from
    the goal, how many blocks closer than the start that is, and the goal's heuristic there (smaller is closer)."""
    rows = []
    jg, rg = group_limit(java), group_limit(rust)
    for key in sorted(jg, key=lambda k: (cases[k[0]]["seed"], k[0], k[1], k[2])):
        if key not in rg:
            continue
        cid, p, f = key
        c = cases[cid]
        jr, rr = jg[key], rg[key]

        def side(runs):
            rem = [hdist(t["best"], c) for t in runs]
            return {
                "nodes": med([t["nodes"] for t in runs]), "ms": med([sum(t["ns"]) / len(t["ns"]) for t in runs]) / 1e6,
                "rem": med(rem), "closer": c["dist"] - med(rem), "besth": med([t["besth"] for t in runs]),
                "types": sorted({t["type"] for t in runs}), "n": len(runs),
                "nodes_min": min(t["nodes"] for t in runs), "nodes_max": max(t["nodes"] for t in runs),
            }
        j, r = side(jr), side(rr)
        rows.append({"id": cid, "seed": c["seed"], "name": c["name"], "goal": c["goal"], "dist": c["dist"], "primary": p, "failure": f, "java": j, "rust": r,
                     "node_ratio": r["nodes"] / j["nodes"], "closer_ratio": (r["closer"] / j["closer"]) if j["closer"] > 0 else float("nan")})
    return rows


def pick_limit_cases(rows, cases, limit, count, finish_ms, taken=()):
    """Cases that neither side finishes inside `limit` ms: the faster of the two medians is over 1.25 x the limit, or one side did not
    finish in the finish run at all. Spread over the seeds, deterministic. Returns case ids."""
    eligible = []
    for r in rows:
        if r["id"] in taken or cases[r["id"]]["stress"]:
            continue
        fastest = min(r["java_ms"], r["rust_ms"])
        unfinished = not r["finished"]
        if unfinished or fastest > 1.25 * limit:
            eligible.append(r["id"])
    eligible.sort(key=lambda c: (cases[c]["seed"], c))
    if len(eligible) <= count:
        return eligible
    step = len(eligible) / count
    return [eligible[int(i * step)] for i in range(count)]


# ----------------------------------------------------------------------------------------------------------------------
# The machine
# ----------------------------------------------------------------------------------------------------------------------

def cpu_model():
    with open("/proc/cpuinfo", encoding="utf-8") as f:
        for line in f:
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    return platform.processor()


def default_cpus():
    """One logical CPU of each of two physical cores (cores 2 and 3 when the machine has them)."""
    r = subprocess.run(["lscpu", "-p=CPU,CORE"], capture_output=True, text=True)
    first = {}
    for line in r.stdout.splitlines():
        if line.startswith("#") or not line.strip():
            continue
        cpu, core = (int(x) for x in line.split(","))
        first.setdefault(core, cpu)
    cores = sorted(first)
    pick = cores[2:4] if len(cores) >= 4 else cores[:2]
    return ",".join(str(first[c]) for c in pick)


HEAVY = {"horizon-client", "cargo", "rustc", "rust-lld", "cc1", "cc1plus", "ld", "lld", "mold", "gradle"}


def busy_pids():
    """Processes that make timings unreliable: a game, a build, a Gradle run. Mine are not running when this is asked."""
    out = []
    me = os.getpid()
    for pid in os.listdir("/proc"):
        if not pid.isdigit() or int(pid) == me:
            continue
        try:
            with open(f"/proc/{pid}/comm") as f:
                comm = f.read().strip()
            if comm in HEAVY:
                out.append((int(pid), comm))
            elif comm == "java":
                with open(f"/proc/{pid}/cmdline", "rb") as f:
                    cmd = f.read()
                if b"GradleWrapperMain" in cmd or b"SearchBench" in cmd:
                    out.append((int(pid), "gradle/java"))
        except OSError:
            continue
    return out


def wait_idle(log, quiet_checks=3):
    """Waits, by PID, until no game, build or Gradle run is going. Returns what it waited for."""
    waited = []
    quiet = 0
    while quiet < quiet_checks:
        pids = busy_pids()
        if not pids:
            quiet += 1
            time.sleep(2)
            continue
        quiet = 0
        for pid, comm in pids:
            log(f"waiting for {comm} (pid {pid})")
            waited.append(comm)
            while os.path.exists(f"/proc/{pid}"):
                time.sleep(5)
    return sorted(set(waited))


def cpu_busy_fraction(seconds=5):
    """Share of all CPUs' time spent on something other than idle and waiting, over a few seconds."""
    def snap():
        with open("/proc/stat") as f:
            v = [int(x) for x in f.readline().split()[1:]]
        return sum(v), v[3] + v[4]
    t0, i0 = snap()
    time.sleep(seconds)
    t1, i1 = snap()
    return 1 - (i1 - i0) / max(1, t1 - t0)


def wait_quiet_cpu(log, limit=0.06, need=3, give_up_s=900):
    """Waits until the whole machine has been nearly idle for `need` samples of 5 s. Returns the busiest quiet-wait sample seen, or None if it gave up."""
    t0, quiet = time.time(), 0
    while quiet < need:
        b = cpu_busy_fraction()
        if b < limit:
            quiet += 1
        else:
            quiet = 0
            log(f"machine busy ({b * 100:.0f} % of all CPUs), waiting")
        if time.time() - t0 > give_up_s:
            log("giving up waiting for a quiet machine")
            return None
    return True


def loadavg():
    with open("/proc/loadavg") as f:
        return [float(x) for x in f.read().split()[:3]]


def read_temp():
    best = None
    base = "/sys/class/thermal"
    try:
        for z in os.listdir(base):
            if not z.startswith("thermal_zone"):
                continue
            try:
                with open(f"{base}/{z}/type") as f:
                    t = f.read().strip()
                with open(f"{base}/{z}/temp") as f:
                    v = int(f.read().strip()) / 1000
            except (OSError, ValueError):
                continue
            if t == "x86_pkg_temp":
                return v
            best = v if best is None else max(best, v)
    except OSError:
        pass
    return best


def read_mhz(cpus):
    out = []
    try:
        with open("/proc/cpuinfo", encoding="utf-8") as f:
            cur = None
            for line in f:
                if line.startswith("processor"):
                    cur = int(line.split(":")[1])
                elif line.startswith("cpu MHz") and cur in cpus:
                    out.append(float(line.split(":")[1]))
    except OSError:
        pass
    return out


class Monitor(threading.Thread):
    """While a timed phase runs: notes any process that disturbs the timings, and samples the pinned CPUs' clock and the package temperature."""

    def __init__(self, cpus, mine):
        super().__init__(daemon=True)
        self.cpus, self.mine, self.stop_flag = [int(c) for c in cpus.split(",")], mine, threading.Event()
        self.intruders, self.mhz, self.temp = {}, [], []

    def run(self):
        while not self.stop_flag.is_set():
            for pid, comm in busy_pids():
                if pid not in self.mine():
                    self.intruders[comm] = self.intruders.get(comm, 0) + 1
            self.mhz += read_mhz(self.cpus)
            t = read_temp()
            if t is not None:
                self.temp.append(t)
            self.stop_flag.wait(1.0)

    def summary(self):
        return {
            "intruders": self.intruders,
            "cpu_mhz": {"min": round(min(self.mhz)), "median": round(med(self.mhz)), "max": round(max(self.mhz))} if self.mhz else None,
            "package_c": {"min": min(self.temp), "max": max(self.temp)} if self.temp else None,
        }


# ----------------------------------------------------------------------------------------------------------------------
# Running
# ----------------------------------------------------------------------------------------------------------------------

def build_rust(log):
    env = dict(os.environ)
    cmd = ["cargo", "build", "--release", "--quiet"]
    if rc.pgrep("-x", "horizon-client"):
        cmd = ["nice", "-n", "15"] + cmd
        env["CARGO_BUILD_JOBS"] = "4"
    r = rc.run(cmd, cwd=BENCH, env=env)
    if r.returncode:
        raise Fail("building tools/bench failed:\n" + (r.stdout + r.stderr)[-3000:])
    exe = os.path.join(BENCH, "target", "release", "search-bench")
    log("built " + exe)
    return exe


def bootstrap_java(upstream, jdk, scratch, log):
    """Has Gradle compile the Java files together with upstream's tests and print the test classpath. Returns the classpath with the
    compiled test classes taken from a copy in the scratch directory (the checkout's build folder is rebuilt by other runs)."""
    if rc.gradle_client_running():
        raise Fail("another Gradle run is in progress; only one at a time")
    init = os.path.join(scratch, "cp.init.gradle")
    cpout = os.path.join(scratch, "cp.txt")
    with open(init, "w", encoding="utf-8") as f:
        f.write(INIT_GRADLE)
    if os.path.exists(cpout):
        os.remove(cpout)
    saved = rs.JAVA_DIRS
    rs.JAVA_DIRS = saved + [BENCH_JAVA]
    files, dirs = [], []
    try:
        files, dirs = rs.copy_java(upstream)
        env = dict(os.environ, JAVA_HOME=jdk, BENCH_CP_OUT=cpout)
        g = rc.run(["./gradlew", "--no-daemon", "--console=plain", "-I", init, "benchClasspath", f"-Porg.gradle.java.installations.paths={jdk}"], cwd=upstream, env=env)
        if g.returncode or not os.path.exists(cpout):
            raise Fail("Gradle failed to compile the benchmark:\n" + (g.stdout + g.stderr)[-3000:])
        classes = os.path.join(scratch, "classes")
        shutil.rmtree(classes, ignore_errors=True)
        shutil.copytree(os.path.join(upstream, "build", "classes", "java", "test"), classes)
    finally:
        rs.JAVA_DIRS = saved
        rc.remove_copied(files, dirs)
    with open(cpout, encoding="utf-8") as f:
        cp = f.read().strip()
    test_dir = os.path.join(upstream, "build", "classes", "java", "test")
    parts = [classes if p == test_dir else p for p in cp.split(":")]
    log("compiled the benchmark, classpath of %d entries" % len(parts))
    return ":".join(parts)


def run_side(name, cmd, cpus, outdir, log):
    """Runs one side pinned to the CPUs, its output into a log file; returns (exit code, seconds, monitor summary)."""
    logf = os.path.join(outdir, name + ".log")
    pids = set()
    mon = Monitor(cpus, lambda: pids)
    t0 = time.time()
    with open(logf, "w", encoding="utf-8") as lf:
        p = subprocess.Popen(["taskset", "-c", cpus] + cmd, stdout=lf, stderr=subprocess.STDOUT, cwd=outdir)  # cwd: Minecraft writes a logs/ folder
        pids.add(p.pid)
        mon.start()
        while p.poll() is None:
            time.sleep(15)
            with open(logf, encoding="utf-8") as f:
                tail = f.read().strip().splitlines()[-1:] or [""]
            log(f"  {name}: {int(time.time() - t0)} s, {tail[0][:100]}")
        mon.stop_flag.set()
        mon.join()
    return p.returncode, time.time() - t0, mon.summary()


def write_limit_list(path, picks):
    with open(path, "w", encoding="utf-8") as f:
        for group, ids in picks:
            primary, failure = (DEFAULT_PAIR if group == "default" else (int(group), int(group)))
            for cid in ids:
                f.write(f"{cid} {primary} {failure} {LIMIT_REPS[group]}\n")


def tables(args, meta, frows, lrows, cases):
    """The markdown of the results, for docs/BENCHMARK.md."""
    out = []
    w = out.append
    w("## Time to finish\n")
    w("| Seed | Case | Goal | Dist | Java ms (min-max) | Rust ms (min-max) | Java/Rust | By round | Nodes | Nodes/s Java | Nodes/s Rust | Same path |")
    w("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for r in frows:
        if not r["finished"]:
            continue
        w(f"| {r['seed']} | {r['name']} | `{r['goal']}` | {r['dist']:.0f} | {r['java_ms']:.1f} ({r['java_min_ms']:.1f}-{r['java_max_ms']:.1f}) | "
          f"{r['rust_ms']:.1f} ({r['rust_min_ms']:.1f}-{r['rust_max_ms']:.1f}) | {r['ratio']:.2f} | {' / '.join(f'{x:.2f}' for x in r['round_ratios'])} | {r['nodes_java']} | {r['java_nps'] / 1e3:.0f}k | {r['rust_nps'] / 1e3:.0f}k | {'yes' if r['same_path'] else 'NO'} |")
    unfinished = [r for r in frows if not r["finished"]]
    if unfinished:
        w("\nNot finished on both sides inside the limit (the same search, stopped by the clock; one run per side). A search that did not finish has run for the whole limit; where only Rust finished, Java/Rust is at least the limit over Rust's time:\n")
        w("| Seed | Case | Goal | Dist | Java | Rust | Java/Rust |")
        w("|---|---|---|---|---|---|---|")
        for r in unfinished:
            jt = "finished" if r["java_type"] == "SUCCESS_TO_GOAL" else "not finished"
            rt = "finished" if r["rust_type"] == "SUCCESS_TO_GOAL" else "not finished"
            if r["java_type"] != "SUCCESS_TO_GOAL" and r["rust_type"] == "SUCCESS_TO_GOAL":
                bound = f"at least {r['ratio']:.2f}"
            elif r["java_type"] == "SUCCESS_TO_GOAL" and r["rust_type"] != "SUCCESS_TO_GOAL":
                bound = f"at most {r['ratio']:.2f}"
            else:
                bound = "-"
            w(f"| {r['seed']} | {r['name']} | `{r['goal']}` | {r['dist']:.0f} | {jt}, {r['java_ms'] / 1000:.1f} s, {r['nodes_java']} nodes | {rt}, {r['rust_ms'] / 1000:.1f} s, {r['nodes_rust']} nodes | {bound} |")
    fin = [r for r in frows if r["finished"]]
    w("\n### Summary (Java time / Rust time, above 1 means Rust was faster)\n")
    w("| Group | Cases | Median | Quartiles | Min | Max | Rust slower |")
    w("|---|---|---|---|---|---|---|")
    for label, rows in [("all", fin)] + [(n, [r for r in fin if r["name"] == n]) for n in ("near", "far", "xz")]:
        s = summarize([r["ratio"] for r in rows])
        if s:
            w(f"| {label} | {s['n']} | {s['median']:.2f} | {s['q1']:.2f}-{s['q3']:.2f} | {s['min']:.2f} | {s['max']:.2f} | {s['rust_slower']} |")
    for k in range(max((len(r["round_ratios"]) for r in fin), default=0)):
        s = summarize([r["round_ratios"][k] for r in fin if len(r["round_ratios"]) > k])
        w(f"| round {k + 1} only | {s['n']} | {s['median']:.2f} | {s['q1']:.2f}-{s['q3']:.2f} | {s['min']:.2f} | {s['max']:.2f} | {s['rust_slower']} |")
    sr = seed_ratios(frows)
    s = summarize(list(sr.values()))
    if s:
        w(f"| per seed (finished cases of a seed added up) | {s['n']} | {s['median']:.2f} | {s['q1']:.2f}-{s['q3']:.2f} | {s['min']:.2f} | {s['max']:.2f} | {s['rust_slower']} |")
    jn = [r["java_nps"] for r in fin]
    rn = [r["rust_nps"] for r in fin]
    if jn:
        w(f"\nNodes per second, median over the finished cases: Java {med(jn) / 1e3:.0f}k (range {min(jn) / 1e3:.0f}k-{max(jn) / 1e3:.0f}k), Rust {med(rn) / 1e3:.0f}k (range {min(rn) / 1e3:.0f}k-{max(rn) / 1e3:.0f}k).")
    w("\n## Same time limit\n")
    for key in ("50", "500", "2000", "default"):
        rows = [r for r in lrows if ((r["primary"], r["failure"]) == DEFAULT_PAIR) == (key == "default") and (key == "default" or r["primary"] == int(key))]
        if not rows:
            continue
        label = "upstream's default limits (primary 500 ms, failure 2000 ms)" if key == "default" else f"{key} ms (primary = failure)"
        w(f"\n### Limit {label}\n")
        w("| Seed | Case | Dist | Java nodes | Rust nodes | Rust/Java nodes | Java closer by | Rust closer by | Java left | Rust left | Java h | Rust h |")
        w("|---|---|---|---|---|---|---|---|---|---|---|---|")
        for r in rows:
            j, rr = r["java"], r["rust"]
            w(f"| {r['seed']} | {r['name']} | {r['dist']:.0f} | {j['nodes']:.0f} | {rr['nodes']:.0f} | {r['node_ratio']:.2f} | {j['closer']:.0f} | {rr['closer']:.0f} | {j['rem']:.0f} | {rr['rem']:.0f} | {j['besth']:.0f} | {rr['besth']:.0f} |")
        nr = summarize([r["node_ratio"] for r in rows])
        cr = summarize([r["closer_ratio"] for r in rows if not math.isnan(r["closer_ratio"])])
        ahead = sum(1 for r in rows if r["rust"]["rem"] < r["java"]["rem"] - 0.5)
        behind = sum(1 for r in rows if r["rust"]["rem"] > r["java"]["rem"] + 0.5)
        w(f"\nRust/Java nodes: median {nr['median']:.2f}, range {nr['min']:.2f}-{nr['max']:.2f} over {nr['n']} searches. "
          + (f"Rust/Java blocks closer to the goal: median {cr['median']:.2f}, range {cr['min']:.2f}-{cr['max']:.2f}. " if cr else "")
          + f"Rust's best partial path ended nearer the goal in {ahead}, farther in {behind}, equal (within half a block) in {len(rows) - ahead - behind}.")
    return "\n".join(out) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    for n in ("upstream", "jdk", "scratch", "cpus"):
        ap.add_argument("--" + n)
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--finish-ms", type=int, default=10000)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--rounds", type=int, default=2, help="how many times the time-to-finish phase is run, Java then Rust each time")
    ap.add_argument("--no-report", action="store_true")
    ap.add_argument("--wait-only", action="store_true")
    ap.add_argument("--tables-from", metavar="SCRATCH", help="only rewrite SCRATCH/tables.md from SCRATCH/results.json with the current code")
    ap.add_argument("--report", help="write the report here instead of reports/bench-<branch>.json")
    a = ap.parse_args(argv)
    get = lambda v, env: v or os.environ.get(env)
    t_start = time.time()

    def log(msg):
        print(f"[{int(time.time() - t_start):5d} s] {msg}", flush=True)

    try:
        if a.tables_from:
            with open(os.path.join(a.tables_from, "results.json"), encoding="utf-8") as f:
                res = json.load(f)
            with open(os.path.join(a.tables_from, "tables.md"), "w", encoding="utf-8") as f:
                f.write(tables(a, res["meta"], res["finish"], res["limit"], read_cases(os.path.join(a.tables_from, "bench"))))
            return 0
        if a.wait_only:
            log("waited for: " + (", ".join(wait_idle(log)) or "nothing"))
            return 0
        upstream, jdk, scratch = (get(getattr(a, n), "BARITONE_" + n.upper()) for n in ("upstream", "jdk", "scratch"))
        for n, v in (("upstream", upstream), ("jdk", jdk), ("scratch", scratch)):
            if not v:
                raise Fail(f"missing --{n} (or the BARITONE_{n.upper()} environment variable)")
        upstream, jdk, scratch = map(os.path.abspath, (upstream, jdk, scratch))
        branch = rc.read_minecraft_version(upstream)
        cpus = a.cpus or default_cpus()
        os.makedirs(scratch, exist_ok=True)
        outdir = os.path.join(scratch, "bench")
        shutil.rmtree(outdir, ignore_errors=True)
        os.makedirs(outdir)
        exe = build_rust(log)
        cp = bootstrap_java(upstream, jdk, scratch, log)
        waited = wait_idle(log)
        quiet = wait_quiet_cpu(log)
        log(f"machine idle (waited for: {', '.join(waited) or 'nothing'}; quiet CPUs: {'yes' if quiet else 'no, gave up'}); load average {loadavg()}")
        java = [os.path.join(jdk, "bin", "java")] + JAVA_FLAGS + ["-cp", cp, JAVA_MAIN]
        seeds = ",".join(str(i) for i in range(1, a.seeds + 1))
        phases = {}

        # A. time to finish, in rounds: Java then Rust, again, so that a drift of the machine (heat, clock) shows between rounds
        jrounds, rrounds = [], []
        for rnd in range(1, a.rounds + 1):
            tag = "" if rnd == 1 else str(rnd)
            log(f"A{rnd}. Java: time to finish")
            code, secs, mon = run_side(f"java-finish{tag}", java + ["mode=finish", f"out={outdir}", f"seeds={seeds}", f"write={1 if rnd == 1 else 0}", f"finishms={a.finish_ms}", f"reps={a.reps}", f"tag={tag}"], cpus, outdir, log)
            phases[f"java_finish_{rnd}"] = dict(seconds=round(secs), **mon)
            if code:
                raise Fail("the Java finish run failed; see " + os.path.join(outdir, f"java-finish{tag}.log"))
            log(f"A{rnd}. Rust: time to finish")
            code, secs, mon = run_side(f"rust-finish{tag}", [exe, "finish", outdir, tag], cpus, outdir, log)
            phases[f"rust_finish_{rnd}"] = dict(seconds=round(secs), **mon)
            if code:
                raise Fail("the Rust finish run failed; see " + os.path.join(outdir, f"rust-finish{tag}.log"))
            jw, jr = read_results(os.path.join(outdir, f"java-finish{tag}.txt"))
            rw, rr = read_results(os.path.join(outdir, f"rust-finish{tag}.txt"))
            if jw != rw:
                raise Fail("the two sides read different worlds: " + str({s: (jw.get(s), rw.get(s)) for s in set(jw) | set(rw) if jw.get(s) != rw.get(s)}))
            jrounds.append({t["id"]: t for t in jr})
            rrounds.append({t["id"]: t for t in rr})
        cases = read_cases(outdir)
        frows = compare_finish(merge_rounds(jrounds), merge_rounds(rrounds), cases)

        # B. same time limit
        picks, taken = [], set()
        for key in ("50", "500", "2000", "default"):
            lim = DEFAULT_PAIR[1] if key == "default" else int(key)
            ids = pick_limit_cases(frows, cases, lim, LIMIT_CASES[key], a.finish_ms)
            if len(ids) < LIMIT_CASES[key]:
                # top up with the "deep" cases (a goal 24 blocks under the surface, reachable only by digging), which are not in the finish run
                deep = sorted((c for c in cases if cases[c]["stress"]), key=lambda c: (cases[c]["seed"], c))
                ids += deep[: LIMIT_CASES[key] - len(ids)]
            picks.append((key, ids))
        listfile = os.path.join(outdir, "limit-list.txt")
        write_limit_list(listfile, picks)
        log("B. Java: same time limit, %d searches" % sum(len(i) for _, i in picks))
        code, secs, mon = run_side("java-limit", java + ["mode=limit", f"out={outdir}", f"list={listfile}"], cpus, outdir, log)
        phases["java_limit"] = dict(seconds=round(secs), **mon)
        if code:
            raise Fail("the Java limit run failed; see " + os.path.join(outdir, "java-limit.log"))
        log("B. Rust: same time limit")
        code, secs, mon = run_side("rust-limit", [exe, "limit", outdir, listfile], cpus, outdir, log)
        phases["rust_limit"] = dict(seconds=round(secs), **mon)
        if code:
            raise Fail("the Rust limit run failed; see " + os.path.join(outdir, "rust-limit.log"))
        _, jl = read_results(os.path.join(outdir, "java-limit.txt"))
        _, rl = read_results(os.path.join(outdir, "rust-limit.txt"))
        lrows = compare_limit(jl, rl, cases)

        # results
        fin = [r for r in frows if r["finished"]]
        meta = {
            "cpu": cpu_model(), "cpus_pinned": cpus, "kernel": platform.release(), "rustc": rc.run(["rustc", "--version"]).stdout.strip(),
            "java": (lambda p: (p.stderr or p.stdout).splitlines()[0].strip())(rc.run([os.path.join(jdk, "bin", "java"), "-version"])),
            "java_flags": JAVA_FLAGS, "phases": phases, "waited_for": waited, "quiet_cpus_before_start": bool(quiet),
        }
        with open(os.path.join(scratch, "results.json"), "w", encoding="utf-8") as f:
            json.dump({"meta": meta, "finish": frows, "limit": lrows, "picks": picks}, f, indent=1)
        with open(os.path.join(scratch, "tables.md"), "w", encoding="utf-8") as f:
            f.write(tables(a, meta, frows, lrows, cases))
        summ = summarize([r["ratio"] for r in fin])
        sr = summarize(list(seed_ratios(frows).values()))
        log(f"finish: {len(frows)} cases, {len(fin)} finished on both sides, {sum(1 for r in fin if r['same_path'])} with the same path")
        if summ:
            log(f"median Java/Rust {summ['median']:.2f}, range {summ['min']:.2f}-{summ['max']:.2f}; per seed median {sr['median']:.2f}, range {sr['min']:.2f}-{sr['max']:.2f}")
        intruders = {k for p in phases.values() for k in p["intruders"]}
        log("disturbing processes seen during the timed phases: " + (", ".join(sorted(intruders)) or "none"))
        if not a.no_report:
            report = {
                "tool": "bench",
                "upstream_branch": branch,
                "upstream_commit": rc.upstream_commit(upstream),
                "minecraft_version": branch,
                "java": meta["java"],
                "rustc": meta["rustc"],
                "machine": {"cpu": meta["cpu"], "cpus_pinned": cpus, "kernel": meta["kernel"]},
                "java_flags": JAVA_FLAGS,
                "parameters": {"seeds": a.seeds, "finish_ms": a.finish_ms, "reps": a.reps, "rounds": a.rounds, "limits_ms": list(LIMITS), "default_pair_ms": list(DEFAULT_PAIR),
                               "limit_cases": LIMIT_CASES, "limit_reps": LIMIT_REPS, "config": 0, "search_settings": 0},
                "sources": {rel: rc.sha256_file(os.path.join(REPO, *rel.split("/"))) for rel in sorted(source_files())},
                "counts": {
                    "worlds": len(jw),
                    "finish_cases": len(frows),
                    "finished_on_both_sides": len(fin),
                    "unfinished_inside_limit": len(frows) - len(fin),
                    "same_search_counters": sum(1 for r in frows if r["same_search"]),
                    "same_path": sum(1 for r in fin if r["same_path"]),
                    "different_path": [r["id"] for r in fin if not r["same_path"]],
                    "rust_slower_cases": sum(1 for r in fin if r["ratio"] < 1.0),
                    "limit_searches": len(lrows),
                },
                "disturbing_processes_seen": sorted(intruders),
                "waited_for": waited,
                "quiet_cpus_before_start": bool(quiet),
            }
            path = a.report or os.path.join(REPO, "reports", f"bench-{branch}.json")
            rc.write_report(report, path)
            log("wrote " + os.path.relpath(path, REPO))
        log(f"tables in {os.path.join(scratch, 'tables.md')}, raw results in {os.path.join(scratch, 'results.json')}")
    except Fail as e:
        print("FAIL:", e, file=sys.stderr)
        return 1
    return 0


def source_files():
    out = ["tools/run_bench.py", "tools/bench/Cargo.toml", "tools/bench/src/main.rs", "tools/bench/java/baritone/pathing/calc/SearchBench.java",
           "tools/search-compare/src/parse.rs", "mod/engine/Cargo.toml"]
    for d, _, names in os.walk(os.path.join(REPO, "mod", "engine", "src")):
        for n in names:
            if n.endswith(".rs"):
                out.append(os.path.relpath(os.path.join(d, n), REPO).replace(os.sep, "/"))
    return out


if __name__ == "__main__":
    sys.exit(main())
