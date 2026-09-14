//! WebM (VP9) to fragmented-MP4 transmuxer for YouTube's high-resolution live renditions.
//!
//! Why this exists: YouTube's live HLS manifest stops at 1080p, permanently. Its
//! ladder is the legacy MUXED itag family (91-94, 300, 301) whose top rung is
//! 1080p60 by construction, so no client or manifest parameter reaches higher.
//! 1440p and 2160p are published only as video-only `adaptiveFormats` itags
//! (271/308 for 1440p, 313/315 for 2160p) delivered as manifestless DASH
//! fragments addressed by `&sq=N`. Those are VP9 in WebM, and hls.js cannot play
//! WebM, only TS and fMP4.
//!
//! Rather than add a second media engine to the frontend, we rewrite the
//! container here and keep serving ordinary HLS: the player, Plyr chrome and the
//! quality menu are all untouched. That mirrors what `ts_fmp4` already does for
//! the Twitch low-latency origin, and reuses its box writers directly.
//!
//! What makes this cheap: VP9 samples inside a WebM SimpleBlock are already raw
//! frames, so unlike H.264-in-TS there is no bitstream rewriting (no Annex B to
//! length-prefixed conversion, no in-band parameter sets). The work is purely
//! container: read EBML, emit `moof` + `mdat`.
//!
//! Scope: exactly the shape YouTube serves for these itags, measured against
//! live payloads on 2026-08-21. One VP9 video track, one Cluster per segment,
//! `TimecodeScale` 1000000 (millisecond ticks), every segment self-initializing
//! (its own EBML header and Tracks). This is not a general-purpose WebM demuxer.

use crate::services::ts_fmp4::{
    build_fragment, build_trak, build_trex, full_box, mp4_box, unity_matrix, TrackRun, TrunSample,
    SAMPLE_FLAGS_NON_SYNC, SAMPLE_FLAGS_SYNC,
};
use anyhow::{anyhow, Result};

// EBML element ids, kept as full ids (with their length marker) so they can be
// compared straight against what the reader returns.
const ID_EBML_HEADER: u64 = 0x1A45_DFA3;
const ID_SEGMENT: u64 = 0x1853_8067;
const ID_INFO: u64 = 0x1549_A966;
const ID_TIMECODE_SCALE: u64 = 0x2AD7_B1;
const ID_TRACKS: u64 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u64 = 0xAE;
const ID_TRACK_NUMBER: u64 = 0xD7;
const ID_TRACK_TYPE: u64 = 0x83;
const ID_CODEC_ID: u64 = 0x86;
const ID_DEFAULT_DURATION: u64 = 0x23E3_83;
const ID_VIDEO: u64 = 0xE0;
const ID_PIXEL_WIDTH: u64 = 0xB0;
const ID_PIXEL_HEIGHT: u64 = 0xBA;
const ID_CLUSTER: u64 = 0x1F43_B675;
const ID_TIMECODE: u64 = 0xE7;
const ID_SIMPLE_BLOCK: u64 = 0xA3;
const ID_BLOCK_GROUP: u64 = 0xA0;
const ID_BLOCK: u64 = 0xA1;

const TRACK_TYPE_VIDEO: u64 = 1;

/// The video track described by a segment's `Tracks` element.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoTrack {
    pub width: u16,
    pub height: u16,
    /// MP4 media timescale, derived from `TimecodeScale`. YouTube uses 1000000ns
    /// per tick, so this is 1000 and block timecodes are already milliseconds.
    pub timescale: u32,
    /// Nominal frame duration in `timescale` ticks, from `DefaultDuration`. Only
    /// used for the LAST sample of a segment, where there is no next timestamp
    /// to subtract from.
    pub default_duration: u32,
}

/// One coded frame. `ts` is absolute on YouTube's media timeline, which is what
/// keeps this in sync with the separately-fetched audio track for free: the
/// audio fMP4's `baseMediaDecodeTime` counts from the same epoch.
#[derive(Debug, Clone)]
pub struct Sample {
    pub ts: u64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Parsed {
    pub track: VideoTrack,
    pub samples: Vec<Sample>,
}

impl Parsed {
    /// Absolute start of this segment on the media timeline, in track ticks.
    pub fn start_ts(&self) -> u64 {
        self.samples.first().map(|s| s.ts).unwrap_or(0)
    }

    /// Covered span in track ticks, last sample's duration included.
    pub fn duration(&self) -> u64 {
        match (self.samples.first(), self.samples.last()) {
            (Some(f), Some(l)) => (l.ts - f.ts) + self.track.default_duration as u64,
            _ => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// EBML reading
// ---------------------------------------------------------------------------

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }

    /// Read a variable-length integer. `keep_marker` distinguishes the two uses:
    /// element IDs keep their length marker (it is part of the id), sizes strip
    /// it (it is only a length prefix).
    fn vint(&mut self, keep_marker: bool) -> Option<(u64, u8)> {
        let first = *self.b.get(self.pos)?;
        if first == 0 {
            return None; // 8+ byte lengths are not something YouTube emits
        }
        let mut len = 1u8;
        while len <= 8 && (first & (0x80 >> (len - 1))) == 0 {
            len += 1;
        }
        if len > 8 || self.pos + len as usize > self.b.len() {
            return None;
        }
        let mut v: u64 = if keep_marker {
            first as u64
        } else if len == 8 {
            // An 8-byte length puts the marker in the top bit of the first byte
            // and no value bits alongside it, so `0xFF >> 8` would overflow a u8
            // rather than yield zero. Live streams hit this constantly: it is how
            // an unknown-size Segment is written.
            0
        } else {
            (first & (0xFF >> len)) as u64
        };
        for k in 1..len as usize {
            v = (v << 8) | self.b[self.pos + k] as u64;
        }
        self.pos += len as usize;
        Some((v, len))
    }

    /// An element header: its id and the byte range of its payload.
    fn element(&mut self, end: usize) -> Option<(u64, usize, usize)> {
        if self.pos >= end {
            return None;
        }
        let (id, _) = self.vint(true)?;
        let (size, size_len) = self.vint(false)?;
        // "Unknown size" is all value bits set. Live muxers use it for Segment
        // (and sometimes Cluster) because the length is not known when the
        // header is written. Treat it as "runs to the end of what we have".
        let unknown = size == (1u64 << (7 * size_len as u32)) - 1;
        let start = self.pos;
        let stop = if unknown {
            end
        } else {
            start.saturating_add(size as usize).min(end)
        };
        self.pos = stop;
        Some((id, start, stop))
    }
}

fn be_uint(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64)
}

/// Walk the direct children of a payload range.
fn children<F: FnMut(u64, usize, usize)>(b: &[u8], start: usize, end: usize, mut f: F) {
    let mut r = Reader { b, pos: start };
    while let Some((id, s, e)) = r.element(end) {
        f(id, s, e);
        if e <= s && r.pos <= s {
            break; // malformed, do not spin
        }
    }
}

/// Parse one YouTube live WebM segment into a track description and its samples.
pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut seg: Option<(usize, usize)> = None;
    children(bytes, 0, bytes.len(), |id, s, e| {
        if id == ID_SEGMENT && seg.is_none() {
            seg = Some((s, e));
        } else if id != ID_EBML_HEADER && seg.is_none() {
            // Some segments omit the EBML header entirely; tolerate anything
            // before Segment rather than failing.
        }
    });
    let (seg_start, seg_end) = seg.ok_or_else(|| anyhow!("no WebM Segment element"))?;

    let mut timecode_scale_ns: u64 = 1_000_000; // WebM default
    let mut default_duration_ns: u64 = 0;
    let mut width = 0u16;
    let mut height = 0u16;
    let mut video_track_num: Option<u64> = None;
    let mut clusters: Vec<(usize, usize)> = Vec::new();

    children(bytes, seg_start, seg_end, |id, s, e| match id {
        ID_INFO => children(bytes, s, e, |id2, s2, e2| {
            if id2 == ID_TIMECODE_SCALE {
                timecode_scale_ns = be_uint(&bytes[s2..e2]);
            }
        }),
        ID_TRACKS => children(bytes, s, e, |id2, s2, e2| {
            if id2 != ID_TRACK_ENTRY {
                return;
            }
            let mut num = None;
            let mut ttype = 0u64;
            let mut codec = String::new();
            let mut dur = 0u64;
            let (mut w, mut h) = (0u16, 0u16);
            children(bytes, s2, e2, |id3, s3, e3| match id3 {
                ID_TRACK_NUMBER => num = Some(be_uint(&bytes[s3..e3])),
                ID_TRACK_TYPE => ttype = be_uint(&bytes[s3..e3]),
                ID_DEFAULT_DURATION => dur = be_uint(&bytes[s3..e3]),
                ID_CODEC_ID => codec = String::from_utf8_lossy(&bytes[s3..e3]).into_owned(),
                ID_VIDEO => children(bytes, s3, e3, |id4, s4, e4| match id4 {
                    ID_PIXEL_WIDTH => w = be_uint(&bytes[s4..e4]) as u16,
                    ID_PIXEL_HEIGHT => h = be_uint(&bytes[s4..e4]) as u16,
                    _ => {}
                }),
                _ => {}
            });
            // Take the first VIDEO track. Codec is checked because a non-VP9
            // track would produce a valid-looking file that nothing can decode.
            if ttype == TRACK_TYPE_VIDEO && video_track_num.is_none() {
                if codec.trim_end_matches('\0') == "V_VP9" {
                    video_track_num = num;
                    width = w;
                    height = h;
                    default_duration_ns = dur;
                }
            }
        }),
        ID_CLUSTER => clusters.push((s, e)),
        _ => {}
    });

    let track_num =
        video_track_num.ok_or_else(|| anyhow!("no V_VP9 video track in this WebM segment"))?;
    if width == 0 || height == 0 {
        return Err(anyhow!("WebM video track has no pixel dimensions"));
    }
    if timecode_scale_ns == 0 {
        return Err(anyhow!("WebM TimecodeScale is zero"));
    }
    // Ticks per second. 1000000ns per tick gives the millisecond timebase
    // YouTube actually uses.
    let timescale = (1_000_000_000u64 / timecode_scale_ns).max(1) as u32;

    let mut samples: Vec<Sample> = Vec::new();
    for (cs, ce) in clusters {
        let mut cluster_tc: u64 = 0;
        // Timecode precedes the blocks in every cluster YouTube emits, and the
        // spec requires it, so a single forward pass is enough.
        children(bytes, cs, ce, |id, s, e| match id {
            ID_TIMECODE => cluster_tc = be_uint(&bytes[s..e]),
            ID_SIMPLE_BLOCK => {
                if let Some(sm) = read_block(bytes, s, e, cluster_tc, track_num, true) {
                    samples.push(sm);
                }
            }
            ID_BLOCK_GROUP => children(bytes, s, e, |id2, s2, e2| {
                if id2 == ID_BLOCK {
                    // A plain Block carries no keyframe flag; BlockGroup is used
                    // for non-key frames, so treating it as a delta frame is
                    // correct rather than merely safe.
                    if let Some(sm) = read_block(bytes, s2, e2, cluster_tc, track_num, false) {
                        samples.push(sm);
                    }
                }
            }),
            _ => {}
        });
    }

    if samples.is_empty() {
        return Err(anyhow!("WebM segment carried no frames"));
    }
    samples.sort_by_key(|s| s.ts);

    let default_duration = if default_duration_ns > 0 {
        ((default_duration_ns * timescale as u64) / 1_000_000_000).max(1) as u32
    } else {
        // No DefaultDuration: fall back to the median gap so the last sample of
        // a segment still gets a sane duration.
        median_delta(&samples).unwrap_or(1)
    };

    Ok(Parsed {
        track: VideoTrack {
            width,
            height,
            timescale,
            default_duration,
        },
        samples,
    })
}

fn median_delta(samples: &[Sample]) -> Option<u32> {
    if samples.len() < 2 {
        return None;
    }
    let mut d: Vec<u64> = samples.windows(2).map(|w| w[1].ts - w[0].ts).collect();
    d.sort_unstable();
    Some(d[d.len() / 2].max(1) as u32)
}

/// Decode a (Simple)Block header: track number, signed relative timecode, flags,
/// then the frame payload. Lacing is rejected rather than mishandled; YouTube
/// does not lace video.
fn read_block(
    b: &[u8],
    start: usize,
    end: usize,
    cluster_tc: u64,
    want_track: u64,
    has_keyframe_flag: bool,
) -> Option<Sample> {
    let mut r = Reader { b, pos: start };
    let (track, _) = r.vint(false)?;
    if track != want_track {
        return None;
    }
    if r.pos + 3 > end {
        return None;
    }
    let rel = i16::from_be_bytes([b[r.pos], b[r.pos + 1]]) as i64;
    let flags = b[r.pos + 2];
    let data_start = r.pos + 3;
    if data_start >= end {
        return None;
    }
    if flags & 0x06 != 0 {
        return None; // laced block, not a shape YouTube emits for video
    }
    let ts = (cluster_tc as i64 + rel).max(0) as u64;
    Some(Sample {
        ts,
        keyframe: if has_keyframe_flag {
            flags & 0x80 != 0
        } else {
            false
        },
        data: b[data_start..end].to_vec(),
    })
}

// ---------------------------------------------------------------------------
// VP9 configuration
// ---------------------------------------------------------------------------

/// The pieces of a `vpcC` box, and of the `vp09.PP.LL.DD` codec string that the
/// HLS playlist advertises. The codec string matters more than the box: MSE
/// rejects a source whose advertised level is below what the stream needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vp9Config {
    pub profile: u8,
    pub level: u8,
    pub bit_depth: u8,
    pub chroma_subsampling: u8,
    pub full_range: bool,
}

impl Default for Vp9Config {
    fn default() -> Self {
        // Profile 0, 8-bit, 4:2:0 colocated. What YouTube serves for every live
        // VP9 rendition measured.
        Vp9Config {
            profile: 0,
            level: 51,
            bit_depth: 8,
            chroma_subsampling: 1,
            full_range: false,
        }
    }
}

struct Bits<'a> {
    b: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    fn f(&mut self, n: usize) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.bit >> 3;
            if byte >= self.b.len() {
                return v << 1;
            }
            let b = (self.b[byte] >> (7 - (self.bit & 7))) & 1;
            v = (v << 1) | b as u32;
            self.bit += 1;
        }
        v
    }
}

impl Vp9Config {
    /// Read profile, bit depth and subsampling out of a VP9 keyframe's
    /// uncompressed header, then size the level from the picture rate.
    ///
    /// Falls back to the measured-typical defaults on anything unexpected: a
    /// wrong-but-plausible config plays, whereas refusing to emit a segment
    /// does not.
    pub fn from_keyframe(data: &[u8], width: u16, height: u16, fps: f64) -> Vp9Config {
        let mut cfg = Vp9Config {
            level: level_for(width, height, fps),
            ..Default::default()
        };
        let mut r = Bits { b: data, bit: 0 };
        if r.f(2) != 2 {
            return cfg; // not a VP9 frame marker
        }
        let low = r.f(1);
        let high = r.f(1);
        let profile = ((high << 1) | low) as u8;
        if profile > 3 {
            return cfg;
        }
        if profile == 3 {
            r.f(1); // reserved_zero
        }
        cfg.profile = profile;
        if r.f(1) == 1 {
            return cfg; // show_existing_frame, carries no color config
        }
        if r.f(1) != 0 {
            return cfg; // not a keyframe, color config is not present
        }
        r.f(1); // show_frame
        r.f(1); // error_resilient_mode
        if r.f(24) != 0x49_8342 {
            return cfg; // frame sync code mismatch
        }
        cfg.bit_depth = if profile >= 2 {
            if r.f(1) == 1 {
                12
            } else {
                10
            }
        } else {
            8
        };
        let color_space = r.f(3);
        if color_space != 7 {
            cfg.full_range = r.f(1) == 1;
            if profile == 1 || profile == 3 {
                let sx = r.f(1);
                let sy = r.f(1);
                cfg.chroma_subsampling = match (sx, sy) {
                    (1, 1) => 1, // 4:2:0
                    (1, 0) => 2, // 4:2:2
                    _ => 3,      // 4:4:4
                };
            } else {
                cfg.chroma_subsampling = 1; // profiles 0/2 are 4:2:0 only
            }
        } else {
            cfg.full_range = true;
            cfg.chroma_subsampling = 3;
        }
        cfg
    }

    /// `vp09.PP.LL.DD`, the form MSE and the HLS `CODECS` attribute expect.
    pub fn codec_string(&self) -> String {
        format!(
            "vp09.{:02}.{:02}.{:02}",
            self.profile, self.level, self.bit_depth
        )
    }

    fn vpcc(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(12);
        p.push(self.profile);
        p.push(self.level);
        // bitDepth:4 | chromaSubsampling:3 | videoFullRangeFlag:1
        p.push((self.bit_depth << 4) | (self.chroma_subsampling << 1) | self.full_range as u8);
        p.push(2); // colourPrimaries: BT.709
        p.push(2); // transferCharacteristics: BT.709
        p.push(2); // matrixCoefficients: BT.709
        p.extend_from_slice(&0u16.to_be_bytes()); // codecInitializationDataSize
        full_box(b"vpcC", 1, 0, &p)
    }
}

/// VP9 level from the spec's max luma sample rate / picture size table, rounded
/// UP. A level that is too high still plays; too low can be refused outright.
fn level_for(width: u16, height: u16, fps: f64) -> u8 {
    const TABLE: &[(u8, u64, u64)] = &[
        (10, 829_440, 36_864),
        (11, 2_764_800, 73_728),
        (20, 4_608_000, 122_880),
        (21, 9_216_000, 245_760),
        (30, 20_736_000, 552_960),
        (31, 36_864_000, 983_040),
        (40, 83_558_400, 2_228_224),
        (41, 160_432_128, 2_228_224),
        (50, 311_951_360, 8_912_896),
        (51, 588_251_136, 8_912_896),
        (52, 1_176_502_272, 8_912_896),
        (60, 1_176_502_272, 35_651_584),
        (61, 2_353_004_544, 35_651_584),
        (62, 4_706_009_088, 35_651_584),
    ];
    let picture = width as u64 * height as u64;
    let rate = (picture as f64 * fps.max(1.0)).round() as u64;
    for (level, max_rate, max_picture) in TABLE {
        if rate <= *max_rate && picture <= *max_picture {
            return *level;
        }
    }
    62
}

// ---------------------------------------------------------------------------
// fMP4 output
// ---------------------------------------------------------------------------

/// `vp09` sample entry (ISO/IEC 14496-15 style visual entry plus `vpcC`).
fn build_vp09(cfg: &Vp9Config, dims: (u16, u16)) -> Vec<u8> {
    let mut e = Vec::with_capacity(96);
    e.extend_from_slice(&[0u8; 6]); // reserved
    e.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    e.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
    e.extend_from_slice(&0u16.to_be_bytes()); // reserved
    e.extend_from_slice(&[0u8; 12]); // pre_defined
    e.extend_from_slice(&dims.0.to_be_bytes());
    e.extend_from_slice(&dims.1.to_be_bytes());
    e.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // horizresolution 72dpi
    e.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // vertresolution 72dpi
    e.extend_from_slice(&0u32.to_be_bytes()); // reserved
    e.extend_from_slice(&1u16.to_be_bytes()); // frame_count
    e.extend_from_slice(&[0u8; 32]); // compressorname
    e.extend_from_slice(&0x0018u16.to_be_bytes()); // depth
    e.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined
    e.extend_from_slice(&cfg.vpcc());
    mp4_box(b"vp09", &e)
}

/// The `ftyp` + `moov` init segment, served once via `EXT-X-MAP`.
pub fn init_segment(track: &VideoTrack, cfg: &Vp9Config) -> Vec<u8> {
    let mut ftyp_payload = Vec::new();
    ftyp_payload.extend_from_slice(b"isom");
    ftyp_payload.extend_from_slice(&512u32.to_be_bytes());
    ftyp_payload.extend_from_slice(b"isom");
    ftyp_payload.extend_from_slice(b"iso6");
    ftyp_payload.extend_from_slice(b"mp41");
    let ftyp = mp4_box(b"ftyp", &ftyp_payload);

    let mut mvhd_payload = Vec::new();
    mvhd_payload.extend_from_slice(&0u32.to_be_bytes()); // creation
    mvhd_payload.extend_from_slice(&0u32.to_be_bytes()); // modification
    mvhd_payload.extend_from_slice(&1000u32.to_be_bytes()); // timescale
    mvhd_payload.extend_from_slice(&0u32.to_be_bytes()); // duration (live)
    mvhd_payload.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
    mvhd_payload.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
    mvhd_payload.extend_from_slice(&[0u8; 10]); // reserved
    mvhd_payload.extend_from_slice(&unity_matrix());
    mvhd_payload.extend_from_slice(&[0u8; 24]); // pre_defined
    mvhd_payload.extend_from_slice(&2u32.to_be_bytes()); // next_track_ID
    let mvhd = full_box(b"mvhd", 0, 0, &mvhd_payload);

    let dims = (track.width, track.height);
    let trak = build_trak(
        1,
        dims,
        track.timescale,
        &build_vp09(cfg, dims),
        b"vide",
        b"VideoHandler\0",
        &full_box(b"vmhd", 0, 1, &[0u8; 8]),
    );

    let mut moov_payload = mvhd;
    moov_payload.extend_from_slice(&trak);
    moov_payload.extend_from_slice(&mp4_box(b"mvex", &build_trex(1)));

    let mut out = ftyp;
    out.extend_from_slice(&mp4_box(b"moov", &moov_payload));
    out
}

/// One `moof` + `mdat` for a parsed segment.
///
/// Sample durations come from the gap to the NEXT sample rather than from
/// `DefaultDuration`, because at 60fps in a millisecond timebase the real gaps
/// alternate 17/17/16 and using a constant would drift against the audio track.
/// Only the final sample, which has no successor, falls back to the nominal.
pub fn media_segment(seq: u32, parsed: &Parsed) -> Vec<u8> {
    let samples = &parsed.samples;
    let mut runs: Vec<TrunSample> = Vec::with_capacity(samples.len());
    let mut data: Vec<u8> = Vec::with_capacity(samples.iter().map(|s| s.data.len()).sum());
    for (i, s) in samples.iter().enumerate() {
        let duration = match samples.get(i + 1) {
            Some(next) => (next.ts - s.ts).max(1) as u32,
            None => parsed.track.default_duration,
        };
        runs.push(TrunSample {
            duration,
            size: s.data.len() as u32,
            flags: if s.keyframe {
                SAMPLE_FLAGS_SYNC
            } else {
                SAMPLE_FLAGS_NON_SYNC
            },
            // VP9 in this configuration has no reordering, so decode and
            // presentation order coincide.
            cts: 0,
        });
        data.extend_from_slice(&s.data);
    }

    build_fragment(
        seq,
        &[TrackRun {
            track_id: 1,
            tfdt: parsed.start_ts(),
            default_flags: None,
            samples: runs,
            data,
        }],
    )
}

// ---------------------------------------------------------------------------
// Audio: box surgery, not muxing
// ---------------------------------------------------------------------------

/// Split a YouTube audio fragment (itag 140) into its init and media halves.
///
/// These arrive as complete self-contained files: `ftyp` `moov` [`emsg`] `moof`
/// `mdat`. HLS wants the init once via `EXT-X-MAP` and the media alone in each
/// segment, so this is a box-boundary split with no re-muxing. `emsg` is dropped
/// because it carries DASH-only inband event signalling.
pub fn split_audio_fragment(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut init: Vec<u8> = Vec::new();
    let mut media: Vec<u8> = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= bytes.len() {
        let mut size = u32::from_be_bytes([
            bytes[pos],
            bytes[pos + 1],
            bytes[pos + 2],
            bytes[pos + 3],
        ]) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let mut header = 8usize;
        if size == 1 {
            if pos + 16 > bytes.len() {
                break;
            }
            size = u64::from_be_bytes(bytes[pos + 8..pos + 16].try_into().unwrap()) as usize;
            header = 16;
        }
        if size == 0 {
            size = bytes.len() - pos;
        }
        if size < header || pos + size > bytes.len() {
            return Err(anyhow!("truncated MP4 box '{}'", String::from_utf8_lossy(kind)));
        }
        match kind {
            b"ftyp" | b"moov" => init.extend_from_slice(&bytes[pos..pos + size]),
            b"moof" | b"mdat" => media.extend_from_slice(&bytes[pos..pos + size]),
            _ => {}
        }
        pos += size;
    }
    if media.is_empty() {
        return Err(anyhow!("audio fragment carried no moof/mdat"));
    }
    Ok((init, media))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal EBML writer, so the fixtures below describe the shape YouTube
    /// sends without committing a multi-megabyte capture to the repo.
    fn elem(id: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        let len = payload.len() as u64;
        // 4-byte size vint keeps this simple and covers every fixture here.
        out.extend_from_slice(&(0x1000_0000u32 | len as u32).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn uint_elem(id: &[u8], v: u64) -> Vec<u8> {
        let mut bytes = v.to_be_bytes().to_vec();
        while bytes.len() > 1 && bytes[0] == 0 {
            bytes.remove(0);
        }
        elem(id, &bytes)
    }

    fn simple_block(track: u8, rel: i16, keyframe: bool, payload: &[u8]) -> Vec<u8> {
        let mut b = vec![0x80 | track];
        b.extend_from_slice(&rel.to_be_bytes());
        b.push(if keyframe { 0x80 } else { 0x00 });
        b.extend_from_slice(payload);
        elem(&[0xA3], &b)
    }

    /// One VP9 track, 2560x1440, 60fps, one cluster, three frames.
    fn fixture() -> Vec<u8> {
        let video = [uint_elem(&[0xB0], 2560), uint_elem(&[0xBA], 1440)].concat();
        let track_entry = [
            uint_elem(&[0xD7], 1),
            uint_elem(&[0x83], 1),
            elem(&[0x86], b"V_VP9"),
            uint_elem(&[0x23, 0xE3, 0x83], 16_666_666),
            elem(&[0xE0], &video),
        ]
        .concat();
        let tracks = elem(&[0x16, 0x54, 0xAE, 0x6B], &elem(&[0xAE], &track_entry));
        let info = elem(
            &[0x15, 0x49, 0xA9, 0x66],
            &uint_elem(&[0x2A, 0xD7, 0xB1], 1_000_000),
        );
        let cluster_body = [
            uint_elem(&[0xE7], 13_631_638_533),
            simple_block(1, 0, true, &[0xAA; 40]),
            simple_block(1, 17, false, &[0xBB; 10]),
            simple_block(1, 34, false, &[0xCC; 12]),
        ]
        .concat();
        let cluster = elem(&[0x1F, 0x43, 0xB6, 0x75], &cluster_body);
        let segment = elem(
            &[0x18, 0x53, 0x80, 0x67],
            &[info, tracks, cluster].concat(),
        );
        [elem(&[0x1A, 0x45, 0xDF, 0xA3], &[0x42, 0x86, 0x81, 0x01]), segment].concat()
    }

    #[test]
    fn parses_track_and_samples() {
        let p = parse(&fixture()).expect("parse");
        assert_eq!(p.track.width, 2560);
        assert_eq!(p.track.height, 1440);
        // TimecodeScale 1000000ns per tick means a millisecond timebase.
        assert_eq!(p.track.timescale, 1000);
        // DefaultDuration 16666666ns at 1000 ticks/s rounds to 16.
        assert_eq!(p.track.default_duration, 16);
        assert_eq!(p.samples.len(), 3);
        assert!(p.samples[0].keyframe);
        assert!(!p.samples[1].keyframe);
        // Absolute media-timeline positions, which is what keeps the separately
        // fetched audio track in sync.
        assert_eq!(p.samples[0].ts, 13_631_638_533);
        assert_eq!(p.samples[2].ts, 13_631_638_567);
        assert_eq!(p.samples[0].data.len(), 40);
    }

    #[test]
    fn sample_durations_follow_real_gaps_not_the_nominal() {
        let p = parse(&fixture()).expect("parse");
        let seg = media_segment(7, &p);
        // trun durations: 17, 17 from the gaps, then the nominal for the last.
        let trun = find_box(&seg, b"trun").expect("trun");
        let count = u32::from_be_bytes(trun[4..8].try_into().unwrap());
        assert_eq!(count, 3);
        let mut d = Vec::new();
        for i in 0..count as usize {
            let off = 12 + i * 16;
            d.push(u32::from_be_bytes(trun[off..off + 4].try_into().unwrap()));
        }
        assert_eq!(d, vec![17, 17, 16]);
    }

    #[test]
    fn fragment_carries_the_absolute_start_time() {
        let p = parse(&fixture()).expect("parse");
        let seg = media_segment(1, &p);
        let tfdt = find_box(&seg, b"tfdt").expect("tfdt");
        assert_eq!(tfdt[0], 1); // version 1, 64-bit
        let bmdt = u64::from_be_bytes(tfdt[4..12].try_into().unwrap());
        assert_eq!(bmdt, 13_631_638_533);
    }

    #[test]
    fn media_segment_is_moof_then_mdat_with_all_sample_bytes() {
        let p = parse(&fixture()).expect("parse");
        let seg = media_segment(1, &p);
        assert_eq!(&seg[4..8], b"moof");
        let mdat = find_box(&seg, b"mdat").expect("mdat");
        assert_eq!(mdat.len(), 40 + 10 + 12);
    }

    #[test]
    fn init_segment_advertises_vp9_and_the_right_dimensions() {
        let p = parse(&fixture()).expect("parse");
        let cfg = Vp9Config::from_keyframe(&p.samples[0].data, 2560, 1440, 60.0);
        let init = init_segment(&p.track, &cfg);
        assert_eq!(&init[4..8], b"ftyp");
        assert!(find_box(&init, b"moov").is_some());
        let vp09 = find_box(&init, b"vp09").expect("vp09 sample entry");
        let w = u16::from_be_bytes(vp09[24..26].try_into().unwrap());
        let h = u16::from_be_bytes(vp09[26..28].try_into().unwrap());
        assert_eq!((w, h), (2560, 1440));
        assert!(find_box(&init, b"vpcC").is_some());
    }

    #[test]
    fn level_is_sized_up_for_the_resolutions_that_matter() {
        // The whole point of this module: 1440p60 and 2160p60 must advertise a
        // level high enough that MSE accepts them.
        assert_eq!(level_for(2560, 1440, 60.0), 50);
        assert_eq!(level_for(3840, 2160, 60.0), 51);
        // 1080p60 is 124.4M luma samples/s, which overflows level 4.0's 83.5M
        // budget, so it correctly lands on 4.1 rather than 4.0.
        assert_eq!(level_for(1920, 1080, 60.0), 41);
        assert_eq!(level_for(1920, 1080, 30.0), 40);
    }

    #[test]
    fn codec_string_is_the_form_mse_expects() {
        let cfg = Vp9Config {
            profile: 0,
            level: 50,
            bit_depth: 8,
            ..Default::default()
        };
        assert_eq!(cfg.codec_string(), "vp09.00.50.08");
    }

    #[test]
    fn unknown_size_segment_still_parses() {
        // Live muxers write Segment with an unknown size because the length is
        // not known when the header goes out.
        let f = fixture();
        let mut broken = f.clone();
        let i = broken
            .windows(4)
            .position(|w| w == [0x18, 0x53, 0x80, 0x67])
            .expect("segment id");
        broken[i + 4..i + 8].copy_from_slice(&[0x1F, 0xFF, 0xFF, 0xFF]);
        let p = parse(&broken).expect("unknown-size parse");
        assert_eq!(p.samples.len(), 3);
    }

    #[test]
    fn eight_byte_unknown_size_does_not_overflow() {
        // Regression: real YouTube segments write Segment as an 8-byte unknown
        // size (0x01FF..FF). Masking the marker out of the first byte with
        // `0xFF >> 8` overflows a u8, which panicked on every live capture while
        // the 4-byte synthetic fixtures passed.
        let f = fixture();
        let mut wide = f.clone();
        let i = wide
            .windows(4)
            .position(|w| w == [0x18, 0x53, 0x80, 0x67])
            .expect("segment id");
        let body = wide.split_off(i + 8);
        wide.truncate(i + 4);
        wide.extend_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        wide.extend_from_slice(&body);
        let p = parse(&wide).expect("8-byte unknown size");
        assert_eq!(p.samples.len(), 3);
    }

    #[test]
    fn rejects_a_non_vp9_track() {
        let f = fixture();
        let mut other = f.clone();
        let i = other
            .windows(5)
            .position(|w| w == b"V_VP9")
            .expect("codec id");
        other[i..i + 5].copy_from_slice(b"V_AV1");
        assert!(parse(&other).is_err());
    }

    #[test]
    fn splits_an_audio_fragment_into_init_and_media() {
        let mut f = Vec::new();
        f.extend_from_slice(&mp4_box(b"ftyp", b"isom"));
        f.extend_from_slice(&mp4_box(b"moov", &[0u8; 16]));
        f.extend_from_slice(&mp4_box(b"emsg", &[0u8; 8]));
        f.extend_from_slice(&mp4_box(b"moof", &[0u8; 24]));
        f.extend_from_slice(&mp4_box(b"mdat", &[0xEE; 32]));
        let (init, media) = split_audio_fragment(&f).expect("split");
        assert_eq!(&init[4..8], b"ftyp");
        assert!(find_box(&init, b"moov").is_some());
        // emsg is DASH-only inband signalling and must not reach the player.
        assert!(find_box(&init, b"emsg").is_none());
        assert!(find_box(&media, b"emsg").is_none());
        assert_eq!(&media[4..8], b"moof");
        assert!(find_box(&media, b"mdat").is_some());
    }

    /// Transmux a REAL captured YouTube segment and write the result out, so the
    /// bytes can be fed to a decoder rather than merely re-parsed by the code
    /// that produced them. Synthetic fixtures cannot catch a wrong box order or
    /// a bad sample entry; a decoder can.
    ///
    /// Opt-in, because the input is a multi-megabyte capture that does not
    /// belong in the repo:
    ///
    /// ```text
    /// STREAMNOOK_WEBM_FIXTURE=C:\path\seg.webm \
    /// STREAMNOOK_WEBM_OUT=C:\path\out \
    ///   cargo test transmuxes_a_real_capture -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a captured YouTube WebM segment on disk"]
    fn transmuxes_a_real_capture() {
        let src = std::env::var("STREAMNOOK_WEBM_FIXTURE").expect("STREAMNOOK_WEBM_FIXTURE");
        let bytes = std::fs::read(&src).expect("read fixture");
        let parsed = parse(&bytes).expect("parse real capture");
        println!(
            "track {}x{} timescale={} default_duration={} samples={}",
            parsed.track.width,
            parsed.track.height,
            parsed.track.timescale,
            parsed.track.default_duration,
            parsed.samples.len()
        );
        let fps = if parsed.track.default_duration > 0 {
            parsed.track.timescale as f64 / parsed.track.default_duration as f64
        } else {
            60.0
        };
        let cfg = Vp9Config::from_keyframe(
            &parsed.samples[0].data,
            parsed.track.width,
            parsed.track.height,
            fps,
        );
        println!("vp9 config {:?} codec={}", cfg, cfg.codec_string());
        assert!(parsed.samples[0].keyframe, "first sample must be a keyframe");

        let init = init_segment(&parsed.track, &cfg);
        let media = media_segment(1, &parsed);

        // Every source byte must survive into the mdat, or frames are being lost.
        let src_bytes: usize = parsed.samples.iter().map(|s| s.data.len()).sum();
        let mdat = find_box(&media, b"mdat").expect("mdat");
        assert_eq!(mdat.len(), src_bytes, "mdat must carry every sample byte");

        if let Ok(dir) = std::env::var("STREAMNOOK_WEBM_OUT") {
            std::fs::create_dir_all(&dir).expect("create out dir");
            std::fs::write(format!("{}/init.mp4", dir), &init).expect("write init");
            std::fs::write(format!("{}/seg1.m4s", dir), &media).expect("write seg");
            println!("wrote init.mp4 ({} B) and seg1.m4s ({} B)", init.len(), media.len());
        }
    }

    /// Payload of the first box of this type, searched recursively.
    fn find_box<'a>(b: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
        let mut pos = 0usize;
        while pos + 8 <= b.len() {
            let size = u32::from_be_bytes(b[pos..pos + 4].try_into().ok()?) as usize;
            if size < 8 || pos + size > b.len() {
                return None;
            }
            let this = &b[pos + 4..pos + 8];
            let payload = &b[pos + 8..pos + size];
            if this == kind {
                return Some(payload);
            }
            if matches!(
                this,
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"stsd" | b"moof" | b"traf" | b"vp09"
            ) {
                // Both containers below hold their children after a fixed
                // preamble: stsd after version/flags + entry count, and a visual
                // sample entry after the 78-byte VisualSampleEntry header.
                let skip = match this {
                    b"stsd" => 8,
                    b"vp09" => 78,
                    _ => 0,
                };
                if let Some(found) = payload.get(skip..).and_then(|inner| find_box(inner, kind)) {
                    return Some(found);
                }
            }
            pos += size;
        }
        None
    }
}
