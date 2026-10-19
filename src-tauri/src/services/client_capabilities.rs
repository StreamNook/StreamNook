//! What this build can draw, for the surfaces only the desktop has.
//!
//! The shells never decide this for themselves: settings pages, search and the
//! changed-settings review all read the same answer, so a row for a feature
//! this build lacks is simply not there anywhere. Platform reach (which
//! platforms this build can watch) lives on the account roster
//! (`account_roster::can_watch`) instead, beside sign-in.

use serde::Serialize;

/// The desktop-only surfaces this build has. Every field is true on desktop and
/// false on the phones.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClientCapabilities {
    /// Combined chat: another platform's chat for the same streamer merged into
    /// the one you are watching, with link suggestions and platform marks.
    pub combined_chat: bool,
    /// Docked chats: several chats kept open beside the stream, switched from a
    /// list or a row of tabs.
    pub chat_dock: bool,
    /// Marks on the chat scrollbar for highlights and mentions. A touch list
    /// draws no scrollbar to mark.
    pub scrollbar_marks: bool,
    /// Words taught to the chat spellchecker. A phone spells with its keyboard,
    /// which never reads them.
    pub spell_dictionary: bool,
}

/// This build's answer.
pub fn current() -> ClientCapabilities {
    let desktop = cfg!(desktop);
    ClientCapabilities {
        combined_chat: desktop,
        chat_dock: desktop,
        scrollbar_marks: desktop,
        spell_dictionary: desktop,
    }
}

/// Whether this build has the capability named `name` (a field of
/// `ClientCapabilities`). An unknown name is never had, so a typo hides a row
/// rather than showing one for a feature that is not there.
pub fn has(name: &str) -> bool {
    let caps = current();
    match name {
        "combined_chat" => caps.combined_chat,
        "chat_dock" => caps.chat_dock,
        "scrollbar_marks" => caps.scrollbar_marks,
        "spell_dictionary" => caps.spell_dictionary,
        _ => false,
    }
}

/// The capabilities, read once by each shell at boot.
#[tauri::command]
pub fn get_client_capabilities() -> ClientCapabilities {
    current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_surfaces_follow_the_build() {
        let caps = current();
        for on in [caps.combined_chat, caps.chat_dock, caps.scrollbar_marks, caps.spell_dictionary] {
            assert_eq!(on, cfg!(desktop));
        }
        assert_eq!(has("combined_chat"), cfg!(desktop));
        assert_eq!(has("scrollbar_marks"), cfg!(desktop));
        assert!(!has("no_such_capability"));
    }

    #[test]
    fn serializes_snake_case_fields() {
        let json = serde_json::to_value(current()).unwrap();
        for key in ["combined_chat", "chat_dock", "scrollbar_marks", "spell_dictionary"] {
            assert_eq!(json[key], cfg!(desktop), "{key}");
        }
        assert_eq!(json.as_object().unwrap().len(), 4);
    }
}
