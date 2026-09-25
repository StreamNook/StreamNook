//! Release notes for the changelog popup, fetched, cached and parsed here so the
//! webview only draws them.
//!
//! The source is GitHub's release list, whose bodies are the `CHANGELOG.md`
//! sections the release workflow extracts: an optional
//! `## <emoji> New: <headline>` + `> <blurb>` lead, then `### <section>` groups
//! of `- **Title**: description` bullets. Images, callouts and loose paragraphs
//! keep their place in document order.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const RELEASES_URL: &str = "https://api.github.com/repos/StreamNook/StreamNook/releases?per_page=20";
const ANDROID_MANIFEST_URL: &str = "https://streamnook.app/api/v1/update-android";
const CACHE_FILE: &str = "changelog_releases.json";

// ---------------------------------------------------------------------------
// Parsed notes
// ---------------------------------------------------------------------------

/// Which glyph a section gets, decided from its heading's words so the older
/// "Added / Fixed / Changed" releases get the same marks as the emoji ones.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    Fixes,
    Performance,
    Maintenance,
    Removed,
    Plugins,
    Interface,
    Changes,
    Features,
    Other,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct NoteItem {
    pub title: String,
    pub desc: String,
    /// A bullet with no bold title: the whole line is the change.
    pub plain: bool,
}

/// One render node. Consecutive bullets collapse into ONE section, so they
/// share a single box with hairlines between them.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum NoteNode {
    Hero {
        title: String,
        desc: String,
    },
    Image {
        url: String,
        alt: String,
    },
    Note {
        desc: String,
    },
    Section {
        label: Option<String>,
        kind: SectionKind,
        intro: Vec<String>,
        items: Vec<NoteItem>,
    },
}

#[derive(Serialize, Clone, Debug)]
pub struct ChangelogRelease {
    /// Without a leading "v".
    pub version: String,
    /// RFC 3339 from GitHub, or a bare `YYYY-MM-DD` from the offline fallback.
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub notes: Vec<NoteNode>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Changelog {
    /// Newest first.
    pub releases: Vec<ChangelogRelease>,
}

static VERSION_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^#{0,3}\s*\[.*?\]\s*-\s*\d{4}-\d{2}-\d{2}").unwrap());
static IMAGE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^!\[(.*?)\]\((.+?)\)\s*$").unwrap());
static LEADING_JUNK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[^A-Za-z0-9]+").unwrap());
static HERO_NEW_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^[^A-Za-z0-9]*\bNew:\s*").unwrap());
static BULLET_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[-*]\s+").unwrap());
static BOLD_ITEM_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\*\*(.+?)\*\*\s*[:—-]?\s*(.*)$").unwrap());
static CALLOUT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\[!\w+\]$").unwrap());
static BOILERPLATE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(#{1,4}\s*(installation|install|bundle components|downloads?|how to (install|update))\b|installation$)",
    )
    .unwrap()
});

fn section_kind(label: &str) -> SectionKind {
    let l = label.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
    if has(&["bug", "fix"]) {
        SectionKind::Fixes
    } else if has(&["perf", "latency", "speed", "faster"]) {
        SectionKind::Performance
    } else if has(&["mainten", "chore", "housekeep", "internal"]) {
        SectionKind::Maintenance
    } else if has(&["remov", "deprecat"]) {
        SectionKind::Removed
    } else if has(&["plugin"]) {
        SectionKind::Plugins
    } else if has(&["interface", "design", "look", "theme"]) {
        SectionKind::Interface
    } else if has(&["chang", "improv", "update"]) {
        SectionKind::Changes
    } else if has(&["feature", "new", "add"]) {
        SectionKind::Features
    } else {
        SectionKind::Other
    }
}

#[derive(Default)]
struct Parser {
    nodes: Vec<NoteNode>,
    hero: Option<(String, Vec<String>)>,
    /// Index into `nodes` of the section still taking bullets.
    section: Option<usize>,
    /// A blockquote spans several source lines; a bare ">" splits paragraphs.
    quote: Option<Vec<Vec<String>>>,
}

impl Parser {
    fn flush_hero(&mut self) {
        if let Some((title, desc)) = self.hero.take() {
            let desc = desc.join(" ").trim().to_string();
            self.nodes.push(NoteNode::Hero { title, desc });
        }
    }

    fn push_note(&mut self, desc: String) {
        if desc.is_empty() {
            return;
        }
        // Text straight under a section heading introduces that section; text
        // after its bullets ends it.
        if let Some(i) = self.section {
            if let Some(NoteNode::Section { intro, items, .. }) = self.nodes.get_mut(i) {
                if items.is_empty() {
                    intro.push(desc);
                    return;
                }
            }
        }
        self.section = None;
        self.nodes.push(NoteNode::Note { desc });
    }

    fn flush_quote(&mut self) {
        let Some(quote) = self.quote.take() else { return };
        let paragraphs: Vec<String> = quote
            .into_iter()
            .map(|p| p.join(" ").trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if let Some((_, desc)) = self.hero.as_mut() {
            desc.extend(paragraphs);
            return;
        }
        for p in paragraphs {
            self.push_note(p);
        }
    }

    fn open_section(&mut self, label: Option<String>) -> usize {
        let kind = label.as_deref().map(section_kind).unwrap_or(SectionKind::Other);
        self.nodes.push(NoteNode::Section { label, kind, intro: Vec::new(), items: Vec::new() });
        let i = self.nodes.len() - 1;
        self.section = Some(i);
        i
    }

    fn line(&mut self, raw: &str) {
        let line = raw.trim();

        if let Some(rest) = line.strip_prefix('>') {
            let t = rest.trim();
            let quote = self.quote.get_or_insert_with(|| vec![Vec::new()]);
            // GitHub callout markers ("[!NOTE]") carry no words of their own.
            if CALLOUT_RE.is_match(t) {
                return;
            }
            if t.is_empty() {
                if quote.last().is_some_and(|p| !p.is_empty()) {
                    quote.push(Vec::new());
                }
                return;
            }
            if let Some(p) = quote.last_mut() {
                p.push(t.to_string());
            }
            return;
        }
        self.flush_quote();
        if line.is_empty() {
            return;
        }

        // Version/date line: the popup header already shows the version.
        if VERSION_LINE_RE.is_match(line) || line == "---" {
            self.flush_hero();
            return;
        }
        if let Some(c) = IMAGE_RE.captures(line) {
            self.flush_hero();
            self.section = None;
            self.nodes.push(NoteNode::Image { alt: c[1].to_string(), url: c[2].to_string() });
            return;
        }
        if let Some(rest) = line.strip_prefix("###") {
            if rest.starts_with(char::is_whitespace) {
                self.flush_hero();
                let label = LEADING_JUNK_RE.replace(rest.trim(), "").trim().to_string();
                self.open_section(Some(label));
                return;
            }
        }
        // Headline (## 🎉 New: ...) becomes the lead; strip the emoji + "New:".
        if let Some(rest) = line.strip_prefix("##") {
            if rest.starts_with(char::is_whitespace) {
                self.flush_hero();
                self.section = None;
                let title = HERO_NEW_RE.replace(rest.trim(), "");
                let title = LEADING_JUNK_RE.replace(&title, "").trim().to_string();
                self.hero = Some((title, Vec::new()));
                return;
            }
        }
        if BULLET_RE.is_match(line) {
            self.flush_hero();
            let text = BULLET_RE.replace(line, "");
            let item = match BOLD_ITEM_RE.captures(&text) {
                Some(c) => NoteItem { title: c[1].to_string(), desc: c[2].to_string(), plain: false },
                None => NoteItem { title: text.to_string(), desc: String::new(), plain: true },
            };
            let i = match self.section {
                Some(i) => i,
                None => self.open_section(None),
            };
            if let Some(NoteNode::Section { items, .. }) = self.nodes.get_mut(i) {
                items.push(item);
            }
            return;
        }
        self.flush_hero();
        self.push_note(line.to_string());
    }
}

/// Parse one release body into render nodes, dropping the download and
/// installation boilerplate release_manager.ps1 appends.
pub fn parse_notes(content: &str) -> Vec<NoteNode> {
    let mut p = Parser::default();
    for raw in content.lines() {
        if BOILERPLATE_RE.is_match(raw.trim()) {
            break;
        }
        p.line(raw);
    }
    p.flush_quote();
    p.flush_hero();
    p.nodes
}

// ---------------------------------------------------------------------------
// Fetch + cache
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The raw list, not the parsed one, so a parser change applies to what is
/// already cached.
#[derive(Serialize, Deserialize, Default)]
struct DiskCache {
    etag: Option<String>,
    releases: Vec<GithubRelease>,
}

fn cache_path() -> Option<PathBuf> {
    crate::services::cache_service::get_cache_dir().ok().map(|d| d.join(CACHE_FILE))
}

fn read_cache() -> Option<DiskCache> {
    let bytes = std::fs::read(cache_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_cache(cache: &DiskCache) {
    let Some(path) = cache_path() else { return };
    match serde_json::to_vec(cache) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                log::warn!("[Changelog] could not write the release cache: {e}");
            }
        }
        Err(e) => log::warn!("[Changelog] could not serialise the release cache: {e}"),
    }
}

/// GitHub's conditional request: with the cached ETag it answers 304 when
/// nothing changed, and a 304 does not count against the 60/hour
/// unauthenticated limit, so checking on every open is free.
async fn fetch_releases(cached: Option<&DiskCache>) -> Result<Option<DiskCache>, String> {
    let mut req = crate::services::http::client()
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "StreamNook");
    if let Some(etag) = cached.and_then(|c| c.etag.as_deref()) {
        req = req.header("If-None-Match", etag);
    }
    let res = req.send().await.map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(format!("GitHub answered HTTP {}", res.status()));
    }
    let etag = res
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let releases: Vec<GithubRelease> =
        res.json().await.map_err(|e| format!("Could not read GitHub's release list: {e}"))?;
    Ok(Some(DiskCache {
        etag,
        releases: releases.into_iter().filter(|r| !r.draft).collect(),
    }))
}

fn normalize(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

fn build(cache: &DiskCache) -> Changelog {
    Changelog {
        releases: cache
            .releases
            .iter()
            .map(|r| ChangelogRelease {
                version: normalize(&r.tag_name),
                published_at: r.published_at.clone(),
                prerelease: r.prerelease,
                notes: parse_notes(r.body.as_deref().unwrap_or_default()),
            })
            .collect(),
    }
}

/// The release list, newest first. Falls back to the cached list when GitHub
/// cannot be reached, and to `fallback_version`'s section of CHANGELOG.md when
/// there is no cache either.
pub async fn load(fallback_version: Option<String>) -> Result<Changelog, String> {
    let cached = read_cache();
    match fetch_releases(cached.as_ref()).await {
        Ok(Some(fresh)) => {
            write_cache(&fresh);
            return Ok(build(&fresh));
        }
        Ok(None) => {
            if let Some(c) = &cached {
                return Ok(build(c));
            }
        }
        Err(e) => {
            log::warn!("[Changelog] {e}");
            if let Some(c) = cached.as_ref().filter(|c| !c.releases.is_empty()) {
                return Ok(build(c));
            }
        }
    }

    let notes = crate::commands::settings::get_release_notes(fallback_version).await?;
    Ok(Changelog {
        releases: vec![ChangelogRelease {
            version: normalize(&notes.version),
            published_at: Some(notes.published_at).filter(|d| !d.is_empty()),
            prerelease: false,
            notes: parse_notes(&notes.body),
        }],
    })
}

/// How many published releases sit after `current`, up to and including
/// `latest`, from the cached list. None when the cache is cold: an honest
/// "there is an update" beats a made-up count. The list holds the newest 20,
/// so a very old client gets the floor, which is why the copy says "at least".
pub fn releases_behind(current: &str, latest: &str) -> Option<u32> {
    use crate::commands::components::parse_version;
    let (cur, lat) = (parse_version(current)?, parse_version(latest)?);
    let cache = read_cache()?;
    let n = cache
        .releases
        .iter()
        .filter(|r| !r.prerelease)
        .filter_map(|r| parse_version(&r.tag_name))
        .filter(|v| cmp(v, &cur).is_gt() && cmp(v, &lat).is_le())
        .count() as u32;
    (n > 0).then_some(n)
}

fn cmp(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    (0..len)
        .map(|i| a.get(i).copied().unwrap_or(0).cmp(&b.get(i).copied().unwrap_or(0)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

// ---------------------------------------------------------------------------
// Android
// ---------------------------------------------------------------------------

/// What an Android change is about, for its icon. A small set with an honest
/// fallback: a confidently wrong icon reads worse than a neutral one.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeTopic {
    Audio,
    Power,
    Playback,
    Profile,
    Link,
    Chat,
    Notifications,
    Fix,
    Build,
    Other,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AndroidChange {
    pub title: String,
    pub body: String,
    pub topic: ChangeTopic,
}

#[derive(Serialize, Clone, Debug)]
pub struct AndroidRelease {
    pub version: String,
    pub published_at: Option<String>,
    pub changes: Vec<AndroidChange>,
}

#[derive(Deserialize)]
struct AndroidManifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
}

static SENTENCE_RE: Lazy<Regex> =
    // The whitespace after the stop is load-bearing: without it "0.1.9" and
    // "e.g." split a sentence in half.
    Lazy::new(|| Regex::new(r"(?s)^(.+?[.!?])\s+(.+)$").unwrap());
static PARAGRAPH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n\s*\n").unwrap());

fn topic(title: &str) -> ChangeTopic {
    let t = title.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    if has(&["audio", "listen", "lock", "media control", "background", "picture-in-picture"]) {
        ChangeTopic::Audio
    } else if has(&["batter", "cooler", "hot", "power", "data", "performance", "quality"]) {
        ChangeTopic::Power
    } else if has(&["pause", "play", "stream", "video", "buffer", "latency"]) {
        ChangeTopic::Playback
    } else if has(&["profile", "badge", "paint", "identity", "avatar", "cosmetic"]) {
        ChangeTopic::Profile
    } else if has(&["link", "clip"]) {
        ChangeTopic::Link
    } else if has(&["chat", "reply", "message", "emote"]) {
        ChangeTopic::Chat
    } else if has(&["notification", "alert"]) {
        ChangeTopic::Notifications
    } else if has(&["fix", "bug", "no longer", "stuck", "crash"]) {
        ChangeTopic::Fix
    } else if has(&["emulator", "build", "install", "update"]) {
        ChangeTopic::Build
    } else {
        ChangeTopic::Other
    }
}

/// Every paragraph in the Android manifest is a headline sentence then the
/// detail, so splitting on the first stop gives a real title and body.
pub fn parse_android_changes(notes: &str) -> Vec<AndroidChange> {
    PARAGRAPH_RE
        .split(notes)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|para| {
            let (title, body) = match SENTENCE_RE.captures(para) {
                Some(c) => (c[1].trim_end_matches('.').to_string(), c[2].trim().to_string()),
                None => (para.trim_end_matches('.').to_string(), String::new()),
            };
            AndroidChange { topic: topic(&title), title, body }
        })
        .collect()
}

/// The Android release notes. None is the manifest's documented "nothing
/// published yet" state (HTTP 503).
pub async fn load_android() -> Result<Option<AndroidRelease>, String> {
    let res = crate::services::http::client()
        .get(ANDROID_MANIFEST_URL)
        .send()
        .await
        .map_err(|e| format!("Could not reach the update server: {e}"))?;
    if !res.status().is_success() {
        return Ok(None);
    }
    let m: AndroidManifest =
        res.json().await.map_err(|e| format!("Could not read the Android manifest: {e}"))?;
    Ok(Some(AndroidRelease {
        version: m.version,
        published_at: m.published_at,
        changes: m.notes.as_deref().map(parse_android_changes).unwrap_or_default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, desc: &str, plain: bool) -> NoteItem {
        NoteItem { title: title.into(), desc: desc.into(), plain }
    }

    #[test]
    fn groups_consecutive_bullets_under_their_heading() {
        let nodes = parse_notes(
            "## [8.6.4] - 2026-09-18\n\n## 🎉 New: a fresh look\n> Softer surfaces and a\n> floating strip.\n\n---\n\n### ✨ Features\n- **A new look.** Softer surfaces.\n- **Notifications.** In the title bar.\n\n### 🔧 Maintenance\n- Builds run in parallel.",
        );
        assert_eq!(
            nodes,
            vec![
                NoteNode::Hero {
                    title: "a fresh look".into(),
                    desc: "Softer surfaces and a floating strip.".into()
                },
                NoteNode::Section {
                    label: Some("Features".into()),
                    kind: SectionKind::Features,
                    intro: vec![],
                    items: vec![
                        item("A new look.", "Softer surfaces.", false),
                        item("Notifications.", "In the title bar.", false),
                    ],
                },
                NoteNode::Section {
                    label: Some("Maintenance".into()),
                    kind: SectionKind::Maintenance,
                    intro: vec![],
                    items: vec![item("Builds run in parallel.", "", true)],
                },
            ]
        );
    }

    #[test]
    fn joins_a_multi_line_callout_and_drops_the_marker() {
        let nodes = parse_notes("> [!NOTE]\n> **Two releases** in\n> one day.\n>\n> Second paragraph.");
        assert_eq!(
            nodes,
            vec![
                NoteNode::Note { desc: "**Two releases** in one day.".into() },
                NoteNode::Note { desc: "Second paragraph.".into() },
            ]
        );
    }

    #[test]
    fn text_under_a_heading_is_its_intro_and_text_after_bullets_is_not() {
        let nodes = parse_notes("### Plugins\nNow opt-in.\n- **Autopilot**: farms points.\nClosing words.");
        assert_eq!(
            nodes,
            vec![
                NoteNode::Section {
                    label: Some("Plugins".into()),
                    kind: SectionKind::Plugins,
                    intro: vec!["Now opt-in.".into()],
                    items: vec![item("Autopilot", "farms points.", false)],
                },
                NoteNode::Note { desc: "Closing words.".into() },
            ]
        );
    }

    #[test]
    fn stops_at_the_installation_boilerplate() {
        assert_eq!(parse_notes("- **Fix**: works.\n### Installation\n- Grab the 7z.").len(), 1);
    }

    #[test]
    fn keeps_a_lead_image_in_place() {
        let nodes = parse_notes("![Chat grows up](https://x/y.webp)\n## 🎉 New: Chat grows up\n> Blurb.");
        assert_eq!(
            nodes[0],
            NoteNode::Image { url: "https://x/y.webp".into(), alt: "Chat grows up".into() }
        );
        assert!(matches!(&nodes[1], NoteNode::Hero { title, .. } if title == "Chat grows up"));
    }

    #[test]
    fn section_kinds_cover_old_and_new_headings() {
        assert_eq!(section_kind("Bug Fixes"), SectionKind::Fixes);
        assert_eq!(section_kind("Fixed"), SectionKind::Fixes);
        assert_eq!(section_kind("Added"), SectionKind::Features);
        assert_eq!(section_kind("Changed"), SectionKind::Changes);
        assert_eq!(section_kind("Also from us"), SectionKind::Other);
    }

    #[test]
    fn android_paragraphs_split_on_the_first_real_stop() {
        let changes = parse_android_changes(
            "Much easier on the battery. Quality now matches the screen.\n\nFixed 0.1.9 crash on resume",
        );
        assert_eq!(changes[0].title, "Much easier on the battery");
        assert_eq!(changes[0].body, "Quality now matches the screen.");
        assert_eq!(changes[0].topic, ChangeTopic::Power);
        assert_eq!(changes[1].title, "Fixed 0.1.9 crash on resume");
        assert_eq!(changes[1].topic, ChangeTopic::Fix);
    }
}
