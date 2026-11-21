//! Tests of the pin row owner (SN-284): row parity with the live chat row over
//! `tests/fixtures/pin-rows.json`, snapshot shapes, badge retries, the remote
//! line, and the private emote table.

use super::*;
use crate::models::chat_layout::MessageSegment;
use crate::services::chat_row_view as view;
use crate::services::emote_service::EmoteSet;
use crate::services::mention_inbox::MentionKind;
use serde_json::{json, Value};

/// The tests share the badge index, the parse tables and the owner.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Deserialize)]
struct Room {
    login: String,
    id: String,
    partner_id: String,
}

#[derive(Deserialize)]
struct ArtIn {
    set: String,
    version: String,
    x1: String,
    x2: String,
    x4: String,
    title: String,
}

#[derive(Deserialize)]
struct Badges {
    global: Vec<ArtIn>,
    room: Vec<ArtIn>,
    partner: Vec<ArtIn>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    kind: String,
    expect_snapshot: String,
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    message: Option<Value>,
    #[serde(default)]
    mention: Option<Value>,
}

#[derive(Deserialize)]
struct Fixture {
    room: Room,
    emote_set: EmoteSet,
    badges: Badges,
    cases: Vec<Case>,
}

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("pin-rows.json")
}

fn load_fixture() -> (Fixture, Value) {
    let text = std::fs::read_to_string(fixture_path()).expect("tests/fixtures/pin-rows.json");
    (serde_json::from_str(&text).expect("fixture parses"), serde_json::from_str(&text).unwrap())
}

fn note_badges(room: Option<&str>, list: &[ArtIn]) {
    view::note_twitch_badge_art(
        room,
        list.iter().map(|a| {
            (
                a.set.as_str(),
                a.version.as_str(),
                view::BadgeArt { x1: a.x1.clone(), x2: a.x2.clone(), x4: a.x4.clone(), title: a.title.clone() },
            )
        }),
    );
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

/// What a pin row and the live row may differ in, by design: the badges' art
/// and title (the web resolves them from its own cache, the pin carries
/// Rust's), the flag saying so, and link previews (a pin draws none).
fn without_declared_differences(mut row: Value) -> Value {
    if let Some(badges) = row.get_mut("badges").and_then(Value::as_array_mut) {
        for badge in badges {
            if let Some(o) = badge.as_object_mut() {
                for key in ["image_url_1x", "image_url_2x", "image_url_4x", "title", "description"] {
                    o.remove(key);
                }
            }
        }
    }
    if let Some(o) = row.as_object_mut() {
        o.remove("badges_resolved");
    }
    if let Some(view) = row.pointer_mut("/metadata/view").and_then(Value::as_object_mut) {
        view.remove("previews");
    }
    row
}

fn entry_of(msg: &ChatMessage) -> MentionEntry {
    crate::services::mention_inbox::entry_for(msg, MentionKind::Mention, 1_700_000_000_000)
}

/// The live chat row, built as the chat builds it: balance the bidi controls, stamp the view.
fn live_row(msg: &ChatMessage) -> Value {
    let mut live = msg.clone();
    crate::services::bidi_text::balance_row(&mut live);
    view::stamp_view(&mut live);
    serde_json::to_value(&live).unwrap()
}

/// The pin row built from a stored snapshot.
fn pin_row_from(entry: &MentionEntry, raw: &RawValue) -> (PinRow, Needs) {
    let snap: Snap = serde_json::from_str(raw.get()).expect("snapshot decodes");
    build_row(entry, snap, true)
}

#[test]
fn a_pin_row_equals_the_live_chat_row_over_the_fixture() {
    let _g = serial();
    reset_for_test();
    let (fx, mut raw_fx) = load_fixture();
    assert!(!fx.cases.is_empty(), "the fixture holds cases");
    let (login, room_id) = (fx.room.login.clone(), fx.room.id.clone());
    note_badges(None, &fx.badges.global);
    note_badges(Some(&room_id), &fx.badges.room);
    note_badges(Some(&fx.room.partner_id), &fx.badges.partner);
    mark_badges_answered_for_test(&room_id);
    mark_badges_answered_for_test(&fx.room.partner_id);
    let update = std::env::var("UPDATE_GOLDEN").is_ok_and(|v| v == "1");
    let mut goldens: Vec<Value> = Vec::new();
    for case in &fx.cases {
        // The chat is joined while the message arrives: its table is the live parse's.
        IrcService::install_parse_lookup_for_test(&login, &fx.emote_set);
        let (live, snapshot, entry) = match case.kind.as_str() {
            "twitch" => {
                let raw = case.raw.clone().unwrap();
                let msg = IrcService::parse_live_line(&raw).expect("the line parses");
                (live_row(&msg), snapshot_of(&msg, Some(&raw)), entry_of(&msg))
            }
            "message" => {
                let msg: ChatMessage = serde_json::from_value(case.message.clone().unwrap()).expect("message");
                (live_row(&msg), snapshot_of(&msg, None), entry_of(&msg))
            }
            "remote" => {
                let mention: crate::services::mention_watch::RemoteMention = serde_json::from_value(case.mention.clone().unwrap()).unwrap();
                let extra = mention.extra();
                let entry = mention.into_entry().expect("a remote entry");
                let raw = extra.raw.expect("raw body and tags");
                // The live equivalent: the same line the chat would have read, with every tag.
                let full = format!(
                    "@{} :{}!{}@{}.tmi.twitch.tv PRIVMSG #{} :{}",
                    raw.tags_raw, entry.sender_login, entry.sender_login, entry.sender_login, entry.channel_login, raw.body_raw
                );
                let msg = IrcService::parse_live_line(&full).expect("the line parses");
                let mut live = live_row(&msg);
                for dropped in DROPPED_TAGS {
                    live["tags"].as_object_mut().unwrap().remove(dropped);
                }
                (live, runtime().block_on(remote_snapshot(&entry, &raw)), entry)
            }
            other => panic!("unknown case kind {other}"),
        };
        crate::services::shared_chat_emotes::reset_for_test();
        // Now the chat is not joined here: the pin uses a private table, as it does for any other chat.
        IrcService::remove_parse_lookup_for_test(&login);
        install_table_for_test(&login, EmoteLookup::build(&fx.emote_set));
        let kind = match &snapshot {
            None => "none",
            Some(r) if r.get().contains("\"lean\":true") => "lean",
            Some(_) => "full",
        };
        assert_eq!(kind, case.expect_snapshot, "{}: the snapshot shape", case.name);
        let (pin, needs) = match &snapshot {
            Some(raw) => pin_row_from(&entry, raw),
            None => (text_row(&entry), Needs::default()),
        };
        if snapshot.is_some() {
            assert!(
                needs.badge_rooms.is_empty() && !needs.global_badges && needs.table.is_none(),
                "{}: everything the row needs is loaded: {needs:?}",
                case.name
            );
            assert_eq!(
                without_declared_differences(pin.row.clone()),
                without_declared_differences(live.clone()),
                "{}: the pin row is the live chat row",
                case.name
            );
            if case.kind != "message" {
                assert_eq!(pin.row["badges_resolved"], json!(true), "{}: Twitch badges carry their art", case.name);
                for badge in pin.row["badges"].as_array().unwrap() {
                    for key in ["image_url_1x", "image_url_2x", "image_url_4x", "title"] {
                        assert!(badge[key].is_string(), "{}: {} has {key}", case.name, badge["name"]);
                    }
                }
            }
        }
        goldens.push(pin.row.clone());
    }
    if update {
        for (i, golden) in goldens.iter().enumerate() {
            raw_fx["cases"][i]["golden"] = golden.clone();
        }
        std::fs::write(fixture_path(), serde_json::to_string_pretty(&raw_fx).unwrap() + "\n").unwrap();
    }
    let stored: Value = serde_json::from_str(&std::fs::read_to_string(fixture_path()).unwrap()).unwrap();
    for (i, golden) in goldens.iter().enumerate() {
        assert_eq!(
            &stored["cases"][i]["golden"], golden,
            "{}: the row moved; regenerate with UPDATE_GOLDEN=1 and review the diff",
            fx.cases[i].name
        );
    }
    assert!(!goldens.is_empty());
    IrcService::remove_parse_lookup_for_test(&login);
    reset_for_test();
}

#[test]
fn emotes_in_the_pin_come_from_the_private_table_and_stack_zero_width_ones() {
    let _g = serial();
    reset_for_test();
    let (fx, _) = load_fixture();
    let case = fx.cases.iter().find(|c| c.name == "twitch_emotes_with_zero_width").unwrap();
    let raw = case.raw.clone().unwrap();
    // Chat not joined, no private table yet: the row says what it needs and shows the words.
    IrcService::remove_parse_lookup_for_test(&fx.room.login);
    let msg = IrcService::parse_live_line(&raw).unwrap();
    let snapshot = snapshot_of(&msg, Some(&raw)).unwrap();
    let entry = entry_of(&msg);
    let (row, needs) = pin_row_from(&entry, &snapshot);
    assert_eq!(needs.table, Some((fx.room.login.clone(), fx.room.id.clone())));
    let names = |row: &PinRow| -> Vec<String> {
        row.row["segments"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["type"] == "emote")
            .map(|s| s["content"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names(&row), vec!["Kappa".to_string()], "only Twitch's own emote before the table");
    install_table_for_test(&fx.room.login, EmoteLookup::build(&fx.emote_set));
    let (row, needs) = pin_row_from(&entry, &snapshot);
    assert!(needs.table.is_none());
    assert_eq!(names(&row), vec!["Kappa", "fixWave", "fixHat", "fixBttv"]);
    let hat = row.row["segments"].as_array().unwrap().iter().find(|s| s["content"] == "fixHat").unwrap();
    assert_eq!(hat["is_zero_width"], json!(true), "the zero-width flag rides the segment");
    reset_for_test();
}

#[test]
fn building_a_pin_row_leaves_no_trace_in_the_live_chat() {
    let _g = serial();
    reset_for_test();
    let (fx, _) = load_fixture();
    let case = fx.cases.iter().find(|c| c.name == "twitch_shared_chat_source_room").unwrap();
    let raw = case.raw.clone().unwrap();
    IrcService::install_parse_lookup_for_test(&fx.room.login, &fx.emote_set);
    let msg = IrcService::parse_live_line(&raw).unwrap();
    let snapshot = snapshot_of(&msg, Some(&raw)).unwrap();
    crate::services::shared_chat_emotes::reset_for_test();
    assert_eq!(crate::services::shared_chat_emotes::pending_for_test(&fx.room.partner_id), None);
    // A display name with spaces would be remembered by the live parse; the pin's build must not.
    let entry = entry_of(&msg);
    let _ = pin_row_from(&entry, &snapshot);
    assert_eq!(
        crate::services::shared_chat_emotes::pending_for_test(&fx.room.partner_id),
        None,
        "no partner lookup, no re-stamp queued"
    );
    IrcService::remove_parse_lookup_for_test(&fx.room.login);
    reset_for_test();
}

#[test]
fn the_parse_with_a_given_context_teaches_the_spaced_name_memory_nothing() {
    let raw = "@display-name=Two\\sWords;id=sp-1;room-id=1;user-id=9 :twowords!twowords@twowords.tmi.twitch.tv PRIVMSG #fixroom :hello";
    let snaps = ParseSnapshots::for_pin(None, "9", None);
    let msg = IrcService::parse_privmsg_with(raw, Some(&snaps)).unwrap();
    assert_eq!(msg.display_name, "Two Words");
    let src = include_str!("irc_service.rs");
    let body = &src[src.find("pub(crate) fn parse_privmsg_with").unwrap()..];
    let body = &body[..body.find("fn parse_usernotice").unwrap()];
    assert!(body.contains("if given.is_none() {\r\n            crate::services::chatter_name::note_spaced") || body.contains("if given.is_none() {\n            crate::services::chatter_name::note_spaced"), "note_spaced only without a given context");
    assert!(body.matches("note_spaced").count() == 2, "both calls are behind the guard");
    assert!(body.contains("None => {") && body.contains("lookup_for(&privmsg_channel"), "the partner lookup only without a given context");
}

#[test]
fn a_snapshot_keeps_every_tag_and_the_lean_one_drops_only_the_segments() {
    let _g = serial();
    let (fx, _) = load_fixture();
    // The oversize message is sixty emotes of the chat's own table: it needs the table to be long.
    IrcService::install_parse_lookup_for_test(&fx.room.login, &fx.emote_set);
    let case = fx.cases.iter().find(|c| c.name == "twitch_plain").unwrap();
    let raw = case.raw.clone().unwrap();
    let msg = IrcService::parse_live_line(&raw).unwrap();
    let snap: Snap = serde_json::from_str(snapshot_of(&msg, Some(&raw)).unwrap().get()).unwrap();
    assert_eq!(snap.msg.tags, msg.tags, "every tag kept");
    assert_eq!(snap.raw.as_deref(), Some(raw.as_str()));
    assert!(!snap.lean && !snap.msg.segments.is_empty());
    let big = fx.cases.iter().find(|c| c.name == "twitch_oversize_goes_lean").unwrap();
    let raw = big.raw.clone().unwrap();
    let msg = IrcService::parse_live_line(&raw).unwrap();
    let shot = snapshot_of(&msg, Some(&raw)).unwrap();
    assert!(shot.get().len() <= SNAPSHOT_CAP);
    let lean: Snap = serde_json::from_str(shot.get()).unwrap();
    assert!(lean.lean && lean.msg.segments.is_empty());
    assert_eq!(lean.msg.tags, msg.tags, "a lean snapshot still keeps every tag");
    IrcService::remove_parse_lookup_for_test(&fx.room.login);
}

#[test]
fn the_dropped_tag_list_is_the_fixture_every_renderer_test_reads() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("pin-dropped-tags.json");
    let fresh = json!({ "dropped": DROPPED_TAGS, "note": "Tags a remote pin snapshot leaves out. No renderer may read one." });
    if std::env::var("UPDATE_GOLDEN").is_ok_and(|v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&fresh).unwrap() + "\n").unwrap();
    }
    let stored: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("run once with UPDATE_GOLDEN=1")).unwrap();
    assert_eq!(stored, fresh, "the dropped tags moved; regenerate with UPDATE_GOLDEN=1 and review");
    assert!(!stored["dropped"].as_array().unwrap().is_empty());
}

#[test]
fn a_remote_line_drops_two_tags_fills_missing_ones_and_wraps_a_me() {
    let mention: crate::services::mention_watch::RemoteMention = serde_json::from_value(json!({
        "id": "r1", "room": "fixroom", "channel_id": "424242", "kind": "mention", "sender_id": "9", "sender_login": "viewer",
        "sender_name": "Two Words", "text": "waves", "at_ms": 1700000000123i64, "tracked": true,
        "body_raw": "waves", "tags_raw": "@client-nonce=abc;flags=0-1:A.3;color=#112233;emotes=", "action": true
    }))
    .unwrap();
    let extra = mention.extra();
    let entry = mention.into_entry().unwrap();
    let line = remote_line(&entry, &extra.raw.unwrap());
    assert!(!line.contains("client-nonce") && !line.contains("flags="));
    for filled in ["id=r1", "user-id=9", "display-name=Two\\sWords", "tmi-sent-ts=1700000000123", "room-id=424242", "color=#112233"] {
        assert!(line.contains(filled), "{filled} in {line}");
    }
    assert!(line.ends_with("PRIVMSG #fixroom :\u{1}ACTION waves\u{1}"), "{line}");
    let msg = IrcService::parse_privmsg_with(&line, Some(&ParseSnapshots::for_pin(None, "9", None))).unwrap();
    assert!(msg.metadata.is_action);
    assert_eq!(msg.content, "waves");
    // Without the raw body and tags nothing is built: the remote row never pins.
    assert_eq!(mention_without_raw().extra().raw, None);
}

fn mention_without_raw() -> crate::services::mention_watch::RemoteMention {
    serde_json::from_value(json!({
        "id": "r2", "room": "fixroom", "kind": "mention", "sender_id": "9", "sender_login": "viewer",
        "text": "hi", "at_ms": 1, "tracked": true
    }))
    .unwrap()
}

#[test]
fn a_failed_badge_read_is_tried_again_after_30_s_2_min_then_10_min_one_room_at_a_time() {
    let t0 = Instant::now();
    let mut book = BadgeBook::default();
    assert!(book.should_fetch("42", t0), "never read: read it");
    book.start("42");
    assert!(!book.should_fetch("42", t0), "one in flight");
    book.finish("42", false, t0);
    assert!(!book.should_fetch("42", t0 + Duration::from_secs(29)));
    assert!(book.should_fetch("42", t0 + Duration::from_secs(30)));
    book.start("42");
    book.finish("42", false, t0 + Duration::from_secs(30));
    assert!(!book.should_fetch("42", t0 + Duration::from_secs(30 + 119)));
    assert!(book.should_fetch("42", t0 + Duration::from_secs(30 + 120)));
    book.start("42");
    book.finish("42", false, t0 + Duration::from_secs(150));
    assert!(!book.should_fetch("42", t0 + Duration::from_secs(150 + 599)));
    assert!(book.should_fetch("42", t0 + Duration::from_secs(150 + 600)));
    book.start("42");
    book.finish("42", false, t0 + Duration::from_secs(750));
    assert!(book.should_fetch("42", t0 + Duration::from_secs(750 + 600)), "the last wait repeats");
    assert!(!book.answered("42"));
    // An answer, even an empty one, is final.
    book.start("42");
    book.finish("42", true, t0 + Duration::from_secs(2000));
    assert!(book.answered("42") && !book.should_fetch("42", t0 + Duration::from_secs(99_999)));
    // The worker reads one room at a time.
    let src = include_str!("mention_pin_row.rs");
    let worker = &src[src.find("async fn badge_worker").unwrap()..];
    let worker = &worker[..worker.find("async fn app_emote_service").unwrap()];
    assert!(worker.contains("badge_queue.pop_front()") && worker.contains("fetch_channel_badges(room.clone()).await"));
    assert!(!worker.contains("spawn"), "reads run one after the other inside one task");
}

#[test]
fn a_row_waits_for_its_rooms_badge_set_then_carries_its_art() {
    let _g = serial();
    reset_for_test();
    let msg: ChatMessage = serde_json::from_value(json!({
        "id": "bw-1", "user_id": "5", "username": "u", "display_name": "U", "color": "#FFFFFF",
        "badges": [{ "name": "subscriber", "version": "6" }], "timestamp": "", "content": "hi", "provider": "twitch",
        "channel": "#waitroom", "tags": { "room-id": "8888801" }, "segments": [], "metadata": {}
    }))
    .unwrap();
    let entry = entry_of(&msg);
    let snap = || serde_json::from_str::<Snap>(snapshot_of(&msg, None).unwrap().get()).unwrap();
    let (row, needs) = build_row(&entry, snap(), false);
    assert_eq!(needs.badge_rooms, vec!["8888801".to_string()]);
    assert_ne!(row.row["badges_resolved"], json!(true), "not resolved until the set is answered");
    // An empty answer is final: the row is complete (the art stays what the global set gives).
    mark_badges_answered_for_test("8888801");
    note_badges(None, &[ArtIn { set: "subscriber".into(), version: "6".into(), x1: "g1".into(), x2: "g2".into(), x4: "g4".into(), title: "Subscriber".into() }]);
    let (row, needs) = build_row(&entry, snap(), false);
    assert!(needs.badge_rooms.is_empty() && !needs.global_badges, "{needs:?}");
    assert_eq!(row.row["badges_resolved"], json!(true));
    assert_eq!(row.row["badges"][0]["image_url_4x"], json!("g4"));
    reset_for_test();
}

#[test]
fn the_cache_serves_a_text_row_for_a_pin_with_no_snapshot_and_keeps_it() {
    let _g = serial();
    reset_for_test();
    let msg: ChatMessage = serde_json::from_value(json!({
        "id": "nc-1", "user_id": "5", "username": "u", "display_name": "U", "color": null, "badges": [], "timestamp": "",
        "content": "plain words", "provider": "twitch", "channel": "#nocache", "tags": {}, "segments": [], "metadata": {}
    }))
    .unwrap();
    let entry = entry_of(&msg);
    let first = cached(&entry).expect("a pin without a snapshot shows its text row at once");
    assert_eq!(first, text_row(&entry));
    assert_eq!(cached(&entry), Some(first), "the same row, hash and all, from the cache");
    reset_for_test();
}

#[test]
fn a_changed_row_hash_is_what_moves_the_revision_not_a_rebuild() {
    let _g = serial();
    reset_for_test();
    let msg: ChatMessage = serde_json::from_value(json!({
        "id": "h-1", "user_id": "5", "username": "u", "display_name": "U", "color": "#123456", "badges": [], "timestamp": "",
        "content": "same", "provider": "kick", "channel": "kick:hslug", "tags": { "b": "2", "a": "1" },
        "segments": [{ "type": "text", "content": "same" }], "metadata": {}
    }))
    .unwrap();
    let entry = entry_of(&msg);
    let shot = snapshot_of(&msg, None).unwrap();
    let (a, _) = pin_row_from(&entry, &shot);
    let (b, _) = pin_row_from(&entry, &shot);
    assert_eq!(a.rev, b.rev, "two builds of the same snapshot hash the same (tags are ordered)");
    let mut moved = msg.clone();
    moved.segments = vec![MessageSegment::Text { content: "different".into() }];
    let (c, _) = pin_row_from(&entry, &snapshot_of(&moved, None).unwrap());
    assert_ne!(a.rev, c.rev);
    reset_for_test();
}

#[test]
fn a_snapshot_that_no_longer_decodes_costs_the_pin_its_decoration_never_the_inbox() {
    assert!(serde_json::from_str::<Snap>("{\"v\":1,\"msg\":42}").is_err());
    let src = include_str!("mention_pin_row.rs");
    let build = &src[src.find("async fn build_and_store").unwrap()..];
    let build = &build[..build.find("fn store_built").unwrap()];
    assert!(build.contains("text_row(entry)"), "a snapshot that fails to decode falls back to the text row");
}

#[test]
fn a_failed_table_or_cheermote_read_waits_out_its_retry_instead_of_starting_again_on_every_rebuild() {
    let _g = serial();
    reset_for_test();
    assert!(begin_table("tbl-room"), "never read: read it");
    assert!(!begin_table("tbl-room"), "one in flight");
    {
        // The read failed: the rebuild that follows asks again at once and is refused.
        let mut o = owner();
        o.table_inflight.remove("tbl-room");
        o.table_book.finish("tbl-room", false, Instant::now());
    }
    assert!(!begin_table("tbl-room"), "a failure waits 30 s, it does not loop with the rebuild");
    assert!(begin_table("other-room"), "another chat is not held back");
    assert!(begin_cheermotes("tbl-room") && !begin_cheermotes("tbl-room"));
    {
        let mut o = owner();
        o.cheer_inflight.remove("tbl-room");
        o.cheer_book.finish("tbl-room", false, Instant::now());
    }
    assert!(!begin_cheermotes("tbl-room"), "the same for cheermotes");
    reset_for_test();
}

#[test]
fn a_full_table_cap_drops_a_chat_with_no_live_pin_before_one_that_still_has_one() {
    let set: EmoteSet = serde_json::from_value(json!({ "twitch": [], "bttv": [], "7tv": [], "ffz": [] })).expect("an empty set");
    let t0 = Instant::now();
    let mut tables: HashMap<String, Table> = HashMap::new();
    tables.insert("live-old".into(), Table { lookup: EmoteLookup::build(&set), at: t0 });
    tables.insert("idle-new".into(), Table { lookup: EmoteLookup::build(&set), at: t0 + Duration::from_secs(5) });
    let live: HashSet<String> = ["live-old".to_string()].into();
    assert_eq!(eviction_victim(&tables, &live).as_deref(), Some("idle-new"), "the live chat's table is the last to go");
    assert_eq!(eviction_victim(&tables, &HashSet::new()).as_deref(), Some("live-old"), "else the oldest");
}

#[test]
fn a_row_stored_incomplete_improves_as_late_data_arrives_and_never_replaces_a_complete_one() {
    let row = |n: u64| PinRow { row: json!({ "n": n }), rev: n };
    let held = |n: u64, complete: bool| Cached { row: json!({ "n": n }), hash: n, built: true, complete };
    // The text row (not built) gives way to any built row.
    let text = Cached { row: json!({}), hash: 0, built: false, complete: false };
    assert_eq!(merged(Some(&text), row(1), false).1, true);
    // An incomplete built row is replaced by a different one (more data landed)...
    let (next, changed) = merged(Some(&held(1, false)), row(2), false);
    assert!(changed && next.is_some_and(|c| c.hash == 2));
    // ...and by the complete one.
    let (next, changed) = merged(Some(&held(2, false)), row(3), true);
    assert!(changed && next.is_some_and(|c| c.hash == 3 && c.complete));
    // The same row again changes nothing; a complete row is never replaced by an incomplete one.
    assert!(!merged(Some(&held(3, true)), row(3), false).1);
    let (next, changed) = merged(Some(&held(3, true)), row(4), false);
    assert!(next.is_none() && !changed);
}
