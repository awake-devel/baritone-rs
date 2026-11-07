// End of the game's input step: the chat's screen goes back.
fn baritone_bp_input_return(value: (), game: &mut crate::game::Game<'_>) {
    use crate::game::Screen;
    use crate::game::baritone_bp as m;
    if m::take_swapped() {
        game.screen = Screen::Chat;
    }
    value
}
