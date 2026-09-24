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

    if let Some(r) = requirement {
        let steps = steps_from_requirement(r);
        if !steps.is_empty() {
            return EarnPath {
                steps,
                random_of,
                inferred: false,
                detail: action.or(how).map(summary),
            };
        }
    }

    // The instruction is written to say how to earn the badge; the writeup and
    // the scraped copy say more besides, so they are asked only if it is silent.
    for source in [action, how, copy].into_iter().flatten() {
        let steps = steps_from_text(source);
        if !steps.is_empty() {
            return EarnPath { steps, random_of, inferred: true, detail: Some(summary(source)) };
        }
    }

    EarnPath {
        steps: vec![EarnStep::Other],
        random_of,
        inferred: true,
        detail: action.or(how).or(copy).map(summary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn from_action(action: &str) -> Vec<EarnStep> {
        earn_path(Some(&json!({ "action": action })), None).steps
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
