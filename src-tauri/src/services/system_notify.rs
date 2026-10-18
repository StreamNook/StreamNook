//! The operating system's own notifications, for when StreamNook is not the
//! window in use.
//!
//! `commands::notification_mirror` decides WHETHER a notification goes here
//! (the "System notifications" setting is on and the main window is not
//! focused); this only shows it.
//!
//! Windows gets the whole thing, as toast XML written here (`toast_xml`): a
//! round face or square artwork, the action as a button, a reply box for
//! whispers and chat mentions, and a click that brings the main window forward
//! and runs the notification's own action. The toast is silent, because the
//! main window already played the user's chosen sound. A click is a
//! `streamnook://notify` link, so it works after the app closed too; a reply
//! arrives through the toast's own event while this process runs.
//!
//! macOS and Linux go through `tauri-plugin-notification`, which is already
//! part of those builds: title and body (and the picture on Linux), no click
//! action. Their click callbacks work differently on each system and want a
//! device to test on.
//!
//! A toast cannot load a picture from the web (an unpackaged app's toast reads
//! local files only), so each picture is copied to
//! `cache/notification-images` first, keyed by its URL, and the folder is
//! swept of week-old files once per run.
//!
//! Some notifications carry more (`RichToast`, Windows only): a picture drawn
//! here, a small line under the text, a row per channel, a banner, a progress
//! bar, extra buttons, a right-click item, and a tag so a newer toast replaces
//! the older one instead of stacking. A quiet one goes straight to Action
//! Center without popping up.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use log::debug;
use sha2::{Digest, Sha256};

use crate::commands::notification_mirror::{MirroredNotice, NoticeAction};
use crate::rt::AppHandle;

const IMAGE_DIR: &str = "notification-images";
const IMAGE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const IMAGE_FETCH_TIMEOUT: Duration = Duration::from_secs(6);
const IMAGE_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Windows shows at most five buttons and menu items together.
const MAX_ACTIONS: usize = 5;

static SWEPT: AtomicBool = AtomicBool::new(false);

/// What a Windows toast can carry beyond a title, a line and one picture.
#[derive(Debug, Clone, Default)]
pub struct RichToast {
    /// A picture drawn here, already on disk, shown as drawn (no crop). Wins
    /// over the notice's own picture.
    pub logo_path: Option<PathBuf>,
    /// The small line under the text.
    pub attribution: Option<String>,
    /// The wide banner on top, already on disk.
    pub hero_path: Option<PathBuf>,
    /// One row per channel: face, name, amount.
    pub rows: Vec<ToastRow>,
    pub progress: Option<ToastProgress>,
    /// Buttons besides the notice's own, each opening its own action.
    pub buttons: Vec<(String, NoticeAction)>,
    /// Right-click menu items.
    pub menu: Vec<(String, NoticeAction)>,
    /// A newer toast with the same tag and group replaces this one.
    pub tag: Option<String>,
    pub group: Option<String>,
    /// The heading these toasts gather under in Action Center: (id, title).
    pub header: Option<(String, String)>,
    /// Straight to Action Center, without popping up.
    pub quiet: bool,
    /// Leaves Action Center after this long.
    pub expires_in: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToastRow {
    pub face_url: Option<String>,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToastProgress {
    pub title: String,
    /// 0.0 to 1.0.
    pub value: f64,
    /// Drawn at the bar's right end ("830 / 1,000").
    pub value_text: String,
    /// Drawn at the bar's left end ("170 to go").
    pub status: String,
}

/// Show `notice` as a system notification. Never fails loudly: a missing
/// picture shows the notification without one, and an OS refusal is logged.
pub async fn show(app: AppHandle, notice: MirroredNotice) {
    show_with(app, notice, None).await;
}

/// `show`, with whatever more the notification carries.
pub async fn show_with(app: AppHandle, notice: MirroredNotice, rich: Option<RichToast>) {
    let image = match notice.image_url.as_deref() {
        Some(url) if !url.is_empty() && rich.as_ref().is_none_or(|r| r.logo_path.is_none()) => local_image(url).await,
        _ => None,
    };
    let mut faces = Vec::new();
    for row in rich.as_ref().map(|r| r.rows.as_slice()).unwrap_or_default() {
        faces.push(match row.face_url.as_deref() {
            Some(url) => local_image(url).await,
            None => None,
        });
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        platform::show(&app, &notice, image.as_deref(), rich.as_ref(), &faces)
    })
    .await;
    match result {
        Ok(Err(e)) => log::warn!("[SystemNotify] show failed: {e}"),
        Err(e) => log::warn!("[SystemNotify] show task failed: {e}"),
        Ok(Ok(())) => {}
    }
}

/// Take the toast with this tag and group out of Action Center (a prediction
/// that stopped taking bets). Windows only; elsewhere nothing to remove.
pub fn remove(app: &AppHandle, tag: &str, group: &str) {
    #[cfg(windows)]
    if let Err(e) = platform::remove(app, tag, group) {
        debug!("[SystemNotify] remove {tag} failed: {e}");
    }
    #[cfg(not(windows))]
    let _ = (app, tag, group);
}

/// Shows a rich sample under the dev identity, for hand-run tests.
#[cfg(all(test, windows))]
pub(crate) fn show_sample(notice: &MirroredNotice, rich: &RichToast, faces: &[Option<PathBuf>]) -> Result<(), String> {
    platform::show_sample(notice, rich, faces)
}

/// File name for a picture URL: stable across runs, so a face seen before is
/// not fetched again.
pub(crate) fn image_key(url: &str) -> String {
    Sha256::digest(url.as_bytes())
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The extension for formats a toast can draw, from the bytes themselves.
fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG") {
        Some("png")
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some("jpg")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

pub(crate) fn image_dir() -> Option<PathBuf> {
    let dir = crate::services::cache_service::get_cache_dir().ok()?.join(IMAGE_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn existing(dir: &Path, key: &str) -> Option<PathBuf> {
    ["png", "jpg", "gif", "webp"]
        .iter()
        .map(|ext| dir.join(format!("{key}.{ext}")))
        .find(|p| p.is_file())
}

fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > IMAGE_MAX_AGE);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A local copy of the picture at `url`, fetched once and kept a week.
pub(crate) async fn local_image(url: &str) -> Option<PathBuf> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    let dir = image_dir()?;
    if !SWEPT.swap(true, Ordering::Relaxed) {
        let d = dir.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || sweep(&d)).await;
    }
    let key = image_key(url);
    if let Some(path) = existing(&dir, &key) {
        return Some(path);
    }
    let response = crate::services::http::client()
        .get(url)
        .timeout(IMAGE_FETCH_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > IMAGE_MAX_BYTES {
        return None;
    }
    let ext = image_ext(&bytes)?;
    let path = dir.join(format!("{key}.{ext}"));
    let write_path = path.clone();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&write_path, &bytes))
        .await
        .ok()?
        .ok()?;
    debug!("[SystemNotify] cached picture {}", path.display());
    Some(path)
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 forbids most control characters outright.
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// The Windows toast for `notice`. A click (body or button) is a
/// `streamnook://notify` link carrying the notice's id and action, which
/// Windows opens whether or not the app still runs. A reply target adds a text
/// box and a Send button; Send is a foreground activation the running app
/// answers in-process, without bringing a window up.
pub(crate) fn toast_xml(
    notice: &MirroredNotice,
    image: Option<&Path>,
    rich: Option<&RichToast>,
    faces: &[Option<PathBuf>],
) -> String {
    use crate::commands::notification_mirror::ToastArgs;
    use crate::commands::notification_mirror::REPLY_INPUT;

    let src = |path: &Path| xml_escape(&path.display().to_string());
    let open = ToastArgs { id: notice.id.clone(), action: notice.action.clone(), reply: None, send: false }.to_url();
    // A button or menu item opens its own action under its own id, so a click
    // on it never runs the body's action instead.
    let link_for = |n: usize, action: &NoticeAction| {
        ToastArgs { id: format!("{}:{n}", notice.id), action: Some(action.clone()), reply: None, send: false }.to_url()
    };
    let clickable = notice.action_label.is_some() || notice.action.is_some();
    let mut xml = String::from("<toast");
    if clickable {
        xml.push_str(&format!(" launch=\"{}\" activationType=\"protocol\"", xml_escape(&open)));
    }
    xml.push('>');
    if let Some((id, title)) = rich.and_then(|r| r.header.as_ref()) {
        let header_open = ToastArgs { id: id.clone(), action: None, reply: None, send: false }.to_url();
        xml.push_str(&format!(
            "<header id=\"{}\" title=\"{}\" arguments=\"{}\" activationType=\"protocol\"/>",
            xml_escape(id),
            xml_escape(title),
            xml_escape(&header_open)
        ));
    }
    xml.push_str("<visual><binding template=\"ToastGeneric\">");
    if let Some(path) = rich.and_then(|r| r.hero_path.as_deref()) {
        xml.push_str(&format!("<image placement=\"hero\" src=\"{}\"/>", src(path)));
    }
    xml.push_str(&format!("<text hint-maxLines=\"1\">{}</text>", xml_escape(&notice.title)));
    if !notice.body.is_empty() {
        xml.push_str(&format!("<text>{}</text>", xml_escape(&notice.body)));
    }
    for (n, row) in rich.map(|r| r.rows.as_slice()).unwrap_or_default().iter().enumerate() {
        xml.push_str("<group>");
        if let Some(Some(face)) = faces.get(n) {
            xml.push_str(&format!(
                "<subgroup hint-weight=\"12\"><image src=\"{}\" hint-crop=\"circle\" hint-removeMargin=\"true\"/></subgroup>",
                src(face)
            ));
        }
        xml.push_str(&format!(
            "<subgroup hint-weight=\"58\" hint-textStacking=\"center\"><text hint-style=\"base\">{}</text></subgroup>\
             <subgroup hint-weight=\"40\" hint-textStacking=\"center\"><text hint-style=\"baseSubtle\" hint-align=\"right\">{}</text></subgroup>",
            xml_escape(&row.name),
            xml_escape(&row.value)
        ));
        xml.push_str("</group>");
    }
    if let Some(p) = rich.and_then(|r| r.progress.as_ref()) {
        xml.push_str(&format!(
            "<progress title=\"{}\" value=\"{:.3}\" valueStringOverride=\"{}\" status=\"{}\"/>",
            xml_escape(&p.title),
            p.value.clamp(0.0, 1.0),
            xml_escape(&p.value_text),
            xml_escape(&p.status)
        ));
    }
    if let Some(line) = rich.and_then(|r| r.attribution.as_deref()) {
        xml.push_str(&format!("<text placement=\"attribution\">{}</text>", xml_escape(line)));
    }
    if let Some(path) = rich.and_then(|r| r.logo_path.as_deref()) {
        xml.push_str(&format!("<image placement=\"appLogoOverride\" hint-crop=\"none\" src=\"{}\"/>", src(path)));
    } else if let Some(path) = image {
        let crop = if notice.round_image { "circle" } else { "none" };
        xml.push_str(&format!(
            "<image placement=\"appLogoOverride\" hint-crop=\"{crop}\" src=\"{}\"/>",
            src(path)
        ));
    }
    xml.push_str("</binding></visual>");

    let mut actions = String::new();
    let mut count = 0;
    if let Some(reply) = &notice.reply {
        let send = ToastArgs { id: notice.id.clone(), action: None, reply: Some(reply.clone()), send: true }.encode();
        actions.push_str(&format!(
            "<input id=\"{REPLY_INPUT}\" type=\"text\" placeHolderContent=\"{}\"/>",
            xml_escape(&reply.placeholder())
        ));
        actions.push_str(&format!(
            "<action content=\"Send\" arguments=\"{}\" activationType=\"foreground\" hint-inputId=\"{REPLY_INPUT}\"/>",
            xml_escape(&send)
        ));
        count += 1;
    }
    if let Some(label) = &notice.action_label {
        actions.push_str(&format!(
            "<action content=\"{}\" arguments=\"{}\" activationType=\"protocol\"/>",
            xml_escape(label),
            xml_escape(&open)
        ));
        count += 1;
    }
    let extra = rich.map(|r| {
        let buttons = r.buttons.iter().map(|(label, action)| (label, action, ""));
        let menu = r.menu.iter().map(|(label, action)| (label, action, " placement=\"contextMenu\""));
        buttons.chain(menu).collect::<Vec<_>>()
    });
    for (n, (label, action, placement)) in extra.unwrap_or_default().into_iter().enumerate() {
        if count >= MAX_ACTIONS {
            break;
        }
        actions.push_str(&format!(
            "<action content=\"{}\" arguments=\"{}\" activationType=\"protocol\"{placement}/>",
            xml_escape(label),
            xml_escape(&link_for(n + 1, action))
        ));
        count += 1;
    }
    if !actions.is_empty() {
        xml.push_str(&format!("<actions>{actions}</actions>"));
    }
    // Silent: the main window already played the user's chosen sound.
    xml.push_str("<audio silent=\"true\"/></toast>");
    xml
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::commands::notification_mirror::REPLY_INPUT;
    use std::collections::VecDeque;
    use std::sync::{Mutex, OnceLock};
    use tauri::Manager;
    use windows::core::{IInspectable, Interface, HSTRING};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::{IPropertyValue, TypedEventHandler};
    use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager};

    /// The last few toasts this process showed, kept so their in-process
    /// handlers stay alive while they sit in Action Center.
    static SHOWN: Mutex<VecDeque<ToastNotification>> = Mutex::new(VecDeque::new());
    const SHOWN_KEPT: usize = 24;

    /// The identity a toast is shown under. Windows only attributes a toast
    /// from an unpackaged exe to an app it can name, and StreamNook ships as a
    /// bare exe with no Start Menu shortcut to carry one, so this registers its
    /// own under HKCU (no admin): display name and icon. Debug builds use their
    /// own, so a dev toast is never mistaken for the installed app's.
    ///
    /// It is never set as the PROCESS's AppUserModelID: that would change how
    /// the taskbar groups the window and orphan every pinned StreamNook.
    fn app_id(app: &AppHandle) -> &'static str {
        static ID: OnceLock<String> = OnceLock::new();
        ID.get_or_init(|| {
            let base = app.config().identifier.clone();
            let (id, name) = if cfg!(debug_assertions) {
                (format!("{base}.debug"), "StreamNook (dev)")
            } else {
                (base, "StreamNook")
            };
            if let Err(e) = register(&id, name) {
                log::warn!("[SystemNotify] could not register the toast identity: {e}");
            }
            id
        })
    }

    pub(super) fn register(id: &str, name: &str) -> Result<(), String> {
        use windows::core::HSTRING;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
            REG_OPTION_NON_VOLATILE, REG_SZ,
        };

        let icon = crate::services::cache_service::get_cache_dir()
            .map_err(|e| e.to_string())?
            .join("notification-icon.png");
        std::fs::write(&icon, include_bytes!("../../icons/128x128.png")).map_err(|e| e.to_string())?;

        let subkey = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{id}"));
        let mut key = HKEY::default();
        // SAFETY: plain Win32 registry calls with owned, NUL-terminated
        // strings; the key handle is closed below on every path.
        unsafe {
            let status = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &subkey,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            );
            if status.is_err() {
                return Err(format!("RegCreateKeyExW: {status:?}"));
            }
            let mut result = Ok(());
            for (value, data) in [("DisplayName", name.to_string()), ("IconUri", icon.display().to_string())] {
                let wide: Vec<u8> = data
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .flat_map(u16::to_le_bytes)
                    .collect();
                let status = RegSetValueExW(key, &HSTRING::from(value), None, REG_SZ, Some(&wide));
                if status.is_err() {
                    result = Err(format!("RegSetValueExW {value}: {status:?}"));
                }
            }
            let _ = RegCloseKey(key);
            result
        }
    }

    /// The text typed into the toast's reply box, if any.
    fn typed_reply(args: &ToastActivatedEventArgs) -> Option<String> {
        let value = args.UserInput().ok()?.Lookup(&HSTRING::from(REPLY_INPUT)).ok()?;
        let text = value.cast::<IPropertyValue>().ok()?.GetString().ok()?.to_string();
        (!text.trim().is_empty()).then_some(text)
    }

    /// Windows counts a toast's expiry in 100 ns ticks since 1601.
    fn expiry_ticks(after: Duration) -> i64 {
        const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
        let unix = SystemTime::now()
            .checked_add(after)
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .unwrap_or_default();
        UNIX_EPOCH_TICKS + (unix.as_nanos() / 100) as i64
    }

    /// Tag, group, quiet and expiry: the parts of a rich toast that are not XML.
    fn apply(toast: &ToastNotification, rich: &RichToast) -> windows::core::Result<()> {
        use windows::Foundation::{DateTime, IReference, PropertyValue};
        if let Some(tag) = &rich.tag {
            toast.SetTag(&HSTRING::from(tag.as_str()))?;
        }
        if let Some(group) = &rich.group {
            toast.SetGroup(&HSTRING::from(group.as_str()))?;
        }
        if rich.quiet {
            toast.SetSuppressPopup(true)?;
        }
        if let Some(after) = rich.expires_in {
            let when = PropertyValue::CreateDateTime(DateTime { UniversalTime: expiry_ticks(after) })?;
            toast.SetExpirationTime(&when.cast::<IReference<DateTime>>()?)?;
        }
        Ok(())
    }

    /// A sample under the dev identity, for hand-run tests (no app running).
    #[cfg(test)]
    pub fn show_sample(notice: &MirroredNotice, rich: &RichToast, faces: &[Option<PathBuf>]) -> Result<(), String> {
        let id = "com.streamnook.dev.debug";
        register(id, "StreamNook (dev)")?;
        let doc = XmlDocument::new().map_err(|e| e.to_string())?;
        doc.LoadXml(&HSTRING::from(toast_xml(notice, None, Some(rich), faces))).map_err(|e| e.to_string())?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| e.to_string())?;
        apply(&toast, rich).map_err(|e| e.to_string())?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(id))
            .and_then(|notifier| notifier.Show(&toast))
            .map_err(|e| e.to_string())
    }

    pub fn remove(app: &AppHandle, tag: &str, group: &str) -> Result<(), String> {
        ToastNotificationManager::History()
            .and_then(|h| h.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(group), &HSTRING::from(app_id(app))))
            .map_err(|e| e.to_string())
    }

    pub fn show(
        app: &AppHandle,
        notice: &MirroredNotice,
        image: Option<&Path>,
        rich: Option<&RichToast>,
        faces: &[Option<PathBuf>],
    ) -> Result<(), String> {
        let doc = XmlDocument::new().map_err(|e| e.to_string())?;
        doc.LoadXml(&HSTRING::from(toast_xml(notice, image, rich, faces))).map_err(|e| e.to_string())?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| e.to_string())?;
        if let Some(rich) = rich {
            apply(&toast, rich).map_err(|e| e.to_string())?;
        }
        // Only Send arrives here: a click is a link Windows opens, and its
        // arguments do not decode as `ToastArgs`, so it is never run twice.
        let handler_app = app.clone();
        toast
            .Activated(&TypedEventHandler::new(move |_, insp| {
                let insp: &Option<IInspectable> = &insp;
                if let Some(args) = insp.as_ref().and_then(|i| i.cast::<ToastActivatedEventArgs>().ok()) {
                    let arguments = args.Arguments().map(|a| a.to_string()).unwrap_or_default();
                    crate::commands::notification_mirror::on_activated(&handler_app, &arguments, typed_reply(&args));
                }
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id(app)))
            .and_then(|notifier| notifier.Show(&toast))
            .map_err(|e| e.to_string())?;
        let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
        shown.push_back(toast);
        while shown.len() > SHOWN_KEPT {
            shown.pop_front();
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    use tauri_plugin_notification::NotificationExt;

    /// Title and text only: the rows and the small line join the text, and a
    /// quiet one is not shown at all (these systems cannot update quietly).
    pub fn show(
        app: &AppHandle,
        notice: &MirroredNotice,
        image: Option<&Path>,
        rich: Option<&RichToast>,
        _faces: &[Option<PathBuf>],
    ) -> Result<(), String> {
        if rich.is_some_and(|r| r.quiet) {
            return Ok(());
        }
        let mut body = notice.body.clone();
        if let Some(r) = rich {
            for line in r.rows.iter().map(|row| format!("{}  {}", row.name, row.value)).chain(r.attribution.clone()) {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&line);
            }
        }
        let mut builder = app.notification().builder().title(&notice.title);
        if !body.is_empty() {
            builder = builder.body(&body);
        }
        // Linux draws a picture from a file path; macOS shows the app icon.
        #[cfg(target_os = "linux")]
        if let Some(path) = rich.and_then(|r| r.logo_path.as_deref()).or(image) {
            builder = builder.icon(path.display().to_string());
        }
        #[cfg(not(target_os = "linux"))]
        let _ = image;
        builder.show().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_key_is_stable_and_short() {
        let a = image_key("https://static-cdn.jtvnw.net/a.png");
        assert_eq!(a, image_key("https://static-cdn.jtvnw.net/a.png"));
        assert_ne!(a, image_key("https://static-cdn.jtvnw.net/b.png"));
        assert_eq!(a.len(), 24);
    }

    #[test]
    fn only_drawable_formats_get_an_extension() {
        assert_eq!(image_ext(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(image_ext(b"\xFF\xD8\xFF\xE0"), Some("jpg"));
        assert_eq!(image_ext(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_ext(b"\0\0\0\x20ftypavif"), None);
        assert_eq!(image_ext(b"<html>"), None);
    }

    use crate::commands::notification_mirror::{NoticeAction, NoticeReply, ToastArgs};

    fn notice() -> MirroredNotice {
        MirroredNotice {
            id: "mention-1".into(),
            kind: "mention".into(),
            title: "Viewer mentioned you in <somechannel>".into(),
            body: "hey @you & \"friends\"".into(),
            image_url: None,
            round_image: true,
            action_label: Some("Open chat".into()),
            action: Some(NoticeAction::Watch { channel: "somechannel".into() }),
            reply: Some(NoticeReply::Chat {
                channel: "somechannel".into(),
                broadcaster_id: Some("1".into()),
                parent_id: "p-1".into(),
                to_name: "Viewer".into(),
            }),
        }
    }

    /// The XML is well formed (every value escaped), and the arguments a click
    /// and a Send hand back decode to what was put in.
    #[test]
    fn toast_xml_escapes_and_carries_its_arguments() {
        let xml = toast_xml(&notice(), Some(Path::new(r"C:\cache\a&b.png")), None, &[]);
        assert!(xml.contains("&lt;somechannel&gt;"));
        assert!(xml.contains("hey @you &amp; &quot;friends&quot;"));
        assert!(xml.contains(r#"src="C:\cache\a&amp;b.png""#));
        assert!(xml.contains(r#"hint-crop="circle""#));
        assert!(xml.contains(r#"<input id="reply" type="text" placeHolderContent="Reply to Viewer"/>"#));
        assert!(xml.contains(r#"activationType="foreground" hint-inputId="reply""#));
        assert!(xml.contains(r#"activationType="protocol""#));
        let attr = |name: &str, nth: usize| -> String {
            let raw = xml.split(&format!("{name}=\"")).nth(nth).unwrap().split('"').next().unwrap();
            raw.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
        };
        let launch = ToastArgs::from_url(&url::Url::parse(&attr("launch", 1)).unwrap()).unwrap();
        assert_eq!(launch.id, "mention-1");
        assert!(!launch.send && launch.action.is_some() && launch.reply.is_none());
        // The button opens the same link as the body.
        assert_eq!(attr("arguments", 2), attr("launch", 1));
        let send = ToastArgs::decode(&attr("arguments", 1)).unwrap();
        assert!(send.send && send.reply.is_some());
    }

    #[test]
    fn plain_notices_stay_simple() {
        let plain = MirroredNotice { action_label: None, action: None, reply: None, ..notice() };
        let xml = toast_xml(&plain, None, None, &[]);
        assert!(!xml.contains("launch=") && !xml.contains("<actions>"));
        assert!(xml.ends_with(r#"<audio silent="true"/></toast>"#));
        assert_eq!(xml_escape("a\u{1}b\nc"), "ab\nc");
    }

    /// A rich toast: header, banner, rows, progress, the small line, a drawn
    /// logo that wins over the notice's picture, extra buttons and a menu item
    /// that each open their own action, and Windows' five-action cap.
    #[test]
    fn rich_toast_xml_carries_every_part() {
        let rich = RichToast {
            logo_path: Some(PathBuf::from(r"C:\cache\badge.png")),
            attribution: Some("Balance 12,450".into()),
            hero_path: Some(PathBuf::from(r"C:\cache\hero.png")),
            rows: vec![ToastRow { face_url: None, name: "Ludwig".into(), value: "+50 Goldwig".into() }],
            progress: Some(ToastProgress {
                title: "Next reward: Water break!".into(),
                value: 1.7,
                value_text: "830 / 1,000".into(),
                status: "170 to go".into(),
            }),
            buttons: vec![("Rewards".into(), NoticeAction::Rewards { channel: "ludwig".into() })],
            menu: vec![("Notification settings".into(), NoticeAction::Settings { tab: "Notifications".into() })],
            header: Some(("channel-points".into(), "Channel points".into())),
            ..Default::default()
        };
        let plain = MirroredNotice { action_label: None, reply: None, ..notice() };
        let faces = [Some(PathBuf::from(r"C:\f.png"))];
        let xml = toast_xml(&plain, Some(Path::new(r"C:\cache\face.png")), Some(&rich), &faces);
        assert!(xml.contains(r#"<header id="channel-points" title="Channel points""#));
        assert!(xml.contains(r#"<image placement="hero" src="C:\cache\hero.png"/>"#));
        assert!(xml.contains(r#"<image placement="appLogoOverride" hint-crop="none" src="C:\cache\badge.png"/>"#));
        assert!(!xml.contains("face.png"), "the drawn logo replaces the notice picture");
        assert!(xml.contains(r#"<image src="C:\f.png" hint-crop="circle""#));
        assert!(xml.contains(">+50 Goldwig</text>"));
        assert!(xml.contains(r#"value="1.000""#), "progress is clamped");
        assert!(xml.contains(r#"<text placement="attribution">Balance 12,450</text>"#));
        assert!(xml.contains(r#"placement="contextMenu""#));
        let rewards = xml.split(r#"<action content="Rewards" arguments=""#).nth(1).unwrap().split('"').next().unwrap();
        let back = ToastArgs::from_url(&url::Url::parse(&rewards.replace("&amp;", "&")).unwrap()).unwrap();
        assert_eq!(back.id, "mention-1:1");
        assert_eq!(back.action, Some(NoticeAction::Rewards { channel: "ludwig".into() }));

        let crowded = RichToast {
            buttons: (0..8).map(|n| (format!("B{n}"), NoticeAction::Drops)).collect(),
            ..Default::default()
        };
        let xml = toast_xml(&notice(), None, Some(&crowded), &[]);
        assert_eq!(xml.matches("<action ").count(), MAX_ACTIONS);
    }

    /// Shows a real toast with a reply box under the dev identity. Run by hand:
    /// `cargo test --no-default-features --lib system_notify -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn shows_a_real_toast() {
        use windows::core::HSTRING;
        let id = "com.streamnook.dev.debug";
        platform::register(id, "StreamNook (dev)").expect("register failed");
        let doc = windows::Data::Xml::Dom::XmlDocument::new().unwrap();
        doc.LoadXml(&HSTRING::from(toast_xml(&notice(), None, None, &[]))).expect("toast XML rejected");
        let toast = windows::UI::Notifications::ToastNotification::CreateToastNotification(&doc).unwrap();
        windows::UI::Notifications::ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(id))
            .unwrap()
            .Show(&toast)
            .expect("toast failed");
    }
}
