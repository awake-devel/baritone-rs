// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// The connection is about to be discarded.
fn baritone_rs_disconnecting(game: &crate::game::Game<'_>) {
    crate::game::baritone_rs::on_level_gone(game.native_window(), "disconnecting");
}
