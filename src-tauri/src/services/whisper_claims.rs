//! Which whisper conversation each window shows, and whether it is being read.
//! Card 7.5 writes it; until then a claim from the main window (or a clear
//! from any window that names a conversation) keeps today's single slot.

pub fn set_claim(label: &str, _owner: &str, key: Option<String>, _page_load: &str, _seq: u64) {
    if key.is_some() || label == "main" {
        crate::services::whisper_inbox::set_active(key);
    }
}

pub fn clear_label(_label: &str) {}

pub fn forget_window(_label: &str) {}

pub fn note_focus(_label: &str, _focused: bool) {}

pub fn note_minimized(_label: &str, _minimized: bool) {}

pub fn set_app_interactive(_interactive: bool) {}

pub fn reading(_owner: &str, _key: &str) -> bool {
    false
}

pub fn clear_scope(_generation: u64) {}
