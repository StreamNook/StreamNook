//! Own-channel EventSub topics: which topics the rules need, how each is
//! subscribed, how Twitch's refusals read, and what a rule's status says.
//!
//! Pure. The socket that carries them is the whisper session in
//! `whisper_service` (D276, D288): it holds the wanted set, subscribes it on
//! each new session (never again on a `session_reconnect`, which carries the
//! subscriptions over), routes each notification by `payload.subscription.type`
//! to [`super::on_own_topic`], and keeps an [`OwnState`] this module turns
//! into each rule's held text. The owner must be the registry's primary:
//! the whisper socket follows the primary, and its token is the primary's.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde_json::{json, Value};

use super::compile::Compiled;
use crate::models::automations::Trigger;

/// The rule's held text while its socket is not up or its topic is not yet held.
pub const WAITING_CONNECTION: &str = "Waiting for a Twitch connection";
/// More than three subscriptions of one type and condition (other devices hold them).
pub const OTHER_DEVICES: &str = "Listening on 3 other devices";
/// Twitch's total cost, session limit or subscription limit.
pub const LIMIT_FULL: &str = "Twitch's subscription limit is full";
pub const NEEDS_AFFILIATE: &str = "Needs affiliate";
pub const SIGN_IN_AGAIN: &str = "Sign in again";
pub const ENDED: &str = "Twitch ended this subscription";
pub const REFUSED: &str = "Twitch refused this subscription";

/// "Waiting for @me to be your main account".
pub fn waiting_main(login: &str) -> String {
    if login.is_empty() {
        "Waiting for this account to be your main account".to_string()
    } else {
        format!("Waiting for @{login} to be your main account")
    }
}

/// One own-channel EventSub topic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TopicKind {
    Follow,
    Redemption,
    PollBegin,
    PredictionBegin,
}

pub const ALL_TOPICS: [TopicKind; 4] = [
    TopicKind::Follow,
    TopicKind::Redemption,
    TopicKind::PollBegin,
    TopicKind::PredictionBegin,
];

impl TopicKind {
    /// Twitch's subscription type.
    pub fn sub_type(self) -> &'static str {
        match self {
            TopicKind::Follow => "channel.follow",
            TopicKind::Redemption => "channel.channel_points_custom_reward_redemption.add",
            TopicKind::PollBegin => "channel.poll.begin",
            TopicKind::PredictionBegin => "channel.prediction.begin",
        }
    }

    pub fn version(self) -> &'static str {
        match self {
            TopicKind::Follow => "2",
            _ => "1",
        }
    }

    pub fn from_sub_type(sub_type: &str) -> Option<TopicKind> {
        ALL_TOPICS.into_iter().find(|k| k.sub_type() == sub_type)
    }
}

/// A topic's condition: a follow names the broadcaster and the moderator
/// (both the owner); the others the broadcaster. The redemption topic is
/// subscribed once, unfiltered, and filtered by reward id in the matcher.
pub fn condition(kind: TopicKind, owner_id: &str) -> Value {
    match kind {
        TopicKind::Follow => {
            json!({ "broadcaster_user_id": owner_id, "moderator_user_id": owner_id })
        }
        _ => json!({ "broadcaster_user_id": owner_id }),
    }
}

/// The `POST /eventsub/subscriptions` body for one topic on one session.
pub fn subscription_body(kind: TopicKind, owner_id: &str, session_id: &str) -> Value {
    json!({
        "type": kind.sub_type(),
        "version": kind.version(),
        "condition": condition(kind, owner_id),
        "transport": { "method": "websocket", "session_id": session_id }
    })
}

/// The id of the subscription a 2xx answer created.
pub fn subscription_id(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v["data"][0]["id"]
        .as_str()
        .map(str::to_string)
        .filter(|id| !id.is_empty())
}

/// Why a topic is not held, in the words a rule's row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopicRefusal {
    DevicesFull,
    ConnectionLimit,
    LimitFull,
    NeedsAffiliate,
    SignInAgain,
    Refused,
    Ended,
}

impl TopicRefusal {
    pub fn text(self) -> &'static str {
        match self {
            TopicRefusal::DevicesFull => OTHER_DEVICES,
            TopicRefusal::ConnectionLimit => WAITING_CONNECTION,
            TopicRefusal::LimitFull => LIMIT_FULL,
            TopicRefusal::NeedsAffiliate => NEEDS_AFFILIATE,
            TopicRefusal::SignInAgain => SIGN_IN_AGAIN,
            TopicRefusal::Refused => REFUSED,
            TopicRefusal::Ended => ENDED,
        }
    }
}

/// What a subscribe answer means for the topic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscribeVerdict {
    /// Held (2xx), or already held (409, no change).
    Held,
    /// Refused: the topic is dropped with this status and not retried.
    Refused(TopicRefusal),
    /// A 5xx or an odd answer: a later sync tries again.
    Retry,
}

/// A 429 has five causes (D276, D288): more than three subscriptions of one
/// type and condition, more than three WebSocket connections for the user,
/// the total cost, the session's subscription limit and the overall
/// subscription limit. The body decides; its exact wording is unverified, so
/// each match is a substring test, and a body that matches none reads "full".
pub fn classify_429(body: &str) -> TopicRefusal {
    let lower = body.to_ascii_lowercase();
    if lower.contains("enabled or pending subscriptions") {
        TopicRefusal::DevicesFull
    } else if lower.contains("transport") {
        TopicRefusal::ConnectionLimit
    } else {
        TopicRefusal::LimitFull
    }
}

/// A 400, 401 or 403: "Needs affiliate" when the body says so, "Sign in again"
/// for a missing scope or token, and one more plain text for any other 400.
fn classify_denied(status: u16, body: &str) -> TopicRefusal {
    let lower = body.to_ascii_lowercase();
    if lower.contains("affiliate") || lower.contains("partner") {
        TopicRefusal::NeedsAffiliate
    } else if status == 400 {
        TopicRefusal::Refused
    } else {
        TopicRefusal::SignInAgain
    }
}

pub fn classify_subscribe(status: u16, body: &str) -> SubscribeVerdict {
    match status {
        200..=299 | 409 => SubscribeVerdict::Held,
        429 => SubscribeVerdict::Refused(classify_429(body)),
        400 | 401 | 403 => SubscribeVerdict::Refused(classify_denied(status, body)),
        _ => SubscribeVerdict::Retry,
    }
}

/// A revocation's status (`authorization_revoked`, `user_removed`,
/// `version_removed`, ...) as the topic's text.
pub fn revoked(status: &str) -> TopicRefusal {
    match status {
        "authorization_revoked" | "user_removed" => TopicRefusal::SignInAgain,
        _ => TopicRefusal::Ended,
    }
}

/// What the whisper session holds for the engine right now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnState {
    /// A whisper session is up and subscribed (the own topics can be too).
    pub session_up: bool,
    pub held: BTreeSet<TopicKind>,
    pub refused: BTreeMap<TopicKind, TopicRefusal>,
}

impl OwnState {
    pub const fn new() -> OwnState {
        OwnState {
            session_up: false,
            held: BTreeSet::new(),
            refused: BTreeMap::new(),
        }
    }
}

/// The topic a trigger listens to, if it listens to one.
pub fn topic_of(when: &Trigger) -> Option<TopicKind> {
    match when {
        Trigger::Follow {} => Some(TopicKind::Follow),
        Trigger::RewardRedeemed { .. } => Some(TopicKind::Redemption),
        Trigger::PollStarted {} => Some(TopicKind::PollBegin),
        Trigger::PredictionStarted {} => Some(TopicKind::PredictionBegin),
        _ => None,
    }
}

/// The owner and the topics the running rules need. Only rules owned by the
/// main account count: the socket is the main account's.
pub fn wanted(compiled: &Compiled, main_id: Option<&str>) -> Option<(String, BTreeSet<TopicKind>)> {
    let main = main_id?;
    let kinds: BTreeSet<TopicKind> = compiled
        .running()
        .filter(|r| r.rule.owner.id == main)
        .filter_map(|r| topic_of(&r.rule.when))
        .collect();
    (!kinds.is_empty()).then(|| (main.to_string(), kinds))
}

/// What to add and what to delete to make `held` match `desired`. A topic
/// Twitch refused is never added again until it leaves the wanted set or a
/// new session starts (a 429 is not retried).
pub fn plan_sync(
    desired: &BTreeSet<TopicKind>,
    held: &BTreeSet<TopicKind>,
    refused: &BTreeMap<TopicKind, TopicRefusal>,
) -> (Vec<TopicKind>, Vec<TopicKind>) {
    let add = desired
        .iter()
        .copied()
        .filter(|k| !held.contains(k) && !refused.contains_key(k))
        .collect();
    let remove = held
        .iter()
        .copied()
        .filter(|k| !desired.contains(k))
        .collect();
    (add, remove)
}

/// Each running own-channel rule's held text, by rule id: not the main
/// account's rule, no socket, a refused or revoked topic, or one not yet
/// subscribed. A rule whose topic is held is absent.
pub fn rule_holds(
    compiled: &Compiled,
    main_id: Option<&str>,
    state: &OwnState,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for cr in compiled.running() {
        let Some(kind) = topic_of(&cr.rule.when) else {
            continue;
        };
        let text = if main_id != Some(cr.rule.owner.id.as_str()) {
            waiting_main(&cr.owner_login)
        } else if !state.session_up {
            WAITING_CONNECTION.to_string()
        } else if let Some(why) = state.refused.get(&kind) {
            why.text().to_string()
        } else if !state.held.contains(&kind) {
            WAITING_CONNECTION.to_string()
        } else {
            continue;
        };
        out.insert(cr.rule.id.clone(), text);
    }
    out
}

/// The last event keys seen, so a notification Twitch redelivers (or two
/// topics that name one event) is answered once on this device.
pub struct SeenKeys {
    order: VecDeque<String>,
    keys: HashSet<String>,
}

impl Default for SeenKeys {
    fn default() -> Self {
        SeenKeys::new()
    }
}

impl SeenKeys {
    const KEEP: usize = 256;

    pub fn new() -> SeenKeys {
        SeenKeys {
            order: VecDeque::new(),
            keys: HashSet::new(),
        }
    }

    /// True the first time `key` is seen.
    pub fn first_time(&mut self, key: &str) -> bool {
        if !self.keys.insert(key.to_string()) {
            return false;
        }
        self.order.push_back(key.to_string());
        if self.order.len() > Self::KEEP {
            if let Some(oldest) = self.order.pop_front() {
                self.keys.remove(&oldest);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::RunState;
    use crate::services::account_automations::compile::fixtures::{
        alt, compiled_with, notify, rule, ME,
    };

    #[test]
    fn each_topic_has_its_own_condition() {
        assert_eq!(
            condition(TopicKind::Follow, "100"),
            json!({ "broadcaster_user_id": "100", "moderator_user_id": "100" })
        );
        for kind in [
            TopicKind::Redemption,
            TopicKind::PollBegin,
            TopicKind::PredictionBegin,
        ] {
            assert_eq!(
                condition(kind, "100"),
                json!({ "broadcaster_user_id": "100" }),
                "{kind:?}: no reward_id, one unfiltered subscription"
            );
        }
    }

    #[test]
    fn a_subscription_body_names_the_websocket_session() {
        let body = subscription_body(TopicKind::Follow, "100", "S1");
        assert_eq!(body["type"], "channel.follow");
        assert_eq!(body["version"], "2");
        assert_eq!(
            body["transport"],
            json!({ "method": "websocket", "session_id": "S1" })
        );
        let redeem = subscription_body(TopicKind::Redemption, "100", "S1");
        assert_eq!(
            redeem["type"],
            "channel.channel_points_custom_reward_redemption.add"
        );
        assert_eq!(redeem["version"], "1");
        for kind in ALL_TOPICS {
            assert_eq!(TopicKind::from_sub_type(kind.sub_type()), Some(kind));
        }
        assert_eq!(
            TopicKind::from_sub_type("user.whisper.message"),
            None,
            "a whisper is not an own topic"
        );
    }

    #[test]
    fn the_created_subscription_id_is_read() {
        assert_eq!(
            subscription_id(r#"{"data":[{"id":"sub-1","status":"enabled"}]}"#).as_deref(),
            Some("sub-1")
        );
        assert_eq!(subscription_id(r#"{"data":[]}"#), None);
        assert_eq!(subscription_id("not json"), None);
    }

    #[test]
    fn a_429_is_read_from_its_body_and_none_is_retried() {
        // The wording of each cause is unverified, so each text has its own row (D288).
        let three = r#"{"error":"Too Many Requests","status":429,"message":"the request exceeds the number of subscriptions that you may create with the same combination of type and condition values: limit of 3 enabled or pending subscriptions"}"#;
        assert_eq!(
            classify_subscribe(429, three),
            SubscribeVerdict::Refused(TopicRefusal::DevicesFull)
        );
        assert_eq!(
            TopicRefusal::DevicesFull.text(),
            "Listening on 3 other devices"
        );
        let transports = r#"{"error":"Too Many Requests","status":429,"message":"number of websocket transports limit exceeded"}"#;
        assert_eq!(
            classify_subscribe(429, transports),
            SubscribeVerdict::Refused(TopicRefusal::ConnectionLimit)
        );
        assert_eq!(
            TopicRefusal::ConnectionLimit.text(),
            "Waiting for a Twitch connection"
        );
        for body in [
            r#"{"status":429,"message":"total cost exceeds the max_total_cost for this connection"}"#,
            r#"{"status":429,"message":"the websocket session has too many subscriptions: session subscription limit"}"#,
            r#"{"status":429,"message":"subscription limit reached"}"#,
            "",
            "slow down",
        ] {
            assert_eq!(
                classify_subscribe(429, body),
                SubscribeVerdict::Refused(TopicRefusal::LimitFull),
                "{body}"
            );
        }
        assert_eq!(
            TopicRefusal::LimitFull.text(),
            "Twitch's subscription limit is full"
        );
        // Refused is final until the topic leaves the wanted set or a new session starts.
        let wanted: BTreeSet<TopicKind> = [TopicKind::Follow].into();
        let refused: BTreeMap<TopicKind, TopicRefusal> =
            [(TopicKind::Follow, TopicRefusal::LimitFull)].into();
        assert_eq!(
            plan_sync(&wanted, &BTreeSet::new(), &refused),
            (vec![], vec![])
        );
    }

    #[test]
    fn the_other_answers_have_their_own_readings() {
        assert_eq!(classify_subscribe(202, "{}"), SubscribeVerdict::Held);
        assert_eq!(
            classify_subscribe(409, "{}"),
            SubscribeVerdict::Held,
            "already held changes nothing"
        );
        assert_eq!(
            classify_subscribe(
                403,
                r#"{"message":"subscription missing proper authorization"}"#
            ),
            SubscribeVerdict::Refused(TopicRefusal::SignInAgain)
        );
        assert_eq!(
            classify_subscribe(401, "{}"),
            SubscribeVerdict::Refused(TopicRefusal::SignInAgain)
        );
        assert_eq!(
            classify_subscribe(403, r#"{"message":"only affiliates and partners"}"#),
            SubscribeVerdict::Refused(TopicRefusal::NeedsAffiliate)
        );
        assert_eq!(
            classify_subscribe(
                400,
                r#"{"message":"channel must be a Partner or Affiliate"}"#
            ),
            SubscribeVerdict::Refused(TopicRefusal::NeedsAffiliate)
        );
        assert_eq!(
            classify_subscribe(400, r#"{"message":"bad condition"}"#),
            SubscribeVerdict::Refused(TopicRefusal::Refused)
        );
        for status in [0u16, 500, 502, 503] {
            assert_eq!(
                classify_subscribe(status, ""),
                SubscribeVerdict::Retry,
                "{status}"
            );
        }
        assert_eq!(TopicRefusal::NeedsAffiliate.text(), "Needs affiliate");
        assert_eq!(TopicRefusal::SignInAgain.text(), "Sign in again");
        assert_eq!(revoked("authorization_revoked"), TopicRefusal::SignInAgain);
        assert_eq!(revoked("user_removed"), TopicRefusal::SignInAgain);
        assert_eq!(revoked("version_removed"), TopicRefusal::Ended);
        assert_eq!(revoked("whatever"), TopicRefusal::Ended);
    }

    #[test]
    fn a_sync_adds_what_is_missing_and_deletes_what_is_not_wanted() {
        let desired: BTreeSet<TopicKind> = [TopicKind::Follow, TopicKind::PollBegin].into();
        let held: BTreeSet<TopicKind> = [TopicKind::Follow, TopicKind::Redemption].into();
        let (add, remove) = plan_sync(&desired, &held, &BTreeMap::new());
        assert_eq!(
            (add, remove),
            (vec![TopicKind::PollBegin], vec![TopicKind::Redemption])
        );
        let (add, remove) = plan_sync(&BTreeSet::new(), &held, &BTreeMap::new());
        assert_eq!(
            (add.len(), remove.len()),
            (0, 2),
            "no wanted topic deletes every held one"
        );
    }

    #[test]
    fn only_the_main_accounts_running_rules_want_topics() {
        let mut alts = rule("alts", Trigger::Follow {}, vec![notify("x")]);
        alts.owner = alt();
        alts.send_as = alt();
        let c = compiled_with(
            &[
                rule("a", Trigger::Follow {}, vec![notify("x")]),
                rule(
                    "b",
                    Trigger::RewardRedeemed {
                        reward_id: Some("r1".into()),
                    },
                    vec![notify("x")],
                ),
                rule(
                    "c",
                    Trigger::RewardRedeemed { reward_id: None },
                    vec![notify("x")],
                ),
                alts,
                rule(
                    "m",
                    Trigger::Mention { replies_too: false },
                    vec![notify("x")],
                ),
            ],
            RunState::On,
        );
        let (owner, kinds) = wanted(&c, Some(ME)).unwrap();
        assert_eq!(owner, ME);
        assert_eq!(
            kinds,
            [TopicKind::Follow, TopicKind::Redemption].into(),
            "several reward rules share one unfiltered subscription"
        );
        assert!(wanted(&c, None).is_none());
        assert!(wanted(
            &compiled_with(
                &[rule(
                    "m",
                    Trigger::Mention { replies_too: false },
                    vec![notify("x")]
                )],
                RunState::On
            ),
            Some(ME)
        )
        .is_none());
        assert!(wanted(
            &compiled_with(
                &[rule("a", Trigger::Follow {}, vec![notify("x")])],
                RunState::Off
            ),
            Some(ME)
        )
        .is_none());
    }

    #[test]
    fn a_test_rule_holds_its_topics_too_so_it_can_log_would() {
        use crate::services::account_automations::compile::fixtures::compiled_for;
        let mut test_rule = rule("t", Trigger::Follow {}, vec![notify("x")]);
        test_rule.live = false;
        let c = compiled_for(
            &[test_rule],
            RunState::On,
            &Default::default(),
            &crate::services::account_automations::compile::fixtures::accounts(),
        );
        assert_eq!(wanted(&c, Some(ME)).unwrap().1, [TopicKind::Follow].into());
        let tested = compiled_with(
            &[rule("l", Trigger::Follow {}, vec![notify("x")])],
            RunState::Test,
        );
        assert!(
            wanted(&tested, Some(ME)).is_some(),
            "run Test holds them for every rule"
        );
    }

    #[test]
    fn a_rules_held_text_follows_the_socket_and_the_refusals() {
        let mut alts = rule("alts", Trigger::Follow {}, vec![notify("x")]);
        alts.owner = alt();
        alts.send_as = alt();
        let c = compiled_with(
            &[
                rule("poll", Trigger::PollStarted {}, vec![notify("x")]),
                rule("follow", Trigger::Follow {}, vec![notify("x")]),
                alts,
            ],
            RunState::On,
        );
        let mut state = OwnState::new();
        let holds = rule_holds(&c, Some(ME), &state);
        assert_eq!(
            holds["poll"], "Waiting for a Twitch connection",
            "no socket (the Refused exit, or over the three-connection limit)"
        );
        assert_eq!(holds["alts"], "Waiting for @myalt to be your main account");
        state.session_up = true;
        state.held.insert(TopicKind::Follow);
        state
            .refused
            .insert(TopicKind::PollBegin, TopicRefusal::NeedsAffiliate);
        let holds = rule_holds(&c, Some(ME), &state);
        assert_eq!(holds["poll"], "Needs affiliate");
        assert!(!holds.contains_key("follow"), "a held topic has no hold");
        state
            .refused
            .insert(TopicKind::PollBegin, TopicRefusal::DevicesFull);
        assert_eq!(
            rule_holds(&c, Some(ME), &state)["poll"],
            "Listening on 3 other devices"
        );
        state.refused.clear();
        assert_eq!(
            rule_holds(&c, Some(ME), &state)["poll"],
            "Waiting for a Twitch connection",
            "not yet subscribed"
        );
        assert_eq!(
            rule_holds(&c, None, &state)["follow"],
            "Waiting for @me to be your main account"
        );
        assert!(rule_holds(
            &compiled_with(
                &[rule("follow", Trigger::Follow {}, vec![notify("x")])],
                RunState::Off
            ),
            Some(ME),
            &state
        )
        .is_empty());
    }

    #[test]
    fn seen_keys_answer_an_event_once() {
        let mut seen = SeenKeys::new();
        assert!(seen.first_time("reward:1"));
        assert!(!seen.first_time("reward:1"));
        for i in 0..300 {
            seen.first_time(&format!("k{i}"));
        }
        assert!(seen.first_time("reward:1"), "an old key ages out");
        assert!(!seen.first_time("k299"));
    }
}
