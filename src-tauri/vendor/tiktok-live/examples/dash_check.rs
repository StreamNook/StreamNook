//! Is TikTok's DASH stream already in the container the player wants?
//!
//! The `cmaf` field turned out to be an `.mpd`, not an HLS playlist. What
//! matters next is what the SEGMENTS are: if they are already fMP4 with an
//! `init...mp4`, then serving this as HLS is a manifest translation and nothing
//! more, with no bitstream or container work anywhere.
//!
//! Fetches the manifest, resolves the first init segment and first media chunk
//! against it, and reports the top-level box of each so the container is read
//! from the bytes rather than from the file extension.
//!
//! Run: cargo run --example dash_check -- <handle>

use std::time::Duration;
use tiktok_live::http::api::{fetch_room_info, FetchParams};
use tiktok_live::http::sigi::scrape_profile;
use tiktok_live::http::ttwid::fetch_ttwid;

const T: Duration = Duration::from_secs(20);
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[tokio::main]
async fn main() {
    let handle = std::env::args()
        .nth(1)
        .expect("usage: dash_check <handle>")
        .trim()
        .trim_start_matches('@')
        .to_lowercase();

    let ttwid = fetch_ttwid(T, None, None).await.expect("ttwid");
    let profile = scrape_profile(&handle, &ttwid, T, None, None, None)
        .await
        .expect("profile");
    assert!(!profile.room_id.is_empty(), "@{handle} is not live");
    let info = fetch_room_info(&profile.room_id, FetchParams { timeout: T, ..Default::default() })
        .await
        .expect("room info");
    let su = info.stream_url.expect("stream_url");
    let r = su
        .renditions
        .iter()
        .find(|r| r.height.is_some() && r.cmaf.is_some())
        .expect("a video tier with a cmaf url");
    let mpd_url = r.cmaf.clone().unwrap();
    println!("tier={} {:?}x{:?} codec={:?}", r.tier, r.width, r.height, r.vcodec);
    println!("mpd: {mpd_url}\n");

    let client = reqwest::Client::builder()
        .timeout(T)
        .user_agent(UA)
        .build()
        .unwrap();
    let body = client
        .get(&mpd_url)
        .header("Referer", "https://www.tiktok.com/")
        .send()
        .await
        .expect("mpd")
        .text()
        .await
        .unwrap();
    println!("--- manifest ({} bytes) ---\n{}\n", body.len(), body);

    // Minimal, deliberately: enough to build the two URLs that answer the
    // container question, not a general MPD parser.
    let init = attr(&body, "initialization=\"");
    let media = attr(&body, "media=\"");
    let start_number = attr(&body, "startNumber=\"");
    println!("initialization={init:?}\nmedia={media:?}\nstartNumber={start_number:?}");

    let base = mpd_url.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
    let query = mpd_url.split_once('?').map(|(_, q)| q).unwrap_or("");

    if let Some(init) = init {
        // The manifest's URIs are relative, and the signature lives in the
        // MANIFEST's query, so it has to be carried onto every segment.
        let url = format!("{base}/{init}?{query}");
        probe_bytes(&client, "init", &url).await;
    }
    if let Some(media) = media {
        let n: u64 = start_number.and_then(|s| s.parse().ok()).unwrap_or(1);
        // How far BACK does the CDN actually retain? The manifest advertises a
        // 12 second shift buffer, which would be six 2 second segments, but an
        // advertised buffer is not a promise that the files are still there.
        // The playlist window has to match what exists or the player's first
        // request is a 404.
        for back in 0..8u64 {
            let num = n.saturating_sub(back);
            let name = media.replace("$Number$", &num.to_string());
            let url = format!("{base}/{name}?{query}");
            let label = format!("chunk newest-{back} (#{num})");
            probe_bytes(&client, &label, &url).await;
        }
    }
}

fn attr(body: &str, key: &str) -> Option<String> {
    let i = body.find(key)? + key.len();
    let rest = &body[i..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

async fn probe_bytes(client: &reqwest::Client, label: &str, url: &str) {
    println!("\n--- {label} ---\n{url}");
    match client
        .get(url)
        .header("Referer", "https://www.tiktok.com/")
        .send()
        .await
    {
        Err(e) => println!("ERROR {e}"),
        Ok(r) => {
            let status = r.status();
            let ct = r
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<none>")
                .to_string();
            let acao = r
                .headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<none>")
                .to_string();
            let bytes = r.bytes().await.unwrap_or_default();
            println!(
                "status={status} content-type={ct} acao={acao} bytes={}",
                bytes.len()
            );
            println!("container: {}", describe_container(&bytes));
        }
    }
}

/// Read the container from the bytes, not from the extension.
fn describe_container(b: &[u8]) -> String {
    if b.len() < 12 {
        return "too short".into();
    }
    let box_type = String::from_utf8_lossy(&b[4..8]).to_string();
    match &b[4..8] {
        b"ftyp" => {
            let brand = String::from_utf8_lossy(&b[8..12]).to_string();
            format!("fMP4 init: ftyp brand={brand} (ready for EXT-X-MAP as is)")
        }
        b"styp" => "fMP4 segment: styp (ready to serve as an HLS fMP4 part)".into(),
        b"moof" => "fMP4 segment: bare moof, no styp (still fMP4)".into(),
        _ if b[0] == 0x47 => "MPEG-TS (sync byte 0x47)".into(),
        _ if b.starts_with(b"FLV") => "FLV".into(),
        _ => format!("unknown, first box/tag = {box_type:?} bytes={:02x?}", &b[..8.min(b.len())]),
    }
}
