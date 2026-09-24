//! FLV (H.264 + AAC) to fragmented MP4, for the TikTok rooms that publish
//! nothing but an FLV pull.
//!
//! Why this exists: most TikTok rooms offer a DASH manifest the relay can
//! translate, but a real share publish only `flv`. Measured across the rooms on
//! TikTok's own Top live pages, two in eleven carried no `cmaf`, no `hls` and no
//! `dash` at all, on every tier. Refusing those would mean one card in five or
//! six not playing. hls.js cannot play FLV, and a second media engine in the
//! page is ruled out, so the container is rewritten here and the page keeps
//! getting ordinary HLS over fMP4.
//!
//! What makes this cheap: FLV already carries everything MP4 wants in the form
//! MP4 wants it. The H.264 sequence header IS an `AVCDecoderConfigurationRecord`
//! (the `avcC` payload, verbatim), each video tag's NAL units are already
//! length-prefixed, and the AAC sequence header IS the `AudioSpecificConfig`.
//! So there is no bitstream rewriting: the work is reading tags, cutting
//! segments, and writing `moof` + `mdat` with the box writers `ts_fmp4`
//! already has.
//!
//! Segments are one second, not one GOP. A player's start waits for whole
//! segments, and cutting on keyframes alone meant waiting for two two-second
//! GOPs to finish arriving. Only the FIRST segment has to open on a keyframe
//! (the player starts there); after it the player appends in order, so a
//! segment may begin mid-GOP, and one still opens on a keyframe wherever the
//! stream puts one near the cut.
//!
//! What the stream looks like, measured from a live pull (the rules below are
//! shaped by each of these):
//!   * the CDN opens with a replay of its cache: every cached AUDIO tag first,
//!     then the video sequence header, then the cached video from its oldest
//!     keyframe (the current GOP, sometimes the one before it too);
//!   * sequence headers carry timestamps unrelated to the media (0 and 50 ms
//!     against a media clock in the millions), so they never drive timing;
//!   * two second GOPs, B-frames (non-zero composition offsets), and SPS/PPS
//!     only in the sequence header, never in-band;
//!   * AAC-LC at a 24 kHz core rate, one frame every 42 to 43 ms.
//!
//! Timing model: video runs on a 90 kHz timescale (FLV's millisecond DTS times
//! 90) and keeps the stream's own clock, so a reconnect that replays the cache
//! lands on the same timeline and is recognised as a replay. Audio runs on its
//! sample rate and its `tfdt` follows the running frame count rather than each
//! frame's rounded millisecond, since AAC is gapless and re-deriving it from
//! milliseconds would put a seam at every fragment. A clock that jumps (an
//! encoder restart) starts a new timeline, marked for the playlist as a
//! discontinuity.

use crate::services::ts_fmp4::{
    build_fragment, build_trak, build_trex, full_box, mp4_box, parse_sps_dimensions,
    unity_matrix, TrackRun, TrunSample, SAMPLE_FLAGS_NON_SYNC, SAMPLE_FLAGS_SYNC,
};
use anyhow::{anyhow, Result};
use std::collections::VecDeque;

const VIDEO_TIMESCALE: u32 = 90_000;
/// A tag larger than this is corruption rather than video, and buffering
/// toward it would let a broken stream grow memory without bound. A 4K
/// keyframe is a few megabytes; TikTok's 720p ones are about thirty kilobytes.
const MAX_TAG: usize = 8 * 1024 * 1024;
/// A segment is cut once it holds this much video. Short, because the relay's
/// start hold waits for a few seconds of whole segments, and a long segment is
/// a long wait for its last frame.
const SEGMENT_MS: u32 = 1_000;
/// A keyframe at least this far into the open segment cuts it early, so the
/// next segment opens on the keyframe rather than just after it.
const KEY_CUT_MS: u32 = 500;
/// How much audio an audio-only segment holds.
const AUDIO_SEGMENT_MS: u32 = 1_000;
/// A timestamp this far from the previous one, in either direction, is a new
/// clock rather than a late or replayed frame. A reconnect replays at most one
/// GOP, a few seconds.
const MAX_JUMP_MS: u32 = 10_000;
/// Closed video segments allowed to wait for their audio before the oldest is
/// written without the audio it is still missing. Bounds the delay a stalled
/// audio track can add, to about this many seconds.
const MAX_WAITING: usize = 4;
/// Samples per AAC frame. HE-AAC decodes to twice this, but its core frames,
/// which are what the container counts, are the same length.
const AAC_FRAME: u64 = 1024;
/// Audio frames held while waiting for the video they belong with: about
/// twenty seconds at TikTok's rate, far past anything a healthy stream needs.
const MAX_QUEUED_AUDIO: usize = 480;

// ──────────────────────────── demuxing ────────────────────────────

/// One FLV tag's worth of media, with the container stripped.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// The file header's statement of which tracks the stream carries.
    Header { audio: bool, video: bool },
    /// `AVCDecoderConfigurationRecord`, exactly as the sequence header holds it.
    VideoConfig(Vec<u8>),
    /// `AudioSpecificConfig`.
    AudioConfig(Vec<u8>),
    /// One access unit: length-prefixed NAL units, which is also MP4's layout.
    Video {
        dts: u32,
        cts: i32,
        key: bool,
        data: Vec<u8>,
    },
    /// One raw AAC frame.
    Audio { ts: u32, data: Vec<u8> },
}

/// Incremental FLV reader. Bytes arrive however the network cuts them, so a
/// tag is only read once all of it is here.
#[derive(Default)]
pub struct Demuxer {
    buf: Vec<u8>,
    header_seen: bool,
}

impl Demuxer {
    /// Feed the next bytes of the stream and take every tag they completed.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut pos = 0usize;
        if !self.header_seen {
            if self.buf.len() < 9 {
                return Ok(out);
            }
            if &self.buf[0..3] != b"FLV" {
                return Err(anyhow!("the stream is not FLV"));
            }
            let header_len = u32::from_be_bytes([self.buf[5], self.buf[6], self.buf[7], self.buf[8]]) as usize;
            if !(9..=1024).contains(&header_len) {
                return Err(anyhow!("FLV header of {} bytes", header_len));
            }
            // The header, then the zero PreviousTagSize that precedes tag one.
            if self.buf.len() < header_len + 4 {
                return Ok(out);
            }
            out.push(Frame::Header {
                audio: self.buf[4] & 0x04 != 0,
                video: self.buf[4] & 0x01 != 0,
            });
            pos = header_len + 4;
            self.header_seen = true;
        }
        while self.buf.len() - pos >= 11 {
            let h = &self.buf[pos..pos + 11];
            let size = u24(&h[1..4]) as usize;
            if size > MAX_TAG {
                return Err(anyhow!("FLV tag of {} bytes", size));
            }
            let total = 11 + size + 4;
            if self.buf.len() - pos < total {
                break;
            }
            // Low 24 bits, then the extension byte as the top 8.
            let ts = u24(&h[4..7]) | ((h[7] as u32) << 24);
            let kind = h[0] & 0x1f;
            if h[0] & 0x20 != 0 {
                return Err(anyhow!("the FLV stream is encrypted"));
            }
            if let Some(f) = parse_tag(kind, ts, &self.buf[pos + 11..pos + 11 + size])? {
                out.push(f);
            }
            pos += total;
        }
        self.buf.drain(..pos);
        Ok(out)
    }
}

fn u24(b: &[u8]) -> u32 {
    ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32
}

fn i24(b: &[u8]) -> i32 {
    let v = u24(b) as i32;
    if v & 0x80_0000 != 0 {
        v - 0x100_0000
    } else {
        v
    }
}

fn parse_tag(kind: u8, ts: u32, body: &[u8]) -> Result<Option<Frame>> {
    match kind {
        8 => {
            let Some(&first) = body.first() else {
                return Ok(None);
            };
            let format = first >> 4;
            if format != 10 {
                return Err(anyhow!("FLV audio format {} is not AAC", format));
            }
            match body.get(1) {
                Some(0) => Ok(Some(Frame::AudioConfig(body[2..].to_vec()))),
                Some(1) if body.len() > 2 => Ok(Some(Frame::Audio {
                    ts,
                    data: body[2..].to_vec(),
                })),
                _ => Ok(None),
            }
        }
        9 => {
            let Some(&first) = body.first() else {
                return Ok(None);
            };
            // The "enhanced" header TikTok's CDNs use for HEVC and AV1.
            if first & 0x80 != 0 {
                return Err(anyhow!("the FLV stream is not H.264"));
            }
            let frame_type = first >> 4;
            let codec = first & 0x0f;
            // A video info/command frame carries no picture.
            if frame_type == 5 {
                return Ok(None);
            }
            if codec != 7 {
                return Err(anyhow!("FLV video codec {} is not H.264", codec));
            }
            if body.len() < 5 {
                return Ok(None);
            }
            let cts = i24(&body[2..5]);
            match body[1] {
                0 => Ok(Some(Frame::VideoConfig(body[5..].to_vec()))),
                1 if body.len() > 5 => Ok(Some(Frame::Video {
                    dts: ts,
                    cts,
                    key: frame_type == 1,
                    data: body[5..].to_vec(),
                })),
                // End of sequence, or an empty NALU packet.
                _ => Ok(None),
            }
        }
        // Script data (`onMetaData`) and anything else carries no media.
        _ => Ok(None),
    }
}

// ──────────────────────────── codec descriptions ────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct VideoInfo {
    avcc: Vec<u8>,
    width: u16,
    height: u16,
}

impl VideoInfo {
    fn parse(avcc: &[u8]) -> Result<Self> {
        // configurationVersion, profile, compatibility, level, length size,
        // SPS count, then the first SPS behind its 16 bit length.
        if avcc.len() < 8 || avcc[0] != 1 {
            return Err(anyhow!("malformed H.264 configuration"));
        }
        let sps_len = u16::from_be_bytes([avcc[6], avcc[7]]) as usize;
        let sps = avcc.get(8..8 + sps_len).ok_or_else(|| anyhow!("truncated H.264 configuration"))?;
        // Geometry is informative in the sample entry (the decoder reads the
        // SPS itself), so a parameter set this parser cannot read costs the
        // numbers, not playback.
        let (width, height) = parse_sps_dimensions(sps).unwrap_or((0, 0));
        Ok(Self {
            avcc: avcc.to_vec(),
            width,
            height,
        })
    }

    fn codec(&self) -> String {
        format!("avc1.{:02x}{:02x}{:02x}", self.avcc[1], self.avcc[2], self.avcc[3])
    }
}

#[derive(Debug, Clone, PartialEq)]
struct AudioInfo {
    asc: Vec<u8>,
    object_type: u8,
    sample_rate: u32,
    channels: u16,
}

const AAC_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

impl AudioInfo {
    fn parse(asc: &[u8]) -> Result<Self> {
        let mut bits = Bits { data: asc, pos: 0 };
        let mut object_type = bits.take(5).ok_or_else(|| anyhow!("empty AAC configuration"))?;
        if object_type == 31 {
            object_type = 32 + bits.take(6).ok_or_else(|| anyhow!("truncated AAC configuration"))?;
        }
        let index = bits.take(4).ok_or_else(|| anyhow!("truncated AAC configuration"))?;
        let sample_rate = if index == 15 {
            bits.take(24).ok_or_else(|| anyhow!("truncated AAC configuration"))?
        } else {
            *AAC_RATES
                .get(index as usize)
                .ok_or_else(|| anyhow!("AAC sample rate index {}", index))?
        };
        let config = bits.take(4).ok_or_else(|| anyhow!("truncated AAC configuration"))?;
        if sample_rate == 0 {
            return Err(anyhow!("AAC sample rate of zero"));
        }
        Ok(Self {
            asc: asc.to_vec(),
            object_type: object_type as u8,
            sample_rate,
            channels: match config {
                0 => 2,
                7 => 8,
                n => n as u16,
            },
        })
    }

    fn codec(&self) -> String {
        format!("mp4a.40.{}", self.object_type)
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn take(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.data.get(self.pos / 8)?;
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        Some(v)
    }
}

// ──────────────────────────── init segment ────────────────────────────

const VIDEO_TRACK: u32 = 1;
const AUDIO_TRACK: u32 = 2;

fn init_segment(video: Option<&VideoInfo>, audio: Option<&AudioInfo>) -> Vec<u8> {
    let mut ftyp_payload = Vec::new();
    ftyp_payload.extend_from_slice(b"isom");
    ftyp_payload.extend_from_slice(&512u32.to_be_bytes());
    for brand in [b"isom", b"iso6", b"mp41"] {
        ftyp_payload.extend_from_slice(brand);
    }
    if video.is_some() {
        ftyp_payload.extend_from_slice(b"avc1");
    }
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
    mvhd_payload.extend_from_slice(&3u32.to_be_bytes()); // next_track_ID
    let mut moov_payload = full_box(b"mvhd", 0, 0, &mvhd_payload);

    let mut mvex_payload = Vec::new();
    if let Some(v) = video {
        moov_payload.extend_from_slice(&build_trak(
            VIDEO_TRACK,
            (v.width, v.height),
            VIDEO_TIMESCALE,
            &avc1_entry(v),
            b"vide",
            b"VideoHandler\0",
            &full_box(b"vmhd", 0, 1, &[0u8; 8]),
        ));
        mvex_payload.extend_from_slice(&build_trex(VIDEO_TRACK));
    }
    if let Some(a) = audio {
        moov_payload.extend_from_slice(&build_trak(
            AUDIO_TRACK,
            (0, 0),
            a.sample_rate,
            &mp4a_entry(a),
            b"soun",
            b"SoundHandler\0",
            &full_box(b"smhd", 0, 0, &[0u8; 4]),
        ));
        mvex_payload.extend_from_slice(&build_trex(AUDIO_TRACK));
    }
    moov_payload.extend_from_slice(&mp4_box(b"mvex", &mvex_payload));

    let mut out = ftyp;
    out.extend_from_slice(&mp4_box(b"moov", &moov_payload));
    out
}

/// `avc1`, parameter sets out of band: FLV never sends them in-band, so the
/// sequence header's record is the only copy and it goes in verbatim.
fn avc1_entry(v: &VideoInfo) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&[0u8; 6]); // reserved
    p.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    p.extend_from_slice(&[0u8; 16]); // pre_defined + reserved
    p.extend_from_slice(&v.width.to_be_bytes());
    p.extend_from_slice(&v.height.to_be_bytes());
    p.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi horizontal
    p.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi vertical
    p.extend_from_slice(&0u32.to_be_bytes()); // reserved
    p.extend_from_slice(&1u16.to_be_bytes()); // frame_count
    p.extend_from_slice(&[0u8; 32]); // compressorname
    p.extend_from_slice(&0x0018u16.to_be_bytes()); // depth 24
    p.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined
    p.extend_from_slice(&mp4_box(b"avcC", &v.avcc));
    mp4_box(b"avc1", &p)
}

/// An MPEG-4 descriptor, with its length in the expandable form when it needs
/// more than one byte. The configuration is copied from the stream, so its
/// length is the stream's to choose.
fn descriptor(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = payload.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        out.extend_from_slice(&[
            0x80 | ((len >> 21) & 0x7f) as u8,
            0x80 | ((len >> 14) & 0x7f) as u8,
            0x80 | ((len >> 7) & 0x7f) as u8,
            (len & 0x7f) as u8,
        ]);
    }
    out.extend_from_slice(payload);
    out
}

fn mp4a_entry(a: &AudioInfo) -> Vec<u8> {
    let mut dec_config = vec![
        0x40, // objectTypeIndication: MPEG-4 audio
        0x15, // streamType audio
        0, 0, 0, // bufferSizeDB
        0, 0, 0, 0, // maxBitrate
        0, 0, 0, 0, // avgBitrate
    ];
    dec_config.extend_from_slice(&descriptor(0x05, &a.asc));
    let mut es = vec![0u8, 0, 0]; // ES_ID + flags
    es.extend_from_slice(&descriptor(0x04, &dec_config));
    es.extend_from_slice(&descriptor(0x06, &[0x02])); // SLConfig: MP4
    let esds = full_box(b"esds", 0, 0, &descriptor(0x03, &es));

    let mut p = Vec::new();
    p.extend_from_slice(&[0u8; 6]); // reserved
    p.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    p.extend_from_slice(&[0u8; 8]); // reserved
    p.extend_from_slice(&a.channels.to_be_bytes());
    p.extend_from_slice(&16u16.to_be_bytes()); // samplesize
    p.extend_from_slice(&0u32.to_be_bytes()); // pre_defined + reserved
    // 16.16 fixed point, so a rate above 65535 cannot be written; the media
    // header's timescale carries the real one.
    p.extend_from_slice(&(a.sample_rate.min(0xFFFF) << 16).to_be_bytes());
    p.extend_from_slice(&esds);
    mp4_box(b"mp4a", &p)
}

// ──────────────────────────── segmenting ────────────────────────────

/// What the muxer hands the relay.
#[derive(Debug)]
pub enum Output {
    /// A new init segment. `epoch` changes whenever the codec configuration
    /// does, and every later segment names the epoch it belongs to.
    Init {
        epoch: u32,
        bytes: Vec<u8>,
        codecs: String,
        width: Option<u16>,
        height: Option<u16>,
    },
    Segment(Segment),
}

#[derive(Debug)]
pub struct Segment {
    pub number: u64,
    pub epoch: u32,
    /// The first segment of a new timeline, which the playlist must mark.
    pub discontinuity: bool,
    pub duration: f64,
    pub video_frames: u32,
    pub bytes: Vec<u8>,
}

struct VideoIn {
    dts: u32,
    cts: i32,
    key: bool,
    data: Vec<u8>,
}

/// A run of video from one keyframe, not yet closed by the next.
struct Open {
    start: u32,
    video: Vec<VideoIn>,
    discontinuity: bool,
}

/// A video run whose end is known, waiting for the audio that belongs with it.
struct Closed {
    start: u32,
    end: u32,
    video: Vec<VideoIn>,
    discontinuity: bool,
}

/// Turns demuxed frames into an init segment and a numbered run of fMP4
/// segments. One per relay session, surviving reconnects: a fresh `Demuxer`
/// per connection, the same `Muxer` across them, which is what lets it
/// recognise a reconnect's cache replay as frames it already has.
#[derive(Default)]
pub struct Muxer {
    declared_audio: bool,
    declared_video: bool,
    video: Option<VideoInfo>,
    audio: Option<AudioInfo>,
    /// The configuration the current init describes, once one is out.
    announced: Option<(Option<VideoInfo>, Option<AudioInfo>)>,
    epoch: u32,
    /// The next segment starts a new timeline.
    break_next: bool,
    open: Option<Open>,
    waiting: VecDeque<Closed>,
    audio_queue: VecDeque<(u32, Vec<u8>)>,
    /// Audio-only mode: the frames of the segment being filled.
    audio_run: Vec<(u32, Vec<u8>)>,
    last_video_dts: Option<u32>,
    last_audio_ts: Option<u32>,
    /// The next audio `tfdt`, in sample-rate ticks.
    audio_clock: Option<u64>,
    next_number: u64,
}

impl Muxer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, frame: Frame) -> Result<Vec<Output>> {
        let mut out = Vec::new();
        match frame {
            Frame::Header { audio, video } => {
                self.declared_audio = audio;
                self.declared_video = video;
            }
            Frame::VideoConfig(avcc) => {
                let info = VideoInfo::parse(&avcc)?;
                if self.video.as_ref() != Some(&info) {
                    // A new configuration mid-stream: what is open was coded
                    // under the old one, so it ends here and what follows is a
                    // new timeline under a new init.
                    if self.video.is_some() {
                        self.flush(&mut out);
                        self.break_next = true;
                    }
                    self.video = Some(info);
                }
                self.announce(&mut out);
            }
            Frame::AudioConfig(asc) => {
                let info = AudioInfo::parse(&asc)?;
                if self.audio.as_ref() != Some(&info) {
                    if self.audio.is_some() {
                        self.flush(&mut out);
                        self.break_next = true;
                    }
                    self.audio = Some(info);
                    self.audio_clock = None;
                }
                self.announce(&mut out);
            }
            // Media is taken as soon as its own track is configured, not only
            // once the init is out: the CDN's replay sends every cached audio
            // frame BEFORE the video configuration, and those frames are the
            // audio for the first segment.
            Frame::Video { dts, cts, key, data } => {
                if self.video.is_some() {
                    self.on_video(dts, cts, key, data, &mut out);
                }
            }
            Frame::Audio { ts, data } => {
                if self.audio.is_some() {
                    self.on_audio(ts, data, &mut out);
                }
            }
        }
        Ok(out)
    }

    /// Put out an init once every track the header promised is configured,
    /// and again whenever that configuration changes.
    fn announce(&mut self, out: &mut Vec<Output>) {
        let video_ready = !self.declared_video || self.video.is_some();
        let audio_ready = !self.declared_audio || self.audio.is_some();
        if !video_ready || !audio_ready || (self.video.is_none() && self.audio.is_none()) {
            return;
        }
        let current = (self.video.clone(), self.audio.clone());
        if self.announced.as_ref() == Some(&current) {
            return;
        }
        if self.announced.is_some() {
            self.epoch += 1;
            self.break_next = true;
        }
        let codecs = [
            self.video.as_ref().map(VideoInfo::codec),
            self.audio.as_ref().map(AudioInfo::codec),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(",");
        out.push(Output::Init {
            epoch: self.epoch,
            bytes: init_segment(self.video.as_ref(), self.audio.as_ref()),
            codecs,
            width: self.video.as_ref().map(|v| v.width).filter(|w| *w > 0),
            height: self.video.as_ref().map(|v| v.height).filter(|h| *h > 0),
        });
        self.announced = Some(current);
    }

    fn on_video(&mut self, dts: u32, cts: i32, key: bool, data: Vec<u8>, out: &mut Vec<Output>) {
        if let Some(last) = self.last_video_dts {
            if dts <= last {
                if last - dts <= MAX_JUMP_MS {
                    // A reconnect replaying the CDN's cache: already have it.
                    return;
                }
                self.restart_timeline(out);
            } else if dts - last > MAX_JUMP_MS {
                self.restart_timeline(out);
            }
        }
        self.last_video_dts = Some(dts);

        // Only a keyframe may open a timeline (nothing before one decodes);
        // once open, a segment is cut at a keyframe from half a second in, or
        // at any frame once it is a second long.
        let cut = match &self.open {
            None => key,
            Some(o) => {
                let held = dts.saturating_sub(o.start);
                (key && held >= KEY_CUT_MS) || held >= SEGMENT_MS
            }
        };
        if cut {
            if let Some(o) = self.open.take() {
                self.waiting.push_back(Closed {
                    start: o.start,
                    end: dts,
                    video: o.video,
                    discontinuity: o.discontinuity,
                });
            }
            // The header promised audio that never configured. After a few
            // seconds of picture, play it without the audio rather than nothing.
            if self.announced.is_none() && self.audio.is_none() && self.waiting.len() >= MAX_WAITING {
                self.declared_audio = false;
                self.announce(out);
            }
            self.open = Some(Open {
                start: dts,
                video: Vec::new(),
                discontinuity: std::mem::take(&mut self.break_next),
            });
        }
        // Nothing decodable comes before the first keyframe.
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.video.push(VideoIn { dts, cts, key, data });
        self.drain_waiting(out, false);
    }

    fn on_audio(&mut self, ts: u32, data: Vec<u8>, out: &mut Vec<Output>) {
        if let Some(last) = self.last_audio_ts {
            if ts <= last && last - ts <= MAX_JUMP_MS {
                return;
            }
            if ts.abs_diff(last) > MAX_JUMP_MS {
                // A new audio clock. The video side marks the timeline break;
                // here only the running count has to be re-anchored.
                self.audio_clock = None;
                if self.video.is_none() {
                    self.flush(out);
                    self.break_next = true;
                }
            }
        }
        self.last_audio_ts = Some(ts);

        if !self.declared_video && self.video.is_none() {
            self.audio_run.push((ts, data));
            let first = self.audio_run[0].0;
            let rate = self.audio.as_ref().map(|a| a.sample_rate).unwrap_or(48_000) as u64;
            let span_ms = (self.audio_run.len() as u64 * AAC_FRAME * 1000 / rate) as u32;
            if span_ms >= AUDIO_SEGMENT_MS || ts.saturating_sub(first) >= AUDIO_SEGMENT_MS {
                self.emit_audio_only(out);
            }
            return;
        }
        self.audio_queue.push_back((ts, data));
        // Bounded however long video keeps us waiting: a stream whose header
        // promised a picture that never arrives must not hold audio forever.
        while self.audio_queue.len() > MAX_QUEUED_AUDIO {
            self.audio_queue.pop_front();
        }
        self.drain_waiting(out, false);
    }

    /// The clock jumped: close what is open on the old timeline and begin a
    /// new one.
    fn restart_timeline(&mut self, out: &mut Vec<Output>) {
        self.flush(out);
        self.break_next = true;
        self.audio_clock = None;
        self.last_audio_ts = None;
        self.audio_queue.clear();
    }

    /// Write out everything held, complete or not. Used when what follows
    /// cannot share a timeline with it.
    fn flush(&mut self, out: &mut Vec<Output>) {
        if let Some(o) = self.open.take() {
            if let Some(last) = o.video.last() {
                // No next keyframe to end on, so the last frame gets the run's
                // average length.
                let span = last.dts.saturating_sub(o.start);
                let n = o.video.len().max(2) as u32 - 1;
                let end = last.dts + (span / n).max(1);
                self.waiting.push_back(Closed {
                    start: o.start,
                    end,
                    video: o.video,
                    discontinuity: o.discontinuity,
                });
            }
        }
        self.drain_waiting(out, true);
        if !self.audio_run.is_empty() {
            self.emit_audio_only(out);
        }
    }

    fn drain_waiting(&mut self, out: &mut Vec<Output>, force: bool) {
        // Nothing can be written before an init describes it. Until then the
        // oldest waiting run is what gives way.
        if self.announced.is_none() {
            while self.waiting.len() > MAX_WAITING + 1 {
                self.waiting.pop_front();
            }
            return;
        }
        while let Some(front) = self.waiting.front() {
            let audio_caught_up = self.audio.is_none()
                || self.last_audio_ts.map(|t| t >= front.end).unwrap_or(false);
            if !(force || audio_caught_up || self.waiting.len() > MAX_WAITING) {
                break;
            }
            let Some(seg) = self.waiting.pop_front() else {
                break;
            };
            self.emit_video(seg, out);
        }
    }

    fn emit_video(&mut self, seg: Closed, out: &mut Vec<Output>) {
        let mut samples = Vec::with_capacity(seg.video.len());
        let mut data = Vec::new();
        for (i, v) in seg.video.iter().enumerate() {
            let next = seg.video.get(i + 1).map(|n| n.dts).unwrap_or(seg.end);
            let duration = (next.saturating_sub(v.dts) * (VIDEO_TIMESCALE / 1000)).max(1);
            samples.push(TrunSample {
                duration,
                size: v.data.len() as u32,
                flags: if v.key { SAMPLE_FLAGS_SYNC } else { SAMPLE_FLAGS_NON_SYNC },
                cts: v.cts.saturating_mul((VIDEO_TIMESCALE / 1000) as i32),
            });
            data.extend_from_slice(&v.data);
        }
        let mut runs = vec![TrackRun {
            track_id: VIDEO_TRACK,
            tfdt: seg.start as u64 * (VIDEO_TIMESCALE / 1000) as u64,
            default_flags: None,
            samples,
            data,
        }];

        // The audio belonging to this stretch. Anything older belonged to a
        // segment already written (or came before the first keyframe) and is
        // dropped rather than written out of order.
        let mut frames = Vec::new();
        while let Some((ts, _)) = self.audio_queue.front() {
            if *ts >= seg.end {
                break;
            }
            let (ts, bytes) = self.audio_queue.pop_front().unwrap_or_default();
            if ts >= seg.start {
                frames.push((ts, bytes));
            }
        }
        if let Some(run) = self.audio_run_for(&frames) {
            runs.push(run);
        }

        let number = self.next_number;
        self.next_number += 1;
        out.push(Output::Segment(Segment {
            number,
            epoch: self.epoch,
            discontinuity: seg.discontinuity,
            duration: (seg.end - seg.start) as f64 / 1000.0,
            video_frames: seg.video.len() as u32,
            bytes: build_fragment(number as u32 + 1, &runs),
        }));
    }

    fn emit_audio_only(&mut self, out: &mut Vec<Output>) {
        let frames = std::mem::take(&mut self.audio_run);
        let Some(run) = self.audio_run_for(&frames) else {
            return;
        };
        let rate = self.audio.as_ref().map(|a| a.sample_rate).unwrap_or(48_000) as f64;
        let number = self.next_number;
        self.next_number += 1;
        out.push(Output::Segment(Segment {
            number,
            epoch: self.epoch,
            discontinuity: std::mem::take(&mut self.break_next),
            duration: run.samples.len() as f64 * AAC_FRAME as f64 / rate,
            video_frames: 0,
            bytes: build_fragment(number as u32 + 1, &[run]),
        }));
    }

    /// One contiguous audio run. The clock re-anchors to a frame's own
    /// timestamp only at a run's start and only when it disagrees by more than
    /// a frame, which is a real gap rather than millisecond rounding.
    fn audio_run_for(&mut self, frames: &[(u32, Vec<u8>)]) -> Option<TrackRun> {
        let (first_ts, _) = frames.first()?;
        let rate = self.audio.as_ref()?.sample_rate as u64;
        let measured = *first_ts as u64 * rate / 1000;
        let tfdt = match self.audio_clock {
            Some(c) if c.abs_diff(measured) <= AAC_FRAME => c,
            _ => measured,
        };
        self.audio_clock = Some(tfdt + frames.len() as u64 * AAC_FRAME);
        let mut data = Vec::new();
        let samples = frames
            .iter()
            .map(|(_, bytes)| {
                data.extend_from_slice(bytes);
                TrunSample {
                    duration: AAC_FRAME as u32,
                    size: bytes.len() as u32,
                    flags: SAMPLE_FLAGS_SYNC,
                    cts: 0,
                }
            })
            .collect();
        Some(TrackRun {
            track_id: AUDIO_TRACK,
            tfdt,
            default_flags: Some(SAMPLE_FLAGS_SYNC),
            samples,
            data,
        })
    }
}

// ──────────────────────────── tests ────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A whole pull as the CDN sends it: header, both configurations, then
    /// `from..to` of interleaved media on the stream's clock.
    pub(crate) fn sample_flv(from: u32, to: u32) -> Vec<u8> {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        bytes.extend(video_config());
        bytes.extend(stream(from, to));
        bytes
    }

    /// A pull carrying HEVC, which must be refused rather than mis-served.
    pub(crate) fn hevc_flv() -> Vec<u8> {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        bytes.extend(tag(9, 0, &[0x1c, 0, 0, 0, 0, 1, 2, 3, 4]));
        bytes
    }

    // The sequence header of a live TikTok pull: High profile, level 3.2,
    // 720x1280, one SPS and one PPS plus the High-profile extension bytes.
    const AVCC: [u8; 50] = [
        0x01, 0x64, 0x00, 0x20, 0xff, 0xe1, 0x00, 0x1e, 0x27, 0x64, 0x00, 0x20, 0xac, 0x13,
        0x1a, 0x48, 0x2d, 0x02, 0x86, 0xc0, 0x5b, 0x80, 0x80, 0x80, 0xa0, 0x00, 0x00, 0x03,
        0x00, 0x20, 0x00, 0x00, 0x0f, 0x01, 0xe2, 0xc4, 0xb2, 0x40, 0x01, 0x00, 0x05, 0x28,
        0xee, 0x02, 0x9c, 0xb0, 0xfd, 0xf8, 0xf8, 0x00,
    ];
    // AAC-LC, 24 kHz, stereo: the configuration measured on a live pull.
    const ASC: [u8; 2] = [0x13, 0x10];

    fn tag(kind: u8, ts: u32, body: &[u8]) -> Vec<u8> {
        let mut t = vec![kind];
        t.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        t.extend_from_slice(&(ts & 0xFF_FFFF).to_be_bytes()[1..]);
        t.push((ts >> 24) as u8);
        t.extend_from_slice(&[0, 0, 0]);
        t.extend_from_slice(body);
        t.extend_from_slice(&((11 + body.len()) as u32).to_be_bytes());
        t
    }

    fn header(audio: bool, video: bool) -> Vec<u8> {
        let flags = (if audio { 4 } else { 0 }) | (if video { 1 } else { 0 });
        let mut h = b"FLV\x01".to_vec();
        h.push(flags);
        h.extend_from_slice(&9u32.to_be_bytes());
        h.extend_from_slice(&0u32.to_be_bytes());
        h
    }

    fn video_config() -> Vec<u8> {
        let mut b = vec![0x17, 0, 0, 0, 0];
        b.extend_from_slice(&AVCC);
        tag(9, 0, &b)
    }

    fn audio_config() -> Vec<u8> {
        let mut b = vec![0xaf, 0];
        b.extend_from_slice(&ASC);
        tag(8, 50, &b)
    }

    fn video(ts: u32, key: bool, cts: i32) -> Vec<u8> {
        let mut b = vec![if key { 0x17 } else { 0x27 }, 1];
        b.extend_from_slice(&(cts as u32 & 0xFF_FFFF).to_be_bytes()[1..]);
        // One length-prefixed NAL unit.
        b.extend_from_slice(&[0, 0, 0, 3, if key { 0x65 } else { 0x41 }, 0xAA, 0xBB]);
        tag(9, ts, &b)
    }

    fn audio(ts: u32) -> Vec<u8> {
        tag(8, ts, &[0xaf, 1, 0x21, 0x10, 0x05])
    }

    /// Everything a muxer puts out for these bytes, fed in awkward pieces.
    fn run(bytes: &[u8], chunk: usize) -> Vec<Output> {
        let mut d = Demuxer::default();
        let mut m = Muxer::new();
        let mut out = Vec::new();
        for piece in bytes.chunks(chunk) {
            for f in d.push(piece).expect("demux") {
                out.extend(m.push(f).expect("mux"));
            }
        }
        out
    }

    fn segments(out: &[Output]) -> Vec<&Segment> {
        out.iter()
            .filter_map(|o| match o {
                Output::Segment(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    fn has_box(bytes: &[u8], kind: &[u8; 4]) -> bool {
        bytes.windows(4).any(|w| w == kind)
    }

    /// The millisecond an AAC frame lands on at 24 kHz: 1024 samples is
    /// 42.67 ms, so FLV's rounded timestamps step 43, 43, 42.
    fn audio_ms(from: u32, i: u32) -> u32 {
        from + (i * 128 + 1) / 3
    }

    /// Frames every 40 ms with a keyframe every 2 s, and audio at the real
    /// 24 kHz cadence, from `from` to `to` on the stream's own clock.
    fn stream(from: u32, to: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut v, mut ai) = (from, 0);
        loop {
            let a = audio_ms(from, ai);
            if a >= to && v >= to {
                break;
            }
            if a <= v && a < to {
                out.extend(audio(a));
                ai += 1;
            } else {
                out.extend(video(v, (v - from) % 2000 == 0, 80));
                v += 40;
            }
        }
        out
    }

    #[test]
    fn tags_split_anywhere_come_out_whole() {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        bytes.extend(video_config());
        bytes.extend(video(1_000, true, 80));
        bytes.extend(audio(1_010));
        let mut d = Demuxer::default();
        let mut frames = Vec::new();
        for b in &bytes {
            frames.extend(d.push(std::slice::from_ref(b)).expect("demux"));
        }
        assert_eq!(frames[0], Frame::Header { audio: true, video: true });
        assert_eq!(frames[1], Frame::AudioConfig(ASC.to_vec()));
        assert_eq!(frames[2], Frame::VideoConfig(AVCC.to_vec()));
        assert!(matches!(frames[3], Frame::Video { dts: 1_000, cts: 80, key: true, .. }));
        assert!(matches!(frames[4], Frame::Audio { ts: 1_010, .. }));
        assert_eq!(frames.len(), 5);
    }

    #[test]
    fn the_extended_timestamp_byte_is_the_top_eight_bits() {
        let mut bytes = header(false, true);
        bytes.extend(video(0x0123_4567, true, 0));
        let frames = Demuxer::default().push(&bytes).expect("demux");
        assert!(matches!(frames[1], Frame::Video { dts: 0x0123_4567, .. }), "{:?}", frames[1]);
    }

    #[test]
    fn a_negative_composition_offset_survives() {
        let mut bytes = header(false, true);
        bytes.extend(video(5_000, false, -40));
        let frames = Demuxer::default().push(&bytes).expect("demux");
        assert!(matches!(frames[1], Frame::Video { cts: -40, .. }));
    }

    #[test]
    fn what_it_cannot_remux_is_refused_by_name() {
        assert!(Demuxer::default().push(b"<html>not a stream").is_err());

        let mut hevc = header(false, true);
        hevc.extend(tag(9, 0, &[0x1c, 1, 0, 0, 0, 1, 2, 3]));
        let e = Demuxer::default().push(&hevc).unwrap_err().to_string();
        assert!(e.contains("not H.264"), "{e}");

        let mut enhanced = header(false, true);
        enhanced.extend(tag(9, 0, &[0x90, b'h', b'v', b'c', b'1']));
        assert!(Demuxer::default().push(&enhanced).is_err());

        let mut mp3 = header(true, false);
        mp3.extend(tag(8, 0, &[0x2f, 1, 2, 3]));
        let e = Demuxer::default().push(&mp3).unwrap_err().to_string();
        assert!(e.contains("not AAC"), "{e}");
    }

    #[test]
    fn a_corrupt_tag_length_is_an_error_not_a_buffer() {
        let mut bytes = header(false, true);
        bytes.extend_from_slice(&[9, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0]);
        assert!(Demuxer::default().push(&bytes).is_err());
    }

    #[test]
    fn the_audio_configuration_reads_rate_and_channels() {
        let a = AudioInfo::parse(&ASC).expect("asc");
        assert_eq!((a.object_type, a.sample_rate, a.channels), (2, 24_000, 2));
        assert_eq!(a.codec(), "mp4a.40.2");
        let b = AudioInfo::parse(&[0x12, 0x10]).expect("asc");
        assert_eq!((b.sample_rate, b.channels), (44_100, 2));
        let c = AudioInfo::parse(&[0x11, 0x90]).expect("asc");
        assert_eq!((c.sample_rate, c.channels), (48_000, 2));
        assert!(AudioInfo::parse(&[]).is_err());
    }

    #[test]
    fn the_video_configuration_names_its_codec_and_geometry() {
        let v = VideoInfo::parse(&AVCC).expect("avcc");
        assert_eq!(v.codec(), "avc1.640020");
        assert_eq!((v.width, v.height), (720, 1280));
        assert!(VideoInfo::parse(&[1, 2, 3]).is_err());
    }

    #[test]
    fn segments_are_a_second_long_and_carry_both_tracks() {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        bytes.extend(video_config());
        bytes.extend(stream(6_000_000, 6_006_100));
        let out = run(&bytes, 777);

        let Output::Init { epoch, bytes: init, codecs, width, height } = &out[0] else {
            panic!("the init comes first");
        };
        assert_eq!(*epoch, 0);
        assert_eq!(codecs, "avc1.640020,mp4a.40.2");
        assert_eq!((*width, *height), (Some(720), Some(1280)));
        for b in [b"ftyp", b"moov", b"avc1", b"avcC", b"mp4a", b"esds", b"mvex"] {
            assert!(has_box(init, b), "init lacks {}", String::from_utf8_lossy(b));
        }

        let segs = segments(&out);
        assert_eq!(segs.len(), 6, "six seconds closed, as one second segments");
        for (i, s) in segs.iter().enumerate() {
            assert_eq!(s.number, i as u64);
            assert!((s.duration - 1.0).abs() < 1e-9, "{}", s.duration);
            assert_eq!(s.video_frames, 25);
            assert!(!s.discontinuity);
            assert_eq!(&s.bytes[4..8], b"moof");
            assert!(has_box(&s.bytes, b"mdat"));
            // Two second GOPs: every other segment opens on the keyframe, and
            // the first always does, since that is where a player starts.
            assert_eq!(opens_on_keyframe(&s.bytes), i % 2 == 0, "segment {i}");
        }
    }

    /// Whether the first video sample of a fragment is a sync sample. The video
    /// run is written first and carries per-sample flags.
    fn opens_on_keyframe(seg: &[u8]) -> bool {
        let at = seg.windows(4).position(|w| w == b"trun").expect("trun");
        // type, version+flags, count, data offset, then sample 0: duration,
        // size, flags.
        let p = at + 4 + 4 + 4 + 4 + 4 + 4;
        u32::from_be_bytes(seg[p..p + 4].try_into().unwrap()) == SAMPLE_FLAGS_SYNC
    }

    #[test]
    fn a_keyframe_early_in_a_segment_does_not_cut_it_short() {
        // A keyframe every 400 ms: one only cuts once the open segment is half
        // a second long, so no segment comes out shorter than that.
        let mut bytes = header(false, true);
        bytes.extend(video_config());
        for i in 0..60u32 {
            let ts = 1_000 + i * 40;
            bytes.extend(video(ts, (i * 40) % 400 == 0, 0));
        }
        let segs_out = run(&bytes, 999);
        let segs = segments(&segs_out);
        assert!(!segs.is_empty());
        for s in &segs {
            assert!(s.duration >= 0.5 - 1e-9, "a {} s segment", s.duration);
            assert!(s.duration <= 1.0 + 1e-9, "a {} s segment", s.duration);
        }
    }

    #[test]
    fn the_audio_clock_counts_frames_rather_than_milliseconds() {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        bytes.extend(video_config());
        bytes.extend(stream(0, 6_100));
        let out = run(&bytes, 4096);
        let segs = segments(&out);
        // tfdt of the audio traf in segment two must equal segment one's first
        // tfdt plus 1024 per frame written in segment one, with no seam.
        let tfdts: Vec<u64> = segs.iter().map(|s| audio_tfdt(&s.bytes)).collect();
        let counts: Vec<u64> = segs.iter().map(|s| audio_samples(&s.bytes)).collect();
        assert!(counts[0] > 20, "{counts:?}");
        assert_eq!(tfdts[1], tfdts[0] + counts[0] * AAC_FRAME);
        assert_eq!(tfdts[2], tfdts[1] + counts[1] * AAC_FRAME);
    }

    /// The second traf's tfdt (audio follows video in each fragment).
    fn audio_tfdt(seg: &[u8]) -> u64 {
        let at: Vec<usize> = seg.windows(4).enumerate().filter(|(_, w)| *w == b"tfdt").map(|(i, _)| i).collect();
        let p = at[1] + 8; // past type, version and flags
        u64::from_be_bytes(seg[p..p + 8].try_into().unwrap())
    }

    fn audio_samples(seg: &[u8]) -> u64 {
        let at: Vec<usize> = seg.windows(4).enumerate().filter(|(_, w)| *w == b"trun").map(|(i, _)| i).collect();
        let p = at[1] + 8;
        u32::from_be_bytes(seg[p..p + 4].try_into().unwrap()) as u64
    }

    #[test]
    fn a_reconnect_replaying_the_cache_adds_nothing_twice() {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        bytes.extend(video_config());
        bytes.extend(stream(10_000, 13_000));
        let mut d = Demuxer::default();
        let mut m = Muxer::new();
        let mut out = Vec::new();
        for f in d.push(&bytes).unwrap() {
            out.extend(m.push(f).unwrap());
        }
        // A new connection: fresh header and configs, then the cached GOP from
        // the keyframe at 12 000, then new frames.
        let mut again = header(true, true);
        again.extend(audio_config());
        again.extend(video_config());
        again.extend(stream(12_000, 16_100));
        let mut d2 = Demuxer::default();
        for f in d2.push(&again).unwrap() {
            out.extend(m.push(f).unwrap());
        }
        let inits = out.iter().filter(|o| matches!(o, Output::Init { .. })).count();
        assert_eq!(inits, 1, "identical configuration, same init");
        let segs = segments(&out);
        let frames: u32 = segs.iter().map(|s| s.video_frames).sum();
        assert_eq!(frames, 150, "10 000 to 16 000 once, at 25 fps");
        assert!(segs.iter().all(|s| !s.discontinuity));
        assert_eq!(segs.iter().map(|s| s.number).collect::<Vec<_>>(), (0..6).collect::<Vec<u64>>());
    }

    #[test]
    fn a_new_clock_is_a_new_timeline() {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        bytes.extend(video_config());
        bytes.extend(stream(6_000_000, 6_004_100));
        // The encoder restarts and its clock begins again near zero.
        bytes.extend(stream(0, 4_100));
        let out = run(&bytes, 1000);
        let segs = segments(&out);
        let first_new = segs.iter().position(|s| s.discontinuity).expect("a marked break");
        assert!(first_new >= 2, "the old timeline's GOPs come out first: {first_new}");
        assert_eq!(segs.iter().filter(|s| s.discontinuity).count(), 1);
        // Numbering carries on across the break.
        for (i, s) in segs.iter().enumerate() {
            assert_eq!(s.number, i as u64);
        }
    }

    #[test]
    fn a_changed_configuration_gets_a_new_init_and_epoch() {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        bytes.extend(video_config());
        bytes.extend(stream(0, 4_100));
        let mut other = AVCC;
        other[3] = 0x1f; // level 3.1
        let mut b = vec![0x17, 0, 0, 0, 0];
        b.extend_from_slice(&other);
        bytes.extend(tag(9, 0, &b));
        bytes.extend(stream(4_100, 8_200));
        let out = run(&bytes, 512);
        let epochs: Vec<u32> = out
            .iter()
            .filter_map(|o| match o {
                Output::Init { epoch, .. } => Some(*epoch),
                _ => None,
            })
            .collect();
        assert_eq!(epochs, vec![0, 1]);
        let segs = segments(&out);
        let first_new = segs.iter().position(|s| s.epoch == 1).expect("segments under the new init");
        assert!(segs[first_new].discontinuity, "a new init is always a new timeline");
    }

    #[test]
    fn an_audio_only_stream_is_cut_by_duration() {
        let mut bytes = header(true, false);
        bytes.extend(audio_config());
        for i in 0..200 {
            bytes.extend(audio(audio_ms(500_000, i)));
        }
        let out = run(&bytes, 333);
        let Output::Init { bytes: init, codecs, width, .. } = &out[0] else {
            panic!("init first");
        };
        assert_eq!(codecs, "mp4a.40.2");
        assert!(width.is_none());
        assert!(!has_box(init, b"avc1"), "no video track in an audio-only init");
        assert!(!has_box(init, b"vide"), "no video handler in an audio-only init");
        assert!(has_box(init, b"soun"));
        let segs = segments(&out);
        assert!(segs.len() >= 3, "{}", segs.len());
        for s in &segs {
            assert!((0.9..1.2).contains(&s.duration), "{}", s.duration);
            assert_eq!(s.video_frames, 0);
        }
    }

    /// The order a live pull actually opens in: the audio configuration, then
    /// every cached audio frame, THEN the video configuration and the cached
    /// GOP from its keyframe, then live interleaving.
    #[test]
    fn the_cache_replay_keeps_its_audio_and_starts_it_at_the_keyframe() {
        let mut bytes = header(true, true);
        bytes.extend(audio_config());
        let mut ai = 0;
        while audio_ms(9_000, ai) < 11_600 {
            bytes.extend(audio(audio_ms(9_000, ai)));
            ai += 1;
        }
        bytes.extend(video_config());
        let mut v = 9_500;
        while v < 13_600 {
            bytes.extend(video(v, (v - 9_500) % 2000 == 0, 0));
            if audio_ms(9_000, ai) < v + 600 {
                bytes.extend(audio(audio_ms(9_000, ai)));
                ai += 1;
            }
            v += 40;
        }
        let out = run(&bytes, 2048);
        let segs = segments(&out);
        assert!(!segs.is_empty());
        // The first segment has its audio: the frames that arrived before the
        // video configuration were held, not thrown away.
        assert!(audio_samples(&segs[0].bytes) >= 20, "{}", audio_samples(&segs[0].bytes));
        // And none of it from before the keyframe.
        let first_audio = audio_tfdt(&segs[0].bytes);
        assert!(first_audio >= 9_500 * 24, "audio at {} ms precedes the keyframe", first_audio / 24);
        assert!(first_audio < 9_550 * 24, "audio starts late, at {} ms", first_audio / 24);
    }

    /// Against a real capture: `SN_FLV_SAMPLE=<path to a .flv>`.
    #[test]
    #[ignore = "needs a captured FLV; set SN_FLV_SAMPLE"]
    fn remuxes_a_real_capture() {
        let path = std::env::var("SN_FLV_SAMPLE").expect("SN_FLV_SAMPLE");
        let bytes = std::fs::read(path).expect("read sample");
        let out = run(&bytes, 64 * 1024);
        let inits: Vec<&String> = out
            .iter()
            .filter_map(|o| match o {
                Output::Init { codecs, .. } => Some(codecs),
                _ => None,
            })
            .collect();
        println!("inits: {inits:?}");
        let segs = segments(&out);
        for s in &segs {
            println!("seg {} dur {:.3} frames {} bytes {} disc {}", s.number, s.duration, s.video_frames, s.bytes.len(), s.discontinuity);
        }
        assert_eq!(inits.len(), 1);
        assert!(!segs.is_empty());
        assert!(segs.iter().all(|s| !s.discontinuity));

        // `SN_FLV_OUT=<path>` writes init + segments as one fragmented MP4, so
        // a real decoder can be pointed at the result.
        if let Ok(dest) = std::env::var("SN_FLV_OUT") {
            let mut file = Vec::new();
            for o in &out {
                match o {
                    Output::Init { bytes, .. } => file.extend_from_slice(bytes),
                    Output::Segment(s) => file.extend_from_slice(&s.bytes),
                }
            }
            std::fs::write(dest, file).expect("write remuxed output");
        }
    }
}
