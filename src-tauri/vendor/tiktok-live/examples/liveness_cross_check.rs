//! Do the two liveness sources agree?
//!
//! The adapter decides "is this creator live" from ONE of them, so if they can
//! disagree, a live creator can look offline and nothing would say why.
//!
//!   A. `scrape_profile` reads `roomId` out of the profile page's rehydration
//!      blob. Chosen by the adapter because the same request also carries the
//!      identity that metadata needs.
//!   B. `fetch_room_id` calls `api-live/user/room`, a dedicated endpoint that
//!      returns both a room id AND a `liveRoom.status`.
//!   C. The `/@handle/live` page itself, as a third opinion.
//!
//! Run: cargo run --example liveness_cross_check -- <handle> [handle...]

use std::time::Duration;
use tiktok_live::http::api::{fetch_room_id, FetchParams};
use tiktok_live::http::sigi::scrape_profile;
use tiktok_live::http::ttwid::fetch_ttwid;

const T: Duration = Duration::from_secs(15);
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[tokio::main]
async fn main() {
    let handles: Vec<String> = std::env::args().skip(1).collect();
    if handles.is_empty() {
        eprintln!("usage: liveness_cross_check <handle> [handle...]");
        std::process::exit(2);
    }
    let ttwid = fetch_ttwid(T, None, None).await.expect("ttwid");
    let client = reqwest::Client::builder()
        .timeout(T)
        .user_agent(UA)
        .build()
        .unwrap();

    for raw in &handles {
        let h = raw.trim().trim_start_matches('@').to_lowercase();
        println!("=== @{h} ===");

        // A: what the adapter uses.
        let a = match scrape_profile(&h, &ttwid, T, None, None, None).await {
            Ok(p) => {
                let live = !p.room_id.is_empty();
                println!("  A scrape_profile   room_id={:?} -> live={}", p.room_id, live);
                Some(live)
            }
            Err(e) => {
                println!("  A scrape_profile   ERROR {e}");
                None
            }
        };

        // B: the dedicated endpoint, which also reports a status code.
        let b = match fetch_room_id(&h, FetchParams { timeout: T, ..Default::default() }).await {
            Ok(r) => {
                println!("  B fetch_room_id    room_id={:?} -> live=true", r.room_id);
                Some(true)
            }
            Err(e) => {
                // This call reports "not live" AS AN ERROR, so the wording is
                // what distinguishes offline from broken.
                println!("  B fetch_room_id    ERROR {e}");
                let s = e.to_string().to_lowercase();
                if s.contains("not online") || s.contains("no active room") || s.contains("room id")
                {
                    Some(false)
                } else {
                    None
                }
            }
        };

        // C: the live page, read the way a person would see it.
        let c = match client
            .get(format!("https://www.tiktok.com/@{h}/live"))
            .header("Cookie", format!("ttwid={ttwid}"))
            .send()
            .await
        {
            Ok(r) => {
                let body = r.text().await.unwrap_or_default();
                let room_ids = count_room_ids(&body);
                let offline_marker = body.contains("LIVE has ended")
                    || body.contains("user_not_live")
                    || body.contains("liveRoomStatus\":4");
                println!(
                    "  C /live page       len={} roomId-mentions={} offline-marker={}",
                    body.len(),
                    room_ids,
                    offline_marker
                );
                Some(room_ids > 0 && !offline_marker)
            }
            Err(e) => {
                println!("  C /live page       ERROR {e}");
                None
            }
        };

        match (a, b) {
            (Some(x), Some(y)) if x == y => println!("  VERDICT: A and B AGREE -> live={x}"),
            (Some(x), Some(y)) => println!(
                "  VERDICT: *** A and B DISAGREE *** scrape={x} endpoint={y} (C={c:?})"
            ),
            _ => println!("  VERDICT: inconclusive (A={a:?} B={b:?} C={c:?})"),
        }
        println!();
    }
}

fn count_room_ids(body: &str) -> usize {
    ["\"roomId\":\"", "\"room_id\":\""]
        .iter()
        .map(|m| {
            let mut n = 0;
            let mut rest = body;
            while let Some(i) = rest.find(m) {
                rest = &rest[i + m.len()..];
                // Only count one that actually holds a value.
                if let Some(end) = rest.find('"') {
                    if end > 8 {
                        n += 1;
                    }
                }
            }
            n
        })
        .sum()
}
