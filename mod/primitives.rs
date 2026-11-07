// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// The path, drawn once a frame.
fn baritone_rs_primitives(primitives: &mut horizon_client::world_primitives::WorldPrimitives) {
    crate::game::baritone_rs::on_primitives(primitives);
}
