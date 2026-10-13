//! How a badge is earned, as a few typed steps a surface can draw as chips.
//!
//! The relay's campaign `requirement` is authoritative (watch minutes, subs,
//! repeat days, a random pool) and is used whenever it says anything. Without
//! it, the steps are read out of the relay's one-line instruction, then its
//! writeup, then the scraped More Info copy, and the path is marked `inferred`
//! so a surface can present it as a glimpse rather than a promise. That covers
//! the badges no campaign describes: TwitchCon passes, clip and co-streamer
//! badges, cheering, attending in person.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EarnStep {
    /// Pay for something that is not a subscription. `ticket` when what you buy
    /// is an event ticket or pass (TwitchCon badges are granted on purchase).
    Purchase { ticket: bool },
    /// Subscribe (or gift one). `count` when more than one is needed.
    Subscribe { count: Option<u32> },
    /// Watch. `minutes` per day when known, `days` when it must repeat.
    Watch { minutes: Option<u32>, days: Option<u32> },
    /// Cheer with Bits.
    Cheer,
    /// Something a creator does: clip, go live, co-stream, apply by form.
    Create,
    /// Be there in person. Never alongside a purchase: ticket badges say
    /// "awarded to attendees" but are granted when the ticket is bought.
    Attend,
    /// None of the above; the detail line says what.
    Other,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct EarnPath {
    /// In the order they happen: what you pay or sub first, then what you watch.
    pub steps: Vec<EarnStep>,
    /// The reward is drawn at random from this many badges.
    pub random_of: Option<u32>,
    /// True when read out of prose rather than the campaign's own numbers.
    pub inferred: bool,
    /// The sentence the path came from, for a tooltip.
    pub detail: Option<String>,
    /// The only channels it counts on, from the campaign's allow-list. `None`
    /// means any channel in the category, or that nobody said.
    pub channels: Option<Vec<String>>,
}

/// Everything a single badge's page says about earning it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct EarnGuide {
    pub earn: EarnPath,
    /// The campaign's Twitch category, when it has one.
    pub category: Option<String>,
    /// The whole requirement as one sentence to the viewer: what to do, how
    /// long, and where. Only when it comes from the campaign's own numbers;
    /// a guess is never stated this plainly.
    pub summary: Option<String>,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("badge earn regex")
}

static SUBSCRIBE: Lazy<Regex> =
    Lazy::new(|| re(r"(?i)\bsubscri|\bgift(?:ed|ing)?\b[^.]{0,30}\bsub"));
static SUB_COUNT: Lazy<Regex> =
    Lazy::new(|| re(r"(?i)\b(\d+|two|three|four|five|ten)\s+(?:gifted\s+|non-prime\s+)?(?:subscriptions|subs|gift subs)\b"));
static WATCH: Lazy<Regex> = Lazy::new(|| re(r"(?i)\bwatch"));
static MINUTES: Lazy<Regex> = Lazy::new(|| re(r"(?i)\b(\d+)\s*(?:minutes?|mins?)\b"));
static HOURS: Lazy<Regex> = Lazy::new(|| re(r"(?i)\b(\d+|an?|one|two|three|four|five)\s+hours?\b"));
static DAYS: Lazy<Regex> =
    Lazy::new(|| re(r"(?i)\b(\d+|two|three|four|five|seven)\s+(?:different\s+|separate\s+|unique\s+)?days\b"));
static TICKET: Lazy<Regex> =
    Lazy::new(|| re(r"(?i)\btickets?\b|\bpass(?:es)?\b"));
static PURCHASE: Lazy<Regex> = Lazy::new(|| {
    re(r"(?i)\btickets?\b|\b(?:\d+-day|one-day|three-day|weekend|vip|badge)\s+pass(?:es)?\b|\b(?:purchas\w*|buy|bought)\b[^.]{0,40}\bpass(?:es)?\b")
});
static CHEER: Lazy<Regex> = Lazy::new(|| re(r"(?i)\bcheer(?:ing|ed|s)?\b|\bbits\b"));
// Only what the earner does as a creator: "watch co-streaming channels" and
// "subscribe to a participating co-streamer" are viewer actions.
static CREATE: Lazy<Regex> = Lazy::new(|| {
    re(r"(?i)\bclips?\b|\bbecome\s+(?:an?\s+)?(?:official\s+)?co-?streamer|\bfill(?:ing)?\s+out\b|\b(?:application|sign-?up)\s+form\b|\bgo(?:ing)?\s+live\b")
});
static ATTEND: Lazy<Regex> =
    Lazy::new(|| re(r"(?i)\battend(?:ee|ees|ed|ing)?\b|\bin[- ]person\b|\bon[- ]site\b"));
static SENTENCE_END: Lazy<Regex> = Lazy::new(|| re(r"[.!?](?:\s|$)"));

fn word_number(raw: &str) -> Option<u32> {
    match raw.to_lowercase().as_str() {
        "a" | "an" | "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "seven" => Some(7),
        "ten" => Some(10),
        n => n.parse().ok(),
    }
}

/// The first sentence or two of a text, short enough for a tooltip.
fn summary(text: &str) -> String {
    let trimmed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = SENTENCE_END
        .find_iter(&trimmed)
        .map(|m| m.end())
        .find(|end| *end >= 60)
        .unwrap_or(trimmed.len());
    let short: String = trimmed[..cut].trim().chars().take(220).collect();
    short
}

/// Steps read out of free text. Empty when the text names no way to earn.
pub(crate) fn steps_from_text(text: &str) -> Vec<EarnStep> {
    let mut steps = Vec::new();
    let subscribe = SUBSCRIBE.is_match(text);
    // "Purchase 2 subscriptions" is subscribing, not buying a pass.
    let purchase = PURCHASE.is_match(text);
    if purchase {
        steps.push(EarnStep::Purchase { ticket: TICKET.is_match(text) });
    }
    if subscribe {
        let count = SUB_COUNT
            .captures(text)
            .and_then(|c| word_number(&c[1]))
            .filter(|n| *n > 1);
        steps.push(EarnStep::Subscribe { count });
    }
    if WATCH.is_match(text) {
        let minutes = MINUTES
            .captures(text)
            .and_then(|c| c[1].parse().ok())
            .or_else(|| HOURS.captures(text).and_then(|c| word_number(&c[1])).map(|h| h * 60));
        let days = DAYS
            .captures(text)
            .and_then(|c| word_number(&c[1]))
            .filter(|n| *n > 1);
        steps.push(EarnStep::Watch { minutes, days });
    }
    if CHEER.is_match(text) {
        steps.push(EarnStep::Cheer);
    }
    if CREATE.is_match(text) {
        steps.push(EarnStep::Create);
    }
    if ATTEND.is_match(text) && !purchase {
        steps.push(EarnStep::Attend);
    }
    steps
}

/// Steps from the relay's campaign-derived `requirement`, when it says anything.
fn steps_from_requirement(requirement: &serde_json::Value) -> Vec<EarnStep> {
    let uint = |key: &str| requirement.get(key).and_then(|v| v.as_u64()).map(|n| n as u32);
    let mut steps = Vec::new();
    let subs = uint("required_subs").unwrap_or(0);
    if subs > 0 {
        steps.push(EarnStep::Subscribe { count: (subs > 1).then_some(subs) });
    }
    let minutes = uint("watch_minutes").unwrap_or(0);
    if minutes > 0 {
        let days = uint("repeat_times").filter(|n| *n > 1);
        steps.push(EarnStep::Watch { minutes: Some(minutes), days });
    }
    steps
}

/// How a badge is earned, from its relay enrichment and scraped copy.
pub fn earn_path(enrichment: Option<&serde_json::Value>, more_info: Option<&str>) -> EarnPath {
    let text = |key: &str| {
        enrichment
            .and_then(|e| e.get(key))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let action = text("action");
    let how = text("how_to_earn");
    let copy = more_info.map(str::trim).filter(|s| !s.is_empty());

    let requirement = enrichment
        .and_then(|e| e.get("requirement"))
        .filter(|r| r.is_object());
    let random_of = requirement
        .and_then(|r| r.get("pool"))
        .and_then(|p| p.as_array())
        .map(|p| p.len() as u32)
        .filter(|n| *n > 1);
    // A null list means the whole category counts; an empty one says nothing.
    let channels = requirement
        .and_then(|r| r.get("channels"))
        .and_then(|c| c.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|c| c.as_str())
                .map(|c| c.trim().trim_start_matches('/').to_lowercase())
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|list| !list.is_empty());

    if let Some(r) = requirement {
        let steps = steps_from_requirement(r);
        if !steps.is_empty() {
            return EarnPath {
                steps,
                random_of,
                inferred: false,
                detail: action.or(how).map(summary),
                channels,
            };
        }
    }

    // The instruction is written to say how to earn the badge; the writeup and
    // the scraped copy say more besides, so they are asked only if it is silent.
    for source in [action, how, copy].into_iter().flatten() {
        let steps = steps_from_text(source);
        if !steps.is_empty() {
            return EarnPath {
                steps,
                random_of,
                inferred: true,
                detail: Some(summary(source)),
                channels,
            };
        }
    }

    EarnPath {
        steps: vec![EarnStep::Other],
        random_of,
        inferred: true,
        detail: action.or(how).or(copy).map(summary),
        channels,
    }
}

/// A live Drops campaign's requirement in the relay's `requirement` shape, so
/// a badge page reads one shape whichever source answered.
pub fn campaign_requirement(
    campaign: &crate::models::drops::DropCampaign,
    title: Option<&str>,
) -> Option<serde_json::Value> {
    let tier = crate::services::drops_overview::badge_tier_for(campaign, title)?;
    let channels: Vec<String> = if campaign.is_acl_based {
        campaign
            .allowed_channels
            .iter()
            .map(|c| c.name.to_lowercase())
            .filter(|c| !c.is_empty())
            .collect()
    } else {
        Vec::new()
    };
    Some(serde_json::json!({
        "watch_minutes": tier.required_minutes_watched.max(0),
        "required_subs": tier.required_subs,
        "category": (campaign.has_category && !campaign.game_name.is_empty())
            .then(|| campaign.game_name.clone()),
        "channels": (!channels.is_empty()).then_some(channels),
        "repeat_times": tier.required_days.max(1),
        "source": "campaign",
    }))
}

/// How a badge is earned and where, for its own page. `live_requirement`, from
/// the viewer's Drops list, beats the relay's because it is read right now.
pub fn earn_guide(
    enrichment: Option<&serde_json::Value>,
    more_info: Option<&str>,
    live_requirement: Option<serde_json::Value>,
) -> EarnGuide {
    let merged = live_requirement.map(|r| {
        let mut e = enrichment.cloned().unwrap_or_else(|| serde_json::json!({}));
        if let Some(obj) = e.as_object_mut() {
            obj.insert("requirement".into(), r);
        }
        e
    });
    let enrichment = merged.as_ref().or(enrichment);
    let earn = earn_path(enrichment, more_info);
    let requirement = enrichment.and_then(|e| e.get("requirement")).filter(|r| r.is_object());
    let category = requirement
        .and_then(|r| r.get("category"))
        .or_else(|| enrichment.and_then(|e| e.get("category")))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let summary = (!earn.inferred)
        .then(|| summary_sentence(&earn, category.as_deref()))
        .flatten();
    EarnGuide { earn, category, summary }
}

fn duration_phrase(minutes: u32) -> String {
    if minutes >= 60 && minutes % 60 == 0 {
        let h = minutes / 60;
        format!("{h} {}", if h == 1 { "hour" } else { "hours" })
    } else {
        format!("{minutes} {}", if minutes == 1 { "minute" } else { "minutes" })
    }
}

/// "dnd", "dnd or critrole", "dnd, critrole or geekandsundry".
fn channel_list(channels: &[String]) -> String {
    match channels {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// The requirement as one sentence, spoken to the viewer. `None` when the path
/// has no subscribe or watch step to state.
fn summary_sentence(earn: &EarnPath, category: Option<&str>) -> Option<String> {
    let sub = earn.steps.iter().find_map(|s| match s {
        EarnStep::Subscribe { count } => Some(count.unwrap_or(1)),
        _ => None,
    });
    let watch = earn.steps.iter().find_map(|s| match s {
        EarnStep::Watch { minutes: Some(m), days } => Some((*m, *days)),
        _ => None,
    });
    if sub.is_none() && watch.is_none() {
        return None;
    }
    let channels = earn.channels.as_deref().unwrap_or(&[]);

    // Where, with the preposition each verb takes: you watch OF a stream but ON
    // a channel, and subscribe TO a streamer but ON a channel.
    let (watch_where, sub_where) = match channels.len() {
        0 => match category {
            Some(cat) => (
                format!("of any {cat} stream with Drops enabled"),
                format!("to a streamer live in {cat}"),
            ),
            None => (
                "of a participating stream".to_string(),
                "to a participating streamer".to_string(),
            ),
        },
        n => {
            let on = match n {
                1 => format!("on {}'s channel", channels[0]),
                2 | 3 => format!("on {}", channel_list(channels)),
                n => format!("on one of the {n} participating channels"),
            };
            let live = match (n, category) {
                (_, None) => String::new(),
                (1, Some(cat)) => format!(" while it streams {cat}"),
                (_, Some(cat)) => format!(" while they stream {cat}"),
            };
            (format!("{on}{live}"), format!("{on}{live}"))
        }
    };

    let mut sentence = String::new();
    if let Some(n) = sub {
        let what = if n > 1 { format!("{n} subs") } else { "a sub".to_string() };
        sentence.push_str(&format!("Subscribe or gift {what} {sub_where}"));
    }
    if let Some((minutes, days)) = watch {
        let how_long = duration_phrase(minutes);
        let repeat = days.map(|d| format!(", on {d} different days")).unwrap_or_default();
        if sentence.is_empty() {
            sentence.push_str(&format!("Watch {how_long} {watch_where}{repeat}"));
        } else {
            sentence.push_str(&format!(", then watch {how_long}{repeat}"));
        }
    }
    sentence.push('.');
    if sub.is_some() {
        sentence.push_str(" Prime subs do not count.");
    }
    Some(sentence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn from_action(action: &str) -> Vec<EarnStep> {
        earn_path(Some(&json!({ "action": action })), None).steps
    }

    #[test]
    fn a_channel_limited_badge_says_which_channel_and_category() {
        // The d20 badge: thirty minutes on dnd, in Dungeons & Dragons only.
        let e = json!({
            "how_to_earn": "The d20 lands the same for the table and the person behind the screen.",
            "requirement": {
                "watch_minutes": 30, "required_subs": 0, "repeat_times": 1,
                "category": "Dungeons & Dragons", "channels": ["dnd"]
            }
        });
        let g = earn_guide(Some(&e), None, None);
        assert_eq!(g.earn.channels, Some(vec!["dnd".to_string()]));
        assert_eq!(
            g.summary.as_deref(),
            Some("Watch 30 minutes on dnd's channel while it streams Dungeons & Dragons.")
        );
    }

    #[test]
    fn a_live_campaign_fills_in_what_the_relay_never_matched() {
        // What the relay had for d20 before its campaign matched: a flavour
        // paragraph, no requirement.
        let relay = json!({ "action": "Watch for 30 minutes during the campaign.", "requirement": null });
        let live = json!({
            "watch_minutes": 30, "required_subs": 0, "repeat_times": 1,
            "category": "Dungeons & Dragons", "channels": ["dnd"]
        });
        let before = earn_guide(Some(&relay), None, None);
        assert!(before.earn.inferred);
        assert_eq!(before.summary, None);
        let after = earn_guide(Some(&relay), None, Some(live));
        assert!(!after.earn.inferred);
        assert_eq!(after.category.as_deref(), Some("Dungeons & Dragons"));
        assert!(after.summary.unwrap().contains("dnd's channel"));
    }

    #[test]
    fn the_summary_reads_for_categories_subs_days_and_long_channel_lists() {
        let guide = |r: serde_json::Value| earn_guide(Some(&json!({ "requirement": r })), None, None).summary;
        assert_eq!(
            guide(json!({ "watch_minutes": 120, "required_subs": 0, "category": "WARDOGS", "channels": null })),
            Some("Watch 2 hours of any WARDOGS stream with Drops enabled.".into())
        );
        assert_eq!(
            guide(json!({ "watch_minutes": 0, "required_subs": 1, "category": "Dungeons & Dragons", "channels": null })),
            Some("Subscribe or gift a sub to a streamer live in Dungeons & Dragons. Prime subs do not count.".into())
        );
        assert_eq!(
            guide(json!({ "watch_minutes": 20, "required_subs": 1, "repeat_times": 3, "category": null })),
            Some("Subscribe or gift a sub to a participating streamer, then watch 20 minutes, on 3 different days. Prime subs do not count.".into())
        );
        assert_eq!(
            guide(json!({ "watch_minutes": 15, "required_subs": 0, "category": "Special Events", "channels": ["ironmouse", "mouseathon"] })),
            Some("Watch 15 minutes on ironmouse or mouseathon while they stream Special Events.".into())
        );
        assert_eq!(
            guide(json!({ "watch_minutes": 60, "required_subs": 0, "channels": ["a1aa", "b2bb", "c3cc", "d4dd"] })),
            Some("Watch 1 hour on one of the 4 participating channels.".into())
        );
    }

    #[test]
    fn the_campaign_requirement_wins_and_is_not_inferred() {
        let e = json!({
            "action": "Watch 90 minutes in PAYDAY 3",
            "requirement": { "watch_minutes": 90, "required_subs": 0, "repeat_times": 1, "pool": null }
        });
        let p = earn_path(Some(&e), None);
        assert_eq!(p.steps, vec![EarnStep::Watch { minutes: Some(90), days: None }]);
        assert!(!p.inferred);
        assert_eq!(p.detail.as_deref(), Some("Watch 90 minutes in PAYDAY 3"));
    }

    #[test]
    fn a_pooled_badge_is_sub_then_watch_on_days_drawn_at_random() {
        // Squirtle, as the relay publishes it.
        let e = json!({ "requirement": {
            "watch_minutes": 20, "required_subs": 1, "repeat_times": 3,
            "pool": [{}, {}, {}], "container": "Great Ball"
        }});
        let p = earn_path(Some(&e), None);
        assert_eq!(
            p.steps,
            vec![
                EarnStep::Subscribe { count: None },
                EarnStep::Watch { minutes: Some(20), days: Some(3) },
            ]
        );
        assert_eq!(p.random_of, Some(3));
        // Pichu: a pool of one is not a random draw.
        let pichu = json!({ "requirement": { "watch_minutes": 20, "required_subs": 0, "repeat_times": 3, "pool": [{}] } });
        assert_eq!(earn_path(Some(&pichu), None).random_of, None);
    }

    #[test]
    fn subscribing_in_every_phrasing_is_one_subscribe_step() {
        for action in [
            "Subscribe or gift a non-Prime subscription to a streamer in the Dungeons & Dragons category.",
            "Subscribe or gift a subscription in PAYDAY 3",
            "Subscribe or gift a subscription.",
        ] {
            assert_eq!(from_action(action), vec![EarnStep::Subscribe { count: None }], "{action}");
        }
    }

    #[test]
    fn buying_subscriptions_is_subscribing_with_a_count_not_a_purchase() {
        assert_eq!(
            from_action("Purchase 2 subscriptions on any channel during the event period."),
            vec![EarnStep::Subscribe { count: Some(2) }]
        );
    }

    #[test]
    fn watching_reads_minutes_hours_and_days() {
        assert_eq!(
            from_action("Watch for 30 minutes during the campaign."),
            vec![EarnStep::Watch { minutes: Some(30), days: None }]
        );
        assert_eq!(
            from_action("Watch 1 hour of any livestream in the CONTROL Resonant category"),
            vec![EarnStep::Watch { minutes: Some(60), days: None }]
        );
        assert_eq!(
            from_action("Watch eligible co-streaming channels in Old School RuneScape during the campaign."),
            vec![EarnStep::Watch { minutes: None, days: None }]
        );
        assert_eq!(
            from_action("Watch a participating channel for 20 minutes on 3 different days."),
            vec![EarnStep::Watch { minutes: Some(20), days: Some(3) }]
        );
    }

    #[test]
    fn a_twitchcon_badge_is_a_ticket_purchase_not_attendance() {
        let copy = "BeachBall badge is a limited-time global chat badge awarded to attendees of TwitchCon San Diego 2026. \
                    The badge is granted to users who purchase a 1-day pass for TwitchCon San Diego 2026.";
        let p = earn_path(None, Some(copy));
        assert_eq!(p.steps, vec![EarnStep::Purchase { ticket: true }]);
        assert!(p.inferred);
        assert_eq!(
            steps_from_text("Purchase your TwitchCon 2025 ticket. Share your personal referral link."),
            vec![EarnStep::Purchase { ticket: true }]
        );
        assert_eq!(steps_from_text("Visit the booth in person on the show floor."), vec![EarnStep::Attend]);
    }

    #[test]
    fn creator_actions_are_create() {
        assert_eq!(from_action("Create and download or share a clip from your stream as a DJ."), vec![EarnStep::Create]);
        assert_eq!(from_action("Fill out a special form to become an official co-streamer."), vec![EarnStep::Create]);
        assert_eq!(from_action("Become an official co-streamer by filling out the special form."), vec![EarnStep::Create]);
    }

    #[test]
    fn a_viewer_supporting_co_streamers_is_not_a_creator() {
        assert_eq!(
            from_action("Subscribe or gift a subscription to a participating co-streamer."),
            vec![EarnStep::Subscribe { count: None }]
        );
    }

    #[test]
    fn cheering_is_its_own_step() {
        assert_eq!(steps_from_text("Cheer 500 Bits in any channel."), vec![EarnStep::Cheer]);
    }

    #[test]
    fn a_silent_instruction_falls_through_to_the_copy_then_to_other() {
        let p = earn_path(Some(&json!({ "action": "" })), Some("Watch 15 minutes during the event."));
        assert_eq!(p.steps, vec![EarnStep::Watch { minutes: Some(15), days: None }]);
        let unknown = earn_path(Some(&json!({ "action": "Be one of the first to try the new feature." })), None);
        assert_eq!(unknown.steps, vec![EarnStep::Other]);
        assert_eq!(unknown.detail.as_deref(), Some("Be one of the first to try the new feature."));
        assert_eq!(earn_path(None, None).steps, vec![EarnStep::Other]);
    }

    #[test]
    fn a_requirement_that_says_nothing_defers_to_the_text() {
        let e = json!({ "action": "Cheer 100 Bits.", "requirement": { "watch_minutes": 0, "required_subs": 0 } });
        let p = earn_path(Some(&e), None);
        assert_eq!(p.steps, vec![EarnStep::Cheer]);
        assert!(p.inferred);
    }
}
