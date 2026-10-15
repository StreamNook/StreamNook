//! A chatter's sub in the channel their profile was opened from, and the words
//! the desktop profile card and the phone's profile sheet show for it.
//!
//! Two signals, strongest first: the sub or founder badge the chatter wears in
//! this room's chat (live and first-party), then IVR's subage answer (tenure,
//! tier, gifter, and whether a lapsed sub ever existed). A worn badge only ever
//! upgrades the answer to current; a missing one never downgrades IVR, because
//! the card can be opened from a mention or an old message that carries none.

use crate::services::ivr::{SubFacts, SubKind};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubState {
    Current,
    Past,
    Never,
    /// IVR could not answer or the standing is hidden, and no badge says more.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubStanding {
    pub state: SubState,
    pub founder: bool,
    pub kind: Option<SubKind>,
    pub tier: Option<u8>,
    pub months: Option<i64>,
    pub streak_months: Option<i64>,
    pub gifter_login: Option<String>,
    pub gifter_display_name: Option<String>,
    /// "Subscribed to FeelsSunnyMan", "Past subscriber", "Not subscribed to ...".
    pub title: Option<String>,
    /// The facts after the title: "Tier 1 · 4 months · Gifted by JustThunderr".
    pub detail: Option<String>,
    /// "Gift Emerald_org a sub", or "Gift a sub" beside a current sub (it
    /// names the channel too when the standing is unknown). None
    /// when there is no one to gift (no channel, or the channel's own owner).
    pub gift_label: Option<String>,
    pub gift_url: Option<String>,
    /// "month" or "months", for the tenure number (`months`) shown large.
    pub months_unit: Option<String>,
    /// "Tier 1", beside the tenure; None for Prime, which is a fact instead.
    pub tier_label: Option<String>,
    /// The facts under the tenure, each tagged so the surface can mark it,
    /// with Twitch's own badge art where one exists.
    pub facts: Vec<SubFact>,
    /// The channel's sub badge for this tenure and tier, at its largest size.
    pub badge_art: Option<String>,
    /// The channel's next sub badge and how far along the way to it they are.
    pub next_badge: Option<NextBadge>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubFactKind {
    Founder,
    Prime,
    Streak,
    Gift,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubFact {
    pub kind: SubFactKind,
    pub text: String,
    /// The badge that says it: the channel's founder badge, Prime Gaming, Sub
    /// Gifter. None where Twitch has no badge for it (a streak).
    pub art: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NextBadge {
    /// The tenure the next badge arrives at.
    pub months: i64,
    pub months_left: i64,
    /// 0 to 1, from the badge they hold to the next one.
    pub progress: f64,
    pub art: Option<String>,
    /// "2 months to the 6-month badge".
    pub text: String,
}

/// One sub badge a channel offers: its version (`3`, `2006`, `3012`) and art.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubBadgeRung {
    pub version: u32,
    pub art: String,
}

/// The badge art a sub tile can show for one channel: its sub badge ladder,
/// and the founder, Prime Gaming and Sub Gifter badges (the channel's own
/// founder art when it has one, else Twitch's global set).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomBadges {
    pub ladder: Vec<SubBadgeRung>,
    pub founder: Option<String>,
    pub prime: Option<String>,
    pub sub_gifter: Option<String>,
}

fn badge_art(list: Option<&serde_json::Value>, set: &str, version: Option<&str>) -> Option<String> {
    list?
        .as_array()?
        .iter()
        .filter(|b| b["setID"].as_str() == Some(set))
        .find(|b| version.is_none_or(|v| b["version"].as_str() == Some(v)))
        .and_then(|b| b["imageURL"].as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Reads a GQL answer carrying `user { broadcastBadges }` and the global
/// `badges` list.
pub fn room_badges_from(gql: &serde_json::Value) -> RoomBadges {
    let channel = gql.pointer("/data/user/broadcastBadges");
    let global = gql.pointer("/data/badges");
    RoomBadges {
        ladder: sub_badge_ladder_from(channel),
        founder: badge_art(channel, "founder", None).or_else(|| badge_art(global, "founder", Some("0"))),
        prime: badge_art(global, "premium", Some("1")),
        sub_gifter: badge_art(global, "sub-gifter", Some("1")),
    }
}

/// The `subscriber` set out of a channel's `broadcastBadges` list.
pub fn sub_badge_ladder_from(broadcast_badges: Option<&serde_json::Value>) -> Vec<SubBadgeRung> {
    let mut rungs: Vec<SubBadgeRung> = broadcast_badges
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|b| b["setID"].as_str() == Some("subscriber"))
        .filter_map(|b| {
            Some(SubBadgeRung {
                version: b["version"].as_str()?.parse().ok()?,
                art: b["imageURL"].as_str().filter(|s| !s.is_empty())?.to_string(),
            })
        })
        .collect();
    rungs.sort_by_key(|r| r.version);
    rungs.dedup_by_key(|r| r.version);
    rungs
}

/// The badge held at `months` of tenure in `tier`, and the next one. Twitch
/// numbers tier 2 and 3 badges from 2000 and 3000, and a channel without
/// tier-specific art shows its tier 1 badges to every tier.
pub fn tenure_badges(
    ladder: &[SubBadgeRung],
    tier: Option<u8>,
    months: i64,
) -> (Option<String>, Option<NextBadge>) {
    let band = |base: u32| -> Vec<(i64, &str)> {
        ladder
            .iter()
            .filter(|r| r.version >= base && r.version < base + 1000)
            .map(|r| (i64::from(r.version - base), r.art.as_str()))
            .collect()
    };
    let tier_base = match tier {
        Some(2) => 2000,
        Some(3) => 3000,
        _ => 0,
    };
    let mut rungs = band(tier_base);
    if rungs.is_empty() && tier_base != 0 {
        rungs = band(0);
    }
    if rungs.is_empty() {
        return (None, None);
    }
    let held = rungs.iter().rev().find(|(m, _)| *m <= months).copied();
    let next = rungs.iter().find(|(m, _)| *m > months).copied();
    let next_badge = next.map(|(at, art)| {
        let from = held.map_or(0, |(m, _)| m);
        let left = at - months;
        let span = (at - from).max(1) as f64;
        let label = if at % 12 == 0 {
            format!("{}-year", at / 12)
        } else {
            format!("{at}-month")
        };
        NextBadge {
            months: at,
            months_left: left,
            progress: ((months - from) as f64 / span).clamp(0.0, 1.0),
            art: Some(art.to_string()),
            text: format!("{} to the {label} badge", months_words(left)),
        }
    });
    (held.map(|(_, art)| art.to_string()), next_badge)
}

const LADDER_TTL: Duration = Duration::from_secs(30 * 60);
type Ladders = Mutex<Option<LruCache<String, (Instant, RoomBadges)>>>;
static LADDERS: Ladders = Mutex::new(None);

/// A channel's sub badges and the global badges the tile uses, from Twitch's
/// public GQL (no sign-in) in one request, cached for half an hour. Empty when
/// Twitch could not be asked; that is not cached.
pub async fn room_badges(channel_login: &str) -> RoomBadges {
    let key = channel_login.to_lowercase();
    if key.is_empty() {
        return RoomBadges::default();
    }
    if let Ok(mut guard) = LADDERS.lock() {
        if let Some((at, rungs)) = guard.as_mut().and_then(|lru| lru.get(&key)) {
            if at.elapsed() < LADDER_TTL {
                return rungs.clone();
            }
        }
    }
    let body = serde_json::json!({
        "operationName": "StreamNookSubBadges",
        "query": "query StreamNookSubBadges($login: String!) { user(login: $login) { broadcastBadges { setID version imageURL(size: QUADRUPLE) } } badges { setID version imageURL(size: QUADRUPLE) } }",
        "variables": { "login": key },
    });
    let answer = crate::services::http::client()
        .post("https://gql.twitch.tv/gql")
        .header("Client-ID", env!("TWITCH_WEB_CLIENT_ID"))
        .json(&body)
        .send()
        .await;
    let json = match answer {
        Ok(resp) if resp.status().is_success() => resp.json::<serde_json::Value>().await.ok(),
        _ => None,
    };
    let Some(json) = json else {
        return RoomBadges::default();
    };
    let rungs = room_badges_from(&json);
    if let Ok(mut guard) = LADDERS.lock() {
        guard
            .get_or_insert_with(|| LruCache::new(NonZeroUsize::new(64).expect("non-zero capacity")))
            .put(key, (Instant::now(), rungs.clone()));
    }
    rungs
}

/// The sub badge a chatter wears in this room, from the badge keys on a chat
/// message (`subscriber/3012`, `founder/0`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WornSub {
    pub tier: Option<u8>,
    pub founder: bool,
}

pub fn worn_sub<S: AsRef<str>>(badge_keys: &[S]) -> Option<WornSub> {
    let mut worn: Option<WornSub> = None;
    for key in badge_keys {
        let key = key.as_ref();
        let (set, version) = key.split_once('/').unwrap_or((key, ""));
        match set {
            "subscriber" => {
                // Twitch numbers tier 2 and tier 3 badge versions from 2000 and
                // 3000 (3012 is a 12-month tier 3 badge).
                let tier = match version.parse::<u32>().unwrap_or(0) {
                    3000.. => 3,
                    2000.. => 2,
                    _ => 1,
                };
                worn.get_or_insert_with(WornSub::default).tier = Some(tier);
            }
            "founder" => worn.get_or_insert_with(WornSub::default).founder = true,
            _ => {}
        }
    }
    worn
}

fn months_words(n: i64) -> String {
    if n == 1 {
        "1 month".to_string()
    } else {
        format!("{n} months")
    }
}

/// Folds IVR's answer and the worn badge into one standing with its words.
/// `login` is the chatter's login, `channel` the room's login.
pub fn sub_standing(
    facts: Option<&SubFacts>,
    worn: Option<WornSub>,
    badges: &RoomBadges,
    login: &str,
    channel: &str,
) -> SubStanding {
    let none = SubFacts::default();
    let f = facts.unwrap_or(&none);
    let state = if worn.is_some() || f.active {
        SubState::Current
    } else if facts.is_none() || f.hidden {
        SubState::Unknown
    } else if f.total_months.is_some() {
        SubState::Past
    } else {
        SubState::Never
    };
    let current = state == SubState::Current;

    // A tier from IVR is read from the sub itself; the badge's is inferred
    // from its version number, so it only fills a gap.
    let kind = if current { f.kind } else { None };
    let tier = if current {
        f.tier.filter(|_| f.active).or(worn.and_then(|w| w.tier))
    } else {
        None
    };
    let founder = worn.is_some_and(|w| w.founder);
    let months = f.total_months;
    let streak_months = if current { f.streak_months } else { None };
    let gifted = kind == Some(SubKind::Gift);
    let gifter_login = if gifted { f.gifter_login.clone() } else { None };
    let gifter_display_name = if gifted {
        f.gifter_display_name.clone().or_else(|| gifter_login.clone())
    } else {
        None
    };

    let channel_name = f
        .channel_display_name
        .clone()
        .unwrap_or_else(|| channel.to_string());
    let recipient = f
        .user_display_name
        .clone()
        .unwrap_or_else(|| login.to_string());

    let prime = kind == Some(SubKind::Prime);
    let tier_label = match tier {
        Some(t) if current && !prime => Some(format!("Tier {t}")),
        _ => None,
    };
    let streak_words = match (streak_months, months) {
        (Some(s), Some(m)) if current && s >= 2 && s < m => Some(format!("{s}-month streak")),
        _ => None,
    };
    let gift_words = gifted.then(|| match &gifter_display_name {
        Some(name) => format!("Gifted by {name}"),
        None => "Gifted".to_string(),
    });

    let mut sub_facts: Vec<SubFact> = Vec::new();
    if current {
        let mut fact = |kind, text: String, art: Option<String>| sub_facts.push(SubFact { kind, text, art });
        if founder {
            fact(SubFactKind::Founder, "Founder".into(), badges.founder.clone());
        }
        if prime {
            fact(SubFactKind::Prime, "Prime".into(), badges.prime.clone());
        }
        if let Some(words) = &streak_words {
            fact(SubFactKind::Streak, words.clone(), None);
        }
        if let Some(words) = &gift_words {
            fact(SubFactKind::Gift, words.clone(), badges.sub_gifter.clone());
        }
    }

    let (title, detail) = match state {
        SubState::Current => {
            // Everything as one line, for a surface that shows a single line.
            let parts: Vec<String> = [
                founder.then(|| "Founder".to_string()),
                if prime { Some("Prime".to_string()) } else { tier_label.clone() },
                months.map(months_words),
                streak_words,
                gift_words,
            ]
            .into_iter()
            .flatten()
            .collect();
            (
                Some(format!("Subscribed to {channel_name}")),
                (!parts.is_empty()).then(|| parts.join(" · ")),
            )
        }
        SubState::Past => (
            Some("Past subscriber".into()),
            months.map(|m| format!("{} total", months_words(m))),
        ),
        SubState::Never => (Some(format!("Not subscribed to {channel_name}")), None),
        SubState::Unknown => (None, None),
    };

    let can_gift = !login.is_empty() && !channel.is_empty() && !login.eq_ignore_ascii_case(channel);
    let (gift_label, gift_url) = if can_gift {
        (
            // With no title under it to name the channel, the label names it.
            Some(match state {
                SubState::Current => "Gift a sub".to_string(),
                SubState::Unknown => format!("Gift {recipient} a sub to {channel_name}"),
                _ => format!("Gift {recipient} a sub"),
            }),
            Some(format!(
                "https://www.twitch.tv/subs/{}?giftrecipient={}",
                channel.to_lowercase(),
                login.to_lowercase()
            )),
        )
    } else {
        (None, None)
    };

    let (badge_art, next_badge) = match (current, months) {
        (true, Some(m)) => tenure_badges(&badges.ladder, tier, m),
        _ => (None, None),
    };

    SubStanding {
        state,
        founder,
        kind,
        tier,
        months,
        streak_months,
        gifter_login,
        gifter_display_name,
        title,
        detail,
        gift_label,
        gift_url,
        months_unit: if current {
            months.map(|m| if m == 1 { "month" } else { "months" }.to_string())
        } else {
            None
        },
        tier_label,
        facts: sub_facts,
        badge_art,
        next_badge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ivr::{fixtures, sub_facts_from};

    fn facts(raw: &str) -> SubFacts {
        sub_facts_from(&serde_json::from_str(raw).expect("fixture parses"))
    }

    const NO_BADGES: [&str; 0] = [];

    #[test]
    fn a_gifted_current_sub_reads_as_subscribed_never_past() {
        let s = sub_standing(
            Some(&facts(fixtures::CURRENT_GIFT)),
            worn_sub(&NO_BADGES),
            &RoomBadges::default(),
            "emerald_org",
            "feelssunnyman",
        );
        assert_eq!(s.state, SubState::Current);
        assert_eq!(s.title.as_deref(), Some("Subscribed to FeelsSunnyMan"));
        assert_eq!(s.detail.as_deref(), Some("Tier 1 · 4 months · Gifted by JustThunderr"));
        assert_eq!(s.gift_label.as_deref(), Some("Gift a sub"));
        assert_eq!(
            s.gift_url.as_deref(),
            Some("https://www.twitch.tv/subs/feelssunnyman?giftrecipient=emerald_org")
        );
    }

    #[test]
    fn a_paid_sub_shows_its_streak_when_it_differs_from_the_total() {
        let s = sub_standing(Some(&facts(fixtures::CURRENT_PAID)), None, &RoomBadges::default(), "justthunderr", "feelssunnyman");
        assert_eq!(s.state, SubState::Current);
        assert_eq!(s.kind, Some(SubKind::Paid));
        assert_eq!(s.detail.as_deref(), Some("Tier 1 · 7 months · 3-month streak"));
    }

    #[test]
    fn a_prime_sub_says_prime_instead_of_a_tier() {
        let s = sub_standing(Some(&facts(fixtures::CURRENT_PRIME)), None, &RoomBadges::default(), "primefan", "feelssunnyman");
        assert_eq!(s.state, SubState::Current);
        assert_eq!(s.detail.as_deref(), Some("Prime · 12 months"));
    }

    #[test]
    fn a_lapsed_sub_reads_as_past_with_its_total() {
        let s = sub_standing(Some(&facts(fixtures::PAST)), None, &RoomBadges::default(), "xqc", "feelssunnyman");
        assert_eq!(s.state, SubState::Past);
        assert_eq!(s.title.as_deref(), Some("Past subscriber"));
        assert_eq!(s.detail.as_deref(), Some("9 months total"));
        assert_eq!(s.gift_label.as_deref(), Some("Gift xQc a sub"));
    }

    #[test]
    fn never_subscribed_offers_the_gift() {
        let s = sub_standing(Some(&facts(fixtures::NEVER)), None, &RoomBadges::default(), "br_winters", "feelssunnyman");
        assert_eq!(s.state, SubState::Never);
        assert_eq!(s.title.as_deref(), Some("Not subscribed to FeelsSunnyMan"));
        assert_eq!(s.detail, None);
        assert_eq!(s.gift_label.as_deref(), Some("Gift br_winters a sub"));
    }

    #[test]
    fn a_worn_sub_badge_beats_a_lapsed_ivr_answer() {
        let s = sub_standing(
            Some(&facts(fixtures::PAST)),
            worn_sub(&["moderator/1", "subscriber/2009"]),
            &RoomBadges::default(),
            "xqc",
            "feelssunnyman",
        );
        assert_eq!(s.state, SubState::Current);
        assert_eq!(s.tier, Some(2));
        assert_eq!(s.detail.as_deref(), Some("Tier 2 · 9 months"));
    }

    #[test]
    fn a_founder_badge_alone_is_a_current_sub() {
        let s = sub_standing(None, worn_sub(&["founder/0"]), &RoomBadges::default(), "fan", "chan");
        assert_eq!(s.state, SubState::Current);
        assert!(s.founder);
        assert_eq!(s.title.as_deref(), Some("Subscribed to chan"));
        assert_eq!(s.detail.as_deref(), Some("Founder"));
    }

    #[test]
    fn no_ivr_and_no_badge_is_unknown_but_still_giftable() {
        let s = sub_standing(None, worn_sub(&["vip/1"]), &RoomBadges::default(), "fan", "chan");
        assert_eq!(s.state, SubState::Unknown);
        assert_eq!((s.title, s.detail), (None, None));
        assert_eq!(s.gift_label.as_deref(), Some("Gift fan a sub to chan"));
    }

    #[test]
    fn a_hidden_standing_is_unknown_not_never() {
        let s = sub_standing(
            Some(&facts(r#"{"statusHidden":true,"streak":null,"cumulative":null,"meta":null}"#)),
            None,
            &RoomBadges::default(),
            "fan",
            "chan",
        );
        assert_eq!(s.state, SubState::Unknown);
    }

    #[test]
    fn the_channel_owner_is_never_offered_a_gift_to_themselves() {
        let s = sub_standing(Some(&facts(fixtures::NEVER)), None, &RoomBadges::default(), "feelssunnyman", "FeelsSunnyMan");
        assert_eq!((s.gift_label, s.gift_url), (None, None));
    }

    #[test]
    fn badge_versions_map_to_tiers() {
        assert_eq!(worn_sub(&["subscriber/0"]).and_then(|w| w.tier), Some(1));
        assert_eq!(worn_sub(&["subscriber/24"]).and_then(|w| w.tier), Some(1));
        assert_eq!(worn_sub(&["subscriber/2003"]).and_then(|w| w.tier), Some(2));
        assert_eq!(worn_sub(&["subscriber/3012"]).and_then(|w| w.tier), Some(3));
        assert_eq!(worn_sub(&["premium/1", "bits/100"]), None);
    }

    /// FeelsSunnyMan's real `subscriber` set from Twitch's public GQL (a tier
    /// 1, 2 and 3 run, 0 to 24 months), plus a bits badge that must be ignored.
    fn fsm_ladder() -> Vec<SubBadgeRung> {
        let art = |v: &str| format!("https://static-cdn.jtvnw.net/badges/v1/{v}/3");
        let rows: Vec<serde_json::Value> = ["0", "2", "3", "6", "9", "12", "18", "24"]
            .iter()
            .flat_map(|m| {
                let m: u32 = m.parse().unwrap();
                [m, 2000 + m, 3000 + m]
            })
            .map(|v| serde_json::json!({ "setID": "subscriber", "version": v.to_string(), "imageURL": art(&v.to_string()) }))
            .chain([serde_json::json!({ "setID": "bits", "version": "100", "imageURL": art("bits100") })])
            .collect();
        room_badges_from(&fsm_gql(rows)).ladder
    }

    /// A GQL answer: the channel's list plus a slice of Twitch's global set.
    fn fsm_gql(channel_rows: Vec<serde_json::Value>) -> serde_json::Value {
        let art = |v: &str| format!("https://static-cdn.jtvnw.net/badges/v1/{v}/3");
        serde_json::json!({ "data": {
            "user": { "broadcastBadges": channel_rows },
            "badges": [
                { "setID": "founder", "version": "0", "imageURL": art("global-founder") },
                { "setID": "premium", "version": "1", "imageURL": art("prime-gaming") },
                { "setID": "sub-gifter", "version": "5", "imageURL": art("gifter-5") },
                { "setID": "sub-gifter", "version": "1", "imageURL": art("gifter-1") },
                { "setID": "subscriber", "version": "0", "imageURL": art("global-sub") }
            ]
        } })
    }

    fn fsm_room() -> RoomBadges {
        RoomBadges { ladder: fsm_ladder(), ..room_badges_from(&fsm_gql(vec![])) }
    }

    #[test]
    fn room_badges_take_global_art_and_prefer_a_channel_founder_badge() {
        let room = room_badges_from(&fsm_gql(vec![]));
        assert!(room.ladder.is_empty());
        assert_eq!(room.founder.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/global-founder/3"));
        assert_eq!(room.prime.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/prime-gaming/3"));
        assert_eq!(room.sub_gifter.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/gifter-1/3"));
        let own = room_badges_from(&fsm_gql(vec![serde_json::json!({
            "setID": "founder", "version": "0", "imageURL": "https://static-cdn.jtvnw.net/badges/v1/fsm-founder/3"
        })]));
        assert_eq!(own.founder.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/fsm-founder/3"));
    }

    #[test]
    fn facts_carry_real_badges_and_the_tier_sits_beside_the_months() {
        let founder = sub_standing(
            Some(&facts(fixtures::CURRENT_PAID)),
            worn_sub(&["founder/0"]),
            &fsm_room(),
            "justthunderr",
            "feelssunnyman",
        );
        assert_eq!(founder.tier_label.as_deref(), Some("Tier 1"));
        let kinds: Vec<SubFactKind> = founder.facts.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, vec![SubFactKind::Founder, SubFactKind::Streak]);
        assert_eq!(founder.facts[0].art.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/global-founder/3"));
        assert_eq!(founder.facts[1].art, None);
        assert_eq!(founder.detail.as_deref(), Some("Founder · Tier 1 · 7 months · 3-month streak"));

        let prime = sub_standing(Some(&facts(fixtures::CURRENT_PRIME)), None, &fsm_room(), "primefan", "feelssunnyman");
        assert_eq!(prime.tier_label, None);
        assert_eq!(prime.facts[0].kind, SubFactKind::Prime);
        assert_eq!(prime.facts[0].art.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/prime-gaming/3"));

        let gift = sub_standing(Some(&facts(fixtures::CURRENT_GIFT)), None, &fsm_room(), "emerald_org", "feelssunnyman");
        assert_eq!(gift.facts[0].kind, SubFactKind::Gift);
        assert_eq!(gift.facts[0].art.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/gifter-1/3"));
    }

    #[test]
    fn the_ladder_reads_only_sub_badges_in_order() {
        let ladder = fsm_ladder();
        assert_eq!(ladder.len(), 24);
        assert_eq!(ladder.first().map(|r| r.version), Some(0));
        assert_eq!(ladder.last().map(|r| r.version), Some(3024));
    }

    #[test]
    fn four_months_holds_the_three_month_badge_and_heads_for_six() {
        let (art, next) = tenure_badges(&fsm_ladder(), Some(1), 4);
        assert_eq!(art.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/3/3"));
        let next = next.expect("a next badge");
        assert_eq!((next.months, next.months_left), (6, 2));
        assert!((next.progress - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(next.text, "2 months to the 6-month badge");
    }

    #[test]
    fn tier_three_reads_its_own_band_and_names_years() {
        let (art, next) = tenure_badges(&fsm_ladder(), Some(3), 14);
        assert_eq!(art.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/3012/3"));
        let next = next.expect("a next badge");
        assert_eq!(next.months, 18);
        assert_eq!(next.text, "4 months to the 18-month badge");
        let (_, to_two_years) = tenure_badges(&fsm_ladder(), Some(3), 23);
        assert_eq!(to_two_years.map(|n| n.text).as_deref(), Some("1 month to the 2-year badge"));
    }

    #[test]
    fn a_tier_without_its_own_art_uses_tier_one_and_the_top_has_no_next() {
        let tier_one_only: Vec<SubBadgeRung> =
            fsm_ladder().into_iter().filter(|r| r.version < 1000).collect();
        let (art, _) = tenure_badges(&tier_one_only, Some(2), 7);
        assert_eq!(art.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/6/3"));
        let (art, next) = tenure_badges(&fsm_ladder(), Some(1), 40);
        assert_eq!(art.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/24/3"));
        assert_eq!(next, None);
        assert_eq!(tenure_badges(&[], Some(1), 4), (None, None));
    }

    #[test]
    fn a_current_sub_carries_its_facts_badge_and_next_milestone() {
        let s = sub_standing(
            Some(&facts(fixtures::CURRENT_GIFT)),
            None,
            &fsm_room(),
            "emerald_org",
            "feelssunnyman",
        );
        assert_eq!(s.months, Some(4));
        assert_eq!(s.months_unit.as_deref(), Some("months"));
        assert_eq!(s.tier_label.as_deref(), Some("Tier 1"));
        let kinds: Vec<SubFactKind> = s.facts.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, vec![SubFactKind::Gift]);
        assert_eq!(s.facts[0].text, "Gifted by JustThunderr");
        assert_eq!(s.badge_art.as_deref(), Some("https://static-cdn.jtvnw.net/badges/v1/3/3"));
        assert_eq!(s.next_badge.map(|n| n.text).as_deref(), Some("2 months to the 6-month badge"));
    }

    #[test]
    fn a_past_sub_carries_no_milestone() {
        let s = sub_standing(Some(&facts(fixtures::PAST)), None, &fsm_room(), "xqc", "feelssunnyman");
        assert_eq!((s.badge_art, s.next_badge, s.months_unit, s.tier_label), (None, None, None, None));
        assert!(s.facts.is_empty());
    }
}
