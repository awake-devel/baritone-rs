// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// End of the game's own input step: the chat screen comes back.
fn baritone_rs_input_return(value: (), game: &mut crate::game::Game<'_>) {
    crate::game::baritone_rs::on_input_return(game);
    value
}
