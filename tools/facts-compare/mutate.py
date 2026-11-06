#!/usr/bin/env python3
"""Injects errors into mod/glue/state_facts.rs and checks that facts-compare fails on each. A file that doesn't build fails
the run, because that error tested nothing. The file is always restored.

  python3 tools/facts-compare/mutate.py RESOURCES JAVA_DUMP

Exit: 0 every error was caught or is listed in KNOWN_NO_EFFECT, 1 otherwise.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
GLUE = os.path.join(HERE, "..", "..", "mod", "glue", "state_facts.rs")
EXPECTED = os.path.join(HERE, "expected-differences.txt")

# Rules that make a small, plausible error at each match, one per mutation.
RULES = [
    (r'has\(f, "(\w+)"\)', lambda m: f'has(f, "{m.group(1)}Renamed")'),
    (r'Some\("(\w+)"\)', lambda m: 'Some("nothing")'),
    (r' != 0', lambda m: ' == 0'),
    (r' && ', lambda m: ' || '),
    (r' \|\| ', lambda m: ' && '),
    (r'!src\.behaviours', lambda m: 'src.behaviours'),
    (r'state_flags::(\w+)', lambda m: 'state_flags::SOLID'),
    (r'Direction::Up\)', lambda m: 'Direction::Down)'),
    (r'"minecraft:(fire|water)"', lambda m: '"minecraft:nothing"'),
    (r'"minecraft:(incorrect_for_diamond_tool|mineable/pickaxe)"', lambda m: '"minecraft:nothing"'),
    (r'prop\("(\w+)"\)', lambda m: 'prop("nothing")'),
    (r'on\("(\w+)"\)', lambda m: 'on("nothing")'),
    (r'Fluid::(Water|FlowingWater|Lava|FlowingLava),', lambda m: 'Fluid::None,'),
    (r'SlabType::(Top|Double),', lambda m: 'SlabType::Bottom,'),
    (r'StairShape::(InnerLeft|InnerRight|OuterLeft|OuterRight),', lambda m: 'StairShape::Straight,'),
    (r'Direction::(South|East|West|Up|Down),', lambda m: 'Direction::North,'),
    (r'Half::Top \}', lambda m: 'Half::Bottom }'),
    (r'8\.0 \} else \{ 1\.0', lambda m: '6.0 } else { 1.0'),
    (r'speed / 30\.0', lambda m: 'speed / 100.0'),
    (r'speed / 100\.0', lambda m: 'speed / 30.0'),
    (r'f\.hardness < 0\.0', lambda m: 'f.hardness <= 0.0'),
    (r'return -1\.0', lambda m: 'return 1.0'),
    (r'\.unwrap_or\(id\)', lambda m: '.map(|_| id).unwrap_or(id)'),
    (r'== Tool::DiamondPickaxe', lambda m: '== Tool::EmptyHand'),
    (r'tool == Tool::DiamondPickaxe && ', lambda m: ''),
    (r'mineable && !denied', lambda m: 'mineable'),
    (r'!f\.requires_tool', lambda m: 'f.requires_tool'),
    (r'== Some\(id\)', lambda m: '!= Some(id)'),
    (r'!= FluidKind::Empty', lambda m: '== FluidKind::Empty'),
    (r'== FluidKind::Empty', lambda m: '!= FluidKind::Empty'),
    (r'is_source\(\)', lambda m: 'is_full()'),
    (r'pathfindable: land', lambda m: 'pathfindable: !land'),
    (r't\.can_open_by_hand', lambda m: '!t.can_open_by_hand'),
    (r'on\("open"\)', lambda m: 'on("powered")'),
]

# (text of the source line, the matched text): errors with no possible effect on vanilla's data, and why.
KNOWN_NO_EFFECT = {
    ('in_tag("minecraft:fire")', '"minecraft:fire"'): "every block in the fire tag is also replaceable, so the replaceable flag already answers",
    ("incorrect_for_diamond_tool", '"minecraft:incorrect_for_diamond_tool"'): "vanilla's tag is empty",
    ("incorrect_for_diamond_tool", "== Tool::DiamondPickaxe"): "vanilla's tag is empty",
    ("incorrect_for_diamond_tool", "tool == Tool::DiamondPickaxe && "): "vanilla's tag is empty",
    ("mineable && !denied", "mineable && !denied"): "vanilla's tag is empty",
    ("default_state(&desc.name)", ".unwrap_or(id)"): "no block's hardness or requires_tool differs between its states",
}


def mutations(text):
    seen = set()
    for pattern, make in RULES:
        for m in re.finditer(pattern, text):
            new = make(m)
            if new is None or (m.start(), new) in seen:
                continue
            seen.add((m.start(), new))
            line = text.count("\n", 0, m.start()) + 1
            source = text.splitlines()[line - 1]
            yield line, m.group(0), new, text[: m.start()] + new + text[m.end() :], source


def main():
    resources, dump = sys.argv[1], sys.argv[2]
    env = dict(os.environ, CARGO_BUILD_JOBS="6")
    manifest = os.path.join(HERE, "Cargo.toml")
    with open(GLUE, encoding="utf-8") as f:
        original = f.read()
    all_mutations = list(mutations(original))
    caught, missed, nobuild = 0, [], []
    try:
        for i, (line, was, now, text, source) in enumerate(all_mutations):
            with open(GLUE, "w", encoding="utf-8") as f:
                f.write(text)
            build = subprocess.run(["nice", "-n", "15", "cargo", "build", "--release", "--quiet", "--manifest-path", manifest], env=env, capture_output=True)
            if build.returncode != 0:
                nobuild.append((line, was, now, source))
                continue
            binary = os.path.join(HERE, "target", "release", "facts-compare")
            run = subprocess.run(["nice", "-n", "15", binary, resources, dump, EXPECTED], capture_output=True)
            if run.returncode != 0:
                caught += 1
            else:
                missed.append((line, was, now, source))
            print(f"{i + 1}/{len(all_mutations)} line {line}: {was} -> {now}: {'caught' if run.returncode else 'MISSED'}", flush=True)
    finally:
        with open(GLUE, "w", encoding="utf-8") as f:
            f.write(original)
    listed = lambda m: any(frag in m[3] and m[1] == was for frag, was in KNOWN_NO_EFFECT)
    unlisted = [m for m in missed if not listed(m)]
    print(f"{len(all_mutations)} errors: {caught} caught, {len(missed)} missed ({len(unlisted)} unlisted), {len(nobuild)} did not build")
    for m in missed:
        print("missed", m[:3], "" if listed(m) else "NOT LISTED")
    for m in nobuild:
        print("no build", m)
    return 1 if unlisted or nobuild else 0


if __name__ == "__main__":
    sys.exit(main())
