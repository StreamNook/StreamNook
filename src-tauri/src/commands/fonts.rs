//! The Custom interface font's files, for the phones (see `services::custom_font`).

use crate::services::custom_font::{self as custom, CustomFont};

/// The faces of a typed font family, read from Google Fonts once and kept on
/// disk, so it holds offline. `with_data` inlines each file as base64 for a page
/// that cannot read a path. None when the name is empty or Google does not know
/// the family; the app then keeps its own font.
#[tauri::command]
pub async fn get_custom_font(family: String, with_data: Option<bool>) -> Result<Option<CustomFont>, String> {
    Ok(if with_data.unwrap_or(false) {
        custom::resolve_with_data(&family).await
    } else {
        custom::resolve(&family).await
    })
}
