use super::*;
use crate::services::recent_emotes::{remember, RecentEmote};
use crate::services::twitch_smilies::{catalog_rows, face_row};
use std::collections::VecDeque;

fn monkey_room() -> EmoteSet {
    let mut set = EmoteSet::new();
    set.twitch = catalog_rows("monkeys").unwrap();
    let mut kappa = face_row("25", "Kappa");
    kappa.emote_type = Some("globals".to_string());
    set.twitch.push(kappa);
    set
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn context<'a>(
    favorites: &'a [String],
    recent: &'a [String],
    names: &'a HashMap<String, String>,
) -> Context<'a> {
    Context {
        room: Room::Twitch,
        channel_id: None,
        favorites,
        recent,
        ffz_subwoofer: false,
        owner_names: names,
        personal: None,
    }
}

fn ids_of(section: &PickerSection) -> Vec<&str> {
    section.emotes.iter().map(|e| e.emote.id.as_str()).collect()
}

#[test]
fn favorites_and_recents_follow_the_face() {
    let room = monkey_room();
    let names = HashMap::new();
    // Stored while the account wore the robots and the purple set; the room wears the monkeys now.
    let favorites = strings(&["9", "445", "25"]);
    let recent = strings(&["1", "499", "25"]);
    let ctx = context(&favorites, &recent, &names);

    let answer = build(Some(&room), &ctx, Some(Tab::Favorites), "");
    let section = |key: &str| {
        answer
            .sections
            .iter()
            .find(|s| s.key == key)
            .unwrap_or_else(|| panic!("no {key} section"))
    };
    // Both `:)` ids are the room's `:)`, listed once, in the order first used.
    assert_eq!(ids_of(section("recent")), ["499", "25"]);
    // Both `<3` ids are the room's `<3`, starred once.
    let starred = section("standard");
    assert_eq!(ids_of(starred), ["483", "25"]);
    assert!(starred.emotes.iter().all(|e| e.favorite));
    let counts: Vec<usize> = answer.tabs.iter().map(|t| t.count).collect();
    assert_eq!(counts[0], 2, "the Favorites tab counts the resolved faces");

    // In the Twitch tab the face is starred and its neighbors are not.
    let twitch = build(Some(&room), &ctx, Some(Tab::Twitch), "");
    let cells: Vec<&PickerEmote> = twitch
        .sections
        .iter()
        .flat_map(|s| s.emotes.iter())
        .collect();
    let flag = |id: &str| {
        cells
            .iter()
            .find(|c| c.emote.id == id)
            .unwrap_or_else(|| panic!("no {id}"))
            .favorite
    };
    assert!(flag("483") && flag("25"));
    assert!(!flag("499") && !flag("484"));

    // A robot `:(` stored long ago is the monkeys' `:(` here.
    let old = strings(&["2"]);
    let old_ctx = context(&old, &old, &names);
    let sad = build(Some(&room), &old_ctx, Some(Tab::Favorites), "");
    assert_eq!(
        ids_of(sad.sections.iter().find(|s| s.key == "recent").unwrap()),
        ["489"]
    );

    // A face this room does not have stays under its stored id and is simply not in the room.
    let mut robots = EmoteSet::new();
    robots.twitch = catalog_rows("robots").unwrap();
    let hash = strings(&["485"]);
    let hash_ctx = context(&hash, &hash, &names);
    let none = build(Some(&robots), &hash_ctx, Some(Tab::Favorites), "");
    assert!(none.sections.iter().all(|s| s.key != "recent"));
    assert_eq!(none.tabs[0].count, 0);
}

#[test]
fn unstarring_a_face_removes_every_stored_id_for_it() {
    let mut stored: Vec<serde_json::Value> = vec![
        serde_json::json!({ "id": "9", "name": "<3" }),
        serde_json::json!({ "id": "445", "name": "<3" }),
        serde_json::json!({ "id": "483", "name": "<3" }),
        serde_json::json!({ "id": "1", "name": ":)" }),
        serde_json::json!({ "id": "25", "name": "Kappa" }),
        serde_json::json!({ "name": "no id" }),
    ];
    crate::services::cache_service::drop_favorite(&mut stored, "483");
    let left: Vec<Option<&str>> = stored
        .iter()
        .map(|e| e.get("id").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(left, [Some("1"), Some("25"), None]);
    crate::services::cache_service::drop_favorite(&mut stored, "25");
    assert_eq!(stored.len(), 2, "an ordinary emote goes by its own id only");
}

#[test]
fn a_sent_face_replaces_the_stored_ones_for_the_same_face() {
    let recent = |id: &str, name: &str| RecentEmote {
        id: id.to_string(),
        name: name.to_string(),
    };
    let mut list: VecDeque<RecentEmote> = VecDeque::from(vec![
        recent("9", "<3"),
        recent("1", ":)"),
        recent("25", "Kappa"),
    ]);
    remember(&mut list, vec![recent("483", "<3")]);
    let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        ["483", "1", "25"],
        "the earlier `<3` of another set is dropped, the `:)` stays"
    );
    remember(&mut list, vec![recent("499", ":)")]);
    let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["499", "483", "25"]);
}
