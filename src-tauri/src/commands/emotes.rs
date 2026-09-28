use crate::services::emote_match::{self, Collector, Context, Order, Profile, Seed, Slot, Spec};
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

#[tauri::command]
pub async fn fetch_channel_emotes(
    channel_name: Option<String>,
    channel_id: Option<String>,
    access_token: Option<String>,
    // Which platform `channel_id` belongs to. Absent = twitch, so callers that
    // predate multi-platform are unchanged.
    provider: Option<String>,
    state: State<'_, EmoteServiceState>,
) -> Result<EmoteSet, String> {
    let service = state.0.read().await;
    service
        .fetch_channel_emotes(channel_name, channel_id, access_token, provider)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_emote_by_name(
    channel_id: Option<String>,
    emote_name: String,
    state: State<'_, EmoteServiceState>,
) -> Result<Option<Emote>, String> {
    let service = state.0.read().await;
    Ok(service.get_emote_by_name(channel_id, &emote_name).await)
}

#[derive(Serialize)]
pub struct EmoteMatchResult {
    pub rows: Vec<emote_match::Row>,
    /// How many emotes matched in all, before the row cap.
    pub total: usize,
    /// False when the channel's emotes are not in memory yet.
    pub ready: bool,
}

/// Emotes matching what the user is typing, ranked, for the Tab cycle
/// (`profile` "cycle") or the emote list ("search"). Reads the sets already in
/// memory and never fetches, so it is cheap enough to run on every keystroke.
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
    state: State<'_, EmoteServiceState>,
) -> Result<EmoteMatchResult, String> {
    let profile = match profile.as_deref() {
        Some("search") => Profile::Search,
        _ => Profile::Cycle,
    };
    let order = match order.as_deref() {
        Some("twitch_first") => Order::TwitchFirst,
        _ => Order::Default,
    };
    let limit = limit.unwrap_or(match profile {
        Profile::Cycle => 50,
        Profile::Search => 60,
    });
    let tier = match tier.as_deref() {
        Some(t @ ("1x" | "2x" | "3x" | "4x")) => t,
        _ => "2x",
    };
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
    );

    let ready = match provider.as_deref().unwrap_or("twitch") {
        "twitch" => {
            let joined = irc_service::with_channel_emotes(&channel, |set| collector.offer_set(set))
                .await
                .is_some();
            let ready = joined
                || match channel_id.as_deref() {
                    Some(id) => {
                        let service = state.0.read().await;
                        service.with_cached_set(id, |set| collector.offer_set(set)).await.is_some()
                    }
                    None => false,
                };
            // The viewer's own 7TV personal emotes work in every Twitch channel.
            // Offered after the channel's rows, so a channel alias keeps its name.
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
                        });
                    }
                });
            }
            ready
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
                    });
                }
            }
            seventv || emojis.is_some()
        }
        // No emote set exists for this platform (TikTok).
        _ => true,
    };

    let (mut rows, total) = collector.finish();

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

    Ok(EmoteMatchResult { rows, total, ready })
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
            global,
            own: !global,
        });
    }
}

trait SeventvMapEntry {
    fn id(&self) -> &str;
    fn url(&self) -> &str;
    fn zero_width(&self) -> bool;
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
}

#[tauri::command]
pub async fn clear_emote_cache(state: State<'_, EmoteServiceState>) -> Result<(), String> {
    let service = state.0.read().await;
    service.clear_cache().await;
    Ok(())
}
