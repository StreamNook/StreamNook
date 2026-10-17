//! The colour themes: the built-in palettes (one JSON file the desktop reads
//! too), the viewer's own custom themes from settings, and the OLED theme's
//! accent-derived colours, resolved into the palette a native shell paints.

use once_cell::sync::Lazy;
use serde_json::{json, Map, Value};

/// The built-in palettes and their gallery categories, shared with
/// `src/themes/index.ts`.
static BUILT_IN: Lazy<Value> = Lazy::new(|| {
    serde_json::from_str(include_str!("../../../src/themes/builtinThemes.json")).unwrap_or_else(|_| json!({}))
});

pub const DEFAULT_THEME_ID: &str = "winters-glass";
pub const OLED_THEME_ID: &str = "prince0fdubai-oled";
pub const DEFAULT_OLED_ACCENT: &str = "#a064ff";
/// The one-tap OLED accents, in the order the card shows them.
pub const OLED_ACCENT_PRESETS: &[(&str, &str)] = &[
    ("Purple", "#a064ff"),
    ("Orange", "#ff9933"),
    ("Blue", "#66b3ff"),
    ("Cyan", "#22d3ee"),
    ("Green", "#22c55e"),
    ("Pink", "#ff66b2"),
    ("Red", "#ff4466"),
    ("Gold", "#ffcc00"),
];

pub fn built_in_themes() -> &'static [Value] {
    BUILT_IN.get("themes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

pub fn categories() -> Value {
    BUILT_IN.get("categories").cloned().unwrap_or_else(|| json!([]))
}

fn hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((byte(0)?, byte(2)?, byte(4)?))
}

/// Mixes a `#rrggbb` toward white by `percent`.
fn lighten(hex: &str, percent: f64) -> String {
    let Some((r, g, b)) = hex_rgb(hex) else { return hex.to_string() };
    let mix = |c: u8| ((c as f64) + (255.0 - c as f64) * percent / 100.0).round().min(255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", mix(r), mix(g), mix(b))
}

/// The OLED theme for a chosen accent: hover, muted, secondary text and the
/// scrollbar follow it. An invalid accent falls back to the default purple.
pub fn oled_palette(base: &Value, accent: Option<&str>) -> Value {
    let accent = accent.filter(|a| hex_rgb(a).is_some()).unwrap_or(DEFAULT_OLED_ACCENT).to_lowercase();
    let (r, g, b) = hex_rgb(&accent).unwrap_or((160, 100, 255));
    let mut palette = base.clone();
    if let Some(p) = palette.as_object_mut() {
        p.insert("accent".into(), json!(accent));
        p.insert("accentHover".into(), json!(lighten(&accent, 20.0)));
        p.insert("accentMuted".into(), json!(format!("rgba({r}, {g}, {b}, 0.5)")));
        p.insert("textSecondary".into(), json!(lighten(&accent, 45.0)));
        p.insert("scrollbarThumb".into(), json!(format!("rgba({r}, {g}, {b}, 0.25)")));
    }
    palette
}

/// A custom theme colour (`{ value: "#rrggbb", opacity: 0..100 }`) as CSS.
fn custom_color(color: &Value) -> Value {
    let value = color.get("value").and_then(Value::as_str).unwrap_or("#000000");
    let opacity = color.get("opacity").and_then(Value::as_f64).unwrap_or(100.0);
    match hex_rgb(value) {
        Some((r, g, b)) if opacity < 100.0 => json!(format!("rgba({r}, {g}, {b}, {})", opacity / 100.0)),
        _ => json!(value),
    }
}

/// A saved custom theme's palette in the built-in palette's shape.
pub fn custom_palette(custom: &Value) -> Value {
    let p = custom.get("palette").cloned().unwrap_or(Value::Null);
    let mut out = Map::new();
    if let Some(fields) = p.as_object() {
        for (key, value) in fields {
            match key.as_str() {
                "glassOpacity" | "glassHoverOpacity" | "glassActiveOpacity" => {
                    out.insert(key.clone(), value.clone());
                }
                "highlight" => {
                    let mut hl = Map::new();
                    for (name, color) in value.as_object().into_iter().flatten() {
                        hl.insert(name.clone(), custom_color(color));
                    }
                    out.insert(key.clone(), Value::Object(hl));
                }
                _ => {
                    out.insert(key.clone(), custom_color(value));
                }
            }
        }
    }
    Value::Object(out)
}

/// Whether text on this background must be light (relative luminance below
/// the midpoint).
pub fn needs_light_content(background: &str) -> bool {
    let rgb = hex_rgb(background).or_else(|| {
        let inner = background.split('(').nth(1)?.trim_end_matches(')');
        let parts: Vec<u8> = inner.split(',').take(3).filter_map(|p| p.trim().parse::<f64>().ok().map(|v| v as u8)).collect();
        (parts.len() == 3).then(|| (parts[0], parts[1], parts[2]))
    });
    let Some((r, g, b)) = rgb else { return true };
    (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0 < 0.5
}

/// The least contrast muted text keeps against its theme's background: WCAG AA for body text.
pub const MUTED_CONTRAST_FLOOR: f64 = 4.5;

/// A CSS colour (`#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`) as red, green and blue from 0
/// to 255 and alpha from 0 to 1.
fn css_rgba(color: &str) -> Option<(f64, f64, f64, f64)> {
    let c = color.trim().to_ascii_lowercase();
    if let Some(h) = c.strip_prefix('#') {
        let h: String = if h.len() == 3 { h.chars().flat_map(|ch| [ch, ch]).collect() } else { h.to_string() };
        if !(h.len() == 6 || h.len() == 8) || !h.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(f64::from);
        let a = if h.len() == 8 { byte(6)? / 255.0 } else { 1.0 };
        return Some((byte(0)?, byte(2)?, byte(4)?, a));
    }
    let inner = c.strip_prefix("rgba(").or_else(|| c.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<f64> = inner
        .split(|ch: char| ch == ',' || ch == '/' || ch.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<f64>().ok())
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [r, g, b] => Some((*r, *g, *b, 1.0)),
        [r, g, b, a, ..] => Some((*r, *g, *b, *a)),
        _ => None,
    }
}

/// WCAG relative luminance of an sRGB colour (channels 0 to 255).
fn luminance(r: f64, g: f64, b: f64) -> f64 {
    let lin = |c: f64| {
        let v = c / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// The contrast of `fg` drawn at `alpha` over the opaque background `bg`.
fn contrast_at(fg: (f64, f64, f64), bg: (f64, f64, f64), alpha: f64) -> f64 {
    let mix = |f: f64, b: f64| f * alpha + b * (1.0 - alpha);
    let l1 = luminance(mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2));
    let l2 = luminance(bg.0, bg.1, bg.2);
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

/// The contrast of a palette's muted text against its background, when both read.
pub fn muted_contrast(palette: &Value) -> Option<f64> {
    let bg = css_rgba(palette.get("background")?.as_str()?)?;
    let fg = css_rgba(palette.get("textMuted")?.as_str()?)?;
    Some(contrast_at((fg.0, fg.1, fg.2), (bg.0, bg.1, bg.2), fg.3))
}

/// The least alpha, in hundredths, at which `fg` reaches the floor over the opaque `bg`, or `None`
/// when it already does at its own alpha. One that cannot reach it even opaque gets 100.
fn floor_hundredths(fg: (f64, f64, f64, f64), bg: (f64, f64, f64, f64)) -> Option<u32> {
    let rgb = (fg.0, fg.1, fg.2);
    let back = (bg.0, bg.1, bg.2);
    if contrast_at(rgb, back, fg.3) >= MUTED_CONTRAST_FLOOR {
        return None;
    }
    let start = (fg.3 * 100.0).round() as u32;
    Some((start..=100).find(|h| contrast_at(rgb, back, *h as f64 / 100.0) >= MUTED_CONTRAST_FLOOR).unwrap_or(100))
}

/// Lifts each saved custom theme's text to both floors, in place: muted text's opacity
/// (`palette.textMuted.opacity`, 0 to 100) against the background, then secondary text's color
/// (and, only if it must, its opacity) on the raised surface. Every shell that paints the theme
/// from settings then shows the lifted values without a contrast rule of its own. Returns
/// whether any theme changed.
pub fn lift_custom_themes(custom_themes: &mut Value) -> bool {
    let mut changed = false;
    for theme in custom_themes.as_array_mut().into_iter().flatten() {
        let palette = custom_palette(theme);
        if let (Some(bg), Some(fg)) = (
            palette.get("background").and_then(Value::as_str).and_then(css_rgba),
            palette.get("textMuted").and_then(Value::as_str).and_then(css_rgba),
        ) {
            if let Some(hundredths) = floor_hundredths(fg, bg) {
                if let Some(muted) = theme.pointer_mut("/palette/textMuted").and_then(Value::as_object_mut) {
                    muted.insert("opacity".into(), json!(hundredths));
                    changed = true;
                }
            }
        }
        // Read again: the secondary floor is measured against the lifted muted text.
        let Some((r, g, b, a)) = palette_secondary_floor(&custom_palette(theme)) else { continue };
        if let Some(sec) = theme.pointer_mut("/palette/textSecondary").and_then(Value::as_object_mut) {
            sec.insert("value".into(), json!(format!("#{:02x}{:02x}{:02x}", r as u8, g as u8, b as u8)));
            if a >= 1.0 {
                sec.insert("opacity".into(), json!(100));
            }
            changed = true;
        }
    }
    changed
}

/// Lifts the palette's muted text to the floor: the same colour at the least alpha (in hundredths)
/// that reaches `MUTED_CONTRAST_FLOOR` against the background. Muted text already at or above it
/// is left exactly as written; one that cannot reach it even opaque goes opaque. The built-in
/// palettes ship at the floor already (a test pins it), so this only ever changes custom themes.
pub fn lift_muted(palette: &mut Value) {
    let (Some(bg), Some(fg)) = (
        palette.get("background").and_then(Value::as_str).and_then(css_rgba),
        palette.get("textMuted").and_then(Value::as_str).and_then(css_rgba),
    ) else {
        return;
    };
    let Some(hundredths) = floor_hundredths(fg, bg) else { return };
    let alpha = format!("{:.2}", hundredths as f64 / 100.0);
    let alpha = alpha.trim_end_matches('0').trim_end_matches('.');
    let lifted = format!("rgba({}, {}, {}, {alpha})", fg.0.round(), fg.1.round(), fg.2.round());
    if let Some(p) = palette.as_object_mut() {
        p.insert("textMuted".into(), json!(lifted));
    }
}

/// The least contrast secondary text keeps on a raised surface (the palette's surface over its
/// background), where segment labels and card details sit. Secondary text is also never quieter
/// there than muted text.
pub const SECONDARY_CONTRAST_FLOOR: f64 = 4.5;

/// `fg` (with its alpha) composited over the opaque `bg`.
fn over(fg: (f64, f64, f64, f64), bg: (f64, f64, f64)) -> (f64, f64, f64) {
    let mix = |f: f64, b: f64| f * fg.3 + b * (1.0 - fg.3);
    (mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2))
}

fn ratio(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let (l1, l2) = (luminance(a.0, a.1, a.2), luminance(b.0, b.1, b.2));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let round = |x: f64| (x + 0.5).floor();
    if s == 0.0 {
        let v = round(l * 255.0);
        return (v, v, v);
    }
    let hue = |p: f64, q: f64, mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    (round(hue(p, q, h + 1.0 / 3.0) * 255.0), round(hue(p, q, h) * 255.0), round(hue(p, q, h - 1.0 / 3.0) * 255.0))
}

/// Secondary text at the floor, or `None` when it already is: the same hue and saturation with
/// its HSL lightness moved away from the surface in 0.5% steps until it reaches
/// `SECONDARY_CONTRAST_FLOOR` and muted text's own contrast there. One that cannot at its alpha
/// tries again opaque; one that cannot at all becomes white or black.
fn floor_secondary(
    sec: (f64, f64, f64, f64),
    bg: (f64, f64, f64),
    surface: (f64, f64, f64, f64),
    muted: (f64, f64, f64, f64),
) -> Option<(f64, f64, f64, f64)> {
    let track = over(surface, bg);
    let target = SECONDARY_CONTRAST_FLOOR.max(ratio(over(muted, track), track));
    let ok = |c: (f64, f64, f64, f64)| ratio(over(c, track), track) >= target;
    if ok(sec) {
        return None;
    }
    let up = ratio((255.0, 255.0, 255.0), track) >= ratio((0.0, 0.0, 0.0), track);
    let (h, s, l) = rgb_to_hsl(sec.0, sec.1, sec.2);
    let alphas: &[f64] = if sec.3 < 1.0 { &[sec.3, 1.0] } else { &[1.0] };
    for &alpha in alphas {
        for step in 0..=200 {
            let nl = if up { (l + step as f64 * 0.005).min(1.0) } else { (l - step as f64 * 0.005).max(0.0) };
            let (r, g, b) = hsl_to_rgb(h, s, nl);
            if ok((r, g, b, alpha)) {
                return Some((r, g, b, alpha));
            }
            if nl == 0.0 || nl == 1.0 {
                break;
            }
        }
    }
    Some(if up { (255.0, 255.0, 255.0, 1.0) } else { (0.0, 0.0, 0.0, 1.0) })
}

/// A palette's secondary text at the floor, read from its CSS colours.
fn palette_secondary_floor(palette: &Value) -> Option<(f64, f64, f64, f64)> {
    let color = |key: &str| palette.get(key).and_then(Value::as_str).and_then(css_rgba);
    let bg = color("background")?;
    floor_secondary(color("textSecondary")?, (bg.0, bg.1, bg.2), color("surface")?, color("textMuted")?)
}

/// Lifts the palette's secondary text to its floor (see `floor_secondary`). Secondary text
/// already there is left exactly as written; the built-ins ship at the floor (a test pins it).
pub fn lift_secondary(palette: &mut Value) {
    let Some((r, g, b, a)) = palette_secondary_floor(palette) else { return };
    let lifted = if a >= 1.0 {
        format!("#{:02x}{:02x}{:02x}", r as u8, g as u8, b as u8)
    } else {
        let alpha = format!("{a:.2}");
        format!("rgba({r}, {g}, {b}, {})", alpha.trim_end_matches('0').trim_end_matches('.'))
    };
    if let Some(p) = palette.as_object_mut() {
        p.insert("textSecondary".into(), json!(lifted));
    }
}

/// Both text floors: muted against the background, then secondary on the raised surface (which
/// reads the lifted muted text).
pub fn lift_text(palette: &mut Value) {
    lift_muted(palette);
    lift_secondary(palette);
}

/// The theme a shell paints.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ResolvedTheme {
    pub id: String,
    pub name: String,
    pub palette: Value,
    /// Status-bar and other system content should be light on this background.
    pub light_content: bool,
    pub effect: Option<String>,
}

/// Resolves a theme id against the custom themes and built-ins (custom first),
/// falling back to the default theme for an unknown id.
pub fn resolve(id: &str, custom_themes: &[Value], oled_accent: Option<&str>) -> ResolvedTheme {
    let (id, name, mut palette, effect) = if let Some(custom) =
        custom_themes.iter().find(|t| t.get("id").and_then(Value::as_str) == Some(id))
    {
        let name = custom.get("name").and_then(Value::as_str).unwrap_or("Custom").to_string();
        (id.to_string(), name, custom_palette(custom), None)
    } else {
        let theme = built_in_themes()
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
            .or_else(|| built_in_themes().iter().find(|t| t.get("id").and_then(Value::as_str) == Some(DEFAULT_THEME_ID)))
            .cloned()
            .unwrap_or(Value::Null);
        let text = |key: &str| theme.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        (text("id"), text("name"), theme.get("palette").cloned().unwrap_or(Value::Null), theme.get("effect").and_then(Value::as_str).map(str::to_string))
    };
    if id == OLED_THEME_ID {
        palette = oled_palette(&palette, oled_accent);
    }
    lift_text(&mut palette);
    let light_content = needs_light_content(palette.get("background").and_then(Value::as_str).unwrap_or("#000000"));
    ResolvedTheme { id, name, palette, light_content, effect }
}

/// The gallery: categories, the built-in themes (OLED in its saved accent),
/// the viewer's custom themes, and the OLED accent presets.
pub fn catalog(custom_themes: &[Value], oled_accent: Option<&str>) -> Value {
    let built_in: Vec<Value> = built_in_themes()
        .iter()
        .map(|t| {
            let mut t = t.clone();
            if t.get("id").and_then(Value::as_str) == Some(OLED_THEME_ID) {
                let palette = oled_palette(t.get("palette").unwrap_or(&Value::Null), oled_accent);
                t["palette"] = palette;
            }
            if let Some(palette) = t.get_mut("palette") {
                lift_text(palette);
            }
            t
        })
        .collect();
    let custom: Vec<Value> = custom_themes
        .iter()
        .filter_map(|c| {
            let mut palette = custom_palette(c);
            lift_text(&mut palette);
            Some(json!({
                "id": c.get("id")?.as_str()?,
                "name": c.get("name").and_then(Value::as_str).unwrap_or("Custom"),
                "created_at": c.get("createdAt").cloned().unwrap_or(Value::Null),
                "palette": palette,
            }))
        })
        .collect();
    json!({
        "categories": categories(),
        "themes": built_in,
        "custom": custom,
        "oled_theme_id": OLED_THEME_ID,
        "default_theme_id": DEFAULT_THEME_ID,
        "oled_accent_presets": OLED_ACCENT_PRESETS.iter().map(|(n, v)| json!({ "name": n, "value": v })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_list_loads() {
        assert_eq!(built_in_themes().len(), 22);
        assert_eq!(categories().as_array().map(Vec::len), Some(5));
        assert_eq!(resolve("nope", &[], None).id, DEFAULT_THEME_ID);
    }

    #[test]
    fn oled_follows_its_accent() {
        let t = resolve(OLED_THEME_ID, &[], Some("#ff9933"));
        assert_eq!(t.palette["accent"], "#ff9933");
        assert_eq!(t.palette["accentMuted"], "rgba(255, 153, 51, 0.5)");
        assert_eq!(t.palette["accentHover"], lighten("#ff9933", 20.0));
        assert_eq!(resolve(OLED_THEME_ID, &[], Some("bad")).palette["accent"], DEFAULT_OLED_ACCENT);
    }

    #[test]
    fn custom_themes_win_and_convert() {
        let custom = json!({ "id": "custom-1", "name": "Mine", "createdAt": 1, "palette": {
            "background": { "value": "#ffffff", "opacity": 100 },
            "surface": { "value": "#102030", "opacity": 50 },
            "glassOpacity": "0.15",
            "highlight": { "pink": { "value": "#ff00ff", "opacity": 100 } },
        }});
        let t = resolve("custom-1", &[custom], None);
        assert_eq!(t.name, "Mine");
        assert_eq!(t.palette["surface"], "rgba(16, 32, 48, 0.5)");
        assert_eq!(t.palette["glassOpacity"], "0.15");
        assert_eq!(t.palette["highlight"]["pink"], "#ff00ff");
        assert!(!t.light_content);
        assert!(needs_light_content("#1e1e2e"));
    }

    /// Every shell paints the built-ins from the JSON file itself (the desktop and Android read it
    /// in TypeScript, without this module), so the floor must already hold in the file: the lift
    /// then changes nothing for them, and every platform shows the same muted colour.
    #[test]
    fn every_built_in_ships_at_the_muted_floor() {
        for theme in built_in_themes() {
            let id = theme["id"].as_str().unwrap_or("?");
            let palette = &theme["palette"];
            let contrast = muted_contrast(palette).unwrap_or_else(|| panic!("{id}: colours do not parse"));
            assert!(contrast >= MUTED_CONTRAST_FLOOR, "{id}: muted text at {contrast:.2}:1");
            let mut lifted = palette.clone();
            lift_muted(&mut lifted);
            assert_eq!(&lifted, palette, "{id}: the lift changed a shipped palette");
        }
        let oled = resolve(OLED_THEME_ID, &[], Some("#22d3ee"));
        assert!(muted_contrast(&oled.palette).unwrap_or(0.0) >= MUTED_CONTRAST_FLOOR);
    }

    /// Secondary text on the raised surface: its contrast there, and muted text's.
    fn secondary_on_surface(palette: &Value) -> (f64, f64) {
        let color = |key: &str| css_rgba(palette[key].as_str().unwrap_or_default()).expect(key);
        let bg = color("background");
        let track = over(color("surface"), (bg.0, bg.1, bg.2));
        (ratio(over(color("textSecondary"), track), track), ratio(over(color("textMuted"), track), track))
    }

    #[test]
    fn every_built_in_ships_at_the_secondary_floor() {
        for theme in built_in_themes() {
            let id = theme["id"].as_str().unwrap_or("?");
            let palette = &theme["palette"];
            let (secondary, muted) = secondary_on_surface(palette);
            assert!(secondary >= SECONDARY_CONTRAST_FLOOR, "{id}: secondary text at {secondary:.2}:1 on the surface");
            assert!(secondary >= muted, "{id}: secondary ({secondary:.2}) quieter than muted ({muted:.2})");
            let mut lifted = palette.clone();
            lift_text(&mut lifted);
            assert_eq!(&lifted, palette, "{id}: the lift changed a shipped palette");
        }
        // OLED derives secondary from the accent; every preset clears it with no lift.
        for (name, accent) in OLED_ACCENT_PRESETS {
            let palette = oled_palette(&resolve(OLED_THEME_ID, &[], None).palette, Some(accent));
            assert!(palette_secondary_floor(&palette).is_none(), "OLED {name} needs a lift");
        }
        // A near-black accent does not, and the resolved palette is lifted.
        let dark = resolve(OLED_THEME_ID, &[], Some("#100010"));
        assert!(secondary_on_surface(&dark.palette).0 >= SECONDARY_CONTRAST_FLOOR);
    }

    #[test]
    fn the_four_inverted_themes_were_baked_to_the_floor() {
        let secondary = |id: &str| resolve(id, &[], None).palette["textSecondary"].clone();
        assert_eq!(secondary("solarized-dark"), "#4aa1de");
        assert_eq!(secondary("monokai"), "#fa5892");
        assert_eq!(secondary("solarized-sand"), "#584a37");
        assert_eq!(secondary("catppuccin-latte"), "#545667");
        assert_eq!(secondary(DEFAULT_THEME_ID), "#97b1b9");
    }

    #[test]
    fn winters_glass_muted_is_three_quarters() {
        assert_eq!(resolve(DEFAULT_THEME_ID, &[], None).palette["textMuted"], "rgba(151, 177, 185, 0.75)");
    }

    #[test]
    fn a_dim_custom_muted_is_lifted_to_the_floor() {
        let custom = json!({ "id": "custom-dim", "name": "Dim", "palette": {
            "background": { "value": "#0c0c0d", "opacity": 100 },
            "textMuted": { "value": "#97b1b9", "opacity": 40 },
        }});
        let resolved = resolve("custom-dim", std::slice::from_ref(&custom), None);
        assert_eq!(resolved.palette["textMuted"], "rgba(151, 177, 185, 0.68)");
        assert!(muted_contrast(&resolved.palette).unwrap_or(0.0) >= MUTED_CONTRAST_FLOOR);
        let catalog = catalog(&[custom], None);
        assert_eq!(catalog["custom"][0]["palette"]["textMuted"], "rgba(151, 177, 185, 0.68)");
        // Already bright enough: left exactly as written.
        let mut bright = json!({ "background": "#0c0c0d", "textMuted": "#ffffff" });
        lift_muted(&mut bright);
        assert_eq!(bright["textMuted"], "#ffffff");
    }

    #[test]
    fn saved_custom_themes_are_lifted_in_place() {
        let mut saved = json!([
            { "id": "custom-dim", "name": "Dim", "palette": {
                "background": { "value": "#0c0c0d", "opacity": 100 },
                "textMuted": { "value": "#97b1b9", "opacity": 40 },
                "accent": { "value": "#22d3ee", "opacity": 100 },
            }},
            { "id": "custom-ok", "name": "Fine", "palette": {
                "background": { "value": "#0c0c0d", "opacity": 100 },
                "textMuted": { "value": "#97b1b9", "opacity": 90 },
            }},
        ]);
        assert!(lift_custom_themes(&mut saved));
        assert_eq!(saved[0]["palette"]["textMuted"], json!({ "value": "#97b1b9", "opacity": 68 }));
        assert_eq!(saved[0]["palette"]["accent"]["opacity"], 100);
        assert_eq!(saved[1]["palette"]["textMuted"]["opacity"], 90);
        // The stored value now paints exactly what resolve paints.
        let resolved = resolve("custom-dim", saved.as_array().unwrap(), None);
        assert_eq!(resolved.palette["textMuted"], "rgba(151, 177, 185, 0.68)");
        // A second pass has nothing left to do.
        assert!(!lift_custom_themes(&mut saved));
        assert!(!lift_custom_themes(&mut json!(null)));
    }

    #[test]
    fn a_custom_secondary_quieter_than_the_floor_is_lifted() {
        // Catppuccin Latte as it shipped before the bake: the custom lift lands on the baked color.
        let mut saved = json!([{ "id": "custom-latte", "name": "Latte", "palette": {
            "background": { "value": "#d8dce4", "opacity": 100 },
            "surface": { "value": "#8839ef", "opacity": 10 },
            "textSecondary": { "value": "#6c6f85", "opacity": 100 },
            "textMuted": { "value": "#4c4f69", "opacity": 89 },
        }}]);
        assert!(lift_custom_themes(&mut saved));
        assert_eq!(saved[0]["palette"]["textSecondary"], json!({ "value": "#545667", "opacity": 100 }));
        let (secondary, muted) = secondary_on_surface(&custom_palette(&saved[0]));
        assert!(secondary >= SECONDARY_CONTRAST_FLOOR && secondary >= muted);
        assert!(!lift_custom_themes(&mut saved));
        // A translucent secondary that cannot reach the floor at its own opacity goes opaque.
        let mut faint = json!([{ "id": "custom-faint", "name": "Faint", "palette": {
            "background": { "value": "#0c0c0d", "opacity": 100 },
            "surface": { "value": "#97b1b9", "opacity": 15 },
            "textSecondary": { "value": "#97b1b9", "opacity": 20 },
            "textMuted": { "value": "#97b1b9", "opacity": 75 },
        }}]);
        assert!(lift_custom_themes(&mut faint));
        assert_eq!(faint[0]["palette"]["textSecondary"]["opacity"], 100);
        let (secondary, muted) = secondary_on_surface(&custom_palette(&faint[0]));
        assert!(secondary >= SECONDARY_CONTRAST_FLOOR && secondary >= muted);
    }
}
