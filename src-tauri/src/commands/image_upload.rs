//! The chat image uploader's commands (`services::image_upload`).

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::models::settings::AppState;
use crate::services::image_upload::{self as upload, UploadHost};
use crate::services::image_upload_store::{self as store, RecentUpload};

/// One upload, as the Images section draws it.
#[derive(Debug, Serialize)]
pub struct UploadedImage {
    pub url: String,
    pub width: u32,
    pub height: u32,
    /// The host key it went to.
    pub host: String,
    /// When its link stops working (ms since the epoch), if it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<i64>,
    /// "2 days left", "Expired"; absent for a link that lasts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_left: Option<String>,
    pub expired: bool,
    /// A kept copy can be sent again for a fresh link.
    pub can_reupload: bool,
    /// The kept copy on disk, drawn in place of a dead link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy_path: Option<String>,
    /// The small picture on disk, which is what the section draws.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_path: Option<String>,
    /// Less than a day left on the link.
    pub urgent: bool,
    pub uploaded_ms: i64,
    /// "2 hours ago".
    pub uploaded: String,
    /// "Oct 21, 3:40 PM".
    pub uploaded_at: String,
    /// "Expires Oct 23, 3:40 PM (2 days left)", "Kept indefinitely", ...
    pub expiry: String,
    /// "1920 × 1080 · PNG · 1.4 MB".
    pub details: String,
}

/// The uploads to one host, newest first.
#[derive(Debug, Serialize)]
pub struct UploadGroup {
    pub host: UploadHost,
    pub items: Vec<UploadedImage>,
}

#[derive(Debug, Serialize)]
pub struct ImageUploadPanel {
    /// Where the next upload goes.
    pub host: UploadHost,
    /// Every host the section can switch to.
    pub hosts: Vec<UploadHost>,
    pub needs_consent: bool,
    /// The uploads by host, the host used most recently first.
    pub groups: Vec<UploadGroup>,
    /// "48 MB on this device": what the kept files take.
    pub storage: String,
    /// The folder can be shown in the file manager (desktop).
    pub can_open_folder: bool,
}

#[derive(Debug, Serialize)]
pub struct DeletedUpload {
    /// The host deleted the file too (not only the list entry).
    pub deleted_on_host: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonBody {
    path: Option<String>,
    data_base64: Option<String>,
}

fn settings_uploader(state: &AppState) -> Result<crate::models::settings::ImageUploaderSettings, String> {
    let s = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(s.chat_input.image_uploader.clone())
}

fn target(state: &AppState) -> Result<upload::Target, String> {
    Ok(upload::target(&settings_uploader(state)?))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn no_image() -> String {
    "No image was sent.".into()
}

fn item(r: &RecentUpload, now: i64) -> UploadedImage {
    let copy_path = r
        .copy
        .as_deref()
        .and_then(store::copy_path)
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned());
    let thumb_path = r
        .thumb
        .as_deref()
        .and_then(store::thumb_path)
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned());
    let host = upload::host_for_key(&r.host);
    UploadedImage {
        url: r.url.clone(),
        width: r.width,
        height: r.height,
        host: r.host.clone(),
        expires_ms: r.expires_ms,
        time_left: r.expires_ms.map(|e| upload::time_left_label(e, now)),
        expired: r.expires_ms.is_some_and(|e| e <= now),
        can_reupload: copy_path.is_some(),
        copy_path,
        thumb_path,
        urgent: r.expires_ms.is_some_and(|e| e > now && e - now < 24 * 3_600_000),
        uploaded_ms: r.at_ms,
        uploaded: upload::ago_label(r.at_ms, now),
        uploaded_at: upload::clock_label(r.at_ms),
        expiry: upload::expiry_label(r.expires_ms, host.expires, &host.label, now),
        details: upload::details_label(r.width, r.height, &r.ext, r.bytes),
    }
}

/// The uploads grouped by host, in the order each host was last used.
fn groups(list: &[RecentUpload], now: i64) -> Vec<UploadGroup> {
    let mut out: Vec<UploadGroup> = Vec::new();
    for r in list {
        match out.iter_mut().find(|g| g.host.id == r.host) {
            Some(g) => g.items.push(item(r, now)),
            None => out.push(UploadGroup { host: upload::host_for_key(&r.host), items: vec![item(r, now)] }),
        }
    }
    out
}

/// The emote menu's Images section.
#[tauri::command]
pub fn image_upload_panel(state: State<'_, AppState>) -> Result<ImageUploadPanel, String> {
    let t = target(&state)?;
    let mut hosts = upload::hosts();
    if !hosts.iter().any(|h| h.id == t.key) {
        hosts.push(t.host());
    }
    let now = now_ms();
    tauri::async_runtime::spawn(fill_missing_thumbs());
    Ok(ImageUploadPanel {
        needs_consent: !store::accepted(&t.key),
        host: t.host(),
        hosts,
        groups: groups(&store::recents(now), now),
        storage: format!("{} on this device", upload::size_label(store::disk_bytes())),
        can_open_folder: cfg!(desktop),
    })
}

static FILLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Makes the small pictures uploads from before they were kept still lack:
/// from the kept file, or else from the link (once, in the background). The
/// next panel read draws them.
async fn fill_missing_thumbs() {
    use std::sync::atomic::Ordering;
    if FILLING.swap(true, Ordering::SeqCst) {
        return;
    }
    for (url, copy) in store::missing_thumbs() {
        let local = copy.as_deref().and_then(store::copy_path).and_then(|p| std::fs::read(p).ok());
        let bytes = match local {
            Some(b) => b,
            None => match fetch_capped(&url).await {
                Some(b) => b,
                None => continue,
            },
        };
        let made = tokio::task::spawn_blocking(move || upload::thumbnail(&bytes)).await.ok().flatten();
        if let Some(name) = made.and_then(|(thumb, ext)| store::keep_thumb(&thumb, ext)) {
            store::set_thumb(&url, name);
        }
    }
    FILLING.store(false, Ordering::SeqCst);
}

/// An upload's picture from its link, at most `MAX_INPUT_BYTES`.
async fn fetch_capped(url: &str) -> Option<Vec<u8>> {
    let mut resp = crate::commands::link_preview::CLIENT
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        out.extend_from_slice(&chunk);
        if out.len() > upload::MAX_INPUT_BYTES {
            return None;
        }
    }
    Some(out)
}

/// Shows the folder the uploads are kept in, in the file manager (desktop).
#[tauri::command]
pub async fn open_image_uploads_folder() -> Result<(), String> {
    let dir = store::folder().ok_or_else(|| "The uploads folder could not be found.".to_string())?;
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if cfg!(mobile) {
        return Err("The uploads folder lives inside the app on phones.".into());
    }
    std::process::Command::new(program).arg(dir.as_os_str()).spawn().map_err(|e| e.to_string())?;
    Ok(())
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
    upload::post(prepared, &t).await.map(|p| p.link)
}

/// Posts `prepared` to `t`, remembers it (in place of `replacing`, keeping a
/// copy where the link will expire) and seeds its link card.
async fn send(
    prepared: upload::Prepared,
    t: &upload::Target,
    replacing: Option<&RecentUpload>,
) -> Result<UploadedImage, String> {
    let (width, height, bytes, ext) = (prepared.width, prepared.height, prepared.bytes.len() as u64, prepared.ext);
    log::info!("[image_upload] {} {} {}x{} {} bytes", t.key, prepared.ext, width, height, prepared.bytes.len());
    let reused = replacing.and_then(|r| r.copy.clone());
    let reused_thumb = replacing.and_then(|r| r.thumb.clone());
    // Kept only once the host has it, so a failed upload leaves no file.
    let original = prepared.bytes.clone();
    let posted = upload::post(prepared, t).await.inspect_err(|e| log::warn!("[image_upload] {}: {}", t.key, e))?;
    let copy = reused.or_else(|| store::keep_copy(&original, ext));
    let thumb = match reused_thumb {
        Some(name) => Some(name),
        None => tokio::task::spawn_blocking(move || upload::thumbnail(&original))
            .await
            .ok()
            .flatten()
            .and_then(|(bytes, ext)| store::keep_thumb(&bytes, ext)),
    };
    let at = now_ms();
    let recent = RecentUpload {
        url: posted.link.clone(),
        width,
        height,
        at_ms: at,
        host: t.key.clone(),
        expires_ms: posted.expires_ms.or_else(|| t.lifetime_hours.map(|h| at + h as i64 * 3_600_000)),
        delete_url: posted.delete_url,
        copy,
        thumb,
        bytes,
        ext: ext.to_string(),
    };
    let shown = item(&recent, at);
    store::remember(recent, replacing.map(|r| r.url.as_str()));
    let mut card = crate::commands::link_preview::image_preview(&posted.link, &posted.link);
    card.width = Some(width);
    card.height = Some(height);
    crate::services::link_preview_cache::seed(&posted.link, card);
    Ok(shown)
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
    send(prepared, &t, None).await
}

/// Sends a kept copy again, to the host it went to, for a fresh link that
/// takes the old one's place in the list.
#[tauri::command]
pub async fn reupload_image(url: String, state: State<'_, AppState>) -> Result<UploadedImage, String> {
    let old = store::find(&url).ok_or_else(|| "That upload is no longer in the list.".to_string())?;
    let path = old
        .copy
        .as_deref()
        .and_then(store::copy_path)
        .filter(|p| p.is_file())
        .ok_or_else(|| "There is no copy of that picture left to send again.".to_string())?;
    let t = upload::target_for_key(&old.host, &settings_uploader(&state)?)
        .ok_or_else(|| "That picture's host is no longer in Settings.".to_string())?;
    let bytes = std::fs::read(&path).map_err(|_| "The kept copy could not be read.".to_string())?;
    let prepared = upload::prepared_copy(bytes)?;
    send(prepared, &t, Some(&old)).await
}

/// Takes an upload off the list, and deletes it on the host where the host
/// allows that. If the host refuses, the entry stays.
#[tauri::command]
pub async fn delete_image_upload(url: String) -> Result<DeletedUpload, String> {
    let entry = store::find(&url).ok_or_else(|| "That upload is no longer in the list.".to_string())?;
    let deleted_on_host = match entry.delete_url.as_deref() {
        Some(delete_url) => {
            let label = upload::host_for_key(&entry.host).label;
            upload::delete_on_host(delete_url, &label).await?;
            true
        }
        None => false,
    };
    store::forget(&url);
    Ok(DeletedUpload { deleted_on_host })
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
