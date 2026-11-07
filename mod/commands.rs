// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// The command names the chat box colours and completes.
fn baritone_rs_commands(commands: &mut Vec<&'static str>) {
    crate::game::baritone_rs::on_commands(commands);
}
