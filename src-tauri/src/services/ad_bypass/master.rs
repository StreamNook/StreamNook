//! Ad-free master acquisition: race the relay pool, validate, splice.
//!
//! Ported from StreamNook 7.8.6 (`services/auth_proxy.rs` at `2bf9720`), where
//! this ran inside the core relay before resolution became a plugin hook.
//! Android can't host that plugin (it is a spawned native child process, which
//! the platform sandbox forbids), so the same logic lives here behind an
//! Android-only module instead.

use anyhow::{anyhow, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, info};
use std::collections::HashSet;
use std::time::Duration;

use super::proxies;
use crate::services::auth_proxy::{extract_attr, USER_AGENT};

/// Per-request budget for a relay. Short on purpose: the pool is raced, so a
/// slow relay simply loses to a fast one.
const RACE_TIMEOUT: Duration = Duration::from_secs(8);

/// True if `body` is actually an HLS master playlist (carries at least one
/// `#EXT-X-STREAM-INF`). The relays are flaky and routinely answer with HTTP
/// 200 plus an HTML "Server error!" page or a JSON `{"error":...}` body;
/// without this check those sail past the status test into the parser, which
/// then finds no variants and fails the whole stream. Treating a non-master 2xx
/// as a miss lets the race try another relay and ultimately fall back to the
/// direct master.
pub fn looks_like_master(body: &str) -> bool {
    body.contains("#EXT-X-STREAM-INF")
}

/// Percent-encodes everything except RFC 3986 unreserved chars + `:` and `/`.
///
/// The relays require URLs in this mangled shape: the query separators `?`,
/// `=`, `&` and `,` become `%3F`, `%3D`, `%26`, `%2C`, so the whole query rides
/// inside the URL path. A clean `?param=value` URL gets a 500 instead.
/// Re-confirmed live against the pool on 2026-08-06.
fn quote_safe_colon_slash(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for ch in s.chars() {
        let safe = matches!(ch,
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '.' | '_' | '~' | ':' | '/'
        );
        if safe {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

/// The playlist URL for one relay base.
fn playlist_url(base: &str, channel: &str) -> String {
    let raw = format!(
        "{}/playlist/{}.m3u8?platform=web&allow_source=true&allow_audio_only=true&fast_bread=true&supported_codecs=av1,h264,h265",
        base.trim_end_matches('/'),
        channel
    );
    quote_safe_colon_slash(&raw)
}

/// GET an anonymous, region-shifted master playlist. Races every base in
/// parallel and returns the first 2xx whose body is a real master playlist, as
/// `(winning_base, master_body)`.
async fn fetch_racing(channel: &str, bases: &[String]) -> Result<(String, String)> {
    if bases.is_empty() {
        return Err(anyhow!("no relay bases configured"));
    }
    let client = crate::services::http::client();

    let mut futs = FuturesUnordered::new();
    for base in bases {
        let url = playlist_url(base, channel);
        let label = base.clone();
        futs.push(async move {
            let resp = client
                .get(&url)
                .timeout(RACE_TIMEOUT)
                .header("User-Agent", USER_AGENT)
                .header("Referer", "https://player.twitch.tv")
                .header("Origin", "https://player.twitch.tv")
                .send()
                .await
                .map_err(|e| anyhow!("{} → {}", label, e))?;
            if !resp.status().is_success() {
                return Err(anyhow!("{} → HTTP {}", label, resp.status()));
            }
            let body = resp
                .text()
                .await
                .map_err(|e| anyhow!("{} → body: {}", label, e))?;
            if !looks_like_master(&body) {
                let first = body.lines().next().unwrap_or("").trim().to_string();
                return Err(anyhow!(
                    "{} → 2xx but not a master playlist ({} bytes, first line: {:?})",
                    label,
                    body.len(),
                    first
                ));
            }
            Ok::<(String, String), anyhow::Error>((label, body))
        });
    }

    let mut last_err: Option<anyhow::Error> = None;
    while let Some(res) = futs.next().await {
        match res {
            Ok((label, body)) => {
                debug!("[AdBypass] relay winner: {}", label);
                return Ok((label, body));
            }
            Err(e) => {
                debug!("[AdBypass] relay miss: {}", e);
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("every relay failed")))
}

/// Race `preferred` first; if they ALL fail, race the rest of the bundled pool.
/// One bundled relay going down must not take ad-free playback down with it.
///
/// `avoid` is the pivot's list of bases already known to be serving this stream
/// ads: they are excluded from the fallback too, otherwise an escalation lands
/// straight back on the relay it is escaping and the viewer gets a forced player
/// reload every cooldown for the length of the ad break.
///
/// `allow_bundled_fallback` is false when `preferred` is the user's own relay
/// list. Someone who typed in a specific relay chose it, and quietly sending
/// their channel name and playback traffic to twelve third-party community
/// relays because theirs hiccuped is not a fallback, it is a surprise. They get
/// the normal no-relay outcome instead: the direct stream, ads and all.
pub async fn fetch_with_fallback(
    channel: &str,
    preferred: &[String],
    avoid: &[String],
    allow_bundled_fallback: bool,
) -> Result<(String, String)> {
    let err = match fetch_racing(channel, preferred).await {
        Ok(win) => return Ok(win),
        Err(e) => e,
    };
    if !allow_bundled_fallback {
        return Err(err);
    }
    let mut skip: HashSet<&str> = preferred.iter().map(|s| s.trim_end_matches('/')).collect();
    skip.extend(avoid.iter().map(|s| s.trim_end_matches('/')));
    let rest: Vec<String> = proxies::bundled()
        .into_iter()
        .filter(|u| !skip.contains(u.as_str()))
        .collect();
    if rest.is_empty() {
        return Err(err);
    }
    debug!(
        "[AdBypass] {} preferred relays failed ({}); falling back to {} bundled relays",
        channel,
        err,
        rest.len()
    );
    fetch_racing(channel, &rest).await
}

/// Parse the height (e.g. `1440`) from a `NAME="1440p60"` attribute.
fn parse_name_height(line: &str) -> Option<u32> {
    let pos = line.find("NAME=\"")? + 6;
    let rest = &line[pos..];
    let end = rest.find('"')?;
    let digits: String = rest[..end].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// The video height of an `#EXT-X-STREAM-INF` line, from `RESOLUTION=WxH` or
/// the `NAME`/`IVS-NAME` label. None for audio-only.
fn stream_inf_height(inf: &str) -> Option<u32> {
    let res = extract_attr(inf, "RESOLUTION").and_then(|r| {
        r.split(['x', 'X'])
            .nth(1)
            .and_then(|h| h.trim().parse::<u32>().ok())
    });
    match (res, parse_name_height(inf)) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// What Twitch says it is withholding from this viewer and why.
///
/// The usher request asks for `include_unavailable=true`, so a tier the viewer
/// cannot have is still described, in a base64 session-data blob, with an
/// `AUTHORIZATION_REASONS` list. `AUTHZ_GEO` means the region relay can lift it;
/// anything else means it cannot. Without this, a missing 1440p tier looks
/// identical whether Twitch never offered it or offered it and the unlock
/// failed, which is the difference between an account question and a bug.
pub fn withheld_reasons(master: &str) -> Vec<String> {
    use base64::prelude::{Engine as _, BASE64_STANDARD};
    let mut out = Vec::new();
    for line in master.lines() {
        if !line.contains("com.amazon.ivs.unavailable-media") {
            continue;
        }
        let Some(b64) = extract_attr(line, "VALUE") else {
            continue;
        };
        let Ok(bytes) = BASE64_STANDARD.decode(b64.as_bytes()) else {
            continue;
        };
        let Ok(entries) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) else {
            continue;
        };
        for e in entries {
            let name = e
                .get("NAME")
                .or_else(|| e.get("GROUP_ID"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let reasons: Vec<&str> = e
                .get("AUTHORIZATION_REASONS")
                .and_then(|r| r.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            out.push(format!("{}:{}", name, reasons.join("+")));
        }
    }
    out
}

/// Every video height a master offers, highest first. For logging.
pub fn sorted_heights(master: &str) -> Vec<u32> {
    let mut v: Vec<u32> = heights(master).into_iter().collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v
}

/// Every video height a master offers.
fn heights(master: &str) -> HashSet<u32> {
    master
        .lines()
        .filter(|l| l.trim_start().starts_with("#EXT-X-STREAM-INF:"))
        .filter_map(stream_inf_height)
        .collect()
}

/// The playlist blocks in `master` for heights `have` does not already cover.
///
/// Anchored on `#EXT-X-STREAM-INF` (same as the resolver's parser) so it works
/// on both master layouts, and each block carries the preceding legacy
/// `#EXT-X-MEDIA` tag when there is one so the quality label survives. The URLs
/// are copied verbatim, which is what carries the geo-unlocked tiers through:
/// by the time the master reaches us those already point at the region relay's
/// media proxy, and rewriting them would undo the unlock.
fn missing_tier_blocks(master: &str, have: &HashSet<u32>) -> Vec<String> {
    let lines: Vec<&str> = master.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let inf = lines[i];
        if !inf.trim_start().starts_with("#EXT-X-STREAM-INF:") {
            i += 1;
            continue;
        }
        // URL = next non-comment, non-empty line.
        let mut j = i + 1;
        while j < lines.len() && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with('#'))
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
                block.push_str(lines[j].trim());
                // Give the grafted rendition its own group so its label cannot
                // overwrite, or be overwritten by, a relay rendition that shares
                // the name Twitch gives every source tier.
                out.push(retag_group(&block, &format!("sn-{h}")));
            }
        }
        i = j + 1;
    }
    out
}

/// Keep the relay master as the base, then append every tier the viewer's own
/// master has that the relay's lacks.
///
/// 7.8.6 grafted only above 1080p, on the assumption that a relay always serves
/// the full ladder up to FULL_HD. Measured against the live pool on 2026-08-06
/// that does not hold: for the same channel at the same moment, some relays
/// answered with a ladder topping out at 720p while others reached 1080p, and
/// the race is won on speed. Keying on "which heights are missing" instead of a
/// fixed 1080 line keeps the viewer's full quality menu no matter which relay
/// wins, and is also what carries the geo-unlocked 1440p tier through, since by
/// then it is simply another tier the relay master does not have.
///
/// A grafted tier is the viewer's own stream, so it can carry ads that the
/// relay's tiers would not. The playlist filter strips those as they are
/// served, which is the same defense that covers a relay leaking an ad.
/// Rewrite a grafted block's rendition group so it cannot collide with the
/// relay master's.
///
/// Twitch names the source rendition `chunked` whatever its height is, so a
/// relay capped at 720p and the viewer's 1440p master both declare
/// `GROUP-ID="chunked"`. The resolver builds its label map as
/// `group id -> name`, last write wins, so appending the grafted block verbatim
/// relabels the relay's 720p tier as "1440p60" and the viewer silently gets
/// 720p from a menu entry that says 1440p.
fn retag_group(block: &str, group: &str) -> String {
    let mut out = String::with_capacity(block.len() + 16);
    for (i, line) in block.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&replace_quoted_attr(
            &replace_quoted_attr(line, "GROUP-ID", group),
            "VIDEO",
            group,
        ));
    }
    out
}

/// Replace `KEY="..."` in a tag line, leaving the line alone when it has no
/// such attribute.
fn replace_quoted_attr(line: &str, key: &str, value: &str) -> String {
    let needle = format!("{}=\"", key);
    let Some(pos) = line.find(&needle) else {
        return line.to_string();
    };
    let rest = &line[pos + needle.len()..];
    let Some(end) = rest.find('"') else {
        return line.to_string();
    };
    format!("{}{}=\"{}\"{}", &line[..pos], key, value, &rest[end + 1..])
}

/// Strip the source marker off a master's own renditions.
///
/// `source_index` in the resolver returns the FIRST rendition tagged
/// `GROUP-ID="chunked"` (or `IVS-VARIANT-SOURCE="source"`), and the relay master
/// is the base of the spliced output, so its tiers always come first. Left
/// alone, "best" therefore resolves to the relay's own top tier and every
/// grafted tier above it is dead weight. With no rendition claiming to be the
/// source, the resolver falls through to picking the highest one, which is what
/// the viewer asked for.
fn demote_source(master: &str) -> String {
    master
        .lines()
        .map(|line| {
            if line.contains("GROUP-ID=\"chunked\"") || line.contains("VIDEO=\"chunked\"") {
                retag_group(line, "sn-relay")
            } else if line.contains("IVS-VARIANT-SOURCE=\"source\"") {
                replace_quoted_attr(line, "IVS-VARIANT-SOURCE", "alt")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn splice(relay_master: &str, auth_master: &str) -> String {
    let have = heights(relay_master);
    let blocks = missing_tier_blocks(auth_master, &have);
    if blocks.is_empty() {
        return relay_master.to_string();
    }
    let relay_top = have.iter().copied().max().unwrap_or(0);
    let graft_top = blocks
        .iter()
        .filter_map(|b| b.lines().find(|l| l.starts_with("#EXT-X-STREAM-INF:")))
        .filter_map(stream_inf_height)
        .max()
        .unwrap_or(0);

    // Only take the source marker away from the relay when something grafted
    // actually outranks it; otherwise the relay's own top tier is still the
    // right answer for "best" and must keep serving it ad-free.
    let mut out = if graft_top > relay_top {
        demote_source(relay_master).trim_end().to_string()
    } else {
        relay_master.trim_end().to_string()
    };

    for block in blocks {
        // Log the real RESOLUTION/CODECS of each grafted variant: this is the
        // proof that a tier labeled "1440p60" really points at a 2560x1440
        // variant from the authenticated master, not a relabeled lower tier.
        if let Some(inf) = block.lines().find(|l| l.starts_with("#EXT-X-STREAM-INF:")) {
            info!(
                "[AdBypass] spliced variant VIDEO={} RESOLUTION={} CODECS={}",
                extract_attr(inf, "VIDEO").unwrap_or_else(|| "?".into()),
                extract_attr(inf, "RESOLUTION").unwrap_or_else(|| "?".into()),
                extract_attr(inf, "CODECS").unwrap_or_else(|| "?".into()),
            );
        }
        out.push('\n');
        out.push_str(&block);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH_MASTER: &str = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1440p60\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=9000000,RESOLUTION=2560x1440,CODECS=\"av01.0.08M.08\",VIDEO=\"chunked\"\n\
https://cdn/auth-1440.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"1080p60\",NAME=\"1080p60\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,CODECS=\"avc1.64002A\",VIDEO=\"1080p60\"\n\
https://cdn/auth-1080.m3u8\n";

    const RELAY_MASTER: &str = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,CODECS=\"avc1.64002A\",VIDEO=\"1080p60\"\n\
https://cdn/relay-1080.m3u8\n";

    #[test]
    fn the_query_rides_inside_the_path() {
        let url = playlist_url("https://relay.example.com/", "shroud");
        assert!(url.starts_with("https://relay.example.com/playlist/shroud.m3u8%3F"));
        assert!(url.contains("platform%3Dweb"));
        assert!(url.contains("%26allow_source%3Dtrue"));
        assert!(!url.contains('?'), "a clean query gets 500'd by the relays");
    }

    #[test]
    fn splice_grafts_the_1440p_tier() {
        let out = splice(RELAY_MASTER, AUTH_MASTER);
        assert!(out.contains("https://cdn/relay-1080.m3u8"), "{out}");
        assert!(out.contains("https://cdn/auth-1440.m3u8"), "{out}");
        assert!(out.contains("RESOLUTION=2560x1440"), "{out}");
        assert!(out.contains("NAME=\"1440p60\""), "the label rides along: {out}");
        assert!(
            !out.contains("https://cdn/auth-1080.m3u8"),
            "a tier the relay already serves is never swapped for the ad-bearing one: {out}"
        );
    }

    #[test]
    fn splice_adds_nothing_when_the_relay_covers_every_tier() {
        let auth_1080_only = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080\nhttps://cdn/auth-1080.m3u8\n";
        assert_eq!(splice(RELAY_MASTER, auth_1080_only), RELAY_MASTER);
    }

    #[test]
    fn splice_restores_a_tier_the_winning_relay_capped_away() {
        // Measured live: relays for one channel disagree about the top of the
        // ladder, and the race is won on speed. A 720p relay must not cost the
        // viewer 1080p.
        let capped_relay = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720\nhttps://cdn/relay-720.m3u8\n";
        let out = splice(capped_relay, AUTH_MASTER);
        assert!(out.contains("https://cdn/relay-720.m3u8"), "{out}");
        assert!(out.contains("https://cdn/auth-1080.m3u8"), "{out}");
        assert!(out.contains("https://cdn/auth-1440.m3u8"), "{out}");
    }

    /// A relay in the legacy layout: it tags its own top tier as the source,
    /// exactly like the viewer's master does, which is what makes the two
    /// collide.
    const LEGACY_CAPPED_RELAY: &str = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"720p60\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720,CODECS=\"avc1.4D402A\",VIDEO=\"chunked\"\n\
https://cdn/relay-720.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"480p30\",NAME=\"480p\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=1400000,RESOLUTION=852x480,CODECS=\"avc1.4D401F\",VIDEO=\"480p30\"\n\
https://cdn/relay-480.m3u8\n";

    #[test]
    fn a_grafted_tier_never_relabels_the_relays_own() {
        // Twitch calls every source rendition "chunked" whatever its height, so
        // appending the viewer's block verbatim used to rename the relay's 720p
        // tier to "1440p60" through the resolver's group-to-name map.
        let spliced = splice(LEGACY_CAPPED_RELAY, AUTH_MASTER);
        let variants = crate::services::twitch_resolver::parse_master(&spliced);
        let by_url = |u: &str| {
            variants
                .iter()
                .find(|v| v.url == u)
                .unwrap_or_else(|| panic!("{u} missing from {spliced}"))
                .clone()
        };
        assert_eq!(by_url("https://cdn/relay-720.m3u8").name, "720p60");
        assert_eq!(by_url("https://cdn/auth-1440.m3u8").name, "1440p60");
        assert_eq!(by_url("https://cdn/relay-480.m3u8").name, "480p");
    }

    #[test]
    fn best_resolves_to_the_grafted_tier_when_it_outranks_the_relay() {
        // The end of the chain, which is the only thing that actually matters:
        // grafting the block is pointless if "best" still picks the relay's own
        // top tier, and it did, because the relay's renditions come first and
        // the resolver takes the FIRST one marked as the source.
        let spliced = splice(LEGACY_CAPPED_RELAY, AUTH_MASTER);
        let variants = crate::services::twitch_resolver::parse_master(&spliced);
        let (idx, _) = crate::services::twitch_resolver::select_variant(&variants, "best")
            .expect("best resolves");
        assert_eq!(
            variants[idx].url, "https://cdn/auth-1440.m3u8",
            "best picked {:?} out of {spliced}",
            variants[idx]
        );
        assert_eq!(variants[idx].height, Some(1440));
    }

    #[test]
    fn the_relay_keeps_the_source_marker_when_nothing_grafted_outranks_it() {
        // Only a mid tier is missing, so the relay's own top tier is still the
        // right answer for "best" and must keep serving it ad-free.
        let auth_mid_only = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=640x360\nhttps://cdn/auth-360.m3u8\n";
        let spliced = splice(LEGACY_CAPPED_RELAY, auth_mid_only);
        assert!(spliced.contains("GROUP-ID=\"chunked\""), "{spliced}");
        let variants = crate::services::twitch_resolver::parse_master(&spliced);
        let (idx, _) = crate::services::twitch_resolver::select_variant(&variants, "best")
            .expect("best resolves");
        assert_eq!(variants[idx].url, "https://cdn/relay-720.m3u8");
    }

    #[test]
    fn a_geo_unlocked_tier_keeps_its_relay_routed_url() {
        // The region-unlock splice upstream has already pointed this tier's
        // playlist at the quality relay. Rewriting it here would re-block it.
        let auth = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=9000000,RESOLUTION=2560x1440,VIDEO=\"chunked\"\n\
https://modroom.streamnook.app/quality-media?u=https%3A%2F%2Fcdn%2F1440.m3u8\n";
        let out = splice(RELAY_MASTER, auth);
        assert!(
            out.contains("https://modroom.streamnook.app/quality-media?u=https%3A%2F%2Fcdn%2F1440.m3u8"),
            "{out}"
        );
    }

    #[test]
    fn height_comes_from_the_name_when_resolution_is_absent() {
        assert_eq!(
            parse_name_height(r#"#EXT-X-STREAM-INF:IVS-NAME="1440p60""#),
            Some(1440)
        );
        assert_eq!(parse_name_height(r#"#EXT-X-MEDIA:NAME="audio_only""#), None);
    }

    #[test]
    fn only_real_masters_pass_validation() {
        assert!(looks_like_master("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nurl\n"));
        assert!(!looks_like_master("<html>Server error!</html>"));
        assert!(!looks_like_master("{\"error\":\"no channel\"}"));
    }
}
