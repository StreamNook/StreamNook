//! Reading the signed-in YouTube account's SUBSCRIPTIONS as a follow list.
//!
//! Distinct from `youtube_media::followed_live`, which answers "which of my
//! subscriptions are live right now" off the subscriptions FEED. This reads the
//! subscribed CHANNELS themselves, so they populate the app's follow list the way
//! Kick's import does. Without it, signing in leaves the Home following list empty
//! and the connection looks like it did nothing.
//!
//! Auth is the cookie session harvested at sign-in (`youtube_auth_service`), the
//! same SAPISIDHASH headers every other private InnerTube call here uses.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashSet;

const INNERTUBE: &str = "https://www.youtube.com/youtubei/v1";
const INNERTUBE_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
/// `youtube.com/feed/channels`: the full "all subscriptions" list. The
/// subscriptions FEED (`FEsubscriptions`) carries videos; this carries channels.
const BROWSE_ID: &str = "FEchannels";

/// One subscribed channel, in the shape the follow importer needs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct YouTubeSubscription {
    /// UC id. Stored as the follow's `channel` because handles can be changed by
    /// their owner, while the UC id is permanent, and both address chat + playback.
    pub channel_id: String,
    pub display_name: String,
    /// `@handle` when the entry carried one, for display and for a readable URL.
    pub handle: Option<String>,
    /// The channel's avatar as shown in the subscriptions feed. Free here — the
    /// feed renders it, so the payload already carries it.
    pub avatar: Option<String>,
}

/// The signed-in account's subscribed channels.
pub async fn subscriptions() -> Result<Vec<YouTubeSubscription>> {
    let headers = crate::services::youtube_auth_service::auth_headers()
        .ok_or_else(|| anyhow!("not signed in to YouTube"))?;

    let body = json!({
        "browseId": BROWSE_ID,
        "context": { "client": { "clientName": "WEB", "clientVersion": "2.20240101.00.00", "hl": "en", "gl": "US" } }
    });
    let mut req = reqwest::Client::new()
        .post(format!("{}/browse?key={}", INNERTUBE, INNERTUBE_KEY))
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.json(&body).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("YouTube subscriptions HTTP {}", resp.status()));
    }
    let json: Value = resp.json().await?;

    let subs = channels_in(&json);
    if subs.is_empty() {
        // Same story as the live feed: a stale cookie set reads as an empty list
        // rather than an error, so try to recover before reporting.
        crate::services::youtube_auth_service::recover_stale_session().await;
        // Signed-out and empty look identical at the HTTP layer (both 200), so
        // report the session problem rather than "you follow nobody".
        return Err(anyhow!(
            "YouTube returned no subscribed channels (the session may have expired)"
        ));
    }
    Ok(subs)
}

/// Every channel entry anywhere in an InnerTube response.
///
/// Matched by SHAPE, not by renderer name: an entry is anything carrying a
/// `browseEndpoint` whose `browseId` is a UC id, plus a title. YouTube has several
/// names for the same row (`channelRenderer`, `gridChannelRenderer`, and the newer
/// `lockupViewModel`) and migrates surfaces between them, so pinning names is what
/// would break. Verified against a live channel search, which returned 20 entries
/// with ids, titles and `/@handle` base urls.
fn channels_in(root: &Value) -> Vec<YouTubeSubscription> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(sub) = channel_entry(node) {
                    if seen.insert(sub.channel_id.clone()) {
                        out.push(sub);
                    }
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    out
}

/// The biggest thumbnail URL anywhere in a node, for a channel avatar.
///
/// Deliberately a scan rather than a fixed path: YouTube has changed the
/// subscriptions renderer shape at least once already, and an avatar that stops
/// resolving should degrade to the existing per-card lookup, not break the import.
fn largest_thumbnail(node: &Value) -> Option<String> {
    fn walk(v: &Value, best: &mut Option<(u64, String)>, depth: usize) {
        if depth > 8 {
            return;
        }
        match v {
            Value::Object(map) => {
                if let Some(list) = map.get("thumbnails").and_then(|t| t.as_array()) {
                    for t in list {
                        let url = t.get("url").and_then(|u| u.as_str());
                        let w = t.get("width").and_then(|w| w.as_u64()).unwrap_or(0);
                        if let Some(url) = url {
                            if best.as_ref().is_none_or(|(bw, _)| w > *bw) {
                                *best = Some((w, url.to_string()));
                            }
                        }
                    }
                }
                for child in map.values() {
                    walk(child, best, depth + 1);
                }
            }
            Value::Array(items) => {
                for child in items {
                    walk(child, best, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut best = None;
    walk(node, &mut best, 0);
    best.map(|(_, url)| url)
}

/// Read one node as a channel entry, or None if it isn't one.
fn channel_entry(node: &Value) -> Option<YouTubeSubscription> {
    let endpoint = node
        .pointer("/navigationEndpoint/browseEndpoint")
        .or_else(|| node.pointer("/browseEndpoint"))
        .or_else(|| node.pointer("/onTap/innertubeCommand/browseEndpoint"))?;
    let channel_id = endpoint.get("browseId").and_then(|v| v.as_str())?;
    if !is_channel_id(channel_id) {
        return None;
    }
    let display_name = title_of(node)?;
    let handle = endpoint
        .get("canonicalBaseUrl")
        .and_then(|v| v.as_str())
        .and_then(|u| u.strip_prefix("/@"))
        .map(|h| format!("@{}", h));
    // Tolerant: the feed has moved between renderer shapes (see the lockupViewModel
    // change), so scan this node for the largest thumbnail rather than assuming one
    // path. None is fine — the backfill below covers it.
    let avatar = largest_thumbnail(node);
    Some(YouTubeSubscription {
        channel_id: channel_id.to_string(),
        display_name,
        handle,
        avatar,
    })
}

fn is_channel_id(id: &str) -> bool {
    id.len() == 24 && id.starts_with("UC")
}

/// The node's own title, across the text shapes YouTube uses.
fn title_of(node: &Value) -> Option<String> {
    if let Some(s) = node.pointer("/title/simpleText").and_then(|v| v.as_str()) {
        return non_empty(s);
    }
    if let Some(runs) = node.pointer("/title/runs").and_then(|v| v.as_array()) {
        let s: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
            .collect();
        return non_empty(&s);
    }
    // The newer view-model shape.
    if let Some(s) = node
        .pointer("/metadata/lockupMetadataViewModel/title/content")
        .and_then(|v| v.as_str())
    {
        return non_empty(s);
    }
    None
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape captured from a live channel search: the same renderer the
    /// subscriptions list uses.
    #[test]
    fn reads_a_channel_renderer() {
        let v = json!({ "contents": [ { "channelRenderer": {
            "channelId": "UCXuqSBlHAE6Xw-yeJA0Tunw",
            "title": { "simpleText": "Linus Tech Tips" },
            "navigationEndpoint": { "browseEndpoint": {
                "browseId": "UCXuqSBlHAE6Xw-yeJA0Tunw",
                "canonicalBaseUrl": "/@LinusTechTips"
            } }
        } } ] });
        let subs = channels_in(&v);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].channel_id, "UCXuqSBlHAE6Xw-yeJA0Tunw");
        assert_eq!(subs[0].display_name, "Linus Tech Tips");
        assert_eq!(subs[0].handle.as_deref(), Some("@LinusTechTips"));
    }

    /// The newer view-model row must parse too, since YouTube migrates surfaces
    /// onto it and pinning renderer names is exactly what would break.
    #[test]
    fn reads_a_lockup_view_model() {
        let v = json!({ "contents": [ { "lockupViewModel": {
            "onTap": { "innertubeCommand": { "browseEndpoint": {
                "browseId": "UCeeFfhMcJa1kjtfZAGskOCA",
                "canonicalBaseUrl": "/@techlinked"
            } } },
            "metadata": { "lockupMetadataViewModel": { "title": { "content": "TechLinked" } } }
        } } ] });
        let subs = channels_in(&v);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].display_name, "TechLinked");
        assert_eq!(subs[0].handle.as_deref(), Some("@techlinked"));
    }

    #[test]
    fn ignores_non_channel_endpoints_and_dedupes() {
        let v = json!({ "a": [
            // A video row: a browseEndpoint, but not a channel id.
            { "videoRenderer": { "title": { "simpleText": "Some video" },
              "navigationEndpoint": { "browseEndpoint": { "browseId": "FEwhat_to_watch" } } } },
            // A channel with no title is unusable.
            { "channelRenderer": { "navigationEndpoint": { "browseEndpoint": {
                "browseId": "UCXuqSBlHAE6Xw-yeJA0Tunw" } } } },
            // The same channel twice (a shelf and its overflow) collapses to one.
            { "channelRenderer": { "title": { "runs": [ { "text": "Dupe" } ] },
              "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdBK94H6oZT2Q7l0-b0xmMg" } } } },
            { "gridChannelRenderer": { "title": { "simpleText": "Dupe" },
              "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdBK94H6oZT2Q7l0-b0xmMg" } } } }
        ] });
        let subs = channels_in(&v);
        assert_eq!(subs.len(), 1, "only the titled, deduped channel survives");
        assert_eq!(subs[0].channel_id, "UCdBK94H6oZT2Q7l0-b0xmMg");
        assert_eq!(subs[0].handle, None);
    }
}
