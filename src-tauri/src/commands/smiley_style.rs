//! The smiley style row's commands: what the row draws, the switch, and what a
//! window needs to dress its own copy of the account's emote list.

use crate::services::twitch_smiley_style::{
    self as style, OwnSmilies, SmileyStyle, SmileyStyleAnswer,
};

/// What the settings row draws, from the saved record: never waits on Twitch,
/// and asks for a fresh read in the background when the record is old. `view`
/// is None until the signed-in account has been read once.
#[tauri::command]
pub async fn get_smiley_style() -> Result<SmileyStyleAnswer, String> {
    Ok(style::style_answer())
}

/// Switches the signed-in account's smiley style to the set `key` names
/// (`robots`, `purple`, `monkeys`, or a `set:` key from the view) through the
/// twitch.tv web session; answers the view with the new set current.
#[tauri::command]
pub async fn set_smiley_style(key: String) -> Result<SmileyStyle, String> {
    style::switch(&key).await
}

/// What a window needs to dress its own emote lists in the account's smiley
/// style, with the counter later `twitch://smilies-changed` events carry.
#[tauri::command]
pub async fn get_own_smilies() -> OwnSmilies {
    style::own_smilies()
}
