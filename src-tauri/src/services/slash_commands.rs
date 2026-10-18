//! The work behind chat slash commands: every Helix and GQL call, the parsing
//! of what Twitch returns, and the line the command prints. The page only
//! dispatches the command and shows the result.
//!
//! Pure builders and parsers are separate from the network calls so the
//! request shapes and the answers are tested against fixtures.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Datelike, Months, Utc};
use reqwest::header::AUTHORIZATION;
use reqwest::Method;
use serde::Serialize;
use serde_json::{json, Value};

use crate::services::follow_lookup;
use crate::services::twitch_service::{ChannelRole, TwitchService};

const APP_CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");
/// The client the app's other private GQL writes (poll votes, reward redeems)
/// use with the Drops session token.
const ANDROID_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

// ───────────────────────────── names and numbers ─────────────────────────────

/// A Twitch login from what someone typed: `@Name`, `#name` or `name`.
/// None when it cannot be a login.
pub fn clean_login(raw: &str) -> Option<String> {
    let s = raw.trim().trim_start_matches(['@', '#']).to_ascii_lowercase();
    let ok = !s.is_empty() && s.len() <= 25 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some(s)
}

/// `1234567` as "1,234,567".
pub fn group(n: i64) -> String {
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

fn plural(n: i64, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// How long from `from` to `to`, in calendar units: "2 years, 3 months, 4 days",
/// or hours and minutes under a day.
pub fn span_words(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    if to <= from {
        return "less than a minute".into();
    }
    let secs = (to - from).num_seconds();
    if secs < 86_400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        return match (h, m) {
            (0, 0) => "less than a minute".into(),
            (0, m) => plural(m, "minute"),
            (h, 0) => plural(h, "hour"),
            (h, m) => format!("{}, {}", plural(h, "hour"), plural(m, "minute")),
        };
    }
    let mut cursor = from;
    let mut months_total = 0u32;
    while let Some(next) = from.checked_add_months(Months::new(months_total + 1)) {
        if next > to {
            break;
        }
        months_total += 1;
        cursor = next;
    }
    let years = i64::from(months_total / 12);
    let months = i64::from(months_total % 12);
    let days = (to - cursor).num_days();
    let mut parts = Vec::new();
    if years > 0 {
        parts.push(plural(years, "year"));
    }
    if months > 0 {
        parts.push(plural(months, "month"));
    }
    if days > 0 || parts.is_empty() {
        parts.push(plural(days, "day"));
    }
    parts.join(", ")
}

fn date_of(t: DateTime<Utc>) -> String {
    format!("{:04}-{:02}-{:02}", t.year(), t.month(), t.day())
}

fn parse_time(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

// ─────────────────────────────────── Helix ───────────────────────────────────

fn helix_error(status: reqwest::StatusCode, body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_string))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("Twitch answered HTTP {}", status.as_u16()))
}

/// A Helix call that did not succeed: the HTTP status when Twitch answered,
/// the words for the person, and on a 429 when the budget refills.
#[derive(Clone, Debug, PartialEq)]
pub struct HelixError {
    pub status: Option<u16>,
    pub message: String,
    /// `Ratelimit-Reset`, in Unix seconds.
    pub reset_at: Option<u64>,
}

impl HelixError {
    fn local(message: impl Into<String>) -> Self {
        Self { status: None, message: message.into(), reset_at: None }
    }
}

/// A Helix call with the signed-in account's token. `path` is everything after
/// `/helix/`. A 204 answers `Value::Null`.
pub async fn helix(method: Method, path: &str, body: Option<Value>) -> Result<Value, String> {
    helix_call(method, path, body).await.map_err(|e| e.message)
}

/// `helix`, keeping the status and the rate-limit reset for callers that pace.
pub async fn helix_call(method: Method, path: &str, body: Option<Value>) -> Result<Value, HelixError> {
    let token = TwitchService::get_token()
        .await
        .map_err(|_| HelixError::local("Sign in to Twitch to use this command"))?;
    let mut req = crate::services::http::client()
        .clone()
        .request(method, format!("https://api.twitch.tv/helix/{path}"))
        .header("Client-Id", APP_CLIENT_ID)
        .header(AUTHORIZATION, format!("Bearer {token}"));
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| HelixError::local(format!("Could not reach Twitch: {e}")))?;
    let status = resp.status();
    let reset_at = resp
        .headers()
        .get("Ratelimit-Reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(HelixError { status: Some(status.as_u16()), message: helix_error(status, &text), reset_at });
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|_| HelixError::local("Twitch sent an answer that could not be read"))
}

/// How long to wait after a 429: until Twitch's `Ratelimit-Reset`, at least a
/// second and at most a minute.
pub fn rate_limit_wait(reset_at: Option<u64>, now_unix: u64) -> std::time::Duration {
    let secs = reset_at.map(|r| r.saturating_sub(now_unix)).unwrap_or(2);
    std::time::Duration::from_secs(secs.clamp(1, 60))
}

/// Retries after a 429, this many times, before giving up on one request.
pub const RATE_LIMIT_RETRIES: u32 = 5;

/// A Helix call that waits out 429s (up to [`RATE_LIMIT_RETRIES`] times).
async fn helix_paced(method: Method, path: &str) -> Result<Value, String> {
    let mut retries = 0;
    loop {
        match helix_call(method.clone(), path, None).await {
            Err(e) if e.status == Some(429) && retries < RATE_LIMIT_RETRIES => {
                retries += 1;
                let wait = rate_limit_wait(e.reset_at, Utc::now().timestamp().max(0) as u64);
                log::info!("[slash] Helix rate limit, waiting {}s before trying again", wait.as_secs());
                tokio::time::sleep(wait).await;
            }
            other => return other.map_err(|e| e.message),
        }
    }
}

/// `{id, login, display_name, created_at}` of a login, None when no such user.
pub async fn user_by_login(login: &str) -> Result<Option<Value>, String> {
    let v = helix(Method::GET, &format!("users?login={login}"), None).await?;
    Ok(v["data"].as_array().and_then(|a| a.first()).cloned())
}

async fn users_by_ids(ids: &[String]) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for chunk in ids.chunks(100) {
        if chunk.is_empty() {
            continue;
        }
        let q: Vec<String> = chunk.iter().map(|id| format!("id={id}")).collect();
        let v = helix(Method::GET, &format!("users?{}", q.join("&")), None).await?;
        out.extend(v["data"].as_array().cloned().unwrap_or_default());
    }
    Ok(out)
}

fn display(user: &Value) -> String {
    user["display_name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| user["login"].as_str())
        .unwrap_or("?")
        .to_string()
}

async fn require_user(login: &str) -> Result<Value, String> {
    user_by_login(login)
        .await?
        .ok_or_else(|| format!("There is no Twitch user named {login}"))
}

/// The signed-in account: (id, login).
pub async fn me() -> Result<(String, String), String> {
    let info = TwitchService::get_user_info()
        .await
        .map_err(|_| "Sign in to Twitch to use this command".to_string())?;
    Ok((info.id, info.login))
}

async fn require_broadcaster(broadcaster_id: &str, what: &str) -> Result<(), String> {
    let (id, _) = me().await?;
    if id != broadcaster_id {
        return Err(format!("Only the broadcaster can {what}"));
    }
    Ok(())
}

// ─────────────────────────────── viewer facts ────────────────────────────────

pub fn followage_text(user: &str, channel: &str, followed_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let you = user == "You";
    match followed_at {
        Some(t) => format!(
            "{user} {} followed {channel} for {} (since {}).",
            if you { "have" } else { "has" },
            span_words(t, now),
            date_of(t)
        ),
        None => format!("{user} {} not follow {channel}.", if you { "do" } else { "does" }),
    }
}

/// `/followage [user] [channel]`. Your own follow comes from Helix; anyone
/// else's from the channel's follower list where Helix shows it (you own or
/// moderate the channel), else api.ivr.fi.
pub async fn followage(channel_login: &str, user_login: Option<&str>) -> Result<String, String> {
    let channel = require_user(channel_login).await?;
    let channel_id = channel["id"].as_str().unwrap_or_default().to_string();
    let (my_id, my_login) = me().await?;
    let target = user_login.unwrap_or(&my_login).to_string();
    if target == my_login {
        if channel_id == my_id {
            return Ok("You can't follow your own channel.".into());
        }
        let v = helix(
            Method::GET,
            &format!("channels/followed?user_id={my_id}&broadcaster_id={channel_id}"),
            None,
        )
        .await?;
        let at = v["data"].as_array().and_then(|a| a.first()).and_then(|f| parse_time(&f["followed_at"]));
        return Ok(followage_text("You", &display(&channel), at, Utc::now()));
    }
    let user = require_user(&target).await?;
    let user_id = user["id"].as_str().unwrap_or_default();
    let (ivr, from_helix) = tokio::join!(
        crate::services::ivr::subage(&target, channel_login),
        follow_lookup::helix_follow(user_id, &channel_id, follow_lookup::ViewerRole::Unknown)
    );
    let ivr_at = ivr.as_ref().ok().and_then(|v| v["followedAt"].as_str());
    let (who, chan) = (display(&user), display(&channel));
    Ok(match follow_lookup::merge(ivr_at, from_helix) {
        follow_lookup::FollowAnswer::Following(at) => {
            followage_text(&who, &chan, parse_time(&Value::String(at)), Utc::now())
        }
        follow_lookup::FollowAnswer::NotFollowing => followage_text(&who, &chan, None, Utc::now()),
        follow_lookup::FollowAnswer::Unknown => followage_unknown_text(&who, &chan),
    })
}

/// When no source can say: Twitch shares a channel's followers only with the
/// channel's broadcaster and moderators.
pub fn followage_unknown_text(user: &str, channel: &str) -> String {
    format!("StreamNook can't see whether {user} follows {channel}: Twitch only shares a channel's followers with its moderators.")
}

pub fn follower_count_text(channel: &str, total: Option<i64>) -> String {
    match total {
        Some(n) => format!("{channel} has {} {}.", group(n), if n == 1 { "follower" } else { "followers" }),
        None => format!("Twitch did not give a follower count for {channel}."),
    }
}

/// `/follows [channel]`: Helix returns the total to any signed-in account.
pub async fn follower_count(channel_login: &str) -> Result<String, String> {
    let channel = require_user(channel_login).await?;
    let id = channel["id"].as_str().unwrap_or_default();
    let v = helix(Method::GET, &format!("channels/followers?broadcaster_id={id}&first=1"), None).await?;
    Ok(follower_count_text(&display(&channel), v["total"].as_i64()))
}

pub fn account_age_text(user: &str, created_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    match created_at {
        Some(t) => format!("{user}'s account is {} old (created {}).", span_words(t, now), date_of(t)),
        None => format!("Twitch did not say when {user}'s account was made."),
    }
}

/// `/accountage [user]`.
pub async fn account_age(user_login: &str) -> Result<String, String> {
    let user = require_user(user_login).await?;
    Ok(account_age_text(&display(&user), parse_time(&user["created_at"]), Utc::now()))
}

pub fn viewer_count_text(channel: &str, stream: Option<&Value>) -> String {
    match stream.and_then(|s| s["viewer_count"].as_i64()) {
        Some(n) => format!("{channel} is live with {} {}.", group(n), if n == 1 { "viewer" } else { "viewers" }),
        None => format!("{channel} is offline."),
    }
}

/// `/viewers [channel]`.
pub async fn viewer_count(channel_login: &str) -> Result<String, String> {
    let channel = require_user(channel_login).await?;
    let v = helix(Method::GET, &format!("streams?user_login={channel_login}"), None).await?;
    let stream = v["data"].as_array().and_then(|a| a.first()).cloned();
    Ok(viewer_count_text(&display(&channel), stream.as_ref()))
}

/// Lines for api.ivr.fi's `founders` answer.
pub fn founders_lines(channel: &str, body: &Value) -> Vec<String> {
    let list = body["founders"].as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        return vec![format!("{channel} has no founders.")];
    }
    let names: Vec<String> = list
        .iter()
        .map(|f| {
            let name = f["displayName"].as_str().or_else(|| f["login"].as_str()).unwrap_or("?");
            if f["isSubscribed"].as_bool() == Some(false) {
                format!("{name} (no longer subscribed)")
            } else {
                name.to_string()
            }
        })
        .collect();
    vec![format!("The founders of {channel} ({}): {}", list.len(), names.join(", "))]
}

/// `/founders [channel]`. Twitch has no Helix list; api.ivr.fi reads the
/// public founders badge holders.
pub async fn founders(channel_login: &str) -> Result<Vec<String>, String> {
    let resp = crate::services::http::client()
        .get(format!("https://api.ivr.fi/v2/twitch/founders/{channel_login}"))
        .send()
        .await
        .map_err(|e| format!("Could not reach the founders lookup: {e}"))?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(vec![format!("{channel_login} has no founders.")]);
    }
    if !status.is_success() {
        return Err(format!("The founders lookup answered HTTP {}", status.as_u16()));
    }
    let body: Value = resp.json().await.map_err(|_| "The founders lookup sent an unreadable answer".to_string())?;
    Ok(founders_lines(channel_login, &body))
}

/// Lines for logs.zonian.dev's name history: `[{user_login, first_timestamp,
/// last_timestamp}]`, oldest name first.
pub fn name_history_lines(current: &str, body: &Value) -> Vec<String> {
    let mut rows: Vec<(String, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> = body
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            let login = r["user_login"].as_str()?.trim().to_string();
            (!login.is_empty()).then(|| (login, parse_time(&r["first_timestamp"]), parse_time(&r["last_timestamp"])))
        })
        .collect();
    rows.sort_by_key(|r| r.1);
    let others: Vec<String> = rows
        .iter()
        .filter(|r| !r.0.eq_ignore_ascii_case(current))
        .map(|r| match (r.1, r.2) {
            (Some(a), Some(b)) => format!("{} ({} to {})", r.0, date_of(a), date_of(b)),
            _ => r.0.clone(),
        })
        .collect();
    if others.is_empty() {
        vec![format!("No earlier names are known for {current}.")]
    } else {
        vec![format!("Earlier names of {current}: {}", others.join(", "))]
    }
}

/// `/namehistory <user>`, by user id so a renamed account still resolves.
/// Accepts a numeric id for an account whose current name is unknown.
pub async fn name_history(user: &str) -> Result<Vec<String>, String> {
    let (id, name) = if user.chars().all(|c| c.is_ascii_digit()) {
        (user.to_string(), user.to_string())
    } else {
        let u = require_user(user).await?;
        (u["id"].as_str().unwrap_or_default().to_string(), display(&u))
    };
    let resp = crate::services::http::client()
        .get(format!("https://logs.zonian.dev/namehistory/{id}"))
        .send()
        .await
        .map_err(|e| format!("Could not reach the name history lookup: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("The name history lookup answered HTTP {}", resp.status().as_u16()));
    }
    let body: Value = resp.json().await.map_err(|_| "The name history lookup sent an unreadable answer".to_string())?;
    Ok(name_history_lines(&name, &body))
}

/// The public GQL read of a channel's chat rules.
pub fn rules_request(login: &str) -> Value {
    json!({
        "query": "query ChannelChatRules($login: String!) { user(login: $login) { displayName chatSettings { rules } } }",
        "variables": { "login": login },
    })
}

pub fn rules_lines(login: &str, body: &Value) -> Result<Vec<String>, String> {
    if let Some(msg) = body["errors"][0]["message"].as_str() {
        return Err(format!("Twitch refused the rules lookup ({msg})"));
    }
    let user = &body["data"]["user"];
    if user.is_null() {
        return Err(format!("There is no Twitch channel named {login}"));
    }
    let name = user["displayName"].as_str().unwrap_or(login);
    let rules: Vec<String> = user["chatSettings"]["rules"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r.as_str().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string))
        .collect();
    if rules.is_empty() {
        return Ok(vec![format!("{name} has not set any chat rules.")]);
    }
    let mut out = vec![format!("{name}'s chat rules:")];
    out.extend(rules.iter().enumerate().map(|(i, r)| format!("{}. {r}", i + 1)));
    Ok(out)
}

/// `/rules [channel]`. Public data: no sign-in needed.
pub async fn chat_rules(channel_login: &str) -> Result<Vec<String>, String> {
    let resp = crate::services::http::client()
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", crate::services::auth_proxy::TWITCH_WEB_CLIENT_ID)
        .json(&rules_request(channel_login))
        .send()
        .await
        .map_err(|e| format!("Could not reach Twitch: {e}"))?;
    let body: Value = resp.json().await.map_err(|_| "Twitch sent an unreadable answer".to_string())?;
    rules_lines(channel_login, &body)
}

/// `/mods` and `/vips`. `listed` is the whole group, None when Twitch would
/// not list it; `in_chat` is who of it is in chat now (Twitch's public
/// chatters list), shown only then.
pub fn role_list_lines(role: ChannelRole, listed: Option<&[String]>, signed_in: bool, in_chat: &[String]) -> Vec<String> {
    let (group, list) = match role {
        ChannelRole::Moderators => ("moderators", "moderator list"),
        ChannelRole::Vips => ("VIPs", "VIP list"),
    };
    let line = match listed {
        Some([]) => format!("There are no {group} of this channel."),
        Some(all) => format!("The {group} of this channel are: {}", all.join(", ")),
        None => {
            let why = if signed_in {
                format!("Twitch would not share this channel's {list}.")
            } else {
                format!("Sign in to see this channel's {list}.")
            };
            if in_chat.is_empty() {
                why
            } else {
                format!("{why} In chat now: {}", in_chat.join(", "))
            }
        }
    };
    vec![line]
}

/// `/mods` and `/vips`: read signed in, since Twitch lists moderators only to
/// a signed-in viewer.
pub async fn role_list(
    auth: &crate::services::twitch_auth_service::TwitchAuthService,
    channel_login: &str,
    role: ChannelRole,
) -> Result<Vec<String>, String> {
    let callers = crate::services::twitch_service::role_list_callers(Some(auth)).await;
    let members = TwitchService::channel_role_members(channel_login, role, &callers)
        .await
        .map_err(|e| format!("Could not reach Twitch: {e}"))?;
    let logins_of = |rows: &[Value], key: &str| -> Vec<String> {
        rows.iter().filter_map(|r| r[key].as_str().map(str::to_string)).collect()
    };
    if let Some(members) = members {
        return Ok(role_list_lines(role, Some(&logins_of(&members, "login")), true, &[]));
    }
    let group = match role {
        ChannelRole::Moderators => "moderators",
        ChannelRole::Vips => "vips",
    };
    let in_chat = TwitchService::get_public_chatters(channel_login)
        .await
        .ok()
        .and_then(|c| c[group].as_array().map(|rows| logins_of(rows, "user_login")))
        .unwrap_or_default();
    let signed_in = callers.iter().any(|c| c.is_signed_in());
    Ok(role_list_lines(role, None, signed_in, &in_chat))
}

/// `/sharedchat`: who is in this channel's Shared Chat session.
pub fn shared_chat_lines(channel: &str, host: Option<&str>, members: &[Value]) -> Vec<String> {
    if members.is_empty() {
        return vec![format!("{channel} is not in a Shared Chat session.")];
    }
    let names: Vec<String> = members
        .iter()
        .map(|u| {
            let n = display(u);
            if host.is_some() && u["id"].as_str() == host {
                format!("{n} (host)")
            } else {
                n
            }
        })
        .collect();
    vec![format!("{channel} is sharing chat with: {}", names.join(", "))]
}

pub async fn shared_chat(broadcaster_id: &str, channel: &str) -> Result<Vec<String>, String> {
    let session = TwitchService::get_shared_chat_session(broadcaster_id)
        .await
        .map_err(|e| format!("Could not read the Shared Chat session: {e}"))?;
    let Some((host, ids)) = session else {
        return Ok(shared_chat_lines(channel, None, &[]));
    };
    let users = users_by_ids(&ids).await?;
    Ok(shared_chat_lines(channel, Some(&host), &users))
}

/// `/raidbrowser`: live channels you follow, busiest first, as raid targets.
pub fn raid_candidate_lines(streams: &[crate::models::stream::TwitchStream], own_login: &str, limit: usize) -> Vec<String> {
    let mut live: Vec<&crate::models::stream::TwitchStream> =
        streams.iter().filter(|s| !s.user_login.eq_ignore_ascii_case(own_login)).collect();
    if live.is_empty() {
        return vec!["None of the channels you follow are live right now.".into()];
    }
    live.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
    let mut out = vec![format!("{} followed channels are live. Raid one with /raid <channel>:", live.len())];
    for s in live.iter().take(limit) {
        let game = if s.game_name.is_empty() { String::new() } else { format!(", {}", s.game_name) };
        out.push(format!("{} ({} viewers{game})", s.user_login, group(i64::from(s.viewer_count))));
    }
    if live.len() > limit {
        out.push(format!("...and {} more, with fewer viewers.", live.len() - limit));
    }
    out
}

// ─────────────────────────────── private GQL ─────────────────────────────────

/// The Drops session token and the account it belongs to. Private GQL writes
/// act as that account, so the broadcaster checks compare against it.
async fn private_session() -> Result<(String, String), String> {
    let token = crate::services::drops_auth_service::DropsAuthService::get_token()
        .await
        .map_err(|_| "This needs your Twitch rewards sign-in (Settings, Accounts)".to_string())?;
    let v: Value = crate::services::http::client()
        .get("https://id.twitch.tv/oauth2/validate")
        .header("Authorization", format!("OAuth {token}"))
        .send()
        .await
        .map_err(|e| format!("Could not reach Twitch: {e}"))?
        .json()
        .await
        .map_err(|_| "Could not check your Twitch session".to_string())?;
    let user_id = v["user_id"]
        .as_str()
        .ok_or("Your Twitch rewards sign-in has expired. Sign in again in Settings, Accounts")?
        .to_string();
    Ok((token, user_id))
}

async fn private_gql(token: &str, body: &Value) -> Result<Value, String> {
    let device = uuid::Uuid::new_v4().simple().to_string();
    let session = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
    let resp = crate::services::http::client()
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", ANDROID_CLIENT_ID)
        .header("Authorization", format!("OAuth {token}"))
        .header("X-Device-Id", device)
        .header("Client-Session-Id", session)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Could not reach Twitch: {e}"))?;
    let v: Value = resp.json().await.map_err(|_| "Twitch sent an unreadable answer".to_string())?;
    // A batched answer is an array; these requests send one operation.
    Ok(match v {
        Value::Array(mut a) if !a.is_empty() => a.remove(0),
        other => other,
    })
}

fn persisted(operation: &str, hash: &str, input: Value) -> Value {
    json!({
        "operationName": operation,
        "variables": { "input": input },
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
    })
}

/// Twitch's web operation that ends a raid countdown early ("Raid Now").
pub const GO_RAID_HASH: &str = "878ca88bed0c5a5f0687ad07562cffc0bf6a3136f15e5015c0f5f5f7f367f70a";
pub const ASSIGN_ROLE_HASH: &str = "2d373c90d0d0e6d4fe771bc6136febe6a148eb3d5700d2a0575883a043fbd581";
pub const UNASSIGN_ROLE_HASH: &str = "5edbf17877acdb91e65243b5148cfd15b98adc6d8f980492dcde9a7f2e8255e2";
pub const ADD_EDITOR_HASH: &str = "3b52bf904ff9ce1b000ac2358080f538fbd1972c1869804f0d0f345d1a56676c";
pub const REMOVE_EDITOR_HASH: &str = "4699d38183050854dba547d07e340e72bf1f04578f1037a38a1189fa1827790f";
/// The role id Twitch uses for lead moderators.
pub const LEAD_MOD_ROLE: &str = "lead_mod";

pub fn go_raid_request(source_id: &str) -> Value {
    persisted("GoRaid", GO_RAID_HASH, json!({ "sourceID": source_id }))
}

pub fn lead_mod_request(assign: bool, channel_id: &str, target_user_id: &str) -> Value {
    let (op, hash) = if assign {
        ("AssignChannelRole", ASSIGN_ROLE_HASH)
    } else {
        ("UnassignChannelRole", UNASSIGN_ROLE_HASH)
    };
    persisted(
        op,
        hash,
        json!({ "channelID": channel_id, "targetUserID": target_user_id, "roleID": LEAD_MOD_ROLE }),
    )
}

pub fn editor_request(add: bool, channel_id: &str, target_login: &str) -> Value {
    let (op, hash) = if add {
        ("AddEditorUser", ADD_EDITOR_HASH)
    } else {
        ("RemoveEditorUser", REMOVE_EDITOR_HASH)
    };
    persisted(op, hash, json!({ "channelID": channel_id, "targetUserLogin": target_login }))
}

/// Read a private GQL write's answer. `payload` names the field under `data`
/// (None when the operation returns nothing worth checking); `flag` is a
/// boolean in it that must be true.
pub fn gql_write_outcome(body: &Value, payload: Option<&str>, flag: Option<&str>, failed: &str) -> Result<(), String> {
    if let Some(msg) = body["errors"][0]["message"].as_str() {
        return Err(match msg {
            "PersistedQueryNotFound" => format!("{failed}: Twitch changed this request, so StreamNook needs an update"),
            "service error" | "service timeout" => format!("{failed}: Twitch had a problem, try again"),
            other => format!("{failed} ({other})"),
        });
    }
    let Some(field) = payload else { return Ok(()) };
    let p = &body["data"][field];
    if p.is_null() {
        return Err(format!("{failed}: Twitch refused it"));
    }
    let err = &p["error"];
    if !err.is_null() {
        let code = err["code"].as_str().or_else(|| err.as_str()).unwrap_or("UNKNOWN");
        return Err(format!("{failed}: {}", role_error_text(code)));
    }
    if let Some(flag) = flag {
        if p[flag].as_bool() != Some(true) {
            return Err(format!("{failed}: Twitch refused it"));
        }
    }
    Ok(())
}

fn role_error_text(code: &str) -> String {
    match code {
        "FORBIDDEN" | "PERMISSION_DENIED" | "UNAUTHORIZED" => "you don't have permission in this channel".into(),
        "TARGET_NOT_FOUND" | "USER_NOT_FOUND" => "that user doesn't exist".into(),
        "TARGET_ALREADY_EDITOR" | "ALREADY_EDITOR" => "they are already an editor".into(),
        "TARGET_NOT_EDITOR" | "NOT_EDITOR" => "they are not an editor".into(),
        "TARGET_NOT_MOD" | "NOT_MODERATOR" => "they need to be a moderator first".into(),
        "ROLE_ALREADY_ASSIGNED" | "ALREADY_ASSIGNED" => "they already have that role".into(),
        "ROLE_NOT_ASSIGNED" | "NOT_ASSIGNED" => "they don't have that role".into(),
        "TARGET_BANNED" => "they are banned in this channel".into(),
        other => other.to_ascii_lowercase().replace('_', " "),
    }
}

/// Read GoRaid's answer: a request-level error, or `goRaid.error` when Twitch
/// took the request and refused it (no raid counting down, not allowed).
pub fn go_raid_outcome(body: &Value) -> Result<(), String> {
    const FAILED: &str = "Could not send the raid";
    gql_write_outcome(body, None, None, FAILED)?;
    let err = &body["data"]["goRaid"]["error"];
    if err.is_null() {
        return Ok(());
    }
    let code = err["code"].as_str().or_else(|| err.as_str()).unwrap_or("UNKNOWN");
    Err(format!("{FAILED}: {}", raid_error_text(code)))
}

fn raid_error_text(code: &str) -> String {
    match code {
        "RAID_NOT_FOUND" | "NOT_FOUND" | "NO_RAID" | "NO_ACTIVE_RAID" | "RAID_DOES_NOT_EXIST" => {
            "no raid is counting down. Start one with /raid <channel>".into()
        }
        "RAID_ALREADY_SENT" | "ALREADY_RAIDED" => "the raid has already gone".into(),
        other => role_error_text(other),
    }
}

/// `/raidsend`: end the running raid countdown now.
pub async fn raid_now(broadcaster_id: &str) -> Result<(), String> {
    require_broadcaster(broadcaster_id, "send a raid").await?;
    let (token, user_id) = private_session().await?;
    if user_id != broadcaster_id {
        return Err("Your Twitch rewards sign-in is a different account than this channel".into());
    }
    let body = private_gql(&token, &go_raid_request(broadcaster_id)).await?;
    go_raid_outcome(&body)
}

/// `/leadmod` and `/unleadmod`.
pub async fn lead_mod(broadcaster_id: &str, target_user_id: &str, assign: bool) -> Result<(), String> {
    require_broadcaster(broadcaster_id, "change lead moderators").await?;
    let (token, user_id) = private_session().await?;
    if user_id != broadcaster_id {
        return Err("Your Twitch rewards sign-in is a different account than this channel".into());
    }
    let body = private_gql(&token, &lead_mod_request(assign, broadcaster_id, target_user_id)).await?;
    if assign {
        gql_write_outcome(&body, Some("assignChannelRole"), Some("isAssigned"), "Could not add the lead moderator")
    } else {
        gql_write_outcome(&body, Some("unassignChannelRole"), Some("isUnassigned"), "Could not remove the lead moderator")
    }
}

/// `/editor` and `/uneditor`.
pub async fn editor(broadcaster_id: &str, target_login: &str, add: bool) -> Result<(), String> {
    require_broadcaster(broadcaster_id, "change editors").await?;
    let (token, user_id) = private_session().await?;
    if user_id != broadcaster_id {
        return Err("Your Twitch rewards sign-in is a different account than this channel".into());
    }
    let body = private_gql(&token, &editor_request(add, broadcaster_id, target_login)).await?;
    if add {
        gql_write_outcome(&body, Some("addEditor"), None, "Could not add the editor")
    } else {
        gql_write_outcome(&body, Some("removeEditor"), None, "Could not remove the editor")
    }
}

// ─────────────────────────────── poll voting ─────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct CachedPoll {
    pub poll_id: String,
    pub title: String,
    pub choices: Vec<(String, String)>,
    /// The whole poll as last sent to the banner, so an end StreamNook makes
    /// itself can be announced with the counts the banner already shows.
    pub last: Value,
}

static POLLS: OnceLock<Mutex<HashMap<String, CachedPoll>>> = OnceLock::new();

fn polls() -> std::sync::MutexGuard<'static, HashMap<String, CachedPoll>> {
    POLLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The poll PubSub announced for `channel_id` (the normalized payload the
/// poll overlay gets). A completed poll is forgotten; a poll StreamNook ended
/// itself is never cached again by a read or a frame that was still on its way.
pub fn note_poll(channel_id: &str, payload: &Value, event: &str) {
    if channel_id.is_empty() {
        return;
    }
    let poll_id = payload["poll_id"].as_str().unwrap_or("");
    if event == "poll-completed" || payload["status"].as_str().is_some_and(|s| s != "ACTIVE") {
        // Only that poll: a newer one may already be running.
        let mut cache = polls();
        if poll_id.is_empty() || cache.get(channel_id).is_some_and(|c| c.poll_id == poll_id) {
            cache.remove(channel_id);
        }
        return;
    }
    if ended_poll_reason(poll_id).is_some() {
        return;
    }
    let choices = payload["choices"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|c| Some((c["id"].as_str()?.to_string(), c["title"].as_str().unwrap_or("").to_string())))
        .collect();
    polls().insert(
        channel_id.to_string(),
        CachedPoll {
            poll_id: poll_id.to_string(),
            title: payload["title"].as_str().unwrap_or("").to_string(),
            choices,
            last: payload.clone(),
        },
    );
}

/// Polls StreamNook ended itself (`/endpoll`, `/cancelpoll`, the moderator
/// menu), with why. The end is announced the moment Twitch accepts it, so a
/// read or a PubSub frame still on its way must neither bring the banner back
/// nor announce the end a second time.
static ENDED_POLLS: OnceLock<Mutex<std::collections::VecDeque<(String, &'static str)>>> = OnceLock::new();
/// One poll at a time per channel; this many covers every chat the app holds.
const ENDED_POLLS_KEPT: usize = 32;

fn ended_polls() -> std::sync::MutexGuard<'static, std::collections::VecDeque<(String, &'static str)>> {
    ENDED_POLLS
        .get_or_init(|| Mutex::new(std::collections::VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Remember that StreamNook ended `poll_id`, and why.
pub fn note_poll_ended(poll_id: &str, reason: &'static str) {
    if poll_id.is_empty() {
        return;
    }
    let mut ended = ended_polls();
    ended.retain(|(p, _)| p != poll_id);
    ended.push_back((poll_id.to_string(), reason));
    while ended.len() > ENDED_POLLS_KEPT {
        ended.pop_front();
    }
}

/// Why StreamNook ended `poll_id`, if it did.
pub fn ended_poll_reason(poll_id: &str) -> Option<&'static str> {
    if poll_id.is_empty() {
        return None;
    }
    ended_polls().iter().find(|(p, _)| p == poll_id).map(|(_, r)| *r)
}

/// The `poll-completed` payload for a poll StreamNook ended: the poll as the
/// banner last had it, with the final per-choice counts from Twitch's answer
/// to the end call laid over it. Twitch's answer counts votes, not voters, so
/// voters stay as last seen. Without a cached copy (the banner never had the
/// poll) it is built from Twitch's answer alone.
pub fn ended_poll_payload(
    channel_id: &str,
    poll_id: &str,
    reason: &str,
    helix_body: &Value,
    last: Option<&Value>,
) -> Value {
    let row = helix_body["data"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["id"].as_str() == Some(poll_id)));
    let mut p = last
        .filter(|l| l["poll_id"].as_str() == Some(poll_id))
        .cloned()
        .unwrap_or_else(|| {
            let r = row.cloned().unwrap_or(Value::Null);
            json!({
                "title": r["title"].as_str().unwrap_or(""),
                "duration_seconds": r["duration"].as_i64().unwrap_or(0),
                "total_voters": 0,
                "total_votes": 0,
                "started_at": r["started_at"].as_str().unwrap_or(""),
                "channel_points_voting": r["channel_points_voting_enabled"].as_bool().unwrap_or(false),
                "channel_points_cost": r["channel_points_per_vote"].as_i64().unwrap_or(0),
                "choices": [],
            })
        });
    if let Some(row) = row {
        let finals: Vec<(String, String, i64)> = row["choices"]
            .as_array()
            .map(|cs| {
                cs.iter()
                    .filter_map(|c| {
                        Some((
                            c["id"].as_str()?.to_string(),
                            c["title"].as_str().unwrap_or("").to_string(),
                            c["votes"].as_i64().unwrap_or(0),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !finals.is_empty() {
            let known = p["choices"].as_array().is_some_and(|cs| !cs.is_empty());
            if known {
                if let Some(cs) = p["choices"].as_array_mut() {
                    for c in cs.iter_mut() {
                        if let Some((_, _, votes)) = finals.iter().find(|(id, _, _)| c["id"].as_str() == Some(id.as_str())) {
                            c["total_votes"] = json!(votes);
                        }
                    }
                }
            } else {
                p["choices"] = Value::Array(
                    finals
                        .iter()
                        .map(|(id, title, votes)| json!({ "id": id, "title": title, "total_votes": votes, "total_voters": 0 }))
                        .collect(),
                );
            }
            p["total_votes"] = json!(finals.iter().map(|(_, _, v)| *v).sum::<i64>());
        }
    }
    p["channel_id"] = json!(channel_id);
    p["poll_id"] = json!(poll_id);
    p["status"] = json!("COMPLETED");
    p["remaining_ms"] = json!(0);
    p["end_reason"] = json!(reason);
    p
}

/// StreamNook ended a poll and Twitch accepted it (`helix_body` is Twitch's
/// answer to the end call; `status` the end asked for, TERMINATED or
/// ARCHIVED). Remember why, forget it as the running poll, and tell every
/// banner now rather than at the next read or a PubSub frame that may never
/// come: an ended poll shows its result, a cancelled one just goes.
pub fn announce_poll_ended(channel_id: &str, poll_id: &str, status: &str, helix_body: &Value) {
    if channel_id.is_empty() || poll_id.is_empty() {
        return;
    }
    let reason = poll_end_reason(None, &status.to_ascii_uppercase());
    note_poll_ended(poll_id, reason);
    let last = active_poll(channel_id).filter(|c| c.poll_id == poll_id).map(|c| c.last);
    let payload = ended_poll_payload(channel_id, poll_id, reason, helix_body, last.as_ref());
    note_poll(channel_id, &payload, "poll-completed");
    emit_poll("poll-completed", &payload);
}

pub fn active_poll(channel_id: &str) -> Option<CachedPoll> {
    polls().get(channel_id).cloned()
}

/// The choice this viewer voted for, per poll id, cast from StreamNook (the
/// poll banner or `/vote`). A vote from StreamNook is the free one Twitch
/// allows per poll (spending channel points for more is not offered), so a
/// remembered vote closes the poll to further votes here.
static MY_POLL_VOTES: OnceLock<Mutex<std::collections::VecDeque<(String, String)>>> = OnceLock::new();
/// Polls are one at a time per channel and end within 30 minutes; this many
/// covers every chat the app can hold.
const MY_POLL_VOTES_KEPT: usize = 32;

fn my_poll_votes() -> std::sync::MutexGuard<'static, std::collections::VecDeque<(String, String)>> {
    MY_POLL_VOTES
        .get_or_init(|| Mutex::new(std::collections::VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The choice id this viewer voted for in `poll_id`, if they voted here.
pub fn my_poll_vote(poll_id: &str) -> Option<String> {
    if poll_id.is_empty() {
        return None;
    }
    my_poll_votes().iter().find(|(p, _)| p == poll_id).map(|(_, c)| c.clone())
}

/// Remember a vote Twitch accepted.
pub fn note_my_vote(poll_id: &str, choice_id: &str) {
    if poll_id.is_empty() || choice_id.is_empty() {
        return;
    }
    let mut votes = my_poll_votes();
    votes.retain(|(p, _)| p != poll_id);
    votes.push_back((poll_id.to_string(), choice_id.to_string()));
    while votes.len() > MY_POLL_VOTES_KEPT {
        votes.pop_front();
    }
}

/// The event the poll banner hears when this viewer's vote lands, from the
/// banner or from `/vote`: `{ channel_id, poll_id, choice_id }`.
pub const POLL_VOTED_EVENT: &str = "poll-voted";

/// Remember an accepted vote and tell every poll banner showing that poll.
pub fn announce_my_vote(channel_id: &str, poll_id: &str, choice_id: &str) {
    use tauri::Emitter;
    note_my_vote(poll_id, choice_id);
    if let Some(app) = crate::services::providers::app_handle() {
        let _ = app.emit(
            POLL_VOTED_EVENT,
            json!({ "channel_id": channel_id, "poll_id": poll_id, "choice_id": choice_id }),
        );
    }
    // The card marks the vote at once, before the next count arrives.
    if let Some(mut last) = active_poll(channel_id).filter(|c| c.poll_id == poll_id).map(|c| c.last) {
        stamp_my_vote(&mut last);
        crate::services::poll_view::publish(&last);
    }
}

/// A normalized poll with this viewer's vote on it as `my_choice_id` (null
/// when they have not voted here), so a banner that mounts mid-poll marks it.
pub fn stamp_my_vote(payload: &mut Value) {
    let mine = payload["poll_id"].as_str().and_then(my_poll_vote);
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("my_choice_id".into(), mine.map(Value::from).unwrap_or(Value::Null));
    }
}

/// Why a poll ended, as the banner treats it: "completed" (its time ran out)
/// and "terminated" (ended early) both keep the result on screen a moment;
/// "archived" (deleted or cancelled, which Twitch hides) closes the banner
/// quietly. Read from the PubSub event type, with the poll's own status
/// winning when it says the poll is gone.
pub fn poll_end_reason(event_type: Option<&str>, status: &str) -> &'static str {
    match status {
        "ARCHIVED" | "MODERATED" | "INVALID" => return "archived",
        "TERMINATED" => return "terminated",
        _ => {}
    }
    match event_type {
        Some("POLL_ARCHIVE") => "archived",
        Some("POLL_TERMINATE") => "terminated",
        _ => "completed",
    }
}

/// How a poll ended, from the status Twitch gives it once it is over. None
/// while Twitch still calls it running or names no status.
pub fn final_status_reason(status: &str) -> Option<&'static str> {
    match status {
        "ARCHIVED" | "MODERATED" | "INVALID" => Some("archived"),
        "TERMINATED" => Some("terminated"),
        "COMPLETED" => Some("completed"),
        _ => None,
    }
}

/// A poll's own status by id (`Query.poll`), for a poll the channel no longer
/// shows. Twitch refuses it anonymously ("unauthenticated"), so it is asked
/// with the rewards sign-in.
pub fn poll_status_request(poll_id: &str) -> Value {
    json!({
        "operationName": "PollStatus",
        "query": "query PollStatus($id: ID!) { poll(id: $id) { id status } }",
        "variables": { "id": poll_id },
    })
}

/// The status in a `poll_status_request` answer.
pub fn poll_status_from_gql(body: &Value) -> Option<String> {
    body["data"]["poll"]["status"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// How a poll that stopped showing ended, in Twitch's own words: by id over
/// Helix when the channel is the signed-in account's own, else `Query.poll`
/// with the rewards sign-in. None when neither can say.
pub async fn fetch_poll_final_reason(channel_id: &str, poll_id: &str) -> Option<&'static str> {
    if poll_id.is_empty() {
        return None;
    }
    if crate::services::account_roster::twitch_user_id().as_deref() == Some(channel_id) {
        match TwitchService::get_poll_status(channel_id, poll_id).await {
            Ok(Some(status)) => {
                if let Some(reason) = final_status_reason(&status) {
                    return Some(reason);
                }
            }
            Ok(None) => {}
            Err(e) => log::debug!("[poll] reading poll {poll_id} by id failed: {e}"),
        }
    }
    let token = crate::services::drops_auth_service::DropsAuthService::get_token().await.ok()?;
    let body = private_gql(&token, &poll_status_request(poll_id)).await.ok()?;
    let status = poll_status_from_gql(&body);
    if status.is_none() {
        log::debug!("[poll] Twitch named no status for poll {poll_id}: {}", body["errors"]);
    }
    status.as_deref().and_then(final_status_reason)
}

/// Why a followed poll ended when Twitch stopped showing it and could not say
/// how it ended, judged by how long it still had to run at the last read: one
/// that vanished with time left was taken down, one at the end of its time
/// ran out.
pub fn vanished_poll_end_reason(remaining_ms_at_last_read: i64, since_last_read_ms: u64) -> &'static str {
    let left = remaining_ms_at_last_read.saturating_sub(since_last_read_ms.min(i64::MAX as u64) as i64);
    // One read interval of slack: the end can land just after a read.
    if left > (POLL_WATCH_SECS as i64) * 1000 {
        "archived"
    } else {
        "completed"
    }
}

/// The poll a channel shows right now, read the way Twitch's own player reads
/// it (`ChannelPollContext_GetViewablePoll`: `user.viewablePoll`), by channel id.
pub fn viewable_poll_request(channel_id: &str) -> Value {
    json!({
        "operationName": "ViewablePoll",
        "query": "query ViewablePoll($id: ID!) { user(id: $id) { id viewablePoll { id title status \
                  durationSeconds startedAt remainingDurationMilliseconds totalVoters votes { total } \
                  settings { communityPointsVotes { isEnabled cost } } \
                  choices { id title totalVoters votes { total } } } } }",
        "variables": { "id": channel_id },
    })
}

/// The poll in a `viewable_poll_request` answer, in the shape PubSub poll
/// events carry (what the poll banner and `note_poll` read). None when the
/// channel shows no poll.
pub fn viewable_poll_payload(channel_id: &str, body: &Value) -> Result<Option<Value>, String> {
    if let Some(msg) = body["errors"][0]["message"].as_str() {
        return Err(format!("Twitch refused the poll lookup ({msg})"));
    }
    let poll = &body["data"]["user"]["viewablePoll"];
    if poll.is_null() || poll["id"].as_str().unwrap_or("").is_empty() {
        return Ok(None);
    }
    let choices: Vec<Value> = poll["choices"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| {
            json!({
                "id": c["id"].as_str().unwrap_or(""),
                "title": c["title"].as_str().unwrap_or(""),
                "total_votes": c["votes"]["total"].as_i64().unwrap_or(0),
                "total_voters": c["totalVoters"].as_i64().unwrap_or(0),
            })
        })
        .collect();
    let points = &poll["settings"]["communityPointsVotes"];
    Ok(Some(json!({
        "channel_id": channel_id,
        "poll_id": poll["id"].as_str().unwrap_or(""),
        "title": poll["title"].as_str().unwrap_or(""),
        "status": poll["status"].as_str().unwrap_or("ACTIVE"),
        "duration_seconds": poll["durationSeconds"].as_i64().unwrap_or(0),
        "remaining_ms": poll["remainingDurationMilliseconds"].as_i64().unwrap_or(0),
        "total_voters": poll["totalVoters"].as_i64().unwrap_or(0),
        "total_votes": poll["votes"]["total"].as_i64().unwrap_or(0),
        "started_at": poll["startedAt"].as_str().unwrap_or(""),
        "channel_points_voting": points["isEnabled"].as_bool().unwrap_or(false),
        "channel_points_cost": points["cost"].as_i64().unwrap_or(0),
        "choices": choices,
    })))
}

/// Ask Twitch for the poll a channel shows. With the rewards sign-in it asks
/// as that account (like the vote itself); without it, anonymously.
pub async fn fetch_viewable_poll(channel_id: &str) -> Result<Option<Value>, String> {
    let request = viewable_poll_request(channel_id);
    let body: Value = match crate::services::drops_auth_service::DropsAuthService::get_token().await {
        Ok(token) => private_gql(&token, &request).await?,
        Err(_) => crate::services::http::client()
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", crate::services::auth_proxy::TWITCH_WEB_CLIENT_ID)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Could not reach Twitch: {e}"))?
            .json()
            .await
            .map_err(|_| "Twitch sent an unreadable answer".to_string())?,
    };
    viewable_poll_payload(channel_id, &body)
}

/// Seconds between two reads of a running poll that PubSub may not be
/// delivering (no rewards sign-in, or a chat that is not the watched stream).
pub const POLL_WATCH_SECS: u64 = 5;
/// Longest a poll is followed: Twitch's longest poll (30 minutes) and a margin.
pub const POLL_WATCH_MAX_SECS: u64 = 1_900;

static POLL_WATCHES: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

fn poll_watches() -> std::sync::MutexGuard<'static, std::collections::HashSet<String>> {
    POLL_WATCHES
        .get_or_init(|| Mutex::new(std::collections::HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn emit_poll(event: &str, payload: &Value) {
    use tauri::Emitter;
    if let Some(app) = crate::services::providers::app_handle() {
        let mut payload = payload.clone();
        stamp_my_vote(&mut payload);
        crate::services::poll_view::publish(&payload);
        let _ = app.emit(event, payload);
    }
}

/// Read a channel's poll now. A running one goes into the `/vote` cache, is
/// announced as `announce` when given, and is followed until it ends (see
/// `spawn_poll_watch`). Returns the poll, running or just ended, if any, with
/// this viewer's vote on it.
pub async fn watch_poll_now(channel_id: &str, announce: Option<&str>) -> Result<Option<Value>, String> {
    if channel_id.is_empty() {
        return Ok(None);
    }
    let mut poll = fetch_viewable_poll(channel_id).await?;
    // A poll StreamNook just ended can still read as running for a moment.
    if let Some(p) = poll.as_mut() {
        if let Some(reason) = p["poll_id"].as_str().and_then(ended_poll_reason) {
            p["status"] = json!("COMPLETED");
            p["end_reason"] = json!(reason);
        }
    }
    if let Some(p) = poll.as_ref().filter(|p| p["status"] == "ACTIVE") {
        note_poll(channel_id, p, "poll-updated");
        if let Some(event) = announce {
            emit_poll(event, p);
        }
        spawn_poll_watch(channel_id.to_string());
    }
    if let Some(p) = poll.as_mut() {
        stamp_my_vote(p);
    }
    Ok(poll)
}

/// Follow a running poll: read it every [`POLL_WATCH_SECS`] and send the same
/// `poll-updated` / `poll-completed` events PubSub sends, so the banner and
/// `/vote` work in any open chat. One watch per channel; PubSub events for the
/// same poll merge with these.
pub fn spawn_poll_watch(channel_id: String) {
    if channel_id.is_empty() || !poll_watches().insert(channel_id.clone()) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        let mut last: Option<(Value, std::time::Instant)> = None;
        let mut failures = 0u32;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(POLL_WATCH_SECS)).await;
            match fetch_viewable_poll(&channel_id).await {
                Ok(Some(p)) if p["status"] == "ACTIVE" => {
                    // StreamNook ended it and this read beat Twitch catching
                    // up: the end was announced already.
                    if p["poll_id"].as_str().is_some_and(|id| ended_poll_reason(id).is_some()) {
                        break;
                    }
                    failures = 0;
                    note_poll(&channel_id, &p, "poll-updated");
                    emit_poll("poll-updated", &p);
                    last = Some((p, std::time::Instant::now()));
                }
                Ok(ended) => {
                    // Still shown with an end status: that status says why.
                    // Gone altogether: how StreamNook ended it, else what
                    // Twitch says of it by id, else judged by the time it had
                    // left.
                    let ended = match ended {
                        Some(p) => {
                            let reason = poll_end_reason(None, p["status"].as_str().unwrap_or(""));
                            Some((p, reason))
                        }
                        None => match last {
                            Some((p, at)) => {
                                let id = p["poll_id"].as_str().unwrap_or("").to_string();
                                let remaining = p["remaining_ms"].as_i64().unwrap_or(0);
                                let since = at.elapsed().as_millis().min(u64::MAX as u128) as u64;
                                let reason = match ended_poll_reason(&id) {
                                    Some(r) => r,
                                    None => match fetch_poll_final_reason(&channel_id, &id).await {
                                        Some(r) => r,
                                        None => vanished_poll_end_reason(remaining, since),
                                    },
                                };
                                Some((p, reason))
                            }
                            None => None,
                        },
                    };
                    if let Some((mut p, reason)) = ended {
                        // PubSub may have announced the end already (it
                        // removes the poll from the cache); say it once.
                        let still_open = active_poll(&channel_id)
                            .is_some_and(|c| Some(c.poll_id.as_str()) == p["poll_id"].as_str());
                        p["status"] = json!("COMPLETED");
                        p["end_reason"] = json!(reason);
                        note_poll(&channel_id, &p, "poll-completed");
                        if still_open {
                            emit_poll("poll-completed", &p);
                        }
                    }
                    break;
                }
                Err(e) => {
                    failures += 1;
                    log::debug!("[poll] reading {channel_id}'s poll failed: {e}");
                    if failures >= 5 {
                        break;
                    }
                }
            }
            if started.elapsed().as_secs() > POLL_WATCH_MAX_SECS {
                break;
            }
        }
        poll_watches().remove(&channel_id);
    });
}

/// Which choice `/vote <arg>` means: its number, its title, or the start of
/// exactly one title.
pub fn pick_choice(poll: &CachedPoll, arg: &str) -> Result<(String, String), String> {
    let arg = arg.trim();
    if arg.is_empty() {
        let list: Vec<String> = poll.choices.iter().enumerate().map(|(i, c)| format!("{} {}", i + 1, c.1)).collect();
        return Err(format!("Say which option: {}", list.join(", ")));
    }
    if let Ok(n) = arg.parse::<usize>() {
        return poll
            .choices
            .get(n.wrapping_sub(1))
            .cloned()
            .ok_or_else(|| format!("This poll has {} options", poll.choices.len()));
    }
    let lower = arg.to_lowercase();
    if let Some(c) = poll.choices.iter().find(|c| c.1.to_lowercase() == lower) {
        return Ok(c.clone());
    }
    let starts: Vec<&(String, String)> = poll.choices.iter().filter(|c| c.1.to_lowercase().starts_with(&lower)).collect();
    match starts.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!("No option matches \"{arg}\"")),
        _ => Err(format!("\"{arg}\" matches more than one option. Use its number")),
    }
}

/// `/delete` refused: Twitch's answer (`TwitchService::delete_chat_message`
/// reports "Twitch rejected delete (HTTP n): body") in plain words. A 404 is
/// Twitch's answer for a message it will not delete: an announcement, one
/// older than 6 hours, or one from the broadcaster or another moderator.
pub fn delete_refusal_text(err: &str) -> String {
    // The status reads "404 Not Found" (reqwest's display), so only its number counts.
    let status = err
        .split("(HTTP ")
        .nth(1)
        .and_then(|s| s.split([' ', ')']).next())
        .and_then(|s| s.parse::<u16>().ok());
    if status == Some(404) {
        "Twitch won't delete that message. Announcements can't be deleted, and neither can messages \
         older than 6 hours or from the broadcaster or another moderator. /clear clears the whole chat \
         if you need it gone."
            .into()
    } else if status == Some(403) {
        "You need to be a moderator here to delete messages.".into()
    } else if status == Some(401) {
        "StreamNook needs moderation access again: sign out and back in to Twitch.".into()
    } else {
        match (status, err.split_once("): ")) {
            (Some(status), Some((_, body))) => format!(
                "Could not delete the message: {}",
                crate::services::twitch_service::helix_refusal_text(status, body)
            ),
            _ => format!("Could not delete the message: {err}"),
        }
    }
}

// ────────────────────────────── chat history ─────────────────────────────────

/// The newest message `login` sent in `channel` (bare login) that the app
/// still holds.
pub fn last_message_of(channel: &str, login: &str) -> Option<crate::services::chat_history::SearchHit> {
    recent_messages_of(channel, login, 1).into_iter().next()
}

/// Up to `limit` of `login`'s newest messages in `channel`, newest first.
pub fn recent_messages_of(channel: &str, login: &str, limit: usize) -> Vec<crate::services::chat_history::SearchHit> {
    let channel = channel.trim_start_matches('#').to_ascii_lowercase();
    crate::services::chat_history::ChatHistory::search(Some(&channel), &format!("from:{login}"), limit)
        .unwrap_or_default()
}

// ─────────────────────────────── mass unban ──────────────────────────────────

pub const MASS_UNBAN_EVENT: &str = "mass-unban-progress";
/// Twitch's Helix budget is 800 points a minute; three a second leaves room for
/// everything else the app does.
pub const MASS_UNBAN_SPACING_MS: u64 = 333;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MassUnbanProgress {
    pub broadcaster_id: String,
    pub unbanned: u32,
    pub failed: u32,
    /// running | done | stopped | error
    pub state: &'static str,
    pub message: String,
}

static UNBAN_JOBS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn unban_jobs() -> std::sync::MutexGuard<'static, HashMap<String, Arc<AtomicBool>>> {
    UNBAN_JOBS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Permanent bans on one page of Helix `moderation/banned` (timeouts end on
/// their own and are left alone), and the cursor to the next page.
pub fn banned_page(body: &Value) -> (Vec<(String, String)>, Option<String>) {
    let bans = body["data"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|b| b["expires_at"].as_str().map(str::is_empty).unwrap_or(true))
        .filter_map(|b| Some((b["user_id"].as_str()?.to_string(), b["user_login"].as_str().unwrap_or("").to_string())))
        .collect();
    let cursor = body["pagination"]["cursor"].as_str().filter(|c| !c.is_empty()).map(str::to_string);
    (bans, cursor)
}

/// One page of permanent and temporary bans, after `cursor` (sent encoded:
/// Helix cursors are base64 and may hold `+`, `/` and `=`).
pub fn banned_page_path(broadcaster_id: &str, cursor: Option<&str>) -> String {
    match cursor {
        Some(c) => format!(
            "moderation/banned?broadcaster_id={broadcaster_id}&first=100&after={}",
            urlencoding::encode(c)
        ),
        None => format!("moderation/banned?broadcaster_id={broadcaster_id}&first=100"),
    }
}

pub fn mass_unban_running(broadcaster_id: &str) -> bool {
    unban_jobs().contains_key(broadcaster_id)
}

/// Stop a running mass unban; false when none was running.
pub fn stop_mass_unban(broadcaster_id: &str) -> bool {
    match unban_jobs().get(broadcaster_id) {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

/// `/massunban`: lift every permanent ban in the broadcaster's own channel,
/// three a second, reporting progress as `MASS_UNBAN_EVENT`.
pub async fn start_mass_unban<F>(broadcaster_id: String, emit: F) -> Result<(), String>
where
    F: Fn(MassUnbanProgress) + Send + 'static,
{
    require_broadcaster(&broadcaster_id, "unban everyone").await?;
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = unban_jobs();
        if jobs.contains_key(&broadcaster_id) {
            return Err("A mass unban is already running here. Stop it with /massunban stop".into());
        }
        jobs.insert(broadcaster_id.clone(), stop.clone());
    }
    tokio::spawn(async move {
        let progress = |unbanned, failed, state, message: String| MassUnbanProgress {
            broadcaster_id: broadcaster_id.clone(),
            unbanned,
            failed,
            state,
            message,
        };
        let (mut unbanned, mut failed) = (0u32, 0u32);
        let mut skip: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut cursor: Option<String> = None;
        let outcome: Result<&'static str, String> = 'run: loop {
            let path = banned_page_path(&broadcaster_id, cursor.as_deref());
            // A 429 waits for Twitch's reset and tries again; only a real
            // refusal ends the run.
            let page = match helix_paced(Method::GET, &path).await {
                Ok(p) => p,
                Err(e) => break Err(e),
            };
            let (bans, next) = banned_page(&page);
            let fresh: Vec<(String, String)> = bans.into_iter().filter(|b| !skip.contains(&b.0)).collect();
            if fresh.is_empty() {
                match next {
                    Some(n) => {
                        cursor = Some(n);
                        continue;
                    }
                    None => break Ok("done"),
                }
            }
            for (user_id, login) in fresh {
                if stop.load(Ordering::SeqCst) {
                    break 'run Ok("stopped");
                }
                let del = format!("moderation/bans?broadcaster_id={broadcaster_id}&moderator_id={broadcaster_id}&user_id={user_id}");
                match helix_paced(Method::DELETE, &del).await {
                    Ok(_) => unbanned += 1,
                    Err(e) => {
                        log::warn!("[massunban] {login}: {e}");
                        failed += 1;
                        skip.insert(user_id);
                    }
                }
                if (unbanned + failed) % 25 == 0 {
                    emit(progress(unbanned, failed, "running", format!("Unbanned {unbanned} so far...")));
                }
                tokio::time::sleep(std::time::Duration::from_millis(MASS_UNBAN_SPACING_MS)).await;
            }
            // Unbanned users leave the list, so the next read starts over;
            // only the ones that failed are stepped past.
            cursor = None;
        };
        unban_jobs().remove(&broadcaster_id);
        let tail = if failed > 0 { format!(", {failed} could not be unbanned") } else { String::new() };
        emit(match outcome {
            Ok("stopped") => progress(unbanned, failed, "stopped", format!("Mass unban stopped after {unbanned}{tail}.")),
            Ok(_) => progress(unbanned, failed, "done", format!("Mass unban finished: {unbanned} unbanned{tail}.")),
            Err(e) => progress(unbanned, failed, "error", format!("Mass unban stopped after {unbanned}: {e}")),
        });
    });
    Ok(())
}

// ──────────────────────────────── low trust ──────────────────────────────────

/// `/lowtrust`: the monitored and restricted users Twitch reported in this
/// channel this session, each with their newest messages.
pub async fn low_trust_lines(channel: &str) -> Result<Vec<String>, String> {
    let flagged = crate::services::suspicious_users::SuspiciousUsers::list(channel);
    if flagged.is_empty() {
        return Ok(vec![
            "No monitored or restricted users have been seen in this channel yet. They show here once Twitch reports one."
                .into(),
        ]);
    }
    let ids: Vec<String> = flagged.iter().map(|f| f.0.clone()).collect();
    let users = users_by_ids(&ids).await.unwrap_or_default();
    let mut out = vec![format!("{} low-trust users in this channel:", flagged.len())];
    for (id, status) in flagged {
        let user = users.iter().find(|u| u["id"].as_str() == Some(id.as_str()));
        let login = user.and_then(|u| u["login"].as_str()).unwrap_or(&id).to_string();
        let name = user.map(display).unwrap_or_else(|| id.clone());
        let recent = recent_messages_of(channel, &login, 3);
        if recent.is_empty() {
            out.push(format!("{name} ({status}): no recent messages"));
        } else {
            let texts: Vec<String> = recent.iter().map(|m| format!("\"{}\"", m.content)).collect();
            out.push(format!("{name} ({status}): {}", texts.join(" / ")));
        }
    }
    Ok(out)
}

// ─────────────────────────── files the commands write ────────────────────────

fn data_dir() -> Result<PathBuf, String> {
    crate::services::cache_service::get_app_data_dir().map_err(|e| e.to_string())
}

pub fn highlights_file() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("stream_highlights.txt"))
}

pub fn exports_dir() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("exports"))
}

/// One line of the stream highlights file.
pub fn highlight_line(now: &str, channel: &str, uptime: Option<&str>, title: Option<&str>, comment: &str) -> String {
    let mut parts = vec![now.to_string(), channel.to_string()];
    if let Some(u) = uptime.filter(|u| !u.is_empty()) {
        parts.push(format!("uptime {u}"));
    }
    if let Some(t) = title.filter(|t| !t.is_empty()) {
        parts.push(format!("\"{}\"", t.replace(['\r', '\n'], " ")));
    }
    if !comment.trim().is_empty() {
        parts.push(comment.trim().replace(['\r', '\n'], " "));
    }
    parts.join(" | ")
}

/// Stream uptime as "1h 05m" from Helix `started_at`.
pub fn uptime_text(started_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - started_at).num_seconds().max(0);
    format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
}

/// `/addstreamhighlight [comment]`: append the moment to a text file.
pub async fn add_stream_highlight(channel_login: &str, comment: &str) -> Result<String, String> {
    let stream = helix(Method::GET, &format!("streams?user_login={channel_login}"), None)
        .await
        .ok()
        .and_then(|v| v["data"].as_array().and_then(|a| a.first()).cloned());
    let now = Utc::now();
    let uptime = stream.as_ref().and_then(|s| parse_time(&s["started_at"])).map(|t| uptime_text(t, now));
    let title = stream.as_ref().and_then(|s| s["title"].as_str().map(str::to_string));
    let local = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let line = highlight_line(&local, channel_login, uptime.as_deref(), title.as_deref(), comment);
    let path = highlights_file()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Could not open the highlights file: {e}"))?;
    writeln!(f, "{line}").map_err(|e| format!("Could not write the highlight: {e}"))?;
    Ok(match uptime {
        Some(u) => format!("Highlight saved at {u} of the stream. /openstreamhighlights shows the list."),
        None => "Highlight saved (the stream looks offline). /openstreamhighlights shows the list.".into(),
    })
}

/// The endings `/exporttext` writes and `/openfile` reveals: plain text only,
/// so neither command can leave behind a file the system would run.
pub const EXPORT_EXTENSIONS: [&str; 5] = ["txt", "csv", "log", "md", "json"];

/// What `/exporttext` and `/openfile` say when a file name is refused.
pub const EXPORT_NAME_RULE: &str =
    "Use a plain file name (letters, digits, spaces, - and _) ending in .txt, .csv, .log, .md or .json; no ending means .txt";

/// A file name `/exporttext` and `/openfile` may use: letters, digits, space,
/// `-`, `_` and `.`, no path, one of [`EXPORT_EXTENSIONS`] (`.txt` added when
/// there is none), never a Windows device name, never ending in a dot or space.
pub fn safe_export_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && !name.ends_with(['.', ' '])
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'));
    if !ok {
        return None;
    }
    let name = if name.contains('.') { name.to_string() } else { format!("{name}.txt") };
    let (stem, ext) = name.rsplit_once('.')?;
    if !EXPORT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
        return None;
    }
    // Windows keeps device names reserved whatever follows the first dot
    // ("CON.txt", "nul.log"), and ignores trailing spaces before it.
    let device = stem.split('.').next().unwrap_or(stem).trim_end().to_ascii_uppercase();
    let numbered = (device.starts_with("COM") || device.starts_with("LPT"))
        && device.len() == 4
        && device.as_bytes()[3].is_ascii_digit();
    if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered {
        return None;
    }
    Some(name)
}

/// `/exporttext <file> <text>`: append a line to a file in the exports folder.
pub fn export_text(file: &str, text: &str) -> Result<PathBuf, String> {
    let name = safe_export_name(file).ok_or(EXPORT_NAME_RULE)?;
    let dir = exports_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    writeln!(f, "{text}").map_err(|e| e.to_string())?;
    Ok(path)
}

// ──────────────────────────────── raw IRC ────────────────────────────────────

/// What `/raw` may send: one line, inside IRC's length limit, never a line
/// that would replace the session's own login or end it, and never a JOIN or
/// PART behind the back of the app's own channel bookkeeping. The verb is
/// read past optional `@tags` and `:prefix` words, the way the server reads it.
pub fn check_raw_line(line: &str) -> Result<String, String> {
    let line = line.trim();
    if line.is_empty() {
        return Err("Usage: /raw <IRC line>".into());
    }
    if line.contains(['\r', '\n', '\0']) {
        return Err("A raw line must be a single line".into());
    }
    if line.len() > 510 {
        return Err("IRC lines are limited to 510 bytes".into());
    }
    let mut words = line.split_whitespace().peekable();
    if words.peek().is_some_and(|w| w.starts_with('@')) {
        words.next();
    }
    if words.peek().is_some_and(|w| w.starts_with(':')) {
        words.next();
    }
    let verb = words.next().unwrap_or("").to_ascii_uppercase();
    match verb.as_str() {
        "" => Err("Usage: /raw <IRC line>".into()),
        "PASS" | "NICK" | "USER" | "CAP" | "QUIT" => {
            Err(format!("{verb} would break the chat connection, so /raw won't send it"))
        }
        "PART" => Err("A raw PART leaves the chat without StreamNook knowing. Use /part to close a chat".into()),
        "JOIN" => Err("A raw JOIN opens a chat StreamNook doesn't show. Use /join to open a chat".into()),
        _ => Ok(line.to_string()),
    }
}

// ───────────────────────────── command lines ─────────────────────────────────

/// Split a `/proc` command line into words, honouring double quotes.
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// The first `PATH` entry holding `program` (with the platform's executable
/// suffixes on Windows).
pub fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(|e| e.to_ascii_lowercase())
            .chain(std::iter::once(String::new()))
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// ──────────────────────────────────── lurk ───────────────────────────────────

/// What `/lurk` says. Each reads after the sender's name as a `/me` action
/// ("br_winters melts into the shadows"), and as plain words where a chat has
/// no actions.
pub const LURK_LINES: &[&str] = &[
    "is now lurking",
    "fades into the background",
    "melts into the shadows",
    "slips into lurk mode",
    "goes full ninja",
    "becomes one with the chat",
    "tiptoes away from the keyboard",
    "is lurking with intent",
    "is watching quietly from the bushes",
    "puts on the invisibility cloak",
    "turns the volume up and the typing down",
    "is now a silent supporter",
    "leaves the stream running and wanders off",
    "is lurking, but the view still counts",
    "dissolves into the viewer count",
    "hides behind the chat box",
    "goes into stealth mode",
    "retreats to the lurk cave",
    "is AFK, but the tab stays open",
    "switches to passive viewing",
    "lurks responsibly",
    "is a professional lurker now",
    "stays for the vibes and leaves the chatting to you",
    "is lurking from the back row",
    "goes quiet but never leaves",
    "joins the lurker guild",
    "sneaks off with the stream still on",
    "has entered lurk-o-vision",
    "is now part of the furniture",
    "becomes a quiet little gremlin in the corner",
    "is now a ghost in the chat",
    "is camouflaged as a regular viewer",
    "switches to background mode",
    "watches from a safe distance",
    "is lurking hard",
    "parks in the back and enjoys the show",
    "goes radio silent",
    "blends in with the wallpaper",
    "clocks out of chat but stays on stream",
    "is here in spirit (and in the viewer count)",
];

/// The `/lurk` message built from line `index`: a `/me` action on Twitch,
/// plain words elsewhere, with the viewer's note after a colon.
pub fn lurk_message(index: usize, note: Option<&str>, action: bool) -> String {
    let line = LURK_LINES[index % LURK_LINES.len()];
    let said = match note.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => format!("{line}: {n}"),
        None => line.to_string(),
    };
    if action {
        format!("/me {said}")
    } else {
        said
    }
}

/// A random `/lurk` message, never the same line twice in a row.
pub fn lurk(note: Option<&str>, action: bool) -> String {
    static LAST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
    let n = LURK_LINES.len();
    let last = LAST.load(Ordering::Relaxed);
    let mut index = (rand::random::<u64>() % n as u64) as usize;
    if index == last {
        index = (index + 1 + (rand::random::<u64>() % (n as u64 - 1)) as usize) % n;
    }
    LAST.store(index, Ordering::Relaxed);
    lurk_message(index, note, action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn lurk_is_an_action_on_twitch_and_words_elsewhere() {
        assert_eq!(lurk_message(0, None, true), "/me is now lurking");
        assert_eq!(lurk_message(0, Some("  making dinner "), true), "/me is now lurking: making dinner");
        assert_eq!(lurk_message(0, Some("   "), false), "is now lurking");
        assert_eq!(lurk_message(LURK_LINES.len() + 2, None, false), LURK_LINES[2]);
    }

    #[test]
    fn lurk_never_repeats_back_to_back() {
        let mut prev = lurk(None, false);
        for _ in 0..200 {
            let next = lurk(None, false);
            assert_ne!(next, prev);
            prev = next;
        }
    }

    #[test]
    fn lurk_lines_read_after_a_name() {
        for line in LURK_LINES {
            assert!(!line.is_empty() && line.chars().next().unwrap().is_lowercase(), "{line}");
            assert!(!line.contains('\u{2014}') && !line.ends_with('.'), "{line}");
        }
    }

    fn t(y: i32, mo: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, 12, 0, 0).unwrap()
    }

    fn fixture(name: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/services/fixtures/slash_commands")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", path.display()))).unwrap()
    }

    #[test]
    fn logins_are_cleaned() {
        assert_eq!(clean_login("@SomeOne"), Some("someone".into()));
        assert_eq!(clean_login("#chan_1"), Some("chan_1".into()));
        assert_eq!(clean_login(""), None);
        assert_eq!(clean_login("no spaces"), None);
        assert_eq!(clean_login("a".repeat(26).as_str()), None);
    }

    #[test]
    fn role_lists_name_the_group_or_say_why_not() {
        let mods = ["alpha".to_string(), "beta".to_string()];
        assert_eq!(
            role_list_lines(ChannelRole::Moderators, Some(&mods), true, &[]),
            ["The moderators of this channel are: alpha, beta"]
        );
        assert_eq!(role_list_lines(ChannelRole::Vips, Some(&[]), true, &[]), ["There are no VIPs of this channel."]);
        // Twitch refused: signed out asks for a sign-in, signed in says Twitch refused,
        // and either way the members in chat now are still named.
        assert_eq!(
            role_list_lines(ChannelRole::Moderators, None, false, &[]),
            ["Sign in to see this channel's moderator list."]
        );
        assert_eq!(
            role_list_lines(ChannelRole::Moderators, None, true, &["gamma".to_string()]),
            ["Twitch would not share this channel's moderator list. In chat now: gamma"]
        );
    }

    #[test]
    fn spans_read_like_people_say_them() {
        assert_eq!(span_words(t(2020, 1, 15), t(2022, 4, 20)), "2 years, 3 months, 5 days");
        assert_eq!(span_words(t(2024, 1, 1), t(2024, 2, 1)), "1 month");
        assert_eq!(span_words(t(2024, 1, 1), t(2024, 1, 4)), "3 days");
        let a = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        assert_eq!(span_words(a, Utc.with_ymd_and_hms(2024, 1, 1, 12, 5, 0).unwrap()), "2 hours, 5 minutes");
        assert_eq!(span_words(a, a), "less than a minute");
    }

    #[test]
    fn grouped_numbers() {
        assert_eq!(group(0), "0");
        assert_eq!(group(1234567), "1,234,567");
        assert_eq!(group(-1000), "-1,000");
    }

    #[test]
    fn followage_lines() {
        assert_eq!(
            followage_text("You", "Chan", Some(t(2023, 1, 1)), t(2024, 3, 1)),
            "You have followed Chan for 1 year, 2 months (since 2023-01-01)."
        );
        assert_eq!(followage_text("You", "Chan", None, t(2024, 1, 1)), "You do not follow Chan.");
        assert_eq!(followage_text("bob", "Chan", None, t(2024, 1, 1)), "bob does not follow Chan.");
        assert_eq!(
            followage_unknown_text("bob", "Chan"),
            "StreamNook can't see whether bob follows Chan: Twitch only shares a channel's followers with its moderators."
        );
    }

    #[test]
    fn follower_and_viewer_lines() {
        assert_eq!(follower_count_text("Chan", Some(12345)), "Chan has 12,345 followers.");
        assert_eq!(follower_count_text("Chan", Some(1)), "Chan has 1 follower.");
        let live = fixture("streams_live.json");
        assert_eq!(viewer_count_text("Chan", live["data"].get(0)), "Chan is live with 4,321 viewers.");
        assert_eq!(viewer_count_text("Chan", None), "Chan is offline.");
    }

    #[test]
    fn account_age_line() {
        assert_eq!(
            account_age_text("Bob", Some(t(2019, 8, 1)), t(2024, 10, 1)),
            "Bob's account is 5 years, 2 months old (created 2019-08-01)."
        );
    }

    #[test]
    fn founders_from_ivr() {
        let body = fixture("ivr_founders.json");
        assert_eq!(
            founders_lines("chan", &body),
            vec!["The founders of chan (2): Alpha, beta_b (no longer subscribed)".to_string()]
        );
        assert_eq!(founders_lines("chan", &json!({ "founders": [] })), vec!["chan has no founders.".to_string()]);
    }

    #[test]
    fn name_history_skips_the_current_name_and_blanks() {
        let body = fixture("name_history.json");
        assert_eq!(
            name_history_lines("Newname", &body),
            vec!["Earlier names of Newname: oldname (2017-03-11 to 2021-05-02)".to_string()]
        );
        assert_eq!(name_history_lines("x", &json!([])), vec!["No earlier names are known for x.".to_string()]);
    }

    #[test]
    fn rules_request_and_answer() {
        let req = rules_request("chan");
        assert_eq!(req["variables"]["login"], "chan");
        assert!(req["query"].as_str().unwrap().contains("chatSettings { rules }"));
        let lines = rules_lines("chan", &fixture("gql_rules.json")).unwrap();
        assert_eq!(lines, vec!["Chan's chat rules:", "1. English please", "2. Fresh memes"]);
        let none = rules_lines("chan", &json!({ "data": { "user": { "displayName": "Chan", "chatSettings": { "rules": [] } } } })).unwrap();
        assert_eq!(none, vec!["Chan has not set any chat rules."]);
        assert!(rules_lines("nobody", &json!({ "data": { "user": null } })).is_err());
    }

    #[test]
    fn shared_chat_marks_the_host() {
        let users = vec![
            json!({ "id": "1", "login": "a", "display_name": "A" }),
            json!({ "id": "2", "login": "b", "display_name": "B" }),
        ];
        assert_eq!(shared_chat_lines("A", Some("2"), &users), vec!["A is sharing chat with: A, B (host)".to_string()]);
        assert_eq!(shared_chat_lines("A", None, &[]), vec!["A is not in a Shared Chat session.".to_string()]);
    }

    #[test]
    fn raid_candidates_are_busiest_first_without_self() {
        let mk = |login: &str, viewers: u32| crate::models::stream::TwitchStream {
            user_login: login.into(),
            viewer_count: viewers,
            game_name: "Chess".into(),
            ..serde_json::from_value(json!({
                "id": "", "user_id": "", "user_name": "", "user_login": "", "title": "", "viewer_count": 0,
                "game_id": "", "game_name": "", "thumbnail_url": "", "started_at": ""
            }))
            .unwrap()
        };
        let lines = raid_candidate_lines(&[mk("small", 5), mk("me", 900), mk("big", 1200)], "me", 10);
        assert_eq!(lines[0], "2 followed channels are live. Raid one with /raid <channel>:");
        assert_eq!(lines[1], "big (1,200 viewers, Chess)");
        assert_eq!(lines[2], "small (5 viewers, Chess)");
        assert_eq!(lines.len(), 3);
        // More live than shown: the header's count is accounted for.
        let many: Vec<_> = (0..14).map(|i| mk(&format!("c{i}"), 100 + i)).collect();
        let lines = raid_candidate_lines(&many, "me", 10);
        assert_eq!(lines[0], "14 followed channels are live. Raid one with /raid <channel>:");
        assert_eq!(lines.len(), 12);
        assert_eq!(lines[11], "...and 4 more, with fewer viewers.");
    }

    #[test]
    fn private_requests_match_the_web_operations() {
        let r = go_raid_request("123");
        assert_eq!(r["operationName"], "GoRaid");
        assert_eq!(r["variables"]["input"]["sourceID"], "123");
        assert_eq!(r["extensions"]["persistedQuery"]["sha256Hash"], GO_RAID_HASH);

        let a = lead_mod_request(true, "1", "2");
        assert_eq!(a["operationName"], "AssignChannelRole");
        assert_eq!(a["variables"]["input"], json!({ "channelID": "1", "targetUserID": "2", "roleID": "lead_mod" }));
        assert_eq!(lead_mod_request(false, "1", "2")["operationName"], "UnassignChannelRole");

        let e = editor_request(true, "1", "bob");
        assert_eq!(e["operationName"], "AddEditorUser");
        assert_eq!(e["variables"]["input"], json!({ "channelID": "1", "targetUserLogin": "bob" }));
        assert_eq!(editor_request(false, "1", "bob")["extensions"]["persistedQuery"]["sha256Hash"], REMOVE_EDITOR_HASH);
    }

    #[test]
    fn private_write_answers() {
        assert!(gql_write_outcome(&fixture("gql_assign_role_ok.json"), Some("assignChannelRole"), Some("isAssigned"), "x").is_ok());
        let err = gql_write_outcome(&fixture("gql_assign_role_error.json"), Some("assignChannelRole"), Some("isAssigned"), "Could not add")
            .unwrap_err();
        assert_eq!(err, "Could not add: they need to be a moderator first");
        let stale = gql_write_outcome(&json!({ "errors": [{ "message": "PersistedQueryNotFound" }] }), None, None, "Could not send");
        assert!(stale.unwrap_err().contains("needs an update"));
        assert!(gql_write_outcome(&json!({ "data": { "goRaid": null } }), None, None, "x").is_ok());
        assert!(gql_write_outcome(&json!({ "data": { "addEditor": null } }), Some("addEditor"), None, "x").is_err());
    }

    #[test]
    fn polls_are_cached_and_choices_picked() {
        let payload = fixture("poll_payload.json");
        note_poll("c1", &payload, "poll-created");
        let poll = active_poll("c1").unwrap();
        assert_eq!(poll.poll_id, "poll-1");
        assert_eq!(pick_choice(&poll, "2").unwrap().0, "b");
        assert_eq!(pick_choice(&poll, "yes").unwrap().0, "a");
        assert_eq!(pick_choice(&poll, "may").unwrap().0, "c");
        assert!(pick_choice(&poll, "4").unwrap_err().contains("3 options"));
        assert!(pick_choice(&poll, "").unwrap_err().starts_with("Say which option: 1 Yes, 2 No, 3 Maybe"));
        assert!(pick_choice(&poll, "zzz").is_err());
        note_poll("c1", &payload, "poll-completed");
        assert!(active_poll("c1").is_none());
    }

    #[test]
    fn a_vote_is_remembered_per_poll_and_stamped_on_the_poll() {
        let mut payload = fixture("poll_payload.json");
        stamp_my_vote(&mut payload);
        assert_eq!(payload["my_choice_id"], Value::Null);

        note_my_vote("poll-1", "b");
        assert_eq!(my_poll_vote("poll-1").as_deref(), Some("b"));
        assert_eq!(my_poll_vote("poll-2"), None);
        stamp_my_vote(&mut payload);
        assert_eq!(payload["my_choice_id"], "b");

        // Bounded: the oldest polls are forgotten first.
        for i in 0..MY_POLL_VOTES_KEPT {
            note_my_vote(&format!("later-{i}"), "a");
        }
        assert_eq!(my_poll_vote("poll-1"), None);
        assert_eq!(my_poll_vote(&format!("later-{}", MY_POLL_VOTES_KEPT - 1)).as_deref(), Some("a"));
    }

    #[test]
    fn a_cancelled_poll_ends_quietly_and_a_finished_one_shows_its_result() {
        assert_eq!(poll_end_reason(Some("POLL_COMPLETE"), "COMPLETED"), "completed");
        assert_eq!(poll_end_reason(Some("POLL_TERMINATE"), "TERMINATED"), "terminated");
        assert_eq!(poll_end_reason(Some("POLL_ARCHIVE"), "ARCHIVED"), "archived");
        // The status wins when it says the poll is gone.
        assert_eq!(poll_end_reason(Some("POLL_TERMINATE"), "ARCHIVED"), "archived");
        assert_eq!(poll_end_reason(Some("POLL_ARCHIVE"), ""), "archived");
        assert_eq!(poll_end_reason(None, "MODERATED"), "archived");
        assert_eq!(poll_end_reason(None, "COMPLETED"), "completed");

        // A followed poll that vanished: with minutes left it was taken down;
        // at the end of its time it ran out.
        assert_eq!(vanished_poll_end_reason(120_000, 5_000), "archived");
        assert_eq!(vanished_poll_end_reason(4_000, 5_000), "completed");
        assert_eq!(vanished_poll_end_reason(9_000, 5_000), "completed");
        assert_eq!(vanished_poll_end_reason(0, 0), "completed");

        // Twitch's own status, read by id once the poll stopped showing.
        assert_eq!(final_status_reason("ARCHIVED"), Some("archived"));
        assert_eq!(final_status_reason("MODERATED"), Some("archived"));
        assert_eq!(final_status_reason("TERMINATED"), Some("terminated"));
        assert_eq!(final_status_reason("COMPLETED"), Some("completed"));
        assert_eq!(final_status_reason("ACTIVE"), None);
        assert_eq!(final_status_reason(""), None);
        let ask = poll_status_request("poll-9");
        assert_eq!(ask["variables"]["id"], "poll-9");
        assert!(ask["query"].as_str().unwrap().contains("poll(id: $id) { id status }"));
        assert_eq!(
            poll_status_from_gql(&json!({ "data": { "poll": { "id": "poll-9", "status": "ARCHIVED" } } })).as_deref(),
            Some("ARCHIVED")
        );
        let refused = json!({ "errors": [{ "message": "unauthenticated" }], "data": { "poll": null } });
        assert_eq!(poll_status_from_gql(&refused), None);
    }

    #[test]
    fn a_poll_streamnook_cancels_closes_at_once_and_stays_closed() {
        let mut payload = fixture("poll_payload.json");
        payload["channel_id"] = json!("c-cancel");
        payload["poll_id"] = json!("poll-cancel");
        payload["total_voters"] = json!(1);
        payload["choices"][1]["total_votes"] = json!(1);
        note_poll("c-cancel", &payload, "poll-updated");
        assert_eq!(active_poll("c-cancel").unwrap().last["total_voters"], 1);

        // Twitch accepted the cancel: remembered, no longer the running poll.
        let helix = json!({ "data": [{ "id": "poll-cancel", "status": "ARCHIVED", "choices": [
            { "id": "a", "title": "Yes", "votes": 0 }, { "id": "b", "title": "No", "votes": 1 }, { "id": "c", "title": "Maybe", "votes": 0 }
        ] }] });
        announce_poll_ended("c-cancel", "poll-cancel", "ARCHIVED", &helix);
        assert_eq!(ended_poll_reason("poll-cancel"), Some("archived"));
        assert!(active_poll("c-cancel").is_none());

        // A read or a frame that was still on its way does not bring it back.
        note_poll("c-cancel", &payload, "poll-updated");
        assert!(active_poll("c-cancel").is_none());
        // A different poll in the same channel still caches.
        let mut next = payload.clone();
        next["poll_id"] = json!("poll-next");
        note_poll("c-cancel", &next, "poll-created");
        assert_eq!(active_poll("c-cancel").unwrap().poll_id, "poll-next");
        // A late end for the cancelled poll leaves the newer one alone.
        let mut late = payload.clone();
        late["status"] = json!("ARCHIVED");
        note_poll("c-cancel", &late, "poll-completed");
        assert_eq!(active_poll("c-cancel").unwrap().poll_id, "poll-next");
    }

    #[test]
    fn an_ended_poll_is_announced_with_its_reason_and_final_counts() {
        let mut last = fixture("poll_payload.json");
        last["total_voters"] = json!(2);
        last["choices"][0]["total_votes"] = json!(1);
        let helix = json!({ "data": [{ "id": "poll-1", "title": "Best?", "status": "TERMINATED", "duration": 60,
            "choices": [{ "id": "a", "title": "Yes", "votes": 3 }, { "id": "b", "title": "No", "votes": 1 }, { "id": "c", "title": "Maybe", "votes": 0 }] }] });

        // /endpoll: the result stays on screen, with Twitch's final counts.
        let p = ended_poll_payload("c1", "poll-1", "terminated", &helix, Some(&last));
        assert_eq!(p["end_reason"], "terminated");
        assert_eq!(p["status"], "COMPLETED");
        assert_eq!(p["remaining_ms"], 0);
        assert_eq!(p["choices"][0]["total_votes"], 3);
        assert_eq!(p["choices"][1]["total_votes"], 1);
        assert_eq!(p["choices"][0]["title"], "Yes");
        assert_eq!(p["total_votes"], 4);
        // Voters are not in Twitch's answer: as last seen.
        assert_eq!(p["total_voters"], 2);

        // The banner never had the poll: built from Twitch's answer alone.
        let bare = ended_poll_payload("c1", "poll-1", "terminated", &helix, None);
        assert_eq!(bare["title"], "Best?");
        assert_eq!(bare["choices"].as_array().unwrap().len(), 3);
        assert_eq!(bare["choices"][0]["total_votes"], 3);
        assert_eq!(bare["total_votes"], 4);
        assert_eq!(bare["channel_id"], "c1");

        // A cached copy of another poll is not used.
        let mut other = last.clone();
        other["poll_id"] = json!("poll-old");
        let p = ended_poll_payload("c1", "poll-1", "archived", &json!({}), Some(&other));
        assert_eq!(p["poll_id"], "poll-1");
        assert_eq!(p["end_reason"], "archived");
        assert_eq!(p["choices"].as_array().unwrap().len(), 0);
        assert_eq!(p["title"], "");

        // The status the end asked for picks the reason.
        assert_eq!(poll_end_reason(None, "TERMINATED"), "terminated");
        assert_eq!(poll_end_reason(None, "ARCHIVED"), "archived");
    }

    #[test]
    fn banned_pages_keep_permanent_bans_only() {
        let (bans, cursor) = banned_page(&fixture("helix_banned.json"));
        assert_eq!(bans, vec![("10".to_string(), "perma".to_string())]);
        assert_eq!(cursor.as_deref(), Some("abc"));
        let (_, none) = banned_page(&json!({ "data": [], "pagination": {} }));
        assert!(none.is_none());
    }

    #[test]
    fn highlight_lines_and_uptime() {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 11, 5, 0).unwrap();
        assert_eq!(uptime_text(start, now), "1h 05m");
        assert_eq!(
            highlight_line("2024-01-01 11:05:00", "chan", Some("1h 05m"), Some("Ranked\ngrind"), " clutch "),
            "2024-01-01 11:05:00 | chan | uptime 1h 05m | \"Ranked grind\" | clutch"
        );
        assert_eq!(highlight_line("t", "chan", None, None, ""), "t | chan");
    }

    #[test]
    fn export_names_stay_in_the_folder() {
        assert_eq!(safe_export_name("notes"), Some("notes.txt".into()));
        assert_eq!(safe_export_name("log.csv"), Some("log.csv".into()));
        assert_eq!(safe_export_name("../evil"), None);
        assert_eq!(safe_export_name("a/b"), None);
        assert_eq!(safe_export_name("C:\\x"), None);
        assert_eq!(safe_export_name(".hidden"), None);
        assert_eq!(safe_export_name(""), None);
    }

    #[test]
    fn export_names_are_text_files_only() {
        for ok in ["ideas", "ideas.md", "log.LOG", "data.json", "my notes"] {
            assert!(safe_export_name(ok).is_some(), "{ok}");
        }
        assert_eq!(safe_export_name("my notes"), Some("my notes.txt".into()));
        // Anything the system could run, or open as a shortcut.
        for bad in ["run.bat", "run.cmd", "x.ps1", "x.vbs", "x.js", "x.hta", "link.url", "app.exe", "v1.2"] {
            assert_eq!(safe_export_name(bad), None, "{bad}");
        }
        // Windows device names, with or without an ending, and trailing dots.
        for bad in ["CON", "con.txt", "nul.log", "COM1", "lpt9.md", "aux .txt", "notes.", "notes.txt."] {
            assert_eq!(safe_export_name(bad), None, "{bad}");
        }
        // Names that only start like a device name are fine.
        assert_eq!(safe_export_name("console"), Some("console.txt".into()));
        assert_eq!(safe_export_name("com10"), Some("com10.txt".into()));
    }

    #[test]
    fn raw_lines_are_checked() {
        assert_eq!(check_raw_line(" PRIVMSG #a :hi "), Ok("PRIVMSG #a :hi".into()));
        assert!(check_raw_line("NICK other").is_err());
        assert!(check_raw_line("quit").is_err());
        assert!(check_raw_line("PRIVMSG #a :x\r\nQUIT").is_err());
        assert!(check_raw_line("").is_err());
        assert!(check_raw_line(&"x".repeat(511)).is_err());
    }

    #[test]
    fn raw_verbs_are_read_past_tags_and_prefix() {
        for bad in ["@client-nonce=1 QUIT", ":me!me@me.tmi.twitch.tv QUIT", "@a=b :p NICK other", "@a=b CAP REQ :x"] {
            assert!(check_raw_line(bad).is_err(), "{bad}");
        }
        assert_eq!(
            check_raw_line("@reply-parent-msg-id=1 PRIVMSG #a :hi"),
            Ok("@reply-parent-msg-id=1 PRIVMSG #a :hi".into())
        );
        assert!(check_raw_line("@only=tags").is_err());
    }

    #[test]
    fn raw_join_and_part_point_to_the_app_commands() {
        assert!(check_raw_line("PART #chan").unwrap_err().contains("/part"));
        assert!(check_raw_line("join #chan").unwrap_err().contains("/join"));
        assert!(check_raw_line("@x=1 PART #chan").is_err());
    }

    #[test]
    fn go_raid_answers() {
        assert!(go_raid_outcome(&json!({ "data": { "goRaid": null } })).is_ok());
        assert!(go_raid_outcome(&json!({ "data": { "goRaid": { "error": null } } })).is_ok());
        let none = go_raid_outcome(&json!({ "data": { "goRaid": { "error": { "code": "RAID_NOT_FOUND" } } } }))
            .unwrap_err();
        assert!(none.contains("no raid is counting down"), "{none}");
        let other = go_raid_outcome(&json!({ "data": { "goRaid": { "error": { "code": "SOMETHING_ELSE" } } } }))
            .unwrap_err();
        assert_eq!(other, "Could not send the raid: something else");
        assert!(go_raid_outcome(&json!({ "errors": [{ "message": "PersistedQueryNotFound" }] }))
            .unwrap_err()
            .contains("needs an update"));
    }

    #[test]
    fn delete_refusals_read_plainly() {
        let not_found = delete_refusal_text(
            r#"Twitch rejected delete (HTTP 404 Not Found): {"error":"Not Found","status":404,"message":"message not found"}"#,
        );
        assert!(not_found.contains("Announcements can't be deleted") && not_found.contains("/clear"), "{not_found}");
        assert!(delete_refusal_text("Twitch rejected delete (HTTP 403 Forbidden): {}").contains("moderator"));
        assert!(delete_refusal_text("Twitch rejected delete (HTTP 404): {}").contains("/clear"));
        assert_eq!(
            delete_refusal_text(r#"Twitch rejected delete (HTTP 400 Bad Request): {"message":"Something new."}"#),
            "Could not delete the message: Something new."
        );
        assert_eq!(delete_refusal_text("offline"), "Could not delete the message: offline");
    }

    #[test]
    fn banned_page_cursors_are_encoded() {
        assert_eq!(banned_page_path("9", None), "moderation/banned?broadcaster_id=9&first=100");
        assert_eq!(
            banned_page_path("9", Some("eyJ+a/b==")),
            "moderation/banned?broadcaster_id=9&first=100&after=eyJ%2Ba%2Fb%3D%3D"
        );
    }

    #[test]
    fn rate_limit_waits_follow_the_reset() {
        use std::time::Duration;
        assert_eq!(rate_limit_wait(Some(1_010), 1_000), Duration::from_secs(10));
        // Already reset, or a reset far away, stays between one second and a minute.
        assert_eq!(rate_limit_wait(Some(900), 1_000), Duration::from_secs(1));
        assert_eq!(rate_limit_wait(Some(9_000), 1_000), Duration::from_secs(60));
        assert_eq!(rate_limit_wait(None, 1_000), Duration::from_secs(2));
    }

    #[test]
    fn command_lines_split_on_quotes() {
        assert_eq!(split_command_line(r#"notepad "C:\a b\c.txt" -x"#), vec!["notepad", r"C:\a b\c.txt", "-x"]);
        assert_eq!(split_command_line(r#"a "" b"#), vec!["a", "", "b"]);
        assert!(split_command_line("   ").is_empty());
    }
}
