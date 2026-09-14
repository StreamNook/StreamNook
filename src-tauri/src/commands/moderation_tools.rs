//! Tauri boundary for the AutoMod queue and streamer mode. Thin adapters;
//! state and decisions live in services::automod_queue / streamer_mode.
//! Each command has its `generate_handler!` entry and its ACL line.

use crate::services::automod_queue::{AutomodQueue, HeldMessage};
use crate::services::streamer_mode::{StreamerMode, StreamerModeState};
use crate::services::twitch_service::TwitchService;

/// Messages AutoMod is currently holding for a channel (oldest first).
#[tauri::command]
pub async fn get_automod_queue(channel: String) -> Result<Vec<HeldMessage>, String> {
    Ok(AutomodQueue::list(&channel))
}

/// Allow or deny a held message. The queue row leaves on the EventSub
/// `automod.message.update` that follows.
#[tauri::command]
pub async fn resolve_automod_message(message_id: String, allow: bool) -> Result<(), String> {
    TwitchService::resolve_automod_message(&message_id, allow)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_streamer_mode_state() -> Result<StreamerModeState, String> {
    Ok(StreamerMode::state())
}

/// `/settitle` and `/setgame`: broadcaster-only Helix channel update.
#[tauri::command]
pub async fn update_channel_info(
    broadcaster_id: String,
    title: Option<String>,
    game_name: Option<String>,
) -> Result<(), String> {
    TwitchService::update_channel_info(&broadcaster_id, title.as_deref(), game_name.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Pronouns from pronouns.alejo.io (opt-in via user_card.show_pronouns).
#[tauri::command]
pub async fn get_user_pronouns(login: String) -> Result<Option<String>, String> {
    Ok(crate::services::pronouns::Pronouns::for_login(&login).await)
}

/// Private note on a user (keyed by Twitch user id, persisted in app data).
#[tauri::command]
pub async fn get_user_note(user_id: String) -> Result<Option<crate::services::user_notes::UserNote>, String> {
    Ok(crate::services::user_notes::UserNotes::get(&user_id))
}

#[tauri::command]
pub async fn set_user_note(
    user_id: String,
    note: String,
) -> Result<Option<crate::services::user_notes::UserNote>, String> {
    crate::services::user_notes::UserNotes::set(&user_id, &note)
}

/// Upload a pasted image to the user's configured host and return the link.
/// The image bytes arrive as the raw IPC body (no JSON/base64 copy); the
/// target and field names ride in headers so no settings lock is needed.
#[tauri::command]
pub async fn upload_image(request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let bytes: Vec<u8> = match request.body() {
        tauri::ipc::InvokeBody::Raw(b) => b.clone(),
        _ => return Err("image body must be raw bytes".into()),
    };
    if bytes.is_empty() {
        return Err("empty image".into());
    }
    if bytes.len() > 25 * 1024 * 1024 {
        return Err("image is larger than 25 MB".into());
    }
    let header = |name: &str| -> String {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let url = header("x-upload-url");
    if !url.starts_with("https://") {
        return Err("upload URL must start with https://".into());
    }
    let field = {
        let f = header("x-form-field");
        if f.is_empty() { "attachment".to_string() } else { f }
    };
    let filename = {
        let f = header("x-filename");
        if f.is_empty() { "image.png".to_string() } else { f }
    };
    let mime = {
        let m = header("x-mime");
        if m.is_empty() { "image/png".to_string() } else { m }
    };
    let response_path = header("x-response-path");
    // Text fields the host requires beside the file (catbox: reqtype=fileupload).
    let extra = header("x-extra-fields");

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename)
        .mime_str(&mime)
        .map_err(|e| e.to_string())?;
    let mut form = reqwest::multipart::Form::new().part(field, part);
    for pair in extra.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            let k = percent_decode(k);
            let v = percent_decode(v);
            if !k.is_empty() {
                form = form.text(k, v);
            }
        }
    }
    let client = crate::services::http::client().clone();
    let resp = client
        .post(&url)
        .header("User-Agent", format!("StreamNook/{}", env!("CARGO_PKG_VERSION")))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("upload failed: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("upload host answered {}", status.as_u16()));
    }
    let link = if response_path.trim().is_empty() {
        text.trim().to_string()
    } else {
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| "upload host did not return JSON".to_string())?;
        let pointer = format!("/{}", response_path.trim().trim_matches('.').replace('.', "/"));
        json.pointer(&pointer)
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| format!("no link at {} in the response", response_path))?
    };
    if !(link.starts_with("http://") || link.starts_with("https://")) {
        return Err("upload host did not return a link".into());
    }
    Ok(link)
}

/// Minimal percent-decoding for the `x-extra-fields` header (keys and values
/// are url-encoded by the frontend; `+` is not treated as a space).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(h) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(h);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
