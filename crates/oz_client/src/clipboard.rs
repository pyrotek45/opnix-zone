//! The system clipboard (copy the lobby code, paste it into JOIN). Never fatal: with no
//! clipboard to talk to (a headless box, a sandbox) copying reports failure and pasting
//! finds nothing.

use std::sync::Mutex;

/// One clipboard for the whole run: on X11 the copied text is served from it, so it has to
/// outlive the copy.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    let mut slot = CLIPBOARD.lock().ok()?;
    if slot.is_none() {
        *slot = arboard::Clipboard::new().ok();
    }
    f(slot.as_mut()?)
}

/// Puts `text` on the clipboard; false if there is none.
pub fn copy(text: &str) -> bool {
    with(|clipboard| clipboard.set_text(text.to_string()).ok()).is_some()
}

/// The clipboard's text, trimmed of whitespace and line breaks (`None` if empty or missing).
pub fn paste() -> Option<String> {
    with(|clipboard| clipboard.get_text().ok()).map(|text| clean(&text)).filter(|text| !text.is_empty())
}

/// Pasted text on one line: no surrounding whitespace, inner line breaks dropped.
pub fn clean(text: &str) -> String {
    text.trim().chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn pasted_codes_are_cleaned_up() {
        assert_eq!(super::clean("  K7Q-4MZ\r\n"), "K7Q-4MZ");
        assert_eq!(super::clean("\tK7Q\n4MZ "), "K7Q4MZ");
    }
}
