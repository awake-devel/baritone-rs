// baritone-rs engine: the parts of Minecraft's `Mth` that the rules read. Written for this project from the game's documented behaviour
// (a quantized sine table, indexed through a `long`). LGPL-3.0-or-later, see lib.rs.
use std::sync::OnceLock;

/// The table's scale: 65536 steps over a full turn.
const SCALE: f64 = 10430.378350470453;

/// Upstream's `Mth.floor(double)`: `(int) d`, one less when that rounded up. A Java `int` cast saturates, and so does `as`. The subtraction wraps.
pub fn floor(d: f64) -> i32 {
    let i = d as i32;
    if d < f64::from(i) {
        i.wrapping_sub(1)
    } else {
        i
    }
}

fn table() -> &'static [f32] {
    static SINE: OnceLock<Box<[f32]>> = OnceLock::new();
    SINE.get_or_init(|| (0..65536).map(|i| (f64::from(i) / SCALE).sin() as f32).collect::<Vec<_>>().into_boxed_slice())
}

/// Upstream's `Mth.sin(double)`: the table entry at `(long) (angle * 10430.378350470453) & 65535`.
pub fn sin(angle: f64) -> f32 {
    table()[((angle * SCALE) as i64 & 65535) as usize]
}

/// Upstream's `Mth.cos(double)`: the same table a quarter turn on.
pub fn cos(angle: f64) -> f32 {
    table()[((angle * SCALE + 16384.0) as i64 & 65535) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_rounds_down_and_saturates() {
        assert_eq!(floor(1.9), 1);
        assert_eq!(floor(-0.1), -1);
        assert_eq!(floor(-3.0), -3);
        assert_eq!(floor(1e30), i32::MAX);
    }

    #[test]
    fn the_table_is_close_to_sine_and_cosine() {
        for deg in (0..360).step_by(15) {
            let a = f64::from(deg).to_radians();
            assert!((f64::from(sin(a)) - a.sin()).abs() < 1e-3, "sin {deg}");
            assert!((f64::from(cos(a)) - a.cos()).abs() < 1e-3, "cos {deg}");
        }
        assert_eq!(sin(0.0), 0.0);
        assert_eq!(cos(0.0), 1.0);
    }
}
