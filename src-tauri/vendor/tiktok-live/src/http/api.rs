use std::time::Duration;

use serde_json::Value;

use crate::errors::TikTokLiveError;
use crate::http::ua::{random_ua, system_locale, system_timezone};
use crate::structs::events::{RoomInfo, StreamUrl, TikTokRendition};

const TIKTOK_URL_WEB: &str = "https://www.tiktok.com/";
const TIKTOK_URL_WEBCAST: &str = "https://webcast.tiktok.com/webcast/";

pub struct RoomIdResponse {
    pub room_id: String,
}

/// Shared parameters for standalone HTTP API calls.
///
/// All fields default to auto-detected or `None`. Use struct update syntax:
/// ```ignore
/// fetch_room_id("user", FetchParams { timeout: Duration::from_secs(5), ..Default::default() })
/// ```
#[derive(Clone, Debug)]
pub struct FetchParams<'a> {
    pub timeout: Duration,
    pub cookies: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub proxy: Option<&'a str>,
    pub language: Option<&'a str>,
    pub region: Option<&'a str>,
}

impl Default for FetchParams<'_> {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            cookies: None,
            user_agent: None,
            proxy: None,
            language: None,
            region: None,
        }
    }
}

impl<'a> FetchParams<'a> {
    fn resolve_locale(&self) -> (String, String, String) {
        let (sys_lang, sys_region) = system_locale();
        let lang = match self.language {
            Some(l) => l.to_string(),
            None => sys_lang,
        };
        let reg = match self.region {
            Some(r) => r.to_string(),
            None => sys_region,
        };
        let browser_lang = format!("{lang}-{reg}");
        (lang, reg, browser_lang)
    }
}

fn build_client(params: &FetchParams<'_>) -> Result<reqwest::Client, TikTokLiveError> {
    let ua = params.user_agent.unwrap_or_else(|| random_ua());
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("Referer", "https://www.tiktok.com/".parse().map_err(|e| TikTokLiveError::invalid(e))?);
    if let Some(c) = params.cookies {
        if !c.is_empty() {
            headers.insert("Cookie", c.parse().map_err(|e| TikTokLiveError::invalid(e))?);
        }
    }

    let mut builder = reqwest::Client::builder().timeout(params.timeout).user_agent(ua).default_headers(headers);

    if let Some(proxy_url) = params.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url).map_err(TikTokLiveError::Http)?);
    }

    builder.build().map_err(TikTokLiveError::Http)
}

/// Resolve a TikTok username to a room ID.
///
/// Returns an error if the user doesn't exist or isn't currently live.
/// Language/region auto-detected from system locale when not set in params.
pub async fn fetch_room_id(username: &str, params: FetchParams<'_>) -> Result<RoomIdResponse, TikTokLiveError> {
    let client = build_client(&params)?;
    let clean = username.trim().trim_start_matches('@');
    let (lang, reg, browser_lang) = params.resolve_locale();

    let url = format!(
        "{}api-live/user/room?aid=1988&app_name=tiktok_web&device_platform=web_pc\
        &app_language={lang}&browser_language={browser_lang}&region={reg}&user_is_login=false\
        &uniqueId={}&sourceType=54&staleTime=600000",
        TIKTOK_URL_WEB, clean
    );

    let resp = client.get(&url).send().await?.text().await?;
    let json: Value = serde_json::from_str(&resp)?;

    let status_code = json.get("statusCode").and_then(|v| v.as_i64());
    match status_code {
        Some(0) => {}
        Some(19881007) => return Err(TikTokLiveError::UserNotFound(clean.to_string())),
        Some(code) => return Err(TikTokLiveError::invalid(format!("tiktok api statusCode={code}"))),
        None => return Err(TikTokLiveError::invalid("no statusCode in response")),
    }

    let room_id = json.pointer("/data/user/roomId").and_then(|r| r.as_str()).ok_or(TikTokLiveError::RoomIdMissing)?;

    if room_id.is_empty() || room_id == "0" {
        return Err(TikTokLiveError::HostNotOnline("no active room".into()));
    }

    let live_status = match json.pointer("/data/liveRoom/status").and_then(|v| v.as_i64()) {
        Some(s) => s,
        None => match json.pointer("/data/user/status").and_then(|v| v.as_i64()) {
            Some(s) => s,
            None => 0,
        },
    };

    if live_status != 2 {
        return Err(TikTokLiveError::HostNotOnline(format!("status={live_status}")));
    }

    Ok(RoomIdResponse { room_id: room_id.to_string() })
}

/// Fetch detailed room info: title, viewer counts, stream URLs.
///
/// This is an **optional** call — not needed for WSS event streaming.
///
/// For 18+ rooms, pass session cookies (`"sessionid=xxx; sid_tt=xxx"`) via
/// `FetchParams { cookies: Some("..."), ..Default::default() }`.
/// Without cookies, 18+ rooms return [`TikTokLiveError::AgeRestricted`].
pub async fn fetch_room_info(room_id: &str, params: FetchParams<'_>) -> Result<RoomInfo, TikTokLiveError> {
    let client = build_client(&params)?;
    let tz_raw = system_timezone();
    let tz = urlencoding::encode(&tz_raw);
    let (lang, _reg, browser_lang) = params.resolve_locale();
    let url = format!(
        "{}room/info/?aid=1988&app_name=tiktok_web&device_platform=web_pc\
        &app_language={lang}&browser_language={browser_lang}&browser_name=Mozilla\
        &browser_online=true&browser_platform=Win32\
        &browser_version=5.0+(Windows+NT+10.0%3B+Win64%3B+x64)\
        &cookie_enabled=true&focus_state=true&from_page=user\
        &screen_height=1080&screen_width=1920\
        &tz_name={tz}&webcast_language={lang}\
        &room_id={}",
        TIKTOK_URL_WEBCAST, room_id
    );

    let resp = client.get(&url).send().await?;
    let status = resp.status();
    let body = resp.text().await?;

    if body.is_empty() {
        return Err(TikTokLiveError::invalid(format!("empty response from room/info (http {})", status)));
    }

    let json: Value = serde_json::from_str(&body)?;

    match json.get("status_code").and_then(|v| v.as_i64()) {
        Some(0) => {}
        Some(4003110) => {
            return Err(TikTokLiveError::AgeRestricted(
                "18+ room — pass session cookies to fetch_room_info()".into(),
            ));
        }
        Some(code) => {
            return Err(TikTokLiveError::invalid(format!("room/info status_code={code}")));
        }
        None => {}
    }

    let data = json["data"].as_object().ok_or_else(|| TikTokLiveError::invalid("missing 'data' in room info"))?;

    let title = data.get("title").and_then(|v| v.as_str()).unwrap_or_default();
    let viewers = data.get("user_count").and_then(|v| v.as_i64()).unwrap_or_default();
    let stats = data.get("stats").and_then(|v| v.as_object());
    let likes = stats.and_then(|s| s.get("like_count")).and_then(|v| v.as_i64()).unwrap_or_default();
    let total_viewers = stats.and_then(|s| s.get("total_user")).and_then(|v| v.as_i64()).unwrap_or_default();

    let stream_url = parse_stream_urls(&json);

    Ok(RoomInfo {
        title: title.to_string(),
        viewers,
        likes,
        total_viewers,
        stream_url,
        raw_json: body,
    })
}

/// Fallback ordering for a tier when the room publishes no qualities ladder.
/// Mirrors the order TikTok's own player presents them in.
fn tier_rank(tier: &str) -> i64 {
    match tier {
        "origin" => 50,
        "uhd" => 40,
        "hd" => 30,
        "sd" => 20,
        "ld" => 10,
        "ao" => 0,
        // An unrecognised tier sorts below every named one except audio, which
        // is where an unknown addition is least likely to do harm.
        _ => 5,
    }
}

/// A URL counts as published only when it is a non-empty http(s) string.
///
/// TikTok marks a packaging it did not produce with an EMPTY STRING rather than
/// by omitting the key, so a plain `as_str()` reads `""` as a valid URL and the
/// caller then tries to play nothing.
fn url_of(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        Some(s.to_string())
    } else {
        None
    }
}

fn url_at(v: &Value, pointer: &str) -> Option<String> {
    url_of(v.pointer(pointer)?)
}

/// Read a number that may arrive as a JSON number or as a string. Both forms
/// appear in `sdk_params` depending on the room.
fn loose_num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Pull the first present key out of an object, trying each spelling in turn.
fn first_key<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| v.get(*k))
}

/// Decode one tier's `sdk_params`, itself a JSON string, into the geometry and
/// codec fields. Returns defaults rather than failing: the URLs are the part
/// that must be right, and a missing resolution only costs menu detail.
fn apply_sdk_params(r: &mut TikTokRendition, raw: Option<&Value>) {
    let Some(text) = raw.and_then(|v| v.as_str()) else {
        return;
    };
    let Ok(p) = serde_json::from_str::<Value>(text) else {
        return;
    };

    // "resolution" is "<width>x<height>", portrait for a normal TikTok LIVE.
    if let Some(res) = first_key(&p, &["resolution", "Resolution"]).and_then(|v| v.as_str()) {
        if let Some((w, h)) = res.split_once('x') {
            r.width = w.trim().parse::<u32>().ok();
            r.height = h.trim().parse::<u32>().ok();
        }
    }
    r.fps = loose_num(first_key(&p, &["fps", "FPS", "Fps"]));
    r.vbitrate = loose_num(first_key(&p, &["vbitrate", "VBitrate", "bitrate"])).map(|n| n as u64);
    r.vcodec = first_key(&p, &["VCodec", "vcodec", "v_codec"])
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase());
}

/// Every tier a LIVE room publishes, best first, with the legacy FLV fields
/// derived from the same pass so the two can never disagree.
///
/// Three shapes are handled, in descending order of richness:
///   1. `pull_data.stream_data`, a JSON STRING holding per-tier `flv` / `hls` /
///      `cmaf` / `dash` plus `sdk_params`. This is what a current room serves.
///   2. `pull_data.options.qualities`, the ladder that names and orders the
///      tiers. Used to label and sort the above when present.
///   3. `hls_pull_url_map` / `flv_pull_url`, the older flat maps. Some rooms
///      still carry only these.
fn parse_stream_urls(json: &Value) -> Option<StreamUrl> {
    let mut renditions: Vec<TikTokRendition> = Vec::new();

    // The ladder lives on the OUTER json, not inside the nested string, and is
    // the only authoritative source for tier naming and order.
    let ladder = json.pointer("/data/stream_url/live_core_sdk_data/pull_data/options/qualities");
    let default_tier = json
        .pointer("/data/stream_url/live_core_sdk_data/pull_data/options/default_quality/sdk_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if let Some(nested) = json
        .pointer("/data/stream_url/live_core_sdk_data/pull_data/stream_data")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    {
        // Enumerate whatever tiers the room actually carries rather than asking
        // for a fixed five, so a tier TikTok adds later needs no code change.
        if let Some(map) = nested.pointer("/data").and_then(|v| v.as_object()) {
            for (tier, body) in map {
                let mut r = TikTokRendition {
                    tier: tier.to_ascii_lowercase(),
                    label: tier.to_ascii_lowercase(),
                    level: tier_rank(&tier.to_ascii_lowercase()),
                    flv: url_at(body, "/main/flv"),
                    hls: url_at(body, "/main/hls"),
                    cmaf: url_at(body, "/main/cmaf"),
                    dash: url_at(body, "/main/dash"),
                    ..Default::default()
                };
                apply_sdk_params(&mut r, body.pointer("/main/sdk_params"));
                if !r.is_empty() {
                    renditions.push(r);
                }
            }
        }
    }

    // Label and order from the ladder where it agrees with what we found.
    if let Some(list) = ladder.and_then(|v| v.as_array()) {
        for q in list {
            let Some(key) = q.get("sdk_key").and_then(|v| v.as_str()) else {
                continue;
            };
            let key = key.to_ascii_lowercase();
            if let Some(r) = renditions.iter_mut().find(|r| r.tier == key) {
                if let Some(name) = q.get("name").and_then(|v| v.as_str()) {
                    if !name.trim().is_empty() {
                        r.label = name.trim().to_string();
                    }
                }
                if let Some(level) = loose_num(q.get("level")) {
                    r.level = level as i64;
                }
                if r.vcodec.is_none() {
                    r.vcodec = q
                        .get("v_codec")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_ascii_lowercase());
                }
            }
        }
    }

    // Older rooms carry only the flat maps. Their keys are TikTok's legacy
    // quality names, which map onto the modern tiers.
    if renditions.is_empty() {
        let legacy = |name: &str| -> &'static str {
            match name.to_ascii_uppercase().as_str() {
                "FULL_HD1" => "uhd",
                "HD1" => "hd",
                "SD1" => "sd",
                "SD2" => "ld",
                _ => "hd",
            }
        };
        for (field, pick) in [("hls_pull_url_map", true), ("flv_pull_url", false)] {
            let Some(map) = json
                .pointer(&format!("/data/stream_url/{}", field))
                .and_then(|v| v.as_object())
            else {
                continue;
            };
            for (name, raw) in map {
                let tier = legacy(name).to_string();
                let Some(url) = url_of(raw) else { continue };
                if let Some(r) = renditions.iter_mut().find(|r| r.tier == tier) {
                    if pick {
                        r.hls.get_or_insert(url);
                    } else {
                        r.flv.get_or_insert(url);
                    }
                    continue;
                }
                renditions.push(TikTokRendition {
                    tier: tier.clone(),
                    label: tier.clone(),
                    level: tier_rank(&tier),
                    hls: if pick { Some(url.clone()) } else { None },
                    flv: if pick { None } else { Some(url) },
                    ..Default::default()
                });
            }
        }
        // The flat single-URL form, when even the maps are absent.
        if renditions.is_empty() {
            if let Some(url) = url_at(json, "/data/stream_url/hls_pull_url") {
                renditions.push(TikTokRendition {
                    tier: "hd".to_string(),
                    label: "hd".to_string(),
                    level: tier_rank("hd"),
                    hls: Some(url),
                    ..Default::default()
                });
            }
        }
    }

    if renditions.is_empty() {
        return None;
    }

    // Best first, which is the order `select`-style helpers assume.
    renditions.sort_by(|a, b| b.level.cmp(&a.level).then_with(|| a.tier.cmp(&b.tier)));

    let flv_of = |tier: &str| -> Option<String> {
        renditions
            .iter()
            .find(|r| r.tier == tier)
            .and_then(|r| r.flv.clone())
    };

    Some(StreamUrl {
        flv_origin: flv_of("origin"),
        // Preserves the original fallback: a room with no `hd` reports `uhd` here.
        flv_hd: flv_of("hd").or_else(|| flv_of("uhd")),
        flv_sd: flv_of("sd"),
        flv_ld: flv_of("ld"),
        flv_ao: flv_of("ao"),
        renditions,
        default_tier,
    })
}

#[cfg(test)]
mod stream_url_tests {
    use super::*;
    use serde_json::json;

    /// A room shaped the way a current LIVE serves: per-tier packagings inside
    /// the `stream_data` STRING, plus the ladder that names and orders them.
    fn modern_room() -> Value {
        let nested = json!({
            "data": {
                "origin": { "main": {
                    "flv": "https://pull.example.com/origin.flv",
                    "hls": "",
                    "cmaf": "https://pull.example.com/origin.m3u8",
                    "dash": "",
                    "sdk_params": "{\"VCodec\":\"bytevc1\",\"resolution\":\"1080x1920\",\"fps\":30,\"vbitrate\":2500000}"
                }},
                "hd": { "main": {
                    "flv": "https://pull.example.com/hd.flv",
                    "hls": "https://pull.example.com/hd.m3u8",
                    "cmaf": "",
                    "sdk_params": "{\"VCodec\":\"h264\",\"resolution\":\"720x1280\",\"fps\":\"30\",\"vbitrate\":\"1200000\"}"
                }},
                "ao": { "main": {
                    "flv": "https://pull.example.com/ao.flv",
                    "hls": "",
                    "sdk_params": "{\"VCodec\":\"\",\"resolution\":\"\"}"
                }}
            }
        });
        json!({
            "data": { "stream_url": { "live_core_sdk_data": { "pull_data": {
                "stream_data": nested.to_string(),
                "options": {
                    "default_quality": { "sdk_key": "hd" },
                    "qualities": [
                        { "name": "Origin", "sdk_key": "origin", "level": 4, "v_codec": "bytevc1" },
                        { "name": "HD",     "sdk_key": "hd",     "level": 3 },
                        { "name": "Audio",  "sdk_key": "ao",     "level": 0 }
                    ]
                }
            }}}}
        })
    }

    #[test]
    fn reads_every_packaging_and_orders_best_first() {
        let s = parse_stream_urls(&modern_room()).expect("renditions");
        let tiers: Vec<&str> = s.renditions.iter().map(|r| r.tier.as_str()).collect();
        assert_eq!(tiers, vec!["origin", "hd", "ao"]);
        assert_eq!(s.default_tier.as_deref(), Some("hd"));

        let origin = &s.renditions[0];
        assert_eq!(origin.label, "Origin");
        assert_eq!(origin.cmaf.as_deref(), Some("https://pull.example.com/origin.m3u8"));
        assert_eq!(origin.flv.as_deref(), Some("https://pull.example.com/origin.flv"));
        // An unpublished packaging is an EMPTY STRING, not a missing key.
        assert_eq!(origin.hls, None, "empty string must not read as a url");
        assert_eq!(origin.dash, None);
    }

    #[test]
    fn decodes_sdk_params_in_both_number_forms() {
        let s = parse_stream_urls(&modern_room()).unwrap();
        let origin = &s.renditions[0];
        assert_eq!((origin.width, origin.height), (Some(1080), Some(1920)));
        assert_eq!(origin.fps, Some(30.0));
        assert_eq!(origin.vbitrate, Some(2_500_000));
        assert_eq!(origin.vcodec.as_deref(), Some("bytevc1"));

        // The same fields arriving as strings must parse identically.
        let hd = &s.renditions[1];
        assert_eq!((hd.width, hd.height), (Some(720), Some(1280)));
        assert_eq!(hd.fps, Some(30.0));
        assert_eq!(hd.vbitrate, Some(1_200_000));
    }

    #[test]
    fn short_side_is_the_portrait_width() {
        let s = parse_stream_urls(&modern_room()).unwrap();
        // 1080x1920 portrait: selecting on `height` would call this a 1920p
        // source and match a "720p" request against the wrong rung.
        assert_eq!(s.renditions[0].short_side(), Some(1080));
        assert_eq!(s.renditions[1].short_side(), Some(720));
    }

    #[test]
    fn legacy_flv_fields_still_derive() {
        let s = parse_stream_urls(&modern_room()).unwrap();
        assert_eq!(s.flv_origin.as_deref(), Some("https://pull.example.com/origin.flv"));
        assert_eq!(s.flv_hd.as_deref(), Some("https://pull.example.com/hd.flv"));
        assert_eq!(s.flv_ao.as_deref(), Some("https://pull.example.com/ao.flv"));
        assert_eq!(s.flv_sd, None);
        assert_eq!(s.flv_ld, None);
    }

    #[test]
    fn hd_falls_back_to_uhd_as_it_always_did() {
        let nested = json!({ "data": { "uhd": { "main": {
            "flv": "https://pull.example.com/uhd.flv"
        }}}});
        let room = json!({ "data": { "stream_url": { "live_core_sdk_data": { "pull_data": {
            "stream_data": nested.to_string()
        }}}}});
        let s = parse_stream_urls(&room).unwrap();
        assert_eq!(s.flv_hd.as_deref(), Some("https://pull.example.com/uhd.flv"));
    }

    #[test]
    fn falls_back_to_the_flat_maps_when_stream_data_is_absent() {
        let room = json!({ "data": { "stream_url": {
            "hls_pull_url_map": {
                "FULL_HD1": "https://pull.example.com/fhd.m3u8",
                "SD1": "https://pull.example.com/sd.m3u8"
            }
        }}});
        let s = parse_stream_urls(&room).expect("legacy map");
        let tiers: Vec<&str> = s.renditions.iter().map(|r| r.tier.as_str()).collect();
        assert_eq!(tiers, vec!["uhd", "sd"], "best first, legacy names mapped");
        assert_eq!(s.renditions[0].hls.as_deref(), Some("https://pull.example.com/fhd.m3u8"));
    }

    #[test]
    fn a_room_with_nothing_playable_reports_none() {
        let nested = json!({ "data": { "hd": { "main": { "flv": "", "hls": "" }}}});
        let room = json!({ "data": { "stream_url": { "live_core_sdk_data": { "pull_data": {
            "stream_data": nested.to_string()
        }}}}});
        assert!(parse_stream_urls(&room).is_none());
        assert!(parse_stream_urls(&json!({ "data": {} })).is_none());
    }
}
