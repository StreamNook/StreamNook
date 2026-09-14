//! Subscribing to a YouTube channel: YouTube's equivalent of a follow.
//!
//! Distinct from `open_subscribe_window`, which opens channel MEMBERSHIP (`/join`),
//! the paid tier that matches a Twitch/Kick sub. This is the free subscribe button.
//!
//! Subscribing is a WRITE, and YouTube guards writes harder than reads: the same
//! SAPISIDHASH headers that read the subscription list are not enough on their own.
//! A subscribe sent with a made-up client version and no `params` comes back HTTP
//! 200 carrying a "Sign in" popup, i.e. silently unauthenticated. Verified against
//! a live session.
//!
//! So this mirrors what the site itself sends rather than inventing a request: the
//! real scraped client version, the page's visitorData, the client headers, and the
//! subscribe/unsubscribe endpoint READ OFF THE PAGE with its params.

use super::youtube::{extract_json, live_page_url};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

const SUBSCRIBE_API: &str = "https://www.youtube.com/youtubei/v1/subscription/subscribe";
const UNSUBSCRIBE_API: &str = "https://www.youtube.com/youtubei/v1/subscription/unsubscribe";

/// Change one channel's subscribe state.
///
/// `identifier` is whatever the app addresses the channel by: a video id (the usual
/// case, since provider rows are keyed by broadcast), an `@handle`, or a UC id.
///
/// The endpoint comes from the WATCH page. Channel pages moved to an entity-backed
/// header that carries neither `subscribeEndpoint` nor `unsubscribeEndpoint`
/// (checked authed against both `@handle` and `/channel/UC…`), so the watch page is
/// the reliable route to it.
pub async fn set_subscribed(identifier: &str, subscribe: bool) -> Result<()> {
    let headers = crate::services::youtube_auth_service::auth_headers()
        .ok_or_else(|| anyhow!("Sign into YouTube to do that"))?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let url = live_page_url(identifier);
    // Fetch the page AS THE SIGNED-IN USER, not anonymously. The `visitorData` and
    // client version harvested here are sent back with the write, and an anonymous
    // page yields a visitorData belonging to no session. Verified working against a
    // live account with the authed page; do not "simplify" this back to a plain
    // fetch.
    let mut page_req = http.get(&url);
    for (k, v) in &headers {
        page_req = page_req.header(k, v);
    }
    let html = page_req.send().await?.text().await?;
    let client_version = json_after(&html, "\"INNERTUBE_CONTEXT_CLIENT_VERSION\":\"")
        .ok_or_else(|| anyhow!("couldn't read YouTube's client version"))?;
    let api_key = json_after(&html, "\"INNERTUBE_API_KEY\":\"")
        .ok_or_else(|| anyhow!("couldn't read YouTube's API key"))?;
    let visitor_data = json_after(&html, "\"visitorData\":\"");
    let initial =
        extract_json(&html, "ytInitialData").ok_or_else(|| anyhow!("couldn't read that page"))?;

    let want = if subscribe {
        "subscribeEndpoint"
    } else {
        "unsubscribeEndpoint"
    };
    let endpoint = find_key(&initial, want).ok_or_else(|| {
        anyhow!(
            "couldn't find YouTube's {} button for '{}'",
            if subscribe { "subscribe" } else { "unsubscribe" },
            identifier
        )
    })?;
    let channel_ids = endpoint
        .get("channelIds")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| anyhow!("YouTube's subscribe button named no channel"))?
        .clone();
    // Params arrive PERCENT-ENCODED in the page ("…%3D%3D"). Sending them as-is
    // hands YouTube a different token than its own button does. Same trap the Kick
    // bearer hit.
    let params = endpoint
        .get("params")
        .and_then(|v| v.as_str())
        .map(percent_decode);

    let mut client = json!({
        "clientName": "WEB",
        "clientVersion": client_version,
        "hl": "en",
        "gl": "US",
    });
    if let Some(vd) = &visitor_data {
        client["visitorData"] = json!(vd);
    }
    let mut body = json!({ "channelIds": channel_ids, "context": { "client": client } });
    if let Some(p) = params {
        body["params"] = json!(p);
    }

    let api = if subscribe {
        SUBSCRIBE_API
    } else {
        UNSUBSCRIBE_API
    };
    let mut req = http
        .post(format!("{}?key={}&prettyPrint=false", api, api_key))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        // The site sends these on every InnerTube call, and a write is where their
        // absence starts to matter. `X-Goog-Visitor-Id` in particular is sent as a
        // HEADER as well as inside the context; a write rejected as signed-out
        // despite valid cookies is the symptom of the header set being incomplete.
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", client_version.as_str())
        .header("X-Youtube-Bootstrap-Logged-In", "true")
        .header(reqwest::header::REFERER, url.as_str());
    if let Some(vd) = &visitor_data {
        req = req.header("X-Goog-Visitor-Id", vd.as_str());
    }
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    let resp = req.json(&body).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("YouTube subscribe HTTP {}", resp.status()));
    }
    let value: Value = resp.json().await?;

    // A REJECTED write still returns 200: YouTube answers with a "Sign in" popup
    // rather than an error status. Treating that as success would silently do
    // nothing while reporting that it worked.
    let raw = value.to_string();
    if raw.contains("signInEndpoint") || raw.contains("\"simpleText\":\"Sign in\"") {
        // Log what YouTube actually said. The generic message is not enough to tell
        // an expired session from an incomplete request, and those need opposite
        // fixes: one is "sign in again", the other is a header we are not sending.
        log::warn!(
            "[YouTube] subscribe rejected as signed-out. visitorData={}, clientVersion={}, response={}",
            visitor_data.as_deref().unwrap_or("none"),
            client_version,
            &raw.chars().take(400).collect::<String>(),
        );
        return Err(anyhow!(
            "YouTube didn't accept the change (the session may need reconnecting)"
        ));
    }
    log::info!(
        "[YouTube] {} ok for '{}'",
        if subscribe { "subscribe" } else { "unsubscribe" },
        identifier
    );
    Ok(())
}

/// The first value under `key` anywhere in a response tree.
fn find_key<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(v) = map.get(key) {
                    return Some(v);
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    None
}

/// The value of a `"key":"…"` pair in raw page HTML.
fn json_after(html: &str, marker: &str) -> Option<String> {
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Minimal decoder for the `%XX` escapes YouTube puts in endpoint params.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Params arrive percent-encoded in the page and must be decoded before being
    /// sent, or YouTube receives a different token than its own button uses.
    /// Both strings captured from a live authed watch page.
    #[test]
    fn decodes_endpoint_params() {
        assert_eq!(
            percent_decode("EgIIAxgAIgs5OGF5MG53QWJ0NA%3D%3D"),
            "EgIIAxgAIgs5OGF5MG53QWJ0NA==",
        );
        assert_eq!(
            percent_decode("CgIIAxILOThheTBud0FidDQYAA%3D%3D"),
            "CgIIAxILOThheTBud0FidDQYAA==",
        );
        // Already-decoded params survive untouched.
        assert_eq!(percent_decode("EgIIAxgA"), "EgIIAxgA");
    }

    /// Shape captured from a live authed watch page: the endpoint is nested inside
    /// the button's service endpoints, so it is found by walking rather than by a
    /// fixed path.
    #[test]
    fn finds_the_subscribe_endpoint_anywhere_in_the_tree() {
        let page = json!({ "contents": { "a": { "b": [ { "subscribeButtonRenderer": {
            "subscribed": false,
            "channelId": "UCDvm7YoLE5r3ZZ6MWyD2vGQ",
            "serviceEndpoints": [ { "subscribeEndpoint": {
                "channelIds": ["UCDvm7YoLE5r3ZZ6MWyD2vGQ"],
                "params": "EgIIAxgAIgs5OGF5MG53QWJ0NA%3D%3D"
            } } ]
        } } ] } } });
        let ep = find_key(&page, "subscribeEndpoint").expect("endpoint");
        assert_eq!(
            ep.pointer("/channelIds/0").and_then(|v| v.as_str()),
            Some("UCDvm7YoLE5r3ZZ6MWyD2vGQ"),
        );
        assert!(find_key(&page, "unsubscribeEndpoint").is_none());
    }

    /// MEMBER shape, captured verbatim from a live signed-in session on a channel
    /// the account holds a membership in. The button navigates to the channel's
    /// membership page: "manage what I have".
    #[test]
    fn recognises_an_existing_membership() {
        let button = json!({ "buttonRenderer": {
            "text": { "runs": [ { "text": "See perks" } ] },
            "navigationEndpoint": {
                "commandMetadata": { "webCommandMetadata": {
                    "url": "/channel/UChNWxrTlmh4IRSevon1X93g/membership",
                    "webPageType": "WEB_PAGE_TYPE_CHANNEL",
                    "apiUrl": "/youtubei/v1/browse"
                } },
                "browseEndpoint": {
                    "browseId": "UChNWxrTlmh4IRSevon1X93g",
                    "params": "EgptZW1iZXJzaGlw8gYJCgciAKIBAggB"
                }
            },
            "tooltip": "Manage membership"
        } });
        assert!(holds_membership(&button));
    }

    /// NON-MEMBER shape, same session, a channel the account does not hold. The
    /// button posts to the offers endpoint: "show me what I can buy".
    #[test]
    fn recognises_a_channel_not_yet_joined() {
        let button = json!({ "timedAnimationButtonRenderer": { "buttonRenderer": { "buttonRenderer": {
            "text": { "runs": [ { "text": "Join" } ] },
            "serviceEndpoint": {
                "commandMetadata": { "webCommandMetadata": {
                    "sendPost": true,
                    "apiUrl": "/youtubei/v1/ypc/get_offers"
                } },
                "ypcGetOffersEndpoint": { "params": "sku-CisKKRIe" }
            },
            "targetId": "sponsorships-button"
        } } } });
        assert!(!holds_membership(&button));
    }

    /// SIGNED-OUT shape: a sign-in modal, carrying neither marker. Reads as "not a
    /// member", which is the safe direction (a wrong "member" would hide the only
    /// way to buy one).
    #[test]
    fn a_signed_out_button_is_not_a_membership() {
        let button = json!({ "timedAnimationButtonRenderer": { "buttonRenderer": { "buttonRenderer": {
            "text": { "runs": [ { "text": "Join" } ] },
            "navigationEndpoint": {
                "commandMetadata": { "webCommandMetadata": { "ignoreNavigation": true } },
                "modalEndpoint": { "modal": { "modalWithTitleAndButtonRenderer": {
                    "title": { "runs": [ { "text": "Want to join this channel?" } ] },
                    "content": { "runs": [ { "text": "Sign in to become a member." } ] }
                } } }
            }
        } } } });
        assert!(!holds_membership(&button));
    }

    /// The visible labels are NOT the signal: they are localised, and both states
    /// would flip on a non-English UI while the endpoints stay put.
    #[test]
    fn labels_are_not_what_decides_it() {
        let localised_member = json!({ "buttonRenderer": {
            "text": { "runs": [ { "text": "Vorteile ansehen" } ] },
            "navigationEndpoint": { "commandMetadata": { "webCommandMetadata": {
                "url": "/channel/UChNWxrTlmh4IRSevon1X93g/membership"
            } } }
        } });
        assert!(holds_membership(&localised_member));

        let localised_join = json!({ "buttonRenderer": {
            "text": { "runs": [ { "text": "Beitreten" } ] },
            "serviceEndpoint": { "ypcGetOffersEndpoint": { "params": "sku-x" } }
        } });
        assert!(!holds_membership(&localised_join));
    }

    /// Membership hub shape, captured verbatim from a live session on a channel the
    /// account holds a membership in. This is where the member's own badge lives,
    /// the direct equivalent of Twitch's subscriber badge.
    #[test]
    fn reads_the_member_badge_tier_and_duration() {
        let hub = json!({
            "tierName": "Channel Supporter",
            "membershipType": "Member",
            "membershipDurationOrExpiry": "2 months",
            "perksTitle": "Your perks",
            "badgeImage": { "sources": [
                { "url": "https://yt3.googleusercontent.com/small=s32-k-nd" },
                { "url": "https://yt3.googleusercontent.com/6YYy=s64-k-nd" }
            ] }
        });
        assert_eq!(
            hub_badge(&hub).as_deref(),
            Some("https://yt3.googleusercontent.com/6YYy=s64-k-nd"),
            "largest source wins",
        );
        assert_eq!(text_of(&hub, "tierName").as_deref(), Some("Channel Supporter"));
        assert_eq!(text_of(&hub, "membershipDurationOrExpiry").as_deref(), Some("2 months"));
    }

    /// A hub without badge art costs the badge and nothing else.
    #[test]
    fn a_hub_without_a_badge_is_not_an_error() {
        let hub = json!({ "tierName": "Supporter", "membershipType": "Member" });
        assert_eq!(hub_badge(&hub), None);
        assert_eq!(text_of(&hub, "membershipDurationOrExpiry"), None);
        assert_eq!(text_of(&hub, "tierName").as_deref(), Some("Supporter"));
    }

    #[test]
    fn reads_a_json_string_from_page_html() {
        let html = r#"junk"INNERTUBE_API_KEY":"AIzaKEY","other":1"#;
        assert_eq!(
            json_after(html, "\"INNERTUBE_API_KEY\":\"").as_deref(),
            Some("AIzaKEY"),
        );
        assert_eq!(json_after(html, "\"MISSING\":\""), None);
    }
}

/// Whether a channel offers memberships, and whether the viewer holds one.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MembershipState {
    /// The channel has memberships to sell. False means the Join button should
    /// not be shown at all: there is nothing to join.
    pub offers: bool,
    /// The viewer already holds a membership.
    pub is_member: bool,
    /// The member's own badge art, the direct equivalent of Twitch's subscriber
    /// badge. Only present when `is_member`.
    pub badge_url: Option<String>,
    /// The tier the viewer is on, e.g. "Channel Supporter".
    pub tier: Option<String>,
    /// How long they have been a member, as YouTube phrases it ("2 months").
    pub duration: Option<String>,
}

/// Read a channel's membership state off its watch page.
///
/// The signal is `membershipButton`, which the authed watch page carries only when
/// the channel actually sells memberships. Verified: present with a "Join" button on
/// a channel that offers them.
///
/// `is_member` is deliberately conservative. The only positive marker observed is
/// the button's own label, and matching on text would break the moment the UI is
/// localised, so a membership is reported ONLY when YouTube stops offering the join
/// action while still showing the button. Reporting "not a member" wrongly leaves
/// the button as it is today; reporting "member" wrongly would hide the way to buy
/// one, so the bias runs that way on purpose.
pub async fn membership_state(identifier: &str) -> Result<MembershipState> {
    let headers = crate::services::youtube_auth_service::auth_headers()
        .ok_or_else(|| anyhow!("Sign into YouTube to do that"))?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let url = live_page_url(identifier);
    // The signed-in page is the one that personalises this button, so the request
    // carries the session rather than going out anonymously.
    let mut req = http.get(&url);
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    let html = req.send().await?.text().await?;
    let Some(initial) = extract_json(&html, "ytInitialData") else {
        return Ok(MembershipState::default());
    };
    // Scraped from the SAME page, so the hub request below speaks the client version
    // and key that page was served with.
    let api_key = json_after(&html, "\"INNERTUBE_API_KEY\":\"").unwrap_or_default();
    let client_version = json_after(&html, "\"INNERTUBE_CONTEXT_CLIENT_VERSION\":\"")
        .unwrap_or_else(|| "2.20240101.00.00".to_string());
    let Some(button) = find_key(&initial, "membershipButton") else {
        // No button at all: this channel sells no memberships.
        return Ok(MembershipState::default());
    };

    // Both states verified against a live signed-in session, one channel the
    // account holds a membership in and one it does not:
    //
    //   MEMBER      navigationEndpoint -> /channel/<UC>/membership   ("manage what I have")
    //   NON-MEMBER  serviceEndpoint    -> ypcGetOffersEndpoint       ("show me what I can buy")
    //
    // Both are structural. The visible labels ("See perks" vs "Join") and the
    // tooltip ("Manage membership") would work today and break the moment the UI is
    // localised, so they are deliberately not used.
    //
    // Signed OUT, the button carries a `modalEndpoint` with a sign-in prompt and
    // neither marker, so it reads as "not a member", which is the safe direction.
    let is_member = holds_membership(button);
    if !is_member {
        return Ok(MembershipState {
            offers: true,
            ..Default::default()
        });
    }

    // Only a member has a badge, so the extra request happens only in that case.
    // The endpoint is taken from the button rather than hardcoded: its `params`
    // select the membership tab, and reading them from the page means a change to
    // that token does not need a code change here.
    let (badge_url, tier, duration) = match button
        .pointer("/buttonRenderer/navigationEndpoint/browseEndpoint")
        .or_else(|| find_key(button, "browseEndpoint"))
    {
        Some(ep) => membership_hub(&http, ep, &headers, &api_key, &client_version).await,
        None => (None, None, None),
    };
    Ok(MembershipState {
        offers: true,
        is_member: true,
        badge_url,
        tier,
        duration,
    })
}

/// The member's badge, tier and duration from the channel's membership hub.
///
/// Shape captured live from a channel the account holds a membership in:
/// `sponsorshipsHubViewModel` carries `tierName`, `membershipType`,
/// `membershipDurationOrExpiry` and `badgeImage.sources[]`. Best-effort throughout:
/// a missing hub costs the badge, never the membership state itself.
async fn membership_hub(
    http: &reqwest::Client,
    endpoint: &Value,
    headers: &[(String, String)],
    api_key: &str,
    client_version: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(browse_id) = endpoint.get("browseId").and_then(|v| v.as_str()) else {
        return (None, None, None);
    };
    let mut body = json!({
        "browseId": browse_id,
        "context": { "client": {
            "clientName": "WEB", "clientVersion": client_version, "hl": "en", "gl": "US"
        } }
    });
    if let Some(p) = endpoint.get("params").and_then(|v| v.as_str()) {
        body["params"] = json!(percent_decode(p));
    }
    let mut req = http
        .post(format!(
            "https://www.youtube.com/youtubei/v1/browse?key={}&prettyPrint=false",
            api_key
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", client_version);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let Ok(resp) = req.json(&body).send().await else {
        return (None, None, None);
    };
    let Ok(value) = resp.json::<Value>().await else {
        return (None, None, None);
    };
    let Some(hub) = find_key(&value, "sponsorshipsHubViewModel") else {
        return (None, None, None);
    };
    (hub_badge(hub), text_of(hub, "tierName"), text_of(hub, "membershipDurationOrExpiry"))
}

/// The largest badge source on a membership hub.
fn hub_badge(hub: &Value) -> Option<String> {
    let url = hub
        .pointer("/badgeImage/sources")?
        .as_array()?
        .last()?
        .get("url")?
        .as_str()?;
    (!url.is_empty()).then(|| url.to_string())
}

fn text_of(hub: &Value, key: &str) -> Option<String> {
    let s = hub.get(key)?.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Whether a `membershipButton` belongs to someone who already holds the
/// membership. See the note above for why this reads endpoints, not labels.
fn holds_membership(button: &Value) -> bool {
    let raw = button.to_string();
    let offers_to_buy = raw.contains("ypcGetOffersEndpoint");
    let manages_existing = raw.contains("/membership");
    manages_existing && !offers_to_buy
}
