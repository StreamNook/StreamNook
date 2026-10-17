//! A channel's own moderator and VIP badge art, from its FrankerFaceZ room.
//!
//! A channel with an FFZ subscription can upload art for Twitch's moderator
//! and VIP badges. Twitch still decides who is a moderator or a VIP (the IRC
//! `badges` tag); FFZ only supplies the picture. The room payload
//! (`/v1/room/id/{twitch_id}`) carries it as `room.mod_urls` and
//! `room.vip_badge`, each `{"1": url, "2": url, "4": url}` for 18, 36 and
//! 72 px, the same sizes as Twitch's own badge `image_url_1x/2x/4x`. Either may
//! be `null`, and older rooms list only `"1"` although FFZ's CDN serves every
//! scale of the same art.
//!
//! Moderator art is a white glyph on a transparent canvas, drawn by FFZ's own
//! client over the moderator badge's green square (`#34AE0A`). That square is
//! painted in here (`badge_with_backdrop`), so every surface gets a finished
//! badge. VIP art arrives coloured and is used as is.
//!
//! The art is substituted into the CHANNEL badge set (`moderator/1`,
//! `vip/1`), which every surface already prefers over the global set for that
//! room: chat rows, the pinned card, the user card, the overlays and mobile
//! all draw it without knowing it exists. The set id, version and title stay
//! Twitch's, so badge order, the role pick, filters and highlights are
//! untouched. Shared-chat rows resolve against their source room's set, so a
//! guest channel's chatters wear the guest channel's art.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use futures::future::{BoxFuture, FutureExt, Shared};
use log::{debug, warn};
use once_cell::sync::Lazy;
use serde_json::Value;

/// Emitted with `{ "channel_id": Some(id) }` when a room's art changes, or
/// `None` when every room's might have (the setting flipped). Windows refetch
/// the named channel's badge set.
pub const CHANGED_EVENT: &str = "twitch-channel-badges-changed";

/// The moderator badge's green, behind FFZ's white moderator glyph.
const MOD_BACKDROP: &str = "#34AE0A";

/// Same lifetime as a channel's emote set: the emote fetch reads the same
/// room payload and refreshes this on every pass.
const ROOM_TTL: Duration = Duration::from_secs(5 * 60);
/// A room whose art could not be fully fetched, or whose read failed, is tried
/// again sooner.
const RETRY_TTL: Duration = Duration::from_secs(60);
/// How long a channel's badge fetch waits for a room it has never read. Art
/// that lands later is announced and every window reads the set again.
pub const FIRST_READ_WAIT: Duration = Duration::from_millis(300);
const MAX_ROOMS: usize = 64;
const PNG_MAX_BYTES: usize = 256 * 1024;

const FFZ_ROOM_BADGE_CDN: &str = "https://cdn.frankerfacez.com/room-badge/";

/// Twitch's badge scales, in `image_url_1x/2x/4x` order.
const SCALES: [&str; 3] = ["1", "2", "4"];

/// Finished image URLs for one role, at Twitch's three badge sizes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoleArt {
    pub x1: String,
    pub x2: String,
    pub x4: String,
}

/// What one room replaces. `None` keeps Twitch's art for that role.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomRoleArt {
    pub moderator: Option<RoleArt>,
    pub vip: Option<RoleArt>,
}

impl RoomRoleArt {
    fn is_empty(&self) -> bool {
        self.moderator.is_none() && self.vip.is_none()
    }

    /// (Twitch set id, Twitch's title, art) for each role this room replaces.
    fn roles(&self) -> impl Iterator<Item = (&'static str, &'static str, &RoleArt)> {
        [("moderator", "Moderator", self.moderator.as_ref()), ("vip", "VIP", self.vip.as_ref())]
            .into_iter()
            .filter_map(|(set, title, art)| art.map(|a| (set, title, a)))
    }

    /// The art for one Twitch set id, if this room replaces it.
    pub fn for_set(&self, set_id: &str) -> Option<&RoleArt> {
        match set_id {
            "moderator" => self.moderator.as_ref(),
            "vip" => self.vip.as_ref(),
            _ => None,
        }
    }
}

/// Image URLs to try for each scale, best first.
type ScaleCandidates = [Vec<String>; 3];

/// Where a room's art comes from, as listed in its payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RoomSources {
    moderator: Option<ScaleCandidates>,
    vip: Option<ScaleCandidates>,
}

struct Entry {
    sources: RoomSources,
    art: RoomRoleArt,
    fetched: Instant,
    complete: bool,
    /// The last read of the room itself failed; the art kept is the last known.
    failed: bool,
}

impl Entry {
    fn fresh(&self) -> bool {
        let ttl = if self.complete && !self.failed { ROOM_TTL } else { RETRY_TTL };
        self.fetched.elapsed() < ttl
    }
}

static ROOMS: Lazy<std::sync::RwLock<HashMap<String, Entry>>> = Lazy::new(Default::default);

type Flight = Shared<BoxFuture<'static, ()>>;

/// One read per room at a time. The badge fetch and the emote fetch both read
/// a room; the second joins the first instead of downloading the art again.
static FLIGHTS: Lazy<std::sync::Mutex<HashMap<String, Flight>>> = Lazy::new(Default::default);

fn running_read(channel_id: &str) -> Option<Flight> {
    FLIGHTS.lock().ok()?.get(channel_id).cloned()
}

/// Run `work` as this room's read, or join the read already running. The read
/// runs on its own task, so a caller that stops waiting never cancels it.
/// None outside a runtime.
fn join_or_start(channel_id: &str, work: impl FnOnce() -> BoxFuture<'static, ()>) -> Option<Flight> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    let mut flights = FLIGHTS.lock().ok()?;
    if let Some(running) = flights.get(channel_id) {
        return Some(running.clone());
    }
    let id = channel_id.to_string();
    let job = work();
    let flight = async move {
        job.await;
        if let Ok(mut flights) = FLIGHTS.lock() {
            flights.remove(&id);
        }
    }
    .boxed()
    .shared();
    flights.insert(channel_id.to_string(), flight.clone());
    drop(flights);
    handle.spawn(flight.clone());
    Some(flight)
}

/// Whether the viewer wants channels' own moderator and VIP art. On until the
/// settings handle says otherwise.
pub fn enabled() -> bool {
    crate::services::providers::setting(|s| s.chat_design.ffz_custom_role_badges).unwrap_or(true)
}

/// The cached art for a Twitch channel id, whatever its age. No network.
pub fn cached(channel_id: &str) -> Option<RoomRoleArt> {
    let rooms = ROOMS.read().ok()?;
    rooms.get(channel_id).map(|e| e.art.clone()).filter(|a| !a.is_empty())
}

/// The art to draw for `set_id` in `channel_id`, honouring the setting.
/// No network: `None` until the room has been read.
pub fn art_for(channel_id: &str, set_id: &str) -> Option<RoleArt> {
    if !enabled() {
        return None;
    }
    cached(channel_id)?.for_set(set_id).cloned()
}

/// Read the room for `channel_id` (a numeric Twitch id) when its entry is
/// stale, joining a read already running. Returns the read, which runs to the
/// end whether or not anyone waits on it, or None when the entry is fresh.
pub fn refresh(channel_id: &str) -> Option<Flight> {
    if !is_twitch_id(channel_id) {
        return None;
    }
    let fresh = ROOMS
        .read()
        .map(|rooms| rooms.get(channel_id).is_some_and(Entry::fresh))
        .unwrap_or(false);
    if fresh {
        return None;
    }
    let id = channel_id.to_string();
    join_or_start(channel_id, move || async move { read_room(&id).await }.boxed())
}

/// Whether this room has been read before, whatever the age or outcome.
pub fn has_entry(channel_id: &str) -> bool {
    ROOMS.read().map(|rooms| rooms.contains_key(channel_id)).unwrap_or(false)
}

async fn read_room(channel_id: &str) {
    let url = format!("https://api.frankerfacez.com/v1/room/id/{channel_id}");
    match crate::services::http::client().get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
            Ok(json) => record(channel_id, sources_from_room(&json)).await,
            Err(e) => {
                warn!("[FfzRoomBadges] room {channel_id} did not parse: {e}");
                record_failure(channel_id);
            }
        },
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            record(channel_id, RoomSources::default()).await
        }
        Ok(resp) => {
            debug!("[FfzRoomBadges] room {channel_id}: status {}", resp.status());
            record_failure(channel_id);
        }
        Err(e) => {
            debug!("[FfzRoomBadges] room {channel_id} request failed: {e}");
            record_failure(channel_id);
        }
    }
}

/// Hand over a room payload another fetch already holds (the channel emote
/// fetch reads the same endpoint), so the art refreshes with it. Returns at
/// once; any image fetches run in the background. Nothing is read while the
/// viewer has the art off.
pub fn note_room_payload(json: &Value) {
    if !enabled() {
        return;
    }
    let Some(channel_id) = room_twitch_id(json) else { return };
    let sources = sources_from_room(json);
    spawn_record(channel_id, sources);
}

/// The channel has no FFZ room (a definitive 404).
pub fn note_no_room(channel_id: &str) {
    if enabled() && is_twitch_id(channel_id) {
        spawn_record(channel_id.to_string(), RoomSources::default());
    }
}

/// Record a payload as the room's read. A read already running for the room
/// finishes first; recording the same sources after it reuses its art, so
/// nothing is downloaded twice.
fn spawn_record(channel_id: String, sources: RoomSources) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    handle.spawn(async move {
        if let Some(running) = running_read(&channel_id) {
            running.await;
        }
        let id = channel_id.clone();
        if let Some(read) = join_or_start(&channel_id, move || async move { record(&id, sources).await }.boxed()) {
            read.await;
        }
    });
}

fn make_room(rooms: &mut HashMap<String, Entry>, channel_id: &str) {
    if rooms.len() >= MAX_ROOMS && !rooms.contains_key(channel_id) {
        if let Some(oldest) = rooms
            .iter()
            .min_by_key(|(_, e)| e.fetched)
            .map(|(k, _)| k.clone())
        {
            rooms.remove(&oldest);
        }
    }
}

/// A read that failed: keep the art the room had, and wait `RETRY_TTL` before
/// reading it again rather than trying on every channel open.
fn record_failure(channel_id: &str) {
    let Ok(mut rooms) = ROOMS.write() else { return };
    if let Some(entry) = rooms.get_mut(channel_id) {
        entry.fetched = Instant::now();
        entry.failed = true;
        return;
    }
    make_room(&mut rooms, channel_id);
    rooms.insert(
        channel_id.to_string(),
        Entry {
            sources: RoomSources::default(),
            art: RoomRoleArt::default(),
            fetched: Instant::now(),
            complete: false,
            failed: true,
        },
    );
}

/// Store a room's sources, fetching the art only when the sources changed or
/// the last attempt was incomplete. Announces a change of art.
async fn record(channel_id: &str, sources: RoomSources) {
    let reuse = ROOMS
        .read()
        .ok()
        .and_then(|rooms| {
            rooms
                .get(channel_id)
                .filter(|e| e.complete && e.sources == sources)
                .map(|e| e.art.clone())
        });
    let (art, complete) = match reuse {
        Some(art) => (art, true),
        None => build_art(&sources).await,
    };
    let changed = {
        let Ok(mut rooms) = ROOMS.write() else { return };
        let previous = rooms.get(channel_id).map(|e| e.art.clone()).unwrap_or_default();
        make_room(&mut rooms, channel_id);
        rooms.insert(
            channel_id.to_string(),
            Entry { sources, art: art.clone(), fetched: Instant::now(), complete, failed: false },
        );
        previous != art
    };
    if changed && enabled() {
        announce(Some(channel_id));
    }
}

/// Tell every window to refetch a channel's badge set (`None`: all of them).
pub fn announce(channel_id: Option<&str>) {
    use tauri::Emitter;
    if let Some(app) = crate::services::providers::app_handle() {
        let _ = app.emit(CHANGED_EVENT, serde_json::json!({ "channel_id": channel_id }));
    }
}

async fn build_art(sources: &RoomSources) -> (RoomRoleArt, bool) {
    let mut complete = true;
    let mut art = RoomRoleArt::default();
    // Both roles, and every scale of each, download at once.
    let (moderator, vip) = tokio::join!(
        async {
            match &sources.moderator {
                Some(c) => Some(fetch_scales(c).await),
                None => None,
            }
        },
        async {
            match &sources.vip {
                Some(c) => Some(fetch_scales(c).await),
                None => None,
            }
        },
    );
    if let Some(fetched) = moderator {
        match fetched {
            Some(pngs) => {
                let painted: Option<Vec<String>> = pngs
                    .iter()
                    .map(|(_, png)| crate::services::badge_service::badge_with_backdrop(png, MOD_BACKDROP))
                    .collect();
                match painted {
                    Some(p) => art.moderator = Some(role_art(p)),
                    None => complete = false,
                }
            }
            None => complete = false,
        }
    }
    if let Some(fetched) = vip {
        match fetched {
            Some(pngs) => art.vip = Some(role_art(pngs.into_iter().map(|(url, _)| url).collect())),
            None => complete = false,
        }
    }
    (art, complete)
}

fn role_art(mut urls: Vec<String>) -> RoleArt {
    let x4 = urls.pop().unwrap_or_default();
    let x2 = urls.pop().unwrap_or_default();
    let x1 = urls.pop().unwrap_or_default();
    RoleArt { x1, x2, x4 }
}

/// For each scale, the first candidate that answers with a PNG: (url, bytes).
/// The three scales are fetched at once; within one scale the candidates are
/// tried in order. `None` when any scale has no working candidate.
async fn fetch_scales(candidates: &ScaleCandidates) -> Option<Vec<(String, Vec<u8>)>> {
    let found = futures::future::join_all(candidates.iter().map(|list| async move {
        for url in list {
            if let Some(png) = fetch_png(url).await {
                return Some((url.clone(), png));
            }
        }
        None
    }))
    .await;
    found.into_iter().collect()
}

async fn fetch_png(url: &str) -> Option<Vec<u8>> {
    let resp = crate::services::http::client().get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    (bytes.len() <= PNG_MAX_BYTES && bytes.starts_with(b"\x89PNG\r\n\x1a\n")).then(|| bytes.to_vec())
}

fn is_twitch_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

fn room_twitch_id(json: &Value) -> Option<String> {
    let id = json.get("room")?.get("twitch_id")?;
    let id = match id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    is_twitch_id(&id).then_some(id)
}

fn sources_from_room(json: &Value) -> RoomSources {
    let room = json.get("room");
    let role = |key: &str| room.and_then(|r| r.get(key)).and_then(scale_candidates);
    RoomSources { moderator: role("mod_urls"), vip: role("vip_badge") }
}

/// Candidate URLs for Twitch's 1x, 2x and 4x from one `{"1","2","4"}` object.
/// Each scale tries, in order: the URL FFZ lists for it; the same art at that
/// scale on FFZ's room-badge CDN (rooms that list only `"1"` still serve
/// every scale); then the nearest listed scale, larger first, so a missing
/// size never blanks the badge. `None` when nothing usable is listed.
fn scale_candidates(urls: &Value) -> Option<ScaleCandidates> {
    let listed: Vec<Option<String>> = SCALES
        .iter()
        .map(|s| {
            urls.get(*s)
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("https://"))
                .map(str::to_string)
        })
        .collect();
    if listed.iter().all(Option::is_none) {
        return None;
    }
    let template = listed.iter().flatten().find(|u| u.starts_with(FFZ_ROOM_BADGE_CDN)).cloned();
    let mut out: ScaleCandidates = Default::default();
    for (i, scale) in SCALES.iter().enumerate() {
        let list = &mut out[i];
        let mut push = |u: String| {
            if !list.contains(&u) {
                list.push(u);
            }
        };
        if let Some(u) = &listed[i] {
            push(u.clone());
        }
        if let Some(t) = &template {
            if let Some((base, _)) = t.rsplit_once('/') {
                push(format!("{base}/{scale}"));
            }
        }
        // Nearest listed scale: larger ones first, then smaller.
        let order: [usize; 2] = match i {
            0 => [1, 2],
            1 => [2, 0],
            _ => [1, 0],
        };
        for j in order {
            if let Some(u) = &listed[j] {
                push(u.clone());
            }
        }
    }
    Some(out)
}

/// Put a room's role art into a Twitch badge set response for that room.
/// Versions of `moderator` / `vip` the response carries get the new images;
/// when it carries none (the channel endpoint usually lists only its own
/// subscriber and bits sets), version `1` is added with Twitch's title, so
/// the channel set wins over the global one for that role.
pub trait RoleArtTarget {
    fn apply_role_art(&mut self, art: &RoomRoleArt);
}

macro_rules! role_art_target {
    ($resp:ty, $set:ident, $ver:ident) => {
        impl RoleArtTarget for $resp {
            fn apply_role_art(&mut self, art: &RoomRoleArt) {
                for (set_id, title, role) in art.roles() {
                    match self.data.iter_mut().find(|s| s.set_id == set_id) {
                        Some(set) => {
                            for v in &mut set.versions {
                                v.image_url_1x = role.x1.clone();
                                v.image_url_2x = role.x2.clone();
                                v.image_url_4x = role.x4.clone();
                            }
                        }
                        None => self.data.push($set {
                            set_id: set_id.to_string(),
                            versions: vec![$ver {
                                id: "1".to_string(),
                                image_url_1x: role.x1.clone(),
                                image_url_2x: role.x2.clone(),
                                image_url_4x: role.x4.clone(),
                                title: title.to_string(),
                                description: title.to_string(),
                                click_action: None,
                                click_url: None,
                            }],
                        }),
                    }
                }
            }
        }
    };
}

use crate::commands::badges::{HelixBadgeSet as CmdSet, HelixBadgeVersion as CmdVersion};
use crate::services::badge_service::{HelixBadgeSet as SvcSet, HelixBadgeVersion as SvcVersion};
role_art_target!(crate::commands::badges::HelixBadgesResponse, CmdSet, CmdVersion);
role_art_target!(crate::services::badge_service::HelixBadgesResponse, SvcSet, SvcVersion);

/// A channel's Twitch badge set with its FFZ role art applied, when the
/// viewer has it on (`enabled`) and the room has any.
pub fn apply_if_enabled<T: RoleArtTarget>(channel_id: &str, badges: &mut T, enabled: bool) {
    if !enabled {
        return;
    }
    if let Some(art) = cached(channel_id) {
        badges.apply_role_art(&art);
    }
}

#[cfg(test)]
pub(crate) fn seed(channel_id: &str, art: RoomRoleArt) {
    let mut rooms = ROOMS.write().unwrap();
    rooms.insert(
        channel_id.to_string(),
        Entry { sources: RoomSources::default(), art, fetched: Instant::now(), complete: true, failed: false },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::badge_service::{HelixBadgeSet, HelixBadgeVersion, HelixBadgesResponse};

    // Trimmed from GET https://api.frankerfacez.com/v1/room/lirik: custom
    // moderator and VIP art at all three scales.
    const LIRIK: &str = r#"{"room":{"_id":135157,"twitch_id":23161357,"id":"lirik","display_name":"LIRIK","set":135169,
        "moderator_badge":"https://cdn.frankerfacez.com/room-badge/mod/id/23161357/v/1ed78e6a/1",
        "vip_badge":{"1":"https://cdn.frankerfacez.com/room-badge/vip/id/23161357/v/e629b8b0/1","2":"https://cdn.frankerfacez.com/room-badge/vip/id/23161357/v/e629b8b0/2","4":"https://cdn.frankerfacez.com/room-badge/vip/id/23161357/v/e629b8b0/4"},
        "mod_urls":{"1":"https://cdn.frankerfacez.com/room-badge/mod/id/23161357/v/1ed78e6a/1","2":"https://cdn.frankerfacez.com/room-badge/mod/id/23161357/v/1ed78e6a/2","4":"https://cdn.frankerfacez.com/room-badge/mod/id/23161357/v/1ed78e6a/4"},
        "user_badges":{"2":["fossabot","analyticsbot"]},"user_badge_ids":{"2":[237719657,78295871]},"css":null}}"#;

    // Trimmed from /v1/room/xqc: moderator art listed at 1x only, no VIP art.
    const XQC: &str = r#"{"room":{"_id":166895,"twitch_id":71092938,"id":"xqc","display_name":"xQc","set":166907,
        "moderator_badge":"https://cdn.frankerfacez.com/room-badge/mod/id/71092938/v/98a226c4/1","vip_badge":null,
        "mod_urls":{"1":"https://cdn.frankerfacez.com/room-badge/mod/id/71092938/v/98a226c4/1"},
        "user_badges":{},"user_badge_ids":{},"css":null}}"#;

    // Trimmed from /v1/room/forsen: an FFZ room with no custom role art.
    const FORSEN: &str = r#"{"room":{"_id":1,"twitch_id":22484632,"id":"forsen","display_name":"forsen",
        "moderator_badge":null,"vip_badge":null,"mod_urls":null,"user_badges":{},"user_badge_ids":{},"css":null}}"#;

    const TWITCH_MOD: &str = "https://static-cdn.jtvnw.net/badges/v1/3267646d-33f0-4b17-b3df-f923a41db1d0/3";
    const TWITCH_VIP: &str = "https://static-cdn.jtvnw.net/badges/v1/b817aba4-fad8-49e2-b88a-7cc744dfa6ec/3";

    fn version(url: &str, title: &str) -> HelixBadgeVersion {
        HelixBadgeVersion {
            id: "1".into(),
            image_url_1x: url.into(),
            image_url_2x: url.into(),
            image_url_4x: url.into(),
            title: title.into(),
            description: title.into(),
            click_action: None,
            click_url: None,
        }
    }

    fn channel_set() -> HelixBadgesResponse {
        HelixBadgesResponse {
            data: vec![HelixBadgeSet { set_id: "subscriber".into(), versions: vec![version("sub.png", "Subscriber")] }],
        }
    }

    fn global_set() -> HelixBadgesResponse {
        HelixBadgesResponse {
            data: vec![
                HelixBadgeSet { set_id: "moderator".into(), versions: vec![version(TWITCH_MOD, "Moderator")] },
                HelixBadgeSet { set_id: "vip".into(), versions: vec![version(TWITCH_VIP, "VIP")] },
            ],
        }
    }

    /// What a surface draws for `set_id`: the channel set first, then global,
    /// as `twitchBadges.parseBadges` and `resolve_badge_ids` both look it up.
    fn drawn(
        channel: &HelixBadgesResponse,
        global: &HelixBadgesResponse,
        set_id: &str,
    ) -> (String, HelixBadgeVersion) {
        let find = |r: &HelixBadgesResponse| {
            r.data
                .iter()
                .find(|s| s.set_id == set_id)
                .and_then(|s| s.versions.iter().find(|v| v.id == "1").map(|v| (s.set_id.clone(), v.clone())))
        };
        find(channel).or_else(|| find(global)).expect("badge resolves")
    }

    /// The art `build_art` would make, with stand-ins for the painted data.
    fn art_from_room(json: &str) -> RoomRoleArt {
        let sources = sources_from_room(&serde_json::from_str(json).unwrap());
        let first = |c: &ScaleCandidates| RoleArt { x1: c[0][0].clone(), x2: c[1][0].clone(), x4: c[2][0].clone() };
        RoomRoleArt { moderator: sources.moderator.as_ref().map(first), vip: sources.vip.as_ref().map(first) }
    }

    #[test]
    fn a_moderator_wears_the_channels_art_and_stays_a_moderator() {
        let art = art_from_room(LIRIK);
        let mut channel = channel_set();
        channel.apply_role_art(&art);
        let (set_id, v) = drawn(&channel, &global_set(), "moderator");
        assert_eq!(set_id, "moderator");
        assert_eq!(v.id, "1");
        assert_eq!(v.title, "Moderator");
        assert_eq!(v.image_url_4x, "https://cdn.frankerfacez.com/room-badge/mod/id/23161357/v/1ed78e6a/4");
        // The channel's own sets are left alone.
        assert_eq!(drawn(&channel, &global_set(), "subscriber").1.image_url_4x, "sub.png");
    }

    #[test]
    fn a_vip_wears_the_channels_vip_art() {
        let art = art_from_room(LIRIK);
        let mut channel = channel_set();
        channel.apply_role_art(&art);
        let (set_id, v) = drawn(&channel, &global_set(), "vip");
        assert_eq!(set_id, "vip");
        assert_eq!(v.title, "VIP");
        assert_eq!(v.image_url_1x, "https://cdn.frankerfacez.com/room-badge/vip/id/23161357/v/e629b8b0/1");
        assert_eq!(v.image_url_2x, "https://cdn.frankerfacez.com/room-badge/vip/id/23161357/v/e629b8b0/2");
        assert_eq!(v.image_url_4x, "https://cdn.frankerfacez.com/room-badge/vip/id/23161357/v/e629b8b0/4");
    }

    #[test]
    fn a_channel_without_custom_art_keeps_twitchs() {
        let art = art_from_room(FORSEN);
        assert!(art.is_empty());
        let mut channel = channel_set();
        channel.apply_role_art(&art);
        assert_eq!(drawn(&channel, &global_set(), "moderator").1.image_url_4x, TWITCH_MOD);
        assert_eq!(drawn(&channel, &global_set(), "vip").1.image_url_4x, TWITCH_VIP);

        // A room with moderator art only leaves VIP to Twitch.
        let xqc = art_from_room(XQC);
        let mut channel = channel_set();
        channel.apply_role_art(&xqc);
        assert_eq!(drawn(&channel, &global_set(), "vip").1.image_url_4x, TWITCH_VIP);
        assert_ne!(drawn(&channel, &global_set(), "moderator").1.image_url_4x, TWITCH_MOD);
    }

    #[test]
    fn the_setting_off_keeps_twitchs_art() {
        seed("23161357", art_from_room(LIRIK));
        let mut off = channel_set();
        apply_if_enabled("23161357", &mut off, false);
        assert_eq!(drawn(&off, &global_set(), "moderator").1.image_url_4x, TWITCH_MOD);
        assert_eq!(drawn(&off, &global_set(), "vip").1.image_url_4x, TWITCH_VIP);

        let mut on = channel_set();
        apply_if_enabled("23161357", &mut on, true);
        assert!(drawn(&on, &global_set(), "moderator").1.image_url_4x.starts_with(FFZ_ROOM_BADGE_CDN));
    }

    #[test]
    fn existing_role_versions_get_the_art_in_place() {
        let art = art_from_room(LIRIK);
        let mut channel = global_set();
        channel.apply_role_art(&art);
        assert_eq!(channel.data.len(), 2, "no duplicate set is added");
        assert!(channel.data[0].versions[0].image_url_1x.ends_with("/1ed78e6a/1"));
        assert_eq!(channel.data[0].versions[0].title, "Moderator");
    }

    #[test]
    fn sizes_map_to_twitchs_1x_2x_4x() {
        let json: Value = serde_json::from_str(LIRIK).unwrap();
        let mods = sources_from_room(&json).moderator.unwrap();
        // 18, 36 and 72 px, the same sizes as Twitch's image_url_1x/2x/4x.
        assert!(mods[0][0].ends_with("/1"));
        assert!(mods[1][0].ends_with("/2"));
        assert!(mods[2][0].ends_with("/4"));
    }

    #[test]
    fn a_room_listing_only_1x_asks_the_cdn_for_the_larger_scales_first() {
        let json: Value = serde_json::from_str(XQC).unwrap();
        let mods = sources_from_room(&json).moderator.unwrap();
        let base = "https://cdn.frankerfacez.com/room-badge/mod/id/71092938/v/98a226c4";
        assert_eq!(mods[0], vec![format!("{base}/1")]);
        assert_eq!(mods[1], vec![format!("{base}/2"), format!("{base}/1")]);
        assert_eq!(mods[2], vec![format!("{base}/4"), format!("{base}/1")]);
    }

    #[test]
    fn a_missing_scale_falls_back_to_the_nearest_listed_larger_first() {
        let urls = serde_json::json!({ "1": "https://example.com/a1", "4": "https://example.com/a4" });
        let c = scale_candidates(&urls).unwrap();
        assert_eq!(c[0], vec!["https://example.com/a1", "https://example.com/a4"]);
        assert_eq!(c[1], vec!["https://example.com/a4", "https://example.com/a1"]);
        assert_eq!(c[2], vec!["https://example.com/a4", "https://example.com/a1"]);
        assert!(scale_candidates(&Value::Null).is_none());
        assert!(scale_candidates(&serde_json::json!({})).is_none());
    }

    #[test]
    fn the_room_is_keyed_by_its_twitch_id() {
        assert_eq!(room_twitch_id(&serde_json::from_str(LIRIK).unwrap()).as_deref(), Some("23161357"));
        assert_eq!(room_twitch_id(&serde_json::json!({ "room": { "twitch_id": null } })), None);
    }

    #[test]
    fn a_failed_read_keeps_the_art_and_backs_off() {
        // Never read: nothing known, so a badge fetch would wait briefly.
        assert!(!has_entry("40000001"));
        record_failure("40000001");
        assert!(has_entry("40000001"), "a failed read is remembered");
        {
            let rooms = ROOMS.read().unwrap();
            let e = rooms.get("40000001").unwrap();
            assert!(e.failed && e.fresh(), "no second read before RETRY_TTL");
            assert!(e.art.is_empty());
        }
        assert!(refresh("40000001").is_none(), "a fresh failure is not read again");

        // A room with art keeps drawing it through a failed refresh.
        seed("40000002", art_from_room(LIRIK));
        record_failure("40000002");
        assert!(cached("40000002").is_some_and(|a| a.moderator.is_some() && a.vip.is_some()));
        let rooms = ROOMS.read().unwrap();
        let e = rooms.get("40000002").unwrap();
        assert!(e.failed && e.complete);
    }

    #[tokio::test]
    async fn concurrent_reads_of_one_room_share_one_flight() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let runs = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let counted = runs.clone();
        let first = join_or_start("40000003", move || {
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                let _ = rx.await;
            }
            .boxed()
        })
        .expect("inside a runtime");
        let counted = runs.clone();
        let second = join_or_start("40000003", move || {
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
            }
            .boxed()
        })
        .expect("inside a runtime");
        assert!(running_read("40000003").is_some());
        let _ = tx.send(());
        first.await;
        second.await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the second caller joined the first read");
        // The finished read leaves the registry, so the next one starts fresh.
        tokio::task::yield_now().await;
        assert!(running_read("40000003").is_none());
    }

    #[test]
    fn moderator_glyphs_are_painted_on_the_moderator_green() {
        // An empty (fully transparent) 18 px glyph shows the square alone.
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 18, 18);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0u8; 18 * 18 * 4]).unwrap();
            writer.finish().unwrap();
        }
        let uri = crate::services::badge_service::badge_with_backdrop(&png, MOD_BACKDROP).unwrap();
        // A PNG, which native shells decode (they cannot read SVG).
        let body = uri.strip_prefix("data:image/png;base64,").expect("a PNG data URI");
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(body).unwrap();
        let (w, h, px) = crate::services::badge_service::decode_rgba(&bytes).unwrap();
        assert_eq!((w, h), (18, 18));
        let centre = (9 * 18 + 9) * 4;
        assert_eq!(&px[centre..centre + 4], &[0x34, 0xAE, 0x0A, 255]);
    }
}
