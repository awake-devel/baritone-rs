// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// Start of the game's own input step: the mouse buttons the executor holds, and the chat screen swapped for Playing for this one call.
fn baritone_rs_input_head(game: &mut crate::game::Game<'_>) {
    crate::game::baritone_rs::on_input_head(game);
}
