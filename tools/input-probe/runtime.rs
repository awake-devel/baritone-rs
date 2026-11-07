// Scenario for the input probe of baritone-rs: drives the player through `native_movement_input` with no screen, the chat
// and the inventory open, and writes one CSV row per tick. Pure logic: the helpers read the game and apply what this
// returns. License: LGPL-3.0-or-later, like the rest of baritone-rs. Not ported from Baritone.

pub const ID: &str = "baritone_probe";
pub const NAME: &str = "baritone-rs input probe";
pub const VERSION: &str = "0.0.0";
pub const DESCRIPTION: &str = "Test only: with BARITONE_PROBE_DIR set, walks a fixed scenario through the movement hook.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenKind {
    Playing,
    Chat,
    Inventory,
    Other,
}

impl ScreenKind {
    fn name(self) -> &'static str {
        match self {
            ScreenKind::Playing => "playing",
            ScreenKind::Chat => "chat",
            ScreenKind::Inventory => "inventory",
            ScreenKind::Other => "other",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SprintMode {
    /// Nothing sets the sprint flag.
    Never,
    /// `set_sprinting(true)` once, on the first driven tick.
    Once,
    /// `set_sprinting(true)` after every tick.
    Every,
}

#[derive(Clone, Copy, Debug)]
pub struct Trial {
    pub name: &'static str,
    pub screen: ScreenKind,
    /// None: the hook is not asked to do anything (the control).
    pub drive: Option<Drive>,
    pub sprint: SprintMode,
    pub yaw: f32,
    pub start: [f64; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drive {
    pub forward: f32,
    pub strafe: f32,
    pub jump: bool,
    pub sneak: bool,
}

const SOUTH_START: [f64; 3] = [1000.5, 100.0, 985.5];
const WEST_START: [f64; 3] = [1040.5, 100.0, 1000.5];
const FWD: Drive = Drive { forward: 1.0, strafe: 0.0, jump: false, sneak: false };

const fn trial(name: &'static str, screen: ScreenKind, drive: Option<Drive>, sprint: SprintMode) -> Trial {
    Trial { name, screen, drive, sprint, yaw: 0.0, start: SOUTH_START }
}

pub const TRIALS: [Trial; 17] = [
    trial("playing_walk", ScreenKind::Playing, Some(FWD), SprintMode::Never),
    trial("playing_sprint_once", ScreenKind::Playing, Some(FWD), SprintMode::Once),
    trial("playing_sprint_every", ScreenKind::Playing, Some(FWD), SprintMode::Every),
    trial("chat_idle_control", ScreenKind::Chat, None, SprintMode::Never),
    trial("chat_walk", ScreenKind::Chat, Some(FWD), SprintMode::Never),
    trial("chat_sprint_once", ScreenKind::Chat, Some(FWD), SprintMode::Once),
    trial("chat_sprint_every", ScreenKind::Chat, Some(FWD), SprintMode::Every),
    trial("chat_jump_walk", ScreenKind::Chat, Some(Drive { jump: true, ..FWD }), SprintMode::Never),
    trial("chat_jump_sprint_every", ScreenKind::Chat, Some(Drive { jump: true, ..FWD }), SprintMode::Every),
    trial("chat_sneak_walk", ScreenKind::Chat, Some(Drive { sneak: true, ..FWD }), SprintMode::Never),
    trial(
        "chat_strafe_left",
        ScreenKind::Chat,
        Some(Drive { forward: 0.0, strafe: 1.0, jump: false, sneak: false }),
        SprintMode::Never,
    ),
    Trial {
        name: "chat_west_yaw90_walk",
        screen: ScreenKind::Chat,
        drive: Some(FWD),
        sprint: SprintMode::Never,
        yaw: 90.0,
        start: WEST_START,
    },
    Trial {
        name: "chat_west_yaw90_sprint_every",
        screen: ScreenKind::Chat,
        drive: Some(FWD),
        sprint: SprintMode::Every,
        yaw: 90.0,
        start: WEST_START,
    },
    trial("inventory_walk", ScreenKind::Inventory, Some(FWD), SprintMode::Never),
    trial("inventory_sprint_once", ScreenKind::Inventory, Some(FWD), SprintMode::Once),
    trial("inventory_sprint_every", ScreenKind::Inventory, Some(FWD), SprintMode::Every),
    trial("inventory_idle_control", ScreenKind::Inventory, None, SprintMode::Never),
];

const BOOT_FORCELOAD: u64 = 100;
const BOOT_FILL: u64 = 140;
const BOOT_DONE: u64 = 170;
const SETTLE: u64 = 25;
const DRIVE: u64 = 60;
const TAIL: u64 = 12;
const GIVE_UP: u64 = 9000;

pub struct Obs {
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub yaw: f32,
    pub on_ground: bool,
    pub sprinting: bool,
    pub sprint_modifier: bool,
    pub screen: ScreenKind,
}

#[derive(Default)]
pub struct Out {
    pub console: Vec<String>,
    pub screen: Option<ScreenKind>,
    pub yaw: Option<f32>,
    pub set_sprinting: Option<bool>,
    pub finished: Option<bool>,
}

/// What the hook saw and did in the tick that just ran.
#[derive(Default, Clone, Copy)]
struct HookLog {
    calls: u32,
    original: [f32; 2],
    original_jump: bool,
    original_sneak: bool,
}

#[derive(Default)]
struct Probe {
    total: u64,
    boot: u64,
    trial: usize,
    r: u64,
    done: bool,
    prev: Option<[f64; 3]>,
    drive: Option<Drive>,
    hook: HookLog,
    rows: Vec<String>,
    console: Vec<String>,
}

const HEADER: &str = "trial,phase,r,x,y,z,yaw,on_ground,sprinting,sprint_modifier,vx,vz,hspeed_bps,screen,hook_calls,game_forward,game_strafe,game_jump,game_sneak,drive_forward,drive_strafe,drive_jump,drive_sneak";

thread_local! {
    static PROBE: std::cell::RefCell<Probe> = std::cell::RefCell::new(Probe::default());
}

static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

fn dir() -> Option<&'static std::path::PathBuf> {
    DIR.get_or_init(|| std::env::var_os("BARITONE_PROBE_DIR").map(Into::into)).as_ref()
}

pub fn enabled() -> bool {
    dir().is_some()
}

/// The hook: records what the game read, then replaces it with the drive of the moment.
pub fn apply_hook(forward: &mut f32, strafe: &mut f32, jump: &mut bool, sneak: &mut bool) {
    PROBE.with(|p| {
        let mut p = p.borrow_mut();
        p.hook.calls += 1;
        p.hook.original = [*forward, *strafe];
        p.hook.original_jump = *jump;
        p.hook.original_sneak = *sneak;
        if let Some(d) = p.drive {
            *forward = d.forward;
            *strafe = d.strafe;
            *jump = d.jump;
            *sneak = d.sneak;
        }
    });
}

pub fn log_console(command: &str, answer: &str) {
    PROBE.with(|p| p.borrow_mut().console.push(format!("{command} -> {answer}")));
}

/// Writes the rows and the console log. True when both files are written.
pub fn write_all() -> bool {
    let Some(dir) = dir() else { return false };
    PROBE.with(|p| {
        let p = p.borrow();
        let csv = format!("{HEADER}\n{}\n", p.rows.join("\n"));
        let console = p.console.join("\n") + "\n";
        std::fs::create_dir_all(dir).is_ok()
            && std::fs::write(dir.join("probe.csv"), csv).is_ok()
            && std::fs::write(dir.join("probe-console.txt"), console).is_ok()
    })
}

pub fn step(obs: Option<Obs>) -> Out {
    PROBE.with(|p| p.borrow_mut().step(obs))
}

impl Probe {
    fn row(&mut self, name: &str, phase: &str, o: &Obs) {
        let speed = self.prev.map_or(0.0, |q| ((o.pos[0] - q[0]).powi(2) + (o.pos[2] - q[2]).powi(2)).sqrt() * 20.0);
        self.prev = Some(o.pos);
        let d = self.drive.unwrap_or_default();
        let h = self.hook;
        self.rows.push(format!(
            "{name},{phase},{},{:.5},{:.5},{:.5},{:.2},{},{},{},{:.5},{:.5},{:.3},{},{},{},{},{},{},{},{},{},{}",
            self.r,
            o.pos[0],
            o.pos[1],
            o.pos[2],
            o.yaw,
            o.on_ground as u8,
            o.sprinting as u8,
            o.sprint_modifier as u8,
            o.vel[0],
            o.vel[2],
            speed,
            o.screen.name(),
            h.calls,
            h.original[1],
            h.original[0],
            h.original_jump as u8,
            h.original_sneak as u8,
            d.forward,
            d.strafe,
            d.jump as u8,
            d.sneak as u8,
        ));
        self.hook = HookLog::default();
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
            self.drive = None;
            out.finished = Some(false);
            return out;
        }
        if self.boot < BOOT_DONE {
            self.boot += 1;
            self.hook = HookLog::default();
            match self.boot {
                BOOT_FORCELOAD => {
                    out.console.push("difficulty peaceful".into());
                    out.console.push("time set day".into());
                    out.console.push("forceload add 970 970 1060 1060".into());
                }
                BOOT_FILL => {
                    out.console.push("fill 980 99 980 1050 99 1050 stone".into());
                    // A fill takes at most 32768 blocks: 71 x 71 x 6 each.
                    out.console.push("fill 980 100 980 1050 105 1050 air".into());
                    out.console.push("fill 980 106 980 1050 111 1050 air".into());
                }
                _ => {}
            }
            return out;
        }
        let t = TRIALS[self.trial];
        let r = self.r;
        let phase = if r < SETTLE {
            "settle"
        } else if r < SETTLE + DRIVE {
            "drive"
        } else {
            "tail"
        };
        // The row is the state after this tick, with what the hook did in it.
        self.row(t.name, phase, &o);
        if r == 0 {
            // Back to the start with the screen closed and no sprint.
            self.drive = None;
            out.screen = Some(ScreenKind::Playing);
            out.set_sprinting = Some(false);
            out.console.push(format!(
                "tp @p {:.1} {:.1} {:.1} {:.1} 0",
                t.start[0], t.start[1], t.start[2], t.yaw
            ));
        } else if r == SETTLE - 1 {
            // Opens the screen and takes over the next tick's input.
            out.screen = Some(t.screen);
            out.yaw = Some(t.yaw);
            self.drive = t.drive;
        }
        if r >= SETTLE - 1 && r < SETTLE + DRIVE - 1 {
            match t.sprint {
                SprintMode::Never => {}
                SprintMode::Once if r == SETTLE - 1 => out.set_sprinting = Some(true),
                SprintMode::Once => {}
                SprintMode::Every => out.set_sprinting = Some(true),
            }
            // A turn made by the game's own yaw would show here: the yaw is held for the whole drive.
            out.yaw = Some(t.yaw);
        }
        if r == SETTLE + DRIVE - 1 {
            self.drive = None;
            out.console.push("data get entity @p Pos".into());
        }
        self.r += 1;
        if self.r >= SETTLE + DRIVE + TAIL {
            self.r = 0;
            self.trial += 1;
            self.prev = None;
            if self.trial == TRIALS.len() {
                self.done = true;
                out.screen = Some(ScreenKind::Playing);
                out.finished = Some(true);
            }
        }
        out
    }
}
