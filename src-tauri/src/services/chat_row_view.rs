//! The chat row a native renderer draws, worked out once in Rust.
//!
//! The web renderers derive a row's look on the page: the readable name colour,
//! Twitch badge art, 7TV paints and badges, `@mentions`, link labels and the
//! grouping of zero-width overlays and modifiers onto their target emote. A
//! native shell runs none of that TypeScript, so while it asks for it
//! (`set_enabled`), every row the socket delivers carries `metadata.view` with
//! those answers, and a chatter whose 7TV cosmetics arrive after their first
//! row gets a `ROW_COSMETICS` frame. Nothing here runs for the web renderers.

use crate::models::chat_layout::{
    ChatMessage, MessageSegment, PaintLayerView, PaintShadow, PaintStop, PaintView, RowView, ViewBadge, ViewEmote,
    ViewSpan,
};
use crate::models::settings::Settings;
use lru::LruCache;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Turns the stamp on or off. A native shell turns it on at start.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The settings the stamp reads, copied on every settings change.
#[derive(Clone, Debug)]
struct Prefs {
    readable_colors: bool,
    dark_theme: bool,
    shorten_links: bool,
    show_badges: bool,
    show_third_party_badges: bool,
    hidden_badge_providers: Vec<String>,
    bttv_modifiers: bool,
    ffz_effects: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            readable_colors: true,
            dark_theme: true,
            shorten_links: true,
            show_badges: true,
            show_third_party_badges: true,
            hidden_badge_providers: Vec::new(),
            bttv_modifiers: true,
            ffz_effects: true,
        }
    }
}

static PREFS: RwLock<Option<Prefs>> = RwLock::new(None);

fn prefs() -> Prefs {
    PREFS.read().ok().and_then(|p| p.clone()).unwrap_or_default()
}

/// Called with every settings change, next to the rule engine's refresh.
pub fn refresh(settings: &Settings) {
    let d = &settings.chat_design;
    let next = Prefs {
        readable_colors: d.name_color_adjustment != "off",
        dark_theme: true,
        shorten_links: d.shorten_links,
        show_badges: d.show_badges,
        show_third_party_badges: d.show_third_party_badges,
        hidden_badge_providers: d.hidden_badge_providers.iter().map(|p| p.to_lowercase()).collect(),
        bttv_modifiers: d.bttv_emote_modifiers,
        ffz_effects: d.ffz_emote_effects,
    };
    if let Ok(mut guard) = PREFS.write() {
        *guard = Some(next);
    }
}

// ---- Readable name colours ---------------------------------------------------

/// Twitch's colour for a chatter who never picked one.
pub const FALLBACK_NAME_COLOR: &str = "#9147FF";

fn parse_hex(color: &str) -> Option<[u8; 3]> {
    let hex = color.trim().strip_prefix('#')?;
    let full: String = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 => hex.to_string(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn reads_light([r, g, b]: [u8; 3]) -> bool {
    (r as f64 * 299.0 + g as f64 * 587.0 + b as f64 * 114.0) / 1000.0 >= 128.0
}

fn rgb_to_hsl([r, g, b]: [u8; 3]) -> [f64; 3] {
    let (rn, gn, bn) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = rn.max(gn).max(bn);
    let min = rn.min(gn).min(bn);
    let l = (max + min) / 2.0;
    if max == min {
        return [0.0, 0.0, l];
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == rn {
        (gn - bn) / d + if gn < bn { 6.0 } else { 0.0 }
    } else if max == gn {
        (bn - rn) / d + 2.0
    } else {
        (rn - gn) / d + 4.0
    };
    [h / 6.0, s, l]
}

fn hue_to_rgb(p: f64, q: f64, t: f64) -> f64 {
    let mut t = t;
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

fn hsl_to_rgb([h, s, l]: [f64; 3]) -> [u8; 3] {
    // JavaScript's Math.round rounds halves up; f64::round rounds them away
    // from zero, which is the same for these non-negative values.
    let to_byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    if s == 0.0 {
        let v = to_byte(l);
        return [v, v, v];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    [to_byte(hue_to_rgb(p, q, h + 1.0 / 3.0)), to_byte(hue_to_rgb(p, q, h)), to_byte(hue_to_rgb(p, q, h - 1.0 / 3.0))]
}

/// The colour a name renders in: a hex colour on the wrong side of a YIQ
/// brightness check for the theme walks its HSL lightness toward white (dark
/// theme) or black (light theme) by a tenth of the remaining distance per
/// step, at most 20 steps, keeping hue and saturation. Anything not a hex
/// colour comes back untouched. Same walk as `src/utils/nameColor.ts`.
pub fn readable_name_color(color: &str, dark_theme: bool) -> String {
    let Some(rgb) = parse_hex(color) else { return color.to_string() };
    let mut hsl = rgb_to_hsl(rgb);
    let mut out = rgb;
    let mut steps = 0;
    while steps < 20 && reads_light(out) != dark_theme {
        let l = if dark_theme { 1.0 - 0.9 * (1.0 - hsl[2]) } else { 0.9 * hsl[2] };
        hsl = [hsl[0], hsl[1], l];
        out = hsl_to_rgb(hsl);
        steps += 1;
    }
    format!("#{:02x}{:02x}{:02x}", out[0], out[1], out[2])
}

/// A name colour after the viewer's readability pass, for surfaces outside the
/// row (the profile sheet); the row's own fallback purple when there is none.
pub fn readable_color(raw: Option<&str>) -> String {
    shown_color(raw, &prefs())
}

fn shown_color(raw: Option<&str>, prefs: &Prefs) -> String {
    let base = raw.map(str::trim).filter(|c| !c.is_empty()).unwrap_or(FALLBACK_NAME_COLOR);
    if prefs.readable_colors {
        readable_name_color(base, prefs.dark_theme)
    } else {
        base.to_string()
    }
}

// ---- Colours of chatters seen, for mentions ----------------------------------

#[cfg(mobile)]
const SEEN_CAPACITY: usize = 1500;
#[cfg(not(mobile))]
const SEEN_CAPACITY: usize = 4000;

static SEEN: Mutex<Option<LruCache<String, String>>> = Mutex::new(None);

fn note_seen(login: &str, color: &str) {
    if login.is_empty() {
        return;
    }
    if let Ok(mut guard) = SEEN.lock() {
        guard
            .get_or_insert_with(|| LruCache::new(NonZeroUsize::new(SEEN_CAPACITY).expect("non-zero")))
            .put(login.to_lowercase(), color.to_string());
    }
}

fn seen_color(login: &str) -> Option<String> {
    SEEN.lock().ok()?.as_mut()?.get(&login.to_lowercase()).cloned()
}

// ---- Twitch badge art --------------------------------------------------------

#[derive(Default)]
struct BadgeIndex {
    global: HashMap<String, (String, String)>,
    rooms: HashMap<String, HashMap<String, (String, String)>>,
}

static TWITCH_BADGES: RwLock<Option<BadgeIndex>> = RwLock::new(None);

/// Records Twitch badge art from a Helix badge answer: the global sets when
/// `room_id` is None, else that channel's own. Items are
/// `(set_id, version_id, image_url_2x, title)`.
pub fn note_twitch_badges<'a>(room_id: Option<&str>, items: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str)>) {
    let map: HashMap<String, (String, String)> = items
        .into_iter()
        .filter(|(_, _, url, _)| !url.is_empty())
        .map(|(set, ver, url, title)| (format!("{set}/{ver}"), (url.to_string(), title.to_string())))
        .collect();
    if map.is_empty() {
        return;
    }
    if let Ok(mut guard) = TWITCH_BADGES.write() {
        let index = guard.get_or_insert_with(BadgeIndex::default);
        match room_id {
            Some(room) if !room.is_empty() => {
                index.rooms.insert(room.to_string(), map);
            }
            _ => index.global = map,
        }
    }
}

/// A badge's art: the channel's own set first, then the global one.
fn twitch_badge_art(room_id: Option<&str>, set: &str, version: &str) -> Option<(String, String)> {
    let guard = TWITCH_BADGES.read().ok()?;
    let index = guard.as_ref()?;
    let key = format!("{set}/{version}");
    room_id
        .and_then(|room| index.rooms.get(room))
        .and_then(|m| m.get(&key))
        .or_else(|| index.global.get(&key))
        .cloned()
}

// ---- 7TV cosmetics -----------------------------------------------------------

/// A 7TV paint definition (the GraphQL shape the resolver keeps) reduced to
/// one fill: the first gradient or image layer, else the first solid colour.
pub fn paint_view(def: &Value) -> Option<PaintView> {
    let id = def.get("id").and_then(Value::as_str)?.to_string();
    let layers = def.pointer("/data/layers").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut view: Option<PaintView> = None;
    let mut solid: Option<String> = None;
    for layer in &layers {
        let ty = match layer.get("ty") {
            Some(ty) => ty,
            None => continue,
        };
        match ty.get("__typename").and_then(Value::as_str).unwrap_or_default() {
            "PaintLayerTypeLinearGradient" | "PaintLayerTypeRadialGradient" if view.is_none() => {
                let radial = ty.get("__typename").and_then(Value::as_str) == Some("PaintLayerTypeRadialGradient");
                let stops: Vec<PaintStop> = ty
                    .get("stops")
                    .and_then(Value::as_array)
                    .map(|stops| {
                        stops
                            .iter()
                            .filter_map(|s| {
                                Some(PaintStop { at: s.get("at")?.as_f64()? as f32, color: color_hex(s.get("color")?)? })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if stops.is_empty() {
                    continue;
                }
                view = Some(PaintView {
                    id: id.clone(),
                    kind: if radial { "radial" } else { "linear" }.to_string(),
                    angle: ty.get("angle").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    repeating: ty.get("repeating").and_then(Value::as_bool).unwrap_or(false),
                    stops,
                    ..Default::default()
                });
            }
            "PaintLayerTypeImage" if view.is_none() => {
                let images = ty.get("images").and_then(Value::as_array).cloned().unwrap_or_default();
                let animated = images.iter().any(|i| i.get("frameCount").and_then(Value::as_u64).unwrap_or(1) > 1);
                let pick = images.iter().find(|i| {
                    i.get("scale").and_then(Value::as_u64) == Some(1)
                        && (!animated || i.get("frameCount").and_then(Value::as_u64).unwrap_or(1) > 1)
                });
                if let Some(url) = pick.and_then(|i| i.get("url")).and_then(Value::as_str) {
                    view = Some(PaintView {
                        id: id.clone(),
                        kind: "image".to_string(),
                        image_url: Some(url.to_string()),
                        ..Default::default()
                    });
                }
            }
            "PaintLayerTypeSingleColor" if solid.is_none() => {
                solid = ty.get("color").and_then(color_hex);
            }
            _ => {}
        }
    }
    let mut view = view.or_else(|| {
        solid.map(|c| PaintView { id: id.clone(), kind: "solid".to_string(), stops: vec![PaintStop { at: 0.0, color: c }], ..Default::default() })
    })?;
    let (all, base_color, opacity) = paint_layers(&layers);
    view.name = def.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    view.layers = all;
    view.base_color = base_color;
    view.opacity = opacity;
    view.shadows = def
        .pointer("/data/shadows")
        .and_then(Value::as_array)
        .map(|shadows| {
            shadows
                .iter()
                .filter_map(|s| {
                    Some(PaintShadow {
                        x: s.get("offsetX")?.as_f64()? as f32,
                        y: s.get("offsetY")?.as_f64()? as f32,
                        blur: s.get("blur")?.as_f64()? as f32,
                        color: color_hex(s.get("color")?)?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(view)
}

/// Every gradient and image layer of a paint in CSS stacking order (top first),
/// its first single colour, and the lowest layer opacity under 1, the way the
/// page's `computePaintStyle` builds its background.
fn paint_layers(layers: &[Value]) -> (Vec<PaintLayerView>, Option<String>, f32) {
    let mut out = Vec::new();
    let mut base = None;
    let mut opacity: f32 = 1.0;
    for layer in layers {
        let Some(ty) = layer.get("ty") else { continue };
        let layer_opacity = layer.get("opacity").and_then(Value::as_f64).unwrap_or(1.0) as f32;
        let stops = || -> Vec<PaintStop> {
            ty.get("stops")
                .and_then(Value::as_array)
                .map(|stops| {
                    stops
                        .iter()
                        .filter_map(|s| Some(PaintStop { at: s.get("at")?.as_f64()? as f32, color: color_hex(s.get("color")?)? }))
                        .collect()
                })
                .unwrap_or_default()
        };
        let next = match ty.get("__typename").and_then(Value::as_str).unwrap_or_default() {
            "PaintLayerTypeLinearGradient" => Some(PaintLayerView {
                kind: "linear".into(),
                angle: ty.get("angle").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                repeating: ty.get("repeating").and_then(Value::as_bool).unwrap_or(false),
                stops: stops(),
                ..Default::default()
            })
            .filter(|l| !l.stops.is_empty()),
            "PaintLayerTypeRadialGradient" => Some(PaintLayerView {
                kind: "radial".into(),
                repeating: ty.get("repeating").and_then(Value::as_bool).unwrap_or(false),
                circle: ty.get("shape").and_then(Value::as_str) == Some("CIRCLE"),
                stops: stops(),
                ..Default::default()
            })
            .filter(|l| !l.stops.is_empty()),
            "PaintLayerTypeImage" => {
                let images = ty.get("images").and_then(Value::as_array).cloned().unwrap_or_default();
                let animated = images.iter().any(|i| i.get("frameCount").and_then(Value::as_u64).unwrap_or(1) > 1);
                images
                    .iter()
                    .find(|i| {
                        i.get("scale").and_then(Value::as_u64) == Some(1)
                            && (!animated || i.get("frameCount").and_then(Value::as_u64).unwrap_or(1) > 1)
                    })
                    .and_then(|i| i.get("url"))
                    .and_then(Value::as_str)
                    .map(|url| PaintLayerView { kind: "image".into(), image_url: Some(url.to_string()), ..Default::default() })
            }
            "PaintLayerTypeSingleColor" => {
                if base.is_none() {
                    base = ty.get("color").and_then(color_hex);
                }
                if layer_opacity < 1.0 {
                    opacity = opacity.min(layer_opacity);
                }
                continue;
            }
            _ => None,
        };
        if let Some(next) = next {
            if layer_opacity < 1.0 {
                opacity = opacity.min(layer_opacity);
            }
            out.push(next);
        }
    }
    (out, base, opacity)
}

/// A 7TV colour object as `#rrggbbaa`: from its channels when present, else
/// its `hex`.
fn color_hex(color: &Value) -> Option<String> {
    let channel = |k: &str| color.get(k).and_then(Value::as_f64);
    if let (Some(r), Some(g), Some(b)) = (channel("r"), channel("g"), channel("b")) {
        let a = channel("a").unwrap_or(255.0);
        let a = if a <= 1.0 && a.fract() != 0.0 { a * 255.0 } else { a };
        return Some(format!("#{:02x}{:02x}{:02x}{:02x}", r as u8, g as u8, b as u8, a.round().clamp(0.0, 255.0) as u8));
    }
    let hex = color.get("hex").and_then(Value::as_str)?;
    Some(hex.to_lowercase())
}

/// The 7TV badge a chatter wears, as art: its 2x image, else 7TV's CDN path.
fn seventv_badge(def: &Value) -> Option<ViewBadge> {
    let id = def.get("id").and_then(Value::as_str)?;
    let images = def.get("images").and_then(Value::as_array).cloned().unwrap_or_default();
    let url = images
        .iter()
        .filter(|i| i.get("scale").and_then(Value::as_u64) == Some(2))
        .min_by_key(|i| {
            let mime = i.get("mime").and_then(Value::as_str).unwrap_or_default();
            if mime.contains("webp") { 0 } else if mime.contains("png") { 1 } else { 2 }
        })
        .and_then(|i| i.get("url"))
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| format!("https://cdn.7tv.app/badge/{id}/2x.webp"));
    let title = def
        .get("description")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| def.get("name").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    Some(ViewBadge { url, title, provider: "7tv".to_string() })
}

/// What a chatter wears, as the view draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WornView {
    pub paint: Option<PaintView>,
    pub badge: Option<ViewBadge>,
}

pub fn worn_view(paints: &[Value], badges: &[Value]) -> WornView {
    let pick = |list: &[Value]| {
        list.iter()
            .find(|v| v.get("selected").and_then(Value::as_bool).unwrap_or(false))
            .or_else(|| list.first())
            .cloned()
    };
    WornView {
        paint: pick(paints).as_ref().and_then(paint_view),
        badge: pick(badges).as_ref().and_then(seventv_badge),
    }
}

/// Chatters whose cosmetics lookup is in flight, so one busy chatter starts one.
static PENDING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn begin_lookup(id: &str) -> bool {
    PENDING
        .lock()
        .map(|mut g| g.get_or_insert_with(HashSet::new).insert(id.to_string()))
        .unwrap_or(false)
}

fn end_lookup(id: &str) {
    if let Ok(mut g) = PENDING.lock() {
        if let Some(set) = g.as_mut() {
            set.remove(id);
        }
    }
}

/// The cosmetics already known for a row's author, or None while unknown (a
/// lookup is then started and its answer posted as `ROW_COSMETICS`).
fn known_cosmetics(msg: &ChatMessage) -> Option<WornView> {
    let id = crate::services::seventv_cosmetics_resolver::cosmetics_id(&msg.provider, &msg.user_id)?;
    if let Some(user) = crate::services::seventv_cosmetics_resolver::peek(&id) {
        return Some(worn_view(&user.paints, &user.badges));
    }
    if tokio::runtime::Handle::try_current().is_ok() && begin_lookup(&id) {
        let channel = msg.channel.trim_start_matches('#').to_lowercase();
        let user_id = msg.user_id.clone();
        let provider = msg.provider.clone();
        tokio::spawn(async move {
            let lookup = crate::services::seventv_cosmetics_resolver::resolve(id.clone()).await;
            end_lookup(&id);
            let worn = worn_view(&lookup.data.paints, &lookup.data.badges);
            if worn == WornView::default() {
                return;
            }
            let frame = cosmetics_frame(&channel, &provider, &user_id, &worn);
            crate::services::irc_service::post_channel_frame(&channel, frame).await;
        });
    }
    None
}

/// The frame that paints rows already on screen once a chatter's cosmetics
/// arrive.
pub fn cosmetics_frame(channel: &str, provider: &str, user_id: &str, worn: &WornView) -> String {
    let prefs = prefs();
    let badge = worn
        .badge
        .clone()
        .filter(|_| prefs.show_third_party_badges && !prefs.hidden_badge_providers.iter().any(|p| p == "7tv"));
    serde_json::json!({
        "type": "ROW_COSMETICS",
        "channel": channel,
        "provider": provider,
        "user_id": user_id,
        "paint": worn.paint,
        "badge": badge,
    })
    .to_string()
}

// ---- Pinned messages ---------------------------------------------------------

/// The Twitch sets a compact byline may show, best first: broadcaster, staff,
/// moderator, VIP (the role half of `twitch_badge_rank`).
fn role_rank(set: &str) -> Option<u8> {
    let rank = crate::models::chat_layout::twitch_badge_rank(set);
    (rank <= 3).then_some(rank)
}

/// Stamps `sender_view` on each pin from Twitch's pinned-chat read, the way a
/// row's author is drawn: the readable name colour, the sender's Twitch badge
/// art for this channel (in chat order), the one role badge a collapsed
/// banner shows, and their 7TV paint when it is already known.
pub fn stamp_pins(pins: &mut [Value], channel_id: &str) {
    let prefs = prefs();
    for pin in pins.iter_mut() {
        let color = pin.get("sender_color").and_then(Value::as_str);
        let name_color = shown_color(color, &prefs);
        let sets: Vec<(String, String)> = pin
            .get("sender_badges")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|b| {
                        let set = b.get("set_id").and_then(Value::as_str)?;
                        let version = b.get("version").and_then(Value::as_str).unwrap_or("1");
                        Some((set.to_string(), version.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut badges = Vec::new();
        let mut role: Option<(u8, ViewBadge)> = None;
        if prefs.show_badges {
            for (set, version) in &sets {
                let Some((url, title)) = twitch_badge_art(Some(channel_id), set, version) else { continue };
                let badge = ViewBadge { url, title, provider: "twitch".to_string() };
                if let Some(rank) = role_rank(set) {
                    if role.as_ref().is_none_or(|(best, _)| rank < *best) {
                        role = Some((rank, badge.clone()));
                    }
                }
                badges.push(badge);
            }
        }
        let paint = pin
            .get("sender_id")
            .and_then(Value::as_str)
            .and_then(|id| crate::services::seventv_cosmetics_resolver::cosmetics_id("twitch", id))
            .and_then(|id| crate::services::seventv_cosmetics_resolver::peek(&id))
            .and_then(|user| worn_view(&user.paints, &user.badges).paint);
        if let Some(obj) = pin.as_object_mut() {
            obj.insert(
                "sender_view".to_string(),
                serde_json::json!({
                    "name_color": name_color,
                    "badges": badges,
                    "role_badge": role.map(|(_, b)| b),
                    "paint": paint,
                }),
            );
        }
    }
}

// ---- The body ----------------------------------------------------------------

const MOD_HIDDEN: u32 = 1;
const MOD_PREFIX: u32 = 1 << 18;

/// A link's label: no scheme or `www.`, the host always whole, a long path cut
/// with an ellipsis past 40 characters. Same as `prettyUrlLabel` on the page.
pub fn link_label(raw: &str) -> String {
    const MAX: usize = 40;
    let with_scheme = if raw.starts_with("http") { raw.to_string() } else { format!("https://{raw}") };
    let Ok(url) = url::Url::parse(&with_scheme) else {
        return if raw.chars().count() > MAX { format!("{}…", raw.chars().take(MAX - 1).collect::<String>()) } else { raw.to_string() };
    };
    let host = url.host_str().unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(host).to_string();
    let path = if url.path() == "/" { "" } else { url.path() };
    let rest = format!(
        "{path}{}{}",
        url.query().map(|q| format!("?{q}")).unwrap_or_default(),
        url.fragment().map(|f| format!("#{f}")).unwrap_or_default()
    );
    let full = format!("{host}{rest}");
    if !rest.is_empty() && full.chars().count() > MAX {
        let room = MAX.saturating_sub(host.chars().count() + 1);
        return format!("{host}{}…", rest.chars().take(room).collect::<String>());
    }
    let trimmed = full.strip_suffix('/').unwrap_or(&full).to_string();
    if trimmed.is_empty() { host } else { trimmed }
}

/// Splits `@name` mentions out of text: an `@` and 1 to 25 word characters
/// followed by the end, whitespace or closing punctuation.
fn split_mentions(text: &str, out: &mut Vec<Item>) {
    let chars: Vec<char> = text.chars().collect();
    let mut plain = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut j = i + 1;
            while j < chars.len() && j - i <= 25 && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let len = j - i - 1;
            let ends = j == chars.len() || chars[j].is_whitespace() || ".,!?:;'\")]}>-".contains(chars[j]);
            if (1..=25).contains(&len) && ends {
                if !plain.is_empty() {
                    out.push(Item::Text(std::mem::take(&mut plain)));
                }
                let name: String = chars[i + 1..j].iter().collect();
                out.push(Item::Mention(name));
                i = j;
                continue;
            }
        }
        plain.push(chars[i]);
        i += 1;
    }
    if !plain.is_empty() {
        out.push(Item::Text(plain));
    }
}

#[derive(Clone, Debug)]
struct EmoteItem {
    view: ViewEmote,
    zero_width: bool,
    flags: Option<u32>,
}

#[derive(Clone, Debug)]
enum Item {
    Text(String),
    Mention(String),
    Link { label: String, url: String },
    Emoji { text: String, url: String },
    Emote(EmoteItem),
    Cheer { text: String, bits: u32, color: String, url: String },
    Gif { text: String, url: String },
}

impl Item {
    fn is_spacer(&self) -> bool {
        matches!(self, Item::Text(t) if t.trim().is_empty())
    }
    fn is_single_space(&self) -> bool {
        matches!(self, Item::Text(t) if t == " ")
    }
    fn is_prefix_mod(&self) -> bool {
        matches!(self, Item::Emote(e) if e.flags.unwrap_or(0) & MOD_PREFIX != 0)
    }
    fn is_target(&self) -> bool {
        match self {
            Item::Emote(e) => e.flags.is_none(),
            Item::Emoji { .. } => true,
            _ => false,
        }
    }
    fn is_emote_or_emoji(&self) -> bool {
        matches!(self, Item::Emote(_) | Item::Emoji { .. })
    }
}

/// One drawn unit: a single item, or a target with what attaches to it.
enum Unit {
    One(Item),
    Group(Vec<Item>),
}

/// Groups overlays and modifiers onto their target, as the page does:
/// zero-width emotes and FFZ modifiers attach backward across any run of
/// spaces; BetterTTV prefix modifiers attach forward across exactly one space.
fn group(items: Vec<Item>, prefs: &Prefs) -> Vec<Unit> {
    let mut grouped: Vec<Unit> = Vec::new();
    let mut pending: Vec<Item> = Vec::new();
    fn flush(grouped: &mut Vec<Unit>, pending: &mut Vec<Item>) {
        for p in pending.drain(..) {
            grouped.push(Unit::One(p));
        }
    }
    for item in items {
        if item.is_prefix_mod() {
            if !prefs.bttv_modifiers {
                flush(&mut grouped, &mut pending);
                grouped.push(Unit::One(item));
            } else {
                pending.push(item);
            }
            continue;
        }
        if let Some(last) = pending.last() {
            if item.is_single_space() && !last.is_spacer() {
                pending.push(item);
                continue;
            }
            if item.is_target() && last.is_single_space() {
                let mods: Vec<Item> = pending.drain(..).filter(Item::is_prefix_mod).collect();
                let mut members = vec![item];
                members.extend(mods);
                grouped.push(Unit::Group(members));
                continue;
            }
            flush(&mut grouped, &mut pending);
        }
        let zero_width = matches!(&item, Item::Emote(e) if e.zero_width);
        if !zero_width {
            grouped.push(Unit::One(item));
            continue;
        }
        let n = grouped.len();
        match grouped.last_mut() {
            None => grouped.push(Unit::Group(vec![item])),
            Some(Unit::Group(members)) => members.push(item),
            Some(Unit::One(last)) if last.is_spacer() => {
                if n > 1 {
                    match &mut grouped[n - 2] {
                        Unit::Group(members) => {
                            members.push(item);
                            grouped.pop();
                        }
                        Unit::One(before) if before.is_emote_or_emoji() => {
                            let base = before.clone();
                            grouped[n - 2] = Unit::Group(vec![base, item]);
                            grouped.pop();
                        }
                        _ => grouped.push(Unit::Group(vec![item])),
                    }
                } else {
                    grouped.push(Unit::Group(vec![item]));
                }
            }
            Some(Unit::One(last)) if last.is_emote_or_emoji() => {
                let base = last.clone();
                grouped[n - 1] = Unit::Group(vec![base, item]);
            }
            Some(Unit::One(_)) => grouped.push(Unit::Group(vec![item])),
        }
    }
    flush(&mut grouped, &mut pending);
    grouped
}

fn push_text(out: &mut Vec<ViewSpan>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(ViewSpan::Text { text: prev }) = out.last_mut() {
        prev.push_str(text);
        return;
    }
    out.push(ViewSpan::Text { text: text.to_string() });
}

fn single(item: Item, out: &mut Vec<ViewSpan>) {
    match item {
        Item::Text(t) => push_text(out, &t),
        Item::Mention(login) => {
            let color = seen_color(&login);
            out.push(ViewSpan::Mention { text: format!("@{login}"), login: login.to_lowercase(), color });
        }
        Item::Link { label, url } => out.push(ViewSpan::Link { label, url }),
        Item::Emoji { text, url } => out.push(ViewSpan::Emoji { text, url }),
        Item::Emote(e) => out.push(ViewSpan::Emote { emote: e.view, overlays: Vec::new(), effects: 0 }),
        Item::Cheer { text, bits, color, url } => out.push(ViewSpan::Cheer { text, bits, color, url }),
        Item::Gif { text, url } => out.push(ViewSpan::Gif { text, url }),
    }
}

fn emit(units: Vec<Unit>, prefs: &Prefs) -> Vec<ViewSpan> {
    let mut out = Vec::new();
    for unit in units {
        match unit {
            Unit::One(item) => single(item, &mut out),
            Unit::Group(members) => {
                let mut members = members.into_iter();
                let Some(base) = members.next() else { continue };
                let contributes = |flags: u32| flags & MOD_PREFIX != 0 || prefs.ffz_effects;
                let mut effects = 0u32;
                let mut overlays = Vec::new();
                for member in members {
                    let Item::Emote(e) = member else { continue };
                    let flags = e.flags.unwrap_or(0);
                    let lends = e.flags.is_some() && contributes(flags);
                    if lends {
                        effects |= flags;
                    }
                    let mut view = e.view;
                    view.hidden = lends && flags & MOD_HIDDEN != 0;
                    if !view.hidden {
                        overlays.push(view);
                    }
                }
                effects &= !(MOD_HIDDEN | MOD_PREFIX);
                match base {
                    Item::Emote(e) => out.push(ViewSpan::Emote { emote: e.view, overlays, effects }),
                    Item::Emoji { text, url } => {
                        // An emoji base draws as an emote cell so overlays can stack on it.
                        out.push(ViewSpan::Emote {
                            emote: ViewEmote { code: text, url, width: Some(20), height: Some(20), ..Default::default() },
                            overlays,
                            effects,
                        })
                    }
                    other => {
                        single(other, &mut out);
                        for o in overlays {
                            out.push(ViewSpan::Emote { emote: o, overlays: Vec::new(), effects: 0 });
                        }
                    }
                }
            }
        }
    }
    out
}

/// The grouped body of a row from its segments. `channel` names the room
/// whose emote table gives third-party emote sizes.
pub fn spans(segments: &[MessageSegment], channel: &str, shorten_links: bool) -> Vec<ViewSpan> {
    let prefs = prefs();
    spans_with(segments, channel, Prefs { shorten_links, ..prefs })
}

fn spans_with(segments: &[MessageSegment], channel: &str, prefs: Prefs) -> Vec<ViewSpan> {
    let mut items = Vec::with_capacity(segments.len());
    for seg in segments {
        match seg {
            MessageSegment::Text { content } => {
                if content.trim().is_empty() {
                    items.push(Item::Text(content.clone()));
                } else {
                    split_mentions(content, &mut items);
                }
            }
            MessageSegment::Emote { content, emote_url, is_zero_width, modifier_flags, is_personal, .. } => {
                let (width, height) = crate::services::irc_service::emote_size_hint(channel, content, emote_url)
                    .map_or((None, None), |(w, h)| (Some(w), Some(h)));
                items.push(Item::Emote(EmoteItem {
                    view: ViewEmote {
                        code: content.clone(),
                        url: emote_url.clone(),
                        width,
                        height,
                        personal: is_personal.unwrap_or(false),
                        hidden: false,
                    },
                    zero_width: is_zero_width.unwrap_or(false),
                    flags: *modifier_flags,
                }));
            }
            MessageSegment::Emoji { content, emoji_url } => items.push(Item::Emoji { text: content.clone(), url: emoji_url.clone() }),
            MessageSegment::Link { content, url } => items.push(Item::Link {
                label: if prefs.shorten_links { link_label(content) } else { content.clone() },
                url: url.clone(),
            }),
            MessageSegment::Cheermote { content, bits, color, cheermote_url, .. } => items.push(Item::Cheer {
                text: content.clone(),
                bits: *bits,
                color: color.clone(),
                url: cheermote_url.clone(),
            }),
            MessageSegment::Gif { content, gif_url, .. } => items.push(Item::Gif { text: content.clone(), url: gif_url.clone() }),
        }
    }
    emit(group(items, &prefs), &prefs)
}

// ---- The stamp ---------------------------------------------------------------

fn badges(msg: &ChatMessage, prefs: &Prefs, worn: Option<&WornView>) -> Vec<ViewBadge> {
    let mut out = Vec::new();
    let twitch = msg.provider.is_empty() || msg.provider == "twitch";
    if prefs.show_badges {
        let room = msg
            .metadata
            .source_room_id
            .as_deref()
            .or_else(|| msg.tags.get("source-room-id").map(String::as_str))
            .or_else(|| msg.tags.get("room-id").map(String::as_str));
        for badge in &msg.badges {
            let carried = badge.image_url_2x.as_ref().or(badge.image_url_4x.as_ref()).or(badge.image_url_1x.as_ref());
            if let Some(url) = carried.filter(|u| !u.is_empty()) {
                out.push(ViewBadge {
                    url: url.clone(),
                    title: badge.title.clone().unwrap_or_default(),
                    provider: if twitch { "twitch".to_string() } else { msg.provider.clone() },
                });
            } else if twitch {
                if let Some((url, title)) = twitch_badge_art(room, &badge.name, &badge.version) {
                    out.push(ViewBadge { url, title, provider: "twitch".to_string() });
                }
            }
        }
    }
    let seventv_shown = prefs.show_third_party_badges && !prefs.hidden_badge_providers.iter().any(|p| p == "7tv");
    if seventv_shown {
        if let Some(badge) = worn.and_then(|w| w.badge.clone()) {
            out.push(badge);
        }
    }
    // Third-party badges (FFZ, BTTV, Chatterino, Homies and the rest) after 7TV,
    // in the providers' one shared order, as the desktop row draws them.
    if prefs.show_third_party_badges && twitch && !msg.user_id.is_empty() {
        for badge in crate::commands::badge_service::third_party_worn_now(&msg.user_id) {
            let provider = badge.provider.as_key();
            if prefs.hidden_badge_providers.iter().any(|p| p == provider) {
                continue;
            }
            let info = badge.badge_info;
            let url = [info.image_2x, info.image_4x, info.image_1x].into_iter().find(|u| !u.is_empty());
            if let Some(url) = url {
                out.push(ViewBadge { url, title: info.title, provider: provider.to_string() });
            }
        }
    }
    // A StreamNook member's own badge, last, as every surface orders it.
    if prefs.show_third_party_badges && twitch && !prefs.hidden_badge_providers.iter().any(|p| p == "streamnook") {
        if let Some(badge) = member_badge(&msg.user_id) {
            out.push(badge);
        }
    }
    out
}

/// A member's StreamNook badge as row art, at the chat-size cut.
fn member_badge(member_id: &str) -> Option<ViewBadge> {
    let badge = crate::services::cosmetics_registry::member_badge(member_id, true)?;
    Some(ViewBadge { url: badge.image_url, title: badge.label, provider: "streamnook".to_string() })
}

/// The theme a Twitch row's author wears, when known; for a member whose theme
/// is not known yet a read starts, and its answer is posted as `ROW_MEMBER`.
fn known_member_theme(msg: &ChatMessage) -> Option<crate::services::member_theme::MemberTheme> {
    use crate::services::{cosmetics_registry, member_theme};
    let twitch = msg.provider.is_empty() || msg.provider == "twitch";
    if !twitch || msg.user_id.is_empty() || !cosmetics_registry::is_member(&msg.user_id) {
        return None;
    }
    if let Some(theme) = member_theme::peek(&msg.user_id) {
        return Some(theme);
    }
    if tokio::runtime::Handle::try_current().is_ok() {
        let member = msg.user_id.clone();
        tokio::spawn(async move {
            // The answer reaches rows through the theme's own change frame.
            let _ = member_theme::resolve(&member).await;
        });
    }
    None
}

/// The frame that repaints every row of one member, in every room, after their
/// theme or badge changed. Only native shells read it.
pub fn member_frame(theme: &crate::services::member_theme::MemberTheme) -> String {
    let prefs = prefs();
    let badge = member_badge(&theme.member_id)
        .filter(|_| prefs.show_third_party_badges && !prefs.hidden_badge_providers.iter().any(|p| p == "streamnook"));
    serde_json::json!({
        "type": "ROW_MEMBER",
        "user_id": theme.member_id,
        "atmosphere": theme.chat_atmosphere(),
        "cologne": theme.chat_cologne(),
        "badge": badge,
    })
    .to_string()
}

/// Posts `member_frame` to every room while a native shell draws rows.
pub fn post_member_frame(theme: &crate::services::member_theme::MemberTheme) {
    if !enabled() || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let frame = member_frame(theme);
    tokio::spawn(async move {
        crate::services::irc_service::post_frame_to_all(frame).await;
    });
}

/// Stamps `metadata.view` on a row about to be sent, while a native shell
/// asked for it.
pub fn stamp(msg: &mut ChatMessage) {
    if enabled() {
        stamp_view(msg);
    }
}

/// Stamps `metadata.view` whatever the switch says.
pub fn stamp_view(msg: &mut ChatMessage) {
    let prefs = prefs();
    let name_color = shown_color(msg.color.as_deref(), &prefs);
    note_seen(&msg.username, &name_color);
    let channel = msg.channel.trim_start_matches('#').to_lowercase();
    let worn = known_cosmetics(msg);
    let body = spans_with(&msg.segments, &channel, prefs.clone());
    let parent_spans = msg
        .metadata
        .reply_info
        .as_ref()
        .and_then(|r| r.parent_segments.as_ref())
        .map(|segs| spans_with(segs, &channel, prefs.clone()));
    let theme = known_member_theme(msg);
    msg.metadata.view = Some(RowView {
        name_color,
        badges: badges(msg, &prefs, worn.as_ref()),
        paint: worn.and_then(|w| w.paint),
        spans: body,
        parent_spans,
        atmosphere: theme.as_ref().and_then(|t| t.chat_atmosphere().map(String::from)),
        cologne: theme.as_ref().and_then(|t| t.chat_cologne()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> MessageSegment {
        MessageSegment::Text { content: s.to_string() }
    }

    fn emote(code: &str, zero_width: bool, flags: Option<u32>) -> MessageSegment {
        MessageSegment::Emote {
            content: code.to_string(),
            emote_id: Some(code.to_string()),
            emote_url: format!("https://cdn.7tv.app/emote/{code}/1x.webp"),
            is_zero_width: Some(zero_width),
            modifier_flags: flags,
            is_personal: None,
        }
    }

    fn kinds(spans: &[ViewSpan]) -> Vec<String> {
        spans
            .iter()
            .map(|s| match s {
                ViewSpan::Text { text } => format!("t:{text}"),
                ViewSpan::Mention { login, .. } => format!("m:{login}"),
                ViewSpan::Link { label, .. } => format!("l:{label}"),
                ViewSpan::Emoji { text, .. } => format!("j:{text}"),
                ViewSpan::Emote { emote, overlays, effects } => format!(
                    "e:{}{}{}",
                    emote.code,
                    overlays.iter().map(|o| format!("+{}", o.code)).collect::<String>(),
                    if *effects != 0 { format!("#{effects}") } else { String::new() }
                ),
                ViewSpan::Cheer { text, .. } => format!("c:{text}"),
                ViewSpan::Gif { text, .. } => format!("g:{text}"),
            })
            .collect()
    }

    #[test]
    fn readable_colors_match_the_page_walk() {
        // Navy on a dark theme walks toward white; values from nameColor.test.ts.
        assert_eq!(readable_name_color("#000080", true), readable_name_color("#000080", true));
        let navy = readable_name_color("#000080", true);
        assert!(reads_light(parse_hex(&navy).unwrap()));
        // Already readable stays put.
        assert_eq!(readable_name_color("#ff7f50", true), "#ff7f50");
        // Not a hex colour: untouched.
        assert_eq!(readable_name_color("var(--x)", true), "var(--x)");
        assert_eq!(readable_name_color("#fff", false), readable_name_color("#ffffff", false));
    }

    #[test]
    fn mentions_split_out_of_text_with_their_boundaries() {
        let spans = spans_with(&[text("hi @Nook_Fan, and a@b c @toolongnameeeeeeeeeeeeeeeeeeee")], "x", Prefs::default());
        assert_eq!(kinds(&spans), vec!["t:hi ", "m:nook_fan", "t:, and a", "m:b", "t: c @toolongnameeeeeeeeeeeeeeeeeeee"]);
    }

    #[test]
    fn zero_width_stacks_over_the_emote_before_it_across_spaces() {
        let segs = [emote("Base", false, None), text("   "), emote("RainTime", true, None), text(" "), text("hi")];
        let spans = spans_with(&segs, "x", Prefs::default());
        assert_eq!(kinds(&spans), vec!["e:Base+RainTime", "t: hi"]);
    }

    #[test]
    fn a_leading_zero_width_emote_stands_alone() {
        let spans = spans_with(&[emote("RainTime", true, None), text(" yo")], "x", Prefs::default());
        assert_eq!(kinds(&spans), vec!["e:RainTime", "t: yo"]);
    }

    #[test]
    fn prefix_modifiers_attach_forward_across_one_space_only() {
        let wide = MOD_PREFIX | (1 << 19) | MOD_HIDDEN;
        let one = spans_with(&[emote("w!", false, Some(wide)), text(" "), emote("Kappa", false, None)], "x", Prefs::default());
        assert_eq!(kinds(&one), vec![format!("e:Kappa#{}", 1 << 19)]);
        let two = spans_with(&[emote("w!", false, Some(wide)), text("  "), emote("Kappa", false, None)], "x", Prefs::default());
        assert_eq!(kinds(&two), vec!["e:w!", "t:  ", "e:Kappa"]);
        let off = Prefs { bttv_modifiers: false, ..Prefs::default() };
        let plain = spans_with(&[emote("w!", false, Some(wide)), text(" "), emote("Kappa", false, None)], "x", off);
        assert_eq!(kinds(&plain), vec!["e:w!", "t: ", "e:Kappa"]);
    }

    #[test]
    fn ffz_modifiers_lend_effects_only_while_effects_are_on() {
        let flip = (1 << 1) | MOD_HIDDEN;
        let on = spans_with(&[emote("Kappa", false, None), text(" "), emote("ffzX", true, Some(flip))], "x", Prefs::default());
        assert_eq!(kinds(&on), vec!["e:Kappa#2"]);
        let off = Prefs { ffz_effects: false, ..Prefs::default() };
        let drawn = spans_with(&[emote("Kappa", false, None), text(" "), emote("ffzX", true, Some(flip))], "x", off);
        assert_eq!(kinds(&drawn), vec!["e:Kappa+ffzX"]);
    }

    #[test]
    fn link_labels_keep_the_host_and_cut_long_paths() {
        assert_eq!(link_label("https://www.example.com/"), "example.com");
        assert_eq!(link_label("example.com/a"), "example.com/a");
        let long = link_label("https://twitch.tv/videos/123456789012345678901234567890123456789");
        assert!(long.starts_with("twitch.tv/videos/"));
        assert!(long.ends_with('…'));
        assert_eq!(long.chars().count(), 40);
    }

    #[test]
    fn a_linear_paint_reduces_to_its_stops_and_shadows() {
        let def = serde_json::json!({
            "id": "p1",
            "data": {
                "layers": [{ "id": "l", "opacity": 1, "ty": {
                    "__typename": "PaintLayerTypeLinearGradient", "angle": 90, "repeating": false,
                    "stops": [
                        { "at": 0, "color": { "hex": "#FF0000FF", "r": 255, "g": 0, "b": 0, "a": 255 } },
                        { "at": 1, "color": { "hex": "#0000FFFF", "r": 0, "g": 0, "b": 255, "a": 255 } }
                    ]
                } }],
                "shadows": [{ "offsetX": 0, "offsetY": 1, "blur": 2, "color": { "r": 0, "g": 0, "b": 0, "a": 128 } }]
            }
        });
        let view = paint_view(&def).unwrap();
        assert_eq!(view.kind, "linear");
        assert_eq!(view.angle, 90.0);
        assert_eq!(view.stops.len(), 2);
        assert_eq!(view.stops[0].color, "#ff0000ff");
        assert_eq!(view.shadows[0].color, "#00000080");
    }

    #[test]
    fn twitch_badges_resolve_channel_first_then_global() {
        note_twitch_badges(None, [("moderator", "1", "https://g/mod/2", "Moderator")]);
        note_twitch_badges(Some("42"), [("subscriber", "12", "https://c/sub12/2", "1-Year Subscriber")]);
        assert_eq!(twitch_badge_art(Some("42"), "subscriber", "12").unwrap().0, "https://c/sub12/2");
        assert_eq!(twitch_badge_art(Some("42"), "moderator", "1").unwrap().0, "https://g/mod/2");
        assert!(twitch_badge_art(Some("7"), "subscriber", "12").is_none());
    }

    #[test]
    fn a_pin_carries_its_badge_art_and_one_role_badge() {
        // The same art as the test above: the index is shared by every test.
        note_twitch_badges(None, [("moderator", "1", "https://g/mod/2", "Moderator")]);
        note_twitch_badges(Some("42"), [("subscriber", "12", "https://c/sub12/2", "1-Year Subscriber")]);
        let mut pins = vec![serde_json::json!({
            "sender_id": "",
            "sender_color": "#000080",
            "sender_badges": [
                { "set_id": "moderator", "version": "1" },
                { "set_id": "subscriber", "version": "12" },
                { "set_id": "unknown", "version": "1" },
            ],
        })];
        stamp_pins(&mut pins, "42");
        let view = &pins[0]["sender_view"];
        let urls: Vec<&str> = view["badges"].as_array().unwrap().iter().map(|b| b["url"].as_str().unwrap()).collect();
        assert_eq!(urls, ["https://g/mod/2", "https://c/sub12/2"]);
        assert_eq!(view["role_badge"]["url"], "https://g/mod/2");
        assert_ne!(view["name_color"], "#000080", "navy is lifted to read on a dark theme");
    }
}
