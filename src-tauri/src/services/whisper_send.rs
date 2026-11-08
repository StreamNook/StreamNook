//! Sending whispers: the queue, delivery checks, retry and discard. Card 7.5
//! writes it; these stubs let the commands compile.

pub use crate::services::whisper_inbox::SendTarget;
use crate::services::whisper_inbox::WhisperUpdate;

/// The longest whisper StreamNook sends, in Unicode scalar values.
pub const MAX_LEN: usize = 500;

/// A whisper's length the way `MAX_LEN` counts it.
pub fn text_len(text: &str) -> usize {
    text.chars().count()
}

pub async fn retry(_key: String, _message_id: String) -> Result<WhisperUpdate, String> {
    Err("not ready".into())
}

pub fn discard(_key: &str, _message_id: &str) -> Result<(), String> {
    Err("not ready".into())
}

/// Pair Twitch's copies of our own whispers with local sends. Card 7.5 writes it.
pub fn adopt(
    _messages: &mut Vec<crate::services::whisper_store::StoredWhisper>,
    _fresh: &mut Vec<crate::services::whisper_store::StoredWhisper>,
    _me: &str,
) -> usize {
    0
}

/// Check every send still waiting on Twitch. Card 7.5 writes it.
pub fn recheck_pending() {}

/// End a signed-out account's sends. Card 7.5 writes it.
pub async fn teardown(_closing: &crate::services::whisper_store::Closing) {}
