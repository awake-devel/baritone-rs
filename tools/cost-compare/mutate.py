# baritone-rs test tool. LGPL-3.0-or-later, see LICENSE.
"""Injects one deliberate error at a time into a copy of the engine and checks that the comparison catches it.

Usage: mutate.py REPO GOLDEN_FILE ["name|name|..."]    run the mutations (all, or the named ones)
       mutate.py REPO --where                          list where each pattern matches, without building anything

The workspace parts the comparison needs (root Cargo.toml and Cargo.lock, rustfmt.toml, mod/engine, tools/cost-compare)
are copied into a temporary directory, once per worker; the repository is never touched. BARITONE_MUTATE_JOBS sets the number
of workers (default: half of the threads, or 2 while a game or another build is running). A mutation is CAUGHT when any family's
WRONG or close count differs from the unmutated run, or when the comparison aborts or runs over RUN_TIMEOUT seconds
(the unmutated one exits 0 in under a minute), MISSED otherwise, and 'not applied' when its pattern is not in the file or
the mutant does not build. Patterns must occur exactly once in their file (--where marks the ones that do not). gate.py reads the CAUGHT, MISSED and 'not applied' lines.

Each mutation is (file, pattern, replacement, name): the first occurrence of the pattern in the file (a path relative to the
repository) is replaced. Patterns match source text exactly, which is why rustfmt.toml sets a very wide max_width.
"""
import hashlib
import json
import os
import queue
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor

E = "mod/engine/src/"
HELPER = E + "movement/helper.rs"
TRAVERSE = E + "movement/traverse.rs"
DESCEND = E + "movement/descend.rs"
ASCEND = E + "movement/ascend.rs"
PILLAR = E + "movement/pillar.rs"
DOWNWARD = E + "movement/downward.rs"
DIAGONAL = E + "movement/diagonal.rs"
PARKOUR = E + "movement/parkour.rs"
MOVES = E + "movement/moves.rs"
GOALS = E + "goals.rs"
COSTS = E + "costs.rs"
MAIN = "tools/cost-compare/src/main.rs"

# (file, pattern, replacement, name)
M = [
    (COSTS, "0.98f64.powf(ticks as f64)", "0.98f64.powi(ticks)", "pow: powi instead of powf"),
    (HELPER, "((1.0f32 - mult) as f64)", "(1.0f64 - mult as f64)", "float: 1-mult widened too early"),
    (HELPER, "if cfg.allow_swimming && cfg.can_sprint {", "if cfg.can_sprint {", "swim: ignore allowSwimming"),
    (HELPER, "b == Block::Cobweb || b == Block::EndPortal", "b == Block::EndPortal", "through: cobweb passable"),
    (HELPER, "|| s.skull_any || b == Block::BubbleColumn", "|| b == Block::BubbleColumn", "through: skull passable"),
    (HELPER, "return if s.wooden_door { Yes } else { No };", "return Yes;", "through: iron door passable"),
    (HELPER, "if s.gate {\n        return Yes;", "if s.gate {\n        return No;", "through: gate blocked"),
    (HELPER, "if s.carpet {\n        return Maybe;", "if s.carpet {\n        return Yes;", "through: carpet always yes"),
    (HELPER, "if c.blocks_to_avoid.contains(&s.name) {\n        return No;", "if false {\n        return No;", "through: tripwire not avoided"),
    (HELPER, "if s.cauldron {\n        return No;", "if s.cauldron {\n        return Yes;", "through: cauldron passable"),
    (HELPER, "return if s.fluid_amount != 8 { No } else { Maybe };", "return Maybe;", "through: partial fluid maybe"),
    (HELPER, "(b != Block::MagmaBlock || c.allow_walk_on_magma)", "true", "walkon: magma always ok"),
    (HELPER, " && b != Block::HoneyBlock", "", "walkon: honey ok"),
    (HELPER, "(is_climbable(s) && c.allow_vines)", "is_climbable(s)", "walkon: vines always"),
    (HELPER, "b == Block::EnderChest || b == Block::Chest || b == Block::TrappedChest", "b == Block::EnderChest", "walkon: chest no"),
    (HELPER, "if s.stair {\n        return Yes;", "if s.stair {\n        return No;", "walkon: stair no"),
    (HELPER, "if !c.allow_walk_on_bottom_slab {", "if false {", "walkon: bottom slab setting"),
    (HELPER, "if s.layers >= 3 {", "if s.layers >= 4 {", "snow: 3 layers becomes 4"),
    (HELPER, "is_water(&up) ^ self.cfg.assume_walk_on_water", "is_water(&up) | self.cfg.assume_walk_on_water", "water pos: xor to or"),
    (HELPER, "up.block == Block::LilyPad || up.carpet", "up.carpet", "water pos: lily pad"),
    (HELPER, "!up.fluid_empty || up.lily", "!up.fluid_empty", "water through: lily above"),
    (
        HELPER,
        "possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1)) || possibly_flowing(&self.get(x, y, z - 1))",
        "false",
        "flowing: skip neighbours",
    ),
    (HELPER, "if !directly_above && s.falling", "if false && s.falling", "adjacent: falling ignored"),
    (HELPER, "if directly_above || self.cfg.strict_liquid_check {", "if directly_above {", "adjacent: strict liquid ignored"),
    (HELPER, "if s.level == 0 {\n                return true;", "if s.level == 1 {\n                return true;", "adjacent: source level"),
    (HELPER, "self.avoid_adjacent_breaking(x + 1, y, z, false) || ", "", "adjacent: skip east"),
    (HELPER, "|| s.block == Block::Ice || s.infested", "|| s.infested", "break: ice allowed"),
    (HELPER, "result += self.cfg.break_penalty;", "", "break: extra penalty dropped"),
    (HELPER, "if above.falling {", "if false {", "break: falling stack ignored"),
    (MAIN, "let tools = if has_pick[ci] { &str_pick } else { &str_hand };\n                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);\n                let mut res = MoveResult::default();", "let tools = &str_pick;\n                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);\n                let mut res = MoveResult::default();", "break: wrong tool table"),
    (MAIN, "let tools = if has_pick[ci] { &str_pick } else { &str_hand };\n                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);\n                let got = guarded(|| {", "let tools = &str_pick;\n                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);\n                let got = guarded(|| {", "break: wrong tool table in the position rules"),
    (MAIN, "let tools = if has_pick[ci] { &str_pick } else { &str_hand };\n                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);\n                let mut res = MoveResult::default();", "let tools = &str_hand;\n                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);\n                let mut res = MoveResult::default();", "break: hand table in the movements"),
    (MAIN, "let tools = if has_pick[ci] { &str_pick } else { &str_hand };\n                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);\n                let got = guarded(|| {", "let tools = &str_hand;\n                let ctx = Ctx::new(&world, &states, &pres[ci], &cfgs[ci], border, tools);\n                let got = guarded(|| {", "break: hand table in the position rules"),
    (HELPER, "if !self.cfg.allow_break && !self.cfg.allow_break_anyway.contains(&s.name) {", "if false {", "break: allowBreak ignored"),
    (HELPER, "s.frost_melt && s.level == 0", "s.frost_melt", "frost: level ignored"),
    (HELPER, "if s.slab_type != SlabType::Bottom {\n                    return true;", "if false {\n                    return true;", "solid: top slab"),
    (HELPER, "s.stair_shape == StairShape::InnerLeft || s.stair_shape == StairShape::InnerRight", "false", "solid: stair shape"),
    (HELPER, "!s.trap_open && s.trap_half == Half::Top", "s.trap_half == Half::Top", "solid: trapdoor"),
    (HELPER, "if self.get(x, y + 1, z).liquid_block {\n                return false;", "if false {\n                return false;", "solid: liquid above"),
    (HELPER, "return s.layers == 1;", "return true;", "replace: snow any layer"),
    (HELPER, "s.block == Block::LargeFern || s.block == Block::TallGrass", "false", "replace: tall grass override"),
    (HELPER, "!self.cfg.allow_place_source && current.fluid_source", "false", "place: source setting"),
    (HELPER, "!self.cfg.allow_place_flow && !current.fluid_empty && !current.fluid_source", "false", "place: flow setting"),
    (TRAVERSE, "if !water && !sneaking && ctx.cfg.can_sprint {", "if !sneaking && ctx.cfg.can_sprint {", "traverse: sprint in water"),
    (TRAVERSE, "if !water && !sneaking && ctx.cfg.can_sprint {", "if !water && ctx.cfg.can_sprint {", "traverse: sprint when sneaking"),
    (TRAVERSE, "if dest_on.block == Block::SoulSand {", "if false {", "traverse: soul sand dest"),
    (TRAVERSE, "wc += ctx.cfg.walk_on_water_penalty;", "", "traverse: water walk penalty"),
    (TRAVERSE, "&& src_down.block == Block::MagmaBlock {", "&& false {", "traverse: magma sneak"),
    (TRAVERSE, "hardness1 *= 5.0;\n            hardness2 *= 5.0;", "", "traverse: ladder x5 removed"),
    (TRAVERSE, "if is_water(&dest_on) && through_water {", "if false {", "traverse: bridge in water check"),
    (TRAVERSE, "if against_x == x && against_z == z {\n                    continue;\n                }", "", "traverse: backplace check removed"),
    (TRAVERSE, ", (0, -1, 0)]", "]", "traverse: down direction dropped"),
    (TRAVERSE, "(src_down.slab && src_down.slab_type != SlabType::Double)", "false", "traverse: slab backplace"),
    (TRAVERSE, "if !standing_on_a_block {\n                return COST_INF;", "if false {\n                return COST_INF;", "traverse: standing check"),
    (TRAVERSE, "(block_src.block == Block::LilyPad || block_src.carpet) && !src_down.fluid_empty", "false", "traverse: lily backplace"),
    (TRAVERSE, "wc *= SNEAK_ONE_BLOCK_COST / WALK_ONE_BLOCK_COST;", "", "traverse: sneak multiplier"),
    (DESCEND, "if is_climbable(&from_down) {\n        return;", "if false {\n        return;", "descend: climbable start"),
    (DESCEND, "dest_down.block == Block::Ladder || dest_down.block == Block::Vine", "false", "descend: ladder dest"),
    (DESCEND, "if ctx.can_use_frost_walker(&dest_down) {", "if false {", "descend: frost"),
    (DESCEND, "if from_down.block == Block::SoulSand {", "if false {", "descend: soul sand"),
    (DESCEND, "fall_n_blocks_cost()[1].max(CENTER_AFTER_FALL_COST)", "fall_n_blocks_cost()[1].min(CENTER_AFTER_FALL_COST)", "descend: max to min"),
    (DESCEND, "if front_break != 0.0 && ctx.get(dest_x, y + 2, dest_z).falling {", "if false {", "fall: falling above"),
    (DESCEND, "if new_y < ctx.min_y {", "if new_y < ctx.min_y - 1 {", "fall: void check"),
    (DESCEND, "if ctx.is_flowing(dest_x, new_y, dest_z, &onto) {", "if false {", "fall: flowing water"),
    (DESCEND, "if !ctx.can_walk_on_at(dest_x, new_y - 1, dest_z) {", "if false {", "fall: floor under water"),
    (DESCEND, "if unprotected <= 11 && is_climbable(&onto) {", "if false {", "fall: climb reset"),
    (DESCEND, "if is_bottom_slab(&onto) {", "if false {", "fall: bottom slab"),
    (DESCEND, "unprotected <= ctx.cfg.max_fall_no_water + 1", "unprotected <= ctx.cfg.max_fall_no_water", "fall: off by one"),
    (DESCEND, "res.cost = tentative + ctx.cfg.place_penalty;", "res.cost = tentative;", "fall: bucket cost"),
    (ASCEND, "if !ctx.can_walk_on(dest_x, y, dest_z, to_place) {", "if !ctx.can_walk_on(dest_z, y, dest_x, to_place) {", "ascend: can_walk_on args swapped"),
    (ASCEND, "let to_place = ctx.get(dest_x, y, dest_z);", "let to_place = ctx.get(dest_x, y - 1, dest_z);", "ascend: landing cell one too low"),
    (ASCEND, "ctx.cost_of_placing_at(dest_x, dest_z, &to_place)", "ctx.cost_of_placing_at(x, z, &to_place)", "ascend: placing cost at the start column"),
    (ASCEND, "if additional_placement_cost >= COST_INF {\n            return COST_INF;", "if false {\n            return COST_INF;", "ascend: placing cost infinite"),
    (ASCEND, "if !ctx.is_replaceable(dest_x, dest_z, &to_place) {\n            return COST_INF;", "if false {\n            return COST_INF;", "ascend: not replaceable"),
    (ASCEND, "[(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)]", "[(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0)]", "ascend: no place against below"),
    (ASCEND, "[(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)]", "[(0, 0, -1), (0, 0, -1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)]", "ascend: south neighbour dropped"),
    (ASCEND, "[(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)]", "[(0, 0, -1), (0, 0, 1), (1, 0, 0), (1, 0, 0), (0, -1, 0)]", "ascend: west neighbour dropped"),
    (ASCEND, "[(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)]", "[(0, 0, 1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)]", "ascend: north neighbour dropped"),
    (ASCEND, "[(0, 0, -1), (0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, -1, 0)]", "[(0, 0, -1), (0, 0, 1), (-1, 0, 0), (-1, 0, 0), (0, -1, 0)]", "ascend: east neighbour dropped"),
    (ASCEND, "let against_y = y.wrapping_add(sy);", "let against_y = y.wrapping_sub(sy);", "ascend: against y sign"),
    (ASCEND, "let against_x = dest_x.wrapping_add(sx);", "let against_x = x.wrapping_add(sx);", "ascend: against x from the start"),
    (ASCEND, "let against_z = dest_z.wrapping_add(sz);", "let against_z = z.wrapping_add(sz);", "ascend: against z from the start"),
    (ASCEND, "if against_x == x && against_z == z {\n                continue;\n            }", "", "ascend: backplace check removed"),
    (ASCEND, "if against_x == x && against_z == z {", "if against_x == x || against_z == z {", "ascend: backplace check or"),
    (ASCEND, "if ctx.can_place_against(against_x, against_y, against_z) {\n                found_place_option = true;", "if !ctx.can_place_against(against_x, against_y, against_z) {\n                found_place_option = true;", "ascend: place against inverted"),
    (ASCEND, "if !found_place_option {\n            return COST_INF;", "if false {\n            return COST_INF;", "ascend: no place option"),
    (ASCEND, "let src_up2 = ctx.get(x, y + 2, z);", "let src_up2 = ctx.get(x, y + 1, z);", "ascend: head cell one too low"),
    (ASCEND, "if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 1, z) || !src_up2.falling) {", "if false && (ctx.can_walk_through_at(x, y + 1, z) || !src_up2.falling) {", "ascend: falling above ignored"),
    (ASCEND, "if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 1, z) || !src_up2.falling) {", "if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 1, z) && !src_up2.falling) {", "ascend: falling or becomes and"),
    (ASCEND, "if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 1, z) || !src_up2.falling) {", "if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 1, z) || src_up2.falling) {", "ascend: head falling negated"),
    (ASCEND, "if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 1, z) || !src_up2.falling) {", "if ctx.get(x, y + 2, z).falling && (ctx.can_walk_through_at(x, y + 1, z) || !src_up2.falling) {", "ascend: falling checked one too low"),
    (ASCEND, "if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 1, z) || !src_up2.falling) {", "if ctx.get(x, y + 3, z).falling && (ctx.can_walk_through_at(x, y + 2, z) || !src_up2.falling) {", "ascend: walk through check one too high"),
    (ASCEND, "let src_down = ctx.get(x, y - 1, z);", "let src_down = ctx.get(x, y - 2, z);", "ascend: floor cell one too low"),
    (ASCEND, "if is_climbable(&src_down) {\n        return COST_INF;", "if false {\n        return COST_INF;", "ascend: climbable floor"),
    (ASCEND, "let jumping_to_bottom_slab = is_bottom_slab(&to_place);", "let jumping_to_bottom_slab = is_bottom_slab(&src_down);", "ascend: slab target taken from the floor"),
    (ASCEND, "if jumping_from_bottom_slab && !jumping_to_bottom_slab {\n        return COST_INF;", "if jumping_from_bottom_slab {\n        return COST_INF;", "ascend: slab to slab refused"),
    (ASCEND, "if jumping_from_bottom_slab && !jumping_to_bottom_slab {\n        return COST_INF;", "if false {\n        return COST_INF;", "ascend: slab to full block allowed"),
    (ASCEND, "walk = jump_one_block_cost().max(WALK_ONE_BLOCK_COST);\n            walk += ctx.cfg.jump_penalty;", "walk = jump_one_block_cost().min(WALK_ONE_BLOCK_COST);\n            walk += ctx.cfg.jump_penalty;", "ascend: slab to slab min"),
    (ASCEND, "walk = jump_one_block_cost().max(WALK_ONE_BLOCK_COST);\n            walk += ctx.cfg.jump_penalty;", "walk = jump_one_block_cost().max(WALK_ONE_BLOCK_COST);", "ascend: slab to slab jump penalty"),
    (ASCEND, "walk = WALK_ONE_BLOCK_COST;\n        }\n    } else {", "walk = WALK_ONE_BLOCK_COST + ctx.cfg.jump_penalty;\n        }\n    } else {", "ascend: up onto slab jump penalty"),
    (ASCEND, "walk = WALK_ONE_BLOCK_COST;\n        }\n    } else {", "walk = jump_one_block_cost();\n        }\n    } else {", "ascend: up onto slab cost"),
    (ASCEND, "if to_place.block == Block::SoulSand {\n            walk = WALK_ONE_OVER_SOUL_SAND_COST;", "if false {\n            walk = WALK_ONE_OVER_SOUL_SAND_COST;", "ascend: soul sand landing"),
    (ASCEND, "walk = WALK_ONE_OVER_SOUL_SAND_COST;", "walk = WALK_ONE_BLOCK_COST;", "ascend: soul sand cost"),
    (ASCEND, "} else if to_place.block == Block::MagmaBlock {\n            walk = SNEAK_ONE_BLOCK_COST;", "} else if false {\n            walk = SNEAK_ONE_BLOCK_COST;", "ascend: magma landing"),
    (ASCEND, "walk = SNEAK_ONE_BLOCK_COST;", "walk = WALK_ONE_BLOCK_COST;", "ascend: magma cost"),
    (ASCEND, "walk = jump_one_block_cost().max(WALK_ONE_BLOCK_COST);\n        }\n        walk += ctx.cfg.jump_penalty;", "walk = jump_one_block_cost().min(WALK_ONE_BLOCK_COST);\n        }\n        walk += ctx.cfg.jump_penalty;", "ascend: plain jump min"),
    (ASCEND, "walk = jump_one_block_cost().max(WALK_ONE_BLOCK_COST);\n        }\n        walk += ctx.cfg.jump_penalty;", "walk = jump_one_block_cost().max(WALK_ONE_BLOCK_COST);\n        }", "ascend: plain jump penalty dropped"),
    (ASCEND, "let mut total_cost = walk + additional_placement_cost;", "let mut total_cost = walk;", "ascend: placement cost dropped"),
    (ASCEND, "total_cost += ctx.mining_duration(x, y + 2, z, src_up2, false);", "total_cost += ctx.mining_duration(x, y + 2, z, src_up2, true);", "ascend: head mining with falling"),
    (ASCEND, "total_cost += ctx.mining_duration(x, y + 2, z, src_up2, false);", "", "ascend: head mining dropped"),
    (ASCEND, "if total_cost >= COST_INF {\n        return COST_INF;\n    }\n    total_cost += ctx.mining_duration_at(dest_x, y + 1, dest_z, false);", "total_cost += ctx.mining_duration_at(dest_x, y + 1, dest_z, false);", "ascend: first infinity check dropped"),
    (ASCEND, "total_cost += ctx.mining_duration_at(dest_x, y + 1, dest_z, false);", "total_cost += ctx.mining_duration_at(dest_x, y + 1, dest_z, true);", "ascend: low target mining with falling"),
    (ASCEND, "total_cost += ctx.mining_duration_at(dest_x, y + 1, dest_z, false);", "", "ascend: low target mining dropped"),
    (ASCEND, "if total_cost >= COST_INF {\n        return COST_INF;\n    }\n    total_cost += ctx.mining_duration_at(dest_x, y + 2, dest_z, true);", "total_cost += ctx.mining_duration_at(dest_x, y + 2, dest_z, true);", "ascend: second infinity check dropped"),
    (ASCEND, "total_cost += ctx.mining_duration_at(dest_x, y + 2, dest_z, true);", "total_cost += ctx.mining_duration_at(dest_x, y + 2, dest_z, false);", "ascend: high target mining without falling"),
    (ASCEND, "total_cost += ctx.mining_duration_at(dest_x, y + 2, dest_z, true);", "", "ascend: high target mining dropped"),
    (PILLAR, "let ladder = is_climbable(&from_state);", "let ladder = is_climbable(&ctx.get(x, y - 1, z));", "pillar: ladder taken from the floor"),
    (PILLAR, "let from_down = ctx.get(x, y - 1, z);", "let from_down = ctx.get(x, y - 2, z);", "pillar: floor cell one too low"),
    (PILLAR, "if is_climbable(&from_down) {\n            return COST_INF;", "if false {\n            return COST_INF;", "pillar: climbable floor"),
    (PILLAR, "if is_bottom_slab(&from_down) {\n            return COST_INF;", "if false {\n            return COST_INF;", "pillar: bottom slab floor"),
    (PILLAR, "if !ladder {\n        if is_climbable(&from_down) {", "if true {\n        if is_climbable(&from_down) {", "pillar: floor checks on a ladder too"),
    (PILLAR, "let to_break = ctx.get(x, y + 2, z);", "let to_break = ctx.get(x, y + 1, z);", "pillar: head cell one too low"),
    (PILLAR, "if to_break.gate {\n        return COST_INF;", "if false {\n        return COST_INF;", "pillar: gate above"),
    (PILLAR, "if is_water(&to_break) && is_water(&from_state) {", "if is_water(&to_break) || is_water(&from_state) {", "pillar: water check or"),
    (PILLAR, "if is_water(&to_break) && is_water(&from_state) {", "if is_water(&from_state) {", "pillar: head water ignored"),
    (PILLAR, "if is_water(&to_break) && is_water(&from_state) {", "if is_water(&to_break) {", "pillar: feet water ignored"),
    (PILLAR, "let up = ctx.get(x, y + 1, z);\n        src_up = Some(up);", "let up = ctx.get(x, y + 2, z);\n        src_up = Some(up);", "pillar: water cell above one too high"),
    (PILLAR, "if is_water(&up) {\n            return LADDER_UP_ONE_COST;", "if false {\n            return LADDER_UP_ONE_COST;", "pillar: swim up"),
    (PILLAR, "return LADDER_UP_ONE_COST;\n        }\n    }\n    let mut place_cost", "return LADDER_DOWN_ONE_COST;\n        }\n    }\n    let mut place_cost", "pillar: swim up cost"),
    (PILLAR, "place_cost = ctx.cost_of_placing_at(x, z, &from_state);", "place_cost = ctx.cost_of_placing_at(x, z, &from_down);", "pillar: placing cost on the floor"),
    (PILLAR, "if place_cost >= COST_INF {\n            return COST_INF;", "if false {\n            return COST_INF;", "pillar: placing cost infinite"),
    (PILLAR, "if from_down.air {\n            place_cost += 0.1;", "if false {\n            place_cost += 0.1;", "pillar: air below extra"),
    (PILLAR, "place_cost += 0.1;", "place_cost += 0.2;", "pillar: air below extra amount"),
    (PILLAR, "if (is_liquid(&from_state) && !ctx.can_place_against(x, y - 1, z)) || (", "if (", "pillar: liquid feet without support"),
    (PILLAR, "(is_liquid(&from_state) && !ctx.can_place_against(x, y - 1, z))", "(is_liquid(&from_state) && ctx.can_place_against(x, y - 1, z))", "pillar: support check inverted"),
    (PILLAR, "(is_liquid(&from_state) && !ctx.can_place_against(x, y - 1, z))", "(is_liquid(&from_state) && !ctx.can_place_against(x, y, z))", "pillar: support cell offset"),
    (PILLAR, "|| (is_liquid(&from_down) && !is_waterlogged_solid_top(&from_down) && ctx.cfg.assume_walk_on_water) {", " {", "pillar: liquid floor with jesus"),
    (PILLAR, "(is_liquid(&from_down) && !is_waterlogged_solid_top(&from_down) && ctx.cfg.assume_walk_on_water)", "(is_liquid(&from_down) && ctx.cfg.assume_walk_on_water)", "pillar: waterlogged top ignored"),
    (PILLAR, "(is_liquid(&from_down) && !is_waterlogged_solid_top(&from_down) && ctx.cfg.assume_walk_on_water)", "(is_liquid(&from_down) && !is_waterlogged_solid_top(&from_down))", "pillar: assume walk on water ignored"),
    (PILLAR, "(is_liquid(&from_down) && !is_waterlogged_solid_top(&from_down) && ctx.cfg.assume_walk_on_water)", "(!from_down.fluid_empty && !is_waterlogged_solid_top(&from_down) && ctx.cfg.assume_walk_on_water && from_down.fluid_amount == 8)", "pillar: liquid floor only for sources"),
    (PILLAR, "(from_state.block == Block::LilyPad || from_state.carpet) && !from_down.fluid_empty", "from_state.carpet && !from_down.fluid_empty", "pillar: lily pad on liquid"),
    (PILLAR, "(from_state.block == Block::LilyPad || from_state.carpet) && !from_down.fluid_empty", "from_state.block == Block::LilyPad && !from_down.fluid_empty", "pillar: carpet on liquid"),
    (PILLAR, "(from_state.block == Block::LilyPad || from_state.carpet) && !from_down.fluid_empty", "(from_state.block == Block::LilyPad || from_state.carpet)", "pillar: liquid floor for lily pad ignored"),
    (PILLAR, "let mut hardness = ctx.mining_duration(x, y + 2, z, to_break, true);", "let mut hardness = ctx.mining_duration(x, y + 2, z, to_break, false);", "pillar: head mining without falling"),
    (PILLAR, "let mut hardness = ctx.mining_duration(x, y + 2, z, to_break, true);", "let mut hardness = ctx.mining_duration(x, y + 1, z, to_break, true);", "pillar: head mining cell"),
    (PILLAR, "if hardness >= COST_INF {\n        return COST_INF;", "if false {\n        return COST_INF;", "pillar: unbreakable head"),
    (PILLAR, "if hardness != 0.0 {\n        if is_climbable(&to_break) {", "if true {\n        if is_climbable(&to_break) {", "pillar: free head still checked"),
    (PILLAR, "if is_climbable(&to_break) {\n            hardness = 0.0;", "if false {\n            hardness = 0.0;", "pillar: climbable head"),
    (PILLAR, "let check = ctx.get(x, y + 3, z);", "let check = ctx.get(x, y + 2, z);", "pillar: falling check cell"),
    (PILLAR, "if check.falling {", "if false {", "pillar: falling above the head"),
    (PILLAR, "if src_up.is_none() {\n                    src_up = Some(ctx.get(x, y + 1, z));", "if src_up.is_none() {\n                    src_up = Some(ctx.get(x, y + 2, z));", "pillar: lazy feet cell above"),
    (PILLAR, "if !to_break.falling || !src_up.unwrap().falling {", "if !to_break.falling && !src_up.unwrap().falling {", "pillar: falling stack and"),
    (PILLAR, "if !to_break.falling || !src_up.unwrap().falling {", "if !to_break.falling {", "pillar: falling cell above feet ignored"),
    (PILLAR, "if !to_break.falling || !src_up.unwrap().falling {", "if !src_up.unwrap().falling {", "pillar: falling head ignored"),
    (PILLAR, "LADDER_UP_ONE_COST + hardness * 5.0", "LADDER_UP_ONE_COST + hardness", "pillar: ladder hardness multiplier"),
    (PILLAR, "LADDER_UP_ONE_COST + hardness * 5.0", "LADDER_UP_ONE_COST", "pillar: ladder hardness dropped"),
    (PILLAR, "jump_one_block_cost() + place_cost + ctx.cfg.jump_penalty + hardness", "jump_one_block_cost() + ctx.cfg.jump_penalty + hardness", "pillar: place cost dropped"),
    (PILLAR, "jump_one_block_cost() + place_cost + ctx.cfg.jump_penalty + hardness", "jump_one_block_cost() + place_cost + hardness", "pillar: jump penalty dropped"),
    (PILLAR, "jump_one_block_cost() + place_cost + ctx.cfg.jump_penalty + hardness", "jump_one_block_cost() + place_cost + ctx.cfg.jump_penalty", "pillar: hardness dropped"),
    (DOWNWARD, "if !ctx.cfg.allow_downward {\n        return COST_INF;", "if false {\n        return COST_INF;", "downward: setting ignored"),
    (DOWNWARD, "if !ctx.can_walk_on_at(x, y - 2, z) {\n        return COST_INF;", "if false {\n        return COST_INF;", "downward: floor two below ignored"),
    (DOWNWARD, "if !ctx.can_walk_on_at(x, y - 2, z) {", "if !ctx.can_walk_on_at(x, y - 1, z) {", "downward: floor cell offset"),
    (DOWNWARD, "let down = ctx.get(x, y - 1, z);", "let down = ctx.get(x, y - 2, z);", "downward: dug cell offset"),
    (DOWNWARD, "down.block == Block::Ladder || down.block == Block::Vine", "down.block == Block::Ladder", "downward: vine climb"),
    (DOWNWARD, "down.block == Block::Ladder || down.block == Block::Vine", "down.block == Block::Vine", "downward: ladder climb"),
    (DOWNWARD, "LADDER_DOWN_ONE_COST\n", "LADDER_UP_ONE_COST\n", "downward: ladder cost direction"),
    (DOWNWARD, "fall_n_blocks_cost()[1] + ctx.mining_duration(x, y - 1, z, down, false)", "fall_n_blocks_cost()[2] + ctx.mining_duration(x, y - 1, z, down, false)", "downward: fall table index"),
    (DOWNWARD, "fall_n_blocks_cost()[1] + ctx.mining_duration(x, y - 1, z, down, false)", "fall_n_blocks_cost()[1] + ctx.mining_duration(x, y - 1, z, down, true)", "downward: mining with falling"),
    (DOWNWARD, "fall_n_blocks_cost()[1] + ctx.mining_duration(x, y - 1, z, down, false)", "fall_n_blocks_cost()[1]", "downward: mining dropped"),
    (DOWNWARD, "fall_n_blocks_cost()[1] + ctx.mining_duration(x, y - 1, z, down, false)", "fall_n_blocks_cost()[1] + ctx.mining_duration(x, y - 2, z, down, false)", "downward: mining cell offset"),
    (DIAGONAL, "let dest_into_upper = ctx.get(dest_x, y + 1, dest_z);", "let dest_into_upper = ctx.get(dest_x, y + 2, dest_z);", "diagonal: upper cell offset"),
    (DIAGONAL, "if !ctx.can_walk_through(dest_x, y + 1, dest_z, dest_into_upper) {\n        return;", "if false {\n        return;", "diagonal: upper cell blocked"),
    (DIAGONAL, "let dest_into = ctx.get(dest_x, y, dest_z);", "let dest_into = ctx.get(dest_x, y - 1, dest_z);", "diagonal: lower cell offset"),
    (DIAGONAL, "if !ctx.can_walk_through(dest_x, y, dest_z, dest_into) {\n        ascend = true;", "if false {\n        ascend = true;", "diagonal: ascend never"),
    (DIAGONAL, "if !ctx.can_walk_through(dest_x, y, dest_z, dest_into) {\n        ascend = true;", "if ctx.can_walk_through(dest_x, y, dest_z, dest_into) {\n        ascend = true;", "diagonal: ascend inverted"),
    (DIAGONAL, "!ctx.cfg.allow_diagonal_ascend || !ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into) || !ctx.can_walk_through_at(dest_x, y + 2, dest_z)", "!ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into) || !ctx.can_walk_through_at(dest_x, y + 2, dest_z)", "diagonal ascend: drop !allow_diagonal_ascend"),
    (DIAGONAL, "!ctx.cfg.allow_diagonal_ascend || !ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into) || !ctx.can_walk_through_at(dest_x, y + 2, dest_z)", "!ctx.cfg.allow_diagonal_ascend || !ctx.can_walk_on(dest_x, y, dest_z, dest_into) || !ctx.can_walk_through_at(dest_x, y + 2, dest_z)", "diagonal ascend: drop !can_walk_through_at(x, y + 2, z)"),
    (DIAGONAL, "!ctx.cfg.allow_diagonal_ascend || !ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into) || !ctx.can_walk_through_at(dest_x, y + 2, dest_z)", "!ctx.cfg.allow_diagonal_ascend || !ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_through_at(dest_x, y + 2, dest_z)", "diagonal ascend: drop !can_walk_on(dest_x, y, dest_z, dest_into)"),
    (DIAGONAL, "!ctx.cfg.allow_diagonal_ascend || !ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into) || !ctx.can_walk_through_at(dest_x, y + 2, dest_z)", "!ctx.cfg.allow_diagonal_ascend || !ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into)", "diagonal ascend: drop !can_walk_through_at(dest_x, y + 2, dest_z)"),
    (DIAGONAL, "!ctx.can_walk_through_at(x, y + 2, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into)", "!ctx.can_walk_through_at(x, y + 1, z) || !ctx.can_walk_on(dest_x, y, dest_z, dest_into)", "diagonal ascend: head cell offset"),
    (DIAGONAL, "dest_walk_on = dest_into;\n        from_down = ctx.get(x, y - 1, z);\n    } else {", "dest_walk_on = dest_into;\n        from_down = ctx.get(x, y - 2, z);\n    } else {", "diagonal ascend: floor offset"),
    (DIAGONAL, "dest_walk_on = ctx.get(dest_x, y - 1, dest_z);\n        from_down = ctx.get(x, y - 1, z);\n        let standing_on_a_block", "dest_walk_on = ctx.get(dest_x, y - 2, dest_z);\n        from_down = ctx.get(x, y - 1, z);\n        let standing_on_a_block", "diagonal: landing cell offset"),
    (DIAGONAL, "dest_walk_on = ctx.get(dest_x, y - 1, dest_z);\n        from_down = ctx.get(x, y - 1, z);\n        let standing_on_a_block", "dest_walk_on = ctx.get(dest_x, y - 1, dest_z);\n        from_down = ctx.get(x, y - 2, z);\n        let standing_on_a_block", "diagonal: floor cell offset"),
    (DIAGONAL, "ctx.must_be_solid_to_walk_on(x, y - 1, z, &from_down)", "true", "diagonal: frost standing check"),
    (DIAGONAL, "ctx.must_be_solid_to_walk_on(x, y - 1, z, &from_down)", "ctx.must_be_solid_to_walk_on(x, y - 1, z, &dest_walk_on)", "diagonal: frost standing on the wrong cell"),
    (DIAGONAL, "frost_walker = standing_on_a_block && ctx.can_use_frost_walker(&dest_walk_on);", "frost_walker = ctx.can_use_frost_walker(&dest_walk_on);", "diagonal: frost needs standing block"),
    (DIAGONAL, "frost_walker = standing_on_a_block && ctx.can_use_frost_walker(&dest_walk_on);", "frost_walker = standing_on_a_block && ctx.can_use_frost_walker(&from_down);", "diagonal: frost on the floor"),
    (DIAGONAL, "if !frost_walker && !ctx.can_walk_on(dest_x, y - 1, dest_z, dest_walk_on) {", "if !ctx.can_walk_on(dest_x, y - 1, dest_z, dest_walk_on) {", "diagonal: frost skips descend"),
    (DIAGONAL, "if !frost_walker && !ctx.can_walk_on(dest_x, y - 1, dest_z, dest_walk_on) {", "if !frost_walker {", "diagonal: descend without walk on"),
    (DIAGONAL, "!ctx.cfg.allow_diagonal_descend || !ctx.can_walk_on_at(dest_x, y - 2, dest_z) || !ctx.can_walk_through(dest_x, y - 1, dest_z, dest_walk_on)", "!ctx.can_walk_on_at(dest_x, y - 2, dest_z) || !ctx.can_walk_through(dest_x, y - 1, dest_z, dest_walk_on)", "diagonal descend: drop !allow_diagonal_descend"),
    (DIAGONAL, "!ctx.cfg.allow_diagonal_descend || !ctx.can_walk_on_at(dest_x, y - 2, dest_z) || !ctx.can_walk_through(dest_x, y - 1, dest_z, dest_walk_on)", "!ctx.cfg.allow_diagonal_descend || !ctx.can_walk_through(dest_x, y - 1, dest_z, dest_walk_on)", "diagonal descend: drop !can_walk_on_at(dest_x, y - 2, dest_z)"),
    (DIAGONAL, "!ctx.cfg.allow_diagonal_descend || !ctx.can_walk_on_at(dest_x, y - 2, dest_z) || !ctx.can_walk_through(dest_x, y - 1, dest_z, dest_walk_on)", "!ctx.cfg.allow_diagonal_descend || !ctx.can_walk_on_at(dest_x, y - 2, dest_z)", "diagonal descend: drop !can_walk_through(dest_x, y - 1, dest_z, dest_walk_on)"),
    (DIAGONAL, "!ctx.can_walk_on_at(dest_x, y - 2, dest_z)", "!ctx.can_walk_on_at(dest_x, y - 1, dest_z)", "diagonal descend: floor offset"),
    (DIAGONAL, "frost_walker &= !ctx.cfg.assume_walk_on_water;", "", "diagonal: frost with jesus"),
    (DIAGONAL, "let start_state = ctx.get(x, y, z);", "let start_state = ctx.get(x, y - 1, z);", "diagonal: start cell offset"),
    (DIAGONAL, "is_blocking_door(&start_state, x, z, dest_x, dest_z) || ", "", "diagonal: start door"),
    (DIAGONAL, "is_blocking_door(&start_state, x, z, dest_x, dest_z)", "is_blocking_door(&start_state, x, z, dest_z, dest_x)", "diagonal: start door args swapped"),
    (DIAGONAL, "is_blocking_door(if ascend { &dest_into_upper } else { &dest_into }, dest_x, dest_z, x, y)", "is_blocking_door(&dest_into, dest_x, dest_z, x, y)", "diagonal: ascend door cell"),
    (DIAGONAL, "is_blocking_door(if ascend { &dest_into_upper } else { &dest_into }, dest_x, dest_z, x, y)", "is_blocking_door(&dest_into_upper, dest_x, dest_z, x, y)", "diagonal: flat door cell"),
    (DIAGONAL, "is_blocking_door(if ascend { &dest_into_upper } else { &dest_into }, dest_x, dest_z, x, y)", "is_blocking_door(if ascend { &dest_into_upper } else { &dest_into }, dest_x, dest_z, x, z)", "diagonal: door z argument fixed"),
    (DIAGONAL, "is_blocking_door(if ascend { &dest_into_upper } else { &dest_into }, dest_x, dest_z, x, y)", "false", "diagonal: destination door"),
    (DIAGONAL, "if dest_walk_on.block == Block::SoulSand {\n        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;", "if false {\n        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;", "diagonal: soul sand landing"),
    (DIAGONAL, "if dest_walk_on.block == Block::SoulSand {\n        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;", "if dest_walk_on.block == Block::SoulSand {\n        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST);", "diagonal: soul sand landing full cost"),
    (DIAGONAL, "} else if ctx.cfg.allow_walk_on_magma && dest_walk_on.block == Block::MagmaBlock {", "} else if dest_walk_on.block == Block::MagmaBlock {", "diagonal: magma landing setting"),
    (DIAGONAL, "} else if ctx.cfg.allow_walk_on_magma && dest_walk_on.block == Block::MagmaBlock {\n        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;\n        sneaking = true;", "} else if ctx.cfg.allow_walk_on_magma && dest_walk_on.block == Block::MagmaBlock {\n        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;", "diagonal: magma landing sneaking"),
    (DIAGONAL, "} else if ctx.cfg.allow_walk_on_magma && dest_walk_on.block == Block::MagmaBlock {\n        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;", "} else if ctx.cfg.allow_walk_on_magma && dest_walk_on.block == Block::MagmaBlock {\n        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST);", "diagonal: magma landing full cost"),
    (DIAGONAL, "} else if frost_walker {", "} else if false {", "diagonal: frost walker no penalty"),
    (DIAGONAL, "} else if dest_walk_on.block == Block::Water {\n        multiplier += ctx.cfg.walk_on_water_penalty * SQRT_2;", "} else if is_water(&dest_walk_on) {\n        multiplier += ctx.cfg.walk_on_water_penalty * SQRT_2;", "diagonal: any water penalty"),
    (DIAGONAL, "} else if dest_walk_on.block == Block::Water {\n        multiplier += ctx.cfg.walk_on_water_penalty * SQRT_2;", "} else if false {\n        multiplier += ctx.cfg.walk_on_water_penalty * SQRT_2;", "diagonal: water penalty never"),
    (DIAGONAL, "multiplier += ctx.cfg.walk_on_water_penalty * SQRT_2;", "multiplier += ctx.cfg.walk_on_water_penalty;", "diagonal: water penalty without sqrt 2"),
    (DIAGONAL, "if is_climbable(&from_down) {\n        return;", "if false {\n        return;", "diagonal: climbable floor"),
    (DIAGONAL, "if from_down.block == Block::SoulSand {\n        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;", "if false {\n        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;", "diagonal: soul sand floor"),
    (DIAGONAL, "if from_down.block == Block::SoulSand {\n        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST) / 2.0;", "if from_down.block == Block::SoulSand {\n        multiplier += (WALK_ONE_OVER_SOUL_SAND_COST - WALK_ONE_BLOCK_COST);", "diagonal: soul sand floor full cost"),
    (DIAGONAL, "} else if ctx.cfg.allow_walk_on_magma && from_down.block == Block::MagmaBlock {", "} else if from_down.block == Block::MagmaBlock {", "diagonal: magma floor setting"),
    (DIAGONAL, "} else if ctx.cfg.allow_walk_on_magma && from_down.block == Block::MagmaBlock {\n        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;\n        sneaking = true;", "} else if ctx.cfg.allow_walk_on_magma && from_down.block == Block::MagmaBlock {\n        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;", "diagonal: magma floor sneaking"),
    (DIAGONAL, "} else if ctx.cfg.allow_walk_on_magma && from_down.block == Block::MagmaBlock {\n        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST) / 2.0;", "} else if ctx.cfg.allow_walk_on_magma && from_down.block == Block::MagmaBlock {\n        multiplier += (SNEAK_ONE_BLOCK_COST - WALK_ONE_BLOCK_COST);", "diagonal: magma floor full cost"),
    (DIAGONAL, "let cutting_over1 = ctx.get(x, y - 1, dest_z);", "let cutting_over1 = ctx.get(dest_x, y - 1, z);", "diagonal: first corner cell swapped"),
    (DIAGONAL, "let cutting_over1 = ctx.get(x, y - 1, dest_z);", "let cutting_over1 = ctx.get(x, y, dest_z);", "diagonal: first corner cell offset"),
    (DIAGONAL, "if (!ctx.cfg.allow_walk_on_magma && cutting_over1.block == Block::MagmaBlock) || is_lava(&cutting_over1) {", "if is_lava(&cutting_over1) {", "diagonal: first corner magma"),
    (DIAGONAL, "if (!ctx.cfg.allow_walk_on_magma && cutting_over1.block == Block::MagmaBlock) || is_lava(&cutting_over1) {", "if (!ctx.cfg.allow_walk_on_magma && cutting_over1.block == Block::MagmaBlock) {", "diagonal: first corner lava"),
    (DIAGONAL, "if (!ctx.cfg.allow_walk_on_magma && cutting_over1.block == Block::MagmaBlock) || is_lava(&cutting_over1) {", "if (ctx.cfg.allow_walk_on_magma && cutting_over1.block == Block::MagmaBlock) || is_lava(&cutting_over1) {", "diagonal: first corner magma setting inverted"),
    (DIAGONAL, "let cutting_over2 = ctx.get(dest_x, y - 1, z);", "let cutting_over2 = ctx.get(x, y - 1, dest_z);", "diagonal: second corner cell swapped"),
    (DIAGONAL, "let cutting_over2 = ctx.get(dest_x, y - 1, z);", "let cutting_over2 = ctx.get(dest_x, y, z);", "diagonal: second corner cell offset"),
    (DIAGONAL, "if (!ctx.cfg.allow_walk_on_magma && cutting_over2.block == Block::MagmaBlock) || is_lava(&cutting_over2) {", "if is_lava(&cutting_over2) {", "diagonal: second corner magma"),
    (DIAGONAL, "if (!ctx.cfg.allow_walk_on_magma && cutting_over2.block == Block::MagmaBlock) || is_lava(&cutting_over2) {", "if (!ctx.cfg.allow_walk_on_magma && cutting_over2.block == Block::MagmaBlock) {", "diagonal: second corner lava"),
    (DIAGONAL, "if (!ctx.cfg.allow_walk_on_magma && cutting_over2.block == Block::MagmaBlock) || is_lava(&cutting_over2) {", "if (ctx.cfg.allow_walk_on_magma && cutting_over2.block == Block::MagmaBlock) || is_lava(&cutting_over2) {", "diagonal: second corner magma setting inverted"),
    (DIAGONAL, "if is_water(&start_state) || is_water(&dest_into) {", "if is_water(&start_state) && is_water(&dest_into) {", "diagonal: water and"),
    (DIAGONAL, "if is_water(&start_state) || is_water(&dest_into) {", "if is_water(&dest_into) {", "diagonal: start in water ignored"),
    (DIAGONAL, "if is_water(&start_state) || is_water(&dest_into) {", "if is_water(&start_state) {", "diagonal: destination in water ignored"),
    (DIAGONAL, "if ascend {\n            return;\n        }\n        // Ignore previous multiplier", "if false {\n            return;\n        }\n        // Ignore previous multiplier", "diagonal: ascend out of water"),
    (DIAGONAL, "multiplier = ctx.water_walk_speed;", "multiplier = WALK_ONE_IN_WATER_COST;", "diagonal: water speed"),
    (DIAGONAL, "multiplier = ctx.water_walk_speed;\n        water = true;", "multiplier = ctx.water_walk_speed;", "diagonal: water flag"),
    (DIAGONAL, "let pb0 = ctx.get(x, y, dest_z);", "let pb0 = ctx.get(dest_x, y, z);", "diagonal: first side cell swapped"),
    (DIAGONAL, "let pb2 = ctx.get(dest_x, y, z);", "let pb2 = ctx.get(x, y, dest_z);", "diagonal: second side cell swapped"),
    (DIAGONAL, "let a_top = ctx.can_walk_through_at(x, y + 2, dest_z);", "let a_top = ctx.can_walk_through_at(x, y + 1, dest_z);", "diagonal ascend: a top offset"),
    (DIAGONAL, "let a_mid = ctx.can_walk_through_at(x, y + 1, dest_z);", "let a_mid = ctx.can_walk_through_at(x, y, dest_z);", "diagonal ascend: a mid offset"),
    (DIAGONAL, "let mut a_low = ctx.can_walk_through(x, y, dest_z, pb0);", "let mut a_low = ctx.can_walk_through(x, y, dest_z, pb2);", "diagonal ascend: a low state"),
    (DIAGONAL, "let b_top = ctx.can_walk_through_at(dest_x, y + 2, z);", "let b_top = ctx.can_walk_through_at(dest_x, y + 1, z);", "diagonal ascend: b top offset"),
    (DIAGONAL, "let b_mid = ctx.can_walk_through_at(dest_x, y + 1, z);", "let b_mid = ctx.can_walk_through_at(dest_x, y, z);", "diagonal ascend: b mid offset"),
    (DIAGONAL, "let mut b_low = ctx.can_walk_through(dest_x, y, z, pb2);", "let mut b_low = ctx.can_walk_through(dest_x, y, z, pb0);", "diagonal ascend: b low state"),
    (DIAGONAL, "a_low &= !is_blocking_door(&pb0, x, dest_z, dest_x, z);", "", "diagonal ascend: a low door"),
    (DIAGONAL, "b_low &= !is_blocking_door(&pb2, dest_x, z, x, dest_z);", "", "diagonal ascend: b low door"),
    (DIAGONAL, "a_low &= !is_blocking_door(&pb0, x, dest_z, dest_x, z);", "a_low &= !is_blocking_door(&pb0, x, dest_z, dest_z, x);", "diagonal ascend: a low door args swapped"),
    (DIAGONAL, "b_low &= !is_blocking_door(&pb2, dest_x, z, x, dest_z);", "b_low &= !is_blocking_door(&pb2, dest_x, z, dest_z, x);", "diagonal ascend: b low door args swapped"),
    (DIAGONAL, "if (!(a_top && a_mid && a_low) && !(b_top && b_mid && b_low)) || avoid_walking_into", "if (!(a_top && a_mid && a_low) || !(b_top && b_mid && b_low)) || avoid_walking_into", "diagonal ascend: both sides blocked or"),
    (DIAGONAL, "if (!(a_top && a_mid && a_low) && !(b_top && b_mid && b_low)) || avoid_walking_into", "if avoid_walking_into", "diagonal ascend: both sides blocked dropped"),
    (DIAGONAL, "|| avoid_walking_into(&pb0, ctx.cfg) || avoid_walking_into(&pb2, ctx.cfg) ||", "|| avoid_walking_into(&pb2, ctx.cfg) ||", "diagonal ascend: avoid a side"),
    (DIAGONAL, "|| avoid_walking_into(&pb0, ctx.cfg) || avoid_walking_into(&pb2, ctx.cfg) ||", "|| avoid_walking_into(&pb0, ctx.cfg) ||", "diagonal ascend: avoid b side"),
    (DIAGONAL, "|| (a_top && a_mid && ctx.can_walk_on(x, y, dest_z, pb0))", "", "diagonal ascend: a side is a floor"),
    (DIAGONAL, "|| (b_top && b_mid && ctx.can_walk_on(dest_x, y, z, pb2))", "", "diagonal ascend: b side is a floor"),
    (DIAGONAL, "(a_top && a_mid && ctx.can_walk_on(x, y, dest_z, pb0))", "(a_top && a_mid && ctx.can_walk_on(x, y, dest_z, pb2))", "diagonal ascend: a floor state"),
    (DIAGONAL, "(b_top && b_mid && ctx.can_walk_on(dest_x, y, z, pb2))", "(b_top && b_mid && ctx.can_walk_on(dest_x, y, z, pb0))", "diagonal ascend: b floor state"),
    (DIAGONAL, "|| (!a_top && a_mid && a_low) ||", "||", "diagonal ascend: a low head"),
    (DIAGONAL, "|| (!b_top && b_mid && b_low) {", " {", "diagonal ascend: b low head"),
    (DIAGONAL, "res.cost = multiplier * SQRT_2 + jump_one_block_cost() + ctx.cfg.jump_penalty;", "res.cost = multiplier * SQRT_2 + jump_one_block_cost();", "diagonal ascend: jump penalty"),
    (DIAGONAL, "res.cost = multiplier * SQRT_2 + jump_one_block_cost() + ctx.cfg.jump_penalty;", "res.cost = multiplier * SQRT_2 + ctx.cfg.jump_penalty;", "diagonal ascend: jump cost"),
    (DIAGONAL, "res.cost = multiplier * SQRT_2 + jump_one_block_cost() + ctx.cfg.jump_penalty;", "res.cost = multiplier + jump_one_block_cost() + ctx.cfg.jump_penalty;", "diagonal ascend: sqrt 2 factor"),
    (DIAGONAL, "res.x = dest_x;\n        res.z = dest_z;\n        res.y = y + 1;", "res.x = dest_z;\n        res.z = dest_x;\n        res.y = y + 1;", "diagonal ascend: result x and z swapped"),
    (DIAGONAL, "res.x = dest_x;\n        res.z = dest_z;\n        res.y = y + 1;", "res.x = dest_x;\n        res.z = dest_z;\n        res.y = y;", "diagonal ascend: result y"),
    (DIAGONAL, "let mut option_a = ctx.mining_duration(x, y, dest_z, pb0, false);", "let mut option_a = ctx.mining_duration(x, y, dest_z, pb0, true);", "diagonal: a low mining with falling"),
    (DIAGONAL, "let mut option_a = ctx.mining_duration(x, y, dest_z, pb0, false);", "let mut option_a = ctx.mining_duration(dest_x, y, z, pb0, false);", "diagonal: a low mining cell swapped"),
    (DIAGONAL, "let mut option_b = ctx.mining_duration(dest_x, y, z, pb2, false);", "let mut option_b = ctx.mining_duration(dest_x, y, z, pb2, true);", "diagonal: b low mining with falling"),
    (DIAGONAL, "let mut option_b = ctx.mining_duration(dest_x, y, z, pb2, false);", "let mut option_b = ctx.mining_duration(x, y, dest_z, pb2, false);", "diagonal: b low mining cell swapped"),
    (DIAGONAL, "option_a += if is_blocking_door(&pb0, x, dest_z, dest_x, z) { 1.0 } else { 0.0 };", "option_a += if is_blocking_door(&pb0, x, dest_z, dest_x, z) { 0.0 } else { 0.0 };", "diagonal: a door cost"),
    (DIAGONAL, "option_a += if is_blocking_door(&pb0, x, dest_z, dest_x, z) { 1.0 } else { 0.0 };", "option_a += if is_blocking_door(&pb0, x, dest_z, dest_z, x) { 1.0 } else { 0.0 };", "diagonal: a door args swapped"),
    (DIAGONAL, "option_b += if is_blocking_door(&pb2, dest_x, z, x, dest_z) { 1.0 } else { 0.0 };", "option_b += if is_blocking_door(&pb2, dest_x, z, x, dest_z) { 0.0 } else { 0.0 };", "diagonal: b door cost"),
    (DIAGONAL, "option_b += if is_blocking_door(&pb2, dest_x, z, x, dest_z) { 1.0 } else { 0.0 };", "option_b += if is_blocking_door(&pb2, dest_x, z, dest_z, x) { 1.0 } else { 0.0 };", "diagonal: b door args swapped"),
    (DIAGONAL, "if option_a != 0.0 && option_b != 0.0 {\n        // check these one at a time", "if false {\n        // check these one at a time", "diagonal: early return after the low cells"),
    (DIAGONAL, "if option_a != 0.0 && option_b != 0.0 {\n        // check these one at a time", "if option_a != 0.0 || option_b != 0.0 {\n        // check these one at a time", "diagonal: early return after the low cells or"),
    (DIAGONAL, "let pb1 = ctx.get(x, y + 1, dest_z);", "let pb1 = ctx.get(dest_x, y + 1, z);", "diagonal: a high cell swapped"),
    (DIAGONAL, "option_a += ctx.mining_duration(x, y + 1, dest_z, pb1, true);", "option_a += ctx.mining_duration(x, y + 1, dest_z, pb1, false);", "diagonal: a high mining without falling"),
    (DIAGONAL, "option_a += ctx.mining_duration(x, y + 1, dest_z, pb1, true);", "", "diagonal: a high mining dropped"),
    (DIAGONAL, "if option_a != 0.0 && option_b != 0.0 {\n        // same deal", "if false {\n        // same deal", "diagonal: early return after a high"),
    (DIAGONAL, "let pb3 = ctx.get(dest_x, y + 1, z);", "let pb3 = ctx.get(x, y + 1, dest_z);", "diagonal: b high cell swapped"),
    (DIAGONAL, "if option_a == 0.0 && ((avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) || avoid_walking_into(&pb3, ctx.cfg)) {", "if option_a == 0.0 && (avoid_walking_into(&pb2, ctx.cfg) || avoid_walking_into(&pb3, ctx.cfg)) {", "diagonal: a side water allowed"),
    (DIAGONAL, "if option_a == 0.0 && ((avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) || avoid_walking_into(&pb3, ctx.cfg)) {", "if option_a == 0.0 && (avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) {", "diagonal: a side high avoid"),
    (DIAGONAL, "if option_a == 0.0 && ((avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) || avoid_walking_into(&pb3, ctx.cfg)) {", "if option_a == 0.0 && (pb2.block != Block::Water || avoid_walking_into(&pb3, ctx.cfg)) {", "diagonal: a side avoid dropped"),
    (DIAGONAL, "if option_a == 0.0 && ((avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) || avoid_walking_into(&pb3, ctx.cfg)) {", "if ((avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) || avoid_walking_into(&pb3, ctx.cfg)) {", "diagonal: a side avoid without free a"),
    (DIAGONAL, "if option_a == 0.0 && ((avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) || avoid_walking_into(&pb3, ctx.cfg)) {", "if option_a == 0.0 && ((avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) || avoid_walking_into(&pb3, ctx.cfg)) {", "diagonal: a side avoid wrong cell"),
    (DIAGONAL, "option_b += ctx.mining_duration(dest_x, y + 1, z, pb3, true);", "option_b += ctx.mining_duration(dest_x, y + 1, z, pb3, false);", "diagonal: b high mining without falling"),
    (DIAGONAL, "option_b += ctx.mining_duration(dest_x, y + 1, z, pb3, true);", "", "diagonal: b high mining dropped"),
    (DIAGONAL, "if option_a != 0.0 && option_b != 0.0 {\n        // and finally", "if false {\n        // and finally", "diagonal: early return after b high"),
    (DIAGONAL, "if option_b == 0.0 && ((avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) || avoid_walking_into(&pb1, ctx.cfg)) {", "if option_b == 0.0 && (avoid_walking_into(&pb0, ctx.cfg) || avoid_walking_into(&pb1, ctx.cfg)) {", "diagonal: b side water allowed"),
    (DIAGONAL, "if option_b == 0.0 && ((avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) || avoid_walking_into(&pb1, ctx.cfg)) {", "if option_b == 0.0 && (avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) {", "diagonal: b side high avoid"),
    (DIAGONAL, "if option_b == 0.0 && ((avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) || avoid_walking_into(&pb1, ctx.cfg)) {", "if option_b == 0.0 && (pb0.block != Block::Water || avoid_walking_into(&pb1, ctx.cfg)) {", "diagonal: b side avoid dropped"),
    (DIAGONAL, "if option_b == 0.0 && ((avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) || avoid_walking_into(&pb1, ctx.cfg)) {", "if ((avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) || avoid_walking_into(&pb1, ctx.cfg)) {", "diagonal: b side avoid without free b"),
    (DIAGONAL, "if option_b == 0.0 && ((avoid_walking_into(&pb0, ctx.cfg) && pb0.block != Block::Water) || avoid_walking_into(&pb1, ctx.cfg)) {", "if option_b == 0.0 && ((avoid_walking_into(&pb2, ctx.cfg) && pb2.block != Block::Water) || avoid_walking_into(&pb1, ctx.cfg)) {", "diagonal: b side avoid wrong cell"),
    (DIAGONAL, "if option_a != 0.0 || option_b != 0.0 {\n        multiplier *= SQRT_2 - 0.001;", "if option_a != 0.0 && option_b != 0.0 {\n        multiplier *= SQRT_2 - 0.001;", "diagonal: edging condition and"),
    (DIAGONAL, "multiplier *= SQRT_2 - 0.001; // TODO tune", "multiplier *= SQRT_2; // TODO tune", "diagonal: edging factor"),
    (DIAGONAL, "if is_climbable(&start_state) {\n            // edging around", "if false {\n            // edging around", "diagonal: edging off a ladder"),
    (DIAGONAL, "if ctx.cfg.can_sprint && !water && !sneaking {", "if !water && !sneaking {", "diagonal: sprint setting"),
    (DIAGONAL, "if ctx.cfg.can_sprint && !water && !sneaking {", "if ctx.cfg.can_sprint && !sneaking {", "diagonal: sprint in water"),
    (DIAGONAL, "if ctx.cfg.can_sprint && !water && !sneaking {", "if ctx.cfg.can_sprint && !water {", "diagonal: sprint when sneaking"),
    (DIAGONAL, "multiplier *= SPRINT_MULTIPLIER;", "", "diagonal: sprint multiplier"),
    (DIAGONAL, "res.cost = multiplier * SQRT_2;\n    if descend {", "res.cost = multiplier;\n    if descend {", "diagonal: flat cost without sqrt 2"),
    (DIAGONAL, "res.cost += fall_n_blocks_cost()[1].max(CENTER_AFTER_FALL_COST);", "res.cost += fall_n_blocks_cost()[1].min(CENTER_AFTER_FALL_COST);", "diagonal: descend max to min"),
    (DIAGONAL, "res.cost += fall_n_blocks_cost()[1].max(CENTER_AFTER_FALL_COST);", "res.cost += fall_n_blocks_cost()[1];", "diagonal: descend centre cost"),
    (DIAGONAL, "res.y = y - 1;", "res.y = y;", "diagonal: descend result y"),
    (DIAGONAL, "    res.x = dest_x;\n    res.z = dest_z;\n}", "    res.x = dest_z;\n    res.z = dest_x;\n}", "diagonal: flat result x and z swapped"),
    (DIAGONAL, "if !state.door {\n        return false;", "if false {\n        return false;", "diagonal door: not a door"),
    (DIAGONAL, "let ox = state.door_facing.step_x();", "let ox = state.door_facing.step_z();", "diagonal door: facing x"),
    (DIAGONAL, "let oz = state.door_facing.step_z();", "let oz = state.door_facing.step_x();", "diagonal door: facing z"),
    (DIAGONAL, "if state.door_hinge_left {", "if !state.door_hinge_left {", "diagonal door: hinge side"),
    (DIAGONAL, "nbr_x = door_x.wrapping_sub(ox).wrapping_add(oz);", "nbr_x = door_x.wrapping_sub(ox).wrapping_sub(oz);", "diagonal door: left x sign"),
    (DIAGONAL, "nbr_z = door_z.wrapping_sub(oz).wrapping_sub(ox);", "nbr_z = door_z.wrapping_sub(oz).wrapping_add(ox);", "diagonal door: left z sign"),
    (DIAGONAL, "nbr_x = door_x.wrapping_sub(ox).wrapping_sub(oz);", "nbr_x = door_x.wrapping_add(ox).wrapping_sub(oz);", "diagonal door: right x sign"),
    (DIAGONAL, "nbr_z = door_z.wrapping_sub(oz).wrapping_add(ox);", "nbr_z = door_z.wrapping_add(oz).wrapping_add(ox);", "diagonal door: right z sign"),
    (DIAGONAL, "nbr_x == other_x && nbr_z == other_z", "nbr_x == other_x || nbr_z == other_z", "diagonal door: neighbour match or"),
    (DIAGONAL, "nbr_x == other_x && nbr_z == other_z", "nbr_x == other_x", "diagonal door: neighbour z ignored"),
    (PARKOUR, "if !ctx.cfg.allow_parkour {\n        return;", "if false {\n        return;", "parkour: setting ignored"),
    (PARKOUR, "if !ctx.cfg.allow_jump_at_build_limit && y >= ctx.max_y {", "if y >= ctx.max_y {", "parkour: build limit setting"),
    (PARKOUR, "if !ctx.cfg.allow_jump_at_build_limit && y >= ctx.max_y {", "if !ctx.cfg.allow_jump_at_build_limit && y > ctx.max_y {", "parkour: build limit boundary"),
    (PARKOUR, "if !ctx.cfg.allow_jump_at_build_limit && y >= ctx.max_y {", "if !ctx.cfg.allow_jump_at_build_limit && y >= ctx.max_y - 1 {", "parkour: build limit one lower"),
    (PARKOUR, "let x_diff = dir.step_x();", "let x_diff = dir.step_z();", "parkour: x step from z"),
    (PARKOUR, "let z_diff = dir.step_z();", "let z_diff = dir.step_x();", "parkour: z step from x"),
    (PARKOUR, "if !ctx.fully_passable_at(x.wrapping_add(x_diff), y, z.wrapping_add(z_diff)) {", "if false {", "parkour: adjacent cell blocked"),
    (PARKOUR, "let adj = ctx.get(x.wrapping_add(x_diff), y.wrapping_sub(1), z.wrapping_add(z_diff));", "let adj = ctx.get(x.wrapping_add(x_diff), y, z.wrapping_add(z_diff));", "parkour: adjacent floor offset"),
    (PARKOUR, "if ctx.can_walk_on(x.wrapping_add(x_diff), y.wrapping_sub(1), z.wrapping_add(z_diff), adj) {\n        // second most common", "if false {\n        // second most common", "parkour: adjacent floor walkable"),
    (PARKOUR, "if avoid_walking_into(&adj, ctx.cfg) && !is_water(&adj) {", "if avoid_walking_into(&adj, ctx.cfg) {", "parkour: adjacent water allowed"),
    (PARKOUR, "if avoid_walking_into(&adj, ctx.cfg) && !is_water(&adj) {", "if false {", "parkour: adjacent hazard"),
    (PARKOUR, "if avoid_walking_into(&adj, ctx.cfg) && !is_water(&adj) {", "if !is_water(&adj) {", "parkour: adjacent hazard check dropped"),
    (PARKOUR, "if !ctx.fully_passable_at(x.wrapping_add(x_diff), y.wrapping_add(1), z.wrapping_add(z_diff)) {", "if false {", "parkour: adjacent head cell"),
    (PARKOUR, "if !ctx.fully_passable_at(x.wrapping_add(x_diff), y.wrapping_add(2), z.wrapping_add(z_diff)) {", "if false {", "parkour: adjacent top cell"),
    (PARKOUR, "if !ctx.fully_passable_at(x, y.wrapping_add(2), z) {", "if false {", "parkour: own top cell"),
    (PARKOUR, "if is_climbable(&standing_on) || standing_on.stair || is_bottom_slab(&standing_on) {", "if standing_on.stair || is_bottom_slab(&standing_on) {", "parkour: climbable floor"),
    (PARKOUR, "if is_climbable(&standing_on) || standing_on.stair || is_bottom_slab(&standing_on) {", "if is_climbable(&standing_on) || is_bottom_slab(&standing_on) {", "parkour: stair floor"),
    (PARKOUR, "if is_climbable(&standing_on) || standing_on.stair || is_bottom_slab(&standing_on) {", "if is_climbable(&standing_on) || standing_on.stair {", "parkour: bottom slab floor"),
    (PARKOUR, "let standing_on = ctx.get(x, y.wrapping_sub(1), z);", "let standing_on = ctx.get(x, y, z);", "parkour: floor cell offset"),
    (PARKOUR, "if ctx.cfg.assume_walk_on_water && !standing_on.fluid_empty && !is_waterlogged_solid_top(&standing_on) {", "if !standing_on.fluid_empty && !is_waterlogged_solid_top(&standing_on) {", "parkour: jesus setting"),
    (PARKOUR, "if ctx.cfg.assume_walk_on_water && !standing_on.fluid_empty && !is_waterlogged_solid_top(&standing_on) {", "if ctx.cfg.assume_walk_on_water && !is_waterlogged_solid_top(&standing_on) {", "parkour: jesus floor liquid"),
    (PARKOUR, "if ctx.cfg.assume_walk_on_water && !standing_on.fluid_empty && !is_waterlogged_solid_top(&standing_on) {", "if ctx.cfg.assume_walk_on_water && !standing_on.fluid_empty {", "parkour: jesus waterlogged top"),
    (PARKOUR, "if !ctx.get(x, y, z).fluid_empty {\n        // can't jump out of water", "if false {\n        // can't jump out of water", "parkour: jump out of water"),
    (PARKOUR, "if ctx.cfg.allow_walk_on_magma && standing_on.block == Block::MagmaBlock {", "if standing_on.block == Block::MagmaBlock {", "parkour: magma floor setting"),
    (PARKOUR, "if ctx.cfg.allow_walk_on_magma && standing_on.block == Block::MagmaBlock {\n        max_jump = 2;", "if ctx.cfg.allow_walk_on_magma && standing_on.block == Block::MagmaBlock {\n        max_jump = 3;", "parkour: magma floor jump length"),
    (PARKOUR, "} else if standing_on.block == Block::SoulSand {\n        max_jump = 2; // 1 block gap", "} else if false {\n        max_jump = 2; // 1 block gap", "parkour: soul sand floor"),
    (PARKOUR, "max_jump = 2; // 1 block gap", "max_jump = 3; // 1 block gap", "parkour: soul sand jump length"),
    (PARKOUR, "max_jump = 4;", "max_jump = 3;", "parkour: sprint jump length"),
    (PARKOUR, "} else if ctx.cfg.can_sprint {", "} else if false {", "parkour: sprint setting"),
    (PARKOUR, "max_jump = 3;\n    }", "max_jump = 2;\n    }", "parkour: walk jump length"),
    (PARKOUR, "let mut verified_max_jump = 1;", "let mut verified_max_jump = 2;", "parkour: verified start"),
    (PARKOUR, "while i <= max_jump {", "while i < max_jump {", "parkour: loop bound"),
    (PARKOUR, "let dest_x = x.wrapping_add(x_diff.wrapping_mul(i));", "let dest_x = x.wrapping_add(z_diff.wrapping_mul(i));", "parkour: loop x from z"),
    (PARKOUR, "let dest_z = z.wrapping_add(z_diff.wrapping_mul(i));", "let dest_z = z.wrapping_add(x_diff.wrapping_mul(i));", "parkour: loop z from x"),
    (PARKOUR, "if !ctx.fully_passable_at(dest_x, y.wrapping_add(1), dest_z) {\n            break;", "if false {\n            break;", "parkour: loop head cell"),
    (PARKOUR, "if !ctx.fully_passable_at(dest_x, y.wrapping_add(2), dest_z) {\n            break;", "if false {\n            break;", "parkour: loop top cell"),
    (PARKOUR, "let dest_into = ctx.get(dest_x, y, dest_z);", "let dest_into = ctx.get(dest_x, y.wrapping_add(1), dest_z);", "parkour: landing cell offset"),
    (PARKOUR, "if !ctx.fully_passable(dest_into) {", "if false {", "parkour: ascend landing branch"),
    (PARKOUR, "if i <= 3 && ctx.cfg.allow_parkour_ascend", "if ctx.cfg.allow_parkour_ascend", "parkour: ascend length limit"),
    (PARKOUR, "if i <= 3 && ctx.cfg.allow_parkour_ascend", "if i <= 4 && ctx.cfg.allow_parkour_ascend", "parkour: ascend length limit one more"),
    (PARKOUR, "if i <= 3 && ctx.cfg.allow_parkour_ascend", "if i <= 2 && ctx.cfg.allow_parkour_ascend", "parkour: ascend length limit one less"),
    (PARKOUR, "&& ctx.cfg.allow_parkour_ascend && ctx.cfg.can_sprint &&", "&& ctx.cfg.can_sprint &&", "parkour: ascend setting"),
    (PARKOUR, "&& ctx.cfg.allow_parkour_ascend && ctx.cfg.can_sprint &&", "&& ctx.cfg.allow_parkour_ascend &&", "parkour: ascend needs sprint"),
    (PARKOUR, "&& ctx.can_walk_on(dest_x, y, dest_z, dest_into) && check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y.wrapping_add(1), dest_z.wrapping_add(z_diff)) {", "&& check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y.wrapping_add(1), dest_z.wrapping_add(z_diff)) {", "parkour: ascend landing walkable"),
    (PARKOUR, "&& ctx.can_walk_on(dest_x, y, dest_z, dest_into) && check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y.wrapping_add(1), dest_z.wrapping_add(z_diff)) {", "&& ctx.can_walk_on(dest_x, y, dest_z, dest_into) {", "parkour: ascend overshoot safety"),
    (PARKOUR, "&& check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y.wrapping_add(1), dest_z.wrapping_add(z_diff)) {", "&& check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y, dest_z.wrapping_add(z_diff)) {", "parkour: ascend overshoot height"),
    (PARKOUR, "&& check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y.wrapping_add(1), dest_z.wrapping_add(z_diff)) {", "&& check_overshoot_safety(ctx, dest_x, y.wrapping_add(1), dest_z) {", "parkour: ascend overshoot cell"),
    (PARKOUR, "res.y = y.wrapping_add(1);", "res.y = y;", "parkour: ascend result y"),
    (PARKOUR, "res.cost = (i as f64) * SPRINT_ONE_BLOCK_COST + ctx.cfg.jump_penalty;", "res.cost = (i as f64) * SPRINT_ONE_BLOCK_COST;", "parkour: ascend jump penalty"),
    (PARKOUR, "res.cost = (i as f64) * SPRINT_ONE_BLOCK_COST + ctx.cfg.jump_penalty;", "res.cost = (i as f64) * WALK_ONE_BLOCK_COST + ctx.cfg.jump_penalty;", "parkour: ascend per block cost"),
    (PARKOUR, "let landing_on = ctx.get(dest_x, y.wrapping_sub(1), dest_z);", "let landing_on = ctx.get(dest_x, y, dest_z);", "parkour: flat landing cell offset"),
    (PARKOUR, "(landing_on.block != Block::Farmland && ctx.can_walk_on(dest_x, y.wrapping_sub(1), dest_z, landing_on))", "ctx.can_walk_on(dest_x, y.wrapping_sub(1), dest_z, landing_on)", "parkour: farmland landing"),
    (PARKOUR, "(landing_on.block != Block::Farmland && ctx.can_walk_on(dest_x, y.wrapping_sub(1), dest_z, landing_on))", "(landing_on.block != Block::Farmland)", "parkour: landing walkable"),
    (PARKOUR, "|| (ctx.cfg.frost_walker.wrapping_add(2).min(16) >= i && ctx.can_use_frost_walker(&landing_on)) {", " {", "parkour: frost landing"),
    (PARKOUR, "ctx.cfg.frost_walker.wrapping_add(2).min(16) >= i", "ctx.cfg.frost_walker.wrapping_add(2) >= i", "parkour: frost cap"),
    (PARKOUR, "ctx.cfg.frost_walker.wrapping_add(2).min(16) >= i", "ctx.cfg.frost_walker.wrapping_add(1).min(16) >= i", "parkour: frost reach"),
    (PARKOUR, "ctx.cfg.frost_walker.wrapping_add(2).min(16) >= i", "ctx.cfg.frost_walker.wrapping_add(2).min(16) > i", "parkour: frost reach boundary"),
    (PARKOUR, "ctx.cfg.frost_walker.wrapping_add(2).min(16) >= i && ctx.can_use_frost_walker(&landing_on)", "ctx.can_use_frost_walker(&landing_on)", "parkour: frost reach dropped"),
    (PARKOUR, "ctx.cfg.frost_walker.wrapping_add(2).min(16) >= i && ctx.can_use_frost_walker(&landing_on)", "ctx.cfg.frost_walker.wrapping_add(2).min(16) >= i", "parkour: frost state dropped"),
    (PARKOUR, "if check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y, dest_z.wrapping_add(z_diff)) {\n                res.x = dest_x;", "if true {\n                res.x = dest_x;", "parkour: flat overshoot safety"),
    (PARKOUR, "if check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y, dest_z.wrapping_add(z_diff)) {\n                res.x = dest_x;", "if check_overshoot_safety(ctx, dest_x, y, dest_z) {\n                res.x = dest_x;", "parkour: flat overshoot cell"),
    (PARKOUR, "res.cost = cost_from_jump_distance(i) + ctx.cfg.jump_penalty;", "res.cost = cost_from_jump_distance(i);", "parkour: flat jump penalty"),
    (PARKOUR, "            break;\n        }\n\n        if !ctx.fully_passable_at(dest_x, y.wrapping_add(3), dest_z) {\n            break;\n        }", "            break;\n        }", "parkour: loop clearance above"),
    (PARKOUR, "if !ctx.fully_passable_at(dest_x, y.wrapping_add(3), dest_z) {", "if !ctx.fully_passable_at(dest_x, y.wrapping_add(2), dest_z) {", "parkour: loop clearance cell"),
    (PARKOUR, "verified_max_jump = i;", "verified_max_jump = i - 1;", "parkour: verified jump"),
    (PARKOUR, "if !ctx.cfg.allow_parkour_place {\n        return;", "if false {\n        return;", "parkour: place setting"),
    (PARKOUR, "let mut i = verified_max_jump;\n    while i > 1 {", "let mut i = verified_max_jump;\n    while i > 2 {", "parkour: place loop bound"),
    (PARKOUR, "let mut i = verified_max_jump;\n    while i > 1 {", "let mut i = verified_max_jump - 1;\n    while i > 1 {", "parkour: place loop start"),
    (PARKOUR, "let dest_x = x.wrapping_add(i.wrapping_mul(x_diff));", "let dest_x = x.wrapping_add(i.wrapping_mul(z_diff));", "parkour: place x from z"),
    (PARKOUR, "let dest_z = z.wrapping_add(i.wrapping_mul(z_diff));", "let dest_z = z.wrapping_add(i.wrapping_mul(x_diff));", "parkour: place z from x"),
    (PARKOUR, "let to_replace = ctx.get(dest_x, y.wrapping_sub(1), dest_z);", "let to_replace = ctx.get(dest_x, y, dest_z);", "parkour: replaced cell offset"),
    (PARKOUR, "let place_cost = ctx.cost_of_placing_at(dest_x, dest_z, &to_replace);", "let place_cost = ctx.cost_of_placing_at(x, z, &to_replace);", "parkour: placing cost position"),
    (PARKOUR, "if place_cost >= COST_INF {\n            i -= 1;", "if false {\n            i -= 1;", "parkour: placing cost infinite"),
    (PARKOUR, "if !ctx.is_replaceable(dest_x, dest_z, &to_replace) {\n            i -= 1;", "if false {\n            i -= 1;", "parkour: not replaceable"),
    (PARKOUR, "if !check_overshoot_safety(ctx, dest_x.wrapping_add(x_diff), y, dest_z.wrapping_add(z_diff)) {\n            i -= 1;", "if false {\n            i -= 1;", "parkour: place overshoot safety"),
    (PARKOUR, "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::West, Direction::Down];", "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::West, Direction::West];", "parkour: place against down dropped"),
    (PARKOUR, "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::West, Direction::Down];", "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::South, Direction::South, Direction::East, Direction::West, Direction::Down];", "parkour: place against north dropped"),
    (PARKOUR, "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::West, Direction::Down];", "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::North, Direction::East, Direction::West, Direction::Down];", "parkour: place against south dropped"),
    (PARKOUR, "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::West, Direction::Down];", "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::West, Direction::West, Direction::Down];", "parkour: place against east dropped"),
    (PARKOUR, "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::West, Direction::Down];", "const HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::East, Direction::Down];", "parkour: place against west dropped"),
    (PARKOUR, "let against_x = dest_x.wrapping_add(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_x());", "let against_x = dest_x.wrapping_sub(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_x());", "parkour: place against x sign"),
    (PARKOUR, "let against_y = y.wrapping_sub(1).wrapping_add(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_y());", "let against_y = y.wrapping_sub(1);", "parkour: place against y step"),
    (PARKOUR, "let against_y = y.wrapping_sub(1).wrapping_add(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_y());", "let against_y = y.wrapping_add(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_y());", "parkour: place against y base"),
    (PARKOUR, "let against_z = dest_z.wrapping_add(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_z());", "let against_z = dest_z.wrapping_sub(HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP[j].step_z());", "parkour: place against z sign"),
    (PARKOUR, "if against_x == dest_x.wrapping_sub(x_diff) && against_z == dest_z.wrapping_sub(z_diff) {", "if false {", "parkour: turn around check"),
    (PARKOUR, "if against_x == dest_x.wrapping_sub(x_diff) && against_z == dest_z.wrapping_sub(z_diff) {", "if against_x == dest_x.wrapping_sub(x_diff) || against_z == dest_z.wrapping_sub(z_diff) {", "parkour: turn around check or"),
    (PARKOUR, "if against_x == dest_x.wrapping_sub(x_diff) && against_z == dest_z.wrapping_sub(z_diff) {", "if against_x == dest_x.wrapping_add(x_diff) && against_z == dest_z.wrapping_add(z_diff) {", "parkour: turn around direction"),
    (PARKOUR, "if ctx.can_place_against(against_x, against_y, against_z) {\n                res.x = dest_x;", "if !ctx.can_place_against(against_x, against_y, against_z) {\n                res.x = dest_x;", "parkour: place against inverted"),
    (PARKOUR, "res.cost = cost_from_jump_distance(i) + place_cost + ctx.cfg.jump_penalty;", "res.cost = cost_from_jump_distance(i) + ctx.cfg.jump_penalty;", "parkour: place cost dropped"),
    (PARKOUR, "res.cost = cost_from_jump_distance(i) + place_cost + ctx.cfg.jump_penalty;", "res.cost = cost_from_jump_distance(i) + place_cost;", "parkour: place jump penalty"),
    (PARKOUR, "!avoid_walking_into(&ctx.get(x, y, z), ctx.cfg) && !avoid_walking_into(&ctx.get(x, y.wrapping_add(1), z), ctx.cfg)", "!avoid_walking_into(&ctx.get(x, y.wrapping_add(1), z), ctx.cfg)", "parkour overshoot: first cell"),
    (PARKOUR, "!avoid_walking_into(&ctx.get(x, y, z), ctx.cfg) && !avoid_walking_into(&ctx.get(x, y.wrapping_add(1), z), ctx.cfg)", "!avoid_walking_into(&ctx.get(x, y, z), ctx.cfg)", "parkour overshoot: second cell"),
    (PARKOUR, "!avoid_walking_into(&ctx.get(x, y, z), ctx.cfg) && !avoid_walking_into(&ctx.get(x, y.wrapping_add(1), z), ctx.cfg)", "!avoid_walking_into(&ctx.get(x, y, z), ctx.cfg) && !avoid_walking_into(&ctx.get(x, y.wrapping_sub(1), z), ctx.cfg)", "parkour overshoot: second cell offset"),
    (PARKOUR, "2 => WALK_ONE_BLOCK_COST * 2.0, // IDK LOL", "2 => WALK_ONE_BLOCK_COST * 3.0, // IDK LOL", "parkour cost: two blocks"),
    (PARKOUR, "3 => WALK_ONE_BLOCK_COST * 3.0,", "3 => WALK_ONE_BLOCK_COST * 2.0,", "parkour cost: three blocks"),
    (PARKOUR, "4 => SPRINT_ONE_BLOCK_COST * 4.0,", "4 => WALK_ONE_BLOCK_COST * 4.0,", "parkour cost: four blocks"),
    (MOVES, "Moves::ParkourEast => 4,", "Moves::ParkourEast => 3,", "moves: parkour east x offset"),
    (MOVES, "Moves::ParkourWest => -4,", "Moves::ParkourWest => -3,", "moves: parkour west x offset"),
    (MOVES, "Moves::TraverseEast | Moves::AscendEast | Moves::DescendEast | Moves::DiagonalNortheast | Moves::DiagonalSoutheast => 1,", "Moves::TraverseEast | Moves::AscendEast | Moves::DescendEast | Moves::DiagonalNortheast => 1,", "moves: diagonal southeast x offset"),
    (MOVES, "Moves::TraverseEast | Moves::AscendEast | Moves::DescendEast | Moves::DiagonalNortheast | Moves::DiagonalSoutheast => 1,", "Moves::AscendEast | Moves::DescendEast | Moves::DiagonalNortheast | Moves::DiagonalSoutheast => 1,", "moves: traverse east x offset"),
    (MOVES, "Moves::TraverseWest | Moves::AscendWest | Moves::DescendWest | Moves::DiagonalNorthwest | Moves::DiagonalSouthwest => -1,", "Moves::TraverseWest | Moves::DescendWest | Moves::DiagonalNorthwest | Moves::DiagonalSouthwest => -1,", "moves: ascend west x offset"),
    (MOVES, "Moves::TraverseWest | Moves::AscendWest | Moves::DescendWest | Moves::DiagonalNorthwest | Moves::DiagonalSouthwest => -1,", "Moves::AscendWest | Moves::DescendWest | Moves::DiagonalNorthwest | Moves::DiagonalSouthwest => -1,", "moves: traverse west x offset"),
    (MOVES, "Moves::Downward | Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth => -1,", "Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth => -1,", "moves: downward y offset"),
    (MOVES, "Moves::Downward | Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth => -1,", "Moves::Downward | Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth => -1,", "moves: descend south y offset"),
    (MOVES, "Moves::Pillar | Moves::AscendNorth | Moves::AscendSouth | Moves::AscendEast | Moves::AscendWest => 1,", "Moves::AscendNorth | Moves::AscendSouth | Moves::AscendEast | Moves::AscendWest => 1,", "moves: pillar y offset"),
    (MOVES, "Moves::Pillar | Moves::AscendNorth | Moves::AscendSouth | Moves::AscendEast | Moves::AscendWest => 1,", "Moves::Pillar | Moves::AscendNorth | Moves::AscendSouth | Moves::AscendEast => 1,", "moves: ascend west y offset"),
    (MOVES, "Moves::TraverseNorth | Moves::AscendNorth | Moves::DescendNorth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest => -1,", "Moves::AscendNorth | Moves::DescendNorth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest => -1,", "moves: traverse north z offset"),
    (MOVES, "Moves::TraverseNorth | Moves::AscendNorth | Moves::DescendNorth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest => -1,", "Moves::TraverseNorth | Moves::AscendNorth | Moves::DescendNorth | Moves::DiagonalNortheast => -1,", "moves: diagonal northwest z offset"),
    (MOVES, "Moves::TraverseSouth | Moves::AscendSouth | Moves::DescendSouth | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest => 1,", "Moves::TraverseSouth | Moves::DescendSouth | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest => 1,", "moves: ascend south z offset"),
    (MOVES, "Moves::TraverseSouth | Moves::AscendSouth | Moves::DescendSouth | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest => 1,", "Moves::TraverseSouth | Moves::AscendSouth | Moves::DescendSouth | Moves::DiagonalSoutheast => 1,", "moves: diagonal southwest z offset"),
    (MOVES, "Moves::ParkourNorth => -4,", "Moves::ParkourNorth => -3,", "moves: parkour north z offset"),
    (MOVES, "Moves::ParkourSouth => 4,", "Moves::ParkourSouth => 3,", "moves: parkour south z offset"),
    (MOVES, "matches!(self, Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "matches!(self, Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast)", "moves: dynamic xz parkour west"),
    (MOVES, "matches!(self, Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "matches!(self, Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest | Moves::DescendEast)", "moves: dynamic xz descend east"),
    (MOVES, "matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "matches!(self, Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "moves: dynamic y descend east"),
    (MOVES, "matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast)", "moves: dynamic y parkour west"),
    (MOVES, "matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "moves: dynamic y diagonal southwest"),
    (MOVES, "matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest)", "matches!(self, Moves::DescendEast | Moves::DescendWest | Moves::DescendNorth | Moves::DescendSouth | Moves::DiagonalNortheast | Moves::DiagonalNorthwest | Moves::DiagonalSoutheast | Moves::DiagonalSouthwest | Moves::ParkourNorth | Moves::ParkourSouth | Moves::ParkourEast | Moves::ParkourWest | Moves::AscendEast)", "moves: dynamic y ascend east"),
    (MOVES, "Moves::DescendEast => descend::cost(ctx, x, y, z, x.wrapping_add(1), z, res),", "Moves::DescendEast => descend::cost(ctx, x, y, z, x.wrapping_sub(1), z, res),", "moves: descend east direction"),
    (MOVES, "Moves::DescendWest => descend::cost(ctx, x, y, z, x.wrapping_sub(1), z, res),", "Moves::DescendWest => descend::cost(ctx, x, y, z, x.wrapping_add(1), z, res),", "moves: descend west direction"),
    (MOVES, "Moves::DescendNorth => descend::cost(ctx, x, y, z, x, z.wrapping_sub(1), res),", "Moves::DescendNorth => descend::cost(ctx, x, y, z, x, z.wrapping_add(1), res),", "moves: descend north direction"),
    (MOVES, "Moves::DescendSouth => descend::cost(ctx, x, y, z, x, z.wrapping_add(1), res),", "Moves::DescendSouth => descend::cost(ctx, x, y, z, x, z.wrapping_sub(1), res),", "moves: descend south direction"),
    (MOVES, "Moves::DescendEast => descend::cost(ctx, x, y, z, x.wrapping_add(1), z, res),", "Moves::DescendEast => descend::cost(ctx, x, y, z, z, x.wrapping_add(1), res),", "moves: descend east args swapped"),
    (MOVES, "Moves::DiagonalNortheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_sub(1), res),", "Moves::DiagonalNortheast => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_sub(1), res),", "moves: diagonal northeast x direction"),
    (MOVES, "Moves::DiagonalNortheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_sub(1), res),", "Moves::DiagonalNortheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_add(1), res),", "moves: diagonal northeast z direction"),
    (MOVES, "Moves::DiagonalNorthwest => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_sub(1), res),", "Moves::DiagonalNorthwest => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_sub(1), res),", "moves: diagonal northwest x direction"),
    (MOVES, "Moves::DiagonalNorthwest => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_sub(1), res),", "Moves::DiagonalNorthwest => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_add(1), res),", "moves: diagonal northwest z direction"),
    (MOVES, "Moves::DiagonalSoutheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_add(1), res),", "Moves::DiagonalSoutheast => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_add(1), res),", "moves: diagonal southeast x direction"),
    (MOVES, "Moves::DiagonalSoutheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_add(1), res),", "Moves::DiagonalSoutheast => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_sub(1), res),", "moves: diagonal southeast z direction"),
    (MOVES, "Moves::DiagonalSouthwest => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_add(1), res),", "Moves::DiagonalSouthwest => diagonal::cost(ctx, x, y, z, x.wrapping_add(1), z.wrapping_add(1), res),", "moves: diagonal southwest x direction"),
    (MOVES, "Moves::DiagonalSouthwest => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_add(1), res),", "Moves::DiagonalSouthwest => diagonal::cost(ctx, x, y, z, x.wrapping_sub(1), z.wrapping_sub(1), res),", "moves: diagonal southwest z direction"),
    (MOVES, "Moves::ParkourNorth => parkour::cost(ctx, x, y, z, Direction::North, res),", "Moves::ParkourNorth => parkour::cost(ctx, x, y, z, Direction::South, res),", "moves: parkour north direction"),
    (MOVES, "Moves::ParkourSouth => parkour::cost(ctx, x, y, z, Direction::South, res),", "Moves::ParkourSouth => parkour::cost(ctx, x, y, z, Direction::North, res),", "moves: parkour south direction"),
    (MOVES, "Moves::ParkourEast => parkour::cost(ctx, x, y, z, Direction::East, res),", "Moves::ParkourEast => parkour::cost(ctx, x, y, z, Direction::West, res),", "moves: parkour east direction"),
    (MOVES, "Moves::ParkourWest => parkour::cost(ctx, x, y, z, Direction::West, res),", "Moves::ParkourWest => parkour::cost(ctx, x, y, z, Direction::East, res),", "moves: parkour west direction"),
    (MOVES, "res.x = x.wrapping_add(self.x_offset());", "res.x = x;", "moves: generic result x"),
    (MOVES, "res.y = y.wrapping_add(self.y_offset());", "res.y = y;", "moves: generic result y"),
    (MOVES, "res.z = z.wrapping_add(self.z_offset());", "res.z = z.wrapping_add(self.x_offset());", "moves: generic result z"),
    (MOVES, "Moves::Downward => downward::cost(ctx, x, y, z),", "Moves::Downward => pillar::cost(ctx, x, y, z),", "moves: downward cost function"),
    (MOVES, "Moves::Pillar => pillar::cost(ctx, x, y, z),", "Moves::Pillar => downward::cost(ctx, x, y, z),", "moves: pillar cost function"),
    (MOVES, "Moves::TraverseNorth => traverse::cost(ctx, x, y, z, x, z.wrapping_sub(1)),", "Moves::TraverseNorth => traverse::cost(ctx, x, y, z, x, z.wrapping_add(1)),", "moves: traverse north direction"),
    (MOVES, "Moves::TraverseSouth => traverse::cost(ctx, x, y, z, x, z.wrapping_add(1)),", "Moves::TraverseSouth => traverse::cost(ctx, x, y, z, x, z.wrapping_sub(1)),", "moves: traverse south direction"),
    (MOVES, "Moves::TraverseEast => traverse::cost(ctx, x, y, z, x.wrapping_add(1), z),", "Moves::TraverseEast => traverse::cost(ctx, x, y, z, x.wrapping_sub(1), z),", "moves: traverse east direction"),
    (MOVES, "Moves::TraverseWest => traverse::cost(ctx, x, y, z, x.wrapping_sub(1), z),", "Moves::TraverseWest => traverse::cost(ctx, x, y, z, x.wrapping_add(1), z),", "moves: traverse west direction"),
    (MOVES, "Moves::TraverseEast => traverse::cost(ctx, x, y, z, x.wrapping_add(1), z),", "Moves::TraverseEast => traverse::cost(ctx, x, y, z, z, x.wrapping_add(1)),", "moves: traverse east args swapped"),
    (MOVES, "Moves::AscendNorth => ascend::cost(ctx, x, y, z, x, z.wrapping_sub(1)),", "Moves::AscendNorth => ascend::cost(ctx, x, y, z, x, z.wrapping_add(1)),", "moves: ascend north direction"),
    (MOVES, "Moves::AscendSouth => ascend::cost(ctx, x, y, z, x, z.wrapping_add(1)),", "Moves::AscendSouth => ascend::cost(ctx, x, y, z, x, z.wrapping_sub(1)),", "moves: ascend south direction"),
    (MOVES, "Moves::AscendEast => ascend::cost(ctx, x, y, z, x.wrapping_add(1), z),", "Moves::AscendEast => ascend::cost(ctx, x, y, z, x.wrapping_sub(1), z),", "moves: ascend east direction"),
    (MOVES, "Moves::AscendWest => ascend::cost(ctx, x, y, z, x.wrapping_sub(1), z),", "Moves::AscendWest => ascend::cost(ctx, x, y, z, x.wrapping_add(1), z),", "moves: ascend west direction"),
    (MOVES, "Moves::AscendEast => ascend::cost(ctx, x, y, z, x.wrapping_add(1), z),", "Moves::AscendEast => ascend::cost(ctx, x, y, z, z, x.wrapping_add(1)),", "moves: ascend east args swapped"),
    (MOVES, "Moves::TraverseNorth => \"TRAVERSE_NORTH\",", "Moves::TraverseNorth => \"TRAVERSE_SOUTH\",", "moves: name of traverse north"),
    (MOVES, "        Moves::Downward,\n        Moves::Pillar,\n        Moves::TraverseNorth,", "        Moves::Pillar,\n        Moves::Downward,\n        Moves::TraverseNorth,", "moves: all order downward and pillar"),
    (MOVES, "        Moves::ParkourEast,\n        Moves::ParkourWest,\n    ];", "        Moves::ParkourWest,\n        Moves::ParkourEast,\n    ];", "moves: all order parkour east and west"),
    (GOALS, "Goal::Near { x, y, z, range_sq: range.wrapping_mul(range) }", "Goal::Near { x, y, z, range_sq: range.saturating_mul(range) }", "goals: near range squared saturates"),
    (GOALS, "Goal::Near { x, y, z, range_sq: range.wrapping_mul(range) }", "Goal::Near { x, y, z, range_sq: range }", "goals: near range not squared"),
    (GOALS, "    assert!(!(dx == 0 && dz == 0), \"GoalStrictDirection: direction has no horizontal step\");\n", "", "goals: strict direction without step allowed"),
    (GOALS, "    assert!(!from.is_empty(), \"Positions to run away from must not be empty\");\n", "", "goals: run away from nothing allowed"),
    (GOALS, "distance_sq: (distance * distance) as i32", "distance_sq: distance as i32", "goals: run away distance not squared"),
    (GOALS, "distance_sq: (distance * distance) as i32", "distance_sq: (distance * distance).round() as i32", "goals: run away distance rounded"),
    (GOALS, "return fall_n_blocks_cost()[2] / 2.0 * (current_y.wrapping_sub(goal_y) as f64);", "return fall_n_blocks_cost()[1] / 2.0 * (current_y.wrapping_sub(goal_y) as f64);", "goals: y level fall table index"),
    (GOALS, "return fall_n_blocks_cost()[2] / 2.0 * (current_y.wrapping_sub(goal_y) as f64);", "return fall_n_blocks_cost()[2] * (current_y.wrapping_sub(goal_y) as f64);", "goals: y level fall not halved"),
    (GOALS, "return fall_n_blocks_cost()[2] / 2.0 * (current_y.wrapping_sub(goal_y) as f64);", "return fall_n_blocks_cost()[2] / 2.0 * (goal_y.wrapping_sub(current_y) as f64);", "goals: y level fall sign"),
    (GOALS, "return (goal_y.wrapping_sub(current_y) as f64) * jump_one_block_cost();", "return (current_y.wrapping_sub(goal_y) as f64) * jump_one_block_cost();", "goals: y level jump sign"),
    (GOALS, "return (goal_y.wrapping_sub(current_y) as f64) * jump_one_block_cost();", "return (goal_y.wrapping_sub(current_y) as f64) * fall_n_blocks_cost()[1];", "goals: y level jump cost"),
    (GOALS, "if current_y > goal_y {\n        // need to descend", "if current_y < goal_y {\n        // need to descend", "goals: y level branch swapped"),
    (GOALS, "let x = x_diff.abs();", "let x = x_diff;", "goals: xz x without abs"),
    (GOALS, "let z = z_diff.abs();", "let z = z_diff;", "goals: xz z without abs"),
    (GOALS, "    if x < z {\n        straight = z - x;", "    if x > z {\n        straight = z - x;", "goals: xz branch swapped"),
    (GOALS, "straight = z - x;", "straight = x - z;", "goals: xz straight z minus x"),
    (GOALS, "straight = x - z;\n        diagonal = z;", "straight = z - x;\n        diagonal = z;", "goals: xz straight x minus z"),
    (GOALS, "diagonal = x;\n    } else {", "diagonal = z;\n    } else {", "goals: xz diagonal in the first branch"),
    (GOALS, "        diagonal = z;\n    }\n    diagonal *= SQRT_2;", "        diagonal = x;\n    }\n    diagonal *= SQRT_2;", "goals: xz diagonal in the second branch"),
    (GOALS, "    diagonal *= SQRT_2;\n", "", "goals: xz diagonal factor"),
    (GOALS, "(diagonal + straight) * s.cost_heuristic", "(diagonal + straight)", "goals: xz cost heuristic"),
    (GOALS, "(diagonal + straight) * s.cost_heuristic", "(diagonal * straight) * s.cost_heuristic", "goals: xz sum becomes product"),
    (GOALS, "heuristic += goal_y_level_calculate(0, y_diff);", "heuristic += goal_y_level_calculate(y_diff, 0);", "goals: block y arguments swapped"),
    (GOALS, "heuristic += goal_y_level_calculate(0, y_diff);", "", "goals: block y part dropped"),
    (GOALS, "heuristic += goal_xz_calculate(x_diff, z_diff, s);", "", "goals: block xz part dropped"),
    (GOALS, "if inside < min_outside {\n            max_inside = java_max(max_inside, inside);", "if inside <= min_outside {\n            max_inside = java_max(max_inside, inside);", "goals: inside strictly below outside"),
    (GOALS, "if inside < min_outside {\n            max_inside = java_max(max_inside, inside);", "if true {\n            max_inside = java_max(max_inside, inside);", "goals: inside filter dropped"),
    (GOALS, "max_inside = java_max(max_inside, inside);", "max_inside = max_inside.max(inside);", "goals: inside max is the Rust max"),
    (GOALS, "max_inside = java_max(max_inside, inside);", "max_inside = java_min(max_inside, inside);", "goals: inside max is a min"),
    (GOALS, "let mut max_inside = f64::NEG_INFINITY;", "let mut max_inside = f64::MIN;", "goals: inside start is the lowest finite"),
    (GOALS, "let mut max_inside = f64::NEG_INFINITY;", "let mut max_inside = 0.0;", "goals: inside start is zero"),
    (GOALS, "Goal::Block { x: gx, y: gy, z: gz } => x == *gx && y == *gy && z == *gz,", "Goal::Block { x: gx, y: gy, z: gz } => y == *gy && z == *gz,", "goals: block in goal x ignored"),
    (GOALS, "Goal::Block { x: gx, y: gy, z: gz } => x == *gx && y == *gy && z == *gz,", "Goal::Block { x: gx, y: gy, z: gz } => x == *gx && z == *gz,", "goals: block in goal y ignored"),
    (GOALS, "Goal::Block { x: gx, y: gy, z: gz } => x == *gx && y == *gy && z == *gz,", "Goal::Block { x: gx, y: gy, z: gz } => x == *gx && y == *gy,", "goals: block in goal z ignored"),
    (GOALS, "Goal::Xz { x: gx, z: gz } | Goal::XzMaintainY { x: gx, z: gz, .. } => x == *gx && z == *gz,", "Goal::Xz { x: gx, z: gz } | Goal::XzMaintainY { x: gx, z: gz, .. } => z == *gz,", "goals: xz in goal x ignored"),
    (GOALS, "Goal::Xz { x: gx, z: gz } | Goal::XzMaintainY { x: gx, z: gz, .. } => x == *gx && z == *gz,", "Goal::Xz { x: gx, z: gz } | Goal::XzMaintainY { x: gx, z: gz, .. } => x == *gx,", "goals: xz in goal z ignored"),
    (GOALS, "Goal::YLevel { level } => y == *level,", "Goal::YLevel { level } => y >= *level,", "goals: y level in goal above"),
    (GOALS, "Goal::TwoBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy || y == gy.wrapping_sub(1)) && z == *gz,", "Goal::TwoBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy) && z == *gz,", "goals: two blocks upper only"),
    (GOALS, "Goal::TwoBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy || y == gy.wrapping_sub(1)) && z == *gz,", "Goal::TwoBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy || y == gy.wrapping_add(1)) && z == *gz,", "goals: two blocks upward"),
    (GOALS, "Goal::TwoBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy || y == gy.wrapping_sub(1)) && z == *gz,", "Goal::TwoBlocks { x: gx, y: gy, z: gz } => (y == *gy || y == gy.wrapping_sub(1)) && z == *gz,", "goals: two blocks in goal x ignored"),
    (GOALS, "Goal::TwoBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy || y == gy.wrapping_sub(1)) && z == *gz,", "Goal::TwoBlocks { x: gx, y: gy, z: gz } => x == *gx && (y == *gy || y == gy.wrapping_sub(1)),", "goals: two blocks in goal z ignored"),
    (GOALS, ".wrapping_add((if y_diff < 0 { y_diff + 1 } else { y_diff }).wrapping_abs())", ".wrapping_add(y_diff.wrapping_abs())", "goals: get to block in goal y offset"),
    (GOALS, ".wrapping_add((if y_diff < 0 { y_diff + 1 } else { y_diff }).wrapping_abs())", ".wrapping_add((if y_diff > 0 { y_diff + 1 } else { y_diff }).wrapping_abs())", "goals: get to block in goal y offset above"),
    (GOALS, ".wrapping_add(z_diff.wrapping_abs()) <= 1", ".wrapping_add(z_diff.wrapping_abs()) <= 2", "goals: get to block in goal distance 2"),
    (GOALS, ".wrapping_add(z_diff.wrapping_abs()) <= 1", ".wrapping_add(z_diff.wrapping_abs()) < 1", "goals: get to block in goal distance below 1"),
    (GOALS, "x_diff.wrapping_abs().wrapping_add((if y_diff < 0", "x_diff.wrapping_add((if y_diff < 0", "goals: get to block in goal x without abs"),
    (GOALS, ".wrapping_add(z_diff.wrapping_abs()) <= 1", ".wrapping_add(z_diff) <= 1", "goals: get to block in goal z without abs"),
    (GOALS, "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) <= *range_sq", "x_diff.wrapping_mul(x_diff).wrapping_add(z_diff.wrapping_mul(z_diff)) <= *range_sq", "goals: near in goal y ignored"),
    (GOALS, "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) <= *range_sq", "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) < *range_sq", "goals: near in goal strictly inside"),
    (GOALS, "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) <= *range_sq", "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) <= range_sq.wrapping_add(1)", "goals: near in goal one outside"),
    (GOALS, "x_diff.wrapping_mul(x_diff).wrapping_add(y_diff.wrapping_mul(y_diff)).wrapping_add(z_diff.wrapping_mul(z_diff)) <= *range_sq", "x_diff.saturating_mul(x_diff).saturating_add(y_diff.saturating_mul(y_diff)).saturating_add(z_diff.saturating_mul(z_diff)) <= *range_sq", "goals: near in goal saturating"),
    (GOALS, "y == s.axis_height && (x == 0 || z == 0 || x.wrapping_abs() == z.wrapping_abs())", "(x == 0 || z == 0 || x.wrapping_abs() == z.wrapping_abs())", "goals: axis in goal height ignored"),
    (GOALS, "y == s.axis_height && (x == 0 || z == 0 || x.wrapping_abs() == z.wrapping_abs())", "y == s.axis_height && (z == 0 || x.wrapping_abs() == z.wrapping_abs())", "goals: axis in goal x axis"),
    (GOALS, "y == s.axis_height && (x == 0 || z == 0 || x.wrapping_abs() == z.wrapping_abs())", "y == s.axis_height && (x == 0 || x.wrapping_abs() == z.wrapping_abs())", "goals: axis in goal z axis"),
    (GOALS, "y == s.axis_height && (x == 0 || z == 0 || x.wrapping_abs() == z.wrapping_abs())", "y == s.axis_height && (x == 0 || z == 0)", "goals: axis in goal diagonal"),
    (GOALS, "y == s.axis_height && (x == 0 || z == 0 || x.wrapping_abs() == z.wrapping_abs())", "y == s.axis_height && (x == 0 || z == 0 || x == z)", "goals: axis in goal diagonal one side"),
    (GOALS, "y == s.axis_height && (x == 0 || z == 0 || x.wrapping_abs() == z.wrapping_abs())", "y == s.axis_height && (x == 0 && z == 0 || x.wrapping_abs() == z.wrapping_abs())", "goals: axis in goal axes and"),
    (GOALS, "Goal::StrictDirection { .. } => false,", "Goal::StrictDirection { .. } => true,", "goals: strict direction in goal"),
    (GOALS, "if *my != y {\n                        return false;", "if *my == y {\n                        return false;", "goals: run away in goal height inverted"),
    (GOALS, "let diff_x = x.wrapping_sub(p.0);", "let diff_x = x.wrapping_sub(p.2);", "goals: run away in goal x from z"),
    (GOALS, "let diff_z = z.wrapping_sub(p.2);", "let diff_z = z.wrapping_sub(p.1);", "goals: run away in goal z from y"),
    (GOALS, "let dist_sq = diff_x.wrapping_mul(diff_x).wrapping_add(diff_z.wrapping_mul(diff_z));", "let dist_sq = diff_x.wrapping_mul(diff_x).wrapping_add(diff_z);", "goals: run away in goal z not squared"),
    (GOALS, "let dist_sq = diff_x.wrapping_mul(diff_x).wrapping_add(diff_z.wrapping_mul(diff_z));", "let dist_sq = diff_x.wrapping_mul(diff_x);", "goals: run away in goal z ignored"),
    (GOALS, "let dist_sq = diff_x.wrapping_mul(diff_x).wrapping_add(diff_z.wrapping_mul(diff_z));", "let dist_sq = diff_x.saturating_mul(diff_x).saturating_add(diff_z.saturating_mul(diff_z));", "goals: run away in goal saturating"),
    (GOALS, "if dist_sq < *distance_sq {\n                        return false;", "if dist_sq <= *distance_sq {\n                        return false;", "goals: run away in goal boundary"),
    (GOALS, "if dist_sq < *distance_sq {\n                        return false;", "if dist_sq > *distance_sq {\n                        return false;", "goals: run away in goal inverted"),
    (GOALS, "if goal.is_in_goal(x, y, z, s) {\n                        return true;\n                    }\n                }\n                false", "if !goal.is_in_goal(x, y, z, s) {\n                        return true;\n                    }\n                }\n                false", "goals: composite in goal inverted"),
    (GOALS, "if goal.is_in_goal(x, y, z, s) {\n                        return true;\n                    }\n                }\n                false", "if goal.is_in_goal(x, y, z, s) {\n                        return true;\n                    }\n                }\n                true", "goals: composite in goal default true"),
    (GOALS, "Goal::Inverted(_) => false,", "Goal::Inverted(_) => true,", "goals: inverted in goal"),
    (GOALS, "// `GoalBlock.heuristic`\n            Goal::Block { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);", "// `GoalBlock.heuristic`\n            Goal::Block { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_add(*gx);", "goals: block heuristic x sign"),
    (GOALS, "// `GoalBlock.heuristic`\n            Goal::Block { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);", "// `GoalBlock.heuristic`\n            Goal::Block { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = gy.wrapping_sub(y);", "goals: block heuristic y sign"),
    (GOALS, "// `GoalBlock.heuristic`\n            Goal::Block { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);", "// `GoalBlock.heuristic`\n            Goal::Block { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_add(*gz);", "goals: block heuristic z sign"),
    (GOALS, "// `GoalBlock.heuristic`\n            Goal::Block { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, y_diff, z_diff as f64, s)", "// `GoalBlock.heuristic`\n            Goal::Block { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, 0, z_diff as f64, s)", "goals: block heuristic y dropped"),
    (GOALS, "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);", "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_add(*gx);", "goals: near heuristic x sign"),
    (GOALS, "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);", "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = gy.wrapping_sub(y);", "goals: near heuristic y sign"),
    (GOALS, "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);", "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_add(*gz);", "goals: near heuristic z sign"),
    (GOALS, "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, y_diff, z_diff as f64, s)", "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, 0, z_diff as f64, s)", "goals: near heuristic y dropped"),
    (GOALS, "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, y_diff, z_diff as f64, s)", "// `GoalNear.heuristic(int,int,int)`\n            Goal::Near { x: gx, y: gy, z: gz, .. } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, if y_diff < 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)", "goals: near heuristic y offset like get to block"),
    (GOALS, "// `GoalXZ.heuristic`\n            Goal::Xz { x: gx, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_xz_calculate(x_diff as f64, z_diff as f64, s)", "// `GoalXZ.heuristic`\n            Goal::Xz { x: gx, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_xz_calculate(x_diff as f64, 0.0, s)", "goals: xz heuristic z dropped"),
    (GOALS, "// `GoalXZ.heuristic`\n            Goal::Xz { x: gx, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_xz_calculate(x_diff as f64, z_diff as f64, s)", "// `GoalXZ.heuristic`\n            Goal::Xz { x: gx, z: gz } => {\n                let x_diff = x.wrapping_add(*gx);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_xz_calculate(x_diff as f64, z_diff as f64, s)", "goals: xz heuristic x sign"),
    (GOALS, "Goal::YLevel { level } => goal_y_level_calculate(*level, y),", "Goal::YLevel { level } => goal_y_level_calculate(y, *level),", "goals: y level heuristic arguments swapped"),
    (GOALS, "// `GoalTwoBlocks.heuristic`\n            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, if y_diff < 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)", "// `GoalTwoBlocks.heuristic`\n            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, y_diff, z_diff as f64, s)", "goals: two blocks heuristic y offset"),
    (GOALS, "// `GoalTwoBlocks.heuristic`\n            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, if y_diff < 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)", "// `GoalTwoBlocks.heuristic`\n            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, if y_diff > 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)", "goals: two blocks heuristic y offset above"),
    (GOALS, "// `GoalTwoBlocks.heuristic`\n            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);", "// `GoalTwoBlocks.heuristic`\n            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = gy.wrapping_sub(y);", "goals: two blocks heuristic y sign"),
    (GOALS, "// `GoalTwoBlocks.heuristic`\n            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);", "// `GoalTwoBlocks.heuristic`\n            Goal::TwoBlocks { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_add(*gx);", "goals: two blocks heuristic x sign"),
    (GOALS, "// `GoalGetToBlock.heuristic`\n            Goal::GetToBlock { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, if y_diff < 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)", "// `GoalGetToBlock.heuristic`\n            Goal::GetToBlock { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, y_diff, z_diff as f64, s)", "goals: get to block heuristic y offset"),
    (GOALS, "// `GoalGetToBlock.heuristic`\n            Goal::GetToBlock { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, if y_diff < 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)", "// `GoalGetToBlock.heuristic`\n            Goal::GetToBlock { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);\n                goal_block_calculate(x_diff as f64, if y_diff > 0 { y_diff + 1 } else { y_diff }, z_diff as f64, s)", "goals: get to block heuristic y offset above"),
    (GOALS, "// `GoalGetToBlock.heuristic`\n            Goal::GetToBlock { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);", "// `GoalGetToBlock.heuristic`\n            Goal::GetToBlock { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = gy.wrapping_sub(y);", "goals: get to block heuristic y sign"),
    (GOALS, "// `GoalGetToBlock.heuristic`\n            Goal::GetToBlock { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_sub(*gz);", "// `GoalGetToBlock.heuristic`\n            Goal::GetToBlock { x: gx, y: gy, z: gz } => {\n                let x_diff = x.wrapping_sub(*gx);\n                let y_diff = y.wrapping_sub(*gy);\n                let z_diff = z.wrapping_add(*gz);", "goals: get to block heuristic z sign"),
    (GOALS, "let x = x.wrapping_abs();\n                let z = z.wrapping_abs();", "let x = x;\n                let z = z.wrapping_abs();", "goals: axis heuristic x without abs"),
    (GOALS, "let x = x.wrapping_abs();\n                let z = z.wrapping_abs();", "let x = x.wrapping_abs();\n                let z = z;", "goals: axis heuristic z without abs"),
    (GOALS, "let shrt = x.min(z);", "let shrt = x.max(z);", "goals: axis heuristic short is max"),
    (GOALS, "let lng = x.max(z);", "let lng = x.min(z);", "goals: axis heuristic long is min"),
    (GOALS, "let diff = lng.wrapping_sub(shrt);", "let diff = shrt.wrapping_sub(lng);", "goals: axis heuristic diff sign"),
    (GOALS, "java_min(x as f64, java_min(z as f64, diff as f64 * SQRT_2_OVER_2))", "java_min(x as f64, diff as f64 * SQRT_2_OVER_2)", "goals: axis heuristic z distance"),
    (GOALS, "java_min(x as f64, java_min(z as f64, diff as f64 * SQRT_2_OVER_2))", "java_min(z as f64, diff as f64 * SQRT_2_OVER_2)", "goals: axis heuristic x distance"),
    (GOALS, "java_min(x as f64, java_min(z as f64, diff as f64 * SQRT_2_OVER_2))", "java_min(x as f64, z as f64)", "goals: axis heuristic diagonal distance"),
    (GOALS, "java_min(x as f64, java_min(z as f64, diff as f64 * SQRT_2_OVER_2))", "(x as f64).min((z as f64).min(diff as f64 * SQRT_2_OVER_2))", "goals: axis heuristic Rust min"),
    (GOALS, "java_min(x as f64, java_min(z as f64, diff as f64 * SQRT_2_OVER_2))", "java_max(x as f64, java_max(z as f64, diff as f64 * SQRT_2_OVER_2))", "goals: axis heuristic max"),
    (GOALS, "let flat_axis_distance = java_min(x as f64, java_min(z as f64, diff as f64 * SQRT_2_OVER_2));", "let flat_axis_distance = java_min(x as f64, java_min(z as f64, diff as f64 * SQRT_2));", "goals: axis heuristic diagonal factor"),
    (GOALS, "flat_axis_distance * s.cost_heuristic + goal_y_level_calculate(s.axis_height, y)", "flat_axis_distance + goal_y_level_calculate(s.axis_height, y)", "goals: axis heuristic cost heuristic"),
    (GOALS, "flat_axis_distance * s.cost_heuristic + goal_y_level_calculate(s.axis_height, y)", "flat_axis_distance * s.cost_heuristic + goal_y_level_calculate(y, s.axis_height)", "goals: axis heuristic y arguments swapped"),
    (GOALS, "flat_axis_distance * s.cost_heuristic + goal_y_level_calculate(s.axis_height, y)", "flat_axis_distance * s.cost_heuristic", "goals: axis heuristic y dropped"),
    (GOALS, "const SQRT_2_OVER_2: f64 = SQRT_2 / 2.0;", "const SQRT_2_OVER_2: f64 = SQRT_2 / 2.0001;", "goals: axis constant"),
    (GOALS, "let distance_from_start_in_desired_direction = x.wrapping_sub(*gx).wrapping_mul(*dx).wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dz));", "let distance_from_start_in_desired_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx));", "goals: strict desired direction swapped"),
    (GOALS, "let distance_from_start_in_desired_direction = x.wrapping_sub(*gx).wrapping_mul(*dx).wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dz));", "let distance_from_start_in_desired_direction = x.wrapping_sub(*gx).wrapping_mul(*dx);", "goals: strict desired direction z dropped"),
    (GOALS, "let distance_from_start_in_desired_direction = x.wrapping_sub(*gx).wrapping_mul(*dx).wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dz));", "let distance_from_start_in_desired_direction = z.wrapping_sub(*gz).wrapping_mul(*dz);", "goals: strict desired direction x dropped"),
    (GOALS, "let distance_from_start_in_desired_direction = x.wrapping_sub(*gx).wrapping_mul(*dx).wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dz));", "let distance_from_start_in_desired_direction = gx.wrapping_sub(x).wrapping_mul(*dx).wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dz));", "goals: strict desired direction x sign"),
    (GOALS, "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_abs().wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx).wrapping_abs());", "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dx).wrapping_abs().wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dz).wrapping_abs());", "goals: strict incorrect direction swapped"),
    (GOALS, "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_abs().wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx).wrapping_abs());", "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx).wrapping_abs());", "goals: strict incorrect x without abs"),
    (GOALS, "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_abs().wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx).wrapping_abs());", "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_abs().wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx));", "goals: strict incorrect z without abs"),
    (GOALS, "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_abs().wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx).wrapping_abs());", "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_abs();", "goals: strict incorrect z dropped"),
    (GOALS, "let distance_from_start_in_incorrect_direction = x.wrapping_sub(*gx).wrapping_mul(*dz).wrapping_abs().wrapping_add(z.wrapping_sub(*gz).wrapping_mul(*dx).wrapping_abs());", "let distance_from_start_in_incorrect_direction = z.wrapping_sub(*gz).wrapping_mul(*dx).wrapping_abs();", "goals: strict incorrect x dropped"),
    (GOALS, "let vertical_distance_from_start = y.wrapping_sub(*gy).wrapping_abs();", "let vertical_distance_from_start = y.wrapping_sub(*gy);", "goals: strict vertical without abs"),
    (GOALS, "let mut heuristic = distance_from_start_in_desired_direction.wrapping_neg().wrapping_mul(100) as f64;", "let mut heuristic = distance_from_start_in_desired_direction.wrapping_mul(100) as f64;", "goals: strict desired sign"),
    (GOALS, "let mut heuristic = distance_from_start_in_desired_direction.wrapping_neg().wrapping_mul(100) as f64;", "let mut heuristic = distance_from_start_in_desired_direction.wrapping_neg().wrapping_mul(10) as f64;", "goals: strict desired weight"),
    (GOALS, "heuristic += distance_from_start_in_incorrect_direction.wrapping_mul(1000) as f64;", "heuristic += distance_from_start_in_incorrect_direction.wrapping_mul(100) as f64;", "goals: strict incorrect weight"),
    (GOALS, "heuristic += vertical_distance_from_start.wrapping_mul(1000) as f64;", "heuristic += vertical_distance_from_start.wrapping_mul(100) as f64;", "goals: strict vertical weight"),
    (GOALS, "heuristic += distance_from_start_in_incorrect_direction.wrapping_mul(1000) as f64;", "heuristic += distance_from_start_in_incorrect_direction.saturating_mul(1000) as f64;", "goals: strict incorrect saturating"),
    (GOALS, "let mut heuristic = distance_from_start_in_desired_direction.wrapping_neg().wrapping_mul(100) as f64;", "let mut heuristic = distance_from_start_in_desired_direction.saturating_neg().saturating_mul(100) as f64;", "goals: strict desired saturating"),
    (GOALS, "let mut min = f64::MAX;\n                for p in from {", "let mut min = 0.0;\n                for p in from {", "goals: run away heuristic start"),
    (GOALS, "for p in from {\n                    let h = goal_xz_calculate(p.0.wrapping_sub(x) as f64, p.2.wrapping_sub(z) as f64, s);\n                    if h < min {", "for p in from {\n                    let h = goal_xz_calculate(p.0.wrapping_sub(x) as f64, p.2.wrapping_sub(z) as f64, s);\n                    if h > min {", "goals: run away heuristic keeps the largest"),
    (GOALS, "p.2.wrapping_sub(z) as f64, s);", "p.1.wrapping_sub(z) as f64, s);", "goals: run away heuristic z from y"),
    (GOALS, "goal_xz_calculate(p.0.wrapping_sub(x) as f64, p.2.wrapping_sub(z) as f64, s);", "goal_xz_calculate(p.2.wrapping_sub(x) as f64, p.0.wrapping_sub(z) as f64, s);", "goals: run away heuristic x and z swapped"),
    (GOALS, "min = -min;", "", "goals: run away heuristic sign"),
    (GOALS, "min = min * 0.6 + goal_y_level_calculate(*my, y) * 1.5;", "min = min * 0.5 + goal_y_level_calculate(*my, y) * 1.5;", "goals: run away heuristic xz weight"),
    (GOALS, "min = min * 0.6 + goal_y_level_calculate(*my, y) * 1.5;", "min = min * 0.6 + goal_y_level_calculate(*my, y) * 1.0;", "goals: run away heuristic y weight"),
    (GOALS, "min = min * 0.6 + goal_y_level_calculate(*my, y) * 1.5;", "min = min * 0.6 + goal_y_level_calculate(y, *my) * 1.5;", "goals: run away heuristic y arguments swapped"),
    (GOALS, "min = min * 0.6 + goal_y_level_calculate(*my, y) * 1.5;", "min = min * 0.6;", "goals: run away heuristic y dropped"),
    (GOALS, "min = min * 0.6 + goal_y_level_calculate(*my, y) * 1.5;", "min = min + goal_y_level_calculate(*my, y) * 1.5;", "goals: run away heuristic xz factor dropped"),
    (GOALS, "min = java_min(min, g.heuristic(x, y, z, s)); // whichever is closest", "min = min.min(g.heuristic(x, y, z, s)); // whichever is closest", "goals: composite heuristic Rust min"),
    (GOALS, "min = java_min(min, g.heuristic(x, y, z, s)); // whichever is closest", "min = java_max(min, g.heuristic(x, y, z, s)); // whichever is closest", "goals: composite heuristic max"),
    (GOALS, "let mut min = f64::MAX;\n                for g in goals {\n                    min = java_min(min, g.heuristic(x, y, z, s));", "let mut min = f64::INFINITY;\n                for g in goals {\n                    min = java_min(min, g.heuristic(x, y, z, s));", "goals: composite heuristic start infinity"),
    (GOALS, "min = java_min(min, g.heuristic_inside(s));", "min = java_max(min, g.heuristic_inside(s));", "goals: composite inside max"),
    (GOALS, "min = java_min(min, g.heuristic_inside(s));", "min = min.min(g.heuristic_inside(s));", "goals: composite inside Rust min"),
    (GOALS, "let mut min = f64::MAX;\n                for g in goals {\n                    // just take", "let mut min = f64::INFINITY;\n                for g in goals {\n                    // just take", "goals: composite inside start infinity"),
    (GOALS, "Goal::Inverted(origin) => -origin.heuristic(x, y, z, s),", "Goal::Inverted(origin) => origin.heuristic(x, y, z, s),", "goals: inverted heuristic sign"),
    (GOALS, "let range = (*range_sq as f64).sqrt().ceil() as i32;", "let range = (*range_sq as f64).sqrt().floor() as i32;", "goals: near inside range floor"),
    (GOALS, "let range = (*range_sq as f64).sqrt().ceil() as i32;", "let range = (*range_sq as f64).sqrt() as i32;", "goals: near inside range truncated"),
    (GOALS, "for dx in -range..=range {", "for dx in -range..range {", "goals: near inside x range"),
    (GOALS, "for dy in -range..=range {", "for dy in -range..range {", "goals: near inside y range"),
    (GOALS, "for dz in -range..=range {", "for dz in -range..range {", "goals: near inside z range"),
    (GOALS, "let h = self.heuristic(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s);", "let h = self.heuristic(gx.wrapping_add(dx), *gy, gz.wrapping_add(dz), s);", "goals: near inside heuristic y"),
    (GOALS, "let h = self.heuristic(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s);", "let h = self.heuristic(gx.wrapping_sub(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s);", "goals: near inside heuristic x sign"),
    (GOALS, "if h < min_outside && self.is_in_goal(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s) {", "if h < min_outside || self.is_in_goal(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s) {", "goals: near inside or"),
    (GOALS, "if h < min_outside && self.is_in_goal(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s) {", "if h <= min_outside && self.is_in_goal(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s) {", "goals: near inside boundary"),
    (GOALS, "if h < min_outside && self.is_in_goal(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s) {", "if self.is_in_goal(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s) {", "goals: near inside minimum check dropped"),
    (GOALS, "if h < min_outside && self.is_in_goal(gx.wrapping_add(dx), gy.wrapping_add(dy), gz.wrapping_add(dz), s) {", "if h < min_outside {", "goals: near inside goal check dropped"),
    (GOALS, "} else {\n                                min_outside = java_min(min_outside, h);\n                            }\n                        }\n                    }\n                }\n                max_inside(&maybe_always_inside, min_outside)\n            }\n            // `GoalStrictDirection.heuristic()`", "} else {\n                                min_outside = min_outside.min(h);\n                            }\n                        }\n                    }\n                }\n                max_inside(&maybe_always_inside, min_outside)\n            }\n            // `GoalStrictDirection.heuristic()`", "goals: near inside outside min is the Rust min"),
    (GOALS, "} else {\n                                min_outside = java_min(min_outside, h);\n                            }\n                        }\n                    }\n                }\n                max_inside(&maybe_always_inside, min_outside)\n            }\n            // `GoalStrictDirection.heuristic()`", "} else {\n                            }\n                        }\n                    }\n                }\n                max_inside(&maybe_always_inside, min_outside)\n            }\n            // `GoalStrictDirection.heuristic()`", "goals: near inside outside min dropped"),
    (GOALS, "Goal::StrictDirection { .. } => f64::NEG_INFINITY,", "Goal::StrictDirection { .. } => 0.0,", "goals: strict inside heuristic"),
    (GOALS, "let distance = (*distance_sq as f64).sqrt().ceil() as i32;", "let distance = (*distance_sq as f64).sqrt().floor() as i32;", "goals: run away inside distance floor"),
    (GOALS, "min_x = min_x.min(p.0.wrapping_sub(distance));", "min_x = min_x.min(p.0.wrapping_add(distance));", "goals: run away inside min x sign"),
    (GOALS, "min_y = min_y.min(p.1.wrapping_sub(distance));", "min_y = min_y.min(p.1.wrapping_add(distance));", "goals: run away inside min y sign"),
    (GOALS, "min_z = min_z.min(p.2.wrapping_sub(distance));", "min_z = min_z.min(p.2.wrapping_add(distance));", "goals: run away inside min z sign"),
    (GOALS, "max_x = max_x.max(p.0.wrapping_add(distance));", "max_x = max_x.max(p.0.wrapping_sub(distance));", "goals: run away inside max x sign"),
    (GOALS, "max_y = max_y.max(p.1.wrapping_add(distance));", "max_y = max_y.max(p.1.wrapping_sub(distance));", "goals: run away inside max y sign"),
    (GOALS, "max_z = max_z.max(p.2.wrapping_add(distance));", "max_z = max_z.max(p.2.wrapping_sub(distance));", "goals: run away inside max z sign"),
    (GOALS, "min_x = min_x.min(p.0.wrapping_sub(distance));", "min_x = min_x.max(p.0.wrapping_sub(distance));", "goals: run away inside min x is max"),
    (GOALS, "max_z = max_z.max(p.2.wrapping_add(distance));", "max_z = max_z.min(p.2.wrapping_add(distance));", "goals: run away inside max z is min"),
    (GOALS, "min_y = min_y.min(p.1.wrapping_sub(distance));", "min_y = min_y.min(p.0.wrapping_sub(distance));", "goals: run away inside min y from x"),
    (GOALS, "for x in min_x..=max_x {", "for x in min_x..max_x {", "goals: run away inside x range"),
    (GOALS, "for y in min_y..=max_y {", "for y in min_y..max_y {", "goals: run away inside y range"),
    (GOALS, "for z in min_z..=max_z {", "for z in min_z..max_z {", "goals: run away inside z range"),
    (GOALS, "let h = self.heuristic(x, y, z, s);\n                            if h < min_outside && self.is_in_goal(x, y, z, s) {", "let h = self.heuristic(x, y, z, s);\n                            if h < min_outside || self.is_in_goal(x, y, z, s) {", "goals: run away inside or"),
    (GOALS, "let h = self.heuristic(x, y, z, s);\n                            if h < min_outside && self.is_in_goal(x, y, z, s) {", "let h = self.heuristic(x, y, z, s);\n                            if h <= min_outside && self.is_in_goal(x, y, z, s) {", "goals: run away inside boundary"),
    (GOALS, "let h = self.heuristic(x, y, z, s);\n                            if h < min_outside && self.is_in_goal(x, y, z, s) {", "let h = self.heuristic(x, y, z, s);\n                            if self.is_in_goal(x, y, z, s) {", "goals: run away inside minimum check dropped"),
    (GOALS, "let h = self.heuristic(x, y, z, s);\n                            if h < min_outside && self.is_in_goal(x, y, z, s) {", "let h = self.heuristic(x, y, z, s);\n                            if h < min_outside {", "goals: run away inside goal check dropped"),
    (GOALS, "let h = self.heuristic(x, y, z, s);\n                            if h < min_outside && self.is_in_goal(x, y, z, s) {\n                                maybe_always_inside.push(h);\n                            } else {\n                                min_outside = java_min(min_outside, h);", "let h = self.heuristic(x, y, z, s);\n                            if h < min_outside && self.is_in_goal(x, y, z, s) {\n                                maybe_always_inside.push(h);\n                            } else {\n                                min_outside = min_outside.min(h);", "goals: run away inside outside min is the Rust min"),
    (GOALS, "Goal::Inverted(_) => f64::NEG_INFINITY,", "Goal::Inverted(_) => 0.0,", "goals: inverted inside heuristic"),
    (GOALS, "_ => 0.0,", "_ => 1.0,", "goals: default inside heuristic"),
    (HELPER, "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "normal cube: drop s.bamboo_stalk"),
    (HELPER, "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "s.bamboo_stalk || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "normal cube: drop s.moving_piston"),
    (HELPER, "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "s.bamboo_stalk || s.moving_piston || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "normal cube: drop s.scaffolding"),
    (HELPER, "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.pointed_dripstone || s.amethyst_cluster", "normal cube: drop s.shulker"),
    (HELPER, "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.amethyst_cluster", "normal cube: drop s.pointed_dripstone"),
    (HELPER, "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster", "s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone", "normal cube: drop s.amethyst_cluster"),
    (HELPER, "    s.shape_full\n}", "    true\n}", "normal cube: full shape ignored"),
    (HELPER, "if s.bamboo_stalk || s.moving_piston || s.scaffolding || s.shulker || s.pointed_dripstone || s.amethyst_cluster {\n        return false;", "if false {\n        return false;", "normal cube: exclusions ignored"),
    (HELPER, "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "(s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "avoid walking into: drop !s.fluid_empty"),
    (HELPER, "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "!s.fluid_empty || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "avoid walking into: drop (s.block == MagmaBlock  !c.allow_walk_on_magma)"),
    (HELPER, "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "avoid walking into: drop s.block == Cactus"),
    (HELPER, "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "avoid walking into: drop s.block == SweetBerryBush"),
    (HELPER, "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "avoid walking into: drop s.base_fire"),
    (HELPER, "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "avoid walking into: drop s.block == EndPortal"),
    (HELPER, "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::BubbleColumn", "avoid walking into: drop s.block == Cobweb"),
    (HELPER, "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb || s.block == Block::BubbleColumn", "!s.fluid_empty || (s.block == Block::MagmaBlock && !c.allow_walk_on_magma) || s.block == Block::Cactus || s.block == Block::SweetBerryBush || s.base_fire || s.block == Block::EndPortal || s.block == Block::Cobweb", "avoid walking into: drop s.block == BubbleColumn"),
    (HELPER, "(s.block == Block::MagmaBlock && !c.allow_walk_on_magma)", "(s.block == Block::MagmaBlock)", "avoid walking into: magma setting"),
    (HELPER, "(s.block == Block::MagmaBlock && !c.allow_walk_on_magma)", "(s.block == Block::MagmaBlock && c.allow_walk_on_magma)", "avoid walking into: magma setting inverted"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop s.base_fire"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop b == Tripwire"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop b == Cobweb"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop b == Vine"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop b == Ladder"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop b == Cocoa"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop s.azalea"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop s.door"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop s.gate"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop s.snow"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop !s.fluid_empty"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.end_portal_block || s.skull_floor || s.shulker", "fully passable: drop s.trapdoor"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.skull_floor || s.shulker", "fully passable: drop s.end_portal_block"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.shulker", "fully passable: drop s.skull_floor"),
    (HELPER, "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor || s.shulker", "s.base_fire || b == Block::Tripwire || b == Block::Cobweb || b == Block::Vine || b == Block::Ladder || b == Block::Cocoa || s.azalea || s.door || s.gate || s.snow || !s.fluid_empty || s.trapdoor || s.end_portal_block || s.skull_floor", "fully passable: drop s.shulker"),
    (HELPER, "    let b = s.block;\n    if s.air {\n        return Yes;\n    }\n    if s.base_fire || b == Block::Tripwire", "    let b = s.block;\n    if s.air {\n        return No;\n    }\n    if s.base_fire || b == Block::Tripwire", "fully passable: air"),
    (HELPER, "    if s.pathfindable {\n        Yes\n    } else {\n        No\n    }\n}\n\n/// `MovementHelper.canWalkOnBlockState`", "    if s.pathfindable {\n        No\n    } else {\n        Yes\n    }\n}\n\n/// `MovementHelper.canWalkOnBlockState`", "fully passable: pathfindable inverted"),
    (HELPER, "    if s.pathfindable {\n        Yes\n    } else {\n        No\n    }\n}\n\n/// `MovementHelper.fullyPassableBlockState`", "    if s.pathfindable {\n        No\n    } else {\n        Yes\n    }\n}\n\n/// `MovementHelper.fullyPassableBlockState`", "through: pathfindable inverted"),
    (HELPER, "        None => b.s.pathfindable,", "        None => true,", "fully passable: maybe branch"),
    (HELPER, "    let b = s.block;\n    if s.air {\n        return Yes;\n    }\n    if s.base_fire || b == Block::Cobweb", "    let b = s.block;\n    if s.air {\n        return No;\n    }\n    if s.base_fire || b == Block::Cobweb", "through: air"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop s.base_fire"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop b == EndPortal"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop b == Cocoa"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop b == BubbleColumn"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop s.shulker"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop s.slab"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop s.trapdoor"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop b == HoneyBlock"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop b == EndRod"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "through blocked: drop b == SweetBerryBush"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || s.amethyst_cluster || s.azalea", "through blocked: drop b == PointedDripstone"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.azalea", "through blocked: drop s.amethyst_cluster"),
    (HELPER, "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster || s.azalea", "s.base_fire || b == Block::Cobweb || b == Block::EndPortal || b == Block::Cocoa || s.skull_any || b == Block::BubbleColumn || s.shulker || s.slab || s.trapdoor || b == Block::HoneyBlock || b == Block::EndRod || b == Block::SweetBerryBush || b == Block::PointedDripstone || s.amethyst_cluster", "through blocked: drop s.azalea"),
    (HELPER, "if b == Block::BigDripleaf {\n        return No;", "if false {\n        return No;", "through: big dripleaf"),
    (HELPER, "if b == Block::PowderSnow {\n        return No;", "if false {\n        return No;", "through: powder snow"),
    (HELPER, "if s.door {\n        return if s.wooden_door { Yes } else { No };", "if false {\n        return if s.wooden_door { Yes } else { No };", "through: doors"),
    (HELPER, "if s.snow {\n        return Maybe;", "if s.snow {\n        return Yes;", "through: snow always yes"),
    (HELPER, "if s.snow {\n        return Maybe;", "if s.snow {\n        return No;", "through: snow always no"),
    (HELPER, "if s.carpet {\n        return Maybe;", "if s.carpet {\n        return No;", "through: carpet always no"),
    (HELPER, "        return if s.fluid_amount != 8 { No } else { Maybe };", "        return if s.fluid_amount != 8 { Maybe } else { No };", "through: fluid branches swapped"),
    (HELPER, "        return if s.fluid_amount != 8 { No } else { Maybe };", "        return No;", "through: fluid always no"),
    (HELPER, "if s.cauldron {\n        return No;", "if false {\n        return No;", "through: cauldron blocked"),
    (HELPER, "if s.gate {\n        return Yes;\n    }\n    if s.carpet", "if false {\n        return Yes;\n    }\n    if s.carpet", "through: gate branch"),
    (HELPER, "if is_block_normal_cube(s) && (b != Block::MagmaBlock || c.allow_walk_on_magma) && b != Block::BubbleColumn && b != Block::HoneyBlock {", "if (b != Block::MagmaBlock || c.allow_walk_on_magma) && b != Block::BubbleColumn && b != Block::HoneyBlock {", "walkon: normal cube required"),
    (HELPER, " && b != Block::BubbleColumn && b != Block::HoneyBlock {", " && b != Block::HoneyBlock {", "walkon: bubble column ok"),
    (HELPER, "(b != Block::MagmaBlock || c.allow_walk_on_magma)", "(b != Block::MagmaBlock || !c.allow_walk_on_magma)", "walkon: magma setting inverted"),
    (HELPER, "if s.azalea {\n        return Yes;\n    }\n    if b == Block::Ladder", "if false {\n        return Yes;\n    }\n    if b == Block::Ladder", "walkon: azalea"),
    (HELPER, "if b == Block::Ladder || (is_climbable(s) && c.allow_vines) {", "if (is_climbable(s) && c.allow_vines) {", "walkon: ladder"),
    (HELPER, "if b == Block::Ladder || (is_climbable(s) && c.allow_vines) {", "if b == Block::Ladder || (is_climbable(s) && !c.allow_vines) {", "walkon: vines setting inverted"),
    (HELPER, "if b == Block::Farmland || b == Block::DirtPath || b == Block::SoulSand {", "if b == Block::DirtPath || b == Block::SoulSand {", "walkon: farmland"),
    (HELPER, "if b == Block::Farmland || b == Block::DirtPath || b == Block::SoulSand {", "if b == Block::Farmland || b == Block::SoulSand {", "walkon: dirt path"),
    (HELPER, "if b == Block::Farmland || b == Block::DirtPath || b == Block::SoulSand {", "if b == Block::Farmland || b == Block::DirtPath {", "walkon: soul sand"),
    (HELPER, "b == Block::EnderChest || b == Block::Chest || b == Block::TrappedChest", "b == Block::Chest || b == Block::TrappedChest", "walkon: ender chest"),
    (HELPER, "b == Block::EnderChest || b == Block::Chest || b == Block::TrappedChest", "b == Block::EnderChest || b == Block::TrappedChest", "walkon: plain chest"),
    (HELPER, "b == Block::EnderChest || b == Block::Chest || b == Block::TrappedChest", "b == Block::EnderChest || b == Block::Chest", "walkon: trapped chest"),
    (HELPER, "if b == Block::Glass || s.stained_glass {", "if s.stained_glass {", "walkon: glass"),
    (HELPER, "if b == Block::Glass || s.stained_glass {", "if b == Block::Glass {", "walkon: stained glass"),
    (HELPER, "if is_water(s) && !is_waterlogged_solid_top(s) {\n        return Maybe;", "if is_water(s) {\n        return Maybe;", "walkon: waterlogged top"),
    (HELPER, "if is_water(s) && !is_waterlogged_solid_top(s) {\n        return Maybe;", "if false {\n        return Maybe;", "walkon: water maybe"),
    (HELPER, "if is_lava(s) && c.assume_walk_on_lava {\n        return Maybe;", "if is_lava(s) {\n        return Maybe;", "walkon: lava setting"),
    (HELPER, "if is_lava(s) && c.assume_walk_on_lava {\n        return Maybe;", "if is_lava(s) && !c.assume_walk_on_lava {\n        return Maybe;", "walkon: lava setting inverted"),
    (HELPER, "if is_lava(s) && c.assume_walk_on_lava {\n        return Maybe;", "if is_lava(s) && c.assume_walk_on_lava {\n        return Yes;", "walkon: lava always yes"),
    (HELPER, "return if s.slab_type != SlabType::Bottom { Yes } else { No };", "return if s.slab_type != SlabType::Bottom { No } else { Yes };", "walkon: slab branches swapped"),
    (HELPER, "return if s.slab_type != SlabType::Bottom { Yes } else { No };", "return if s.slab_type == SlabType::Double { Yes } else { No };", "walkon: top slab"),
    (HELPER, "        return Yes;\n    }\n    No\n}", "        return Yes;\n    }\n    Yes\n}", "walkon: default"),
    (HELPER, "s.fluid == Fluid::Water || s.fluid == Fluid::FlowingWater\n}", "s.fluid == Fluid::Water\n}", "is_water: flowing water"),
    (HELPER, "s.fluid == Fluid::Lava || s.fluid == Fluid::FlowingLava\n}", "s.fluid == Fluid::Lava\n}", "is_lava: flowing lava"),
    (HELPER, "s.fluid == Fluid::Lava || s.fluid == Fluid::FlowingLava\n}", "s.fluid == Fluid::FlowingLava\n}", "is_lava: source lava"),
    (HELPER, "s.fluid == Fluid::Water || s.fluid == Fluid::FlowingWater\n}", "s.fluid == Fluid::FlowingWater\n}", "is_water: source water"),
    (HELPER, "pub fn is_liquid(s: &State) -> bool {\n    !s.fluid_empty", "pub fn is_liquid(s: &State) -> bool {\n    s.fluid != Fluid::None && s.fluid != Fluid::FlowingLava", "is_liquid: flowing lava"),
    (HELPER, "Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "climbable: drop Ladder"),
    (HELPER, "Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "Block::Ladder | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "climbable: drop Vine"),
    (HELPER, "Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "Block::Ladder | Block::Vine | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "climbable: drop WeepingVines"),
    (HELPER, "Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "Block::Ladder | Block::Vine | Block::WeepingVines | Block::TwistingVines | Block::TwistingVinesPlant", "climbable: drop WeepingVinesPlant"),
    (HELPER, "Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVinesPlant", "climbable: drop TwistingVines"),
    (HELPER, "Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines | Block::TwistingVinesPlant", "Block::Ladder | Block::Vine | Block::WeepingVines | Block::WeepingVinesPlant | Block::TwistingVines", "climbable: drop TwistingVinesPlant"),
    (HELPER, "s.slab && s.slab_type == SlabType::Bottom", "s.slab", "bottom slab: type ignored"),
    (HELPER, "s.fluid_flowing_kind && s.fluid_amount != 8", "s.fluid_flowing_kind", "possibly flowing: amount ignored"),
    (HELPER, "s.fluid_flowing_kind && s.fluid_amount != 8", "s.fluid_amount != 8", "possibly flowing: kind ignored"),
    (HELPER, "s.air || s.block == Block::Lava || s.block == Block::Water", "s.air || s.block == Block::Water", "transparent: lava"),
    (HELPER, "s.air || s.block == Block::Lava || s.block == Block::Water", "s.air || s.block == Block::Lava", "transparent: water"),
    (HELPER, "s.waterlogged && s.top_sturdy", "s.waterlogged", "waterlogged top: sturdy ignored"),
    (HELPER, "s.waterlogged && s.top_sturdy", "s.top_sturdy", "waterlogged top: waterlogged ignored"),
    (HELPER, "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "(x as f64) >= self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "border: min x boundary"),
    (HELPER, "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "(x as f64) > self.min_x && (x as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "border: max x one block in"),
    (HELPER, "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) >= self.min_z && ((z + 1) as f64) < self.max_z", "border: min z boundary"),
    (HELPER, "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && (z as f64) < self.max_z", "border: max z one block in"),
    (HELPER, "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "(x as f64) > self.min_x && ((x + 1) as f64) <= self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "border: max x boundary"),
    (HELPER, "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) <= self.max_z", "border: max z boundary"),
    (HELPER, "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x", "border: z ignored"),
    (HELPER, "(x as f64) > self.min_x && ((x + 1) as f64) < self.max_x && (z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "(z as f64) > self.min_z && ((z + 1) as f64) < self.max_z", "border: x ignored"),
    (HELPER, "is_block_normal_cube(s) || s.block == Block::Glass || s.stained_glass, never_break", "s.block == Block::Glass || s.stained_glass, never_break", "place against: normal cube"),
    (HELPER, "is_block_normal_cube(s) || s.block == Block::Glass || s.stained_glass, never_break", "is_block_normal_cube(s) || s.stained_glass, never_break", "place against: glass"),
    (HELPER, "is_block_normal_cube(s) || s.block == Block::Glass || s.stained_glass, never_break", "is_block_normal_cube(s) || s.block == Block::Glass, never_break", "place against: stained glass"),
    (HELPER, "never_break: c.blocks_to_disallow_breaking.contains(&s.name) || s.block == Block::Ice || s.infested }", "never_break: s.block == Block::Ice || s.infested }", "never break: disallow list"),
    (HELPER, "never_break: c.blocks_to_disallow_breaking.contains(&s.name) || s.block == Block::Ice || s.infested }", "never_break: c.blocks_to_disallow_breaking.contains(&s.name) || s.block == Block::Ice }", "never break: infested"),
    (HELPER, "never_break: c.blocks_to_disallow_breaking.contains(&s.name) || s.block == Block::Ice || s.infested }", "never_break: c.blocks_to_disallow_breaking.contains(&s.name) || s.infested }", "never break: ice"),
    (HELPER, "if !self.cfg.allow_break && !self.cfg.allow_break_anyway.contains(&s.name) {", "if !self.cfg.allow_break {", "break: allow break anyway list"),
    (HELPER, "if !self.cfg.allow_break && !self.cfg.allow_break_anyway.contains(&s.name) {", "if !self.cfg.allow_break || !self.cfg.allow_break_anyway.contains(&s.name) {", "break: allow break anyway or"),
    (HELPER, "if !self.cfg.allow_break && !self.cfg.allow_break_anyway.contains(&s.name) {", "if !self.cfg.allow_break && self.cfg.allow_break_anyway.contains(&s.name) {", "break: allow break anyway inverted"),
    (HELPER, "if !self.cfg.allow_break && !self.cfg.allow_break_anyway.contains(&s.name) {", "if self.cfg.allow_break_anyway.contains(&s.name) {", "break: allow break ignored with the anyway list"),
    (HELPER, "        return COST_INF;\n        }\n        1.0\n    }", "        return COST_INF;\n        }\n        2.0\n    }", "break: cost multiplier"),
    (HELPER, "if self.avoid_breaking(x, y, z, b) {\n                return COST_INF;", "if false {\n                return COST_INF;", "mining: avoid breaking"),
    (HELPER, "if !b.fluid_empty {\n                return COST_INF;", "if false {\n                return COST_INF;", "mining: fluid cannot be broken"),
    (HELPER, "if mult >= COST_INF {\n                return COST_INF;", "if false {\n                return COST_INF;", "mining: multiplier infinite"),
    (HELPER, "if str_vs_block <= 0.0 {", "if str_vs_block < 0.0 {", "mining: zero strength"),
    (HELPER, "let mut result = 1.0 / str_vs_block;", "let mut result = str_vs_block;", "mining: strength inverted"),
    (HELPER, "result *= mult;", "", "mining: multiplier applied"),
    (HELPER, "if include_falling {\n                let above", "if true {\n                let above", "mining: falling flag ignored"),
    (HELPER, "let above = self.get(x, y + 1, z);\n                if above.falling {", "let above = self.get(x, y + 2, z);\n                if above.falling {", "mining: falling cell offset"),
    (HELPER, "result += self.mining_duration(x, y + 1, z, above, true);", "result += self.mining_duration(x, y + 1, z, above, false);", "mining: falling recursion flag"),
    (HELPER, "if above.falling {\n                    result +=", "if true {\n                    result +=", "mining: falling above ignored"),
    (HELPER, "self.cfg.frost_walker != 0 && s.frost_melt && s.level == 0", "s.frost_melt && s.level == 0", "frost: level setting"),
    (HELPER, "self.cfg.frost_walker != 0 && s.frost_melt && s.level == 0", "self.cfg.frost_walker != 0 && s.level == 0", "frost: melting state"),
    (HELPER, "if is_climbable(s) {\n            return false;", "if false {\n            return false;", "solid: climbable"),
    (HELPER, "if s.stair_half == Half::Top {\n                    return true;", "if false {\n                    return true;", "solid: upside down stair"),
    (HELPER, "} else if s.block == Block::Scaffolding || s.leaves {", "} else if s.leaves {", "solid: scaffolding"),
    (HELPER, "} else if s.block == Block::Scaffolding || s.leaves {", "} else if s.block == Block::Scaffolding {", "solid: leaves"),
    (HELPER, "if self.cfg.assume_walk_on_water {\n                return false;\n            }\n            if self.get(x, y + 1, z).liquid_block {", "if false {\n                return false;\n            }\n            if self.get(x, y + 1, z).liquid_block {", "solid: assume walk on water"),
    (HELPER, "if !s.fluid_empty {\n            if s.slab {", "if s.fluid != Fluid::None {\n            if s.slab {", "solid: fluid kind instead of emptiness"),
    (HELPER, "if self.get(x, y + 1, z).liquid_block {\n                return false;\n            }\n        }\n        true", "if self.get(x, y + 1, z).liquid_block {\n                return false;\n            }\n        }\n        false", "solid: default"),
    (HELPER, "if !self.border.can_place_at(x, z) {\n            return COST_INF;\n        }\n        if !self.cfg.allow_place_source", "if false {\n            return COST_INF;\n        }\n        if !self.cfg.allow_place_source", "place: border"),
    (HELPER, "if !self.cfg.has_throwaway {\n            return COST_INF;", "if false {\n            return COST_INF;", "place: throwaway"),
    (HELPER, "        self.cfg.place_penalty\n    }", "        0.0\n    }", "place: penalty"),
    (HELPER, "self.border.can_place_at(x, z) && self.pre.flags[self.get(x, y, z).id].place_against", "self.pre.flags[self.get(x, y, z).id].place_against", "place against: border"),
    (HELPER, "self.border.can_place_at(x, z) && self.pre.flags[self.get(x, y, z).id].place_against", "self.border.can_place_at(x, z)", "place against: flag"),
    (HELPER, "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "avoid breaking: drop !border.can_place_at(x, z)"),
    (HELPER, "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "!self.border.can_place_at(x, z) || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "avoid breaking: drop pre.flags[b.id].never_break"),
    (HELPER, "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "avoid breaking: drop avoid_adjacent_breaking(x, y + 1, z, true)"),
    (HELPER, "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "avoid breaking: drop avoid_adjacent_breaking(x - 1, y, z, false)"),
    (HELPER, "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "avoid breaking: drop avoid_adjacent_breaking(x, y, z + 1, false)"),
    (HELPER, "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false) || self.avoid_adjacent_breaking(x, y, z - 1, false)", "!self.border.can_place_at(x, z) || self.pre.flags[b.id].never_break || self.avoid_adjacent_breaking(x, y + 1, z, true) || self.avoid_adjacent_breaking(x + 1, y, z, false) || self.avoid_adjacent_breaking(x - 1, y, z, false) || self.avoid_adjacent_breaking(x, y, z + 1, false)", "avoid breaking: drop avoid_adjacent_breaking(x, y, z - 1, false)"),
    (HELPER, "self.avoid_adjacent_breaking(x, y + 1, z, true)", "self.avoid_adjacent_breaking(x, y + 1, z, false)", "avoid breaking: above directly"),
    (HELPER, "if !directly_above && s.falling && self.cfg.avoid_updating_falling && self.get(x, y - 1, z).falling_free {", "if !directly_above && s.falling && self.get(x, y - 1, z).falling_free {", "adjacent: avoid updating falling setting"),
    (HELPER, "if !directly_above && s.falling && self.cfg.avoid_updating_falling && self.get(x, y - 1, z).falling_free {", "if !directly_above && s.falling && self.cfg.avoid_updating_falling {", "adjacent: falling free below"),
    (HELPER, "if !directly_above && s.falling && self.cfg.avoid_updating_falling && self.get(x, y - 1, z).falling_free {", "if s.falling && self.cfg.avoid_updating_falling && self.get(x, y - 1, z).falling_free {", "adjacent: falling directly above"),
    (HELPER, "if s.liquid_block {\n            if directly_above", "if !s.fluid_empty {\n            if directly_above", "adjacent: liquid block"),
    (HELPER, "return !self.get(x, y - 1, z).liquid_block;", "return self.get(x, y - 1, z).liquid_block;", "adjacent: liquid below inverted"),
    (HELPER, "return !self.get(x, y - 1, z).liquid_block;", "return true;", "adjacent: liquid below ignored"),
    (HELPER, "        !s.fluid_empty\n    }\n\n    /// `PrecomputedData.neverBreak` at a position", "        false\n    }\n\n    /// `PrecomputedData.neverBreak` at a position", "adjacent: fluid fallthrough"),
    (HELPER, "if !s.fluid_flowing_kind {\n            return false;", "if false {\n            return false;", "flowing: kind"),
    (HELPER, "if s.fluid_amount != 8 {\n            return true;\n        }\n        possibly_flowing", "if false {\n            return true;\n        }\n        possibly_flowing", "flowing: partial amount"),
    (HELPER, "possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1)) || possibly_flowing(&self.get(x, y, z - 1))", "possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1)) || possibly_flowing(&self.get(x, y, z - 1))", "flowing neighbours: drop possibly_flowing(get(x + 1, y, z))"),
    (HELPER, "possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1)) || possibly_flowing(&self.get(x, y, z - 1))", "possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1)) || possibly_flowing(&self.get(x, y, z - 1))", "flowing neighbours: drop possibly_flowing(get(x - 1, y, z))"),
    (HELPER, "possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1)) || possibly_flowing(&self.get(x, y, z - 1))", "possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z - 1))", "flowing neighbours: drop possibly_flowing(get(x, y, z + 1))"),
    (HELPER, "possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1)) || possibly_flowing(&self.get(x, y, z - 1))", "possibly_flowing(&self.get(x + 1, y, z)) || possibly_flowing(&self.get(x - 1, y, z)) || possibly_flowing(&self.get(x, y, z + 1))", "flowing neighbours: drop possibly_flowing(get(x, y, z - 1))"),
    (HELPER, "if s.carpet {\n            return self.can_walk_on_at(x, y - 1, z);", "if s.carpet {\n            return self.can_walk_on_at(x, y, z);", "through position: carpet floor offset"),
    (HELPER, "if s.carpet {\n            return self.can_walk_on_at(x, y - 1, z);", "if s.carpet {\n            return true;", "through position: carpet floor"),
    (HELPER, "if !self.world.contains_loaded_chunk(x, z) {\n                return true;\n            }\n            if s.layers >= 3 {", "if false {\n                return true;\n            }\n            if s.layers >= 3 {", "through position: unloaded snow"),
    (HELPER, "if !self.world.contains_loaded_chunk(x, z) {\n                return true;\n            }\n            if s.layers >= 3 {", "if !self.world.contains_loaded_chunk(x, z) {\n                return false;\n            }\n            if s.layers >= 3 {", "through position: unloaded snow blocked"),
    (HELPER, "if !self.world.contains_loaded_chunk(x, z) {\n                return true;\n            }\n            if s.layers >= 3 {", "if self.world.contains_loaded_chunk(x, z) {\n                return true;\n            }\n            if s.layers >= 3 {", "through position: loaded snow inverted"),
    (HELPER, "            if s.layers >= 3 {\n                return false;\n            }\n            return self.can_walk_on_at(x, y - 1, z);", "            if s.layers >= 3 {\n                return false;\n            }\n            return true;", "through position: snow floor"),
    (HELPER, "            if s.layers >= 3 {\n                return false;\n            }\n            return self.can_walk_on_at(x, y - 1, z);", "            if s.layers >= 3 {\n                return false;\n            }\n            return self.can_walk_on_at(x, y, z);", "through position: snow floor offset"),
    (HELPER, "            if s.layers >= 3 {\n                return false;", "            if s.layers >= 3 {\n                return true;", "through position: deep snow"),
    (HELPER, "if self.is_flowing(x, y, z, s) {\n                return false;\n            }\n            if self.cfg.assume_walk_on_water {", "if false {\n                return false;\n            }\n            if self.cfg.assume_walk_on_water {", "through position: flowing fluid"),
    (HELPER, "if self.cfg.assume_walk_on_water {\n                return false;\n            }\n            let up", "if false {\n                return false;\n            }\n            let up", "through position: jesus"),
    (HELPER, "let up = self.get(x, y + 1, z);\n            if !up.fluid_empty || up.lily {", "let up = self.get(x, y + 2, z);\n            if !up.fluid_empty || up.lily {", "through position: fluid above offset"),
    (HELPER, "if !up.fluid_empty || up.lily {\n                return false;\n            }\n            return s.fluid", "if up.lily {\n                return false;\n            }\n            return s.fluid", "through position: liquid above"),
    (HELPER, "return s.fluid == Fluid::Water || s.fluid == Fluid::FlowingWater;", "return s.fluid == Fluid::Water;", "through position: flowing water swim"),
    (HELPER, "return s.fluid == Fluid::Water || s.fluid == Fluid::FlowingWater;", "return true;", "through position: lava swim"),
    (HELPER, "        s.pathfindable\n    }\n\n    /// `MovementHelper.canWalkOnPosition`", "        true\n    }\n\n    /// `MovementHelper.canWalkOnPosition`", "through position: pathfindable fallthrough"),
    (HELPER, "if up.block == Block::LilyPad || up.carpet {\n                return true;", "if up.block == Block::LilyPad {\n                return true;", "water pos: carpet"),
    (HELPER, "if self.is_flowing(x, y, z, s) || up.fluid == Fluid::FlowingWater {", "if up.fluid == Fluid::FlowingWater {", "water pos: flowing"),
    (HELPER, "if self.is_flowing(x, y, z, s) || up.fluid == Fluid::FlowingWater {", "if self.is_flowing(x, y, z, s) {", "water pos: flowing water above"),
    (HELPER, "return is_water(&up) && !self.cfg.assume_walk_on_water;", "return is_water(&up);", "water pos: flowing with jesus"),
    (HELPER, "return is_water(&up) && !self.cfg.assume_walk_on_water;", "return !self.cfg.assume_walk_on_water;", "water pos: flowing water above needed"),
    (HELPER, "let up = self.get(x, y + 1, z);\n            if up.block == Block::LilyPad", "let up = self.get(x, y + 2, z);\n            if up.block == Block::LilyPad", "water pos: cell above offset"),
    (HELPER, "if is_lava(s) && !self.is_flowing(x, y, z, s) && self.cfg.assume_walk_on_lava {", "if is_lava(s) && self.cfg.assume_walk_on_lava {", "lava pos: flowing"),
    (HELPER, "if is_lava(s) && !self.is_flowing(x, y, z, s) && self.cfg.assume_walk_on_lava {", "if is_lava(s) && !self.is_flowing(x, y, z, s) {", "lava pos: setting"),
    (HELPER, "if is_lava(s) && !self.is_flowing(x, y, z, s) && self.cfg.assume_walk_on_lava {", "if is_lava(s) && !self.is_flowing(x, y, z, s) && !self.cfg.assume_walk_on_lava {", "lava pos: setting inverted"),
    (HELPER, "if is_lava(s) && !self.is_flowing(x, y, z, s) && self.cfg.assume_walk_on_lava {", "if !self.is_flowing(x, y, z, s) && self.cfg.assume_walk_on_lava {", "lava pos: lava required"),
    (HELPER, "let id = if rel < 0 || rel >= self.height {", "let id = if rel <= 0 || rel >= self.height {", "get: lowest row"),
    (HELPER, "let id = if rel < 0 || rel >= self.height {", "let id = if rel < 0 || rel > self.height {", "get: highest row"),
    (HELPER, "let id = if rel < 0 || rel >= self.height {", "let id = if rel >= self.height {", "get: below the world"),
    (HELPER, "let id = if rel < 0 || rel >= self.height {", "let id = if rel < 0 {", "get: above the world"),
    (HELPER, "water_speed = java_min(water_speed, SWIM_ONE_BLOCK_COST);", "water_speed = java_max(water_speed, SWIM_ONE_BLOCK_COST);", "water speed: swim min"),
    (HELPER, "WALK_ONE_IN_WATER_COST * ((1.0f32 - mult) as f64) + WALK_ONE_BLOCK_COST * (mult as f64)", "WALK_ONE_IN_WATER_COST * ((1.0f32 - mult) as f64)", "water speed: walk part"),
    (HELPER, "WALK_ONE_IN_WATER_COST * ((1.0f32 - mult) as f64) + WALK_ONE_BLOCK_COST * (mult as f64)", "WALK_ONE_IN_WATER_COST * (mult as f64) + WALK_ONE_BLOCK_COST * ((1.0f32 - mult) as f64)", "water speed: parts swapped"),
    (HELPER, "max_y: min_y + height - 1,", "max_y: min_y + height,", "context: max y"),
    (HELPER, "if s.layers >= 3 {", "if s.layers > 3 {", "snow: 3 layers boundary"),
    (HELPER, "if s.air {\n            return true;\n        }\n        if s.snow {\n            if !self.world.contains_loaded_chunk(x, z) {", "if false {\n            return true;\n        }\n        if s.snow {\n            if !self.world.contains_loaded_chunk(x, z) {", "replace: air"),
    (HELPER, "if !self.world.contains_loaded_chunk(x, z) {\n                return true;\n            }\n            return s.layers == 1;", "if false {\n                return true;\n            }\n            return s.layers == 1;", "replace: unloaded snow"),
    (HELPER, "if !self.world.contains_loaded_chunk(x, z) {\n                return true;\n            }\n            return s.layers == 1;", "if !self.world.contains_loaded_chunk(x, z) {\n                return false;\n            }\n            return s.layers == 1;", "replace: unloaded snow refused"),
    (HELPER, "if !self.world.contains_loaded_chunk(x, z) {\n                return true;\n            }\n            return s.layers == 1;", "if self.world.contains_loaded_chunk(x, z) {\n                return true;\n            }\n            return s.layers == 1;", "replace: loaded snow inverted"),
    (HELPER, "        s.replaceable\n    }", "        false\n    }", "replace: flag"),
]

RUN_TIMEOUT = 300  # seconds; the unmutated comparison takes under a minute on 10000 grids
ABORTED = {"aborted": (0, 0)}  # what build_and_run returns when the comparison itself exits with an error
COPIED = ["Cargo.toml", "Cargo.lock", "rustfmt.toml", "mod/engine/Cargo.toml", "mod/engine/src", "tools/cost-compare/Cargo.toml", "tools/cost-compare/src", "tools/search-compare/Cargo.toml", "tools/search-compare/src"]
FAMILY = re.compile(r"^(\w+): \d+ evals, \d+ finite, \d+ distinct, exact \d+, within 1e-9 (\d+), WRONG (\d+)\s*$", re.M)


def where(repo):
    for name_file, pat, _, name in [(f, a, b, n) for f, a, b, n in M]:
        with open(os.path.join(repo, name_file), encoding="utf-8") as f:
            text = f.read()
        i = text.find(pat)
        if i < 0:
            print(f"NOT FOUND  {name}  ({name_file})")
            continue
        line = text.count("\n", 0, i) + 1
        fn = re.findall(r"^\s*(?:pub )?fn (\w+)", text[:i], re.M)
        print(f"{name_file}:{line} in {fn[-1] if fn else '-'}: {name}" + ("  (AMBIGUOUS: pattern occurs " + str(text.count(pat)) + " times)" if text.count(pat) > 1 else ""))


def counts(output):
    """{family: (close, wrong)} from the comparison's stdout."""
    return {m.group(1): (int(m.group(2)), int(m.group(3))) for m in FAMILY.finditer(output)}


def low_priority():
    return any(subprocess.run(["pgrep", "-x", p], capture_output=True).returncode == 0 for p in ("horizon-client", "rustc"))


def default_jobs():
    """Half of the threads: the comparison reads a large file, and the laptop stays usable."""
    return max(1, (os.cpu_count() or 2) // 2)


def run_timeout(baseline_seconds):
    """The limit for one mutant's run: an error that makes the engine loop is noticed without waiting 5 minutes. The baselines ran with
    all workers busy, so they already include what the machine can do under this load."""
    return max(60, min(RUN_TIMEOUT, 4 * baseline_seconds))


class ResultCache:
    """Verdicts of mutations already run on exactly these inputs, so a killed or repeated run does not start again.

    A verdict is reused only when the compiler, every file the workers copy and the golden file are byte-identical and the mutation is
    the same text. Editing the engine therefore discards everything. The file lives next to the golden file, outside the repository.
    BARITONE_MUTATE_NO_CACHE=1 turns this off."""

    VERSION = 1

    def __init__(self, repo, copied, golden, tool):
        self.lock = threading.Lock()
        self.results = {}
        self.path = None
        if os.environ.get("BARITONE_MUTATE_NO_CACHE"):
            return
        h = hashlib.sha256(f"{self.VERSION} {tool}\n".encode())
        h.update(subprocess.run(["rustc", "-vV"], capture_output=True).stdout)
        for rel in copied:
            top = os.path.join(repo, rel)
            walked = [(d, n) for d, _, names in os.walk(top) for n in names] if os.path.isdir(top) else [(os.path.dirname(top), os.path.basename(top))]
            for d, n in sorted(walked):
                p = os.path.join(d, n)
                h.update(os.path.relpath(p, repo).encode() + b"\0" + file_hash(p).encode())
        h.update(file_hash(golden).encode())
        folder = os.path.dirname(os.path.abspath(golden))
        prefix = f"mutate-results-{tool}-"
        self.path = os.path.join(folder, prefix + h.hexdigest()[:24] + ".jsonl")
        for name in os.listdir(folder):
            if name.startswith(prefix) and os.path.join(folder, name) != self.path:
                os.remove(os.path.join(folder, name))
        if os.path.exists(self.path):
            with open(self.path, encoding="utf-8") as f:
                for line in f:
                    try:
                        key, value = json.loads(line)
                    except ValueError:
                        continue  # the last line of a killed run
                    self.results[key] = value

    @staticmethod
    def key(mutation):
        return hashlib.sha256(json.dumps(mutation[:3]).encode()).hexdigest()

    def get(self, mutation):
        return self.results.get(self.key(mutation))

    def put(self, mutation, value):
        if self.path:
            with self.lock, open(self.path, "a", encoding="utf-8") as f:
                f.write(json.dumps([self.key(mutation), value]) + "\n")


def file_hash(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def baseline_failed(why):
    print("baseline run failed:", why)
    print("\n0 caught, 0 missed, 1 not applied")
    print("missed: []")
    print("not applied:", ["baseline (" + why + ")"])
    return 1


class Worker:
    """One copy of the workspace with its own target folder, so that workers never see each other's errors."""

    def __init__(self, repo, gold, root, low):
        self.gold = gold
        self.ws = os.path.join(root, "ws")
        for rel in COPIED:
            src, dst = os.path.join(repo, rel), os.path.join(self.ws, rel)
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            if os.path.isdir(src):
                shutil.copytree(src, dst, ignore=shutil.ignore_patterns("target"))
            else:
                shutil.copyfile(src, dst)
        self.env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(root, "target"), CARGO_BUILD_JOBS="2")
        self.prefix = ["nice", "-n", "15"] if low else []
        self.exe = os.path.join(root, "target", "release", "cost-compare")

    def build_and_run(self, args=("quiet",), timeout=RUN_TIMEOUT):
        c = subprocess.run(self.prefix + ["cargo", "build", "--release", "--quiet", "-p", "cost-compare"], cwd=self.ws, env=self.env, capture_output=True, text=True)
        if c.returncode:
            return None, "no build: " + c.stderr[:200]
        try:
            r = subprocess.run(self.prefix + [self.exe, self.gold, *args], capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            # an error that makes the engine loop (a goal's search box turned huge) is noticed too
            return ABORTED, "comparison timed out"
        if r.returncode == -signal.SIGKILL:
            # The machine killed it, most likely for memory. That says nothing about the error, so it must not count as caught.
            return None, "killed from outside"
        if r.returncode:
            # the unmutated run exits 0, so an abort (a failed assertion on the move table, a parse error) is the comparison noticing the error
            return ABORTED, "comparison aborted: " + (r.stdout + r.stderr)[-200:]
        return counts(r.stdout), ""

    def try_mutation(self, mutation, args, timeout):
        file, a, b, _ = mutation
        path = os.path.join(self.ws, file)
        with open(path, encoding="utf-8") as f:
            orig = f.read()
        if orig.count(a) < 1:
            return None, ""
        with open(path, "w", encoding="utf-8") as f:
            f.write(orig.replace(a, b, 1))
        try:
            return self.build_and_run(args, timeout)
        finally:
            with open(path, "w", encoding="utf-8") as f:
                f.write(orig)


def main():
    repo = os.path.abspath(sys.argv[1])
    if len(sys.argv) > 2 and sys.argv[2] == "--where":
        where(repo)
        return 0
    gold = os.path.abspath(sys.argv[2])
    mutations = M
    if len(sys.argv) > 3:
        keep = set(sys.argv[3].split("|"))
        mutations = [m for m in M if m[3] in keep]
    low = low_priority()
    jobs = int(os.environ.get("BARITONE_MUTATE_JOBS") or (2 if low else default_jobs()))
    root = tempfile.mkdtemp(prefix="cost-compare-")
    began = time.time()
    try:
        with ThreadPoolExecutor(jobs) as pool:
            workers = list(pool.map(lambda i: Worker(repo, gold, os.path.join(root, str(i)), low), range(jobs)))
            started = time.time()
            baselines = list(pool.map(lambda w: w.build_and_run(), workers))
            timeout = run_timeout(time.time() - started)
            for base, why in baselines:
                if base is None or base is ABORTED:
                    return baseline_failed(why)
            if any(b[0] != baselines[0][0] for b in baselines):
                return baseline_failed("the workers disagree on the unmutated run")
            base = baselines[0][0]
            print(f"baseline: wrong={sum(w for _, w in base.values())} close={sum(c for c, _ in base.values())} ({len(base)} families), {jobs} workers", flush=True)
            # Stopping at the first wrong result only proves something when the unmutated run has none.
            args = ("quiet", "first") if not any(w for _, w in base.values()) else ("quiet",)
            cache = ResultCache(repo, COPIED, gold, "cost-compare")
            free = queue.SimpleQueue()
            for w in workers:
                free.put(w)

            def judged(got):
                """(verdict, wrong, close) from what a run gave."""
                if got is ABORTED:
                    return "CAUGHT", 0, 0
                return "CAUGHT" if got != base else "MISSED", sum(w for _, w in got.values()), sum(c for c, _ in got.values())

            def one(mutation):
                done = cache.get(mutation)
                if done:
                    return done, ""
                w = free.get()
                try:
                    got, why = w.try_mutation(mutation, args, timeout)
                finally:
                    free.put(w)
                if got is None:
                    return None, why
                verdict = judged(got)
                cache.put(mutation, verdict)
                return verdict, why

            caught, missed, bad = [], [], []
            # map() gives the results in the list's order, so the output does not depend on the number of workers.
            for (file, a, b, name), (got, why) in zip(mutations, pool.map(one, mutations)):
                if got is None:
                    bad.append(name + (" (" + why + ")" if why else ""))
                    continue
                verdict, wrong, close = got
                print(f"{verdict:7s} wrong={wrong:6d} close={close:6d} {name}", flush=True)
                (caught if verdict == "CAUGHT" else missed).append(name)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    print(f"\n{len(caught)} caught, {len(missed)} missed, {len(bad)} not applied")
    print(f"took {time.time() - began:.0f} s with {jobs} workers")
    print("missed:", missed)
    print("not applied:", bad)
    return 0


if __name__ == "__main__":
    sys.exit(main())
