//! MPEG-TS to fragmented-MP4 transmuxer for the low-latency origin.
//!
//! Why this exists: hls.js demuxes MPEG-TS in JavaScript and rewrites sample
//! timestamps statefully as it appends (each fragment is chained onto the end of
//! the timeline built so far). That makes appends NON-IDEMPOTENT: when hls.js
//! decides to re-fetch a fragment it already buffered, the re-appended video
//! lands AFTER the buffered end (a 2s duplicate insert) while the audio
//! coalesces in place, permanently forking the A/V timelines. Proven live
//! 2026-06-11 (streamdatabase capture): avGap stepped 0 to -14s in seven exact
//! -2s steps, one per whole-fragment re-fetch, with byte-identical part hashes.
//! Serving fMP4 removes that entire failure class: sample timestamps are
//! explicit in the bytes (`tfdt`), the browser places samples by timestamp, and
//! a duplicate append overwrites the same time range. Idempotent by
//! construction, whatever still triggers the re-fetch.
//!
//! Scope: exactly the streams the TS origin sees (Twitch H.264 "chunked"
//! quality: one H.264 video PES stream, access-unit aligned, plus one ADTS AAC
//! audio PES stream). This is not a general-purpose TS demuxer.
//!
//! Timing model: video track timescale is the native 90 kHz PES clock; audio
//! track timescale is the AAC sample rate so every frame is exactly 1024 ticks.
//! 33-bit PES timestamps are unwrapped against the previous value (closest
//! representation wins), so the ~26.5h rollover never folds the timeline. Each
//! fragment carries a 64-bit `tfdt`, which is what makes placement absolute.
//!
//! Mid-stream encoder changes (server-side ad splices, broadcaster
//! reconfigures) are survivable by design: the sample entry is `avc3` with
//! SPS/PPS kept in-band, so the decoder switches parameter sets when the bytes
//! do. And the output is ONE seamless timeline by invariant: every input-clock
//! discontinuity beyond the stream's own frame cadence — splice restarts,
//! encoder gaps, abandoned segment tails — is bridged onto the working cadence
//! (see `SPLICE_TOLERANCE_MIN`), audio riding the same offset for A/V sync.
//! The playlist's declared durations cannot express holes, so an unbridged gap
//! would desync hls.js's playlist arithmetic from the buffer. Only a window
//! rebuild re-baselines instead (its skip IS mirrored in the playlist).
//!
//! Audio `tfdt` follows the running sample count, not each fragment's measured
//! PES PTS. AAC is gapless by construction, but re-deriving a fragment's start
//! from the quantized 90 kHz measurement (through a truncating rescale) lands a
//! few ticks off where the previous fragment ended, and MSE renders every such
//! sub-frame seam as a gap or overwrite at the splice point (audible pop, and
//! a wild measurement can push a sample far enough out of range to error the
//! media element). The measurement only re-anchors the timeline when it
//! disagrees by more than one frame, which is a genuine splice.

use log::{info, warn};

const TS_PACKET: usize = 188;
const TS_SYNC: u8 = 0x47;
/// 90 kHz ticks for one frame of 60 fps video, the fallback duration for the
/// final sample of a fragment (its real duration is only known from the NEXT
/// access unit, which belongs to the next fragment).
const DEFAULT_VIDEO_DUR: u64 = 1500;
const ADTS_SAMPLE_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];
/// Audio re-anchor threshold: one AAC frame in the audio track's timescale.
/// Within a frame, the measured PES PTS is quantization jitter and the running
/// sample count wins; beyond it, the input genuinely jumped (ad splice, encoder
/// restart) and the measurement wins.
const AUDIO_RESYNC_TICKS: u64 = 1024;
/// Cap on filling an audio gap with silent frames; a longer jump is a genuine
/// splice and re-anchors instead. ~3s at 48 kHz.
const SILENT_FILL_MAX_FRAMES: u64 = 144;

/// Canned silent AAC-LC frame (1024 zero samples) by channel configuration.
/// An audio hole inside the muxed buffer blocks the media element at the hole
/// even when video is buffered ahead (observed as stalls with ~0.6s of forward
/// buffer), so small gaps left by a bridged discontinuity (abandoned tail,
/// splice skew) are filled with a beat of silence instead.
fn silent_aac_frame(channel_config: u8) -> Option<&'static [u8]> {
    match channel_config {
        1 => Some(&[0x00, 0xC8, 0x00, 0x80, 0x23, 0x80]),
        2 => Some(&[0x21, 0x00, 0x49, 0x90, 0x02, 0x19, 0x00, 0x23, 0x80]),
        _ => None,
    }
}
/// Minimum bridge tolerance (90 kHz ticks) on the video decode clock; the
/// working tolerance is `max(this, 2 * last_video_delta)` so low-framerate
/// streams (whose normal frame spacing is large) are never "bridged" into
/// fast-forward. Deviations beyond the tolerance — backward OR forward — are
/// bridged onto the working cadence. Forward gaps are NOT passed through as
/// elapsed time: the playlist's declared durations cannot express a hole, so
/// any unbridged PTS gap re-opens the playlist-vs-buffer drift that desyncs
/// hls.js's fragment lookup (and surfaces as bufferSeekOverHole blips). The
/// latency cost of compressing a gap is absorbed by the catch-up governor.
const SPLICE_TOLERANCE_MIN: u64 = 22_500;
/// Upper bound on a believable frame interval (1 fps) for cadence tracking.
const MAX_FRAME_DELTA: u64 = 90_000;

// ──────────────────────────── demuxed sample types ────────────────────────────

struct VideoSample {
    /// Unwrapped 90 kHz decode timestamp.
    dts: u64,
    /// Unwrapped 90 kHz presentation timestamp (>= dts except for malformed input).
    pts: u64,
    keyframe: bool,
    /// AVCC payload (4-byte length-prefixed NAL units, AUD stripped; SPS/PPS
    /// kept in-band per the avc3 sample-entry contract).
    bytes: Vec<u8>,
}

struct AudioFrame {
    /// Unwrapped 90 kHz presentation timestamp.
    pts: u64,
    /// Raw AAC frame (ADTS header stripped).
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct AacConfig {
    /// MPEG-4 audio object type (ADTS profile + 1, e.g. 2 = AAC-LC).
    object_type: u8,
    sampling_index: u8,
    sample_rate: u32,
    channel_config: u8,
}

// ──────────────────────────── PES assembly ────────────────────────────

/// Collects the payload bytes of one PES packet across TS packets. Twitch video
/// PES carry length 0 (unbounded), so a PES is only complete when the NEXT
/// payload-unit-start arrives on the same PID; the final PES of a segment
/// therefore completes from the first packet of the following segment, shifting
/// one access unit into the next fragment (harmless: placement is by `tfdt`).
#[derive(Default)]
struct PesAssembler {
    buf: Vec<u8>,
    pts: Option<u64>,
    dts: Option<u64>,
    started: bool,
    /// True for the video PES, whose elementary stream is NAL-framed. Twitch's
    /// TS does not align PES packet boundaries to access-unit boundaries: a new
    /// video PES can open with the tail of the previous access unit (raw slice
    /// bytes, no start code) before the next AU's first NAL. When set, `start`
    /// carries that prefix back onto the PES being completed so the previous
    /// frame keeps its tail. Off for audio, whose ADTS framing self-syncs.
    nal_framed: bool,
    /// New-PES bytes accumulated while locating the next access unit's first
    /// NAL start code, when it did not fall in the PES's opening packet. Until
    /// the start code is found these bytes are still the previous AU's tail.
    carry: Vec<u8>,
    carry_pts: Option<u64>,
    carry_dts: Option<u64>,
    carrying: bool,
}

struct PesPacket {
    pts: Option<u64>,
    dts: Option<u64>,
    payload: Vec<u8>,
}

impl PesAssembler {
    /// Start a new PES from a payload-unit-start packet's payload. Returns the
    /// previous access unit, now complete — though for video that completion may
    /// instead happen later in `continuation`, when the next AU's start code
    /// lands in a continuation packet (see `carry`).
    fn start(&mut self, payload: &[u8]) -> Option<PesPacket> {
        // PES header: start code (3) + stream_id (1) + packet_length (2) +
        // flags (2) + header_data_length (1), then optional fields.
        if payload.len() < 9 || payload[0] != 0 || payload[1] != 0 || payload[2] != 1 {
            let done = self.take();
            self.started = false;
            return done;
        }
        let pts_dts_flags = (payload[7] >> 6) & 0x3;
        let header_len = payload[8] as usize;
        let body = 9 + header_len;
        let mut pts = None;
        let mut dts = None;
        if pts_dts_flags >= 2 && payload.len() >= 14 {
            pts = Some(read_ts33(&payload[9..14]));
            if pts_dts_flags == 3 && payload.len() >= 19 {
                dts = Some(read_ts33(&payload[14..19]));
            }
        }
        let es: &[u8] = payload.get(body..).unwrap_or(&[]);

        if !self.nal_framed {
            // Audio: ADTS self-syncs, so one PES is one self-contained run.
            let done = self.take();
            self.buf.clear();
            self.buf.extend_from_slice(es);
            self.pts = pts;
            self.dts = dts;
            self.started = true;
            return done;
        }

        // A new PES opened while still mid-carry (the prior boundary never
        // resolved): fold the carry back as tail and continue.
        if self.carrying {
            let carry = std::mem::take(&mut self.carry);
            self.buf.extend_from_slice(&carry);
            self.carrying = false;
            self.carry_pts = None;
            self.carry_dts = None;
        }

        if !self.started {
            // No AU in progress: begin at the first start code, dropping any
            // leading partial bytes from a mid-stream join.
            let split = first_nal_start_code(es).unwrap_or(0);
            self.buf.clear();
            self.buf.extend_from_slice(&es[split..]);
            self.pts = pts;
            self.dts = dts;
            self.started = true;
            return None;
        }

        // Twitch's TS splits PES packets without regard to access-unit
        // boundaries, so the new PES can open with the previous frame's slice
        // tail (raw NAL bytes, no start code) before the next AU's first NAL.
        match first_nal_start_code(es) {
            Some(split) => {
                // Boundary visible in the opening packet: carry the tail back,
                // complete the previous AU, open the next one here.
                self.buf.extend_from_slice(&es[..split]);
                let done = self.take();
                self.buf.clear();
                self.buf.extend_from_slice(&es[split..]);
                self.pts = pts;
                self.dts = dts;
                self.started = true;
                done
            }
            None => {
                // Boundary not yet visible: stash the new PES timing and keep
                // searching across continuation packets. The previous AU stays
                // open until `continuation` finds the start code.
                self.carry.clear();
                self.carry.extend_from_slice(es);
                self.carry_pts = pts;
                self.carry_dts = dts;
                self.carrying = true;
                None
            }
        }
    }

    /// Append a continuation packet. Returns a completed AU when a deferred
    /// boundary (see `start`) resolves in this packet.
    fn continuation(&mut self, payload: &[u8]) -> Option<PesPacket> {
        if !self.started {
            return None;
        }
        if !self.carrying {
            self.buf.extend_from_slice(payload);
            return None;
        }
        // Still locating the next AU's first start code inside the carried
        // bytes. The carry is the previous AU's tail (mid-NAL, emulation-
        // prevented, so it holds no spurious start code) up to where the next
        // AU begins. Search from just before the prior end so a start code
        // straddling the packet seam is still caught.
        let from = self.carry.len().saturating_sub(3);
        self.carry.extend_from_slice(payload);
        let split = from + first_nal_start_code(&self.carry[from..])?;
        let next_au = self.carry.split_off(split); // carry now = previous AU tail
        self.buf.extend_from_slice(&self.carry);
        let done = PesPacket {
            pts: self.pts.take(),
            dts: self.dts.take(),
            payload: std::mem::take(&mut self.buf),
        };
        self.buf.extend_from_slice(&next_au);
        self.pts = self.carry_pts.take();
        self.dts = self.carry_dts.take();
        self.carry.clear();
        self.carrying = false;
        Some(done)
    }

    fn take(&mut self) -> Option<PesPacket> {
        self.carrying = false;
        self.carry.clear();
        if !self.started || self.buf.is_empty() {
            return None;
        }
        self.started = false;
        Some(PesPacket {
            pts: self.pts.take(),
            dts: self.dts.take(),
            payload: std::mem::take(&mut self.buf),
        })
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.pts = None;
        self.dts = None;
        self.started = false;
        self.carry.clear();
        self.carry_pts = None;
        self.carry_dts = None;
        self.carrying = false;
    }
}

/// 33-bit PES timestamp packed across 5 bytes with marker bits (ISO 13818-1).
fn read_ts33(b: &[u8]) -> u64 {
    (((b[0] as u64 >> 1) & 0x07) << 30)
        | ((b[1] as u64) << 22)
        | (((b[2] as u64 >> 1) & 0x7F) << 15)
        | ((b[3] as u64) << 7)
        | ((b[4] as u64 >> 1) & 0x7F)
}

/// Unwrap a 33-bit timestamp to the representation closest to `prev` (the
/// standard MPEG rollover rule). Real discontinuities (ad splices) pass through
/// as timeline jumps, same as the raw TS would show them.
fn unwrap33(prev: Option<u64>, raw: u64) -> u64 {
    const WRAP: u64 = 1 << 33;
    let Some(prev) = prev else { return raw };
    let base = (prev / WRAP) * WRAP;
    let mut best = base + raw;
    for cand in [
        base.checked_sub(WRAP).map(|b| b + raw),
        Some(base + WRAP + raw),
    ]
    .into_iter()
    .flatten()
    {
        if cand.abs_diff(prev) < best.abs_diff(prev) {
            best = cand;
        }
    }
    best
}

// ──────────────────────────── the transmuxer ────────────────────────────

pub struct Transmuxer {
    pmt_pid: Option<u16>,
    video_pid: Option<u16>,
    audio_pid: Option<u16>,
    video_pes: PesAssembler,
    audio_pes: PesAssembler,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    aac: Option<AacConfig>,
    /// Last unwrapped video DTS / audio PTS, the rollover anchors.
    video_clock: Option<u64>,
    audio_clock: Option<u64>,
    /// Where the next fragment's first audio sample must land (audio-timescale
    /// ticks) for gapless playback. None until the first audio fragment.
    next_audio_dts: Option<u64>,
    /// Input-to-output shift (90 kHz) bridging splice clock restarts; applied
    /// to both tracks. 0 until a splice is seen.
    splice_offset: i64,
    /// Last OUTPUT video DTS and the working frame cadence, the bridge
    /// expectation. `last_video_out` is None after `reset_assembly` so a window
    /// rebuild's forward skip re-baselines instead of bridging (the playlist
    /// mirrors that skip as a media-sequence jump, unlike in-segment gaps).
    last_video_out: Option<u64>,
    /// Last INPUT video DTS, the cadence measuring stick (output is bridged).
    last_video_in: Option<u64>,
    last_video_delta: u64,
    /// Outlier frame interval awaiting confirmation by a repeat (see the
    /// cadence-tracking comment in `video_sample`).
    pending_delta: Option<u64>,
    /// Set by `drop_partial_input`: the next video sample bridges onto the
    /// cadence unconditionally (the dropped tail is known lost media; even a
    /// sub-tolerance gap must not become a served hole).
    force_bridge_next: bool,
    pending_video: Vec<VideoSample>,
    pending_audio: Vec<AudioFrame>,
    /// moof sequence number (informational; placement is by tfdt).
    seq: u32,
    init: Option<Vec<u8>>,
    /// Track set is locked when the init segment is built; audio frames that
    /// appear after a video-only init are dropped (the init can't be re-issued).
    init_has_audio: Option<bool>,
    warned_late_audio: bool,
    warned_audio_config: bool,
}

impl Transmuxer {
    pub fn new() -> Self {
        Self {
            pmt_pid: None,
            video_pid: None,
            audio_pid: None,
            video_pes: PesAssembler {
                nal_framed: true,
                ..Default::default()
            },
            audio_pes: PesAssembler::default(),
            sps: None,
            pps: None,
            aac: None,
            video_clock: None,
            audio_clock: None,
            next_audio_dts: None,
            splice_offset: 0,
            last_video_out: None,
            last_video_in: None,
            last_video_delta: DEFAULT_VIDEO_DUR,
            pending_delta: None,
            force_bridge_next: false,
            pending_video: Vec::new(),
            pending_audio: Vec::new(),
            seq: 0,
            init: None,
            init_has_audio: None,
            warned_late_audio: false,
            warned_audio_config: false,
        }
    }

    /// Demux one TS part and emit it as one CMAF part (`moof`+`mdat`) plus its
    /// measured media duration in seconds. Returns None when the part yielded
    /// no complete samples (caller skips publishing it). Parts must be fed in
    /// stream order; PES state carries across calls on purpose (PES packets
    /// straddle part and segment cuts).
    ///
    /// The duration is the sample span actually written into the fragment
    /// (video DTS deltas; audio frame count when a part has no video). DTS is
    /// monotonic and uniform regardless of B-frame reordering, so summing
    /// these durations tiles the timeline exactly — unlike presentation-
    /// timestamp deltas measured at arbitrary byte-stream cut points, which
    /// B-frame arrival order inflates.
    pub fn push_part(&mut self, ts: &[u8]) -> Option<(Vec<u8>, f64)> {
        self.demux(ts);
        self.emit_fragment()
    }

    /// Build (once) the init segment. None until an SPS+PPS pair has been seen,
    /// which on Twitch TS happens in the first part of the first segment.
    pub fn init_segment(&mut self) -> Option<Vec<u8>> {
        if self.init.is_none() {
            let sps = self.sps.clone()?;
            let pps = self.pps.clone()?;
            let dims = parse_sps_dimensions(&sps)?;
            self.init_has_audio = Some(self.aac.is_some());
            self.init = Some(build_init(&sps, &pps, dims, self.aac));
        }
        self.init.clone()
    }

    /// Drop any half-assembled PES and undrained samples. Called when the
    /// origin rebuilds its window across a gap: completing a pre-gap PES with
    /// post-gap continuation bytes would emit one garbage sample.
    pub fn reset_assembly(&mut self) {
        self.video_pes.reset();
        self.audio_pes.reset();
        self.pending_video.clear();
        self.pending_audio.clear();
        // The window jumped; expecting gapless audio continuation across it
        // would glue unrelated frames together, and the bridge expectation must
        // re-baseline so the skip passes through as real elapsed time (the
        // offset itself persists: it maps this encoder's clock to the output).
        self.next_audio_dts = None;
        self.last_video_out = None;
        self.last_video_in = None;
        self.pending_delta = None;
        self.force_bridge_next = false;
    }

    /// Drop half-assembled PES state WITHOUT touching the timeline
    /// expectations. For an ABANDONED segment tail (mid-body stall, the reader
    /// moved on): the playlist stays sn-contiguous, so the lost media must be
    /// BRIDGED by the next segment (`last_video_out` kept = the gap exceeds
    /// tolerance and snaps onto the cadence), unlike a window rebuild where the
    /// playlist itself jumps. Without this distinction every abandoned tail
    /// re-opened playlist-vs-buffer drift as a served hole (bufferSeekOverHole
    /// blips, observed 2026-06-12).
    pub fn drop_partial_input(&mut self) {
        self.video_pes.reset();
        self.audio_pes.reset();
        self.pending_video.clear();
        self.pending_audio.clear();
        // The dropped tail is KNOWN lost media, not organic stream behavior:
        // the next sample must land exactly on the cadence even when the gap
        // is under the bridge tolerance. Sub-tolerance abandon gaps otherwise
        // leak through as real buffer holes (133ms observed live) and each one
        // permanently shifts hls.js's playlist arithmetic off the buffer by
        // its size — the accumulating "next unloaded part starts at +0.5s"
        // misalignment.
        self.force_bridge_next = true;
    }

    // ──────────────── demux ────────────────

    fn demux(&mut self, data: &[u8]) {
        let mut i = 0;
        while i + TS_PACKET <= data.len() {
            if data[i] != TS_SYNC {
                match data[i + 1..].iter().position(|&b| b == TS_SYNC) {
                    Some(off) => {
                        i += 1 + off;
                        continue;
                    }
                    None => break,
                }
            }
            self.packet(&data[i..i + TS_PACKET]);
            i += TS_PACKET;
        }
    }

    fn packet(&mut self, pkt: &[u8]) {
        let pusi = pkt[1] & 0x40 != 0;
        let pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        let adaptation = (pkt[3] >> 4) & 0x3;
        if adaptation == 0 || adaptation == 2 {
            return; // no payload
        }
        let mut p = 4;
        if adaptation == 3 {
            p = 5 + pkt[4] as usize;
            if p >= TS_PACKET {
                return;
            }
        }
        let payload = &pkt[p..];

        if pid == 0 {
            if pusi {
                self.parse_pat(payload);
            }
            return;
        }
        if Some(pid) == self.pmt_pid {
            if pusi {
                self.parse_pmt(payload);
            }
            return;
        }
        if Some(pid) == self.video_pid {
            if pusi {
                if let Some(pes) = self.video_pes.start(payload) {
                    self.video_sample(pes);
                }
            } else if let Some(pes) = self.video_pes.continuation(payload) {
                self.video_sample(pes);
            }
        } else if Some(pid) == self.audio_pid {
            if pusi {
                if let Some(pes) = self.audio_pes.start(payload) {
                    self.audio_frames(pes);
                }
            } else if let Some(pes) = self.audio_pes.continuation(payload) {
                self.audio_frames(pes);
            }
        }
    }

    /// PSI sections here (PAT/PMT) are tiny and always fit one packet.
    fn parse_pat(&mut self, payload: &[u8]) {
        let Some(section) = psi_section(payload) else {
            return;
        };
        // Entries: program_number (2) + PID (2), first non-zero program wins.
        let mut i = 8;
        while i + 4 <= section.len() {
            let program = ((section[i] as u16) << 8) | section[i + 1] as u16;
            let pid = (((section[i + 2] & 0x1F) as u16) << 8) | section[i + 3] as u16;
            if program != 0 {
                self.pmt_pid = Some(pid);
                return;
            }
            i += 4;
        }
    }

    fn parse_pmt(&mut self, payload: &[u8]) {
        let Some(section) = psi_section(payload) else {
            return;
        };
        if section.len() < 12 {
            return;
        }
        let program_info_len = (((section[10] & 0x0F) as usize) << 8) | section[11] as usize;
        let mut i = 12 + program_info_len;
        while i + 5 <= section.len() {
            let stream_type = section[i];
            let pid = (((section[i + 1] & 0x1F) as u16) << 8) | section[i + 2] as u16;
            let es_info_len = (((section[i + 3] & 0x0F) as usize) << 8) | section[i + 4] as usize;
            match stream_type {
                0x1B => self.video_pid = Some(pid), // H.264
                0x0F => self.audio_pid = Some(pid), // AAC in ADTS
                _ => {}
            }
            i += 5 + es_info_len;
        }
    }

    fn video_sample(&mut self, pes: PesPacket) {
        let Some(raw_pts) = pes.pts else { return };
        // DTS defaults to PTS (no B-frame reordering); both unwrap against the
        // same video clock so a PTS/DTS pair never lands on opposite sides of a
        // rollover.
        let dts_in = unwrap33(self.video_clock, pes.dts.unwrap_or(raw_pts));
        let pts_in = unwrap33(Some(dts_in), raw_pts);
        self.video_clock = Some(dts_in);

        // Timeline bridging: served output must be ONE seamless timeline that
        // tiles with the declared part durations. Any input-clock discontinuity
        // beyond the cadence tolerance — an encoder-change splice's arbitrary
        // restart, an encoder gap, an abandoned segment tail — is bridged onto
        // the working cadence; audio rides the same offset so A/V sync within
        // the new program is preserved. (Window rebuilds are the one exception:
        // `reset_assembly` clears the expectation so the skip re-baselines.)
        // Cadence tracking feeds the expectation, the tolerance scale, and the
        // final-sample duration estimate. Measured from INPUT deltas (output is
        // bridged, so it would be circular). A delta far above the working
        // cadence is held as a candidate and accepted only when the next delta
        // confirms it: a genuinely low-framerate stream establishes its real
        // cadence after one frame, while a one-off gap (abandoned tail, encoder
        // hiccup) stays an outlier and gets bridged.
        if let Some(prev_in) = self.last_video_in {
            let d = dts_in.saturating_sub(prev_in);
            if d > 0 && d <= MAX_FRAME_DELTA {
                // Accept: near the working cadence, or a repeat of the pending
                // outlier (a real cadence change). Otherwise hold as pending.
                if d <= self.last_video_delta.saturating_mul(2)
                    || self.pending_delta.is_some_and(|p| d.abs_diff(p) <= p / 4)
                {
                    self.last_video_delta = d;
                    self.pending_delta = None;
                } else {
                    self.pending_delta = Some(d);
                }
            }
        }
        self.last_video_in = Some(dts_in);

        let mut out = dts_in as i128 + self.splice_offset as i128;
        if let Some(prev) = self.last_video_out {
            let expected = (prev + self.last_video_delta) as i128;
            let tolerance = SPLICE_TOLERANCE_MIN.max(2 * self.last_video_delta) as i128;
            if self.force_bridge_next || (out - expected).abs() > tolerance {
                info!(
                    "[TsFmp4] video timeline discontinuity: bridging input dts {dts_in} to output {expected}"
                );
                self.splice_offset += (expected - out) as i64;
                out = expected;
            }
        }
        self.force_bridge_next = false;
        let dts = out.max(0) as u64;
        let pts = (pts_in as i128 + self.splice_offset as i128).max(0) as u64;
        self.last_video_out = Some(dts);

        let mut bytes = Vec::with_capacity(pes.payload.len());
        let mut keyframe = false;
        for nal in annexb_nals(&pes.payload) {
            match nal[0] & 0x1F {
                // SPS/PPS stay IN the sample (the avc3 contract): the decoder
                // picks up mid-stream parameter changes from the bytes. The
                // first-seen pair is also captured for the init segment.
                7 => {
                    if self.sps.is_none() {
                        self.sps = Some(nal.to_vec());
                    }
                    bytes.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    bytes.extend_from_slice(nal);
                }
                8 => {
                    if self.pps.is_none() {
                        self.pps = Some(nal.to_vec());
                    }
                    bytes.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    bytes.extend_from_slice(nal);
                }
                9 => {} // access unit delimiter: implicit in the sample framing
                t => {
                    if t == 5 {
                        keyframe = true;
                    }
                    bytes.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    bytes.extend_from_slice(nal);
                }
            }
        }
        if !bytes.is_empty() {
            self.pending_video.push(VideoSample {
                dts,
                pts,
                keyframe,
                bytes,
            });
        }
    }

    fn audio_frames(&mut self, pes: PesPacket) {
        let Some(raw_pts) = pes.pts else { return };
        let base_in = unwrap33(self.audio_clock, raw_pts);
        self.audio_clock = Some(base_in);
        // Audio rides the video-anchored splice offset so a bridged program
        // keeps its own A/V alignment; the `next_audio_dts` continuity then
        // absorbs the sub-frame residue at the boundary.
        let base = (base_in as i128 + self.splice_offset as i128).max(0) as u64;
        let mut data = &pes.payload[..];
        let mut n: u64 = 0;
        while data.len() >= 7 {
            if data[0] != 0xFF || data[1] & 0xF0 != 0xF0 {
                // Lost ADTS sync inside a PES: drop the rest of this packet.
                return;
            }
            let profile = (data[2] >> 6) & 0x3;
            let sampling_index = (data[2] >> 2) & 0xF;
            let channel_config = ((data[2] & 0x1) << 2) | ((data[3] >> 6) & 0x3);
            let frame_len = (((data[3] & 0x3) as usize) << 11)
                | ((data[4] as usize) << 3)
                | ((data[5] >> 5) as usize);
            let header_len = if data[1] & 0x1 == 0 { 9 } else { 7 };
            if frame_len < header_len || frame_len > data.len() {
                return; // malformed or truncated frame
            }
            let Some(&rate) = ADTS_SAMPLE_RATES.get(sampling_index as usize) else {
                return;
            };
            let parsed = AacConfig {
                object_type: profile + 1,
                sampling_index,
                sample_rate: rate,
                channel_config,
            };
            match self.aac {
                None => self.aac = Some(parsed),
                // Mid-stream AAC config change (e.g. an ad at a different sample
                // rate or channel count). The init's AudioSpecificConfig is
                // fixed and fMP4 AAC has no in-band reconfig (unlike avc3
                // video), so decoding these frames against it would garble/pop.
                // Drop them: the running clock then silence-fills the short gap
                // or re-anchors a long one, so the worst case is a beat of
                // silence, never a pop. (Proper handling = SourceBuffer
                // changeType + a fresh init; future work tied to the ad path.)
                Some(established) if established != parsed => {
                    if !self.warned_audio_config {
                        warn!(
                            "[TsFmp4] audio config changed mid-stream ({established:?} -> {parsed:?}); dropping mismatched frames to prevent garble"
                        );
                        self.warned_audio_config = true;
                    }
                    return;
                }
                Some(_) => {}
            }
            if self.init_has_audio == Some(false) {
                if !self.warned_late_audio {
                    warn!("[TsFmp4] audio appeared after a video-only init; dropping it");
                    self.warned_late_audio = true;
                }
                return;
            }
            // Frame i of this PES presents at PES PTS + i * 1024 samples,
            // computed per frame from the rational so no rounding accumulates.
            let pts = base + (n * 1024 * 90_000) / rate as u64;
            self.pending_audio.push(AudioFrame {
                pts,
                bytes: data[header_len..frame_len].to_vec(),
            });
            n += 1;
            data = &data[frame_len..];
        }
    }

    // ──────────────── mux ────────────────

    fn emit_fragment(&mut self) -> Option<(Vec<u8>, f64)> {
        let video: Vec<VideoSample> = std::mem::take(&mut self.pending_video);
        let audio: Vec<AudioFrame> = std::mem::take(&mut self.pending_audio);
        if video.is_empty() && audio.is_empty() {
            return None;
        }
        self.seq += 1;

        // Per-sample durations are DTS deltas; the final sample's real duration
        // is unknowable here (its successor is in the next fragment) so it
        // reuses the previous delta. The next fragment's tfdt re-anchors, so the
        // estimate can never accumulate.
        let mut vruns: Vec<TrunSample> = Vec::with_capacity(video.len());
        for (i, s) in video.iter().enumerate() {
            let dur = match video.get(i + 1) {
                Some(next) => next.dts.saturating_sub(s.dts),
                None => vruns
                    .last()
                    .map(|r| r.duration as u64)
                    .unwrap_or(DEFAULT_VIDEO_DUR),
            };
            vruns.push(TrunSample {
                duration: dur.min(u32::MAX as u64) as u32,
                size: s.bytes.len() as u32,
                flags: if s.keyframe {
                    SAMPLE_FLAGS_SYNC
                } else {
                    SAMPLE_FLAGS_NON_SYNC
                },
                cts: (s.pts as i64 - s.dts as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            });
        }

        let aac = self.aac;
        let mut tracks: Vec<TrackRun> = Vec::new();
        if let Some(first) = video.first() {
            tracks.push(TrackRun {
                track_id: 1,
                tfdt: first.dts,
                default_flags: None,
                samples: vruns.clone(),
                data: video.iter().flat_map(|s| s.bytes.iter().copied()).collect(),
            });
        }
        let mut audio_frames_emitted: u64 = 0;
        if let (Some(cfg), Some(first)) = (aac, audio.first()) {
            // Rescaled into the audio track's sample-rate timescale, where
            // every AAC frame is exactly 1024 ticks.
            let measured = rescale(first.pts, 90_000, cfg.sample_rate);
            // Gapless placement: the running sample count owns the timeline
            // while the measurement agrees within a frame; a small forward gap
            // (bridged discontinuity's lost audio) is filled with silence so
            // the muxed buffer never contains an audio hole; anything bigger
            // or backward re-anchors (see module docs).
            let mut lead_silence: u64 = 0;
            let tfdt = match self.next_audio_dts {
                Some(expected) if expected.abs_diff(measured) <= AUDIO_RESYNC_TICKS => expected,
                Some(expected)
                    if measured > expected
                        && (measured - expected) <= SILENT_FILL_MAX_FRAMES * 1024
                        && silent_aac_frame(cfg.channel_config).is_some() =>
                {
                    // Round to whole frames; the residue (≤ half a frame) is
                    // absorbed by the continuity clock.
                    lead_silence = (measured - expected + 512) / 1024;
                    info!("[TsFmp4] audio gap: {lead_silence} silent frames inserted");
                    expected
                }
                Some(expected) => {
                    info!("[TsFmp4] audio timeline re-anchored: expected {expected}, measured {measured}");
                    measured
                }
                None => measured,
            };
            audio_frames_emitted = lead_silence + audio.len() as u64;
            self.next_audio_dts = Some(tfdt + audio_frames_emitted * 1024);
            let mut samples: Vec<TrunSample> =
                Vec::with_capacity(lead_silence as usize + audio.len());
            let mut data: Vec<u8> = Vec::new();
            if lead_silence > 0 {
                let sf = silent_aac_frame(cfg.channel_config).expect("checked in match guard");
                for _ in 0..lead_silence {
                    samples.push(TrunSample {
                        duration: 1024,
                        size: sf.len() as u32,
                        flags: SAMPLE_FLAGS_SYNC,
                        cts: 0,
                    });
                    data.extend_from_slice(sf);
                }
            }
            for f in &audio {
                samples.push(TrunSample {
                    duration: 1024,
                    size: f.bytes.len() as u32,
                    flags: SAMPLE_FLAGS_SYNC,
                    cts: 0,
                });
                data.extend_from_slice(&f.bytes);
            }
            tracks.push(TrackRun {
                track_id: 2,
                tfdt,
                default_flags: Some(SAMPLE_FLAGS_SYNC),
                samples,
                data,
            });
        }

        // The fragment's media span. Video owns the pacing when present (the
        // playlist durations must tile the video timeline); an audio-only part
        // (e.g. a PAT/PMT+audio tail) spans its emitted frame count.
        let duration = if video.is_empty() {
            self.aac
                .map(|cfg| audio_frames_emitted as f64 * 1024.0 / cfg.sample_rate as f64)
                .unwrap_or_default()
        } else {
            vruns.iter().map(|r| r.duration as u64).sum::<u64>() as f64 / 90_000.0
        };
        Some((build_fragment(self.seq, &tracks), duration))
    }
}

impl Default for Transmuxer {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip the pointer field and return one PSI section (from table_id onward).
fn psi_section(payload: &[u8]) -> Option<&[u8]> {
    let pointer = *payload.first()? as usize;
    let start = 1 + pointer;
    if payload.len() <= start + 3 {
        return None;
    }
    let s = &payload[start..];
    let section_len = (((s[1] & 0x0F) as usize) << 8) | s[2] as usize;
    let total = 3 + section_len;
    if s.len() < total {
        return None;
    }
    // Trim the 4-byte CRC so entry loops can run to the slice end.
    Some(&s[..total.saturating_sub(4)])
}

/// Byte offset of the first Annex B start code (`00 00 01`) in `data`, or None.
/// A 4-byte start code (`00 00 00 01`) matches at its trailing three bytes, so
/// the returned offset may leave one leading zero in the carried-back prefix;
/// that zero is a harmless trailing_zero_8bits on the previous NAL.
fn first_nal_start_code(data: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Iterate Annex B NAL units (00 00 01 / 00 00 00 01 start codes).
fn annexb_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    for (idx, &s) in starts.iter().enumerate() {
        let mut end = match starts.get(idx + 1) {
            Some(&next) => next - 3,
            None => data.len(),
        };
        // A 4-byte start code leaves one zero before the next 3-byte match.
        while end > s && data[end - 1] == 0 {
            end -= 1;
        }
        if end > s {
            nals.push(&data[s..end]);
        }
    }
    nals
}

fn rescale(value: u64, from: u32, to: u32) -> u64 {
    ((value as u128) * (to as u128) / (from as u128)) as u64
}

// ──────────────────────────── SPS parsing ────────────────────────────

/// Bit reader over an RBSP (emulation prevention bytes removed).
struct BitReader {
    data: Vec<u8>,
    pos: usize,
}

impl BitReader {
    fn new(nal: &[u8]) -> Self {
        // Strip emulation prevention: 00 00 03 -> 00 00.
        let mut data = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for &b in nal {
            if zeros >= 2 && b == 3 {
                zeros = 0;
                continue;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            data.push(b);
        }
        Self { data, pos: 0 }
    }

    fn bit(&mut self) -> Option<u32> {
        let byte = self.data.get(self.pos / 8)?;
        let bit = (byte >> (7 - (self.pos % 8))) & 1;
        self.pos += 1;
        Some(bit as u32)
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1 << zeros) - 1 + self.bits(zeros)?)
    }

    fn se(&mut self) -> Option<i32> {
        let v = self.ue()?;
        Some(if v % 2 == 0 {
            -((v / 2) as i32)
        } else {
            ((v + 1) / 2) as i32
        })
    }
}

/// Decode coded width/height from an SPS NAL (with its header byte). Only the
/// fields up to the cropping window are read.
fn parse_sps_dimensions(sps: &[u8]) -> Option<(u16, u16)> {
    let mut r = BitReader::new(sps.get(1..)?);
    let profile_idc = r.bits(8)?;
    r.bits(8)?; // constraint flags + reserved
    r.bits(8)?; // level_idc
    r.ue()?; // seq_parameter_set_id

    let mut chroma_format_idc = 1;
    let mut separate_colour_plane = false;
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format_idc = r.ue()?;
        if chroma_format_idc == 3 {
            separate_colour_plane = r.bit()? == 1;
        }
        r.ue()?; // bit_depth_luma_minus8
        r.ue()?; // bit_depth_chroma_minus8
        r.bit()?; // qpprime_y_zero_transform_bypass
        if r.bit()? == 1 {
            // seq_scaling_matrix_present
            let lists = if chroma_format_idc == 3 { 12 } else { 8 };
            for i in 0..lists {
                if r.bit()? == 1 {
                    skip_scaling_list(&mut r, if i < 6 { 16 } else { 64 })?;
                }
            }
        }
    }

    r.ue()?; // log2_max_frame_num_minus4
    let poc_type = r.ue()?;
    if poc_type == 0 {
        r.ue()?; // log2_max_pic_order_cnt_lsb_minus4
    } else if poc_type == 1 {
        r.bit()?; // delta_pic_order_always_zero
        r.se()?; // offset_for_non_ref_pic
        r.se()?; // offset_for_top_to_bottom_field
        let n = r.ue()?;
        for _ in 0..n {
            r.se()?;
        }
    }
    r.ue()?; // max_num_ref_frames
    r.bit()?; // gaps_in_frame_num_value_allowed

    let width_mbs = r.ue()? + 1;
    let height_map_units = r.ue()? + 1;
    let frame_mbs_only = r.bit()?;
    if frame_mbs_only == 0 {
        r.bit()?; // mb_adaptive_frame_field
    }
    r.bit()?; // direct_8x8_inference
    let (mut crop_l, mut crop_r, mut crop_t, mut crop_b) = (0, 0, 0, 0);
    if r.bit()? == 1 {
        crop_l = r.ue()?;
        crop_r = r.ue()?;
        crop_t = r.ue()?;
        crop_b = r.ue()?;
    }

    let chroma_array_type = if separate_colour_plane {
        0
    } else {
        chroma_format_idc
    };
    let (sub_w, sub_h) = match chroma_array_type {
        1 => (2, 2),
        2 => (2, 1),
        3 => (1, 1),
        _ => (1, 1),
    };
    let crop_unit_x = if chroma_array_type == 0 { 1 } else { sub_w };
    let crop_unit_y = (if chroma_array_type == 0 { 1 } else { sub_h }) * (2 - frame_mbs_only);

    let width = width_mbs * 16 - (crop_l + crop_r) * crop_unit_x;
    let height = (2 - frame_mbs_only) * height_map_units * 16 - (crop_t + crop_b) * crop_unit_y;
    Some((width as u16, height as u16))
}

fn skip_scaling_list(r: &mut BitReader, size: u32) -> Option<()> {
    let mut last = 8i32;
    let mut next = 8i32;
    for _ in 0..size {
        if next != 0 {
            next = (last + r.se()? + 256) % 256;
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

// ──────────────────────────── MP4 box building ────────────────────────────

/// is_leading/depends_on/is_depended_on/redundancy/padding/non_sync/priority.
pub(crate) const SAMPLE_FLAGS_SYNC: u32 = 0x0200_0000; // depends on nothing, sync
pub(crate) const SAMPLE_FLAGS_NON_SYNC: u32 = 0x0101_0000; // depends on others, non-sync

#[derive(Clone)]
pub(crate) struct TrunSample {
    pub(crate) duration: u32,
    pub(crate) size: u32,
    pub(crate) flags: u32,
    pub(crate) cts: i32,
}

pub(crate) struct TrackRun {
    pub(crate) track_id: u32,
    /// In this track's own timescale.
    pub(crate) tfdt: u64,
    /// When set, flags live in the tfhd default and the trun omits per-sample
    /// flags (audio: every frame is a sync sample).
    pub(crate) default_flags: Option<u32>,
    pub(crate) samples: Vec<TrunSample>,
    pub(crate) data: Vec<u8>,
}

pub(crate) fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + payload.len());
    b.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    b.extend_from_slice(kind);
    b.extend_from_slice(payload);
    b
}

pub(crate) fn full_box(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + payload.len());
    p.push(version);
    p.extend_from_slice(&flags.to_be_bytes()[1..]);
    p.extend_from_slice(payload);
    mp4_box(kind, &p)
}

pub(crate) fn build_fragment(seq: u32, tracks: &[TrackRun]) -> Vec<u8> {
    // moof layout must be sized before trun data offsets are known, so each
    // trun is written with a placeholder and patched once the moof length is.
    let mfhd = full_box(b"mfhd", 0, 0, &seq.to_be_bytes());
    let mut trafs: Vec<Vec<u8>> = Vec::new();
    let mut offset_positions: Vec<usize> = Vec::new(); // absolute, within moof
    let mut running = 8 + mfhd.len(); // moof header + mfhd

    for t in tracks {
        let mut tfhd_flags = 0x02_0000; // default-base-is-moof
        let mut tfhd_payload = t.track_id.to_be_bytes().to_vec();
        if let Some(f) = t.default_flags {
            tfhd_flags |= 0x20; // default-sample-flags present
            tfhd_payload.extend_from_slice(&f.to_be_bytes());
        }
        let tfhd = full_box(b"tfhd", 0, tfhd_flags, &tfhd_payload);
        let tfdt = full_box(b"tfdt", 1, 0, &t.tfdt.to_be_bytes());

        let per_sample_flags = t.default_flags.is_none();
        let mut trun_flags = 0x000001 | 0x000100 | 0x000200; // offset, duration, size
        if per_sample_flags {
            trun_flags |= 0x000400 | 0x000800; // flags, cts
        }
        let mut trun_payload = Vec::new();
        trun_payload.extend_from_slice(&(t.samples.len() as u32).to_be_bytes());
        let offset_field = trun_payload.len();
        trun_payload.extend_from_slice(&0u32.to_be_bytes()); // patched below
        for s in &t.samples {
            trun_payload.extend_from_slice(&s.duration.to_be_bytes());
            trun_payload.extend_from_slice(&s.size.to_be_bytes());
            if per_sample_flags {
                trun_payload.extend_from_slice(&s.flags.to_be_bytes());
                trun_payload.extend_from_slice(&s.cts.to_be_bytes());
            }
        }
        let trun = full_box(b"trun", 1, trun_flags, &trun_payload);

        // Offset of the trun's data_offset field inside the traf: traf header
        // (8) + tfhd + tfdt + trun header (12 = box header + version/flags).
        let field_in_traf = 8 + tfhd.len() + tfdt.len() + 12 + offset_field;
        offset_positions.push(running + field_in_traf);

        let mut traf_payload = tfhd;
        traf_payload.extend_from_slice(&tfdt);
        traf_payload.extend_from_slice(&trun);
        let traf = mp4_box(b"traf", &traf_payload);
        running += traf.len();
        trafs.push(traf);
    }

    let mut moof_payload = mfhd;
    for t in &trafs {
        moof_payload.extend_from_slice(t);
    }
    let mut moof = mp4_box(b"moof", &moof_payload);
    let moof_len = moof.len();

    // Each track's samples start after the moof and the mdat header, preceded
    // by every earlier track's data.
    let mut data_start = moof_len + 8;
    for (i, t) in tracks.iter().enumerate() {
        let pos = offset_positions[i];
        moof[pos..pos + 4].copy_from_slice(&(data_start as u32).to_be_bytes());
        data_start += t.data.len();
    }

    let total_data: usize = tracks.iter().map(|t| t.data.len()).sum();
    let mut out = Vec::with_capacity(moof_len + 8 + total_data);
    out.extend_from_slice(&moof);
    out.extend_from_slice(&((8 + total_data) as u32).to_be_bytes());
    out.extend_from_slice(b"mdat");
    for t in tracks {
        out.extend_from_slice(&t.data);
    }
    out
}

fn build_init(sps: &[u8], pps: &[u8], dims: (u16, u16), aac: Option<AacConfig>) -> Vec<u8> {
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
    mvhd_payload.extend_from_slice(&3u32.to_be_bytes()); // next_track_ID
    let mvhd = full_box(b"mvhd", 0, 0, &mvhd_payload);

    let video_trak = build_trak(
        1,
        dims,
        90_000,
        &build_avc3(sps, pps, dims),
        b"vide",
        b"VideoHandler\0",
        &full_box(b"vmhd", 0, 1, &[0u8; 8]),
    );

    let mut moov_payload = mvhd;
    moov_payload.extend_from_slice(&video_trak);

    let mut mvex_payload = build_trex(1);
    if let Some(cfg) = aac {
        let audio_trak = build_trak(
            2,
            (0, 0),
            cfg.sample_rate,
            &build_mp4a(cfg),
            b"soun",
            b"SoundHandler\0",
            &full_box(b"smhd", 0, 0, &[0u8; 4]),
        );
        moov_payload.extend_from_slice(&audio_trak);
        mvex_payload.extend_from_slice(&build_trex(2));
    }
    moov_payload.extend_from_slice(&mp4_box(b"mvex", &mvex_payload));

    let mut out = ftyp;
    out.extend_from_slice(&mp4_box(b"moov", &moov_payload));
    out
}

pub(crate) fn unity_matrix() -> [u8; 36] {
    let mut m = [0u8; 36];
    m[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    m[16..20].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    m[32..36].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    m
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_trak(
    track_id: u32,
    dims: (u16, u16),
    timescale: u32,
    sample_entry: &[u8],
    handler: &[u8; 4],
    handler_name: &[u8],
    media_header: &[u8],
) -> Vec<u8> {
    let mut tkhd_payload = Vec::new();
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // creation
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // modification
    tkhd_payload.extend_from_slice(&track_id.to_be_bytes());
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // reserved
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // duration (live)
    tkhd_payload.extend_from_slice(&[0u8; 8]); // reserved
    tkhd_payload.extend_from_slice(&0u16.to_be_bytes()); // layer
    tkhd_payload.extend_from_slice(&0u16.to_be_bytes()); // alternate_group
    tkhd_payload.extend_from_slice(&(if handler == b"soun" { 0x0100u16 } else { 0 }).to_be_bytes());
    tkhd_payload.extend_from_slice(&0u16.to_be_bytes()); // reserved
    tkhd_payload.extend_from_slice(&unity_matrix());
    tkhd_payload.extend_from_slice(&((dims.0 as u32) << 16).to_be_bytes());
    tkhd_payload.extend_from_slice(&((dims.1 as u32) << 16).to_be_bytes());
    let tkhd = full_box(b"tkhd", 0, 3, &tkhd_payload); // enabled + in_movie

    let mut mdhd_payload = Vec::new();
    mdhd_payload.extend_from_slice(&0u32.to_be_bytes()); // creation
    mdhd_payload.extend_from_slice(&0u32.to_be_bytes()); // modification
    mdhd_payload.extend_from_slice(&timescale.to_be_bytes());
    mdhd_payload.extend_from_slice(&0u32.to_be_bytes()); // duration (live)
    mdhd_payload.extend_from_slice(&0x55C4u16.to_be_bytes()); // language "und"
    mdhd_payload.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
    let mdhd = full_box(b"mdhd", 0, 0, &mdhd_payload);

    let mut hdlr_payload = Vec::new();
    hdlr_payload.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
    hdlr_payload.extend_from_slice(handler);
    hdlr_payload.extend_from_slice(&[0u8; 12]); // reserved
    hdlr_payload.extend_from_slice(handler_name);
    let hdlr = full_box(b"hdlr", 0, 0, &hdlr_payload);

    let dref_entry = full_box(b"url ", 0, 1, &[]); // self-contained
    let mut dref_payload = 1u32.to_be_bytes().to_vec();
    dref_payload.extend_from_slice(&dref_entry);
    let dinf = mp4_box(b"dinf", &full_box(b"dref", 0, 0, &dref_payload));

    let mut stsd_payload = 1u32.to_be_bytes().to_vec();
    stsd_payload.extend_from_slice(sample_entry);
    let stsd = full_box(b"stsd", 0, 0, &stsd_payload);
    let mut stbl_payload = stsd;
    stbl_payload.extend_from_slice(&full_box(b"stts", 0, 0, &0u32.to_be_bytes()));
    stbl_payload.extend_from_slice(&full_box(b"stsc", 0, 0, &0u32.to_be_bytes()));
    stbl_payload.extend_from_slice(&full_box(b"stsz", 0, 0, &[0u8; 8]));
    stbl_payload.extend_from_slice(&full_box(b"stco", 0, 0, &0u32.to_be_bytes()));
    let stbl = mp4_box(b"stbl", &stbl_payload);

    let mut minf_payload = media_header.to_vec();
    minf_payload.extend_from_slice(&dinf);
    minf_payload.extend_from_slice(&stbl);
    let minf = mp4_box(b"minf", &minf_payload);

    let mut mdia_payload = mdhd;
    mdia_payload.extend_from_slice(&hdlr);
    mdia_payload.extend_from_slice(&minf);
    let mdia = mp4_box(b"mdia", &mdia_payload);

    let mut trak_payload = tkhd;
    trak_payload.extend_from_slice(&mdia);
    mp4_box(b"trak", &trak_payload)
}

pub(crate) fn build_trex(track_id: u32) -> Vec<u8> {
    let mut p = track_id.to_be_bytes().to_vec();
    p.extend_from_slice(&1u32.to_be_bytes()); // default_sample_description_index
    p.extend_from_slice(&0u32.to_be_bytes()); // default_sample_duration
    p.extend_from_slice(&0u32.to_be_bytes()); // default_sample_size
    p.extend_from_slice(&0u32.to_be_bytes()); // default_sample_flags
    full_box(b"trex", 0, 0, &p)
}

/// `avc3` sample entry, NOT `avc1`: avc3 is the ISO 14496-15 variant whose
/// contract is that parameter sets may travel in-band with the samples, which
/// is what lets a mid-stream encoder change (server-side ad splice, broadcaster
/// reconfigure) decode correctly without re-issuing the init segment. The avcC
/// still carries the first-seen SPS/PPS as the starting config; the decoder
/// switches when an in-band set arrives. hls.js parses avc3 the same as avc1
/// (codec string `avc3.PPCCLL`), and Chromium MSE supports it.
fn build_avc3(sps: &[u8], pps: &[u8], dims: (u16, u16)) -> Vec<u8> {
    let mut avcc = vec![
        1,      // configurationVersion
        sps[1], // AVCProfileIndication
        sps[2], // profile_compatibility
        sps[3], // AVCLevelIndication
        0xFF,   // lengthSizeMinusOne = 3 (4-byte lengths)
        0xE1,   // one SPS
    ];
    avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(sps);
    avcc.push(1); // one PPS
    avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(pps);

    let mut p = Vec::new();
    p.extend_from_slice(&[0u8; 6]); // reserved
    p.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    p.extend_from_slice(&[0u8; 16]); // pre_defined + reserved
    p.extend_from_slice(&dims.0.to_be_bytes());
    p.extend_from_slice(&dims.1.to_be_bytes());
    p.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi horizontal
    p.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi vertical
    p.extend_from_slice(&0u32.to_be_bytes()); // reserved
    p.extend_from_slice(&1u16.to_be_bytes()); // frame_count
    p.extend_from_slice(&[0u8; 32]); // compressorname
    p.extend_from_slice(&0x0018u16.to_be_bytes()); // depth 24
    p.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined
    p.extend_from_slice(&mp4_box(b"avcC", &avcc));
    mp4_box(b"avc3", &p)
}

fn build_mp4a(cfg: AacConfig) -> Vec<u8> {
    // AudioSpecificConfig: 5 bits object type, 4 bits frequency index, 4 bits
    // channel configuration, 3 bits zero.
    let asc: [u8; 2] = [
        (cfg.object_type << 3) | (cfg.sampling_index >> 1),
        ((cfg.sampling_index & 1) << 7) | (cfg.channel_config << 3),
    ];

    // ES descriptor tree with single-byte sizes (every payload here is < 128).
    let dec_specific = [&[0x05u8, asc.len() as u8][..], &asc].concat();
    let mut dec_config = vec![
        0x40, // objectTypeIndication: MPEG-4 audio
        0x15, // streamType audio + upStream 0 + reserved 1
        0, 0, 0, // bufferSizeDB
        0, 0x1F, 0x40, 0x00, // maxBitrate 2 Mbps
        0, 0x1F, 0x40, 0x00, // avgBitrate
    ];
    dec_config.extend_from_slice(&dec_specific);
    let mut es = vec![0u8, 0, 0]; // ES_ID + streamDependence flags
    es.push(0x04);
    es.push(dec_config.len() as u8);
    es.extend_from_slice(&dec_config);
    es.extend_from_slice(&[0x06, 0x01, 0x02]); // SLConfig: MP4 (2)
    let mut esds_payload = vec![0x03u8, es.len() as u8];
    esds_payload.extend_from_slice(&es);
    let esds = full_box(b"esds", 0, 0, &esds_payload);

    let channels = if cfg.channel_config == 7 {
        8
    } else {
        cfg.channel_config as u16
    };
    let mut p = Vec::new();
    p.extend_from_slice(&[0u8; 6]); // reserved
    p.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    p.extend_from_slice(&[0u8; 8]); // reserved
    p.extend_from_slice(&channels.to_be_bytes());
    p.extend_from_slice(&16u16.to_be_bytes()); // samplesize
    p.extend_from_slice(&0u32.to_be_bytes()); // pre_defined + reserved
    p.extend_from_slice(&(cfg.sample_rate << 16).to_be_bytes());
    p.extend_from_slice(&esds);
    mp4_box(b"mp4a", &p)
}

// ──────────────────────────── tests ────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// One PES payload (header + elementary bytes), as `start`/`continuation`
    /// receive it after the TS layer is stripped.
    fn pes_payload(es: &[u8], pts: u64, dts: Option<u64>) -> Vec<u8> {
        let mut p = pes_header(0xE0, pts, dts);
        p.extend_from_slice(es);
        p
    }

    // Twitch's TS splits PES packets without regard to access-unit boundaries,
    // so a new video PES can open with the previous frame's slice tail (raw NAL
    // bytes, no start code) before the next AU's first NAL. The previous frame
    // must keep that tail, with its own timing, or the decoder runs short and
    // corrupts the bottom of the picture.

    #[test]
    fn cross_pes_tail_in_opening_packet_is_carried_back() {
        let mut a = PesAssembler {
            nal_framed: true,
            ..Default::default()
        };
        // AU A: AUD + IDR slice.
        let au_a = annexb(&[&[9, 0xF0][..], &[5, 0xAA, 0xBB]]);
        assert!(a.start(&pes_payload(&au_a, 100, Some(100))).is_none());
        // AU B's PES opens with AU A's slice tail (CC DD EE), then AU B.
        let mut es2 = vec![0xCC, 0xDD, 0xEE];
        es2.extend_from_slice(&annexb(&[&[9, 0xF0][..], &[1, 0x40]]));
        let done = a
            .start(&pes_payload(&es2, 200, Some(200)))
            .expect("AU A completes");
        assert_eq!(done.pts, Some(100), "tail keeps AU A's timing, not AU B's");
        assert!(
            done.payload.windows(3).any(|w| w == [0xCC, 0xDD, 0xEE]),
            "AU A kept its spilled slice tail"
        );
    }

    #[test]
    fn cross_pes_tail_resolves_across_continuation_packet() {
        let mut a = PesAssembler {
            nal_framed: true,
            ..Default::default()
        };
        let au_a = annexb(&[&[9, 0xF0][..], &[5, 0xAA, 0xBB]]);
        assert!(a.start(&pes_payload(&au_a, 100, Some(100))).is_none());
        // AU B's PES opens with ONLY the tail; its AUD lands in a continuation.
        assert!(
            a.start(&pes_payload(&[0xCC, 0xDD, 0xEE], 200, Some(200)))
                .is_none(),
            "boundary deferred until the start code arrives"
        );
        let done = a
            .continuation(&annexb(&[&[9, 0xF0][..], &[1, 0x40]]))
            .expect("AU A completes on the continuation packet");
        assert_eq!(done.pts, Some(100));
        assert!(done.payload.windows(3).any(|w| w == [0xCC, 0xDD, 0xEE]));
    }

    // ──── synthetic TS construction ────

    const VIDEO_PID: u16 = 0x100;
    const AUDIO_PID: u16 = 0x101;
    const PMT_PID: u16 = 0x20;

    fn ts_packet(pid: u16, pusi: bool, cc: u8, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= 184);
        let mut p = vec![0u8; TS_PACKET];
        p[0] = TS_SYNC;
        p[1] = (if pusi { 0x40 } else { 0 }) | ((pid >> 8) as u8 & 0x1F);
        p[2] = (pid & 0xFF) as u8;
        if payload.len() == 184 {
            p[3] = 0x10 | (cc & 0xF); // payload only
            p[4..].copy_from_slice(payload);
        } else {
            // Adaptation field of stuffing pads the payload to the packet end.
            let af_len = 183 - payload.len();
            p[3] = 0x30 | (cc & 0xF);
            p[4] = af_len as u8;
            if af_len > 0 {
                p[5] = 0; // no flags
                for b in &mut p[6..5 + af_len] {
                    *b = 0xFF;
                }
            }
            p[5 + af_len..].copy_from_slice(payload);
        }
        p
    }

    fn psi_packet(pid: u16, table: &[u8]) -> Vec<u8> {
        let mut payload = vec![0u8]; // pointer_field
        payload.extend_from_slice(table);
        ts_packet(pid, true, 0, &payload)
    }

    fn pat() -> Vec<u8> {
        let mut entries = Vec::new();
        entries.extend_from_slice(&1u16.to_be_bytes()); // program 1
        entries.extend_from_slice(&(0xE000 | PMT_PID).to_be_bytes());
        psi_table(0x00, &entries)
    }

    fn pmt() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(0xE000 | VIDEO_PID).to_be_bytes()[..]); // PCR PID
        body.extend_from_slice(&0xF000u16.to_be_bytes()); // program_info_length 0
        for (stream_type, pid) in [(0x1Bu8, VIDEO_PID), (0x0F, AUDIO_PID)] {
            body.push(stream_type);
            body.extend_from_slice(&(0xE000 | pid).to_be_bytes());
            body.extend_from_slice(&0xF000u16.to_be_bytes()); // es_info_length 0
        }
        psi_table(0x02, &body)
    }

    fn psi_table(table_id: u8, body: &[u8]) -> Vec<u8> {
        // header after length: id_ext (2) + version (1) + section numbers (2)
        let section_len = 5 + body.len() + 4; // + CRC
        let mut t = vec![table_id];
        t.extend_from_slice(&(0xB000 | section_len as u16).to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes()); // tsid / program
        t.push(0xC1); // version 0, current
        t.push(0); // section_number
        t.push(0); // last_section_number
        t.extend_from_slice(body);
        t.extend_from_slice(&[0u8; 4]); // CRC (unchecked)
        t
    }

    fn pes_header(stream_id: u8, pts: u64, dts: Option<u64>) -> Vec<u8> {
        let mut h = vec![0, 0, 1, stream_id, 0, 0];
        let (flags, hdr_len) = match dts {
            Some(_) => (0xC0u8, 10u8),
            None => (0x80, 5),
        };
        h.push(0x80); // marker bits
        h.push(flags);
        h.push(hdr_len);
        h.extend_from_slice(&encode_ts33(if dts.is_some() { 0x3 } else { 0x2 }, pts));
        if let Some(d) = dts {
            h.extend_from_slice(&encode_ts33(0x1, d));
        }
        h
    }

    fn encode_ts33(prefix: u8, ts: u64) -> [u8; 5] {
        [
            (prefix << 4) | (((ts >> 30) as u8 & 0x07) << 1) | 1,
            (ts >> 22) as u8,
            (((ts >> 15) as u8 & 0x7F) << 1) | 1,
            (ts >> 7) as u8,
            ((ts as u8 & 0x7F) << 1) | 1,
        ]
    }

    /// 1280x720, Main profile: frame_mbs_only, 80x45 macroblocks, no cropping.
    fn test_sps() -> Vec<u8> {
        let mut w = BitWriter::new(vec![0x67, 77, 0, 31]); // NAL header + profile/constraints/level
        w.ue(0); // sps_id
        w.ue(4); // log2_max_frame_num_minus4
        w.ue(0); // pic_order_cnt_type
        w.ue(4); // log2_max_pic_order_cnt_lsb_minus4
        w.ue(3); // max_num_ref_frames
        w.bit(0); // gaps_in_frame_num
        w.ue(79); // pic_width_in_mbs_minus1
        w.ue(44); // pic_height_in_map_units_minus1
        w.bit(1); // frame_mbs_only
        w.bit(1); // direct_8x8_inference
        w.bit(0); // frame_cropping
        w.bit(0); // vui_parameters_present
        w.stop_bit();
        w.finish()
    }

    struct BitWriter {
        out: Vec<u8>,
        acc: u8,
        n: u8,
    }

    impl BitWriter {
        fn new(prefix: Vec<u8>) -> Self {
            Self {
                out: prefix,
                acc: 0,
                n: 0,
            }
        }
        fn bit(&mut self, b: u8) {
            self.acc = (self.acc << 1) | (b & 1);
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc);
                self.acc = 0;
                self.n = 0;
            }
        }
        fn ue(&mut self, v: u32) {
            let x = v + 1;
            let len = 32 - x.leading_zeros();
            for _ in 0..len - 1 {
                self.bit(0);
            }
            for i in (0..len).rev() {
                self.bit(((x >> i) & 1) as u8);
            }
        }
        fn stop_bit(&mut self) {
            self.bit(1);
        }
        fn finish(mut self) -> Vec<u8> {
            while self.n != 0 {
                self.bit(0);
            }
            self.out
        }
    }

    fn annexb(nals: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for n in nals {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(n);
        }
        out
    }

    fn adts_frame(payload_len: usize) -> Vec<u8> {
        let frame_len = 7 + payload_len;
        let mut f = vec![
            0xFF,
            0xF1,                                       // MPEG-4, no CRC
            (1 << 6) | (3 << 2),                        // AAC-LC (profile 1), 48 kHz (index 3)
            (2 << 6) | ((frame_len >> 11) as u8 & 0x3), // stereo
            (frame_len >> 3) as u8,
            ((frame_len as u8 & 0x7) << 5) | 0x1F,
            0xFC,
        ];
        f.extend(std::iter::repeat_n(0xABu8, payload_len));
        f
    }

    /// Spread one PES across as many TS packets as it needs.
    fn pes_packets(pid: u16, es: &[u8], pts: u64, dts: Option<u64>, cc: &mut u8) -> Vec<u8> {
        let mut full = pes_header(if pid == VIDEO_PID { 0xE0 } else { 0xC0 }, pts, dts);
        full.extend_from_slice(es);
        let mut out = Vec::new();
        let mut first = true;
        for chunk in full.chunks(184) {
            out.extend_from_slice(&ts_packet(pid, first, *cc, chunk));
            *cc = (*cc + 1) & 0xF;
            first = false;
        }
        out
    }

    /// A 4-frame segment: PAT/PMT, then AUs at 90k-tick cadence `step` starting
    /// at `pts0` (SPS/PPS/IDR on the first AU), with one stereo audio PES.
    fn test_segment(pts0: u64, step: u64) -> Vec<u8> {
        let sps = test_sps();
        let pps = vec![0x68, 0xCE, 0x3C, 0x80];
        let mut ts = Vec::new();
        ts.extend_from_slice(&psi_packet(0, &pat()));
        ts.extend_from_slice(&psi_packet(PMT_PID, &pmt()));
        let mut vcc = 0u8;
        let mut acc = 0u8;
        for i in 0..4u64 {
            let es = if i == 0 {
                annexb(&[
                    &[9, 0xF0][..],
                    &sps,
                    &pps,
                    &[5, 0x88, 0x80, 0x10],
                    &[6, 1, 2, 3],
                ])
            } else {
                annexb(&[&[9, 0xF0][..], &[1, 0x9A, 0x40 + i as u8, 0x22]])
            };
            let pts = pts0 + i * step;
            ts.extend_from_slice(&pes_packets(VIDEO_PID, &es, pts, Some(pts), &mut vcc));
            if i == 0 {
                let mut audio = adts_frame(40);
                audio.extend_from_slice(&adts_frame(42));
                ts.extend_from_slice(&pes_packets(AUDIO_PID, &audio, pts, None, &mut acc));
            }
        }
        ts
    }

    // ──── box walking ────

    fn walk(data: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 8 <= data.len() {
            let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
            let kind = String::from_utf8_lossy(&data[i + 4..i + 8]).to_string();
            assert!(
                size >= 8 && i + size <= data.len(),
                "bad box {kind} size {size}"
            );
            out.push((kind, data[i + 8..i + size].to_vec()));
            i += size;
        }
        assert_eq!(i, data.len(), "trailing bytes after last box");
        out
    }

    fn find<'a>(boxes: &'a [(String, Vec<u8>)], kind: &str) -> &'a [u8] {
        &boxes
            .iter()
            .find(|(k, _)| k == kind)
            .unwrap_or_else(|| panic!("missing {kind}"))
            .1
    }

    // ──── the tests ────

    #[test]
    fn sps_dimensions_parse() {
        assert_eq!(parse_sps_dimensions(&test_sps()), Some((1280, 720)));
    }

    #[test]
    fn transmux_produces_valid_init_and_fragment() {
        let mut t = Transmuxer::new();
        // Two segments so the dangling final PES of segment 1 completes.
        let (frag1, _) = t
            .push_part(&test_segment(90_000, 1500))
            .expect("samples in push 1");
        let (frag2, _) = t
            .push_part(&test_segment(96_000, 1500))
            .expect("samples in push 2");
        let init = t.init_segment().expect("init after SPS/PPS seen");

        // Init: ftyp + moov with two traks and two trexes.
        let top = walk(&init);
        assert_eq!(top[0].0, "ftyp");
        let moov = walk(find(&top, "moov"));
        assert_eq!(moov.iter().filter(|(k, _)| k == "trak").count(), 2);
        let mvex = walk(find(&moov, "mvex"));
        assert_eq!(mvex.iter().filter(|(k, _)| k == "trex").count(), 2);
        // avcC carries the SPS bytes verbatim, inside an avc3 sample entry
        // (in-band parameter sets allowed; survives mid-stream encoder changes).
        let init_str = init.windows(test_sps().len()).any(|w| w == test_sps());
        assert!(init_str, "SPS embedded in avcC");
        assert!(init.windows(4).any(|w| w == b"avc3"), "avc3 sample entry");
        assert!(
            !init.windows(4).any(|w| w == b"avc1"),
            "no avc1 sample entry"
        );

        // Fragment 1: moof + mdat, video traf has 3 complete AUs. The 4th video
        // PES and the lone audio PES of segment 1 only complete when segment 2
        // delivers the next PUSI on their PIDs (the fixture has ONE audio PES
        // per segment; real Twitch TS has many, so audio lags by far less).
        let frag = walk(&frag1);
        assert_eq!(frag[0].0, "moof");
        assert_eq!(frag[1].0, "mdat");
        let moof = walk(&frag[0].1);
        let trafs: Vec<_> = moof.iter().filter(|(k, _)| k == "traf").collect();
        assert_eq!(
            trafs.len(),
            1,
            "video only: the single audio PES is still open"
        );
        let v_traf = walk(&trafs[0].1);
        let v_tfdt = find(&v_traf, "tfdt");
        assert_eq!(v_tfdt[0], 1, "64-bit tfdt");
        assert_eq!(
            u64::from_be_bytes(v_tfdt[4..12].try_into().unwrap()),
            90_000
        );
        let v_trun = find(&v_traf, "trun");
        assert_eq!(u32::from_be_bytes(v_trun[4..8].try_into().unwrap()), 3);

        // Fragment 2 starts with the video AU that completed late (pts0 + 3
        // steps) and now carries segment 1's audio: 2 frames anchored at the
        // segment-1 PES PTS, rescaled into the 48 kHz track timescale.
        let moof2 = walk(&walk(&frag2)[0].1);
        let trafs2: Vec<_> = moof2.iter().filter(|(k, _)| k == "traf").collect();
        assert_eq!(trafs2.len(), 2);
        let v2 = walk(&trafs2[0].1);
        let tfdt2 = u64::from_be_bytes(find(&v2, "tfdt")[4..12].try_into().unwrap());
        assert_eq!(tfdt2, 90_000 + 3 * 1500);
        let a_traf = walk(&trafs2[1].1);
        let a_trun = find(&a_traf, "trun");
        assert_eq!(u32::from_be_bytes(a_trun[4..8].try_into().unwrap()), 2);
        let a_tfdt = find(&a_traf, "tfdt");
        assert_eq!(
            u64::from_be_bytes(a_tfdt[4..12].try_into().unwrap()),
            48_000
        );
    }

    #[test]
    fn trun_data_offsets_point_at_track_data() {
        let mut t = Transmuxer::new();
        let (frag, _) = t.push_part(&test_segment(0, 1500)).expect("samples");
        let boxes = walk(&frag);
        let moof_len = 8 + boxes[0].1.len();
        let moof = walk(&boxes[0].1);
        let trafs: Vec<_> = moof.iter().filter(|(k, _)| k == "traf").collect();
        let v = walk(&trafs[0].1);
        let v_trun = find(&v, "trun");
        let v_off = u32::from_be_bytes(v_trun[8..12].try_into().unwrap()) as usize;
        assert_eq!(v_off, moof_len + 8, "video data starts at mdat payload");
        // The first NAL length prefix at that offset is sane (AUD stripped,
        // SPS kept in-band, so the first sample leads with the fixture SPS).
        let nal_len = u32::from_be_bytes(frag[v_off..v_off + 4].try_into().unwrap());
        assert_eq!(nal_len, test_sps().len() as u32);
    }

    #[test]
    fn deterministic_output() {
        let seg = test_segment(123_456, 1500);
        let mut a = Transmuxer::new();
        let mut b = Transmuxer::new();
        assert_eq!(a.push_part(&seg), b.push_part(&seg));
        assert_eq!(a.init_segment(), b.init_segment());
    }

    #[test]
    fn pts_rollover_unwraps_monotonically() {
        const WRAP: u64 = 1 << 33;
        let mut t = Transmuxer::new();
        let _ = t.push_part(&test_segment(WRAP - 3000, 1500)); // last AU wraps past 2^33
        let (frag, _) = t
            .push_part(&test_segment(1500, 1500))
            .expect("post-wrap samples");
        let moof = walk(&walk(&frag)[0].1);
        let trafs: Vec<_> = moof.iter().filter(|(k, _)| k == "traf").collect();
        let v = walk(&trafs[0].1);
        let tfdt = u64::from_be_bytes(find(&v, "tfdt")[4..12].try_into().unwrap());
        // The dangling 4th AU of segment 1 had raw pts (2^33 - 3000) + 4500,
        // which wrapped to 1500; unwrapped it must continue past 2^33.
        assert_eq!(tfdt, WRAP + 1500);
    }

    #[test]
    fn unwrap33_chooses_nearest() {
        const WRAP: u64 = 1 << 33;
        assert_eq!(unwrap33(None, 42), 42);
        assert_eq!(unwrap33(Some(WRAP - 100), 50), WRAP + 50); // forward across wrap
        assert_eq!(unwrap33(Some(WRAP + 50), WRAP - 100), WRAP - 100); // small backstep
        assert_eq!(unwrap33(Some(2 * WRAP + 10), 5), 2 * WRAP + 5);
    }

    #[test]
    fn empty_or_garbage_input_yields_no_fragment() {
        let mut t = Transmuxer::new();
        assert!(t.push_part(&[]).is_none());
        assert!(t.push_part(&[0u8; 188]).is_none());
        assert!(t.init_segment().is_none());
    }

    #[test]
    fn reset_assembly_drops_partial_pes() {
        let mut t = Transmuxer::new();
        let _ = t.push_part(&test_segment(90_000, 1500));
        t.reset_assembly();
        // Continuing with a fresh segment must not glue the old dangling PES
        // onto the new stream: the first fragment after reset starts at the new
        // segment's own first COMPLETED AU (its first PES terminates when the
        // second arrives, so 3 of its 4 AUs complete within the push).
        let (frag, _) = t
            .push_part(&test_segment(900_000, 1500))
            .expect("post-reset samples");
        let moof = walk(&walk(&frag)[0].1);
        let trafs: Vec<_> = moof.iter().filter(|(k, _)| k == "traf").collect();
        let v = walk(&trafs[0].1);
        let tfdt = u64::from_be_bytes(find(&v, "tfdt")[4..12].try_into().unwrap());
        assert_eq!(tfdt, 900_000);
    }

    /// PAT/PMT plus one audio PES carrying two ADTS frames (2048 audio ticks =
    /// 3840 ticks at 90 kHz between gapless PES starts).
    fn audio_segment(pts: u64) -> Vec<u8> {
        let mut ts = Vec::new();
        ts.extend_from_slice(&psi_packet(0, &pat()));
        ts.extend_from_slice(&psi_packet(PMT_PID, &pmt()));
        let mut acc = 0u8;
        let mut audio = adts_frame(40);
        audio.extend_from_slice(&adts_frame(42));
        ts.extend_from_slice(&pes_packets(AUDIO_PID, &audio, pts, None, &mut acc));
        ts
    }

    /// An ADTS frame with a chosen sampling_index and channel_config (for the
    /// mid-stream config-change guard test). Mirrors `adts_frame` otherwise.
    fn adts_frame_cfg(payload_len: usize, sampling_index: u8, channel_config: u8) -> Vec<u8> {
        let frame_len = 7 + payload_len;
        let mut f = vec![
            0xFF,
            0xF1,
            (1 << 6) | (sampling_index << 2) | ((channel_config >> 2) & 1),
            ((channel_config & 0x3) << 6) | ((frame_len >> 11) as u8 & 0x3),
            (frame_len >> 3) as u8,
            ((frame_len as u8 & 0x7) << 5) | 0x1F,
            0xFC,
        ];
        f.extend(std::iter::repeat_n(0xABu8, payload_len));
        f
    }

    fn audio_segment_cfg(pts: u64, sampling_index: u8, channel_config: u8) -> Vec<u8> {
        let mut ts = Vec::new();
        ts.extend_from_slice(&psi_packet(0, &pat()));
        ts.extend_from_slice(&psi_packet(PMT_PID, &pmt()));
        let mut acc = 0u8;
        let mut audio = adts_frame_cfg(40, sampling_index, channel_config);
        audio.extend_from_slice(&adts_frame_cfg(42, sampling_index, channel_config));
        ts.extend_from_slice(&pes_packets(AUDIO_PID, &audio, pts, None, &mut acc));
        ts
    }

    #[test]
    fn mid_stream_audio_config_change_is_dropped_not_garbled() {
        // First config (48 kHz stereo) establishes the init's AudioSpecificConfig.
        // A later PES at a DIFFERENT config (44.1 kHz mono) must be dropped, not
        // emitted against the wrong config (which would garble/pop).
        let mut t = Transmuxer::new();
        assert!(t.push_part(&audio_segment_cfg(90_000, 3, 2)).is_none()); // open PES
        let (a, _) = t.push_part(&audio_segment_cfg(93_840, 3, 2)).unwrap(); // matching -> emitted
        let moof = walk(&walk(&a)[0].1);
        assert_eq!(
            moof.iter().filter(|(k, _)| k == "traf").count(),
            1,
            "matching audio emitted"
        );

        // Now a mismatched-config PES arrives. It completes the prior matching
        // PES (still 48k stereo, fine), but the NEW 44.1k-mono frames must not
        // be emitted on the next completion.
        let _ = t.push_part(&audio_segment_cfg(97_680, 4, 1)); // index 4 = 44.1k, mono
        let after = t.push_part(&audio_segment_cfg(101_520, 4, 1));
        // The mismatched frames were dropped: the completing PES yields no audio
        // samples, so the fragment is empty (None) rather than garbled audio.
        assert!(
            after.is_none(),
            "mismatched-config audio dropped, not emitted"
        );
    }

    fn audio_tfdt(frag: &[u8]) -> u64 {
        let moof = walk(&walk(frag)[0].1);
        let trafs: Vec<_> = moof.iter().filter(|(k, _)| k == "traf").collect();
        assert_eq!(trafs.len(), 1, "audio-only fixture: exactly one traf");
        let a = walk(&trafs[0].1);
        u64::from_be_bytes(find(&a, "tfdt")[4..12].try_into().unwrap())
    }

    #[test]
    fn audio_tfdt_stays_gapless_across_fragments() {
        // PES PTS carries quantization jitter around the gapless 3840-tick
        // cadence; placement must follow the running sample count, not the
        // jitter, or every fragment boundary gets a sub-frame seam (audible
        // pop). A PES only completes when the next one starts, so each push
        // emits the PREVIOUS push's frames.
        let mut t = Transmuxer::new();
        assert!(t.push_part(&audio_segment(90_000)).is_none());
        let (a, _) = t.push_part(&audio_segment(93_870)).unwrap(); // +3840 +30 jitter
        assert_eq!(
            audio_tfdt(&a),
            48_000,
            "first fragment anchors at the measurement"
        );
        let (b, _) = t.push_part(&audio_segment(97_655)).unwrap(); // +3840 -55 jitter
        assert_eq!(
            audio_tfdt(&b),
            50_048,
            "16-tick early measurement coalesced"
        );
        let (c, _) = t.push_part(&audio_segment(9_000_000)).unwrap(); // a real splice
        assert_eq!(
            audio_tfdt(&c),
            52_096,
            "expectation chains, not measured 52082"
        );
        let (d, _) = t.push_part(&audio_segment(9_003_840)).unwrap();
        assert_eq!(
            audio_tfdt(&d),
            4_800_000,
            "a splice re-anchors at the measurement"
        );
    }

    /// PAT/PMT plus four video AUs whose PES arrive in DECODE order with
    /// B-frame presentation reordering: display order I B B P, so the PES
    /// stream carries presentation timestamps 0, +3, +1, +2 frames while the
    /// decode timestamps tick uniformly.
    fn bframe_segment(dts0: u64) -> Vec<u8> {
        let sps = test_sps();
        let pps = vec![0x68, 0xCE, 0x3C, 0x80];
        let mut ts = Vec::new();
        ts.extend_from_slice(&psi_packet(0, &pat()));
        ts.extend_from_slice(&psi_packet(PMT_PID, &pmt()));
        let mut vcc = 0u8;
        const CTS_FRAMES: [u64; 4] = [0, 3, 1, 2];
        for i in 0..4u64 {
            let es = if i == 0 {
                annexb(&[&[9, 0xF0][..], &sps, &pps, &[5, 0x88, 0x80, 0x10]])
            } else {
                annexb(&[&[9, 0xF0][..], &[1, 0x9A, 0x40 + i as u8, 0x22]])
            };
            let dts = dts0 + i * 1500;
            let pts = dts0 + CTS_FRAMES[i as usize] * 1500;
            ts.extend_from_slice(&pes_packets(VIDEO_PID, &es, pts, Some(dts), &mut vcc));
        }
        ts
    }

    #[test]
    fn duration_follows_dts_cadence_despite_pts_reorder() {
        // The published part duration must measure the decode-timestamp span.
        // Presentation-timestamp deltas at the part cuts read 3 frames ahead
        // here (the forward reference); summing those is exactly the playlist
        // duration inflation that desynced hls.js's fragment lookup from the
        // buffer on B-frame channels.
        let mut t = Transmuxer::new();
        let (_, d1) = t
            .push_part(&bframe_segment(90_000))
            .expect("3 AUs complete");
        assert!((d1 - 4500.0 / 90_000.0).abs() < 1e-9, "3 frames, got {d1}");
        let (_, d2) = t
            .push_part(&bframe_segment(96_000))
            .expect("4 AUs complete");
        // The dangling 4th AU of segment 1 plus 3 of segment 2.
        assert!((d2 - 6000.0 / 90_000.0).abs() < 1e-9, "4 frames, got {d2}");
    }

    fn video_tfdt(frag: &[u8]) -> u64 {
        let moof = walk(&walk(frag)[0].1);
        let trafs: Vec<_> = moof.iter().filter(|(k, _)| k == "traf").collect();
        let v = walk(&trafs[0].1);
        u64::from_be_bytes(find(&v, "tfdt")[4..12].try_into().unwrap())
    }

    #[test]
    fn splice_clock_restart_bridges_to_seamless_output() {
        // A server-side splice restarts the PES clock at an arbitrary base.
        // Output must stay one seamless timeline: forward restarts beyond
        // SPLICE_FWD and any backward restart bridge onto the working cadence.
        let mut t = Transmuxer::new();
        let _ = t.push_part(&test_segment(90_000, 1500)); // output dts 90000..93000
        let (frag2, d2) = t
            .push_part(&test_segment(9_000_000, 1500))
            .expect("samples");
        // Fragment 2 leads with the dangling pre-splice AU (94500); the new
        // program's clock (9_000_000) bridges to the next cadence slot (96000).
        assert_eq!(video_tfdt(&frag2), 94_500);
        assert!(
            (d2 - 6000.0 / 90_000.0).abs() < 1e-9,
            "4 frames despite the jump, got {d2}"
        );
        // A later BACKWARD restart bridges the same way.
        let (frag3, _) = t.push_part(&test_segment(1_500, 1500)).expect("samples");
        assert_eq!(
            video_tfdt(&frag3),
            100_500,
            "output timeline continues unbroken"
        );
    }

    #[test]
    fn audio_gap_filled_with_silent_frames() {
        // A bridged discontinuity can lose a beat of audio. The muxed buffer
        // must never contain an audio hole (the element stalls AT the hole
        // even with video buffered ahead), so a small forward gap is filled
        // with canned silent frames and the timeline stays gapless.
        let mut t = Transmuxer::new();
        assert!(t.push_part(&audio_segment(90_000)).is_none());
        let (a, _) = t.push_part(&audio_segment(93_840)).unwrap(); // gapless +2 frames
        assert_eq!(audio_tfdt(&a), 48_000);
        let (b, _) = t.push_part(&audio_segment(107_280)).unwrap(); // still gapless
        assert_eq!(audio_tfdt(&b), 50_048);
        // PES3 (pts 107280) measures 5 frames past the expectation: filled.
        let (c, _) = t.push_part(&audio_segment(111_120)).unwrap();
        assert_eq!(
            audio_tfdt(&c),
            52_096,
            "gap start stays on the running clock"
        );
        let moof = walk(&walk(&c)[0].1);
        let trafs: Vec<_> = moof.iter().filter(|(k, _)| k == "traf").collect();
        let a_traf = walk(&trafs[0].1);
        let a_trun = find(&a_traf, "trun");
        let count = u32::from_be_bytes(a_trun[4..8].try_into().unwrap());
        assert_eq!(count, 7, "5 silent frames + 2 real ones");
    }

    #[test]
    fn abandon_bridges_even_sub_tolerance_gaps() {
        // After drop_partial_input (an abandoned segment tail), the next
        // sample must land exactly on the cadence even when the lost tail is
        // SMALLER than the discontinuity tolerance — a 133ms hole observed
        // live was leaking through and shifting hls.js's playlist arithmetic
        // off the buffer by its size at every abandon.
        let mut t = Transmuxer::new();
        let _ = t.push_part(&test_segment(90_000, 1500)); // out dts 90000..93000
        t.drop_partial_input(); // dangling 4th AU lost (the abandoned tail)
                                // Next segment starts 12000 ticks (133ms) past the cadence slot —
                                // well under the 0.25s tolerance, bridged only because of the abandon.
        let (frag2, _) = t.push_part(&test_segment(106_500, 1500)).expect("samples");
        assert_eq!(video_tfdt(&frag2), 94_500, "lands exactly on the cadence");
    }

    #[test]
    fn small_forward_gap_bridges_instead_of_leaving_a_hole() {
        // An abandoned segment tail (or encoder hiccup) loses sub-second media.
        // The playlist stays contiguous and cannot express the hole, so the
        // output must close it: the next samples snap onto the cadence. The
        // one-off 0.5s interval must NOT be adopted as a new frame cadence
        // (it needs a confirming repeat), or the gap would pass through.
        let mut t = Transmuxer::new();
        let _ = t.push_part(&test_segment(90_000, 1500)); // out dts 90000..93000
        let (frag2, d2) = t.push_part(&test_segment(141_000, 1500)).expect("samples");
        // Dangler at 94500, then 141000 bridges onto 96000.
        assert_eq!(video_tfdt(&frag2), 94_500);
        assert!(
            (d2 - 6000.0 / 90_000.0).abs() < 1e-9,
            "gapless 4 frames, got {d2}"
        );
    }
}
