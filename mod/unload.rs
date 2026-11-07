// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// The client dropped its level: a path to a place in it means nothing now.
fn baritone_rs_level_unload(game: &mut crate::game::Game<'_>) {
    crate::game::baritone_rs::on_level_gone(game.native_window(), "the level was dropped");
}
