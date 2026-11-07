// Scenario for the break and place probe of baritone-rs. It sets the hotbar slot, breaks and places blocks and uses a water
// bucket, with no screen and with the chat open, and writes one CSV row per tick. Pure logic: the helpers read the game and
// apply what this returns. License: LGPL-3.0-or-later, like the rest of baritone-rs. Not ported from Baritone.

pub const ID: &str = "baritone_bp_probe";
pub const NAME: &str = "baritone-rs break and place probe";
pub const VERSION: &str = "0.0.0";
pub const DESCRIPTION: &str = "Test only: with BARITONE_BP_DIR set, breaks and places blocks through the game's own input step.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Slot,
    Break,
    Place,
    Pillar,
    Bucket,
}

#[derive(Clone, Copy, Debug)]
pub struct Trial {
    pub name: &'static str,
    pub kind: Kind,
    /// The chat is open during the action.
    pub chat: bool,
    /// With the chat open: the game's input step runs with the screen Playing (the mod's way). False is the control.
    pub swap: bool,
    pub slot: usize,
    /// The block put at the break target (Break).
    pub block: &'static str,
    /// Ticks after which the trial ends.
    pub win: u64,
}

const fn t(name: &'static str, kind: Kind, chat: bool, swap: bool, slot: usize, block: &'static str, win: u64) -> Trial {
    Trial { name, kind, chat, swap, slot, block, win }
}

// Hotbar: slot 0 iron pickaxe, slot 1 stone x64, slot 2 water bucket; slot 8 is empty.
pub const TRIALS: [Trial; 18] = [
    t("slot_playing", Kind::Slot, false, true, 0, "", 40),
    t("slot_chat_swap", Kind::Slot, true, true, 0, "", 40),
    t("slot_chat_noswap", Kind::Slot, true, false, 0, "", 40),
    t("break_dirt_hand_playing", Kind::Break, false, true, 8, "minecraft:dirt", 80),
    t("break_dirt_hand_chat", Kind::Break, true, true, 8, "minecraft:dirt", 80),
    t("break_stone_pick_playing", Kind::Break, false, true, 0, "minecraft:stone", 80),
    t("break_stone_pick_chat", Kind::Break, true, true, 0, "minecraft:stone", 80),
    t("break_stone_pick_chat_noswap", Kind::Break, true, false, 0, "minecraft:stone", 60),
    t("break_stone_hand_playing", Kind::Break, false, true, 8, "minecraft:stone", 260),
    t("break_stone_hand_chat", Kind::Break, true, true, 8, "minecraft:stone", 260),
    t("place_floor_playing", Kind::Place, false, true, 1, "", 40),
    t("place_floor_chat", Kind::Place, true, true, 1, "", 40),
    t("place_floor_chat_noswap", Kind::Place, true, false, 1, "", 40),
    t("pillar_playing", Kind::Pillar, false, true, 1, "", 70),
    t("pillar_chat", Kind::Pillar, true, true, 1, "", 70),
    t("bucket_playing", Kind::Bucket, false, true, 2, "", 200),
    t("bucket_chat", Kind::Bucket, true, true, 2, "", 200),
    t("pillar_chat_noswap", Kind::Pillar, true, false, 1, "", 50),
];

/// The break target (eye level, 1.5 blocks south of the start), the floor block in front, and the block under the feet.
const TARGET: [i32; 3] = [1000, 101, 1002];
const FRONT: [i32; 3] = [1000, 100, 1002];
const UNDER: [i32; 3] = [1000, 100, 1000];
pub const TARGET_POS: [i32; 3] = TARGET;
pub const FRONT_POS: [i32; 3] = FRONT;
pub const UNDER_POS: [i32; 3] = UNDER;
const START: [f64; 3] = [1000.5, 100.0, 1000.5];
/// Pitch (down) from the eye at 101.62 to the top face of the floor block in front, at its middle: atan(1.62 / 2).
const PITCH_FLOOR: f32 = 39.0;

const BOOT_FORCELOAD: u64 = 100;
const BOOT_FILL: u64 = 140;
const BOOT_MODE: u64 = 165;
const BOOT_DONE: u64 = 170;
const SETTLE: u64 = 30;
const GIVE_UP: u64 = 12000;

pub struct Obs {
    pub feet_y: f64,
    pub on_ground: bool,
    pub chat: bool,
    pub selected: usize,
    pub held: String,
    pub target: String,
    pub front: String,
    pub under: String,
    /// The game's own mining numbers for the target, when asked (`wants_tool`).
    pub tool: Option<String>,
}

#[derive(Default)]
pub struct Out {
    pub console: Vec<String>,
    /// Some(true): open the chat, Some(false): close it.
    pub chat: Option<bool>,
    pub selected: Option<usize>,
    pub aim: Option<(f32, f32)>,
    pub finished: Option<bool>,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Intent {
    pub active: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub swap: bool,
}

#[derive(Default)]
struct Probe {
    total: u64,
    boot: u64,
    trial: usize,
    r: u64,
    done: bool,
    intent: Intent,
    swapped: bool,
    want_tool: bool,
    rows: Vec<String>,
    console: Vec<String>,
    summary: Vec<String>,
    answers: Vec<(String, String)>,
    t0: Option<std::time::Instant>,
    // Per-trial state.
    t_client: Option<u64>,
    t_server: Option<u64>,
    ms_client: Option<u128>,
    ms_server: Option<u128>,
    t_client2: Option<u64>,
    t_server2: Option<u64>,
    tries: u32,
    last_try: u64,
    end_at: Option<u64>,
}

const HEADER: &str = "trial,a,feet_y,on_ground,chat,selected,held,left,right,jump,swap,target,front,under";

thread_local! {
    static PROBE: std::cell::RefCell<Probe> = std::cell::RefCell::new(Probe::default());
}

static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

fn dir() -> Option<&'static std::path::PathBuf> {
    DIR.get_or_init(|| std::env::var_os("BARITONE_BP_DIR").map(Into::into)).as_ref()
}

pub fn enabled() -> bool {
    dir().is_some()
}

pub fn intent() -> Intent {
    PROBE.with(|p| p.borrow().intent)
}

pub fn set_swapped(on: bool) {
    PROBE.with(|p| p.borrow_mut().swapped = on);
}

pub fn take_swapped() -> bool {
    PROBE.with(|p| std::mem::take(&mut p.borrow_mut().swapped))
}

pub fn wants_tool() -> bool {
    PROBE.with(|p| std::mem::take(&mut p.borrow_mut().want_tool))
}

pub fn target_pos() -> [i32; 3] {
    TARGET
}

pub fn log_console(command: &str, answer: &str) {
    PROBE.with(|p| {
        let mut p = p.borrow_mut();
        let ms = p.t0.map_or(0, |t| t.elapsed().as_millis());
        p.console.push(format!("[{ms} ms] {command} -> {answer}"));
        p.answers.push((command.to_string(), answer.to_string()));
    });
}

pub fn write_all() -> bool {
    let Some(dir) = dir() else { return false };
    PROBE.with(|p| {
        let p = p.borrow();
        std::fs::create_dir_all(dir).is_ok()
            && std::fs::write(dir.join("probe.csv"), format!("{HEADER}\n{}\n", p.rows.join("\n"))).is_ok()
            && std::fs::write(dir.join("probe-console.txt"), p.console.join("\n") + "\n").is_ok()
            && std::fs::write(dir.join("summary.txt"), p.summary.join("\n") + "\n").is_ok()
    })
}

pub fn step(obs: Option<Obs>) -> Out {
    PROBE.with(|p| p.borrow_mut().step(obs))
}

fn is(name: &str, id: &str) -> bool {
    name.trim_start_matches("minecraft:") == id
}

fn passed(answer: &str) -> bool {
    answer.to_lowercase().contains("passed")
}

fn at(p: [i32; 3]) -> String {
    format!("{} {} {}", p[0], p[1], p[2])
}

impl Probe {
    fn reset_trial_state(&mut self) {
        self.t_client = None;
        self.t_server = None;
        self.ms_client = None;
        self.ms_server = None;
        self.t_client2 = None;
        self.t_server2 = None;
        self.tries = 0;
        self.last_try = 0;
        self.end_at = None;
    }

    fn row(&mut self, name: &str, a: u64, o: &Obs) {
        let i = self.intent;
        self.rows.push(format!(
            "{name},{a},{:.4},{},{},{},{},{},{},{},{},{},{},{}",
            o.feet_y,
            o.on_ground as u8,
            o.chat as u8,
            o.selected,
            o.held,
            i.left as u8,
            i.right as u8,
            i.jump as u8,
            i.swap as u8,
            o.target,
            o.front,
            o.under,
        ));
    }

    /// Answers of the polls of the last step: (client tick of the poll is a - 1).
    fn polled(&mut self, prefix: &str) -> Option<bool> {
        let mut found = None;
        for (cmd, ans) in std::mem::take(&mut self.answers) {
            if cmd.starts_with(prefix) {
                found = Some(passed(&ans));
            }
        }
        found
    }

    fn ms(&self) -> u128 {
        self.t0.map_or(0, |t| t.elapsed().as_millis())
    }

    fn finish_trial(&mut self, t: Trial, a: u64, out: &mut Out) {
        self.summary.push(format!(
            "RESULT {} kind={:?} chat={} swap={} client_tick={:?} client_ms={:?} server_tick={:?} server_ms={:?} second_client={:?} second_server={:?} ticks_run={}",
            t.name, t.kind, t.chat, t.swap, self.t_client, self.ms_client, self.t_server, self.ms_server, self.t_client2, self.t_server2, a
        ));
        self.intent.left = false;
        self.intent.right = false;
        self.intent.jump = false;
        self.r = 0;
        self.trial += 1;
        self.reset_trial_state();
        if self.trial == TRIALS.len() {
            self.done = true;
            self.intent = Intent::default();
            out.chat = Some(false);
            out.finished = Some(true);
        }
    }

    fn step(&mut self, obs: Option<Obs>) -> Out {
        let mut out = Out::default();
        if self.done {
            return out;
        }
        let Some(o) = obs else { return out };
        self.total += 1;
        if self.total > GIVE_UP {
            self.done = true;
            self.intent = Intent::default();
            out.finished = Some(false);
            return out;
        }
        if self.boot < BOOT_DONE {
            self.boot += 1;
            match self.boot {
                BOOT_FORCELOAD => {
                    out.console.push("difficulty peaceful".into());
                    out.console.push("time set day".into());
                    out.console.push("gamerule doMobSpawning false".into());
                    out.console.push("forceload add 970 970 1060 1060".into());
                }
                BOOT_FILL => {
                    out.console.push("fill 980 99 980 1050 99 1050 stone".into());
                    out.console.push("fill 980 100 980 1050 105 1050 air".into());
                    out.console.push("fill 980 106 980 1050 111 1050 air".into());
                }
                BOOT_MODE => out.console.push("gamemode survival @p".into()),
                _ => {}
            }
            return out;
        }
        let tr = TRIALS[self.trial];
        let r = self.r;
        if r == 0 {
            // Back to the start with the screen closed, an empty inventory and the three items.
            self.t0 = None;
            self.intent = Intent { active: true, ..Intent::default() };
            self.answers.clear();
            out.chat = Some(false);
            out.selected = Some(3);
            out.aim = Some((0.0, 0.0));
            for c in [
                "clear @p".to_string(),
                "kill @e[type=item]".to_string(),
                format!("tp @p {:.1} {:.1} {:.1} 0 0", START[0], START[1], START[2]),
                "fill 996 100 996 1005 106 1008 air".to_string(),
            ] {
                out.console.push(c);
            }
            if tr.kind == Kind::Break {
                out.console.push(format!("setblock {} {}", at(TARGET), tr.block));
            }
            out.console.push("give @p minecraft:iron_pickaxe".into());
            out.console.push("give @p minecraft:stone 64".into());
            out.console.push("give @p minecraft:water_bucket".into());
            self.r += 1;
            return out;
        }
        if r < SETTLE - 1 {
            self.r += 1;
            return out;
        }
        if r == SETTLE - 1 {
            self.t0 = Some(std::time::Instant::now());
            out.console.push("data get entity @p Inventory".into());
            out.chat = Some(tr.chat);
            self.intent.swap = tr.swap;
        }
        let a = r - (SETTLE - 1);
        self.row(tr.name, a, &o);
        if let Some(tool) = &o.tool {
            self.summary.push(format!("TOOL {} a={a} {tool}", tr.name));
        }
        match tr.kind {
            Kind::Slot => self.slot_trial(a, &mut out),
            Kind::Break => self.break_trial(tr, a, &o, &mut out),
            Kind::Place => self.place_trial(tr, a, &o, &mut out),
            Kind::Pillar => self.pillar_trial(tr, a, &o, &mut out),
            Kind::Bucket => self.bucket_trial(tr, a, &o, &mut out),
        }
        if self.end_at.is_some_and(|e| a >= e) || a >= tr.win {
            if self.end_at.is_none() {
                self.summary.push(format!("NOTE {} timed out at a={a}", tr.name));
            }
            self.finish_trial(tr, a, &mut out);
            return out;
        }
        self.r += 1;
        out
    }

    fn slot_trial(&mut self, a: u64, out: &mut Out) {
        // Select a slot, wait for the tick that sends it, and ask the server what the player holds.
        for (start, slot) in [(0u64, 1usize), (6, 2), (12, 8), (18, 0)] {
            if a == start {
                out.selected = Some(slot);
            }
            if a == start + 4 {
                out.console.push("data get entity @p SelectedItemSlot".into());
                out.console.push("data get entity @p SelectedItem".into());
            }
        }
        if a == 24 {
            self.end_at = Some(25);
        }
    }

    fn break_trial(&mut self, tr: Trial, a: u64, o: &Obs, out: &mut Out) {
        let poll = format!("execute if block {} minecraft:air", at(TARGET));
        if a == 0 {
            out.selected = Some(tr.slot);
            out.aim = Some((0.0, 0.0));
            self.intent.left = true;
            self.want_tool = true;
            return;
        }
        if let Some(true) = self.polled(&poll)
            && self.t_server.is_none()
        {
            self.t_server = Some(a - 1);
            self.ms_server = Some(self.ms());
        }
        if self.t_client.is_none() && is(&o.target, "air") {
            self.t_client = Some(a);
            self.ms_client = Some(self.ms());
            self.intent.left = false;
        }
        if self.end_at.is_none() {
            out.console.push(poll);
        }
        // Keep polling the server a little after the client saw it, then ask what dropped.
        let seen = self.t_client.into_iter().chain(self.t_server).min();
        if let Some(s) = seen
            && self.end_at.is_none()
            && a >= s + 3
            && self.t_server.is_some()
        {
            let pos = "1000.5 101.5 1002.5";
            out.console.push(format!("execute positioned {pos} if entity @e[type=item,distance=..3]"));
            out.console.push(format!("execute positioned {pos} run data get entity @e[type=item,distance=..3,limit=1,sort=nearest] Item"));
            out.console.push("data get entity @p SelectedItem".into());
            out.console.push("data get entity @p Inventory".into());
            self.end_at = Some(a + 3);
        }
        if a + 1 == tr.win && self.end_at.is_none() {
            out.console.push(format!("execute if block {} {}", at(TARGET), tr.block));
            out.console.push("data get entity @p SelectedItem".into());
        }
    }

    fn place_trial(&mut self, tr: Trial, a: u64, o: &Obs, out: &mut Out) {
        let poll = format!("execute if block {} minecraft:stone", at(FRONT));
        if a == 0 {
            out.selected = Some(tr.slot);
            out.aim = Some((0.0, PITCH_FLOOR));
        }
        if a == 3 {
            self.intent.right = true;
        }
        if a == 4 {
            self.intent.right = false;
        }
        if a >= 3 {
            if let Some(true) = self.polled(&poll)
                && self.t_server.is_none()
            {
                self.t_server = Some(a - 1);
                self.ms_server = Some(self.ms());
            }
            if self.t_client.is_none() && is(&o.front, "stone") {
                self.t_client = Some(a);
                self.ms_client = Some(self.ms());
            }
            if self.end_at.is_none() {
                out.console.push(poll);
            }
        }
        if a == 14 {
            out.console.push("data get entity @p SelectedItem".into());
            self.end_at = Some(15);
        }
    }

    fn pillar_trial(&mut self, tr: Trial, a: u64, o: &Obs, out: &mut Out) {
        let poll = format!("execute if block {} minecraft:stone", at(UNDER));
        if a == 0 {
            out.selected = Some(tr.slot);
            out.aim = Some((0.0, 90.0));
        }
        if a == 4 {
            self.intent.jump = true;
        }
        if a >= 5 {
            if let Some(true) = self.polled(&poll)
                && self.t_server.is_none()
            {
                self.t_server = Some(a - 1);
                self.ms_server = Some(self.ms());
            }
            if self.t_client.is_none() && is(&o.under, "stone") {
                self.t_client = Some(a);
                self.ms_client = Some(self.ms());
                self.intent.jump = false;
            }
            if self.end_at.is_none() {
                out.console.push(poll);
            }
        }
        // The use button is pressed for one tick at a time, while rising above the block's top.
        if self.intent.right {
            self.intent.right = false;
        } else if a >= 5 && self.t_client.is_none() && self.tries < 6 && o.feet_y > 101.02 && (self.tries == 0 || a >= self.last_try + 3) {
            self.intent.right = true;
            self.tries += 1;
            self.last_try = a;
            self.summary.push(format!("NOTE {} use pressed at a={a} feet_y={:.4}", tr.name, o.feet_y));
        }
        if self.t_client.is_some() && self.end_at.is_none() && a >= self.t_client.unwrap_or(0) + 25 {
            out.console.push("data get entity @p Pos".into());
            out.console.push("data get entity @p SelectedItem".into());
            self.end_at = Some(a + 2);
        }
        if a + 1 == tr.win && self.end_at.is_none() {
            out.console.push("data get entity @p Pos".into());
        }
    }

    fn bucket_trial(&mut self, tr: Trial, a: u64, o: &Obs, out: &mut Out) {
        if a == 0 {
            out.selected = Some(tr.slot);
            out.aim = Some((0.0, PITCH_FLOOR));
        }
        // First click: water onto the floor. Second click, same aim: the bucket takes it back.
        if a == 3 || a == 17 {
            self.intent.right = true;
        }
        if a == 4 || a == 18 {
            self.intent.right = false;
        }
        if (3..16).contains(&a) {
            if let Some(true) = self.polled(&format!("execute if block {} minecraft:water", at(FRONT)))
                && self.t_server.is_none()
            {
                self.t_server = Some(a - 1);
                self.ms_server = Some(self.ms());
            }
            if self.t_client.is_none() && is(&o.front, "water") {
                self.t_client = Some(a);
                self.ms_client = Some(self.ms());
            }
            out.console.push(format!("execute if block {} minecraft:water", at(FRONT)));
        }
        if a == 15 {
            out.console.push("data get entity @p SelectedItem".into());
        }
        if a >= 17 {
            // The source is gone at once; flowing water may stay for a while until it decays. Poll until none is left.
            if let Some(false) = self.polled(&format!("execute if block {} minecraft:water", at(FRONT)))
                && self.t_server2.is_none()
            {
                self.t_server2 = Some(a - 1);
            }
            if self.t_client2.is_none() && !is(&o.front, "water") && a >= 18 {
                self.t_client2 = Some(a);
            }
            if self.t_server2.is_none() {
                out.console.push(format!("execute if block {} minecraft:water", at(FRONT)));
            }
        }
        if a == 24 || a == 60 {
            out.console.push("data get entity @p SelectedItem".into());
        }
        if a >= 150 || (a >= 30 && self.t_server2.is_some() && self.t_client2.is_some()) {
            out.console.push("data get entity @p SelectedItem".into());
            self.end_at = Some(a + 1);
        }
    }
}
