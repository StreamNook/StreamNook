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
    );

    let ready = match provider.as_deref().unwrap_or("twitch") {
        "twitch" => {
            // The viewer's own 7TV personal emotes work in every Twitch channel,
            // and in their own messages they render in place of a channel emote
            // of the same name. Offered first, so they also keep a name that
            // differs from a channel one only by case.
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
        .chain(&set.bttv)
        .chain(&set.ffz)
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
            width: None,
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
