//! Category links: reading one, building one, and asking Twitch what a
//! category id names.
//!
//! | Form | Shape                                                    |
//! |------|----------------------------------------------------------|
//! | Web  | `https://streamnook.app/c/<id>[/<slug>][?d=MMDD]`        |
//! | App  | `streamnook:///category/<id>` (empty host)               |
//! | App  | `streamnook://category/<id>` (host form, also read)      |
//!
//! The id is a Twitch game id (digits, 1 to 15). The slug is decoration: it is
//! never used to route, and the page ignores it. Released apps read the empty
//! host form as nothing, which is why the site writes that one.
//!
//! This file sends nothing to Twitch itself. The one anonymous read of a game
//! lives in `favorite_categories`, and the signed-in fallback goes through the
//! Twitch service, so the manifest of Twitch calls stays complete.

use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::favorite_categories;
use super::share_link::SHARE_ORIGIN;
use super::twitch_service::TwitchService;

/// The only provider a category link names today. A link that names another
/// one is refused, which keeps the words free for later.
const PROVIDER: &str = "twitch";

/// How long the anonymous read gets before the signed-in fallback is asked.
/// It only decides when to ask the second source; it never ends the resolve.
const FIRST_READ_BOUND: Duration = Duration::from_millis(2500);

/// A category a link points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryLink {
    pub provider: &'static str,
    pub id: String,
}

/// The decoded, non-empty path segments of a URL.
fn path_segments(url: &url::Url) -> Vec<String> {
    url.path_segments()
        .map(|segments| {
            segments
                .filter(|s| !s.is_empty())
                .map(|s| {
                    urlencoding::decode(s)
                        .map(|d| d.into_owned())
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Read a category link back into the category it names.
///
/// Accepts `streamnook:///category/<id>[/...]`, `streamnook://category/<id>[/...]`
/// and `https://streamnook.app/c/<id>[/<slug>]` (also `www.` and `http`). The
/// first segment after the kind must be a Twitch game id; a provider word such
/// as `kick` is refused. The bare host form `streamnook://category` names no
/// category and is not read here.
pub fn parse(url: &url::Url) -> Option<CategoryLink> {
    let mut segments = path_segments(url);
    match url.scheme() {
        "streamnook" => match url.host_str().unwrap_or("") {
            "" => {
                if !segments
                    .first()
                    .is_some_and(|s| s.eq_ignore_ascii_case("category"))
                {
                    return None;
                }
                segments.remove(0);
            }
            host if host.eq_ignore_ascii_case("category") => {}
            _ => return None,
        },
        "https" | "http" => {
            let host = url.host_str().unwrap_or("");
            if host != "streamnook.app" && host != "www.streamnook.app" {
                return None;
            }
            if segments.first().map(String::as_str) != Some("c") {
                return None;
            }
            segments.remove(0);
        }
        _ => return None,
    }
    let id = segments.first()?;
    favorite_categories::valid_category_id(id).then(|| CategoryLink {
        provider: PROVIDER,
        id: id.clone(),
    })
}

/// Runs of consecutive code points that fold to one ASCII letter, as
/// `(first code point, letters)`. A `.` inside a run is a code point that does
/// not fold. The runs are exactly the characters in U+00C0 to U+FFFF whose
/// canonical decomposition (NFD), after the combining marks U+0300 to U+036F
/// are removed, is one ASCII Latin letter. A letter that does not decompose
/// (O with stroke, sharp s, L with stroke, D with stroke, AE, OE) is
/// deliberately absent: it is a separator,
/// as it is on the site. The letter is kept in its original case here and
/// lowercased after the lookup, so the dotted capital I gives `i`.
const FOLD_RUNS: &[(u32, &str)] = &[
    (0x00C0, "AAAAAA.CEEEEIIII.NOOOOO..UUUUY..aaaaaa.ceeeeiiii.nooooo..uuuuy.yAaAaAaCcCcCcCcDd..EeEeEeEeEeGgGgGgGgHh..IiIiIiIiI...JjKk.LlLlLl....NnNnNn...OoOoOo..RrRrRrSsSsSsSsTtTt..UuUuUuUuUuUuWwYyYZzZzZz"),
    (0x01A0, "Oo"),
    (0x01AF, "Uu"),
    (0x01CD, "AaIiOoUuUuUuUuUu.AaAa....GgKkOoOo..j...Gg..NnAa....AaAaEeEeIiIiOoOoRrRrUuUuSsTt..Hh"),
    (0x0226, "AaEeOoOoOoOoYy"),
    (0x1E00, "AaBbBbBbCcDdDdDdDdDdEeEeEeEeEeFfGgHhHhHhHhHhIiIiKkKkKkLlLlLlLlMmMmMmNnNnNnNnOoOoOoOoPpPpRrRrRrRrSsSsSsSsSsTtTtTtTtUuUuUuUuUuVvVvWwWwWwWwWwXxXxYyZzZzZzhtwy"),
    (0x1EA0, "AaAaAaAaAaAaAaAaAaAaAaAaEeEeEeEeEeEeEeEeIiIiOoOoOoOoOoOoOoOoOoOoOoOoUuUuUuUuUuUuUuYyYyYyYy"),
    (0x212A, "KA"),
];

/// The ASCII letter a non-ASCII character folds to, in its original case.
fn fold_letter(c: char) -> Option<char> {
    let code = c as u32;
    for &(start, letters) in FOLD_RUNS {
        if code < start {
            return None;
        }
        if let Some(&byte) = letters.as_bytes().get((code - start) as usize) {
            return (byte != b'.').then_some(byte as char);
        }
    }
    None
}

/// The decoration at the end of a share link: the name as lowercase words
/// joined by `-`. Accents and other marks fold to their base letter, the
/// combining marks U+0300 to U+036F vanish (so a decomposed "Pokemon" with an accent reads
/// the same as the composed one), and everything else, which is every script
/// but Latin plus the letters that do not decompose, separates words. Empty
/// when nothing is left. The cap is 80 characters, with a trailing `-` the cap
/// leaves trimmed again.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in name.chars() {
        if ('\u{300}'..='\u{36F}').contains(&c) {
            continue;
        }
        let base = if c.is_ascii() {
            Some(c)
        } else {
            fold_letter(c)
        };
        match base
            .map(|b| b.to_ascii_lowercase())
            .filter(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            Some(letter) => {
                if gap && !out.is_empty() {
                    out.push('-');
                }
                gap = false;
                out.push(letter);
            }
            None => gap = true,
        }
    }
    out.truncate(80);
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// The web link to copy or share. `day` is the MMDD tag the stream share links
/// carry (Discord caches a preview per exact URL); the page ignores it.
pub fn share_url(id: &str, name: &str, day: &str) -> String {
    let words = slug(name);
    let tail = if words.is_empty() {
        String::new()
    } else {
        format!("/{words}")
    };
    format!("{SHARE_ORIGIN}/c/{id}{tail}?d={day}")
}

/// The app link the landing page hands off to.
pub fn deep_link(id: &str) -> String {
    format!("streamnook:///category/{id}")
}

/// `share_url` for an id the page would answer: an id that is not a Twitch
/// game id builds no link at all.
pub fn share_link_for(id: &str, name: &str, day: &str) -> Result<String, String> {
    if !favorite_categories::valid_category_id(id) {
        return Err("not a Twitch category id".into());
    }
    Ok(share_url(id, name, day))
}

/// What Twitch says a category id is. The wire shape is
/// `{"state":"found","id":"509658","name":"Just Chatting","box_art_url":"...","viewer_count":123}`,
/// `{"state":"missing"}` or `{"state":"unreachable"}`: the field names match
/// the `TwitchCategory` the page already holds, and `viewer_count` is null when
/// the second source answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CategoryAnswer {
    Found {
        id: String,
        name: String,
        box_art_url: String,
        viewer_count: Option<u64>,
    },
    /// Twitch answered and has no such category. The only way to get this.
    Missing,
    /// No answer to trust: offline, rate limited, signed out with the first
    /// read failing, or a body of a shape nobody expected.
    Unreachable,
}

/// A text field of a game, empty when it is absent or not text.
fn text<'a>(game: &'a Value, key: &str) -> &'a str {
    game.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Read the anonymous game answer (`data.g0`). `Missing` only when `g0` is
/// present and null; a body with no `data`, no `g0`, an error or a rate-limit
/// message is `Unreachable`, never `Missing`.
pub fn answer_from(body: &Value) -> CategoryAnswer {
    let game = body.get("data").and_then(|data| data.get("g0"));
    if game == Some(&Value::Null) {
        return CategoryAnswer::Missing;
    }
    let Some(game) = game else {
        return CategoryAnswer::Unreachable;
    };
    let (id, name) = (text(game, "id"), text(game, "name"));
    if id.is_empty() || name.is_empty() {
        return CategoryAnswer::Unreachable;
    }
    CategoryAnswer::Found {
        id: id.to_string(),
        name: name.to_string(),
        box_art_url: text(game, "boxArtURL").to_string(),
        viewer_count: game.get("viewersCount").and_then(Value::as_u64),
    }
}

/// Read the signed-in fallback's rows. A first row with a name is `Found`
/// with no viewer count; an empty list is `Unreachable`, never `Missing`,
/// because an empty list can also mean a refused request.
pub fn answer_from_helix(rows: &[Value]) -> CategoryAnswer {
    let Some(game) = rows.first() else {
        return CategoryAnswer::Unreachable;
    };
    let (id, name) = (text(game, "id"), text(game, "name"));
    if id.is_empty() || name.is_empty() {
        return CategoryAnswer::Unreachable;
    }
    CategoryAnswer::Found {
        id: id.to_string(),
        name: name.to_string(),
        box_art_url: text(game, "box_art_url").to_string(),
        viewer_count: None,
    }
}

/// The resolve with both reads injected, so tests drive it with fake futures
/// and a paused clock.
///
/// `first` is the anonymous read. It gets `FIRST_READ_BOUND`; an error, a
/// timeout, or an answer that is neither `Found` nor `Missing` (a rate-limit
/// body, a body with no `data`) all hand over to `second`, the signed-in
/// fallback, which is not polled otherwise. There is no outer bound: the
/// fallback runs under the shared client's own timeout, and the page decides
/// how long a person waits. A failed fallback is `Unreachable`.
pub(crate) async fn resolve_with<F, S>(first: F, second: S) -> CategoryAnswer
where
    F: Future<Output = anyhow::Result<Value>>,
    S: Future<Output = anyhow::Result<Vec<Value>>>,
{
    if let Ok(Ok(body)) = tokio::time::timeout(FIRST_READ_BOUND, first).await {
        match answer_from(&body) {
            CategoryAnswer::Unreachable => {}
            settled => return settled,
        }
    }
    match second.await {
        Ok(rows) => answer_from_helix(&rows),
        Err(_) => CategoryAnswer::Unreachable,
    }
}

/// The fallback read: Twitch's own games list, only with a sign-in. Signed
/// out, the token read fails from the stored file and nothing is sent.
async fn helix_games(id: &str) -> anyhow::Result<Vec<Value>> {
    TwitchService::get_token().await?;
    TwitchService::get_games_by_ids(&[id.to_string()]).await
}

/// What a category id names. An id that is not a Twitch game id is
/// `Unreachable` with no network at all.
pub async fn resolve(id: &str) -> CategoryAnswer {
    if !favorite_categories::valid_category_id(id) {
        return CategoryAnswer::Unreachable;
    }
    resolve_with(favorite_categories::read_one(id), helix_games(id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};

    const LINK_FIXTURE: &str = include_str!("../../tests/fixtures/inbound_link.json");
    const SLUG_FIXTURE: &str = include_str!("../../tests/fixtures/slug_fold.json");

    fn read(link: &str) -> Option<CategoryLink> {
        parse(&url::Url::parse(link).expect("test url parses"))
    }

    fn category(id: &str) -> Option<CategoryLink> {
        Some(CategoryLink {
            provider: "twitch",
            id: id.to_string(),
        })
    }

    fn fixture() -> Value {
        serde_json::from_str(LINK_FIXTURE).expect("the link fixture is JSON")
    }

    /// The source above the test module, with LF line ends on any checkout.
    fn live_source() -> String {
        let all = include_str!("category_link.rs").replace("\r\n", "\n");
        all.split("#[cfg(test)]")
            .next()
            .expect("the file has a live part")
            .to_string()
    }

    fn body_of(src: &str, signature: &str) -> String {
        let start = src
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` is in the source"));
        let rest = &src[start..];
        let end = rest
            .find("\n}\n")
            .unwrap_or_else(|| panic!("`{signature}` ends"));
        rest[..end].to_string()
    }

    fn in_order(body: &str, needles: &[&str]) {
        let mut at = 0;
        for needle in needles {
            let found = body[at..]
                .find(needle)
                .unwrap_or_else(|| panic!("`{needle}` is missing or out of order"));
            at += found + needle.len();
        }
    }

    fn gql_found() -> Value {
        json!({ "data": { "g0": {
            "id": "509658", "name": "Just Chatting",
            "boxArtURL": "https://static-cdn.jtvnw.net/ttv-boxart/509658-285x380.jpg",
            "viewersCount": 243261
        } } })
    }

    fn helix_row() -> Value {
        json!({
            "id": "509658", "name": "Just Chatting",
            "box_art_url": "https://static-cdn.jtvnw.net/ttv-boxart/509658-{width}x{height}.jpg"
        })
    }

    fn found_from_gql() -> CategoryAnswer {
        CategoryAnswer::Found {
            id: "509658".into(),
            name: "Just Chatting".into(),
            box_art_url: "https://static-cdn.jtvnw.net/ttv-boxart/509658-285x380.jpg".into(),
            viewer_count: Some(243261),
        }
    }

    fn found_from_helix() -> CategoryAnswer {
        CategoryAnswer::Found {
            id: "509658".into(),
            name: "Just Chatting".into(),
            box_art_url: "https://static-cdn.jtvnw.net/ttv-boxart/509658-{width}x{height}.jpg"
                .into(),
            viewer_count: None,
        }
    }

    fn rate_limited() -> Value {
        json!({ "error": "Too Many Requests", "status": 429, "message": "rate limited" })
    }

    async fn first_after(delay: Duration, answer: anyhow::Result<Value>) -> anyhow::Result<Value> {
        tokio::time::sleep(delay).await;
        answer
    }

    async fn second_after(
        delay: Duration,
        answer: anyhow::Result<Vec<Value>>,
    ) -> anyhow::Result<Vec<Value>> {
        tokio::time::sleep(delay).await;
        answer
    }

    /// A resolve that never ends fails the test instead of hanging it: the
    /// paused clock would otherwise wait forever.
    async fn within_a_minute<T>(work: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(60), work)
            .await
            .expect("the resolve ended inside a minute of paused time")
    }

    // parse

    #[test]
    fn the_hostless_link_the_site_writes_opens_a_category() {
        // Fails if the empty-host form is refused: it is the form the landing page hands off to.
        assert_eq!(read("streamnook:///category/509658"), category("509658"));
        assert_eq!(read("streamnook:/category/1"), category("1"));
        assert_eq!(read("streamnook:///category/509658/"), category("509658"));
        assert_eq!(
            read("streamnook:///Category/509658/just-chatting"),
            category("509658")
        );
        assert_eq!(read("streamnook:///category/%35"), category("5"));
    }

    #[test]
    fn the_host_form_and_the_web_link_open_a_category_too() {
        // Fails if either form is dropped: a host-form link would open channel "category".
        assert_eq!(read("streamnook://category/509658"), category("509658"));
        assert_eq!(
            read("streamnook://CATEGORY/509658/extra"),
            category("509658")
        );
        assert_eq!(
            read("https://streamnook.app/c/509658/just-chatting?d=1114"),
            category("509658")
        );
        assert_eq!(
            read("https://www.streamnook.app/c/509658"),
            category("509658")
        );
        assert_eq!(read("http://streamnook.app/c/1"), category("1"));
        assert_eq!(
            read("streamnook:///category/999999999999999"),
            category("999999999999999")
        );
    }

    #[test]
    fn nothing_usable_is_nothing() {
        // Fails if a bare kind, a provider word, a non-id or a foreign host is accepted.
        for junk in [
            "streamnook://category",
            "streamnook://category/",
            "streamnook:///category",
            "streamnook:///category/",
            "streamnook:///category/abc",
            "streamnook:///category/kick/1",
            "streamnook://category/kick/1",
            "streamnook:///category/1234567890123456",
            "streamnook://notify/category/1",
            "streamnook://watch/xqc",
            "streamnook:///badge/clover",
            "https://streamnook.app/c/abc",
            "https://streamnook.app/c",
            "https://streamnook.app/c/",
            "https://streamnook.app/w/xqc",
            "https://example.com/c/509658",
            "ftp://streamnook.app/c/1",
            "other:///category/1",
        ] {
            assert_eq!(read(junk), None, "{junk}");
        }
    }

    // slug, share_url, deep_link

    #[test]
    fn names_become_lowercase_words() {
        // Fails if accents are not folded, or non-Latin text leaves characters in the slug.
        assert_eq!(slug("Just Chatting"), "just-chatting");
        assert_eq!(slug("Grand Theft Auto V"), "grand-theft-auto-v");
        assert_eq!(slug("Pok\u{e9}mon"), "pokemon");
        assert_eq!(slug("\u{14c}kami"), "okami");
        assert_eq!(slug("\u{3057}\u{3083}\u{3061}"), "");
        assert_eq!(slug("  --Hi!!  there-- "), "hi-there");
        assert_eq!(slug("!!!"), "");
        assert_eq!(slug(""), "");
    }

    #[test]
    fn combining_marks_vanish_and_do_not_separate() {
        // Fails if a mark becomes a separator: a decomposed name would differ from the composed one.
        assert_eq!(slug("Poke\u{301}mon"), "pokemon");
        assert_eq!(slug("a\u{301}b"), "ab");
        assert_eq!(slug("\u{301}a"), "a");
        assert_eq!(slug("a\u{36f}\u{300}b"), "ab");
    }

    #[test]
    fn letters_that_do_not_decompose_separate_words() {
        // Fails if a table maps these to a letter: the site gives these exact slugs.
        assert_eq!(slug("\u{d8}lstr\u{f8}m"), "lstr-m");
        assert_eq!(slug("Stra\u{df}e"), "stra-e");
        assert_eq!(slug("\u{141}\u{f3}d\u{17a}"), "odz");
        assert_eq!(slug("\u{110}a"), "a");
        assert_eq!(slug("\u{c6}a\u{152}"), "a");
    }

    #[test]
    fn letters_that_decompose_fold_to_their_base() {
        // Fails if the table stops short of a block: Romanian, Vietnamese, the
        // dotted capital I, the Kelvin sign and the Angstrom sign all fold on the site.
        assert_eq!(slug("\u{218}"), "s");
        assert_eq!(slug("\u{21b}a"), "ta");
        assert_eq!(slug("\u{130}"), "i");
        assert_eq!(slug("Vi\u{1ec7}t"), "viet");
        assert_eq!(slug("\u{212a}"), "k");
        assert_eq!(slug("\u{212b}"), "a");
        assert_eq!(slug("\u{1ebf}"), "e");
    }

    #[test]
    fn the_cap_is_eighty_and_never_ends_on_a_dash() {
        // Fails if the cap is missing, or the trim after it is.
        assert_eq!(slug(&"a".repeat(100)).len(), 80);
        let landing_on_a_dash = format!("{} b", "a".repeat(79));
        let capped = slug(&landing_on_a_dash);
        assert_eq!(capped, "a".repeat(79));
        assert!(!capped.ends_with('-'));
    }

    #[test]
    fn share_and_deep_links_have_one_shape() {
        // Fails if the path, the slug part or the day tag moves: pages and old caches key on it.
        assert_eq!(
            share_url("509658", "Just Chatting", "1114"),
            "https://streamnook.app/c/509658/just-chatting?d=1114"
        );
        assert_eq!(
            share_url("509658", "\u{3057}\u{3083}\u{3061}", "1114"),
            "https://streamnook.app/c/509658?d=1114"
        );
        assert_eq!(
            share_url("1", "", "0101"),
            "https://streamnook.app/c/1?d=0101"
        );
        assert_eq!(deep_link("509658"), "streamnook:///category/509658");
    }

    #[test]
    fn every_built_link_reads_back_to_its_category() {
        // Fails if a builder and the parser drift apart.
        for (id, name) in [
            ("509658", "Just Chatting"),
            ("1", ""),
            ("27471", "Minecraft"),
            ("21779", "Pok\u{e9}mon"),
        ] {
            let web = share_url(id, name, "1114");
            let app = deep_link(id);
            assert_eq!(read(&web), category(id), "{web}");
            assert_eq!(read(&app), category(id), "{app}");
        }
    }

    #[test]
    fn a_share_link_is_built_only_for_a_twitch_game_id() {
        // Fails if the command builds a link for an id the page would answer with a 404.
        for bad in ["abc", "", "1234567890123456", "12 3", "1.5"] {
            assert_eq!(
                share_link_for(bad, "Just Chatting", "1114"),
                Err("not a Twitch category id".to_string()),
                "{bad:?}"
            );
        }
        assert_eq!(
            share_link_for("509658", "Just Chatting", "1114"),
            Ok("https://streamnook.app/c/509658/just-chatting?d=1114".to_string())
        );
    }

    // the slug fold table

    #[test]
    fn fold_runs_are_sorted_and_do_not_overlap() {
        // Fails if a paste puts a run out of order: the lookup stops at the first run past the character.
        let mut end = 0u32;
        for &(start, letters) in FOLD_RUNS {
            assert!(
                start >= end,
                "the run at U+{start:04X} overlaps the one before it"
            );
            assert!(
                letters
                    .bytes()
                    .all(|b| b == b'.' || b.is_ascii_alphabetic()),
                "U+{start:04X}"
            );
            end = start + letters.len() as u32;
        }
        assert!(FOLD_RUNS[0].0 >= 0xC0);
    }

    #[test]
    fn every_code_point_folds_as_the_checked_in_fixture_says() {
        // Fails if the table maps a letter the site separates (O with stroke to `o`), lowercases before it
        // folds (the dotted capital I would differ), stops before U+1EFF or the Kelvin and Angstrom signs, or turns a
        // combining mark into a separator.
        let fixture: Value = serde_json::from_str(SLUG_FIXTURE).expect("the slug fixture is JSON");
        let folds = fixture["folds"].as_object().expect("`folds` is an object");
        assert!(!folds.is_empty(), "the fixture lists folds");
        let mut listed = 0usize;
        let mut deletions: Vec<u32> = Vec::new();
        for code in 0x00C0u32..=0xFFFF {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let key = format!("a{c}b");
            let want = match folds.get(&key) {
                Some(folded) => {
                    listed += 1;
                    folded.as_str().expect("a fold is text")
                }
                None => "a-b",
            };
            assert_eq!(slug(&key), want, "U+{code:04X}");
            if want == "ab" {
                deletions.push(code);
            }
        }
        assert_eq!(
            listed,
            folds.len(),
            "every fixture key is `a<ch>b` for a walked code point"
        );
        assert_eq!(
            deletions,
            (0x0300u32..=0x036F).collect::<Vec<_>>(),
            "the empty folds are exactly the 112 combining marks"
        );
    }

    // answer_from, answer_from_helix

    #[test]
    fn a_game_with_a_name_is_found() {
        // Fails if the id, name, art or viewer count is read from the wrong key.
        assert_eq!(answer_from(&gql_found()), found_from_gql());
    }

    #[test]
    fn a_null_viewer_count_is_found_with_none() {
        // Fails if a null `viewersCount` makes the whole answer Unreachable.
        let mut body = gql_found();
        body["data"]["g0"]["viewersCount"] = Value::Null;
        assert_eq!(
            answer_from(&body),
            CategoryAnswer::Found {
                id: "509658".into(),
                name: "Just Chatting".into(),
                box_art_url: "https://static-cdn.jtvnw.net/ttv-boxart/509658-285x380.jpg".into(),
                viewer_count: None,
            }
        );
    }

    #[test]
    fn only_a_null_game_is_missing() {
        // Fails if Missing is read from an absent `g0`, a rate-limit body or a wrong shape.
        assert_eq!(
            answer_from(&json!({ "data": { "g0": null } })),
            CategoryAnswer::Missing
        );
        for body in [
            json!({}),
            json!({ "data": {} }),
            json!({ "data": null }),
            json!({ "data": { "g0": "game" } }),
            json!({ "data": { "g0": { "id": "1" } } }),
            json!({ "data": { "g0": { "name": "No id" } } }),
            json!({ "data": { "g0": { "id": "1", "name": "" } } }),
            rate_limited(),
            json!([1, 2, 3]),
        ] {
            assert_eq!(answer_from(&body), CategoryAnswer::Unreachable, "{body}");
        }
    }

    #[test]
    fn a_helix_row_with_a_name_is_found_without_viewers() {
        // Fails if the row is read from the wrong keys or invents a viewer count.
        assert_eq!(answer_from_helix(&[helix_row()]), found_from_helix());
    }

    #[test]
    fn an_empty_helix_list_is_unreachable_never_missing() {
        // Fails if an empty list means "no such category": a refused request also reads empty.
        assert_eq!(answer_from_helix(&[]), CategoryAnswer::Unreachable);
        assert_eq!(
            answer_from_helix(&[json!({ "id": "1" })]),
            CategoryAnswer::Unreachable
        );
        assert_eq!(answer_from_helix(&[json!({})]), CategoryAnswer::Unreachable);
    }

    // resolve_with

    #[tokio::test(start_paused = true)]
    async fn a_first_read_that_never_answers_hands_over_at_two_and_a_half_seconds() {
        // Fails without the bound on the first read: the resolve would wait on it for good.
        let start = tokio::time::Instant::now();
        let first = std::future::pending::<anyhow::Result<Value>>();
        let second = second_after(Duration::from_secs(1), Ok(vec![helix_row()]));
        assert_eq!(
            within_a_minute(resolve_with(first, second)).await,
            found_from_helix()
        );
        assert_eq!(start.elapsed(), Duration::from_millis(3500));
    }

    #[tokio::test(start_paused = true)]
    async fn a_first_read_error_then_a_slow_second_read_is_still_found() {
        // Fails if any outer bound is put around both reads: this answer lands at 5 s.
        let start = tokio::time::Instant::now();
        let first = first_after(Duration::ZERO, Err(anyhow::anyhow!("offline")));
        let second = second_after(Duration::from_secs(5), Ok(vec![helix_row()]));
        assert_eq!(
            within_a_minute(resolve_with(first, second)).await,
            found_from_helix()
        );
        assert_eq!(start.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_second_read_is_unreachable() {
        // Fails if a second read's error is read as a category that does not exist.
        let first = first_after(Duration::ZERO, Err(anyhow::anyhow!("offline")));
        let second = second_after(Duration::ZERO, Err(anyhow::anyhow!("signed out")));
        assert_eq!(
            within_a_minute(resolve_with(first, second)).await,
            CategoryAnswer::Unreachable
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_missing_game_never_asks_the_second_source() {
        // Fails if the fallback is polled after a clean "no such game".
        let asked = AtomicBool::new(false);
        let first = first_after(Duration::ZERO, Ok(json!({ "data": { "g0": null } })));
        let second = async {
            asked.store(true, Ordering::SeqCst);
            Ok::<Vec<Value>, anyhow::Error>(vec![helix_row()])
        };
        assert_eq!(
            within_a_minute(resolve_with(first, second)).await,
            CategoryAnswer::Missing
        );
        assert!(!asked.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn a_found_game_inside_the_bound_never_asks_the_second_source() {
        // Fails if the second source is read when the first already answered.
        let asked = AtomicBool::new(false);
        let start = tokio::time::Instant::now();
        let first = first_after(Duration::from_secs(2), Ok(gql_found()));
        let second = async {
            asked.store(true, Ordering::SeqCst);
            Ok::<Vec<Value>, anyhow::Error>(vec![])
        };
        assert_eq!(
            within_a_minute(resolve_with(first, second)).await,
            found_from_gql()
        );
        assert!(!asked.load(Ordering::SeqCst));
        assert_eq!(start.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn a_rate_limited_or_empty_first_answer_goes_to_the_second_source() {
        // Fails if an answer that is neither Found nor Missing ends the resolve as Unreachable.
        for body in [rate_limited(), json!({ "data": {} })] {
            let start = tokio::time::Instant::now();
            let first = first_after(Duration::ZERO, Ok(body));
            let second = second_after(Duration::from_secs(1), Ok(vec![helix_row()]));
            assert_eq!(
                within_a_minute(resolve_with(first, second)).await,
                found_from_helix()
            );
            assert_eq!(start.elapsed(), Duration::from_secs(1));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_first_answer_after_the_bound_is_not_waited_for() {
        // Fails if the first read keeps its place after the bound: it would answer Found at 3 s.
        let start = tokio::time::Instant::now();
        let first = first_after(Duration::from_secs(3), Ok(gql_found()));
        let second = second_after(Duration::ZERO, Err(anyhow::anyhow!("signed out")));
        assert_eq!(
            within_a_minute(resolve_with(first, second)).await,
            CategoryAnswer::Unreachable
        );
        assert_eq!(start.elapsed(), Duration::from_millis(2500));
    }

    #[tokio::test]
    async fn an_invalid_id_is_unreachable_with_no_network() {
        // Behavior check only: the order pin below is what proves no read starts for these.
        for bad in ["abc", "", "1234567890123456", "1\") { x }"] {
            assert_eq!(resolve(bad).await, CategoryAnswer::Unreachable, "{bad:?}");
        }
    }

    // the wire shape, against the one shared fixture

    #[test]
    fn the_answers_match_the_shared_fixture() {
        // Fails if a field name, the tag or a null moves: Swift and the page read the same file.
        let fixture = fixture();
        let answers = &fixture["categoryAnswer"];
        assert_eq!(
            serde_json::to_value(found_from_gql()).unwrap(),
            answers["found"]
        );
        assert_eq!(
            serde_json::to_value(answer_from_helix(&[helix_row()])).unwrap(),
            answers["foundNoViewers"]
        );
        assert_eq!(
            serde_json::to_value(CategoryAnswer::Missing).unwrap(),
            answers["missing"]
        );
        assert_eq!(
            serde_json::to_value(CategoryAnswer::Unreachable).unwrap(),
            answers["unreachable"]
        );
        let cases = answers.as_object().expect("categoryAnswer is an object");
        assert_eq!(cases.len(), 4);
        for (name, case) in cases {
            let read: CategoryAnswer =
                serde_json::from_value(case.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&serde_json::to_value(read).unwrap(), case, "{name}");
        }
    }

    #[test]
    fn the_fixture_names_the_three_ids_the_fake_core_answers() {
        // Fails if the ids drift: the iPhone's fake answers by these three.
        let fixture = fixture();
        let by_id = &fixture["resolveCategoryLink"];
        assert_eq!(by_id["509658"], fixture["categoryAnswer"]["found"]);
        assert_eq!(
            by_id["999999999999999"],
            fixture["categoryAnswer"]["missing"]
        );
        assert_eq!(by_id["2"], fixture["categoryAnswer"]["unreachable"]);
        assert_eq!(by_id.as_object().expect("an object").len(), 3);
    }

    #[test]
    fn the_share_link_case_is_what_the_builder_gives() {
        // Fails if the share link and the fixture's one case disagree.
        let fixture = fixture();
        let case = &fixture["shareLink"];
        let (id, name, day) = (
            case["id"].as_str().unwrap(),
            case["name"].as_str().unwrap(),
            case["day"].as_str().unwrap(),
        );
        assert_eq!(
            share_link_for(id, name, day),
            Ok(case["url"].as_str().unwrap().to_string())
        );
    }

    // source pins

    #[test]
    fn resolve_checks_the_id_before_it_reads() {
        // Fails if a read can start for an id that was never checked.
        let live = live_source();
        in_order(
            &body_of(&live, "pub async fn resolve("),
            &["valid_category_id(", "read_one("],
        );
    }

    #[test]
    fn the_second_source_needs_a_sign_in_before_it_sends() {
        // Fails if the games list is asked while signed out: it would send a request that cannot answer.
        let live = live_source();
        in_order(
            &body_of(&live, "async fn helix_games("),
            &["get_token(", "get_games_by_ids("],
        );
    }

    #[test]
    fn this_file_sends_no_twitch_gql() {
        // Fails if a sender call or its name appears here, comments included: the manifest scan reads them.
        let live = live_source();
        assert!(live.contains("pub fn parse("), "the live part was read");
        for sender in [
            concat!("gql.twitch", ".tv"),
            concat!("twitch_web_gql", "::post"),
            concat!("gql_query", "("),
            concat!("gql_public", "_read("),
            concat!("gql_web", "_query("),
        ] {
            assert!(
                !live.contains(sender),
                "`{sender}` is spelled in category_link.rs"
            );
        }
    }
}
