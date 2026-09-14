//! SABR/UMP client for YouTube live.
//!
//! Why this exists: on many broadcasts YouTube serves the high renditions ONLY
//! through SABR. There is no per-itag URL to fetch (the ones in
//! `adaptiveFormats` are vestigial and 403), and the HLS ladder stops at 1080p
//! by construction. TheBurntPeanut is one of these, which is what forced the
//! issue. Reverse engineered from a live capture on 2026-08-21.
//!
//! Shape of the exchange:
//!
//! ```text
//! POST <serverAbrStreamingUrl>&rn=N&cpn=<16>&cver=<v>&alr=yes
//!   body:     VideoPlaybackAbrRequest (protobuf, ~1.8 KB)
//!   response: application/vnd.yt-ump  (UMP part stream, megabytes)
//! ```
//!
//! The response is worth understanding because it is what makes this tractable:
//! video arrives as `moof` + `mdat` already, so it needs no transmuxing at all.
//! `FORMAT_INITIALIZATION_METADATA` carries the init segment's shape, each
//! `MEDIA_HEADER` opens a stream identified by a header id, and the `MEDIA`
//! parts that follow carry that stream's bytes. Reassemble by header id and the
//! result is directly servable as fMP4.
//!
//! What this module does NOT do is mint the Proof-of-Origin token. That lives in
//! `StreamerContext.po_token`, is bound to the session, and requires running
//! YouTube's BotGuard interpreter. See `youtube_potoken`.

use anyhow::{anyhow, Result};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// UMP framing
// ---------------------------------------------------------------------------

/// UMP part types seen on the live path. Named rather than numbered because a
/// bare `35` in a match arm tells the next reader nothing.
pub mod part {
    pub const MEDIA_HEADER: u32 = 20;
    pub const MEDIA: u32 = 21;
    pub const MEDIA_END: u32 = 22;
    pub const LIVE_METADATA: u32 = 31;
    pub const NEXT_REQUEST_POLICY: u32 = 35;
    pub const FORMAT_INITIALIZATION_METADATA: u32 = 42;
    pub const SABR_REDIRECT: u32 = 43;
    pub const SABR_ERROR: u32 = 44;
    pub const REQUEST_IDENTIFIER: u32 = 52;
    pub const REQUEST_CANCELLATION_POLICY: u32 = 53;
    pub const REQUEST_PIPELINING: u32 = 58;
    pub const STREAM_PROTECTION_STATUS: u32 = 60;
}

/// Read a UMP varint.
///
/// NOT a protobuf varint. The leading one-bits of the first byte give the total
/// length; the remaining low bits of that byte are the least significant part of
/// the value, and each following byte contributes eight more bits upward.
fn ump_varint(b: &[u8], pos: usize) -> Option<(u64, usize)> {
    let first = *b.get(pos)?;
    let n = match first {
        0..=127 => 1usize,
        128..=191 => 2,
        192..=223 => 3,
        224..=239 => 4,
        _ => 5,
    };
    if pos + n > b.len() {
        return None;
    }
    // For widths 1-4 the first byte's remaining low bits are the least
    // significant part of the value. For width 5 they are DISCARDED: the four
    // trailing bytes carry the whole number.
    let (mut val, mut shift) = if n == 5 {
        (0u64, 0u32)
    } else {
        let mask = (1u16 << (8 - n)) - 1;
        ((first as u64) & (mask as u64), (8 - n) as u32)
    };
    for i in 1..n {
        val |= (b[pos + i] as u64) << shift;
        shift += 8;
    }
    Some((val, pos + n))
}

/// Walk a UMP stream, handing each part to `f` as (type, payload).
pub fn for_each_part<F: FnMut(u32, &[u8])>(b: &[u8], mut f: F) {
    let mut p = 0usize;
    while p < b.len() {
        let Some((kind, q)) = ump_varint(b, p) else { break };
        let Some((size, r)) = ump_varint(b, q) else { break };
        let size = size as usize;
        if r + size > b.len() {
            // A truncated tail is normal when a response is cut short; take what
            // is there rather than discarding the whole response.
            f(kind as u32, &b[r..]);
            break;
        }
        f(kind as u32, &b[r..r + size]);
        p = r + size;
    }
}

// ---------------------------------------------------------------------------
// Minimal protobuf
// ---------------------------------------------------------------------------

fn pb_read_varint(b: &[u8], pos: usize) -> Option<(u64, usize)> {
    let mut r = 0u64;
    let mut s = 0u32;
    let mut p = pos;
    loop {
        let x = *b.get(p)?;
        r |= ((x & 0x7F) as u64) << s;
        p += 1;
        if x & 0x80 == 0 {
            return Some((r, p));
        }
        s += 7;
        if s > 63 {
            return None;
        }
    }
}

/// One pass over a protobuf message, yielding (field number, value).
enum Val<'a> {
    Var(u64),
    Bytes(&'a [u8]),
}

fn pb_fields<'a, F: FnMut(u32, Val<'a>)>(b: &'a [u8], mut f: F) {
    let mut p = 0usize;
    while p < b.len() {
        let Some((key, q)) = pb_read_varint(b, p) else { return };
        let fnum = (key >> 3) as u32;
        match key & 7 {
            0 => {
                let Some((v, r)) = pb_read_varint(b, q) else { return };
                f(fnum, Val::Var(v));
                p = r;
            }
            2 => {
                let Some((len, r)) = pb_read_varint(b, q) else { return };
                let end = r + len as usize;
                if end > b.len() {
                    return;
                }
                f(fnum, Val::Bytes(&b[r..end]));
                p = end;
            }
            5 => p = q + 4,
            1 => p = q + 8,
            _ => return,
        }
    }
}

fn w_varint(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        out.push(if n != 0 { b | 0x80 } else { b });
        if n == 0 {
            return;
        }
    }
}

fn w_var_field(out: &mut Vec<u8>, fnum: u32, v: u64) {
    w_varint(out, ((fnum as u64) << 3) | 0);
    w_varint(out, v);
}

fn w_bytes_field(out: &mut Vec<u8>, fnum: u32, raw: &[u8]) {
    w_varint(out, ((fnum as u64) << 3) | 2);
    w_varint(out, raw.len() as u64);
    out.extend_from_slice(raw);
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// One stream opened by a `MEDIA_HEADER`.
#[derive(Debug, Clone, Default)]
pub struct MediaStream {
    pub header_id: u32,
    pub itag: u32,
    /// True once a `MEDIA_END` for this header id has been seen.
    pub ended: bool,
    pub bytes: Vec<u8>,
}

impl MediaStream {
    /// `ftyp`/`moov` init or a `moof` fragment: either way MP4, which the relay
    /// can serve directly. WebM (Opus audio) needs containerising first.
    pub fn is_mp4(&self) -> bool {
        self.bytes.len() > 8 && matches!(&self.bytes[4..8], b"ftyp" | b"moof" | b"styp" | b"sidx")
    }

    pub fn is_webm(&self) -> bool {
        self.bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) || self.bytes.starts_with(&[0x1F, 0x43, 0xB6, 0x75])
    }
}

#[derive(Debug, Clone, Default)]
pub struct SabrResponse {
    /// Reassembled streams in the order their headers appeared.
    pub streams: Vec<MediaStream>,
    /// `mimeType` strings from `FORMAT_INITIALIZATION_METADATA`, for logging.
    pub formats: Vec<String>,
    /// Format ids the server echoed back, complete with `lastModified`.
    ///
    /// This is the only place that value is available: the watch page's
    /// `adaptiveFormats` do NOT carry `lastModified` for a live stream, so the
    /// first request goes out without it and later ones use what came back here.
    pub format_ids: Vec<FormatId>,
    /// Set when the server reported a playback error rather than media.
    pub error: Option<String>,
    pub media_bytes: usize,
}

pub fn parse_response(b: &[u8]) -> SabrResponse {
    let mut out = SabrResponse::default();
    let mut index: HashMap<u32, usize> = HashMap::new();

    for_each_part(b, |kind, payload| match kind {
        part::MEDIA_HEADER => {
            let mut hid = 0u64;
            let mut itag = 0u64;
            pb_fields(payload, |f, v| match (f, v) {
                (1, Val::Var(x)) => hid = x,
                (3, Val::Var(x)) => itag = x,
                _ => {}
            });
            let hid = hid as u32;
            if !index.contains_key(&hid) {
                index.insert(hid, out.streams.len());
                out.streams.push(MediaStream {
                    header_id: hid,
                    itag: itag as u32,
                    ..Default::default()
                });
            } else if itag != 0 {
                if let Some(&i) = index.get(&hid) {
                    out.streams[i].itag = itag as u32;
                }
            }
        }
        part::MEDIA => {
            // A MEDIA part is [header id varint][raw bytes].
            let Some((hid, q)) = ump_varint(payload, 0) else { return };
            let hid = hid as u32;
            let i = *index.entry(hid).or_insert_with(|| {
                out.streams.push(MediaStream { header_id: hid, ..Default::default() });
                out.streams.len() - 1
            });
            out.streams[i].bytes.extend_from_slice(&payload[q..]);
            out.media_bytes += payload.len() - q;
        }
        part::MEDIA_END => {
            if let Some((hid, _)) = ump_varint(payload, 0) {
                if let Some(&i) = index.get(&(hid as u32)) {
                    out.streams[i].ended = true;
                }
            }
        }
        part::FORMAT_INITIALIZATION_METADATA => {
            pb_fields(payload, |f, v| match (f, v) {
                (2, Val::Bytes(raw)) => {
                    // A nested FormatId, and the only source of `lastModified`.
                    let mut id = FormatId::default();
                    pb_fields(raw, |g, w| match (g, w) {
                        (1, Val::Var(x)) => id.itag = x as u32,
                        (2, Val::Var(x)) => id.last_modified = x,
                        (3, Val::Bytes(x)) => {
                            id.xtags = std::str::from_utf8(x).ok().map(String::from)
                        }
                        _ => {}
                    });
                    if id.itag != 0 {
                        out.format_ids.push(id);
                    }
                }
                (5, Val::Bytes(s)) => {
                    if let Ok(s) = std::str::from_utf8(s) {
                        out.formats.push(s.to_string());
                    }
                }
                _ => {}
            });
        }
        part::SABR_ERROR => {
            out.error = Some(format!("SABR_ERROR ({} bytes)", payload.len()));
        }
        _ => {}
    });

    out
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// Identifies one rendition. `last_modified` matters: the server rejects a
/// format id that does not match what its player response advertised.
#[derive(Debug, Clone, Default)]
pub struct FormatId {
    pub itag: u32,
    pub last_modified: u64,
    pub xtags: Option<String>,
}

impl FormatId {
    fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        w_var_field(&mut o, 1, self.itag as u64);
        if self.last_modified != 0 {
            w_var_field(&mut o, 2, self.last_modified);
        }
        if let Some(x) = &self.xtags {
            w_bytes_field(&mut o, 3, x.as_bytes());
        }
        o
    }
}

/// Everything needed to ask SABR for one video plus one audio rendition.
#[derive(Debug, Clone)]
pub struct SabrRequest {
    /// `videoPlaybackUstreamerConfig` from the player response, already decoded
    /// from base64url.
    pub ustreamer_config: Vec<u8>,
    pub video: FormatId,
    pub audio: FormatId,
    pub width: u32,
    pub height: u32,
    /// InnerTube client version, e.g. "2.20260820.08.00".
    pub client_version: String,
    /// Proof-of-Origin token. Without it the server parses the request and
    /// answers 403, so this is the difference between a working client and a
    /// well-formed one.
    pub po_token: Option<Vec<u8>>,
}

impl SabrRequest {
    /// `ClientAbrState`: mostly a description of what the client will accept.
    /// The resolution fields are what stop the server from capping us at 1080p.
    fn client_abr_state(&self) -> Vec<u8> {
        let mut o = Vec::new();
        w_var_field(&mut o, 14, 0); // time since last seek
        w_var_field(&mut o, 16, self.height as u64); // sticky resolution
        w_var_field(&mut o, 18, self.width as u64);
        w_var_field(&mut o, 19, self.height as u64);
        w_var_field(&mut o, 21, self.height as u64); // max accepted resolution
        w_var_field(&mut o, 28, 0);
        w_var_field(&mut o, 29, 0);
        w_var_field(&mut o, 34, 3); // want video and audio
        w_var_field(&mut o, 46, 1);
        w_var_field(&mut o, 57, 194);
        w_var_field(&mut o, 58, 0);
        w_var_field(&mut o, 59, 2160); // ceiling
        w_var_field(&mut o, 71, 1);
        w_var_field(&mut o, 80, 1);
        w_var_field(&mut o, 85, 1);
        o
    }

    fn streamer_context(&self) -> Vec<u8> {
        let mut client = Vec::new();
        w_bytes_field(&mut client, 1, b"en_US");
        w_var_field(&mut client, 16, 1);
        w_bytes_field(&mut client, 17, self.client_version.as_bytes());
        w_bytes_field(&mut client, 18, b"Windows");
        w_bytes_field(&mut client, 19, b"10.0");

        let mut o = Vec::new();
        w_bytes_field(&mut o, 1, &client);
        if let Some(t) = &self.po_token {
            w_bytes_field(&mut o, 2, t);
        }
        o
    }

    /// The `VideoPlaybackAbrRequest` body.
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        w_bytes_field(&mut o, 1, &self.client_abr_state());
        w_bytes_field(&mut o, 5, &self.ustreamer_config);
        w_bytes_field(&mut o, 16, &self.audio.encode());
        w_bytes_field(&mut o, 17, &self.video.encode());
        w_bytes_field(&mut o, 19, &self.streamer_context());
        o
    }
}

/// Add the per-request parameters YouTube's player appends to the base URL.
pub fn request_url(server_abr_url: &str, client_version: &str, cpn: &str, rn: u64) -> String {
    format!(
        "{}&rn={}&cpn={}&cver={}&alr=yes",
        server_abr_url, rn, cpn, client_version
    )
}

/// A client playback nonce: 16 chars from YouTube's alphabet.
pub fn make_cpn() -> String {
    use rand::RngExt;
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut r = rand::rng();
    (0..16).map(|_| A[r.random_range(0..A.len())] as char).collect()
}

/// POST one SABR request and parse the UMP response.
pub async fn fetch(
    http: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
) -> Result<SabrResponse> {
    let resp = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-protobuf")
        .header(reqwest::header::ORIGIN, "https://www.youtube.com")
        .header(reqwest::header::REFERER, "https://www.youtube.com/")
        .body(body)
        .send()
        .await?;
    let status = resp.status();
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !status.is_success() {
        // A 403 that still answers with the UMP content type means the request
        // PARSED and was refused on authorisation, which almost always means a
        // missing or stale PO token rather than a malformed body. Worth saying,
        // because the two failures look identical from the status code alone.
        let hint = if ctype.contains("yt-ump") {
            " (request was understood; this is an authorisation failure, check the PO token)"
        } else {
            ""
        };
        return Err(anyhow!("SABR returned {}{}", status, hint));
    }
    let bytes = resp.bytes().await?;
    Ok(parse_response(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ump_part(kind: u32, payload: &[u8]) -> Vec<u8> {
        // Single-byte varints cover every value used in these fixtures.
        let mut o = vec![kind as u8, payload.len() as u8];
        o.extend_from_slice(payload);
        o
    }

    #[test]
    fn ump_varint_decodes_each_width() {
        // 1-byte: high bit clear.
        assert_eq!(ump_varint(&[0x3a], 0).unwrap().0, 58);
        // 2-byte: 10xxxxxx, low bits of byte 0 then 8 more.
        let (v, p) = ump_varint(&[0x80 | 0x05, 0x01], 0).unwrap();
        assert_eq!(p, 2);
        assert_eq!(v, 0x05 | (1 << 6));
        // 5-byte does not panic.
        assert!(ump_varint(&[0xF8, 1, 2, 3, 4], 0).is_some());
    }

    #[test]
    fn parses_parts_and_reassembles_streams_by_header_id() {
        let mut s = Vec::new();
        s.extend(ump_part(part::REQUEST_IDENTIFIER, b"abc"));
        // header id 0, itag 308
        let mut h = Vec::new();
        w_var_field(&mut h, 1, 0);
        w_var_field(&mut h, 3, 308);
        s.extend(ump_part(part::MEDIA_HEADER, &h));
        // Two MEDIA parts for header 0, each prefixed with the header id.
        s.extend(ump_part(part::MEDIA, &[0x00, b'A', b'B']));
        s.extend(ump_part(part::MEDIA, &[0x00, b'C']));
        s.extend(ump_part(part::MEDIA_END, &[0x00]));

        let r = parse_response(&s);
        assert_eq!(r.streams.len(), 1);
        assert_eq!(r.streams[0].itag, 308);
        assert_eq!(r.streams[0].bytes, b"ABC");
        assert!(r.streams[0].ended);
        assert_eq!(r.media_bytes, 3);
    }

    #[test]
    fn interleaved_streams_stay_separate() {
        // Video and audio arrive interleaved; mixing them would corrupt both.
        let mut s = Vec::new();
        for (hid, itag) in [(0u64, 140u64), (1, 308)] {
            let mut h = Vec::new();
            w_var_field(&mut h, 1, hid);
            w_var_field(&mut h, 3, itag);
            s.extend(ump_part(part::MEDIA_HEADER, &h));
        }
        s.extend(ump_part(part::MEDIA, &[0x00, b'a']));
        s.extend(ump_part(part::MEDIA, &[0x01, b'v']));
        s.extend(ump_part(part::MEDIA, &[0x00, b'a']));
        s.extend(ump_part(part::MEDIA, &[0x01, b'v']));
        let r = parse_response(&s);
        assert_eq!(r.streams.len(), 2);
        assert_eq!(r.streams[0].bytes, b"aa");
        assert_eq!(r.streams[1].bytes, b"vv");
    }

    #[test]
    fn truncated_tail_keeps_what_arrived() {
        let mut s = ump_part(part::MEDIA_HEADER, &{
            let mut h = Vec::new();
            w_var_field(&mut h, 1, 0);
            h
        });
        // Claim 100 bytes but supply 3. The size must stay under 128 to remain a
        // ONE-byte varint: 200 would be read as the start of a three-byte one and
        // swallow the payload, which is what an earlier version of this fixture
        // got wrong.
        s.extend_from_slice(&[part::MEDIA as u8, 100, 0x00, b'X', b'Y']);
        let r = parse_response(&s);
        assert_eq!(r.streams[0].bytes, b"XY");
    }

    #[test]
    fn request_encodes_the_fields_the_server_expects() {
        let req = SabrRequest {
            ustreamer_config: vec![0xAA, 0xBB],
            video: FormatId { itag: 308, last_modified: 123, xtags: None },
            audio: FormatId { itag: 140, last_modified: 456, xtags: None },
            width: 2560,
            height: 1440,
            client_version: "2.20260820.08.00".into(),
            po_token: Some(vec![1, 2, 3]),
        };
        let body = req.encode();
        let mut seen: Vec<u32> = Vec::new();
        pb_fields(&body, |f, _| seen.push(f));
        // Field order and presence, matching the captured request exactly.
        assert_eq!(seen, vec![1, 5, 16, 17, 19]);

        // The ustreamer config must survive byte-for-byte.
        let mut cfg = Vec::new();
        pb_fields(&body, |f, v| {
            if f == 5 {
                if let Val::Bytes(b) = v {
                    cfg = b.to_vec();
                }
            }
        });
        assert_eq!(cfg, vec![0xAA, 0xBB]);
    }

    #[test]
    fn po_token_is_omitted_when_absent_and_present_when_not() {
        let mut req = SabrRequest {
            ustreamer_config: vec![],
            video: FormatId { itag: 308, ..Default::default() },
            audio: FormatId { itag: 140, ..Default::default() },
            width: 2560,
            height: 1440,
            client_version: "v".into(),
            po_token: None,
        };
        let without = req.streamer_context();
        req.po_token = Some(vec![9; 89]);
        let with = req.streamer_context();
        assert!(with.len() > without.len() + 88);
        let mut fields: Vec<u32> = Vec::new();
        pb_fields(&without, |f, _| fields.push(f));
        assert_eq!(fields, vec![1], "no token means no field 2 at all");
    }

    #[test]
    fn cpn_is_sixteen_chars_from_the_expected_alphabet() {
        let c = make_cpn();
        assert_eq!(c.chars().count(), 16);
        assert!(c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
    }

    /// Parse the REAL 6.6 MB capture. Synthetic fixtures cannot catch a varint
    /// width or a part type that only shows up in production traffic.
    ///
    /// ```text
    /// STREAMNOOK_SABR_CAPTURE=<path to resp.bin> \
    ///   cargo test parses_a_real_sabr_capture -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a captured SABR response on disk"]
    fn parses_a_real_sabr_capture() {
        let p = std::env::var("STREAMNOOK_SABR_CAPTURE").expect("STREAMNOOK_SABR_CAPTURE");
        let bytes = std::fs::read(p).expect("read capture");
        let r = parse_response(&bytes);
        println!("formats: {:?}", r.formats);
        println!("media bytes: {:.2} MB", r.media_bytes as f64 / 1048576.0);
        for s in &r.streams {
            println!(
                "  header_id={:<3} itag={:<5} {:>9} bytes  ended={}  mp4={} webm={}",
                s.header_id,
                s.itag,
                s.bytes.len(),
                s.ended,
                s.is_mp4(),
                s.is_webm()
            );
        }
        assert!(!r.streams.is_empty(), "expected reassembled streams");
        assert!(r.media_bytes > 1_000_000, "expected megabytes of media");
        // The whole point: video comes back as fMP4 needing no transmux.
        assert!(
            r.streams.iter().any(|s| s.is_mp4()),
            "expected at least one MP4 stream"
        );
    }
}

// ---------------------------------------------------------------------------
// Session and feed
// ---------------------------------------------------------------------------

/// A live SABR session: everything needed to keep asking for the next media.
///
/// The three session-bound pieces travel together on purpose. The abr url, the
/// ustreamer config and the PO token are minted against one attested session,
/// and mixing them across sessions is refused. Measured: splicing a valid token
/// into a request built from a fresh watch page still returns 403.
#[derive(Debug, Clone)]
pub struct SabrSession {
    pub abr_url: String,
    pub ustreamer_config: Vec<u8>,
    pub video: FormatId,
    pub audio: FormatId,
    pub width: u32,
    pub height: u32,
    pub client_version: String,
    /// What the PO token is bound to, usually this session's visitor data.
    pub content_binding: String,
    /// A token minted alongside this session's other credentials.
    ///
    /// Set this whenever the session came from the webview builder. Minting
    /// separately produces a token from a DIFFERENT attested session, and the
    /// server refuses that mixture with a 403 no matter what it is bound to.
    pub po_token: Option<Vec<u8>>,
    cpn: String,
    rn: u64,
}

impl SabrSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        abr_url: String,
        ustreamer_config: Vec<u8>,
        video: FormatId,
        audio: FormatId,
        width: u32,
        height: u32,
        client_version: String,
        content_binding: String,
    ) -> Self {
        Self {
            abr_url,
            ustreamer_config,
            video,
            audio,
            width,
            height,
            client_version,
            content_binding,
            po_token: None,
            cpn: make_cpn(),
            rn: 0,
        }
    }

    /// Ask for the next window of media.
    ///
    /// Mints (or reuses) the PO token, builds the request and parses the UMP
    /// answer. A 403 here after previously working almost always means the token
    /// went stale, so that case drops it and lets the next call re-mint.
    pub async fn next(&mut self, http: &reqwest::Client) -> Result<SabrResponse> {
        let po_token = match &self.po_token {
            Some(t) => t.clone(),
            None => crate::services::youtube_potoken::mint(&self.content_binding)
                .await
                .map_err(|e| anyhow!("no PO token, so SABR cannot be asked: {}", e))?,
        };

        let req = SabrRequest {
            ustreamer_config: self.ustreamer_config.clone(),
            video: self.video.clone(),
            audio: self.audio.clone(),
            width: self.width,
            height: self.height,
            client_version: self.client_version.clone(),
            po_token: Some(po_token),
        };
        self.rn += 1;
        let url = request_url(&self.abr_url, &self.client_version, &self.cpn, self.rn);

        match fetch(http, &url, req.encode()).await {
            Ok(r) => {
                self.learn_formats(&r);
                Ok(r)
            }
            Err(e) => {
                if e.to_string().contains("403") {
                    crate::services::youtube_potoken::invalidate(&self.content_binding);
                }
                Err(e)
            }
        }
    }

    /// Adopt the `lastModified` the server echoed back.
    ///
    /// The watch page does not publish it for live formats, so the first request
    /// necessarily goes out without one and every later request carries what the
    /// server told us. Only the matching itag is updated, so an unrelated format
    /// in the response cannot rewrite our selection.
    fn learn_formats(&mut self, r: &SabrResponse) {
        for id in &r.format_ids {
            if id.last_modified == 0 {
                continue;
            }
            if id.itag == self.video.itag && self.video.last_modified != id.last_modified {
                self.video.last_modified = id.last_modified;
            } else if id.itag == self.audio.itag && self.audio.last_modified != id.last_modified {
                self.audio.last_modified = id.last_modified;
            }
        }
    }
}

/// Rolling fMP4 segments pulled out of SABR responses, ready to serve as HLS.
///
/// SABR hands back a whole window of interleaved audio and video rather than one
/// numbered segment, so this splits it: the first `ftyp`/`moov` per track is the
/// init segment, and every `moof` chunk becomes one HLS segment.
#[derive(Debug, Default)]
pub struct SabrFeed {
    pub video_init: Option<Vec<u8>>,
    pub audio_init: Option<Vec<u8>>,
    pub video: std::collections::VecDeque<(u64, Vec<u8>)>,
    pub audio: std::collections::VecDeque<(u64, Vec<u8>)>,
    next_video_seq: u64,
    next_audio_seq: u64,
    /// Segments kept per track. Bounded so a long session cannot grow forever.
    pub window: usize,
}

impl SabrFeed {
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(2),
            ..Default::default()
        }
    }

    /// Fold one response in, returning how many (video, audio) segments landed.
    pub fn absorb(&mut self, r: &SabrResponse, video_itag: u32, audio_itag: u32) -> (usize, usize) {
        let (mut v, mut a) = (0usize, 0usize);
        for s in &r.streams {
            if s.bytes.is_empty() {
                continue;
            }
            let is_video = s.itag == video_itag;
            let is_audio = s.itag == audio_itag;
            if !is_video && !is_audio {
                continue;
            }
            // An init segment leads with ftyp; media leads with moof. Anything
            // else (a WebM cluster, say) is a container this path cannot serve.
            let leads_with = |k: &[u8]| s.bytes.len() > 8 && &s.bytes[4..8] == k;
            if leads_with(b"ftyp") {
                if is_video && self.video_init.is_none() {
                    self.video_init = Some(s.bytes.clone());
                } else if is_audio && self.audio_init.is_none() {
                    self.audio_init = Some(s.bytes.clone());
                }
            } else if leads_with(b"moof") {
                if is_video {
                    self.video.push_back((self.next_video_seq, s.bytes.clone()));
                    self.next_video_seq += 1;
                    v += 1;
                } else {
                    self.audio.push_back((self.next_audio_seq, s.bytes.clone()));
                    self.next_audio_seq += 1;
                    a += 1;
                }
            }
        }
        while self.video.len() > self.window {
            self.video.pop_front();
        }
        while self.audio.len() > self.window {
            self.audio.pop_front();
        }
        (v, a)
    }

    pub fn segment(&self, video: bool, seq: u64) -> Option<&[u8]> {
        let q = if video { &self.video } else { &self.audio };
        q.iter().find(|(n, _)| *n == seq).map(|(_, b)| b.as_slice())
    }

    /// Sequence numbers currently servable, oldest first.
    pub fn range(&self, video: bool) -> Option<(u64, u64)> {
        let q = if video { &self.video } else { &self.audio };
        Some((q.front()?.0, q.back()?.0))
    }

    /// Ready once both tracks have an init and at least one segment. Serving a
    /// playlist before that gives the player a 404 on its first fetch.
    pub fn ready(&self) -> bool {
        self.video_init.is_some()
            && self.audio_init.is_some()
            && !self.video.is_empty()
            && !self.audio.is_empty()
    }
}

#[cfg(test)]
mod feed_tests {
    use super::*;

    fn stream(itag: u32, kind: &[u8; 4], n: usize) -> MediaStream {
        let mut b = vec![0, 0, 0, 0];
        b.extend_from_slice(kind);
        b.extend(std::iter::repeat(0xAB).take(n));
        MediaStream {
            header_id: 0,
            itag,
            ended: true,
            bytes: b,
        }
    }

    fn resp(streams: Vec<MediaStream>) -> SabrResponse {
        SabrResponse {
            streams,
            ..Default::default()
        }
    }

    #[test]
    fn splits_init_from_media_and_numbers_segments() {
        let mut f = SabrFeed::new(4);
        let (v, a) = f.absorb(
            &resp(vec![
                stream(400, b"ftyp", 10),
                stream(140, b"ftyp", 10),
                stream(400, b"moof", 50),
                stream(140, b"moof", 20),
                stream(400, b"moof", 50),
            ]),
            400,
            140,
        );
        assert_eq!((v, a), (2, 1));
        assert!(f.video_init.is_some() && f.audio_init.is_some());
        assert_eq!(f.range(true), Some((0, 1)));
        assert_eq!(f.range(false), Some((0, 0)));
        assert!(f.ready());
    }

    #[test]
    fn sequence_numbers_continue_across_responses() {
        let mut f = SabrFeed::new(8);
        f.absorb(
            &resp(vec![stream(400, b"ftyp", 4), stream(400, b"moof", 9)]),
            400,
            140,
        );
        f.absorb(&resp(vec![stream(400, b"moof", 9)]), 400, 140);
        // Restarting numbering per response would make the player refetch seq 0
        // and get different bytes.
        assert_eq!(f.range(true), Some((0, 1)));
    }

    #[test]
    fn the_window_evicts_oldest_first() {
        let mut f = SabrFeed::new(2);
        for _ in 0..5 {
            f.absorb(&resp(vec![stream(400, b"moof", 5)]), 400, 140);
        }
        assert_eq!(f.video.len(), 2);
        assert_eq!(f.range(true), Some((3, 4)));
        assert!(f.segment(true, 0).is_none(), "evicted segments must be gone");
        assert!(f.segment(true, 4).is_some());
    }

    #[test]
    fn only_the_first_init_per_track_is_kept() {
        let mut f = SabrFeed::new(4);
        f.absorb(&resp(vec![stream(400, b"ftyp", 4)]), 400, 140);
        let first = f.video_init.clone();
        f.absorb(&resp(vec![stream(400, b"ftyp", 99)]), 400, 140);
        assert_eq!(
            f.video_init, first,
            "a re-sent init must not replace the served one"
        );
    }

    #[test]
    fn ignores_tracks_that_were_not_requested() {
        let mut f = SabrFeed::new(4);
        // itag 251 is Opus in WebM: a container this path cannot serve.
        let (v, a) = f.absorb(&resp(vec![stream(251, b"moof", 20)]), 400, 140);
        assert_eq!((v, a), (0, 0));
        assert!(!f.ready());
    }

    #[test]
    fn not_ready_until_both_tracks_have_an_init_and_a_segment() {
        let mut f = SabrFeed::new(4);
        f.absorb(
            &resp(vec![stream(400, b"ftyp", 4), stream(400, b"moof", 9)]),
            400,
            140,
        );
        assert!(!f.ready(), "audio still missing");
        f.absorb(
            &resp(vec![stream(140, b"ftyp", 4), stream(140, b"moof", 9)]),
            400,
            140,
        );
        assert!(f.ready());
    }
}
