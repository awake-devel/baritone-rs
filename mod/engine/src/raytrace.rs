// Ported from cabaletta/baritone at 1b8e87c: src/api/java/baritone/api/utils/RayTraceUtils.java and, from RotationUtils.java,
// calcLookDirectionFromRotation. LGPL-3.0-or-later, see lib.rs.
//
// A ray through the blocks of a `WorldView`, to tell whether an aim reaches a block. Upstream asks the game (`Level.clip` with the
// block's outline shape). The engine has no shapes, only per-state flags, so here a block is hit when it is not air, not a fluid block
// and either a full cube or not walkable (docs/deviations.md D25): the ray meets the whole cube of such a block. The game decides the
// real hit; the glue reports it in `PlayerView::looking_at`, and the movements click on that when they have it.
use crate::blocks::Direction;
use crate::look::{Rotation, Vec3};
use crate::movement::helper::Ctx;
use crate::pos::Pos;
use crate::world::WorldView;

/// A block the crosshair is on: upstream's `BlockHitResult`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockHit {
    pub pos: Pos,
    /// The face that was hit (`BlockHitResult.getDirection`).
    pub face: Direction,
    /// Where the ray meets the block.
    pub point: Vec3,
}

const DEG_TO_RAD_F: f32 = 0.017_453_292;

/// `RotationUtils.calcLookDirectionFromRotation`: the look vector of a rotation. Upstream uses `Mth.sin` and `Mth.cos`, which read a table
/// of 65536 entries; this is `f32::sin` and `f32::cos`, which agree to about a ten-thousandth (docs/deviations.md D17).
pub fn calc_look_direction_from_rotation(r: Rotation) -> Vec3 {
    let flat_z = ((-r.yaw * DEG_TO_RAD_F) - std::f32::consts::PI).cos();
    let flat_x = ((-r.yaw * DEG_TO_RAD_F) - std::f32::consts::PI).sin();
    let pitch_base = -((-r.pitch * DEG_TO_RAD_F).cos());
    let pitch_height = (-r.pitch * DEG_TO_RAD_F).sin();
    Vec3::new(f64::from(flat_x * pitch_base), f64::from(pitch_height), f64::from(flat_z * pitch_base))
}

/// Whether a ray stops at a block of this state (the engine's stand-in for a non-empty outline shape, with fluids ignored).
pub fn blocks_ray(s: &crate::blocks::State) -> bool {
    !s.air && !s.liquid_block && (s.shape_full || !s.pathfindable)
}

/// Upstream's `RayTraceUtils.rayTraceTowards`: the first block the ray from `start` along `rotation` meets within `reach`. The caller gives the
/// start: the eye, or `inferSneakingEyePosition` for a player who would sneak.
pub fn ray_trace_towards<W: WorldView>(ctx: &Ctx<'_, W>, start: Vec3, rotation: Rotation, reach: f64) -> Option<BlockHit> {
    let d = calc_look_direction_from_rotation(rotation);
    let end = Vec3::new(start.x + d.x * reach, start.y + d.y * reach, start.z + d.z * reach);
    clip(ctx, start, end)
}

/// Upstream's `RayTraceUtils.inferSneakingEyePosition`: the eye of a player who is sneaking, at the player's position.
pub fn infer_sneaking_eye_position(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y + 1.27, z)
}

/// Upstream's `Level.clip` with `ClipContext.Block.OUTLINE` and `ClipContext.Fluid.NONE`, over whole cubes.
fn clip<W: WorldView>(ctx: &Ctx<'_, W>, start: Vec3, end: Vec3) -> Option<BlockHit> {
    let dir = [end.x - start.x, end.y - start.y, end.z - start.z];
    let s = [start.x, start.y, start.z];
    let mut cell = [s[0].floor() as i32, s[1].floor() as i32, s[2].floor() as i32];
    if dir.iter().any(|c| !c.is_finite()) || s.iter().any(|c| !c.is_finite()) {
        return None;
    }
    let mut step = [0i32; 3];
    let mut t_max = [f64::INFINITY; 3];
    let mut t_delta = [f64::INFINITY; 3];
    for a in 0..3 {
        if dir[a] > 0.0 {
            step[a] = 1;
            t_delta[a] = 1.0 / dir[a];
            t_max[a] = (f64::from(cell[a]) + 1.0 - s[a]) / dir[a];
        } else if dir[a] < 0.0 {
            step[a] = -1;
            t_delta[a] = -1.0 / dir[a];
            t_max[a] = (f64::from(cell[a]) - s[a]) / dir[a];
        }
    }
    let state_at = |c: [i32; 3]| ctx.get(c[0], c[1], c[2]);
    let hit_at = |c: [i32; 3], t: f64, face: Direction| BlockHit { pos: Pos::new(c[0], c[1], c[2]), face, point: Vec3::new(s[0] + dir[0] * t, s[1] + dir[1] * t, s[2] + dir[2] * t) };
    if blocks_ray(state_at(cell).s) {
        // the start is inside a block: the hit is at the start, on the face the ray leaves by turned round
        let a = (0..3).max_by(|&i, &j| dir[i].abs().total_cmp(&dir[j].abs())).unwrap_or(1);
        let face = match (a, dir[a] > 0.0) {
            (0, true) => Direction::West,
            (0, false) => Direction::East,
            (1, true) => Direction::Down,
            (1, false) => Direction::Up,
            (_, true) => Direction::North,
            (_, false) => Direction::South,
        };
        return Some(hit_at(cell, 0.0, face));
    }
    // at most as many cells as the segment can cross
    for _ in 0..4096 {
        let axis = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
            0
        } else if t_max[1] <= t_max[2] {
            1
        } else {
            2
        };
        let t = t_max[axis];
        if t > 1.0 || !t.is_finite() {
            return None;
        }
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        if blocks_ray(state_at(cell).s) {
            // stepping towards +x enters the block through its west face
            let face = match (axis, step[axis] > 0) {
                (0, true) => Direction::West,
                (0, false) => Direction::East,
                (1, true) => Direction::Down,
                (1, false) => Direction::Up,
                (_, true) => Direction::North,
                (_, false) => Direction::South,
            };
            return Some(hit_at(cell, t, face));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Fixture, TestWorld, STONE};

    fn world() -> TestWorld {
        let mut w = TestWorld::flat(10, 10);
        w.set(5, 1, 5, STONE);
        w
    }

    #[test]
    fn look_directions_of_the_cardinal_rotations() {
        let near = |a: Vec3, x: f64, y: f64, z: f64| (a.x - x).abs() < 1e-5 && (a.y - y).abs() < 1e-5 && (a.z - z).abs() < 1e-5;
        // yaw 0 looks south (+z), 90 west (-x), 180 north (-z), -90 east (+x); pitch positive looks down
        assert!(near(calc_look_direction_from_rotation(Rotation::new(0.0, 0.0)), 0.0, 0.0, 1.0));
        assert!(near(calc_look_direction_from_rotation(Rotation::new(90.0, 0.0)), -1.0, 0.0, 0.0));
        assert!(near(calc_look_direction_from_rotation(Rotation::new(180.0, 0.0)), 0.0, 0.0, -1.0));
        assert!(near(calc_look_direction_from_rotation(Rotation::new(-90.0, 0.0)), 1.0, 0.0, 0.0));
        assert!(near(calc_look_direction_from_rotation(Rotation::new(0.0, 90.0)), 0.0, -1.0, 0.0));
        assert!(near(calc_look_direction_from_rotation(Rotation::new(0.0, -90.0)), 0.0, 1.0, 0.0));
    }

    #[test]
    fn a_ray_finds_the_first_block_and_the_face_it_enters_by() {
        let w = world();
        let fx = Fixture::new();
        let ctx = fx.ctx(&w);
        // from the west of the block at (5,1,5), looking east (yaw -90), eye at its height
        let hit = ray_trace_towards(&ctx, Vec3::new(2.5, 1.5, 5.5), Rotation::new(-90.0, 0.0), 4.5).expect("hit");
        assert_eq!(hit.pos, Pos::new(5, 1, 5));
        assert_eq!(hit.face, Direction::West);
        assert!((hit.point.x - 5.0).abs() < 1e-9);
        // out of reach
        assert!(ray_trace_towards(&ctx, Vec3::new(0.2, 1.5, 5.5), Rotation::new(-90.0, 0.0), 4.5).is_none());
        // from above, looking down onto the floor
        let hit = ray_trace_towards(&ctx, Vec3::new(2.5, 2.62, 2.5), Rotation::new(0.0, 90.0), 4.5).expect("hit");
        assert_eq!((hit.pos, hit.face), (Pos::new(2, 0, 2), Direction::Up));
        // a ray looking up at the open sky hits nothing
        assert!(ray_trace_towards(&ctx, Vec3::new(2.5, 2.62, 2.5), Rotation::new(0.0, -90.0), 4.5).is_none());
    }

    #[test]
    fn a_ray_that_starts_inside_a_block_hits_at_once() {
        let w = world();
        let fx = Fixture::new();
        let ctx = fx.ctx(&w);
        let hit = ray_trace_towards(&ctx, Vec3::new(5.5, 1.5, 5.5), Rotation::new(0.0, 0.0), 4.5).expect("hit");
        assert_eq!(hit.pos, Pos::new(5, 1, 5));
        assert_eq!(hit.point, Vec3::new(5.5, 1.5, 5.5));
    }

    #[test]
    fn a_slanted_ray_agrees_with_stepping_along_it() {
        // an independent check: walk the same ray in tiny steps and take the first cube it enters
        let mut w = TestWorld::flat(12, 12);
        for (x, y, z) in [(6, 1, 3), (7, 2, 7), (3, 1, 8), (9, 1, 9)] {
            w.set(x, y, z, STONE);
        }
        let fx = Fixture::new();
        let ctx = fx.ctx(&w);
        let mut checked = 0;
        for yaw in (-180..180).step_by(7) {
            for pitch in (-60..80).step_by(11) {
                let start = Vec3::new(5.3, 2.62, 5.6);
                let rot = Rotation::new(yaw as f32, pitch as f32);
                let got = ray_trace_towards(&ctx, start, rot, 4.5);
                let d = calc_look_direction_from_rotation(rot);
                let mut want = None;
                let n = 45_000;
                for i in 0..=n {
                    let t = 4.5 * f64::from(i) / f64::from(n);
                    let p = (start.x + d.x * t, start.y + d.y * t, start.z + d.z * t);
                    let c = (p.0.floor() as i32, p.1.floor() as i32, p.2.floor() as i32);
                    if blocks_ray(ctx.get(c.0, c.1, c.2).s) {
                        want = Some(Pos::new(c.0, c.1, c.2));
                        break;
                    }
                }
                assert_eq!(got.map(|h| h.pos), want, "yaw {yaw} pitch {pitch}");
                checked += 1;
            }
        }
        assert!(checked > 300);
    }
}
