// Fixed-course walker for baritone-rs. Pure logic: the helpers read the game and apply what this returns.
// License: LGPL-3.0-or-later, like the rest of baritone-rs. Not ported from Baritone.

pub const ID: &str = "baritone_walk";
pub const NAME: &str = "baritone-rs walk test";
pub const VERSION: &str = "0.0.0";
pub const DESCRIPTION: &str = "Test only: /baritonewalk walks a fixed course and writes every tick to a CSV file.";

#[derive(Clone, Copy)]
pub struct Waypoint {
    pub at: [f64; 3],
    pub sprint: bool,
}

/// Walk, then sprint, then a step up, a step down and a turn, all on the platform the test script builds.
pub const COURSE: [Waypoint; 5] = [
    Waypoint { at: [1024.5, 100.0, 1000.5], sprint: false },
    Waypoint { at: [1024.5, 100.0, 1024.5], sprint: true },
    Waypoint { at: [1018.5, 101.0, 1024.5], sprint: false },
    Waypoint { at: [1012.5, 100.0, 1024.5], sprint: false },
    Waypoint { at: [1012.5, 100.0, 1012.5], sprint: false },
];

const ARRIVED: f64 = 0.3;
const TIMEOUT_TICKS: u64 = 1200;
const STEP_MARGIN: f64 = 0.25;
const JUMP_RANGE: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub yaw: Option<f32>,
    pub forward: bool,
    pub sprint: bool,
    pub jump: bool,
    pub finished: Option<bool>,
}

pub const RELEASE: Output = Output { yaw: None, forward: false, sprint: false, jump: false, finished: None };

#[derive(Default)]
pub struct Walker {
    pub running: bool,
    next: usize,
    tick: u64,
    rows: Vec<String>,
}

impl Walker {
    pub fn start(&mut self) {
        *self = Walker { running: true, rows: vec!["tick,x,y,z,yaw,on_ground,waypoint,forward,sprint,jump".to_string()], ..Walker::default() };
    }

    pub fn step(&mut self, p: [f64; 3], yaw: f32, on_ground: bool) -> Output {
        if !self.running {
            return RELEASE;
        }
        self.tick += 1;
        while self.next < COURSE.len() && horizontal(p, COURSE[self.next].at) < ARRIVED {
            self.next += 1;
        }
        let mut out = RELEASE;
        if self.next == COURSE.len() {
            out.finished = Some(true);
        } else if self.tick > TIMEOUT_TICKS {
            out.finished = Some(false);
        } else {
            let w = COURSE[self.next];
            let (dx, dz) = (w.at[0] - p[0], w.at[2] - p[2]);
            // yaw 0 faces south (+z), 90 west (-x)
            out.yaw = Some((-dx).atan2(dz).to_degrees() as f32);
            out.forward = true;
            out.sprint = w.sprint;
            // Hold jump near a step, until the feet are level with the waypoint.
            out.jump = w.at[1] - p[1] > STEP_MARGIN && horizontal(p, w.at) < JUMP_RANGE;
        }
        self.rows.push(format!(
            "{},{:.6},{:.6},{:.6},{:.3},{},{},{},{},{}",
            self.tick, p[0], p[1], p[2], yaw, on_ground as u8, self.next, out.forward as u8, out.sprint as u8, out.jump as u8
        ));
        if out.finished.is_some() {
            self.running = false;
        }
        out
    }

    pub fn csv(&self) -> String {
        self.rows.join("\n") + "\n"
    }
}

fn horizontal(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub fn log_path() -> std::path::PathBuf {
    std::env::var_os("BARITONE_WALK_LOG").map_or_else(|| "baritone-walk.csv".into(), Into::into)
}

thread_local! {
    pub static WALKER: std::cell::RefCell<Walker> = std::cell::RefCell::new(Walker::default());
}
