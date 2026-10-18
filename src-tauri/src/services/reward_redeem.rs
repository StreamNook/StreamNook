//! Redeeming a channel reward, for every shell: what redeeming each reward
//! involves (its kind and the words its controls show, stamped on the judged
//! list as `ChannelReward::view`), and one command that redeems by reward id.
//! It looks the reward up in Rust's judged list, refuses one the verdict
//! blocks, routes it to the right Twitch call by its built-in type, and keeps
//! one transaction id per reward and text until it succeeds, so a retry after
//! a lost answer is recognised by Twitch instead of charged twice.

use crate::models::drops::{ChannelReward, RedemptionResult, RewardView, UnlockedEmote};
use crate::services::channel_rewards::{self, HIGHLIGHTED_MESSAGE_REWARD, SUB_MODE_BYPASS_REWARD};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewardKind {
    Highlight,
    SubMessage,
    RandomEmote,
    ChooseEmote,
    ModifyEmote,
    Input,
    Standard,
}

impl RewardKind {
    pub fn name(self) -> &'static str {
        match self {
            RewardKind::Highlight => "highlight",
            RewardKind::SubMessage => "sub_message",
            RewardKind::RandomEmote => "random_emote",
            RewardKind::ChooseEmote => "choose_emote",
            RewardKind::ModifyEmote => "modify_emote",
            RewardKind::Input => "input",
            RewardKind::Standard => "standard",
        }
    }

    /// The redeem carries the viewer's typed text.
    pub fn takes_text(self) -> bool {
        matches!(self, RewardKind::Highlight | RewardKind::SubMessage | RewardKind::Input)
    }

    /// It opens an emote picker before anything is spent.
    pub fn picks_emote(self) -> bool {
        matches!(self, RewardKind::ChooseEmote | RewardKind::ModifyEmote)
    }
}

/// What redeeming a reward involves, from its built-in type (never its title).
pub fn kind_of(reward_type: Option<&str>, input_required: bool) -> RewardKind {
    match reward_type {
        Some(HIGHLIGHTED_MESSAGE_REWARD) => RewardKind::Highlight,
        Some(SUB_MODE_BYPASS_REWARD) => RewardKind::SubMessage,
        Some("RANDOM_SUB_EMOTE_UNLOCK") => RewardKind::RandomEmote,
        Some("CHOSEN_SUB_EMOTE_UNLOCK") => RewardKind::ChooseEmote,
        Some("CHOSEN_MODIFIED_SUB_EMOTE_UNLOCK") => RewardKind::ModifyEmote,
        _ if input_required => RewardKind::Input,
        _ => RewardKind::Standard,
    }
}

/// `v` to one decimal place, or whole from `whole_from` up, never with ".0".
fn short_number(v: f64, whole_from: f64) -> String {
    let r = if v >= whole_from { v.round() } else { (v * 10.0).round() / 10.0 };
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// A cost for a narrow pill: 950, 3k, 2.5k, 1.2M. The rounded value picks the
/// unit, so 999,999 reads "1M", never "1000k".
pub fn cost_short(cost: i64) -> String {
    if cost >= 999_500 {
        return format!("{}M", short_number(cost as f64 / 1_000_000.0, 10.0));
    }
    if cost >= 1000 {
        return format!("{}k", short_number(cost as f64 / 1000.0, 100.0));
    }
    cost.to_string()
}

pub fn view_of(reward: &ChannelReward) -> RewardView {
    let kind = kind_of(reward.reward_type.as_deref(), reward.is_user_input_required);
    let message = matches!(kind, RewardKind::Highlight | RewardKind::SubMessage);
    RewardView {
        kind: kind.name().to_string(),
        takes_text: kind.takes_text(),
        cost_label: cost_short(reward.cost as i64),
        action: match kind {
            RewardKind::Highlight => "Send highlighted",
            RewardKind::SubMessage => "Send",
            RewardKind::ChooseEmote | RewardKind::ModifyEmote => "Choose emote",
            _ => "Redeem",
        }
        .to_string(),
        busy: if message { "Sending..." } else { "Redeeming..." }.to_string(),
        placeholder: kind.takes_text().then(|| {
            if kind == RewardKind::Input { "Type your answer..." } else { "Type your message..." }.to_string()
        }),
        hint: (!kind.takes_text() && !kind.picks_emote()).then(|| "Can't be undone".to_string()),
    }
}

/// What a redeem came to, with the words to show.
#[derive(Debug, Clone, Serialize)]
pub struct RedeemOutcome {
    pub success: bool,
    /// The toast: "Redeemed: <title>", "Highlighted message sent!", or why it failed.
    pub message: String,
    pub new_balance: Option<i32>,
    /// The emote an unlock gave, for the reveal.
    pub unlocked_emote: Option<UnlockedEmote>,
}

/// One id per reward and text until it succeeds.
static TRANSACTIONS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn transaction_for(login: &str, reward_id: &str, text: &str) -> String {
    let key = format!("{login}\n{reward_id}\n{text}");
    let mut slot = TRANSACTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let map = slot.get_or_insert_with(HashMap::new);
    map.entry(key).or_insert_with(|| channel_rewards::transaction_id_or_new(None)).clone()
}

fn forget_transaction(login: &str, reward_id: &str, text: &str) {
    let key = format!("{login}\n{reward_id}\n{text}");
    if let Some(map) = TRANSACTIONS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        map.remove(&key);
    }
}

fn failed(message: impl Into<String>) -> RedeemOutcome {
    RedeemOutcome { success: false, message: message.into(), new_balance: None, unlocked_emote: None }
}

/// The words for a result: the success toast for the kind, else Twitch's own
/// message, else the kind's failure words.
fn outcome(kind: RewardKind, reward: &ChannelReward, result: RedemptionResult, emote: Option<UnlockedEmote>) -> RedeemOutcome {
    if !result.success {
        let fallback = match kind {
            RewardKind::ModifyEmote => "Failed to modify emote",
            RewardKind::ChooseEmote => "Failed to unlock emote",
            _ => "Redemption failed",
        };
        return failed(result.error_message.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| fallback.to_string()));
    }
    let unlocked = result.unlocked_emote.or(emote);
    let message = match kind {
        RewardKind::Highlight => "Highlighted message sent!".to_string(),
        RewardKind::SubMessage => "Message sent".to_string(),
        RewardKind::RandomEmote | RewardKind::ChooseEmote | RewardKind::ModifyEmote => match &unlocked {
            Some(e) => format!("Unlocked {}", e.name),
            None => "Random emote unlocked!".to_string(),
        },
        _ => format!("Redeemed: {}", reward.title),
    };
    RedeemOutcome { success: true, message, new_balance: result.new_balance, unlocked_emote: unlocked }
}

/// Redeem `reward_id` on a channel. `text` is the typed answer or message;
/// `emote_id` / `emote_name` the pick for an emote unlock.
pub async fn redeem(
    login: &str,
    channel_id: &str,
    reward_id: &str,
    text: Option<String>,
    emote_id: Option<String>,
    emote_name: Option<String>,
) -> RedeemOutcome {
    let login = login.trim().to_lowercase();
    let Some(list) = channel_rewards::cached(&login).await else {
        return failed("Rewards are still loading. Try again in a moment");
    };
    let Some(reward) = list.rewards.into_iter().find(|r| r.id == reward_id) else {
        return failed("This reward is no longer offered");
    };
    if !reward.availability.redeemable {
        let why = reward.availability.detail.clone().or_else(|| reward.availability.label.clone());
        return failed(why.unwrap_or_else(|| "This reward can't be redeemed right now".to_string()));
    }
    let kind = kind_of(reward.reward_type.as_deref(), reward.is_user_input_required);
    let text = text.map(|t| t.trim().to_string()).unwrap_or_default();
    if kind.takes_text() && text.is_empty() {
        return failed("Type something first");
    }
    let cost = reward.cost;
    let channel_id = channel_id.to_string();
    let tx = transaction_for(&login, reward_id, &text);
    use crate::commands::drops as cmd;
    let result = match kind {
        RewardKind::Highlight => cmd::send_highlighted_message(channel_id, text.clone(), cost, Some(tx)).await,
        RewardKind::SubMessage => cmd::send_sub_mode_message(channel_id, login.clone(), text.clone(), cost, Some(tx)).await,
        RewardKind::RandomEmote => cmd::unlock_random_emote(channel_id, cost).await,
        RewardKind::ChooseEmote | RewardKind::ModifyEmote => {
            let Some(id) = emote_id.clone().filter(|e| !e.is_empty()) else {
                return failed(if kind == RewardKind::ModifyEmote {
                    "Please select an emote and modifier"
                } else {
                    "Please select an emote"
                });
            };
            if kind == RewardKind::ModifyEmote {
                cmd::unlock_modified_emote(channel_id, id, cost).await
            } else {
                cmd::unlock_chosen_emote(channel_id, id, cost).await
            }
        }
        RewardKind::Input | RewardKind::Standard => {
            cmd::redeem_channel_reward(
                channel_id,
                reward.id.clone(),
                cost,
                reward.title.clone(),
                reward.prompt.clone(),
                (kind == RewardKind::Input).then(|| text.clone()),
                Some(tx),
            )
            .await
        }
    };
    let picked = emote_id.zip(emote_name).map(|(id, name)| UnlockedEmote {
        image_url: format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0"),
        id,
        name,
    });
    if let Ok(r) = &result {
        channel_rewards::note_redeem_answer(&login, reward_id, r.success, r.error_code.as_deref());
    }
    let done = match result {
        Ok(r) => outcome(kind, &reward, r, picked),
        Err(e) => failed(if e.trim().is_empty() { "Redemption failed".to_string() } else { e }),
    };
    if done.success {
        forget_transaction(&login, reward_id, &text);
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_the_built_in_type_never_the_title() {
        assert_eq!(kind_of(Some(HIGHLIGHTED_MESSAGE_REWARD), true), RewardKind::Highlight);
        assert_eq!(kind_of(Some(SUB_MODE_BYPASS_REWARD), true), RewardKind::SubMessage);
        assert_eq!(kind_of(Some("RANDOM_SUB_EMOTE_UNLOCK"), false), RewardKind::RandomEmote);
        assert_eq!(kind_of(Some("CHOSEN_SUB_EMOTE_UNLOCK"), false), RewardKind::ChooseEmote);
        assert_eq!(kind_of(Some("CHOSEN_MODIFIED_SUB_EMOTE_UNLOCK"), false), RewardKind::ModifyEmote);
        assert_eq!(kind_of(None, true), RewardKind::Input);
        assert_eq!(kind_of(None, false), RewardKind::Standard);
        assert!(RewardKind::Highlight.takes_text() && RewardKind::SubMessage.takes_text() && RewardKind::Input.takes_text());
        assert!(!RewardKind::Standard.takes_text() && !RewardKind::RandomEmote.takes_text());
    }

    #[test]
    fn short_costs() {
        assert_eq!(cost_short(950), "950");
        assert_eq!(cost_short(3_000), "3k");
        assert_eq!(cost_short(2_500), "2.5k");
        assert_eq!(cost_short(150_000), "150k");
        assert_eq!(cost_short(20_000_000), "20M");
        assert_eq!(cost_short(1_200_000), "1.2M");
        // The rounded value picks the unit, and a trailing .0 is dropped.
        assert_eq!(cost_short(999_999), "1M");
        assert_eq!(cost_short(999_500), "1M");
        assert_eq!(cost_short(999_499), "999k");
        assert_eq!(cost_short(1_001), "1k");
        assert_eq!(cost_short(9_999), "10k");
        assert_eq!(cost_short(99_999), "100k");
        assert_eq!(cost_short(1_050), "1.1k");
        assert_eq!(cost_short(1_999_999), "2M");
        assert_eq!(cost_short(9_999_999), "10M");
        assert_eq!(cost_short(12_400_000), "12M");
    }

    #[test]
    fn one_transaction_per_reward_and_text_until_it_succeeds() {
        let a = transaction_for("x", "r1", "hi");
        assert_eq!(a, transaction_for("x", "r1", "hi"));
        assert!((16..=64).contains(&a.len()) && a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, transaction_for("x", "r1", "hello"));
        forget_transaction("x", "r1", "hi");
        assert_ne!(a, transaction_for("x", "r1", "hi"));
    }

    #[test]
    fn words_on_the_controls_carry_no_dash() {
        let dashes = ['\u{2013}', '\u{2014}'];
        for t in [None, Some(HIGHLIGHTED_MESSAGE_REWARD), Some(SUB_MODE_BYPASS_REWARD), Some("RANDOM_SUB_EMOTE_UNLOCK")] {
            let k = kind_of(t, false);
            let failed = outcome(k, &dummy(), RedemptionResult { success: false, error_code: None, error_message: None, new_balance: None, unlocked_emote: None }, None);
            assert!(!failed.message.contains(dashes));
        }
    }

    fn dummy() -> ChannelReward {
        serde_json::from_value(serde_json::json!({
            "id": "r", "title": "Hydrate", "cost": 500, "prompt": null, "image_url": null,
            "background_color": "#9147FF", "is_enabled": true, "is_paused": false, "is_in_stock": true,
            "is_user_input_required": false, "cooldown_expires_at": null, "max_per_stream": null,
            "max_per_user_per_stream": null, "global_cooldown_seconds": null
        }))
        .unwrap()
    }

    #[test]
    fn views_and_outcomes() {
        let r = dummy();
        let v = view_of(&r);
        assert_eq!((v.kind.as_str(), v.action.as_str(), v.busy.as_str()), ("standard", "Redeem", "Redeeming..."));
        assert_eq!(v.hint.as_deref(), Some("Can't be undone"));
        assert_eq!(v.cost_label, "500");
        let ok = outcome(RewardKind::Standard, &r, RedemptionResult { success: true, error_code: None, error_message: None, new_balance: Some(10), unlocked_emote: None }, None);
        assert_eq!(ok.message, "Redeemed: Hydrate");
        let mut hl = dummy();
        hl.reward_type = Some(HIGHLIGHTED_MESSAGE_REWARD.into());
        let v = view_of(&hl);
        assert_eq!((v.action.as_str(), v.busy.as_str(), v.placeholder.as_deref(), v.hint.clone()), ("Send highlighted", "Sending...", Some("Type your message..."), None));
    }
}
