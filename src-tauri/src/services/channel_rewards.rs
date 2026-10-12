//! Channel point rewards: whether the viewer can redeem each one right now and,
//! when not, why, in words the menu shows as-is.
//!
//! Some verdicts depend on things that change while the menu is open: chat
//! going in or out of sub-only mode (the bypass reward only works in it), the
//! viewer's subscriber badge, and the stream going live or offline (rewards
//! with a per-stream limit only work live). Rust tracks those from the IRC room
//! state and the channel-state poll, keeps the last list it judged per channel,
//! and re-judges it on a change, pushing the result as `channel-rewards`. No
//! refetch from Twitch is needed for that.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use log::debug;
use tauri::Emitter;

use crate::models::drops::{ChannelReward, ChannelRewardsSnapshot, RedemptionResult, RewardAvailability, RewardBlock};

pub const EVENT: &str = "channel-rewards";

/// The built-in reward whose text input the menu sends itself.
pub const HIGHLIGHTED_MESSAGE_REWARD: &str = "SEND_HIGHLIGHTED_MESSAGE";
/// The built-in reward that lets a non-subscriber send one message while chat
/// is in sub-only mode.
pub const SUB_MODE_BYPASS_REWARD: &str = "SINGLE_MESSAGE_BYPASS_SUB_MODE";

/// What a verdict is judged against besides the reward itself. Every `None`
/// means unknown, and unknown never blocks on its own (Twitch still refuses a
/// redeem it will not take), except for the sub-only bypass, which only works
/// once chat is known to be in sub-only mode.
#[derive(Clone, Debug, Default)]
pub struct RewardContext {
    pub balance: Option<i64>,
    /// The channel's own name for its points ("Coins"); "points" when unset.
    pub points_name: Option<String>,
    pub subs_only: Option<bool>,
    pub viewer_is_sub: Option<bool>,
    /// The broadcaster, a moderator or a VIP: sub-only mode does not stop them
    /// chatting, so the bypass reward would spend points for nothing.
    pub viewer_privileged: Option<bool>,
    pub live: Option<bool>,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Room {
    subs_only: Option<bool>,
    viewer_is_sub: Option<bool>,
    viewer_privileged: Option<bool>,
}

/// The last list fetched for a channel, kept so a room or stream change can be
/// judged again without asking Twitch.
#[derive(Clone)]
struct Judged {
    rewards: Vec<ChannelReward>,
    balance: Option<i64>,
    points_name: Option<String>,
    fetched_at_ms: i64,
}

static ROOMS: OnceLock<Mutex<HashMap<String, Room>>> = OnceLock::new();
static LISTS: OnceLock<Mutex<HashMap<String, Judged>>> = OnceLock::new();

fn rooms() -> std::sync::MutexGuard<'static, HashMap<String, Room>> {
    ROOMS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn lists() -> std::sync::MutexGuard<'static, HashMap<String, Judged>> {
    LISTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Update one channel's room facts; true when something changed.
fn update_room(login: &str, f: impl FnOnce(&mut Room)) -> bool {
    let mut rooms = rooms();
    let room = rooms.entry(login.to_lowercase()).or_default();
    let before = *room;
    f(room);
    *room != before
}

/// A ROOMSTATE carried the `subs-only` tag for `login` (bare Twitch login).
pub fn note_subs_only(login: &str, on: bool) {
    if update_room(login, |r| r.subs_only = Some(on)) {
        rejudge(login);
    }
}

/// A USERSTATE carried the viewer's badges in `login`. Founders count as
/// subscribers, as they do on Twitch. The broadcaster, moderators and VIPs
/// chat through sub-only mode as they are, the same rule the send limiter uses.
pub fn note_viewer_badges(login: &str, badges: &str) {
    let is_sub = badges
        .split(',')
        .any(|b| b.starts_with("subscriber/") || b.starts_with("founder/"));
    let privileged = crate::services::chat_send_limiter::Role::from_badges(badges).privileged();
    if update_room(login, |r| {
        r.viewer_is_sub = Some(is_sub);
        r.viewer_privileged = Some(privileged);
    }) {
        rejudge(login);
    }
}

/// The channel went live or offline, or the channel-state poll answered for it
/// the first time.
pub fn note_live_changed(login: &str) {
    rejudge(login);
}

/// IRC left the channel: its room facts are no longer current and its kept
/// list is dropped. A menu still showing that list is told the room is unknown.
pub fn forget_room(login: &str) {
    let login = login.to_lowercase();
    rooms().remove(&login);
    let Some(judged) = lists().remove(&login) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let live = crate::services::channel_state::is_live(&login).await;
        push(&login, judged, Room::default(), live);
    });
}

/// IRC restarted: no room fact is current. Open lists are re-judged now, and
/// again as each channel's ROOMSTATE and USERSTATE come back.
pub fn forget_all_rooms() {
    rooms().clear();
    let logins: Vec<String> = lists().keys().cloned().collect();
    for login in logins {
        rejudge(&login);
    }
}

fn room_of(login: &str) -> Room {
    rooms().get(&login.to_lowercase()).copied().unwrap_or_default()
}

/// The channel's own name for its points, from the last list fetched for it.
pub fn points_name_of(login: &str) -> Option<String> {
    lists().get(&login.to_lowercase()).and_then(|j| j.points_name.clone())
}

fn context(judged: &Judged, room: Room, live: Option<bool>) -> RewardContext {
    RewardContext {
        balance: judged.balance,
        points_name: judged.points_name.clone(),
        subs_only: room.subs_only,
        viewer_is_sub: room.viewer_is_sub,
        viewer_privileged: room.viewer_privileged,
        live,
    }
}

/// Judge a freshly fetched list for `login`, remember it for later re-judging,
/// and return what the menu shows.
pub async fn judge_fetched(
    login: &str,
    rewards: Vec<ChannelReward>,
    balance: Option<i64>,
    points_name: Option<String>,
    fetched_at_ms: i64,
) -> ChannelRewardsSnapshot {
    let login = login.to_lowercase();
    let live = crate::services::channel_state::is_live(&login).await;
    let judged = Judged { rewards, balance, points_name, fetched_at_ms };
    lists().insert(login.clone(), judged.clone());
    let ctx = context(&judged, room_of(&login), live);
    snapshot(&login, judged.rewards, &ctx, fetched_at_ms, chrono::Utc::now().timestamp_millis())
}

/// Re-judge the last list for `login` against the current room and stream and
/// push it. A no-op for a channel whose rewards were never opened.
fn rejudge(login: &str) {
    let login = login.to_lowercase();
    if !lists().contains_key(&login) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let live = crate::services::channel_state::is_live(&login).await;
        // Read after the wait, so the newest room facts are the ones judged.
        let Some(judged) = lists().get(&login).cloned() else {
            return;
        };
        push(&login, judged, room_of(&login), live);
    });
}

/// Judge `judged` against `room` and send it to the menus as `channel-rewards`.
fn push(login: &str, judged: Judged, room: Room, live: Option<bool>) {
    let ctx = context(&judged, room, live);
    let snap = snapshot(login, judged.rewards, &ctx, judged.fetched_at_ms, chrono::Utc::now().timestamp_millis());
    if let Some(app) = crate::services::providers::app_handle() {
        if let Err(e) = app.emit(EVENT, &snap) {
            debug!("[ChannelRewards] emit failed: {e}");
        }
    }
}

/// The list as the viewer should see it: rewards Twitch hides from
/// subscribers are left out for one, and every reward carries its verdict.
fn snapshot(
    login: &str,
    rewards: Vec<ChannelReward>,
    ctx: &RewardContext,
    fetched_at_ms: i64,
    now_ms: i64,
) -> ChannelRewardsSnapshot {
    let rewards = rewards
        .into_iter()
        .filter(|r| !(r.is_hidden_for_subs && ctx.viewer_is_sub == Some(true)))
        .map(|mut r| {
            r.availability = reward_availability(&r, ctx, now_ms);
            r
        })
        .collect();
    ChannelRewardsSnapshot {
        login: login.to_string(),
        rewards,
        balance: ctx.balance,
        fetched_at_ms,
    }
}

/// Decide whether the viewer can redeem `reward` right now. `now_ms` is Unix
/// ms. Earlier checks win when several apply, in Twitch's checkout order.
pub fn reward_availability(reward: &ChannelReward, ctx: &RewardContext, now_ms: i64) -> RewardAvailability {
    let cooldown_until_ms = reward
        .cooldown_expires_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis())
        .filter(|&until| until > now_ms);
    let affordable = ctx.balance.map_or(true, |b| b >= i64::from(reward.cost));
    let is_bypass = reward.reward_type.as_deref() == Some(SUB_MODE_BYPASS_REWARD);
    let stream_cap_reached = matches!(
        (reward.max_per_stream, reward.redemptions_redeemed_current_stream),
        (Some(max), Some(done)) if max > 0 && done >= max
    );
    // Twitch refuses a reward with a per-stream limit while the channel is
    // offline, since there is no stream to count against.
    let has_stream_limit = reward.max_per_stream.is_some() || reward.max_per_user_per_stream.is_some();
    // Twitch turns `isInStock` off while a reward's global cooldown runs and
    // leaves it off after the cooldown ends, so for a reward with a cooldown
    // timestamp the stock flag only restates the cooldown (judged above from
    // the timestamp itself). Twitch's own client reads it the same way.
    let stock_held_by_cooldown =
        reward.global_cooldown_seconds.is_some() && reward.cooldown_expires_at.is_some();
    let out_of_stock = !reward.is_in_stock && !stock_held_by_cooldown;

    let reason = if !reward.is_enabled {
        Some(RewardBlock::Disabled)
    } else if reward.is_paused {
        Some(RewardBlock::Paused)
    } else if is_bypass && ctx.viewer_privileged == Some(true) {
        Some(RewardBlock::AlreadyCanChat)
    } else if is_bypass && ctx.subs_only.is_none() {
        Some(RewardBlock::ChatReconnecting)
    } else if is_bypass && ctx.subs_only != Some(true) {
        Some(RewardBlock::NeedsSubOnlyMode)
    } else if reward.is_sub_only && ctx.viewer_is_sub == Some(false) {
        Some(RewardBlock::SubscribersOnly)
    } else if has_stream_limit && ctx.live == Some(false) {
        Some(RewardBlock::OnlyWhileLive)
    } else if stream_cap_reached {
        Some(RewardBlock::MaxPerStreamReached)
    } else if cooldown_until_ms.is_some() {
        Some(RewardBlock::OnCooldown)
    } else if out_of_stock {
        Some(RewardBlock::OutOfStock)
    } else if !affordable {
        Some(RewardBlock::NotEnoughPoints)
    } else {
        None
    };

    let (label, detail) = match reason {
        Some(block) => {
            let (label, detail) = words(block, reward, ctx);
            (Some(label), Some(detail))
        }
        None => (None, None),
    };
    RewardAvailability {
        redeemable: reason.is_none(),
        reason,
        cooldown_until_ms,
        affordable,
        label,
        detail,
    }
}

/// The row label and the detail sentence for a blocked reward, spoken to the
/// viewer. A running cooldown's time left is added by the view, which ticks.
/// Each says only what the reward's title does not: the title already names
/// what the reward is, so the words carry the current state.
fn words(block: RewardBlock, reward: &ChannelReward, ctx: &RewardContext) -> (String, String) {
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    let shared = |(a, b): (&str, &str)| (a.to_string(), b.to_string());
    match block {
        RewardBlock::Disabled => pair("Off", "Turned off by the streamer"),
        RewardBlock::Paused => pair("Paused", "Paused by the streamer"),
        RewardBlock::AlreadyCanChat => shared(ALREADY_CAN_CHAT),
        RewardBlock::ChatReconnecting => shared(CHAT_RECONNECTING),
        RewardBlock::NeedsSubOnlyMode => shared(NOT_SUB_ONLY),
        RewardBlock::SubscribersOnly => pair("Subscribers only", "Only subscribers can redeem this"),
        RewardBlock::OnlyWhileLive => pair("Live only", "Opens when the stream goes live"),
        RewardBlock::MaxPerStreamReached => pair("Sold out", "Sold out for this stream"),
        RewardBlock::OnCooldown => pair("Cooldown", "On cooldown"),
        RewardBlock::OutOfStock => pair("Out of stock", "Out of stock right now"),
        RewardBlock::NotEnoughPoints => {
            let cost = i64::from(reward.cost);
            let have = ctx.balance.unwrap_or(0);
            let name = points_word(ctx.points_name.as_deref());
            (
                format!("Need {} more", group(cost - have)),
                format!("Not enough {name} (need {}, you have {})", group(cost), group(have)),
            )
        }
    }
}

/// The bypass reward's words, shared by its verdict and the send's refusal.
const NOT_SUB_ONLY: (&str, &str) = ("Chat isn't sub-only", "Chat isn't in sub-only mode right now");
const ALREADY_CAN_CHAT: (&str, &str) = ("You can already chat", "Your role lets you chat in sub-only mode");
const CHAT_RECONNECTING: (&str, &str) = ("Chat reconnecting", "Chat is reconnecting. Try again in a moment");

/// The channel's name for its points, or "points".
fn points_word(points_name: Option<&str>) -> &str {
    points_name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or("points")
}

/// `1234567` as "1,234,567".
fn group(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A line for a built-in reward's detail view, only where it adds something
/// its title does not say. Titles like "Gigantify an Emote" need none.
pub fn built_in_description(reward_type: &str) -> Option<&'static str> {
    Some(match reward_type {
        HIGHLIGHTED_MESSAGE_REWARD => "Everyone in chat sees it in a colored box",
        SUB_MODE_BYPASS_REWARD => "Posts even though you're not subscribed",
        "CHOSEN_MODIFIED_SUB_EMOTE_UNLOCK" => "Effects include black and white, flipped and sunglasses",
        _ => return None,
    })
}

/// Twitch's own document for the bypass send, as its web client writes it
/// (`SendSubsOnlyMessage`). Sent inline, like the chosen-emote unlock, so it
/// does not depend on a persisted hash.
pub const SUB_MODE_MESSAGE_QUERY: &str = "mutation SendSubsOnlyMessage($input: SendChatMessageThroughSubscriberModeInput!) {
  sendChatMessageThroughSubscriberMode(input: $input) {
    balance
    error {
      code
    }
  }
}";

/// Twitch's limit for one chat message.
pub const CHAT_MESSAGE_MAX: usize = 500;

/// Why the bypass message cannot go out in `login` right now, as an error code
/// and the viewer's words; None when chat is known to be sub-only and the
/// viewer needs the reward to talk in it. A stale menu never spends points.
pub fn sub_mode_send_refusal(login: &str) -> Option<(&'static str, &'static str)> {
    let room = room_of(login);
    if room.viewer_privileged == Some(true) {
        return Some(("ALREADY_CAN_CHAT", ALREADY_CAN_CHAT.1));
    }
    if room.viewer_is_sub == Some(true) {
        return Some(("ALREADY_CAN_CHAT", "Your subscription lets you chat in sub-only mode"));
    }
    match room.subs_only {
        None => Some(("ROOM_UNKNOWN", CHAT_RECONNECTING.1)),
        Some(false) => Some(("SUBS_ONLY_OFF", NOT_SUB_ONLY.1)),
        Some(true) => None,
    }
}

/// The id Twitch uses to recognise a retry of the same redeem. The menu keeps
/// one per message, so a retry after a lost answer is not sent and charged
/// twice; a missing or malformed id gets a fresh one.
pub fn transaction_id_or_new(given: Option<String>) -> String {
    given
        .map(|id| id.trim().to_string())
        .filter(|id| (16..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string())
}

/// The GQL body for sending `message` through sub-only mode for `cost`
/// points. `channel_id` is the numeric channel id.
pub fn sub_mode_message_request(channel_id: &str, message: &str, cost: i32, transaction_id: &str) -> serde_json::Value {
    serde_json::json!({
        "operationName": "SendSubsOnlyMessage",
        "query": SUB_MODE_MESSAGE_QUERY,
        "variables": {
            "input": {
                "channelID": channel_id,
                "cost": cost,
                "message": message,
                "transactionID": transaction_id,
            }
        }
    })
}

/// What the viewer reads when Twitch no longer accepts the reward token.
pub const SIGNED_OUT_TEXT: &str = "Twitch didn't accept your sign-in. Sign in to Twitch again and retry";

/// The code Twitch answers when a transaction id was already spent: the
/// earlier attempt went through, so a retry reads as sent.
pub const ALREADY_COMMITTED: &str = "TRANSACTION_ALREADY_COMMITTED";

/// Read Twitch's answer to a bypass send: the HTTP status, the parsed body
/// (Null when it was not JSON) and the channel's name for its points.
/// Twitch's own error text is for the log, never the toast.
pub fn sub_mode_message_outcome(status: u16, resp: &serde_json::Value, points_name: Option<&str>) -> RedemptionResult {
    let fail = |code: &str, text: String| RedemptionResult {
        success: false,
        error_code: Some(code.to_string()),
        error_message: Some(text),
        new_balance: None,
        unlocked_emote: None,
    };
    // An expired or revoked token answers 401 with Twitch's own error object.
    if status == 401 || resp["status"].as_u64() == Some(401) {
        return fail("UNAUTHORIZED", SIGNED_OUT_TEXT.to_string());
    }
    if resp["errors"].as_array().is_some_and(|e| !e.is_empty()) {
        return fail("GQL_ERROR", "Twitch couldn't take the message right now. Try again in a moment".to_string());
    }
    let payload = &resp["data"]["sendChatMessageThroughSubscriberMode"];
    if !payload.is_object() {
        return fail("UNKNOWN", "Unexpected answer from Twitch".to_string());
    }
    let sent = |balance: Option<i32>| RedemptionResult {
        success: true,
        error_code: None,
        error_message: None,
        new_balance: balance,
        unlocked_emote: None,
    };
    if let Some(code) = payload["error"]["code"].as_str() {
        if code == ALREADY_COMMITTED {
            return sent(None);
        }
        return fail(code, sub_mode_error_text(code, points_name));
    }
    if payload["error"].is_object() {
        return fail("UNKNOWN", sub_mode_error_text("UNKNOWN", points_name));
    }
    sent(payload["balance"].as_i64().map(|b| b as i32))
}

/// The viewer-facing words for an error code of Twitch's chat-message reward
/// mutations (the enum the web client shares between this and Highlight My
/// Message). `points_name` is the channel's name for its points.
pub fn sub_mode_error_text(code: &str, points_name: Option<&str>) -> String {
    if code == "INSUFFICIENT_POINTS" {
        return format!("You don't have enough {} for this", points_word(points_name));
    }
    let text = match code {
        "SUBS_ONLY" => "Chat's sub-only setting blocked the message",
        "CHANNEL_SETTINGS" => "Your message conflicts with this channel's moderation settings",
        "AUTOMOD_HELD" => "AutoMod held your message",
        "MESSAGE_REJECTED" => "Twitch rejected the message",
        "MSG_DUPLICATE" => "You just sent that. Try something different",
        "MESSAGE_IS_COMMAND" => "Commands can't be sent this way",
        "INVALID_EMOTE" => "Your message has an emote you can't use here",
        "EMOTE_ONLY" => "Chat is in emote-only mode",
        "FOLLOWERS_ONLY" | "FOLLOWERS_ONLY_ZERO" => "Chat is in followers-only mode",
        "R9K_MODE" => "Chat is in unique-chat mode and that message isn't unique",
        "SLOW_MODE" => "Chat is in slow mode. Wait a moment and try again",
        "BLIZZARD_CONNECT_MODE" | "VERIFIED_ACCOUNT" => "This chat needs a verified email or phone number",
        "USER_BANNED" => "You're banned from this chat",
        "USER_TIMED_OUT" => "You're timed out in this chat",
        "USER_SUSPENDED" => "Your account is suspended",
        "USER_HAS_CHAT_CONSEQUENCE" | "RESTRICTED_SUSPICIOUS_USER" => "You can't chat here right now",
        "REWARD_DISABLED" => "Turned off by the streamer",
        "REWARD_FORBIDDEN" => "You can't redeem this reward on this channel",
        "REWARD_COST_MISMATCH" => "The price changed. Reopen rewards and try again",
        "REWARD_GLOBAL_COOLDOWN" => "On cooldown. Try again soon",
        "REWARD_MAX_PER_STREAM" | "REWARD_MAX_PER_USER_PER_STREAM" => "Sold out for this stream",
        "STREAM_NOT_LIVE" => "Opens when the stream goes live",
        "TRANSACTION_ALREADY_COMMITTED" => "Already sent",
        "TRANSACTION_IN_PROGRESS" => "Still sending. Give it a moment",
        _ => "Something went wrong on Twitch. Try again later",
    };
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW_MS: i64 = 1_700_000_000_000;

    fn reward() -> ChannelReward {
        ChannelReward {
            id: "r1".to_string(),
            title: "Hydrate".to_string(),
            cost: 500,
            prompt: None,
            image_url: None,
            background_color: "#9147FF".to_string(),
            is_enabled: true,
            is_paused: false,
            is_in_stock: true,
            is_user_input_required: false,
            cooldown_expires_at: None,
            max_per_stream: None,
            max_per_user_per_stream: None,
            global_cooldown_seconds: None,
            reward_type: None,
            redemptions_redeemed_current_stream: None,
            is_sub_only: false,
            is_hidden_for_subs: false,
            description: None,
            availability: RewardAvailability::default(),
        }
    }

    fn ctx(balance: Option<i64>) -> RewardContext {
        RewardContext { balance, ..Default::default() }
    }

    fn judge(r: &ChannelReward, balance: Option<i64>, now: i64) -> RewardAvailability {
        reward_availability(r, &ctx(balance), now)
    }

    fn bypass() -> ChannelReward {
        let mut r = reward();
        r.title = "Send a Message in Sub-Only Mode".to_string();
        r.cost = 3_000;
        r.is_user_input_required = true;
        r.is_hidden_for_subs = true;
        r.reward_type = Some(SUB_MODE_BYPASS_REWARD.to_string());
        r
    }

    fn iso(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339()
    }

    #[test]
    fn an_open_affordable_reward_is_redeemable() {
        let a = judge(&reward(), Some(1_000), NOW_MS);
        assert!(a.redeemable);
        assert_eq!(a.reason, None);
        assert!(a.affordable);
        assert_eq!(a.cooldown_until_ms, None);
        assert_eq!(a.label, None);
        assert_eq!(a.detail, None);
    }

    #[test]
    fn an_unknown_balance_never_blocks() {
        let a = judge(&reward(), None, NOW_MS);
        assert!(a.redeemable);
        assert!(a.affordable);
    }

    #[test]
    fn too_few_points_says_how_many() {
        let mut r = reward();
        r.cost = 3_000;
        let a = judge(&r, Some(1_250), NOW_MS);
        assert!(!a.redeemable);
        assert_eq!(a.reason, Some(RewardBlock::NotEnoughPoints));
        assert!(!a.affordable);
        assert_eq!(a.label.as_deref(), Some("Need 1,750 more"));
        assert_eq!(a.detail.as_deref(), Some("Not enough points (need 3,000, you have 1,250)"));
        assert!(judge(&r, Some(3_000), NOW_MS).redeemable);
    }

    #[test]
    fn too_few_points_uses_the_channel_points_name() {
        let mut r = reward();
        r.cost = 3_000;
        let c = RewardContext { balance: Some(10), points_name: Some("Coins".into()), ..Default::default() };
        assert_eq!(
            reward_availability(&r, &c, NOW_MS).detail.as_deref(),
            Some("Not enough Coins (need 3,000, you have 10)")
        );
    }

    #[test]
    fn disabled_paused_and_out_of_stock_block_in_that_order() {
        let mut r = reward();
        r.is_in_stock = false;
        assert_eq!(judge(&r, Some(1_000), NOW_MS).reason, Some(RewardBlock::OutOfStock));
        r.is_paused = true;
        let paused = judge(&r, Some(1_000), NOW_MS);
        assert_eq!(paused.reason, Some(RewardBlock::Paused));
        assert_eq!(paused.detail.as_deref(), Some("Paused by the streamer"));
        r.is_enabled = false;
        assert_eq!(judge(&r, Some(1_000), NOW_MS).reason, Some(RewardBlock::Disabled));
    }

    #[test]
    fn a_running_cooldown_blocks_until_it_ends() {
        let mut r = reward();
        r.cooldown_expires_at = Some(iso(NOW_MS + 84_000));
        let a = judge(&r, Some(1_000), NOW_MS);
        assert_eq!(a.reason, Some(RewardBlock::OnCooldown));
        assert_eq!(a.cooldown_until_ms, Some(NOW_MS + 84_000));
        assert_eq!(a.label.as_deref(), Some("Cooldown"));

        let later = judge(&r, Some(1_000), NOW_MS + 84_000);
        assert!(later.redeemable);
        assert_eq!(later.cooldown_until_ms, None);
    }

    #[test]
    fn twitch_nanosecond_timestamps_parse() {
        let mut r = reward();
        r.cooldown_expires_at = Some("2023-11-14T22:14:20.462676805Z".to_string());
        let a = judge(&r, Some(1_000), NOW_MS);
        assert_eq!(a.reason, Some(RewardBlock::OnCooldown));
        assert_eq!(a.cooldown_until_ms, Some(1_700_000_060_462));
    }

    #[test]
    fn a_cooldown_outranks_too_few_points() {
        let mut r = reward();
        r.cooldown_expires_at = Some(iso(NOW_MS + 1_000));
        let a = judge(&r, Some(0), NOW_MS);
        assert_eq!(a.reason, Some(RewardBlock::OnCooldown));
        assert!(!a.affordable);
    }

    #[test]
    fn a_cooldown_is_not_out_of_stock() {
        let mut r = reward();
        r.is_in_stock = false;
        r.global_cooldown_seconds = Some(60);
        r.cooldown_expires_at = Some(iso(NOW_MS + 30_000));
        assert_eq!(judge(&r, Some(1_000), NOW_MS).reason, Some(RewardBlock::OnCooldown));
        // Twitch still says out of stock once the cooldown has passed.
        assert!(judge(&r, Some(1_000), NOW_MS + 30_000).redeemable);
    }

    #[test]
    fn out_of_stock_without_a_cooldown_still_blocks() {
        let mut r = reward();
        r.is_in_stock = false;
        r.cooldown_expires_at = Some(iso(NOW_MS - 1_000));
        assert_eq!(judge(&r, Some(1_000), NOW_MS).reason, Some(RewardBlock::OutOfStock));
    }

    #[test]
    fn custom_input_rewards_are_supported() {
        let mut r = reward();
        r.is_user_input_required = true;
        assert!(judge(&r, Some(1_000), NOW_MS).redeemable);
    }

    #[test]
    fn highlight_my_message_is_supported() {
        let mut r = reward();
        r.is_user_input_required = true;
        r.reward_type = Some(HIGHLIGHTED_MESSAGE_REWARD.to_string());
        assert!(judge(&r, Some(1_000), NOW_MS).redeemable);
    }

    #[test]
    fn the_bypass_reward_needs_sub_only_mode() {
        let r = bypass();
        let off = RewardContext { balance: Some(5_000), subs_only: Some(false), ..Default::default() };
        let a = reward_availability(&r, &off, NOW_MS);
        assert_eq!(a.reason, Some(RewardBlock::NeedsSubOnlyMode));
        assert_eq!(a.label.as_deref(), Some("Chat isn't sub-only"));
        assert_eq!(a.detail.as_deref(), Some("Chat isn't in sub-only mode right now"));
        // Unknown room state (chat reconnecting) never reads as redeemable.
        let unknown = reward_availability(&r, &ctx(Some(5_000)), NOW_MS);
        assert_eq!(unknown.reason, Some(RewardBlock::ChatReconnecting));
        assert_eq!(unknown.detail.as_deref(), Some("Chat is reconnecting. Try again in a moment"));
    }

    #[test]
    fn the_broadcaster_mods_and_vips_already_chat_in_sub_only_mode() {
        let r = bypass();
        let mod_view = RewardContext {
            balance: Some(5_000),
            subs_only: Some(true),
            viewer_is_sub: Some(false),
            viewer_privileged: Some(true),
            ..Default::default()
        };
        let a = reward_availability(&r, &mod_view, NOW_MS);
        assert!(!a.redeemable);
        assert_eq!(a.reason, Some(RewardBlock::AlreadyCanChat));
        assert_eq!(a.label.as_deref(), Some("You can already chat"));
        // The role only matters for the bypass; other rewards stay open.
        assert!(reward_availability(&reward(), &mod_view, NOW_MS).redeemable);
        // A plain viewer in sub-only mode can still use it.
        let viewer = RewardContext { viewer_privileged: Some(false), ..mod_view };
        assert!(reward_availability(&r, &viewer, NOW_MS).redeemable);
    }

    #[test]
    fn roles_come_from_the_badges() {
        note_viewer_badges("roletest", "broadcaster/1");
        assert_eq!(room_of("roletest").viewer_privileged, Some(true));
        note_viewer_badges("roletest", "vip/1");
        assert_eq!(room_of("roletest").viewer_privileged, Some(true));
        note_viewer_badges("roletest", "premium/1");
        assert_eq!(room_of("roletest").viewer_privileged, Some(false));
        forget_room("roletest");
    }

    #[test]
    fn the_bypass_send_refuses_what_its_verdict_refuses() {
        let login = "sendrefusaltest";
        forget_room(login);
        assert_eq!(sub_mode_send_refusal(login).map(|r| r.0), Some("ROOM_UNKNOWN"));
        note_subs_only(login, false);
        assert_eq!(sub_mode_send_refusal(login).map(|r| r.0), Some("SUBS_ONLY_OFF"));
        note_subs_only(login, true);
        note_viewer_badges(login, "premium/1");
        assert_eq!(sub_mode_send_refusal(login), None);
        note_viewer_badges(login, "moderator/1");
        assert_eq!(sub_mode_send_refusal(login).map(|r| r.0), Some("ALREADY_CAN_CHAT"));
        note_viewer_badges(login, "subscriber/6");
        assert_eq!(sub_mode_send_refusal(login).map(|r| r.0), Some("ALREADY_CAN_CHAT"));
        forget_room(login);
    }

    #[test]
    fn leaving_a_channel_drops_its_kept_list() {
        let login = "forgetlisttest";
        lists().insert(
            login.to_string(),
            Judged { rewards: vec![reward()], balance: Some(10), points_name: Some("Coins".into()), fetched_at_ms: NOW_MS },
        );
        assert_eq!(points_name_of(login).as_deref(), Some("Coins"));
        forget_room(login);
        assert!(!lists().contains_key(login));
        assert_eq!(points_name_of(login), None);
    }

    #[test]
    fn transaction_ids_are_kept_when_well_formed() {
        let id = "0123456789abcdef0123456789abcdef".to_string();
        assert_eq!(transaction_id_or_new(Some(id.clone())), id);
        for bad in [None, Some(String::new()), Some("short".into()), Some("has spaces in it 0123456789".into())] {
            let fresh = transaction_id_or_new(bad);
            assert_eq!(fresh.len(), 32);
            assert!(fresh.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert_ne!(transaction_id_or_new(None), transaction_id_or_new(None));
    }

    #[test]
    fn the_bypass_reward_is_redeemable_in_sub_only_mode() {
        let r = bypass();
        let on = RewardContext { balance: Some(5_000), subs_only: Some(true), ..Default::default() };
        let a = reward_availability(&r, &on, NOW_MS);
        assert!(a.redeemable);
        assert_eq!(a.detail, None);
        // Points still count once the mode allows it.
        let poor = RewardContext { balance: Some(100), ..on };
        assert_eq!(reward_availability(&r, &poor, NOW_MS).reason, Some(RewardBlock::NotEnoughPoints));
    }

    #[test]
    fn no_reason_or_description_restates_its_title() {
        for t in [
            HIGHLIGHTED_MESSAGE_REWARD,
            SUB_MODE_BYPASS_REWARD,
            "RANDOM_SUB_EMOTE_UNLOCK",
            "CHOSEN_SUB_EMOTE_UNLOCK",
            "CHOSEN_MODIFIED_SUB_EMOTE_UNLOCK",
            "SEND_GIGANTIFIED_EMOTE",
        ] {
            if let Some(d) = built_in_description(t) {
                let d = d.to_lowercase();
                assert!(!d.contains("sub-only"), "{t}: {d}");
                assert!(!d.contains("unlock"), "{t}: {d}");
                assert!(!d.contains("highlight"), "{t}: {d}");
            }
        }
        let off = RewardContext { subs_only: Some(false), ..Default::default() };
        let a = reward_availability(&bypass(), &off, NOW_MS);
        assert_eq!(a.detail.as_deref().map(|d| d.matches("sub-only").count()), Some(1));
    }

    #[test]
    fn the_sub_mode_request_carries_input_and_document() {
        let body = sub_mode_message_request("71092938", "hi chat", 3_000, "abc123");
        assert_eq!(body["operationName"], "SendSubsOnlyMessage");
        assert_eq!(
            body["variables"]["input"],
            serde_json::json!({
                "channelID": "71092938",
                "cost": 3000,
                "message": "hi chat",
                "transactionID": "abc123"
            })
        );
        let q = body["query"].as_str().unwrap();
        assert!(q.contains("SendChatMessageThroughSubscriberModeInput!"));
        assert!(q.contains("sendChatMessageThroughSubscriberMode(input: $input)"));
        assert!(body.get("extensions").is_none());
    }

    #[test]
    fn a_sub_mode_send_succeeds_with_the_new_balance() {
        let resp = serde_json::json!({ "data": { "sendChatMessageThroughSubscriberMode": {
            "balance": 9_250, "error": null, "__typename": "SendChatMessageThroughSubscriberModePayload"
        } } });
        let r = sub_mode_message_outcome(200, &resp, None);
        assert!(r.success);
        assert_eq!(r.new_balance, Some(9_250));
        assert_eq!(r.error_code, None);
    }

    #[test]
    fn a_retried_transaction_that_already_went_through_reads_as_sent() {
        let resp = serde_json::json!({ "data": { "sendChatMessageThroughSubscriberMode": {
            "balance": null, "error": { "code": "TRANSACTION_ALREADY_COMMITTED" }
        } } });
        let r = sub_mode_message_outcome(200, &resp, None);
        assert!(r.success);
        assert_eq!(r.error_code, None);
        assert_eq!(r.error_message, None);
    }

    #[test]
    fn an_expired_token_asks_for_a_new_sign_in() {
        // Twitch's 401 body for a revoked or expired OAuth token.
        let body = r#"{"error":"Unauthorized","status":401,"message":"The \"Authorization\" token is invalid."}"#;
        let resp: serde_json::Value = serde_json::from_str(body).unwrap();
        let r = sub_mode_message_outcome(401, &resp, None);
        assert!(!r.success);
        assert_eq!(r.error_code.as_deref(), Some("UNAUTHORIZED"));
        assert_eq!(r.error_message.as_deref(), Some(SIGNED_OUT_TEXT));
        // The body alone says so too, whatever status a proxy passed on.
        assert_eq!(sub_mode_message_outcome(200, &resp, None).error_code.as_deref(), Some("UNAUTHORIZED"));
        // A body that was not JSON at all.
        assert_eq!(sub_mode_message_outcome(401, &serde_json::Value::Null, None).error_code.as_deref(), Some("UNAUTHORIZED"));
        assert_eq!(sub_mode_message_outcome(502, &serde_json::Value::Null, None).error_code.as_deref(), Some("UNKNOWN"));
    }

    #[test]
    fn too_few_points_names_the_channel_points() {
        let resp = serde_json::json!({ "data": { "sendChatMessageThroughSubscriberMode": {
            "balance": null, "error": { "code": "INSUFFICIENT_POINTS" }
        } } });
        let r = sub_mode_message_outcome(200, &resp, Some("Coins"));
        assert_eq!(r.error_message.as_deref(), Some("You don't have enough Coins for this"));
    }

    #[test]
    fn sub_mode_send_errors_read_as_words() {
        let err = |code: &str| {
            serde_json::json!({ "data": { "sendChatMessageThroughSubscriberMode": {
                "balance": null, "error": { "code": code, "__typename": "SendChatMessageThroughSubscriberModeError" }
            } } })
        };
        let cases = [
            ("INSUFFICIENT_POINTS", "You don't have enough points for this"),
            ("SUBS_ONLY", "Chat's sub-only setting blocked the message"),
            ("MESSAGE_REJECTED", "Twitch rejected the message"),
            ("MSG_DUPLICATE", "You just sent that. Try something different"),
            ("SLOW_MODE", "Chat is in slow mode. Wait a moment and try again"),
            ("USER_BANNED", "You're banned from this chat"),
            ("REWARD_COST_MISMATCH", "The price changed. Reopen rewards and try again"),
            ("SOMETHING_NEW", "Something went wrong on Twitch. Try again later"),
        ];
        for (code, text) in cases {
            let r = sub_mode_message_outcome(200, &err(code), None);
            assert!(!r.success, "{code}");
            assert_eq!(r.error_code.as_deref(), Some(code));
            assert_eq!(r.error_message.as_deref(), Some(text), "{code}");
        }
    }

    #[test]
    fn sub_mode_send_gql_errors_and_odd_answers_fail() {
        let gql = serde_json::json!({ "errors": [{ "message": "failed integrity check" }], "data": null });
        let r = sub_mode_message_outcome(200, &gql, None);
        assert!(!r.success);
        assert_eq!(r.error_code.as_deref(), Some("GQL_ERROR"));
        // Twitch's own text stays in the log.
        let text = r.error_message.unwrap();
        assert!(!text.contains("integrity"), "{text}");
        assert_eq!(text, "Twitch couldn't take the message right now. Try again in a moment");

        let empty = serde_json::json!({ "data": { "sendChatMessageThroughSubscriberMode": null } });
        assert_eq!(sub_mode_message_outcome(200, &empty, None).error_code.as_deref(), Some("UNKNOWN"));
    }

    #[test]
    fn a_sub_only_reward_blocks_a_known_non_sub() {
        let mut r = reward();
        r.is_sub_only = true;
        let non_sub = RewardContext { balance: Some(1_000), viewer_is_sub: Some(false), ..Default::default() };
        let a = reward_availability(&r, &non_sub, NOW_MS);
        assert_eq!(a.reason, Some(RewardBlock::SubscribersOnly));
        assert_eq!(a.label.as_deref(), Some("Subscribers only"));
        let sub = RewardContext { viewer_is_sub: Some(true), ..non_sub.clone() };
        assert!(reward_availability(&r, &sub, NOW_MS).redeemable);
        // Unknown subscription never blocks.
        assert!(judge(&r, Some(1_000), NOW_MS).redeemable);
    }

    #[test]
    fn the_per_stream_cap_reads_sold_out() {
        let mut r = reward();
        r.max_per_stream = Some(3);
        r.redemptions_redeemed_current_stream = Some(2);
        assert!(judge(&r, Some(1_000), NOW_MS).redeemable);
        r.redemptions_redeemed_current_stream = Some(3);
        let a = judge(&r, Some(1_000), NOW_MS);
        assert_eq!(a.reason, Some(RewardBlock::MaxPerStreamReached));
        assert_eq!(a.detail.as_deref(), Some("Sold out for this stream"));
        r.max_per_stream = None;
        assert!(judge(&r, Some(1_000), NOW_MS).redeemable);
    }

    #[test]
    fn a_limited_reward_only_works_live() {
        let mut r = reward();
        r.max_per_user_per_stream = Some(1);
        let offline = RewardContext { balance: Some(1_000), live: Some(false), ..Default::default() };
        assert_eq!(reward_availability(&r, &offline, NOW_MS).reason, Some(RewardBlock::OnlyWhileLive));
        let live = RewardContext { live: Some(true), ..offline.clone() };
        assert!(reward_availability(&r, &live, NOW_MS).redeemable);
        // An unlimited reward works offline.
        assert!(reward_availability(&reward(), &offline, NOW_MS).redeemable);
    }

    #[test]
    fn subscribers_do_not_see_rewards_hidden_from_them() {
        let list = vec![reward(), bypass()];
        let sub = RewardContext { viewer_is_sub: Some(true), ..Default::default() };
        let snap = snapshot("chan", list.clone(), &sub, NOW_MS, NOW_MS);
        assert_eq!(snap.rewards.len(), 1);
        let unknown = snapshot("chan", list, &RewardContext::default(), NOW_MS, NOW_MS);
        assert_eq!(unknown.rewards.len(), 2);
        assert_eq!(unknown.login, "chan");
    }

    #[test]
    fn badges_mark_subscribers_and_founders() {
        note_viewer_badges("badgetest", "moderator/1,subscriber/12");
        assert_eq!(room_of("badgetest").viewer_is_sub, Some(true));
        note_viewer_badges("badgetest", "founder/0");
        assert_eq!(room_of("badgetest").viewer_is_sub, Some(true));
        note_viewer_badges("badgetest", "premium/1");
        assert_eq!(room_of("badgetest").viewer_is_sub, Some(false));
        note_subs_only("BadgeTest", true);
        assert_eq!(room_of("badgetest").subs_only, Some(true));
        forget_room("badgetest");
        assert_eq!(room_of("badgetest").subs_only, None);
    }

    #[test]
    fn numbers_group_by_thousands() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1,000");
        assert_eq!(group(1_234_567), "1,234,567");
    }

    #[test]
    fn availability_serializes_for_the_menu() {
        let mut r = reward();
        r.is_paused = true;
        let v = serde_json::to_value(judge(&r, Some(1_000), NOW_MS)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "redeemable": false,
                "reason": "paused",
                "cooldown_until_ms": null,
                "affordable": true,
                "label": "Paused",
                "detail": "Paused by the streamer"
            })
        );
    }
}
