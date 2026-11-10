// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/process/BuilderProcess.java (all of it but the selection, Schematica and
// Litematica parts), src/api/java/baritone/api/process/IBuilderProcess.java, src/main/java/baritone/utils/PathingCommandContext.java
// (as the cost overlay below). LGPL-3.0-or-later, see lib.rs.
//
// `build <file>` and `clearArea`: walks a schematic into the world. The decisions and their order are upstream's: every tick it looks for what is
// still wrong, breaks what is wrong within reach, places what can be placed from where the player stands, and otherwise gives the path a goal
// next to the work. What is new structure: the game is asked its facts through [`BuildFacts`] (placement states, collision, the real ray trace);
// the process owns no game object; and the calculation context it hands the search is a [`CostOverlay`] holding snapshots (docs/deviations.md D44, D45).
//
// A schematic here is a chain of wrappers (substitute, mirror, rotation) built once by `build`. The skip mask, the layer bounds and the origin
// are applied by [`View`] on each lookup with the settings of the tick, which is what upstream's wrappers do (they read the settings live).
use super::custom_goal::{PathingCommand, PathingCommandType};
use super::mine::dist_sq;
use super::pathing::ProcessContext;
use crate::blocks::Direction;
use crate::config::Config;
use crate::costs::COST_INF;
use crate::exec::helper::{is_looking_at, reachable};
use crate::exec::movements::Env;
use crate::exec::state::Controls;
use crate::goals::{goal_place, Goal};
use crate::inventory::{switch_to_best_tool_for, ThrowawayHint};
use crate::look::{calc_rotation_from_vec3d, Rotation, Vec3};
use crate::movement::helper::{Border, CostOverlay};
use crate::pos::Pos;
use crate::raytrace::{infer_sneaking_eye_position, BlockHit};
use crate::schematic::{in_bounds, BlockStates, MaskSchematic, MirroredSchematic, RotatedSchematic, Schematic, StateId, StaticSchematicView, SubstituteSchematic};
use crate::settings::BuildSettings;
use crate::world::WorldView;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

/// `VoxelShape.bounds()` of a block, in the block's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
}

/// A place a stack can be held in: a hotbar slot, a slot of the main inventory or the off hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvSlot {
    Hotbar(usize),
    /// Slot 9 to 35 of `Inventory.items`, which `InventoryBehavior.throwaway` looks at with `allowInventory`.
    Main(usize),
    Offhand,
}

/// What the builder asks the game during its tick, on the game thread. The glue implements it over the live world and player; tests stub it.
pub trait BuildFacts {
    /// Like `BuilderProcess.approxPlaceable(36)`: for main slots 0..36, the `getStateForPlacement` of the stack's `BlockItem` with the hit (player
    /// position, UP, playerFeet); air for an empty stack, a non-block item, or a null state.
    fn approx_placeable(&self) -> Vec<StateId>;
    /// The same for the off hand (`InventoryBehavior.throwaway` checks it).
    fn offhand_placeable(&self) -> StateId;
    /// `((BlockItem) stack.getItem()).getBlock()` in `BlockUtils` form, or `None` when the stack holds no block item.
    fn item_block(&self, slot: InvSlot) -> Option<String>;
    /// Inside `hasAnyItemThatWouldPlace`: the player's view set to `rot`, the `BlockPlaceContext` from `hit`; `None` when the hotbar slot holds no
    /// `BlockItem`, the state is null, or `!canPlace`.
    fn state_for_placement(&self, hotbar_slot: usize, hit: &BlockHit, rot: Rotation) -> Option<StateId>;
    /// `BlockState.canSurvive(level, pos)`.
    fn can_survive(&self, state: StateId, pos: Pos) -> bool;
    /// `BuilderProcess.placementPlausible`: the collision shape is empty, or `Level.isUnobstructed(null, shape moved to pos)`.
    fn placement_plausible(&self, state: StateId, pos: Pos) -> bool;
    /// `getShape(level, pos).bounds()` of the block at `pos`; `None` when the shape is empty.
    fn outline_bounds(&self, pos: Pos) -> Option<Aabb>;
    /// Like `RayTraceUtils.rayTraceTowards(player, rot, reach, true)`: `Level.clip` from `start` (the engine passes `inferSneakingEyePosition`) along
    /// `rot` for `reach`, `OUTLINE`, `Fluid.NONE`. `None` on a miss.
    fn ray_trace(&self, start: Vec3, rot: Rotation, reach: f64) -> Option<BlockHit>;
}

/// Facts for a caller that never builds: air everywhere, nothing placeable, nothing hit.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoBuildFacts;

impl BuildFacts for NoBuildFacts {
    fn approx_placeable(&self) -> Vec<StateId> {
        vec![0; 36]
    }
    fn offhand_placeable(&self) -> StateId {
        0
    }
    fn item_block(&self, _slot: InvSlot) -> Option<String> {
        None
    }
    fn state_for_placement(&self, _hotbar_slot: usize, _hit: &BlockHit, _rot: Rotation) -> Option<StateId> {
        None
    }
    fn can_survive(&self, _state: StateId, _pos: Pos) -> bool {
        false
    }
    fn placement_plausible(&self, _state: StateId, _pos: Pos) -> bool {
        false
    }
    fn outline_bounds(&self, _pos: Pos) -> Option<Aabb> {
        None
    }
    fn ray_trace(&self, _start: Vec3, _rot: Rotation, _reach: f64) -> Option<BlockHit> {
        None
    }
}

/// What a tick of the builder reads besides the world and the player: the game's facts, the settings and the block table.
#[derive(Clone, Copy)]
pub struct BuildEnv<'a> {
    pub facts: &'a dyn BuildFacts,
    pub settings: &'a BuildSettings,
    pub states: &'a Arc<BlockStates>,
}

// ---------------------------------------------------------------- valid, sameBlockstate, containsBlockState

/// `ORIENTATION_PROPS`: the properties `buildIgnoreDirection` ignores, each as a name and a block that has that very `Property` object (`Property`
/// equality is the name and the values it takes). `vine` has the five `PipeBlock` booleans.
const ORIENTATION_PROPS: [(&str, &str); 11] = [("axis", "oak_log"), ("facing", "oak_stairs"), ("half", "oak_stairs"), ("shape", "oak_stairs"), ("north", "vine"), ("east", "vine"), ("south", "vine"), ("west", "vine"), ("up", "vine"), ("open", "oak_trapdoor"), ("half", "oak_trapdoor")];

/// `BuilderProcess.valid`, `sameBlockstate` and `containsBlockState`: what counts as the right block, with the settings of a tick. `liquid[id]` is
/// `block instanceof LiquidBlock`.
#[derive(Clone, Copy)]
pub struct ValidRules<'a> {
    pub states: &'a BlockStates,
    pub liquid: &'a [bool],
    pub settings: &'a BuildSettings,
}

impl ValidRules<'_> {
    fn is_liquid(&self, id: StateId) -> bool {
        self.liquid.get(id).copied().unwrap_or(false)
    }

    /// Whether the property `name` of this state is one of `ORIENTATION_PROPS`, by `Property` identity.
    fn is_orientation(&self, id: StateId, name: &str) -> bool {
        ORIENTATION_PROPS.iter().any(|(prop, reference)| *prop == name && self.states.default_state(reference).is_some_and(|r| self.states.same_property(id, r, name)))
    }

    /// `BuilderProcess.sameBlockstate`: the same block, and the same values of every property that is not ignored.
    pub fn same_blockstate(&self, first: StateId, second: StateId) -> bool {
        if self.states.block(first) != self.states.block(second) {
            return false;
        }
        let ignore_direction = self.settings.build_ignore_direction;
        let ignored = &self.settings.build_ignore_properties;
        if !ignore_direction && ignored.is_empty() {
            return first == second; // early return if no properties are being ignored
        }
        for (name, value) in self.states.props(first) {
            if self.states.prop(second, name) != Some(value.as_str()) && !(ignore_direction && self.is_orientation(first, name)) && !ignored.contains(name) {
                return false;
            }
        }
        true
    }

    /// `BuilderProcess.containsBlockState`: the state in a list, by `sameBlockstate`.
    /// `BuilderProcess.containsBlockState`.
    pub fn contains_block_state(&self, states: &[StateId], state: StateId) -> bool {
        states.iter().any(|&testee| self.same_blockstate(testee, state))
    }

    /// `BuilderProcess.valid`: whether `current` is acceptable where the schematic wants `desired` (`None`: it does not care). `item_verify` is
    /// set when the state is one an item would place, which `buildIgnoreExisting` and `buildValidSubstitutes` do not excuse.
    pub fn valid(&self, current: StateId, desired: Option<StateId>, item_verify: bool) -> bool {
        let Some(desired) = desired else {
            return true;
        };
        let s = self.settings;
        let st = self.states;
        if self.is_liquid(current) && s.ok_if_water {
            return true;
        }
        let current_air = st.is_air(current);
        let desired_air = st.is_air(desired);
        if current_air && desired_air {
            return true;
        }
        if current_air && s.ok_if_air.iter().any(|b| b == st.block(desired)) {
            return true;
        }
        if desired_air && s.build_ignore_blocks.iter().any(|b| b == st.block(current)) {
            return true;
        }
        if !current_air && s.build_ignore_existing && !item_verify {
            return true;
        }
        if !item_verify && s.build_valid_substitutes.iter().find(|(k, _)| k == st.block(desired)).is_some_and(|(_, list)| list.iter().any(|b| b == st.block(current))) {
            return true;
        }
        if current == desired {
            return true;
        }
        self.same_blockstate(current, desired)
    }
}

// ---------------------------------------------------------------- the schematic as the builder sees it

/// The schematic with what the builder puts on it each tick: the origin, the layer bounds once the layer wrapper exists, and the skip mask.
/// `BuilderCalculationContext.getSchematic` and `BuilderProcess.placeAt` read it. Positions given to `get` are the world's; the others are relative.
#[derive(Clone)]
struct View {
    chain: Arc<dyn Schematic>,
    origin: Pos,
    /// The wrapper made when `buildInLayers` is on: the lowest and the highest y that is in, inclusive. Once made it stays (see U37).
    bounds: Option<(i32, i32)>,
    /// `buildSkipBlocks`, of the tick.
    skip: Vec<String>,
    /// Like `BuilderProcess.approxPlaceable`: the 36 states of the main inventory, of the tick.
    approx: Vec<StateId>,
    states: Arc<BlockStates>,
}

impl View {
    /// `ISchematic.inSchematic` of the whole stack: the layer wrapper (if any) over the skip mask over the chain. `x`, `y`, `z` are relative.
    fn in_schematic(&self, x: i32, y: i32, z: i32, current: StateId) -> bool {
        if let Some((min_y, max_y)) = self.bounds {
            // the anonymous `ISchematic`: `ISchematic.super.inSchematic(...) && y >= minYInclusive && y <= maxYInclusive && realSchematic.inSchematic(...)`
            if !in_bounds(&*self.chain, x, y, z) || y < min_y || y > max_y {
                return false;
            }
        }
        // the skip mask: `MaskSchematic.inSchematic` is `schematic.inSchematic && partOfMask`, and `partOfMask` is
        // `!buildSkipBlocks.contains(desiredState(x, y, z, current, emptyList()).getBlock())`. `desiredState` has no side effect, so with an empty list it is not asked.
        if !self.chain.in_schematic(x, y, z, Some(current)) {
            return false;
        }
        self.skip.is_empty() || !self.skip.iter().any(|b| b == self.states.block(self.chain.desired_state(x, y, z, Some(current), &[])))
    }

    /// The state the stack wants at a relative position: `desiredState` with `BuilderProcess.this.approxPlaceable`.
    fn desired(&self, x: i32, y: i32, z: i32, current: StateId) -> StateId {
        self.chain.desired_state(x, y, z, Some(current), &self.approx)
    }

    /// `BuilderProcess.BuilderCalculationContext.getSchematic`: the state wanted at a world position, or `None` when the position is not in the schematic.
    fn get(&self, x: i32, y: i32, z: i32, current: StateId) -> Option<StateId> {
        let (rx, ry, rz) = (x.wrapping_sub(self.origin.x), y.wrapping_sub(self.origin.y), z.wrapping_sub(self.origin.z));
        if self.in_schematic(rx, ry, rz, current) {
            Some(self.desired(rx, ry, rz, current))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------- BuilderCalculationContext

/// `BuilderProcess.BuilderCalculationContext`'s overrides, over snapshots taken when the tick made it. Upstream reads `approxPlaceable` and the settings
/// live from the search thread (a race); a snapshot gives the search one consistent answer (docs/deviations.md D45).
struct BuilderOverlay {
    view: View,
    /// `approxPlaceable(9)`: what the hotbar places.
    placeable: Vec<StateId>,
    settings: BuildSettings,
    liquid: Arc<Vec<bool>>,
}

impl BuilderOverlay {
    fn rules(&self) -> ValidRules<'_> {
        ValidRules { states: &self.view.states, liquid: &self.liquid, settings: &self.settings }
    }
}

impl CostOverlay for BuilderOverlay {
    /// `BuilderCalculationContext.costOfPlacingAt`.
    fn cost_of_placing_at(&self, cfg: &Config, border: &Border, x: i32, y: i32, z: i32, current: usize) -> f64 {
        if !border.can_place_at(x, z) {
            // make calculation fail properly if we can't build (`isPossiblyProtected` is always false)
            return COST_INF;
        }
        match self.view.get(x, y, z, current) {
            Some(sch) => {
                // TODO this can return true even when allowPlace is off.... is that an issue? (upstream; it is U39)
                if self.view.states.is_air(sch) {
                    // we want this to be air, but they're asking if they can place here
                    // this won't be a schematic block, this will be a throwaway
                    return cfg.place_penalty * self.settings.place_incorrect_block_penalty_multiplier;
                    // we're going to have to break it eventually
                }
                if self.placeable.contains(&sch) {
                    return 0.0; // thats right we gonna make it FREE to place a block where it should go in a structure
                }
                if !cfg.has_throwaway {
                    return COST_INF;
                }
                // we want it to be something that we don't have
                // even more of a pain to place something wrong
                cfg.place_penalty * 1.5 * self.settings.place_incorrect_block_penalty_multiplier
            }
            None => {
                if cfg.has_throwaway {
                    cfg.place_penalty
                } else {
                    COST_INF
                }
            }
        }
    }

    /// `BuilderCalculationContext.breakCostMultiplierAt`.
    fn break_cost_multiplier_at(&self, cfg: &Config, x: i32, y: i32, z: i32, current: usize) -> f64 {
        if !cfg.allow_break && !cfg.allow_break_anyway.iter().any(|b| b == self.view.states.block(current)) {
            return COST_INF;
        }
        match self.view.get(x, y, z, current) {
            Some(sch) => {
                if self.view.states.is_air(sch) {
                    // it should be air
                    // regardless of current contents, we can break it
                    return 1.0;
                }
                // it should be a real block
                // is it already that block?
                if self.rules().valid(current, Some(sch), false) {
                    self.settings.break_correct_block_penalty_multiplier
                } else {
                    // can break if it's wrong
                    // would be great to return less than 1 here, but that would actually make the cost calculation messed up
                    // since we're breaking a block, if we underestimate the cost, then it'll fail when it really takes the correct amount of time
                    1.0
                }
            }
            None => 1.0, // why not lol
        }
    }
}

/// The bits of `BuilderCalculationContext` the tick itself reads: the stack over the schematic and what the hotbar places.
struct Bcc {
    view: View,
    placeable: Vec<StateId>,
}

// ---------------------------------------------------------------- the process

/// `BuilderProcess.Placement`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub hotbar_selection: usize,
    pub place_against: Pos,
    pub side: Direction,
    pub rot: Rotation,
}

/// `Direction.values()`.
const DIRECTIONS: [Direction; 6] = [Direction::Down, Direction::Up, Direction::North, Direction::South, Direction::West, Direction::East];

/// `Movement.HORIZONTALS_BUT_ALSO_DOWN_____SO_EVERY_DIRECTION_EXCEPT_UP`.
const HORIZONTALS_BUT_ALSO_DOWN: [Direction; 5] = [Direction::North, Direction::South, Direction::East, Direction::West, Direction::Down];

/// `Direction.getOpposite`.
fn opposite(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::East => Direction::West,
        Direction::West => Direction::East,
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
    }
}

/// `BuilderProcess.aabbSideMultipliers`: the five aim points of a side.
/// `BuilderProcess.aabbSideMultipliers`.
fn aabb_side_multipliers(side: Direction) -> Vec<Vec3> {
    match side {
        Direction::Up => vec![Vec3::new(0.5, 1.0, 0.5), Vec3::new(0.1, 1.0, 0.5), Vec3::new(0.9, 1.0, 0.5), Vec3::new(0.5, 1.0, 0.1), Vec3::new(0.5, 1.0, 0.9)],
        Direction::Down => vec![Vec3::new(0.5, 0.0, 0.5), Vec3::new(0.1, 0.0, 0.5), Vec3::new(0.9, 0.0, 0.5), Vec3::new(0.5, 0.0, 0.1), Vec3::new(0.5, 0.0, 0.9)],
        Direction::North | Direction::South | Direction::East | Direction::West => {
            let x = if side.step_x() == 0 { 0.5 } else { f64::from(1 + side.step_x()) / 2.0 };
            let z = if side.step_z() == 0 { 0.5 } else { f64::from(1 + side.step_z()) / 2.0 };
            vec![Vec3::new(x, 0.25, z), Vec3::new(x, 0.75, z)]
        }
    }
}

/// `Vec3i.toString`.
fn vec3i_string(v: (i32, i32, i32)) -> String {
    format!("Vec3i{{x={}, y={}, z={}}}", v.0, v.1, v.2)
}

/// `BlockPos.toString`.
fn block_pos_string(p: Pos) -> String {
    format!("BlockPos{{x={}, y={}, z={}}}", p.x, p.y, p.z)
}

/// `FillSchematic` of air, which `clearArea` builds: the block is air, whatever is there. The block index that `FillSchematic.<init>` takes is not
/// at hand, and air has one state, so the answer is the same (docs/deviations.md D44).
#[derive(Debug)]
struct AirFill {
    width_x: i32,
    height_y: i32,
    length_z: i32,
    air: StateId,
}

impl Schematic for AirFill {
    /// `FillSchematic.desiredState` for air.
    fn desired_state(&self, _x: i32, _y: i32, _z: i32, current: Option<StateId>, approx_placeable: &[StateId]) -> StateId {
        if let Some(c) = current {
            if c == self.air {
                return c;
            }
        }
        for &placeable in approx_placeable {
            if placeable == self.air {
                return placeable;
            }
        }
        self.air
    }
    fn width_x(&self) -> i32 {
        self.width_x
    }
    fn height_y(&self) -> i32 {
        self.height_y
    }
    fn length_z(&self) -> i32 {
        self.length_z
    }
}

/// `BuilderProcess`.
pub struct BuilderProcess {
    /// Upstream's `HashSet`; ordered by position here so that the goals made from it are equal from one tick to the next (docs/deviations.md D44).
    incorrect_positions: Option<BTreeSet<(i32, i32, i32)>>,
    /// Positions that are completed even if they're out of render distance and we can't make sure right now.
    observed_completed: Option<HashSet<Pos>>,
    name: Option<String>,
    /// The chain of `build`: the schematic with the substitute, mirror and rotation wrappers. Upstream's `schematic` field is this, under the skip mask
    /// and the layer wrapper that [`View`] adds. `None` is `schematic == null`: the process is not active.
    chain: Option<Arc<dyn Schematic>>,
    /// The layer wrapper's bounds: set from the first tick with `buildInLayers`, and kept (the wrapper stays in `schematic`).
    layer_bounds: Option<(i32, i32)>,
    origin: Pos,
    ticks: i32,
    paused: bool,
    layer: i32,
    num_repeats: i32,
    approx_placeable: Vec<StateId>,
    /// `stopAtHeight`.
    pub stop_at_height: i32,
    /// `startAtLayer` as last read, for `onLostControl`, which has no settings to read.
    start_at_layer: i32,
    messages: Vec<String>,
    /// What upstream prints with `System.out.println` (the map art line): not chat.
    console: Vec<String>,
    finished: bool,
    context: Option<Arc<ProcessContext>>,
    /// `block instanceof LiquidBlock` for each state of the table, made from the first tick's table.
    liquid: Option<Arc<Vec<bool>>>,
}

impl Default for BuilderProcess {
    fn default() -> Self {
        BuilderProcess::new()
    }
}

impl BuilderProcess {
    /// `BuilderProcess.<init>`.
    pub fn new() -> BuilderProcess {
        BuilderProcess { incorrect_positions: None, observed_completed: None, name: None, chain: None, layer_bounds: None, origin: Pos::new(0, 0, 0), ticks: 0, paused: false, layer: 0, num_repeats: 0, approx_placeable: vec![], stop_at_height: 0, start_at_layer: 0, messages: vec![], console: vec![], finished: false, context: None, liquid: None }
    }

    /// `BuilderProcess.build(String,ISchematic,Vec3i)`: the wrappers, the orientation offsets and the layers.
    /// `IBuilderProcess.build(String,ISchematic,Vec3i)`, which is `BuilderProcess.build(String,ISchematic,Vec3i)`: wraps the schematic in the
    /// substitute, the mirror and the rotation (in that order, each only when set), moves the origin by the size for the orientation settings and starts
    /// over. The skip mask is not made here: it reads `buildSkipBlocks` on every lookup, as upstream's does.
    pub fn build(&mut self, name: String, schematic: Arc<dyn Schematic>, origin: Pos, bs: &BuildSettings, states: &Arc<BlockStates>) {
        self.name = Some(name);
        let mut chain = Arc::clone(&schematic);
        if !bs.build_substitutes.is_empty() {
            chain = Arc::new(SubstituteSchematic::new(chain, bs.build_substitutes.clone(), Arc::clone(states)));
        }
        if bs.build_schematic_mirror != crate::schematic::BlockMirror::None {
            chain = Arc::new(MirroredSchematic::new(chain, bs.build_schematic_mirror, Arc::clone(states)));
        }
        if bs.build_schematic_rotation != crate::schematic::BlockRotation::None {
            chain = Arc::new(RotatedSchematic::new(chain, bs.build_schematic_rotation, Arc::clone(states)));
        }
        self.chain = Some(chain);
        self.layer_bounds = None;
        let (mut x, mut y, mut z) = (origin.x, origin.y, origin.z);
        // the size is the parameter's, not the rotated chain's: upstream's parameter shadows the field (U35)
        if bs.schematic_orientation_x {
            x = x.wrapping_add(schematic.width_x());
        }
        if bs.schematic_orientation_y {
            y = y.wrapping_add(schematic.height_y());
        }
        if bs.schematic_orientation_z {
            z = z.wrapping_add(schematic.length_z());
        }
        self.origin = Pos::new(x, y, z);
        self.paused = false;
        self.start_at_layer = bs.start_at_layer;
        self.layer = bs.start_at_layer;
        self.stop_at_height = schematic.height_y();
        self.num_repeats = 0;
        self.observed_completed = Some(HashSet::new());
        self.incorrect_positions = None;
        self.context = None;
    }

    /// `BuilderProcess.build(String,File,Vec3i)`: after the file is parsed: map art, then `build`.
    /// `IBuilderProcess.build(String,File,Vec3i)`: the interface method.
    /// `BuilderProcess.build(String,File,Vec3i)` after the parse: `applyMapArtAndSelection` (the selection is left out), then `build`. `Err` is the exception
    /// the map art schematic throws. The map art line goes to the console lines, not to chat (D50).
    pub fn build_parsed(&mut self, name: String, parsed: Arc<dyn StaticSchematicView>, origin: Pos, bs: &BuildSettings, states: &Arc<BlockStates>) -> Result<(), String> {
        let schematic: Arc<dyn Schematic> = if bs.map_art_mode {
            let (mask, lines) = MaskSchematic::map_art(parsed, states)?;
            self.console.extend(lines);
            Arc::new(mask)
        } else {
            parsed
        };
        self.build(name, schematic, origin, bs, states);
        Ok(())
    }

    /// `IBuilderProcess.clearArea`: the interface method.
    /// `BuilderProcess.clearArea`: a build of air over the box between the two corners.
    pub fn clear_area(&mut self, corner1: Pos, corner2: Pos, bs: &BuildSettings, states: &Arc<BlockStates>) {
        let origin = Pos::new(corner1.x.min(corner2.x), corner1.y.min(corner2.y), corner1.z.min(corner2.z));
        let width_x = corner1.x.wrapping_sub(corner2.x).wrapping_abs().wrapping_add(1);
        let height_y = corner1.y.wrapping_sub(corner2.y).wrapping_abs().wrapping_add(1);
        let length_z = corner1.z.wrapping_sub(corner2.z).wrapping_abs().wrapping_add(1);
        self.build("clear area".to_string(), Arc::new(AirFill { width_x, height_y, length_z, air: states.air() }), origin, bs, states);
    }

    /// `BuilderProcess.resume`: leave the pause.
    /// `IBuilderProcess.resume`: the interface method.
    /// `BuilderProcess.resume`.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// `BuilderProcess.pause`: pause the build.
    /// `IBuilderProcess.pause`: the interface method.
    /// `BuilderProcess.pause`.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// `BuilderProcess.isPaused`: whether the build is paused.
    /// `IBuilderProcess.isPaused`: the interface method.
    /// `BuilderProcess.isPaused`.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// `BuilderProcess.isActive`: whether a schematic is loaded.
    /// `BuilderProcess.isActive`.
    pub fn is_active(&self) -> bool {
        self.chain.is_some()
    }

    /// `BuilderProcess.getApproxPlaceable`: the last answer of `approxPlaceable`.
    /// `IBuilderProcess.getApproxPlaceable`: the interface method.
    /// `BuilderProcess.getApproxPlaceable`.
    pub fn approx_placeable(&self) -> Vec<StateId> {
        self.approx_placeable.clone()
    }

    /// `BuilderProcess.getMinLayer`: the lowest layer being built.
    /// `IBuilderProcess.getMinLayer`: the interface method.
    /// `BuilderProcess.getMinLayer`.
    pub fn min_layer(&self, bs: &BuildSettings) -> Option<i32> {
        if bs.build_in_layers {
            Some(self.layer)
        } else {
            None
        }
    }

    /// `BuilderProcess.getMaxLayer`: the highest layer being built.
    /// `IBuilderProcess.getMaxLayer`: the interface method.
    /// `BuilderProcess.getMaxLayer`.
    pub fn max_layer(&self, bs: &BuildSettings) -> Option<i32> {
        if bs.build_in_layers {
            Some(self.stop_at_height)
        } else {
            None
        }
    }

    /// `BuilderProcess.displayName0`: the name of the build.
    /// `BuilderProcess.displayName0`.
    pub fn display_name(&self) -> String {
        if self.paused {
            "Builder Paused".to_string()
        } else {
            format!("Building {}", self.name.as_deref().unwrap_or("null"))
        }
    }

    /// The origin of the build, which `buildRepeat` moves.
    pub fn origin(&self) -> Pos {
        self.origin
    }

    /// The layer the build is at.
    pub fn layer(&self) -> i32 {
        self.layer
    }

    /// The positions still wrong as of the last tick, in position order. Empty before the first tick.
    pub fn incorrect_positions(&self) -> Vec<Pos> {
        self.incorrect_positions.as_ref().map(|s| s.iter().map(|&(x, y, z)| Pos::new(x, y, z)).collect()).unwrap_or_default()
    }

    /// The chat lines the process has said (`logDirect`). Takes them.
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    /// The lines upstream prints to the console with `System.out.println`, not to chat (the map art line, D50). Takes them.
    pub fn take_console_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.console)
    }

    /// Whether "Done building" was said since the last call: what `notificationOnBuildFinished` is for. The glue decides about the notification.
    pub fn take_finished(&mut self) -> bool {
        std::mem::take(&mut self.finished)
    }

    /// The calculation context of the last tick's command, for the pathing to hand the search (`PathingCommandContext.desiredCalcContext`).
    pub fn context(&self) -> Option<Arc<ProcessContext>> {
        self.context.clone()
    }

    /// `BuilderProcess.onLostControl`: forget the build.
    /// `BuilderProcess.onLostControl`.
    pub fn on_lost_control(&mut self) {
        self.incorrect_positions = None;
        self.name = None;
        self.chain = None;
        self.layer_bounds = None;
        self.layer = self.start_at_layer;
        self.num_repeats = 0;
        self.paused = false;
        self.observed_completed = None;
        self.context = None;
    }

    fn view(&self, bs: &BuildSettings, states: &Arc<BlockStates>) -> Option<View> {
        Some(View { chain: Arc::clone(self.chain.as_ref()?), origin: self.origin, bounds: self.layer_bounds, skip: bs.build_skip_blocks.clone(), approx: self.approx_placeable.clone(), states: Arc::clone(states) })
    }

    /// `BuilderProcess.placeAt`: the state the build wants at a world position, given what is there now. `None` when not building, outside the
    /// schematic, or where it wants air.
    pub fn place_at(&self, x: i32, y: i32, z: i32, current: StateId, bs: &BuildSettings, states: &Arc<BlockStates>) -> Option<StateId> {
        let view = self.view(bs, states)?;
        let state = view.get(x, y, z, current)?;
        if states.is_air(state) {
            return None;
        }
        Some(state)
    }

    /// What `InventoryBehavior.selectThrowawayForLocation` asks of the builder, as a snapshot that does not borrow the process.
    pub fn throwaway_hint<'a>(&self, be: &BuildEnv<'a>) -> Option<BuilderHint<'a>> {
        Some(BuilderHint { view: self.view(be.settings, be.states)?, facts: be.facts })
    }

    fn liquid_flags<W: WorldView>(&mut self, env: &Env<'_, W>) -> Arc<Vec<bool>> {
        match &self.liquid {
            Some(l) if l.len() == env.ctx.states.len() => Arc::clone(l),
            _ => {
                let l = Arc::new(env.ctx.states.iter().map(|s| s.liquid_block).collect::<Vec<_>>());
                self.liquid = Some(Arc::clone(&l));
                l
            }
        }
    }

    /// `BuilderProcess.approxPlaceable`: asked of the game through `BuildFacts`.
    /// `BuilderProcess.approxPlaceable(int)` is `facts.approx_placeable()`; the first `size` of it.
    fn facts_approx(be: &BuildEnv<'_>, size: usize) -> Vec<StateId> {
        let mut all = be.facts.approx_placeable();
        all.resize(size.max(all.len().min(size)), be.states.air());
        all.truncate(size);
        all
    }

    // ------------------------------------------------------------ the tick

    /// `BuilderProcess.onTick(boolean,boolean)`: the entry of a tick.
    /// `BuilderProcess.onTick(boolean,boolean)`. `calc_failed` is not read: upstream passes it on and never looks at it. `safe_to_cancel` is `PathingBehavior.isSafeToCancel`. `None` is upstream's `null`: the build is done.
    pub fn on_tick<W: WorldView>(&mut self, _calc_failed: bool, safe_to_cancel: bool, env: &Env<'_, W>, be: &BuildEnv<'_>, controls: &mut Controls) -> Option<PathingCommand> {
        // `isShiftKeyDown` is what the last tick forced, read before the keys are cleared
        let was_sneaking = controls.keys.sneak;
        self.tick_inner(safe_to_cancel, env, be, controls, was_sneaking, 0)
    }

    /// `BuilderProcess.onTick(boolean,boolean,int)`: a tick, with the count of recursions.
    /// `BuilderProcess.onTick(boolean,boolean,int)`.
    #[allow(clippy::too_many_arguments)]
    fn tick_inner<W: WorldView>(&mut self, safe_to_cancel: bool, env: &Env<'_, W>, be: &BuildEnv<'_>, controls: &mut Controls, was_sneaking: bool, recursions: i32) -> Option<PathingCommand> {
        if recursions > 100 {
            // onTick calls itself, don't crash
            self.context = None;
            return Some(PathingCommand { goal: None, kind: PathingCommandType::SetGoalAndPath });
        }
        let bs = be.settings;
        self.start_at_layer = bs.start_at_layer;
        self.context = None;
        self.approx_placeable = Self::facts_approx(be, 36);
        if controls.keys.click_left {
            self.ticks = 5;
        } else {
            self.ticks = self.ticks.wrapping_sub(1);
        }
        controls.clear_all_keys();
        if self.paused {
            return Some(PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal });
        }
        let chain = Arc::clone(self.chain.as_ref()?);
        if bs.build_in_layers {
            // layer = 0 should be nothing
            // layer = realSchematic.heightY() should be everything
            let height = chain.height_y();
            if bs.layer_order {
                // top to bottom
                self.layer_bounds = Some((height.wrapping_sub(self.layer.wrapping_mul(bs.layer_height)), height.wrapping_sub(1)));
            } else {
                self.layer_bounds = Some((0, self.layer.wrapping_mul(bs.layer_height).wrapping_sub(1)));
            }
        }
        let liquid = self.liquid_flags(env);
        let bcc = Bcc { view: self.view(bs, be.states)?, placeable: Self::facts_approx(be, 9) };
        if !self.recalc(env, &bcc, &ValidRules { states: be.states, liquid: &liquid, settings: bs }, bs) {
            if bs.build_in_layers && self.layer.wrapping_mul(bs.layer_height) < self.stop_at_height {
                self.messages.push(format!("Starting layer {}", self.layer));
                self.layer = self.layer.wrapping_add(1);
                return self.tick_inner(safe_to_cancel, env, be, controls, was_sneaking, recursions + 1);
            }
            let repeat = bs.build_repeat;
            let max = bs.build_repeat_count;
            self.num_repeats = self.num_repeats.wrapping_add(1);
            if repeat == (0, 0, 0) || (max != -1 && self.num_repeats >= max) {
                self.messages.push("Done building".to_string());
                self.finished = true;
                self.on_lost_control();
                return None;
            }
            // build repeat time
            self.layer = 0;
            self.origin = Pos::new(self.origin.x.wrapping_add(repeat.0), self.origin.y.wrapping_add(repeat.1), self.origin.z.wrapping_add(repeat.2));
            // `if (!buildRepeatSneaky) schematic.reset()`: `schematic` is the skip mask, whose `reset` does not reach the chain, so it does nothing (U36)
            self.messages.push(format!("Repeating build in vector {}, new origin is {}", vec3i_string(repeat), block_pos_string(self.origin)));
            return self.tick_inner(safe_to_cancel, env, be, controls, was_sneaking, recursions + 1);
        }
        let rules = ValidRules { states: be.states, liquid: &liquid, settings: bs };
        if bs.distance_trim {
            self.trim(env);
        }

        let to_break = self.to_break_near_player(env, &bcc, &rules, bs);
        if let Some((pos, rot)) = to_break {
            if safe_to_cancel && env.p.on_ground {
                // we'd like to pause to break this block
                // only change look direction if it's safe (don't want to fuck up an in progress parkour for example
                controls.look = Some(rot);
                controls.look_force = true;
                let id = env.ctx.get(pos.x, pos.y, pos.z).id;
                if let Some(slot) = switch_to_best_tool_for(env.inv, id, &env.set.tool) {
                    controls.select = Some(slot);
                }
                if env.p.crouching || was_sneaking {
                    // really horrible bug where a block is visible for breaking while sneaking but not otherwise
                    // so you can't see it, it goes to place something else, sneaks, then the next tick it tries to break
                    // and is unable since it's unsneaked in the intermediary tick
                    controls.keys.sneak = true;
                }
                if is_looking_at(env.p, pos) || env.p.rotations().is_really_close_to(rot) {
                    controls.keys.click_left = true;
                }
                return Some(PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal });
            }
        }
        let mut desirable_on_hotbar: Vec<StateId> = vec![];
        let to_place = self.search_for_placables(env, be, &bcc, &rules, &mut desirable_on_hotbar);
        if let Some(pl) = to_place {
            if safe_to_cancel && env.p.on_ground && self.ticks <= 0 {
                controls.look = Some(pl.rot);
                controls.look_force = true;
                controls.select = Some(pl.hotbar_selection);
                controls.keys.sneak = true;
                let looking = env.p.looking_at.is_some_and(|h| h.pos == pl.place_against && h.face == pl.side);
                if looking || env.p.rotations().is_really_close_to(pl.rot) {
                    controls.keys.click_right = true;
                }
                return Some(PathingCommand { goal: None, kind: PathingCommandType::CancelAndSetGoal });
            }
        }

        if env.set.tool.allow_inventory {
            let approx = &self.approx_placeable;
            let mut useful_slots: Vec<usize> = vec![];
            let mut no_valid_hotbar_option: Vec<StateId> = vec![];
            'desired: for &desired in &desirable_on_hotbar {
                for i in 0..9 {
                    if approx.get(i).is_some_and(|&a| rules.valid(a, Some(desired), true)) {
                        useful_slots.push(i);
                        continue 'desired;
                    }
                }
                no_valid_hotbar_option.push(desired);
            }

            'inventory: for i in 9..36 {
                for &desired in &no_valid_hotbar_option {
                    if approx.get(i).is_some_and(|&a| rules.valid(a, Some(desired), true)) {
                        if !controls.inventory.attempt_to_put_on_hotbar(env.inv, i, &env.set.tool, &|slot| useful_slots.contains(&slot)) {
                            // awaiting inventory move, so pause
                            return Some(PathingCommand { goal: None, kind: PathingCommandType::RequestPause });
                        }
                        break 'inventory;
                    }
                }
            }
        }

        let approx = self.approx_placeable.clone();
        let mut goal = self.assemble(env, be, &bcc, &rules, &approx[..9.min(approx.len())], false);
        if goal.is_none() {
            // we're far away, so assume that we have our whole inventory to recalculate placeable properly
            goal = self.assemble(env, be, &bcc, &rules, &approx, true);
            if goal.is_none() {
                if bs.skip_failed_layers && bs.build_in_layers && self.layer.wrapping_mul(bs.layer_height) < chain.height_y() {
                    self.messages.push(format!("Skipping layer that I cannot construct! Layer #{}", self.layer));
                    self.layer = self.layer.wrapping_add(1);
                    return self.tick_inner(safe_to_cancel, env, be, controls, was_sneaking, recursions + 1);
                }
                self.messages.push("Unable to do it. Pausing. resume to resume, cancel to cancel".to_string());
                self.paused = true;
                return Some(PathingCommand { goal: None, kind: PathingCommandType::RequestPause });
            }
        }
        // `new PathingCommandContext(goal, FORCE_REVALIDATE_GOAL_AND_PATH, bcc)`
        let mut cfg = env.ctx.cfg.clone();
        cfg.jump_penalty += 10.0;
        cfg.search.backtrack_cost_favoring_coefficient = 1.0;
        let overlay = BuilderOverlay { view: bcc.view, placeable: bcc.placeable, settings: bs.clone(), liquid };
        self.context = Some(Arc::new(ProcessContext { cfg, costs: Arc::new(overlay) }));
        Some(PathingCommand { goal, kind: PathingCommandType::ForceRevalidateGoalAndPath })
    }

    /// `BuilderProcess.recalc`: the incorrect positions of one layer.
    /// `BuilderProcess.recalc`.
    fn recalc<W: WorldView>(&mut self, env: &Env<'_, W>, bcc: &Bcc, rules: &ValidRules<'_>, bs: &BuildSettings) -> bool {
        if self.incorrect_positions.is_none() {
            self.incorrect_positions = Some(BTreeSet::new());
            self.full_recalc(env, bcc, rules, bs);
            if self.incorrect_positions.as_ref().is_none_or(|s| s.is_empty()) {
                return false;
            }
        }
        self.recalc_nearby(env, bcc, rules, bs);
        if self.incorrect_positions.as_ref().is_none_or(|s| s.is_empty()) {
            self.full_recalc(env, bcc, rules, bs);
        }
        self.incorrect_positions.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// `BuilderProcess.trim`: forgets what is further than 200 (squared) from the player, unless that is everything.
    fn trim<W: WorldView>(&mut self, env: &Env<'_, W>) {
        let Some(set) = &self.incorrect_positions else {
            return;
        };
        let here = env.p.block_position();
        let copy: BTreeSet<(i32, i32, i32)> = set.iter().copied().filter(|&(x, y, z)| dist_sq(Pos::new(x, y, z), here) <= 200.0).collect();
        if !copy.is_empty() {
            self.incorrect_positions = Some(copy);
        }
    }

    /// `BuilderProcess.recalcNearby`: looks again at the cells around the player.
    fn recalc_nearby<W: WorldView>(&mut self, env: &Env<'_, W>, bcc: &Bcc, rules: &ValidRules<'_>, bs: &BuildSettings) {
        let center = env.p.feet();
        let radius = bs.builder_tick_scan_radius;
        let incorrect = self.incorrect_positions.get_or_insert_with(BTreeSet::new);
        let observed = self.observed_completed.get_or_insert_with(HashSet::new);
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    let (x, y, z) = (center.x.wrapping_add(dx), center.y.wrapping_add(dy), center.z.wrapping_add(dz));
                    let current = env.ctx.get(x, y, z).id;
                    if let Some(desired) = bcc.view.get(x, y, z, current) {
                        // we care about this position
                        if rules.valid(current, Some(desired), false) {
                            incorrect.remove(&(x, y, z));
                            observed.insert(Pos::new(x, y, z));
                        } else {
                            incorrect.insert((x, y, z));
                            observed.remove(&Pos::new(x, y, z));
                        }
                    }
                }
            }
        }
    }

    /// `BuilderProcess.fullRecalc`: looks at every cell of the schematic, stopping once more than `incorrectSize` are wrong. A cell out of render
    /// distance counts as wrong until it has been seen right.
    fn full_recalc<W: WorldView>(&mut self, env: &Env<'_, W>, bcc: &Bcc, rules: &ValidRules<'_>, bs: &BuildSettings) {
        let mut incorrect = BTreeSet::new();
        let observed = self.observed_completed.get_or_insert_with(HashSet::new);
        let view = &bcc.view;
        let (width, height, length) = (view.chain.width_x(), view.chain.height_y(), view.chain.length_z());
        let limit = i64::from(bs.incorrect_size);
        'cells: for y in 0..height {
            for z in 0..length {
                for x in 0..width {
                    let block = Pos::new(x.wrapping_add(self.origin.x), y.wrapping_add(self.origin.y), z.wrapping_add(self.origin.z));
                    let current = env.ctx.get(block.x, block.y, block.z).id;
                    if !view.in_schematic(x, y, z, current) {
                        continue;
                    }
                    if env.ctx.world.contains_loaded_chunk(block.x, block.z) {
                        // check if its in render distance, not if its in cache
                        // we can directly observe this block, it is in render distance
                        if rules.valid(current, Some(view.desired(x, y, z, current)), false) {
                            observed.insert(block);
                        } else {
                            incorrect.insert((block.x, block.y, block.z));
                            observed.remove(&block);
                            if incorrect.len() as i64 > limit {
                                break 'cells;
                            }
                        }
                        continue;
                    }
                    // this is not in render distance
                    if !observed.contains(&block) {
                        // and we've never seen this position be correct
                        // therefore mark as incorrect
                        incorrect.insert((block.x, block.y, block.z));
                        if incorrect.len() as i64 > limit {
                            break 'cells;
                        }
                    }
                }
            }
        }
        self.incorrect_positions = Some(incorrect);
    }

    /// `BuilderProcess.toBreakNearPlayer`: the first wrong block within reach that can be looked at, and the rotation that looks at it.
    fn to_break_near_player<W: WorldView>(&self, env: &Env<'_, W>, bcc: &Bcc, rules: &ValidRules<'_>, bs: &BuildSettings) -> Option<(Pos, Rotation)> {
        let center = env.p.feet();
        let path_start = env.path_start;
        for dx in -5..=5 {
            for dy in (if bs.break_from_above { -1 } else { 0 })..=5 {
                for dz in -5..=5 {
                    let (x, y, z) = (center.x.wrapping_add(dx), center.y.wrapping_add(dy), center.z.wrapping_add(dz));
                    if dy == -1 && x == path_start.x && z == path_start.z {
                        continue; // dont mine what we're supported by, but not directly standing on
                    }
                    let curr = env.ctx.get(x, y, z).id;
                    let Some(desired) = bcc.view.get(x, y, z, curr) else {
                        continue; // irrelevant
                    };
                    let block = rules.states.block(curr);
                    if !rules.states.is_air(curr) && !(block == "water" || block == "lava") && !rules.valid(curr, Some(desired), false) {
                        let pos = Pos::new(x, y, z);
                        if let Some(rot) = reachable(env, pos, env.game.reach, false) {
                            return Some((pos, rot));
                        }
                    }
                }
            }
        }
        None
    }

    /// `BuilderProcess.searchForPlacables`: the first wrong cell around the player that can be filled from where he stands. `desirable_on_hotbar` gets the state
    /// wanted at every wrong cell looked at, up to and including the one that is placed (upstream's `desirableOnHotbar` out parameter).
    fn search_for_placables<W: WorldView>(&self, env: &Env<'_, W>, be: &BuildEnv<'_>, bcc: &Bcc, rules: &ValidRules<'_>, desirable_on_hotbar: &mut Vec<StateId>) -> Option<Placement> {
        let center = env.p.feet();
        for dx in -5..=5 {
            for dy in -5..=1 {
                for dz in -5..=5 {
                    let (x, y, z) = (center.x.wrapping_add(dx), center.y.wrapping_add(dy), center.z.wrapping_add(dz));
                    let curr = env.ctx.get(x, y, z);
                    let Some(desired) = bcc.view.get(x, y, z, curr.id) else {
                        continue; // irrelevant
                    };
                    if env.ctx.is_replaceable(x, z, &curr) && !rules.valid(curr.id, Some(desired), false) {
                        if dy == 1 && env.ctx.get(x, y.wrapping_add(1), z).air {
                            continue;
                        }
                        desirable_on_hotbar.push(desired);
                        if let Some(p) = self.possible_to_place(desired, x, y, z, env, be, rules) {
                            return Some(p);
                        }
                    }
                }
            }
        }
        None
    }

    /// `BuilderProcess.possibleToPlace`: a face to place `to_place` against from where the player stands, with the hotbar slot that places it.
    #[allow(clippy::too_many_arguments)]
    fn possible_to_place<W: WorldView>(&self, to_place: StateId, x: i32, y: i32, z: i32, env: &Env<'_, W>, be: &BuildEnv<'_>, rules: &ValidRules<'_>) -> Option<Placement> {
        // these two don't care which face we're placing against, but they were asked for every face
        // placementPlausible is an entity collision query, so that's up to six of those per position per tick
        // still only asked once we've found something solid to place against, like before
        let mut checked_placeable = false;
        let here = Pos::new(x, y, z);
        for against in DIRECTIONS {
            let place_against_pos = here.relative(against, 1);
            let place_against_state = env.ctx.get(place_against_pos.x, place_against_pos.y, place_against_pos.z);
            if env.ctx.is_replaceable(place_against_pos.x, place_against_pos.z, &place_against_state) {
                continue;
            }
            if !checked_placeable {
                if !be.facts.can_survive(to_place, here) || !be.facts.placement_plausible(to_place, here) {
                    return None;
                }
                checked_placeable = true;
            }
            let Some(aabb) = be.facts.outline_bounds(place_against_pos) else {
                continue;
            };
            for m in aabb_side_multipliers(against) {
                let place_x = f64::from(place_against_pos.x) + aabb.min_x * m.x + aabb.max_x * (1.0 - m.x);
                let place_y = f64::from(place_against_pos.y) + aabb.min_y * m.y + aabb.max_y * (1.0 - m.y);
                let place_z = f64::from(place_against_pos.z) + aabb.min_z * m.z + aabb.max_z * (1.0 - m.z);
                let eye = infer_sneaking_eye_position(env.p.x, env.p.y, env.p.z);
                let rot = calc_rotation_from_vec3d(eye, Vec3::new(place_x, place_y, place_z), env.p.rotations());
                let actual_rot = env.game.aim.peek_rotation(env.p.rotations(), rot);
                let Some(result) = be.facts.ray_trace(eye, actual_rot, env.game.reach) else {
                    continue;
                };
                if result.pos == place_against_pos && result.face == opposite(against) {
                    if let Some(hotbar) = self.has_any_item_that_would_place(to_place, &result, actual_rot, be, rules) {
                        return Some(Placement { hotbar_selection: hotbar, place_against: place_against_pos, side: opposite(against), rot });
                    }
                }
            }
        }
        None
    }

    /// `BuilderProcess.hasAnyItemThatWouldPlace`: the first hotbar slot whose item would place a state that is valid for `desired`.
    fn has_any_item_that_would_place(&self, desired: StateId, result: &BlockHit, rot: Rotation, be: &BuildEnv<'_>, rules: &ValidRules<'_>) -> Option<usize> {
        for i in 0..9 {
            // an empty stack, a stack that is not a block, a null state and `!canPlace` all give `None`
            let Some(would_be_placed) = be.facts.state_for_placement(i, result, rot) else {
                continue;
            };
            if rules.valid(would_be_placed, Some(desired), true) {
                return Some(i);
            }
        }
        None
    }

    /// `BuilderProcess.assemble(BuilderCalculationContext,List)`: the two-argument form, without the messages.
    /// `BuilderProcess.assemble(BuilderCalculationContext,List,boolean)`: sorts the wrong positions into what can be placed, what must be broken, and what is missing, and makes the goal
    /// from them. `None` when nothing can be done now; with `log_missing` it says what is missing.
    fn assemble<W: WorldView>(&mut self, env: &Env<'_, W>, be: &BuildEnv<'_>, bcc: &Bcc, rules: &ValidRules<'_>, approx_placeable: &[StateId], log_missing: bool) -> Option<Goal> {
        let mut placeable: Vec<Pos> = vec![];
        let mut breakable: Vec<Pos> = vec![];
        let mut source_liquids: Vec<Pos> = vec![];
        let mut flowing_liquids: Vec<Pos> = vec![];
        // the states that are missing and how many places want each, in the order they were first met
        let mut missing: Vec<(StateId, i32)> = vec![];
        let mut out_of_bounds: Vec<(i32, i32, i32)> = vec![];
        let positions: Vec<(i32, i32, i32)> = self.incorrect_positions.iter().flatten().copied().collect();
        for &(x, y, z) in &positions {
            let pos = Pos::new(x, y, z);
            let state = env.ctx.get(x, y, z);
            if rules.states.is_air(state.id) {
                match bcc.view.get(x, y, z, state.id) {
                    None => out_of_bounds.push((x, y, z)),
                    Some(desired) => {
                        if rules.contains_block_state(approx_placeable, desired) {
                            placeable.push(pos);
                        } else if let Some(entry) = missing.iter_mut().find(|(s, _)| *s == desired) {
                            entry.1 += 1;
                        } else {
                            missing.push((desired, 1));
                        }
                    }
                }
            } else if state.liquid_block {
                // if the block itself is JUST a liquid (i.e. not just a waterlogged block), we CANNOT break it
                // TODO for 1.13 make sure that this only matches pure water, not waterlogged blocks
                if !crate::movement::helper::possibly_flowing(&state) {
                    // if it's a source block then we want to replace it with a throwaway
                    source_liquids.push(pos);
                } else {
                    flowing_liquids.push(pos);
                }
            } else {
                breakable.push(pos);
            }
        }
        if let Some(set) = &mut self.incorrect_positions {
            for p in &out_of_bounds {
                set.remove(p);
            }
        }
        let to_break: Vec<Goal> = breakable.iter().map(|&pos| self.break_goal(env, pos, rules.settings)).collect();
        let mut to_place: Vec<Goal> = vec![];
        // List.contains for every position was quadratic, and placeable is in the thousands on a big build
        let placeable_set: HashSet<Pos> = placeable.iter().copied().collect();
        for &pos in &placeable {
            if !placeable_set.contains(&pos.below(1)) && !placeable_set.contains(&pos.below(2)) {
                to_place.push(self.placement_goal(env, be, pos, bcc));
            }
        }
        for &pos in &source_liquids {
            to_place.push(Goal::Block { x: pos.x, y: pos.y.wrapping_add(1), z: pos.z });
        }
        if !to_place.is_empty() {
            return Some(Goal::Janky { primary: Box::new(Goal::Composite(to_place)), fallback: Box::new(Goal::Composite(to_break)) });
        }
        if to_break.is_empty() {
            if log_missing && !missing.is_empty() {
                self.messages.push("Missing materials for at least:".to_string());
                self.messages.push(missing.iter().map(|(s, n)| format!("{}x {}", n, rules.states.state_to_string(*s))).collect::<Vec<_>>().join("\n"));
            }
            if log_missing && !flowing_liquids.is_empty() {
                self.messages.push("Unreplaceable liquids at at least:".to_string());
                self.messages.push(flowing_liquids.iter().map(|p| format!("{} {} {}", p.x, p.y, p.z)).collect::<Vec<_>>().join("\n"));
            }
            return None;
        }
        Some(Goal::Composite(to_break))
    }

    /// `BuilderProcess.placementGoal`: stand next to a block the new one can be placed against, else at the block.
    fn placement_goal<W: WorldView>(&self, env: &Env<'_, W>, be: &BuildEnv<'_>, pos: Pos, bcc: &Bcc) -> Goal {
        // upstream reads `ctx.world()`, the live world, where a column that is not loaded is `void_air`; the context here also reads the cache (D50)
        let live = |x: i32, y: i32, z: i32| -> (bool, StateId) {
            if env.ctx.world.contains_loaded_chunk(x, z) {
                let b = env.ctx.get(x, y, z);
                (b.air, b.id)
            } else {
                (true, be.states.air())
            }
        };
        let (here_air, current) = live(pos.x, pos.y, pos.z);
        if !here_air {
            // TODO can this even happen?
            return goal_place(pos);
        }
        let allow_same_level = !live(pos.x, pos.y.wrapping_add(1), pos.z).0;
        for facing in HORIZONTALS_BUT_ALSO_DOWN {
            let against = pos.relative(facing, 1);
            let plausible = bcc.view.get(pos.x, pos.y, pos.z, current).is_some_and(|desired| be.facts.placement_plausible(desired, pos));
            if env.ctx.can_place_against_bsi(against.x, against.y, against.z) && plausible {
                return Goal::Adjacent { x: pos.x, y: pos.y, z: pos.z, no: against, allow_same_level };
            }
        }
        goal_place(pos)
    }

    /// `BuilderProcess.breakGoal`: the goal to break a block.
    /// `BuilderProcess.breakGoal`.
    fn break_goal<W: WorldView>(&self, env: &Env<'_, W>, pos: Pos, bs: &BuildSettings) -> Goal {
        if bs.goal_break_from_above && env.ctx.get(pos.x, pos.y.wrapping_add(1), pos.z).air && env.ctx.get(pos.x, pos.y.wrapping_add(2), pos.z).air {
            // TODO maybe possible without the up(2) check?
            return Goal::Janky { primary: Box::new(Goal::Break { x: pos.x, y: pos.y, z: pos.z }), fallback: Box::new(Goal::BreakFromAbove { x: pos.x, y: pos.y.wrapping_add(1), z: pos.z }) };
        }
        Goal::Break { x: pos.x, y: pos.y, z: pos.z }
    }
}

/// What `InventoryBehavior.selectThrowawayForLocation` asks of the builder, as a snapshot of the tick it was made in.
pub struct BuilderHint<'a> {
    view: View,
    facts: &'a dyn BuildFacts,
}

impl ThrowawayHint for BuilderHint<'_> {
    /// `BuilderProcess.placeAt`.
    fn place_at(&self, x: i32, y: i32, z: i32, current: usize) -> Option<usize> {
        let state = self.view.get(x, y, z, current)?;
        if self.view.states.is_air(state) {
            return None;
        }
        Some(state)
    }

    /// `maybe.equals(block.getStateForPlacement(...))` for the stack in the slot, with the hit `approxPlaceable` uses: the state of the last tick's list
    /// for the hotbar, and the game's for the off hand.
    fn slot_places(&self, slot: InvSlot, maybe: usize) -> bool {
        match slot {
            InvSlot::Hotbar(i) | InvSlot::Main(i) => self.view.approx.get(i) == Some(&maybe),
            InvSlot::Offhand => self.facts.offhand_placeable() == maybe,
        }
    }

    /// `((BlockItem) stack.getItem()).getBlock().equals(maybe.getBlock())`.
    fn slot_is_block_of(&self, slot: InvSlot, maybe: usize) -> bool {
        self.facts.item_block(slot).is_some_and(|b| b == self.view.states.block(maybe))
    }
}

#[cfg(test)]
mod tests;
