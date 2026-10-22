//! Twitch's Shared Viewership: channels streaming together (Stream Together
//! with Shared Chat on), who is in the group and the combined viewer count.
//!
//! Two surfaces read it: the chat state of a watched channel
//! (`channel_state`) and the stream cards of the Home snapshot
//! (`home_snapshot`). A channel is often on both at once, so answers are
//! cached for `FRESH` and a second caller inside that window reads the cache
//! instead of asking Twitch again. One anonymous GQL request covers up to
//! `BATCH` channels, through the same list field twitch.tv reads for the
//! channels on its screen, and up to `PARALLEL` of them run at once.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use log::warn;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::services::twitch_service::TwitchService;

/// Channels per request. They go in one list argument, not an alias each, so
/// Twitch's cap of 15 aliases per request does not apply; a Home's worth of
/// cards fits in one.
pub(crate) const BATCH: usize = 100;
/// Requests in flight at once, so a longer card list still answers inside the
/// Home snapshot's wait.
const PARALLEL: usize = 3;
/// How long an answer serves every caller.
const FRESH: Duration = Duration::from_secs(30);
/// Entries not asked about for this long are dropped.
const KEEP: Duration = Duration::from_secs(300);

/// One channel in a collaboration.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Collaborator {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// This channel's own viewers. Always live: a member whose stream is
    /// down is not streaming together and is left out of the group.
    pub viewer_count: u64,
    /// The channel that started the group.
    pub is_leader: bool,
    /// The channel the group was read for.
    pub is_self: bool,
}

/// A channel's collaboration. `members` holds the channel itself first, then
/// the rest by their own viewer count; it always has at least two.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Collaboration {
    /// Unique viewers across every member, which can be less than the sum.
    pub shared_viewers: u64,
    pub members: Vec<Collaborator>,
    /// The group in plain words, for screen readers: "1,500 watching together
    /// with A, B and C".
    pub label: String,
}

impl Collaboration {
    fn new(shared_viewers: u64, members: Vec<Collaborator>) -> Self {
        let label = format!("{} watching together with {}", grouped(shared_viewers), others_phrase(&members));
        Self { shared_viewers, members, label }
    }
}

/// The others in a group, never the channel itself: "A", "A and B",
/// "A, B and C".
pub fn others_phrase(members: &[Collaborator]) -> String {
    let others: Vec<&str> = members.iter().filter(|m| !m.is_self).map(|m| m.display_name.as_str()).collect();
    match others.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// A count with thousands separators: 12345 reads "12,345".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

type Cache = HashMap<String, (Instant, Option<Collaboration>)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The collaboration of each Twitch channel id in `ids` (`None`: not in one).
/// An id whose request failed is left out, so the caller keeps what it had.
pub async fn fetch(ids: &[String]) -> HashMap<String, Option<Collaboration>> {
    let mut out = HashMap::new();
    let mut due = Vec::new();
    {
        let mut cache = cache().lock().await;
        cache.retain(|_, (at, _)| at.elapsed() < KEEP);
        for id in ids {
            match cache.get(id) {
                Some((at, collab)) if at.elapsed() < FRESH => {
                    out.insert(id.clone(), collab.clone());
                }
                _ if !due.contains(id) => due.push(id.clone()),
                _ => {}
            }
        }
    }
    // Owned chunks: a borrowed one makes the future not `Send` for the
    // spawned tasks that await it.
    let mut answers = stream::iter(due.chunks(BATCH).map(<[String]>::to_vec))
        .map(|chunk| async move {
            let answer = TwitchService::get_collaborations(&chunk).await;
            (chunk, answer)
        })
        .buffer_unordered(PARALLEL);
    while let Some((chunk, answer)) = answers.next().await {
        let rows = match answer {
            Ok(rows) => rows,
            Err(e) => {
                warn!("[Collaboration] {} channels: {e}", chunk.len());
                continue;
            }
        };
        // One row per id in order, but each is read by the id it carries.
        let by_id: HashMap<&str, &serde_json::Value> =
            rows.iter().filter_map(|row| Some((row.get("id")?.as_str()?, row))).collect();
        let now = Instant::now();
        let mut cache = cache().lock().await;
        for id in &chunk {
            let collab = by_id.get(id.as_str()).and_then(|row| parse(row));
            cache.insert(id.clone(), (now, collab.clone()));
            out.insert(id.clone(), collab);
        }
    }
    out
}

/// Read one row of the answer, for the channel it names. Only members who are
/// live count; `None` unless the channel is one of them with a combined count
/// from Twitch, and at least two remain.
fn parse(row: &serde_json::Value) -> Option<Collaboration> {
    let self_id = row.get("id").and_then(|v| v.as_str())?;
    let session = row.get("session").filter(|s| !s.is_null())?;
    let host_id = session.pointer("/host/id").and_then(|v| v.as_str());
    let mut shared_viewers = None;
    let mut members: Vec<Collaborator> = session
        .get("guests")?
        .as_array()?
        .iter()
        .filter_map(|g| {
            let u = g.get("user")?;
            let stream = u.get("stream")?;
            let viewer_count = stream.get("viewersCount")?.as_u64()?;
            let user_id = u.get("id")?.as_str()?.to_string();
            let login = u.get("login")?.as_str()?.to_string();
            let display_name = u
                .get("displayName")
                .and_then(|n| n.as_str())
                .filter(|n| !n.is_empty())
                .unwrap_or(&login)
                .to_string();
            let is_self = user_id == self_id;
            if is_self {
                shared_viewers = stream.get("collaborationViewersCount").and_then(|v| v.as_u64());
            }
            Some(Collaborator {
                is_self,
                is_leader: host_id == Some(user_id.as_str()),
                avatar_url: u.get("profileImageURL").and_then(|v| v.as_str()).map(String::from),
                viewer_count,
                user_id,
                login,
                display_name,
            })
        })
        .collect();
    let shared_viewers = shared_viewers?;
    if members.len() < 2 {
        return None;
    }
    members.sort_by(|a, b| {
        b.is_self
            .cmp(&a.is_self)
            .then(b.viewer_count.cmp(&a.viewer_count))
    });
    Some(Collaboration::new(shared_viewers, members))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A guest slot as Twitch sends it; `viewers: None` is a guest who is not
    /// live. Every live guest carries the group's combined count.
    fn guest(id: &str, login: &str, viewers: Option<u64>, combined: u64) -> serde_json::Value {
        json!({"user": {
            "id": id, "login": login, "displayName": login.to_uppercase(),
            "profileImageURL": format!("https://x/{login}.png"),
            "stream": viewers.map(|v| json!({"viewersCount": v, "collaborationViewersCount": combined}))
        }})
    }

    fn row(id: &str, host: &str, guests: Vec<serde_json::Value>) -> serde_json::Value {
        json!({"id": id, "session": {"host": {"id": host}, "guests": guests}})
    }

    #[test]
    fn self_first_then_by_viewers() {
        let v = row("2", "1", vec![
            guest("1", "small", Some(100), 1500),
            guest("2", "me", Some(400), 1500),
            guest("3", "big", Some(1000), 1500),
            guest("5", "offline", None, 1500),
        ]);
        let c = parse(&v).unwrap();
        assert_eq!(c.shared_viewers, 1500);
        let logins: Vec<&str> = c.members.iter().map(|m| m.login.as_str()).collect();
        assert_eq!(logins, ["me", "big", "small"]);
        assert!(c.members[0].is_self);
        assert!(c.members[2].is_leader);
        assert_eq!(c.members[1].display_name, "BIG");
        assert_eq!(c.label, "1,500 watching together with BIG and SMALL");
    }

    #[test]
    fn label_names_the_others_in_plain_words() {
        let m = |name: &str, is_self: bool| Collaborator {
            user_id: name.into(),
            login: name.into(),
            display_name: name.into(),
            avatar_url: None,
            viewer_count: 1,
            is_leader: false,
            is_self,
        };
        assert_eq!(others_phrase(&[m("me", true), m("A", false)]), "A");
        assert_eq!(others_phrase(&[m("me", true), m("A", false), m("B", false), m("C", false)]), "A, B and C");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1234567), "1,234,567");
    }

    #[test]
    fn solo_offline_and_lone_member_have_none() {
        let solo = json!({"id": "1", "session": null});
        assert!(parse(&solo).is_none());
        // The channel itself is not live, so it is not streaming together.
        let offline = row("1", "2", vec![guest("1", "me", None, 10), guest("2", "a", Some(5), 10), guest("3", "b", Some(5), 10)]);
        assert!(parse(&offline).is_none());
        let lone = row("1", "1", vec![guest("1", "me", Some(10), 10), guest("3", "down", None, 10)]);
        assert!(parse(&lone).is_none());
        // A session that does not have the channel in it is someone else's.
        let elsewhere = row("1", "2", vec![guest("2", "a", Some(5), 10), guest("3", "b", Some(5), 10)]);
        assert!(parse(&elsewhere).is_none());
    }

    /// Groups come and go, so the channels are passed in: `SN_COLLAB_IDS`,
    /// comma-separated Twitch ids of channels streaming together right now.
    #[tokio::test]
    #[ignore = "network: asks Twitch about channels named in SN_COLLAB_IDS"]
    async fn live_groups_answer_for_a_list_of_channels() {
        let ids: Vec<String> = std::env::var("SN_COLLAB_IDS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        assert!(!ids.is_empty(), "set SN_COLLAB_IDS");
        let found = fetch(&ids).await;
        for id in &ids {
            let collab = found.get(id).cloned().flatten();
            let collab = collab.unwrap_or_else(|| panic!("{id}: no group"));
            println!("{id}: {} ({} members)", collab.label, collab.members.len());
            assert!(collab.members[0].is_self);
            assert!(collab.members.iter().any(|m| m.is_leader));
        }
    }
}
