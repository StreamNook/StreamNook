//! Twitch Shared Chat for stream cards: channels whose chat is merged with
//! other channels' outside a Stream Together group (a group already implies
//! Shared Chat, and `collaboration` covers it).
//!
//! The Home snapshot asks for the channels on its cards. Twitch only answers
//! this for a signed-in caller, one channel per Helix request, so answers are
//! cached (`FRESH_IN` while a channel is sharing, `FRESH_OUT` while it is not;
//! sessions last hours) and at most `PARALLEL` requests run at once. The
//! participants' names and faces come from one anonymous GQL batch.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use log::debug;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::services::collaboration::Collaborator;
use crate::services::twitch_service::TwitchService;

/// Helix requests in flight at once.
const PARALLEL: usize = 6;
/// Channels per GQL faces request.
const BATCH: usize = 30;
/// How long "is sharing" serves every caller.
const FRESH_IN: Duration = Duration::from_secs(120);
/// How long "is not sharing" serves every caller.
const FRESH_OUT: Duration = Duration::from_secs(300);
/// Entries not asked about for this long are dropped.
const KEEP: Duration = Duration::from_secs(600);

/// A channel's Shared Chat session. `members` holds the channel itself first,
/// then the rest by their own viewer count; only live channels, at least two.
/// `is_leader` marks the session's host.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct SharedChat {
    pub members: Vec<Collaborator>,
}

type Cache = HashMap<String, (Instant, Option<SharedChat>)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fresh(at: Instant, answer: &Option<SharedChat>) -> bool {
    at.elapsed() < if answer.is_some() { FRESH_IN } else { FRESH_OUT }
}

/// The Shared Chat session of each Twitch channel id in `ids` (`None`: not in
/// one). An id whose request failed is left out, so the caller keeps what it
/// had; signed out, every id is left out.
pub async fn fetch(ids: &[String]) -> HashMap<String, Option<SharedChat>> {
    let mut out = HashMap::new();
    let mut due = Vec::new();
    {
        let mut cache = cache().lock().await;
        cache.retain(|_, (at, _)| at.elapsed() < KEEP);
        for id in ids {
            match cache.get(id) {
                Some((at, answer)) if fresh(*at, answer) => {
                    out.insert(id.clone(), answer.clone());
                }
                _ if !due.contains(id) => due.push(id.clone()),
                _ => {}
            }
        }
    }
    if due.is_empty() || TwitchService::get_token().await.is_err() {
        return out;
    }

    let sessions: Vec<(String, Option<(String, Vec<String>)>)> = stream::iter(due)
        .map(|id| async move {
            let answer = TwitchService::get_shared_chat_session(&id).await;
            (id, answer)
        })
        .buffer_unordered(PARALLEL)
        .filter_map(|(id, answer)| async move {
            match answer {
                Ok(session) => Some((id, session)),
                Err(e) => {
                    debug!("[SharedChat] {id}: {e}");
                    None
                }
            }
        })
        .collect()
        .await;

    let mut wanted: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for (_, session) in &sessions {
        for p in session.iter().flat_map(|(_, participants)| participants) {
            if seen.insert(p.clone()) {
                wanted.push(p.clone());
            }
        }
    }
    let mut faces: HashMap<String, serde_json::Value> = HashMap::new();
    let mut faces_failed = false;
    for chunk in wanted.chunks(BATCH) {
        match TwitchService::get_channel_faces(chunk).await {
            Ok(data) => {
                for (i, id) in chunk.iter().enumerate() {
                    if let Some(user) = data.get(format!("u{i}").as_str()).filter(|u| !u.is_null()) {
                        faces.insert(id.clone(), user.clone());
                    }
                }
            }
            Err(e) => {
                debug!("[SharedChat] faces for {} channels: {e}", chunk.len());
                faces_failed = true;
            }
        }
    }

    let now = Instant::now();
    let mut cache = cache().lock().await;
    for (id, session) in sessions {
        let answer = match session {
            None => None,
            // Without the faces the group cannot be drawn; ask again next pass.
            Some(_) if faces_failed => continue,
            Some((host, participants)) => build(&id, &host, &participants, &faces),
        };
        cache.insert(id.clone(), (now, answer.clone()));
        out.insert(id, answer);
    }
    out
}

/// The group as cards draw it: live participants only, the channel first,
/// then by their own viewers. `None` unless at least two are live.
fn build(
    self_id: &str,
    host: &str,
    participants: &[String],
    faces: &HashMap<String, serde_json::Value>,
) -> Option<SharedChat> {
    let mut members: Vec<Collaborator> = participants
        .iter()
        .filter_map(|pid| {
            let u = faces.get(pid)?;
            let viewer_count = u.pointer("/stream/viewersCount")?.as_u64()?;
            let login = u.get("login")?.as_str()?.to_string();
            let display_name = u
                .get("displayName")
                .and_then(|n| n.as_str())
                .filter(|n| !n.is_empty())
                .unwrap_or(&login)
                .to_string();
            Some(Collaborator {
                is_self: pid == self_id,
                is_leader: pid == host,
                avatar_url: u.get("profileImageURL").and_then(|v| v.as_str()).map(String::from),
                viewer_count,
                user_id: pid.clone(),
                login,
                display_name,
            })
        })
        .collect();
    if members.len() < 2 || !members.iter().any(|m| m.is_self) {
        return None;
    }
    members.sort_by(|a, b| b.is_self.cmp(&a.is_self).then(b.viewer_count.cmp(&a.viewer_count)));
    Some(SharedChat { members })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn faces(list: &[(&str, &str, Option<u64>)]) -> HashMap<String, serde_json::Value> {
        list.iter()
            .map(|(id, login, viewers)| {
                (
                    id.to_string(),
                    json!({"id": id, "login": login, "displayName": login.to_uppercase(),
                        "profileImageURL": format!("https://x/{login}.png"),
                        "stream": viewers.map(|v| json!({"viewersCount": v}))}),
                )
            })
            .collect()
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn self_first_then_by_viewers_live_only() {
        let f = faces(&[("1", "host", Some(50)), ("2", "me", Some(10)), ("3", "big", Some(900)), ("4", "off", None)]);
        let g = build("2", "1", &ids(&["1", "2", "3", "4"]), &f).unwrap();
        let logins: Vec<&str> = g.members.iter().map(|m| m.login.as_str()).collect();
        assert_eq!(logins, ["me", "big", "host"]);
        assert!(g.members[0].is_self);
        assert!(g.members[2].is_leader);
        assert_eq!(g.members[1].display_name, "BIG");
    }

    #[test]
    fn needs_two_live_including_self() {
        let f = faces(&[("1", "host", Some(50)), ("2", "me", Some(10)), ("3", "off", None)]);
        assert!(build("2", "1", &ids(&["2", "3"]), &f).is_none());
        assert!(build("9", "1", &ids(&["1", "2"]), &f).is_none());
        assert!(build("2", "1", &ids(&["1", "2"]), &f).is_some());
    }
}
