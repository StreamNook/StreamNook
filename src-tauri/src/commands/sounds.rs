//! Custom notification sounds. A picked file is copied into the app's own
//! `sounds` folder, so the sound keeps playing after the original is moved,
//! renamed or on a drive that is unplugged.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Larger than any notification sound needs; keeps a stray music file out.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const EXTENSIONS: [&str; 9] = ["mp3", "wav", "ogg", "oga", "opus", "m4a", "aac", "flac", "webm"];

#[derive(Debug, Serialize)]
pub struct ImportedSound {
    pub id: String,
    pub name: String,
    pub path: String,
}

fn sounds_dir() -> Result<PathBuf, String> {
    let dir = crate::services::cache_service::get_app_data_dir()
        .map_err(|e| e.to_string())?
        .join("sounds");
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
}
