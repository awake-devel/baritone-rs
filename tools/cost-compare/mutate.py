# Injects one deliberate error at a time into the spike and checks that the comparison catches it.
# Usage: mutate.py CRATE_DIR GOLDEN_FILE ["name|name|..."]
import os, re, shutil, subprocess, sys, tempfile

SRC = os.path.abspath(sys.argv[1])
GOLD = os.path.abspath(sys.argv[2])
ROOT = tempfile.mkdtemp(prefix="cost-compare-")
TMP = f"{ROOT}/mutant"
M = [
    ("pow: powi instead of powf", "0.98f64.powf(ticks as f64)", "0.98f64.powi(ticks)"),
    ("float: 1-mult widened too early", "((1.0f32 - mult) as f64)", "(1.0f64 - mult as f64)"),
    ("swim: ignore allowSwimming", "if cfg.allow_swimming && cfg.can_sprint {", "if cfg.can_sprint {"),
    ("through: cobweb passable", '|| b == "cobweb" || b == "end_portal"', '|| b == "end_portal"'),
    ("through: skull passable", "|| s.skull ||", "||"),
    ("through: iron door passable", "if s.wooden_door { Yes } else { No }", "Yes"),
    ("through: gate blocked", "if s.gate {\n        return Yes;", "if s.gate {\n        return No;"),
    ("through: carpet always yes", "if s.carpet {\n        return Maybe;", "if s.carpet {\n        return Yes;"),
    ("through: tripwire not avoided", 'b == "tripwire"', 'b == "nothing"'),
    ("through: cauldron passable", "if s.cauldron {\n        return No;", "if s.cauldron {\n        return Yes;"),
    ("through: partial fluid maybe", "if s.fluid_amount != 8 { No } else { Maybe }", "Maybe"),
    ("walkon: magma always ok", '(b != "magma_block" || c.allow_walk_on_magma)', "true"),
    ("walkon: honey ok", '&& b != "honey_block"', ""),
    ("walkon: vines always", '(is_climbable(b) && c.allow_vines)', "is_climbable(b)"),
    ("walkon: chest no", 'b == "ender_chest" || b == "chest" || b == "trapped_chest"', 'b == "ender_chest"'),
    ("walkon: stair no", "if s.stair {\n        return Yes;", "if s.stair {\n        return No;"),
    ("walkon: bottom slab setting", "if !c.allow_walk_on_bottom_slab {", "if false {"),
    ("snow: 3 layers becomes 4", "if s.layers >= 3 {", "if s.layers >= 4 {"),
    ("water pos: xor to or", "is_water(up) ^ self.cfg.assume_walk_on_water", "is_water(up) | self.cfg.assume_walk_on_water"),
    ("water pos: lily pad", 'up.block == "lily_pad" || up.carpet', "up.carpet"),
    ("water through: lily above", "!up.fluid_empty || up.lily", "!up.fluid_empty"),
    ("flowing: skip neighbours", "possibly_flowing(self.get(x + 1, y, z)) || possibly_flowing(self.get(x - 1, y, z)) || possibly_flowing(self.get(x, y, z + 1)) || possibly_flowing(self.get(x, y, z - 1))", "false"),
    ("adjacent: falling ignored", "if !directly_above && s.falling && self.cfg.avoid_updating_falling", "if false && s.falling"),
    ("adjacent: strict liquid ignored", "if directly_above || self.cfg.strict_liquid_check {", "if directly_above {"),
    ("adjacent: source level", "if s.level == 0 {\n                return true;", "if s.level == 1 {\n                return true;"),
    ("adjacent: skip east", "|| self.avoid_adjacent_breaking(x + 1, y, z, false)\n", "\n"),
    ("break: ice allowed", 's.block == "ice" || s.infested', "s.infested"),
    ("break: extra penalty dropped", "result += self.cfg.break_penalty;", ""),
    ("break: falling stack ignored", "if above.falling {", "if false {"),
    ("break: wrong tool table", "if self.cfg.pick { s.str_pick } else { s.str_hand }", "s.str_pick"),
    ("break: allowBreak ignored", "if !self.cfg.allow_break {", "if false {"),
    ("frost: level ignored", "s.frost_melt && s.level == 0", "s.frost_melt"),
    ("solid: top slab", 'if s.slab_type != "BOTTOM" {\n                    return true;', 'if false {\n                    return true;'),
    ("solid: stair shape", 's.stair_shape == "INNER_LEFT" || s.stair_shape == "INNER_RIGHT"', "false"),
    ("solid: trapdoor", "!s.trap_open && s.trap_half == \"TOP\"", "s.trap_half == \"TOP\""),
    ("solid: liquid above", "if self.get(x, y + 1, z).liquid_block {\n                return false;", "if false {\n                return false;"),
    ("replace: snow any layer", "return s.layers == 1;", "return true;"),
    ("replace: tall grass override", 's.block == "large_fern" || s.block == "tall_grass"', "false"),
    ("place: source setting", "!self.cfg.allow_place_source && current.fluid_source", "false"),
    ("place: flow setting", "!self.cfg.allow_place_flow && !current.fluid_empty && !current.fluid_source", "false"),
    ("traverse: sprint in water", "if !water && !sneaking && self.cfg.can_sprint {", "if !sneaking && self.cfg.can_sprint {"),
    ("traverse: sprint when sneaking", "if !water && !sneaking && self.cfg.can_sprint {", "if !water && self.cfg.can_sprint {"),
    ("traverse: soul sand dest", 'if dest_on.block == "soul_sand" {', 'if false {'),
    ("traverse: water walk penalty", "wc += self.cfg.walk_on_water_penalty;", ""),
    ("traverse: magma sneak", '&& src_down_block == "magma_block" {', '&& false {'),
    ("traverse: ladder x5 removed", "hardness1 *= 5.0;\n                hardness2 *= 5.0;", ""),
    ("traverse: bridge in water check", "if is_water(dest_on) && through_water {", "if false {"),
    ("traverse: backplace check removed", "if against_x == x && against_z == z {\n                        continue;\n                    }", ""),
    ("traverse: down direction dropped", ", (0, -1, 0)]", "]"),
    ("traverse: slab backplace", '(src_down.slab && src_down.slab_type != "DOUBLE")', "false"),
    ("traverse: standing check", "if !standing_on_a_block {\n                    return COST_INF;", "if false {\n                    return COST_INF;"),
    ("traverse: lily backplace", '(block_src.block == "lily_pad" || block_src.carpet) && !src_down.fluid_empty', "false"),
    ("traverse: sneak multiplier", "wc *= SNEAK_ONE_BLOCK_COST / WALK_ONE_BLOCK_COST;", ""),
    ("descend: climbable start", "if is_climbable(from_down) {\n            return;", "if false {\n            return;"),
    ("descend: ladder dest", 'dest_down.block == "ladder" || dest_down.block == "vine"', "false"),
    ("descend: frost", "if self.can_use_frost_walker(dest_down) {", "if false {"),
    ("descend: soul sand", 'if from_down == "soul_sand" {', "if false {"),
    ("descend: max to min", "fall_n_blocks_cost()[1].max(CENTER_AFTER_FALL_COST)", "fall_n_blocks_cost()[1].min(CENTER_AFTER_FALL_COST)"),
    ("fall: falling above", "if front_break != 0.0 && self.get(dest_x, y + 2, dest_z).falling {", "if false {"),
    ("fall: void check", "if new_y < self.world.min_y {", "if new_y < self.world.min_y - 1 {"),
    ("fall: flowing water", "if self.is_flowing(dest_x, new_y, dest_z, onto) {", "if false {"),
    ("fall: floor under water", "if !self.can_walk_on_at(dest_x, new_y - 1, dest_z) {", "if false {"),
    ("fall: climb reset", "if unprotected <= 11 && is_climbable(&onto.block) {", "if false {"),
    ("fall: bottom slab", "if is_bottom_slab(onto) {", "if false {"),
    ("fall: off by one", "unprotected <= self.cfg.max_fall_no_water + 1", "unprotected <= self.cfg.max_fall_no_water"),
    ("fall: bucket cost", "res.cost = tentative + self.cfg.place_penalty;", "res.cost = tentative;"),
]
if len(sys.argv) > 3:
    keep = set(sys.argv[3].split("|"))
    M = [m for m in M if m[0] in keep]
os.makedirs(TMP, exist_ok=True)
env = dict(os.environ, CARGO_TARGET_DIR=f"{ROOT}/target")
orig = open(f"{SRC}/src/lib.rs").read()
caught, missed, bad = [], [], []
for name, a, b in M:
    if orig.count(a) < 1:
        bad.append(name)
        continue
    shutil.rmtree(TMP, ignore_errors=True)
    shutil.copytree(SRC, TMP, ignore=shutil.ignore_patterns("target"))
    open(f"{TMP}/src/lib.rs", "w").write(orig.replace(a, b, 1))
    c = subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=TMP, env=env, capture_output=True, text=True)
    if c.returncode:
        bad.append(name + " (no build: " + c.stderr[:200] + ")")
        continue
    r = subprocess.run([f"{ROOT}/target/release/baritone-spike", GOLD, "quiet"], capture_output=True, text=True).stdout
    wrong = sum(map(int, re.findall(r"WRONG (\d+)", r)))
    close = sum(map(int, re.findall(r"within 1e-9 (\d+)", r)))
    print(f"{'CAUGHT ' if wrong or close else 'MISSED '} wrong={wrong:6d} close={close:6d} {name}", flush=True)
    (caught if wrong or close else missed).append(name)
shutil.rmtree(ROOT, ignore_errors=True)
print(f"\n{len(caught)} caught, {len(missed)} missed, {len(bad)} not applied")
print("missed:", missed)
print("not applied:", bad)
