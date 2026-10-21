//! The pixel size of an image link, read from its first bytes, so every app
//! draws the card at its final size at once (the iPhone lays its rows out
//! before drawing them). A missing size is fine: cards fall back to their
//! old sizing.

use std::time::Duration;

/// Enough to find any common image's size marker, even past a big EXIF block.
const PROBE_CAP: usize = 512 * 1024;

pub async fn size(url: &str) -> Option<(u32, u32)> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    // The link-preview client: some image CDNs only answer browsers.
    let mut resp = crate::commands::link_preview::CLIENT
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes=0-{}", PROBE_CAP - 1))
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let mut head = Vec::new();
    while head.len() < PROBE_CAP {
        let chunk = resp.chunk().await.ok()??;
        head.extend_from_slice(&chunk);
        if let Ok(s) = imagesize::blob_size(&head) {
            return (s.width > 0 && s.height > 0).then_some((s.width as u32, s.height as u32));
        }
    }
    None
}
