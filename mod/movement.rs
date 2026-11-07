// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// The keys the game read, then what a running path wants instead. Does nothing while no path runs.
fn baritone_rs_movement(forward: &mut f32, strafe: &mut f32, jump: &mut bool, sneak: &mut bool) {
    crate::game::baritone_rs::on_movement(forward, strafe, jump, sneak);
}
