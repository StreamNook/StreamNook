//! Favorite Twitch categories: the in-place edit of the starred list and the
//! live numbers the row above Home's Categories grid shows.
//!
//! The list lives in settings (`favorite_categories`), edited only through
//! `apply` so two windows starring at once cannot drop each other's change.
//! The numbers come from one anonymous GQL read for every starred category
//! (`game(id:) { viewersCount }`): category viewer counts are public, so no
//! sign-in is needed. `home_snapshot` owns when they refresh.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::{json, Value};

use crate::models::settings::FavoriteCategory;
use crate::services::twitch_service::TwitchService;

/// Categories asked for in one GQL request: one alias each.
pub(crate) const BATCH: usize = crate::services::twitch_limits::GQL_MAX_ALIASES;

/// One tile in the favorites row, in the shape the Categories grid draws.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FavoriteCategoryRow {
    pub id: String,
    pub name: String,
    pub box_art_url: String,
    /// `None` until Twitch has answered for this category.
    pub viewer_count: Option<u64>,
}

/// What Twitch says about one category right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    pub name: String,
    pub box_art_url: String,
    pub viewer_count: u64,
}

/// Star (`favorite`) or unstar `category` in `list`, in place. A newly starred
/// one goes last; starring one already there refreshes its saved name and art.
/// Returns false when nothing changed.
pub fn apply(list: &mut Vec<FavoriteCategory>, category: FavoriteCategory, favorite: bool) -> bool {
    let id = category.id.trim().to_string();
    if id.is_empty() {
        return false;
    }
    let at = list.iter().position(|c| c.id == id);
    match (favorite, at) {
        (true, Some(i)) => {
            let saved = &mut list[i];
            let mut changed = false;
            if !category.name.is_empty() && saved.name != category.name {
                saved.name = category.name;
                changed = true;
            }
            if !category.box_art_url.is_empty() && saved.box_art_url != category.box_art_url {
                saved.box_art_url = category.box_art_url;
                changed = true;
            }
            changed
        }
        (true, None) => {
            list.push(FavoriteCategory { id, ..category });
            true
        }
        (false, Some(i)) => {
            list.remove(i);
            true
        }
        (false, None) => false,
    }
}

/// A Twitch game id is digits; anything else never reaches a query.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// One request reading `ids`, aliased `g0`, `g1`, ... in order.
pub fn live_query(ids: &[&str]) -> String {
    let fields: Vec<String> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            format!(r#"g{i}: game(id: "{id}") {{ id name boxArtURL(width: 285, height: 380) viewersCount }}"#)
        })
        .collect();
    format!("{{ {} }}", fields.join(" "))
}

/// The categories a `live_query` answer names, by id.
pub fn parse_live(body: &Value, ids: &[&str]) -> HashMap<String, Live> {
    let mut out = HashMap::new();
    let Some(data) = body.get("data") else { return out };
    for (i, id) in ids.iter().enumerate() {
        let Some(game) = data.get(format!("g{i}")).filter(|g| !g.is_null()) else { continue };
        let Some(viewers) = game.get("viewersCount").and_then(Value::as_u64) else { continue };
        let text = |key: &str| game.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        out.insert(
            id.to_string(),
            Live { name: text("name"), box_art_url: text("boxArtURL"), viewer_count: viewers },
        );
    }
    out
}

/// Live numbers for every starred category, in batched anonymous requests. A
/// failed batch leaves its categories out; the caller keeps their last counts.
pub async fn fetch_live(favorites: &[FavoriteCategory]) -> HashMap<String, Live> {
    let ids: Vec<&str> = favorites.iter().map(|c| c.id.as_str()).filter(|id| valid_id(id)).collect();
    let mut out = HashMap::new();
    for chunk in ids.chunks(BATCH) {
        match TwitchService::gql_public_read(json!({ "query": live_query(chunk) })).await {
            Ok(body) => out.extend(parse_live(&body, chunk)),
            Err(e) => log::debug!("[FavoriteCategories] live read failed: {e}"),
        }
    }
    out
}

/// The row, in the order the categories were starred: Twitch's name and art
/// when it answered, the saved ones otherwise, and the last known count for a
/// category this round did not reach.
pub fn rows(
    favorites: &[FavoriteCategory],
    live: &HashMap<String, Live>,
    previous: &[FavoriteCategoryRow],
) -> Vec<FavoriteCategoryRow> {
    favorites
        .iter()
        .map(|saved| {
            let now = live.get(&saved.id);
            let pick = |fresh: Option<&String>, kept: &String| match fresh {
                Some(v) if !v.is_empty() => v.clone(),
                _ => kept.clone(),
            };
            FavoriteCategoryRow {
                id: saved.id.clone(),
                name: pick(now.map(|l| &l.name), &saved.name),
                box_art_url: pick(now.map(|l| &l.box_art_url), &saved.box_art_url),
                viewer_count: now
                    .map(|l| l.viewer_count)
                    .or_else(|| previous.iter().find(|p| p.id == saved.id).and_then(|p| p.viewer_count)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(id: &str, name: &str) -> FavoriteCategory {
        FavoriteCategory { id: id.into(), name: name.into(), box_art_url: format!("art-{id}") }
    }

    #[test]
    fn starring_adds_once_and_unstarring_removes() {
        let mut list = vec![];
        assert!(apply(&mut list, cat("1", "Just Chatting"), true));
        assert!(apply(&mut list, cat("2", "Minecraft"), true));
        assert!(!apply(&mut list, cat("1", "Just Chatting"), true), "already starred");
        assert_eq!(list.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["1", "2"]);
        assert!(apply(&mut list, cat("1", ""), false));
        assert!(!apply(&mut list, cat("1", ""), false), "already gone");
        assert_eq!(list, vec![cat("2", "Minecraft")]);
        assert!(!apply(&mut list, cat("  ", "x"), true), "no id, no entry");
    }

    #[test]
    fn starring_again_refreshes_a_renamed_category() {
        let mut list = vec![cat("7", "Old name")];
        assert!(apply(&mut list, cat("7", "New name"), true));
        assert_eq!(list[0].name, "New name");
        assert!(!apply(&mut list, FavoriteCategory { id: "7".into(), ..Default::default() }, true), "blank fields keep what is saved");
    }

    #[test]
    fn only_numeric_ids_reach_the_query() {
        assert!(valid_id("509658"));
        assert!(!valid_id("1\") { x }"));
        assert!(!valid_id(""));
        assert_eq!(
            live_query(&["1", "2"]),
            r#"{ g0: game(id: "1") { id name boxArtURL(width: 285, height: 380) viewersCount } g1: game(id: "2") { id name boxArtURL(width: 285, height: 380) viewersCount } }"#
        );
    }

    #[test]
    fn the_answer_reads_by_alias_and_skips_unknown_games() {
        let body = json!({ "data": {
            "g0": { "id": "509658", "name": "Just Chatting", "boxArtURL": "https://a/509658-285x380.jpg", "viewersCount": 243261 },
            "g1": null,
        }});
        let live = parse_live(&body, &["509658", "999"]);
        assert_eq!(live.len(), 1);
        assert_eq!(live["509658"].viewer_count, 243261);
        assert_eq!(live["509658"].name, "Just Chatting");
    }

    #[test]
    fn rows_keep_star_order_saved_words_and_last_counts() {
        let favorites = vec![cat("1", "Saved one"), cat("2", "Saved two"), cat("3", "Saved three")];
        let live = HashMap::from([(
            "2".to_string(),
            Live { name: "Live two".into(), box_art_url: String::new(), viewer_count: 50 },
        )]);
        let previous = vec![FavoriteCategoryRow { id: "1".into(), name: "x".into(), box_art_url: "x".into(), viewer_count: Some(10) }];
        let out = rows(&favorites, &live, &previous);
        assert_eq!(out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["1", "2", "3"]);
        assert_eq!(out[0].viewer_count, Some(10), "missed this round: last count");
        assert_eq!(out[0].name, "Saved one");
        assert_eq!(out[1].name, "Live two");
        assert_eq!(out[1].box_art_url, "art-2", "blank live art keeps the saved one");
        assert_eq!(out[1].viewer_count, Some(50));
        assert_eq!(out[2].viewer_count, None, "never answered");
    }
}
