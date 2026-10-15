//! Twitch bits: the viewer's balance, a channel's cheer rules, checking a
//! message that cheers, and sending it.
//!
//! Twitch has no public API for cheering. The twitch.tv chat box sends a cheer
//! with the private GQL mutation `sendCheer` (read out of its bundle, see the
//! vault's Twitch GQL discovery notes), not as chat text, and so does this. A
//! typed `Cheer100` that went out as a normal chat message would spend nothing
//! and show as plain text, so `send_chat_message` hands any message that cheers
//! here first and the page asks before anything is spent.
//!
//! The rules are the ones the twitch.tv chat box checks before it sends (the
//! channel's minimum cheer, a minimum per cheermote, 100,000 bits per
//! cheermote, the balance, 500 characters, no command, no display-only
//! cheermote). Twitch checks again on its side; these catch the common cases
//! before a request and word them for the viewer.
//!
//! No poll, no socket. The balance is read once per signed-in account (the
//! first chat that opens on a channel with bits on, for the wallet button),
//! cached, and refreshed by each cheer's own answer and when the Get Bits page
//! closes; every change goes out through `channel_state` to the open chats.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::services::irc_service::{match_cheermote, CheermoteSet};
use crate::services::twitch_web_gql::post;

/// Twitch's ceiling for one cheermote (`Cheer100000`).
pub const MAX_PER_CHEERMOTE: u32 = 100_000;
/// Twitch's longest cheer message.
pub const MAX_CHEER_CHARS: usize = 500;
/// A channel's cheer settings change rarely; ten minutes keeps a busy chat
/// from asking again on every cheer.
const CHANNEL_TTL: Duration = Duration::from_secs(600);
const CHANNEL_CACHE_CAP: usize = 64;

/// A channel's cheer settings. Public on Twitch, so asked without a token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ChannelBits {
    pub enabled: bool,
    /// The smallest whole cheer the channel accepts.
    pub cheer_min: u32,
    /// The smallest amount one cheermote in it may carry.
    pub emote_min: u32,
}

impl ChannelBits {
    /// The smallest amount a cheer made of one cheermote can carry here: it
    /// has to clear both the channel's minimum and the per-cheermote minimum.
    pub fn smallest_single(&self) -> u32 {
        self.cheer_min.max(self.emote_min).max(1)
    }

    /// The ready-made amounts the picker offers: the smallest cheer here, then
    /// every cheermote tier above it that one cheermote can carry, at most
    /// `QUICK_AMOUNTS` of them.
    pub fn quick_amounts(&self, tiers: impl IntoIterator<Item = u32>) -> Vec<u32> {
        let min = self.smallest_single();
        let mut out: Vec<u32> = tiers.into_iter().filter(|&b| b > min && b <= MAX_PER_CHEERMOTE).collect();
        out.push(min);
        out.sort_unstable();
        out.dedup();
        out.truncate(QUICK_AMOUNTS);
        out
    }
}

/// How many ready-made amounts the picker shows (Twitch's tier ladder is six).
const QUICK_AMOUNTS: usize = 6;

/// What the cheers in a message add up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cheers {
    pub count: usize,
    pub total: u32,
    pub smallest: u32,
    pub largest: u32,
    /// A cheermote the viewer cannot cheer with (display-only, or a tier
    /// Twitch marks as not cheerable).
    pub display_only: bool,
}

/// A typed message that cheers, held back for the viewer to confirm.
#[derive(Clone, Debug, Serialize)]
pub struct CheerCheck {
    /// Bare lowercase login of the channel.
    pub channel: String,
    pub channel_id: String,
    pub text: String,
    pub total: u32,
    /// None when the balance could not be read (signed out of twitch.tv).
    pub balance: Option<i64>,
    /// Why it cannot go out as it is, in words. Empty = ready to cheer.
    pub problems: Vec<String>,
    /// The balance is what stands in the way: offer Get Bits.
    pub needs_bits: bool,
    /// Twitch's animated Cheer gem for this total (gray, purple, green, blue,
    /// red by amount), from the channel's own cheermote data.
    pub icon_url: Option<String>,
    /// The message with its cheers taken out, for the cheer bar's remove.
    pub plain_text: String,
}

/// What became of a cheer.
#[derive(Clone, Debug, Default, Serialize)]
pub struct CheerOutcome {
    pub sent: bool,
    /// The balance after the cheer, as Twitch reported it.
    pub balance: Option<i64>,
    /// Why it did not go out, in words.
    pub problem: Option<String>,
    /// Twitch would send it with the message removed (an anonymous cheer whose
    /// text it refused): the viewer may continue or edit.
    pub can_continue: bool,
    /// The balance is what stands in the way: offer Get Bits.
    pub needs_bits: bool,
    /// Twitch's security check refused the call (nothing was spent): a fresh
    /// integrity token may get it through.
    #[serde(skip)]
    pub integrity_refused: bool,
}

/// Twitch's own `Cheer` cheermote: its tiers are the bits gems Twitch draws
/// everywhere it shows an amount of bits.
pub const BITS_ICON_PREFIX: &str = "cheer";

/// The animated Cheer gem Twitch shows for `bits` (the highest tier the amount
/// reaches; below 1 it is the 1-bit gem).
pub fn cheer_icon(set: &CheermoteSet, bits: i64) -> Option<String> {
    let tiers = set.get(BITS_ICON_PREFIX)?;
    let n = bits.max(1);
    tiers
        .iter()
        .rev()
        .find(|t| i64::from(t.min_bits) <= n)
        .or_else(|| tiers.first())
        .map(|t| t.url.clone())
}

/// The cheers in `text` against the channel's cheermote set.
pub fn scan(text: &str, set: &CheermoteSet) -> Cheers {
    let mut c = Cheers::default();
    for word in text.split_whitespace() {
        let Some((_, bits, tiers)) = match_cheermote(word, set) else { continue };
        let tier = tiers.iter().rev().find(|t| t.min_bits <= bits).or_else(|| tiers.first());
        if !tier.is_some_and(|t| t.can_cheer) {
            c.display_only = true;
        }
        c.count += 1;
        c.total = c.total.saturating_add(bits);
        c.largest = c.largest.max(bits);
        c.smallest = if c.smallest == 0 { bits } else { c.smallest.min(bits) };
    }
    c
}

/// `text` without its cheer words (the same words `scan` counts), spacing
/// tidied.
pub fn strip_cheers(text: &str, set: &CheermoteSet) -> String {
    text.split_whitespace()
        .filter(|word| match_cheermote(word, set).is_none())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 12345 -> "12,345".
pub fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// The reasons a cheer cannot go out as typed, in the order Twitch's own chat
/// box checks them, worded for the viewer. Also whether the balance is one of
/// them.
pub fn problems(text: &str, c: &Cheers, ch: &ChannelBits, balance: Option<i64>) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut needs_bits = false;
    if c.largest > MAX_PER_CHEERMOTE {
        out.push(format!(
            "One cheermote can carry at most {} bits. Split it into several.",
            grouped(i64::from(MAX_PER_CHEERMOTE))
        ));
    }
    if let Some(have) = balance {
        if i64::from(c.total) > have {
            needs_bits = true;
            out.push(if have <= 0 {
                "You don't have any bits yet.".to_string()
            } else {
                format!(
                    "You have {} bits. This cheer needs {} more.",
                    grouped(have),
                    grouped(i64::from(c.total) - have)
                )
            });
        }
    }
    if c.total < ch.cheer_min {
        out.push(format!(
            "This channel's smallest cheer is {} bits. Add {} more.",
            grouped(i64::from(ch.cheer_min)),
            grouped(i64::from(ch.cheer_min - c.total))
        ));
    }
    if c.smallest < ch.emote_min {
        out.push(format!(
            "Each cheermote here needs at least {} bits.",
            grouped(i64::from(ch.emote_min))
        ));
    }
    if text.chars().count() > MAX_CHEER_CHARS {
        out.push(format!("A cheer message can be at most {MAX_CHEER_CHARS} characters."));
    }
    let t = text.trim_start();
    if (t.starts_with('/') && !t.starts_with("/me")) || (t.starts_with('.') && !t.starts_with(".me")) {
        out.push("A cheer can't start with a command.".to_string());
    }
    if c.display_only {
        out.push("One of these cheermotes is for show only and can't be cheered with.".to_string());
    }
    (out, needs_bits)
}

/// Twitch's `validationError.code` for a refused cheer, in words.
pub fn error_words(code: &str) -> String {
    match code {
        "AMOUNT_BELOW_MIN_BITS" => "That's below this channel's smallest cheer.".into(),
        "EMOTE_AMOUNT_BELOW_MIN_BITS" => "A cheermote is below this channel's smallest amount per cheermote.".into(),
        "INSUFFICIENT_BALANCE" => "You don't have enough bits for this cheer.".into(),
        "CHANNEL_BLOCKED_TERMS" => "Your message has a word this channel blocks. Edit it and try again.".into(),
        "CHANNEL_INELIGIBLE" => "This channel can't receive bits right now.".into(),
        "USER_INELIGIBLE" => "Your account can't cheer right now.".into(),
        "USER_BANNED" => "You're banned in this channel.".into(),
        "USER_SUSPENDED" => "Your account is suspended.".into(),
        "INVALID_ANONYMOUS_MESSAGE" => {
            "Twitch would send this cheer anonymously without your message.".into()
        }
        "INVALID_BITS_AMOUNT" | "INVALID_BITS_MESSAGE" => "Twitch didn't accept the cheermotes in this message.".into(),
        "MESSAGE_LENGTH_EXCEEDED" => format!("A cheer message can be at most {MAX_CHEER_CHARS} characters."),
        "TOO_LARGE_BITS_EMOTE" => format!(
            "One cheermote can carry at most {} bits.",
            grouped(i64::from(MAX_PER_CHEERMOTE))
        ),
        "TOO_LARGE_CHEER" => "That cheer is larger than Twitch allows in one message.".into(),
        "ZALGO_MESSAGE" => "Twitch refused the special characters in this message.".into(),
        "REQUEST_THROTTLED" => "Too many cheers at once. Wait a moment and try again.".into(),
        "INVALID_EVENT_ID" => "Twitch didn't accept this cheer. Try again.".into(),
        _ => "Your cheer couldn't be processed. Try again later.".into(),
    }
}

// ── balance ──────────────────────────────────────────────────────────────────

/// The signed-in viewer's balance: (twitch.tv user id, bits).
static BALANCE: Mutex<Option<(String, i64)>> = Mutex::new(None);

fn store_balance(user_id: &str, bits: i64) {
    let before = BALANCE.lock().unwrap_or_else(|e| e.into_inner()).replace((user_id.to_string(), bits));
    let Some(change) = balance_change(before.as_ref(), user_id, bits) else { return };
    tauri::async_runtime::spawn(async move {
        crate::services::channel_state::bits_balance_changed(bits, change).await;
    });
}

/// What a newly read balance means for the open chats: `None` = nothing
/// changed, `Some(rise)` = show the new number, with a "+N" when the same
/// account's balance went up.
fn balance_change(before: Option<&(String, i64)>, user_id: &str, bits: i64) -> Option<Option<i64>> {
    match before {
        Some((id, b)) if id == user_id && *b == bits => None,
        Some((id, b)) if id == user_id && bits > *b => Some(Some(bits - b)),
        _ => Some(None),
    }
}

/// One balance read at a time: several chats opening at once share it.
static BALANCE_READ: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The viewer's balance for the wallet button: the cached one, or one read
/// (shared by every caller waiting on it). None when the twitch.tv session
/// cannot be read.
pub async fn known_balance(auth: &crate::services::twitch_auth_service::TwitchAuthService) -> Option<i64> {
    if let Some((_, b)) = cached_balance() {
        return Some(b);
    }
    let _one = BALANCE_READ.lock().await;
    if let Some((_, b)) = cached_balance() {
        return Some(b);
    }
    let token = crate::services::twitch_web_gql::web_session_token(auth).await.ok()?;
    fetch_balance(&token).await.ok().map(|(_, b)| b)
}

/// Twitch's still Cheer gem for `balance` on `login`'s cheermote set (the
/// button face never animates; the open menu does). None until the set is
/// known.
pub fn still_gem(login: &str, balance: Option<i64>) -> Option<String> {
    let set = crate::services::irc_service::channel_cheermote_set(login)?;
    cheer_icon(&set, balance.unwrap_or(1)).map(|url| still_url(&url))
}

/// Twitch's CDN keeps each cheermote tier as `.../dark/animated/100/2.gif` and
/// `.../dark/static/100/2.png`.
fn still_url(url: &str) -> String {
    if !url.contains("/animated/") {
        return url.to_string();
    }
    let still = url.replacen("/animated/", "/static/", 1);
    match still.strip_suffix(".gif") {
        Some(base) => format!("{base}.png"),
        None => still,
    }
}

/// The last balance read for the signed-in viewer.
pub fn cached_balance() -> Option<(String, i64)> {
    BALANCE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The account changed: its balance is no longer the viewer's.
pub fn forget_account() {
    *BALANCE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    crate::services::twitch_integrity::invalidate();
    tauri::async_runtime::spawn(crate::services::channel_state::refresh_all_bits());
}

const BALANCE_QUERY: &str = "query BitsBalance { currentUser { id bitsBalance } }";

/// Read the balance of the account the twitch.tv session belongs to, and
/// remember it. Returns (that account's user id, bits).
pub async fn fetch_balance(token: &str) -> Result<(String, i64), String> {
    let json = post(
        Some(token),
        serde_json::json!({ "operationName": "BitsBalance", "query": BALANCE_QUERY }),
        "Bits",
    )
    .await?;
    let user = json.pointer("/data/currentUser").filter(|u| !u.is_null()).ok_or("not signed in to Twitch")?;
    let id = user.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let bits = user.get("bitsBalance").and_then(|v| v.as_i64()).unwrap_or(0);
    store_balance(&id, bits);
    Ok((id, bits))
}

// ── channel settings ─────────────────────────────────────────────────────────

fn channel_cache() -> &'static Mutex<HashMap<String, (Instant, ChannelBits)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, ChannelBits)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

const CHANNEL_QUERY: &str = "query BitsChannel($login: String!) { user(login: $login) { id cheer { isBitsEnabled settings { cheerMinimumBits emoteMinimumBits } } } }";

/// Reads a `BitsChannel` answer. A channel with no cheer settings at all has
/// bits off.
pub fn read_channel_bits(json: &serde_json::Value) -> Option<ChannelBits> {
    let user = json.pointer("/data/user").filter(|u| !u.is_null())?;
    let Some(cheer) = user.get("cheer").filter(|c| !c.is_null()) else {
        return Some(ChannelBits::default());
    };
    let n = |p: &str| cheer.pointer(p).and_then(|v| v.as_u64()).unwrap_or(1).max(1) as u32;
    Some(ChannelBits {
        enabled: cheer.get("isBitsEnabled").and_then(|v| v.as_bool()).unwrap_or(false),
        cheer_min: n("/settings/cheerMinimumBits"),
        emote_min: n("/settings/emoteMinimumBits"),
    })
}

/// A channel's cheer settings (bare login), cached for ten minutes.
pub async fn channel_bits(login: &str) -> Result<ChannelBits, String> {
    let key = login.trim().trim_start_matches('#').to_ascii_lowercase();
    if let Some((at, bits)) = channel_cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if at.elapsed() < CHANNEL_TTL {
            return Ok(*bits);
        }
    }
    let json = post(
        None,
        serde_json::json!({ "operationName": "BitsChannel", "query": CHANNEL_QUERY, "variables": { "login": key } }),
        "Bits",
    )
    .await?;
    let bits = read_channel_bits(&json).ok_or("Twitch did not answer for that channel")?;
    let mut cache = channel_cache().lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= CHANNEL_CACHE_CAP && !cache.contains_key(&key) {
        if let Some(oldest) = cache.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) {
            cache.remove(&oldest);
        }
    }
    cache.insert(key, (Instant::now(), bits));
    Ok(bits)
}

// ── sending ──────────────────────────────────────────────────────────────────

const SEND_MUTATION: &str = "mutation ChatInput_SendCheer($input: SendCheerInput!) { sendCheer(input: $input) { id currentUser { id bitsBalance } validationError { code message } } }";

/// Send a cheer as the twitch.tv web client does. `continue_anyway` answers
/// Twitch's anonymous-cheer prompt. Never retried: a refusal comes back as
/// words for the viewer.
pub async fn send(
    token: &str,
    channel_id: &str,
    text: &str,
    bits: u32,
    continue_anyway: bool,
    integrity: Option<&crate::services::twitch_integrity::Integrity>,
) -> Result<CheerOutcome, String> {
    let input = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "targetID": channel_id,
        "bits": bits,
        "content": text,
        "isAutoModEnabled": true,
        "shouldCheerAnyway": continue_anyway,
        "imageID": serde_json::Value::Null,
        "pinCheer": false,
    });
    let json = crate::services::twitch_web_gql::post_with(
        Some(token),
        serde_json::json!({ "operationName": "ChatInput_SendCheer", "query": SEND_MUTATION, "variables": { "input": input } }),
        "Bits",
        integrity,
    )
    .await?;
    Ok(read_send(&json))
}

/// Reads a `sendCheer` answer.
pub fn read_send(json: &serde_json::Value) -> CheerOutcome {
    let node = json.pointer("/data/sendCheer").filter(|n| !n.is_null());
    let balance = node.and_then(|n| n.pointer("/currentUser/bitsBalance")).and_then(|v| v.as_i64());
    if let (Some(bits), Some(id)) = (balance, node.and_then(|n| n.pointer("/currentUser/id")).and_then(|v| v.as_str())) {
        store_balance(id, bits);
    }
    if let Some(err) = node.and_then(|n| n.get("validationError")).filter(|e| !e.is_null()) {
        let code = err.get("code").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
        log::warn!("[Bits] cheer refused: {code}");
        return CheerOutcome {
            sent: false,
            balance,
            problem: Some(error_words(code)),
            can_continue: code == "INVALID_ANONYMOUS_MESSAGE",
            needs_bits: code == "INSUFFICIENT_BALANCE",
            ..Default::default()
        };
    }
    if node.is_none() {
        // A GraphQL error (a refused token, an integrity check): say so rather
        // than pretend it went out.
        let msg = json
            .pointer("/errors/0/message")
            .and_then(|v| v.as_str())
            .unwrap_or("no answer");
        log::warn!("[Bits] cheer failed: {msg}");
        let integrity_refused = json.pointer("/errors/0/extensions/code").and_then(|v| v.as_str())
            == Some("IntegrityCheckFailed")
            || msg.contains("integrity");
        let problem = if integrity_refused {
            "Twitch's security check didn't pass, so nothing was spent. Try again in a moment.".to_string()
        } else {
            format!("Twitch didn't take the cheer ({msg}). Nothing was spent.")
        };
        return CheerOutcome { sent: false, balance, problem: Some(problem), integrity_refused, ..Default::default() };
    }
    CheerOutcome { sent: true, balance, ..Default::default() }
}

// ── typed messages ───────────────────────────────────────────────────────────

/// Whether a typed message for `channel` cheers. `None` = send it as chat:
/// a command, no cheermote in it, a channel whose cheermotes or settings are
/// not known, or a channel with bits off (where Twitch reads it as text too).
/// `Some` = hold it: the check carries the total, the balance and anything
/// that stops it. `sender_is_main` is false when Send as picked another
/// account; cheers come from the twitch.tv session, which is the main one.
pub async fn typed_cheer(
    auth: &crate::services::twitch_auth_service::TwitchAuthService,
    text: &str,
    channel: &str,
    channel_id: Option<&str>,
    sender_is_main: bool,
) -> Option<CheerCheck> {
    let t = text.trim_start();
    if t.starts_with('/') && !t.starts_with("/me ") {
        return None;
    }
    let key = channel.trim().trim_start_matches('#').to_ascii_lowercase();
    let channel_id = channel_id.filter(|id| !id.is_empty())?;
    let set = crate::services::irc_service::channel_cheermote_set(&key)?;
    let cheers = scan(text, &set);
    if cheers.count == 0 {
        return None;
    }
    let ch = channel_bits(&key).await.ok()?;
    if !ch.enabled {
        return None;
    }
    // The viewer is about to see the confirm card: mint Twitch's security
    // token now, so the send does not wait on it.
    crate::services::twitch_integrity::warm();
    Some(check_for(auth, text, &key, channel_id, &set, &cheers, &ch, sender_is_main, false).await)
}

/// The full check for a message already known to cheer.
pub async fn check_for(
    auth: &crate::services::twitch_auth_service::TwitchAuthService,
    text: &str,
    key: &str,
    channel_id: &str,
    set: &CheermoteSet,
    cheers: &Cheers,
    ch: &ChannelBits,
    sender_is_main: bool,
    fresh: bool,
) -> CheerCheck {
    // While the viewer types (`fresh` false) the cached balance answers, kept
    // current by every cheer and purchase; the send itself reads it again.
    let cached = if fresh { None } else { cached_balance() };
    let session = match cached {
        Some(s) => Some(s),
        None => match crate::services::twitch_web_gql::web_session_token(auth).await {
            Ok(token) => fetch_balance(&token).await.ok(),
            Err(_) => None,
        },
    };
    let balance = session.as_ref().map(|(_, b)| *b);
    let (mut list, needs_bits) = problems(text, cheers, ch, balance);
    match session {
        None => list.push("Sign in to Twitch again to cheer.".to_string()),
        Some((web_id, _)) => {
            let main = crate::services::account_store::AccountStore::primary().map(|a| a.user_id);
            if main.as_deref().is_some_and(|m| !web_id.is_empty() && m != web_id) {
                list.push(
                    "Your twitch.tv session belongs to a different account than the one you chat as. Sign in again to cheer."
                        .to_string(),
                );
            }
        }
    }
    if !sender_is_main {
        list.push("Cheers come from your main account. Switch Send as back to it to cheer.".to_string());
    }
    CheerCheck {
        channel: key.to_string(),
        channel_id: channel_id.to_string(),
        text: text.to_string(),
        total: cheers.total,
        balance,
        problems: list,
        needs_bits,
        icon_url: cheer_icon(set, i64::from(cheers.total)),
        plain_text: strip_cheers(text, set),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_face_gem_is_the_still_image() {
        assert_eq!(
            still_url("https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/100/2.gif"),
            "https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/static/100/2.png"
        );
        let already = "https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/static/1/2.png";
        assert_eq!(still_url(already), already);
    }

    #[test]
    fn a_balance_read_says_what_changed() {
        let me = ("1".to_string(), 500);
        assert_eq!(balance_change(Some(&me), "1", 500), None);
        assert_eq!(balance_change(Some(&me), "1", 800), Some(Some(300)));
        assert_eq!(balance_change(Some(&me), "1", 400), Some(None));
        assert_eq!(balance_change(Some(&me), "2", 900), Some(None));
        assert_eq!(balance_change(None, "1", 500), Some(None));
    }
    use crate::services::irc_service::CheermoteTier;

    fn set() -> CheermoteSet {
        let tier = |prefix: &str, min_bits: u32, can_cheer: bool| CheermoteTier {
            min_bits,
            prefix: prefix.to_string(),
            can_cheer,
            ..Default::default()
        };
        let mut s = CheermoteSet::new();
        s.insert("cheer".into(), vec![tier("Cheer", 1, true), tier("Cheer", 100, true), tier("Cheer", 1000, true)]);
        s.insert("cheerwhal".into(), vec![tier("Cheerwhal", 1, true)]);
        s.insert("showoff".into(), vec![tier("ShowOff", 1, false)]);
        s
    }

    #[test]
    fn scan_adds_up_every_cheermote() {
        let c = scan("hi Cheer100 and cheer50 plus Cheerwhal5", &set());
        assert_eq!(c.count, 3);
        assert_eq!(c.total, 155);
        assert_eq!(c.smallest, 5);
        assert_eq!(c.largest, 100);
        assert!(!c.display_only);
    }

    #[test]
    fn removing_the_cheer_keeps_the_rest_of_the_message() {
        assert_eq!(strip_cheers("hi Cheer100 and  cheer50 plus Cheerwhal5", &set()), "hi and plus");
        assert_eq!(strip_cheers("Cheer1 testing", &set()), "testing");
        assert_eq!(strip_cheers("cheers100x stays", &set()), "cheers100x stays");
    }

    #[test]
    fn scan_ignores_words_that_only_look_like_cheers() {
        assert_eq!(scan("Cheer0 cheer cheers100x 100 cheer1e3", &set()).count, 0);
    }

    #[test]
    fn a_display_only_cheermote_is_flagged() {
        assert!(scan("ShowOff10", &set()).display_only);
    }

    #[test]
    fn problems_follow_twitchs_checks() {
        let ch = ChannelBits { enabled: true, cheer_min: 200, emote_min: 10 };
        let c = scan("Cheer100 Cheer5", &set());
        let (p, needs) = problems("Cheer100 Cheer5", &c, &ch, Some(50));
        assert!(needs);
        assert_eq!(p[0], "You have 50 bits. This cheer needs 55 more.");
        assert_eq!(p[1], "This channel's smallest cheer is 200 bits. Add 95 more.");
        assert_eq!(p[2], "Each cheermote here needs at least 10 bits.");
        let (p, _) = problems("/ban x Cheer100", &scan("Cheer100", &set()), &ChannelBits { enabled: true, cheer_min: 1, emote_min: 1 }, Some(500));
        assert_eq!(p, vec!["A cheer can't start with a command.".to_string()]);
        let (p, _) = problems("/me Cheer100", &scan("Cheer100", &set()), &ChannelBits { enabled: true, cheer_min: 1, emote_min: 1 }, Some(500));
        assert!(p.is_empty(), "/me is allowed");
    }

    #[test]
    fn the_smallest_single_cheer_clears_both_minimums() {
        let ch = |cheer_min, emote_min| ChannelBits { enabled: true, cheer_min, emote_min };
        assert_eq!(ch(200, 1).smallest_single(), 200);
        assert_eq!(ch(1, 50).smallest_single(), 50);
        assert_eq!(ch(0, 0).smallest_single(), 1);
        let min = ch(200, 10).smallest_single();
        let text = format!("Cheer{min}");
        let (p, _) = problems(&text, &scan(&text, &set()), &ch(200, 10), Some(1_000));
        assert!(p.is_empty(), "the smallest pick passes every check: {p:?}");
    }

    #[test]
    fn quick_amounts_start_at_the_smallest_cheer() {
        let ladder = [1, 100, 1000, 5000, 10000, 100000, 1000, 100];
        let ch = |cheer_min, emote_min| ChannelBits { enabled: true, cheer_min, emote_min };
        assert_eq!(ch(200, 1).quick_amounts(ladder), vec![200, 1000, 5000, 10000, 100000]);
        assert_eq!(ch(1, 1).quick_amounts(ladder), vec![1, 100, 1000, 5000, 10000, 100000]);
        assert_eq!(ch(100, 1).quick_amounts(ladder), vec![100, 1000, 5000, 10000, 100000]);
        assert_eq!(ch(1, 1).quick_amounts([250_000]), vec![1], "nothing over one cheermote's ceiling");
    }

    #[test]
    fn a_zero_balance_says_so_plainly() {
        let ch = ChannelBits { enabled: true, cheer_min: 1, emote_min: 1 };
        let (p, needs) = problems("Cheer1", &scan("Cheer1", &set()), &ch, Some(0));
        assert!(needs);
        assert_eq!(p, vec!["You don't have any bits yet.".to_string()]);
    }

    #[test]
    fn the_bits_icon_follows_the_amount() {
        let tier = |min_bits: u32| CheermoteTier { min_bits, url: format!("gem-{min_bits}"), can_cheer: true, ..Default::default() };
        let mut s = CheermoteSet::new();
        s.insert("cheer".into(), vec![tier(1), tier(100), tier(1000), tier(5000), tier(10000)]);
        assert_eq!(cheer_icon(&s, 0).as_deref(), Some("gem-1"));
        assert_eq!(cheer_icon(&s, 99).as_deref(), Some("gem-1"));
        assert_eq!(cheer_icon(&s, 1250).as_deref(), Some("gem-1000"));
        assert_eq!(cheer_icon(&s, 250_000).as_deref(), Some("gem-10000"));
        assert_eq!(cheer_icon(&CheermoteSet::new(), 5), None);
    }

    #[test]
    fn grouped_numbers_read_like_twitchs() {
        assert_eq!(grouped(5), "5");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(100000), "100,000");
        assert_eq!(grouped(1234567), "1,234,567");
    }

    #[test]
    fn channel_settings_read_from_the_answer() {
        let on = serde_json::json!({"data":{"user":{"id":"1","cheer":{"isBitsEnabled":true,"settings":{"cheerMinimumBits":200,"emoteMinimumBits":1}}}}});
        assert_eq!(read_channel_bits(&on), Some(ChannelBits { enabled: true, cheer_min: 200, emote_min: 1 }));
        let none = serde_json::json!({"data":{"user":{"id":"1","cheer":null}}});
        assert_eq!(read_channel_bits(&none), Some(ChannelBits::default()));
        assert_eq!(read_channel_bits(&serde_json::json!({"data":{"user":null}})), None);
    }

    #[test]
    fn send_answers_read_into_outcomes() {
        let ok = serde_json::json!({"data":{"sendCheer":{"id":"x","currentUser":{"id":"9","bitsBalance":40},"validationError":null}}});
        let o = read_send(&ok);
        assert!(o.sent);
        assert_eq!(o.balance, Some(40));

        let low = serde_json::json!({"data":{"sendCheer":{"id":null,"currentUser":{"id":"9","bitsBalance":3},"validationError":{"code":"INSUFFICIENT_BALANCE","message":""}}}});
        let o = read_send(&low);
        assert!(!o.sent && o.needs_bits && !o.can_continue);

        let anon = serde_json::json!({"data":{"sendCheer":{"id":null,"currentUser":null,"validationError":{"code":"INVALID_ANONYMOUS_MESSAGE","message":""}}}});
        assert!(read_send(&anon).can_continue);

        let refused = serde_json::json!({"errors":[{"extensions":{"code":"IntegrityCheckFailed"},"message":"failed integrity check","path":["sendCheer"]}],"data":{"sendCheer":null}});
        let o = read_send(&refused);
        assert!(!o.sent);
        assert!(o.integrity_refused);
        assert_eq!(o.problem.as_deref(), Some("Twitch's security check didn't pass, so nothing was spent. Try again in a moment."));

        let other = serde_json::json!({"errors":[{"message":"service timeout"}],"data":{"sendCheer":null}});
        let o = read_send(&other);
        assert!(!o.integrity_refused);
        assert_eq!(o.problem.as_deref(), Some("Twitch didn't take the cheer (service timeout). Nothing was spent."));
    }
}
