//! Generic HLS master-playlist parsing for provider streams (Kick/IVS, YouTube).
//!
//! This mirrors the ANCHORING approach of `twitch_resolver::parse_master`
//! (`#EXT-X-STREAM-INF` + its following URI line) but is intentionally
//! self-contained: it carries its own tiny attribute reader and NONE of the
//! Twitch codec-preference / entitlement statics. IVS masters served to Kick
//! carry the same `IVS-NAME` / `IVS-VARIANT-SOURCE` attributes Twitch's do;
//! YouTube's live master carries plain `RESOLUTION` / `FRAME-RATE` and no names,
//! so we derive `"1080p60"`-style labels from resolution.

use crate::services::providers::source::PlaybackQuality;

/// Parse a master playlist into renditions, best-first (source rendition, then
/// descending by height/fps/bandwidth). Absolute segment-less: `url` is joined
/// against `base` when the STREAM-INF URI is relative.
pub fn parse(master: &str, base: &str) -> Vec<PlaybackQuality> {
    let lines: Vec<&str> = master.lines().collect();

    // Legacy `#EXT-X-MEDIA:TYPE=VIDEO` names keyed by GROUP-ID (empty on the
    // modern MEDIA-less masters IVS/YouTube ship).
    let mut media_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for line in &lines {
        let l = line.trim();
        if l.starts_with("#EXT-X-MEDIA:") && l.contains("TYPE=VIDEO") {
            if let Some(gid) = attr(l, "GROUP-ID") {
                media_names.insert(gid, attr(l, "NAME").unwrap_or_default());
            }
        }
    }

    let mut out: Vec<(PlaybackQuality, bool)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let inf = lines[i].trim();
        if !inf.starts_with("#EXT-X-STREAM-INF:") {
            i += 1;
            continue;
        }
        // URL is the next non-comment, non-empty line.
        let mut j = i + 1;
        while j < lines.len() {
            let u = lines[j].trim();
            if u.is_empty() || u.starts_with('#') {
                j += 1;
                continue;
            }
            break;
        }
        if j >= lines.len() {
            break;
        }

        let (width, height) = parse_resolution(inf);
        let fps = attr(inf, "FRAME-RATE").and_then(|s| s.parse().ok());
        let bandwidth = attr(inf, "BANDWIDTH").and_then(|s| s.parse().ok());
        let codecs = attr(inf, "CODECS");
        let video_group = attr(inf, "VIDEO");

        let is_source = video_group.as_deref() == Some("chunked")
            || attr(inf, "IVS-VARIANT-SOURCE").is_some_and(|s| s.eq_ignore_ascii_case("source"));

        let name = video_group
            .as_ref()
            .and_then(|g| media_names.get(g).cloned())
            .filter(|n| !n.is_empty())
            .or_else(|| attr(inf, "IVS-NAME"))
            .or_else(|| attr(inf, "STABLE-VARIANT-ID"))
            .or_else(|| derive_name(height, fps, codecs.as_deref()))
            .or_else(|| video_group.clone())
            .unwrap_or_else(|| format!("variant{}", out.len()));

        out.push((
            PlaybackQuality {
                name,
                url: join_url(base, lines[j].trim()),
                width,
                height,
                fps,
                bandwidth,
            },
            is_source,
        ));
        i = j + 1;
    }

    // Order: source rendition first, then by height/fps/bandwidth descending.
    out.sort_by(|(a, a_src), (b, b_src)| {
        b_src
            .cmp(a_src)
            .then(b.height.cmp(&a.height))
            .then(
                b.fps
                    .partial_cmp(&a.fps)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(b.bandwidth.cmp(&a.bandwidth))
    });
    out.into_iter().map(|(q, _)| q).collect()
}

/// Choose a rendition for a requested quality. `best`/`source` → first (the
/// sort already put the source rendition there); `worst` → last video rendition;
/// `audio_only` → a resolution-less rendition; a numeric request → the closest
/// height at or below the request, else the nearest. Returns `(index, label)`.
pub fn select(qualities: &[PlaybackQuality], requested: &str) -> Option<(usize, String)> {
    if qualities.is_empty() {
        return None;
    }
    let req = requested.trim().to_ascii_lowercase();

    if req.is_empty() || req == "best" || req == "source" {
        return Some((0, qualities[0].name.clone()));
    }
    if req == "worst" {
        let idx = qualities
            .iter()
            .enumerate()
            .filter(|(_, q)| q.height.is_some())
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(qualities.len() - 1);
        return Some((idx, qualities[idx].name.clone()));
    }
    if req == "audio_only" || req == "audio-only" || req == "audio" {
        if let Some((i, q)) = qualities.iter().enumerate().find(|(_, q)| q.height.is_none()) {
            return Some((i, q.name.clone()));
        }
        return Some((0, qualities[0].name.clone()));
    }

    // Exact name match first (covers TikTok-style named tiers if ever routed here).
    if let Some(i) = qualities.iter().position(|q| q.name.eq_ignore_ascii_case(&req)) {
        return Some((i, qualities[i].name.clone()));
    }

    // Numeric: match the requested height (e.g. "720", "720p", "720p60").
    if let Some(target) = req
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .split('p')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
    {
        // Highest rendition at or below the target; else the lowest above it.
        let at_or_below = qualities
            .iter()
            .enumerate()
            .filter(|(_, q)| q.height.map(|h| h <= target).unwrap_or(false))
            .max_by_key(|(_, q)| q.height.unwrap_or(0));
        if let Some((i, q)) = at_or_below {
            return Some((i, q.name.clone()));
        }
        let above = qualities
            .iter()
            .enumerate()
            .filter(|(_, q)| q.height.is_some())
            .min_by_key(|(_, q)| q.height.unwrap_or(u32::MAX));
        if let Some((i, q)) = above {
            return Some((i, q.name.clone()));
        }
    }

    Some((0, qualities[0].name.clone()))
}

/// The quality menu (names best-first) plus `best`/`worst` aliases.
pub fn quality_names(qualities: &[PlaybackQuality]) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(qualities.len() + 2);
    names.push("best".to_string());
    names.extend(qualities.iter().map(|q| q.name.clone()));
    if qualities.iter().any(|q| q.height.is_some()) {
        names.push("worst".to_string());
    }
    names
}

fn derive_name(height: Option<u32>, fps: Option<f64>, codecs: Option<&str>) -> Option<String> {
    match height {
        Some(h) => {
            let f = fps.unwrap_or(30.0).round() as u32;
            if f >= 50 {
                Some(format!("{}p{}", h, f))
            } else {
                Some(format!("{}p", h))
            }
        }
        None => {
            let audio = codecs.is_some_and(|c| {
                c.contains("mp4a") && !c.contains("avc") && !c.contains("av01") && !c.contains("hvc") && !c.contains("hev")
            });
            audio.then(|| "audio_only".to_string())
        }
    }
}

fn parse_resolution(inf_line: &str) -> (Option<u32>, Option<u32>) {
    match attr(inf_line, "RESOLUTION") {
        Some(res) => {
            let mut parts = res.split(['x', 'X']);
            let w = parts.next().and_then(|p| p.trim().parse().ok());
            let h = parts.next().and_then(|p| p.trim().parse().ok());
            (w, h)
        }
        None => (None, None),
    }
}

/// Read `KEY=VALUE` from a `#EXT-X-STREAM-INF:` attribute list, handling both
/// quoted (`NAME="1080p60"`) and bare (`BANDWIDTH=6000000`) values.
fn attr(line: &str, key: &str) -> Option<String> {
    let hay = line;
    let mut from = 0;
    while let Some(pos) = hay[from..].find(key) {
        let start = from + pos;
        let after = start + key.len();
        // Ensure it's the whole attribute name: preceded by ',' / ':' and
        // followed by '='.
        let prev_ok = start == 0
            || matches!(hay.as_bytes()[start - 1], b',' | b':');
        if prev_ok && hay.as_bytes().get(after) == Some(&b'=') {
            let val = &hay[after + 1..];
            if let Some(rest) = val.strip_prefix('"') {
                return rest.split('"').next().map(|s| s.to_string());
            }
            return Some(val.split(',').next().unwrap_or("").trim().to_string());
        }
        from = after;
    }
    None
}

/// Join a possibly-relative STREAM-INF URI against the master's base URL. The
/// base is the master URL up to and including its last '/'.
fn join_url(base: &str, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    let base_no_query = base.split('?').next().unwrap_or(base);
    match base_no_query.rfind('/') {
        Some(slash) => format!("{}{}", &base_no_query[..=slash], uri),
        None => uri.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real Kick master (captured 2026-08-20, korekore_ch), trimmed to three
    // renditions. Kick's IVS stack serves the LEGACY layout: `#EXT-X-MEDIA`
    // carries the label, the STREAM-INF links it by `VIDEO="<group>"`, and there
    // is NO `IVS-NAME` and NO `IVS-VARIANT-SOURCE` — so best-first ordering has
    // to come from resolution, not a source flag. Variant URLs are ABSOLUTE and
    // on a different host than the master.
    const KICK_MASTER: &str = "\
#EXTM3U
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"720p60\",NAME=\"720p60\",AUTOSELECT=YES,DEFAULT=YES
#EXT-X-STREAM-INF:BANDWIDTH=3422999,RESOLUTION=1280x720,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"720p60\",FRAME-RATE=60.000
https://fa723fc1b171.usw24.playlist.live-video.net/v1/playlist/aaa/index.m3u8
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"480p30\",NAME=\"480p\",AUTOSELECT=YES,DEFAULT=YES
#EXT-X-STREAM-INF:BANDWIDTH=1427999,RESOLUTION=852x480,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"480p30\",FRAME-RATE=30.000
https://fa723fc1b171.usw24.playlist.live-video.net/v1/playlist/bbb/index.m3u8
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"360p30\",NAME=\"360p\",AUTOSELECT=YES,DEFAULT=YES
#EXT-X-STREAM-INF:BANDWIDTH=630000,RESOLUTION=640x360,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"360p30\",FRAME-RATE=30.000
relative/index.m3u8
";

    #[test]
    fn parses_real_kick_master_best_first() {
        let q = parse(KICK_MASTER, "https://host.example/api/video/v1/x.m3u8?token=abc");
        assert_eq!(q.len(), 3);
        // Labels come from the linked #EXT-X-MEDIA NAME, not derived.
        assert_eq!(q[0].name, "720p60");
        assert_eq!(q[1].name, "480p");
        assert_eq!(q[2].name, "360p");
        // No source flag anywhere -> ordering falls back to height descending.
        assert_eq!(q[0].height, Some(720));
        // Absolute variant URLs on another host survive untouched.
        assert!(q[0].url.starts_with("https://fa723fc1b171.usw24.playlist"));
        // A relative URI joins against the master's base, query stripped.
        assert_eq!(q[2].url, "https://host.example/api/video/v1/relative/index.m3u8");
    }

    #[test]
    fn selects_best_worst_and_numeric() {
        let q = parse(KICK_MASTER, "https://host.example/master.m3u8");
        assert_eq!(select(&q, "best").unwrap().1, "720p60");
        assert_eq!(select(&q, "worst").unwrap().1, "360p");
        assert_eq!(select(&q, "480p").unwrap().1, "480p");
        // 1080 is above everything on offer -> the highest available wins.
        assert_eq!(select(&q, "1080p60").unwrap().1, "720p60");
        // 400 sits between tiers -> the highest at-or-below (360p).
        assert_eq!(select(&q, "400").unwrap().1, "360p");
        // Below every tier -> nearest above.
        assert_eq!(select(&q, "144").unwrap().1, "360p");
    }

    // YouTube-style master: no names, RESOLUTION only -> derived labels.
    const YT_MASTER: &str = "\
#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=4000000,RESOLUTION=1920x1080,FRAME-RATE=30
https://manifest.googlevideo.com/1080/file.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1200000,RESOLUTION=640x360,FRAME-RATE=30
https://manifest.googlevideo.com/360/file.m3u8
";

    #[test]
    fn derives_youtube_names() {
        let q = parse(YT_MASTER, "https://manifest.googlevideo.com/master.m3u8");
        assert_eq!(q[0].name, "1080p");
        assert_eq!(q[1].name, "360p");
    }
}
