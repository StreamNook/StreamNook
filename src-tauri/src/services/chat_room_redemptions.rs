//! Other viewers' reward redemptions as rows in every open Twitch chat room,
//! whether or not its stream is playing (the phones' Chats screen, a
//! chat-only window, a MultiNook tile that is not focused).
//!
//! The watched socket (`BackgroundService::set_watched_channel`) follows only
//! the channel on screen, so a room with no stream playing never heard
//! Twitch's redemption feed. This keeps a second socket that carries that
//! feed alone (`community-points-channel-v1`) for every joined room, and
//! nothing else: the watched channel's points, predictions and chest stay on
//! the watched socket, gated by what is being watched. A room both sockets
//! carry is handled by the watched one only.
//!
//! IRC nudges this on every confirmed join and every part; a burst (a
//! reconnect rejoining every room) settles into one change, and the socket is
//! rebuilt only when the set of rooms or the sign-in changed. With no room
//! open, no socket.

use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::Notify;

use crate::services::channel_points_websocket_service::ChannelPointsWebSocketService;
use crate::services::drops_auth_service::DropsAuthService;
use crate::services::irc_service::IrcService;
use crate::services::twitch_service::TwitchService;

/// Quiet time after the last nudge before the rooms are read again.
const SETTLE: Duration = Duration::from_secs(2);
/// A room whose id is not known yet is looked at again this soon.
const ID_RETRY: Duration = Duration::from_secs(10);
/// How often the sign-in is read again while rooms are open, so a refreshed
/// token reaches the socket before the old one stops working.
const RECHECK: Duration = Duration::from_secs(30 * 60);
/// One topic per room; four connections of 50 leave the other connections
/// Twitch allows per address to the watched socket.
const MAX_ROOMS: usize = 200;

fn wake() -> &'static Notify {
    static WAKE: OnceLock<Notify> = OnceLock::new();
    WAKE.get_or_init(Notify::new)
}

/// The open chat rooms changed (or the sign-in did): follow them again.
pub fn nudge() {
    if cfg!(test) {
        return;
    }
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        tauri::async_runtime::spawn(follow_rooms());
    });
    wake().notify_one();
}

/// (room id, login) for each joined login whose id is known, sorted by id
/// and capped, and whether any joined login's id is still unknown.
fn rooms_for(joined: &[String], id_of: impl Fn(&str) -> Option<String>) -> (Vec<(String, String)>, bool) {
    let mut missing = false;
    let mut rooms: Vec<(String, String)> = Vec::new();
    for login in joined {
        match id_of(login).filter(|id| !id.is_empty()) {
            Some(id) => rooms.push((id, login.clone())),
            None => missing = true,
        }
    }
    rooms.sort();
    rooms.dedup_by(|a, b| a.0 == b.0);
    rooms.truncate(MAX_ROOMS);
    (rooms, missing)
}

/// The sign-in the socket listens with: the drops one the watched socket
/// uses, else the app's own.
async fn token() -> Option<String> {
    if let Ok(t) = DropsAuthService::get_token().await {
        if !t.is_empty() {
            return Some(t);
        }
    }
    TwitchService::get_token().await.ok().filter(|t| !t.is_empty())
}

async fn follow_rooms() {
    let mut socket = ChannelPointsWebSocketService::new_chat_rooms();
    let mut followed: Vec<String> = Vec::new();
    let mut followed_token = String::new();
    let mut next_wait: Option<Duration> = None;
    loop {
        match next_wait {
            None => wake().notified().await,
            Some(wait) => {
                let _ = tokio::time::timeout(wait, wake().notified()).await;
            }
        }
        while tokio::time::timeout(SETTLE, wake().notified()).await.is_ok() {}

        let joined = IrcService::current_channel_list().await;
        let (rooms, missing) = rooms_for(&joined, crate::services::channel_ids::known);
        next_wait = missing.then_some(ID_RETRY);

        if rooms.is_empty() {
            if !followed.is_empty() {
                socket.disconnect_all().await;
                followed.clear();
                followed_token.clear();
                log::info!("[RoomFeed] no chat room open; redemption feed closed");
            }
            continue;
        }
        next_wait = Some(next_wait.unwrap_or(RECHECK));

        let Some(token) = token().await else {
            if !followed.is_empty() {
                socket.disconnect_all().await;
                followed.clear();
                followed_token.clear();
            }
            log::info!("[RoomFeed] no sign-in; redemption feed not opened");
            continue;
        };
        let ids: Vec<String> = rooms.iter().map(|(id, _)| id.clone()).collect();
        if ids == followed && token == followed_token {
            continue;
        }
        let Some(app) = crate::services::providers::app_handle() else { continue };
        if let Err(e) = socket.connect_to_channels(ids.clone(), "", &token, app).await {
            log::warn!("[RoomFeed] redemption feed did not open: {e}");
            continue;
        }
        for (id, login) in &rooms {
            socket.register_channel_mapping(id, login, login).await;
        }
        log::info!("[RoomFeed] redemption feed follows {} chat room(s)", ids.len());
        followed = ids;
        followed_token = token;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooms_are_the_joined_logins_with_a_known_id() {
        let joined = vec!["b".to_string(), "a".to_string(), "ghost".to_string()];
        let (rooms, missing) = rooms_for(&joined, |l| match l {
            "a" => Some("2".into()),
            "b" => Some("1".into()),
            _ => None,
        });
        assert_eq!(rooms, vec![("1".into(), "b".into()), ("2".into(), "a".into())]);
        assert!(missing);
    }

    #[test]
    fn the_room_count_is_capped() {
        let joined: Vec<String> = (0..MAX_ROOMS + 5).map(|i| format!("c{i}")).collect();
        let (rooms, missing) = rooms_for(&joined, |l| Some(l.trim_start_matches('c').to_string()));
        assert_eq!(rooms.len(), MAX_ROOMS);
        assert!(!missing);
    }
}
