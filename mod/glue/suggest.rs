// baritone-rs glue: the seam through which the mod completes the words after its command. Written for this project. LGPL-3.0-or-later, see CREDITS.md.
//
// This module is added to `horizon-command`, the crate that owns the command tree, because the chat box (`horizon-client-render`) asks the tree
// for completions and the mod (in `horizon-client`) is above both: the mod hands in a function, and a hook on the tree's completion asks it.
// There is nothing about baritone in here but the name; the function decides whether the text is its own.
pub type Provider = fn(&str, Option<[i32; 3]>) -> Option<(usize, Vec<(String, Option<String>)>)>;

static PROVIDER: std::sync::Mutex<Option<Provider>> = std::sync::Mutex::new(None);

/// The function that completes: it gets the command text up to the cursor (with its slash) and the block the crosshair is on, and answers
/// `None` for text that is not its own, or the byte where the suggestions start and their text and tooltip.
pub fn register(provider: Provider) {
    *PROVIDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(provider);
}

/// `Dispatcher::client_suggestions_asking` after it ran: the suggestions of the mod's command replace the tree's, which has none past the name.
pub fn apply(value: (crate::Range, Vec<crate::Suggestion>, bool), input: &str, cursor: usize, target: Option<[i32; 3]>) -> (crate::Range, Vec<crate::Suggestion>, bool) {
    let provider = *PROVIDER.lock().unwrap_or_else(|e| e.into_inner());
    let Some(provider) = provider else {
        return value;
    };
    let mut end = cursor.min(input.len());
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    let text = &input[..end];
    let Some((start, list)) = provider(text, target) else {
        return value;
    };
    let start = start.min(end);
    let list = list.into_iter().map(|(text, tooltip)| crate::Suggestion { range: (start, end), text, tooltip, int_value: None }).collect();
    let (range, list) = crate::merge_suggestions(input, list);
    (range, list, false)
}
