//! What kind of hype train a channel is running, and how every StreamNook
//! surface draws it. Twitch runs several kinds now (`hypeTrainType`: REGULAR,
//! GOLDEN_KAPPA, TREASURE, MYTHIC, COMMUNITY, PINATA) and themes some regular
//! ones: during the GTA VI event a regular train on a channel streaming GTA VI
//! (or a shared train with any participant there) is a Heist Train. Twitch
//! gates that theme behind an experiment flag nobody outside can read, so it is
//! gated here on the event's window instead.
//!
//! The words and the palette are decided here once; the desktop, Android and
//! iPhone bars and the stream-card badges draw what this says and pick no
//! colors of their own. A regular train sends no palette: the apps keep their
//! own theme-colored rainbow for it.

use serde::{Deserialize, Serialize};

/// Where Twitch keeps its hype train marks (read from twitch.tv's banner
/// code, 2026-11-21; each one checked to answer).
const TWITCH_ASSETS: &str = "https://static-cdn.jtvnw.net/twilight-static-assets";

/// Twitch's category id for Grand Theft Auto VI.
pub const GTA_VI_GAME_ID: &str = "1867279146";

/// The Heist Train theme ends with the GTA VI event (Nov 18 to 30, 2026):
/// 2026-12-01 00:00 UTC.
pub const HEIST_THEME_UNTIL_MS: i64 = 1_796_083_200_000;

/// A treasure train's sub-gift discount: `discount_percent` off gift subs once
/// the train reaches `unlock_level`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Treasure {
    pub discount_percent: i32,
    pub unlock_level: i32,
}

/// How a themed train is painted: `background` the card's gradient (left to
/// right), `accent` the timer ring, level words and marks, `fill` the track,
/// `badge` the stream card's chip (one color that says the kind at a glance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub background: Vec<String>,
    pub accent: String,
    pub fill: String,
    pub badge: String,
}

/// The kind of train and everything the bars and badges say about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainLook {
    /// `regular`, `golden_kappa`, `treasure`, `mythic`, `community` or `pinata`.
    pub kind: String,
    /// `heist` for a regular train in the GTA VI theme.
    pub theme: Option<String>,
    pub shared: bool,
    pub all_time_high: bool,
    pub treasure: Option<Treasure>,
    /// "Hype Train", "Heist Train", "Shared Heist Train", ...
    pub name: String,
    /// The badge's word: "Heist", "Golden", ... empty for a plain train (the
    /// badge says "LVL").
    pub short_name: String,
    /// What the bar says as the train starts, when its kind has words of its own.
    pub kickoff: Option<String>,
    /// The treasure discount in words, Twitch's numbers: "-35% gifts from L5"
    /// (what is read out, and the desktop's tooltip).
    pub treasure_words: Option<String>,
    /// The bar's own name for it, short so the amount still needed always
    /// fits beside it: "Heist", "Shared Heist", "Golden Kappa", "Shared";
    /// empty for a plain train (the bar just says its level).
    pub bar_name: String,
    /// The bar's treasure chip, short: "-35% at L5".
    pub treasure_short: Option<String>,
    /// None: the app's own look (a regular train).
    pub palette: Option<Palette>,
    /// Twitch's own mark for the kind (the heist glitch logo, the gold coin,
    /// the Golden Kappa, ...), drawn before the bar's name. Loaded from
    /// Twitch's CDN, never bundled. None for a plain train.
    pub icon: Option<String>,
    /// Twitch's trophy for an all-time-high train.
    pub trophy_icon: Option<String>,
    /// The channel's own hype train color, when it set one (kept for later; a
    /// regular train still draws the app's rainbow).
    pub channel_color: Option<String>,
}

impl Default for TrainLook {
    fn default() -> Self {
        train_look(&Facts::default())
    }
}

/// What Twitch says about one train.
#[derive(Debug, Clone, Default)]
pub struct Facts<'a> {
    /// `hypeTrainType`, as Twitch spells it.
    pub train_type: Option<&'a str>,
    pub golden_kappa: bool,
    pub treasure: bool,
    pub shared: bool,
    pub all_time_high: bool,
    /// `treasureTrainDetails`: discount percent and the level it unlocks at.
    pub treasure_details: Option<(i32, i32)>,
    /// The channel's category and every shared participant's.
    pub games: Vec<&'a str>,
    pub channel_color: Option<&'a str>,
    pub now_ms: i64,
}

fn kind_of(f: &Facts) -> &'static str {
    match f.train_type.map(str::to_ascii_uppercase).as_deref() {
        Some("REGULAR") => "regular",
        Some("GOLDEN_KAPPA") => "golden_kappa",
        Some("TREASURE") => "treasure",
        Some("MYTHIC") => "mythic",
        Some("COMMUNITY") => "community",
        Some("PINATA") => "pinata",
        // No type (an older answer): the flags still say golden or treasure.
        None if f.golden_kappa => "golden_kappa",
        None if f.treasure => "treasure",
        // A kind Twitch adds later draws as a regular train until it has a look.
        _ => "regular",
    }
}

fn palette(bg: &[&str], accent: &str, fill: &str, badge: &str) -> Option<Palette> {
    Some(Palette {
        background: bg.iter().map(|c| c.to_string()).collect(),
        accent: accent.to_string(),
        fill: fill.to_string(),
        badge: badge.to_string(),
    })
}

/// The look for one train.
pub fn train_look(f: &Facts) -> TrainLook {
    let kind = kind_of(f);
    let heist = kind == "regular" && f.now_ms < HEIST_THEME_UNTIL_MS && f.games.iter().any(|g| *g == GTA_VI_GAME_ID);
    // Twitch's colors per kind (banner v3). Treasure and pinata are our own
    // approximations of Twitch's named tokens.
    let (base, short, colors) = match (kind, heist) {
        (_, true) => ("Heist Train", "Heist", palette(&["#445ed6", "#fc52ae", "#ed815f"], "#ffd24a", "#ffffff", "#fc52ae")),
        ("golden_kappa", _) => ("Golden Kappa Train", "Golden", palette(&["#9a6a00", "#5c3d00"], "#ffd700", "#ff9a1f", "#ffc83d")),
        ("treasure", _) => ("Treasure Train", "Treasure", palette(&["#0a6e8c", "#5c16c5"], "#ffc83d", "#ff4fd8", "#3fd0ff")),
        ("mythic", _) => ("Mythic Train", "Mythic", palette(&["#4b3300", "#7f6b30", "#4b3300"], "#ffd760", "#ffd760", "#ffd760")),
        ("community", _) => ("Community Train", "Community", palette(&["#1f69ff", "#0e3fa8"], "#9ec3ff", "#9ec3ff", "#6aa5ff")),
        ("pinata", _) => ("Piñata Train", "Piñata", palette(&["#d9407a", "#f08a24", "#2fb3c4"], "#ffe066", "#ffffff", "#f08a24")),
        _ => ("Hype Train", "", None),
    };
    let name = if f.shared { format!("Shared {base}") } else { base.to_string() };
    let treasure = (kind == "treasure")
        .then_some(f.treasure_details)
        .flatten()
        .filter(|(pct, _)| *pct > 0)
        .map(|(discount_percent, unlock_level)| Treasure { discount_percent, unlock_level: unlock_level.max(1) });
    let treasure_words = treasure
        .as_ref()
        .map(|t| format!("-{}% gifts from L{}", t.discount_percent, t.unlock_level));
    let treasure_short = treasure.as_ref().map(|t| format!("-{}% at L{}", t.discount_percent, t.unlock_level));
    // The bar's short name: the kind's word ("Golden Kappa" keeps its whole
    // name, "Golden" alone reads as a color), "Shared" before it.
    let kind_word = match short {
        "Golden" => "Golden Kappa",
        other => other,
    };
    let icon = match (kind, heist) {
        (_, true) => Some(format!("{TWITCH_ASSETS}/heist-train-glitch.png")),
        ("golden_kappa", _) => Some("https://static-cdn.jtvnw.net/emoticons/v2/80393/default/dark/2.0".to_string()),
        ("treasure", _) => Some(format!("{TWITCH_ASSETS}/Hype_Train_Banner_Icon_Gold_Coin.png")),
        ("mythic", _) => Some(format!("{TWITCH_ASSETS}/mythic_train_icon.png")),
        ("community", _) => Some(format!("{TWITCH_ASSETS}/1a923a797768495fad684853aafd3bf281a60b7c.png")),
        _ => None,
    };
    let bar_name = match (f.shared, kind_word.is_empty()) {
        (true, true) => "Shared".to_string(),
        (true, false) => format!("Shared {kind_word}"),
        (false, _) => kind_word.to_string(),
    };
    TrainLook {
        kind: kind.to_string(),
        theme: heist.then(|| "heist".to_string()),
        shared: f.shared,
        all_time_high: f.all_time_high,
        treasure,
        name,
        short_name: short.to_string(),
        kickoff: heist.then(|| "The Heist Is On!".to_string()),
        treasure_words,
        bar_name,
        treasure_short,
        palette: colors,
        icon,
        trophy_icon: f.all_time_high.then(|| format!("{TWITCH_ASSETS}/hype-train-trophy.png")),
        channel_color: f
            .channel_color
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| if c.starts_with('#') { c.to_string() } else { format!("#{c}") }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DURING: i64 = 1_795_000_000_000; // 2026-11-19, inside the GTA VI event

    fn facts(train_type: Option<&'static str>) -> Facts<'static> {
        Facts { train_type, now_ms: DURING, ..Facts::default() }
    }

    #[test]
    fn every_kind_twitch_names_maps_to_its_own() {
        for (twitch, kind, name) in [
            ("REGULAR", "regular", "Hype Train"),
            ("GOLDEN_KAPPA", "golden_kappa", "Golden Kappa Train"),
            ("TREASURE", "treasure", "Treasure Train"),
            ("MYTHIC", "mythic", "Mythic Train"),
            ("COMMUNITY", "community", "Community Train"),
            ("PINATA", "pinata", "Piñata Train"),
        ] {
            let look = train_look(&facts(Some(twitch)));
            assert_eq!(look.kind, kind, "{twitch}");
            assert_eq!(look.name, name, "{twitch}");
            assert_eq!(look.palette.is_none(), kind == "regular", "{twitch}");
        }
    }

    #[test]
    fn an_unknown_type_stays_regular_and_a_missing_one_reads_the_flags() {
        let look = train_look(&facts(Some("ROCKET")));
        assert_eq!((look.kind.as_str(), look.palette.is_none()), ("regular", true));
        assert_eq!(train_look(&Facts { golden_kappa: true, ..facts(None) }).kind, "golden_kappa");
        assert_eq!(train_look(&Facts { treasure: true, ..facts(None) }).kind, "treasure");
        assert_eq!(train_look(&facts(None)).kind, "regular");
        // The type wins over a stale flag.
        assert_eq!(train_look(&Facts { golden_kappa: true, ..facts(Some("MYTHIC")) }).kind, "mythic");
    }

    #[test]
    fn heist_is_a_regular_train_on_gta_vi_inside_the_event() {
        let on_gta = Facts { games: vec![GTA_VI_GAME_ID], ..facts(Some("REGULAR")) };
        let look = train_look(&on_gta);
        assert_eq!(look.theme.as_deref(), Some("heist"));
        assert_eq!(look.kind, "regular");
        assert_eq!(look.name, "Heist Train");
        assert_eq!(look.short_name, "Heist");
        assert_eq!(look.kickoff.as_deref(), Some("The Heist Is On!"));
        assert!(look.icon.as_deref().is_some_and(|u| u.ends_with("/heist-train-glitch.png")));
        assert!(train_look(&facts(Some("REGULAR"))).icon.is_none(), "a plain train wears no mark");
        assert_eq!(look.palette.as_ref().map(|p| p.background.len()), Some(3));
        // Another category, another kind, or after the event: no heist.
        assert!(train_look(&Facts { games: vec!["509658"], ..facts(Some("REGULAR")) }).theme.is_none());
        assert!(train_look(&Facts { games: vec![GTA_VI_GAME_ID], ..facts(Some("GOLDEN_KAPPA")) }).theme.is_none());
        assert!(train_look(&Facts { now_ms: HEIST_THEME_UNTIL_MS, ..on_gta.clone() }).theme.is_none());
        assert!(train_look(&Facts { now_ms: HEIST_THEME_UNTIL_MS - 1, ..on_gta }).theme.is_some());
    }

    #[test]
    fn a_shared_train_is_heist_when_any_participant_plays_gta_vi() {
        let look = train_look(&Facts { shared: true, games: vec!["509658", GTA_VI_GAME_ID], ..facts(Some("REGULAR")) });
        assert_eq!(look.theme.as_deref(), Some("heist"));
        assert_eq!(look.name, "Shared Heist Train");
        assert_eq!(look.bar_name, "Shared Heist");
        let shared_plain = train_look(&Facts { shared: true, ..facts(Some("REGULAR")) });
        assert_eq!((shared_plain.name.as_str(), shared_plain.bar_name.as_str()), ("Shared Hype Train", "Shared"));
        // The bar's short names, so the amount still needed always fits beside them.
        assert_eq!(train_look(&facts(Some("GOLDEN_KAPPA"))).bar_name, "Golden Kappa");
        assert_eq!(train_look(&facts(Some("REGULAR"))).bar_name, "");
    }

    #[test]
    fn a_treasure_train_carries_twitchs_numbers() {
        let look = train_look(&Facts { treasure_details: Some((35, 5)), ..facts(Some("TREASURE")) });
        assert_eq!(look.treasure, Some(Treasure { discount_percent: 35, unlock_level: 5 }));
        assert_eq!(look.treasure_words.as_deref(), Some("-35% gifts from L5"));
        assert_eq!(look.treasure_short.as_deref(), Some("-35% at L5"));
        assert_eq!(look.bar_name, "Treasure");
        // Details on another kind, or no discount, say nothing.
        assert!(train_look(&Facts { treasure_details: Some((35, 5)), ..facts(Some("REGULAR")) }).treasure.is_none());
        assert!(train_look(&Facts { treasure_details: Some((0, 5)), ..facts(Some("TREASURE")) }).treasure_words.is_none());
    }

    #[test]
    fn all_time_high_and_the_channel_color_ride_along() {
        let look = train_look(&Facts { all_time_high: true, channel_color: Some("9147FF"), ..facts(Some("REGULAR")) });
        assert!(look.all_time_high);
        assert!(look.trophy_icon.as_deref().is_some_and(|u| u.ends_with("/hype-train-trophy.png")));
        assert_eq!(look.channel_color.as_deref(), Some("#9147FF"));
        assert!(look.palette.is_none(), "a regular train keeps the app's own look");
        assert_eq!(TrainLook::default().name, "Hype Train");
    }
}
