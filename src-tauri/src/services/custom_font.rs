//! The Custom interface font on the phones: a family name the viewer typed,
//! read from Google Fonts and kept on disk.
//!
//! Desktop draws a typed family through a web stylesheet. A phone app wants
//! the font files themselves: the iPhone registers them with CoreText, and the
//! Android page loads them as FontFaces. So the css2 API is asked as a plain
//! client (it then answers with whole TrueType files, one per weight rather
//! than browser subsets), each face is downloaded once, and the set is kept
//! under the cache dir with a small index. A later launch reads the index and
//! never touches the network, so the font holds offline. A family Google does
//! not know gives no font at all, and the app falls back to its own quietly.

use log::{info, warn};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// The weights the interface draws. A family answers with the ones it has.
const WEIGHTS: &str = "300;400;500;600;700";
/// A single face larger than this is not a font anyone meant.
const FACE_CAP: usize = 6 * 1024 * 1024;
const INDEX: &str = "faces.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomFace {
    pub weight: u16,
    /// Where the face is kept on disk.
    pub path: String,
    /// The file itself, base64, when the caller asked for it (a web page that
    /// cannot read the path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomFont {
    /// The family as typed and cleaned, which is how it is asked for.
    pub family: String,
    pub faces: Vec<CustomFace>,
}

#[derive(Serialize, Deserialize)]
struct Index {
    family: String,
    faces: Vec<IndexFace>,
}

#[derive(Serialize, Deserialize)]
struct IndexFace {
    weight: u16,
    file: String,
}

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    // A plain client name, not a browser's: css2 then serves whole TrueType
    // files, which CoreText and FontFace both take.
    reqwest::Client::builder()
        .user_agent(concat!("StreamNook/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
        .expect("custom font client")
});

/// Families Google did not know this run, so typing past one does not ask again.
static MISSES: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
/// One fetch at a time: a typed name settles on one family, and two reads of
/// the same family must not write its folder at once.
static FETCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What a typed name keeps: no quotes, commas, semicolons, braces, parens or
/// backslashes, trimmed, at most 64 characters. The same rule as desktop's
/// `sanitizeFontFamily`.
pub fn sanitize(raw: &str) -> String {
    let kept: String = raw.chars().filter(|c| !"\"',;{}()\\".contains(*c)).collect();
    kept.trim().chars().take(64).collect()
}

/// A folder name for a family: lowercase letters, digits and dashes.
fn slug(family: &str) -> String {
    let mut out = String::new();
    for c in family.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn fonts_dir() -> Option<PathBuf> {
    let base = crate::services::app_paths::cache_base()
        .or_else(|| crate::services::twitch_service::get_app_data_dir().ok().map(|d| d.join("cache")))?;
    Some(base.join("fonts"))
}

/// The weight and file of each TrueType or OpenType face in a css2 answer.
fn parse_css(css: &str) -> Vec<(u16, String)> {
    static BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@font-face\s*\{([^}]*)\}").expect("block"));
    static WEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"font-weight:\s*(\d+)").expect("weight"));
    static SRC: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"src:\s*url\(([^)]+)\)\s*format\('(?:truetype|opentype)'\)").expect("src")
    });
    let mut faces: Vec<(u16, String)> = Vec::new();
    for block in BLOCK.captures_iter(css) {
        let body = &block[1];
        let (Some(weight), Some(src)) = (WEIGHT.captures(body), SRC.captures(body)) else { continue };
        let Ok(weight) = weight[1].parse::<u16>() else { continue };
        let url = src[1].trim_matches(|c| c == '\'' || c == '"').to_string();
        if url.starts_with("https://") && !faces.iter().any(|(w, _)| *w == weight) {
            faces.push((weight, url));
        }
    }
    faces.sort_by_key(|(w, _)| *w);
    faces
}

fn css2_url(family: &str, weights: bool) -> String {
    let name = urlencoding::encode(family).replace("%20", "+");
    if weights {
        format!("https://fonts.googleapis.com/css2?family={name}:wght@{WEIGHTS}")
    } else {
        format!("https://fonts.googleapis.com/css2?family={name}")
    }
}

/// The kept set for `family`, when every face it lists is still on disk.
fn read_index(folder: &Path, family: &str) -> Option<CustomFont> {
    let index: Index = serde_json::from_slice(&std::fs::read(folder.join(INDEX)).ok()?).ok()?;
    if !index.family.eq_ignore_ascii_case(family) || index.faces.is_empty() {
        return None;
    }
    let faces = index
        .faces
        .iter()
        .map(|f| {
            let path = folder.join(&f.file);
            path.is_file().then(|| CustomFace { weight: f.weight, path: path.to_string_lossy().into_owned(), data: None })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(CustomFont { family: index.family, faces })
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

/// Why a family has no font: Google does not know it, or Google (or the
/// phone's connection) did not answer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Miss {
    NotFound,
    Unreachable,
}

/// What a typed family came to, as the phones show it: drawn, no such
/// Google Font, or not reachable right now (asked again next time).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Lookup {
    Ready { font: CustomFont },
    NotFound,
    Unreachable,
}

impl From<Result<CustomFont, Miss>> for Lookup {
    fn from(got: Result<CustomFont, Miss>) -> Self {
        match got {
            Ok(font) => Lookup::Ready { font },
            Err(Miss::NotFound) => Lookup::NotFound,
            Err(Miss::Unreachable) => Lookup::Unreachable,
        }
    }
}

async fn fetch_css(family: &str) -> Result<String, Miss> {
    for weights in [true, false] {
        let response = CLIENT.get(css2_url(family, weights)).send().await.map_err(|e| {
            warn!("[CustomFont] {family}: Google Fonts did not answer: {e}");
            Miss::Unreachable
        })?;
        let status = response.status();
        if status.is_success() {
            return response.text().await.map_err(|_| Miss::Unreachable);
        }
        // 400 means Google does not know the family (or, with weights, has
        // none of them); without weights it answers the family's own.
        if status != reqwest::StatusCode::BAD_REQUEST {
            warn!("[CustomFont] {family}: Google Fonts answered {status}");
            return Err(Miss::Unreachable);
        }
    }
    Err(Miss::NotFound)
}

async fn download(folder: &Path, family: &str) -> Result<CustomFont, Miss> {
    let css = fetch_css(family).await?;
    let found = parse_css(&css);
    if found.is_empty() {
        return Err(Miss::NotFound);
    }
    std::fs::create_dir_all(folder).map_err(|_| Miss::Unreachable)?;
    let mut faces = Vec::new();
    let mut index = Index { family: family.to_string(), faces: Vec::new() };
    for (weight, url) in found {
        let Ok(response) = CLIENT.get(&url).send().await else { continue };
        if !response.status().is_success() {
            continue;
        }
        let Ok(bytes) = response.bytes().await else { continue };
        if bytes.is_empty() || bytes.len() > FACE_CAP {
            continue;
        }
        let file = format!("{weight}.ttf");
        let path = folder.join(&file);
        if let Err(e) = write_atomic(&path, &bytes) {
            warn!("[CustomFont] could not keep {family} {weight}: {e}");
            continue;
        }
        index.faces.push(IndexFace { weight, file });
        faces.push(CustomFace { weight, path: path.to_string_lossy().into_owned(), data: None });
    }
    if faces.is_empty() {
        return Err(Miss::Unreachable);
    }
    // The index last, so a half-written set is never read as whole.
    let written = serde_json::to_vec(&index).ok().map(|json| write_atomic(&folder.join(INDEX), &json));
    if !matches!(written, Some(Ok(()))) {
        warn!("[CustomFont] could not keep the index for {family}");
    }
    info!("[CustomFont] {family}: downloaded {} faces", faces.len());
    Ok(CustomFont { family: family.to_string(), faces })
}

/// The faces of a typed family: from disk when kept, else read once and kept.
pub async fn resolve(raw: &str) -> Lookup {
    let family = sanitize(raw);
    if family.is_empty() {
        return Lookup::NotFound;
    }
    let Some(folder) = fonts_dir().map(|d| d.join(slug(&family))) else {
        return Lookup::Unreachable;
    };
    if let Some(kept) = read_index(&folder, &family) {
        info!("[CustomFont] {family}: kept on disk ({} faces)", kept.faces.len());
        return Lookup::Ready { font: kept };
    }
    let key = family.to_lowercase();
    if MISSES.lock().map(|m| m.contains(&key)).unwrap_or(false) {
        return Lookup::NotFound;
    }
    let _one = FETCH.lock().await;
    if let Some(kept) = read_index(&folder, &family) {
        return Lookup::Ready { font: kept };
    }
    let got = download(&folder, &family).await;
    // Only a family Google does not know is remembered; an unreachable one is
    // asked again next time.
    if got == Err(Miss::NotFound) {
        info!("[CustomFont] {family}: no such Google Font");
        if let Ok(mut misses) = MISSES.lock() {
            misses.insert(key);
        }
    }
    got.into()
}

/// The same with each face's file inlined, for the Android page.
pub async fn resolve_with_data(raw: &str) -> Lookup {
    use base64::Engine;
    let mut font = match resolve(raw).await {
        Lookup::Ready { font } => font,
        other => return other,
    };
    for face in &mut font.faces {
        let Ok(bytes) = std::fs::read(&face.path) else { return Lookup::Unreachable };
        face.data = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
    }
    Lookup::Ready { font }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_names_its_state() {
        let ready = serde_json::to_value(Lookup::Ready {
            font: CustomFont { family: "Poppins".into(), faces: vec![CustomFace { weight: 400, path: "/f/400.ttf".into(), data: None }] },
        })
        .unwrap();
        assert_eq!(ready["state"], "ready");
        assert_eq!(ready["font"]["faces"][0]["weight"], 400);
        assert_eq!(serde_json::to_value(Lookup::NotFound).unwrap(), serde_json::json!({ "state": "not_found" }));
        assert_eq!(serde_json::to_value(Lookup::Unreachable).unwrap(), serde_json::json!({ "state": "unreachable" }));
    }

    #[test]
    fn a_typed_name_keeps_only_a_plain_family() {
        assert_eq!(sanitize("  Poppins  "), "Poppins");
        assert_eq!(sanitize("\"Bebas Neue\";} body{"), "Bebas Neue body");
        assert_eq!(sanitize(&"x".repeat(80)).len(), 64);
    }

    #[test]
    fn folders_are_plain_slugs() {
        assert_eq!(slug("Playfair Display"), "playfair-display");
        assert_eq!(slug("JetBrains Mono!"), "jetbrains-mono");
        assert_eq!(slug("../etc"), "etc");
    }

    #[test]
    fn css2_answers_give_one_truetype_face_per_weight() {
        let css = "@font-face {\n  font-family: 'Poppins';\n  font-style: normal;\n  font-weight: 700;\n  src: url(https://fonts.gstatic.com/s/poppins/v24/b.ttf) format('truetype');\n}\n\
                   @font-face {\n  font-family: 'Poppins';\n  font-weight: 400;\n  src: url(https://fonts.gstatic.com/s/poppins/v24/a.ttf) format('truetype');\n}\n\
                   @font-face {\n  font-family: 'Poppins';\n  font-weight: 500;\n  src: url(https://fonts.gstatic.com/s/poppins/v24/c.woff2) format('woff2');\n}";
        assert_eq!(
            parse_css(css),
            vec![
                (400, "https://fonts.gstatic.com/s/poppins/v24/a.ttf".to_string()),
                (700, "https://fonts.gstatic.com/s/poppins/v24/b.ttf".to_string()),
            ]
        );
        assert!(parse_css("/* Font family not found */").is_empty());
    }

    #[test]
    fn the_request_names_the_family_and_the_weights() {
        assert_eq!(
            css2_url("Bebas Neue", true),
            "https://fonts.googleapis.com/css2?family=Bebas+Neue:wght@300;400;500;600;700"
        );
        assert_eq!(css2_url("Lato", false), "https://fonts.googleapis.com/css2?family=Lato");
    }

    #[test]
    fn a_kept_set_is_read_back_only_while_every_face_is_on_disk() {
        let folder = std::env::temp_dir().join(format!("sn-custom-font-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("400.ttf"), b"face").unwrap();
        let index = Index { family: "Poppins".into(), faces: vec![IndexFace { weight: 400, file: "400.ttf".into() }] };
        std::fs::write(folder.join(INDEX), serde_json::to_vec(&index).unwrap()).unwrap();

        let kept = read_index(&folder, "poppins").expect("kept");
        assert_eq!(kept.family, "Poppins");
        assert_eq!(kept.faces.len(), 1);
        assert!(read_index(&folder, "Lato").is_none(), "another family's folder is not this one");

        std::fs::remove_file(folder.join("400.ttf")).unwrap();
        assert!(read_index(&folder, "Poppins").is_none(), "a missing face means read it again");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
