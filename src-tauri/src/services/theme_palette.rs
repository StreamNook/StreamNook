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
            t
        })
        .collect();
    let custom: Vec<Value> = custom_themes
        .iter()
        .filter_map(|c| {
            Some(json!({
                "id": c.get("id")?.as_str()?,
                "name": c.get("name").and_then(Value::as_str).unwrap_or("Custom"),
                "created_at": c.get("createdAt").cloned().unwrap_or(Value::Null),
                "palette": custom_palette(c),
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
}
