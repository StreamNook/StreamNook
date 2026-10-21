//! The chat image uploader's commands (`services::image_upload`).

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::models::settings::AppState;
use crate::services::image_upload::{self as upload, UploadHost};
use crate::services::image_upload_store::{self as store, RecentUpload};

#[derive(Debug, Serialize)]
pub struct UploadedImage {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize)]
pub struct ImageUploadPanel {
    pub host: UploadHost,
    pub needs_consent: bool,
    pub recents: Vec<UploadedImage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonBody {
    path: Option<String>,
    data_base64: Option<String>,
}

fn target(state: &AppState) -> Result<upload::Target, String> {
    let s = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(upload::target(&s.chat_input.image_uploader))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn no_image() -> String {
    "No image was sent.".into()
}

/// The emote menu's Images section.
#[tauri::command]
pub fn image_upload_panel(state: State<'_, AppState>) -> Result<ImageUploadPanel, String> {
    let t = target(&state)?;
    Ok(ImageUploadPanel {
        needs_consent: !store::accepted(&t.key),
        host: t.host(),
        recents: store::recents(now_ms())
            .into_iter()
            .map(|r| UploadedImage { url: r.url, width: r.width, height: r.height })
            .collect(),
    })
}

/// The viewer said yes to sending images to the current host.
#[tauri::command]
pub fn accept_image_upload_host(state: State<'_, AppState>) -> Result<(), String> {
    store::accept(&target(&state)?.key);
    Ok(())
}

/// The preset hosts, for Settings on every app.
#[tauri::command]
pub fn image_upload_hosts() -> Vec<UploadHost> {
    upload::hosts()
}

/// Sends a 1-pixel PNG to the current host and returns its link. Not
/// remembered, and asks no consent (pressing Test is the consent).
#[tauri::command]
pub async fn test_image_upload(state: State<'_, AppState>) -> Result<String, String> {
    let t = target(&state)?;
    let prepared = upload::Prepared { bytes: upload::test_png(), ext: "png", mime: "image/png", width: 1, height: 1 };
    upload::post(prepared, &t).await
}

/// Uploads one chat image. Body: raw bytes (desktop paste and HTML5 drops),
/// `{ dataBase64 }` (Android and the iPhone, whose IPC carries JSON only) or
/// `{ path }` (desktop's picker and native drops).
#[tauri::command]
pub async fn upload_chat_image(
    request: tauri::ipc::Request<'_>,
    state: State<'_, AppState>,
) -> Result<UploadedImage, String> {
    let t = target(&state)?;
    if !store::accepted(&t.key) {
        return Err("consent-required".into());
    }
    let bytes = match request.body() {
        tauri::ipc::InvokeBody::Raw(b) => b.clone(),
        // Desktop's postMessage fallback (custom protocol blocked) sends raw
        // bytes as a JSON array of numbers.
        tauri::ipc::InvokeBody::Json(serde_json::Value::Array(items)) => items
            .iter()
            .map(|v| v.as_u64().filter(|n| *n <= 255).map(|n| n as u8))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(no_image)?,
        tauri::ipc::InvokeBody::Json(v) => {
            let body: JsonBody = serde_json::from_value(v.clone()).map_err(|_| no_image())?;
            if let Some(b64) = body.data_base64 {
                base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| no_image())?
            } else if let Some(path) = body.path {
                read_picked(&path)?
            } else {
                return Err(no_image());
            }
        }
    };
    let for_prepare = t.clone();
    let prepared = tokio::task::spawn_blocking(move || upload::prepare(bytes, &for_prepare))
        .await
        .map_err(|e| e.to_string())??;
    let (width, height) = (prepared.width, prepared.height);
    log::info!("[image_upload] {} {} {}x{} {} bytes", t.key, prepared.ext, width, height, prepared.bytes.len());
    let url = upload::post(prepared, &t).await.inspect_err(|e| log::warn!("[image_upload] {}: {}", t.key, e))?;
    let at = now_ms();
    store::remember(RecentUpload {
        url: url.clone(),
        width,
        height,
        at_ms: at,
        host: t.key.clone(),
        expires_ms: t.lifetime_hours.map(|h| at + h as i64 * 3_600_000),
    });
    let mut card = crate::commands::link_preview::image_preview(&url, &url);
    card.width = Some(width);
    card.height = Some(height);
    crate::services::link_preview_cache::seed(&url, card);
    Ok(UploadedImage { url, width, height })
}

/// A file the viewer picked or dropped: an image by name, at most 25 MB.
fn read_picked(path: &str) -> Result<Vec<u8>, String> {
    let p = std::path::Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["jpg", "jpeg", "png", "gif", "webp", "avif", "bmp", "heic", "heif"].contains(&ext.as_str()) {
        return Err("That file isn't a picture StreamNook can send.".into());
    }
    let opened = || "That file could not be opened.".to_string();
    let meta = std::fs::metadata(p).map_err(|_| opened())?;
    if !meta.is_file() {
        return Err(opened());
    }
    if meta.len() > upload::MAX_INPUT_BYTES as u64 {
        return Err("That image is larger than 25 MB.".into());
    }
    std::fs::read(p).map_err(|_| opened())
}
