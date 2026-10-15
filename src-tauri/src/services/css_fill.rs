//! CSS backgrounds as data a native renderer can draw.
//!
//! An Atmosphere's look is written in the `atmospheres` table as CSS (`swatch`,
//! `base_layers`, `layers`, `chat_edge`, `chat_rim`), which the web pages paint
//! as is. A native shell has no CSS engine, so the core reads those strings into
//! layers once: gradients with their stops resolved, images, solid colours. The
//! grammar is the small subset the catalog uses (linear, repeating linear and
//! radial gradients, `url(...)` with `center / cover`, hex and rgb colours).
//! Anything else is left out rather than guessed at.

use serde::{Deserialize, Serialize};

/// A CSS background: its layers top first, as CSS lists them.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Fill {
    #[serde(default)]
    pub layers: Vec<FillLayer>,
}

impl Fill {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FillLayer {
    /// `angle` in degrees, CSS convention: 0 points up, 90 to the right.
    Linear { angle: f32, repeating: bool, stops: Vec<FillStop> },
    /// `size` is the ending shape's radii as fractions of the box (width,
    /// height); None means CSS's default extent, the farthest corner. `center`
    /// is a fraction of the box.
    Radial {
        circle: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<[f32; 2]>,
        center: [f32; 2],
        repeating: bool,
        stops: Vec<FillStop>,
    },
    /// An image; `cover` scales it to fill the box, centred.
    Image { url: String, cover: bool },
    Solid { color: String },
}

/// A gradient stop. `at` is a fraction of the gradient line, or a length in CSS
/// pixels when `px` is set (repeating gradients written in px).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FillStop {
    /// `#rrggbbaa`.
    pub color: String,
    pub at: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub px: bool,
}

/// Reads a CSS background value. `none`, empty and unreadable input give an
/// empty fill.
pub fn parse(css: &str) -> Fill {
    let css = css.trim();
    if css.is_empty() || css.eq_ignore_ascii_case("none") {
        return Fill::default();
    }
    Fill { layers: split_top_level(css, ',').into_iter().filter_map(|l| layer(l.trim())).collect() }
}

/// A CSS colour as `#rrggbbaa`, for the plain colour fields.
pub fn color(css: &str) -> Option<String> {
    parse_color(css.trim()).map(|c| c.hex())
}

/// An `"r, g, b"` triple (the catalog's accent format) as `#rrggbbff`.
pub fn rgb_triple(css: &str) -> Option<String> {
    let parts: Vec<f32> = css.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    (parts.len() == 3).then(|| Rgba { r: parts[0], g: parts[1], b: parts[2], a: 1.0 }.hex())
}

fn layer(css: &str) -> Option<FillLayer> {
    let lower = css.to_ascii_lowercase();
    for (prefix, repeating, radial) in [
        ("repeating-linear-gradient(", true, false),
        ("linear-gradient(", false, false),
        ("repeating-radial-gradient(", true, true),
        ("radial-gradient(", false, true),
    ] {
        if lower.starts_with(prefix) {
            let inner = inside_parens(&css[prefix.len() - 1..])?;
            return if radial { radial_layer(inner, repeating) } else { linear_layer(inner, repeating) };
        }
    }
    if lower.starts_with("url(") {
        let inner = inside_parens(&css[3..])?;
        let url = inner.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
        let rest = lower[3 + inner.len() + 2..].to_string();
        return (!url.is_empty()).then(|| FillLayer::Image { url, cover: rest.contains("cover") });
    }
    parse_color(css).map(|c| FillLayer::Solid { color: c.hex() })
}

fn linear_layer(inner: &str, repeating: bool) -> Option<FillLayer> {
    let mut args = split_top_level(inner, ',');
    let mut angle = 180.0;
    if let Some(first) = args.first() {
        if let Some(a) = direction(first.trim()) {
            angle = a;
            args.remove(0);
        }
    }
    let stops = stops(&args)?;
    Some(FillLayer::Linear { angle, repeating, stops })
}

fn radial_layer(inner: &str, repeating: bool) -> Option<FillLayer> {
    let mut args = split_top_level(inner, ',');
    let mut circle = false;
    let mut size = None;
    let mut center = [0.5, 0.5];
    if let Some(first) = args.first().map(|s| s.trim().to_ascii_lowercase()) {
        if parse_color_stop(&first).is_none() {
            let (shape, at) = match first.split_once(" at ") {
                Some((s, a)) => (s.trim().to_string(), Some(a.trim().to_string())),
                None if first.starts_with("at ") => (String::new(), Some(first[3..].trim().to_string())),
                None => (first.clone(), None),
            };
            let mut lengths = Vec::new();
            for word in shape.split_whitespace() {
                match word {
                    "circle" => circle = true,
                    "ellipse" => {}
                    w => {
                        if let Some(p) = w.strip_suffix('%').and_then(|v| v.parse::<f32>().ok()) {
                            lengths.push(p / 100.0);
                        }
                    }
                }
            }
            if lengths.len() == 2 {
                size = Some([lengths[0], lengths[1]]);
            } else if lengths.len() == 1 {
                size = Some([lengths[0], lengths[0]]);
            }
            if let Some(at) = at {
                center = position(&at);
            }
            args.remove(0);
        }
    }
    let stops = stops(&args)?;
    Some(FillLayer::Radial { circle, size, center, repeating, stops })
}

/// `135deg`, `0.25turn`, `to bottom` and friends, as CSS degrees.
fn direction(arg: &str) -> Option<f32> {
    let a = arg.to_ascii_lowercase();
    if let Some(v) = a.strip_suffix("deg") {
        return v.trim().parse().ok();
    }
    if let Some(v) = a.strip_suffix("turn") {
        return v.trim().parse::<f32>().ok().map(|t| t * 360.0);
    }
    if let Some(v) = a.strip_suffix("rad") {
        return v.trim().parse::<f32>().ok().map(f32::to_degrees);
    }
    let sides = a.strip_prefix("to ")?;
    let has = |s: &str| sides.split_whitespace().any(|w| w == s);
    Some(match (has("top"), has("bottom"), has("left"), has("right")) {
        (true, _, false, false) => 0.0,
        (_, true, false, false) => 180.0,
        (false, false, _, true) => 90.0,
        (false, false, true, _) => 270.0,
        (true, _, _, true) => 45.0,
        (_, true, _, true) => 135.0,
        (_, true, true, _) => 225.0,
        (true, _, true, _) => 315.0,
        _ => return None,
    })
}

/// `50% 0%`, `center`, `top left` as a fraction of the box.
fn position(at: &str) -> [f32; 2] {
    let words: Vec<&str> = at.split_whitespace().collect();
    let axis = |w: &str| -> Option<(Option<f32>, Option<f32>)> {
        Some(match w {
            "left" => (Some(0.0), None),
            "right" => (Some(1.0), None),
            "top" => (None, Some(0.0)),
            "bottom" => (None, Some(1.0)),
            "center" => (None, None),
            _ => return None,
        })
    };
    let pct = |w: &str| w.strip_suffix('%').and_then(|v| v.parse::<f32>().ok()).map(|v| v / 100.0);
    let mut out = [0.5, 0.5];
    match words.as_slice() {
        [a, b] if pct(a).is_some() || pct(b).is_some() => {
            out[0] = pct(a).unwrap_or(0.5);
            out[1] = pct(b).unwrap_or(0.5);
        }
        words => {
            for w in words {
                if let Some((x, y)) = axis(w) {
                    if let Some(x) = x {
                        out[0] = x;
                    }
                    if let Some(y) = y {
                        out[1] = y;
                    }
                }
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Rgba {
    fn hex(&self) -> String {
        let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        format!("#{:02x}{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b), c(self.a * 255.0))
    }
}

fn parse_color(css: &str) -> Option<Rgba> {
    let s = css.trim().to_ascii_lowercase();
    match s.as_str() {
        "transparent" => return Some(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }),
        "white" => return Some(Rgba { r: 255.0, g: 255.0, b: 255.0, a: 1.0 }),
        "black" => return Some(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
        _ => {}
    }
    if let Some(hex) = s.strip_prefix('#') {
        let digits: Vec<f32> = hex.chars().map(|c| c.to_digit(16).map(|d| d as f32)).collect::<Option<_>>()?;
        let pair = |i: usize| digits[i] * 16.0 + digits[i + 1];
        return match digits.len() {
            3 | 4 => Some(Rgba {
                r: digits[0] * 17.0,
                g: digits[1] * 17.0,
                b: digits[2] * 17.0,
                a: if digits.len() == 4 { digits[3] * 17.0 / 255.0 } else { 1.0 },
            }),
            6 | 8 => Some(Rgba {
                r: pair(0),
                g: pair(2),
                b: pair(4),
                a: if digits.len() == 8 { pair(6) / 255.0 } else { 1.0 },
            }),
            _ => None,
        };
    }
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    if parts.len() < 3 {
        return None;
    }
    let channel = |p: &str| -> Option<f32> {
        match p.strip_suffix('%') {
            Some(v) => v.parse::<f32>().ok().map(|v| v * 2.55),
            None => p.parse().ok(),
        }
    };
    let alpha = match parts.get(3) {
        Some(p) => match p.strip_suffix('%') {
            Some(v) => v.parse::<f32>().ok()? / 100.0,
            None => p.parse().ok()?,
        },
        None => 1.0,
    };
    Some(Rgba { r: channel(parts[0])?, g: channel(parts[1])?, b: channel(parts[2])?, a: alpha })
}

/// A stop's colour and its positions as written (CSS allows two per stop).
fn parse_color_stop(arg: &str) -> Option<(Rgba, Vec<(f32, bool)>)> {
    let arg = arg.trim();
    let (color_part, rest) = if arg.starts_with("rgb") || arg.starts_with("RGB") {
        let close = arg.find(')')?;
        (&arg[..=close], &arg[close + 1..])
    } else {
        match arg.find(char::is_whitespace) {
            Some(i) => (&arg[..i], &arg[i..]),
            None => (arg, ""),
        }
    };
    let color = parse_color(color_part)?;
    let mut at = Vec::new();
    for word in rest.split_whitespace() {
        if let Some(v) = word.strip_suffix('%').and_then(|v| v.parse::<f32>().ok()) {
            at.push((v / 100.0, false));
        } else if let Some(v) = word.strip_suffix("px").and_then(|v| v.parse::<f32>().ok()) {
            at.push((v, true));
        } else if word == "0" {
            at.push((0.0, false));
        } else {
            return None;
        }
    }
    Some((color, at))
}

/// The stops with every position filled in the way CSS does: a missing first is
/// 0, a missing last is the end, the ones between spread evenly, and a position
/// below an earlier one is raised to it. A `transparent` stop takes its
/// neighbours' colour at zero alpha (split in two between different colours),
/// which is what CSS's premultiplied blend looks like.
fn stops(args: &[&str]) -> Option<Vec<FillStop>> {
    let mut raw: Vec<(Rgba, Option<(f32, bool)>)> = Vec::new();
    for arg in args {
        let (color, at) = parse_color_stop(arg)?;
        if at.is_empty() {
            raw.push((color, None));
        }
        for p in at {
            raw.push((color, Some(p)));
        }
    }
    if raw.is_empty() {
        return None;
    }
    let n = raw.len();
    let px = raw.iter().any(|(_, p)| matches!(p, Some((_, true))));
    if raw[0].1.is_none() {
        raw[0].1 = Some((0.0, px));
    }
    if n > 1 && raw[n - 1].1.is_none() {
        raw[n - 1].1 = Some((1.0, false));
    }
    let mut i = 0;
    while i < n {
        if raw[i].1.is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let mut end = i;
        while raw[end].1.is_none() {
            end += 1;
        }
        let (a, unit) = raw[start].1.unwrap();
        let (b, _) = raw[end].1.unwrap();
        let span = (end - start) as f32;
        for (k, slot) in raw.iter_mut().enumerate().take(end).skip(start + 1) {
            slot.1 = Some((a + (b - a) * (k - start) as f32 / span, unit));
        }
        i = end;
    }
    let mut highest = f32::MIN;
    let positioned: Vec<(Rgba, f32, bool)> = raw
        .into_iter()
        .map(|(c, p)| {
            let (at, is_px) = p.unwrap_or((0.0, false));
            highest = highest.max(at);
            (c, highest, is_px)
        })
        .collect();
    let mut out = Vec::new();
    for (i, (c, at, is_px)) in positioned.iter().enumerate() {
        if c.a > 0.0 || positioned.len() == 1 {
            out.push(FillStop { color: c.hex(), at: *at, px: *is_px });
            continue;
        }
        let before = positioned[..i].iter().rev().find(|(c, ..)| c.a > 0.0).map(|(c, ..)| *c);
        let after = positioned[i + 1..].iter().find(|(c, ..)| c.a > 0.0).map(|(c, ..)| *c);
        let clear = |c: Rgba| FillStop { color: Rgba { a: 0.0, ..c }.hex(), at: *at, px: *is_px };
        match (before, after) {
            (Some(b), Some(a)) if b != a => {
                out.push(clear(b));
                out.push(clear(a));
            }
            (Some(b), _) => out.push(clear(b)),
            (None, Some(a)) => out.push(clear(a)),
            (None, None) => out.push(FillStop { color: c.hex(), at: *at, px: *is_px }),
        }
    }
    Some(out)
}

/// The text between the parenthesis that opens `s` and its match.
fn inside_parens(s: &str) -> Option<&str> {
    let mut depth = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.into_iter().filter(|p| !p.trim().is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(fill: &Fill) -> (f32, bool, &Vec<FillStop>) {
        match &fill.layers[0] {
            FillLayer::Linear { angle, repeating, stops } => (*angle, *repeating, stops),
            other => panic!("not linear: {other:?}"),
        }
    }

    #[test]
    fn the_aurora_swatch_fills_its_missing_stop_positions() {
        let fill = parse("linear-gradient(135deg, #10b981, #2dd4bf 40%, #22d3ee 70%, #8b5cf6)");
        let (angle, repeating, stops) = linear(&fill);
        assert_eq!(angle, 135.0);
        assert!(!repeating);
        let at: Vec<f32> = stops.iter().map(|s| s.at).collect();
        assert_eq!(at, vec![0.0, 0.4, 0.7, 1.0]);
        assert_eq!(stops[0].color, "#10b981ff");
    }

    #[test]
    fn repeating_px_curtains_keep_their_pixel_positions() {
        let fill = parse(
            "repeating-linear-gradient(95deg, rgba(16,185,129,0) 0px, rgba(45,212,191,0.18) 120px, rgba(34,211,238,0.10) 200px, rgba(16,185,129,0) 320px)",
        );
        let (angle, repeating, stops) = linear(&fill);
        assert_eq!(angle, 95.0);
        assert!(repeating);
        assert!(stops.iter().all(|s| s.px));
        assert_eq!(stops.last().unwrap().at, 320.0);
        assert_eq!(stops[1].color, "#2dd4bf2e");
    }

    #[test]
    fn a_radial_glow_reads_its_size_and_centre() {
        let fill = parse("radial-gradient(ellipse 120% 60% at 50% 0%, rgba(45,212,191,0.10), transparent 65%)");
        match &fill.layers[0] {
            FillLayer::Radial { circle, size, center, stops, .. } => {
                assert!(!circle);
                assert_eq!(*size, Some([1.2, 0.6]));
                assert_eq!(*center, [0.5, 0.0]);
                assert_eq!(stops.len(), 2);
                assert_eq!(stops[1].at, 0.65);
                // transparent takes its neighbour's colour at zero alpha.
                assert_eq!(stops[1].color, "#2dd4bf00");
            }
            other => panic!("not radial: {other:?}"),
        }
    }

    #[test]
    fn transparent_between_two_colours_splits_in_two() {
        let fill = parse("linear-gradient(to bottom, transparent, rgba(45,212,191,0.85) 22%, transparent 50%, #8b5cf6)");
        let (angle, _, stops) = linear(&fill);
        assert_eq!(angle, 180.0);
        assert_eq!(stops[0].color, "#2dd4bf00");
        assert_eq!(stops[2].color, "#2dd4bf00");
        assert_eq!(stops[3].color, "#8b5cf600");
        assert_eq!(stops[2].at, stops[3].at);
    }

    #[test]
    fn an_image_swatch_is_one_cover_image() {
        let fill = parse("url(https://cdn.streamnook.app/atmospheres/midnight.webp) center / cover");
        assert_eq!(
            fill.layers,
            vec![FillLayer::Image { url: "https://cdn.streamnook.app/atmospheres/midnight.webp".into(), cover: true }]
        );
    }

    #[test]
    fn layers_split_on_top_level_commas_only() {
        let fill = parse("linear-gradient(180deg, #f6e3a0, #9c7528), #04070c");
        assert_eq!(fill.layers.len(), 2);
        assert_eq!(fill.layers[1], FillLayer::Solid { color: "#04070cff".into() });
    }

    #[test]
    fn none_and_garbage_are_empty() {
        assert!(parse("none").is_empty());
        assert!(parse("").is_empty());
        assert!(parse("conic-gradient(red, blue)").is_empty());
    }

    #[test]
    fn plain_colours_and_accent_triples() {
        assert_eq!(color("#04070c").as_deref(), Some("#04070cff"));
        assert_eq!(color("#fff").as_deref(), Some("#ffffffff"));
        assert_eq!(rgb_triple("45, 212, 191").as_deref(), Some("#2dd4bfff"));
    }
}
