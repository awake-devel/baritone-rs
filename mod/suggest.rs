// baritone-rs, the mod. LGPL-3.0-or-later, see CREDITS.md.
// The chat box asks the command tree for completions; the words after `/b` are the mod's to offer.
fn baritone_rs_suggest(value: (Range, Vec<Suggestion>, bool), parse: &ParseResults, cursor: usize, reg: &Registries) -> (Range, Vec<Suggestion>, bool) {
    crate::baritone_rs_suggest::apply(value, &parse.input, cursor, reg.target_block)
}
