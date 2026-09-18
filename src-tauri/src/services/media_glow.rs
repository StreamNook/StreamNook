//! The one colour a stream reads as, from its thumbnail.
//!
//! Cider tints its chrome from the artwork of the current track. StreamNook has
//! no artwork, so the nearest thing is the card's thumbnail: decoded once, never
//! changing, and the same answer for every surface that shows that stream. The
//! page scales it to 16x9 and sends 576 bytes; choosing the colour and caching
//! it lives here, so scrolling a card out of a grid and back does not resample
//! and two surfaces showing one stream cannot disagree about its colour.
//!
//! This is the STILL path, and deliberately the only one. The immersive light
//! that tracks a playing frame does NOT come through here: it has to resample
//! every presented frame to look attached to the picture, and a frame across
//! IPC at that rate is exactly the bulk media copy the efficiency standard
//! forbids. It computes its own colours in the page (`src/utils/
//! mediaGlowColor.ts`), and it wants different arithmetic anyway — see the note
//! on `dominant` below.
//!
//! Keyed by the composite slot key (`makeKey`) or a thumbnail URL, never a bare
//! Twitch login — two tiles can show the same channel on different providers.

use std::collections::{HashMap, VecDeque};

use lazy_static::lazy_static;
use tokio::sync::Mutex;

/// How many slots keep a colour. Live tiles are bounded by the grid, but the
/// Home page keys one entry per thumbnail and scrolls forever, so this is a
/// FIFO cap rather than a formality: a few hundred entries of twelve bytes,
/// and the oldest are the ones furthest up a grid nobody is looking at.
const MAX_TRACKED: usize = 256;

/// How far each new sample pulls the stored colour. A thumbnail is sampled once
/// and lands whole; this only matters when the same key is fed repeatedly, and
/// then it keeps the card from stepping between two near-identical answers.
const SMOOTHING: f32 = 0.30;

/// Bits kept per channel when bucketing. Four gives 16 levels per channel:
/// coarse enough that near-identical pixels land together on a 144-pixel
/// sample, fine enough to tell a blue sky from a blue jacket.
const BUCKET_BITS: u8 = 4;

/// Below this much saturation in the SOURCE, a frame has no colour to borrow
/// and its hue is noise: at s = 0.04 the hue is decided by two or three RGB
/// units of sensor grain, so forcing it up to a usable saturation does not
/// reveal a colour, it invents one — and invents a different one every sample.
/// Measured on real Twitch thumbnails, which are far dimmer than they look.
const SAT_FLOOR: f32 = 0.06;

/// Source saturation is MAPPED into the output band rather than clamped to its
/// floor. Clamping made a barely-tinted frame and a vivid one come out equally
/// saturated, which threw away the only thing that distinguishes one card from
/// another. Mapping keeps their relative difference.
const SAT_REF: f32 = 0.60;
const SAT_MIN: f32 = 0.30;
const SAT_MAX: f32 = 0.80;

/// Lightness IS clamped: it is about legibility, not character. Too dark and
/// the glow is invisible, too light and it competes with the picture.
const LIGHT_MIN: f32 = 0.45;
const LIGHT_MAX: f32 = 0.65;

#[derive(Clone, Copy)]
struct Rgb {
    r: f32,
    g: f32,
    b: f32,
}

/// What one sample yields. `None` where the image carried no colour worth
/// showing, which is a real answer: a grey thumbnail should leave the card on
/// the theme accent rather than be given an invented hue.
#[derive(Clone, Copy, Default)]
struct Bands {
    overall: Option<Rgb>,
}

/// The serialised answer. A hex string, or null where there was no colour.
#[derive(serde::Serialize)]
pub struct Glow {
    pub overall: Option<String>,
}

lazy_static! {
    static ref LIVE: Mutex<HashMap<String, Bands>> = Mutex::new(HashMap::new());
    /// Insertion order, so the cap above can evict the least recently added.
    static ref ORDER: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
}

/// Ease one channel toward a new reading. `None` for the target means the frame
/// had no colour there, so the previous one is held rather than faded to grey:
/// a momentary dark scene should not drain the light out and back.
fn ease(prev: Option<Rgb>, target: Option<Rgb>) -> Option<Rgb> {
    match (prev, target) {
        (_, None) => prev,
        // First reading lands whole: there is nothing to ease from, and easing
        // up from black would show a grey wash for several seconds.
        (None, Some(t)) => Some(t),
        (Some(p), Some(t)) => Some(Rgb {
            r: p.r + (t.r - p.r) * SMOOTHING,
            g: p.g + (t.g - p.g) * SMOOTHING,
            b: p.b + (t.b - p.b) * SMOOTHING,
        }),
    }
}

/// Feed one downscaled image, RGBA. Returns the smoothed colour, `None` where
/// it carried nothing worth showing.
pub async fn submit(key: &str, rgba: &[u8], _width: usize, _height: usize) -> Option<Glow> {
    let target = Bands { overall: dominant(rgba) };

    let mut live = LIVE.lock().await;
    let prev = live.get(key).copied().unwrap_or_default();
    let next = Bands { overall: ease(prev.overall, target.overall) };
    if live.insert(key.to_string(), next).is_none() {
        let mut order = ORDER.lock().await;
        order.push_back(key.to_string());
        while order.len() > MAX_TRACKED {
            if let Some(old) = order.pop_front() {
                live.remove(&old);
            }
        }
    }
    Some(Glow { overall: next.overall.map(hex) })
}

/// A slot closed. Dropping the entry means the next stream in that slot starts
/// from its own first frame rather than easing out of the previous channel's
/// colour.
#[allow(dead_code)]
pub async fn forget(key: &str) {
    LIVE.lock().await.remove(key);
    ORDER.lock().await.retain(|k| k != key);
}

/// Pick the colour of a frame.
///
/// A plain mean is the obvious approach and it is wrong: averaging a whole
/// frame converges on mud, because opposing hues cancel. What reads as "the
/// colour of this scene" is the most *present* colour, so pixels are bucketed
/// and each bucket scored by how many pixels it holds AND how much colour they
/// carry. A small vivid area beats a large grey one, which is why a dark scene
/// with a neon sign glows like the sign.
fn dominant(rgba: &[u8]) -> Option<Rgb> {
    if rgba.len() < 4 {
        return None;
    }
    let shift = 8 - BUCKET_BITS;
    let mut buckets: HashMap<u16, (u32, f32, f32, f32)> = HashMap::new();

    for px in rgba.chunks_exact(4) {
        // Fully transparent pixels are letterboxing, not picture.
        if px[3] < 16 {
            continue;
        }
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let id = ((px[0] >> shift) as u16) << (BUCKET_BITS * 2)
            | ((px[1] >> shift) as u16) << BUCKET_BITS
            | (px[2] >> shift) as u16;
        let e = buckets.entry(id).or_insert((0, 0.0, 0.0, 0.0));
        e.0 += 1;
        e.1 += r;
        e.2 += g;
        e.3 += b;
    }
    if buckets.is_empty() {
        return None;
    }

    let mut best: Option<(f32, Rgb)> = None;
    for (count, sr, sg, sb) in buckets.values() {
        let n = *count as f32;
        let (r, g, b) = (sr / n / 255.0, sg / n / 255.0, sb / n / 255.0);
        let (_, s, l) = to_hsl(r, g, b);

        // Near-black and near-white carry no hue worth borrowing, whatever
        // their saturation claims.
        if !(0.08..=0.94).contains(&l) {
            continue;
        }
        // Colourfulness counts for more than area, and mid-lightness pixels are
        // preferred over ones already close to the clamp band's edges.
        let score = n * (0.25 + s * 1.75) * (1.0 - (l - 0.5).abs());
        if best.map_or(true, |(bs, _)| score > bs) {
            best = Some((score, Rgb { r, g, b }));
        }
    }

    let (_, win) = best?;
    let (h, s, l) = to_hsl(win.r, win.g, win.b);
    if s < SAT_FLOOR {
        return None;
    }
    // Proportional within the band, so a washed-out scene reads as a washed-out
    // glow rather than being shouted up to match a neon one.
    let mapped = SAT_MIN + ((s - SAT_FLOOR) / (SAT_REF - SAT_FLOOR)).clamp(0.0, 1.0) * (SAT_MAX - SAT_MIN);
    let (r, g, b) = from_hsl(h, mapped, l.clamp(LIGHT_MIN, LIGHT_MAX));
    Some(Rgb { r, g, b })
}

fn to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d.abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s.abs() < f32::EPSILON {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    (
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    )
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

fn hex(c: Rgb) -> String {
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", to8(c.r), to8(c.g), to8(c.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(px: &[(u8, u8, u8)]) -> Vec<u8> {
        px.iter().flat_map(|(r, g, b)| [*r, *g, *b, 255]).collect()
    }

    #[test]
    fn empty_and_black_frames_yield_nothing() {
        assert!(dominant(&[]).is_none());
        assert!(dominant(&frame(&[(0, 0, 0); 16])).is_none());
    }

    #[test]
    fn a_small_vivid_area_beats_a_large_grey_one() {
        // Twelve grey pixels and four saturated red ones: the red wins, which
        // is the whole point of scoring by colourfulness rather than area.
        let mut px = vec![(90u8, 90u8, 90u8); 12];
        px.extend(vec![(220u8, 30u8, 40u8); 4]);
        let (h, _, _) = {
            let c = dominant(&frame(&px)).expect("a colour");
            to_hsl(c.r, c.g, c.b)
        };
        // Red sits at either end of the hue circle.
        assert!(h < 0.05 || h > 0.95, "expected a red hue, got {h}");
    }

    #[test]
    fn lightness_is_brought_into_the_usable_band() {
        // A blinding near-white blue would wash out as a glow; it comes back
        // inside the band with its hue intact.
        let c = dominant(&frame(&[(200, 215, 255); 16])).expect("a colour");
        let (_, s, l) = to_hsl(c.r, c.g, c.b);
        assert!((LIGHT_MIN - 0.01..=LIGHT_MAX + 0.01).contains(&l), "l={l}");
        assert!(s <= SAT_MAX + 0.01, "s={s}");
    }

    #[test]
    fn a_near_grey_frame_gets_no_colour_rather_than_an_invented_one() {
        // Measured from a real Twitch thumbnail (#3a3539): its hue is decided
        // by three RGB units, so there is nothing honest to borrow.
        assert!(dominant(&frame(&[(0x3a, 0x35, 0x39); 16])).is_none());
    }

    #[test]
    fn a_washed_out_scene_stays_washed_out_next_to_a_vivid_one() {
        let dull = dominant(&frame(&[(150, 120, 110); 16])).expect("a colour");
        let vivid = dominant(&frame(&[(220, 40, 30); 16])).expect("a colour");
        let (_, dull_s, _) = to_hsl(dull.r, dull.g, dull.b);
        let (_, vivid_s, _) = to_hsl(vivid.r, vivid.g, vivid.b);
        assert!(dull_s < vivid_s, "dull={dull_s} vivid={vivid_s}");
    }

    #[tokio::test]
    async fn first_sample_lands_whole_then_eases() {
        forget("k").await;
        let red = frame(&[(200, 40, 40); 16]);
        let first = submit("k", &red, 16, 1).await.unwrap().overall.expect("a colour");

        // A completely different frame moves the colour, but not all the way.
        let blue = frame(&[(40, 60, 200); 16]);
        let second = submit("k", &blue, 16, 1).await.unwrap().overall.expect("a colour");
        assert_ne!(first, second, "the colour should move toward the new frame");

        let third = submit("k", &blue, 16, 1).await.unwrap().overall.expect("a colour");
        assert_ne!(second, third, "and keep moving while the frame holds");
        forget("k").await;
    }

    #[tokio::test]
    async fn a_dark_band_holds_its_last_colour_rather_than_draining() {
        forget("hold").await;
        let lit = frame(&[(200, 60, 50); 12]);
        let first = submit("hold", &lit, 4, 3).await.unwrap().overall.clone().expect("a colour");
        // An all-black image has nothing to say; the card should not flicker off.
        let dark = frame(&[(0, 0, 0); 12]);
        let after = submit("hold", &dark, 4, 3).await.unwrap().overall.clone().expect("held");
        assert_eq!(first, after, "a dark frame must not drain the colour");
        forget("hold").await;
    }
}
