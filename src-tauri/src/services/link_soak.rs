//! Shaped-network harness for live playback on a poor link (`link_adapt`).
//!
//! Runs the REAL relay (resolver, `StreamServer`, the parts origin or the
//! whole-segment projection, `link_adapt`) against a live channel, with a
//! simulated player reading it over HTTP the way the phones do, and reports
//! stalls, time frozen, distance behind live and the rendition timeline.
//! Shape the upstream with a proxy in front of it (`HTTPS_PROXY`): the relay
//! and the player's segment fetches both go through it, as on a phone where
//! the relay and the player share the one cellular link.
//!
//! Run (network, long): from src-tauri,
//!   HTTPS_PROXY=http://127.0.0.1:8899 SOAK_CHANNEL=xqc SOAK_SECS=180 \
//!   SOAK_ADAPT=1 SOAK_LL=0 cargo test --no-default-features --lib link_soak -- --ignored --nocapture
//!
//! - `SOAK_ADAPT` 1/0: run with or without the link governor (before/after).
//! - `SOAK_LL` 1/0: the parts origin (the iPhone, Android when entitled) or the
//!   whole-segment path (Android with ad-free playback on).
//! - `SOAK_CUSHION`: the player's distance from the edge before any extra
//!   cushion (default 5.0 whole-segment, 1.2 parts, the phones' defaults).
//! - `SOAK_PHONE` 1/0 (default 1): the phones' rules (the player follows the
//!   governor's cushion, the origin waits longer for late tails) or the
//!   desktop's (the cushion stays the player's own, home deadlines).
//! - `SOAK_CODECS`: codec families the resolver may pick, e.g. `av1,hevc,h264`
//!   (the desktop default on a machine that decodes them; 1440p is HEVC). Unset:
//!   H.264 only.
//!
//! Continuity: every part or segment handed to the player is logged (`SEG`),
//! with GAP / REPEAT / SHORT / PDT-GAP / PDT-OVERLAP lines for dropped or
//! doubled media, FREEZE lines with a cause, RELOAD-MODEL lines for where a
//! reloading player would land after a relay restart, and a CONTINUITY summary.

#![allow(clippy::print_stdout)]

use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct Seg {
    sn: u64,
    dur: f64,
    uri: String,
    pdt: Option<f64>,
}

fn parse_pdt(v: &str) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(v.trim())
        .ok()
        .map(|d| d.timestamp_millis() as f64 / 1000.0)
}

fn now_epoch() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

/// Whole segments of a media playlist.
fn segments(pl: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut sn = 0u64;
    let mut dur = None;
    let mut pdt = None;
    for l in pl.lines() {
        if let Some(v) = l.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            sn = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = l.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            pdt = parse_pdt(v);
        } else if let Some(v) = l.strip_prefix("#EXTINF:") {
            dur = v.split(',').next().and_then(|d| d.parse::<f64>().ok());
        } else if !l.is_empty() && !l.starts_with('#') {
            if let Some(d) = dur.take() {
                out.push(Seg { sn, dur: d, uri: l.trim().to_string(), pdt: pdt.take() });
                sn += 1;
            }
        }
    }
    out
}

/// Parts of a low-latency playlist: (sn, k, duration, uri). Segment sequence
/// numbers come from the part URIs (`part/<sn>/<k>.<ext>`).
fn parts(pl: &str) -> Vec<(u64, usize, f64, String)> {
    pl.lines()
        .filter_map(|l| l.strip_prefix("#EXT-X-PART:"))
        .filter_map(|attrs| {
            let dur = attrs
                .split("DURATION=")
                .nth(1)?
                .split([',', '"'])
                .next()?
                .parse::<f64>()
                .ok()?;
            let uri = attrs.split("URI=\"").nth(1)?.split('"').next()?.to_string();
            let path = uri.strip_prefix("part/")?;
            let path = path.split('.').next()?;
            let mut it = path.split('/');
            let sn = it.next()?.parse().ok()?;
            let k = it.next()?.parse().ok()?;
            Some((sn, k, dur, uri))
        })
        .collect()
}

#[derive(Default)]
struct Player {
    /// Media seconds held ahead of the playhead.
    buffered: f64,
    playing: bool,
    started: Option<Instant>,
    stalled_since: Option<Instant>,
    last_stall_report: Option<Instant>,
    stalls: Vec<f64>,
    last: Option<Instant>,
    /// Programme date of the playhead, advanced with it.
    playhead_pdt: Option<f64>,
    behind: Vec<f64>,
    /// Every freeze once it ended, however short: (start, length), start in
    /// seconds since the run began (set by the loop through `t0`).
    freezes: Vec<(f64, f64)>,
}

impl Player {
    fn advance(&mut self, adapt: bool) {
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |l| (now - l).as_secs_f64());
        self.last = Some(now);
        if !self.playing {
            return;
        }
        if self.buffered > 0.0 {
            let step = dt.min(self.buffered);
            self.buffered -= step;
            if let Some(p) = self.playhead_pdt.as_mut() {
                *p += step;
            }
            if self.buffered <= 1e-3 && self.stalled_since.is_none() {
                self.stalled_since = Some(now - Duration::from_secs_f64(dt - step));
            }
        }
        if let Some(s) = self.stalled_since {
            let held = s.elapsed().as_secs_f64();
            let due = self.last_stall_report.is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
            if adapt && held >= 0.5 && due {
                self.last_stall_report = Some(now);
                crate::services::link_adapt::note_stall(crate::services::link_adapt::SOLO, held, false);
            }
        } else if let Some(p) = self.playhead_pdt {
            self.behind.push(now_epoch() - p);
        }
    }

    /// Adds media; returns the length of a freeze this ended, if any.
    fn add(&mut self, media: f64, pdt: Option<f64>, start_at: f64, adapt: bool, t0: Instant) -> Option<f64> {
        self.buffered += media;
        if self.playhead_pdt.is_none() {
            self.playhead_pdt = pdt;
        }
        if !self.playing && self.buffered >= start_at {
            self.playing = true;
            self.started = Some(Instant::now());
            self.last = Some(Instant::now());
        }
        let mut ended = None;
        if let Some(s) = self.stalled_since {
            if self.buffered >= 0.3 {
                let len = s.elapsed().as_secs_f64();
                self.stalled_since = None;
                self.last_stall_report = None;
                if len > 0.0 {
                    self.freezes.push(((s - t0).as_secs_f64(), len));
                    ended = Some(len);
                }
                if len >= 0.1 {
                    self.stalls.push(len);
                    println!("[link-soak] STALL {len:.2}s");
                    if adapt {
                        crate::services::link_adapt::note_stall(crate::services::link_adapt::SOLO, len, true);
                    }
                }
            }
        }
        ended
    }
}

/// What the simulated player received, checked segment by segment: dropped
/// media (a sequence or part gap, a segment shorter than its EXTINF), media
/// delivered twice, and the programme-date timeline of the playlist.
#[derive(Default)]
struct Continuity {
    /// (sn, EXTINF, programme date) as first listed.
    listed: std::collections::BTreeMap<u64, (f64, Option<f64>)>,
    /// sn -> (media seconds delivered, parts, rung at delivery, first part index).
    delivered: std::collections::BTreeMap<u64, (f64, u32, String, usize)>,
    last: Option<(u64, usize)>,
    /// The first segment the player joined part-way: never judged short.
    first_sn: Option<u64>,
    /// Finished segments still waiting for their EXTINF.
    pending: Vec<u64>,
    checked: std::collections::BTreeSet<u64>,
    gaps: Vec<String>,
    repeats: Vec<String>,
    shorts: Vec<String>,
}

impl Continuity {
    fn note_listing(&mut self, segs: &[Seg], t: f64) {
        for s in segs {
            match self.listed.get(&s.sn) {
                None => {
                    // Timeline check against the previous sn as first listed.
                    if let (Some(pdt), Some((pd, Some(ppdt)))) = (s.pdt, self.listed.get(&(s.sn.wrapping_sub(1))).cloned()) {
                        let d = pdt - (ppdt + pd);
                        if d > 0.05 {
                            let m = format!("t={t:.1}s sn {}: programme date {d:+.3}s after the end of sn {} (timeline hole)", s.sn, s.sn - 1);
                            println!("[link-soak] PDT-GAP {m}");
                            self.gaps.push(m);
                        } else if d < -0.05 {
                            let m = format!("t={t:.1}s sn {}: programme date {d:+.3}s overlaps sn {} (same span listed twice)", s.sn, s.sn - 1);
                            println!("[link-soak] PDT-OVERLAP {m}");
                            self.repeats.push(m);
                        }
                    }
                    self.listed.insert(s.sn, (s.dur, s.pdt));
                }
                Some(&(dur, pdt)) => {
                    let moved = match (pdt, s.pdt) {
                        (Some(a), Some(b)) => (a - b).abs() > 0.05,
                        _ => false,
                    };
                    if moved || (dur - s.dur).abs() > 0.05 {
                        println!(
                            "[link-soak] RELIST t={t:.1}s sn {} relisted: EXTINF {dur:.3} -> {:.3}, pdt {:?} -> {:?}",
                            s.sn, s.dur, pdt, s.pdt
                        );
                    }
                }
            }
        }
        self.check_pending(t);
    }

    /// One part handed to the player.
    fn note_part(&mut self, sn: u64, k: usize, dur: f64, rung: &str, t: f64) {
        if let Some((lsn, lk)) = self.last {
            let ok = (sn == lsn && k == lk + 1) || (sn == lsn + 1 && k == 0) || (lk == usize::MAX && sn == lsn + 1 && k == 0);
            if !ok {
                if (sn, k) <= (lsn, lk) {
                    let m = format!("t={t:.1}s part {sn}/{k} after {lsn}/{lk} (rung {rung})");
                    println!("[link-soak] REPEAT {m}");
                    self.repeats.push(m);
                } else {
                    let prev_rung = self.delivered.get(&lsn).map(|d| d.2.clone()).unwrap_or_default();
                    let m = format!("t={t:.1}s part {sn}/{k} after {lsn}/{lk} (rungs {prev_rung} -> {rung})");
                    println!("[link-soak] GAP {m}");
                    self.gaps.push(m);
                }
            }
            if sn > lsn {
                self.finish(lsn, t);
            }
        } else {
            self.first_sn = Some(sn);
        }
        self.last = Some((sn, k));
        let e = self.delivered.entry(sn).or_insert((0.0, 0, rung.to_string(), k));
        e.0 += dur;
        e.1 += 1;
        if e.2 != rung {
            e.2 = format!("{}+{rung}", e.2);
        }
    }

    /// A whole segment handed to the player (the whole-segment path).
    fn note_segment(&mut self, sn: u64, dur: f64, rung: &str, t: f64, after_resync: bool) {
        if let Some((lsn, _)) = self.last {
            if sn == lsn {
                let m = format!("t={t:.1}s sn {sn} delivered twice (rung {rung})");
                println!("[link-soak] REPEAT {m}");
                self.repeats.push(m);
            } else if sn != lsn + 1 {
                let cause = if after_resync { "window resync" } else { "unexplained" };
                let m = format!("t={t:.1}s sn {sn} after {lsn}: {} segments skipped ({cause}, rung {rung})", sn.saturating_sub(lsn + 1));
                println!("[link-soak] GAP {m}");
                self.gaps.push(m);
            }
        }
        self.last = Some((sn, 0));
        self.delivered.insert(sn, (dur, 1, rung.to_string(), 0));
    }

    fn finish(&mut self, sn: u64, t: f64) {
        if let Some((media, n, rung, k0)) = self.delivered.get(&sn).cloned() {
            let (extinf, pdt) = self.listed.get(&sn).cloned().unwrap_or((f64::NAN, None));
            println!(
                "[link-soak] SEG t={t:.1}s sn={sn} parts={n} from={k0} media={media:.3}s extinf={extinf:.3}s pdt={} rung={rung}",
                pdt.map_or("-".into(), |p| format!("{p:.3}"))
            );
        }
        self.pending.push(sn);
        self.check_pending(t);
    }

    fn check_pending(&mut self, t: f64) {
        let mut keep = Vec::new();
        for sn in std::mem::take(&mut self.pending) {
            let Some(&(extinf, _)) = self.listed.get(&sn) else {
                keep.push(sn);
                continue;
            };
            if !self.checked.insert(sn) || Some(sn) == self.first_sn {
                continue;
            }
            let Some((media, n, rung, k0)) = self.delivered.get(&sn).cloned() else { continue };
            if k0 != 0 {
                // Joined part-way (a restart): the head was not delivered.
                let m = format!("t={t:.1}s sn {sn}: delivery began at part {k0} (rung {rung}), head missing");
                println!("[link-soak] GAP {m}");
                self.gaps.push(m);
            } else if media + 0.034 < extinf {
                let m = format!("t={t:.1}s sn {sn}: {media:.3}s of {extinf:.3}s delivered in {n} parts, {:.3}s dropped (rung {rung})", extinf - media);
                println!("[link-soak] SHORT {m}");
                self.shorts.push(m);
            } else if media > extinf + 0.05 {
                let m = format!("t={t:.1}s sn {sn}: {media:.3}s delivered for a {extinf:.3}s segment (rung {rung})");
                println!("[link-soak] REPEAT {m}");
                self.repeats.push(m);
            }
        }
        self.pending = keep;
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "network soak; run explicitly with SOAK_CHANNEL / HTTPS_PROXY"]
async fn link_soak() {
    let env = |k: &str| std::env::var(k).ok();
    // The governor's own decisions (measurements, switches) in the run log.
    let _ = env_logger::Builder::new()
        .parse_filters(&env("RUST_LOG").unwrap_or_else(|| "warn,streamnook_lib::services::link_adapt=info".into()))
        .is_test(true)
        .try_init();
    let channel = env("SOAK_CHANNEL").unwrap_or_else(|| "xqc".into());
    let secs: u64 = env("SOAK_SECS").and_then(|v| v.parse().ok()).unwrap_or(180);
    let adapt = env("SOAK_ADAPT").is_none_or(|v| v == "1");
    let ll = env("SOAK_LL").is_some_and(|v| v == "1");
    let phone = env("SOAK_PHONE").is_none_or(|v| v == "1");
    let base_cushion: f64 = env("SOAK_CUSHION")
        .and_then(|v| v.parse().ok())
        .unwrap_or(if ll { 1.2 } else { 5.0 });
    println!(
        "[link-soak] channel={channel} secs={secs} adapt={adapt} ll={ll} phone={phone} cushion={base_cushion} proxy={:?}",
        env("HTTPS_PROXY")
    );

    // From before the resolve: what a viewer waits for the first frame.
    let opened = Instant::now();
    crate::services::link_adapt::force_enabled(adapt);
    crate::services::link_adapt::force_phone_rules(phone);
    crate::services::ll_origin::set_disabled(!ll);
    // The desktop reports what it decodes (`reportCodecPreference`; AV1 and
    // HEVC with the default enhanced-codecs setting on). Unset: H.264 only.
    if let Some(c) = env("SOAK_CODECS") {
        crate::services::twitch_resolver::set_codec_preference(c.split(',').map(|s| s.trim().to_string()).collect());
        println!("[link-soak] codecs allowed: {c}");
    }
    let r = match crate::services::twitch_resolver::resolve_live(&channel, None, "best").await {
        Ok(r) => r,
        Err(e) => {
            println!("[link-soak] ABORT: resolve failed: {e}");
            return;
        }
    };
    let variants = crate::services::twitch_resolver::playable_variants(&r.master, &channel);
    for v in &variants {
        println!(
            "[link-soak] ladder {} {}x{} fps={:?} bw={:?} codecs={:?}{}",
            v.name,
            v.width.unwrap_or(0),
            v.height.unwrap_or(0),
            v.fps,
            v.bandwidth,
            v.codecs,
            if v.url == r.url { "  <- selected" } else { "" }
        );
    }
    let serve = crate::services::link_adapt::begin(
        crate::services::link_adapt::SOLO,
        &variants,
        &r.url,
        true,
        crate::services::stream_server::solo_twitch_route(),
    )
    .map_or_else(|| r.url.clone(), |rung| rung.url);
    let generation = crate::services::link_adapt::generation(crate::services::link_adapt::SOLO);
    let port = match crate::services::stream_server::StreamServer::start_proxy_server(serve).await {
        Ok(p) => p,
        Err(e) => {
            println!("[link-soak] ABORT: relay failed: {e}");
            return;
        }
    };
    // As `start_stream`: the first downloads may move the start rung before
    // the player loads.
    crate::services::link_adapt::start_measure(crate::services::link_adapt::SOLO, generation).await;
    if let Some(rung) =
        crate::services::link_adapt::start_check(crate::services::link_adapt::SOLO, generation)
    {
        println!("[link-soak] starting on {} after the first downloads", rung.name);
        let _ = crate::services::stream_server::StreamServer::start_proxy_server(rung.url).await;
    }
    crate::services::stream_server::set_solo_session(Some(channel.clone()));
    let on_ll = crate::services::ll_origin::is_active();
    println!("[link-soak] relay on {port}, parts origin active: {on_ll}");
    if ll && !on_ll {
        println!("[link-soak] ABORT: the parts origin declined this channel");
        return;
    }

    let local = format!("http://127.0.0.1:{port}/");
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .unwrap();
    let mut player = Player::default();
    let mut timeline: Vec<(f64, String)> = Vec::new();
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(secs);
    let mut next_sn: Option<u64> = None;
    let mut newest_part: Option<(u64, usize)> = None;
    let mut fetch: Option<tokio::task::JoinHandle<Option<(u64, f64, Option<f64>, f64, u64)>>> = None;
    let mut playlist_at = Instant::now() - Duration::from_secs(10);
    let mut listing: Vec<Seg> = Vec::new();
    let mut transfers: Vec<f64> = Vec::new();
    let mut resyncs = 0u32;
    let mut cont = Continuity::default();
    // Times (s since t0) of rung switches, window resyncs and ad-marker changes,
    // to name the cause of each freeze.
    let mut switch_times: Vec<f64> = Vec::new();
    let mut resync_times: Vec<f64> = Vec::new();
    let mut ad_changes: Vec<(f64, bool)> = Vec::new();
    let mut ad_on = false;
    let mut last_media_at: f64 = 0.0;
    let mut freeze_log: Vec<String> = Vec::new();
    let mut reload_check: Option<f64> = None;
    let mut loss_times: Vec<f64> = Vec::new();
    let mut after_resync = false;
    let classify = |start: f64, len: f64, idle: f64, sw: &[f64], rs: &[f64], ads: &[(f64, bool)], loss: &[f64]| -> String {
        let near = |ts: &[f64], before: f64| ts.iter().any(|&t| t >= start - before && t <= start + 0.5);
        let ad_ts: Vec<f64> = ads.iter().map(|a| a.0).collect();
        if near(sw, 12.0) {
            format!("rung switch (len {len:.2}s)")
        } else if near(rs, 5.0) {
            format!("window resync (len {len:.2}s)")
        } else if near(loss, 6.0) {
            format!("right after media the relay skipped or cut short (see GAP/SHORT; len {len:.2}s)")
        } else if near(&ad_ts, 10.0) {
            format!("ad transition (len {len:.2}s)")
        } else {
            format!("buffer ran dry: no media arrived for {idle:.2}s (len {len:.2}s)")
        }
    };

    while Instant::now() < deadline {
        player.advance(adapt);
        let extra = crate::services::link_adapt::extra_cushion(crate::services::link_adapt::SOLO);
        let mut rung_now = String::new();
        if let Some((rung, _)) = crate::services::link_adapt::snapshot(crate::services::link_adapt::SOLO) {
            if timeline.last().is_none_or(|(_, n)| *n != rung) {
                let t = t0.elapsed().as_secs_f64();
                println!("[link-soak] {t:6.1}s rung {rung} (buffer {:.2}s)", player.buffered);
                if !timeline.is_empty() {
                    switch_times.push(t);
                    reload_check = Some(t);
                }
                timeline.push((t, rung.clone()));
            }
            rung_now = rung;
        }

        if on_ll {
            // Parts: every newly listed part is local and instant; what limits
            // the player is how fast the origin publishes them.
            if playlist_at.elapsed() >= Duration::from_millis(250) {
                playlist_at = Instant::now();
                if let Ok(pl) = http.get(format!("{local}stream.m3u8")).send().await {
                    let text = pl.text().await.unwrap_or_default();
                    let segs = segments(&text);
                    let listed = parts(&text);
                    let tn = t0.elapsed().as_secs_f64();
                    cont.note_listing(&segs, tn);
                    let ad = text.contains("stitched-ad") || text.contains(",Amazon") || text.contains("twitch-ad");
                    if ad != ad_on {
                        ad_on = ad;
                        ad_changes.push((tn, ad));
                        println!("[link-soak] {tn:6.1}s ad markers {}", if ad { "ON" } else { "OFF" });
                    }
                    // A relay restart: the real desktop player reloads on the new
                    // URL and starts its cushion behind the newest part. Where
                    // that lands against the playhead it had is the jump a viewer
                    // sees (forward = media skipped, back = media replayed). The
                    // harness itself keeps feeding from the newest part.
                    if let (Some(at), false) = (reload_check, listed.is_empty()) {
                        reload_check = None;
                        let mut acc = 0.0;
                        let mut first = listed.len() - 1;
                        for (i, p) in listed.iter().enumerate().rev() {
                            acc += p.2;
                            first = i;
                            if acc >= base_cushion {
                                break;
                            }
                        }
                        let (sn, k, _, _) = listed[first];
                        let offset: f64 = listed.iter().filter(|p| p.0 == sn && p.1 < k).map(|p| p.2).sum();
                        let seg_pdt = cont.listed.get(&sn).and_then(|s| s.1).or_else(|| {
                            cont.listed.get(&sn.saturating_sub(1)).and_then(|(d, p)| p.map(|p| p + d))
                        });
                        let start_pdt = seg_pdt.map(|p| p + offset);
                        match (start_pdt, player.playhead_pdt) {
                            (Some(s), Some(h)) => println!(
                                "[link-soak] RELOAD-MODEL switch at {at:.1}s: a reload starts at part {sn}/{k} (pdt {s:.3}), playhead at {h:.3}: {} {:.2}s; harness continues after {:?}, new listing starts {}/{}",
                                if s >= h { "skips" } else { "replays" },
                                (s - h).abs(),
                                newest_part,
                                listed[0].0,
                                listed[0].1
                            ),
                            _ => println!("[link-soak] RELOAD-MODEL switch at {at:.1}s: start part {sn}/{k}, no programme date to compare"),
                        }
                    }
                    if newest_part.is_none() && !listed.is_empty() {
                        // hls.js starts `cushion` behind the newest part: skip
                        // everything older, and date the playhead there.
                        let mut acc = 0.0;
                        let mut first = listed.len() - 1;
                        for (i, p) in listed.iter().enumerate().rev() {
                            acc += p.2;
                            first = i;
                            if acc >= base_cushion + extra {
                                break;
                            }
                        }
                        let (sn, k, _, _) = listed[first];
                        let offset: f64 =
                            listed.iter().filter(|p| p.0 == sn && p.1 < k).map(|p| p.2).sum();
                        // The in-progress segment has no EXTINF yet: date it
                        // from the end of the one before.
                        player.playhead_pdt = segs
                            .iter()
                            .find(|s| s.sn == sn)
                            .and_then(|s| s.pdt)
                            .or_else(|| {
                                segs.iter().find(|s| s.sn + 1 == sn).and_then(|s| s.pdt.map(|p| p + s.dur))
                            })
                            .map(|p| p + offset);
                        newest_part = if first == 0 { None } else { Some((listed[first - 1].0, listed[first - 1].1)) };
                        if newest_part.is_none() {
                            newest_part = Some(if k == 0 { (sn.saturating_sub(1), usize::MAX) } else { (sn, k - 1) });
                        }
                    }
                    // Time since the previous poll that brought new media.
                    let idle_before = t0.elapsed().as_secs_f64() - last_media_at;
                    let mut any_new = false;
                    for (sn, k, dur, _uri) in listed {
                        if newest_part.is_some_and(|(nsn, nk)| (sn, k) <= (nsn, nk)) {
                            continue;
                        }
                        newest_part = Some((sn, k));
                        let tn = t0.elapsed().as_secs_f64();
                        let issues = cont.gaps.len() + cont.shorts.len() + cont.repeats.len();
                        cont.note_part(sn, k, dur, &rung_now, tn);
                        if cont.gaps.len() + cont.shorts.len() + cont.repeats.len() != issues {
                            loss_times.push(tn);
                        }
                        let idle = idle_before;
                        any_new = true;
                        if let Some(len) = player.add(dur, None, base_cushion + extra, adapt, t0) {
                            let start = tn - len;
                            let why = classify(start, len, idle, &switch_times, &resync_times, &ad_changes, &loss_times);
                            let m = format!("t={start:.1}s at part {sn}/{k} rung {rung_now}: {why}");
                            println!("[link-soak] FREEZE {m}");
                            freeze_log.push(m);
                        }
                    }
                    if any_new {
                        last_media_at = t0.elapsed().as_secs_f64();
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        }

        // Whole segments: poll the playlist, fetch one segment at a time.
        if playlist_at.elapsed() >= Duration::from_millis(1000) {
            playlist_at = Instant::now();
            if let Ok(pl) = http.get(format!("{local}stream.m3u8")).send().await {
                let text = pl.text().await.unwrap_or_default();
                listing = segments(&text);
                let tn = t0.elapsed().as_secs_f64();
                cont.note_listing(&listing, tn);
                let ad = text.contains("stitched-ad") || text.contains(",Amazon") || text.contains("twitch-ad");
                if ad != ad_on {
                    ad_on = ad;
                    ad_changes.push((tn, ad));
                    println!("[link-soak] {tn:6.1}s ad markers {}", if ad { "ON" } else { "OFF" });
                }
                if next_sn.is_none() && !listing.is_empty() {
                    let mut acc = 0.0;
                    let mut start = listing.last().unwrap().sn;
                    for s in listing.iter().rev() {
                        acc += s.dur;
                        start = s.sn;
                        if acc >= base_cushion + extra {
                            break;
                        }
                    }
                    next_sn = Some(start);
                }
            }
        }
        if let Some(h) = fetch.as_ref() {
            if h.is_finished() {
                let h = fetch.take().unwrap();
                if let Ok(Some((bytes, took, pdt, dur, sn))) = h.await {
                    transfers.push(bytes as f64 * 8.0 / took / 1e6);
                    if adapt {
                        crate::services::link_adapt::note_player_segment(
                            crate::services::link_adapt::SOLO,
                            bytes,
                            took,
                            Some(crate::services::link_adapt::TWITCH_SEGMENT_SECS),
                            None,
                        );
                    }
                    let tn = t0.elapsed().as_secs_f64();
                    println!(
                        "[link-soak] SEG t={tn:.1}s sn={sn} bytes={bytes} took={took:.2}s dur={dur:.3}s pdt={} rung={rung_now}",
                        pdt.map_or("-".into(), |p| format!("{p:.3}"))
                    );
                    cont.note_segment(sn, dur, &rung_now, tn, after_resync);
                    after_resync = false;
                    let idle = tn - last_media_at;
                    last_media_at = tn;
                    if let Some(len) = player.add(dur, pdt, (base_cushion + extra).min(3.0) * 0.6, adapt, t0) {
                        let start = tn - len;
                        let why = classify(start, len, idle, &switch_times, &resync_times, &ad_changes, &loss_times);
                        let m = format!("t={start:.1}s at sn {sn} rung {rung_now}: {why}");
                        println!("[link-soak] FREEZE {m}");
                        freeze_log.push(m);
                    }
                    next_sn = next_sn.map(|n| n + 1);
                }
            }
        }
        // hls.js: a player that fell out of the live window resyncs to the
        // edge minus its cushion, skipping what it missed.
        if fetch.is_none() {
            if let (Some(n), Some(first)) = (next_sn, listing.first()) {
                if n < first.sn {
                    let mut acc = 0.0;
                    let mut start = listing.last().unwrap().sn;
                    for s in listing.iter().rev() {
                        acc += s.dur;
                        start = s.sn;
                        if acc >= base_cushion + extra {
                            break;
                        }
                    }
                    println!("[link-soak] fell out of the window at {n}; resync to {start}");
                    resyncs += 1;
                    resync_times.push(t0.elapsed().as_secs_f64());
                    after_resync = true;
                    next_sn = Some(start);
                    player.buffered = 0.0;
                    player.playhead_pdt = listing.iter().find(|s| s.sn == start).and_then(|s| s.pdt);
                }
            }
        }
        if fetch.is_none() {
            if let Some(seg) = next_sn.and_then(|n| listing.iter().find(|s| s.sn == n).cloned()) {
                let url = if seg.uri.starts_with("http") { seg.uri.clone() } else { format!("{local}{}", seg.uri) };
                let http = http.clone();
                fetch = Some(tokio::spawn(async move {
                    let t = Instant::now();
                    let res = http.get(&url).timeout(Duration::from_secs(20)).send().await.ok()?;
                    if !res.status().is_success() {
                        return None;
                    }
                    let body = res.bytes().await.ok()?;
                    Some((body.len() as u64, t.elapsed().as_secs_f64(), seg.pdt, seg.dur, seg.sn))
                }));
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    player.advance(adapt);
    if let Some(s) = player.stalled_since {
        player.stalls.push(s.elapsed().as_secs_f64());
    }
    let wall = player.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
    let frozen: f64 = player.stalls.iter().sum();
    let mut behind = player.behind.clone();
    let behind_max = behind.iter().cloned().fold(f64::NAN, f64::max);
    let behind_end = behind.last().copied().unwrap_or(f64::NAN);
    println!("\n========== LINK SOAK: {channel} adapt={adapt} ll={on_ll} phone={phone} ==========");
    println!(
        "played {wall:.0}s  stalls {}  frozen {frozen:.1}s ({:.1}%)",
        player.stalls.len(),
        100.0 * frozen / wall.max(1.0)
    );
    println!(
        "behind live p50 {:.2}s  p90 {:.2}s  (programme date of the playhead)",
        pct(&mut behind, 0.5),
        pct(&mut behind, 0.9)
    );
    println!("behind live max {behind_max:.2}s  end {behind_end:.2}s");
    if !transfers.is_empty() {
        println!("segment transfer p50 {:.2} Mbit/s", pct(&mut transfers, 0.5));
    }
    println!("resyncs (fell out of the live window): {resyncs}");
    if let Some(first) = player.started {
        println!("first frame {:.1}s after opening", (first - opened).as_secs_f64());
    }
    println!("rungs: {timeline:?}");
    let all_frozen: f64 = player.freezes.iter().map(|f| f.1).sum();
    println!(
        "CONTINUITY: gaps {}  repeats {}  short segments (dropped tails) {}  freezes {} ({all_frozen:.2}s, any length)  segments delivered {}",
        cont.gaps.len(),
        cont.repeats.len(),
        cont.shorts.len(),
        player.freezes.len(),
        cont.delivered.len()
    );
    for m in cont.gaps.iter().chain(&cont.repeats).chain(&cont.shorts) {
        println!("  {m}");
    }
    for m in &freeze_log {
        println!("  freeze {m}");
    }
    if let Some(s) = player.stalled_since {
        println!("  freeze still open at the end: {:.2}s", s.elapsed().as_secs_f64());
    }
    println!("extra cushion at the end: {:.1}s", crate::services::link_adapt::extra_cushion(crate::services::link_adapt::SOLO));
    crate::services::link_adapt::end(crate::services::link_adapt::SOLO);
    let _ = crate::services::stream_server::StreamServer::stop().await;
}
