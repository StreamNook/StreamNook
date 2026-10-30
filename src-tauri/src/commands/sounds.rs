//! Custom notification sounds. A picked file is copied into the app's own
//! `sounds` folder, so the sound keeps playing after the original is moved,
//! renamed or on a drive that is unplugged.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::models::settings::Settings;
use crate::rt::AppHandle;

/// Larger than any notification sound needs; keeps a stray music file out.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const EXTENSIONS: [&str; 9] = ["mp3", "wav", "ogg", "oga", "opus", "m4a", "aac", "flac", "webm"];
/// The imported sounds' folder, under the app data folder.
pub(crate) const FOLDER: &str = "sounds";
/// Written once the legacy sound files have been chosen; they are never chosen again.
const FROZEN_MARKER: &str = "legacy_sounds_frozen";
/// The sound files outside the app's own folder that the asset protocol may
/// play (a JSON array of absolute paths).
const ALLOW_FILE: &str = "legacy_sound_allow.json";

#[derive(Debug, Serialize)]
pub struct ImportedSound {
    pub id: String,
    pub name: String,
    pub path: String,
}

fn sounds_dir() -> Result<PathBuf, String> {
    let dir = crate::services::cache_service::get_app_data_dir()
        .map_err(|e| e.to_string())?
        .join(FOLDER);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create the sounds folder: {e}"))?;
    Ok(dir)
}

/// The lowercase extension when it is one this app plays.
fn sound_extension(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str()).then_some(ext)
}

fn import_into(src: &Path, dir: &Path) -> Result<ImportedSound, String> {
    let ext = sound_extension(src).ok_or("That file type is not a supported sound (mp3, wav, ogg, m4a, flac).")?;
    let meta = std::fs::metadata(src).map_err(|e| format!("could not read the file: {e}"))?;
    if !meta.is_file() {
        return Err("That is not a file.".into());
    }
    if meta.len() > MAX_BYTES {
        return Err("That sound is over 2 MB. Pick a short clip.".into());
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let dest = dir.join(format!("{id}.{ext}"));
    std::fs::copy(src, &dest).map_err(|e| format!("could not copy the sound: {e}"))?;
    let name = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Custom sound")
        .to_string();
    Ok(ImportedSound {
        id,
        name,
        path: dest.to_string_lossy().into_owned(),
    })
}

/// Copy the sound file at `path` into the app's sounds folder.
#[tauri::command]
pub fn import_custom_sound(path: String) -> Result<ImportedSound, String> {
    import_into(Path::new(&path), &sounds_dir()?)
}

/// Delete an imported sound. Only files inside the sounds folder are ever
/// removed, so a sound added before imports existed (which still points at
/// the user's own file) is left alone.
#[tauri::command]
pub fn remove_custom_sound(path: String) -> Result<(), String> {
    let dir = sounds_dir()?;
    let (Ok(dir), Ok(target)) = (dir.canonicalize(), Path::new(&path).canonicalize()) else {
        return Ok(());
    };
    if target.parent() == Some(dir.as_path()) {
        std::fs::remove_file(&target).map_err(|e| format!("could not delete the sound: {e}"))?;
    }
    Ok(())
}

// Sounds added before imports existed still point at the user's own file. The
// page plays sound files through the asset protocol, whose scope names the
// app's folders only, so those files are chosen once, from the settings the app
// really loaded, and kept in a list the app alone writes. No settings value read
// later can widen the scope.

/// The `path` of every custom sound entry in the settings.
fn custom_sound_paths(settings: &Settings) -> Vec<String> {
    settings
        .extra
        .get("chat_highlights")
        .and_then(|h| h.get("custom_sounds"))
        .and_then(|list| list.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|entry| entry.get("path").and_then(|p| p.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether the asset protocol may play a sound file outside the app's own
/// folder: an absolute path with no `..`, a sound extension, not under
/// `app_data`. It does not matter whether the file is there now; a drive that is
/// unplugged today is back tomorrow.
fn is_legacy_sound_path(path: &str, app_data: &Path) -> bool {
    let path = Path::new(path);
    path.is_absolute()
        && !path.components().any(|c| matches!(c, Component::ParentDir))
        && sound_extension(path).is_some()
        && !path.starts_with(app_data)
}

/// The paths that pass [`is_legacy_sound_path`], once each, in order.
fn select_legacy_sounds(paths: &[String], app_data: &Path) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for path in paths {
        if is_legacy_sound_path(path, app_data) && !kept.contains(path) {
            kept.push(path.clone());
        }
    }
    kept
}

fn write_allow_list(dir: &Path, paths: &[String]) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(paths).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("{ALLOW_FILE}.tmp"));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, dir.join(ALLOW_FILE))
}

/// The saved list, checked again against the same rule.
fn read_allow_list(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join(ALLOW_FILE)) else {
        return Vec::new();
    };
    let Ok(list) = serde_json::from_str::<Vec<String>>(&text) else {
        return Vec::new();
    };
    select_legacy_sounds(&list, dir)
}

/// Chooses the legacy sound files from `settings` the first time it runs in
/// `dir`, and never again. Answers whether it chose them now.
fn freeze_in(dir: &Path, settings: &Settings) -> bool {
    let marker = dir.join(FROZEN_MARKER);
    if marker.exists() {
        return false;
    }
    let keep = select_legacy_sounds(&custom_sound_paths(settings), dir);
    // The list goes first: a launch that dies between the two writes chooses
    // again, where the other order would lose the sounds for good.
    if let Err(e) = write_allow_list(dir, &keep) {
        log::warn!("[Sounds] could not save the legacy sound list ({e}); trying again next launch");
        return false;
    }
    if let Err(e) = std::fs::write(&marker, b"1") {
        log::warn!("[Sounds] could not mark the legacy sounds as chosen ({e})");
    }
    true
}

/// Keeps the sound files that settings from before imports name, once, so the
/// page can still play them. Called with the settings as they were read from
/// the file, never with defaults: a read that failed says nothing about which
/// sounds a user had, and the next launch asks again.
pub fn freeze_legacy_sounds(settings: &Settings) {
    if let Ok(dir) = crate::services::cache_service::get_app_data_dir() {
        freeze_in(&dir, settings);
    }
}

/// The legacy sound files the asset protocol may play.
pub fn legacy_sound_paths() -> Vec<PathBuf> {
    crate::services::cache_service::get_app_data_dir()
        .map(|dir| read_allow_list(&dir).into_iter().map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn drop_from_list(dir: &Path, path: &str) -> std::io::Result<()> {
    let mut list = read_allow_list(dir);
    let before = list.len();
    list.retain(|p| Path::new(p) != Path::new(path));
    if list.len() == before {
        return Ok(());
    }
    write_allow_list(dir, &list)
}

/// A legacy sound that was deleted: its file leaves the scope now and the list,
/// so the next launch does not allow it again.
pub fn forget_legacy_sound(app: &AppHandle, path: &str) {
    let Ok(dir) = crate::services::cache_service::get_app_data_dir() else {
        return;
    };
    if !is_legacy_sound_path(path, &dir) {
        return;
    }
    if let Err(e) = drop_from_list(&dir, path) {
        log::warn!("[Sounds] could not update the legacy sound list ({e})");
    }
    crate::services::asset_scope::forbid_file(app, Path::new(path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_supported_sounds_and_refuses_the_rest() {
        let tmp = std::env::temp_dir().join(format!("sn-sounds-{}", uuid::Uuid::new_v4().simple()));
        let src_dir = tmp.join("src");
        let dest_dir = tmp.join("dest");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&dest_dir).unwrap();

        let ok = src_dir.join("Ding Dong.MP3");
        std::fs::write(&ok, b"ID3 not really audio").unwrap();
        let got = import_into(&ok, &dest_dir).unwrap();
        assert_eq!(got.name, "Ding Dong");
        assert!(got.path.ends_with(".mp3"));
        assert!(Path::new(&got.path).exists());

        let txt = src_dir.join("notes.txt");
        std::fs::write(&txt, b"hi").unwrap();
        assert!(import_into(&txt, &dest_dir).is_err());

        let big = src_dir.join("song.wav");
        std::fs::write(&big, vec![0u8; (MAX_BYTES + 1) as usize]).unwrap();
        assert!(import_into(&big, &dest_dir).is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A folder of its own under the system temp folder.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sn-sounds-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn settings_with<S: AsRef<str>>(paths: &[S]) -> Settings {
        let sounds: Vec<serde_json::Value> = paths
            .iter()
            .enumerate()
            .map(|(i, p)| serde_json::json!({ "id": format!("file:{i}"), "name": format!("Sound {i}"), "path": p.as_ref() }))
            .collect();
        let mut settings = Settings::default();
        settings
            .extra
            .insert("chat_highlights".into(), serde_json::json!({ "custom_sounds": sounds }));
        settings
    }

    #[test]
    fn only_sound_files_outside_the_app_folder_are_kept_whether_or_not_they_exist() {
        let app = std::env::temp_dir().join("sn-app-data");
        let elsewhere = std::env::temp_dir().join("sn-elsewhere");
        let gone = text(&elsewhere.join("gone.MP3"));
        let flac = text(&elsewhere.join("ding.flac"));
        let paths = vec![
            gone.clone(),
            text(&elsewhere.join("notes.txt")),
            text(&elsewhere.join("settings.json")),
            text(&elsewhere.join("..").join("secret").join("x.mp3")),
            "sounds/relative.mp3".to_string(),
            "ding.wav".to_string(),
            String::new(),
            text(&app.join("sounds").join("inside.mp3")),
            gone.clone(),
            flac.clone(),
        ];
        assert_eq!(select_legacy_sounds(&paths, &app), vec![gone, flac]);
    }

    #[test]
    fn the_settings_entries_are_read_from_the_custom_sounds_list() {
        assert!(custom_sound_paths(&Settings::default()).is_empty());
        let mut settings = settings_with(&["/a/one.mp3", "/a/two.wav"]);
        assert_eq!(custom_sound_paths(&settings), vec!["/a/one.mp3", "/a/two.wav"]);
        settings.extra.insert(
            "chat_highlights".into(),
            serde_json::json!({ "custom_sounds": [{ "id": "file:x", "name": "No path" }, { "path": 7 }, { "path": "/a/ok.ogg" }] }),
        );
        assert_eq!(custom_sound_paths(&settings), vec!["/a/ok.ogg"]);
    }

    #[test]
    fn the_legacy_sounds_are_chosen_once_from_the_settings_it_is_given() {
        let app = scratch("freeze");
        let elsewhere = std::env::temp_dir().join("sn-elsewhere");
        let first = text(&elsewhere.join("one.wav"));
        let second = text(&elsewhere.join("two.ogg"));
        let junk = text(&elsewhere.join("notes.txt"));

        assert!(freeze_in(&app, &settings_with(&[&first, &junk])));
        assert_eq!(read_allow_list(&app), vec![first.clone()]);
        assert!(app.join(FROZEN_MARKER).exists());

        // Settings read on a later launch are not looked at again.
        assert!(!freeze_in(&app, &settings_with(&[&first, &second])));
        assert_eq!(read_allow_list(&app), vec![first]);

        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn a_list_that_cannot_be_written_chooses_nothing_and_asks_again_next_launch() {
        let missing = std::env::temp_dir().join(format!("sn-sounds-missing-{}", uuid::Uuid::new_v4().simple()));
        let one = text(&std::env::temp_dir().join("sn-elsewhere").join("one.wav"));
        assert!(!freeze_in(&missing, &settings_with(&[&one])));
        assert!(!missing.join(FROZEN_MARKER).exists());
    }

    #[test]
    fn a_saved_list_is_checked_again_when_it_is_read() {
        let app = scratch("tamper");
        let elsewhere = std::env::temp_dir().join("sn-elsewhere");
        let good = text(&elsewhere.join("good.mp3"));
        let list = vec![good.clone(), text(&elsewhere.join("settings.json")), "relative.mp3".to_string()];
        std::fs::write(app.join(ALLOW_FILE), serde_json::to_vec(&list).unwrap()).unwrap();
        assert_eq!(read_allow_list(&app), vec![good]);
        std::fs::write(app.join(ALLOW_FILE), b"not json").unwrap();
        assert!(read_allow_list(&app).is_empty());
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn a_forgotten_sound_leaves_the_list_and_the_rest_stay() {
        let app = scratch("forget");
        let elsewhere = std::env::temp_dir().join("sn-elsewhere");
        let a = text(&elsewhere.join("a.mp3"));
        let b = text(&elsewhere.join("b.mp3"));
        assert!(freeze_in(&app, &settings_with(&[&a, &b])));

        drop_from_list(&app, &a).unwrap();
        assert_eq!(read_allow_list(&app), vec![b.clone()]);
        // A path that was never on the list changes nothing.
        drop_from_list(&app, &text(&elsewhere.join("c.mp3"))).unwrap();
        assert_eq!(read_allow_list(&app), vec![b]);

        let _ = std::fs::remove_dir_all(&app);
    }
}
