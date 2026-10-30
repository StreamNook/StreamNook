//! What the asset protocol may serve, to the page and to any other webview of
//! the app: the app's own picture and sound folders, one entry per folder, and
//! the sound files a user pointed the settings at before imports existed. Never
//! a state file (the settings, the upload list, the cache manifest, a token, the
//! cache server's port and token) and never a path that the page or remote data
//! names.
//!
//! The folders are listed twice on purpose. `tauri.conf.json` and
//! `tauri.linux.conf.json` name them statically under `$LOCALDATA/StreamNook`,
//! the debug build's `$LOCALDATA/com.streamnook.dev` and the phone's
//! `$APPDATA/StreamNook`, so no request can reach the scope before setup runs.
//! [`allow_app_assets`] adds the folders as this code resolves them, which holds
//! where a static entry cannot (a data folder with glob characters in its name,
//! a folder the platform moves), plus the frozen legacy sounds, which sit in no
//! folder of ours.

use std::path::{Path, PathBuf};

use crate::rt::AppHandle;
use crate::services::{cache_service, image_upload_store, universal_cache_service};

/// The universal cache folders the page draws pictures from: one per
/// `CacheType`, the same five `cache_file_server` serves.
const CACHE_DIRS: [&str; 5] = [
    "badges",
    "emotes",
    "third-party-badges",
    "cosmetics",
    "streamnook-art",
];

/// Every folder the asset protocol serves, as the app's own resolvers name
/// them: the five cache folders, the kept copy and the small picture of each
/// upload, and the imported sounds. A folder that does not exist yet is listed
/// all the same; the resolvers create the few they need.
pub fn asset_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Ok(universal) = universal_cache_service::get_universal_cache_dir() {
        folders.extend(CACHE_DIRS.iter().map(|dir| universal.join(dir)));
    }
    // The kept files and their small pictures sit side by side; each resolver
    // answers a file's path, so its parent is the folder.
    for file in [
        image_upload_store::copy_path("x"),
        image_upload_store::thumb_path("x"),
    ] {
        if let Some(folder) = file.as_deref().and_then(Path::parent) {
            folders.push(folder.to_path_buf());
        }
    }
    if let Ok(app_data) = cache_service::get_app_data_dir() {
        folders.push(app_data.join(crate::commands::sounds::FOLDER));
    }
    folders
}

/// Allows the app's asset folders and the frozen legacy sounds. Folders are
/// allowed without their subfolders: every file the page asks for sits directly
/// in one of them.
#[cfg(not(target_os = "ios"))]
pub fn allow_app_assets(app: &AppHandle) {
    use tauri::Manager as _;
    let scope = app.asset_protocol_scope();
    for folder in asset_folders() {
        if let Err(e) = scope.allow_directory(&folder, false) {
            log::warn!("[AssetScope] could not allow {}: {e}", folder.display());
        }
    }
    allow_legacy_sounds(app);
}

/// Allows the frozen legacy sound files. A phone chooses them after it has
/// allowed its folders, so this is a call of its own.
#[cfg(not(target_os = "ios"))]
pub fn allow_legacy_sounds(app: &AppHandle) {
    use tauri::Manager as _;
    let scope = app.asset_protocol_scope();
    for file in crate::commands::sounds::legacy_sound_paths() {
        if let Err(e) = scope.allow_file(&file) {
            log::warn!("[AssetScope] could not allow {}: {e}", file.display());
        }
    }
}

/// Takes one file out of the scope for the rest of this run, whatever allowed it.
#[cfg(not(target_os = "ios"))]
pub fn forbid_file(app: &AppHandle, path: &Path) {
    use tauri::Manager as _;
    if let Err(e) = app.asset_protocol_scope().forbid_file(path) {
        log::warn!("[AssetScope] could not forbid {}: {e}", path.display());
    }
}

/// The iPhone app has no webview and no asset protocol.
#[cfg(target_os = "ios")]
pub fn forbid_file(_app: &AppHandle, _path: &Path) {}

#[cfg(all(test, not(target_os = "ios")))]
mod tests {
    use super::*;
    use serde_json::Value;

    const BASE_CONFIG: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"));
    const LINUX_CONFIG: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tauri.linux.conf.json"
    ));

    /// The folders the configs name the scope under: the release data folder,
    /// the debug build's, and the phone's.
    const ROOTS: [&str; 3] = [
        "$LOCALDATA/StreamNook",
        "$LOCALDATA/com.streamnook.dev",
        "$APPDATA/StreamNook",
    ];

    /// Files the page must never be able to read, relative to the data folder.
    const STATE_FILES: [&str; 14] = [
        "settings.json",
        "image_uploads/uploads.json",
        "cache/universal/manifest.json",
        "cache_server_port",
        "cache_server_token",
        "legacy_sound_allow.json",
        "legacy_sounds_frozen",
        "cookies.json",
        ".token_vault_key",
        ".twitch_token",
        ".kick_token",
        ".seventv_token",
        ".twitch_drops_token",
        ".youtube_session",
    ];

    fn scope_of(config: &str) -> Value {
        let config: Value = serde_json::from_str(config).expect("the config is JSON");
        config["app"]["security"]["assetProtocol"]["scope"].clone()
    }

    fn entries(scope: &Value, key: &str) -> Vec<String> {
        scope[key]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|e| e.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The entries under `root`, with the root cut off, sorted.
    fn under(root: &str, entries: &[String]) -> Vec<String> {
        let prefix = format!("{root}/");
        let mut got: Vec<String> = entries
            .iter()
            .filter_map(|e| e.strip_prefix(&prefix).map(str::to_string))
            .collect();
        got.sort();
        got
    }

    /// Whether an entry (`<folder>/*` or `<folder>/.*`, relative to a root)
    /// names a file (relative to the same root). The scope matches with literal
    /// separators and literal leading dots, so `*` covers the direct children of
    /// its folder that do not start with a dot, and `.*` the ones that do.
    fn covers(entry: &str, file: &str) -> bool {
        let (dir, pattern) = entry.rsplit_once('/').unwrap_or(("", entry));
        let (file_dir, name) = file.rsplit_once('/').unwrap_or(("", file));
        match pattern {
            "*" => dir == file_dir && !name.starts_with('.'),
            ".*" => dir == file_dir && name.starts_with('.'),
            other => panic!("unexpected scope pattern {other}"),
        }
    }

    /// An asset folder relative to the data folder, with `/` separators.
    fn relative(folder: &Path, app_data: &Path) -> String {
        let rest = folder
            .strip_prefix(app_data)
            .unwrap_or_else(|_| panic!("{folder:?} is outside {app_data:?}"));
        rest.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn neither_config_has_a_double_star_and_both_scopes_are_per_folder() {
        for (name, config) in [
            ("tauri.conf.json", BASE_CONFIG),
            ("tauri.linux.conf.json", LINUX_CONFIG),
        ] {
            assert!(!config.contains("**"), "{name} has a ** in it");
            let scope = scope_of(config);
            assert!(scope.is_object(), "{name}: the scope is the object form");
            assert_eq!(
                scope["requireLiteralLeadingDot"],
                Value::Bool(true),
                "{name}"
            );
            let allow = entries(&scope, "allow");
            assert!(!allow.is_empty(), "{name}: nothing is allowed");
            for entry in &allow {
                assert!(
                    ROOTS.iter().any(|r| entry.starts_with(&format!("{r}/"))),
                    "{name}: {entry} is under no root"
                );
                let folder = entry
                    .strip_suffix("/*")
                    .unwrap_or_else(|| panic!("{name}: {entry} is not a folder entry"));
                assert!(
                    !folder.contains(['*', '?', '[', ']', '{', '}']),
                    "{name}: {entry} has glob characters in the folder"
                );
            }
        }
    }

    #[test]
    fn the_linux_scope_is_the_base_scope() {
        assert_eq!(scope_of(LINUX_CONFIG), scope_of(BASE_CONFIG));
    }

    #[test]
    fn every_root_lists_the_same_folders_and_the_same_denies() {
        let scope = scope_of(BASE_CONFIG);
        let (allow, deny) = (entries(&scope, "allow"), entries(&scope, "deny"));
        let first_allow = under(ROOTS[0], &allow);
        let first_deny = under(ROOTS[0], &deny);
        assert_eq!(
            first_allow.len(),
            8,
            "five cache folders, two upload folders, sounds"
        );
        for root in ROOTS {
            assert_eq!(under(root, &allow), first_allow, "{root}");
            assert_eq!(under(root, &deny), first_deny, "{root}");
        }
        assert_eq!(
            allow.len(),
            ROOTS.len() * first_allow.len(),
            "an allow entry sits outside the roots"
        );
        assert_eq!(
            deny.len(),
            ROOTS.len() * first_deny.len(),
            "a deny entry sits outside the roots"
        );
    }

    #[test]
    fn the_allowed_folders_are_exactly_the_folders_the_resolvers_name() {
        let app_data = cache_service::get_app_data_dir().expect("a data folder");
        let name = app_data
            .file_name()
            .and_then(|n| n.to_str())
            .expect("a folder name")
            .to_string();
        let root = format!("$LOCALDATA/{name}");
        assert!(
            ROOTS.contains(&root.as_str()),
            "{root} is not a root the config names"
        );

        let allow = under(&root, &entries(&scope_of(BASE_CONFIG), "allow"));
        let folders = asset_folders();
        assert_eq!(folders.len(), 8, "{folders:?}");
        let mut want: Vec<String> = folders
            .iter()
            .map(|f| format!("{}/*", relative(f, &app_data)))
            .collect();
        want.sort();
        assert_eq!(allow, want);

        // Each entry, as the glob the scope builds from it, names its folder's files.
        for entry in &want {
            let mut pattern_path = app_data.clone();
            let mut file = app_data.clone();
            for part in entry.split('/') {
                pattern_path.push(part);
                file.push(if part == "*" { "a.png" } else { part });
            }
            let pattern =
                tauri::scope::fs::Pattern::new(&pattern_path.to_string_lossy()).expect("a glob");
            assert!(
                pattern.matches_path(&file),
                "{entry} misses its folder's files"
            );
        }
    }

    #[test]
    fn state_files_are_not_allowed_and_are_denied() {
        let scope = scope_of(BASE_CONFIG);
        for root in ROOTS {
            let allow = under(root, &entries(&scope, "allow"));
            let deny = under(root, &entries(&scope, "deny"));
            for file in STATE_FILES {
                assert!(
                    !allow.iter().any(|e| covers(e, file)),
                    "{root}: {file} is allowed"
                );
                assert!(
                    deny.iter().any(|e| covers(e, file)),
                    "{root}: {file} is not denied"
                );
            }
        }
    }

    #[test]
    fn no_deny_entry_hides_an_asset_folder() {
        let scope = scope_of(BASE_CONFIG);
        let deny = under(ROOTS[0], &entries(&scope, "deny"));
        for entry in under(ROOTS[0], &entries(&scope, "allow")) {
            let folder = entry.strip_suffix("/*").expect("a folder entry");
            let file = format!("{folder}/a.png");
            assert!(
                !deny.iter().any(|d| covers(d, &file)),
                "a deny entry covers {file}"
            );
        }
    }

    #[test]
    fn the_cache_folders_match_the_cache_types() {
        let src = include_str!("universal_cache_service.rs");
        for dir in CACHE_DIRS {
            assert!(
                src.contains(&format!("=> \"{dir}\"")),
                "{dir} is a CacheType folder"
            );
        }
        // `CacheType::dir_name` is private, so its arms are counted in the source.
        let arms = src
            .lines()
            .filter(|l| l.trim_start().starts_with("CacheType::") && l.contains("=> \""))
            .count();
        assert_eq!(
            arms,
            CACHE_DIRS.len(),
            "every CacheType folder is listed here"
        );
    }
}
