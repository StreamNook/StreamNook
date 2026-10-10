//! A Twitch channel's offline room: what the player area shows while the
//! channel is not streaming, the way twitch.tv shows an offline channel. Its
//! offline image, when it was last live and with what, the latest broadcast to
//! watch, and the next scheduled stream.
//!
//! One public GQL read answers all of it, with no account, so signed-out
//! viewers get the same room. A short cache collapses rapid reopens and
//! concurrent opens of one channel into one request; the live check that keeps
//! the room honest belongs to the watch session, not to this snapshot.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::models::stream::TwitchStream;
use crate::services::twitch_service::{live_row_from_gql, TwitchService};

/// Long enough to collapse a burst of opens, short enough that the live flag
/// it carries is never meaningfully stale.
const CACHE_TTL: Duration = Duration::from_secs(60);
/// Scheduled streams further out than this read as noise on an offline card.
const SCHEDULE_HORIZON_DAYS: i64 = 7;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RoomVod {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub length_seconds: u64,
    pub thumbnail_url: String,
    pub category: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RoomSegment {
    pub start_at: String,
    pub end_at: Option<String>,
    pub title: Option<String>,
    pub category: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OfflineRoom {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub offline_image_url: Option<String>,
    /// The channel is streaming after all (the card that opened the room was
    /// stale): the view starts the stream instead.
    pub live: Option<TwitchStream>,
    /// When the last broadcast ended, as Twitch reports it.
    pub last_live_at: Option<String>,
    pub last_title: Option<String>,
    pub last_category: Option<String>,
    pub latest_vod: Option<RoomVod>,
    pub next_stream: Option<RoomSegment>,
    /// peepoSad, waiting for them: 7TV's global one (7TV dresses its globals
    /// up for holidays, so this follows the season on its own), else FFZ's.
    pub sad_emote_url: String,
    /// peepoHappy, for the moment they go live, chosen the same way.
    pub happy_emote_url: String,
    /// A rare golden peepoSad (1 in `SHINY_ODDS` rooms).
    pub shiny: bool,
    /// How many people are in the chat right now, waiting with you. The
    /// session refreshes it while the room is open.
    pub chatters: Option<u64>,
}

/// One room in this many gets the golden peepoSad.
const SHINY_ODDS: u32 = 50;

fn roll_shiny() -> bool {
    rand::random::<u32>() % SHINY_ODDS == 0
}

/// FrankerFaceZ's most-used peepoSad and peepoHappy at their largest size, for
/// when 7TV's globals cannot be read.
const FALLBACK_SAD_EMOTE: &str = "https://cdn.frankerfacez.com/emote/230082/4";
const FALLBACK_HAPPY_EMOTE: &str = "https://cdn.frankerfacez.com/emote/228449/4";

/// The room's two moods, as 4x image URLs.
#[derive(Clone, Debug, PartialEq)]
struct Moods {
    sad: String,
    happy: String,
}

/// How long the global set's emotes are remembered when the room had to look
/// them up itself: 7TV changes its globals for holidays, not by the minute.
const MOODS_TTL: Duration = Duration::from_secs(3600);
static MOODS: Mutex<Option<(Instant, Moods)>> = Mutex::new(None);

/// peepoSad (waiting) and peepoHappy (they went live) as 7TV's global set
/// ships them right now, at 4x: 7TV dresses its globals up for holidays, so
/// they follow the season on their own. First from the shared cache chat loads
/// (no request); a room opened before chat loaded it (just after launch) asks
/// 7TV once and remembers the answer for an hour. Chat keeps 1x URLs (32 px),
/// which the room draws far larger, so addresses are rebuilt from the id.
async fn moods() -> Moods {
    let globals = crate::services::emote_service::seventv_globals_snapshot().await;
    let from_cache = |name: &str| {
        globals.iter().find(|e| e.name == name && !e.id.is_empty()).map(|e| seventv_4x(&e.id))
    };
    if let (Some(sad), Some(happy)) = (from_cache("peepoSad"), from_cache("peepoHappy")) {
        return Moods { sad, happy };
    }
    if let Some(m) = MOODS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|(at, _)| at.elapsed() < MOODS_TTL)
        .map(|(_, m)| m.clone())
    {
        return m;
    }
    let set: Option<Value> = async {
        crate::services::http::client()
            .get("https://7tv.io/v3/emote-sets/global")
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()
    }
    .await;
    let pick = |name: &str, fallback: &str| {
        set.as_ref()
            .and_then(|s| global_emote_id(s, name))
            .map(|id| seventv_4x(&id))
            .unwrap_or_else(|| fallback.to_string())
    };
    let m = Moods { sad: pick("peepoSad", FALLBACK_SAD_EMOTE), happy: pick("peepoHappy", FALLBACK_HAPPY_EMOTE) };
    // A failed look is not remembered: the next room asks again.
    if set.is_some() {
        *MOODS.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), m.clone()));
    }
    m
}

/// An emote's id by name in a 7TV emote-set document.
fn global_emote_id(set: &Value, name: &str) -> Option<String> {
    set.get("emotes")?
        .as_array()?
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some(name))
        .and_then(|e| e.get("id").and_then(|i| i.as_str()))
        .map(String::from)
}

fn seventv_4x(id: &str) -> String {
    format!("https://cdn.7tv.app/emote/{id}/4x.avif")
}

static CACHE: Mutex<Option<HashMap<String, (Instant, OfflineRoom)>>> = Mutex::new(None);
static IN_FLIGHT: Mutex<Option<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = Mutex::new(None);

fn cached(login: &str) -> Option<OfflineRoom> {
    let guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()?
        .get(login)
        .filter(|(at, _)| at.elapsed() < CACHE_TTL)
        .map(|(_, room)| room.clone())
}

fn remember(login: &str, room: &OfflineRoom) {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
    map.insert(login.to_string(), (Instant::now(), room.clone()));
}

fn flight(login: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut guard = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, lock| Arc::strong_count(lock) > 1);
    map.entry(login.to_string()).or_default().clone()
}

/// A Twitch login as Twitch issues them: 1-25 letters, digits or underscores.
pub fn valid_login(login: &str) -> bool {
    (1..=25).contains(&login.len()) && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The room for `login`, from the cache or one public read.
pub async fn open(login: &str) -> Result<OfflineRoom, String> {
    let login = login.trim().to_lowercase();
    if !valid_login(&login) {
        return Err(format!("not a Twitch channel name: {login}"));
    }
    if let Some(room) = cached(&login) {
        return Ok(room);
    }
    let lock = flight(&login);
    let _held = lock.lock().await;
    // A concurrent open of the same channel may have just filled it.
    if let Some(room) = cached(&login) {
        return Ok(room);
    }
    let user = TwitchService::get_offline_room(&login).await.map_err(|e| e.to_string())?;
    let mut room = parse(&user, Utc::now()).ok_or_else(|| format!("Twitch has no channel named {login}"))?;
    let (m, chatters) = tokio::join!(moods(), TwitchService::get_chatter_count(&login));
    room.sad_emote_url = m.sad;
    room.happy_emote_url = m.happy;
    room.chatters = chatters.ok();
    room.shiny = roll_shiny();
    remember(&login, &room);
    Ok(room)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(String::from)
}

fn game_name(v: &Value) -> Option<String> {
    v.get("game").filter(|g| !g.is_null()).and_then(|g| text(g, "displayName"))
}

/// Read the GQL `user` node; `None` when the channel does not exist.
fn parse(user: &Value, now: DateTime<Utc>) -> Option<OfflineRoom> {
    let user_id = text(user, "id")?;
    let login = text(user, "login")?;
    let last = user.get("lastBroadcast").filter(|b| !b.is_null());
    let latest_vod = user
        .pointer("/videos/edges/0/node")
        .filter(|n| !n.is_null())
        .and_then(|n| {
            Some(RoomVod {
                id: text(n, "id")?,
                title: text(n, "title").unwrap_or_default(),
                created_at: text(n, "createdAt").unwrap_or_default(),
                length_seconds: n.get("lengthSeconds").and_then(|l| l.as_u64()).unwrap_or(0),
                thumbnail_url: text(n, "previewThumbnailURL").unwrap_or_default(),
                category: game_name(n),
            })
        });
    let next_stream = user
        .pointer("/channel/schedule/nextSegment")
        .filter(|s| !s.is_null())
        .and_then(|s| pick_segment(s, now));
    Some(OfflineRoom {
        display_name: text(user, "displayName").unwrap_or_else(|| login.clone()),
        avatar_url: text(user, "profileImageURL"),
        offline_image_url: text(user, "offlineImageURL"),
        live: live_row_from_gql(user),
        last_live_at: last.and_then(|b| text(b, "startedAt")),
        last_title: last.and_then(|b| text(b, "title")),
        last_category: last.and_then(game_name),
        latest_vod,
        next_stream,
        sad_emote_url: FALLBACK_SAD_EMOTE.to_string(),
        happy_emote_url: FALLBACK_HAPPY_EMOTE.to_string(),
        shiny: false,
        chatters: None,
        user_id,
        login,
    })
}

/// The next scheduled stream worth showing: not cancelled, not already over,
/// and within the week.
fn pick_segment(seg: &Value, now: DateTime<Utc>) -> Option<RoomSegment> {
    if seg.get("isCancelled").and_then(|c| c.as_bool()).unwrap_or(false) {
        return None;
    }
    let start_at = text(seg, "startAt")?;
    let start = DateTime::parse_from_rfc3339(&start_at).ok()?.with_timezone(&Utc);
    let end_at = text(seg, "endAt");
    let end = end_at
        .as_deref()
        .and_then(|e| DateTime::parse_from_rfc3339(e).ok())
        .map(|e| e.with_timezone(&Utc))
        .unwrap_or(start);
    if end < now || start > now + chrono::Duration::days(SCHEDULE_HORIZON_DAYS) {
        return None;
    }
    Some(RoomSegment {
        start_at,
        end_at,
        title: text(seg, "title"),
        category: seg
            .pointer("/categories/0/name")
            .and_then(|c| c.as_str())
            .filter(|c| !c.is_empty())
            .map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-09T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn user() -> Value {
        json!({
            "id": "37402112", "login": "shroud", "displayName": "shroud",
            "profileImageURL": "https://static-cdn.jtvnw.net/a.png",
            "offlineImageURL": "https://static-cdn.jtvnw.net/offline.jpeg",
            "stream": null,
            "lastBroadcast": { "startedAt": "2026-10-09T05:44:57Z", "title": "AION", "game": { "displayName": "AION 2" } },
            "videos": { "edges": [ { "node": {
                "id": "2895277045", "title": "AION", "createdAt": "2026-10-08T17:08:40Z", "lengthSeconds": 45373,
                "previewThumbnailURL": "https://static-cdn.jtvnw.net/thumb.jpg", "game": { "displayName": "AION 2" }
            } } ] },
            "channel": { "schedule": { "nextSegment": {
                "startAt": "2026-10-09T18:00:00Z", "endAt": "2026-10-10T02:00:00Z", "title": "", "isCancelled": false, "categories": []
            } } }
        })
    }

    #[test]
    fn reads_an_offline_channel() {
        let room = parse(&user(), now()).unwrap();
        assert_eq!(room.user_id, "37402112");
        assert!(room.live.is_none());
        assert_eq!(room.offline_image_url.as_deref(), Some("https://static-cdn.jtvnw.net/offline.jpeg"));
        assert_eq!(room.last_category.as_deref(), Some("AION 2"));
        let vod = room.latest_vod.unwrap();
        assert_eq!((vod.id.as_str(), vod.length_seconds), ("2895277045", 45373));
        let next = room.next_stream.unwrap();
        assert_eq!(next.start_at, "2026-10-09T18:00:00Z");
        assert_eq!(next.title, None, "an empty title is no title");
    }

    #[test]
    fn a_live_channel_says_so() {
        let mut u = user();
        u["stream"] = json!({ "id": "9", "type": "live", "createdAt": "2026-10-09T11:00:00Z", "viewersCount": 5, "title": "hi", "game": null });
        let live = parse(&u, now()).unwrap().live.unwrap();
        assert_eq!((live.user_login.as_str(), live.viewer_count), ("shroud", 5));
    }

    #[test]
    fn a_missing_channel_is_none_and_bare_channels_still_parse() {
        assert!(parse(&Value::Null, now()).is_none());
        let bare = json!({ "id": "1", "login": "x", "stream": null, "lastBroadcast": null, "videos": { "edges": [] }, "channel": { "schedule": null } });
        let room = parse(&bare, now()).unwrap();
        assert_eq!(room.display_name, "x");
        assert!(room.latest_vod.is_none() && room.next_stream.is_none() && room.last_live_at.is_none());
    }

    #[test]
    fn only_a_near_uncancelled_upcoming_segment_shows() {
        let seg = |start: &str, end: &str, cancelled: bool| {
            json!({ "startAt": start, "endAt": end, "isCancelled": cancelled, "categories": [{ "name": "Art" }] })
        };
        let picked = pick_segment(&seg("2026-10-10T18:00:00Z", "2026-10-10T20:00:00Z", false), now()).unwrap();
        assert_eq!(picked.category.as_deref(), Some("Art"));
        assert!(pick_segment(&seg("2026-10-10T18:00:00Z", "2026-10-10T20:00:00Z", true), now()).is_none(), "cancelled");
        assert!(pick_segment(&seg("2026-10-08T18:00:00Z", "2026-10-08T20:00:00Z", false), now()).is_none(), "over");
        assert!(pick_segment(&seg("2026-10-30T18:00:00Z", "2026-10-30T20:00:00Z", false), now()).is_none(), "too far");
        assert!(pick_segment(&seg("2026-10-09T11:00:00Z", "2026-10-09T14:00:00Z", false), now()).is_some(), "in its slot now");
    }

    #[test]
    fn finds_the_moods_in_the_global_set() {
        let set = json!({ "emotes": [ { "id": "a", "name": "PETPET" }, { "id": "01GCS52CXG0004ZMF9GMF8X2AN", "name": "peepoSad" }, { "id": "h", "name": "peepoHappy" } ] });
        assert_eq!(global_emote_id(&set, "peepoSad").as_deref(), Some("01GCS52CXG0004ZMF9GMF8X2AN"));
        assert_eq!(global_emote_id(&set, "peepoHappy").as_deref(), Some("h"));
        assert!(global_emote_id(&json!({ "emotes": [] }), "peepoSad").is_none());
    }

    #[test]
    fn the_sad_emote_is_the_largest_size() {
        assert_eq!(seventv_4x("01GCS52CXG0004ZMF9GMF8X2AN"), "https://cdn.7tv.app/emote/01GCS52CXG0004ZMF9GMF8X2AN/4x.avif");
        assert!(FALLBACK_SAD_EMOTE.ends_with("/4") && FALLBACK_HAPPY_EMOTE.ends_with("/4"));
    }

    #[test]
    fn logins_are_validated() {
        assert!(valid_login("shroud") && valid_login("a_b_9"));
        assert!(!valid_login("") && !valid_login("bad name") && !valid_login("../x") && !valid_login(&"a".repeat(26)));
    }
}
