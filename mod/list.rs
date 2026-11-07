// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// The mod's row in Options > Mods.
fn baritone_rs_listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    use crate::game::baritone_rs as m;
    mods.push(crate::game::native_hooks::NativeMod { id: m::ID.into(), title: format!("{} {}", m::NAME, m::VERSION), description: m::DESCRIPTION.into(), keys: Vec::new() });
}
