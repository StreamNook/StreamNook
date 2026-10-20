//! Where the wallet button's split glyph draws its divider.
//!
//! A channel with both channel points and Bits shows one 22 px square: the
//! points art in an 11 px box at the upper left, the Bits gem in an 11 px box
//! at the lower right, and a hairline across the diagonal gap between them.
//! A line placed by the boxes alone looks wrong for most art: a round icon
//! leaves a wide gap at its box corner, a full-bleed square leaves none, art
//! with transparent padding sits deep inside its box, and the gem is a tall
//! diamond whose tip barely reaches the gap. So this reads the art itself.
//!
//! Each mark's alpha is reduced to two things on the 22 px grid:
//! - how far its visible mass reaches toward the gap, measured along the
//!   glyph's main diagonal (`x + y`). The reach is a low percentile of the
//!   mark's own pixels rather than its single nearest pixel: a broad round
//!   edge facing the gap reads as close, a lone tip does not, and the eye
//!   judges the gap by mass;
//! - how wide it spans across the gap (`x - y`), so the line is as long as
//!   the two marks' outer bounds along that diagonal and never longer.
//!
//! The line sits midway between the two reaches and runs the union of the two
//! spans. Twitch's default points glyph is drawn here from its own geometry.
//! Every answer is cached by its pair of URLs, so a channel's art is fetched
//! and measured once per session; until it arrives, and for art that cannot
//! be read, the shells draw `SplitLine::FALLBACK`.

use std::collections::{HashMap, VecDeque};

use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::Mutex;

/// The glyph's grid.
const GRID: f32 = 22.0;
/// Each mark's box: its top-left corner on the grid and its side.
const POINTS_BOX: (f32, f32) = (0.5, 0.5);
const GEM_BOX: (f32, f32) = (10.5, 10.5);
const MARK: f32 = 11.0;
/// Alpha at or above this counts as visible (the glyph's own edge AA does not).
const VISIBLE_ALPHA: u8 = 40;
/// The share of a mark's visible pixels, counted from the gap side, at which
/// its edge is judged to be. Tuned by eye on screenshots: Twitch's ring
/// against the 100-bit gem looks evenly spaced at this value.
const REACH_PERCENTILE: f32 = 0.06;
/// The line never comes nearer than this to either mark's nearest pixel
/// (half the 1 px stroke plus a hair).
const MIN_CLEARANCE: f32 = 0.9;
/// Art bigger than this is not points art; it is not fetched.
const MAX_ART_BYTES: usize = 256 * 1024;
const MAX_TRACKED: usize = 128;

/// The divider on the 22 px grid, from its upper-right end to its lower-left.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct SplitLine {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl SplitLine {
    /// The line for Twitch's ring against the 100-bit gem, which the shells
    /// draw until (or unless) the art is measured.
    pub const FALLBACK: SplitLine = SplitLine { x1: 15.71, y1: 8.79, x2: 8.79, y2: 15.71 };

    /// The line at `s` (its `x + y`) running from `d_max` to `d_min` (its `x - y`).
    fn at(s: f32, d_min: f32, d_max: f32) -> SplitLine {
        let round = |v: f32| (v * 100.0).round() / 100.0;
        SplitLine {
            x1: round((s + d_max) / 2.0),
            y1: round((s - d_max) / 2.0),
            x2: round((s + d_min) / 2.0),
            y2: round((s - d_min) / 2.0),
        }
    }
}

/// One mark reduced to what the line needs, on the grid.
#[derive(Clone, Copy, Debug)]
struct Reach {
    /// The `x + y` its visible mass reaches toward the gap.
    mass: f32,
    /// The `x + y` of its single nearest pixel.
    hard: f32,
    /// Its span across the gap, `x - y`.
    d_min: f32,
    d_max: f32,
}

/// A mark's alpha, row by row, `w` x `h`.
struct Alpha {
    w: usize,
    h: usize,
    a: Vec<u8>,
}

impl Alpha {
    fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> Alpha {
        Alpha { w: w as usize, h: h as usize, a: rgba.chunks_exact(4).map(|p| p[3]).collect() }
    }

    /// Twitch's channel points glyph (the desktop's ChannelPointsIcon) on a
    /// 24 unit square: a ring between radius 9 and 11, and inside its top-right
    /// a quarter arc between radius 5 and 7.
    fn default_points(side: usize) -> Alpha {
        let mut a = vec![0u8; side * side];
        for y in 0..side {
            for x in 0..side {
                let px = (x as f32 + 0.5) / side as f32 * 24.0 - 12.0;
                let py = (y as f32 + 0.5) / side as f32 * 24.0 - 12.0;
                let r = (px * px + py * py).sqrt();
                let ring = (9.0..=11.0).contains(&r);
                let arc = (5.0..=7.0).contains(&r) && px >= 0.0 && py <= 0.0;
                if ring || arc {
                    a[y * side + x] = 255;
                }
            }
        }
        Alpha { w: side, h: side, a }
    }

    /// The mark drawn into its box at `origin` with `object-fit: contain`
    /// (centered, aspect kept), reduced to its reach. `toward_gap_is_high`:
    /// the points art faces the gap with its highest `x + y`, the gem with its
    /// lowest. `None` when nothing is visible.
    fn reach(&self, origin: (f32, f32), toward_gap_is_high: bool) -> Option<Reach> {
        let scale = MARK / self.w.max(self.h) as f32;
        let off_x = origin.0 + (MARK - self.w as f32 * scale) / 2.0;
        let off_y = origin.1 + (MARK - self.h as f32 * scale) / 2.0;
        let mut sums = Vec::new();
        let (mut d_min, mut d_max) = (f32::MAX, f32::MIN);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.a[y * self.w + x] < VISIBLE_ALPHA {
                    continue;
                }
                let gx = off_x + (x as f32 + 0.5) * scale;
                let gy = off_y + (y as f32 + 0.5) * scale;
                sums.push(gx + gy);
                d_min = d_min.min(gx - gy);
                d_max = d_max.max(gx - gy);
            }
        }
        if sums.is_empty() {
            return None;
        }
        sums.sort_by(|a, b| a.total_cmp(b));
        if toward_gap_is_high {
            sums.reverse();
        }
        let at = ((sums.len() as f32 * REACH_PERCENTILE) as usize).min(sums.len() - 1);
        // Half a source pixel: the pixel's own edge, not its center, is what shows.
        let half = scale / 2.0 * if toward_gap_is_high { 1.0 } else { -1.0 };
        Some(Reach { mass: sums[at] + half, hard: sums[0] + half, d_min: d_min - scale / 2.0, d_max: d_max + scale / 2.0 })
    }
}

/// The line between the points art's reach and the gem's.
fn line_between(points: Reach, gem: Reach) -> SplitLine {
    let mut s = (points.mass + gem.mass) / 2.0;
    // `x + y` moves 1.414 per pixel of perpendicular distance.
    let clear = MIN_CLEARANCE * std::f32::consts::SQRT_2;
    let (low, high) = (points.hard + clear, gem.hard - clear);
    s = if low <= high { s.clamp(low, high) } else { (points.hard + gem.hard) / 2.0 };
    // The union of the two spans, kept inside the glyph's own square.
    let d_max = points.d_max.max(gem.d_max).min(GRID - (s - GRID).abs());
    let d_min = points.d_min.min(gem.d_min).max(-(GRID - (s - GRID).abs()));
    SplitLine::at(s, d_min, d_max)
}

fn decode(bytes: &[u8]) -> Option<Alpha> {
    let (w, h, rgba) = crate::services::badge_service::decode_rgba(bytes)?;
    Some(Alpha::from_rgba(w, h, &rgba))
}

static CACHE: Lazy<Mutex<(HashMap<String, SplitLine>, VecDeque<String>)>> = Lazy::new(Default::default);

async fn fetch(url: &str) -> Option<Vec<u8>> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let response = crate::services::http::client().get(parsed).send().await.ok()?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n as usize > MAX_ART_BYTES) {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    (bytes.len() <= MAX_ART_BYTES).then(|| bytes.to_vec())
}

/// The divider for this pair of marks: `points_icon` is the channel's own
/// points art (None for Twitch's glyph), `gem` the Bits gem. Art that cannot
/// be fetched or read gets the fallback, and is tried again next session.
pub async fn split_line(points_icon: Option<&str>, gem: &str) -> SplitLine {
    let key = format!("{}|{gem}", points_icon.unwrap_or(""));
    if let Some(line) = CACHE.lock().await.0.get(&key) {
        return *line;
    }
    let points_bytes = match points_icon {
        Some(url) => match fetch(url).await {
            Some(b) => Some(b),
            None => return SplitLine::FALLBACK,
        },
        None => None,
    };
    let Some(gem_bytes) = fetch(gem).await else {
        return SplitLine::FALLBACK;
    };
    let measured = tokio::task::spawn_blocking(move || {
        let points = match points_bytes {
            Some(b) => decode(&b)?,
            None => Alpha::default_points(96),
        };
        let gem = decode(&gem_bytes)?;
        Some(line_between(points.reach(POINTS_BOX, true)?, gem.reach(GEM_BOX, false)?))
    })
    .await
    .ok()
    .flatten();
    let Some(line) = measured else {
        return SplitLine::FALLBACK;
    };
    let mut guard = CACHE.lock().await;
    let (map, order) = &mut *guard;
    if map.insert(key.clone(), line).is_none() {
        order.push_back(key);
        while order.len() > MAX_TRACKED {
            if let Some(old) = order.pop_front() {
                map.remove(&old);
            }
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(side: usize) -> Alpha {
        Alpha { w: side, h: side, a: vec![255; side * side] }
    }

    fn disc(side: usize, pad: usize) -> Alpha {
        let mut a = vec![0u8; side * side];
        let c = side as f32 / 2.0;
        let r = c - pad as f32;
        for y in 0..side {
            for x in 0..side {
                let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
                if dx * dx + dy * dy <= r * r {
                    a[y * side + x] = 255;
                }
            }
        }
        Alpha { w: side, h: side, a }
    }

    /// A tall diamond like the 100-bit gem: 42 of 56 wide, 53 tall.
    fn diamond() -> Alpha {
        let side = 56;
        let mut a = vec![0u8; side * side];
        for y in 0..side {
            for x in 0..side {
                let (dx, dy) = ((x as f32 + 0.5 - 28.0).abs() / 21.0, (y as f32 + 0.5 - 28.5).abs() / 26.5);
                if dx + dy <= 1.0 {
                    a[y * side + x] = 255;
                }
            }
        }
        Alpha { w: side, h: side, a }
    }

    fn gap(line: SplitLine, reach: Reach, high: bool) -> f32 {
        let s = line.x1 + line.y1;
        (if high { s - reach.hard } else { reach.hard - s }) / std::f32::consts::SQRT_2
    }

    #[test]
    fn the_line_clears_every_kind_of_art() {
        let gem = diamond().reach(GEM_BOX, false).unwrap();
        for art in [square(112), disc(112, 0), disc(112, 20), Alpha::default_points(96)] {
            let points = art.reach(POINTS_BOX, true).unwrap();
            let line = line_between(points, gem);
            assert!(gap(line, points, true) >= MIN_CLEARANCE - 0.01, "{line:?} hugs the points art");
            assert!(gap(line, gem, false) >= MIN_CLEARANCE - 0.01, "{line:?} hugs the gem");
            // It stays a diagonal: x + y is the same at both ends.
            assert!(((line.x1 + line.y1) - (line.x2 + line.y2)).abs() < 0.02);
        }
    }

    #[test]
    fn padded_art_brings_the_line_in_and_full_art_pushes_it_out() {
        let gem = diamond().reach(GEM_BOX, false).unwrap();
        let full = line_between(square(112).reach(POINTS_BOX, true).unwrap(), gem);
        let padded = line_between(disc(112, 20).reach(POINTS_BOX, true).unwrap(), gem);
        assert!(full.x1 + full.y1 > padded.x1 + padded.y1);
    }

    #[test]
    fn the_line_spans_the_marks_and_no_further() {
        let gem = diamond().reach(GEM_BOX, false).unwrap();
        let ring = Alpha::default_points(96).reach(POINTS_BOX, true).unwrap();
        let line = line_between(ring, gem);
        let span = line.x1 - line.y1;
        assert!(span <= ring.d_max.max(gem.d_max) + 0.01);
        // As wide as the ring, the wider of the two: about 11 px of line.
        let length = ((line.x1 - line.x2).powi(2) + (line.y1 - line.y2).powi(2)).sqrt();
        assert!((10.5..=11.5).contains(&length), "{length}");
        // A full-bleed square spans its whole box corner to corner.
        let sq = line_between(square(112).reach(POINTS_BOX, true).unwrap(), gem);
        let sq_length = ((sq.x1 - sq.x2).powi(2) + (sq.y1 - sq.y2).powi(2)).sqrt();
        assert!(sq_length > 15.0, "{sq_length}");
    }

    /// Prints the line for local art, for the screenshot harness:
    /// `SN_GLYPH_GEM=gem.png SN_GLYPH_ART=a.png,b.png,default cargo test print_lines -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_lines() {
        let gem = decode(&std::fs::read(std::env::var("SN_GLYPH_GEM").unwrap()).unwrap()).unwrap();
        let gem = gem.reach(GEM_BOX, false).unwrap();
        for art in std::env::var("SN_GLYPH_ART").unwrap().split(',') {
            let alpha = if art == "default" { Alpha::default_points(96) } else { decode(&std::fs::read(art).unwrap()).unwrap() };
            let line = line_between(alpha.reach(POINTS_BOX, true).unwrap(), gem);
            println!("LINE {art} {} {} {} {}", line.x1, line.y1, line.x2, line.y2);
        }
    }

    #[test]
    fn the_fallback_is_the_ring_against_the_diamond() {
        let line = line_between(Alpha::default_points(96).reach(POINTS_BOX, true).unwrap(), diamond().reach(GEM_BOX, false).unwrap());
        let f = SplitLine::FALLBACK;
        for (a, b) in [(line.x1, f.x1), (line.y1, f.y1), (line.x2, f.x2), (line.y2, f.y2)] {
            assert!((a - b).abs() < 0.35, "measured {line:?}, fallback {f:?}");
        }
    }
}
