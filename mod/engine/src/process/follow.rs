// Ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/process/FollowProcess.java. LGPL-3.0-or-later, see lib.rs.
//
// `follow`: the goal is a `GoalNear` around each entity that passes the filter, all of them in one `GoalComposite`, asked again each tick so that the goal
// moves with the entities. The process reads the entities from [`ProcessInputs::entities`], which the glue fills, and never asks the game.
use super::custom_goal::{PathingCommand, PathingCommandType};
use super::{EntityInfo, ProcessInputs};
use crate::goals::{goal_near, goal_xz_from_direction, Goal};
use crate::mth;
use crate::settings::FollowSettings;

/// What `FollowProcess.filter` is: upstream holds a `Predicate<Entity>`, and the commands only ever make these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowFilter {
    /// Upstream's `FollowCommand.FollowGroup.ENTITIES`: every `LivingEntity`.
    Living,
    /// Upstream's `FollowCommand.FollowGroup.PLAYERS`: every `Player`.
    Players,
    /// `follow entity <type...>`: the entities whose type is one of these registry ids (`e.getType().equals(c)`).
    Types(Vec<String>),
    /// `follow player <name...>`: these entities (`entities::contains`, which is `Entity.equals`: the same id).
    Entities(Vec<u32>),
    /// What `FollowProcess.pickup` follows: a dropped item whose stack is one of these items.
    Items(Vec<String>),
}

impl FollowFilter {
    /// `filter.test(entity)`.
    pub fn test(&self, e: &EntityInfo) -> bool {
        match self {
            FollowFilter::Living => e.living,
            FollowFilter::Players => e.kind == "minecraft:player",
            FollowFilter::Types(types) => types.contains(&e.kind),
            FollowFilter::Entities(ids) => ids.contains(&e.id),
            FollowFilter::Items(items) => e.kind == "minecraft:item" && e.item.as_ref().is_some_and(|i| items.contains(i)),
        }
    }
}

/// What `FollowProcess` reads besides its own filter: the entities, where the player is, and the settings.
#[derive(Clone, Copy, Debug)]
pub struct FollowEnv<'a> {
    pub inputs: &'a ProcessInputs,
    /// `ctx.player().position()`.
    pub player: [f64; 3],
    pub settings: &'a FollowSettings,
}

/// `FollowProcess`.
#[derive(Clone, Debug, Default)]
pub struct FollowProcess {
    filter: Option<FollowFilter>,
    /// Walk straight into the target, regardless of settings (`pickup`).
    into: bool,
}

impl FollowProcess {
    /// `FollowProcess.<init>`: `FollowProcess::new`; no filter.
    pub fn new() -> FollowProcess {
        FollowProcess::default()
    }

    /// `FollowProcess.follow`: sets the filter and turns `into` off.
    pub fn follow(&mut self, filter: FollowFilter) {
        self.filter = Some(filter);
        self.into = false;
    }

    /// `FollowProcess.pickup`: follow the dropped items of these kinds and walk into them. (Upstream takes a `Predicate<ItemStack>`; the item names stand in.)
    pub fn pickup(&mut self, items: Vec<String>) {
        self.filter = Some(FollowFilter::Items(items));
        self.into = true;
    }

    /// `FollowProcess.currentFilter`: the filter, if one is set.
    pub fn current_filter(&self) -> Option<&FollowFilter> {
        self.filter.as_ref()
    }

    /// Whether a filter is set. The process can be inactive with one (no entity passes it), and starts again when one does.
    pub fn has_filter(&self) -> bool {
        self.filter.is_some()
    }

    /// `FollowProcess.followable`: alive, and within `followTargetMaxDistance` of the player when that is set.
    fn followable(e: &EntityInfo, env: &FollowEnv<'_>) -> bool {
        if !e.alive {
            return false;
        }
        // upstream also refuses `ctx.player()`: the glue never lists it
        let max_dist = env.settings.follow_target_max_distance;
        if max_dist != 0 {
            let (dx, dy, dz) = (e.position[0] - env.player[0], e.position[1] - env.player[1], e.position[2] - env.player[2]);
            // `maxDist * maxDist` is an int product
            if dx * dx + dy * dy + dz * dz > f64::from(max_dist.wrapping_mul(max_dist)) {
                return false;
            }
        }
        true
    }

    /// `FollowProcess.scanWorld`: the entities that pass the filter and `followable`, each once, in the order the world lists them. Upstream keeps the
    /// answer in a field; it is worked out where it is read, which gives the same list within a tick.
    fn scan_world<'a>(&self, env: &FollowEnv<'a>) -> Vec<&'a EntityInfo> {
        let Some(filter) = &self.filter else {
            return vec![];
        };
        let mut out: Vec<&EntityInfo> = vec![];
        // `distinct()` is a hash set upstream: linear, not a search through `out` for every entity
        let mut seen = std::collections::HashSet::new();
        for e in &env.inputs.entities {
            if Self::followable(e, env) && filter.test(e) && seen.insert(e.id) {
                out.push(e);
            }
        }
        out
    }

    /// `FollowProcess.following`: the entities being followed, by the inputs of this tick.
    pub fn following<'a>(&self, env: &FollowEnv<'a>) -> Vec<&'a EntityInfo> {
        self.scan_world(env)
    }

    /// `FollowProcess.isActive`: there is a filter and an entity passes it.
    pub fn is_active(&self, env: &FollowEnv<'_>) -> bool {
        self.filter.is_some() && !self.scan_world(env).is_empty()
    }

    /// `FollowProcess.towards`: the goal for one entity.
    fn towards(&self, e: &EntityInfo, s: &FollowSettings) -> Goal {
        let (x, y, z);
        if s.follow_offset_distance == 0.0 || self.into {
            // `Entity.blockPosition`
            x = mth::floor(e.position[0]);
            y = mth::floor(e.position[1]);
            z = mth::floor(e.position[2]);
        } else {
            let g = goal_xz_from_direction(e.position[0], e.position[2], s.follow_offset_direction, s.follow_offset_distance);
            let Goal::Xz { x: gx, z: gz } = g else { unreachable!("fromDirection makes a GoalXZ") };
            x = gx;
            y = mth::floor(e.position[1]);
            z = gz;
        }
        if self.into {
            return Goal::Block { x, y, z };
        }
        goal_near(x, y, z, s.follow_radius)
    }

    /// `FollowProcess.onTick`: the goal is a composite of the goals of the entities followed, revalidated every tick.
    pub fn on_tick(&self, env: &FollowEnv<'_>) -> PathingCommand {
        let goals = self.scan_world(env).into_iter().map(|e| self.towards(e, env.settings)).collect();
        PathingCommand { goal: Some(Goal::Composite(goals)), kind: PathingCommandType::RevalidateGoalAndPath }
    }

    /// `FollowProcess.onLostControl`: forgets the filter.
    pub fn on_lost_control(&mut self) {
        self.filter = None;
    }

    /// `FollowProcess.displayName0`: `"Following " + cache`, the cache printed as a list of entities.
    pub fn display_name(&self, env: &FollowEnv<'_>) -> String {
        if self.filter.is_none() {
            return "Following null".to_string();
        }
        format!("Following [{}]", self.scan_world(env).into_iter().map(entity_to_string).collect::<Vec<_>>().join(", "))
    }
}

/// Upstream's `Entity.toString`: `Class['Name'/id, l='ClientLevel', x=0.00, y=0.00, z=0.00]`. The game's class names and display names are not known here, so the
/// registry path is made into a class-like name (`zombie_villager` is `ZombieVillager`) and a title-like one (`Zombie Villager`); a player's own name is exact.
pub fn entity_to_string(e: &EntityInfo) -> String {
    let path = e.kind.split_once(':').map_or(e.kind.as_str(), |(_, p)| p);
    let class = if e.kind == "minecraft:player" { "RemotePlayer".to_string() } else { camel(path, "") };
    let name = e.player_name.clone().unwrap_or_else(|| camel(path, " "));
    entity_string(&class, &name, e.id as i32, e.position)
}

/// The text `Entity.toString` makes from its parts.
pub fn entity_string(class: &str, name: &str, id: i32, p: [f64; 3]) -> String {
    format!("{class}['{name}'/{id}, l='ClientLevel', x={:.2}, y={:.2}, z={:.2}]", p[0], p[1], p[2])
}

fn camel(path: &str, sep: &str) -> String {
    path.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: u32, kind: &str, living: bool) -> EntityInfo {
        EntityInfo { id, kind: format!("minecraft:{kind}"), player_name: (kind == "player").then(|| format!("P{id}")), item: None, position: [0.5, 1.0, 0.5], living, alive: true }
    }

    fn env<'a>(inputs: &'a ProcessInputs, s: &'a FollowSettings) -> FollowEnv<'a> {
        FollowEnv { inputs, player: [0.0, 1.0, 0.0], settings: s }
    }

    fn world() -> ProcessInputs {
        ProcessInputs { entities: vec![e(1, "pig", true), e(2, "cow", true), e(3, "player", true), e(4, "item", false), e(5, "pig", true), e(6, "arrow", false), e(1, "pig", true)], ..Default::default() }
    }

    fn ids(p: &FollowProcess, inputs: &ProcessInputs) -> Vec<u32> {
        let s = FollowSettings::default();
        p.following(&env(inputs, &s)).iter().map(|e| e.id).collect()
    }

    #[test]
    fn a_scan_of_many_entities_is_linear() {
        // review finding 1: the dedupe searched the answer for each entity, so 4096 entities cost 8 ms per scan, several scans a tick. 40000 entities
        // with a quadratic dedupe take seconds even in release; a linear one takes milliseconds.
        let entities: Vec<EntityInfo> = (0..40_000u32).map(|i| e(i, "pig", true)).collect();
        let w = ProcessInputs { entities, ..Default::default() };
        let s = FollowSettings { follow_target_max_distance: 0, ..FollowSettings::default() };
        let mut p = FollowProcess::new();
        p.follow(FollowFilter::Living);
        let t = std::time::Instant::now();
        let n = p.following(&env(&w, &s)).len();
        assert_eq!(n, 40_000);
        assert!(t.elapsed() < std::time::Duration::from_millis(500), "scan took {:?}", t.elapsed());
    }

    #[test]
    fn the_groups_pick_living_entities_and_players() {
        let w = world();
        let mut p = FollowProcess::new();
        p.follow(FollowFilter::Living);
        assert_eq!(ids(&p, &w), vec![1, 2, 3, 5], "every LivingEntity, a player too, once each");
        p.follow(FollowFilter::Players);
        assert_eq!(ids(&p, &w), vec![3]);
    }

    #[test]
    fn a_type_filter_picks_that_type_only() {
        let w = world();
        let mut p = FollowProcess::new();
        p.follow(FollowFilter::Types(vec!["minecraft:pig".into()]));
        assert_eq!(ids(&p, &w), vec![1, 5]);
        p.follow(FollowFilter::Types(vec!["minecraft:cow".into(), "minecraft:arrow".into()]));
        assert_eq!(ids(&p, &w), vec![2, 6]);
        p.follow(FollowFilter::Types(vec!["minecraft:item".into()]));
        assert_eq!(ids(&p, &w), vec![4], "a non-living entity can be followed by type");
        p.follow(FollowFilter::Types(vec!["pig".into()]));
        assert!(ids(&p, &w).is_empty(), "types are ids with their namespace");
    }

    #[test]
    fn an_entity_filter_picks_those_entities() {
        let w = world();
        let mut p = FollowProcess::new();
        p.follow(FollowFilter::Entities(vec![3, 6, 99]));
        assert_eq!(ids(&p, &w), vec![3, 6]);
        p.follow(FollowFilter::Entities(vec![]));
        assert!(ids(&p, &w).is_empty());
        assert!(p.has_filter() && !p.is_active(&env(&w, &FollowSettings::default())));
    }

    #[test]
    fn the_dead_and_the_far_are_not_followable() {
        let mut w = world();
        w.entities.pop(); // the repeated entity
        w.entities[0].alive = false;
        w.entities[1].position = [30.5, 1.0, 0.5];
        let mut p = FollowProcess::new();
        p.follow(FollowFilter::Living);
        assert_eq!(ids(&p, &w), vec![2, 3, 5]);
        let s = FollowSettings { follow_target_max_distance: 10, ..Default::default() };
        assert_eq!(p.following(&env(&w, &s)).iter().map(|e| e.id).collect::<Vec<_>>(), vec![3, 5]);
        // exactly at the distance is fine, beyond is not: the comparison is on squares
        let mut w2 = ProcessInputs { entities: vec![e(1, "pig", true), e(2, "pig", true)], ..Default::default() };
        w2.entities[0].position = [0.0, 1.0, 10.0];
        w2.entities[1].position = [0.0, 1.0, 10.001];
        assert_eq!(p.following(&env(&w2, &s)).iter().map(|e| e.id).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn a_dropped_item_is_picked_up_by_walking_into_it() {
        let mut item = e(4, "item", false);
        item.item = Some("minecraft:diamond".to_string());
        let mut stick = e(5, "item", false);
        stick.item = Some("minecraft:stick".to_string());
        item.position = [10.7, 1.0, -3.2];
        let w = ProcessInputs { entities: vec![item, stick], ..Default::default() };
        let mut p = FollowProcess::new();
        p.pickup(vec!["minecraft:diamond".to_string()]);
        let s = FollowSettings { follow_offset_distance: 4.0, ..Default::default() };
        let cmd = p.on_tick(&env(&w, &s));
        // into: the block the item is in, whatever the offset is
        assert_eq!(cmd.goal, Some(Goal::Composite(vec![Goal::Block { x: 10, y: 1, z: -4 }])));
        assert_eq!(cmd.kind, PathingCommandType::RevalidateGoalAndPath);
    }

    #[test]
    fn the_goal_is_a_near_goal_around_each_entity_and_is_revalidated() {
        let w = world();
        let mut p = FollowProcess::new();
        p.follow(FollowFilter::Types(vec!["minecraft:pig".into()]));
        let cmd = p.on_tick(&env(&w, &FollowSettings::default()));
        assert_eq!(cmd.kind, PathingCommandType::RevalidateGoalAndPath);
        assert_eq!(cmd.goal, Some(Goal::Composite(vec![goal_near(0, 1, 0, 3), goal_near(0, 1, 0, 3)])));
        let s = FollowSettings { follow_radius: 7, ..Default::default() };
        assert_eq!(p.on_tick(&env(&w, &s)).goal, Some(Goal::Composite(vec![goal_near(0, 1, 0, 7), goal_near(0, 1, 0, 7)])));
    }

    #[test]
    fn losing_control_forgets_the_filter() {
        let w = world();
        let mut p = FollowProcess::new();
        assert!(!p.is_active(&env(&w, &FollowSettings::default())));
        p.follow(FollowFilter::Living);
        assert!(p.is_active(&env(&w, &FollowSettings::default())));
        p.on_lost_control();
        assert!(!p.has_filter() && !p.is_active(&env(&w, &FollowSettings::default())));
        assert_eq!(p.display_name(&env(&w, &FollowSettings::default())), "Following null");
    }

    #[test]
    fn entities_print_like_entity_to_string() {
        let mut pl = e(3, "player", true);
        pl.position = [1.0, 64.0, -2.345];
        assert_eq!(entity_to_string(&pl), "RemotePlayer['P3'/3, l='ClientLevel', x=1.00, y=64.00, z=-2.35]");
        assert_eq!(entity_to_string(&e(9, "zombie_villager", true)), "ZombieVillager['Zombie Villager'/9, l='ClientLevel', x=0.50, y=1.00, z=0.50]");
    }
}
