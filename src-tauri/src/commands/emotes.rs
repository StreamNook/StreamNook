use crate::services::emote_credit::{self as credit, CreditProvider, EmoteCredit};
use crate::services::emote_match::{self, Collector, Context, Seed, Slot, Spec};
use crate::services::emote_picker as picker;
use crate::services::emote_prefetch_service::emote_cache_key;
use crate::services::emote_service::{seventv_globals_snapshot, Emote, EmoteService, EmoteSet};
use crate::services::providers::{kick_emotes, youtube, youtube_emotes};
use crate::services::universal_cache_service::{self, CacheType};
use crate::services::account_store::AccountStore;
use crate::services::{cache_service, irc_service};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

pub struct EmoteServiceState(pub Arc<RwLock<EmoteService>>);

/// The 7TV image size a page renders at, from what it asked for. 7TV is the
/// only provider cached per size; the others have one file per emote.
pub(crate) fn render_tier(tier: Option<&str>) -> &'static str {
    match tier {
        Some("1x") => "1x",
        Some("3x") => "3x",
        Some("4x") => "4x",
        _ => "2x",
    }
}

/// Put each emote's disk-cache file in `local_url`, at `tier` for 7TV, from
/// the in-memory manifest (one read lock for the whole set). A page used to
/// work this out itself, which meant pulling the entire cache index (tens of
/// thousands of id -> path pairs, megabytes over IPC) into every window that
/// shows emotes. Only ever applied to the copy handed to a page: a path goes
/// stale when the cache is cleared, so the cached and stored sets never hold one.
pub(crate) fn stamp_local_paths<'a>(emotes: impl IntoIterator<Item = &'a mut Emote>, tier: &str) {
    let mut emotes: Vec<&mut Emote> = emotes.into_iter().collect();
    let keys: Vec<String> = emotes.iter().map(|e| emote_cache_key(&e.provider, &e.id, tier)).collect();
    let paths = universal_cache_service::cached_file_paths(CacheType::Emote, &keys);
    if paths.is_empty() {
        return;
    }
    for (emote, key) in emotes.iter_mut().zip(&keys) {
        emote.local_url = paths.get(key).cloned();
    }
}

pub(crate) fn stamp_set_paths(set: &mut EmoteSet, tier: &str) {
    let EmoteSet { twitch, bttv, seven_tv, ffz, kick, .. } = set;
    stamp_local_paths(
        twitch.iter_mut().chain(bttv.iter_mut()).chain(seven_tv.iter_mut()).chain(ffz.iter_mut()).chain(kick.iter_mut()),
        tier,
    );
}

/// A channel's emote set for a page, with disk-cache paths filled in for the
/// size the page renders. Rust finds the signed-in viewer's token itself
/// (their sub, follower and bits emotes), so no page ever holds it.
#[tauri::command]
pub async fn fetch_channel_emotes(
    channel_name: Option<String>,
    channel_id: Option<String>,
    // Which platform `channel_id` belongs to. Absent = twitch, so callers that
    // predate multi-platform are unchanged.
    provider: Option<String>,
    tier: Option<String>,
    state: State<'_, EmoteServiceState>,
) -> Result<EmoteSet, String> {
    // Only Twitch has per-viewer emotes. Signed out is fine: globals and the
    // channel's third-party sets still come back.
    let access_token = match provider.as_deref() {
        None | Some("twitch") => crate::services::twitch_service::TwitchService::get_token().await.ok(),
        _ => None,
    };
    let service = state.0.read().await;
    let mut set = service
        .fetch_channel_emotes(channel_name, channel_id, access_token, provider)
        .await
        .map_err(|e| e.to_string())?;
    drop(service);
    stamp_set_paths(&mut set, render_tier(tier.as_deref()));
    Ok(set)
}

#[tauri::command]
pub async fn get_emote_by_name(
    channel_id: Option<String>,
    emote_name: String,
    tier: Option<String>,
    state: State<'_, EmoteServiceState>,
) -> Result<Option<Emote>, String> {
    let service = state.0.read().await;
    let mut emote = service.get_emote_by_name(channel_id, &emote_name).await;
    drop(service);
    if let Some(e) = emote.as_mut() {
        stamp_local_paths(std::iter::once(e), render_tier(tier.as_deref()));
    }
    Ok(emote)
}

#[derive(Serialize)]
pub struct EmoteMatchResult {
    pub rows: Vec<emote_match::Row>,
    /// How many emotes matched in all, before the row cap.
    pub total: usize,
    /// False when the channel's emotes are not in memory yet.
    pub ready: bool,
    /// Matches per provider before the row cap (`twitch`, `7tv`, ...).
    pub counts: emote_match::ProviderCounts,
}

/// Emotes matching what the user is typing, ranked, for the Tab cycle
/// (`profile` "cycle"), the emote list ("search") or the emote fill menu
/// ("fill": the desktop colon list and both phones' strip, see
/// `emote_match::Request`). Reads the sets already in memory and never
/// fetches, so it is cheap enough to run on every keystroke.
///
/// `channel` and `channel_id` are what `ensureChannelEmotes` fetched the set
/// with: a Twitch login and user id, a Kick slug, or a YouTube identifier.
/// `tier` is the 7TV image size the page renders, for the disk-cache lookup.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn match_emote_tokens(
    provider: Option<String>,
    channel: String,
    channel_id: Option<String>,
    query: String,
    profile: Option<String>,
    order: Option<String>,
    tier: Option<String>,
    limit: Option<usize>,
    picker: Option<bool>,
    typing: Option<bool>,
    state: State<'_, EmoteServiceState>,
) -> Result<EmoteMatchResult, String> {
    // `typing`: asked as the viewer types (the fill menu), not on Tab; a word
    // under `MIN_PREFIX_CHARS` finds only an exact emote name.
    let request = emote_match::Request::resolve(profile.as_deref(), order.as_deref(), limit, typing);
    let (profile, order, limit, typing) = (request.profile, request.order, request.limit, request.typing);
    let tier = render_tier(tier.as_deref());
    let channel_id = channel_id.filter(|id| !id.is_empty());

    // Taken before any set is locked, so nothing awaits under those locks.
    let seventv_globals: HashSet<String> = seventv_globals_snapshot().await.into_iter().map(|e| e.id).collect();
    let favorites = cache_service::favorite_emote_ids();
    let ffz_subwoofer = crate::commands::ffz::cached_is_subwoofer().await;

    let mut collector = Collector::new(
        Spec {
            query: &query,
            profile,
            order,
            contains: emote_match::contains_mode(),
            limit,
        },
        Context {
            channel_id: channel_id.as_deref(),
            favorites: &favorites,
            seventv_globals: &seventv_globals,
            ffz_subwoofer,
        },
    )
    // The emote menu shows FFZ subscriber effects locked instead of hiding them.
    .include_locked(picker.unwrap_or(false))
    .typing(typing);

    let ready = match provider.as_deref().unwrap_or("twitch") {
        "twitch" => {
            // The viewer's own 7TV personal emotes work in every Twitch channel,
            // and in their own messages they render in place of a channel emote
            // of the exactly same name; the collector drops that channel row.
            if let Some(me) = AccountStore::primary() {
                irc_service::with_personal_emotes(&me.user_id, |map| {
                    for (name, e) in map {
                        collector.offer(Slot::SevenTv, name, &e.id, false, || Seed {
                            id: e.id.clone(),
                            url: e.url.clone(),
                            insert_text: None,
                            emote_type: Some(emote_match::PERSONAL_EMOTE_TYPE.to_string()),
                            is_zero_width: e.is_zero_width,
                            modifier_flags: e.modifier_flags,
                            global: true,
                            own: false,
                            owner_name: e.owner_name.clone(),
                            original_name: e.original_name.clone(),
                            animated: e.animated,
                            width: e.width,
                        });
                    }
                });
            }
            let joined = irc_service::with_channel_emotes(&channel, |set| collector.offer_set(set))
                .await
                .is_some();
            joined
                || match channel_id.as_deref() {
                    Some(id) => {
                        let service = state.0.read().await;
                        service.with_cached_set(id, |set| collector.offer_set(set)).await.is_some()
                    }
                    None => false,
                }
        }
        "kick" => {
            let seventv = kick_emotes::with_seventv(&channel, |map| offer_seventv_map(&mut collector, map)).is_some();
            let native = kick_emotes::with_native(&channel, |list| {
                for e in list {
                    let global = emote_match::kick_set_is_global(&e.set);
                    collector.offer(Slot::Kick, &e.name, &e.id, false, || Seed {
                        id: e.id.clone(),
                        url: format!("https://files.kick.com/emotes/{}/fullsize", e.id),
                        insert_text: None,
                        emote_type: Some(e.set.clone()),
                        is_zero_width: Some(false),
                        modifier_flags: None,
                        global,
                        own: !global,
                        ..Default::default()
                    });
                }
            })
            .is_some();
            seventv || native
        }
        "youtube" => {
            let seventv = youtube_emotes::with_seventv(&channel, |map| offer_seventv_map(&mut collector, map)).is_some();
            let emojis = youtube::channel_emoji_set(&channel)
                .or_else(|| channel_id.as_deref().and_then(youtube::channel_emoji_set));
            if let Some(list) = &emojis {
                for e in list.iter().filter(|e| !e.locked) {
                    // Unicode entries send the character itself; custom emoji ids
                    // are `UC…/hash`, which is what tells the two apart.
                    let insert_text = (e.is_global && !e.id.contains('/')).then(|| e.id.clone());
                    collector.offer(Slot::YouTube, &e.name, &e.id, false, || Seed {
                        id: e.id.clone(),
                        url: e.url.clone(),
                        insert_text,
                        emote_type: None,
                        is_zero_width: None,
                        modifier_flags: None,
                        global: e.is_global,
                        own: !e.is_global,
                        ..Default::default()
                    });
                }
            }
            seventv || emojis.is_some()
        }
        // No emote set exists for this platform (TikTok).
        _ => true,
    };

    let (mut rows, total, counts) = collector.finish_counted();

    // Disk-first images for the rows that made the cut.
    let keys: Vec<Option<String>> = rows
        .iter()
        .map(|r| r.slot.cache_provider().map(|p| emote_cache_key(&p, &r.id, tier)))
        .collect();
    let wanted: Vec<String> = keys.iter().flatten().cloned().collect();
    let paths = universal_cache_service::cached_file_paths(CacheType::Emote, &wanted);
    for (row, key) in rows.iter_mut().zip(keys) {
        row.local_path = key.and_then(|k| paths.get(&k).cloned());
    }

    // The cycle and the fill menu offer the room's recent chatters after the
    // emotes (provider `user`); a colon word names none (`chatter_rows`).
    // A word too short for the emote list names no chatter either.
    if request.chatters && !(typing && emote_match::is_short_word(&query)) {
        let provider = provider.as_deref().unwrap_or("twitch");
        let recent = crate::services::chat_history::ChatHistory::recent_chatters(
            &emote_match::history_room(provider, &channel),
            0,
        );
        // The streamer leads the chatters, whether or not they have chatted.
        let owner = room_owner(provider, &channel, channel_id.as_deref(), &recent);
        let chatters = emote_match::chatter_rows(
            &query,
            &rows,
            emote_match::contains_mode(),
            &emote_match::owner_first(owner, recent),
        );
        rows.extend(chatters);
    }

    Ok(EmoteMatchResult { rows, total, ready, counts })
}

/// Display names of subscription emote owners (and of the streamers the `@`
/// list leads with), kept for the life of the process. An id Helix did not
/// answer is not asked again; its section keeps the "Channel <id>" title.
type OwnerNames = (HashMap<String, String>, HashSet<String>);

fn owner_names() -> &'static std::sync::Mutex<OwnerNames> {
    static NAMES: std::sync::OnceLock<std::sync::Mutex<OwnerNames>> = std::sync::OnceLock::new();
    NAMES.get_or_init(Default::default)
}

/// Names for `ids`, asking Helix once (in batches of 100) for any not yet known.
async fn resolve_owner_names(ids: Vec<String>) -> HashMap<String, String> {
    let missing: Vec<String> = {
        let guard = owner_names().lock().unwrap_or_else(|e| e.into_inner());
        ids.iter().filter(|id| !guard.0.contains_key(*id) && !guard.1.contains(*id)).cloned().collect()
    };
    if !missing.is_empty() {
        let found = crate::services::twitch_service::TwitchService::users_by_ids(&missing).await;
        let mut guard = owner_names().lock().unwrap_or_else(|e| e.into_inner());
        for id in missing {
            match found.get(&id) {
                Some((_, display, _)) => {
                    guard.0.insert(id, display.clone());
                }
                None => {
                    guard.1.insert(id);
                }
            }
        }
    }
    let guard = owner_names().lock().unwrap_or_else(|e| e.into_inner());
    ids.into_iter().filter_map(|id| guard.0.get(&id).map(|n| (id, n.clone()))).collect()
}

/// Twitch user `id`'s display name from the owner-name cache, without waiting.
/// A miss asks Helix once in the background, so the next keystroke has it.
fn owner_name_now(id: &str) -> Option<String> {
    {
        let guard = owner_names().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(name) = guard.0.get(id) {
            return Some(name.clone());
        }
        if guard.1.contains(id) {
            return None;
        }
    }
    static ASKING: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> = std::sync::OnceLock::new();
    let asking = ASKING.get_or_init(Default::default);
    if asking.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string()) {
        let id = id.to_string();
        tauri::async_runtime::spawn(async move {
            resolve_owner_names(vec![id.clone()]).await;
            asking.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        });
    }
    None
}

/// The room's streamer as a chatter `(login, user_id, display)`, so every
/// chatter list can lead with them: their own row once they have chatted, else
/// in a Twitch room the room's login with the cached display name (the login
/// until Helix answers). A Kick or YouTube room names its streamer only once
/// they chat: a Kick room key is its slug, which need not be the username, and
/// a YouTube room key is a channel id.
fn room_owner(
    provider: &str,
    channel: &str,
    channel_id: Option<&str>,
    recent: &[(String, String, String)],
) -> Option<(String, String, String)> {
    let login = channel.trim().trim_start_matches('#').to_lowercase();
    if let Some(owner) = emote_match::find_owner(&login, channel_id, recent) {
        return Some(owner);
    }
    if provider != "twitch" || login.is_empty() {
        return None;
    }
    let id = channel_id.unwrap_or_default().to_string();
    let display = if id.is_empty() { None } else { owner_name_now(&id) };
    let display = display.unwrap_or_else(|| login.clone());
    Some((login, id, display))
}

/// The composer's `@` list for a room: its streamer first, then its recent
/// chatters, newest first, whose login, shown name or nickname starts with
/// `query`. A command's username argument (`command_arg`, as in /ban or
/// /timeout) keeps plain recency: most of those cannot act on the streamer.
#[tauri::command]
pub async fn mention_suggestions(
    provider: Option<String>,
    channel: String,
    channel_id: Option<String>,
    query: String,
    command_arg: Option<bool>,
) -> Vec<emote_match::MentionRow> {
    let provider = provider.as_deref().unwrap_or("twitch");
    let channel_id = channel_id.filter(|id| !id.is_empty());
    let recent =
        crate::services::chat_history::ChatHistory::recent_chatters(&emote_match::history_room(provider, &channel), 0);
    let owner = if command_arg.unwrap_or(false) {
        None
    } else {
        room_owner(provider, &channel, channel_id.as_deref(), &recent)
    };
    emote_match::mention_rows(provider, &query, &emote_match::owner_first(owner, recent), emote_match::MENTION_LIMIT)
}

/// Saved favorite emote ids, in the order they were saved.
fn favorite_id_list() -> Vec<String> {
    cache_service::load_favorite_emotes()
        .ok()
        .flatten()
        .and_then(|data| serde_json::from_str::<Vec<serde_json::Value>>(&data).ok())
        .map(|list| {
            list.iter()
                .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// The emote picker for a room: its tabs with counts, and the sections of
/// `tab` (the room's default when absent) filtered by `query`, each cell with
/// its flags and its disk-cache file at `tier`. Reads the sets already in
/// memory, like `match_emote_tokens`; `ready` is false until the room's set
/// has been fetched (`fetch_channel_emotes`).
#[tauri::command]
pub async fn emote_picker(
    provider: Option<String>,
    channel: String,
    channel_id: Option<String>,
    tab: Option<String>,
    query: Option<String>,
    tier: Option<String>,
    state: State<'_, EmoteServiceState>,
) -> Result<picker::PickerAnswer, String> {
    use picker::{Room, Tab};
    let room = Room::parse(provider.as_deref());
    let channel_id = channel_id.filter(|id| !id.is_empty());
    let tab = tab.as_deref().and_then(Tab::parse);
    let query = query.unwrap_or_default();
    let tier = render_tier(tier.as_deref());

    let personal = match room {
        Room::Twitch => AccountStore::primary()
            .and_then(|me| irc_service::with_personal_emotes(&me.user_id, |map| map.clone())),
        _ => None,
    };
    let favorites = favorite_id_list();
    let recent = crate::services::recent_emotes::ids();
    let ffz_subwoofer = crate::commands::ffz::cached_is_subwoofer().await;

    // The joined chat's set first (it takes live 7TV changes), else the one
    // the page fetched for the channel.
    let joined = room == Room::Twitch && irc_service::with_channel_emotes(&channel, |_| ()).await.is_some();
    let owners = if joined {
        irc_service::with_channel_emotes(&channel, picker::subscription_owners).await
    } else {
        match channel_id.as_deref() {
            Some(id) => state.0.read().await.with_cached_set(id, picker::subscription_owners).await,
            None => None,
        }
    }
    .unwrap_or_default();
    let names = if owners.is_empty() { HashMap::new() } else { resolve_owner_names(owners).await };

    let ctx = picker::Context {
        room,
        channel_id: channel_id.as_deref(),
        favorites: &favorites,
        recent: &recent,
        ffz_subwoofer,
        owner_names: &names,
        personal: personal.as_ref(),
    };
    let run = |set: &EmoteSet| picker::build(Some(set), &ctx, tab, &query);
    let found = if joined {
        irc_service::with_channel_emotes(&channel, run).await
    } else if room == picker::Room::Kick {
        // A Kick room carries no Twitch channel id: its set is the slug's,
        // native emotes and 7TV, from the Kick caches the chat resolve fills.
        let set = crate::services::providers::kick_emotes::channel_emote_set(&channel).await;
        Some(run(&set))
    } else {
        match channel_id.as_deref() {
            Some(id) => state.0.read().await.with_cached_set(id, run).await,
            None => None,
        }
    };
    let mut answer = found.unwrap_or_else(|| picker::build(None, &ctx, tab, &query));
    stamp_local_paths(answer.sections.iter_mut().flat_map(|s| s.emotes.iter_mut().map(|c| &mut c.emote)), tier);
    Ok(answer)
}

/// A channel emote that the viewer's own 7TV personal emote of the same name
/// replaces in their own messages.
#[derive(Serialize, Debug, PartialEq)]
pub struct PersonalOverride {
    pub name: String,
    /// The personal emote that shows instead.
    pub id: String,
    pub url: String,
}

/// Names in a Twitch channel's third-party sets (7TV, BTTV, FFZ) that the
/// signed-in viewer's personal set also uses. Typing one of them shows the
/// personal emote, not the channel's, so the picker marks those tiles. Reads
/// only what is in memory; empty until both sets are known.
#[tauri::command]
pub async fn personal_emote_overrides(
    channel: String,
    channel_id: Option<String>,
    state: State<'_, EmoteServiceState>,
) -> Result<Vec<PersonalOverride>, String> {
    let Some(me) = AccountStore::primary() else {
        return Ok(Vec::new());
    };
    let Some(personal) = irc_service::with_personal_emotes(&me.user_id, |map| map.clone()) else {
        return Ok(Vec::new());
    };
    if personal.is_empty() {
        return Ok(Vec::new());
    }
    let find = |set: &EmoteSet| overrides_in(set, &personal);
    if let Some(found) = irc_service::with_channel_emotes(&channel, find).await {
        return Ok(found);
    }
    let Some(id) = channel_id.filter(|id| !id.is_empty()) else {
        return Ok(Vec::new());
    };
    let service = state.0.read().await;
    Ok(service.with_cached_set(&id, find).await.unwrap_or_default())
}

fn overrides_in(set: &EmoteSet, personal: &HashMap<String, Emote>) -> Vec<PersonalOverride> {
    let mut seen = HashSet::new();
    let mut out: Vec<PersonalOverride> = set
        .seven_tv
        .iter()
        .chain(&set.ffz)
        .chain(&set.bttv)
        .filter_map(|e| {
            let mine = personal.get(&e.name)?;
            // Already the same emote: nothing changes when it is typed.
            if mine.id == e.id || !seen.insert(e.name.clone()) {
                return None;
            }
            Some(PersonalOverride { name: e.name.clone(), id: mine.id.clone(), url: mine.url.clone() })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 7TV emotes held as a name -> emote map (the Kick and YouTube stores).
fn offer_seventv_map<E: SeventvMapEntry>(collector: &mut Collector<'_>, map: &HashMap<String, E>) {
    for (name, e) in map {
        let global = collector.is_seventv_global(e.id());
        collector.offer(Slot::SevenTv, name, e.id(), false, || Seed {
            id: e.id().to_string(),
            url: e.url().to_string(),
            insert_text: None,
            emote_type: None,
            is_zero_width: Some(e.zero_width()),
            modifier_flags: None,
            owner_name: e.owner_name().map(str::to_string),
            original_name: e.original_name().map(str::to_string),
            animated: None,
            width: None,
            global,
            own: !global,
        });
    }
}

trait SeventvMapEntry {
    fn id(&self) -> &str;
    fn url(&self) -> &str;
    fn zero_width(&self) -> bool;
    fn owner_name(&self) -> Option<&str>;
    fn original_name(&self) -> Option<&str>;
}

impl SeventvMapEntry for kick_emotes::KickEmote {
    fn id(&self) -> &str {
        &self.id
    }
    fn url(&self) -> &str {
        &self.url
    }
    fn zero_width(&self) -> bool {
        self.zero_width
    }
    fn owner_name(&self) -> Option<&str> {
        self.owner_name.as_deref()
    }
    fn original_name(&self) -> Option<&str> {
        self.original_name.as_deref()
    }
}

impl SeventvMapEntry for youtube_emotes::YouTubeEmote {
    fn id(&self) -> &str {
        &self.id
    }
    fn url(&self) -> &str {
        &self.url
    }
    fn zero_width(&self) -> bool {
        self.zero_width
    }
    fn owner_name(&self) -> Option<&str> {
        self.owner_name.as_deref()
    }
    fn original_name(&self) -> Option<&str> {
        self.original_name.as_deref()
    }
}

#[tauri::command]
pub async fn clear_emote_cache(state: State<'_, EmoteServiceState>) -> Result<(), String> {
    let service = state.0.read().await;
    service.clear_cache().await;
    credit::clear();
    Ok(())
}

/// Who made an emote, for a hover card that just opened. `provider` is
/// `twitch`, `7tv`, `bttv` or `ffz`; `name` is the name it is shown under;
/// `channel` / `channel_id` are the room it is shown in. The sets Rust holds
/// answer first, then the provider's per-emote API (cached). `detail` is the
/// click card: creator chips (picture, link) and the emote's page. None when
/// the provider or id is not one this can look up.
#[tauri::command]
pub async fn emote_credit(
    provider: String,
    id: String,
    name: Option<String>,
    channel: Option<String>,
    channel_id: Option<String>,
    detail: Option<bool>,
    state: State<'_, EmoteServiceState>,
) -> Result<Option<EmoteCredit>, String> {
    let Some(provider) = CreditProvider::parse(&provider) else {
        return Ok(None);
    };
    if !provider.valid_id(&id) {
        return Ok(None);
    }
    let seventv_global = provider == CreditProvider::SevenTv
        && seventv_globals_snapshot().await.iter().any(|e| e.id == id);
    let mut held = None;
    if let Some(channel) = channel.as_deref().filter(|c| !c.is_empty()) {
        held = irc_service::with_channel_emotes(channel, |set| {
            credit::find_in_set(set, provider, &id, seventv_global)
        })
        .await
        .flatten();
    }
    if held.is_none() {
        if let Some(cid) = channel_id.as_deref().filter(|c| !c.is_empty()) {
            let service = state.0.read().await;
            held = service
                .with_cached_set(cid, |set| credit::find_in_set(set, provider, &id, seventv_global))
                .await
                .flatten();
        }
    }
    // The viewer's own 7TV personal emotes show in every channel.
    if held.is_none() && provider == CreditProvider::SevenTv {
        if let Some(me) = AccountStore::primary() {
            held = irc_service::with_personal_emotes(&me.user_id, |map| {
                map.values().find(|e| e.id == id).map(|e| credit::held_from(provider, e, false))
            })
            .flatten();
        }
    }
    Ok(credit::resolve(provider, &id, name.as_deref(), held, detail.unwrap_or(false)).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::emote_service::EmoteProvider;

    fn emote(provider: EmoteProvider, id: &str, name: &str) -> Emote {
        Emote {
            id: id.into(),
            name: name.into(),
            url: format!("https://example.invalid/{id}"),
            provider,
            is_zero_width: None,
            local_url: None,
            emote_type: None,
            owner_id: None,
            owner_name: None,
            original_name: None,
            owner_ref: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
        }
    }

    #[test]
    fn overrides_name_channel_emotes_a_personal_one_replaces() {
        let mut set = EmoteSet::new();
        set.seven_tv = vec![emote(EmoteProvider::SevenTV, "pumpkin", "buh"), emote(EmoteProvider::SevenTV, "same", "glorp")];
        set.bttv = vec![emote(EmoteProvider::BTTV, "b1", "buh"), emote(EmoteProvider::BTTV, "b2", "catJAM")];
        set.twitch = vec![emote(EmoteProvider::Twitch, "t1", "Kappa")];
        let personal: HashMap<String, Emote> = [
            ("buh".to_string(), emote(EmoteProvider::SevenTV, "mine", "buh")),
            ("glorp".to_string(), emote(EmoteProvider::SevenTV, "same", "glorp")),
            ("Kappa".to_string(), emote(EmoteProvider::SevenTV, "mykappa", "Kappa")),
        ]
        .into_iter()
        .collect();
        let found = overrides_in(&set, &personal);
        // One entry per name; the identical glorp and Twitch's own Kappa are not overrides.
        assert_eq!(
            found,
            vec![PersonalOverride { name: "buh".into(), id: "mine".into(), url: "https://example.invalid/mine".into() }]
        );
    }
}
