//! Twitch's Shared Viewership: channels streaming together (Stream Together
//! with Shared Chat on), who is in the group and the combined viewer count.
//!
//! Two surfaces read it: the chat state of a watched channel
//! (`channel_state`) and the stream cards of the Home snapshot
//! (`home_snapshot`). A channel is often on both at once, so answers are
//! cached for `FRESH` and a second caller inside that window reads the cache
//! instead of asking Twitch again. One anonymous GQL request covers up to
//! `BATCH` channels.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use log::debug;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::services::twitch_service::TwitchService;

/// Channels per request.
const BATCH: usize = 30;
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
    for chunk in due.chunks(BATCH) {
        let data = match TwitchService::get_collaborations(chunk).await {
            Ok(data) => data,
            Err(e) => {
                debug!("[Collaboration] {} channels: {e}", chunk.len());
                continue;
            }
        };
        let now = Instant::now();
        let mut cache = cache().lock().await;
        for (i, id) in chunk.iter().enumerate() {
            let collab = data.get(format!("c{i}").as_str()).and_then(parse);
            cache.insert(id.clone(), (now, collab.clone()));
            out.insert(id.clone(), collab);
        }
    }
    out
}

/// Read one `user` node of the query. Only ACTIVE members who are live count;
/// `None` unless Twitch reports a combined count and at least two remain.
fn parse(user: &serde_json::Value) -> Option<Collaboration> {
    let self_id = user.get("id").and_then(|v| v.as_str())?;
    let shared_viewers = user.pointer("/stream/collaborationViewersCount")?.as_u64()?;
    let mut members: Vec<Collaborator> = user
        .pointer("/channel/collaboration/collaborators")?
        .as_array()?
        .iter()
        .filter(|c| c.get("status").and_then(|s| s.as_str()).is_none_or(|s| s == "ACTIVE"))
        .filter_map(|c| {
            let u = c.get("user")?;
            let viewer_count = u.pointer("/stream/viewersCount")?.as_u64()?;
            let user_id = u.get("id")?.as_str()?.to_string();
            let login = u.get("login")?.as_str()?.to_string();
            let display_name = u
                .get("displayName")
                .and_then(|n| n.as_str())
                .filter(|n| !n.is_empty())
                .unwrap_or(&login)
                .to_string();
            Some(Collaborator {
                is_self: user_id == self_id,
                is_leader: c.get("role").and_then(|r| r.as_str()) == Some("LEADER"),
                avatar_url: u.get("profileImageURL").and_then(|v| v.as_str()).map(String::from),
                viewer_count,
                user_id,
                login,
                display_name,
            })
        })
        .collect();
    if members.len() < 2 {
        return None;
    }
    members.sort_by(|a, b| {
        b.is_self
            .cmp(&a.is_self)
            .then(b.viewer_count.cmp(&a.viewer_count))
    });
    Some(Collaboration { shared_viewers, members })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn member(id: &str, login: &str, role: &str, status: &str, viewers: Option<u64>) -> serde_json::Value {
        json!({"role": role, "status": status, "user": {
            "id": id, "login": login, "displayName": login.to_uppercase(),
            "profileImageURL": format!("https://x/{login}.png"),
            "stream": viewers.map(|v| json!({"viewersCount": v}))
        }})
    }

    #[test]
    fn self_first_then_by_viewers() {
        let v = json!({"id": "2", "stream": {"viewersCount": 400, "collaborationViewersCount": 1500},
            "channel": {"collaboration": {"collaborators": [
                member("1", "small", "LEADER", "ACTIVE", Some(100)),
                member("2", "me", "MEMBER", "ACTIVE", Some(400)),
                member("3", "big", "MEMBER", "ACTIVE", Some(1000)),
                member("4", "gone", "MEMBER", "LEFT", Some(5000)),
                member("5", "offline", "MEMBER", "ACTIVE", None),
            ]}}});
        let c = parse(&v).unwrap();
        assert_eq!(c.shared_viewers, 1500);
        let logins: Vec<&str> = c.members.iter().map(|m| m.login.as_str()).collect();
        assert_eq!(logins, ["me", "big", "small"]);
        assert!(c.members[0].is_self);
        assert!(c.members[2].is_leader);
        assert_eq!(c.members[1].display_name, "BIG");
    }

    #[test]
    fn solo_offline_and_lone_member_have_none() {
        let solo = json!({"id": "1", "stream": {"viewersCount": 10, "collaborationViewersCount": null},
            "channel": {"collaboration": null}});
        assert!(parse(&solo).is_none());
        let offline = json!({"id": "1", "stream": null, "channel": {"collaboration": null}});
        assert!(parse(&offline).is_none());
        let lone = json!({"id": "1", "stream": {"viewersCount": 10, "collaborationViewersCount": 10},
            "channel": {"collaboration": {"collaborators": [
                member("1", "me", "LEADER", "ACTIVE", Some(10)),
                member("2", "left", "MEMBER", "LEFT", Some(3)),
                member("3", "down", "MEMBER", "ACTIVE", None),
            ]}}});
        assert!(parse(&lone).is_none());
    }
}
