//! The favorite channels: which channels are favorites
//! (`Settings.favorite_streamers`) and the name and face kept for each
//! (`Settings.favorite_channels`). Every change goes through
//! `commands::settings::set_favorite_channel` (and the identity fill here);
//! every other save keeps both lists as Rust holds them
//! (`Settings::adopt_backend_owned`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use tauri::Manager;

use crate::models::settings::{AppState, FavoriteChannel, Settings};
use crate::rt::AppHandle;
use crate::services::providers::key::{
    favorite_id, is_provider_id, is_youtube_channel_id, make_key, normalize_channel,
    strip_own_prefix,
};
use crate::services::twitch_service::TwitchService;

/// The settings keys only Rust writes. `updateSettings` on the page keeps the
/// same list (`RUST_ONLY_KEYS` in `src/utils/favoriteDoor.ts`).
pub const RUST_ONLY_KEYS: [&str; 2] = ["favorite_streamers", "favorite_channels"];

/// Whether a dotted settings path is inside one of the favorite lists.
pub fn is_rust_only_path(path: &str) -> bool {
    RUST_ONLY_KEYS.contains(&path.split('.').next().unwrap_or(""))
}

/// A channel as a press names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelPick {
    pub provider: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: Option<String>,
    pub avatar: Option<String>,
}

impl ChannelPick {
    /// Trimmed, the platform lowercased, empty strings as none.
    pub fn new(
        provider: &str,
        user_id: Option<String>,
        user_login: &str,
        user_name: Option<String>,
        avatar: Option<String>,
    ) -> Self {
        let filled = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        ChannelPick {
            provider: provider.trim().to_lowercase(),
            user_id: user_id.unwrap_or_default().trim().to_string(),
            user_login: user_login.trim().to_string(),
            user_name: filled(user_name),
            avatar: filled(avatar),
        }
    }
}

/// What a pick still needs before it names a channel.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    /// It names one already.
    None,
    /// A Twitch room known by login only.
    TwitchByLogin,
    /// A YouTube row known by video id only.
    YouTubeChannel,
}

pub fn lookup_needed(pick: &ChannelPick) -> Lookup {
    let login = strip_own_prefix(&pick.provider, &pick.user_login);
    if login.is_empty() || favorite_id(&pick.provider, &pick.user_id, &pick.user_login).is_some() {
        return Lookup::None;
    }
    match pick.provider.as_str() {
        "twitch" => Lookup::TwitchByLogin,
        "youtube" => Lookup::YouTubeChannel,
        _ => Lookup::None,
    }
}

/// The identifier chat and playback address: for YouTube the UC id when the
/// row has one, never the video id it arrived as.
fn channel_of(provider: &str, user_id: &str, user_login: &str) -> String {
    let user_id = strip_own_prefix(provider, user_id);
    let user_login = strip_own_prefix(provider, user_login);
    if provider == "youtube" && user_id.starts_with("UC") {
        return user_id.to_string();
    }
    normalize_channel(provider, user_login)
}

/// When a favorite was added, in the page's own `toISOString` shape.
pub fn now_stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The favorite a pick names: its id (`key::favorite_id`) and the name and
/// face kept for it. `None` when the pick names no channel.
pub fn identity(pick: &ChannelPick, now: &str) -> Option<FavoriteChannel> {
    let id = favorite_id(&pick.provider, &pick.user_id, &pick.user_login)?;
    Some(FavoriteChannel {
        id,
        provider: pick.provider.clone(),
        channel: channel_of(&pick.provider, &pick.user_id, &pick.user_login),
        display_name: pick.user_name.clone(),
        avatar: pick.avatar.clone(),
        added_at: now.to_string(),
    })
}

/// The two lists after setting `identity`'s channel to `favorite`, or `None`
/// when nothing changes. Adding keeps the stored entry's place, its `added_at`
/// and any name or face the pick lacks; removing drops every entry with the id.
pub fn apply(
    ids: &[String],
    channels: &[FavoriteChannel],
    identity: FavoriteChannel,
    favorite: bool,
) -> Option<(Vec<String>, Vec<FavoriteChannel>)> {
    let id = identity.id.clone();
    let listed = ids.iter().any(|i| *i == id);
    if !favorite {
        if !listed && !channels.iter().any(|c| c.id == id) {
            return None;
        }
        let next_ids: Vec<String> = ids.iter().filter(|i| **i != id).cloned().collect();
        let next_channels: Vec<FavoriteChannel> =
            channels.iter().filter(|c| c.id != id).cloned().collect();
        return Some((next_ids, next_channels));
    }
    let old = channels.iter().find(|c| c.id == id);
    let added_at = if listed {
        old.map(|o| o.added_at.clone())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| identity.added_at.clone())
    } else {
        identity.added_at.clone()
    };
    let merged = FavoriteChannel {
        display_name: identity
            .display_name
            .clone()
            .or_else(|| old.and_then(|o| o.display_name.clone())),
        avatar: identity
            .avatar
            .clone()
            .or_else(|| old.and_then(|o| o.avatar.clone())),
        added_at,
        ..identity
    };
    let mut next_ids = ids.to_vec();
    if !listed {
        next_ids.push(id.clone());
    }
    let at = channels.iter().position(|c| c.id == id);
    let mut next_channels: Vec<FavoriteChannel> =
        channels.iter().filter(|c| c.id != id).cloned().collect();
    let at = at.unwrap_or(next_channels.len()).min(next_channels.len());
    next_channels.insert(at, merged);
    if next_ids.as_slice() == ids && next_channels.as_slice() == channels {
        return None;
    }
    Some((next_ids, next_channels))
}

/// `apply` on the settings as Rust holds them.
pub fn favorite_patch(
    current: &Settings,
    identity: FavoriteChannel,
    favorite: bool,
) -> Option<(Vec<String>, Vec<FavoriteChannel>)> {
    apply(
        &current.favorite_streamers,
        &current.favorite_channels,
        identity,
        favorite,
    )
}

/// Whether a save changed either favorite list.
pub fn favorites_changed(before: &Settings, after: &Settings) -> bool {
    before.favorite_streamers != after.favorite_streamers
        || before.favorite_channels != after.favorite_channels
}

/// An id as it should be stored: a bare 24-character `UC` id (written by an
/// old sidebar with no prefix) becomes `youtube:UC...`, and a doubled prefix of
/// the id's own platform (`kick:kick:x`) collapses.
fn rekey(id: &str) -> String {
    if is_youtube_channel_id(id) {
        return make_key("youtube", id);
    }
    if let Some((provider, _)) = id.split_once(':') {
        if is_provider_id(provider) {
            let channel = strip_own_prefix(provider, id);
            return format!("{provider}:{channel}");
        }
    }
    id.to_string()
}

/// Put the favorite lists right on every load and import: legacy ids re-keyed
/// (see `rekey`), then both lists deduped by id (the first entry wins, keeping
/// the earliest `added_at` and any name or face a later duplicate had).
/// Answers whether anything changed.
pub fn repair(settings: &mut Settings) -> bool {
    let before = (
        settings.favorite_streamers.clone(),
        settings.favorite_channels.clone(),
    );
    let mut seen = HashSet::new();
    let ids: Vec<String> = settings
        .favorite_streamers
        .iter()
        .map(|id| rekey(id))
        .filter(|id| seen.insert(id.clone()))
        .collect();
    let mut channels: Vec<FavoriteChannel> = Vec::new();
    for entry in &settings.favorite_channels {
        let id = rekey(&entry.id);
        let mut entry = entry.clone();
        if id != entry.id {
            if id.starts_with("youtube:") && !entry.id.starts_with("youtube:") {
                entry.provider = "youtube".to_string();
            }
            entry.channel = strip_own_prefix(&entry.provider, &entry.channel).to_string();
            entry.id = id;
        }
        match channels.iter_mut().find(|c| c.id == entry.id) {
            Some(first) => {
                if first.display_name.is_none() {
                    first.display_name = entry.display_name.clone();
                }
                if first.avatar.is_none() {
                    first.avatar = entry.avatar.clone();
                }
                if !entry.added_at.is_empty()
                    && (first.added_at.is_empty() || entry.added_at < first.added_at)
                {
                    first.added_at = entry.added_at.clone();
                }
            }
            None => channels.push(entry),
        }
    }
    settings.favorite_streamers = ids;
    settings.favorite_channels = channels;
    (
        settings.favorite_streamers.clone(),
        settings.favorite_channels.clone(),
    ) != before
}

// ---- The identity fill ------------------------------------------------------
//
// Favorites saved as bare Twitch ids before the identity list existed have no
// name or face, so they cannot be drawn while offline. After each successful
// followed poll the fill asks Twitch for them, backing off after a failed or
// partial try, and skips an id Twitch has not returned three times this run.

/// How many tries an id may go unanswered before the fill stops asking.
const FILL_MISS_LIMIT: u32 = 3;

#[derive(Default)]
struct FillState {
    /// The account the counts below belong to.
    generation: u64,
    next_at: Option<Instant>,
    failures: u32,
    misses: HashMap<String, u32>,
}

static FILL: Lazy<Mutex<FillState>> = Lazy::new(|| Mutex::new(FillState::default()));
static FILL_RUNNING: AtomicBool = AtomicBool::new(false);

/// The wait after `failures` failed or partial tries in a row.
pub fn backoff_after(failures: u32) -> Duration {
    match failures {
        0 => Duration::ZERO,
        1 => Duration::from_secs(60),
        2 => Duration::from_secs(5 * 60),
        _ => Duration::from_secs(30 * 60),
    }
}

/// Forget the counts when the account changed; then whether a try may run now.
fn fill_due(state: &mut FillState, generation: u64, now: Instant) -> bool {
    if state.generation != generation {
        *state = FillState {
            generation,
            ..FillState::default()
        };
    }
    state.next_at.map_or(true, |at| now >= at)
}

/// Record one try: `missing` was asked for and `found` came back.
fn record_fill(
    state: &mut FillState,
    missing: &[String],
    found: &HashMap<String, (String, String, Option<String>)>,
    now: Instant,
) {
    let partial = missing.iter().any(|id| !found.contains_key(id));
    if partial {
        for id in missing.iter().filter(|id| !found.contains_key(*id)) {
            *state.misses.entry(id.clone()).or_default() += 1;
        }
        state.failures += 1;
        state.next_at = Some(now + backoff_after(state.failures));
    } else {
        state.failures = 0;
        state.next_at = None;
    }
}

/// The bare Twitch ids listed with no identity, minus those skipped.
fn missing_identities(settings: &Settings, misses: &HashMap<String, u32>) -> Vec<String> {
    let have: HashSet<&str> = settings
        .favorite_channels
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    settings
        .favorite_streamers
        .iter()
        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
        .filter(|id| !have.contains(id.as_str()))
        .filter(|id| misses.get(*id).copied().unwrap_or(0) < FILL_MISS_LIMIT)
        .cloned()
        .collect()
}

/// The lists with identities added for `found` (id to login, name, avatar),
/// only for ids still listed and still without one. `None` when none apply.
pub fn fill_patch(
    current: &Settings,
    found: &HashMap<String, (String, String, Option<String>)>,
) -> Option<(Vec<String>, Vec<FavoriteChannel>)> {
    let have: HashSet<&str> = current
        .favorite_channels
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let added: Vec<FavoriteChannel> = current
        .favorite_streamers
        .iter()
        .filter(|id| !have.contains(id.as_str()))
        .filter_map(|id| {
            let (login, name, avatar) = found.get(id)?;
            Some(FavoriteChannel {
                id: id.clone(),
                provider: "twitch".to_string(),
                channel: login.to_lowercase(),
                display_name: Some(name.clone()).filter(|n| !n.is_empty()),
                avatar: avatar.clone().filter(|a| !a.is_empty()),
                added_at: String::new(),
            })
        })
        .collect();
    if added.is_empty() {
        return None;
    }
    let mut channels = current.favorite_channels.clone();
    channels.extend(added);
    Some((current.favorite_streamers.clone(), channels))
}

/// Fill in a name and face for favorites saved as bare Twitch ids. Called
/// after each successful followed poll; a no-op until its backoff allows.
pub fn spawn_fill_identities(app: AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let generation = TwitchService::account_generation();
    let missing = {
        let Ok(settings) = state.settings.lock() else {
            return;
        };
        let Ok(mut fill) = FILL.lock() else { return };
        if !fill_due(&mut fill, generation, Instant::now()) {
            return;
        }
        missing_identities(&settings, &fill.misses)
    };
    if missing.is_empty() || FILL_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let found = TwitchService::users_by_ids(&missing).await;
        if let Ok(mut fill) = FILL.lock() {
            if fill.generation == generation {
                record_fill(&mut fill, &missing, &found, Instant::now());
            }
        }
        if !found.is_empty() && TwitchService::account_generation() == generation {
            if let Some(state) = app.try_state::<AppState>() {
                match crate::commands::settings::commit_favorite_patch(&app, &state, |current| {
                    fill_patch(current, &found)
                }) {
                    Ok(true) => log::info!(
                        "[Favorites] filled in identity for {} channel(s)",
                        found.len()
                    ),
                    Ok(false) => {}
                    Err(e) => log::warn!("[Favorites] identity fill write failed: {e}"),
                }
            }
        }
        FILL_RUNNING.store(false, Ordering::Release);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fav(id: &str, provider: &str, channel: &str) -> FavoriteChannel {
        FavoriteChannel {
            id: id.into(),
            provider: provider.into(),
            channel: channel.into(),
            display_name: None,
            avatar: None,
            added_at: String::new(),
        }
    }

    fn pick(provider: &str, user_id: &str, login: &str) -> ChannelPick {
        ChannelPick::new(provider, Some(user_id.into()), login, None, None)
    }

    #[test]
    fn identity_keys_and_addresses_each_platform() {
        let t = identity(
            &ChannelPick::new(
                "twitch",
                Some("71092938".into()),
                "XQC",
                Some("xQc".into()),
                Some("a.png".into()),
            ),
            "T",
        )
        .unwrap();
        assert_eq!(
            (t.id.as_str(), t.provider.as_str(), t.channel.as_str()),
            ("71092938", "twitch", "xqc")
        );
        assert_eq!(
            (
                t.display_name.as_deref(),
                t.avatar.as_deref(),
                t.added_at.as_str()
            ),
            (Some("xQc"), Some("a.png"), "T")
        );
        let k = identity(&pick("kick", "12345", "kick:XQC"), "T").unwrap();
        assert_eq!((k.id.as_str(), k.channel.as_str()), ("kick:xqc", "xqc"));
        let y = identity(
            &pick("youtube", "UCMNEVbszv8ZyvSXoTn3yhpQ", "AGr94tpNVkw"),
            "T",
        )
        .unwrap();
        assert_eq!(
            (y.id.as_str(), y.channel.as_str()),
            (
                "youtube:UCMNEVbszv8ZyvSXoTn3yhpQ",
                "UCMNEVbszv8ZyvSXoTn3yhpQ"
            )
        );
        let h = identity(&pick("youtube", "", "@Chan"), "T").unwrap();
        assert_eq!(
            (h.id.as_str(), h.channel.as_str()),
            ("youtube:@Chan", "@Chan")
        );
        assert_eq!(identity(&pick("youtube", "", "AGr94tpNVkw"), "T"), None);
        assert_eq!(identity(&pick("twitch", "", "xqc"), "T"), None);
        // Empty names and faces are none, so they never overwrite a kept one.
        let blank = ChannelPick::new("kick", None, "xqc", Some("  ".into()), Some("".into()));
        assert_eq!((blank.user_name, blank.avatar), (None, None));
    }

    #[test]
    fn a_pick_says_what_it_still_needs() {
        assert_eq!(
            lookup_needed(&pick("twitch", "", "xqc")),
            Lookup::TwitchByLogin
        );
        assert_eq!(lookup_needed(&pick("twitch", "1", "xqc")), Lookup::None);
        assert_eq!(
            lookup_needed(&pick("youtube", "", "AGr94tpNVkw")),
            Lookup::YouTubeChannel
        );
        assert_eq!(
            lookup_needed(&pick("youtube", "UCabc", "AGr94tpNVkw")),
            Lookup::None
        );
        assert_eq!(lookup_needed(&pick("kick", "", "xqc")), Lookup::None);
        assert_eq!(lookup_needed(&pick("twitch", "", "")), Lookup::None);
    }

    #[test]
    fn adding_appends_once_and_keeps_order() {
        let ids = vec!["1".to_string()];
        let channels = vec![fav("1", "twitch", "one")];
        let (ids2, ch2) = apply(&ids, &channels, fav("kick:two", "kick", "two"), true).unwrap();
        assert_eq!(ids2, vec!["1", "kick:two"]);
        assert_eq!(
            ch2.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "kick:two"]
        );
        assert_eq!(
            apply(&ids2, &ch2, fav("kick:two", "kick", "two"), true),
            None
        );
    }

    #[test]
    fn a_new_face_refreshes_the_entry_and_keeps_its_date_and_place() {
        let ids = vec!["1".to_string(), "2".to_string()];
        let mut old = fav("1", "twitch", "one");
        old.added_at = "2026-01-01T00:00:00.000Z".into();
        old.avatar = Some("old.png".into());
        let channels = vec![old, fav("2", "twitch", "two")];
        let mut new = fav("1", "twitch", "one");
        new.avatar = Some("new.png".into());
        new.added_at = "2026-10-31T00:00:00.000Z".into();
        let (ids2, ch2) = apply(&ids, &channels, new, true).unwrap();
        assert_eq!(ids2, ids);
        assert_eq!(ch2[0].id, "1");
        assert_eq!(ch2[0].avatar.as_deref(), Some("new.png"));
        assert_eq!(ch2[0].added_at, "2026-01-01T00:00:00.000Z");
    }

    #[test]
    fn a_readd_without_a_face_keeps_the_stored_one() {
        let ids = vec!["1".to_string()];
        let mut old = fav("1", "twitch", "one");
        old.avatar = Some("kept.png".into());
        old.display_name = Some("One".into());
        let channels = vec![old.clone()];
        assert_eq!(
            apply(&ids, &channels, fav("1", "twitch", "one"), true),
            None
        );
    }

    #[test]
    fn removing_drops_every_entry_with_the_id_and_nothing_else() {
        let ids = vec!["1".to_string(), "2".to_string(), "1".to_string()];
        let channels = vec![
            fav("1", "twitch", "one"),
            fav("2", "twitch", "two"),
            fav("1", "twitch", "one"),
        ];
        let (ids2, ch2) = apply(&ids, &channels, fav("1", "twitch", "one"), false).unwrap();
        assert_eq!(ids2, vec!["2"]);
        assert_eq!(
            ch2.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["2"]
        );
        assert_eq!(apply(&ids2, &ch2, fav("1", "twitch", "one"), false), None);
    }

    #[test]
    fn the_patch_reads_the_settings_lists() {
        let mut s = Settings::default();
        assert_eq!(favorite_patch(&s, fav("kick:a", "kick", "a"), false), None);
        let (ids, ch) = favorite_patch(&s, fav("kick:a", "kick", "a"), true).unwrap();
        s.favorite_streamers = ids;
        s.favorite_channels = ch;
        assert_eq!(favorite_patch(&s, fav("kick:a", "kick", "a"), true), None);
    }

    #[test]
    fn a_change_to_either_list_counts() {
        let a = Settings::default();
        let mut b = Settings::default();
        assert!(!favorites_changed(&a, &b));
        b.favorite_channels = vec![fav("1", "twitch", "one")];
        assert!(favorites_changed(&a, &b));
        let mut c = Settings::default();
        c.favorite_streamers = vec!["1".into()];
        assert!(favorites_changed(&a, &c));
    }

    #[test]
    fn repair_rekeys_legacy_ids_collapses_prefixes_and_dedupes() {
        let uc = "UCMNEVbszv8ZyvSXoTn3yhpQ";
        let mut s = Settings::default();
        s.favorite_streamers = vec![
            uc.into(),
            "kick:kick:xqc".into(),
            "kick:xqc".into(),
            "180118013".into(),
            format!("youtube:{uc}"),
        ];
        let mut first = fav("kick:xqc", "kick", "xqc");
        first.added_at = "2026-05-01T00:00:00.000Z".into();
        let mut doubled = fav("kick:kick:xqc", "kick", "kick:xqc");
        doubled.added_at = "2026-02-01T00:00:00.000Z".into();
        doubled.avatar = Some("face.png".into());
        s.favorite_channels = vec![fav(uc, "twitch", uc), first, doubled];
        assert!(repair(&mut s));
        assert_eq!(
            s.favorite_streamers,
            vec![
                format!("youtube:{uc}"),
                "kick:xqc".to_string(),
                "180118013".to_string()
            ]
        );
        assert_eq!(s.favorite_channels.len(), 2);
        assert_eq!(s.favorite_channels[0].id, format!("youtube:{uc}"));
        assert_eq!(s.favorite_channels[0].provider, "youtube");
        assert_eq!(s.favorite_channels[0].channel, uc);
        assert_eq!(s.favorite_channels[1].id, "kick:xqc");
        assert_eq!(s.favorite_channels[1].channel, "xqc");
        assert_eq!(s.favorite_channels[1].added_at, "2026-02-01T00:00:00.000Z");
        assert_eq!(s.favorite_channels[1].avatar.as_deref(), Some("face.png"));
        assert!(!repair(&mut s), "a second pass changes nothing");
    }

    #[test]
    fn repair_leaves_good_ids_alone() {
        let mut s = Settings::default();
        s.favorite_streamers = vec![
            "180118013".into(),
            "youtube:UCMNEVbszv8ZyvSXoTn3yhpQ".into(),
            "ucantseeme".into(),
        ];
        assert!(!repair(&mut s));
    }

    #[test]
    fn the_fill_adds_only_listed_ids_still_missing_one() {
        let mut s = Settings::default();
        s.favorite_streamers = vec!["1".into(), "2".into()];
        s.favorite_channels = vec![fav("2", "twitch", "two")];
        let mut found = HashMap::new();
        found.insert(
            "1".to_string(),
            (
                "One".to_string(),
                "One".to_string(),
                Some("1.png".to_string()),
            ),
        );
        found.insert(
            "2".to_string(),
            ("Two".to_string(), "Two".to_string(), None),
        );
        // Removed while the lookup ran: never re-added.
        found.insert(
            "3".to_string(),
            ("Three".to_string(), "Three".to_string(), None),
        );
        let (ids, ch) = fill_patch(&s, &found).unwrap();
        assert_eq!(ids, s.favorite_streamers);
        assert_eq!(
            ch.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["2", "1"]
        );
        assert_eq!(ch[1].channel, "one");
        assert_eq!(ch[1].avatar.as_deref(), Some("1.png"));
        s.favorite_channels = ch;
        assert_eq!(fill_patch(&s, &found), None);
    }

    #[test]
    fn the_fill_asks_only_for_bare_twitch_ids_and_skips_repeated_misses() {
        let mut s = Settings::default();
        s.favorite_streamers = vec!["1".into(), "kick:x".into(), "2".into(), "3".into()];
        s.favorite_channels = vec![fav("2", "twitch", "two")];
        let mut misses = HashMap::new();
        misses.insert("3".to_string(), FILL_MISS_LIMIT);
        assert_eq!(missing_identities(&s, &misses), vec!["1"]);
    }

    #[test]
    fn the_fill_backs_off_after_a_partial_try_and_resets_on_an_account_change() {
        let now = Instant::now();
        let mut state = FillState::default();
        assert!(fill_due(&mut state, 5, now));
        let missing = vec!["1".to_string(), "2".to_string()];
        let mut found = HashMap::new();
        found.insert("1".to_string(), ("a".to_string(), "A".to_string(), None));
        record_fill(&mut state, &missing, &found, now);
        assert_eq!(state.failures, 1);
        assert_eq!(state.misses.get("2"), Some(&1));
        assert!(!fill_due(&mut state, 5, now + Duration::from_secs(59)));
        assert!(fill_due(&mut state, 5, now + Duration::from_secs(60)));
        record_fill(&mut state, &missing, &HashMap::new(), now);
        assert_eq!(state.failures, 2);
        assert!(!fill_due(&mut state, 5, now + Duration::from_secs(299)));
        record_fill(&mut state, &missing, &HashMap::new(), now);
        assert_eq!(state.next_at, Some(now + Duration::from_secs(1800)));
        // Another account starts clean.
        assert!(fill_due(&mut state, 6, now));
        assert_eq!((state.failures, state.misses.len()), (0, 0));
        // A full answer clears the wait.
        record_fill(
            &mut state,
            &missing,
            &{
                let mut all = found.clone();
                all.insert("2".to_string(), ("b".to_string(), "B".to_string(), None));
                all
            },
            now,
        );
        assert_eq!((state.failures, state.next_at), (0, None));
    }

    #[test]
    fn the_rust_only_paths_are_the_two_lists() {
        assert!(is_rust_only_path("favorite_streamers"));
        assert!(is_rust_only_path("favorite_channels.0.avatar"));
        assert!(!is_rust_only_path("favorite_categories"));
        assert!(!is_rust_only_path(
            "live_notifications.show_favorite_live_notifications"
        ));
    }
}
