//! The starter templates: ready-made rules, as data. A template is added in
//! Test, owned by and sending as the main account; the rule is then edited
//! and saved like any other.

use serde::Serialize;
use sn_mention_rule::judge::KNOWN_BOTS;

use super::compile::summarize;
use crate::models::automations::{
    AccountRef, Action, BadgeLeg, BadgeLink, ChannelScope, Conditions, EventKind, Limits, Line, LinkFallback, LinkPreset,
    Pick, Rule, Trigger,
};

/// (id, name, description), in the order the page lists them.
const TEMPLATES: &[(&str, &str, &str)] = &[
    ("away-reply", "Away reply", "Answers people who mention you while you're away on this device"),
    ("welcome", "Welcome first-timers", "Says hello to someone's first message in your chat while you're live"),
    ("raid-thanks", "Raid thanks", "Thanks a raider and welcomes their viewers"),
    ("sub-thanks", "Sub and gift thanks", "Thanks new subs, resubs and gifted subs"),
    ("badge-news", "Badge news", "Posts each new Twitch badge in your chat while you're live"),
];

/// One template as the page lists it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct TemplateView {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// The rule's summary line.
    pub summary: String,
}

pub fn templates() -> Vec<TemplateView> {
    TEMPLATES
        .iter()
        .filter_map(|&(id, name, description)| {
            let rule = template_rule(id, &AccountRef::default(), 0)?;
            Some(TemplateView { id, name, description, summary: summarize(&rule) })
        })
        .collect()
}

/// The rule a template adds: in Test, switched on, owned by and sending as
/// `primary`, with `origin` naming the template. None for an unknown id. The
/// caller gives it its ids and checks it with `validate`.
pub fn template_rule(id: &str, primary: &AccountRef, now_ms: i64) -> Option<Rule> {
    let &(_, name, _) = TEMPLATES.iter().find(|(t, _, _)| *t == id)?;
    let bots = || KNOWN_BOTS.iter().map(|b| b.to_string()).collect::<Vec<_>>();
    let line = |text: &str| Line { id: String::new(), text: text.to_string() };
    let reply = |text: &str| Action::Reply { id: String::new(), texts: vec![line(text)], pick: Pick::Random, thread: false };
    let send = |text: &str| Action::Send { id: String::new(), to: None, texts: vec![line(text)], pick: Pick::Random };
    let mut rule = Rule {
        name: name.to_string(),
        enabled: true,
        live: false,
        owner: primary.clone(),
        send_as: primary.clone(),
        origin: Some(format!("template:{id}")),
        created_ms: now_ms,
        updated_ms: now_ms,
        ..Rule::default()
    };
    match id {
        "away-reply" => {
            rule.when = Trigger::Mention { replies_too: true };
            rule.only_if = Conditions {
                channels: ChannelScope::AnyOpen {},
                only_away: true,
                skip_users: bots(),
                ..Conditions::default()
            };
            rule.limits = Limits { user_cooldown_seconds: 600, ..Limits::default() };
            rule.actions = vec![reply("@{user} I'm away right now: {away}")];
        }
        "welcome" => {
            rule.when = Trigger::FirstMessage {};
            rule.only_if = Conditions { only_live: true, skip_users: bots(), ..Conditions::default() };
            rule.actions = vec![reply("Welcome in, {user}!")];
        }
        "raid-thanks" => {
            rule.when = Trigger::ChannelEvent {
                events: vec![EventKind::Raid],
                min_months: 0,
                min_gifts: 0,
                min_viewers: 0,
                min_bits: 0,
            };
            rule.actions = vec![send("Thank you for the raid, {user}! Welcome, all {viewers} of you.")];
        }
        "sub-thanks" => {
            rule.when = Trigger::ChannelEvent {
                events: vec![EventKind::Sub, EventKind::Resub, EventKind::Gift, EventKind::GiftBomb],
                min_months: 0,
                min_gifts: 0,
                min_viewers: 0,
                min_bits: 0,
            };
            rule.actions = vec![send("Thank you, {user}! You {event}.")];
        }
        "badge-news" => {
            rule.when = Trigger::BadgeDrop {
                on: BadgeLeg::Both,
                link: BadgeLink { preset: LinkPreset::StreamNook, custom: String::new(), if_not_ready: LinkFallback::StreamNook },
                batch_text: "{badge.count} new Twitch badges just dropped: {badges}!".to_string(),
            };
            rule.only_if.only_live = true;
            rule.limits.rule_cooldown_seconds = 0;
            rule.actions = vec![send("New Twitch badge just dropped: {badge}! {badge.how} {badge.link}")];
        }
        _ => return None,
    }
    Some(rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::{validate, ValidateCtx};
    use crate::services::account_automations::compile::fixtures::me;

    #[test]
    fn every_template_saves_as_a_test_rule_of_the_main_account() {
        for view in templates() {
            let mut rule = template_rule(view.id, &me(), 42).unwrap();
            assert!(!rule.live && rule.enabled, "{}", view.id);
            assert_eq!(rule.owner, me());
            assert_eq!(rule.send_as, me());
            assert_eq!(rule.origin.as_deref(), Some(format!("template:{}", view.id).as_str()));
            assert_eq!((rule.created_ms, rule.updated_ms), (42, 42));
            let errors = validate(&mut rule, &ValidateCtx::default());
            assert!(errors.is_empty(), "{}: {errors:?}", view.id);
            assert!(!view.summary.is_empty());
        }
        assert_eq!(
            templates().iter().map(|t| t.id).collect::<Vec<_>>(),
            vec!["away-reply", "welcome", "raid-thanks", "sub-thanks", "badge-news"]
        );
        assert!(template_rule("nope", &me(), 0).is_none());
    }

    #[test]
    fn the_templates_do_what_their_names_say() {
        let away = template_rule("away-reply", &me(), 0).unwrap();
        assert_eq!(away.when, Trigger::Mention { replies_too: true });
        assert_eq!(away.only_if.channels, ChannelScope::AnyOpen {});
        assert!(away.only_if.only_away);
        assert_eq!(away.limits.user_cooldown_seconds, 600);
        assert!(away.only_if.skip_users.iter().any(|u| u == "nightbot"));
        assert_eq!(away.actions[0].lines()[0].text, "@{user} I'm away right now: {away}");

        let welcome = template_rule("welcome", &me(), 0).unwrap();
        assert_eq!(welcome.when, Trigger::FirstMessage {});
        assert_eq!(welcome.only_if.channels, ChannelScope::Own {});
        assert!(welcome.only_if.only_live);

        let subs = template_rule("sub-thanks", &me(), 0).unwrap();
        assert!(matches!(&subs.when, Trigger::ChannelEvent { events, .. } if events.len() == 4));

        let badge = template_rule("badge-news", &me(), 0).unwrap();
        assert!(matches!(&badge.when, Trigger::BadgeDrop { on: BadgeLeg::Both, link, .. } if link.preset == LinkPreset::StreamNook));
        assert_eq!(
            templates()[4].summary,
            "When a new Twitch badge is announced or can be earned in your channel while you're live: send a message"
        );
    }
}
