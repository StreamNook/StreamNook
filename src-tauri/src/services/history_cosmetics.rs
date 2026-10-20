//! History rows wear their authors' cosmetics.
//!
//! The join backfill is parsed in one batch, usually before most of its
//! authors' 7TV cosmetics, personal emote sets and StreamNook themes are known.
//! A native shell puts those rows on screen only when the history answer
//! returns, so a repaint frame sent while they were being parsed finds nothing
//! to patch and the backlog stays plain. History therefore looks its authors up
//! itself: a short wait before the answer covers most of them, and the rest are
//! re-stamped and patched in (`HISTORY_SEGMENTS` carrying the new view) once
//! their lookups finish, then once more when their personal emote sets land.
//! The channel's own moderator and VIP art (its FFZ room) is the same kind of
//! late answer, so a backlog answered before the room was read is patched again
//! once it has been. So is the channel's own badge set (its subscriber art):
//! history reads it itself when it is not noted yet, waits the same short time
//! for it, and patches the backlog once it lands.
//! Only native shells draw from the stamped view; the web renderers resolve a
//! row's cosmetics on the page.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::task::JoinHandle;

use crate::models::chat_layout::ChatMessage;
use crate::services::cosmetics_registry as registry;
use crate::services::irc_service::IrcService;
use crate::services::{chat_row_view, ffz_room_badges, member_theme, seventv_cosmetics_resolver as resolver};

/// The longest the history answer waits for its authors' lookups. Short on
/// purpose: an empty chat is worse than a name gaining its paint a moment later
/// (which moves nothing), so anything slower arrives by patch.
const ANSWER_WAIT: Duration = Duration::from_millis(250);
/// How long the follow-up waits for lookups the answer could not wait for.
const FOLLOW_WAIT: Duration = Duration::from_secs(20);
/// How long it then waits for the authors' personal emote sets.
const PERSONAL_WAIT: Duration = Duration::from_secs(6);
/// Lets the shell put the answer's rows on screen before a patch names them.
const PATCH_DELAY: Duration = Duration::from_millis(500);
/// How long the follow-up waits for the channel's FFZ room, or its own badge
/// set, to be read.
const ROOM_ART_WAIT: Duration = Duration::from_secs(10);
/// How long the registry may take to load before members are looked up.
const REGISTRY_WAIT: Duration = Duration::from_secs(5);

/// The lookups a backlog still needs: 7TV cosmetics not cached yet (with the
/// Twitch id, whose personal set is then waited on), and StreamNook members
/// whose theme is not known yet.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Needed {
    pub cosmetics: Vec<(String, Option<String>)>,
    pub members: Vec<String>,
}

impl Needed {
    fn is_empty(&self) -> bool {
        self.cosmetics.is_empty() && self.members.is_empty()
    }
}

/// Works out what a backlog's authors (`(provider, user_id)`) still need.
pub(crate) fn needed(
    authors: &[(String, String)],
    cached: impl Fn(&str) -> bool,
    is_member: impl Fn(&str) -> bool,
    theme_known: impl Fn(&str) -> bool,
) -> Needed {
    let mut need = Needed::default();
    let mut seen = HashSet::new();
    for (provider, user_id) in authors {
        if user_id.is_empty() || !seen.insert((provider.as_str(), user_id.as_str())) {
            continue;
        }
        if let Some(id) = resolver::cosmetics_id(provider, user_id) {
            if !cached(&id) {
                let twitch = (provider.is_empty() || provider == "twitch").then(|| user_id.clone());
                need.cosmetics.push((id, twitch));
            }
        }
        if (provider.is_empty() || provider == "twitch") && is_member(user_id) && !theme_known(user_id) {
            need.members.push(user_id.clone());
        }
    }
    need
}

/// What a row looks like on a native shell: its segments and its view.
fn signature(m: &ChatMessage) -> String {
    serde_json::to_string(&(
        &m.segments,
        m.metadata.reply_info.as_ref().and_then(|r| r.parent_segments.as_ref()),
        &m.metadata.view,
    ))
    .unwrap_or_default()
}

/// The rows of `again` that look different from what the shell was given
/// (`before`: id to signature), as `HISTORY_SEGMENTS` patches. Rows the shell
/// never had are left out.
pub(crate) fn patches(before: &HashMap<String, String>, again: &[ChatMessage]) -> Vec<Value> {
    again
        .iter()
        .filter(|m| before.get(&m.id).is_some_and(|sig| *sig != signature(m)))
        .map(|m| {
            let mut row = json!({
                "id": m.id,
                "segments": m.segments,
                "parent_segments": m.metadata.reply_info.as_ref().and_then(|r| r.parent_segments.as_ref()),
            });
            if let Some(view) = &m.metadata.view {
                row["view"] = json!(view);
            }
            row
        })
        .collect()
}

/// Runs the lookups; answers the authors' personal sets `(twitch_id, set_id)`.
async fn look_up(need: Needed) -> Vec<(String, String)> {
    let cosmetics = futures::future::join_all(need.cosmetics.into_iter().map(|(id, twitch)| async move {
        let lookup = resolver::resolve(id).await;
        twitch.zip(lookup.data.personal_set)
    }));
    let members = futures::future::join_all(need.members.iter().map(|id| member_theme::resolve(id)));
    let (sets, _) = tokio::join!(cosmetics, members);
    sets.into_iter().flatten().collect()
}

/// What a backlog's authors still need, read from the caches now.
fn needed_now(authors: &[(String, String)]) -> Needed {
    needed(
        authors,
        |id| resolver::peek(id).is_some(),
        registry::is_member,
        |id| member_theme::peek(id).is_some(),
    )
}

/// The backlog as the history answer returns it on a native shell. When every
/// author is already known it is answered at once; otherwise it waits at most
/// `ANSWER_WAIT` for their lookups, parses again, and patches in the slower
/// ones later. `rows` is the first parse of `raw`; on the web renderers it is
/// returned untouched.
pub async fn decorate(channel: &str, raw: &[String], rows: Vec<ChatMessage>) -> Vec<ChatMessage> {
    if !chat_row_view::enabled() || rows.is_empty() {
        return rows;
    }
    let started = Instant::now();
    let authors: Vec<(String, String)> = rows
        .iter()
        .map(|m| (m.provider.clone(), m.user_id.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let room = rows.iter().find_map(|m| m.tags.get("room-id").cloned());
    // A room not read yet draws Twitch's own moderator and VIP badges until it is.
    let room_art = room.clone().filter(|room| ffz_room_badges::enabled() && !ffz_room_badges::has_entry(room));
    // A channel whose own badge set is not noted yet would put Twitch's default
    // subscriber badge on every backlog row: it is read now and briefly waited
    // on, and a read that takes longer patches the backlog when it lands.
    let mut rows = rows;
    let mut set_read = room.filter(|room| !chat_row_view::has_room_badges(room)).map(|room| {
        tokio::spawn(async move {
            if let Err(e) = crate::commands::badges::fetch_channel_badges(room.clone()).await {
                log::debug!("[ChatHistory] channel badges for {room} unavailable: {e}");
            }
        })
    });
    if let Some(read) = set_read.as_mut() {
        if tokio::time::timeout(ANSWER_WAIT, read).await.is_ok() {
            set_read = None;
            rows = IrcService::parse_historical_messages(raw.to_vec()).await;
        }
    }
    // Members are only known once the registry has loaded; before that, the
    // lookup task waits for it and the members arrive by patch.
    if registry::ready(Duration::ZERO).await && needed_now(&authors).is_empty() {
        log::info!("[ChatHistory] {channel}: every author's cosmetics known, answered at once");
        if room_art.is_some() || set_read.is_some() {
            spawn_follow(channel, raw, &rows, None, Vec::new(), room_art, set_read);
        }
        return rows;
    }
    let mut lookups: JoinHandle<Option<Vec<(String, String)>>> = tokio::spawn(async move {
        registry::ready(REGISTRY_WAIT).await;
        let need = needed_now(&authors);
        if need.is_empty() {
            return None;
        }
        Some(look_up(need).await)
    });
    let rows = match tokio::time::timeout(ANSWER_WAIT, &mut lookups).await {
        Ok(Ok(None)) | Ok(Err(_)) => {
            if room_art.is_some() || set_read.is_some() {
                spawn_follow(channel, raw, &rows, None, Vec::new(), room_art, set_read);
            }
            rows
        }
        Ok(Ok(Some(sets))) => {
            let rows = IrcService::parse_historical_messages(raw.to_vec()).await;
            if !sets.is_empty() || room_art.is_some() || set_read.is_some() {
                spawn_follow(channel, raw, &rows, None, sets, room_art, set_read);
            }
            rows
        }
        Err(_) => {
            // Some lookups are slow: answer with what is known now, patch the rest.
            let rows = IrcService::parse_historical_messages(raw.to_vec()).await;
            spawn_follow(channel, raw, &rows, Some(lookups), Vec::new(), room_art, set_read);
            rows
        }
    };
    log::info!("[ChatHistory] {channel}: waited {} ms for authors' cosmetics", started.elapsed().as_millis());
    rows
}

fn spawn_follow(
    channel: &str,
    raw: &[String],
    shown: &[ChatMessage],
    pending: Option<JoinHandle<Option<Vec<(String, String)>>>>,
    sets: Vec<(String, String)>,
    room_art: Option<String>,
    set_read: Option<JoinHandle<()>>,
) {
    let channel = channel.to_string();
    let raw = raw.to_vec();
    let mut before: HashMap<String, String> = shown.iter().map(|m| (m.id.clone(), signature(m))).collect();
    tokio::spawn(async move {
        // First, as it is usually quick: the channel's own badge set.
        if let Some(read) = set_read {
            let _ = tokio::time::timeout(ROOM_ART_WAIT, read).await;
            tokio::time::sleep(PATCH_DELAY).await;
            patch(&channel, &raw, &mut before).await;
        }
        let mut sets = sets;
        if let Some(handle) = pending {
            if let Ok(Ok(Some(found))) = tokio::time::timeout(FOLLOW_WAIT, handle).await {
                sets = found;
            }
            tokio::time::sleep(PATCH_DELAY).await;
            patch(&channel, &raw, &mut before).await;
        }
        if !sets.is_empty() {
            let deadline = Instant::now() + PERSONAL_WAIT;
            loop {
                let mut all = true;
                for (twitch_id, set_id) in &sets {
                    if !IrcService::has_personal_set(twitch_id, set_id).await {
                        all = false;
                        break;
                    }
                }
                if all || Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            tokio::time::sleep(PATCH_DELAY).await;
            patch(&channel, &raw, &mut before).await;
        }
        if let Some(room) = room_art {
            if !ffz_room_badges::has_entry(&room) {
                match ffz_room_badges::refresh(&room) {
                    Some(read) => {
                        let _ = tokio::time::timeout(ROOM_ART_WAIT, read).await;
                    }
                    None => {
                        let deadline = Instant::now() + ROOM_ART_WAIT;
                        while !ffz_room_badges::has_entry(&room) && Instant::now() < deadline {
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                    }
                }
            }
            tokio::time::sleep(PATCH_DELAY).await;
            patch(&channel, &raw, &mut before).await;
        }
    });
}

/// Parses the backlog again and sends the rows that now look different.
async fn patch(channel: &str, raw: &[String], before: &mut HashMap<String, String>) {
    let again = IrcService::parse_historical_messages(raw.to_vec()).await;
    let rows = patches(before, &again);
    for m in &again {
        if let Some(sig) = before.get_mut(&m.id) {
            *sig = signature(m);
        }
    }
    if rows.is_empty() {
        return;
    }
    log::debug!("[ChatHistory] {channel}: {} backlog rows now wear their authors' cosmetics", rows.len());
    let frame = json!({ "type": "HISTORY_SEGMENTS", "channel": channel, "rows": rows });
    crate::services::irc_service::post_channel_frame(channel, frame.to_string()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authors(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(p, u)| (p.to_string(), u.to_string())).collect()
    }

    #[test]
    fn only_unknown_authors_are_looked_up_once() {
        let need = needed(
            &authors(&[("twitch", "1"), ("twitch", "1"), ("twitch", "2"), ("twitch", "3"), ("twitch", "")]),
            |id| id == "2",
            |id| id == "1" || id == "3",
            |id| id == "3",
        );
        assert_eq!(need.cosmetics, vec![("1".to_string(), Some("1".to_string())), ("3".to_string(), Some("3".to_string()))]);
        assert_eq!(need.members, vec!["1".to_string()]);
    }

    #[test]
    fn nothing_needed_when_everything_is_known() {
        let need = needed(&authors(&[("twitch", "1")]), |_| true, |_| true, |_| true);
        assert!(need.is_empty());
    }

    #[tokio::test]
    async fn a_patch_names_only_rows_that_changed_and_carries_the_view() {
        let line = |id: &str| {
            format!("@display-name=Dee;emotes=;historical=1;id={id};room-id=12;tmi-sent-ts=1700000000000;user-id=48 :dee!dee@dee.tmi.twitch.tv PRIVMSG #histcosm_chan :hello")
        };
        let rows = IrcService::parse_historical_messages(vec![line("a"), line("b")]).await;
        assert_eq!(rows.len(), 2);
        let before: HashMap<String, String> = rows.iter().map(|m| (m.id.clone(), signature(m))).collect();
        assert!(patches(&before, &rows).is_empty());

        // The author's paint arrived: only that row's view changed.
        let mut again = rows.clone();
        again[0].metadata.view = Some(Default::default());
        let out = patches(&before, &again);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], "a");
        assert!(out[0].get("view").is_some());

        // A row the shell never had is left out.
        let mut unknown = rows[1].clone();
        unknown.id = "z".into();
        unknown.metadata.view = Some(Default::default());
        assert!(patches(&before, &[unknown]).is_empty());
    }
}
