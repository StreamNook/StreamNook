//! Chat composer spell checking. The work runs off the async runtime: the first
//! call parses the dictionary, and suggestion can take a few milliseconds.

use crate::services::spellcheck::{self, SpellVerdict};

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn spell_warm() -> Result<(), String> {
    blocking(spellcheck::warm).await
}

#[tauri::command]
pub async fn spell_check(words: Vec<String>) -> Result<Vec<String>, String> {
    blocking(move || spellcheck::check(&words)).await
}

#[tauri::command]
pub async fn spell_suggest(word: String) -> Result<SpellVerdict, String> {
    blocking(move || spellcheck::suggest(&word)).await
}
