#!/usr/bin/env python3
# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Turns the cost-compare harness (tools/cost-compare/java, written for upstream's 1.21.4 branch) into the variant for
upstream's 26.3 branch, at run time. There is one Java source; this file is the list of everything that differs.

Every entry of REPLACEMENTS must match exactly `count` times and every entry of REGIONS exactly once, or the script fails
loudly: a change of the Java source that this table does not follow can never go through unnoticed. The parts that cannot
be a replacement (building the context and the world by reflection, binding the item components, a stand-in for a method
26.3 lacks) are in port-26.3/baritone/pathing/movement/Compat263.java, which is copied next to the ported test.

  python3 tools/cost-compare/port_26_3.py OUT_DIR    write the ported test files under OUT_DIR (for looking at them)
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
JAVA = os.path.join(HERE, "java")
EXTRA = os.path.join(HERE, "port-26.3")
DUMP = "baritone/pathing/movement/SpikeGoldenDump.java"

# (name, old text, new text, how many times old must occur)
REPLACEMENTS = [
    ("ResourceLocation was renamed Identifier: import",
     "import net.minecraft.resources.ResourceLocation;", "import net.minecraft.resources.Identifier;", 1),
    ("ResourceLocation was renamed Identifier: withDefaultNamespace",
     "ResourceLocation.withDefaultNamespace(", "Identifier.withDefaultNamespace(", 1),
    ("ResourceLocation was renamed Identifier: parse",
     "ResourceLocation.parse(", "Identifier.parse(", 1),
    ("WaterlilyBlock was renamed LilyPadBlock",
     "WaterlilyBlock", "LilyPadBlock", 1),
    ("the white glass is a member of Blocks.STAINED_GLASS",
     "Blocks.WHITE_STAINED_GLASS", "Blocks.STAINED_GLASS.white()", 2),
    ("the white carpet is a member of Blocks.CARPET",
     "Blocks.WHITE_CARPET", "Blocks.CARPET.white()", 6),
    ("26.3 has no allowSwimming setting; the configuration still writes the flag, upstream ignores it",
     "s.allowSwimming.value = c.swim;", "// 26.3 has no allowSwimming setting: c.swim is written to the file, but nothing applies it.", 1),
    ("26.3 has no isWaterloggedSolidTop; the registry sweep column keeps the 1.21.4 body (not an answer of 26.3)",
     "b(MovementHelper.isWaterloggedSolidTop(s))", "b(Compat263.isWaterloggedSolidTop(s))", 1),
    ("26.3 items need their component initializers run after the tags are bound",
     'System.out.println("bound fluid tags: " + bindTags(BuiltInRegistries.FLUID, Registries.FLUID, "fluid"));',
     'System.out.println("bound fluid tags: " + bindTags(BuiltInRegistries.FLUID, Registries.FLUID, "fluid"));\n'
     '        System.out.println("bound component sets: " + Compat263.bindItemComponents());', 1),
    ("26.3 has no test constructors: the world and the context come from Compat263",
     "Ctx ctx = new Ctx(new Grid(gridBorder, states, loaded), ", "CalculationContext ctx = Compat263.ctx(Compat263.grid(gridBorder, states, loaded), ", 1),
    ("the white concrete powder is a member of Blocks.CONCRETE_POWDER",
     "Blocks.WHITE_CONCRETE_POWDER", "Blocks.CONCRETE_POWDER.white()", 1),
    ("Blocks.CHAIN was split by metal; the iron one is the old chain",
     "Blocks.CHAIN,", "Blocks.IRON_CHAIN,", 1),
    ("copper doors and trapdoors come in weathering collections; the plain one is `weathering().unaffected()`",
     "Blocks.COPPER_DOOR}", "Blocks.COPPER_DOOR.weathering().unaffected()}", 1),
    ("copper trapdoor, see above",
     "Blocks.COPPER_TRAPDOOR.defaultBlockState()", "Blocks.COPPER_TRAPDOOR.weathering().unaffected().defaultBlockState()", 1),
    ("26.3 has no CalculationContext overload of canPlaceAgainst; the Baritone answer is the same",
     "MovementHelper.canPlaceAgainst(ctx, x, y, z)", "MovementHelper.canPlaceAgainst(ctx.bsi, x, y, z)", 1),
    ("26.3 has no CalculationContext overload of avoidBreaking (1.21.4 reads a precomputed flag, 26.3 the settings list)",
     "MovementHelper.avoidBreaking(ctx, x, y, z, st)", "MovementHelper.avoidBreaking(ctx.bsi, x, y, z, st)", 1),
    ("the white bed is a member of Blocks.BED; blocks new in 26.3 join the palette, at the end so that no earlier index moves",
     "Blocks.WHITE_BED}) {", "Blocks.BED.white(), Blocks.COPPER_CHEST.weathering().unaffected(), Blocks.SULFUR_SPIKE, Blocks.OAK_SHELF}) {", 1),
]

# (name, text where the region starts, text where it ends (not included), replacement). Each anchor must occur once.
REGIONS = [
    ("Grid extends the 1.21.4 test constructor of BlockStateInterface; Compat263.Grid replaces it",
     "    static final class Grid extends BlockStateInterface {\n", "    static final class Tools extends ToolSet {\n", ""),
    ("Ctx extends the protected 1.21.4 constructor of CalculationContext; Compat263.ctx replaces it",
     "    static final class Ctx extends CalculationContext {\n",
     "    // ------------------------------------------------------------------------------------------------------------\n"
     "    // Configurations:", ""),
]


class PortError(Exception):
    pass


def port_text(text):
    for name, old, new, count in REPLACEMENTS:
        n = text.count(old)
        if n != count:
            raise PortError(f"replacement '{name}': expected {count} match(es), found {n}")
        text = text.replace(old, new)
    for name, start, end, new in REGIONS:
        for label, anchor in (("start", start), ("end", end)):
            n = text.count(anchor)
            if n != 1:
                raise PortError(f"region '{name}': the {label} anchor occurs {n} times, expected once")
        a = text.index(start)
        b = text.index(end)
        if b <= a:
            raise PortError(f"region '{name}': the end comes before the start")
        text = text[:a] + new + text[b:]
    return text


def read_tree(base):
    out = {}
    for d, _, names in os.walk(base):
        for n in names:
            path = os.path.join(d, n)
            rel = os.path.relpath(path, base).replace(os.sep, "/")
            with open(path, encoding="utf-8") as f:
                out[rel] = f.read()
    return out


def ported_files():
    """{relative path under src/test/java: text} for the 26.3 variant."""
    files = read_tree(JAVA)
    if DUMP not in files:
        raise PortError("the harness has no " + DUMP)
    files[DUMP] = port_text(files[DUMP])
    for rel, text in read_tree(EXTRA).items():
        if rel in files:
            raise PortError("the extra file " + rel + " would replace a harness file")
        files[rel] = text
    return files


def source_files():
    """Paths of the files the variant is made from, relative to this directory (for hashing)."""
    return ["port_26_3.py"] + ["port-26.3/" + r for r in sorted(read_tree(EXTRA))]


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        files = ported_files()
    except PortError as e:
        print("FAIL:", e, file=sys.stderr)
        return 1
    for rel, text in files.items():
        dest = os.path.join(argv[1], *rel.split("/"))
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "w", encoding="utf-8") as f:
            f.write(text)
    print("wrote", len(files), "files")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
