//! TikTok LIVE chat adapter (read-only).
//!
//! Reads a creator's LIVE webcast over TikTok's anonymous WebSocket and normalizes
//! each event into the shared `ChatMessage` published onto the local-WS bus. The
//! webcast protocol (room-id resolve -> ttwid device cookie -> signed-free WSS ->
//! gzip'd protobuf push frames) lives in the vendored `tiktok-live` crate; this
//! file maps its events onto our model.
//!
//! No login, no signing service: TikTok hands out a `ttwid` device cookie to any
//! anonymous GET, and the webcast socket accepts it for an audience connection. If
//! TikTok ever hardens the handshake, the documented fallback is EulerStream's
//! signing API (it ships a Rust SDK and a per-user free tier).
//!
//! What we surface (the multistreamer command-center asks): live chat, follows,
//! gifts (roses etc.), shares, and hearts/likes -> the activity feed + inline. Sends
//! are ban-risk on TikTok, so `send_capability` reports read-only.

use super::{
    dec_bridge_users, inc_bridge_users, key, publish_chat_message, publish_frame, ChatProvider,
    SendCapability, SendOutcome,
};
use crate::models::chat_layout::{
    Badge, ChatMessage, LayoutResult, MessageMetadata, MessageSegment,
};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use tiktok_live::http::api::{fetch_room_id, fetch_room_info, FetchParams};
use tiktok_live::http::sigi::scrape_profile;
use tiktok_live::http::ttwid::fetch_ttwid;
use tiktok_live::structs::proto::{
    Image, UserIdentity, WebcastChatMessage, WebcastControlMessage, WebcastGiftMessage,
    WebcastImDeleteMessage, WebcastLikeMessage, WebcastRoomUserSeqMessage, WebcastSocialMessage,
};
use tiktok_live::structs::TikTokLiveEvent;
use tiktok_live::TikTokLive;

// After a LIVE ends / the socket gives up, wait this long before re-resolving so a
// creator who drops and comes back (or restarts) reconnects on their own.
const RE_RESOLVE_DELAY_SECS: u64 = 20;
// Internal WSS reconnect budget before the stream surfaces as ended. Kept low so our
// own outer loop re-resolves the room id quickly (the right move for go-live detection).
const WS_MAX_RETRIES: u32 = 3;
// Per-(channel, user) like throttle. Hearts fire in bursts; without this a busy room
// floods chat + the activity feed. One like event per user per window is plenty.
const LIKE_THROTTLE_MS: u128 = 4000;

static FALLBACK_SEQ: AtomicU64 = AtomicU64::new(0);

// TEMP discovery logging: sample the first few of each event kind so the real field
// shapes (avatar presence, gift combos, like bursts) show in the log and we can
// decide consolidation. Strip once the patterns are confirmed.
static CHAT_SAMPLES: AtomicU64 = AtomicU64::new(0);
static GIFT_SAMPLES: AtomicU64 = AtomicU64::new(0);
static LIKE_SAMPLES: AtomicU64 = AtomicU64::new(0);
static SOCIAL_SAMPLES: AtomicU64 = AtomicU64::new(0);
static SEQ_SAMPLES: AtomicU64 = AtomicU64::new(0);

fn sample(counter: &AtomicU64, limit: u64) -> bool {
    counter.fetch_add(1, Ordering::Relaxed) < limit
}

struct Connection {
    consumers: HashSet<String>,
    // None while the first liveness resolve is in flight; Some once the stream task
    // is spawned. Reserving the slot before the (network) resolve stops a concurrent
    // connect for the same handle from spinning a second stream.
    task: Option<JoinHandle<()>>,
}

pub struct TikTokProvider {
    // Active stream tasks keyed by the lowercased handle.
    conns: Mutex<HashMap<String, Connection>>,
}

impl TikTokProvider {
    pub fn new() -> Self {
        Self {
            conns: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ChatProvider for TikTokProvider {
    fn id(&self) -> &'static str {
        "tiktok"
    }

    async fn connect(&self, channel: &str, window: &str) -> Result<()> {
        let handle = clean_handle(channel);
        let id_lc = handle.to_lowercase();
        {
            let mut conns = self.conns.lock().await;
            if let Some(conn) = conns.get_mut(&id_lc) {
                conn.consumers.insert(window.to_string());
                return Ok(());
            }
            let mut consumers = HashSet::new();
            consumers.insert(window.to_string());
            conns.insert(
                id_lc.clone(),
                Connection {
                    consumers,
                    task: None,
                },
            );
        }

        // Count this as a bridge user BEFORE the (network) resolve, mirroring Kick /
        // YouTube: otherwise a concurrent Twitch start_chat could see "no providers"
        // mid-resolve and tear the shared local-WS bridge down.
        inc_bridge_users();

        // Resolve the room id now so a clear "not live" error surfaces on the pane
        // immediately (the way Kick/YouTube resolve do). The streaming task re-resolves
        // on its own if the LIVE later ends and restarts.
        if let Err(e) = fetch_room_id(&handle, params()).await {
            dec_bridge_users();
            self.conns.lock().await.remove(&id_lc);
            return Err(anyhow::anyhow!(friendly_resolve_error(&e)));
        }
        // Resolve the chrome (name / avatar / viewers / title) right away so the
        // header fills on the pane's first poll. A partial seed here would satisfy
        // the frontend's "meta arrived" check and stop its fast-poll before the
        // avatar lands, so resolve the real thing instead of seeding a stub.
        {
            let h = handle.clone();
            let id = id_lc.clone();
            tokio::spawn(async move { refresh_meta(&h, &id).await });
        }

        let task = {
            let handle = handle.clone();
            let id_lc2 = id_lc.clone();
            let channel_key = key::make_key("tiktok", &handle);
            tokio::spawn(async move { run_connection(handle, channel_key, id_lc2).await })
        };
        let mut conns = self.conns.lock().await;
        match conns.get_mut(&id_lc) {
            Some(conn) if conn.task.is_none() => conn.task = Some(task),
            _ => {
                task.abort();
                dec_bridge_users();
            }
        }
        Ok(())
    }

    async fn disconnect(&self, channel: &str, window: &str) -> Result<()> {
        let id_lc = clean_handle(channel).to_lowercase();
        let mut conns = self.conns.lock().await;
        let drop_it = if let Some(conn) = conns.get_mut(&id_lc) {
            conn.consumers.remove(window);
            conn.consumers.is_empty()
        } else {
            false
        };
        if drop_it {
            if let Some(conn) = conns.remove(&id_lc) {
                if let Some(task) = conn.task {
                    task.abort();
                    dec_bridge_users();
                }
            }
        }
        Ok(())
    }

    async fn release_window(&self, window: &str) {
        let mut conns = self.conns.lock().await;
        conns.retain(|id_lc, conn| {
            if !conn.consumers.remove(window) || !conn.consumers.is_empty() {
                return true;
            }
            // Live connection (task Some) released here; a still-resolving one
            // (task None) is released by connect()'s attach/failure path.
            if let Some(task) = conn.task.take() {
                task.abort();
                dec_bridge_users();
                log::debug!("[TikTok] released '{}' with window '{}'", id_lc, window);
            }
            false
        });
    }

    async fn send(&self, _channel: &str, _text: &str, _reply_to: Option<&str>) -> Result<SendOutcome> {
        // Sending to TikTok LIVE from outside the app is ban-risk, so v1 is read-only.
        Ok(SendOutcome {
            message_id: None,
            is_sent: false,
            drop_reason: Some("Sending to TikTok isn't available".to_string()),
        })
    }

    async fn send_capability(&self, _channel: &str) -> SendCapability {
        SendCapability::ReadOnly
    }
}

fn params<'a>() -> FetchParams<'a> {
    FetchParams {
        timeout: Duration::from_secs(10),
        ..Default::default()
    }
}

/// Strip a leading `@` / a `tiktok:`/`tiktok/` prefix the key codec or a paste might
/// carry; the webcast API wants the bare unique id.
fn clean_handle(channel: &str) -> String {
    channel
        .trim()
        .trim_start_matches("tiktok:")
        .trim_start_matches("tiktok/")
        .trim_start_matches('@')
        .to_string()
}

fn friendly_resolve_error(e: &tiktok_live::errors::TikTokLiveError) -> String {
    use tiktok_live::errors::TikTokLiveError as E;
    match e {
        E::UserNotFound(u) => format!("@{} not found on TikTok", u),
        E::HostNotOnline(_) | E::RoomIdMissing => "This TikTok creator isn't live right now".to_string(),
        E::AgeRestricted(_) => "This TikTok LIVE is age-restricted".to_string(),
        other => format!("Couldn't connect to TikTok LIVE: {}", other),
    }
}

// --- Streaming loop ---------------------------------------------------------

/// Own the webcast stream for a handle: (re)build it, consume events, and re-resolve
/// after it ends so a creator going offline then live again reconnects. The task is
/// aborted by `disconnect` when the last consumer leaves.
async fn run_connection(handle: String, channel_key: String, id_lc: String) {
    loop {
        match TikTokLive::builder(&handle)
            .max_retries(WS_MAX_RETRIES)
            .connect()
            .await
        {
            Ok(mut stream) => {
                while let Some(event) = stream.next_event().await {
                    match event {
                        TikTokLiveEvent::Connected { .. } => {
                            log::info!("[TikTok] connected to @{} (LIVE)", handle);
                            // Enrich the chrome (avatar / name / viewers / title) once the
                            // socket is up; viewers then track live off RoomUserSeq.
                            let h = handle.clone();
                            let id = id_lc.clone();
                            tokio::spawn(async move { refresh_meta(&h, &id).await });
                        }
                        TikTokLiveEvent::Disconnected => break,
                        other => handle_event(other, &channel_key, &id_lc).await,
                    }
                }
                log::info!("[TikTok] stream ended for @{}", handle);
            }
            Err(e) => {
                log::info!("[TikTok] {} not connectable: {}", handle, e);
            }
        }
        set_live(&id_lc, false);
        tokio::time::sleep(Duration::from_secs(RE_RESOLVE_DELAY_SECS)).await;
    }
}

/// Map one webcast event onto the bus. Only the convenience-routed variants are
/// handled (Follow/Share over raw Social); raw Social/Member are ignored to avoid
/// double counting.
async fn handle_event(event: TikTokLiveEvent, channel_key: &str, id_lc: &str) {
    match event {
        TikTokLiveEvent::Chat(m) => {
            if let Some(msg) = build_chat_message(&m, channel_key) {
                publish_chat_message(&msg).await;
            }
        }
        TikTokLiveEvent::Gift(m) => {
            if let Some(msg) = build_gift_message(&m, channel_key) {
                publish_chat_message(&msg).await;
            }
        }
        TikTokLiveEvent::Follow(m) => {
            if let Some(msg) = build_social_message(&m, channel_key, "tiktok_follow", "followed") {
                publish_chat_message(&msg).await;
            }
        }
        TikTokLiveEvent::Share(m) => {
            if let Some(msg) = build_social_message(&m, channel_key, "tiktok_share", "shared") {
                publish_chat_message(&msg).await;
            }
        }
        TikTokLiveEvent::Like(m) => {
            if let Some(msg) = build_like_message(&m, channel_key) {
                publish_chat_message(&msg).await;
            }
        }
        TikTokLiveEvent::RoomUserSeq(m) => update_viewers(id_lc, &m),
        TikTokLiveEvent::ImDelete(m) => emit_deletions(&m, channel_key).await,
        TikTokLiveEvent::LiveEnded(_) => set_live(id_lc, false),
        _ => {}
    }
}

// --- Message builders -------------------------------------------------------

fn build_chat_message(m: &WebcastChatMessage, channel_key: &str) -> Option<ChatMessage> {
    let user = m.user.as_ref()?;
    if sample(&CHAT_SAMPLES, 8) {
        log::info!(
            "[TikTok][chat] nick='{}' uid={} avatar={} badges(list={} legacy={})",
            user.nickname,
            user.user_id,
            image_url(&user.avatar_thumb)
                .or_else(|| image_url(&user.avatar_medium))
                .is_some(),
            user.badge_list.len(),
            user.user_badges.len(),
        );
    }
    let id = msg_id(m.common.as_ref().map(|c| c.msg_id).unwrap_or(0));
    let segments = vec![MessageSegment::Text {
        content: m.comment.clone(),
    }];
    Some(base_message(
        user,
        channel_key,
        id,
        segments,
        m.comment.clone(),
        None,
        None,
    ))
}

/// A gift. Combo gifts fire many events during a streak; only emit when the streak is
/// over (or for non-combo gifts) so the final count shows once, not per tick.
fn build_gift_message(m: &WebcastGiftMessage, channel_key: &str) -> Option<ChatMessage> {
    let user = m.user.as_ref()?;
    if sample(&GIFT_SAMPLES, 20) {
        log::info!(
            "[TikTok][gift] from='{}' giftId={} repeat={} group={} repeat_end={} combo={} streak_over={} diamonds_each={:?} name={:?}",
            user.nickname,
            m.gift_id,
            m.repeat_count,
            m.group_count,
            m.repeat_end,
            m.is_combo_gift(),
            m.is_streak_over(),
            m.gift_details.as_ref().map(|g| g.diamond_count),
            m.gift_details.as_ref().map(|g| g.gift_name.as_str()),
        );
    }
    if !m.is_streak_over() {
        return None;
    }
    let id = msg_id(m.common.as_ref().map(|c| c.msg_id).unwrap_or(0));
    let gift_name = m
        .gift_details
        .as_ref()
        .map(|g| {
            if !g.gift_name.is_empty() {
                g.gift_name.clone()
            } else {
                g.describe.clone()
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "a gift".to_string());
    let count = m.repeat_count.max(1);
    let diamonds = m.diamond_total().max(0);
    let icon_url = m
        .gift_details
        .as_ref()
        .and_then(|g| image_url(&g.gift_image).or_else(|| image_url(&g.icon)));
    let phrase = if count > 1 {
        format!("sent {} \u{00d7}{}", gift_name, count)
    } else {
        format!("sent {}", gift_name)
    };
    let diamond_text = format!("{} diamond{}", diamonds, if diamonds == 1 { "" } else { "s" });
    let system = format!("{} ({})", phrase, diamond_text);
    // Lead with the gift's own (often animated) icon so chat shows WHICH gift it is.
    let mut segments = Vec::new();
    if let Some(ref url) = icon_url {
        segments.push(MessageSegment::Emote {
            content: gift_name.clone(),
            emote_id: None,
            emote_url: url.clone(),
            is_zero_width: Some(false),
            modifier_flags: None,
            is_personal: None,
        });
        segments.push(MessageSegment::Text {
            content: format!(" {}", phrase),
        });
    } else {
        segments.push(MessageSegment::Text {
            content: phrase.clone(),
        });
    }
    let mut msg = base_message(
        user,
        channel_key,
        id,
        segments,
        phrase,
        None,
        Some(("tiktok_gift", system)),
    );
    msg.tags.insert("tt-gift-name".to_string(), gift_name);
    msg.tags.insert("tt-gift-count".to_string(), count.to_string());
    msg.tags
        .insert("tt-gift-diamonds".to_string(), diamonds.to_string());
    if let Some(url) = icon_url {
        msg.tags.insert("tt-gift-image".to_string(), url);
    }
    Some(msg)
}

fn build_social_message(
    m: &WebcastSocialMessage,
    channel_key: &str,
    kind: &str,
    verb: &str,
) -> Option<ChatMessage> {
    let user = m.user.as_ref()?;
    if sample(&SOCIAL_SAMPLES, 12) {
        log::info!("[TikTok][social] {} from='{}'", verb, user.nickname);
    }
    let id = msg_id(m.common.as_ref().map(|c| c.msg_id).unwrap_or(0));
    let segments = vec![MessageSegment::Text {
        content: verb.to_string(),
    }];
    Some(base_message(
        user,
        channel_key,
        id,
        segments,
        verb.to_string(),
        None,
        Some((kind, verb.to_string())),
    ))
}

/// Hearts. Throttled per user so a burst-y room doesn't flood; `like_count` is the
/// size of the tap burst TikTok already batched into this message.
fn build_like_message(m: &WebcastLikeMessage, channel_key: &str) -> Option<ChatMessage> {
    let user = m.user.as_ref()?;
    if sample(&LIKE_SAMPLES, 20) {
        log::info!(
            "[TikTok][like] from='{}' like_count={} total_like={}",
            user.nickname, m.like_count, m.total_like_count,
        );
    }
    let uid = user.user_id;
    if !like_allowed(channel_key, uid) {
        return None;
    }
    let n = m.like_count.max(1);
    let id = msg_id(m.common.as_ref().map(|c| c.msg_id).unwrap_or(0));
    let phrase = format!("sent {} like{}", n, if n == 1 { "" } else { "s" });
    let segments = vec![MessageSegment::Text {
        content: phrase.clone(),
    }];
    let mut msg = base_message(
        user,
        channel_key,
        id,
        segments,
        phrase.clone(),
        None,
        Some(("tiktok_like", phrase)),
    );
    msg.tags.insert("tt-like-count".to_string(), n.to_string());
    Some(msg)
}

/// Assemble a ChatMessage from a webcast user + prepared segments. `event`
/// (msg_type, system message) marks gift/follow/share/like so they render inline and
/// flow to the activity feed (same tag vocabulary Twitch/Kick/YouTube events use).
fn base_message(
    user: &UserIdentity,
    channel_key: &str,
    id: String,
    segments: Vec<MessageSegment>,
    content: String,
    color_override: Option<String>,
    event: Option<(&str, String)>,
) -> ChatMessage {
    let display_name = if !user.nickname.is_empty() {
        user.nickname.clone()
    } else {
        user.unique_id.clone()
    };
    let user_id = user.user_id.to_string();
    let color = color_override.or_else(|| Some(color_for(&user_id)));
    let badges = parse_badges(user);

    let mut tags = HashMap::new();
    tags.insert("display-name".to_string(), display_name.clone());
    tags.insert("id".to_string(), id.clone());
    if let Some(url) = image_url(&user.avatar_thumb).or_else(|| image_url(&user.avatar_medium)) {
        // The chatter's profile picture rides every event; stamp it so the frontend
        // renders TikTok's native inline avatar (same path as YouTube).
        tags.insert("avatar".to_string(), url);
    }

    let metadata = match &event {
        Some((kind, system)) => {
            tags.insert("msg-id".to_string(), (*kind).to_string());
            tags.insert("system-msg".to_string(), system.clone());
            MessageMetadata {
                msg_type: Some((*kind).to_string()),
                system_message: Some(system.clone()),
                ..Default::default()
            }
        }
        None => MessageMetadata::default(),
    };

    ChatMessage {
        id,
        user_id,
        username: if user.unique_id.is_empty() {
            display_name.to_lowercase()
        } else {
            user.unique_id.clone()
        },
        display_name,
        color,
        badges,
        timestamp: chrono::Utc::now().to_rfc3339(),
        content,
        provider: "tiktok".to_string(),
        channel: channel_key.to_string(),
        emotes: Vec::new(),
        tags,
        layout: LayoutResult {
            height: 0.0,
            width: 0.0,
            has_reply: false,
            is_first_message: false,
        },
        segments,
        metadata,
    }
}

/// TikTok chat badges carry real artwork in the payload (fans-club, top-gifter,
/// moderator, subscriber, etc.), so we emit only image-backed badges -> the frontend
/// renders them by url with no bundled art needed.
fn parse_badges(user: &UserIdentity) -> Vec<Badge> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for b in &user.badge_list {
        if let Some(ib) = &b.image_badge {
            push_badge(&mut out, &mut seen, image_url(&ib.image), "");
        } else if let Some(cb) = &b.combine_badge {
            push_badge(&mut out, &mut seen, image_url(&cb.icon), &cb.str_value);
        }
    }
    // Older streams expose direct badge images instead of the structured list.
    if out.is_empty() {
        for img in &user.user_badges {
            push_badge(&mut out, &mut seen, img.url_list.first().cloned(), "");
        }
    }
    out
}

fn push_badge(out: &mut Vec<Badge>, seen: &mut HashSet<String>, url: Option<String>, title: &str) {
    let Some(u) = url else { return };
    if u.is_empty() || !seen.insert(u.clone()) {
        return;
    }
    out.push(Badge {
        name: "tiktok".to_string(),
        version: "1".to_string(),
        image_url_1x: Some(u),
        image_url_2x: None,
        image_url_4x: None,
        title: (!title.is_empty()).then(|| title.to_string()),
        description: None,
    });
}

// --- Moderation observation -------------------------------------------------

/// Mirror TikTok's message/user deletions onto the same CLEARMSG/CLEARCHAT frames the
/// Twitch path emits, so the existing deletion overlay + mod log pick them up.
async fn emit_deletions(m: &WebcastImDeleteMessage, channel_key: &str) {
    for msg_id in &m.delete_msg_ids_list {
        let frame = serde_json::json!({
            "type": "CLEARMSG",
            "provider": "tiktok",
            "channel": channel_key,
            "target_msg_id": msg_id.to_string(),
        });
        publish_frame(frame.to_string()).await;
    }
    for user_id in &m.delete_user_ids_list {
        let frame = serde_json::json!({
            "type": "CLEARCHAT",
            "provider": "tiktok",
            "channel": channel_key,
            "target_user_id": user_id.to_string(),
        });
        publish_frame(frame.to_string()).await;
    }
}

// --- Channel metadata (chrome) ----------------------------------------------

/// Live metadata for the MultiChat chrome. Field names mirror Kick/YouTube so the
/// frontend reads all three through one `ProviderChannelMeta` shape. `user_id` is the
/// creator's numeric id as a string.
#[derive(Clone, Default, serde::Serialize)]
pub struct TikTokChannelMeta {
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub viewer_count: Option<u64>,
    pub start_time: Option<String>,
    pub title: Option<String>,
    pub profile_pic: Option<String>,
    pub is_live: bool,
}

static TT_META: OnceLock<std::sync::Mutex<HashMap<String, TikTokChannelMeta>>> = OnceLock::new();

fn tt_meta_cache() -> &'static std::sync::Mutex<HashMap<String, TikTokChannelMeta>> {
    TT_META.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn store_meta(id_lc: &str, meta: TikTokChannelMeta) {
    if let Ok(mut m) = tt_meta_cache().lock() {
        m.insert(id_lc.to_string(), meta);
    }
}

fn set_live(id_lc: &str, live: bool) {
    if let Ok(mut m) = tt_meta_cache().lock() {
        if let Some(meta) = m.get_mut(id_lc) {
            meta.is_live = live;
        }
    }
}

fn update_viewers(id_lc: &str, m: &WebcastRoomUserSeqMessage) {
    if sample(&SEQ_SAMPLES, 10) {
        log::info!(
            "[TikTok][seq] viewer_count={} total_user={} popularity={}",
            m.viewer_count, m.total_user, m.popularity,
        );
    }
    // `viewer_count` is the CURRENT concurrent viewers. `total_user` is cumulative
    // (everyone who's ever joined) — a large, ever-growing number that was wrongly
    // shown as the viewer count.
    let viewers = m.viewer_count;
    if viewers <= 0 {
        return;
    }
    if let Ok(mut cache) = tt_meta_cache().lock() {
        if let Some(meta) = cache.get_mut(id_lc) {
            meta.viewer_count = Some(viewers as u64);
        }
    }
}

/// The cached live metadata for a handle, if resolved. Returned by the
/// `get_tiktok_channel_meta` command for the chat chrome.
pub fn channel_meta(identifier: &str) -> Option<TikTokChannelMeta> {
    let id_lc = clean_handle(identifier).to_lowercase();
    tt_meta_cache().lock().ok().and_then(|m| m.get(&id_lc).cloned())
}

/// Enrich the chrome from the profile page (avatar / name / numeric id) + the
/// webcast room/info (viewers / title). Best-effort: failures leave the seeded meta.
async fn refresh_meta(handle: &str, id_lc: &str) {
    let ua = None;
    let ttwid = match fetch_ttwid(Duration::from_secs(10), ua, None).await {
        Ok(t) => t,
        Err(_) => return,
    };
    let Ok(profile) = scrape_profile(handle, &ttwid, Duration::from_secs(10), ua, None, None).await
    else {
        log::info!("[TikTok][meta] scrape_profile failed for @{}", handle);
        return;
    };
    log::info!(
        "[TikTok][meta] nick='{}' uid={} avatar_large={} room_id='{}'",
        profile.nickname,
        profile.user_id,
        !profile.avatar_large.is_empty(),
        profile.room_id,
    );

    let mut meta = channel_meta(id_lc).unwrap_or_default();
    meta.is_live = true;
    meta.user_id = Some(profile.user_id.clone());
    if !profile.nickname.is_empty() {
        meta.username = Some(profile.nickname.clone());
    }
    let avatar = [
        &profile.avatar_large,
        &profile.avatar_medium,
        &profile.avatar_thumb,
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .cloned();
    if let Some(a) = avatar {
        meta.profile_pic = Some(a);
    }

    if !profile.room_id.is_empty() {
        if let Ok(info) = fetch_room_info(&profile.room_id, params()).await {
            if !info.title.is_empty() {
                meta.title = Some(info.title);
            }
            if info.viewers > 0 {
                meta.viewer_count = Some(info.viewers as u64);
            }
        }
    }
    store_meta(id_lc, meta);
}

// --- Small helpers ----------------------------------------------------------

fn image_url(img: &Option<Image>) -> Option<String> {
    img.as_ref().and_then(|i| i.url_list.first().cloned())
}

fn msg_id(raw: i64) -> String {
    if raw != 0 {
        raw.to_string()
    } else {
        format!("tt-{}", FALLBACK_SEQ.fetch_add(1, Ordering::Relaxed))
    }
}

/// A stable, readable name color derived from the user id (TikTok gives none).
fn color_for(user_id: &str) -> String {
    const PALETTE: [&str; 14] = [
        "#ff4f4f", "#ff8c42", "#ffd23f", "#9ee493", "#4fd1c5", "#4f9dff", "#7c6cff", "#c77dff",
        "#ff6fae", "#f25c54", "#43aa8b", "#577590", "#e07a5f", "#81b29a",
    ];
    let mut hash: u32 = 2166136261;
    for b in user_id.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    PALETTE[(hash as usize) % PALETTE.len()].to_string()
}

static LIKE_GATE: OnceLock<std::sync::Mutex<HashMap<String, Instant>>> = OnceLock::new();

/// True at most once per `LIKE_THROTTLE_MS` per (channel, user).
fn like_allowed(channel_key: &str, user_id: i64) -> bool {
    let key = format!("{}|{}", channel_key, user_id);
    let now = Instant::now();
    let Ok(mut gate) = LIKE_GATE
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
    else {
        return true;
    };
    if let Some(last) = gate.get(&key) {
        if now.duration_since(*last).as_millis() < LIKE_THROTTLE_MS {
            return false;
        }
    }
    gate.insert(key, now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_handles() {
        assert_eq!(clean_handle("@Repullze"), "Repullze");
        assert_eq!(clean_handle("tiktok:repullze"), "repullze");
        assert_eq!(clean_handle("  @user "), "user");
    }

    #[test]
    fn color_is_stable() {
        assert_eq!(color_for("12345"), color_for("12345"));
    }

    #[test]
    fn like_throttle_gates_second_call() {
        assert!(like_allowed("tiktok:t", 1));
        assert!(!like_allowed("tiktok:t", 1));
        assert!(like_allowed("tiktok:t", 2));
    }
}
