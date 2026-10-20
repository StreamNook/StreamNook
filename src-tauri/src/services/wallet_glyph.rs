//! How the wallet button's split glyph lays out its two marks and divider.
//!
//! A channel with both channel points and Bits shows one 24 px square: the
//! points art in a box at the upper left, the Bits gem in a box at the lower
//! right, and a hairline across the diagonal gap between them. Laid out by the
//! boxes alone it looks wrong for most art: round art leaves a wide gap at its
//! box corner, full-bleed square art leaves almost none, art with transparent
//! padding sits deep inside its box, and the gem is a tall diamond whose tip is
//! all that reaches the gap. So this reads the art itself.
//!
//! Each mark's alpha gives its nearest visible pixel to the gap, measured along
//! the glyph's main diagonal (`x + y`), and its visible bounds. Then:
//! - the points art slides toward the gem (up to `MAX_SLIDE`) until the gap
//!   between the two marks is `GAP`, so every channel's art sits the same
//!   distance from the gem;
//! - the line sits in the middle of that gap, `TOWARD_GEM` nearer the gem's tip
//!   than the art: a broad round or straight edge reads closer than a lone tip
//!   at the same distance (tuned by eye on screenshots of real channel art);
//! - it runs as long as the two boxes span across the gap, inside the square;
//! - the whole glyph shifts so its visible marks are centered in the square.
//!
//! Twitch's default points glyph is measured from its own geometry. Answers are
//! cached by the pair of URLs, so a channel's art is fetched and measured once
//! per session; until it arrives, and for art that cannot be read, the shells
//! draw `SplitLayout::FALLBACK`.

use std::collections::{HashMap, VecDeque};

use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::Mutex;

/// The glyph's square.
const GRID: f32 = 24.0;
/// Each mark's box side. The gem's box sits in the lower-right corner; the
/// art's starts in the upper-left corner and slides along the diagonal.
const MARK: f32 = 11.5;
const GEM_AT: f32 = GRID - MARK;
/// The visible gap between the two marks, nearest pixel to nearest pixel.
const GAP: f32 = 5.0;
/// How far the art may slide toward the gem to close a wider gap.
const MAX_SLIDE: f32 = 3.0;
/// The line's offset from the middle of the gap toward the gem.
const TOWARD_GEM: f32 = 0.4;
/// The line never comes nearer than this to either mark (half the 1 px
/// stroke plus a hair).
const MIN_CLEARANCE: f32 = 0.9;
/// Alpha at or above this counts as visible.
const VISIBLE_ALPHA: u8 = 40;
/// Art bigger than this is not points art; it is not fetched.
const MAX_ART_BYTES: usize = 256 * 1024;
const MAX_TRACKED: usize = 128;
const SQRT_2: f32 = std::f32::consts::SQRT_2;

/// The glyph's layout on its 24 px square.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct SplitLayout {
    /// The grid's side and each mark's box side, so the shells draw what was measured.
    pub grid: f32,
    pub mark: f32,
    /// The points art's box: its top-left corner at (`art_at`, `art_at`).
    pub art_at: f32,
    /// The gem's box: its top-left corner at (`gem_at`, `gem_at`).
    pub gem_at: f32,
    /// The divider, from its upper-right end to its lower-left.
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    /// Moves the whole glyph so its visible marks are centered.
    pub shift_x: f32,
    pub shift_y: f32,
}

impl SplitLayout {
    /// Twitch's ring against the 100-bit gem (measured on the real gem), which
    /// the shells draw until (or unless) the art is measured. The desktop,
    /// Android and iPhone copies carry these numbers too.
    pub const FALLBACK: SplitLayout = SplitLayout {
        grid: GRID,
        mark: MARK,
        art_at: 2.42,
        gem_at: GEM_AT,
        x1: 19.74,
        y1: 8.24,
        x2: 8.24,
        y2: 19.74,
        shift_x: -0.73,
        shift_y: -1.35,
    };
}

/// A mark's alpha, row by row, `w` x `h`.
struct Alpha {
    w: usize,
    h: usize,
    a: Vec<u8>,
}

/// One mark placed in its box: its nearest pixel to the gap and its visible bounds.
#[derive(Clone, Copy, Debug)]
struct Placed {
    /// The `x + y` of its pixel nearest the gap.
    near: f32,
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
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
                if (9.0..=11.0).contains(&r) || ((5.0..=7.0).contains(&r) && px >= 0.0 && py <= 0.0) {
                    a[y * side + x] = 255;
                }
            }
        }
        Alpha { w: side, h: side, a }
    }

    /// The mark drawn into the box at (`at`, `at`) with `object-fit: contain`
    /// (centered, aspect kept). `gap_is_high`: the art faces the gap with its
    /// highest `x + y`, the gem with its lowest. `None` when nothing is visible.
    fn place(&self, at: f32, gap_is_high: bool) -> Option<Placed> {
        let scale = MARK / self.w.max(self.h) as f32;
        let off_x = at + (MARK - self.w as f32 * scale) / 2.0;
        let off_y = at + (MARK - self.h as f32 * scale) / 2.0;
        let half = scale / 2.0;
        let mut p = Placed { near: if gap_is_high { f32::MIN } else { f32::MAX }, min_x: f32::MAX, min_y: f32::MAX, max_x: f32::MIN, max_y: f32::MIN };
        let mut any = false;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.a[y * self.w + x] < VISIBLE_ALPHA {
                    continue;
                }
                any = true;
                let (gx, gy) = (off_x + (x as f32 + 0.5) * scale, off_y + (y as f32 + 0.5) * scale);
                // The pixel's own edge, not its center, is what shows.
                p.near = if gap_is_high { p.near.max(gx + gy + scale) } else { p.near.min(gx + gy - scale) };
                p.min_x = p.min_x.min(gx - half);
                p.min_y = p.min_y.min(gy - half);
                p.max_x = p.max_x.max(gx + half);
                p.max_y = p.max_y.max(gy + half);
            }
        }
        any.then_some(p)
    }

    fn moved(p: Placed, by: f32) -> Placed {
        Placed { near: p.near + 2.0 * by, min_x: p.min_x + by, min_y: p.min_y + by, max_x: p.max_x + by, max_y: p.max_y + by }
    }
}

fn round(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// The layout for the points art against the gem.
fn layout(art: &Alpha, gem: &Alpha) -> Option<SplitLayout> {
    let gem_p = gem.place(GEM_AT, false)?;
    let art_0 = art.place(0.0, true)?;
    // `x + y` moves sqrt(2) per pixel of distance across the gap, and sliding
    // the art by d along the diagonal moves its `x + y` by 2d.
    let slide = ((gem_p.near - art_0.near - GAP * SQRT_2) / 2.0).clamp(0.0, MAX_SLIDE);
    let art_p = Alpha::moved(art_0, slide);
    let clear = MIN_CLEARANCE * SQRT_2;
    let (low, high) = (art_p.near + clear, gem_p.near - clear);
    let s = (art_p.near + gem_p.near) / 2.0 + TOWARD_GEM * SQRT_2;
    let s = if low <= high { s.clamp(low, high) } else { (art_p.near + gem_p.near) / 2.0 };
    // As long as the boxes span across the gap (x - y from -MARK to MARK), kept inside the square.
    let reach = MARK.min(GRID - (s - GRID).abs());
    let min_x = art_p.min_x.min(gem_p.min_x);
    let max_x = art_p.max_x.max(gem_p.max_x);
    let min_y = art_p.min_y.min(gem_p.min_y);
    let max_y = art_p.max_y.max(gem_p.max_y);
    Some(SplitLayout {
        grid: GRID,
        mark: MARK,
        art_at: round(slide),
        gem_at: GEM_AT,
        x1: round((s + reach) / 2.0),
        y1: round((s - reach) / 2.0),
        x2: round((s - reach) / 2.0),
        y2: round((s + reach) / 2.0),
        shift_x: round(GRID / 2.0 - (min_x + max_x) / 2.0),
        shift_y: round(GRID / 2.0 - (min_y + max_y) / 2.0),
    })
}

fn decode(bytes: &[u8]) -> Option<Alpha> {
    let (w, h, rgba) = crate::services::badge_service::decode_rgba(bytes)?;
    Some(Alpha::from_rgba(w, h, &rgba))
}

static CACHE: Lazy<Mutex<(HashMap<String, SplitLayout>, VecDeque<String>)>> = Lazy::new(Default::default);

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

/// The layout for this pair of marks: `points_icon` is the channel's own
/// points art (None for Twitch's glyph), `gem` the Bits gem. Art that cannot
/// be fetched or read gets the fallback, and is tried again next time.
pub async fn split_layout(points_icon: Option<&str>, gem: &str) -> SplitLayout {
    let key = format!("{}|{gem}", points_icon.unwrap_or(""));
    if let Some(found) = CACHE.lock().await.0.get(&key) {
        return *found;
    }
    let points_bytes = match points_icon {
        Some(url) => match fetch(url).await {
            Some(b) => Some(b),
            None => return SplitLayout::FALLBACK,
        },
        None => None,
    };
    let Some(gem_bytes) = fetch(gem).await else {
        return SplitLayout::FALLBACK;
    };
    let measured = tokio::task::spawn_blocking(move || {
        let art = match points_bytes {
            Some(b) => decode(&b)?,
            None => Alpha::default_points(96),
        };
        layout(&art, &decode(&gem_bytes)?)
    })
    .await
    .ok()
    .flatten();
    let Some(found) = measured else {
        return SplitLayout::FALLBACK;
    };
    let mut guard = CACHE.lock().await;
    let (map, order) = &mut *guard;
    if map.insert(key.clone(), found).is_none() {
        order.push_back(key);
        while order.len() > MAX_TRACKED {
            if let Some(old) = order.pop_front() {
                map.remove(&old);
            }
        }
    }
    found
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

    /// The visible gaps on each side of the line, in px: (art side, gem side).
    fn gaps(art: &Alpha, gem: &Alpha, l: SplitLayout) -> (f32, f32) {
        let s = l.x1 + l.y1;
        let a = art.place(l.art_at, true).unwrap();
        let g = gem.place(l.gem_at, false).unwrap();
        ((s - a.near) / SQRT_2, (g.near - s) / SQRT_2)
    }

    fn arts() -> Vec<(&'static str, Alpha)> {
        vec![("square", square(112)), ("disc", disc(112, 0)), ("padded", disc(112, 24)), ("ring", Alpha::default_points(96))]
    }

    #[test]
    fn every_kind_of_art_sits_the_same_gap_from_the_gem() {
        let gem = diamond();
        for (name, art) in arts() {
            let l = layout(&art, &gem).unwrap();
            let (a, g) = gaps(&art, &gem, l);
            // The gap is GAP wherever the slide could close it (not at its limit).
            assert!((a + g - GAP).abs() < 0.05 || l.art_at >= MAX_SLIDE || l.art_at == 0.0, "{name}: {a} + {g} with {l:?}");
            assert!(a >= MIN_CLEARANCE - 0.01 && g >= MIN_CLEARANCE - 0.01, "{name}: the line hugs a mark ({a}, {g})");
            // Nearer the gem's tip than the art, by about twice TOWARD_GEM.
            assert!(a > g, "{name}: {a} vs {g}");
            // A diagonal, inside the square.
            assert!(((l.x1 + l.y1) - (l.x2 + l.y2)).abs() < 0.02);
            for v in [l.x1, l.y1, l.x2, l.y2] {
                assert!((0.0..=GRID).contains(&v), "{name}: {l:?}");
            }
        }
    }

    #[test]
    fn padded_art_slides_further_than_full_bleed_art() {
        let gem = diamond();
        let full = layout(&square(112), &gem).unwrap();
        let padded = layout(&disc(112, 24), &gem).unwrap();
        assert!(padded.art_at > full.art_at, "{padded:?} vs {full:?}");
        assert!(padded.art_at <= MAX_SLIDE);
    }

    #[test]
    fn the_line_spans_both_boxes() {
        let l = layout(&Alpha::default_points(96), &diamond()).unwrap();
        let length = ((l.x1 - l.x2).powi(2) + (l.y1 - l.y2).powi(2)).sqrt();
        assert!((length - MARK * SQRT_2).abs() < 0.1, "{length}");
    }

    #[test]
    fn the_marks_end_up_centered() {
        for (name, art) in arts() {
            let l = layout(&art, &diamond()).unwrap();
            let a = art.place(l.art_at, true).unwrap();
            let g = diamond().place(l.gem_at, false).unwrap();
            let cx = (a.min_x.min(g.min_x) + a.max_x.max(g.max_x)) / 2.0 + l.shift_x;
            let cy = (a.min_y.min(g.min_y) + a.max_y.max(g.max_y)) / 2.0 + l.shift_y;
            assert!((cx - GRID / 2.0).abs() < 0.02 && (cy - GRID / 2.0).abs() < 0.02, "{name}: {cx}, {cy}");
        }
    }

    #[test]
    fn fallback_is_the_ring_against_the_gem() {
        let l = layout(&Alpha::default_points(96), &diamond()).unwrap();
        let f = SplitLayout::FALLBACK;
        for (a, b) in [(l.art_at, f.art_at), (l.x1, f.x1), (l.y1, f.y1), (l.x2, f.x2), (l.y2, f.y2), (l.shift_x, f.shift_x), (l.shift_y, f.shift_y)] {
            assert!((a - b).abs() < 0.3, "measured {l:?}, fallback {f:?}");
        }
    }

    /// Prints the layout for local art, for the screenshot harness:
    /// `SN_GLYPH_GEM=gem.png SN_GLYPH_ART=default,a.png cargo test print_layouts -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_layouts() {
        let gem = decode(&std::fs::read(std::env::var("SN_GLYPH_GEM").unwrap()).unwrap()).unwrap();
        for art in std::env::var("SN_GLYPH_ART").unwrap().split(',') {
            let alpha = if art == "default" { Alpha::default_points(96) } else { decode(&std::fs::read(art).unwrap()).unwrap() };
            println!("LAYOUT {art} {}", serde_json::to_string(&layout(&alpha, &gem).unwrap()).unwrap());
        }
    }
}
