// native_movement_input: the keys the game read, then what the probe wants instead. Records what the game read, to show
// that the hook runs with a screen open and that the game itself gave no movement then.
fn baritone_probe_movement(forward: &mut f32, strafe: &mut f32, jump: &mut bool, sneak: &mut bool) {
    use crate::game::baritone_probe as m;
    m::apply_hook(forward, strafe, jump, sneak);
}
