use super::*;
use crate::services::twitch_smiley_style::{begin_test, test_wear, TEST_LOCK};

fn row(provider: EmoteProvider, id: &str, name: &str, kind: Option<&str>) -> Emote {
    let mut e = face_row(id, name);
    e.provider = provider;
    e.emote_type = kind.map(str::to_string);
    e
}

fn kappa() -> Emote {
    row(EmoteProvider::Twitch, "25", "Kappa", Some("globals"))
}

fn robots() -> Vec<Emote> {
    catalog_rows("robots").expect("robots")
}

fn monkeys_swap() -> SmileySwap {
    swap_to(catalog_rows("monkeys").expect("monkeys"))
}

fn ids(list: &[Emote]) -> Vec<&str> {
    list.iter().map(|e| e.id.as_str()).collect()
}

#[test]
fn catalog_matches_twitch_sets() {
    let counts: Vec<usize> = SETS.iter().map(|s| s.faces.len()).collect();
    assert_eq!(counts, [14, 14, 19]);
    let keys: Vec<&str> = SETS.iter().map(|s| s.key).collect();
    assert_eq!(keys, ["robots", "purple", "monkeys"]);
    let first_ids: Vec<&str> = SETS.iter().map(|s| s.faces[0].0).collect();
    assert_eq!(first_ids, ["1", "432", "483"]);
    let last_ids: Vec<&str> = SETS.iter().map(|s| s.faces[s.faces.len() - 1].0).collect();
    assert_eq!(last_ids, ["14", "445", "501"]);
    // Twitch's ids run unbroken inside each set.
    for set in SETS {
        let numbers: Vec<u32> = set
            .faces
            .iter()
            .map(|(id, _)| id.parse().unwrap())
            .collect();
        assert!(numbers.windows(2).all(|w| w[1] == w[0] + 1), "{}", set.key);
    }
    // Every id belongs to one set only.
    let mut all: Vec<&str> = SETS
        .iter()
        .flat_map(|s| s.faces.iter().map(|(id, _)| *id))
        .collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 47);
    // The faces every set shares, and the ones only the monkeys have.
    let faces_of = |key: &str| -> Vec<&'static str> {
        catalog_set(key)
            .unwrap()
            .faces
            .iter()
            .map(|(_, f)| *f)
            .collect()
    };
    let mut shared = faces_of("robots");
    shared.sort();
    let mut purple = faces_of("purple");
    purple.sort();
    assert_eq!(shared, purple);
    let monkeys = faces_of("monkeys");
    assert!(shared.iter().all(|f| monkeys.contains(f)));
    for extra in ["#/", ":>", "<]", ":7", ":s"] {
        assert!(
            monkeys.contains(&extra) && !shared.contains(&extra),
            "{extra}"
        );
    }
    // A face reads the same whether it came as the face or went through the pattern reader.
    for set in SETS {
        for (id, face) in set.faces {
            assert_eq!(
                display_code(face).unwrap_or_else(|| face.to_string()),
                *face,
                "{id}"
            );
            assert_eq!(
                emote_url(id),
                format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0")
            );
        }
    }
    // Every row is a smilies row of the Twitch provider.
    for e in robots() {
        assert_eq!(
            (e.provider, e.emote_type.as_deref()),
            (EmoteProvider::Twitch, Some("smilies"))
        );
    }
}

#[test]
fn catalog_lookups() {
    assert_eq!(catalog_key_of("1"), Some("robots"));
    assert_eq!(catalog_key_of("445"), Some("purple"));
    assert_eq!(catalog_key_of("501"), Some("monkeys"));
    assert_eq!(catalog_key_of("25"), None);
    assert_eq!(face_of("485"), Some("#/"));
    assert_eq!(face_of("25"), None);
    assert!(same_face("9", "445") && same_face("445", "483"));
    assert!(!same_face("9", "440"), "`<3` is not `:)`");
    assert!(!same_face("25", "25"), "only smilies have a face");
    assert_eq!(
        catalog_key_for_ids(["483", "484", "9"].into_iter()),
        Some("monkeys")
    );
    assert_eq!(catalog_key_for_ids(["9000", "9001"].into_iter()), None);
    assert!(catalog_rows("nonsense").is_none());
}

#[test]
fn apply_swap_leaves_a_kick_like_list_alone() {
    // A table with no Twitch smilies: Kick native rows and FFZ rows that share a number.
    let mut list = vec![
        row(EmoteProvider::Kick, "440", "KickFace", Some("Emotes")),
        row(EmoteProvider::FFZ, "1", "ffzOne", None),
    ];
    let before = ids(&list).join(",");
    assert!(!apply_swap(&mut list, &monkeys_swap()));
    assert_eq!(ids(&list).join(","), before);
}

#[test]
fn apply_swap_leaves_the_signed_out_fallback_alone() {
    let mut list = vec![kappa(), row(EmoteProvider::Twitch, "354", "4Head", None)];
    assert!(!apply_swap(&mut list, &monkeys_swap()));
    assert_eq!(ids(&list), ["25", "354"]);
}

#[test]
fn apply_swap_keeps_kappa_and_ffz_numeric_id_440() {
    let mut list = robots();
    list.push(kappa());
    list.push(row(EmoteProvider::FFZ, "440", "ffzSame", None));
    list.push(row(EmoteProvider::SevenTV, "9", "stvNine", None));
    assert!(apply_swap(&mut list, &monkeys_swap()));
    let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"Kappa") && names.contains(&"ffzSame") && names.contains(&"stvNine"));
    // The robot faces are gone and the monkey faces are in.
    assert!(!list
        .iter()
        .any(|e| e.provider == EmoteProvider::Twitch && catalog_key_of(&e.id) == Some("robots")));
    assert_eq!(
        list.iter()
            .filter(
                |e| e.provider == EmoteProvider::Twitch && catalog_key_of(&e.id) == Some("monkeys")
            )
            .count(),
        19
    );
}

#[test]
fn apply_swap_drops_other_sets_and_stale_smilies() {
    let mut list = robots();
    list.extend(catalog_rows("purple").unwrap());
    // A smiley of a set nobody knows is dropped too: only the current set's faces remain.
    list.push(row(EmoteProvider::Twitch, "9999", ":)", Some("smilies")));
    list.push(kappa());
    apply_swap(&mut list, &monkeys_swap());
    let smilies: Vec<&str> = list
        .iter()
        .filter(|e| is_smiley(e))
        .map(|e| e.id.as_str())
        .collect();
    let expected: Vec<String> = (483..=501).map(|n| n.to_string()).collect();
    assert_eq!(
        smilies,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(list.iter().any(|e| e.name == "Kappa"));
}

#[test]
fn apply_swap_is_idempotent() {
    let mut list = robots();
    list.push(kappa());
    let swap = monkeys_swap();
    assert!(apply_swap(&mut list, &swap));
    let once = ids(&list).join(",");
    assert!(
        !apply_swap(&mut list, &swap),
        "nothing changed the second time"
    );
    assert_eq!(ids(&list).join(","), once);
}

#[tokio::test]
async fn the_gate_keeps_other_tests_from_seeing_a_swap() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    test_wear("42", "monkeys");
    let mut dressed = robots();
    wear_own(&mut dressed);
    assert_eq!(ids(&dressed).len(), 19);
    assert!(dressed
        .iter()
        .all(|e| catalog_key_of(&e.id) == Some("monkeys")));
    // On a thread that did not ask, no swap exists and a list is left as it is.
    set_testing(false);
    assert!(own_swap().is_none());
    let mut other = robots();
    wear_own(&mut other);
    assert_eq!(ids(&other).len(), 14);
    assert_eq!(catalog_key_of(&other[0].id), Some("robots"));
}

#[test]
fn a_stored_id_resolves_to_the_same_face_in_the_room() {
    let mut room = catalog_rows("monkeys").unwrap();
    room.push(kappa());
    assert_eq!(
        resolve_in(&room, "9"),
        "483",
        "a robot `<3` is the monkey `<3`"
    );
    assert_eq!(resolve_in(&room, "445"), "483");
    assert_eq!(resolve_in(&room, "483"), "483");
    assert_eq!(resolve_in(&room, "25"), "25", "an ordinary emote stays");
    assert_eq!(resolve_in(&room, "77"), "77", "an id nobody knows stays");
    // A face this room lacks keeps its stored id.
    assert_eq!(resolve_in(&robots(), "485"), "485");
    // Lists dedupe on the resolved id, keeping the order.
    let stored: Vec<String> = ["9", "25", "445", "483", "1"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(resolve_ids(&room, &stored), ["483", "25", "499"]);
    // With no smilies in the room, nothing changes but duplicates.
    let plain = vec![kappa()];
    assert_eq!(resolve_ids(&plain, &stored), stored);
}

#[test]
fn a_starred_smiley_ranks_as_the_same_face_in_every_set() {
    let stored: HashSet<String> = ["9", "25"].iter().map(|s| s.to_string()).collect();
    let by_face = favorites_by_face(&stored).expect("a smiley is starred");
    // The robot `<3` stars the purple and the monkey `<3` too.
    for id in ["9", "445", "483", "25"] {
        assert!(by_face.contains(id), "{id} ranks as a favorite");
    }
    // Only that face: the monkey `:)` is not starred.
    assert!(!by_face.contains("499"));
    assert_eq!(by_face.len(), 4);
    // A list without a smiley is used as it is.
    let plain: HashSet<String> = ["25", "88"].iter().map(|s| s.to_string()).collect();
    assert!(favorites_by_face(&plain).is_none());
}
