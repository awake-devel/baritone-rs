#!/usr/bin/env python3
"""L1: compares what Horizon says about every block state with what Java Minecraft says.

Steps: copy FactsDump.java into a checkout of upstream's 26.3 branch, run it with Gradle to write the dump, remove it again,
then run tools/facts-compare against a Horizon version's prepared resources. The dump stays in the scratch folder.

  python3 tools/run_facts.py --upstream upstream/baritone --jdk DIR --scratch DIR \
      --horizon upstream/horizon-nightly --resources DIR

With --versions-against, it also dumps a checkout of upstream's 1.21.4 branch and lists what changed in the game's facts
between the two versions, because the rules were tuned on 1.21.4. That needs --jdk-old.

The report is written only after the comparison and the injected errors pass, so a report on disk means both passed.

Exit: 0 passed, 1 a difference is not explained or an injected error was missed, 2 the tool could not run.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
TOOL = os.path.join(HERE, "facts-compare")
JAVA = os.path.join(TOOL, "java", "baritone", "FactsDump.java")
EXPECTED = os.path.join(TOOL, "expected-differences.txt")
REPORT = os.path.join(REPO, "reports", "facts-compare-26.3.json")
MUTATE = os.path.join(TOOL, "mutate.py")
INPUTS = (
    "mod/glue/state_facts.rs",
    "tools/facts-compare/Cargo.lock",
    "tools/facts-compare/Cargo.toml",
    "tools/facts-compare/expected-differences.txt",
    "tools/facts-compare/java/baritone/FactsDump.java",
    "tools/facts-compare/mutate.py",
    "tools/facts-compare/src/main.rs",
    "tools/run_facts.py",
)
ENGINE = os.path.join(REPO, "mod", "engine", "src")
EXPORTS = ("facts.json", "gameplay-facts.json", "block-behaviours.json", "block-tags.json", "mob-data.json")
TEST = "baritone.FactsDump"


class Fail(Exception):
    pass


def for_old_game(text):
    """FactsDump.java is written against 26.3. These are the API differences in 1.21.4."""
    text = re.sub(r"[ \t]*//@26\.3-begin.*?//@26\.3-end\n", "", text, flags=re.S)
    text = text.replace("net.minecraft.resources.Identifier", "net.minecraft.resources.ResourceLocation")
    text = text.replace("Identifier.withDefaultNamespace", "ResourceLocation.withDefaultNamespace")
    text = text.replace("Identifier.parse", "ResourceLocation.parse")
    return text.replace("LilyPadBlock", "WaterlilyBlock")


def git_head(path):
    r = subprocess.run(["git", "-C", path, "rev-parse", "HEAD"], capture_output=True, text=True)
    if r.returncode != 0:
        raise Fail(f"{path} is not a git checkout")
    return r.stdout.strip()


def game_version(upstream):
    with open(os.path.join(upstream, "gradle.properties"), encoding="utf-8") as f:
        for line in f:
            if line.startswith("minecraft_version="):
                return line.split("=", 1)[1].strip()
    raise Fail("no minecraft_version in gradle.properties")


def dump(upstream, jdk, out, old):
    """Writes the dump of one upstream checkout. The test file is always removed again."""
    upstream, jdk, out = map(os.path.abspath, (upstream, jdk, out))
    dest = os.path.join(upstream, "src", "test", "java", "baritone", "FactsDump.java")
    if os.path.exists(dest):
        raise Fail("refusing to overwrite an existing upstream file: " + dest)
    created = not os.path.isdir(os.path.dirname(dest))
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with open(JAVA, encoding="utf-8") as f:
        text = f.read()
    with open(dest, "w", encoding="utf-8") as f:
        f.write(for_old_game(text) if old else text)
    env = dict(os.environ, JAVA_HOME=jdk, FACTS_OUT=out)
    try:
        r = subprocess.run(
            ["./gradlew", "--no-daemon", "--console=plain", "cleanTest", "test", "--tests", TEST,
             f"-Porg.gradle.java.installations.paths={jdk}"],
            cwd=upstream, env=env, capture_output=True, text=True,
        )
    finally:
        os.remove(dest)
        if created:
            os.rmdir(os.path.dirname(dest))
    if r.returncode != 0 or not os.path.exists(out):
        raise Fail("the Java dump failed:\n" + (r.stdout + r.stderr)[-4000:])


def read_dump(path):
    states = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.split()
            states[p[2]] = dict(kv.split("=", 1) for kv in p[3:])
    return states


def compare_versions(old_path, new_path):
    """What changed in the game's facts between the two versions. Prints it and returns the counts."""
    old, new = read_dump(old_path), read_dump(new_path)
    blocks = lambda states: {d.split("[")[0] for d in states}
    changed = {}
    for desc in old.keys() & new.keys():
        for k, v in old[desc].items():
            if k.startswith("info_"):
                continue
            if new[desc].get(k) != v:
                changed.setdefault((k, desc.split("[")[0]), []).append((v, new[desc].get(k)))
    for (k, b), vals in sorted(changed.items()):
        print(f"  {k} {b}: {len(vals)} states, 1.21.4={vals[0][0]} 26.3={vals[0][1]}")
    gone, added = sorted(blocks(old) - blocks(new)), sorted(blocks(new) - blocks(old))
    print(f"  common states {len(old.keys() & new.keys())}; blocks only in 1.21.4: {len(gone)}; only in 26.3: {len(added)}")
    if gone:
        print("  only in 1.21.4: " + " ".join(g.replace("minecraft:", "") for g in gone))
    return {"common_states": len(old.keys() & new.keys()), "fact_changes": len(changed), "blocks_gone": len(gone), "blocks_added": len(added)}


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def inputs_hash(upstream, horizon, resources):
    """sha256 over everything a result depends on. If it matches the report's, nothing needs to run again."""
    h = hashlib.sha256()
    paths = [os.path.join(REPO, *rel.split("/")) for rel in INPUTS]
    for folder, _, names in sorted(os.walk(ENGINE)):
        paths += [os.path.join(folder, n) for n in sorted(names)]
    paths += [os.path.join(resources, name) for name in EXPORTS]
    for path in paths:
        h.update(sha(path).encode())
    h.update(git_head(upstream).encode())
    h.update(git_head(horizon).encode())
    return h.hexdigest()


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--upstream", required=True, help="checkout of upstream's 26.3 branch")
    ap.add_argument("--jdk", required=True, help="JDK 25 home")
    ap.add_argument("--scratch", required=True)
    ap.add_argument("--horizon", required=True, help="the Horizon checkout that tools/facts-compare builds against")
    ap.add_argument("--resources", required=True, help="that version's prepared .local/horizon-client/26.3")
    ap.add_argument("--versions-against", help="checkout of upstream's 1.21.4 branch")
    ap.add_argument("--jdk-old", help="JDK 21 home, for --versions-against")
    ap.add_argument("--reuse-dump", action="store_true", help="use the dump already in the scratch folder")
    ap.add_argument("--no-mutation", action="store_true", help="skip the injected errors; no report is written")
    args = ap.parse_args(argv)
    try:
        if os.path.exists(REPORT):
            os.remove(REPORT)
        os.makedirs(args.scratch, exist_ok=True)
        new_dump = os.path.join(args.scratch, "facts-26.3.txt")
        if not (args.reuse_dump and os.path.exists(new_dump)):
            dump(args.upstream, args.jdk, new_dump, old=False)
        versions = None
        if args.versions_against:
            if not args.jdk_old:
                raise Fail("--versions-against needs --jdk-old")
            old_dump = os.path.join(args.scratch, "facts-1.21.4.txt")
            if not (args.reuse_dump and os.path.exists(old_dump)):
                dump(args.versions_against, args.jdk_old, old_dump, old=True)
            print("Facts that changed between the game versions:")
            versions = compare_versions(old_dump, new_dump)
        with tempfile.TemporaryDirectory(dir=args.scratch) as d:
            part = os.path.join(d, "report.json")
            r = subprocess.run(
                ["cargo", "run", "--release", "--quiet", "--manifest-path", os.path.join(TOOL, "Cargo.toml"), "--",
                 args.resources, new_dump, EXPECTED, part],
                cwd=REPO,
            )
            if r.returncode not in (0, 1) or not os.path.exists(part):
                raise Fail(f"facts-compare exited with {r.returncode}")
            with open(part, encoding="utf-8") as f:
                result = json.load(f)
        if r.returncode != 0:
            print("FAILED: the facts differ in a way nobody has explained", file=sys.stderr)
            return 1
        if args.no_mutation:
            print("Comparison passed. No report: the injected errors did not run.")
            return 0
        m = subprocess.run([sys.executable, MUTATE, args.resources, new_dump], cwd=REPO, capture_output=True, text=True)
        tail = [ln for ln in m.stdout.splitlines() if not re.match(r"^\d+/\d+ line", ln)]
        print("\n".join(tail))
        if m.returncode != 0:
            print("FAILED: an injected error was missed\n" + m.stderr[-2000:], file=sys.stderr)
            return 1
        report = {
            "tool": "facts-compare",
            "inputs": inputs_hash(args.upstream, args.horizon, args.resources),
            "injected_errors": tail[0] if tail else "",
            "minecraft_version": game_version(args.upstream),
            "upstream_commit": git_head(args.upstream),
            "horizon_commit": git_head(args.horizon),
            "exports": {name: sha(os.path.join(args.resources, name)) for name in EXPORTS},
            "results": result,
            "version_changes": versions,
        }
        with open(REPORT, "w", encoding="utf-8") as f:
            json.dump(report, f, indent=1, sort_keys=True)
            f.write("\n")
        print("PASSED, report written")
        return 0
    except Fail as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
