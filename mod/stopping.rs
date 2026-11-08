// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// The server begins an orderly stop (the single player's world is closing, the game is quitting): the chunk cache is saved now, before the process ends.
fn baritone_rs_server_stopping(dedicated: bool) {
    let _ = dedicated;
    crate::game::baritone_rs::on_server_stopping();
}
