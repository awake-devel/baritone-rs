// Start of the game's input step. With the intent active: the attack and use buttons are set the way a held mouse button
// would be, and when the chat is open (and the intent says swap) the screen is Playing for this one call.
fn baritone_bp_input_head(game: &mut crate::game::Game<'_>) {
    use crate::game::Screen;
    use crate::game::baritone_bp as m;
    let i = m::intent();
    if !i.active {
        return;
    }
    game.left = i.left;
    game.right = i.right;
    if i.swap && game.screen == Screen::Chat {
        game.screen = Screen::Playing;
        m::set_swapped(true);
    }
}
