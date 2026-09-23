//! Can an anonymous client, holding only a ttwid, see a list of live rooms?
//!
//! This decides whether a Top live feed can ship without a sign-in. The
//! logged-out WEB PAGE renders empty, but the page is a different client from a
//! direct API call carrying a device token, so the page is not evidence on its
//! own. Try every discovery surface and report exactly what each one answers.
//!
//! The answer, so far: every direct surface refuses (the live feed endpoint
//! demands request signatures only TikTok's own page script can produce), yet
//! the logged-out LIVE explore page, given a real viewport, fetches a full feed
//! for itself. That is why StreamNook reads the directory from a hidden page
//! rather than from any of these. Rerun this if TikTok changes its web client.
//!
//! Run: cargo run --example live_discover

use std::time::Duration;
use tiktok_live::http::ttwid::fetch_ttwid;

const T: Duration = Duration::from_secs(15);
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[tokio::main]
async fn main() {
    let ttwid = fetch_ttwid(T, None, None).await.expect("ttwid");
    println!("ttwid acquired\n");

    let client = reqwest::Client::builder()
        .timeout(T)
        .user_agent(UA)
        .build()
        .expect("client");

    let common = "aid=1988&app_language=en&app_name=tiktok_web&browser_language=en-US\
&browser_platform=Win32&browser_name=Mozilla&browser_version=5.0&browser_online=true\
&cookie_enabled=true&device_platform=web_pc&focus_state=true&from_page=live\
&history_len=3&is_fullscreen=false&is_page_visible=true&os=windows&priority_region=US\
&referer=&region=US&screen_height=1080&screen_width=1920&tz_name=America%2FLos_Angeles\
&webcast_language=en";

    let targets: Vec<(&str, String)> = vec![
        (
            "webcast/feed (recommend)",
            format!("https://webcast.tiktok.com/webcast/feed/?{common}&count=10&scene=live_recommend"),
        ),
        (
            "webcast/room/list",
            format!("https://webcast.tiktok.com/webcast/room/list/?{common}&count=10"),
        ),
        (
            "api/live/discover",
            format!("https://www.tiktok.com/api/live/discover/?{common}&count=10"),
        ),
        (
            "api/recommend/user_live",
            format!("https://www.tiktok.com/api/recommend/user_live/?{common}&count=10"),
        ),
        (
            "webcast/explore (categories)",
            format!("https://webcast.tiktok.com/webcast/explore/category/?{common}"),
        ),
        (
            "live page html",
            "https://www.tiktok.com/live".to_string(),
        ),
        // Category pages under /live/explore/. These are ordinary server
        // rendered pages rather than signed API calls, so they may answer where
        // the endpoints above refuse.
        (
            "live/explore/Popular",
            "https://www.tiktok.com/live/explore/Popular".to_string(),
        ),
        (
            "live/explore/Gaming",
            "https://www.tiktok.com/live/explore/Gaming".to_string(),
        ),
    ];

    for (name, url) in targets {
        let res = client
            .get(&url)
            .header("Referer", "https://www.tiktok.com/")
            .header("Cookie", format!("ttwid={ttwid}"))
            .send()
            .await;
        match res {
            Err(e) => println!("{name:<30} ERROR {e}"),
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                let verdict = verdict_of(&body);
                println!("{name:<30} status={status} len={} -> {verdict}", body.len());
                if let Some(sample) = sample_of(&body) {
                    println!("{:>32}{}", "", sample);
                }
                // Any creator handles at all? Even without room ids, a list of
                // names is enough to probe for liveness one by one.
                let handles = handles_in(&body);
                if !handles.is_empty() {
                    println!("{:>32}handles: {}", "", handles.join(" "));
                }
            }
        }
    }
}

/// Did this answer actually carry live rooms?
fn verdict_of(body: &str) -> String {
    if body.is_empty() {
        return "EMPTY BODY".into();
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        let code = v
            .get("status_code")
            .or_else(|| v.get("statusCode"))
            .and_then(|c| c.as_i64());
        let msg = v
            .get("status_msg")
            .or_else(|| v.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("");
        // Count anything that looks like a room id anywhere in the payload.
        let rooms = count_rooms(&v);
        return format!(
            "json status_code={code:?} msg={msg:?} rooms_found={rooms}"
        );
    }
    // HTML: does the rehydration blob mention any live room?
    let live_hits = body.matches("\"roomId\"").count() + body.matches("\"room_id\"").count();
    format!("html, roomId mentions={live_hits}")
}

fn count_rooms(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Object(map) => {
            let mut n = 0;
            for (k, val) in map {
                if (k == "roomId" || k == "room_id" || k == "id_str")
                    && val.as_str().map(|s| s.len() > 8).unwrap_or(false)
                {
                    n += 1;
                }
                n += count_rooms(val);
            }
            n
        }
        serde_json::Value::Array(a) => a.iter().map(count_rooms).sum(),
        _ => 0,
    }
}

/// Creator handles mentioned anywhere in a response, deduped.
fn handles_in(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for marker in ["\"uniqueId\":\"", "\"unique_id\":\""] {
        let mut rest = body;
        while let Some(i) = rest.find(marker) {
            rest = &rest[i + marker.len()..];
            if let Some(end) = rest.find('"') {
                let h = &rest[..end];
                if !h.is_empty() && h.len() <= 24 && !out.iter().any(|x| x == h) {
                    out.push(h.to_string());
                }
            }
            if out.len() >= 25 {
                return out;
            }
        }
    }
    out
}

fn sample_of(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(220).collect::<String>().replace('\n', " "))
}
