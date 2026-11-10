//! Whether a search result is about what the viewer typed.
//!
//! Twitch's channel search matches on channel names, so its rows already name
//! the query. YouTube's live search does not: asked for "ishowspeed" it answers
//! with whatever is live that YouTube thinks is related (a Fifa stream, a
//! cemetery webcam, a Roblox giveaway with `#ishowspeed` in its hashtags), and
//! TikTok's live search is just as loose. So every platform's rows are scored
//! here against the query, and a row that names it nowhere is dropped rather
//! than sorted lower.
//!
//! A row names the query when its channel (display name, login, handle) holds
//! it, or failing that when its title says it in words. Hashtags and @mentions
//! do not count: stuffing a famous name into a title's tags is exactly how
//! unrelated streams get into those results.
//!
//! The rank is the same scale on every platform, so the page can put the
//! channel's own live stream first whichever platform it is on.

use crate::models::provider_stream::ProviderStream;
use crate::models::stream::TwitchStream;

/// The channel's name is the query.
pub const RANK_EXACT: u8 = 0;
/// The channel's name starts with the query.
pub const RANK_PREFIX: u8 = 1;
/// The channel's name holds the query somewhere.
pub const RANK_CONTAINS: u8 = 2;
/// The channel's name is the query with one letter off.
pub const RANK_NEAR: u8 = 3;
/// Only the stream's title names the query.
pub const RANK_TITLE: u8 = 4;
/// A Twitch row none of the above matched. Twitch's own search is a name
/// search, so it is kept (it vouched for the row) and placed last.
pub const RANK_TWITCH_KEPT: u8 = 5;

/// The query, read once.
pub struct Query {
    /// Lowercase words, punctuation removed.
    tokens: Vec<String>,
    /// The words run together: "I Show Speed" and "IShowSpeed" both read
    /// "ishowspeed".
    compact: String,
}

impl Query {
    /// `None` when nothing searchable is left (only punctuation), and then
    /// nothing is filtered.
    pub fn new(raw: &str) -> Option<Self> {
        let tokens = words(raw);
        let compact: String = tokens.concat();
        if compact.is_empty() {
            return None;
        }
        Some(Query { tokens, compact })
    }

    /// How well a row matches, best first, or `None` when it does not name the
    /// query at all. `names` are the channel's display name, login or handle;
    /// `title` is the stream title.
    pub fn rank(&self, names: &[&str], title: &str) -> Option<u8> {
        let by_name = names.iter().filter_map(|n| self.rank_name(n)).min();
        by_name.or_else(|| self.names_in_title(title).then_some(RANK_TITLE))
    }

    fn rank_name(&self, name: &str) -> Option<u8> {
        let name: String = words(name).concat();
        if name.is_empty() {
            return None;
        }
        let q = &self.compact;
        if name == *q {
            Some(RANK_EXACT)
        } else if name.starts_with(q.as_str()) {
            Some(RANK_PREFIX)
        } else if name.contains(q.as_str())
            || (self.tokens.len() > 1 && self.tokens.iter().all(|t| name.contains(t.as_str())))
        {
            Some(RANK_CONTAINS)
        } else if q.chars().count() >= 5 && within_one_edit(&name, q) {
            Some(RANK_NEAR)
        } else {
            None
        }
    }

    /// The title says the query in plain words: every query word appears as a
    /// word, or the query is a run of whole words ("I Show Speed").
    fn names_in_title(&self, title: &str) -> bool {
        let said: Vec<String> = title
            .split_whitespace()
            .filter(|w| !w.starts_with('#') && !w.starts_with('@'))
            .flat_map(words)
            .collect();
        if said.is_empty() {
            return false;
        }
        if self.tokens.iter().all(|t| said.iter().any(|w| w == t)) {
            return true;
        }
        (0..said.len()).any(|start| {
            let mut run = String::new();
            for w in &said[start..] {
                run.push_str(w);
                if run.len() >= self.compact.len() {
                    return run == self.compact;
                }
            }
            false
        })
    }
}

/// Lowercase alphanumeric runs, so "IShowSpeed's" reads ["ishowspeed", "s"].
fn words(raw: &str) -> Vec<String> {
    raw.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// One insertion, deletion or substitution apart (or equal).
fn within_one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < short.len() && j < long.len() {
        if short[i] == long[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if short.len() == long.len() {
            i += 1;
        }
        j += 1;
    }
    edits + (long.len() - j) + (short.len() - i) <= 1
}

/// The names a platform's row can be matched by. A YouTube row's login is the
/// video id, which names nothing.
fn provider_names(row: &ProviderStream) -> Vec<&str> {
    if row.provider == "youtube" {
        vec![row.user_name.as_str()]
    } else {
        vec![row.user_name.as_str(), row.user_login.as_str()]
    }
}

/// Keep the rows that name `query`, best match first (then live, then most
/// watched), and return each kept row's rank in the same order.
pub fn keep_relevant(query: &str, rows: &mut Vec<ProviderStream>) -> Vec<u8> {
    let Some(q) = Query::new(query) else {
        return vec![RANK_TWITCH_KEPT; rows.len()];
    };
    let mut ranked: Vec<(u8, ProviderStream)> = std::mem::take(rows)
        .into_iter()
        .filter_map(|row| q.rank(&provider_names(&row), &row.title).map(|r| (r, row)))
        .collect();
    ranked.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then_with(|| b.is_live.cmp(&a.is_live))
            .then_with(|| b.viewer_count.cmp(&a.viewer_count))
    });
    let ranks = ranked.iter().map(|(r, _)| *r).collect();
    rows.extend(ranked.into_iter().map(|(_, row)| row));
    ranks
}

/// Each Twitch row's rank, in the order Twitch's own ranking left them. No row
/// is dropped: see `RANK_TWITCH_KEPT`.
pub fn twitch_ranks(query: &str, rows: &[TwitchStream]) -> Vec<u8> {
    let Some(q) = Query::new(query) else {
        return vec![RANK_TWITCH_KEPT; rows.len()];
    };
    rows.iter()
        .map(|row| {
            q.rank(&[row.user_name.as_str(), row.user_login.as_str()], &row.title)
                .unwrap_or(RANK_TWITCH_KEPT)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn youtube(author: &str, title: &str, viewers: u32) -> ProviderStream {
        ProviderStream {
            provider: "youtube".into(),
            key: format!("youtube:{author}"),
            id: "vid".into(),
            user_id: "UCxxxxxxxxxxxxxxxxxxxxxx".into(),
            user_login: "dQw4w9WgXcQ".into(),
            user_name: author.into(),
            title: title.into(),
            viewer_count: viewers,
            game_id: String::new(),
            game_name: String::new(),
            category_thumbnail: None,
            thumbnail_url: String::new(),
            started_at: String::new(),
            profile_image_url: None,
            is_live: true,
            watch_url: String::new(),
            tags: None,
        }
    }

    fn names(rows: &[ProviderStream]) -> Vec<&str> {
        rows.iter().map(|r| r.user_name.as_str()).collect()
    }

    /// What YouTube's live search really answered for "ishowspeed" on
    /// 2026-11-10 (author, title), with IShowSpeed's own stream added as it
    /// reads when he is live.
    fn ishowspeed_answer() -> Vec<ProviderStream> {
        vec![
            youtube("TxC ImInferior", "Fifa 20", 0),
            youtube("ANIYETTAN", "MEENAMMA ANI IS LIVE | unlimited custom room 😱 | #gyangaming  #ishowspeed  #raistar #nonstopgaming", 0),
            youtube("Lazy Girl", "MONKEY LIVE   | ft-LazyGirl | #omegle #lazygirl #girl #facecam", 23),
            youtube("RamuneXD", "Roblox RIVALS Live!🔴 | Playing Roblox With Viewers 🔥", 0),
            youtube("City of Goshen Environmental Resilience Department", "Violett Cemetery", 0),
            youtube("SINVY - Animations", "🔴 (Roblox Vore) RUMI in Animal Hospital? 🏥😳 [24/7 LIVE]", 0),
            youtube("XAJJU EXTRA ", "Granny Live || Granny Horror Gameplay #granny #shortslive #shortsfeed #ishowspeed #short #horrorr", 946),
            youtube("RunningBall", "Speed 4", 98),
            youtube("Speed Clips Daily", "IShowSpeed's best moments 24/7", 310),
            youtube("IShowSpeed", "SPEED IN BRAZIL 🇧🇷", 120_000),
        ]
    }

    #[test]
    fn unrelated_live_streams_are_dropped_and_the_channel_itself_leads() {
        let mut rows = ishowspeed_answer();
        let ranks = keep_relevant("ishowspeed", &mut rows);
        assert_eq!(names(&rows), vec!["IShowSpeed", "Speed Clips Daily"]);
        assert_eq!(ranks, vec![RANK_EXACT, RANK_TITLE]);
    }

    #[test]
    fn a_hashtag_alone_never_names_the_query() {
        let mut rows = ishowspeed_answer();
        keep_relevant("ishowspeed", &mut rows);
        assert!(!names(&rows).contains(&"XAJJU EXTRA "), "#ishowspeed in the tags is bait, not a match");
        assert!(!names(&rows).contains(&"ANIYETTAN"));
    }

    #[test]
    fn case_spaces_and_a_part_of_the_name_all_find_the_channel() {
        for query in ["IShowSpeed", "ishow speed", "@IShowSpeed", "ishowspeed "] {
            let mut rows = ishowspeed_answer();
            keep_relevant(query, &mut rows);
            assert_eq!(rows.first().map(|r| r.user_name.as_str()), Some("IShowSpeed"), "{query}");
        }
        // "speed" is part of his name, so he leads; other channels named
        // "speed" follow, and a title that says the word comes last.
        let mut rows = ishowspeed_answer();
        let ranks = keep_relevant("speed", &mut rows);
        assert_eq!(names(&rows), vec!["Speed Clips Daily", "IShowSpeed", "RunningBall"]);
        assert_eq!(ranks, vec![RANK_PREFIX, RANK_CONTAINS, RANK_TITLE]);
    }

    #[test]
    fn one_letter_off_still_finds_the_channel() {
        let mut rows = ishowspeed_answer();
        let ranks = keep_relevant("ishowsped", &mut rows);
        assert_eq!(names(&rows), vec!["IShowSpeed"]);
        assert_eq!(ranks, vec![RANK_NEAR]);
    }

    #[test]
    fn a_title_naming_the_query_in_words_is_kept() {
        let q = Query::new("kai cenat").unwrap();
        assert_eq!(q.rank(&["Restream Hub"], "Kai Cenat mafiathon day 3"), Some(RANK_TITLE));
        assert_eq!(q.rank(&["Restream Hub"], "watch with @KaiCenat"), None);
        let q = Query::new("ishowspeed").unwrap();
        assert_eq!(q.rank(&["Fan"], "I Show Speed live in Tokyo"), Some(RANK_TITLE));
        // A word that merely starts the same way is not the query.
        assert_eq!(q.rank(&["Fan"], "ishowspeedrun attempts"), None);
    }

    #[test]
    fn tiktok_and_kick_match_on_the_handle_too() {
        let mut row = youtube("Yayo", "Send daily heart", 51);
        row.provider = "tiktok".into();
        row.user_login = "yayoworldwide1".into();
        let mut rows = vec![row];
        assert_eq!(keep_relevant("yayoworldwide", &mut rows), vec![RANK_PREFIX]);
        // On YouTube the login is a video id and must not be matched.
        let mut rows = vec![youtube("Someone", "Untitled", 0)];
        keep_relevant("dQw4w9WgXcQ", &mut rows);
        assert!(rows.is_empty());
    }

    #[test]
    fn a_query_with_nothing_to_match_filters_nothing() {
        let mut rows = ishowspeed_answer();
        let ranks = keep_relevant("  !! ", &mut rows);
        assert_eq!(rows.len(), 10);
        assert!(ranks.iter().all(|r| *r == RANK_TWITCH_KEPT));
    }

    #[test]
    fn twitch_rows_are_ranked_but_never_dropped() {
        let row = |login: &str, name: &str| TwitchStream {
            id: login.into(),
            user_id: login.into(),
            user_name: name.into(),
            user_login: login.into(),
            title: String::new(),
            viewer_count: 0,
            game_id: String::new(),
            game_name: String::new(),
            thumbnail_url: String::new(),
            started_at: String::new(),
            broadcaster_type: None,
            profile_image_url: None,
            is_live: Some(true),
            tags: None,
            language: None,
            drops_earnable: false,
        };
        let rows = vec![row("ishowspeed", "IShowSpeed"), row("i_show_speed_fan", "SpeedFan"), row("xqc", "xQc")];
        assert_eq!(twitch_ranks("ishowspeed", &rows), vec![RANK_EXACT, RANK_PREFIX, RANK_TWITCH_KEPT]);
    }

    #[test]
    fn one_edit_means_one() {
        assert!(within_one_edit("ishowspeed", "ishowsped"));
        assert!(within_one_edit("ishowspeed", "ishowspeeds"));
        assert!(within_one_edit("ishowspeed", "ishowspeee"));
        assert!(!within_one_edit("ishowspeed", "ishowsp"));
        assert!(!within_one_edit("ishowspeed", "ushowspeef"));
    }
}
