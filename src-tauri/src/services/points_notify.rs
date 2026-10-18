//! Channel points and prediction notifications for when StreamNook is not the
//! window in use, drawn with each channel's own look.
//!
//! What a channel's points look like is public: its face, its own name for its
//! points ("Goldwig") and icon, its color and its reward list come from one
//! anonymous query per channel, kept six hours. From those this draws, once per
//! channel, the face with the points icon at its corner (a transparent ring is
//! cut around the icon, so it sits on light and dark notifications alike),
//! Twitch's grey default orb in the channel's color for a channel without its
//! own icon, and a banner for the rare moments (a watch streak, a prediction
//! won). The pictures live with the other notification pictures, so a channel
//! is drawn once.
//!
//! A burst of earns (channel_points_summary.rs) becomes:
//! - one channel with a bonus chest, a raid, a watch streak or anything else
//!   that is not routine: a notification for that channel, which replaces that
//!   channel's earlier one in Action Center;
//! - several such channels at once: one notification, a row per channel;
//! - points for watching (or a rise only the balance poll saw): no popup; the
//!   channel's entry in Action Center updates quietly with a running total for
//!   this session. Windows only, since the others cannot update quietly.
//! A prediction payout is left to the prediction's own notification.
//!
//! Predictions: the result (won, lost, refunded) under the channel points
//! setting, and, when asked for, the moment one opens on a watched channel,
//! which leaves Action Center when betting closes.
//!
//! Nothing here runs between earns, and with the notifications off nothing is
//! fetched or drawn.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use image::{imageops, Rgba, RgbaImage};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tauri::Manager;

use crate::commands::notification_mirror::{self, MirroredNotice, NoticeAction, Route};
use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::channel_points_summary::ChannelEarned;
use crate::services::system_notify::{self, RichToast, ToastProgress, ToastRow};

const LOOK_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const LOOK_MISS_TTL: Duration = Duration::from_secs(10 * 60);
const LOOK_MAX: usize = 256;
const LOOK_TIMEOUT: Duration = Duration::from_secs(4);
/// How long a burst waits for faces and names before going out without them.
const DRESS_WAIT: Duration = Duration::from_secs(2);
const DRESS_MAX: usize = 12;
const STREAK_WAIT: Duration = Duration::from_secs(2);
const GROUP: &str = "channel-points";
const HEADER: (&str, &str) = ("channel-points", "Channel points");
/// Points notifications leave Action Center after this long.
const KEEP_FOR: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_ROWS: usize = 3;
/// Twitch purple, for a channel that never picked a color.
const DEFAULT_COLOR: [u8; 3] = [0x91, 0x47, 0xFF];
const BADGE_SIZE: u32 = 192;
const HERO_W: u32 = 728;
const HERO_H: u32 = 360;
/// The points icon on the banner: Twitch's 112 px icon at exactly twice.
const HERO_ICON: u32 = 224;
/// Part of every drawn picture's file name; bump it when the drawing changes.
const ART_VERSION: &str = "2";
const PREDICTIONS_KEPT: usize = 32;

const LOOK_QUERY: &str = "query($id: ID, $login: String) { user(id: $id, login: $login, lookupType: ALL) { \
id login displayName primaryColorHex profileImageURL(width: 300) \
channel { communityPointsSettings { name image { url4x } defaultImage { url4x } \
customRewards { title cost isEnabled isPaused } } } } }";

/// A reward the viewer could redeem now, by price.
#[derive(Debug, Clone, PartialEq)]
pub struct RewardCost {
    pub title: String,
    pub cost: i64,
}

/// What a channel's points look like. Public, read without signing in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PointsLook {
    pub channel_id: String,
    pub login: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// The channel's own name for its points; None = Twitch's "Channel Points".
    pub points_name: Option<String>,
    /// The channel's own points icon; None = Twitch's default orb.
    pub icon_url: Option<String>,
    pub default_icon_url: Option<String>,
    pub color: Option<[u8; 3]>,
    /// Rewards that are on and not paused.
    pub rewards: Vec<RewardCost>,
}

impl PointsLook {
    fn currency(&self) -> &str {
        self.points_name.as_deref().unwrap_or("Channel Points")
    }
}

fn parse_hex(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn parse_look(v: &Value) -> Option<PointsLook> {
    let user = v.pointer("/data/user").filter(|u| u.is_object())?;
    let text = |node: Option<&Value>, path: &str| {
        node.and_then(|n| n.pointer(path))
            .and_then(|x| x.as_str())
            .filter(|x| !x.is_empty())
            .map(String::from)
    };
    let settings = user.pointer("/channel/communityPointsSettings");
    let login = text(Some(user), "/login")?;
    let rewards = settings
        .and_then(|s| s.get("customRewards"))
        .and_then(|r| r.as_array())
        .map(|list| {
            list.iter()
                .filter(|r| r.get("isEnabled").and_then(|b| b.as_bool()).unwrap_or(false))
                .filter(|r| !r.get("isPaused").and_then(|b| b.as_bool()).unwrap_or(false))
                .filter_map(|r| {
                    Some(RewardCost {
                        title: r.get("title")?.as_str()?.trim().to_string(),
                        cost: r.get("cost")?.as_i64().filter(|c| *c > 0)?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PointsLook {
        channel_id: text(Some(user), "/id")?,
        display_name: text(Some(user), "/displayName").unwrap_or_else(|| login.clone()),
        login,
        avatar_url: text(Some(user), "/profileImageURL"),
        points_name: text(settings, "/name"),
        icon_url: text(settings, "/image/url4x"),
        default_icon_url: text(settings, "/defaultImage/url4x"),
        color: text(Some(user), "/primaryColorHex").as_deref().and_then(parse_hex),
        rewards,
    })
}

struct CachedLook {
    look: Option<PointsLook>,
    at: Instant,
}

static LOOKS: Lazy<Mutex<HashMap<String, CachedLook>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn cached_look(channel_id: &str) -> Option<Option<PointsLook>> {
    let looks = LOOKS.lock().unwrap_or_else(|e| e.into_inner());
    let hit = looks.get(channel_id)?;
    let ttl = if hit.look.is_some() { LOOK_TTL } else { LOOK_MISS_TTL };
    (hit.at.elapsed() < ttl).then(|| hit.look.clone())
}

fn remember_look(channel_id: &str, look: Option<PointsLook>) {
    let mut looks = LOOKS.lock().unwrap_or_else(|e| e.into_inner());
    if looks.len() >= LOOK_MAX && !looks.contains_key(channel_id) {
        if let Some(oldest) = looks.iter().min_by_key(|(_, c)| c.at).map(|(k, _)| k.clone()) {
            looks.remove(&oldest);
        }
    }
    looks.insert(channel_id.to_string(), CachedLook { look, at: Instant::now() });
}

async fn fetch_look(variables: Value) -> Option<PointsLook> {
    let response = crate::services::http::client()
        .post("https://gql.twitch.tv/gql")
        .header("Client-ID", crate::services::auth_proxy::TWITCH_WEB_CLIENT_ID)
        .timeout(LOOK_TIMEOUT)
        .json(&json!({ "query": LOOK_QUERY, "variables": variables }))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    parse_look(&response.json::<Value>().await.ok()?)
}

/// The points look of the channel with this Twitch id, cached.
pub async fn look(channel_id: &str) -> Option<PointsLook> {
    if channel_id.is_empty() || !channel_id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if let Some(hit) = cached_look(channel_id) {
        return hit;
    }
    let found = fetch_look(json!({ "id": channel_id })).await;
    remember_look(channel_id, found.clone());
    found
}

async fn look_by_login(login: &str) -> Option<PointsLook> {
    let found = fetch_look(json!({ "login": login })).await?;
    remember_look(&found.channel_id, Some(found.clone()));
    Some(found)
}

/// What the notification settings ask for right now.
#[derive(Debug, Clone, Copy, Default)]
struct Wants {
    points: bool,
    rewards_button: bool,
    next_reward: bool,
    predictions_started: bool,
}

fn wants(app: &AppHandle) -> Wants {
    let Some(state) = app.try_state::<AppState>() else { return Wants::default() };
    let settings = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    let n = &settings.live_notifications;
    Wants {
        points: n.enabled && n.show_channel_points_notifications,
        rewards_button: n.points_rewards_button,
        next_reward: n.points_next_reward,
        predictions_started: n.enabled && n.show_prediction_notifications,
    }
}

/// Fill in each channel's face and points look, for the page's own toast and
/// the notification list. Does nothing while points notifications are off.
pub async fn dress(app: &AppHandle, channels: &mut [ChannelEarned]) {
    if !wants(app).points {
        return;
    }
    let ids: Vec<String> = channels.iter().filter_map(|c| c.channel_id.clone()).take(DRESS_MAX).collect();
    let lookups = futures::future::join_all(ids.iter().map(|id| look(id)));
    let Ok(looks) = tokio::time::timeout(DRESS_WAIT, lookups).await else { return };
    for look in looks.into_iter().flatten() {
        if let Some(c) = channels.iter_mut().find(|c| c.channel_id.as_deref() == Some(look.channel_id.as_str())) {
            c.avatar_url = look.avatar_url.clone();
            c.points_name = look.points_name.clone();
            c.icon_url = look.icon_url.clone();
            c.login.get_or_insert(look.login.clone());
        }
    }
}

fn is_routine(code: &str) -> bool {
    matches!(code.to_ascii_uppercase().as_str(), "WATCH" | "AUTOMATION")
}

fn is_prediction(code: &str) -> bool {
    code.to_ascii_uppercase().contains("PREDICTION")
}

/// What a channel earned in this burst, less a prediction payout (the
/// prediction's own notification says that).
fn shown_points(c: &ChannelEarned) -> i64 {
    c.reasons.iter().filter(|r| !is_prediction(&r.name)).map(|r| r.points).sum()
}

fn notable(c: &ChannelEarned) -> bool {
    c.reasons.iter().any(|r| r.points > 0 && !is_routine(&r.name) && !is_prediction(&r.name))
}

fn is_streak(c: &ChannelEarned) -> bool {
    c.reasons.iter().any(|r| r.name.eq_ignore_ascii_case("WATCH_STREAK"))
}

/// The reason, worded for the notification. Codes not worded here stay
/// unnamed rather than guessed.
fn reason_words(code: &str) -> Option<&'static str> {
    Some(match code.to_ascii_uppercase().as_str() {
        "CLAIM" => "Bonus chest",
        "WATCH_STREAK" => "Watch streak",
        "RAID" => "Raid",
        _ => return None,
    })
}

#[derive(Debug, PartialEq)]
enum Plan {
    /// A popup for this channel.
    One(usize),
    /// One popup, a row for each of these channels.
    Rows(Vec<usize>),
    /// This channel's Action Center entry updates without popping up.
    Quiet(usize),
}

fn plan(channels: &[ChannelEarned], quiet_ok: bool) -> Vec<Plan> {
    let usable = |c: &ChannelEarned| c.channel_id.is_some() && shown_points(c) > 0;
    let picked: Vec<usize> = (0..channels.len()).filter(|&i| usable(&channels[i]) && notable(&channels[i])).collect();
    let mut plans = match picked.len() {
        0 => Vec::new(),
        1 => vec![Plan::One(picked[0])],
        _ => vec![Plan::Rows(picked)],
    };
    if quiet_ok {
        plans.extend((0..channels.len()).filter(|&i| usable(&channels[i]) && !notable(&channels[i])).map(Plan::Quiet));
    }
    plans
}

/// What a channel gave this session, for its quiet Action Center entry.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Tally {
    total: i64,
    chests: u32,
}

static TALLIES: Lazy<Mutex<HashMap<String, Tally>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn add_to_tally(c: &ChannelEarned) -> Option<Tally> {
    let id = c.channel_id.as_deref()?;
    let mut tallies = TALLIES.lock().unwrap_or_else(|e| e.into_inner());
    if tallies.len() >= LOOK_MAX && !tallies.contains_key(id) {
        return None;
    }
    let tally = tallies.entry(id.to_string()).or_default();
    tally.total += shown_points(c);
    tally.chests += c.chests;
    Some(*tally)
}

/// 12450 -> "12,450".
fn fmt_num(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        out.insert(0, '-');
    }
    out
}

/// The bar toward the cheapest reward the balance does not cover yet.
fn next_reward(rewards: &[RewardCost], balance: i64) -> Option<ToastProgress> {
    let next = rewards.iter().filter(|r| r.cost > balance).min_by_key(|r| r.cost)?;
    Some(ToastProgress {
        title: format!("Next reward: {}", next.title),
        value: balance.max(0) as f64 / next.cost as f64,
        value_text: format!("{} / {}", fmt_num(balance.max(0)), fmt_num(next.cost)),
        status: format!("{} to go", fmt_num(next.cost - balance.max(0))),
    })
}

// Pictures

fn coverage(distance: f32, radius: f32) -> f32 {
    (radius - distance + 0.5).clamp(0.0, 1.0)
}

/// `icon` scaled to fit a `side` square. Twice or more is by whole multiples
/// and nearest neighbour, so pixel art stays crisp; anything else is smooth.
fn fit(icon: &RgbaImage, side: u32) -> RgbaImage {
    let (w, h) = icon.dimensions();
    let longest = w.max(h).max(1);
    if side >= 2 * longest {
        let k = side / longest;
        return imageops::resize(icon, w * k, h * k, imageops::FilterType::Nearest);
    }
    let scale = side as f32 / longest as f32;
    let (sw, sh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
    imageops::resize(icon, sw, sh, imageops::FilterType::Lanczos3)
}

fn overlay_centered(base: &mut RgbaImage, top: &RgbaImage, cx: u32, cy: u32) {
    let x = cx as i64 - top.width() as i64 / 2;
    let y = cy as i64 - top.height() as i64 / 2;
    imageops::overlay(base, top, x, y);
}

/// The grey of the default orb's body (measured: every opaque pixel of
/// orb-4.png reads 107), so tinting against it gives the body exactly the
/// channel's color and leaves the shine lighter.
const ORB_BODY_LUMA: f32 = 107.0;

/// Twitch's default orb is grey; the site draws it in the channel's color.
fn tint(icon: &RgbaImage, color: [u8; 3]) -> RgbaImage {
    let mut out = icon.clone();
    for px in out.pixels_mut() {
        let lum = 0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32;
        for c in 0..3 {
            px[c] = (lum / ORB_BODY_LUMA * color[c] as f32).round().min(255.0) as u8;
        }
    }
    out
}

/// The points icon's side on the badge, the ring cut around it, and how far it
/// sits in from the corner so an icon drawn edge to edge never reads as cut off.
const BADGE_ICON: f32 = 0.44;
const BADGE_RING: f32 = 0.035;
const BADGE_INSET: f32 = 2.0;

/// The badge icon's centre (on both axes) and side, in badge pixels.
fn badge_icon_box() -> (f32, f32) {
    let side = (BADGE_SIZE as f32 * BADGE_ICON).round();
    (BADGE_SIZE as f32 - side / 2.0 - BADGE_INSET, side)
}

/// The face, round, with the points icon in its lower right corner and a
/// transparent ring cut around the icon.
fn draw_badge(face: &RgbaImage, icon: Option<&RgbaImage>) -> RgbaImage {
    let size = BADGE_SIZE as f32;
    let mut out = imageops::resize(face, BADGE_SIZE, BADGE_SIZE, imageops::FilterType::Lanczos3);
    let (centre, badge) = badge_icon_box();
    let ring = (size * BADGE_RING).round();
    let half = size / 2.0;
    for (x, y, px) in out.enumerate_pixels_mut() {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut alpha = coverage(((fx - half).powi(2) + (fy - half).powi(2)).sqrt(), half);
        if icon.is_some() {
            let d = ((fx - centre).powi(2) + (fy - centre).powi(2)).sqrt();
            alpha *= 1.0 - coverage(d, badge / 2.0 + ring);
        }
        px[3] = (px[3] as f32 * alpha).round() as u8;
    }
    if let Some(icon) = icon {
        let scaled = fit(icon, badge as u32);
        overlay_centered(&mut out, &scaled, centre as u32, centre as u32);
    }
    out
}

/// A channel color too dark to read as a color is lifted a little.
fn lift(color: [u8; 3]) -> [u8; 3] {
    let max = *color.iter().max().unwrap_or(&0) as f32;
    if max >= 96.0 {
        return color;
    }
    let k = 96.0 / max.max(1.0);
    color.map(|c| (c as f32 * k).min(255.0) as u8)
}

/// The banner for a rare moment: the channel's color, darker at the edges,
/// with a soft light behind its points icon. No text on it.
fn draw_hero(icon: &RgbaImage, color: [u8; 3]) -> RgbaImage {
    let color = lift(color);
    let (w, h) = (HERO_W as f32, HERO_H as f32);
    let (rx, ry) = (w * 0.42, h * 0.95);
    let mut out = RgbaImage::from_fn(HERO_W, HERO_H, |x, y| {
        let (px, py) = (x as f32 + 0.5 - w / 2.0, y as f32 + 0.5 - h / 2.0);
        let t = (1.0 - ((px / rx).powi(2) + (py / ry).powi(2)).sqrt()).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let mix = 0.18 + 0.42 * t;
        let glow = (1.0 - (px * px + py * py).sqrt() / (h * 0.42)).clamp(0.0, 1.0).powi(2) * 0.22;
        let ch = |c: u8| (c as f32 * mix + 255.0 * glow).min(255.0) as u8;
        Rgba([ch(color[0]), ch(color[1]), ch(color[2]), 255])
    });
    let scaled = fit(icon, HERO_ICON);
    overlay_centered(&mut out, &scaled, HERO_W / 2, HERO_H / 2);
    out
}

fn art_path(kind: &str, parts: &[&str]) -> Option<PathBuf> {
    let key = system_notify::image_key(&parts.join("|"));
    Some(system_notify::image_dir()?.join(format!("{kind}-{key}.png")))
}

async fn picture(url: &str) -> Option<RgbaImage> {
    let path = system_notify::local_image(url).await?;
    tauri::async_runtime::spawn_blocking(move || image::open(path).ok().map(|i| i.to_rgba8())).await.ok().flatten()
}

fn icon_key(look: &PointsLook) -> String {
    match &look.icon_url {
        Some(url) => url.clone(),
        None => format!("{}#{:?}", look.default_icon_url.as_deref().unwrap_or(""), look.color),
    }
}

/// The channel's points icon as drawn: its own, or the default orb in its color.
async fn icon_image(look: &PointsLook) -> Option<RgbaImage> {
    if let Some(url) = &look.icon_url {
        return picture(url).await;
    }
    let orb = picture(look.default_icon_url.as_deref()?).await?;
    Some(tint(&orb, look.color.unwrap_or(DEFAULT_COLOR)))
}

async fn save(path: PathBuf, draw: impl FnOnce() -> RgbaImage + Send + 'static) -> Option<PathBuf> {
    let out = path.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || draw().save(&out)).await.ok()?;
    if let Err(e) = saved {
        log::debug!("[PointsNotify] could not save {}: {e}", path.display());
        return None;
    }
    Some(path)
}

async fn badge_for(look: &PointsLook) -> Option<PathBuf> {
    let avatar = look.avatar_url.clone()?;
    let path = art_path("badge", &[ART_VERSION, &avatar, &icon_key(look)])?;
    if path.is_file() {
        return Some(path);
    }
    let (face, icon) = tokio::join!(picture(&avatar), icon_image(look));
    let face = face?;
    save(path, move || draw_badge(&face, icon.as_ref())).await
}

async fn hero_for(look: &PointsLook) -> Option<PathBuf> {
    let color = look.color.unwrap_or(DEFAULT_COLOR);
    let path = art_path("hero", &[ART_VERSION, &icon_key(look), &format!("{color:?}")])?;
    if path.is_file() {
        return Some(path);
    }
    let icon = icon_image(look).await?;
    save(path, move || draw_hero(&icon, color)).await
}

// Notifications

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", chrono::Utc::now().timestamp_millis())
}

/// The parts every points notification shares: its Action Center heading,
/// the tag that makes a newer one replace it, and how long it stays.
fn base_rich(tag: String, keep_for: Duration) -> RichToast {
    RichToast {
        header: Some((HEADER.0.to_string(), HEADER.1.to_string())),
        tag: Some(tag),
        group: Some(GROUP.to_string()),
        expires_in: Some(keep_for),
        ..Default::default()
    }
}

/// The extras the viewer turned on: the bar toward the next reward, and the
/// Rewards button with a right-click way to these settings.
fn add_extras(rich: &mut RichToast, look: &PointsLook, balance: Option<i64>, wants: Wants) {
    if wants.next_reward {
        rich.progress = balance.and_then(|b| next_reward(&look.rewards, b));
    }
    if wants.rewards_button {
        rich.buttons.push(("Rewards".into(), NoticeAction::Rewards { channel: look.login.clone() }));
        rich.menu.push(("Notification settings".into(), NoticeAction::Settings { tab: "Notifications".into() }));
    }
}

fn points_notice(id: String, look: &PointsLook, title: String, body: String, system: bool) -> MirroredNotice {
    MirroredNotice {
        id,
        kind: "channel_points".into(),
        title,
        body,
        image_url: look.avatar_url.clone(),
        round_image: true,
        // The system notification opens the channel from its body; a MultiChat
        // card needs a label to be clickable at all.
        action_label: (!system).then(|| "Watch".to_string()),
        action: Some(NoticeAction::Watch { channel: look.login.clone() }),
        reply: None,
    }
}

/// One channel's notable earn: a bonus chest, a raid, a watch streak.
async fn one_channel(
    c: &ChannelEarned,
    look: &PointsLook,
    streak: Option<i32>,
    wants: Wants,
    system: bool,
) -> (MirroredNotice, Option<RichToast>) {
    let points = fmt_num(shown_points(c));
    let currency = look.currency();
    let name = &look.display_name;
    let streak_moment = is_streak(c);
    let (title, body) = if streak_moment {
        let title = match streak.filter(|n| *n > 0) {
            Some(n) => format!("{n}-stream watch streak"),
            None => "Watch streak".to_string(),
        };
        (title, format!("+{points} {currency} \u{b7} {name}"))
    } else {
        let mut words: Vec<&str> = c.reasons.iter().filter(|r| r.points > 0).filter_map(|r| reason_words(&r.name)).collect();
        words.dedup();
        let body = if words.is_empty() { name.clone() } else { format!("{name} \u{b7} {}", words.join(", ")) };
        (format!("+{points} {currency}"), body)
    };
    let notice = points_notice(new_id(&format!("points-{}", look.channel_id)), look, title, body, system);
    if !system {
        return (notice, None);
    }
    let mut rich = base_rich(format!("pts-{}", look.channel_id), KEEP_FOR);
    rich.logo_path = badge_for(look).await;
    rich.attribution = c.balance.map(|b| format!("Balance {}", fmt_num(b)));
    if streak_moment {
        rich.hero_path = hero_for(look).await;
    }
    add_extras(&mut rich, look, c.balance, wants);
    (notice, Some(rich))
}

/// A channel's quiet Action Center entry: what it gave this session.
async fn quiet_entry(c: &ChannelEarned, look: &PointsLook, tally: Tally, wants: Wants) -> (MirroredNotice, RichToast) {
    let title = format!("+{} {} this session", fmt_num(tally.total), look.currency());
    let mut body = format!("{} \u{b7} Watching", look.display_name);
    if tally.chests > 0 {
        let s = if tally.chests == 1 { "" } else { "s" };
        body.push_str(&format!(", {} bonus chest{s}", tally.chests));
    }
    let notice = points_notice(new_id(&format!("points-{}", look.channel_id)), look, title, body, true);
    let mut rich = base_rich(format!("pts-{}", look.channel_id), KEEP_FOR);
    rich.quiet = true;
    rich.logo_path = badge_for(look).await;
    rich.attribution = c.balance.map(|b| format!("Balance {}", fmt_num(b)));
    add_extras(&mut rich, look, c.balance, wants);
    (notice, rich)
}

/// Several channels at once: a row each, busiest first.
fn several(picked: &[(&ChannelEarned, PointsLook)], system: bool) -> (MirroredNotice, Option<RichToast>) {
    let line = |c: &ChannelEarned, l: &PointsLook| format!("+{} {}", fmt_num(shown_points(c)), l.currency());
    let first = &picked[0].1;
    let mut notice = points_notice(
        new_id("points-several"),
        first,
        format!("Points from {} channels", picked.len()),
        String::new(),
        system,
    );
    if !system {
        notice.body = picked.iter().map(|(c, l)| format!("{} {}", l.display_name, line(c, l))).collect::<Vec<_>>().join(", ");
        return (notice, None);
    }
    let mut rich = base_rich("pts-several".into(), KEEP_FOR);
    rich.rows = picked
        .iter()
        .take(MAX_ROWS)
        .map(|(c, l)| ToastRow { face_url: l.avatar_url.clone(), name: l.display_name.clone(), value: line(c, l) })
        .collect();
    if picked.len() > MAX_ROWS {
        rich.attribution = Some(format!("and {} more", picked.len() - MAX_ROWS));
    }
    (notice, Some(rich))
}

async fn streak_count(channel_id: &str) -> Option<i32> {
    let ask = crate::commands::watch_streak::get_watch_streak(channel_id.to_string());
    tokio::time::timeout(STREAK_WAIT, ask).await.ok()?.ok()?.map(|m| m.streak_count)
}

/// A burst of earns, gathered by channel_points_summary.rs and dressed.
pub async fn on_burst(app: AppHandle, channels: Vec<ChannelEarned>) {
    let tallies: HashMap<String, Tally> = channels
        .iter()
        .filter_map(|c| Some((c.channel_id.clone()?, add_to_tally(c)?)))
        .collect();
    let wants = wants(&app);
    if !wants.points {
        return;
    }
    let route = notification_mirror::current_route(&app);
    if route == Route::Nowhere {
        return;
    }
    let system = route == Route::System;
    for step in plan(&channels, system && cfg!(windows)) {
        let built = match step {
            Plan::One(i) => {
                let c = &channels[i];
                let Some(look) = look(c.channel_id.as_deref().unwrap_or_default()).await else { continue };
                let streak = if is_streak(c) { streak_count(&look.channel_id).await } else { None };
                one_channel(c, &look, streak, wants, system).await
            }
            Plan::Rows(picked) => {
                let mut rows = Vec::new();
                for i in picked {
                    if let Some(look) = look(channels[i].channel_id.as_deref().unwrap_or_default()).await {
                        rows.push((&channels[i], look));
                    }
                }
                match rows.len() {
                    0 => continue,
                    1 => one_channel(rows[0].0, &rows[0].1, None, wants, system).await,
                    _ => several(&rows, system),
                }
            }
            Plan::Quiet(i) => {
                let c = &channels[i];
                let id = c.channel_id.as_deref().unwrap_or_default();
                let (Some(look), Some(tally)) = (look(id).await, tallies.get(id).copied()) else { continue };
                let (notice, rich) = quiet_entry(c, &look, tally, wants).await;
                (notice, Some(rich))
            }
        };
        notification_mirror::deliver_with(app.clone(), built.0, built.1).await;
    }
}

// Predictions

#[derive(Debug, Clone, Default, PartialEq)]
struct Prediction {
    id: String,
    channel_id: String,
    title: String,
    /// (outcome id, outcome title)
    outcomes: Vec<(String, String)>,
    winner: Option<String>,
    window_secs: i64,
}

static PREDICTIONS: Lazy<Mutex<VecDeque<Prediction>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static RESULTS_SHOWN: Lazy<Mutex<VecDeque<String>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

/// Remember what a prediction on a watched channel asks, from the channel's
/// prediction events (`event` is Twitch's event object).
pub fn note_prediction(channel_id: &str, event: &Value) {
    let Some(id) = event.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else { return };
    let text = |k: &str| event.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let outcomes: Vec<(String, String)> = event
        .get("outcomes")
        .and_then(|o| o.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|o| Some((o.get("id")?.as_str()?.to_string(), o.get("title")?.as_str()?.trim().to_string())))
                .collect()
        })
        .unwrap_or_default();
    let mut kept = PREDICTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let p = match kept.iter().position(|p| p.id == id) {
        Some(i) => &mut kept[i],
        None => {
            kept.push_back(Prediction { id: id.to_string(), channel_id: channel_id.to_string(), ..Default::default() });
            while kept.len() > PREDICTIONS_KEPT {
                kept.pop_front();
            }
            kept.back_mut().expect("just pushed")
        }
    };
    if !text("title").is_empty() {
        p.title = text("title").trim().to_string();
    }
    if !outcomes.is_empty() {
        p.outcomes = outcomes;
    }
    if let Some(w) = event.get("winning_outcome_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        p.winner = Some(w.to_string());
    }
    if let Some(secs) = event.get("prediction_window_seconds").and_then(|v| v.as_i64()).filter(|s| *s > 0) {
        p.window_secs = secs;
    }
}

fn prediction(id: &str) -> Option<Prediction> {
    PREDICTIONS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|p| p.id == id).cloned()
}

fn outcome_title(p: &Prediction, outcome_id: &str) -> Option<String> {
    p.outcomes.iter().find(|(id, _)| id == outcome_id).map(|(_, t)| t.clone())
}

/// "Yes · No", or the first three and how many more.
fn outcome_list(p: &Prediction) -> String {
    let mut titles: Vec<String> = p.outcomes.iter().take(3).map(|(_, t)| t.clone()).collect();
    if p.outcomes.len() > 3 {
        titles.push(format!("{} more", p.outcomes.len() - 3));
    }
    titles.join(" \u{b7} ")
}

fn closes_in(secs: i64) -> String {
    format!("Closes in {}:{:02}", secs / 60, secs % 60)
}

async fn balance_on(login: &str) -> Option<i64> {
    crate::services::channel_state::get(login).await?.points?.balance
}

fn prediction_started_notice(
    look: &PointsLook,
    p: &Prediction,
    balance: Option<i64>,
    system: bool,
) -> (MirroredNotice, Option<RichToast>) {
    let notice = MirroredNotice {
        id: new_id(&format!("prediction-{}", p.id)),
        kind: "channel_points".into(),
        title: format!("{} started a prediction", look.display_name),
        body: p.title.clone(),
        image_url: look.avatar_url.clone(),
        round_image: true,
        action_label: Some("Open prediction".into()),
        action: Some(NoticeAction::Watch { channel: look.login.clone() }),
        reply: None,
    };
    if !system {
        return (notice, None);
    }
    let window = p.window_secs.max(30);
    let mut rich = base_rich(format!("predopen-{}", p.id), Duration::from_secs(window as u64));
    rich.progress = Some(ToastProgress {
        title: outcome_list(p),
        value: 1.0,
        value_text: closes_in(window),
        status: balance.map(|b| format!("You have {} {}", fmt_num(b), look.currency())).unwrap_or_default(),
    });
    (notice, Some(rich))
}

/// A prediction opened on a watched channel (opt-in).
pub async fn prediction_started(app: AppHandle, channel_id: String, prediction_id: String) {
    if !wants(&app).predictions_started {
        return;
    }
    let route = notification_mirror::current_route(&app);
    if route == Route::Nowhere {
        return;
    }
    let (Some(p), Some(look)) = (prediction(&prediction_id), look(&channel_id).await) else { return };
    let balance = balance_on(&look.login).await;
    let system = route == Route::System;
    let (notice, mut rich) = prediction_started_notice(&look, &p, balance, system);
    if let Some(rich) = rich.as_mut() {
        rich.logo_path = badge_for(&look).await;
    }
    notification_mirror::deliver_with(app, notice, rich).await;
}

/// Betting closed: the "started" notification leaves Action Center.
pub fn prediction_closed(app: &AppHandle, prediction_id: &str) {
    if cfg!(windows) && wants(app).predictions_started {
        system_notify::remove(app, &format!("predopen-{prediction_id}"), GROUP);
    }
}

fn prediction_result_notice(
    look: &PointsLook,
    p: Option<&Prediction>,
    outcome_id: &str,
    result: &str,
    points: i64,
    points_won: Option<i64>,
    system: bool,
) -> Option<(MirroredNotice, Option<RichToast>, bool)> {
    let currency = look.currency();
    let question = p.map(|p| p.title.clone()).filter(|t| !t.is_empty());
    let with = |extra: Option<String>| match (question.clone(), extra) {
        (Some(q), Some(e)) => format!("{q} \u{b7} {e}"),
        (Some(q), None) => q,
        (None, Some(e)) => e,
        (None, None) => look.display_name.clone(),
    };
    let (title, body, won) = match result.to_ascii_uppercase().as_str() {
        "WIN" => (
            format!("You won {} {currency}", fmt_num(points_won.filter(|w| *w > 0).unwrap_or(points))),
            with(p.and_then(|p| outcome_title(p, outcome_id)).map(|t| format!("You picked {t}"))),
            true,
        ),
        "LOSE" => (
            format!("Prediction lost: {} {currency}", fmt_num(points)),
            with(p.and_then(|p| outcome_title(p, p.winner.as_deref()?)).map(|t| format!("{t} won"))),
            false,
        ),
        "REFUND" => (format!("{} {currency} refunded", fmt_num(points)), with(None), false),
        _ => return None,
    };
    let event = p.map(|p| p.id.clone()).unwrap_or_else(|| look.channel_id.clone());
    let notice = MirroredNotice {
        id: new_id(&format!("prediction-{event}")),
        kind: "channel_points".into(),
        title,
        body,
        image_url: look.avatar_url.clone(),
        round_image: true,
        action_label: (!system).then(|| "Watch".to_string()),
        action: Some(NoticeAction::Watch { channel: look.login.clone() }),
        reply: None,
    };
    if !system {
        return Some((notice, None, won));
    }
    let mut rich = base_rich(format!("pred-{event}"), KEEP_FOR);
    rich.attribution = Some(look.display_name.clone());
    Some((notice, Some(rich), won))
}

/// The viewer's own prediction settled: won, lost or refunded.
pub async fn prediction_result(
    app: AppHandle,
    channel_id: String,
    event_id: String,
    outcome_id: String,
    result: String,
    points: i64,
    points_won: Option<i64>,
) {
    {
        let mut shown = RESULTS_SHOWN.lock().unwrap_or_else(|e| e.into_inner());
        if event_id.is_empty() || shown.contains(&event_id) {
            return;
        }
        shown.push_back(event_id.clone());
        while shown.len() > PREDICTIONS_KEPT {
            shown.pop_front();
        }
    }
    if !wants(&app).points {
        return;
    }
    let route = notification_mirror::current_route(&app);
    if route == Route::Nowhere {
        return;
    }
    let Some(look) = look(&channel_id).await else { return };
    let p = prediction(&event_id);
    let system = route == Route::System;
    let Some((notice, mut rich, won)) =
        prediction_result_notice(&look, p.as_ref(), &outcome_id, &result, points, points_won, system)
    else {
        return;
    };
    if let Some(rich) = rich.as_mut() {
        rich.logo_path = badge_for(&look).await;
        if won {
            rich.hero_path = hero_for(&look).await;
        }
    }
    notification_mirror::deliver_with(app, notice, rich).await;
}

// The Settings preview and the hand test

const SAMPLE_QUESTION: &str = "Will he finish the run tonight?";

fn sample_burst(look: &PointsLook, reasons: &[(&str, i64)], balance: i64) -> ChannelEarned {
    ChannelEarned {
        name: look.display_name.clone(),
        points: reasons.iter().map(|r| r.1).sum(),
        channel_id: Some(look.channel_id.clone()),
        login: Some(look.login.clone()),
        reasons: reasons
            .iter()
            .map(|(name, points)| crate::services::channel_points_summary::Earned { name: name.to_string(), points: *points })
            .collect(),
        balance: Some(balance),
        chests: reasons.iter().filter(|r| r.0 == "CLAIM").count() as u32,
        ..Default::default()
    }
}

fn sample_prediction(look: &PointsLook) -> Prediction {
    Prediction {
        id: "sample".into(),
        channel_id: look.channel_id.clone(),
        title: SAMPLE_QUESTION.into(),
        outcomes: vec![("yes".into(), "Yes".into()), ("no".into(), "No".into())],
        winner: Some("yes".into()),
        window_secs: 120,
    }
}

/// A sample of `kind` as a system notification, on real public channels with
/// made-up numbers. Nothing is earned, spent or bet.
async fn sample(kind: &str, wants: Wants) -> Result<(MirroredNotice, RichToast), String> {
    let get = |login: &'static str| async move { look_by_login(login).await.ok_or(format!("could not read {login}")) };
    let built = match kind {
        "one" => {
            let l = get("ludwig").await?;
            one_channel(&sample_burst(&l, &[("CLAIM", 50)], 830), &l, None, wants, true).await
        }
        "orb" => {
            let l = get("xqc").await?;
            one_channel(&sample_burst(&l, &[("CLAIM", 50)], 3210), &l, None, wants, true).await
        }
        "streak" => {
            let l = get("ludwig").await?;
            one_channel(&sample_burst(&l, &[("WATCH_STREAK", 450)], 12_900), &l, Some(5), wants, true).await
        }
        "several" => {
            let (a, b, c) = (get("ludwig").await?, get("lirik").await?, get("atrioc").await?);
            let bursts = [
                sample_burst(&a, &[("CLAIM", 50)], 830),
                sample_burst(&b, &[("RAID", 250)], 18_650),
                sample_burst(&c, &[("CLAIM", 50)], 4_020),
            ];
            let picked = [(&bursts[1], b.clone()), (&bursts[0], a.clone()), (&bursts[2], c.clone())];
            several(&picked, true)
        }
        "quiet" => {
            let l = get("lirik").await?;
            let (notice, mut rich) =
                quiet_entry(&sample_burst(&l, &[("WATCH", 10)], 18_650), &l, Tally { total: 120, chests: 2 }, wants).await;
            // A preview has to be seen: the real one never pops up.
            rich.quiet = false;
            (notice, Some(rich))
        }
        "won" | "lost" | "refunded" => {
            let l = get("lirik").await?;
            let p = sample_prediction(&l);
            let (result, outcome) = match kind {
                "won" => ("WIN", "yes"),
                "lost" => ("LOSE", "no"),
                _ => ("REFUND", "yes"),
            };
            let (notice, mut rich, won) =
                prediction_result_notice(&l, Some(&p), outcome, result, 500, Some(12_400), true).ok_or("no notice")?;
            if let Some(rich) = rich.as_mut() {
                rich.logo_path = badge_for(&l).await;
                if won {
                    rich.hero_path = hero_for(&l).await;
                }
            }
            (notice, rich)
        }
        "prediction" => {
            let l = get("lirik").await?;
            let (notice, mut rich) = prediction_started_notice(&l, &sample_prediction(&l), Some(18_650), true);
            if let Some(rich) = rich.as_mut() {
                rich.logo_path = badge_for(&l).await;
            }
            (notice, rich)
        }
        other => return Err(format!("unknown sample {other}")),
    };
    let (notice, rich) = built;
    let mut rich = rich.unwrap_or_default();
    // Samples never replace a real notification.
    rich.tag = rich.tag.map(|t| format!("sample-{t}").chars().take(64).collect::<String>());
    Ok((notice, rich))
}

/// Settings > Notifications: show a sample channel points or prediction
/// notification (`kind`: one, orb, several, streak, quiet, won, lost,
/// refunded, prediction) with the extras the settings have on, whichever
/// window is in use.
#[tauri::command]
pub async fn test_points_notification(app: AppHandle, kind: String) -> Result<(), String> {
    let (notice, rich) = sample(&kind, wants(&app)).await?;
    system_notify::show_with(app, notice, Some(rich)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::channel_points_summary::Earned;

    fn burst(id: &str, reasons: &[(&str, i64)]) -> ChannelEarned {
        ChannelEarned {
            name: id.into(),
            points: reasons.iter().map(|r| r.1).sum(),
            channel_id: Some(id.into()),
            reasons: reasons.iter().map(|(n, p)| Earned { name: n.to_string(), points: *p }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn parses_the_public_look() {
        let v: Value = serde_json::from_str(
            r##"{"data":{"user":{"id":"40934651","login":"ludwig","displayName":"Ludwig","primaryColorHex":"00FFE2",
            "profileImageURL":"https://x/face.png","channel":{"communityPointsSettings":{"name":"Goldwig",
            "image":{"url4x":"https://x/icon-4.png"},"defaultImage":{"url4x":"https://x/orb-4.png"},
            "customRewards":[{"title":" Water break! ","cost":1000,"isEnabled":true,"isPaused":false},
            {"title":"Off","cost":5,"isEnabled":false,"isPaused":false},
            {"title":"Paused","cost":7,"isEnabled":true,"isPaused":true}]}}}}}"##,
        )
        .unwrap();
        let look = parse_look(&v).unwrap();
        assert_eq!(look.channel_id, "40934651");
        assert_eq!(look.currency(), "Goldwig");
        assert_eq!(look.color, Some([0x00, 0xFF, 0xE2]));
        assert_eq!(look.icon_url.as_deref(), Some("https://x/icon-4.png"));
        assert_eq!(look.rewards, vec![RewardCost { title: "Water break!".into(), cost: 1000 }]);
        // No custom name or icon: Twitch's own words and the default orb.
        let plain: Value = serde_json::from_str(
            r#"{"data":{"user":{"id":"1","login":"xqc","displayName":"xQc","primaryColorHex":null,
            "profileImageURL":"https://x/f.png","channel":{"communityPointsSettings":{"name":null,"image":null,
            "defaultImage":{"url4x":"https://x/orb-4.png"}}}}}}"#,
        )
        .unwrap();
        let look = parse_look(&plain).unwrap();
        assert_eq!(look.currency(), "Channel Points");
        assert!(look.icon_url.is_none() && look.color.is_none() && look.rewards.is_empty());
        assert!(parse_look(&serde_json::json!({"data":{"user":null}})).is_none());
    }

    #[test]
    fn routine_earns_go_quiet_and_notable_ones_pop() {
        let watch = burst("1", &[("WATCH", 10)]);
        let chest = burst("2", &[("claim", 50), ("WATCH", 10)]);
        let raid = burst("3", &[("RAID", 250)]);
        let poll = burst("4", &[("automation", 10)]);
        assert_eq!(plan(&[watch.clone()], true), vec![Plan::Quiet(0)]);
        assert_eq!(plan(&[watch.clone(), chest.clone()], true), vec![Plan::One(1), Plan::Quiet(0)]);
        assert_eq!(
            plan(&[chest.clone(), raid.clone(), poll.clone()], true),
            vec![Plan::Rows(vec![0, 1]), Plan::Quiet(2)]
        );
        // Where nothing can update quietly, routine earns say nothing.
        assert_eq!(plan(&[watch, poll, chest], false), vec![Plan::One(2)]);
    }

    #[test]
    fn a_prediction_payout_is_left_to_the_prediction() {
        let payout = burst("1", &[("PREDICTION", 12_400)]);
        assert!(plan(&[payout], true).is_empty());
        let mixed = burst("1", &[("PREDICTION", 12_400), ("CLAIM", 50)]);
        assert_eq!(shown_points(&mixed), 50);
        let unnamed = ChannelEarned { channel_id: None, ..burst("1", &[("CLAIM", 50)]) };
        assert!(plan(&[unnamed], true).is_empty());
    }

    #[test]
    fn numbers_and_the_next_reward() {
        assert_eq!(fmt_num(0), "0");
        assert_eq!(fmt_num(999), "999");
        assert_eq!(fmt_num(12_450), "12,450");
        assert_eq!(fmt_num(-1_234_567), "-1,234,567");
        let rewards = vec![
            RewardCost { title: "Cheap".into(), cost: 100 },
            RewardCost { title: "Water break!".into(), cost: 1000 },
            RewardCost { title: "Big".into(), cost: 50_000 },
        ];
        let bar = next_reward(&rewards, 830).unwrap();
        assert_eq!(bar.title, "Next reward: Water break!");
        assert_eq!(bar.value_text, "830 / 1,000");
        assert_eq!(bar.status, "170 to go");
        assert!((bar.value - 0.83).abs() < 1e-9);
        assert!(next_reward(&rewards, 60_000).is_none(), "nothing left to save for");
    }

    #[test]
    fn the_badge_is_round_with_a_ring_cut_around_the_icon() {
        let face = RgbaImage::from_pixel(300, 300, Rgba([200, 50, 50, 255]));
        let icon = RgbaImage::from_pixel(112, 112, Rgba([255, 215, 0, 255]));
        let badge = draw_badge(&face, Some(&icon));
        assert_eq!(badge.dimensions(), (BADGE_SIZE, BADGE_SIZE));
        assert_eq!(badge.get_pixel(0, 0)[3], 0, "corners are outside the circle");
        assert_eq!(badge.get_pixel(96, 40)[3], 255, "the face is solid");
        assert_eq!(badge.get_pixel(150, 150)[0], 255, "the icon sits in the corner");
        let (centre, side) = badge_icon_box();
        assert!(centre + side / 2.0 <= BADGE_SIZE as f32 - BADGE_INSET, "the icon keeps off the edge");
        // The ring between face and icon is see-through.
        let ring = (centre - side / 2.0 - 3.0) as u32;
        assert_eq!(badge.get_pixel(ring, centre as u32)[3], 0);
        let plain = draw_badge(&face, None);
        assert_eq!(plain.get_pixel(150, 150)[3], 255, "no icon, no cut");
    }

    #[test]
    fn pixel_art_scales_by_whole_steps_and_the_orb_takes_the_color() {
        let icon = RgbaImage::from_pixel(112, 112, Rgba([10, 10, 10, 255]));
        assert_eq!(fit(&icon, HERO_ICON).dimensions(), (224, 224));
        assert_eq!(fit(&RgbaImage::new(72, 72), HERO_ICON).dimensions(), (216, 216));
        assert_eq!(fit(&icon, 88).dimensions(), (88, 88));
        let orb = RgbaImage::from_pixel(4, 4, Rgba([107, 107, 107, 255]));
        assert_eq!(*tint(&orb, [255, 191, 0]).get_pixel(0, 0), Rgba([255, 191, 0, 255]));
        assert!(lift([0x2A, 0x29, 0x2E])[2] >= 95, "a near-black color is lifted");
        assert_eq!(lift([0x00, 0xFF, 0xE2]), [0x00, 0xFF, 0xE2]);
        let hero = draw_hero(&icon, [0, 255, 226]);
        assert_eq!(hero.dimensions(), (HERO_W, HERO_H));
        assert_eq!(hero.get_pixel(HERO_W / 2, HERO_H / 2)[0], 10, "the icon is centered");
    }

    #[test]
    fn prediction_words() {
        let look = PointsLook {
            channel_id: "23161357".into(),
            login: "lirik".into(),
            display_name: "LIRIK".into(),
            points_name: Some("PotatoCoin".into()),
            ..Default::default()
        };
        let p = Prediction {
            id: "e1".into(),
            title: "Will he finish?".into(),
            outcomes: vec![("a".into(), "Yes".into()), ("b".into(), "No".into())],
            winner: Some("a".into()),
            ..Default::default()
        };
        let (won, _, hero) = prediction_result_notice(&look, Some(&p), "a", "WIN", 500, Some(12_400), true).unwrap();
        assert_eq!(won.title, "You won 12,400 PotatoCoin");
        assert_eq!(won.body, "Will he finish? \u{b7} You picked Yes");
        assert!(hero);
        let (lost, rich, hero) = prediction_result_notice(&look, Some(&p), "b", "LOSE", 500, None, true).unwrap();
        assert_eq!(lost.title, "Prediction lost: 500 PotatoCoin");
        assert_eq!(lost.body, "Will he finish? \u{b7} Yes won");
        assert!(!hero);
        assert_eq!(rich.unwrap().tag.as_deref(), Some("pred-e1"));
        let (refund, _, _) = prediction_result_notice(&look, None, "a", "REFUND", 500, None, false).unwrap();
        assert_eq!(refund.title, "500 PotatoCoin refunded");
        assert_eq!(refund.body, "LIRIK");
        assert!(prediction_result_notice(&look, None, "a", "SOMETHING", 1, None, true).is_none());
        let many = Prediction {
            outcomes: (0..5).map(|n| (n.to_string(), format!("O{n}"))).collect(),
            ..p.clone()
        };
        assert_eq!(outcome_list(&many), "O0 \u{b7} O1 \u{b7} O2 \u{b7} 2 more");
        assert_eq!(closes_in(125), "Closes in 2:05");
    }

    #[test]
    fn a_prediction_is_remembered_and_updated() {
        let created = serde_json::json!({"id":"p-77","title":"Q?","prediction_window_seconds":90,
            "outcomes":[{"id":"x","title":"Yes"},{"id":"y","title":"No"}]});
        note_prediction("5", &created);
        note_prediction("5", &serde_json::json!({"id":"p-77","winning_outcome_id":"y"}));
        let p = prediction("p-77").unwrap();
        assert_eq!((p.title.as_str(), p.window_secs, p.winner.as_deref()), ("Q?", 90, Some("y")));
        assert_eq!(outcome_title(&p, "x").as_deref(), Some("Yes"));
    }

    /// Shows every sample as a real toast under the dev identity, drawing the
    /// pictures from the real public channels. Run by hand:
    /// `cargo test --no-default-features --lib points_notify -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn shows_every_sample() {
        let all = Wants { points: true, rewards_button: true, next_reward: true, predictions_started: true };
        let only = std::env::var("SN_POINTS_SAMPLE").ok();
        let kinds = ["one", "orb", "several", "streak", "quiet", "won", "lost", "refunded", "prediction"];
        tauri::async_runtime::block_on(async {
            for kind in kinds.iter().filter(|k| only.as_deref().is_none_or(|o| o == **k)) {
                let (notice, rich) = sample(kind, all).await.expect("sample failed");
                let mut faces = Vec::new();
                for row in &rich.rows {
                    faces.push(match row.face_url.as_deref() {
                        Some(url) => system_notify::local_image(url).await,
                        None => None,
                    });
                }
                println!("{kind}: {}", system_notify::toast_xml(&notice, None, Some(&rich), &faces));
                system_notify::show_sample(&notice, &rich, &faces).expect("toast refused");
                std::thread::sleep(Duration::from_millis(400));
            }
        });
    }
}
