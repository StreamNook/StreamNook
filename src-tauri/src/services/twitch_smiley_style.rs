//! Which set of art the signed-in Twitch account's text faces (`:)` `;)` `<3`
//! and the rest) are drawn in, and the switch that changes it.
//!
//! The choice lives on the Twitch account, so everyone who sees the account's
//! messages sees it. Twitch's Prime and Turbo settings page reads and changes
//! it through the twitch.tv web session; StreamNook does the same with the
//! session its sign-in already holds. Both documents are inline: Twitch can
//! change its private API, and a stored hash would fail with no word of what
//! changed.
//!
//! The app keeps one `Record` per account (the last read or the last switch),
//! saves it, and derives from it the one "swap" every copy of the account's
//! emote list is put in: the Rust chat tables, the emote cache, the disk
//! dictionaries, and each window's own copy (told by `twitch://smilies-changed`).
//! Every write to that state goes through `write_own`, which moves `rev`
//! whenever anything a window draws changed.
//!
//! A read or a switch that began before the account or the fence moved is
//! dropped when it lands, and for a minute after a switch a read that shows
//! the old set is dropped too, since Twitch may still answer from before the
//! write. When Twitch's emote list names a different set than the record,
//! one read decides which is right.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::services::emote_service::{Emote, EmoteService, HelixOutcome};
use crate::services::twitch_integrity::Integrity;
use crate::services::twitch_service::TwitchService;
use crate::services::twitch_smilies::{
    self as smilies, observable, read_own, visible_record, HelixEntry, Own, SmileySwap, OWN,
};
use crate::services::twitch_web_gql;

// Twitch's own documents, as the Prime settings page sends them.
const READ_DOC: &str = "query SettingsPrimePage_PrimeEmotesSetPicker { currentUser { id hasPrime hasTurbo primeSettings { id primeEmoticons { currentSetName emoticonSets { id emotes { id text } } } } } }";
const READ_NAME: &str = "SettingsPrimePage_PrimeEmotesSetPicker";
const UPDATE_DOC: &str = "mutation SettingsPrimePage_PrimeEmotesSetPicker_UpdatePrimeSettings($input: UpdatePrimeSettingsInput!) { updatePrimeSettings(input: $input) { primeSettings { id primeEmoticons { currentSetName } } } }";
const UPDATE_NAME: &str = "SettingsPrimePage_PrimeEmotesSetPicker_UpdatePrimeSettings";

const LOG_TAG: &str = "TwitchSmileyStyle";

/// Sent to every window when the account's smiley style changes.
pub const SMILIES_CHANGED_EVENT: &str = "twitch://smilies-changed";

/// Where an account without Prime or Turbo goes to get the other sets.
pub const UPGRADE_URL: &str = "https://www.twitch.tv/settings/prime";

pub const TEXT_FAILED: &str = "Twitch did not change your smiley style. Try again in a moment.";
pub const TEXT_UNCONFIRMED: &str =
    "Twitch has not confirmed the change yet. It shows here once it does.";
pub const TEXT_NEEDS_PRIME: &str = "That smiley style comes with Twitch Prime or Turbo.";

/// For this long after a switch, a read that shows the old set is not believed.
const READ_AFTER_WRITE_MS: i64 = 60_000;
/// A saved record this old is read again by the scheduler.
const SCHEDULE_MAX_AGE_MS: i64 = 10 * 60_000;
/// A record this old is read again when the settings row opens.
const ROW_FRESH_MS: i64 = 30_000;
/// The first read after an account is known waits this long.
const FIRST_READ_DELAY: Duration = Duration::from_secs(5);
/// How often the scheduler looks at the clock.
const SCHEDULER_TICK: Duration = Duration::from_secs(60);

/// A face of a set, with the image to draw it from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmileyFace {
    pub id: String,
    pub text: String,
    pub url: String,
}

/// One tile of the picker: a set, the three faces it shows, and all of them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmileySetChoice {
    /// `robots`, `purple`, `monkeys`, or `set:<Twitch's id>` for a set the app
    /// does not know by name.
    pub key: String,
    /// The set's name; empty for a set the app does not know by name.
    pub name: String,
    pub faces: Vec<SmileyFace>,
    pub preview: Vec<SmileyFace>,
    /// Whether the account may switch to it.
    pub available: bool,
}

/// What the settings row draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmileyStyle {
    pub can_switch: bool,
    pub current: String,
    pub sets: Vec<SmileySetChoice>,
    pub upgrade_url: String,
}

/// What the app knows about one account's style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub user_id: String,
    pub view: SmileyStyle,
    /// When the account was switched from here, while that is recent news.
    #[serde(default)]
    pub chosen_at_ms: Option<i64>,
    pub read_at_ms: i64,
    /// The set Helix keeps listing for this account although Twitch's settings
    /// say otherwise.
    #[serde(default)]
    pub helix_wrong_for: Option<String>,
    /// Twitch's settings cannot be read for this account, so Helix's list is
    /// the only live source.
    #[serde(default)]
    pub yield_to_helix: bool,
}

/// What `get_smiley_style` answers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SmileyStyleAnswer {
    pub view: Option<SmileyStyle>,
    pub rev: u64,
}

/// What `get_own_smilies` answers and `twitch://smilies-changed` carries.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OwnSmilies {
    pub rev: u64,
    pub swap: Option<SmileySwap>,
    pub view: Option<SmileyStyle>,
    pub shown: bool,
}

fn face(id: &str, text: &str) -> SmileyFace {
    SmileyFace {
        id: id.to_string(),
        text: text.to_string(),
        url: smilies::emote_url(id),
    }
}

/// The three faces a tile shows: `:)` `;)` `<3` where the set has them.
fn preview_of(faces: &[SmileyFace]) -> Vec<SmileyFace> {
    let picked: Vec<SmileyFace> = smilies::PREVIEW_FACES
        .iter()
        .filter_map(|text| faces.iter().find(|f| f.text == *text).cloned())
        .collect();
    if picked.is_empty() {
        faces.iter().take(3).cloned().collect()
    } else {
        picked
    }
}

impl SmileySetChoice {
    fn catalog(set: &smilies::CatalogSet, available: bool) -> Self {
        let faces: Vec<SmileyFace> = set.faces.iter().map(|(id, text)| face(id, text)).collect();
        SmileySetChoice {
            key: set.key.to_string(),
            name: set.name.to_string(),
            preview: preview_of(&faces),
            faces,
            available,
        }
    }

    fn from_read(key: &str, raw: &RawSet, available: bool) -> Self {
        let mut faces: Vec<SmileyFace> = raw
            .faces
            .iter()
            .map(|(id, text)| face(id, &face_text(text)))
            .collect();
        faces.sort_by_key(|f| (f.id.parse::<u64>().unwrap_or(u64::MAX), f.id.clone()));
        SmileySetChoice {
            key: key.to_string(),
            name: String::new(),
            preview: preview_of(&faces),
            faces,
            available,
        }
    }
}

/// The face Twitch's text stands for: a face as it is, a pattern as the face
/// it matches.
fn face_text(raw: &str) -> String {
    smilies::display_code(raw).unwrap_or_else(|| raw.to_string())
}

fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

struct RawSet {
    id: String,
    faces: Vec<(String, String)>,
}

fn raw_sets(prime: &Value) -> Vec<RawSet> {
    let Some(sets) = prime["emoticonSets"].as_array() else {
        return Vec::new();
    };
    sets.iter()
        .filter_map(|set| {
            let id = id_text(&set["id"])?;
            let faces = set["emotes"]
                .as_array()
                .map(|emotes| {
                    emotes
                        .iter()
                        .filter_map(|e| Some((id_text(&e["id"])?, e["text"].as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            Some(RawSet { id, faces })
        })
        .collect()
}

/// What one read of the account's smiley settings said.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParsedRead {
    /// The account the answer is for.
    pub user_id: String,
    pub view: SmileyStyle,
    /// (key, Twitch's id for that set) of every set the answer listed.
    pub ids: Vec<(String, String)>,
}

/// Reads an answer to `READ_DOC`. `Ok(None)` is the answer of a session
/// that is not signed in to Twitch (`currentUser` is null).
pub(crate) fn parse_read(json: &Value) -> Result<Option<ParsedRead>, String> {
    let data = json
        .get("data")
        .filter(|d| d.is_object())
        .ok_or_else(|| "the answer has no data".to_string())?;
    let user = data
        .get("currentUser")
        .ok_or_else(|| "the answer has no currentUser".to_string())?;
    if user.is_null() {
        return Ok(None);
    }
    let user_id = id_text(&user["id"]).ok_or_else(|| "currentUser has no id".to_string())?;
    let has_prime = user["hasPrime"].as_bool().unwrap_or(false);
    let has_turbo = user["hasTurbo"].as_bool().unwrap_or(false);
    let prime = &user["primeSettings"]["primeEmoticons"];
    let raw = raw_sets(prime);
    let can_switch = (has_prime || has_turbo) && !raw.is_empty();

    let mut keyed: Vec<(String, &RawSet)> = Vec::new();
    for set in &raw {
        let key = smilies::catalog_key_for_ids(set.faces.iter().map(|(id, _)| id.as_str()))
            .map(str::to_string)
            .unwrap_or_else(|| format!("set:{}", set.id));
        if keyed.iter().all(|(k, _)| *k != key) {
            keyed.push((key, set));
        }
    }
    let current = prime["currentSetName"]
        .as_str()
        .and_then(|name| keyed.iter().find(|(_, set)| set.id == name))
        .map(|(key, _)| key.clone())
        .unwrap_or_else(|| "robots".to_string());

    let mut sets: Vec<SmileySetChoice> = smilies::SETS
        .iter()
        .map(|set| {
            let offered = keyed.iter().any(|(key, _)| key == set.key);
            SmileySetChoice::catalog(set, set.key == "robots" || (can_switch && offered))
        })
        .collect();
    let mut unknown: Vec<&(String, &RawSet)> = keyed
        .iter()
        .filter(|(key, _)| smilies::catalog_set(key).is_none())
        .collect();
    unknown.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    for (key, set) in unknown {
        sets.push(SmileySetChoice::from_read(key, set, can_switch));
    }

    Ok(Some(ParsedRead {
        user_id,
        view: SmileyStyle {
            can_switch,
            current,
            sets,
            upgrade_url: UPGRADE_URL.to_string(),
        },
        ids: keyed
            .iter()
            .map(|(key, set)| (key.clone(), set.id.clone()))
            .collect(),
    }))
}

/// The swap that dresses a list in the set the view says is current.
pub(crate) fn swap_from_view(view: &SmileyStyle) -> Option<SmileySwap> {
    let choice = view.sets.iter().find(|s| s.key == view.current)?;
    if choice.faces.is_empty() {
        return None;
    }
    let rows = choice
        .faces
        .iter()
        .map(|f| {
            let mut row = smilies::face_row(&f.id, &f.text);
            row.url = f.url.clone();
            row
        })
        .collect();
    Some(smilies::swap_to(rows))
}

pub(crate) fn read_body() -> Value {
    json!({ "operationName": READ_NAME, "query": READ_DOC, "variables": {} })
}

pub(crate) fn update_body(user_id: &str, set_id: &str) -> Value {
    json!({
        "operationName": UPDATE_NAME,
        "query": UPDATE_DOC,
        "variables": { "input": { "primeEmoticonsSetName": set_id, "userID": user_id } },
    })
}

/// What Twitch answered to the write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteOutcome {
    /// Twitch stored the set that was sent.
    Confirmed,
    /// Twitch refused, or stored another set.
    Refused,
    /// Nothing readable came back; a read has to decide.
    Unknown,
}

pub(crate) fn classify_write(json: &Value, sent_set: &str) -> WriteOutcome {
    if json
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|e| !e.is_empty())
    {
        return WriteOutcome::Refused;
    }
    match json
        .pointer("/data/updatePrimeSettings/primeSettings/primeEmoticons/currentSetName")
        .and_then(Value::as_str)
    {
        Some(name) if name == sent_set => WriteOutcome::Confirmed,
        Some(_) => WriteOutcome::Refused,
        None => WriteOutcome::Unknown,
    }
}

fn mentions_integrity(json: &Value) -> bool {
    json["errors"].as_array().is_some_and(|e| {
        e.iter().any(|x| {
            x["message"]
                .as_str()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains("integrity")
        })
    })
}

/// Sends a write, and once more with Twitch's security token when the first
/// answer asks for one. `send` takes the token to send with; `mint` makes one.
pub(crate) async fn write_with_retry<S, SF, M, MF>(send: S, mint: M) -> Result<Value, String>
where
    S: Fn(Option<Integrity>) -> SF,
    SF: Future<Output = Result<Value, String>>,
    M: Fn() -> MF,
    MF: Future<Output = Result<Integrity, String>>,
{
    let first = send(None).await?;
    if !mentions_integrity(&first) {
        return Ok(first);
    }
    match mint().await {
        Ok(token) => send(Some(token)).await,
        Err(_) => Ok(first),
    }
}

/// What a read of the settings came to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ReadOutcome {
    Read(ParsedRead),
    /// No twitch.tv session, or the session belongs to nobody.
    NoSession,
    /// The request failed or the answer made no sense.
    Failed,
}

pub(crate) fn read_outcome(json: &Value) -> ReadOutcome {
    match parse_read(json) {
        Ok(Some(read)) => ReadOutcome::Read(read),
        Ok(None) => ReadOutcome::NoSession,
        Err(_) => ReadOutcome::Failed,
    }
}

/// The twitch.tv side, so tests never reach the network or the machine's own
/// sign-in.
#[async_trait]
pub(crate) trait Gql: Send + Sync {
    async fn read(&self) -> ReadOutcome;
    /// Twitch's raw answer to switching `user_id` to the set Twitch calls `set_id`.
    async fn write(&self, user_id: &str, set_id: &str) -> Result<Value, String>;
}

/// The real thing: Twitch's GQL on the app's twitch.tv web session.
pub(crate) struct WebGql;

#[async_trait]
impl Gql for WebGql {
    async fn read(&self) -> ReadOutcome {
        let Some(token) = twitch_web_gql::app_web_session().await else {
            return ReadOutcome::NoSession;
        };
        match twitch_web_gql::post(Some(&token), read_body(), LOG_TAG).await {
            Ok(json) => read_outcome(&json),
            Err(e) => {
                log::debug!("[{LOG_TAG}] read failed: {e}");
                ReadOutcome::Failed
            }
        }
    }

    async fn write(&self, user_id: &str, set_id: &str) -> Result<Value, String> {
        let Some(token) = twitch_web_gql::app_web_session().await else {
            return Err("not signed in to Twitch".to_string());
        };
        let body = update_body(user_id, set_id);
        write_with_retry(
            |integrity: Option<Integrity>| {
                let token = token.clone();
                let body = body.clone();
                async move {
                    twitch_web_gql::post_with(Some(&token), body, LOG_TAG, integrity.as_ref()).await
                }
            },
            || async {
                crate::services::twitch_integrity::user_asked();
                crate::services::twitch_integrity::get().await
            },
        )
        .await
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Runs `f` on the state under its write lock, then says whether anything a
/// window draws changed (and moves `rev` if so). `f` gets the account
/// generation read inside the lock. A record changed by `f` is saved.
pub(crate) fn write_own<R>(f: impl FnOnce(&mut Own, u64) -> R) -> (R, bool) {
    let (result, changed, save) = {
        let mut own = OWN.write().unwrap_or_else(|e| e.into_inner());
        let g = TwitchService::account_generation();
        let before = observable(&own, g);
        let ver = own.ver;
        let result = f(&mut own, g);
        let changed = before != observable(&own, g);
        if changed {
            own.rev += 1;
        }
        let save = if own.ver != ver {
            own.record.clone().map(|r| (own.ver, r))
        } else {
            None
        };
        (result, changed, save)
    };
    if let Some((ver, record)) = save {
        save_record(ver, &record);
    }
    (result, changed)
}

/// Why a commit was not made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dropped {
    /// The fence moved since the read began (a switch, a load, an account move).
    Generation,
    /// The account is not the one the read was for.
    Account,
    /// Within a minute of a switch, the read shows the old set.
    AfterWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Committed,
    /// The read cannot answer for this account.
    NoAnswer,
    Failed,
    Dropped(Dropped),
}

/// The set Helix lists for `user_id` when it is not the record's current set.
pub(crate) fn standing_disagreement(own: &Own, user_id: &str) -> Option<String> {
    let record = own
        .record
        .as_ref()
        .filter(|r| r.user_id == user_id && !r.yield_to_helix)?;
    let entry = own.helix.get(user_id).filter(|e| !e.rows.is_empty())?;
    let current = record
        .view
        .sets
        .iter()
        .find(|s| s.key == record.view.current)?;
    if entry
        .rows
        .iter()
        .any(|row| current.faces.iter().any(|f| f.id == row.id))
    {
        return None;
    }
    let key = entry
        .rows
        .iter()
        .find_map(|row| {
            record
                .view
                .sets
                .iter()
                .find(|s| s.faces.iter().any(|f| f.id == row.id))
        })
        .map(|s| s.key.clone())
        .unwrap_or_else(|| "other".to_string());
    Some(key)
}

/// Commits what a read found, if it still may be. `gen0` and `account_gen`
/// are the fence and the account generation when the read began.
pub(crate) fn commit_read_in(
    own: &mut Own,
    g: u64,
    gen0: u64,
    account_gen: u64,
    user_id: &str,
    outcome: &ReadOutcome,
    now: i64,
) -> Verdict {
    if own.gen != gen0 {
        return Verdict::Dropped(Dropped::Generation);
    }
    if account_gen != g || !matches!(&own.identity, Some((u, ag)) if u == user_id && *ag == g) {
        return Verdict::Dropped(Dropped::Account);
    }
    let standing = standing_disagreement(own, user_id);
    match outcome {
        ReadOutcome::Read(read) if read.user_id == user_id => {
            let prev = own.record.as_ref().filter(|r| r.user_id == user_id);
            if let Some(prev) = prev {
                let fresh_switch = prev
                    .chosen_at_ms
                    .is_some_and(|at| now.saturating_sub(at) < READ_AFTER_WRITE_MS);
                if fresh_switch && read.view.current != prev.view.current {
                    return Verdict::Dropped(Dropped::AfterWrite);
                }
            }
            let same_set = prev.is_some_and(|r| r.view.current == read.view.current);
            let chosen_at_ms = if same_set {
                prev.and_then(|r| r.chosen_at_ms)
            } else {
                None
            };
            let mut helix_wrong_for = if same_set {
                prev.and_then(|r| r.helix_wrong_for.clone())
            } else {
                None
            };
            if same_set {
                if let Some(key) = &standing {
                    helix_wrong_for = Some(key.clone());
                }
            }
            own.record = Some(Record {
                user_id: user_id.to_string(),
                view: read.view.clone(),
                chosen_at_ms,
                read_at_ms: now,
                helix_wrong_for,
                yield_to_helix: false,
            });
            own.ver += 1;
            own.disagree_failures = 0;
            if let Some(key) = standing {
                own.decided = Some((own.ver, key));
            }
            Verdict::Committed
        }
        ReadOutcome::Read(_) | ReadOutcome::NoSession => {
            if let Some(record) = own.record.as_mut().filter(|r| r.user_id == user_id) {
                record.helix_wrong_for = None;
                if standing.is_some() {
                    record.yield_to_helix = true;
                }
                own.ver += 1;
            }
            if let Some(key) = standing {
                own.decided = Some((own.ver, key));
            }
            Verdict::NoAnswer
        }
        ReadOutcome::Failed => {
            if let Some(key) = standing {
                let known_wrong = own
                    .record
                    .as_ref()
                    .and_then(|r| r.helix_wrong_for.as_deref())
                    == Some(key.as_str());
                if !known_wrong {
                    own.disagree_failures += 1;
                    if own.disagree_failures >= 2 {
                        if let Some(record) = own.record.as_mut() {
                            record.yield_to_helix = true;
                            own.ver += 1;
                            own.decided = Some((own.ver, key));
                        }
                    }
                }
            }
            Verdict::Failed
        }
    }
}

/// Commits a switch that Twitch confirmed (or a later read showed), if the
/// fence is still the one the switch took and the account is the same.
pub(crate) fn commit_switch_in(
    own: &mut Own,
    g: u64,
    fence: u64,
    user_id: &str,
    view: &SmileyStyle,
    now: i64,
) -> bool {
    if own.gen != fence || !matches!(&own.identity, Some((u, ag)) if u == user_id && *ag == g) {
        return false;
    }
    own.record = Some(Record {
        user_id: user_id.to_string(),
        view: view.clone(),
        chosen_at_ms: Some(now),
        read_at_ms: now,
        helix_wrong_for: None,
        yield_to_helix: false,
    });
    own.ver += 1;
    own.gen += 1;
    own.disagree_failures = 0;
    true
}

/// Files what Helix listed as `user_id`'s smilies. Answers the set key to ask
/// a read about when the list disagrees with a record nobody could have
/// switched a moment ago.
pub(crate) fn note_helix_in(
    own: &mut Own,
    g: u64,
    user_id: &str,
    started_at: i64,
    rows: Vec<Emote>,
    now: i64,
) -> Option<String> {
    if own.helix.get(user_id).is_some_and(|e| e.at > started_at) {
        return None;
    }
    let rows: Vec<Emote> = rows
        .into_iter()
        .filter(|e| e.emote_type.as_deref() == Some("smilies"))
        .collect();
    own.helix.insert(
        user_id.to_string(),
        HelixEntry {
            rows,
            at: started_at,
        },
    );
    if !matches!(&own.identity, Some((u, ag)) if u == user_id && *ag == g) {
        return None;
    }
    let key = standing_disagreement(own, user_id)?;
    let record = own.record.as_ref()?;
    if record.helix_wrong_for.as_deref() == Some(key.as_str()) {
        return None;
    }
    if record
        .chosen_at_ms
        .is_some_and(|at| now.saturating_sub(at) < READ_AFTER_WRITE_MS)
    {
        return None;
    }
    if own.decided.as_ref() == Some(&(own.ver, key.clone())) {
        return None;
    }
    Some(key)
}

/// Sets who the state belongs to. False when it already did, or when the
/// account moved on again meanwhile.
pub(crate) fn on_identity_in(
    own: &mut Own,
    g: u64,
    user_id: &str,
    generation: u64,
    loaded: Option<Record>,
) -> bool {
    if matches!(&own.identity, Some((u, ag)) if u == user_id && *ag == generation)
        || generation != g
    {
        return false;
    }
    own.identity = Some((user_id.to_string(), generation));
    own.helix.retain(|k, _| k == user_id);
    let kept = own.record.take().filter(|r| r.user_id == user_id);
    let loaded = loaded.filter(|r| r.user_id == user_id);
    let loaded_used = kept.is_none() && loaded.is_some();
    own.record = kept.or(loaded);
    if loaded_used {
        own.gen += 1;
        own.ver += 1;
    }
    own.disagree_failures = 0;
    own.decided = None;
    true
}

/// How long to wait before the next read after `failures` failed ones.
pub(crate) fn backoff_ms(failures: u32) -> i64 {
    if failures == 0 {
        return 0;
    }
    let minutes = 10i64.saturating_mul(1i64 << (failures - 1).min(6));
    minutes.min(60) * 60_000
}

#[derive(Default)]
struct Refresh {
    last_attempt_ms: i64,
    failures: u32,
    /// The account generation of the read that is running, if one is. A read
    /// left over from the previous account never holds up the new one's.
    in_flight: Option<u64>,
}

static REFRESH: Lazy<Mutex<Refresh>> = Lazy::new(|| Mutex::new(Refresh::default()));

fn lock_refresh() -> std::sync::MutexGuard<'static, Refresh> {
    REFRESH.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a read may start now: none is running for this account, the last
/// failures' wait is over, and the record (if any) is older than `max_age_ms`.
fn read_due(
    read_at_ms: Option<i64>,
    now: i64,
    refresh: &Refresh,
    max_age_ms: i64,
    account_gen: u64,
) -> bool {
    if refresh.in_flight == Some(account_gen) {
        return false;
    }
    if refresh.failures > 0 && now < refresh.last_attempt_ms + backoff_ms(refresh.failures) {
        return false;
    }
    match read_at_ms {
        Some(at) => now - at > max_age_ms,
        None => true,
    }
}

/// A read in progress for one account generation; dropping it lets the next
/// one start.
struct Attempt {
    account_gen: u64,
}

impl Attempt {
    fn finish(self, verdict: Verdict) {
        let mut refresh = lock_refresh();
        // The account moved while this read ran: its outcome is not the new
        // account's, so it neither counts a failure nor clears one.
        if self.account_gen != TwitchService::account_generation() {
            return;
        }
        match verdict {
            Verdict::Committed => refresh.failures = 0,
            Verdict::NoAnswer | Verdict::Failed => {
                refresh.failures = refresh.failures.saturating_add(1)
            }
            Verdict::Dropped(_) => {}
        }
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        let mut refresh = lock_refresh();
        if refresh.in_flight == Some(self.account_gen) {
            refresh.in_flight = None;
        }
    }
}

fn begin_attempt(
    now: i64,
    read_at_ms: Option<i64>,
    max_age_ms: i64,
    account_gen: u64,
) -> Option<Attempt> {
    let mut refresh = lock_refresh();
    if !read_due(read_at_ms, now, &refresh, max_age_ms, account_gen) {
        return None;
    }
    refresh.in_flight = Some(account_gen);
    refresh.last_attempt_ms = now;
    Some(Attempt { account_gen })
}

struct ReadContext {
    gen: u64,
    account_gen: u64,
    user_id: String,
    read_at_ms: Option<i64>,
}

fn read_context() -> Option<ReadContext> {
    let own = read_own();
    let g = TwitchService::account_generation();
    let (user_id, account_gen) = own.identity.clone()?;
    if account_gen != g {
        return None;
    }
    let read_at_ms = visible_record(&own, g).map(|r| r.read_at_ms);
    Some(ReadContext {
        gen: own.gen,
        account_gen,
        user_id,
        read_at_ms,
    })
}

/// Commits a finished read; true in the second place when windows must be told.
pub(crate) fn commit_read(
    gen0: u64,
    account_gen: u64,
    user_id: &str,
    outcome: &ReadOutcome,
    now: i64,
) -> (Verdict, bool) {
    write_own(|own, g| commit_read_in(own, g, gen0, account_gen, user_id, outcome, now))
}

/// Reads the settings and commits the answer, when a read is due under
/// `max_age_ms` (and none is running, and the back-off allows it).
pub(crate) async fn run_read(io: &dyn Gql, max_age_ms: i64) -> Option<Verdict> {
    let ctx = read_context()?;
    let attempt = begin_attempt(now_ms(), ctx.read_at_ms, max_age_ms, ctx.account_gen)?;
    let outcome = io.read().await;
    let (verdict, changed) =
        commit_read(ctx.gen, ctx.account_gen, &ctx.user_id, &outcome, now_ms());
    attempt.finish(verdict);
    if changed {
        apply_everywhere().await;
    }
    Some(verdict)
}

/// Asks for a read in the background, when one is due under `max_age_ms`.
pub(crate) fn ask_for_read(max_age_ms: i64) {
    if !background_allowed() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let _ = run_read(&WebGql, max_age_ms).await;
    });
}

fn background_allowed() -> bool {
    cfg!(not(test))
}

/// What `get_smiley_style` answers; asks for a read when the record is old.
pub fn style_answer() -> SmileyStyleAnswer {
    ask_for_read(ROW_FRESH_MS);
    let own = read_own();
    let record = visible_record(&own, TwitchService::account_generation());
    SmileyStyleAnswer {
        view: record.map(|r| r.view.clone()),
        rev: own.rev,
    }
}

/// What the windows need to dress their own copies, and the row to draw.
pub fn own_smilies() -> OwnSmilies {
    let own = read_own();
    let seen = observable(&own, TwitchService::account_generation());
    OwnSmilies {
        rev: own.rev,
        swap: seen.swap,
        view: seen.view,
        shown: seen.shown,
    }
}

/// Whether the settings row has anything to draw: a record exists for the
/// signed-in account.
pub fn row_shown() -> bool {
    if !smilies::state_visible() {
        return false;
    }
    let own = read_own();
    visible_record(&own, TwitchService::account_generation()).is_some()
}

fn emit_changed() {
    let Some(app) = crate::services::providers::app_handle() else {
        return;
    };
    use tauri::Emitter;
    let payload = own_smilies();
    let _ = app.emit(SMILIES_CHANGED_EVENT, &payload);
}

async fn app_emote_service() -> Option<EmoteService> {
    use tauri::Manager;
    let app = crate::services::providers::app_handle()?;
    let state = app.try_state::<crate::commands::emotes::EmoteServiceState>()?;
    let service = state.0.read().await.clone();
    Some(service)
}

/// Puts every copy the app holds in the account's style and tells the windows.
/// Each copy reads the style inside its own lock, so whichever apply runs last
/// leaves the newest.
pub(crate) async fn apply_everywhere_with(service: Option<&EmoteService>) {
    crate::services::irc_service::replace_own_smilies().await;
    if let Some(service) = service {
        service.normalize_cached_smilies().await;
    }
    emit_changed();
}

pub(crate) async fn apply_everywhere() {
    let service = app_emote_service().await;
    apply_everywhere_with(service.as_ref()).await;
}

fn apply_in_background() {
    if background_allowed() {
        tauri::async_runtime::spawn(async {
            apply_everywhere().await;
        });
    }
}

/// Files an emote answer for `user_id`. Called by every complete Helix answer,
/// whichever account's token it was fetched with.
pub fn note_helix(user_id: &str, started_at: i64, rows: Vec<Emote>) {
    let now = now_ms();
    let (wants, changed) =
        write_own(|own, g| note_helix_in(own, g, user_id, started_at, rows, now));
    if changed {
        apply_in_background();
    }
    if wants.is_some() {
        ask_for_read(0);
    }
}

/// The account's Twitch id is known (and the token it was read with is still
/// the signed-in one): load what was saved for it and start reading.
pub fn on_identity(user_id: &str, generation: u64) {
    let known = matches!(&read_own().identity, Some((u, g)) if u == user_id && *g == generation);
    if known {
        return;
    }
    let loaded = load_record(user_id);
    let (_, changed) = write_own(|own, g| on_identity_in(own, g, user_id, generation, loaded));
    if changed {
        apply_in_background();
    }
    // Every new account gets its first read soon, not only the first one this
    // process saw; the scheduler's later ticks keep it fresh.
    if background_allowed() && SCHEDULER.load(Ordering::SeqCst) {
        tauri::async_runtime::spawn(async {
            tokio::time::sleep(FIRST_READ_DELAY).await;
            let _ = run_read(&WebGql, 0).await;
        });
    }
    start_scheduler();
}

/// The signed-in account changed (a sign-in, a switch, a sign-out): forget the
/// old account's style at once, tell every window, and fetch the new
/// account's list until Helix has answered for it.
pub fn on_account_moved() {
    let fence = {
        let mut own = OWN.write().unwrap_or_else(|e| e.into_inner());
        own.gen += 1;
        own.rev += 1;
        own.identity = None;
        own.record = None;
        own.helix.clear();
        own.disagree_failures = 0;
        own.decided = None;
        own.gen
    };
    lock_refresh().failures = 0;
    if background_allowed() {
        tauri::async_runtime::spawn(after_account_moved(fence));
    }
}

/// Helix's side of the account move.
#[async_trait]
pub(crate) trait HelixReader: Send + Sync {
    /// The signed-in account's token now, if any.
    async fn token(&self) -> Option<String>;
    /// Fetches the signed-in user's emote list with `token`; the fetch files
    /// its own answer.
    async fn fetch(&self, token: &str) -> HelixOutcome;
}

struct AppHelix;

#[async_trait]
impl HelixReader for AppHelix {
    async fn token(&self) -> Option<String> {
        TwitchService::get_token()
            .await
            .ok()
            .filter(|t| !t.is_empty())
    }

    async fn fetch(&self, token: &str) -> HelixOutcome {
        match app_emote_service().await {
            Some(service) => {
                service
                    .fetch_user_twitch_emotes_typed(Some(token), None)
                    .await
                    .0
            }
            None => HelixOutcome::Failed,
        }
    }
}

/// The wait before the next try of the after-move loop: 2 s, 10 s, 60 s, then
/// doubling to an hour.
pub(crate) fn move_wait(step: usize) -> Duration {
    const FIRST: [u64; 3] = [2, 10, 60];
    match FIRST.get(step) {
        Some(secs) => Duration::from_secs(*secs),
        None => Duration::from_secs((60u64 << (step - 2).min(6)).min(3600)),
    }
}

fn filed_for_identity() -> bool {
    let own = read_own();
    own.identity
        .as_ref()
        .is_some_and(|(user_id, _)| own.helix.contains_key(user_id))
}

async fn after_account_moved(fence: u64) {
    let service = app_emote_service().await;
    after_account_moved_with(fence, service.as_ref(), &AppHelix, &move_wait).await;
}

/// Clears what the old account left in the emote cache, tells the windows, then
/// fetches the new account's emote list until it answers, the token is
/// refused, or the account moves again. A fetch that fails is tried again later;
/// a phone that is not in front waits.
pub(crate) async fn after_account_moved_with(
    fence: u64,
    service: Option<&EmoteService>,
    reader: &dyn HelixReader,
    wait: &(dyn Fn(usize) -> Duration + Sync),
) {
    if let Some(service) = service {
        service.clear_cache().await;
    }
    apply_everywhere_with(service).await;
    let mut step = 0usize;
    loop {
        crate::services::window_visibility::until_shown().await;
        if read_own().gen != fence {
            return;
        }
        let Some(token) = reader.token().await else {
            return;
        };
        if reader.fetch(&token).await == HelixOutcome::Refused {
            return;
        }
        if read_own().gen != fence || filed_for_identity() {
            return;
        }
        tokio::time::sleep(wait(step)).await;
        step += 1;
    }
}

static SCHEDULER: AtomicBool = AtomicBool::new(false);

/// One task per process: a first read soon after the account is known, then a
/// look at the clock every minute. A phone that is not in front skips the look.
fn start_scheduler() {
    if !background_allowed() || SCHEDULER.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(FIRST_READ_DELAY).await;
        let _ = run_read(&WebGql, 0).await;
        loop {
            tokio::time::sleep(SCHEDULER_TICK).await;
            if crate::services::window_visibility::phone_idle() {
                continue;
            }
            let _ = run_read(&WebGql, SCHEDULE_MAX_AGE_MS).await;
        }
    });
}

/// How long the switch waits between reads while Twitch's answer to the write
/// is unknown: the quick ones decide the switch's own result, the slow ones run
/// in the background after it.
pub(crate) struct Waits {
    pub quick: Vec<Duration>,
    pub slow: Vec<Duration>,
}

impl Waits {
    /// Reads at 2 s and 5 s after the write, then 15 s and 60 s.
    pub(crate) fn production() -> Self {
        Waits {
            quick: vec![Duration::from_secs(2), Duration::from_secs(3)],
            slow: vec![Duration::from_secs(10), Duration::from_secs(45)],
        }
    }
}

/// A switch that failed: what to tell the viewer, and whether to keep checking.
pub(crate) struct SwitchFailed {
    pub text: &'static str,
    pub check_later: Option<LaterCheck>,
}

/// What a background check needs to know about the switch it is checking.
pub(crate) struct LaterCheck {
    pub key: String,
    pub user_id: String,
    pub fence: u64,
}

fn failed(text: &'static str) -> SwitchFailed {
    SwitchFailed {
        text,
        check_later: None,
    }
}

static SWITCH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn identity_now() -> Option<(String, u64)> {
    let own = read_own();
    own.identity
        .clone()
        .filter(|(_, g)| *g == TwitchService::account_generation())
}

/// Switches the signed-in account to the set `key` names. The write is always
/// sent (a read can lag a switch made seconds ago; the write is idempotent).
pub(crate) async fn switch_with(
    key: &str,
    io: &dyn Gql,
    waits: &Waits,
) -> Result<SmileyStyle, SwitchFailed> {
    let _serial = SWITCH_LOCK.lock().await;
    let Some((user_id, _)) = identity_now() else {
        return Err(failed(TEXT_FAILED));
    };
    let fence = write_own(|own, _| {
        own.gen += 1;
        own.gen
    })
    .0;
    crate::services::twitch_integrity::user_asked();
    let fresh = match io.read().await {
        ReadOutcome::Read(read) if read.user_id == user_id => read,
        _ => return Err(failed(TEXT_FAILED)),
    };
    match fresh.view.sets.iter().find(|s| s.key == key) {
        Some(choice) if fresh.view.can_switch && choice.available => {}
        Some(_) => return Err(failed(TEXT_NEEDS_PRIME)),
        None => return Err(failed(TEXT_FAILED)),
    }
    let Some(set_id) = fresh
        .ids
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, id)| id.clone())
    else {
        return Err(failed(TEXT_NEEDS_PRIME));
    };
    let outcome = match io.write(&user_id, &set_id).await {
        Ok(json) => classify_write(&json, &set_id),
        Err(e) => {
            log::debug!("[{LOG_TAG}] write failed: {e}");
            WriteOutcome::Unknown
        }
    };
    let mut view = fresh.view.clone();
    match outcome {
        WriteOutcome::Refused => return Err(failed(TEXT_FAILED)),
        WriteOutcome::Confirmed => {}
        WriteOutcome::Unknown => {
            let mut shown = None;
            for wait in &waits.quick {
                tokio::time::sleep(*wait).await;
                if let ReadOutcome::Read(read) = io.read().await {
                    if read.user_id == user_id && read.view.current == key {
                        shown = Some(read.view);
                        break;
                    }
                }
            }
            match shown {
                Some(seen) => view = seen,
                None => {
                    return Err(SwitchFailed {
                        text: TEXT_UNCONFIRMED,
                        check_later: Some(LaterCheck {
                            key: key.to_string(),
                            user_id,
                            fence,
                        }),
                    })
                }
            }
        }
    }
    view.current = key.to_string();
    let now = now_ms();
    let (committed, changed) =
        write_own(|own, g| commit_switch_in(own, g, fence, &user_id, &view, now));
    if !committed {
        return Err(failed(TEXT_FAILED));
    }
    if changed {
        apply_everywhere().await;
    }
    Ok(view)
}

/// Reads after `waits` until one shows the set the switch asked for; commits it
/// then. Stops when the fence has moved (a later switch, an account move).
/// True when it committed.
pub(crate) async fn background_check_with(
    io: &dyn Gql,
    check: &LaterCheck,
    waits: &[Duration],
) -> bool {
    for wait in waits {
        tokio::time::sleep(*wait).await;
        if read_own().gen != check.fence {
            return false;
        }
        if let ReadOutcome::Read(read) = io.read().await {
            if read.user_id == check.user_id && read.view.current == check.key {
                let now = now_ms();
                let (committed, changed) = write_own(|own, g| {
                    commit_switch_in(own, g, check.fence, &check.user_id, &read.view, now)
                });
                if changed {
                    apply_everywhere().await;
                }
                return committed;
            }
        }
    }
    false
}

/// The command's switch: a failure that left the answer unknown keeps checking
/// in the background.
pub async fn switch(key: &str) -> Result<SmileyStyle, String> {
    match switch_with(key, &WebGql, &Waits::production()).await {
        Ok(view) => Ok(view),
        Err(failure) => {
            if let Some(check) = failure.check_later {
                tauri::async_runtime::spawn(async move {
                    let waits = Waits::production();
                    background_check_with(&WebGql, &check, &waits.slow).await;
                });
            }
            Err(failure.text.to_string())
        }
    }
}

/// What is saved for an account.
#[derive(Serialize, Deserialize)]
struct Stored {
    ver: u64,
    record: Record,
}

const FILE_PROVIDER: &str = "twitch_smiley_style";

/// The last version written for each account, so an older snapshot that gets
/// here late never replaces a newer file.
static SAVED: Lazy<Mutex<HashMap<String, u64>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[cfg(not(test))]
fn record_path(user_id: &str) -> Option<PathBuf> {
    crate::services::emote_set_cache::provider_path(FILE_PROVIDER, user_id)
}

#[cfg(test)]
static TEST_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_test_dir(dir: Option<PathBuf>) {
    *TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

#[cfg(test)]
fn record_path(user_id: &str) -> Option<PathBuf> {
    let dir = TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    Some(dir.join(format!("{FILE_PROVIDER}-{user_id}.json")))
}

/// Saves a record unless a newer version of it was already saved. Written to a
/// temporary file and renamed over the old one, so a crash leaves one or the other.
pub(crate) fn save_record(ver: u64, record: &Record) {
    let Some(path) = record_path(&record.user_id) else {
        return;
    };
    let mut saved = SAVED.lock().unwrap_or_else(|e| e.into_inner());
    if saved.get(&record.user_id).is_some_and(|last| *last >= ver) {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(&Stored {
        ver,
        record: record.clone(),
    }) else {
        return;
    };
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, &bytes).is_err() {
        return;
    }
    if std::fs::rename(&temp, &path).is_err() {
        let _ = std::fs::remove_file(&temp);
        return;
    }
    saved.insert(record.user_id.clone(), ver);
}

/// What was saved for an account; an unreadable file reads as nothing.
pub(crate) fn load_record(user_id: &str) -> Option<Record> {
    let bytes = std::fs::read(record_path(user_id)?).ok()?;
    let stored: Stored = serde_json::from_slice(&bytes).ok()?;
    (stored.record.user_id == user_id).then_some(stored.record)
}

/// Tests share the process-wide state; each one takes this for its whole run.
#[cfg(test)]
pub(crate) static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Ends a test: the state is cleared and the test gate closed again.
#[cfg(test)]
pub(crate) struct TestGuard;

/// Starts a test: opens the gate on this thread and clears the state.
#[cfg(test)]
pub(crate) fn begin_test() -> TestGuard {
    smilies::set_testing(true);
    reset_for_test();
    TestGuard
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    *OWN.write().unwrap_or_else(|e| e.into_inner()) = Own::new();
    *lock_refresh() = Refresh::default();
    SAVED.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Signs `user_id` in and commits a record, as a read would, whose current set is `key`.
/// Answers the account generation.
#[cfg(test)]
pub(crate) fn test_wear(user_id: &str, key: &str) -> u64 {
    let g = write_own(|own, g| {
        on_identity_in(own, g, user_id, g, None);
        g
    })
    .0;
    let answer: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/smiley_style_prime.json"))
            .expect("fixture json");
    let mut read = parse_read(&answer)
        .expect("parses")
        .expect("a signed-in answer");
    read.view.current = key.to_string();
    read.user_id = user_id.to_string();
    let gen0 = read_own().gen;
    commit_read(gen0, g, user_id, &ReadOutcome::Read(read), 1_000);
    g
}

#[cfg(test)]
impl Drop for TestGuard {
    fn drop(&mut self) {
        reset_for_test();
        smilies::set_testing(false);
        set_test_dir(None);
    }
}

#[cfg(test)]
#[path = "twitch_smiley_style_tests.rs"]
mod tests;
