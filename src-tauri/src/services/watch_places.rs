//! Where something can be watched, ready to draw as tappable cards: each
//! category name resolved to Twitch's own category (its id, box art and live
//! viewers) and each channel with its face and whether it is live. A badge's
//! earn guide names these; desktop's badge page and the phones' badge sheet
//! show them the same way.

use std::collections::HashSet;

use serde::Serialize;

use crate::models::stream::TwitchStream;
use crate::services::count_text;
use crate::services::profile_view::{pick_avatar, AvatarPick};
use crate::services::twitch_service::{CategoryRow, TwitchService};
use crate::utils::text_fold::fold_search;

/// Channels resolved when the caller names no limit: a badge page's cards and
/// the chips in its copy together.
const DEFAULT_CHANNELS: usize = 16;
/// One Helix call each for faces and liveness: a campaign's list past this
/// is left as names.
const MAX_CHANNELS: usize = 100;
const MAX_CATEGORIES: usize = 4;
/// Box art as the cards draw it (3:4, sharp at twice the card's size).
const BOX_ART_W: u32 = 144;
const BOX_ART_H: u32 = 192;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlaceCategory {
    /// The name it was asked for, so a caller can match its own links.
    pub query: String,
    /// Twitch's id; absent when the lookup failed and the name is kept as is.
    pub id: Option<String>,
    /// Twitch's own spelling.
    pub name: String,
    pub box_art_url: Option<String>,
    /// "1.2K viewers", "No live streams"; absent when not counted.
    pub viewers_line: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PlaceChannel {
    pub login: String,
    pub user_id: Option<String>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// The live stream, handed to `startStream` so the player opens with it.
    pub live: Option<TwitchStream>,
    /// "Live · Just Chatting · 1.2K viewers", or "Offline".
    pub status_line: String,
    /// The channel's 7TV profile picture at the cards' size, for a shell
    /// with no 7TV lookup of its own (the iPhone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<AvatarPick>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct WatchPlaces {
    pub categories: Vec<PlaceCategory>,
    /// Live channels first, each group in the order asked.
    pub channels: Vec<PlaceChannel>,
}

/// `limit` caps the channels resolved (16 by default, at most 100).
pub async fn resolve(
    categories: Vec<String>,
    channels: Vec<String>,
    limit: Option<usize>,
) -> WatchPlaces {
    let limit = limit.unwrap_or(DEFAULT_CHANNELS).clamp(1, MAX_CHANNELS);
    let (categories, channels) = tokio::join!(
        resolve_categories(categories),
        resolve_channels(channels, limit)
    );
    WatchPlaces {
        categories,
        channels,
    }
}

async fn resolve_categories(names: Vec<String>) -> Vec<PlaceCategory> {
    let mut out: Vec<PlaceCategory> = Vec::new();
    let mut asked = HashSet::new();
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        if out.len() >= MAX_CATEGORIES || !asked.insert(fold_search(name)) {
            continue;
        }
        let Some(place) = resolve_category(name).await else {
            continue;
        };
        // Two spellings that resolve to one category draw one card.
        let dup = out.iter().any(|p| match (&p.id, &place.id) {
            (Some(a), Some(b)) => a == b,
            _ => fold_search(&p.name) == fold_search(&place.name),
        });
        if !dup {
            out.push(place);
        }
    }
    out
}

/// One name as Twitch knows it. A name Twitch cannot resolve is dropped
/// rather than drawn as a card that leads nowhere; a failed lookup keeps the
/// name as asked, since browsing by name may still find it.
async fn resolve_category(name: &str) -> Option<PlaceCategory> {
    let rows = match TwitchService::search_categories(name, 5).await {
        Ok(rows) => rows,
        Err(e) => {
            log::debug!("[WatchPlaces] category search for {name:?} failed: {e}");
            return Some(PlaceCategory {
                query: name.to_string(),
                id: None,
                name: name.to_string(),
                box_art_url: None,
                viewers_line: None,
            });
        }
    };
    if let Some(row) = pick_category(name, &rows) {
        return Some(place_from(name, row));
    }
    // Event names get mistaken for categories ("La Velada del Año VI" when the
    // category is "La Velada"): shorter and shorter prefixes, taking only a
    // category the whole name starts with.
    for query in prefix_queries(name) {
        let Ok(rows) = TwitchService::search_categories(&query, 5).await else {
            return None;
        };
        if let Some(row) = prefix_hit(name, &rows) {
            return Some(place_from(name, row));
        }
    }
    None
}

fn place_from(query: &str, row: &CategoryRow) -> PlaceCategory {
    PlaceCategory {
        query: query.to_string(),
        id: Some(row.id.clone()),
        name: row.name.clone(),
        box_art_url: sized_box_art(&row.box_art_url),
        viewers_line: count_text::viewers_line(row.viewer_count),
    }
}

/// The category a name means among search results: the one named exactly
/// that, else the longest that contains it or is contained in it (the more
/// specific "Tom Clancy's Rainbow Six Siege X" over "Rainbow Six"), else
/// Twitch's best guess.
pub fn pick_category<'a>(name: &str, rows: &'a [CategoryRow]) -> Option<&'a CategoryRow> {
    let want = fold_search(name.trim());
    if let Some(exact) = rows.iter().find(|r| fold_search(&r.name) == want) {
        return Some(exact);
    }
    rows.iter()
        .filter(|r| {
            let have = fold_search(&r.name);
            have.contains(&want) || want.contains(&have)
        })
        .max_by_key(|r| r.name.chars().count())
        .or_else(|| rows.first())
}

/// The name minus one trailing word at a time, down to two words.
pub fn prefix_queries(name: &str) -> Vec<String> {
    let words: Vec<&str> = name.split_whitespace().collect();
    (2..words.len())
        .rev()
        .map(|n| words[..n].join(" "))
        .collect()
}

/// A result the whole name starts with, at least four characters long.
pub fn prefix_hit<'a>(name: &str, rows: &'a [CategoryRow]) -> Option<&'a CategoryRow> {
    let whole = fold_search(name.trim());
    rows.iter()
        .find(|r| r.name.chars().count() >= 4 && whole.starts_with(&fold_search(&r.name)))
}

/// Helix answers a `{width}x{height}` template; GQL a sized url, kept as is.
fn sized_box_art(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    Some(
        url.replace("{width}", &BOX_ART_W.to_string())
            .replace("{height}", &BOX_ART_H.to_string()),
    )
}

async fn resolve_channels(logins: Vec<String>, limit: usize) -> Vec<PlaceChannel> {
    let mut seen = HashSet::new();
    let logins: Vec<String> = logins
        .iter()
        .map(|l| l.trim().trim_start_matches('@').to_lowercase())
        .filter(|l| !l.is_empty() && seen.insert(l.clone()))
        .take(limit)
        .collect();
    if logins.is_empty() {
        return Vec::new();
    }
    let (users, live) = tokio::join!(
        TwitchService::users_by_logins(&logins),
        TwitchService::check_streams_online(&logins)
    );
    // Signed out, Helix refuses the users lookup: every name is kept then,
    // with no face, rather than none drawn at all.
    let users = users
        .map_err(|e| log::debug!("[WatchPlaces] user lookup failed: {e}"))
        .ok();
    let live = live.unwrap_or_default();
    let mut out: Vec<PlaceChannel> = logins
        .into_iter()
        .filter(|login| {
            // A name Twitch does not know (a typo in the copy) draws nothing.
            users.as_ref().is_none_or(|u| u.contains_key(login))
                || live
                    .iter()
                    .any(|s| s.user_login.eq_ignore_ascii_case(login))
        })
        .map(|login| {
            let user = users.as_ref().and_then(|u| u.get(&login));
            let stream = live
                .iter()
                .find(|s| s.user_login.eq_ignore_ascii_case(&login))
                .cloned();
            let display_name = stream
                .as_ref()
                .map(|s| s.user_name.clone())
                .filter(|n| !n.is_empty())
                .or_else(|| user.map(|u| u.1.clone()))
                .unwrap_or_else(|| login.clone());
            PlaceChannel {
                user_id: user
                    .map(|u| u.0.clone())
                    .or_else(|| stream.as_ref().map(|s| s.user_id.clone()))
                    .filter(|id| !id.is_empty()),
                avatar_url: user.and_then(|u| u.2.clone()),
                status_line: status_line(stream.as_ref()),
                display_name,
                live: stream,
                login,
                picture: None,
            }
        })
        .collect();
    // Stable, so each group keeps the order it was asked in.
    out.sort_by_key(|c| c.live.is_none());
    add_pictures(&mut out).await;
    out
}

/// 7TV pictures for the cards a sheet draws: every channel of a short list,
/// only the live ones of a campaign's long one (its offline channels stay
/// names).
async fn add_pictures(channels: &mut [PlaceChannel]) {
    let all = channels.len() <= DEFAULT_CHANNELS;
    let ids: Vec<String> = channels
        .iter()
        .filter(|c| all || c.live.is_some())
        .filter_map(|c| c.user_id.clone())
        .collect();
    if ids.is_empty() {
        return;
    }
    let pictures = crate::services::seventv_cosmetics_resolver::profile_pictures(ids).await;
    for c in channels.iter_mut() {
        let Some(id) = &c.user_id else { continue };
        if let Some(Some(p)) = pictures.get(id) {
            c.picture = pick_avatar(p, 36.0, 3.0, false);
        }
    }
}

pub fn status_line(stream: Option<&TwitchStream>) -> String {
    let Some(s) = stream else {
        return "Offline".to_string();
    };
    let mut parts = vec!["Live".to_string()];
    if !s.game_name.trim().is_empty() {
        parts.push(s.game_name.trim().to_string());
    }
    if s.viewer_count > 0 {
        parts.extend(count_text::viewers_line(Some(s.viewer_count as u64)));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str) -> CategoryRow {
        CategoryRow {
            id: id.into(),
            name: name.into(),
            box_art_url: String::new(),
            viewer_count: None,
        }
    }

    #[test]
    fn the_exact_name_wins_over_a_longer_one() {
        let rows = [row("1", "Rainbow Six Siege X"), row("2", "Rainbow Six")];
        assert_eq!(
            pick_category("rainbow six", &rows).map(|r| r.id.as_str()),
            Some("2")
        );
    }

    #[test]
    fn without_an_exact_name_the_most_specific_containing_one_wins() {
        let rows = [
            row("1", "Siege"),
            row("2", "Tom Clancy's Rainbow Six Siege X"),
            row("3", "Rainbow"),
        ];
        assert_eq!(
            pick_category("Rainbow Six Siege", &rows).map(|r| r.id.as_str()),
            Some("2")
        );
    }

    #[test]
    fn accents_and_case_do_not_matter() {
        let rows = [row("1", "Pokémon Legends: Z-A")];
        assert_eq!(
            pick_category("pokemon legends: z-a", &rows).map(|r| r.id.as_str()),
            Some("1")
        );
    }

    #[test]
    fn an_event_name_falls_back_to_the_category_it_starts_with() {
        assert_eq!(
            prefix_queries("La Velada del Año VI"),
            ["La Velada del Año", "La Velada del", "La Velada"]
        );
        assert!(prefix_queries("Fortnite").is_empty());
        let rows = [row("1", "Velada"), row("2", "La Velada")];
        assert_eq!(
            prefix_hit("La Velada del Año VI", &rows).map(|r| r.id.as_str()),
            Some("2")
        );
        assert!(
            prefix_hit("La Velada del Año VI", &[row("3", "La")]).is_none(),
            "too short to trust"
        );
    }

    #[test]
    fn helix_box_art_is_sized_and_gql_art_kept() {
        assert_eq!(
            sized_box_art("https://x/art-{width}x{height}.jpg").as_deref(),
            Some("https://x/art-144x192.jpg")
        );
        assert_eq!(
            sized_box_art("https://x/art-285x380.jpg").as_deref(),
            Some("https://x/art-285x380.jpg")
        );
        assert_eq!(sized_box_art(""), None);
    }

    fn stream(game: &str, viewers: u32) -> TwitchStream {
        serde_json::from_value(serde_json::json!({
            "id": "", "user_id": "", "user_name": "", "user_login": "", "title": "",
            "viewer_count": viewers, "game_name": game, "thumbnail_url": "", "started_at": "",
        }))
        .expect("stream fixture")
    }

    #[test]
    fn a_live_line_names_the_category_and_viewers() {
        assert_eq!(
            status_line(Some(&stream("Just Chatting", 1234))),
            "Live · Just Chatting · 1.2K viewers"
        );
        assert_eq!(status_line(Some(&stream("", 0))), "Live");
        assert_eq!(status_line(None), "Offline");
    }
}
