// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/utils/Rotation.java, RotationUtils.java (calcRotationFromVec3d,
// wrapAnglesToRelative), VecUtils.java (getBlockPosCenter, distanceToCenter) and the aim processor in
// src/main/java/baritone/behavior/LookBehavior.java. LGPL-3.0-or-later, see lib.rs.
//
// What is here: the aim a walking movement needs. What is not (on purpose, for this version): free look, smoothing
// (`smoothLook`), the random offsets (`randomLooking`, `randomLooking113`), ray traces (`reachable`), and swim and
// elytra aiming. See docs/deviations.md.
use crate::pos::Pos;

/// `Rotation`: yaw and pitch in degrees, as floats. Upstream's constructor throws on a NaN or an infinity; here such a
/// value becomes 0, because the executor must never panic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation {
    pub yaw: f32,
    pub pitch: f32,
}

/// A position in the world, in blocks (`Vec3`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }
}

/// `VecUtils.getBlockPosCenter`: the assumed center of a block, 0.5 on each axis.
pub fn block_pos_center(p: Pos) -> Vec3 {
    Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5)
}

/// `VecUtils.distanceToCenter`: the distance from a point to the assumed center of a block.
pub fn distance_to_center(p: Pos, x: f64, y: f64, z: f64) -> f64 {
    let xdiff = p.x as f64 + 0.5 - x;
    let ydiff = p.y as f64 + 0.5 - y;
    let zdiff = p.z as f64 + 0.5 - z;
    (xdiff * xdiff + ydiff * ydiff + zdiff * zdiff).sqrt()
}

const RAD_TO_DEG: f64 = 180.0 / std::f64::consts::PI;

/// `Rotation.add`.
impl std::ops::Add for Rotation {
    type Output = Rotation;
    /// `Rotation.add`:
    fn add(self, o: Rotation) -> Rotation {
        Rotation::new(self.yaw + o.yaw, self.pitch + o.pitch)
    }
}

/// `Rotation.subtract`.
impl std::ops::Sub for Rotation {
    type Output = Rotation;
    /// `Rotation.subtract`:
    fn sub(self, o: Rotation) -> Rotation {
        Rotation::new(self.yaw - o.yaw, self.pitch - o.pitch)
    }
}

impl Rotation {
    pub fn new(yaw: f32, pitch: f32) -> Rotation {
        if !yaw.is_finite() || !pitch.is_finite() {
            return Rotation { yaw: 0.0, pitch: 0.0 };
        }
        Rotation { yaw, pitch }
    }
    /// `Rotation.clamp`: the pitch to -90 to 90.
    pub fn clamp(self) -> Rotation {
        Rotation::new(self.yaw, clamp_pitch(self.pitch))
    }
    /// `Rotation.normalize`: the yaw to -180 to 180.
    pub fn normalize(self) -> Rotation {
        Rotation::new(normalize_yaw(self.yaw), self.pitch)
    }
    /// `Rotation.withPitch`:
    /// `Rotation.withPitch`.
    pub fn with_pitch(self, pitch: f32) -> Rotation {
        Rotation::new(self.yaw, pitch)
    }
    /// `Rotation.isReallyCloseTo`:
    /// `Rotation.isReallyCloseTo`.
    pub fn is_really_close_to(self, o: Rotation) -> bool {
        self.yaw_is_really_close(o) && (self.pitch - o.pitch).abs() < 0.01
    }
    /// `Rotation.yawIsReallyClose`:
    /// `Rotation.yawIsReallyClose`.
    pub fn yaw_is_really_close(self, o: Rotation) -> bool {
        let yaw_diff = (normalize_yaw(self.yaw) - normalize_yaw(o.yaw)).abs();
        yaw_diff < 0.01 || yaw_diff > 359.99
    }
}

/// `Rotation.clampPitch`: Java's `Math.max(-90, Math.min(90, pitch))`.
pub fn clamp_pitch(pitch: f32) -> f32 {
    // NaN cannot reach here: `Rotation::new` removes it.
    (-90.0f32).max(90.0f32.min(pitch))
}

/// `Rotation.yawDistanceFromOffset`: the difference of two yaws, the short way round when they are on opposite sides of the back of the view.
pub fn yaw_distance_from_offset(yaw: f32, offset_yaw: f32) -> f32 {
    if ((yaw > 0.0) ^ (offset_yaw > 0.0)) && ((yaw > 90.0 || yaw < -90.0) ^ (offset_yaw > 90.0 || offset_yaw < -90.0)) {
        if yaw < 0.0 {
            360.0 + (yaw - offset_yaw)
        } else {
            360.0 - (yaw - offset_yaw)
        }
    } else {
        yaw - offset_yaw
    }
}

/// `Rotation.normalizeYaw`: Java's `%` on floats is `fmod`, as Rust's.
pub fn normalize_yaw(yaw: f32) -> f32 {
    let mut new_yaw = yaw % 360.0;
    if new_yaw < -180.0 {
        new_yaw += 360.0;
    }
    if new_yaw > 180.0 {
        new_yaw -= 360.0;
    }
    new_yaw
}

/// `RotationUtils.calcRotationFromVec3d(Vec3,Vec3)`:
/// `RotationUtils.calcRotationFromVec3d(Vec3,Vec3)` (the private one): the rotation that looks from `orig` to `dest`.
/// Upstream uses `Mth.atan2`, which Minecraft implements with its own approximation. This is `f64::atan2`: see docs/deviations.md.
fn calc_rotation_from_vec3d_abs(orig: Vec3, dest: Vec3) -> Rotation {
    let delta = [orig.x - dest.x, orig.y - dest.y, orig.z - dest.z];
    let yaw = delta[0].atan2(-delta[2]);
    let dist = (delta[0] * delta[0] + delta[2] * delta[2]).sqrt();
    let pitch = delta[1].atan2(dist);
    Rotation::new((yaw * RAD_TO_DEG) as f32, (pitch * RAD_TO_DEG) as f32)
}

/// `RotationUtils.wrapAnglesToRelative`: the target as an angle near the current one, so the turn takes the short way.
pub fn wrap_angles_to_relative(current: Rotation, target: Rotation) -> Rotation {
    if current.yaw_is_really_close(target) {
        return Rotation::new(current.yaw, target.pitch);
    }
    (target - current).normalize() + current
}

/// `RotationUtils.calcRotationFromVec3d(Vec3,Vec3,Rotation)`: the rotation from `orig` to `dest`, relative to the current one.
pub fn calc_rotation_from_vec3d(orig: Vec3, dest: Vec3, current: Rotation) -> Rotation {
    wrap_angles_to_relative(current, calc_rotation_from_vec3d_abs(orig, dest))
}

/// Java's `Math.round(float)`: `floor(x + 0.5)`.
fn java_round(x: f32) -> f64 {
    (x as f64 + 0.5).floor()
}

/// `LookBehavior.AbstractAimProcessor`, without the random offsets: what the game is asked to turn to, given where the player
/// looks now and what the movement wants. The sensitivity is the game's mouse sensitivity setting (0 to 1, default 0.5).
#[derive(Clone, Copy, Debug)]
pub struct AimProcessor {
    pub sensitivity: f64,
}

impl AimProcessor {
    pub fn new(sensitivity: f64) -> AimProcessor {
        AimProcessor { sensitivity }
    }

    /// `LookBehavior.AbstractAimProcessor.nudgeToLevel`: a pitch outside -20 to 10 moves one degree towards it.
    /// A function and not a method, because upstream's is private and has no state.
    fn nudge_to_level(pitch: f32) -> f32 {
        if pitch < -20.0 {
            pitch + 1.0
        } else if pitch > 10.0 {
            pitch - 1.0
        } else {
            pitch
        }
    }

    /// `LookBehavior.AbstractAimProcessor.mouseToAngle`: a mouse movement in pixels as degrees. One double and one float factor, as upstream.
    fn mouse_to_angle(&self, mouse_delta: f64) -> f32 {
        let f = self.sensitivity * (0.6f32 as f64) + (0.2f32 as f64);
        (mouse_delta * f * f * f * 8.0f64) as f32 * 0.15f32
    }

    /// `LookBehavior.AbstractAimProcessor.angleToMouse`: the mouse movement that is closest to an angle.
    fn angle_to_mouse(&self, angle_delta: f32) -> f64 {
        let min_angle_change = self.mouse_to_angle(1.0);
        java_round(angle_delta / min_angle_change)
    }

    /// `LookBehavior.AbstractAimProcessor.calculateMouseMove`: the angle after the mouse movement closest to the target.
    fn calculate_mouse_move(&self, current: f32, target: f32) -> f32 {
        let delta = target - current;
        let delta_px = self.angle_to_mouse(delta);
        current + self.mouse_to_angle(delta_px)
    }

    /// `LookBehavior.AbstractAimProcessor.peekRotation`: `prev` is the rotation the player has now.
    pub fn peek_rotation(&self, prev: Rotation, rotation: Rotation) -> Rotation {
        let desired_yaw = rotation.yaw;
        let mut desired_pitch = rotation.pitch;
        // the target does not care about the pitch (it is the current one), so it is safe to bring it to a normal level
        if desired_pitch == prev.pitch {
            desired_pitch = Self::nudge_to_level(desired_pitch);
        }
        Rotation::new(self.calculate_mouse_move(prev.yaw, desired_yaw), self.calculate_mouse_move(prev.pitch, desired_pitch)).clamp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn looking_at_the_cardinal_directions() {
        let eye = Vec3::new(0.5, 1.62, 0.5);
        let cur = Rotation::new(0.0, 0.0);
        // Minecraft yaw: 0 is south (+z), 90 west (-x), 180 north (-z), -90 east (+x)
        assert!(close(calc_rotation_from_vec3d(eye, Vec3::new(0.5, 1.62, 5.5), cur).yaw, 0.0, 1e-4));
        assert!(close(calc_rotation_from_vec3d(eye, Vec3::new(-4.5, 1.62, 0.5), cur).yaw, 90.0, 1e-3));
        assert!(close(calc_rotation_from_vec3d(eye, Vec3::new(5.5, 1.62, 0.5), cur).yaw, -90.0, 1e-3));
        // pitch: negative looks up
        assert!(calc_rotation_from_vec3d(eye, Vec3::new(0.5, 6.62, 5.5), cur).pitch < 0.0);
        assert!(calc_rotation_from_vec3d(eye, Vec3::new(0.5, -3.0, 5.5), cur).pitch > 0.0);
    }

    #[test]
    fn the_turn_takes_the_short_way() {
        let eye = Vec3::new(0.0, 0.0, 0.0);
        // looking at 170 degrees, the target is at -170 (a point just east of north): 20 degrees away, so the result is near 190, not -170
        let cur = Rotation::new(170.0, 0.0);
        let r = calc_rotation_from_vec3d(eye, Vec3::new(0.173648, 0.0, -0.984808), cur);
        assert!(close(r.yaw, 190.0, 0.05), "{r:?}");
    }

    #[test]
    fn a_target_that_is_really_close_keeps_the_yaw() {
        let cur = Rotation::new(33.333, 5.0);
        let t = Rotation::new(33.333 + 360.0, -7.0);
        let w = wrap_angles_to_relative(cur, t);
        assert_eq!(w.yaw, 33.333);
        assert_eq!(w.pitch, -7.0);
    }

    #[test]
    fn normalize_and_clamp() {
        assert_eq!(normalize_yaw(190.0), -170.0);
        assert_eq!(normalize_yaw(-190.0), 170.0);
        assert_eq!(normalize_yaw(720.0), 0.0);
        assert_eq!(clamp_pitch(120.0), 90.0);
        assert_eq!(clamp_pitch(-120.0), -90.0);
        // a NaN becomes 0 and does not panic
        assert_eq!(Rotation::new(f32::NAN, 1.0), Rotation { yaw: 0.0, pitch: 0.0 });
    }

    #[test]
    fn the_aim_lands_on_the_mouse_grid_and_nudges_a_dull_pitch() {
        let aim = AimProcessor::new(0.5);
        let prev = Rotation::new(0.0, 30.0);
        // a pitch equal to the current one is nudged one degree towards level
        let r = aim.peek_rotation(prev, Rotation::new(0.0, 30.0));
        assert!(r.pitch < 30.0 && r.pitch > 28.0, "{r:?}");
        // a yaw goal is reached to within one mouse step (about 0.15 degrees at the default sensitivity)
        let r = aim.peek_rotation(Rotation::new(0.0, 0.0), Rotation::new(37.3, 1.0));
        assert!(close(r.yaw, 37.3, 0.2), "{r:?}");
        assert!(r.pitch.abs() <= 90.0);
    }

    #[test]
    fn centers_and_distances() {
        let c = block_pos_center(Pos::new(1, 2, 3));
        assert_eq!((c.x, c.y, c.z), (1.5, 2.5, 3.5));
        assert!(close(distance_to_center(Pos::new(0, 0, 0), 0.5, 0.5, 3.5) as f32, 3.0, 1e-6));
    }
}
