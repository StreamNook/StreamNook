// Twitch stream resolution primitives (GQL PlaybackAccessToken to usher master,
// plus out-of-band entitlement detection). These are the building blocks
// `twitch_resolver` composes. The relay is ad-neutral: nothing here races
// proxies, splices masters, or strips segments; a playback plugin that owns
// resolution can hand the relay an upstream instead (see docs/plugins/HOOKS.md).

use anyhow::{anyhow, Context, Result};
use base64::prelude::{Engine as _, BASE64_STANDARD};
use log::{warn};
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) const TWITCH_WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
pub(crate) const PLAYBACK_ACCESS_TOKEN_HASH: &str =
    "ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9";
pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:146.0) Gecko/20100101 Firefox/146.0";

// Out-of-band entitlement detection.
//
// The PlaybackAccessToken `turbo`/`subscriber` flags are vestigial — verified
// 2026-06-02 they read false even for a Turbo account fetched from twitch.tv
// itself, so they CANNOT decide ad-free routing. Instead we ask Twitch
// directly: `currentUser.hasTurbo` (account-wide) and
// `user.self.subscriptionBenefit` (per channel). Twitch applies the ad bypass
// at the usher/SSAI layer keyed on the authenticated session, so the authed
// manifest is genuinely ad-free for an entitled viewer (proven: anonymous
// manifests stitch a `twitch-stitched-ad` pod at the same instant the authed
// one stays clean). When entitled we serve the authed master directly.
const TURBO_TTL: Duration = Duration::from_secs(1800); // 30 min; Turbo rarely changes
const SUB_TTL: Duration = Duration::from_secs(600); // 10 min per channel

// Both caches key by the viewer's OAuth token, so switching accounts (a
// different token) or a token refresh re-checks entitlement instead of serving
// the previous account's status. A Turbo main account would otherwise keep an
// alternate account flagged ad-free until the TTL expired.
static TURBO_CACHE: OnceCell<Mutex<HashMap<String, (bool, Instant)>>> = OnceCell::new();
static SUB_CACHE: OnceCell<Mutex<HashMap<(String, String), (bool, Instant)>>> = OnceCell::new();

/// Drop every cached Turbo/sub verdict. Called on account switch and logout so
/// the next resolution re-checks entitlement for the now-active account rather
/// than serving a verdict cached against the previous account's token. The
/// caches key by token, so a genuinely different token would re-check on its own
/// once the TTL lapsed; this just collapses that window to zero on a known
/// account transition.
pub(crate) fn clear_entitlement_caches() {
    if let Some(cache) = TURBO_CACHE.get() {
        cache.lock().unwrap().clear();
    }
    if let Some(cache) = SUB_CACHE.get() {
        cache.lock().unwrap().clear();
    }
}

/// POST an inline GQL query with the viewer's web cookie; return the JSON body.
async fn gql_query(oauth_token: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(USER_AGENT)
        .build()?;
    let resp: serde_json::Value = client
        .post("https://gql.twitch.tv/gql")
        .header("Authorization", format!("OAuth {}", oauth_token))
        .header("Client-ID", TWITCH_WEB_CLIENT_ID)
        .json(&body)
        .send()
        .await
        .context("gql request failed")?
        .json()
        .await
        .context("gql response not JSON")?;
    Ok(resp)
}

/// Account-wide Turbo status (`currentUser.hasTurbo`). Cached per OAuth token
/// for `TURBO_TTL`. A Turbo account is ad-free on EVERY channel via the
/// authenticated stream.
pub(crate) async fn account_has_turbo(oauth_token: &str) -> bool {
    {
        let lock = TURBO_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some((val, at)) = lock.lock().unwrap().get(oauth_token).copied() {
            if at.elapsed() < TURBO_TTL {
                return val;
            }
        }
    }
    let body = serde_json::json!({ "query": "query{currentUser{hasTurbo}}" });
    let turbo = match gql_query(oauth_token, body).await {
        Ok(v) => v
            .pointer("/data/currentUser/hasTurbo")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        Err(e) => {
            warn!("[AuthProxy] hasTurbo query failed: {}", e);
            false
        }
    };
    TURBO_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(oauth_token.to_string(), (turbo, Instant::now()));
    turbo
}

/// Whether the viewer has an active sub to `channel`
/// (`user.self.subscriptionBenefit` non-null). Cached per channel for `SUB_TTL`.
/// A subbed channel is ad-free via the authenticated stream — same usher
/// mechanism as Turbo, scoped to that channel.
pub(crate) async fn is_subscribed(channel: &str, oauth_token: &str) -> bool {
    let key = (oauth_token.to_string(), channel.to_string());
    {
        let lock = SUB_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some((val, at)) = lock.lock().unwrap().get(&key).copied() {
            if at.elapsed() < SUB_TTL {
                return val;
            }
        }
    }
    let body = serde_json::json!({
        "query": "query($l:String!){user(login:$l){self{subscriptionBenefit{id}}}}",
        "variables": { "l": channel }
    });
    let subbed = match gql_query(oauth_token, body).await {
        Ok(v) => v
            .pointer("/data/user/self/subscriptionBenefit")
            .map(|x| !x.is_null())
            .unwrap_or(false),
        Err(e) => {
            warn!("[AuthProxy] sub-status query for {} failed: {}", channel, e);
            false
        }
    };
    SUB_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(key, (subbed, Instant::now()));
    subbed
}

/// How a channel's playback resolved, surfaced to the UI (so it can say
/// "Subscribed: native ad-free" vs "Resolved by a playback plugin: EU").
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaybackStatus {
    pub channel: String,
    /// "turbo" | "subscribed" | "auth-only" | "plugin"
    pub mode: String,
    /// True when relying on Twitch's own ad-free entitlement.
    pub entitled: bool,
    /// Upstream base a resolution-owning plugin reported, when one resolved.
    pub proxy_base: Option<String>,
    /// Region label the plugin reported for that base, best-effort.
    pub proxy_region: Option<String>,
}

static STATUS: OnceCell<Mutex<HashMap<String, PlaybackStatus>>> = OnceCell::new();

pub(crate) fn set_status(status: PlaybackStatus) {
    let lock = STATUS.get_or_init(|| Mutex::new(HashMap::new()));
    lock.lock().unwrap().insert(status.channel.clone(), status);
}

/// Most recent playback decision for a channel (for the UI).
pub fn get_status(channel: &str) -> Option<PlaybackStatus> {
    let lock = STATUS.get_or_init(|| Mutex::new(HashMap::new()));
    let map = lock.lock().unwrap();
    map.get(channel).cloned()
}

/// Direct Twitch fetch: GQL playbackAccessToken then usher master. Sends the
/// viewer's own credential when present (entitled accounts get their ad-free
/// master this way); works anonymously otherwise, same as the logged-out web
/// player. Returns the master playlist body.
pub(crate) async fn fetch_auth_master(channel: &str, oauth_token: Option<&str>) -> Result<String> {
    // Dev switch: drop the viewer credential so the playback token carries no
    // entitlement. Lets an ad-free account (Turbo/sub) reproduce what un-
    // entitled viewers receive — server-side ad splices in particular — which
    // is otherwise untestable from such an account. Set
    // STREAMNOOK_FORCE_ANON_PLAYBACK=1 in the dev shell; never set in release.
    let oauth_token = if std::env::var("STREAMNOOK_FORCE_ANON_PLAYBACK")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
    {
        warn!("[AuthProxy] STREAMNOOK_FORCE_ANON_PLAYBACK set: requesting {channel} anonymously");
        None
    } else {
        oauth_token
    };
    // Region unlock (below) only helps an authenticated viewer: an anonymous token
    // stays not-logged-in blocked from the high tiers regardless of fetch region.
    let authenticated = oauth_token.is_some();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(USER_AGENT)
        .build()?;

    let gql_body = serde_json::json!({
        "operationName": "PlaybackAccessToken",
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": PLAYBACK_ACCESS_TOKEN_HASH,
            }
        },
        "variables": {
            "isLive": true,
            "login": channel,
            "isVod": false,
            "vodID": "",
            "playerType": "embed",
            "platform": "site",
        }
    });

    let mut req = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-ID", TWITCH_WEB_CLIENT_ID)
        .json(&gql_body);
    if let Some(token) = oauth_token {
        req = req.header("Authorization", format!("OAuth {}", token));
    }
    let gql_resp: serde_json::Value = req
        .send()
        .await
        .context("GQL request failed")?
        .json()
        .await
        .context("GQL response not JSON")?;

    let pat = gql_resp
        .get("data")
        .and_then(|d| d.get("streamPlaybackAccessToken"))
        .ok_or_else(|| anyhow!("GQL missing streamPlaybackAccessToken: {}", gql_resp))?;
    let sig = pat
        .get("signature")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no sig"))?;
    let value = pat
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no token value"))?;

    let usher = format!(
        "https://usher.ttvnw.net/api/channel/hls/{ch}.m3u8\
         ?platform=web&player_type=embed&allow_source=true&allow_audio_only=true\
         &playlist_include_framerate=true&supported_codecs=av1,h264,h265&fast_bread=true\
         &include_unavailable=true&sig={sig}&token={tok}",
        ch = channel,
        sig = sig,
        tok = urlencoding::encode(value),
    );

    let resp = client
        .get(&usher)
        .header("Referer", "https://player.twitch.tv")
        .header("Origin", "https://player.twitch.tv")
        .send()
        .await
        .context("usher request failed")?;
    if !resp.status().is_success() {
        return Err(anyhow!("usher returned {}", resp.status()));
    }
    let master = resp.text().await?;

    // Region unlock: when logged in but geo-blocked from a high tier (1440p/2160p,
    // flagged AUTHZ_GEO in the include_unavailable blob), fetch a region master
    // through the streamnook.app relay and graft ONLY the unlocked high tiers onto
    // the viewer's own master. The base tiers keep their locally-signed URLs, which
    // play reliably from here; only the added tier rides the relay's region-signed
    // URL. This never swaps a tier the viewer already had, so playback of the base
    // qualities can't regress if a relay URL doesn't hold up from the viewer's
    // location. Best-effort, and only fires for an authenticated, geo-blocked viewer.
    if authenticated && geo_blocked_tier_present(&master) {
        // Authenticated but geo-blocked from the high tiers: graft the
        // region-unlocked tiers on. authenticated == oauth_token.is_some().
        let oauth = oauth_token.expect("authenticated implies an oauth token");
        match recover_geo_blocked_master(channel, oauth).await {
            Some(relay_master) => return Ok(splice_missing_tiers(&master, &relay_master)),
            None => log::debug!("[auth_proxy] {channel}: geo-unlock relay recover failed"),
        }
    }

    Ok(master)
}

/// The streamnook.app relay that re-fetches a usher master from an allowed region,
/// so a viewer geo-blocked from the high tiers gets one carrying their real signed
/// URLs. See the QualityRelay Durable Object in the streamnook.app worker.
const QUALITY_RELAY_URL: &str = "https://modroom.streamnook.app/quality-master";

/// The relay's media-playlist proxy. A region-unlocked tier's playlist is routed
/// through this so its live reloads keep egressing from the relay's region (the
/// geo gate is re-checked on every reload). Segments still load direct.
const QUALITY_MEDIA_URL: &str = "https://modroom.streamnook.app/quality-media";

/// Wrap a region-signed media-playlist URL so the player fetches it (and every
/// live reload) through the relay's region instead of directly from the viewer's
/// blocked location. This is what makes an unlocked tier actually playable, not
/// just present in the menu.
fn relay_media_url(original: &str) -> String {
    format!(
        "{}?region=weur&u={}",
        QUALITY_MEDIA_URL,
        urlencoding::encode(original)
    )
}

/// True when `master` describes a rendition Twitch hid from THIS viewer for their
/// country (`AUTHZ_GEO`), read from the `include_unavailable` session-data blob.
/// That is the only block the region relay can lift; a not-logged-in block is
/// cleared by the viewer's own login, not by the fetch region.
fn geo_blocked_tier_present(master: &str) -> bool {
    for line in master.lines() {
        let l = line.trim_start();
        if !l.contains("com.amazon.ivs.unavailable-media") {
            continue;
        }
        let Some(b64) = extract_quoted_attr(l, "VALUE") else {
            continue;
        };
        let Ok(bytes) = BASE64_STANDARD.decode(b64.as_bytes()) else {
            continue;
        };
        let Ok(entries) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) else {
            continue;
        };
        for e in entries {
            let geo = e
                .get("AUTHORIZATION_REASONS")
                .and_then(|r| r.as_array())
                .map(|a| a.iter().any(|v| v.as_str() == Some("AUTHZ_GEO")))
                .unwrap_or(false);
            if geo {
                return true;
            }
        }
    }
    false
}

/// Pull `KEY="..."` out of an HLS tag line.
fn extract_quoted_attr(line: &str, key: &str) -> Option<String> {
    let needle = format!("{}=\"", key);
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The video height of an `#EXT-X-STREAM-INF` line, from `RESOLUTION=WxH`
/// (unquoted) or the `NAME="1440p60"` label. None for audio-only.
fn stream_inf_height(inf: &str) -> Option<u32> {
    if let Some(pos) = inf.find("RESOLUTION=") {
        let rest = &inf[pos + "RESOLUTION=".len()..];
        let val = rest.split(',').next().unwrap_or(rest);
        if let Some(h) = val.split(['x', 'X']).nth(1).and_then(|h| h.trim().parse::<u32>().ok()) {
            return Some(h);
        }
    }
    if let Some(name) = extract_quoted_attr(inf, "NAME") {
        let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(h) = digits.parse::<u32>() {
            return Some(h);
        }
    }
    None
}

/// Heights of every video rendition in `master`.
fn stream_inf_heights(master: &str) -> std::collections::HashSet<u32> {
    let mut set = std::collections::HashSet::new();
    for line in master.lines() {
        if line.trim_start().starts_with("#EXT-X-STREAM-INF:") {
            if let Some(h) = stream_inf_height(line) {
                set.insert(h);
            }
        }
    }
    set
}

/// The `#EXT-X-STREAM-INF` (plus any preceding `#EXT-X-MEDIA` tag and the URL)
/// blocks in `master` whose height is NOT already in `have`.
fn missing_tier_blocks(master: &str, have: &std::collections::HashSet<u32>) -> Vec<String> {
    let lines: Vec<&str> = master.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let inf = lines[i];
        if !inf.trim_start().starts_with("#EXT-X-STREAM-INF:") {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < lines.len()
            && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with('#'))
        {
            j += 1;
        }
        if j >= lines.len() {
            break;
        }
        if let Some(h) = stream_inf_height(inf) {
            if !have.contains(&h) {
                let mut block = String::new();
                if i > 0 {
                    let prev = lines[i - 1].trim_start();
                    if prev.starts_with("#EXT-X-MEDIA:") && prev.contains("TYPE=VIDEO") {
                        block.push_str(lines[i - 1]);
                        block.push('\n');
                    }
                }
                block.push_str(inf);
                block.push('\n');
                // Route this unlocked tier's playlist reloads through the relay's
                // region so it stays playable from a blocked location.
                block.push_str(&relay_media_url(lines[j].trim()));
                out.push(block);
            }
        }
        i = j + 1;
    }
    out
}

/// Keep `base` (the viewer's own locally-signed master, whose tiers play reliably
/// from their location) and append only the video renditions `extra` (the region
/// relay's master) has that `base` lacks — the unlocked high tiers. Never swaps a
/// base tier for a region-signed URL, so the qualities the viewer already had can't
/// regress if a relay URL fails to play from where they are.
fn splice_missing_tiers(base: &str, extra: &str) -> String {
    let have = stream_inf_heights(base);
    let blocks = missing_tier_blocks(extra, &have);
    if blocks.is_empty() {
        return base.to_string();
    }
    let mut out = base.trim_end().to_string();
    for b in &blocks {
        out.push('\n');
        out.push_str(b);
    }
    out.push('\n');
    out
}

/// Ask the relay to re-fetch `channel`'s master from an allowed region. AUTHZ_GEO
/// is baked into the playback token when it is MINTED (by the mint request's IP),
/// so the relay has to mint it in-region — which needs the viewer's login. We hand
/// the relay the OAuth token over HTTPS; it mints + fetches from Europe and never
/// stores it. Returns the region master when it is a usable playlist, else None so
/// the caller keeps the local one.
async fn recover_geo_blocked_master(channel: &str, oauth: &str) -> Option<String> {
    let mut req = crate::services::http::client()
        .post(QUALITY_RELAY_URL)
        .json(&serde_json::json!({
            "channel": channel,
            "oauth": oauth,
            "region": "weur",
        }));
    if let Some(key) = option_env!("STREAMNOOK_RELAY_KEY") {
        req = req.header("X-SN-Relay-Key", key);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let master = resp.text().await.ok()?;
    master.contains("#EXT-X-STREAM-INF").then_some(master)
}

/// Pull an attribute value out of an `EXT-X-STREAM-INF` line. Handles both
/// quoted (CODECS="...", VIDEO="...") and unquoted (RESOLUTION=1920x1080) forms.
pub(crate) fn extract_attr(line: &str, key: &str) -> Option<String> {
    let needle_q = format!("{}=\"", key);
    if let Some(pos) = line.find(&needle_q) {
        let rest = &line[pos + needle_q.len()..];
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    let needle = format!("{}=", key);
    let pos = line.find(&needle)?;
    let rest = &line[pos + needle.len()..];
    let end = rest.find(',').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}
