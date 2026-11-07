// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// /b and /baritone: goto, stop, help.
fn baritone_rs_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    crate::game::baritone_rs::on_command(game, command, ran);
}
