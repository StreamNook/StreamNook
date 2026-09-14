use crate::services::drops_auth_service::DropsAuthService;
use serde::{Deserialize, Serialize};

const JUSTLOG_BASE: &str = "https://logs.ivr.fi";
// best-logs (ZonianMidian) routes a channel/user to whichever justlog instance
// actually logs it. logs.ivr.fi covers a lot but not everything, so without this
// the Justlog source silently goes empty on channels it doesn't log.
const BESTLOGS_BASE: &str = "https://logs.zonian.dev";
const ROBOTTY_BASE: &str = "https://recent-messages.robotty.de";
const TWITCH_GQL_URL: &str = "https://gql.twitch.tv/gql";
// Twitch's GQL treats recentChatMessages / viewerCardModLogs as first-party
// queries: it 401s a USER token not minted under a first-party client-id, even if
// the Client-ID header matches. So these calls use the SAME pair the rest of the
// app's authed GQL uses (chat_identity, badges, channel points): the Twitch
// Android client-id plus `DropsAuthService::get_token()` (a first-party token).
// `TwitchService::get_token()` is StreamNook's OWN app token — fine for Helix, 401
// for GQL. Persisted-query hashes are global, so the MessageBufferChatHistory hash
// still resolves under the Android client-id.
const TWITCH_ANDROID_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

/// First-party GQL headers, matching the app's other authenticated GQL calls.
fn gql_headers(token: &str) -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
    let device_id = uuid::Uuid::new_v4().to_string().replace('-', "");
    let session_id = uuid::Uuid::new_v4().to_string().replace('-', "");
    let mut headers = HeaderMap::new();
    headers.insert("Client-ID", HeaderValue::from_static(TWITCH_ANDROID_CLIENT_ID));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("OAuth {}", token))
            .unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert("Origin", HeaderValue::from_static("https://www.twitch.tv"));
    headers.insert("Referer", HeaderValue::from_static("https://www.twitch.tv"));
    if let Ok(v) = HeaderValue::from_str(&device_id) {
        headers.insert("X-Device-Id", v);
    }
    if let Ok(v) = HeaderValue::from_str(&session_id) {
        headers.insert("Client-Session-Id", v);
    }
    headers
}

// Persisted-query hash for Twitch's `MessageBufferChatHistory` operation.
// Captured from Twitch's web client traffic via NodeCapture on 2026-01-12;
// works for any logged-in user (NOT mod-gated). If Twitch ever rotates this
// hash the request returns `PERSISTED_QUERY_NOT_FOUND` and the function
// silently returns empty — fallback sources still cover us.
const MESSAGE_BUFFER_CHAT_HISTORY_HASH: &str =
    "33dba0e0c249135052e930cbd6c4a66daa32249ba00d1c8def75857fa3f3431d";

/// One historical message normalized across all sources we pull from. The
/// optional `id` is the Twitch message ID (UUID-format, comes from the IRC
/// `id=` tag) and is the canonical dedupe key when present — every source
/// can provide it since they're all reading the same underlying IRC stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JustlogMessage {
    pub timestamp: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Why Twitch's own deep archive (`viewerCardModLogs`) produced what it did.
///
/// This exists because every failure mode used to collapse into an empty list,
/// so "you aren't a moderator here", "your login expired" and "this user never
/// spoke" were indistinguishable in the UI and all rendered as a blank panel.
/// Twitch only serves this archive to moderators of the channel in question, so
/// `NoAccess` is the *normal* outcome for most channels, not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModLogsStatus {
    /// Authorized and returned at least one message.
    Available,
    /// Authorized, but this user has no retained messages in this channel.
    NoMessages,
    /// Twitch returned UNAUTHORIZED: the viewer is not a moderator here (or the
    /// broadcaster has withheld viewer-card log access from moderators).
    NoAccess,
    /// No first-party token available, so the query was never attempted.
    AuthRequired,
    /// Caller did not supply both channel id and user id.
    Skipped,
    /// Network error, HTTP error, or a response shape we did not recognize.
    Failed,
}

/// Result of a user-history lookup: the merged messages plus why the
/// Twitch-native source contributed what it did. The third-party sources are
/// deliberately not surfaced individually; they are best-effort supplements and
/// their individual failures are not actionable by the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatLogsResult {
    pub messages: Vec<JustlogMessage>,
    pub mod_logs_status: ModLogsStatus,
}

#[derive(Debug, Deserialize)]
struct JustlogResponse {
    messages: Vec<JustlogRawMessage>,
}

#[derive(Debug, Deserialize)]
struct JustlogRawMessage {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RobottyResponse {
    #[serde(default)]
    messages: Vec<String>,
}

/// best-logs discovery response. We only need the instance lists; each is an
/// array of justlog base URLs (e.g. "https://logs.ivr.fi") sorted with the
/// best-coverage instance first.
#[derive(Debug, Deserialize)]
struct BestLogsResponse {
    #[serde(default, rename = "userLogs")]
    user_logs: Option<BestLogsInstanceList>,
    #[serde(default, rename = "channelLogs")]
    channel_logs: Option<BestLogsInstanceList>,
}

#[derive(Debug, Deserialize)]
struct BestLogsInstanceList {
    #[serde(default)]
    instances: Vec<String>,
}

/// Fetch a user's historical chat messages in a given channel by combining
/// four upstream sources, ranked by reliability:
///
/// 1. **Twitch GQL `ViewerCardModLogsMessagesBySender`**: deep per-user
///    history straight from Twitch, retained for roughly 3.5 years. Gated on
///    `moderationSettings.canAccessViewerCardModLogs` — the requester must be a
///    moderator of the channel, EXCEPT that anyone may always read their own
///    history anywhere. Denial comes back as a typed `ModLogsStatus::NoAccess`
///    rather than an empty list, so the UI can say why the panel is bare.
///    Expect this to be unavailable in most channels a viewer merely watches.
/// 2. **Twitch GQL `MessageBufferChatHistory`**: Twitch's channel-wide
///    chat backlog (the same query the web client uses on channel join).
///    Works for ANY logged-in user (not mod-gated). Verified via NodeCapture
///    2026-01-12 — captured with a regular user token returning 200.
///    Returns ~30 recent channel messages from all senders; filtered
///    client-side to the target user.
/// 3. **Justlog**, routed via best-logs: deep per-user history for logged
///    channels. best-logs (`logs.zonian.dev`) resolves which justlog instance
///    logs the channel; falls back to `logs.ivr.fi` directly. 404s when no
///    instance logs the channel.
/// 4. **recent-messages.robotty.de**: third-party channel backlog mirror,
///    last ~100 messages. Available for nearly any channel.
///
/// All fire in parallel. Results merged, sorted by parsed unix-ms, deduped
/// by Twitch message ID. Any source failing is non-fatal — whichever
/// sources returned data become the answer.
#[tauri::command]
pub async fn fetch_user_chat_logs(
    channel: String,
    username: String,
    #[allow(non_snake_case)] channelId: Option<String>,
    #[allow(non_snake_case)] userId: Option<String>,
) -> Result<ChatLogsResult, String> {
    if channel.is_empty() || username.is_empty() {
        return Ok(ChatLogsResult {
            messages: Vec::new(),
            mod_logs_status: ModLogsStatus::Skipped,
        });
    }

    let channel_lower = channel.to_lowercase();
    let username_lower = username.to_lowercase();

    let client = reqwest::Client::builder()
        .user_agent("StreamNook")
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    // Mod-gated Twitch GQL only fires when we have both IDs.
    let twitch_modlogs_fut = async {
        match (&channelId, &userId) {
            (Some(cid), Some(uid)) if !cid.is_empty() && !uid.is_empty() => {
                fetch_from_twitch_gql(&client, cid, uid).await
            }
            _ => (Vec::new(), ModLogsStatus::Skipped),
        }
    };

    // MessageBufferChatHistory works for any logged-in user. Needs channel_id
    // for the persisted query variables.
    let twitch_buffer_fut = async {
        match &channelId {
            Some(cid) if !cid.is_empty() => {
                fetch_from_message_buffer(&client, cid, &channel_lower, &username_lower).await
            }
            _ => Ok(Vec::new()),
        }
    };

    // FAST sources only. These return in ~1s, so the card can render them right
    // away. Deep history (Justlog, which can take several seconds via the proxy)
    // is a SEPARATE command (`fetch_user_deep_logs`) the frontend fires in
    // parallel and merges in when it lands — so the whole popup never blocks on
    // the slow source. Each capped so one bad source can't stall the fast batch.
    let src_cap = std::time::Duration::from_secs(6);
    let ((mut modlogs_msgs, mod_logs_status), twitch_buffer_result, robotty_result) = tokio::join!(
        async {
            tokio::time::timeout(src_cap, twitch_modlogs_fut)
                .await
                .unwrap_or((Vec::new(), ModLogsStatus::Failed))
        },
        async { tokio::time::timeout(src_cap, twitch_buffer_fut).await.unwrap_or_else(|_| Ok(Vec::new())) },
        async { tokio::time::timeout(src_cap, fetch_from_robotty(&client, &channel_lower, &username_lower)).await.unwrap_or_else(|_| Ok(Vec::new())) },
    );

    // Per-source diagnostic. Count: -1 = errored, 0 = empty, >0 = messages.
    let count_of = |r: &Result<Vec<JustlogMessage>, String>| r.as_ref().map(|m| m.len() as i64).unwrap_or(-1);
    let err_of = |r: &Result<Vec<JustlogMessage>, String>| r.as_ref().err().cloned().unwrap_or_default();
    log::info!(
        "[chatlogs/fast] channel={} user={} -> modlogs={} ({:?}) buffer={} robotty={}\n  buffer_err=[{}]\n  robotty_err=[{}]",
        channel_lower,
        username_lower,
        modlogs_msgs.len(),
        mod_logs_status,
        count_of(&twitch_buffer_result),
        count_of(&robotty_result),
        err_of(&twitch_buffer_result),
        err_of(&robotty_result),
    );

    let mut merged: Vec<JustlogMessage> = Vec::new();
    merged.append(&mut modlogs_msgs);
    if let Ok(mut m) = twitch_buffer_result {
        merged.append(&mut m);
    }
    if let Ok(mut m) = robotty_result {
        merged.append(&mut m);
    }

    Ok(ChatLogsResult {
        messages: sort_and_dedupe(merged),
        mod_logs_status,
    })
}

/// Deep per-user history: Justlog only (routed via best-logs, with proxy
/// fallback for DNS-blocked instances). Split out from `fetch_user_chat_logs`
/// because it can take several seconds — the card shows the fast sources
/// immediately and merges this in when it arrives, so the popup never blocks on
/// it. The frontend de-dupes across both calls, so overlap is harmless.
#[tauri::command]
pub async fn fetch_user_deep_logs(
    channel: String,
    username: String,
) -> Result<Vec<JustlogMessage>, String> {
    if channel.is_empty() || username.is_empty() {
        return Ok(Vec::new());
    }
    let channel_lower = channel.to_lowercase();
    let username_lower = username.to_lowercase();

    let client = reqwest::Client::builder()
        .user_agent("StreamNook")
        .timeout(std::time::Duration::from_secs(16))
        .build()
        .map_err(|e| e.to_string())?;

    // Generous cap: this is OFF the card's critical path, so we'd rather wait for
    // a slow proxy (a heavy chatter's full history measured ~7s) than drop it.
    let cap = std::time::Duration::from_secs(15);
    let result = tokio::time::timeout(cap, fetch_from_justlog(&client, &channel_lower, &username_lower))
        .await
        .unwrap_or_else(|_| Err("justlog: timed out".to_string()));

    log::info!(
        "[chatlogs/deep] channel={} user={} -> justlog={}\n  justlog_err=[{}]",
        channel_lower,
        username_lower,
        result.as_ref().map(|m| m.len() as i64).unwrap_or(-1),
        result.as_ref().err().cloned().unwrap_or_default(),
    );

    Ok(sort_and_dedupe(result.unwrap_or_default()))
}

/// Sort by parsed unix-millis (not the timestamp string — sources format it
/// differently) and dedupe by Twitch message id, falling back to (content +
/// timestamp within 2s) when a source omitted the id. Shared by the fast and
/// deep log commands.
fn sort_and_dedupe(mut merged: Vec<JustlogMessage>) -> Vec<JustlogMessage> {
    merged.sort_by_key(|m| parse_timestamp_ms(&m.timestamp).unwrap_or(0));

    let mut deduped: Vec<JustlogMessage> = Vec::with_capacity(merged.len());
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in merged {
        if let Some(id) = &msg.id {
            if !seen_ids.insert(id.clone()) {
                continue;
            }
        } else {
            let msg_ms = parse_timestamp_ms(&msg.timestamp).unwrap_or(0);
            let dup = deduped.iter().any(|existing| {
                if existing.content != msg.content {
                    return false;
                }
                let ex_ms = parse_timestamp_ms(&existing.timestamp).unwrap_or(0);
                (existing.id.is_none() || existing.id.as_ref() == msg.id.as_ref())
                    && (ex_ms - msg_ms).abs() <= 2_000
            });
            if dup {
                continue;
            }
        }
        deduped.push(msg);
    }
    deduped
}

/// Parse any of the timestamp formats our three sources can emit into unix
/// milliseconds. Accepts: `2024-01-15T18:30:00Z`, `2024-01-15T18:30:00.123Z`,
/// and `2024-01-15T18:30:00.123+00:00`. Returns None when nothing matches so
/// callers can fall back to zero for sorting (parsing failures end up at the
/// top of the list, but they shouldn't happen with our known sources).
fn parse_timestamp_ms(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Hits Twitch's `MessageBufferChatHistory` GQL operation — the same call
/// Twitch's web client makes when you load a channel page. Returns ~30
/// recent channel messages from all senders; we filter to the target user
/// client-side.
///
/// Works for any logged-in user (verified via NodeCapture 2026-01-12 with a
/// regular non-mod OAuth token returning 200). Uses Twitch's persisted-query
/// hash for efficiency; if Twitch rotates the hash the response carries
/// `PERSISTED_QUERY_NOT_FOUND` and we silently fall back to empty.
async fn fetch_from_message_buffer(
    client: &reqwest::Client,
    channel_id: &str,
    channel_login: &str,
    target_login: &str,
) -> Result<Vec<JustlogMessage>, String> {
    let token = match DropsAuthService::get_token().await {
        Ok(t) if !t.is_empty() => t,
        _ => return Ok(Vec::new()),
    };

    let body = serde_json::json!({
        "operationName": "MessageBufferChatHistory",
        "variables": {
            "channelID": channel_id,
            "channelLogin": channel_login,
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": MESSAGE_BUFFER_CHAT_HISTORY_HASH,
            }
        }
    });

    let response = client
        .post(TWITCH_GQL_URL)
        .headers(gql_headers(&token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("MessageBufferChatHistory request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "MessageBufferChatHistory HTTP {}",
            response.status()
        ));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse MessageBufferChatHistory: {}", e))?;

    // The Twitch GQL endpoint can return either a single object or an array
    // of objects (for batched queries). Real responses we've captured are a
    // single object; handle both shapes defensively.
    let root = if json.is_array() {
        json.get(0).cloned().unwrap_or(serde_json::Value::Null)
    } else {
        json
    };

    let messages_arr = match root
        .pointer("/data/channel/recentChatMessages")
        .and_then(|v| v.as_array())
    {
        Some(arr) => arr.clone(),
        None => return Ok(Vec::new()),
    };

    let target_lower = target_login.to_lowercase();
    let messages: Vec<JustlogMessage> = messages_arr
        .iter()
        .filter_map(|msg| {
            // Filter to the target user by sender login (case-insensitive).
            let sender_login = msg
                .get("sender")
                .and_then(|s| s.get("login"))
                .and_then(|l| l.as_str())?;
            if sender_login.to_lowercase() != target_lower {
                return None;
            }
            // Skip deleted messages — they show as a placeholder in chat UI
            // but rendering them in a history view is misleading.
            if !msg.get("deletedAt").map(|v| v.is_null()).unwrap_or(true) {
                return None;
            }
            let id = msg.get("id").and_then(|v| v.as_str()).map(String::from);
            let timestamp = msg.get("sentAt").and_then(|v| v.as_str())?.to_string();
            let content = msg
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(|t| t.as_str())?
                .to_string();
            if content.trim().is_empty() {
                return None;
            }
            Some(JustlogMessage {
                timestamp,
                content,
                id,
            })
        })
        .collect();

    Ok(messages)
}

async fn fetch_from_twitch_gql(
    client: &reqwest::Client,
    channel_id: &str,
    sender_id: &str,
) -> (Vec<JustlogMessage>, ModLogsStatus) {
    // The viewerCardModLogs root field is mod-gated. Non-mods get a
    // ViewerCardModLogsMessagesError back, which is a normal outcome we report
    // as NoAccess rather than swallowing. Authenticated request requires a
    // first-party token; without one there is nothing to try.
    let token = match DropsAuthService::get_token().await {
        Ok(t) if !t.is_empty() => t,
        _ => return (Vec::new(), ModLogsStatus::AuthRequired),
    };

    // Minimal inline query — only the chat-message variant of the message node
    // union. We deliberately don't depend on Twitch's persisted-query hashes
    // (they rotate); shipping the full query text is a few hundred bytes and is
    // stable, and Twitch accepts non-persisted documents.
    //
    // `first: 500` rather than a page at a time: measured against Twitch, there
    // is no server-side cap below 1000, and 500 returns a typical user's entire
    // multi-year history in this channel in one round trip. `pageInfo` is
    // requested so a genuinely heavy chatter who exceeds it is detectable rather
    // than silently truncated.
    let query = r#"
        query ViewerCardModLogsMessagesBySender($channelID: ID!, $senderID: ID!) {
          logs: viewerCardModLogs(channelID: $channelID, targetID: $senderID) {
            messages(first: 500) {
              __typename
              ... on ViewerCardModLogsMessagesError {
                code
              }
              ... on ViewerCardModLogsMessagesConnection {
                edges {
                  node {
                    __typename
                    ... on ViewerCardModLogsChatMessage {
                      id
                      sentAt
                      content { text }
                    }
                  }
                }
                pageInfo { hasNextPage }
              }
            }
          }
        }
    "#;

    let body = serde_json::json!({
        "operationName": "ViewerCardModLogsMessagesBySender",
        "query": query,
        "variables": {
            "channelID": channel_id,
            "senderID": sender_id,
        }
    });

    let response = match client
        .post(TWITCH_GQL_URL)
        .headers(gql_headers(&token))
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[chatlogs/modlogs] request failed: {}", e);
            return (Vec::new(), ModLogsStatus::Failed);
        }
    };

    if !response.status().is_success() {
        log::warn!("[chatlogs/modlogs] HTTP {}", response.status());
        return (Vec::new(), ModLogsStatus::Failed);
    }

    let json: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(e) => {
            log::warn!("[chatlogs/modlogs] parse failed: {}", e);
            return (Vec::new(), ModLogsStatus::Failed);
        }
    };

    // `messages` is a union. The error variant carries a code — UNAUTHORIZED
    // when the viewer isn't a moderator of this channel, which is the common
    // case and must not read as a failure.
    let messages_node = json.pointer("/data/logs/messages");
    match messages_node
        .and_then(|m| m.get("__typename"))
        .and_then(|t| t.as_str())
    {
        Some("ViewerCardModLogsMessagesError") => {
            let code = messages_node
                .and_then(|m| m.get("code"))
                .and_then(|c| c.as_str())
                .unwrap_or("");
            log::info!("[chatlogs/modlogs] denied: {}", code);
            return (Vec::new(), ModLogsStatus::NoAccess);
        }
        Some("ViewerCardModLogsMessagesConnection") => {}
        // Null logs (channel/user not found) or a shape we don't recognize.
        other => {
            log::warn!("[chatlogs/modlogs] unexpected union variant: {:?}", other);
            return (Vec::new(), ModLogsStatus::Failed);
        }
    }

    if messages_node
        .and_then(|m| m.pointer("/pageInfo/hasNextPage"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        log::info!("[chatlogs/modlogs] more than 500 messages available; result truncated");
    }

    let edges = match messages_node
        .and_then(|m| m.get("edges"))
        .and_then(|v| v.as_array())
    {
        Some(e) => e,
        None => return (Vec::new(), ModLogsStatus::NoMessages),
    };

    let messages: Vec<JustlogMessage> = edges
        .iter()
        .filter_map(|edge| {
            let node = edge.get("node")?;
            // Skip non-chat-message variants (mod actions, caught messages, etc.).
            if node.get("__typename")?.as_str()? != "ViewerCardModLogsChatMessage" {
                return None;
            }
            let timestamp = node.get("sentAt")?.as_str()?.to_string();
            let content = node
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(|t| t.as_str())?
                .to_string();
            if content.trim().is_empty() {
                return None;
            }
            let id = node.get("id").and_then(|v| v.as_str()).map(String::from);
            Some(JustlogMessage {
                timestamp,
                content,
                id,
            })
        })
        .collect();

    let status = if messages.is_empty() {
        ModLogsStatus::NoMessages
    } else {
        ModLogsStatus::Available
    };
    (messages, status)
}

/// Cached best-logs resolution for a channel: the ordered list of justlog
/// instance base URLs that log it (best coverage first). An empty list means
/// best-logs couldn't resolve any (we then fall back to ivr.fi); we cache that
/// too, on a shorter TTL, so a hiccup doesn't make us re-hit best-logs on every
/// card open.
struct InstanceCacheEntry {
    instances: Vec<String>,
    at: std::time::Instant,
}

// A resolved instance is very stable (it's just which justlog server logs a
// channel), so cache it for an hour. A miss is cached only 10 min so we recover
// quickly once best-logs is reachable / the channel starts being logged.
const INSTANCE_TTL_OK: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const INSTANCE_TTL_MISS: std::time::Duration = std::time::Duration::from_secs(10 * 60);

fn instance_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, InstanceCacheEntry>>
{
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, InstanceCacheEntry>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Ask best-logs which justlog instances hold THIS user's logs in THIS channel,
/// in best-coverage order. Per-user, not per-channel: only some instances that
/// log a channel actually have a given user, and the channel's top instance can
/// be down while a lower one holds the user — so we ask per-user and try that
/// exact ordered list. Cached per (channel, user). Empty when best-logs is
/// unreachable or the user isn't logged anywhere.
///
/// Uses best-logs' lightweight discovery endpoint (`/api/{channel}/{user}`), NOT
/// its `/channel/...` proxy mirror: the mirror is Cloudflare rate-limited (it
/// returns a 1015 page under even light load), the discovery endpoint is not.
async fn resolve_justlog_instances(
    client: &reqwest::Client,
    channel: &str,
    username: &str,
) -> Vec<String> {
    let key = format!("{}/{}", channel, username);
    // Serve from cache while fresh — this is what keeps us from pelting best-logs
    // on every single user-card open (repeat opens of the same user are free).
    if let Ok(cache) = instance_cache().lock() {
        if let Some(entry) = cache.get(&key) {
            let ttl = if entry.instances.is_empty() {
                INSTANCE_TTL_MISS
            } else {
                INSTANCE_TTL_OK
            };
            if entry.at.elapsed() < ttl {
                return entry.instances.clone();
            }
        }
    }

    let instances = fetch_bestlogs_instances(client, channel, username).await;

    if let Ok(mut cache) = instance_cache().lock() {
        cache.insert(
            key,
            InstanceCacheEntry {
                instances: instances.clone(),
                at: std::time::Instant::now(),
            },
        );
    }
    instances
}

/// Uncached single best-logs discovery request: the ordered instance list that
/// holds this user's logs in this channel.
async fn fetch_bestlogs_instances(
    client: &reqwest::Client,
    channel: &str,
    username: &str,
) -> Vec<String> {
    let url = format!("{}/api/{}/{}", BESTLOGS_BASE, channel, username);
    // Short, dedicated timeout: discovery is just a routing hint. If best-logs
    // is slow or Cloudflare-challenges us, bail fast and let the caller fall
    // back to logs.ivr.fi rather than burning the justlog branch's whole budget.
    let response = match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return Vec::new(),
    };
    let parsed: BestLogsResponse = match response.json().await {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    // Prefer instances that actually hold THIS user; fall back to channel-wide.
    let mut list = parsed.user_logs.map(|l| l.instances).unwrap_or_default();
    if list.is_empty() {
        list = parsed.channel_logs.map(|l| l.instances).unwrap_or_default();
    }
    list.retain(|base| base.starts_with("https://") || base.starts_with("http://"));
    list
}

async fn fetch_from_justlog(
    client: &reqwest::Client,
    channel: &str,
    username: &str,
) -> Result<Vec<JustlogMessage>, String> {
    // Instances that hold this user's logs (best coverage first), from best-logs
    // and cached per (channel, user). Always append logs.ivr.fi as a final
    // fallback so we still try something when best-logs is unreachable.
    let mut candidates = resolve_justlog_instances(client, channel, username).await;
    if !candidates.iter().any(|b| b == JUSTLOG_BASE) {
        candidates.push(JUSTLOG_BASE.to_string());
    }

    // Try instances in order until one actually returns messages. The top one is
    // usually right, but if it's down/slow the same logs live on another instance
    // — don't let deep history silently vanish because one instance had a bad
    // moment. `any_reachable` lets us tell "user genuinely not logged" (some
    // instance answered, just empty) from "every instance unreachable" (surfaced
    // as an error so the diagnostic shows exactly what failed).
    let mut any_reachable = false;
    let mut attempts: Vec<String> = Vec::new();
    for base in candidates.into_iter().take(5) {
        match fetch_justlog_at(client, &base, channel, username).await {
            Ok(msgs) if !msgs.is_empty() => return Ok(msgs),
            Ok(_) => {
                any_reachable = true;
                attempts.push(format!("{}=empty", base));
            }
            Err(e) => attempts.push(e),
        }
    }
    if any_reachable {
        // Some instance answered (just no messages for this user) — genuinely
        // not logged. No need to hit the proxy.
        return Ok(Vec::new());
    }

    // Every direct instance was unreachable. The common real-world cause is the
    // user's own DNS (Pi-hole / AdGuard) NXDOMAIN-ing the `logs.*` justlog domains
    // as "trackers" — the instances resolve fine on public DNS but not on theirs.
    // best-logs' proxy fetches server-side over `logs.zonian.dev` (not on those
    // blocklists), so it returns the same logs even when the direct domains are
    // blocked. Last resort only, so normal users never touch it.
    match fetch_justlog_at(client, BESTLOGS_BASE, channel, username).await {
        Ok(msgs) => Ok(msgs),
        Err(proxy_err) => Err(format!(
            "justlog: all instances failed [{}] and proxy failed [{}]",
            attempts.join(" | "),
            proxy_err
        )),
    }
}

/// One justlog instance's per-user history. A 404 (this instance doesn't log the
/// channel/user) is a normal empty, not an error, so the caller moves on to the
/// next instance. Per-request timeout keeps a single slow instance from eating
/// the whole justlog budget.
async fn fetch_justlog_at(
    client: &reqwest::Client,
    base: &str,
    channel: &str,
    username: &str,
) -> Result<Vec<JustlogMessage>, String> {
    let url = format!("{}/channel/{}/user/{}?json=1", base, channel, username);

    let response = client
        .get(&url)
        // Room for a large payload (a heavy chatter's full history via the proxy
        // can be >1000 messages) while still fitting inside the justlog cap. A
        // DNS-blocked instance fails instantly (NXDOMAIN), so this only bounds
        // genuinely-slow responses.
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| {
            // Classify the failure so an unreachable instance is obvious: is it a
            // connection failure (DNS/refused/TLS) vs a slow instance (timeout)?
            let kind = if e.is_timeout() {
                "timeout"
            } else if e.is_connect() {
                "connect"
            } else {
                "other"
            };
            format!("Justlog request failed ({}, {}): {}", base, kind, e)
        })?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    if !response.status().is_success() {
        return Err(format!("Justlog API error ({}): {}", base, response.status()));
    }

    let parsed: JustlogResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Justlog response ({}): {}", base, e))?;

    Ok(parsed
        .messages
        .into_iter()
        .filter_map(|m| {
            let content = m.text?;
            if content.trim().is_empty() {
                return None;
            }
            Some(JustlogMessage {
                timestamp: m.timestamp.unwrap_or_default(),
                content,
                id: m.id,
            })
        })
        .collect())
}

async fn fetch_from_robotty(
    client: &reqwest::Client,
    channel: &str,
    username: &str,
) -> Result<Vec<JustlogMessage>, String> {
    let url = format!(
        "{}/api/v2/recent-messages/{}?limit=100",
        ROBOTTY_BASE, channel
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Robotty request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Robotty API error: {}", response.status()));
    }

    let parsed: RobottyResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Robotty response: {}", e))?;

    Ok(parsed
        .messages
        .iter()
        .filter_map(|raw| parse_robotty_message(raw, username))
        .collect())
}

/// Minimal IRCv3 line parser scoped to what recent-messages.robotty.de hands
/// us: each entry is a raw IRC line of the form
/// `@tag1=val1;tag2=val2;... :nick!nick@nick.tmi.twitch.tv PRIVMSG #channel :body`.
///
/// We only need three things out of it: confirm the sender's login matches our
/// target, extract `tmi-sent-ts` for the timestamp, and pull the body. We
/// deliberately don't bring in a full IRC parser dependency for this — the
/// format is rigid enough that careful string splitting suffices.
fn parse_robotty_message(raw: &str, target_login: &str) -> Option<JustlogMessage> {
    // Robotty includes non-PRIVMSG events too (USERNOTICE, CLEARCHAT, etc.).
    // Skip anything that isn't a regular chat message — those aren't what the
    // user is asking to see when they click "show messages."
    if !raw.contains(" PRIVMSG ") {
        return None;
    }

    let line = raw.strip_prefix('@')?;
    let (tags, rest) = line.split_once(' ')?;

    // ":nick!nick@nick.tmi.twitch.tv ..." — login is everything before the !.
    let nick_section = rest.strip_prefix(':')?;
    let nick = nick_section.split_once('!')?.0;
    if !nick.eq_ignore_ascii_case(target_login) {
        return None;
    }

    // Body lives after the second " :" (first is between IRC params and trailing).
    let body_start = rest.find(" PRIVMSG ")? + " PRIVMSG ".len();
    let after_privmsg = &rest[body_start..];
    let body = after_privmsg.split_once(" :")?.1.to_string();
    if body.trim().is_empty() {
        return None;
    }

    // Pull tmi-sent-ts (unix milliseconds as a decimal string) out of the tags.
    let ts_ms: i64 = tags
        .split(';')
        .find_map(|t| t.strip_prefix("tmi-sent-ts="))
        .and_then(|s| s.parse().ok())?;

    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)?.to_rfc3339();

    // The IRC `id=<uuid>` tag is the canonical Twitch message ID — same id
    // Twitch GQL and Justlog will report for this message. Critical for
    // cross-source dedupe; without it we fall back to content+time matching.
    let id = tags
        .split(';')
        .find_map(|t| t.strip_prefix("id="))
        .map(String::from);

    Some(JustlogMessage {
        timestamp,
        content: body,
        id,
    })
}
