//! Shared Chat rows wear the emotes and badges of the chat they were sent in.
//!
//! In a Twitch Shared Chat, rows from a partner channel arrive in the home room
//! with `source-room-id` naming the partner. Their senders type the partner's
//! 7TV, BetterTTV and FrankerFaceZ emotes, so such a row parses with that room's
//! set (which carries every provider's globals) and never with the home room's,
//! the way Chatterino draws them. Their subscriber, bits and other channel badges
//! (`source-badges`) are that room's too, so the row carries the room's own badge
//! art; global badges are untouched. The first row from a room not seen yet
//! starts one fetch of its emote set (through the shared emote service, so a
//! partner also open in another chat reuses its cached set) and its channel
//! badges; rows parsed before they land use the home table and are re-stamped
//! once they do, so nothing waits.
//!
//! A set whose 7TV channel lookup failed is partial: it is used only while the
//! room has nothing better, rows parsed with it stay queued, and it is fetched
//! again in the background (`table_repair::DELAYS`) while a chat shows the
//! room, until a whole set lands and those rows are re-stamped with it.
//!
//! A room's set lives while a chat that showed it is open, and at most
//! `MAX_ROOMS` rooms are held.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::services::emote_service::EmoteSet;
use crate::services::irc_service::EmoteLookup;
use crate::services::providers::table_repair::{self, Retrying};

/// The most partner rooms held at once (a session is two to six channels).
const MAX_ROOMS: usize = 16;
/// The most rows per room waiting for its set to land.
const MAX_PENDING: usize = 300;
/// How long after a failed fetch the next row may ask again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// One background retry loop per room while its set is partial.
static RETRYING: Retrying = Retrying::new();

#[derive(Default)]
struct Room {
    lookup: Option<Arc<EmoteLookup>>,
    /// Whether `lookup` came from a whole answer (the 7TV channel lookup answered).
    whole: bool,
    /// The partial table a whole one replaced, which the rows queued under it
    /// were parsed with.
    replaced: Option<Arc<EmoteLookup>>,
    /// Home channels whose chat showed this room's rows.
    homes: HashSet<String>,
    fetching: bool,
    failed_at: Option<Instant>,
    /// `(home channel, raw line)` parsed before the set landed.
    pending: VecDeque<(String, String)>,
    used: Option<Instant>,
}

fn rooms() -> &'static Mutex<HashMap<String, Room>> {
    static ROOMS: OnceLock<Mutex<HashMap<String, Room>>> = OnceLock::new();
    ROOMS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The room id a row was sent in when it is not the room it arrived in.
pub(crate) fn partner_room<'a>(source_room_id: Option<&'a str>, room_id: Option<&str>) -> Option<&'a str> {
    let source = source_room_id.filter(|s| !s.is_empty())?;
    (room_id != Some(source)).then_some(source)
}

/// The table a row from `room_id`, arriving in `home`, parses with. None until
/// that room's set has landed: the caller parses with the home table, and the
/// row (`raw`) is queued for a re-stamp. The first such row starts the fetch.
/// While the room's set is partial the row parses with it and is queued too,
/// for the whole set its retry loop is fetching.
pub(crate) fn lookup_for(home: &str, room_id: &str, raw: &str) -> Option<Arc<EmoteLookup>> {
    let start = {
        let Ok(mut map) = rooms().lock() else { return None };
        if !map.contains_key(room_id) {
            evict_for_new_room(&mut map);
        }
        let room = map.entry(room_id.to_string()).or_default();
        room.homes.insert(home.to_string());
        room.used = Some(Instant::now());
        if room.whole {
            if let Some(lookup) = &room.lookup {
                return Some(lookup.clone());
            }
        }
        if room.pending.len() < MAX_PENDING && !room.pending.iter().any(|(h, r)| h == home && r == raw) {
            room.pending.push_back((home.to_string(), raw.to_string()));
        }
        if let Some(lookup) = &room.lookup {
            return Some(lookup.clone());
        }
        let may_retry = room.failed_at.map_or(true, |at| at.elapsed() >= RETRY_AFTER);
        // A running retry loop fetches it on its own schedule.
        let start = !room.fetching && may_retry && !RETRYING.running(room_id);
        if start {
            room.fetching = true;
        }
        start
    };
    if start && tokio::runtime::Handle::try_current().is_ok() {
        let room_id = room_id.to_string();
        tokio::spawn(async move { fetch(room_id).await });
    }
    None
}

/// Makes room for one more room: drops the least recently used room that is
/// not mid-fetch.
fn evict_for_new_room(map: &mut HashMap<String, Room>) {
    while map.len() >= MAX_ROOMS {
        let oldest = map
            .iter()
            .filter(|(_, r)| !r.fetching)
            .min_by_key(|(_, r)| r.used)
            .map(|(id, _)| id.clone());
        match oldest {
            Some(id) => {
                map.remove(&id);
            }
            None => break,
        }
    }
}

async fn fetch(room_id: String) {
    if !fetch_once(&room_id).await {
        spawn_retries(room_id);
    }
}

/// One fetch of the room's set and channel badges, put in place, with the rows
/// waiting for them re-stamped. True when the set was whole.
async fn fetch_once(room_id: &str) -> bool {
    let (set, badges_ok) = tokio::join!(fetch_set(room_id), fetch_badges(room_id));
    match &set {
        None => log::warn!("[SharedChat] emote set for room {room_id} did not load; partner rows keep the home set"),
        Some((_, false)) => {
            log::warn!("[SharedChat] 7TV set for room {room_id} did not resolve; partner rows use what loaded until it does")
        }
        Some((_, true)) => {}
    }
    let whole = set.as_ref().is_some_and(|(_, whole)| *whole);
    let Some((lookup, pending)) = install(room_id, set.as_ref().map(|(s, w)| (s, *w)), badges_ok) else {
        return whole;
    };
    if !pending.is_empty() {
        crate::services::irc_service::IrcService::restamp_partner_rows(room_id, lookup.as_deref(), pending).await;
    }
    whole
}

/// While a chat shows the room, fetches its partial (or failed) set again
/// until a whole one lands.
fn spawn_retries(room_id: String) {
    let open = room_id.clone();
    let key = room_id.clone();
    table_repair::spawn(&RETRYING, key, move || room_open(&open), move || {
        let room_id = room_id.clone();
        async move {
            if !begin_retry(&room_id) {
                return false;
            }
            invalidate(&room_id).await;
            fetch_once(&room_id).await
        }
    });
}

/// Whether a chat still shows the room.
fn room_open(room_id: &str) -> bool {
    rooms().lock().map(|m| m.get(room_id).is_some_and(|r| !r.homes.is_empty())).unwrap_or(false)
}

/// Marks a retry fetch as running; false when the room is gone or a fetch
/// already runs.
fn begin_retry(room_id: &str) -> bool {
    let Ok(mut map) = rooms().lock() else { return false };
    let Some(room) = map.get_mut(room_id) else { return false };
    if room.fetching {
        return false;
    }
    room.fetching = true;
    true
}

/// Drops the emote service's cached answer for the room, so the retry fetches
/// rather than reading back the partial answer it cached a moment ago.
async fn invalidate(room_id: &str) {
    use tauri::Manager;
    let Some(app) = crate::services::providers::app_handle() else { return };
    let service = app.state::<crate::models::settings::AppState>().emote_service.read().await.clone();
    service.invalidate_channel(room_id).await;
}

/// Reads the room's channel badges into the art every row view draws from.
async fn fetch_badges(room_id: &str) -> bool {
    match crate::commands::badges::fetch_channel_badges(room_id.to_string()).await {
        Ok(_) => true,
        Err(e) => {
            log::debug!("[SharedChat] badges for room {room_id}: {e}");
            false
        }
    }
}

/// The room's set and whether it is whole: false when its 7TV channel lookup
/// failed (the service then hands back what it held, or the globals).
async fn fetch_set(room_id: &str) -> Option<(EmoteSet, bool)> {
    use tauri::Manager;
    let app = crate::services::providers::app_handle()?;
    let service = app.state::<crate::models::settings::AppState>().emote_service.read().await.clone();
    // By id alone: every provider's channel set is keyed by the Twitch id, and the
    // service's cache is too, so a partner open in another chat is not fetched again.
    match service.fetch_channel_emotes_checked(None, Some(room_id.to_string()), None, None).await {
        Ok((set, seven_tv_ok)) => Some((set, seven_tv_ok)),
        Err(e) => {
            log::debug!("[SharedChat] room {room_id}: {e}");
            None
        }
    }
}

/// Puts what a room fetch brought in place (its emote set with whether it is
/// whole, when it loaded, and whether its channel badges did); answers the
/// room's lookup and, when anything landed, the rows to re-stamp. A whole set
/// always replaces what is held; a partial one only while the room has nothing
/// better (`table_repair::decide`). Rows queued under a partial set stay queued
/// for the whole one. A failed set fetch may be retried later.
pub(crate) fn install(
    room_id: &str,
    set: Option<(&EmoteSet, bool)>,
    badges_landed: bool,
) -> Option<(Option<Arc<EmoteLookup>>, Vec<(String, String)>)> {
    let mut map = rooms().lock().ok()?;
    let room = map.get_mut(room_id)?;
    room.fetching = false;
    match set {
        Some((set, whole)) => {
            room.failed_at = None;
            let lookup = EmoteLookup::build(set);
            let standing = if room.whole { table_repair::Standing::Whole } else { table_repair::Standing::Partial };
            let held = room.lookup.as_ref().map(|l| (standing, l.len()));
            if table_repair::decide(held, whole, lookup.len()) != table_repair::Install::Keep {
                room.replaced = if whole && !room.whole { room.lookup.take() } else { None };
                room.lookup = Some(lookup);
                room.whole = whole;
            }
        }
        None => room.failed_at = Some(Instant::now()),
    }
    let pending = if set.is_none() && !badges_landed {
        Vec::new()
    } else if room.whole {
        room.pending.drain(..).collect()
    } else {
        room.pending.iter().cloned().collect()
    };
    Some((room.lookup.clone(), pending))
}

/// Gives a partner row's badges its room's own art (subscriber, bits and the
/// channel's custom badges, its FFZ moderator and VIP art). Global badges keep
/// no carried art: every renderer draws those from the global set.
pub(crate) fn wear_room_badges(room_id: &str, badges: &mut [crate::models::chat_layout::Badge]) {
    for badge in badges.iter_mut() {
        if badge.image_url_1x.is_some() {
            continue;
        }
        if let Some((url, title)) = crate::services::chat_row_view::room_badge_art(room_id, &badge.name, &badge.version) {
            badge.image_url_1x = Some(url.clone());
            badge.image_url_2x = Some(url.clone());
            badge.image_url_4x = Some(url);
            badge.title.get_or_insert(title);
        }
    }
}

/// Whether a partner row (`raw`) has a badge its room's own set now draws.
pub(crate) fn badges_change(raw: &str, room_id: &str) -> bool {
    let tags = raw.strip_prefix('@').and_then(|r| r.split_once(' ')).map(|(t, _)| t).unwrap_or("");
    let badges = tags.split(';').find_map(|t| t.strip_prefix("source-badges=")).unwrap_or("");
    badges.split(',').filter(|b| !b.is_empty()).any(|b| {
        let (set, version) = b.split_once('/').unwrap_or((b, ""));
        crate::services::chat_row_view::room_badge_art(room_id, set, version).is_some()
    })
}

/// The home chat `home` was left: rooms no other open chat shows are dropped.
pub(crate) fn release_home(home: &str) {
    if let Ok(mut map) = rooms().lock() {
        map.retain(|_, room| {
            room.homes.remove(home);
            room.pending.retain(|(h, _)| h != home);
            !room.homes.is_empty() || room.fetching
        });
    }
}

/// Every chat was left.
pub(crate) fn release_all() {
    if let Ok(mut map) = rooms().lock() {
        map.clear();
    }
}

/// Whether a row with these words parses differently with the partner's table
/// than with the table it was first parsed with: the home table, or the
/// partial partner table this one replaced.
pub(crate) fn words_change(text: &str, partner: &EmoteLookup, home: Option<&EmoteLookup>) -> bool {
    let replaced = replaced_by(partner);
    text.split_whitespace().any(|word| {
        let now = partner.get(word).map(|e| e.id.as_str());
        let before = home.and_then(|h| h.get(word)).map(|e| e.id.as_str());
        let before_partial = replaced.as_deref().map(|r| r.get(word).map(|e| e.id.as_str()));
        now != before || before_partial.is_some_and(|b| b != now)
    })
}

/// The partial table `partner` replaced in its room, if it did.
fn replaced_by(partner: &EmoteLookup) -> Option<Arc<EmoteLookup>> {
    let map = rooms().lock().ok()?;
    map.values()
        .find(|room| room.lookup.as_ref().is_some_and(|l| std::ptr::eq(Arc::as_ptr(l), partner)))
        .and_then(|room| room.replaced.clone())
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    release_all();
}

#[cfg(test)]
pub(crate) fn pending_for_test(room_id: &str) -> Option<(bool, usize)> {
    let map = rooms().lock().ok()?;
    map.get(room_id).map(|r| (r.fetching, r.pending.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::emote_service::{Emote, EmoteProvider};
    use crate::services::irc_service::IrcService;
    use crate::models::chat_layout::MessageSegment;

    /// Tests share one process-wide table.
    static LOCK: Mutex<()> = Mutex::new(());

    fn emote(id: &str, name: &str) -> Emote {
        Emote {
            id: id.to_string(),
            name: name.to_string(),
            url: format!("https://cdn.7tv.app/emote/{id}/1x.avif"),
            provider: EmoteProvider::SevenTV,
            is_zero_width: Some(false),
            local_url: None,
            emote_type: None,
            owner_id: None,
            owner_name: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
            original_name: None,
            owner_ref: None,
        }
    }

    fn set(emotes: &[(&str, &str)]) -> EmoteSet {
        let mut s = EmoteSet::new();
        s.seven_tv = emotes.iter().map(|(id, name)| emote(id, name)).collect();
        s
    }

    fn line(id: &str, source: Option<&str>, body: &str) -> String {
        let source = source.map(|s| format!("source-room-id={s};")).unwrap_or_default();
        format!("@display-name=Dee;emotes=;historical=1;id={id};room-id=12;{source}tmi-sent-ts=1700000000000;user-id=48 :dee!dee@dee.tmi.twitch.tv PRIVMSG #sharedhome :{body}")
    }

    fn emote_names(segments: &[MessageSegment]) -> Vec<String> {
        segments
            .iter()
            .filter_map(|s| match s {
                MessageSegment::Emote { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_a_row_from_another_room_is_a_partner_row() {
        assert_eq!(partner_room(Some("99"), Some("12")), Some("99"));
        assert_eq!(partner_room(Some("12"), Some("12")), None);
        assert_eq!(partner_room(None, Some("12")), None);
        assert_eq!(partner_room(Some(""), Some("12")), None);
    }

    #[tokio::test]
    async fn a_partner_row_parses_with_its_rooms_set_and_not_the_homes() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        IrcService::install_parse_lookup_for_test("sharedhome", &set(&[("h1", "homeEmote"), ("g1", "globalEmote")]));
        // The partner's set holds its own emotes and the globals, as a fetched set does.
        let partner = set(&[("p1", "partnerEmote"), ("g1", "globalEmote")]);

        // Before the partner's set lands: home table, row queued, fetch asked.
        let rows = IrcService::parse_historical_messages(vec![line("a", Some("990"), "partnerEmote homeEmote")]).await;
        assert_eq!(emote_names(&rows[0].segments), vec!["homeEmote".to_string()]);
        assert_eq!(pending_for_test("990").map(|(_, waiting)| waiting), Some(1));

        // It lands: the queued row is handed back, and new rows parse with it.
        let (_, waiting) = install("990", Some((&partner, true)), false).expect("room known");
        assert_eq!(waiting.len(), 1);
        let rows = IrcService::parse_historical_messages(vec![line(
            "b",
            Some("990"),
            "partnerEmote homeEmote globalEmote",
        )])
        .await;
        assert_eq!(emote_names(&rows[0].segments), vec!["partnerEmote".to_string(), "globalEmote".to_string()]);

        // A home row still parses with the home table.
        let rows = IrcService::parse_historical_messages(vec![line("c", None, "partnerEmote homeEmote")]).await;
        assert_eq!(emote_names(&rows[0].segments), vec!["homeEmote".to_string()]);
        reset_for_test();
    }

    #[test]
    fn a_partner_row_wears_its_rooms_badges_and_globals_carry_nothing() {
        use crate::models::chat_layout::Badge;
        crate::services::chat_row_view::note_twitch_badges(
            Some("991"),
            [("subscriber", "12", "https://static-cdn.jtvnw.net/badges/v1/partner-sub-12/2", "1-Year Subscriber")],
        );
        let badge = |name: &str, version: &str| Badge {
            name: name.into(),
            version: version.into(),
            image_url_1x: None,
            image_url_2x: None,
            image_url_4x: None,
            title: None,
            description: None,
        };
        let mut badges = vec![badge("subscriber", "12"), badge("premium", "1")];
        wear_room_badges("991", &mut badges);
        assert_eq!(badges[0].image_url_2x.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/partner-sub-12/2"));
        assert_eq!(badges[0].title.as_deref(), Some("1-Year Subscriber"));
        assert!(badges[1].image_url_1x.is_none(), "a global badge keeps no carried art");

        let raw = "@badges=premium/1;source-badges=subscriber/12,premium/1;room-id=12;source-room-id=991 :a!a@a.tmi.twitch.tv PRIVMSG #h :hi";
        assert!(badges_change(raw, "991"));
        assert!(!badges_change(raw, "no-such-room"));
    }

    #[test]
    fn the_body_is_read_with_or_without_its_colon() {
        use crate::services::irc_service::irc_trailing_text;
        assert_eq!(irc_trailing_text("@id=1 :a!a@a.tmi.twitch.tv PRIVMSG #c :two words"), "two words");
        assert_eq!(irc_trailing_text("@id=1 :a!a@a.tmi.twitch.tv PRIVMSG #c partnerEmote"), "partnerEmote");
        assert_eq!(irc_trailing_text(":a!a@a.tmi.twitch.tv PRIVMSG #c :x"), "x");
        assert_eq!(irc_trailing_text("@id=1 :tmi.twitch.tv USERNOTICE #c"), "");
    }

    #[test]
    fn only_rows_whose_emotes_change_are_patched() {
        let home = EmoteLookup::build(&set(&[("h1", "homeEmote"), ("g1", "globalEmote")]));
        let partner = EmoteLookup::build(&set(&[("p1", "partnerEmote"), ("g1", "globalEmote")]));
        assert!(words_change("look partnerEmote", &partner, Some(&home)));
        assert!(words_change("homeEmote was wrong", &partner, Some(&home)));
        assert!(!words_change("globalEmote only", &partner, Some(&home)));
        assert!(!words_change("no emotes at all", &partner, Some(&home)));
    }

    #[test]
    fn a_partial_set_serves_only_until_a_whole_one_lands_and_its_rows_stay_queued() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        let globals = set(&[("g1", "globalEmote")]);
        let whole = set(&[("p1", "partnerEmote"), ("g1", "globalEmote")]);

        // No runtime here, so no fetch starts; the row is queued for the room.
        assert!(lookup_for("homep", "880", "row1").is_none());
        // The 7TV channel lookup failed: the globals serve, the row stays queued.
        let (lookup, restamp) = install("880", Some((&globals, false)), false).expect("room known");
        assert_eq!(lookup.map(|l| l.len()), Some(1));
        assert_eq!(restamp.len(), 1, "re-stamped with what loaded");
        assert_eq!(pending_for_test("880").map(|(_, n)| n), Some(1), "and still waiting for the whole set");
        // New rows parse with the partial set and wait for the whole one too.
        assert!(lookup_for("homep", "880", "row2").is_some());
        assert_eq!(pending_for_test("880").map(|(_, n)| n), Some(2));
        // A partial answer that is no fuller changes nothing.
        let (lookup, _) = install("880", Some((&set(&[]), false)), false).expect("room known");
        assert_eq!(lookup.map(|l| l.len()), Some(1));

        // The whole set lands: every queued row is re-stamped and let go.
        let (lookup, restamp) = install("880", Some((&whole, true)), false).expect("room known");
        assert_eq!(lookup.map(|l| l.len()), Some(2));
        assert_eq!(restamp.len(), 2);
        assert_eq!(pending_for_test("880").map(|(_, n)| n), Some(0));
        // A later partial answer never replaces it, and rows no longer queue.
        let (lookup, _) = install("880", Some((&globals, false)), false).expect("room known");
        assert_eq!(lookup.map(|l| l.len()), Some(2));
        assert!(lookup_for("homep", "880", "row3").is_some());
        assert_eq!(pending_for_test("880").map(|(_, n)| n), Some(0));
        reset_for_test();
    }

    #[test]
    fn a_row_parsed_with_the_partial_set_is_patched_even_when_the_home_set_agrees() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        // Both channels carry the same 7TV emote; the partner's partial set lacked it.
        let home = EmoteLookup::build(&set(&[("s1", "sharedEmote")]));
        lookup_for("homeq", "881", "row");
        install("881", Some((&set(&[("g1", "globalEmote")]), false)), false);
        let (lookup, _) =
            install("881", Some((&set(&[("s1", "sharedEmote"), ("g1", "globalEmote")]), true)), false).expect("room known");
        let whole = lookup.expect("whole set");
        assert!(words_change("sharedEmote", &whole, Some(&home)), "it was text under the partial set");
        assert!(!words_change("plain words", &whole, Some(&home)));
        reset_for_test();
    }

    #[test]
    fn the_retry_loop_stops_once_no_chat_shows_the_room() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        lookup_for("homer", "882", "row");
        assert!(room_open("882"));
        assert!(!begin_retry("882"), "a fetch is already marked running");
        install("882", None, false);
        assert!(begin_retry("882"), "a retry may fetch once nothing else does");
        install("882", None, false);
        release_home("homer");
        assert!(!room_open("882"));
        assert!(!begin_retry("882"), "the room is gone");
        reset_for_test();
    }

    #[test]
    fn leaving_the_home_chat_drops_its_rooms_and_the_table_stays_bounded() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        // No runtime here, so no fetch starts; the rooms are still recorded.
        for i in 0..(MAX_ROOMS + 4) {
            let room = format!("r{i}");
            assert!(lookup_for("homea", &room, "raw").is_none());
            if let Ok(mut map) = rooms().lock() {
                map.get_mut(&room).unwrap().fetching = false;
            }
        }
        assert!(rooms().lock().unwrap().len() <= MAX_ROOMS);
        lookup_for("homeb", "shared", "raw");
        lookup_for("homea", "shared", "raw");
        if let Ok(mut map) = rooms().lock() {
            map.get_mut("shared").unwrap().fetching = false;
        }
        release_home("homea");
        let map = rooms().lock().unwrap();
        assert!(map.contains_key("shared"), "homeb still shows it");
        assert!(map.keys().all(|k| k == "shared"));
        drop(map);
        release_home("homeb");
        assert!(rooms().lock().unwrap().is_empty());
    }
}
