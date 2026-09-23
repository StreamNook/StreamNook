//! Resolve one live TikTok room and report everything the watch path depends on.
//!
//! Answers, in one pass against a real room:
//!   1. which packagings (cmaf / hls / flv / dash) each tier actually publishes,
//!      and how often a field is present but EMPTY
//!   2. whether the pull CDN sends `access-control-allow-origin`
//!   3. whether it accepts a fetch with no `Referer`
//!   4. what a media playlist actually contains (absolute vs relative URIs,
//!      `#EXT-X-MAP`, `#EXT-X-PART`), which decides the relay's rewrite rules
//!
//! Run: cargo run --example live_probe -- <handle> [more handles...]

use std::time::Duration;
use tiktok_live::http::api::{fetch_room_info, FetchParams};
use tiktok_live::http::sigi::scrape_profile;
use tiktok_live::http::ttwid::fetch_ttwid;

const T: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--wait` polls until the first handle goes live, so a probe can be armed
    // before the broadcast starts rather than raced against it.
    let wait = args.iter().any(|a| a == "--wait");
    // `--raw` prints each tier's untouched `sdk_params`, for when a field the
    // parser expects turns out to be spelled differently or missing.
    let raw = args.iter().any(|a| a == "--raw");
    args.retain(|a| a != "--wait" && a != "--raw");
    let handles: Vec<String> = args;
    if handles.is_empty() {
        eprintln!("usage: live_probe [--wait] <handle> [handle...]");
        std::process::exit(2);
    }

    if wait {
        let h = handles[0].trim().trim_start_matches('@').to_lowercase();
        let ttwid = fetch_ttwid(T, None, None).await.expect("ttwid");
        // Unattended, so the cadence is light: a broadcast is not missed by
        // half a minute, and a tighter loop is hundreds of profile fetches an
        // hour from one address for no gain.
        let every = Duration::from_secs(30);
        println!("waiting for @{h} to go live (checking every 30s, Ctrl-C to stop)");
        loop {
            match scrape_profile(&h, &ttwid, T, None, None, None).await {
                Ok(p) if !p.room_id.is_empty() => {
                    println!("@{h} is LIVE (room {})\n", p.room_id);
                    break;
                }
                Ok(_) => print!("."),
                Err(e) => print!("[{e}]"),
            }
            use std::io::Write;
            let _ = std::io::stdout().flush();
            tokio::time::sleep(every).await;
        }
    }

    let ttwid = match fetch_ttwid(T, None, None).await {
        Ok(t) => {
            println!("ttwid acquired ({} chars)\n", t.len());
            t
        }
        Err(e) => {
            eprintln!("FATAL: could not get a ttwid: {e}");
            std::process::exit(1);
        }
    };

    for handle in &handles {
        let h = handle.trim().trim_start_matches('@').to_lowercase();
        println!("=== @{h} ===");
        let profile = match scrape_profile(&h, &ttwid, T, None, None, None).await {
            Ok(p) => p,
            Err(e) => {
                println!("  scrape failed: {e}\n");
                continue;
            }
        };
        println!(
            "  nickname={:?} user_id={} followers={} verified={} room_id={:?}",
            profile.nickname,
            profile.user_id,
            profile.follower_count,
            profile.verified,
            profile.room_id
        );
        if profile.room_id.is_empty() {
            println!("  NOT LIVE\n");
            continue;
        }

        let info = match fetch_room_info(&profile.room_id, FetchParams { timeout: T, ..Default::default() }).await {
            Ok(i) => i,
            Err(e) => {
                println!("  room/info failed: {e}\n");
                continue;
            }
        };
        println!("  title={:?} viewers={}", info.title, info.viewers);

        // create_time is what finally fills `started_at`, so confirm it is there.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&info.raw_json) {
            println!(
                "  create_time={:?}",
                v.pointer("/data/create_time").and_then(|c| c.as_i64())
            );
            if raw {
                let nested = v
                    .pointer("/data/stream_url/live_core_sdk_data/pull_data/stream_data")
                    .and_then(|s| s.as_str())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
                if let Some(map) = nested.as_ref().and_then(|n| n.pointer("/data")).and_then(|d| d.as_object()) {
                    for (tier, body) in map {
                        println!(
                            "  raw sdk_params[{tier}] = {}",
                            body.pointer("/main/sdk_params").and_then(|p| p.as_str()).unwrap_or("<none>")
                        );
                    }
                }
            }
        }

        let Some(su) = info.stream_url else {
            println!("  NO stream_url parsed (this is the interesting failure)\n");
            continue;
        };
        println!("  default_tier={:?}  tiers={}", su.default_tier, su.renditions.len());
        println!(
            "  {:<8} {:<10} {:>10} {:>5} {:>9}  {:^5} {:^5} {:^5} {:^5}",
            "tier", "label", "res", "fps", "vbitrate", "cmaf", "hls", "flv", "dash"
        );
        for r in &su.renditions {
            let res = match (r.width, r.height) {
                (Some(w), Some(h)) => format!("{w}x{h}"),
                _ => "-".into(),
            };
            let mark = |o: &Option<String>| if o.is_some() { "yes" } else { " - " };
            println!(
                "  {:<8} {:<10} {:>10} {:>5} {:>9}  {:^5} {:^5} {:^5} {:^5}  codec={:?}",
                r.tier,
                r.label,
                res,
                r.fps.map(|f| f.to_string()).unwrap_or_else(|| "-".into()),
                r.vbitrate.map(|b| b.to_string()).unwrap_or_else(|| "-".into()),
                mark(&r.cmaf),
                mark(&r.hls),
                mark(&r.flv),
                mark(&r.dash),
                r.vcodec
            );
        }

        // Probe EVERY packaging of the best video tier, not just the one the
        // adapter would pick: the field names are TikTok's, and a name is not a
        // promise about the format behind it.
        let best = su.renditions.iter().find(|r| r.height.is_some());
        match best {
            None => println!("\n  no video tier at all\n"),
            Some(r) => {
                for (kind, url) in [
                    ("hls", r.hls.clone()),
                    ("cmaf", r.cmaf.clone()),
                    ("dash", r.dash.clone()),
                ] {
                    let Some(url) = url else { continue };
                    println!("\n  probing {kind}: {}", truncate(&url, 120));
                    probe_playlist(&url).await;
                }
                if r.hls.is_none() && r.cmaf.is_none() {
                    println!("\n  FLV ONLY: StreamNook plays this room through its FLV remux.");
                }
                println!();
            }
        }
    }
}

async fn probe_playlist(url: &str) {
    let client = reqwest::Client::builder()
        .timeout(T)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("client");

    for (label, referer) in [("with Referer", true), ("no Referer", false)] {
        let mut req = client.get(url);
        if referer {
            req = req.header("Referer", "https://www.tiktok.com/");
        }
        match req.send().await {
            Ok(res) => {
                let status = res.status();
                let acao = res
                    .headers()
                    .get("access-control-allow-origin")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("<none>")
                    .to_string();
                println!("    {label:<13} status={status} access-control-allow-origin={acao}");
                if referer && status.is_success() {
                    let body = res.text().await.unwrap_or_default();
                    describe_playlist(&body);
                    // Fetch the first segment DIRECTLY from the CDN, exactly as
                    // the CDN would be asked by any client. If this 404s here,
                    // the playlist is stale at source and no relay can help.
                    if let Some(seg) = body
                        .lines()
                        .map(str::trim)
                        .find(|l| !l.is_empty() && !l.starts_with('#'))
                    {
                        let (base, query) = split_url(url);
                        let seg_url = if seg.starts_with("http") {
                            seg.to_string()
                        } else {
                            format!("{base}/{seg}?{query}")
                        };
                        match client
                            .get(&seg_url)
                            .header("Referer", "https://www.tiktok.com/")
                            .send()
                            .await
                        {
                            Ok(s) => println!(
                                "      DIRECT segment fetch: status={} bytes={:?}",
                                s.status(),
                                s.content_length()
                            ),
                            Err(e) => println!("      DIRECT segment fetch: ERROR {e}"),
                        }
                    }
                }
            }
            Err(e) => println!("    {label:<13} ERROR {e}"),
        }
    }
}

/// The shape of the playlist is what decides the relay's rewrite rules.
fn describe_playlist(body: &str) {
    let mut absolute = 0;
    let mut relative = 0;
    let mut extinf = 0;
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('#') {
            if t.starts_with("#EXTINF") {
                extinf += 1;
            }
            continue;
        }
        if t.starts_with("http://") || t.starts_with("https://") {
            absolute += 1;
        } else {
            relative += 1;
        }
    }
    println!(
        "      segments: {extinf} EXTINF, {absolute} absolute URIs, {relative} relative URIs"
    );
    println!(
        "      EXT-X-MAP={} EXT-X-PART={} SERVER-CONTROL={} ENDLIST={} TARGETDURATION={:?}",
        body.contains("#EXT-X-MAP"),
        body.contains("#EXT-X-PART"),
        body.contains("#EXT-X-SERVER-CONTROL"),
        body.contains("#EXT-X-ENDLIST"),
        body.lines()
            .find(|l| l.starts_with("#EXT-X-TARGETDURATION"))
            .map(|l| l.trim().to_string())
    );
    // The first few lines say more than any summary.
    println!("      --- head ---");
    for line in body.lines().take(6) {
        println!("      {}", truncate(line, 150));
    }
    // The segment lines are the ones that decide whether a rewrite is correct.
    println!("      --- segment lines ---");
    for line in body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .take(4)
    {
        println!("      {}", truncate(line, 200));
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(n).collect::<String>())
    }
}

/// Split a url into its directory and its query, which is how a signed CDN
/// wants a sibling file addressed.
fn split_url(url: &str) -> (String, String) {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or(path);
    (dir.to_string(), query.to_string())
}
