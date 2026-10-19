//! The Custom interface font's files, for the phones (see `services::custom_font`).

use crate::services::custom_font::{self as custom, Lookup};

/// The faces of a typed font family, read from Google Fonts once and kept on
/// disk, so it holds offline. `with_data` inlines each file as base64 for a page
/// that cannot read a path. The answer says `ready` with the font, `not_found`
/// when Google has no such family, or `unreachable` when it could not be asked.
#[tauri::command]
pub async fn get_custom_font(family: String, with_data: Option<bool>) -> Result<Lookup, String> {
    Ok(if with_data.unwrap_or(false) {
        custom::resolve_with_data(&family).await
    } else {
        custom::resolve(&family).await
    })
}
