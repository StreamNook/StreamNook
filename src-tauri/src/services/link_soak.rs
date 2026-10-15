//! Shaped-network harness for live playback on the phones (`link_adapt`).
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
                crate::services::link_adapt::note_stall(held, false);
            }
        } else if let Some(p) = self.playhead_pdt {
            self.behind.push(now_epoch() - p);
        }
    }

    fn add(&mut self, media: f64, pdt: Option<f64>, start_at: f64, adapt: bool) {
        self.buffered += media;
        if self.playhead_pdt.is_none() {
            self.playhead_pdt = pdt;
        }
        if !self.playing && self.buffered >= start_at {
            self.playing = true;
            self.started = Some(Instant::now());
            self.last = Some(Instant::now());
        }
        if let Some(s) = self.stalled_since {
            if self.buffered >= 0.3 {
                let len = s.elapsed().as_secs_f64();
                self.stalled_since = None;
                self.last_stall_report = None;
                if len >= 0.1 {
                    self.stalls.push(len);
                    println!("[link-soak] STALL {len:.2}s");
                    if adapt {
                        crate::services::link_adapt::note_stall(len, true);
                    }
                }
            }
        }
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
    let channel = env("SOAK_CHANNEL").unwrap_or_else(|| "xqc".into());
    let secs: u64 = env("SOAK_SECS").and_then(|v| v.parse().ok()).unwrap_or(180);
    let adapt = env("SOAK_ADAPT").is_none_or(|v| v == "1");
    let ll = env("SOAK_LL").is_some_and(|v| v == "1");
    let base_cushion: f64 = env("SOAK_CUSHION")
        .and_then(|v| v.parse().ok())
        .unwrap_or(if ll { 1.2 } else { 5.0 });
    println!(
        "[link-soak] channel={channel} secs={secs} adapt={adapt} ll={ll} cushion={base_cushion} proxy={:?}",
        env("HTTPS_PROXY")
    );

    crate::services::link_adapt::force_enabled(adapt);
    crate::services::ll_origin::set_disabled(!ll);
    let r = match crate::services::twitch_resolver::resolve_live(&channel, None, "best").await {
        Ok(r) => r,
        Err(e) => {
            println!("[link-soak] ABORT: resolve failed: {e}");
            return;
        }
    };
    let variants = crate::services::twitch_resolver::parse_master(&r.master);
    let serve = crate::services::link_adapt::begin(&variants, &r.url, true);
    let port = match crate::services::stream_server::StreamServer::start_proxy_server(serve).await {
        Ok(p) => p,
        Err(e) => {
            println!("[link-soak] ABORT: relay failed: {e}");
            return;
        }
    };
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
    let mut fetch: Option<tokio::task::JoinHandle<Option<(u64, f64, Option<f64>, f64)>>> = None;
    let mut playlist_at = Instant::now() - Duration::from_secs(10);
    let mut listing: Vec<Seg> = Vec::new();
    let mut transfers: Vec<f64> = Vec::new();
    let mut resyncs = 0u32;

    while Instant::now() < deadline {
        player.advance(adapt);
        let extra = crate::services::link_adapt::extra_cushion();
        if let Some((rung, _)) = crate::services::link_adapt::snapshot() {
            if timeline.last().is_none_or(|(_, n)| *n != rung) {
                println!("[link-soak] {:6.1}s rung {rung}", t0.elapsed().as_secs_f64());
                timeline.push((t0.elapsed().as_secs_f64(), rung));
            }
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
                        player.playhead_pdt =
                            segs.iter().find(|s| s.sn == sn).and_then(|s| s.pdt).map(|p| p + offset);
                        newest_part = if first == 0 { None } else { Some((listed[first - 1].0, listed[first - 1].1)) };
                        if newest_part.is_none() {
                            newest_part = Some(if k == 0 { (sn.saturating_sub(1), usize::MAX) } else { (sn, k - 1) });
                        }
                    }
                    for (sn, k, dur, _uri) in listed {
                        if newest_part.is_some_and(|(nsn, nk)| (sn, k) <= (nsn, nk)) {
                            continue;
                        }
                        newest_part = Some((sn, k));
                        player.add(dur, None, base_cushion + extra, adapt);
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
                listing = segments(&pl.text().await.unwrap_or_default());
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
                if let Ok(Some((bytes, took, pdt, dur))) = h.await {
                    transfers.push(bytes as f64 * 8.0 / took / 1e6);
                    if adapt {
                        crate::services::link_adapt::note_throughput(bytes, took);
                    }
                    player.add(dur, pdt, (base_cushion + extra).min(3.0) * 0.6, adapt);
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
                    Some((body.len() as u64, t.elapsed().as_secs_f64(), seg.pdt, seg.dur))
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
    println!("\n========== LINK SOAK: {channel} adapt={adapt} ll={on_ll} ==========");
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
    if !transfers.is_empty() {
        println!("segment transfer p50 {:.2} Mbit/s", pct(&mut transfers, 0.5));
    }
    println!("resyncs (fell out of the live window): {resyncs}");
    println!("rungs: {timeline:?}");
    println!("extra cushion at the end: {:.1}s", crate::services::link_adapt::extra_cushion());
    crate::services::link_adapt::end();
    let _ = crate::services::stream_server::StreamServer::stop().await;
}
