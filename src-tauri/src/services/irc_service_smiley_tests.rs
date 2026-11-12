use super::*;
use crate::services::emote_service::EmoteProvider;
use crate::services::emote_service::HelixOutcome;
use crate::services::twitch_service::TwitchService;
use crate::services::twitch_smiley_style::{
    after_account_moved_with, apply_everywhere_with, begin_test, note_helix, on_identity_in,
    test_wear, write_own, HelixReader, TEST_LOCK,
};
use crate::services::twitch_smilies::{catalog_key_of, catalog_rows, read_own, swap_to};

fn kappa() -> Emote {
    let mut e = crate::services::twitch_smilies::face_row("25", "Kappa");
    e.emote_type = Some("globals".to_string());
    e
}

fn robot_table() -> EmoteSet {
    let mut set = EmoteSet::new();
    set.twitch = catalog_rows("robots").unwrap();
    set.twitch.push(kappa());
    set
}

async fn hold(key: &str, set: EmoteSet) {
    rebuild_parse_lookup(key, &set);
    get_channel_emotes()
        .lock()
        .await
        .insert(key.to_string(), set);
}

async fn held_ids(key: &str) -> Vec<String> {
    get_channel_emotes()
        .lock()
        .await
        .get(key)
        .map(|s| s.twitch.iter().map(|e| e.id.clone()).collect())
        .unwrap_or_default()
}

async fn forget(key: &str) {
    get_channel_emotes().lock().await.remove(key);
    drop_parse_lookup(key);
}

fn has_robot_or_purple_faces(ids: &[String]) -> bool {
    ids.iter()
        .any(|id| matches!(catalog_key_of(id), Some("robots") | Some("purple")))
}

#[tokio::test]
async fn a_table_installed_after_a_switch_wears_the_switched_set() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let key = "smiley_install_chan";
    // A fetch began while the account wore the robots; the switch lands before it installs.
    test_wear("42", "monkeys");
    let installed =
        IrcService::install_fetched_set(key, "smiley-install-id", 0, robot_table(), true, false)
            .await;
    assert!(installed);
    let ids = held_ids(key).await;
    assert!(
        ids.contains(&"499".to_string()) && ids.contains(&"483".to_string()),
        "{ids:?}"
    );
    assert!(!has_robot_or_purple_faces(&ids), "{ids:?}");
    assert!(ids.contains(&"25".to_string()), "Kappa stays");
    // The parse table the install built holds the monkey faces too.
    let lookup = parse_lookup_for(key).expect("a parse table");
    assert_eq!(
        lookup
            .natives
            .iter()
            .filter(|e| e.name == ":)")
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        ["499"]
    );
    forget(key).await;
}

#[tokio::test]
async fn a_sent_smiley_wears_the_switched_set() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let key = "smiley_sent_chan";
    // The room's table was installed with the robot faces before the switch.
    hold(key, robot_table()).await;
    test_wear("42", "monkeys");
    replace_own_smilies().await;

    let own = OwnMessage {
        channel: format!("#{key}"),
        text: ":) <3".into(),
        local_id: "local-smiley".into(),
        sender_id: "42".into(),
        sender_login: "me".into(),
        sender_display_name: "Me".into(),
        color: String::new(),
        badges: String::new(),
        room_id: "7".into(),
        reply_to: None,
    };
    let msg = IrcService::build_own_message(own).await.expect("a row");
    let mut tagged: Vec<&str> = msg.emotes.iter().map(|e| e.id.as_str()).collect();
    tagged.sort();
    assert_eq!(tagged, ["483", "499"], "the monkeys' `<3` and `:)`");
    forget(key).await;
}

#[tokio::test]
async fn tab_lists_the_switched_faces() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let key = "smiley_tab_chan";
    hold(key, robot_table()).await;
    test_wear("42", "purple");
    replace_own_smilies().await;
    let names: Vec<(String, String)> = with_channel_emotes(key, |set| {
        set.twitch
            .iter()
            .filter(|e| e.emote_type.as_deref() == Some("smilies"))
            .map(|e| (e.id.clone(), e.name.clone()))
            .collect()
    })
    .await
    .expect("the table");
    assert_eq!(names.len(), 14);
    assert!(names
        .iter()
        .all(|(id, _)| catalog_key_of(id) == Some("purple")));
    assert!(names.contains(&("440".to_string(), ":)".to_string())));
    let table = with_channel_emotes(key, |set| set.twitch.clone())
        .await
        .unwrap();
    let positions = native_emote_positions(":) ;)", &table);
    assert_eq!(
        positions.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["440", "439"]
    );
    forget(key).await;
}

#[tokio::test]
async fn replace_own_smilies_leaves_other_tables_alone() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let (kick, plain) = ("smiley_kick_chan", "smiley_plain_chan");
    let mut kick_table = EmoteSet::new();
    let mut face = crate::services::twitch_smilies::face_row("440", "KickFace");
    face.provider = EmoteProvider::Kick;
    kick_table.kick.push(face);
    hold(kick, kick_table).await;
    let mut plain_table = EmoteSet::new();
    plain_table.twitch.push(kappa());
    hold(plain, plain_table).await;
    test_wear("42", "monkeys");
    replace_own_smilies().await;
    assert!(held_ids(kick).await.is_empty());
    assert_eq!(held_ids(plain).await, ["25"]);
    forget(kick).await;
    forget(plain).await;
}

/// Files the new account's robot faces, as a Helix answer for it would.
struct FilesRobots;

#[async_trait::async_trait]
impl HelixReader for FilesRobots {
    async fn token(&self) -> Option<String> {
        Some("token-b".to_string())
    }

    async fn fetch(&self, _token: &str) -> HelixOutcome {
        note_helix("B", 1, catalog_rows("robots").unwrap());
        HelixOutcome::Answered
    }
}

#[tokio::test]
async fn account_moved_replaces_old_faces_with_the_new_accounts_helix_set() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let key = "smiley_moved_chan";
    let service = EmoteService::new();

    // Account A wears the monkeys in the room's table and in the cache.
    test_wear("A", "monkeys");
    let mut monkey_table = EmoteSet::new();
    monkey_table.twitch = catalog_rows("monkeys").unwrap();
    hold(key, monkey_table.clone()).await;
    service.test_cache_set("c1", monkey_table).await;
    assert!(held_ids(key).await.contains(&"483".to_string()));

    // A signs out and B signs in; B's Helix answer lists the robots.
    TwitchService::bump_account_generation();
    assert!(read_own().identity.is_none());
    let fence = read_own().gen;
    write_own(|own, g| on_identity_in(own, g, "B", g, None));
    after_account_moved_with(fence, Some(&service), &FilesRobots, &|_| {
        std::time::Duration::ZERO
    })
    .await;
    apply_everywhere_with(Some(&service)).await;

    let held = held_ids(key).await;
    assert!(
        held.iter().all(|id| catalog_key_of(id) != Some("monkeys")),
        "{held:?}"
    );
    assert!(
        held.contains(&"1".to_string()) && held.contains(&"9".to_string()),
        "{held:?}"
    );
    assert!(
        service.test_cached_ids("c1").await.is_empty(),
        "the cache was cleared for the new account"
    );
    assert_eq!(
        crate::services::twitch_smiley_style::own_smilies().swap,
        Some(swap_to(catalog_rows("robots").unwrap()))
    );
    forget(key).await;
}
