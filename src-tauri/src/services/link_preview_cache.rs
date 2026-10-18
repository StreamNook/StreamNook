//! Link previews for chat rows: which links in a row expand, and their answers.
//!
//! `commands::link_preview` knows how to read one URL. This keeps what it read
//! (bounded, so a busy chat cannot grow it), runs one fetch per URL at a time
//! however many rows or windows ask, and caps how many run at once, so a burst
//! of history rows cannot open dozens of sockets. Only trusted hosts expand on
//! their own: an arbitrary pasted link is never fetched passively, which would
//! show the viewer's IP to whoever posted it.
//!
//! Native shells get previews on the row (`RowView.previews`); a preview not
//! known yet when the row is stamped arrives later as a `ROW_PREVIEW` frame for
//! every room, keyed by URL, so every row carrying that link fills at once.

use crate::commands::link_preview::{build_preview, LinkPreview};
use crate::models::chat_layout::{RowPreview, ViewSpan};
use futures::future::{BoxFuture, FutureExt, Shared};
use lru::LruCache;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// How many answers are kept. A chat repeats a handful of links; this is far
/// more than a session needs and small in memory.
const CAP: usize = 256;
/// A good answer stays this long (Discord counts and titles drift slowly).
const KEEP_OK: Duration = Duration::from_secs(30 * 60);
/// A URL with no preview is not asked again for this long.
const KEEP_NONE: Duration = Duration::from_secs(5 * 60);
/// Fetches running at once across every room and window.
const SLOTS: usize = 4;
/// Links per row that expand, as desktop shows them.
pub const PER_ROW: usize = 2;

/// Registrable domains whose links expand on their own. A host matches when it
/// equals one or is a subdomain of it. Desktop's `linkPreviewService.ts` keeps
/// the same list for its own rows (a parity test holds them together).
pub const TRUSTED_BASE_DOMAINS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "twitch.tv",
    "twitter.com",
    "x.com",
    "imgur.com",
    "giphy.com",
    "gph.is",
    "tenor.com",
    "reddit.com",
    "redd.it",
    "github.com",
    "streamable.com",
    "kick.com",
    "spotify.com",
    "soundcloud.com",
    "tiktok.com",
    "discord.gg",
    "discord.com",
    "discordapp.com",
    "steampowered.com",
    "steamcommunity.com",
    "instagram.com",
    "nuuls.com",
    "catbox.moe",
    "uguu.se",
    "bsky.app",
    "vimeo.com",
    "bandcamp.com",
];

struct Entry {
    at: Instant,
    preview: Option<LinkPreview>,
}

static CACHE: LazyLock<Mutex<LruCache<String, Entry>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(CAP).expect("cap"))));

type Fetch = Shared<BoxFuture<'static, Option<LinkPreview>>>;
static INFLIGHT: LazyLock<Mutex<HashMap<String, Fetch>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static RUNNING: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(SLOTS));
/// URLs whose `ROW_PREVIEW` frame is on its way, so a link pasted by twenty
/// chatters posts one frame.
static WANTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// What is known for `url`: `Some(Some(_))` a preview, `Some(None)` known to
/// have none, `None` not asked (or the answer went stale).
pub fn peek(url: &str) -> Option<Option<LinkPreview>> {
    let mut cache = CACHE.lock().ok()?;
    let entry = cache.get(url)?;
    let keep = if entry.preview.is_some() { KEEP_OK } else { KEEP_NONE };
    if entry.at.elapsed() > keep {
        cache.pop(url);
        return None;
    }
    Some(entry.preview.clone())
}

fn store(url: &str, preview: Option<LinkPreview>) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.put(url.to_string(), Entry { at: Instant::now(), preview });
    }
}

/// Seeds an answer, for fixtures made without the network.
#[cfg(test)]
pub(crate) fn remember_for_test(url: &str, preview: LinkPreview) {
    store(url, Some(preview));
}

/// The preview for `url`, read once however many callers ask at the same time.
pub async fn get(url: &str) -> Option<LinkPreview> {
    if let Some(known) = peek(url) {
        return known;
    }
    let fetch = {
        let Ok(mut inflight) = INFLIGHT.lock() else { return None };
        inflight
            .entry(url.to_string())
            .or_insert_with(|| {
                let url = url.to_string();
                async move {
                    let got = {
                        let _slot = RUNNING.acquire().await.ok();
                        build_preview(&url).await.ok()
                    };
                    store(&url, got.clone());
                    if let Ok(mut inflight) = INFLIGHT.lock() {
                        inflight.remove(&url);
                    }
                    got
                }
                .boxed()
                .shared()
            })
            .clone()
    };
    fetch.await
}

/// Starts reading `url` for rows already sent, and posts the answer to every
/// room as `ROW_PREVIEW`.
fn want(url: &str) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let first = WANTED.lock().map(|mut w| w.insert(url.to_string())).unwrap_or(false);
    if !first {
        return;
    }
    let url = url.to_string();
    tokio::spawn(async move {
        let got = get(&url).await;
        if let Ok(mut w) = WANTED.lock() {
            w.remove(&url);
        }
        crate::services::irc_service::post_frame_to_all(preview_frame(&url, got.as_ref())).await;
    });
}

/// The frame that fills (or, with no preview, drops) the card for `url` on
/// every row that carries it.
pub fn preview_frame(url: &str, preview: Option<&LinkPreview>) -> String {
    serde_json::json!({ "type": "ROW_PREVIEW", "url": url, "preview": preview }).to_string()
}

fn host_matches(host: &str, bases: &[impl AsRef<str>]) -> bool {
    let host = host.to_lowercase();
    bases.iter().any(|base| {
        let base = base.as_ref().trim().to_lowercase();
        !base.is_empty() && (host == base || host.ends_with(&format!(".{base}")))
    })
}

/// Whether links to `host` expand on their own: the built-in list or a host
/// the viewer chose to trust (`chat_design.link_preview_trusted_domains`).
pub fn is_trusted(host: &str, user_trusted: &[String]) -> bool {
    host_matches(host, TRUSTED_BASE_DOMAINS) || host_matches(host, user_trusted)
}

/// The link as fetched: a bare `www.` link gains its scheme.
fn absolute(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}

/// Which links in a row's body expand: trusted ones, in order, deduped, at
/// most `PER_ROW`. A link with a recent answer of "no preview" is left out.
pub fn row_urls(spans: &[ViewSpan], user_trusted: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for span in spans {
        let ViewSpan::Link { url, .. } = span else { continue };
        let url = absolute(url);
        let Ok(parsed) = url::Url::parse(&url) else { continue };
        if !matches!(parsed.scheme(), "http" | "https") {
            continue;
        }
        let Some(host) = parsed.host_str() else { continue };
        if !is_trusted(host, user_trusted) || out.contains(&url) {
            continue;
        }
        out.push(url);
        if out.len() == PER_ROW {
            break;
        }
    }
    out
}

/// The cards a row carries as it is stamped. A link already read carries its
/// preview; one not read yet starts its read and carries `pending`, with
/// `reserve` set when its shape almost always has a card (a Discord invite, a
/// clip, a video), so the shell holds the card's room and nothing jumps when
/// the answer lands.
pub fn row_previews(spans: &[ViewSpan], user_trusted: &[String], keep_link: bool) -> Vec<RowPreview> {
    row_urls(spans, user_trusted)
        .into_iter()
        .filter_map(|url| match peek(&url) {
            Some(None) => None,
            Some(Some(preview)) => Some(RowPreview { url, preview: Some(preview), reserve: false, keep_link }),
            None => {
                want(&url);
                let reserve = crate::commands::link_preview::has_dedicated_card(&url);
                Some(RowPreview { url, preview: None, reserve, keep_link })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str) -> ViewSpan {
        ViewSpan::Link { label: url.to_string(), url: url.to_string() }
    }

    #[test]
    fn only_trusted_links_expand_two_at_most_in_order() {
        let spans = vec![
            link("https://evil.example/grab"),
            link("https://discord.gg/abc"),
            ViewSpan::Text { text: " and ".into() },
            link("https://discord.gg/abc"),
            link("www.youtube.com/watch?v=dQw4w9WgXcQ"),
            link("https://clips.twitch.tv/Some"),
        ];
        assert_eq!(
            row_urls(&spans, &[]),
            vec!["https://discord.gg/abc".to_string(), "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string()]
        );
    }

    #[test]
    fn a_host_the_viewer_trusts_expands_with_its_subdomains() {
        let spans = vec![link("https://blog.example.org/post")];
        assert!(row_urls(&spans, &[]).is_empty());
        assert_eq!(row_urls(&spans, &["example.org".to_string()]).len(), 1);
        // A lookalike is not a subdomain.
        assert!(!is_trusted("notdiscord.gg", &[]));
        assert!(is_trusted("ptb.discord.com", &[]));
    }

    #[test]
    fn answers_are_kept_and_a_known_miss_is_left_off_the_row() {
        let hit = "https://discord.gg/kept-test";
        let miss = "https://discord.gg/miss-test";
        store(hit, Some(LinkPreview { url: hit.into(), kind: "discord".into(), ..Default::default() }));
        store(miss, None);
        let rows = row_previews(&[link(hit), link(miss)], &[], false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].preview.as_ref().map(|p| p.kind.as_str()), Some("discord"));
        assert!(!rows[0].reserve);
    }

    #[test]
    fn a_link_not_read_yet_is_pending_and_reserved_when_it_has_a_card() {
        let rows = row_previews(&[link("https://discord.gg/pending-test"), link("https://github.com/x/y")], &[], true);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.preview.is_none() && r.keep_link));
        assert!(rows[0].reserve, "a Discord invite always has a card");
        assert!(!rows[1].reserve, "a generic page may have none");
    }

    #[test]
    fn the_frame_names_the_url_and_carries_null_for_no_preview() {
        let frame: serde_json::Value = serde_json::from_str(&preview_frame("https://x.com/a", None)).unwrap();
        assert_eq!(frame["type"], "ROW_PREVIEW");
        assert_eq!(frame["url"], "https://x.com/a");
        assert!(frame["preview"].is_null());
    }

    #[tokio::test]
    async fn concurrent_asks_share_one_answer() {
        let url = "https://discord.gg/shared-test";
        store(url, Some(LinkPreview { url: url.into(), kind: "discord".into(), ..Default::default() }));
        let (a, b) = tokio::join!(get(url), get(url));
        assert_eq!(a, b);
        assert!(a.is_some());
    }
}
