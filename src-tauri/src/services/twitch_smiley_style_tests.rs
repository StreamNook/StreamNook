use super::*;
use crate::services::twitch_smilies::effective;
use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;

const PRIME: &str = include_str!("../../tests/fixtures/smiley_style_prime.json");
const NO_PRIME: &str = include_str!("../../tests/fixtures/smiley_style_no_prime.json");

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture json")
}

fn prime_read() -> ParsedRead {
    parse_read(&json_of(PRIME))
        .expect("parses")
        .expect("a signed-in answer")
}

fn gql_id_of(read: &ParsedRead, key: &str) -> String {
    read.ids
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, id)| id.clone())
        .expect("the key was listed")
}

/// A read of the Prime fixture whose current set is `key`.
fn read_showing(key: &str) -> ReadOutcome {
    let mut read = prime_read();
    read.view.current = key.to_string();
    ReadOutcome::Read(read)
}

fn confirmed(set_id: &str) -> Value {
    json!({ "data": { "updatePrimeSettings": { "primeSettings": { "primeEmoticons": { "currentSetName": set_id } } } } })
}

fn no_waits(quick: usize, slow: usize) -> Waits {
    Waits {
        quick: vec![Duration::ZERO; quick],
        slow: vec![Duration::ZERO; slow],
    }
}

/// Answers from a script and records what was written.
struct Script {
    reads: Mutex<VecDeque<ReadOutcome>>,
    writes: Mutex<VecDeque<Result<Value, String>>>,
    sent: Mutex<Vec<(String, String)>>,
}

impl Script {
    fn new(reads: Vec<ReadOutcome>, writes: Vec<Result<Value, String>>) -> Self {
        Script {
            reads: Mutex::new(reads.into()),
            writes: Mutex::new(writes.into()),
            sent: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Gql for Script {
    async fn read(&self) -> ReadOutcome {
        self.reads
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ReadOutcome::Failed)
    }

    async fn write(&self, user_id: &str, set_id: &str) -> Result<Value, String> {
        self.sent
            .lock()
            .unwrap()
            .push((user_id.to_string(), set_id.to_string()));
        self.writes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err("no answer".to_string()))
    }
}

/// Makes `user_id` the signed-in account; answers the account generation.
fn sign_in(user_id: &str) -> u64 {
    write_own(|own, g| {
        on_identity_in(own, g, user_id, g, None);
        g
    })
    .0
}

fn record_of(user_id: &str, view: SmileyStyle, chosen_at_ms: Option<i64>) -> Record {
    Record {
        user_id: user_id.to_string(),
        view,
        chosen_at_ms,
        read_at_ms: 0,
        helix_wrong_for: None,
        yield_to_helix: false,
    }
}

fn seed(record: Record) {
    write_own(|own, _| {
        own.record = Some(record);
        own.ver += 1;
    });
}

fn view_with(current: &str) -> SmileyStyle {
    let mut view = prime_read().view;
    view.current = current.to_string();
    view
}

fn helix_rows(key: &str) -> Vec<Emote> {
    smilies::catalog_rows(key).expect("a catalog set")
}

/// Commits a read that began now. The fence is read first: a guard held while the
/// writer waits for the lock would never let go.
fn commit(g: u64, user_id: &str, outcome: &ReadOutcome, now: i64) -> (Verdict, bool) {
    let gen0 = read_own().gen;
    commit_read(gen0, g, user_id, outcome, now)
}

fn rev() -> u64 {
    read_own().rev
}

fn current_of_record() -> String {
    read_own()
        .record
        .as_ref()
        .expect("a record")
        .view
        .current
        .clone()
}

fn expected_current_of(json: &Value) -> String {
    let prime = &json["data"]["currentUser"]["primeSettings"]["primeEmoticons"];
    let name = prime["currentSetName"].as_str().expect("a current set");
    let set = prime["emoticonSets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == name)
        .expect("the current set is listed");
    let first = set["emotes"][0]["id"].as_str().unwrap();
    smilies::catalog_key_of(first)
        .expect("a catalog set")
        .to_string()
}

#[test]
fn view_for_a_prime_account() {
    let read = prime_read();
    assert_eq!(read.user_id, "42");
    let view = &read.view;
    assert!(view.can_switch);
    assert_eq!(view.upgrade_url, "https://www.twitch.tv/settings/prime");
    assert_eq!(view.current, expected_current_of(&json_of(PRIME)));
    // The fixture is Twitch's real answer: its current set is PURPLE.
    assert_eq!(view.current, "purple");
    let keys: Vec<&str> = view.sets.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["robots", "purple", "monkeys"]);
    assert!(view.sets.iter().all(|s| s.available));
    let counts: Vec<usize> = view.sets.iter().map(|s| s.faces.len()).collect();
    assert_eq!(counts, [14, 14, 19]);
    for set in &view.sets {
        let preview: Vec<&str> = set.preview.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(preview, [":)", ";)", "<3"], "{}", set.key);
        for f in &set.faces {
            assert_eq!(
                f.url,
                format!(
                    "https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/2.0",
                    f.id
                )
            );
        }
    }
    // Each catalog key pairs with the id Twitch named that set, written out
    // so a mix-up between sets cannot hide behind a computed expectation.
    let pairs: Vec<(&str, &str)> = read
        .ids
        .iter()
        .map(|(k, id)| (k.as_str(), id.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [("robots", "ROBOTS"), ("purple", "PURPLE"), ("monkeys", "MONKEYS")]
    );
}

#[test]
fn view_for_an_account_without_prime() {
    let read = parse_read(&json_of(NO_PRIME)).unwrap().expect("signed in");
    assert!(!read.view.can_switch);
    assert_eq!(read.view.current, "robots");
    let available: Vec<bool> = read.view.sets.iter().map(|s| s.available).collect();
    assert_eq!(available, [true, false, false]);
    assert_eq!(read.view.sets.len(), 3);
    assert!(read.ids.is_empty());
}

#[test]
fn no_view_without_current_user() {
    assert_eq!(
        parse_read(&json!({ "data": { "currentUser": null } })),
        Ok(None)
    );
    assert_eq!(
        read_outcome(&json!({ "data": { "currentUser": null } })),
        ReadOutcome::NoSession
    );
    assert!(parse_read(&json!({ "errors": [{ "message": "service error" }] })).is_err());
    assert_eq!(
        read_outcome(&json!({ "errors": [{ "message": "service error" }] })),
        ReadOutcome::Failed
    );
    assert!(parse_read(&json!({ "data": {} })).is_err());
}

#[test]
fn an_unknown_set_follows_the_known_ones() {
    let mut answer = json_of(PRIME);
    let sets = answer["data"]["currentUser"]["primeSettings"]["primeEmoticons"]["emoticonSets"]
        .as_array_mut()
        .unwrap();
    sets.push(json!({ "id": "999", "emotes": [{ "id": "9001", "text": ":)" }, { "id": "9000", "text": ":(" }] }));
    sets.push(json!({ "id": "100", "emotes": [{ "id": "8000", "text": "<3" }] }));
    answer["data"]["currentUser"]["primeSettings"]["primeEmoticons"]["currentSetName"] =
        json!("999");
    let read = parse_read(&answer).unwrap().unwrap();
    let keys: Vec<&str> = read.view.sets.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["robots", "purple", "monkeys", "set:100", "set:999"]);
    assert_eq!(read.view.current, "set:999");
    let unknown = &read.view.sets[4];
    assert_eq!(unknown.name, "");
    assert!(unknown.available);
    let ids: Vec<&str> = unknown.faces.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["9000", "9001"]);
    let swap = swap_from_view(&read.view).expect("a swap for the unknown set");
    assert_eq!(swap.rows.len(), 2);
    assert!(
        swap.remove_ids.contains(&"1".to_string()) && swap.remove_ids.contains(&"501".to_string())
    );
}

#[test]
fn faces_read_as_patterns_or_faces() {
    let mut answer = json_of(PRIME);
    let sets = answer["data"]["currentUser"]["primeSettings"]["primeEmoticons"]["emoticonSets"]
        .as_array_mut()
        .unwrap();
    sets.push(json!({ "id": "777", "emotes": [
        { "id": "7001", "text": r"\:-?\)" },
        { "id": "7002", "text": r"\&lt\;3" },
        { "id": "7003", "text": ";)" },
    ] }));
    let read = parse_read(&answer).unwrap().unwrap();
    let unknown = read
        .view
        .sets
        .iter()
        .find(|s| s.key == "set:777")
        .expect("listed");
    let texts: Vec<&str> = unknown.faces.iter().map(|f| f.text.as_str()).collect();
    assert_eq!(texts, [":)", "<3", ";)"]);
    let preview: Vec<&str> = unknown.preview.iter().map(|f| f.text.as_str()).collect();
    assert_eq!(preview, [":)", ";)", "<3"]);
}

#[test]
fn update_body_names_the_operation_and_input() {
    let body = update_body("42", "MONKEYS");
    assert_eq!(
        body["operationName"],
        "SettingsPrimePage_PrimeEmotesSetPicker_UpdatePrimeSettings"
    );
    assert_eq!(
        body["variables"]["input"],
        json!({ "primeEmoticonsSetName": "MONKEYS", "userID": "42" })
    );
    let query = body["query"].as_str().unwrap();
    assert!(query.starts_with(&["mutation ", UPDATE_NAME, "("].concat()));
    assert!(query.contains("updatePrimeSettings(input: $input)"));
    let read = read_body();
    assert_eq!(
        read["operationName"],
        "SettingsPrimePage_PrimeEmotesSetPicker"
    );
    assert!(read["query"]
        .as_str()
        .unwrap()
        .starts_with(&["query ", READ_NAME].concat()));
}

#[test]
fn update_outcome_definite_and_unknown() {
    assert_eq!(
        classify_write(&confirmed("MONKEYS"), "MONKEYS"),
        WriteOutcome::Confirmed
    );
    assert_eq!(classify_write(&confirmed("ROBOTS"), "MONKEYS"), WriteOutcome::Refused);
    assert_eq!(
        classify_write(&json!({ "errors": [{ "message": "service error" }] }), "MONKEYS"),
        WriteOutcome::Refused
    );
    assert_eq!(
        classify_write(
            &json!({ "errors": [{ "message": "failed integrity check" }] }),
            "MONKEYS"
        ),
        WriteOutcome::Refused
    );
    assert_eq!(classify_write(&Value::Null, "MONKEYS"), WriteOutcome::Unknown);
    assert_eq!(
        classify_write(&json!("not json"), "MONKEYS"),
        WriteOutcome::Unknown
    );
    assert_eq!(
        classify_write(&json!({ "data": { "updatePrimeSettings": null } }), "MONKEYS"),
        WriteOutcome::Unknown
    );
}

fn token() -> Integrity {
    Integrity {
        token: "t".into(),
        device_id: "d".into(),
        user_agent: "u".into(),
    }
}

#[tokio::test]
async fn integrity_retry_sends_twice_with_a_token() {
    let sent: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());
    let calls = AtomicUsize::new(0);
    let send = |integrity: Option<Integrity>| {
        let sent = &sent;
        let calls = &calls;
        async move {
            sent.lock().unwrap().push(integrity.map(|i| i.token));
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(json!({ "errors": [{ "message": "failed integrity check" }] }))
            } else {
                Ok(confirmed("MONKEYS"))
            }
        }
    };
    let answer = write_with_retry(send, || async { Ok(token()) })
        .await
        .unwrap();
    assert_eq!(classify_write(&answer, "MONKEYS"), WriteOutcome::Confirmed);
    assert_eq!(*sent.lock().unwrap(), vec![None, Some("t".to_string())]);
}

#[tokio::test]
async fn a_clean_answer_is_sent_once_and_an_unmintable_token_keeps_the_first_answer() {
    let calls = AtomicUsize::new(0);
    let clean = write_with_retry(
        |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok(confirmed("MONKEYS")) }
        },
        || async { Ok(token()) },
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(classify_write(&clean, "MONKEYS"), WriteOutcome::Confirmed);

    let calls = AtomicUsize::new(0);
    let refused = write_with_retry(
        |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok(json!({ "errors": [{ "message": "failed integrity check" }] })) }
        },
        || async { Err("no token".to_string()) },
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(classify_write(&refused, "MONKEYS"), WriteOutcome::Refused);
}

#[tokio::test]
async fn rev_moves_on_every_writer() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    sign_in("42");
    let start = rev();

    // A first read: the record appears.
    let gen0 = read_own().gen;
    let g = TwitchService::account_generation();
    let (verdict, changed) = commit_read(gen0, g, "42", &read_showing("robots"), 1_000);
    assert_eq!((verdict, changed), (Verdict::Committed, true));
    assert_eq!(rev(), start + 1);

    // The same answer again: only the time moved.
    let (verdict, changed) = commit(g, "42", &read_showing("robots"), 2_000);
    assert_eq!((verdict, changed), (Verdict::Committed, false));
    assert_eq!(rev(), start + 1);

    // A read that shows another set.
    let (_, changed) = commit(g, "42", &read_showing("monkeys"), 3_000);
    assert!(changed);
    assert_eq!(rev(), start + 2);

    // A can_switch flip with the same set.
    let mut flipped = prime_read();
    flipped.view.current = "monkeys".to_string();
    flipped.view.can_switch = false;
    let (_, changed) = commit(g, "42", &ReadOutcome::Read(flipped), 4_000);
    assert!(changed);
    assert_eq!(rev(), start + 3);

    // A switch commit to another set.
    let fence = write_own(|own, _| {
        own.gen += 1;
        own.gen
    })
    .0;
    let (ok, changed) =
        write_own(|own, g| commit_switch_in(own, g, fence, "42", &view_with("purple"), 5_000));
    assert!(ok && changed);
    assert_eq!(rev(), start + 4);

    // A move of the account always counts.
    on_account_moved();
    assert_eq!(rev(), start + 5);
}

#[tokio::test]
async fn same_account_sign_in_again_reloads() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let first = sign_in("42");
    assert!(
        !write_own(|own, g| on_identity_in(own, g, "42", first, None)).0,
        "the same pair does nothing"
    );

    TwitchService::bump_account_generation();
    assert!(read_own().identity.is_none(), "the move forgot the account");
    let second = TwitchService::account_generation();
    assert_ne!(first, second);
    let saved = record_of("42", view_with("purple"), None);
    let (acted, _) = write_own(|own, g| on_identity_in(own, g, "42", second, Some(saved.clone())));
    assert!(acted);
    assert_eq!(read_own().identity, Some(("42".to_string(), second)));
    assert_eq!(current_of_record(), "purple");
}

#[tokio::test]
async fn switch_sends_even_when_the_read_says_current() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    sign_in("42");
    let read = prime_read();
    let purple = gql_id_of(&read, "purple");
    let script = Script::new(vec![read_showing("purple")], vec![Ok(confirmed(&purple))]);
    let view = switch_with("purple", &script, &no_waits(2, 2))
        .await
        .ok()
        .expect("switched");
    assert_eq!(view.current, "purple");
    assert_eq!(
        *script.sent.lock().unwrap(),
        vec![("42".to_string(), purple)]
    );
    assert_eq!(current_of_record(), "purple");
    assert!(read_own().record.as_ref().unwrap().chosen_at_ms.is_some());
}

#[tokio::test]
async fn a_switch_refuses_what_the_account_cannot_have() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    sign_in("42");
    let plain = parse_read(&json_of(NO_PRIME)).unwrap().unwrap();
    let script = Script::new(vec![ReadOutcome::Read(plain)], vec![]);
    let failure = switch_with("monkeys", &script, &no_waits(2, 2))
        .await
        .err()
        .expect("refused");
    assert_eq!(failure.text, TEXT_NEEDS_PRIME);
    assert!(
        script.sent.lock().unwrap().is_empty(),
        "nothing was written"
    );
    // A refused write puts the old set back in front of the viewer.
    let script = Script::new(
        vec![read_showing("robots")],
        vec![Ok(confirmed("somewhere else"))],
    );
    let failure = switch_with("purple", &script, &no_waits(2, 2))
        .await
        .err()
        .expect("refused");
    assert_eq!(failure.text, TEXT_FAILED);
    assert!(failure.check_later.is_none());
    assert!(read_own().record.is_none(), "nothing was committed");
}

#[tokio::test]
async fn unknown_outcome_resolves_by_later_reads() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    sign_in("42");
    // Fresh read, then the quick re-reads (2 s and 5 s) and the slow ones (15 s and 60 s).
    let script = Script::new(
        vec![
            read_showing("robots"),
            read_showing("robots"),
            read_showing("robots"),
            read_showing("robots"),
            read_showing("monkeys"),
        ],
        vec![Err("connection reset".to_string())],
    );
    let failure = switch_with("monkeys", &script, &no_waits(2, 2))
        .await
        .err()
        .expect("not confirmed");
    assert_eq!(failure.text, TEXT_UNCONFIRMED);
    assert!(
        read_own().record.is_none(),
        "nothing is committed before a read shows it"
    );
    let check = failure.check_later.expect("a background check");
    assert!(background_check_with(&script, &check, &[Duration::ZERO, Duration::ZERO]).await);
    assert_eq!(current_of_record(), "monkeys");
    assert!(read_own().record.as_ref().unwrap().chosen_at_ms.is_some());

    // A later switch cancels a check that is still waiting.
    let stale = LaterCheck {
        key: "purple".into(),
        user_id: "42".into(),
        fence: read_own().gen.wrapping_sub(1),
    };
    let script = Script::new(vec![read_showing("purple")], vec![]);
    assert!(!background_check_with(&script, &stale, &[Duration::ZERO]).await);
    assert_eq!(current_of_record(), "monkeys");
}

#[tokio::test]
async fn a_quick_re_read_that_shows_the_set_is_a_success() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    sign_in("42");
    let script = Script::new(
        vec![
            read_showing("robots"),
            read_showing("robots"),
            read_showing("monkeys"),
        ],
        vec![Err("timed out".to_string())],
    );
    let view = switch_with("monkeys", &script, &no_waits(2, 2))
        .await
        .ok()
        .expect("the second read showed it");
    assert_eq!(view.current, "monkeys");
    assert_eq!(current_of_record(), "monkeys");
}

#[test]
fn a_stale_record_and_a_disagreeing_helix_answer_ask_for_a_read() {
    let mut own = Own::new();
    own.identity = Some(("42".into(), 7));
    own.record = Some(record_of("42", view_with("robots"), None));
    own.ver = 3;
    // Helix lists the monkeys; the record says robots.
    let ask = note_helix_in(&mut own, 7, "42", 10, helix_rows("monkeys"), 1_000_000);
    assert_eq!(ask.as_deref(), Some("monkeys"));
    // Helix agrees with the record: nothing to ask.
    assert_eq!(
        note_helix_in(&mut own, 7, "42", 11, helix_rows("robots"), 1_000_000),
        None
    );
    // Another account's answer is filed, never asked about.
    assert_eq!(
        note_helix_in(&mut own, 7, "9", 12, helix_rows("monkeys"), 1_000_000),
        None
    );
    assert!(own.helix.contains_key("9"));
    // A switch a moment ago: Helix is probably behind.
    own.record.as_mut().unwrap().chosen_at_ms = Some(990_000);
    assert_eq!(
        note_helix_in(&mut own, 7, "42", 13, helix_rows("monkeys"), 1_000_000),
        None
    );
    own.record.as_mut().unwrap().chosen_at_ms = None;
    // A read already confirmed the record for that set.
    own.record.as_mut().unwrap().helix_wrong_for = Some("monkeys".into());
    assert_eq!(
        note_helix_in(&mut own, 7, "42", 14, helix_rows("monkeys"), 1_000_000),
        None
    );
    own.record.as_mut().unwrap().helix_wrong_for = None;
    // A read already decided this record version and set.
    own.decided = Some((3, "monkeys".into()));
    assert_eq!(
        note_helix_in(&mut own, 7, "42", 15, helix_rows("monkeys"), 1_000_000),
        None
    );
    own.ver = 4;
    assert_eq!(
        note_helix_in(&mut own, 7, "42", 16, helix_rows("monkeys"), 1_000_000).as_deref(),
        Some("monkeys")
    );
    // An older answer for the same account never replaces a newer one.
    assert_eq!(
        note_helix_in(&mut own, 7, "42", 5, helix_rows("robots"), 1_000_000),
        None
    );
    assert_eq!(own.helix["42"].at, 16);
}

#[test]
fn commit_drops_a_read_started_before_a_switch() {
    let mut own = Own::new();
    own.identity = Some(("42".into(), 7));
    own.record = Some(record_of("42", view_with("purple"), None));
    own.gen = 5;
    let verdict = commit_read_in(&mut own, 7, 4, 7, "42", &read_showing("robots"), 100);
    assert_eq!(verdict, Verdict::Dropped(Dropped::Generation));
    assert_eq!(own.record.as_ref().unwrap().view.current, "purple");
}

#[test]
fn commit_drops_a_disagreeing_read_within_60s() {
    let mut own = Own::new();
    own.identity = Some(("42".into(), 7));
    own.record = Some(record_of("42", view_with("purple"), Some(1_000)));
    let early = commit_read_in(
        &mut own,
        7,
        0,
        7,
        "42",
        &read_showing("robots"),
        1_000 + 59_000,
    );
    assert_eq!(early, Verdict::Dropped(Dropped::AfterWrite));
    assert_eq!(own.record.as_ref().unwrap().view.current, "purple");
    // A read that agrees is welcome at once, and keeps the switch time.
    let agree = commit_read_in(
        &mut own,
        7,
        0,
        7,
        "42",
        &read_showing("purple"),
        1_000 + 30_000,
    );
    assert_eq!(agree, Verdict::Committed);
    assert_eq!(own.record.as_ref().unwrap().chosen_at_ms, Some(1_000));
    // After the minute the read is believed.
    let late = commit_read_in(
        &mut own,
        7,
        0,
        7,
        "42",
        &read_showing("robots"),
        1_000 + 61_000,
    );
    assert_eq!(late, Verdict::Committed);
    let record = own.record.as_ref().unwrap();
    assert_eq!(record.view.current, "robots");
    assert_eq!(record.chosen_at_ms, None);
}

#[test]
fn commit_ignores_another_account() {
    let mut own = Own::new();
    own.identity = Some(("7".into(), 3));
    own.record = Some(record_of("7", view_with("robots"), None));
    // The read was for the account that was signed in before.
    let stale_account = commit_read_in(&mut own, 3, 0, 2, "7", &read_showing("monkeys"), 100);
    assert_eq!(stale_account, Verdict::Dropped(Dropped::Account));
    // The twitch.tv session belongs to someone else (the fixture is account 42).
    let other = commit_read_in(&mut own, 3, 0, 3, "7", &read_showing("monkeys"), 100);
    assert_eq!(other, Verdict::NoAnswer);
    assert_eq!(own.record.as_ref().unwrap().view.current, "robots");
    // The identity is some other account altogether.
    let moved = commit_read_in(&mut own, 4, 0, 3, "7", &read_showing("monkeys"), 100);
    assert_eq!(moved, Verdict::Dropped(Dropped::Account));
}

#[test]
fn own_swap_ignores_a_record_of_another_user_or_generation() {
    let mut own = Own::new();
    own.identity = Some(("2".into(), 5));
    own.record = Some(record_of("1", view_with("monkeys"), None));
    assert_eq!(effective(&own, 5), None, "a record of another user");
    own.record = Some(record_of("2", view_with("monkeys"), None));
    assert!(effective(&own, 5).is_some());
    assert_eq!(effective(&own, 6), None, "the account moved on");
    own.identity = None;
    assert_eq!(effective(&own, 5), None);
}

fn kept_ids(swap: &SmileySwap) -> Vec<String> {
    swap.rows.iter().map(|e| e.id.clone()).collect()
}

#[test]
fn own_swap_precedence() {
    let robots = kept_ids(&smilies::swap_to(helix_rows("robots")));
    let monkeys = kept_ids(&smilies::swap_to(helix_rows("monkeys")));
    let case = |record: Option<(&str, bool)>, helix: Option<&str>| {
        let mut own = Own::new();
        own.identity = Some(("42".into(), 1));
        own.record = record.map(|(set, yielding)| {
            let mut r = record_of("42", view_with(set), None);
            r.yield_to_helix = yielding;
            r
        });
        if let Some(set) = helix {
            own.helix.insert(
                "42".into(),
                HelixEntry {
                    rows: helix_rows(set),
                    at: 1,
                },
            );
        }
        effective(&own, 1).map(|s| kept_ids(&s))
    };
    // A record wins over Helix.
    assert_eq!(
        case(Some(("robots", false)), Some("monkeys")),
        Some(robots.clone())
    );
    // A record that yields gives way to Helix.
    assert_eq!(
        case(Some(("robots", true)), Some("monkeys")),
        Some(monkeys.clone())
    );
    // Helix alone.
    assert_eq!(case(None, Some("monkeys")), Some(monkeys.clone()));
    // A record alone, also while it yields to a Helix that has said nothing.
    assert_eq!(case(Some(("monkeys", false)), None), Some(monkeys));
    assert_eq!(case(Some(("monkeys", true)), None), None);
    // Neither.
    assert_eq!(case(None, None), None);
    // An empty Helix list never strips the faces.
    let mut own = Own::new();
    own.identity = Some(("42".into(), 1));
    own.helix.insert(
        "42".into(),
        HelixEntry {
            rows: vec![],
            at: 1,
        },
    );
    assert_eq!(effective(&own, 1), None);
    assert!(!robots.is_empty());
}

#[tokio::test]
async fn helix_disagreement_rules() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let g = sign_in("42");
    let helix = |set: &str| {
        write_own(|own, _| {
            own.helix.insert(
                "42".into(),
                HelixEntry {
                    rows: helix_rows(set),
                    at: 1,
                },
            );
        });
    };
    let fresh = |current: &str| {
        seed(record_of("42", view_with(current), None));
        helix("monkeys");
    };

    // A read that confirms the record: Helix is wrong for that set. Only `ver` moves, not `rev`.
    fresh("robots");
    let (rev_before, ver_before) = (rev(), read_own().ver);
    let (verdict, _) = commit(g, "42", &read_showing("robots"), 10);
    assert_eq!(verdict, Verdict::Committed);
    assert_eq!(
        read_own()
            .record
            .as_ref()
            .unwrap()
            .helix_wrong_for
            .as_deref(),
        Some("monkeys")
    );
    assert_eq!(
        rev(),
        rev_before,
        "the swap, the view and the row are the same"
    );
    assert!(read_own().ver > ver_before);
    assert_eq!(
        read_own().decided,
        Some((read_own().ver, "monkeys".to_string()))
    );

    // A read that shows another set: the record follows Twitch.
    fresh("robots");
    commit(g, "42", &read_showing("monkeys"), 10);
    let record = read_own().record.clone().unwrap();
    assert_eq!(
        (record.view.current.as_str(), record.helix_wrong_for),
        ("monkeys", None)
    );

    // A read that cannot answer for this account: Helix is all that is live.
    fresh("robots");
    commit(g, "42", &ReadOutcome::NoSession, 10);
    let record = read_own().record.clone().unwrap();
    assert!(record.yield_to_helix && record.helix_wrong_for.is_none());
    assert_eq!(
        effective(&read_own(), g).map(|s| kept_ids(&s)),
        Some(kept_ids(&smilies::swap_to(helix_rows("monkeys"))))
    );

    // A later successful read gives the record its say again.
    commit(g, "42", &read_showing("robots"), 20);
    assert!(!read_own().record.as_ref().unwrap().yield_to_helix);

    // Two failed reads while they disagree: yield. One is not enough.
    fresh("robots");
    commit(g, "42", &ReadOutcome::Failed, 10);
    assert!(!read_own().record.as_ref().unwrap().yield_to_helix);
    assert_eq!(read_own().disagree_failures, 1);
    commit(g, "42", &ReadOutcome::Failed, 11);
    assert!(read_own().record.as_ref().unwrap().yield_to_helix);

    // Failures that happen while Helix agrees do not count.
    seed(record_of("42", view_with("monkeys"), None));
    write_own(|own, _| own.disagree_failures = 0);
    commit(g, "42", &ReadOutcome::Failed, 12);
    assert_eq!(read_own().disagree_failures, 0);

    // A commit that changes the current set clears what was learned about Helix.
    fresh("robots");
    commit(g, "42", &read_showing("robots"), 30);
    assert!(read_own()
        .record
        .as_ref()
        .unwrap()
        .helix_wrong_for
        .is_some());
    commit(g, "42", &read_showing("purple"), 40);
    assert!(read_own()
        .record
        .as_ref()
        .unwrap()
        .helix_wrong_for
        .is_none());
}

#[tokio::test]
async fn a_view_only_change_reaches_windows() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let g = sign_in("42");
    // Helix already says robots; the first record says the same: the swap does not move,
    // but the row now has a view to draw.
    write_own(|own, _| {
        own.helix.insert(
            "42".into(),
            HelixEntry {
                rows: helix_rows("robots"),
                at: 1,
            },
        );
    });
    let before = (rev(), own_smilies().swap);
    let (_, changed) = commit(g, "42", &read_showing("robots"), 10);
    assert!(changed);
    assert_eq!(rev(), before.0 + 1);
    let shown = own_smilies();
    assert_eq!(shown.swap, before.1);
    assert!(shown.shown && shown.view.is_some());
    // A can_switch flip with the same set also counts.
    let mut flipped = prime_read();
    flipped.view.current = "robots".into();
    flipped.view.can_switch = false;
    let (_, changed) = commit(g, "42", &ReadOutcome::Read(flipped), 20);
    assert!(changed);
    assert_eq!(rev(), before.0 + 2);
}

#[tokio::test]
async fn row_shown_follows_the_record() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    assert!(!row_shown(), "signed out");
    let g = sign_in("42");
    assert!(!row_shown(), "never read");
    commit(g, "42", &read_showing("robots"), 10);
    assert!(row_shown());
    TwitchService::bump_account_generation();
    assert!(!row_shown(), "another account");
    // The gate: off the test thread's switch, nothing is shown.
    smilies::set_testing(false);
    assert!(!row_shown());
    let source = include_str!("settings_catalog.rs");
    assert!(
        source.contains("twitch_smiley_style::row_shown()"),
        "the catalog capability answers from the record"
    );
}

#[test]
fn backoff_schedule() {
    let minutes: Vec<i64> = (0..7).map(|n| backoff_ms(n) / 60_000).collect();
    assert_eq!(minutes, [0, 10, 20, 40, 60, 60, 60]);
    let waits: Vec<u64> = (0..9).map(|s| move_wait(s).as_secs()).collect();
    assert_eq!(waits, [2, 10, 60, 120, 240, 480, 960, 1920, 3600]);

    let now = 10_000_000;
    let idle = Refresh::default();
    // The row mounts: a record under 30 s old is fresh enough.
    assert!(!read_due(Some(now - 29_000), now, &idle, ROW_FRESH_MS, 1));
    assert!(read_due(Some(now - 31_000), now, &idle, ROW_FRESH_MS, 1));
    assert!(read_due(None, now, &idle, ROW_FRESH_MS, 1));
    // The scheduler: ten minutes.
    assert!(!read_due(
        Some(now - 9 * 60_000),
        now,
        &idle,
        SCHEDULE_MAX_AGE_MS,
        1
    ));
    assert!(read_due(
        Some(now - 11 * 60_000),
        now,
        &idle,
        SCHEDULE_MAX_AGE_MS,
        1
    ));
    // After two failures the next read waits twenty minutes from the last attempt.
    let failing = Refresh {
        last_attempt_ms: now,
        failures: 2,
        in_flight: None,
    };
    assert!(!read_due(None, now + 19 * 60_000, &failing, 0, 1));
    assert!(read_due(None, now + 20 * 60_000, &failing, 0, 1));
    // One read at a time per account; the previous account's read never
    // holds up the new account's first one.
    let running = Refresh {
        in_flight: Some(1),
        ..Refresh::default()
    };
    assert!(!read_due(None, now, &running, 0, 1));
    assert!(read_due(None, now, &running, 0, 2));
}

#[tokio::test]
async fn a_read_left_from_the_previous_account_neither_blocks_nor_counts() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let old_gen = TwitchService::account_generation();
    let old = begin_attempt(1_000, None, 0, old_gen).expect("the old account's read starts");
    TwitchService::bump_account_generation();
    let new_gen = TwitchService::account_generation();
    let new = begin_attempt(1_001, None, 0, new_gen).expect("the new account reads at once");
    // The old read lands late and fails: the new account is not charged.
    old.finish(Verdict::Failed);
    assert_eq!(lock_refresh().failures, 0);
    // And it does not free the new account's slot.
    assert_eq!(lock_refresh().in_flight, Some(new_gen));
    assert!(begin_attempt(1_002, None, 0, new_gen).is_none(), "one at a time");
    new.finish(Verdict::Committed);
    assert_eq!(lock_refresh().in_flight, None);
}

#[tokio::test]
async fn a_read_attempt_counts_failures_and_success_resets_them() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let g = TwitchService::account_generation();
    let first = begin_attempt(1_000, None, 0, g).expect("the first read may start");
    assert!(begin_attempt(1_001, None, 0, g).is_none(), "one at a time");
    first.finish(Verdict::Failed);
    assert_eq!(lock_refresh().failures, 1);
    assert!(
        begin_attempt(1_000 + 9 * 60_000, None, 0, g).is_none(),
        "ten minutes after a failure"
    );
    let second = begin_attempt(1_000 + 10 * 60_000, None, 0, g).expect("the wait is over");
    second.finish(Verdict::Committed);
    assert_eq!(lock_refresh().failures, 0);
    // A read that was dropped for its fence says nothing about Twitch.
    let third = begin_attempt(2_000_000, None, 0, g).unwrap();
    third.finish(Verdict::Dropped(Dropped::Generation));
    assert_eq!(lock_refresh().failures, 0);
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "streamnook-smiley-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn on_identity_loads_the_saved_record() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let dir = temp_dir();
    set_test_dir(Some(dir.clone()));
    save_record(5, &record_of("42", view_with("monkeys"), None));
    let loaded = load_record("42");
    assert!(loaded.is_some());
    let gen_before = read_own().gen;
    let g = TwitchService::account_generation();
    let (acted, changed) = write_own(|own, g| on_identity_in(own, g, "42", g, loaded.clone()));
    assert!(acted);
    assert!(changed, "a saved record makes the row and the swap appear");
    assert_eq!(current_of_record(), "monkeys");
    assert_eq!(
        read_own().gen,
        gen_before + 1,
        "a loaded record moves the fence once"
    );
    assert_eq!(read_own().identity, Some(("42".to_string(), g)));
    assert!(load_record("7").is_none());
    std::fs::write(dir.join("twitch_smiley_style-9.json"), b"{ not json").unwrap();
    assert!(
        load_record("9").is_none(),
        "an unreadable file reads as no record"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn concurrent_identity_calls_bump_once() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let g = TwitchService::account_generation();
    let saved = record_of("42", view_with("purple"), None);
    let gen_before = read_own().gen;
    let a = write_own(|own, g| on_identity_in(own, g, "42", g, Some(saved.clone()))).0;
    let b = write_own(|own, g| on_identity_in(own, g, "42", g, Some(saved.clone()))).0;
    assert!(a && !b);
    assert_eq!(read_own().gen, gen_before + 1);
    assert_eq!(read_own().identity, Some(("42".to_string(), g)));
    // An identity with no saved record moves nothing.
    let gen_now = read_own().gen;
    write_own(|own, g| on_identity_in(own, g, "8", g, None));
    assert_eq!(read_own().gen, gen_now);
    assert!(read_own().record.is_none());
}

#[tokio::test]
async fn identity_from_an_old_generation_is_ignored() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let g = TwitchService::account_generation();
    let (acted, _) = write_own(|own, g| on_identity_in(own, g, "42", g.wrapping_add(1), None));
    assert!(!acted);
    assert!(read_own().identity.is_none());

    let info = crate::models::user::UserInfo {
        id: "42".into(),
        login: "me".into(),
        display_name: "Me".into(),
        email: None,
        profile_image_url: None,
        broadcaster_type: None,
    };
    let before = crate::services::account_roster::twitch_user_id();
    assert!(!crate::services::account_roster::set_twitch_identity_at(
        Some(info),
        g.wrapping_add(7)
    ));
    assert_eq!(crate::services::account_roster::twitch_user_id(), before);
    assert!(read_own().identity.is_none());
}

#[tokio::test]
async fn record_saves_never_go_backwards() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let dir = temp_dir();
    set_test_dir(Some(dir.clone()));
    save_record(5, &record_of("42", view_with("monkeys"), None));
    save_record(4, &record_of("42", view_with("purple"), None));
    assert_eq!(load_record("42").unwrap().view.current, "monkeys");
    // A change that leaves the swap alone (the can_switch flag) is saved too.
    let mut flagged = record_of("42", view_with("monkeys"), None);
    flagged.view.can_switch = false;
    save_record(6, &flagged);
    assert!(!load_record("42").unwrap().view.can_switch);
    // And through the writer: a record change bumps `ver` and reaches the file.
    reset_for_test();
    let g = sign_in("42");
    commit(g, "42", &read_showing("robots"), 10);
    assert_eq!(load_record("42").unwrap().view.current, "robots");
    let _ = std::fs::remove_dir_all(dir);
}

struct FakeHelix {
    /// What each try answers with.
    answers: Mutex<VecDeque<HelixOutcome>>,
    /// The signed-in account's smilies each try files, with the account it files them under.
    files: Mutex<VecDeque<(String, Vec<Emote>)>>,
    token: Option<String>,
    tries: AtomicUsize,
}

#[async_trait]
impl HelixReader for FakeHelix {
    async fn token(&self) -> Option<String> {
        self.token.clone()
    }

    async fn fetch(&self, _token: &str) -> HelixOutcome {
        self.tries.fetch_add(1, Ordering::SeqCst);
        let outcome = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(HelixOutcome::Failed);
        if outcome == HelixOutcome::Answered {
            if let Some((user_id, rows)) = self.files.lock().unwrap().pop_front() {
                note_helix(&user_id, 1, rows);
            }
        }
        outcome
    }
}

fn fake(
    token: Option<&str>,
    answers: Vec<HelixOutcome>,
    files: Vec<(&str, Vec<Emote>)>,
) -> FakeHelix {
    FakeHelix {
        answers: Mutex::new(answers.into()),
        files: Mutex::new(files.into_iter().map(|(u, r)| (u.to_string(), r)).collect()),
        token: token.map(str::to_string),
        tries: AtomicUsize::new(0),
    }
}

#[tokio::test]
async fn after_move_loop_outcomes() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let never = |_: usize| Duration::ZERO;

    // Signed out: no token ends it before any fetch.
    let fence = write_own(|own, _| own.gen).0;
    let signed_out = fake(None, vec![], vec![]);
    after_account_moved_with(fence, None, &signed_out, &never).await;
    assert_eq!(signed_out.tries.load(Ordering::SeqCst), 0);

    // A refused token ends it.
    let refused = fake(Some("t"), vec![HelixOutcome::Refused], vec![]);
    after_account_moved_with(fence, None, &refused, &never).await;
    assert_eq!(refused.tries.load(Ordering::SeqCst), 1);

    // A failure backs off and tries again; an answer for another account does not end it;
    // an answer for the signed-in account does.
    sign_in("42");
    let fence = read_own().gen;
    let steady = fake(
        Some("t"),
        vec![
            HelixOutcome::Failed,
            HelixOutcome::Answered,
            HelixOutcome::Answered,
        ],
        vec![("7", helix_rows("robots")), ("42", helix_rows("monkeys"))],
    );
    after_account_moved_with(fence, None, &steady, &never).await;
    assert_eq!(steady.tries.load(Ordering::SeqCst), 3);
    assert!(read_own().helix.contains_key("42"));

    // An answer with no smilies still ends it (and never strips a face).
    write_own(|own, _| own.helix.clear());
    let empty = fake(
        Some("t"),
        vec![HelixOutcome::Answered],
        vec![("42", vec![])],
    );
    after_account_moved_with(fence, None, &empty, &never).await;
    assert_eq!(empty.tries.load(Ordering::SeqCst), 1);
    assert_eq!(own_smilies().swap, None);

    // A move of the account while it waits ends it.
    write_own(|own, _| own.helix.clear());
    let waiting = fake(
        Some("t"),
        vec![HelixOutcome::Failed, HelixOutcome::Failed],
        vec![],
    );
    let moved = |_: usize| {
        write_own(|own, _| own.gen += 1);
        Duration::ZERO
    };
    after_account_moved_with(fence, None, &waiting, &moved).await;
    assert_eq!(waiting.tries.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn identity_arriving_after_the_helix_answer_applies_it() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    // The new account's Helix answer lands before the account is known.
    note_helix("B", 5, helix_rows("monkeys"));
    assert_eq!(own_smilies().swap, None, "not the identity's account yet");
    let (rev_before, gen_before) = (rev(), read_own().gen);
    let (acted, changed) = write_own(|own, g| on_identity_in(own, g, "B", g, None));
    assert!(acted && changed);
    // `write_own` reported the change: the swap now exists.
    assert_eq!(
        read_own().gen,
        gen_before,
        "no record was loaded, so the fence stays"
    );
    assert!(rev() > rev_before);
    assert_eq!(
        own_smilies().swap,
        Some(smilies::swap_to(helix_rows("monkeys")))
    );
    assert!(read_own().helix.contains_key("B"));
}

#[tokio::test]
async fn note_helix_bumps_rev_only_when_the_swap_changes() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    sign_in("42");
    let start = rev();
    note_helix("42", 1, helix_rows("monkeys"));
    assert_eq!(rev(), start + 1);
    note_helix("42", 2, helix_rows("monkeys"));
    assert_eq!(rev(), start + 1, "the same answer again");
    note_helix("42", 3, helix_rows("robots"));
    assert_eq!(rev(), start + 2);
    // Another account's answer never moves what this account wears.
    note_helix("9", 4, helix_rows("purple"));
    assert_eq!(rev(), start + 2);
}

#[tokio::test]
async fn a_fetch_with_the_previous_accounts_token_is_filed_under_that_account_and_never_shapes_the_new_one(
) {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    sign_in("new");
    // A late answer fetched with the old account's token.
    note_helix("old", 1, helix_rows("monkeys"));
    assert_eq!(own_smilies().swap, None);
    assert!(read_own().helix.contains_key("old"));
    // The new account's own answer.
    note_helix("new", 2, helix_rows("purple"));
    assert_eq!(
        own_smilies().swap,
        Some(smilies::swap_to(helix_rows("purple")))
    );
    // The old account's entry is pruned when the identity is set again (no
    // account move here: that would empty the map whatever the prune does).
    write_own(|own, _| own.identity = None);
    write_own(|own, g| on_identity_in(own, g, "new", g, None));
    assert!(!read_own().helix.contains_key("old"));
    assert!(read_own().helix.contains_key("new"));
}

#[tokio::test]
async fn a_later_helix_success_heals_a_failed_switch_fallback() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    sign_in("42");
    // The record is gone (cleared by an account move) and Helix is stale.
    note_helix("42", 1, helix_rows("robots"));
    assert_eq!(
        own_smilies().swap,
        Some(smilies::swap_to(helix_rows("robots")))
    );
    // A switch the account made elsewhere shows in the next complete Helix answer.
    note_helix("42", 2, helix_rows("monkeys"));
    assert_eq!(
        own_smilies().swap,
        Some(smilies::swap_to(helix_rows("monkeys")))
    );
}

#[test]
fn the_call_sites_dress_every_list_that_is_installed() {
    let fetch = include_str!("emote_service.rs").replace("\r\n", "\n");
    let start = fetch
        .find("async fn fetch_user_twitch_emotes_inner(")
        .expect("the Helix fetch");
    let body = &fetch[start..];
    let body = &body[..body
        .find("\n    // Private helper methods")
        .expect("end of fetch")];
    let tidy = body.find("tidy_names(").expect("names tidied");
    let note = body
        .find("twitch_smiley_style::note_helix(")
        .expect("the answer is filed");
    let empty = body
        .rfind("all_emotes.is_empty()")
        .expect("the empty-list return");
    assert!(
        tidy < note && note < empty,
        "tidy, then file the answer, then the empty-list return"
    );
    assert!(
        body.find("if complete {")
            .expect("only a complete answer is filed")
            < note,
        "a partial answer is never filed"
    );
    assert!(
        body.find("twitch_smilies::wear_own(&mut all_emotes)")
            .is_some(),
        "the answer wears the account's style"
    );
    assert_eq!(
        fetch
            .split_whitespace()
            .collect::<String>()
            .matches("put_dressed(&mutcache")
            .count(),
        2,
        "both cache puts dress the set"
    );

    let irc = include_str!("irc_service.rs").replace("\r\n", "\n");
    assert_eq!(
        irc.matches("twitch_smilies::wear_own(&mut").count(),
        2,
        "the disk seed and the fetched set"
    );
    assert!(irc.contains("pub(crate) async fn replace_own_smilies()"));

    let cache = include_str!("emote_set_cache.rs").replace("\r\n", "\n");
    assert!(
        cache.contains("twitch_smilies::wear_own(&mut set.twitch)"),
        "a loaded dictionary wears the style"
    );

    let smilies_source = include_str!("twitch_smilies.rs").replace("\r\n", "\n");
    let wear = &smilies_source[smilies_source.find("pub fn wear_own(").expect("wear_own")..];
    assert!(wear[..wear.find("\n}\n").unwrap()].contains("own_swap()"));

    let service = include_str!("twitch_service.rs").replace("\r\n", "\n");
    let bump = &service[service
        .find("pub(crate) fn bump_account_generation()")
        .expect("the bump")..];
    assert!(
        bump[..bump.find("\n    }\n").unwrap()].contains("twitch_smiley_style::on_account_moved()")
    );
    let delete = &service[service
        .find("fn delete_token_file(_slot")
        .expect("sign-out")..];
    let delete = &delete[..delete.find("\n    }\n").unwrap()];
    assert!(
        delete.find("bump_account_generation()").unwrap()
            < delete.find("set_twitch_identity(None)").unwrap(),
        "sign-out bumps before it clears the identity"
    );
    let user_info = &service[service
        .find("pub async fn get_user_info()")
        .expect("get_user_info")..];
    let user_info = &user_info[..user_info.find("\n    }\n").unwrap()];
    assert!(
        user_info.contains("get_token_with_generation()")
            && user_info.contains("set_twitch_identity_at(")
    );
}
