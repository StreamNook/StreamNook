//! Game suggestions for a game picker: the games running drops right now
//! first, then Twitch's own category search. Every suggestion carries the
//! game's exact Twitch name, so a list built from picks always matches the
//! names drop campaigns use.

use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GameSuggestion {
    pub id: String,
    pub name: String,
    pub box_art_url: String,
    /// Drop campaigns running for this game now; 0 when it has none.
    pub drop_campaigns: u32,
}

/// Box art at the picker's size, from any Twitch box-art URL: a
/// `{width}x{height}` template (Helix games) or one already sized, such as
/// Helix search's `-52x72.jpg` or a campaign's `boxArtURL`. The size is twice
/// what the picker draws, so the art stays sharp. Twitch serves art only at
/// the URLs it hands out (many are `<id>_IGDB-...`), so the id alone is never
/// used to build one.
pub fn sized_box_art(url: &str) -> String {
    const SIZE: &str = "144x192";
    if url.contains("{width}x{height}") {
        return url.replace("{width}x{height}", SIZE);
    }
    if let Some(dash) = url.rfind('-') {
        let (head, tail) = url.split_at(dash);
        let dims = tail.trim_start_matches('-');
        if let Some((size, ext)) = dims.split_once('.') {
            let is_size = size
                .split_once('x')
                .is_some_and(|(w, h)| !w.is_empty() && !h.is_empty() && w.chars().chain(h.chars()).all(|c| c.is_ascii_digit()));
            if is_size {
                return format!("{head}-{SIZE}.{ext}");
            }
        }
    }
    url.to_string()
}

/// How well `name` answers `query` (both lowercase): exact, prefix, a word
/// starting with it, or anywhere. `None` when it does not match at all.
fn match_rank(name: &str, query: &str) -> Option<u8> {
    if name == query {
        Some(0)
    } else if name.starts_with(query) {
        Some(1)
    } else if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        Some(2)
    } else if name.contains(query) {
        Some(3)
    } else {
        None
    }
}

/// Merges the drop games (one entry per game, with its campaign count) and a
/// category search into one ranked list. An empty query lists every drop game
/// by name. Search results that repeat a drop game are folded into it.
pub fn rank(
    query: &str,
    drop_games: &[GameSuggestion],
    searched: Vec<GameSuggestion>,
    limit: usize,
) -> Vec<GameSuggestion> {
    let q = query.trim().to_lowercase();
    let mut matched: Vec<(u8, &GameSuggestion)> = drop_games
        .iter()
        .filter_map(|g| {
            if q.is_empty() {
                Some((0, g))
            } else {
                match_rank(&g.name.to_lowercase(), &q).map(|r| (r, g))
            }
        })
        .collect();
    matched.sort_by(|(ra, a), (rb, b)| ra.cmp(rb).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));

    let mut out: Vec<GameSuggestion> = matched.into_iter().map(|(_, g)| g.clone()).collect();
    let mut seen: HashSet<String> = out.iter().map(|g| g.name.to_lowercase()).collect();
    for g in searched {
        if seen.insert(g.name.to_lowercase()) {
            let drops = drop_games
                .iter()
                .find(|d| d.id == g.id || d.name.eq_ignore_ascii_case(&g.name))
                .map_or(0, |d| d.drop_campaigns);
            out.push(GameSuggestion { drop_campaigns: drops, ..g });
        }
    }
    out.truncate(limit);
    out
}

/// One entry per saved name, in order: the drop game it names, else the
/// Twitch category of exactly that name (`found`), else the bare name with no
/// id or art (a name Twitch does not know, such as a typo saved by an older
/// version).
pub fn describe(names: &[String], drop_games: &[GameSuggestion], found: &[GameSuggestion]) -> Vec<GameSuggestion> {
    names
        .iter()
        .map(|name| {
            let key = name.to_lowercase();
            drop_games
                .iter()
                .chain(found.iter())
                .find(|g| g.name.to_lowercase() == key)
                .cloned()
                .unwrap_or_else(|| GameSuggestion {
                    id: String::new(),
                    name: name.clone(),
                    box_art_url: String::new(),
                    drop_campaigns: 0,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: &str, name: &str, drops: u32) -> GameSuggestion {
        GameSuggestion { id: id.into(), name: name.into(), box_art_url: String::new(), drop_campaigns: drops }
    }

    #[test]
    fn box_art_is_resized_whatever_shape_twitch_hands_out() {
        assert_eq!(
            sized_box_art("https://static-cdn.jtvnw.net/ttv-boxart/32982_IGDB-{width}x{height}.jpg"),
            "https://static-cdn.jtvnw.net/ttv-boxart/32982_IGDB-144x192.jpg"
        );
        assert_eq!(
            sized_box_art("https://static-cdn.jtvnw.net/ttv-boxart/1234-52x72.jpg"),
            "https://static-cdn.jtvnw.net/ttv-boxart/1234-144x192.jpg"
        );
        assert_eq!(
            sized_box_art("https://static-cdn.jtvnw.net/ttv-boxart/Some-Game_IGDB-285x380.jpg"),
            "https://static-cdn.jtvnw.net/ttv-boxart/Some-Game_IGDB-144x192.jpg"
        );
        assert_eq!(sized_box_art("https://x.example/art.png"), "https://x.example/art.png");
    }

    #[test]
    fn drop_games_come_first_ranked_by_how_well_they_match() {
        let drops = vec![
            game("1", "HITMAN World of Assassination", 2),
            game("2", "Hytale", 1),
            game("3", "Shadow Hit", 1),
            game("4", "Rust", 3),
        ];
        let searched = vec![game("9", "Hitman 3", 0), game("1", "HITMAN World of Assassination", 0)];
        let names: Vec<String> = rank("hit", &drops, searched, 10).into_iter().map(|g| g.name).collect();
        assert_eq!(names, vec!["HITMAN World of Assassination", "Shadow Hit", "Hitman 3"]);
    }

    #[test]
    fn a_search_result_that_is_a_drop_game_keeps_its_drop_count() {
        // Twitch's search can find a drop game the name match missed (an alias).
        let drops = vec![game("1", "Rust", 3)];
        let out = rank("zzz", &drops, vec![game("1", "Rust", 0)], 10);
        assert_eq!(out[0].drop_campaigns, 3);
    }

    #[test]
    fn an_empty_query_lists_drop_games_by_name() {
        let drops = vec![game("2", "Rust", 1), game("1", "ARC Raiders", 1)];
        let names: Vec<String> = rank("  ", &drops, Vec::new(), 10).into_iter().map(|g| g.name).collect();
        assert_eq!(names, vec!["ARC Raiders", "Rust"]);
    }

    #[test]
    fn saved_names_resolve_to_drop_games_then_categories_then_bare() {
        let drops = vec![game("1", "Rust", 2)];
        let found = vec![game("7", "Hitman 3", 0)];
        let names = vec!["rust".to_string(), "Hitman 3".to_string(), "Hitmn".to_string()];
        let out = describe(&names, &drops, &found);
        assert_eq!((out[0].id.as_str(), out[0].drop_campaigns), ("1", 2));
        assert_eq!(out[1].id, "7");
        assert_eq!((out[2].id.as_str(), out[2].name.as_str()), ("", "Hitmn"));
    }

    #[test]
    fn the_limit_holds() {
        let drops: Vec<GameSuggestion> = (0..20).map(|i| game(&i.to_string(), &format!("Game {i}"), 1)).collect();
        assert_eq!(rank("game", &drops, Vec::new(), 8).len(), 8);
    }
}
