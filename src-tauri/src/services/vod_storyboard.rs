//! Seek-preview sprite sheets ("storyboards") of a finished Twitch VOD.
//!
//! `video.seekPreviewsURL` points at `<dir>/storyboards/<id>-info.json`: a
//! JSON array of variants, each `{count, width, height, rows, cols, interval,
//! quality, images}` where `images` are sheet file names relative to the
//! manifest's directory and `interval` is seconds per frame. Probed
//! 2026-09-21 on xqc 2871438321: low 160x90 in one 40x5 sheet, high 220x124
//! in four 10x5 sheets, both 200 frames at 219 s.
//!
//! The URL is advertised for a RECORDING VOD too, but the sheets are rendered
//! after the broadcast ends and the manifest answers 403 until then, so the
//! caller only fetches for `recorded`.

use std::sync::LazyLock;
use std::time::Duration;

use log::debug;
use serde::{Deserialize, Serialize};

use super::auth_proxy;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoryboardVariant {
    #[serde(default)]
    pub quality: String,
    pub width: u32,
    pub height: u32,
    pub rows: u32,
    pub cols: u32,
    pub count: u32,
    #[serde(rename(deserialize = "interval"))]
    pub interval_secs: u32,
    pub images: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Storyboard {
    /// Directory the sheet names are relative to; ends in `/`.
    pub base_url: String,
    /// Smallest frame height first.
    pub variants: Vec<StoryboardVariant>,
}

/// Parse a manifest body. `None` for anything unusable.
pub fn parse(manifest_url: &str, text: &str) -> Option<Storyboard> {
    let mut variants: Vec<StoryboardVariant> = serde_json::from_str(text).ok()?;
    variants.retain(|v| {
        v.count > 0
            && v.rows > 0
            && v.cols > 0
            && v.width > 0
            && v.height > 0
            && v.interval_secs > 0
            && !v.images.is_empty()
    });
    if variants.is_empty() {
        return None;
    }
    variants.sort_by_key(|v| v.height);
    let base_url = manifest_url[..=manifest_url.rfind('/')?].to_string();
    Some(Storyboard { base_url, variants })
}

/// One client for the module: a per-call builder would rebuild the TLS config
/// on every VOD start. The timeout is short because this ride-along must
/// never hold up `start_vod`: a slow CDN costs at most 3 s, then the VOD
/// plays without previews.
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .user_agent(auth_proxy::USER_AGENT)
        .build()
        .expect("storyboard http client")
});

/// A real manifest is under a kilobyte; anything past this is not one.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// GET the manifest. Never fails the caller: any error is `None`.
pub async fn fetch(manifest_url: &str) -> Option<Storyboard> {
    let resp = CLIENT.get(manifest_url).send().await.ok()?;
    if !resp.status().is_success() {
        debug!("[Storyboard] {} -> {}", manifest_url, resp.status());
        return None;
    }
    if resp.content_length().is_some_and(|n| n > MAX_MANIFEST_BYTES) {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    parse(manifest_url, std::str::from_utf8(&bytes).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST_URL: &str = "https://d1m7jfoe9zdc1j.cloudfront.net/4d854b049e2c157a3aba_xqc_321317436761_1789151438/storyboards/2871438321-info.json";

    /// The real manifest for xqc VOD 2871438321, probed 2026-09-21. Note the
    /// high variant is listed second but has the larger frame.
    const XQC: &str = r#"[{"count":200,"width":160,"rows":40,"images":["2871438321-low-0.jpg"],"interval":219,"quality":"low","cols":5,"height":90},{"count":200,"width":220,"rows":10,"images":["2871438321-high-0.jpg","2871438321-high-1.jpg","2871438321-high-2.jpg","2871438321-high-3.jpg"],"interval":219,"quality":"high","cols":5,"height":124}]"#;

    #[test]
    fn parses_the_probed_manifest() {
        let sb = parse(MANIFEST_URL, XQC).expect("a real manifest parses");
        assert!(sb.base_url.ends_with("/storyboards/"), "{}", sb.base_url);
        assert_eq!(
            sb.base_url,
            "https://d1m7jfoe9zdc1j.cloudfront.net/4d854b049e2c157a3aba_xqc_321317436761_1789151438/storyboards/"
        );
        assert_eq!(sb.variants.len(), 2);
        let low = &sb.variants[0];
        let high = &sb.variants[1];
        assert_eq!(low.quality, "low");
        assert_eq!((low.width, low.height, low.rows, low.cols), (160, 90, 40, 5));
        assert_eq!(low.images, vec!["2871438321-low-0.jpg"]);
        assert_eq!(high.quality, "high");
        assert_eq!((high.width, high.height, high.rows, high.cols), (220, 124, 10, 5));
        assert_eq!(high.images.len(), 4);
        for v in &sb.variants {
            assert_eq!(v.count, 200);
            assert_eq!(v.interval_secs, 219);
        }
    }

    #[test]
    fn rejects_bad_json() {
        assert!(parse(MANIFEST_URL, "<html>403 Forbidden</html>").is_none());
        assert!(parse(MANIFEST_URL, "{\"count\":200}").is_none());
        assert!(parse(MANIFEST_URL, "").is_none());
    }

    #[test]
    fn rejects_empty_array() {
        assert!(parse(MANIFEST_URL, "[]").is_none());
    }

    #[test]
    fn drops_a_zero_count_variant() {
        let text = r#"[{"count":0,"width":160,"rows":40,"images":["a.jpg"],"interval":219,"quality":"low","cols":5,"height":90},{"count":200,"width":220,"rows":10,"images":["b.jpg"],"interval":219,"quality":"high","cols":5,"height":124}]"#;
        let sb = parse(MANIFEST_URL, text).expect("one usable variant remains");
        assert_eq!(sb.variants.len(), 1);
        assert_eq!(sb.variants[0].quality, "high");
        // Every variant unusable: no storyboard at all.
        let text = r#"[{"count":0,"width":160,"rows":40,"images":["a.jpg"],"interval":219,"quality":"low","cols":5,"height":90}]"#;
        assert!(parse(MANIFEST_URL, text).is_none());
    }
}
