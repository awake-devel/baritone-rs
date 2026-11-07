// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// Once a tick, on the window's game: the state machine, and the scenario trigger when it is switched on.
fn baritone_rs_tick(game: &mut crate::game::Game<'_>) {
    crate::game::baritone_rs::on_tick(game);
}
