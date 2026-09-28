/// Emoji Commands - Exposes emoji conversion functionality to frontend
use crate::services::emoji_service;

/// Converts emoji shortcodes in text to Unicode emojis
/// Called from frontend to offload emoji map from JavaScript heap
#[tauri::command]
pub fn convert_emoji_shortcodes(text: String) -> String {
    emoji_service::convert_emoji_shortcodes(&text)
}

/// The same conversion for many texts in one round trip. A page of stream
/// cards asks for its titles together, and one call for the page costs what
/// one title used to.
#[tauri::command]
pub fn convert_emoji_shortcodes_batch(texts: Vec<String>) -> Vec<String> {
    texts.iter().map(|text| emoji_service::convert_emoji_shortcodes(text)).collect()
}
