// /baritonewalk starts the course.
fn baritone_walk_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    use crate::game::baritone_walk as m;
    if command.trim() != "baritonewalk" {
        return;
    }
    *ran = true;
    m::WALKER.with(|w| w.borrow_mut().start());
    game.native_chat("baritonewalk started");
}
