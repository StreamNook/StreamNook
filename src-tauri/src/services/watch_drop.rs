//! The drop being earned on the stream being watched, for the phones' bar in
//! the player: which reward, how far along, and whether it waits to be
//! claimed. Only a channel that can earn something is ever asked about: the
//! campaign list (cached) is checked first, and the inventory, a network
//! read, only when some campaign counts here.
//!
//! The progress is the inventory's own per-drop minutes: `get_drop_progress`
//! and the `drop-progress` events belong to the desktop automation and are
//! empty on a phone.

use crate::models::drops::{earnable_here, InventoryItem};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WatchDrop {
    pub campaign_id: String,
    pub drop_id: String,
    /// Twitch wants it alongside the drop id to claim.
    pub instance_id: Option<String>,
    /// The reward's name, else the drop's, else "Drop".
    pub name: String,
    pub image: String,
    /// "12m to watch"; empty once the minutes are in (the Claim button says it).
    pub trailing: String,
    /// The drop's state in a few words for its one-line strip, where no Claim button shows:
    /// "12m to watch", or "Ready" once the minutes are in.
    #[serde(default)]
    pub status: String,
    /// "Campaign name · 2 of 5" (the tier part only with more than one tier).
    pub line: String,
    /// 0 to 100.
    pub percent: f32,
    /// The minutes are in and it is not claimed: the bar offers the claim.
    pub complete: bool,
    /// "Claim", or "Open" for a random draw.
    pub claim_label: String,
    /// "Hoodie, 12 minutes left".
    pub spoken: String,
}

/// What the watched channel is earning, from the inventory, by the shared
/// picker (services/watch_drops.rs): a finished drop waiting on a claim first,
/// then the one with the fewest minutes left, on the campaign's own clock
/// (Twitch zeroes a claimed reward's counter) and skipping rewards already
/// owned. Campaigns count only when earnable on this channel AND in the game
/// it is streaming, or earning right now across categories; with the game not
/// known yet there is nothing to show.
pub fn pick(items: &[InventoryItem], channel_id: &str, login: &str, game_id: &str, game_name: &str) -> Option<WatchDrop> {
    // Twitch credits a campaign only in its own category, so with the game not
    // known yet nothing can honestly show. (Counting a campaign that names this
    // channel flashed another game's drop for the half second before the
    // stream's row resolved.)
    if game_id.is_empty() && game_name.is_empty() {
        return None;
    }
    let inventory = crate::models::drops::InventoryResponse {
        items: items.to_vec(),
        total_campaigns: 0,
        active_campaigns: 0,
        upcoming_campaigns: 0,
        expired_campaigns: 0,
        completed_drops: Vec::new(),
    };
    let place = crate::services::watch_drops::Place {
        channel_id: channel_id.to_string(),
        login: login.to_string(),
        game_id: game_id.to_string(),
        game_name: game_name.to_string(),
    };
    let view = crate::services::watch_drops::build(&crate::services::watch_drops::Inputs::bare(
        &place,
        &[],
        Some(&inventory),
        chrono::Utc::now().timestamp_millis(),
    ));
    if view.lead.is_none() {
        log::warn!(
            "[DropProgress] no match: game=\"{game_name}\" channel=\"{login}\" campaigns={} listed={}",
            items.len(),
            view.campaigns.len()
        );
    }
    crate::services::watch_drops::lead_for(&view)
}

/// The bar for the watched channel, or None: not connected, nothing earnable
/// here, or nothing left to earn. A failed read answers None too.
pub async fn get(
    drops_service: &tokio::sync::Mutex<crate::services::drops_service::DropsService>,
    channel_id: &str,
    login: &str,
    game_id: &str,
    game_name: &str,
) -> Option<WatchDrop> {
    // The shared model answers when it follows this very place: fresher (it
    // hears Twitch's pushes) and no read of its own. Elsewhere this reads for
    // itself as before.
    if let Some(answer) = crate::services::watch_drops::answer_for(channel_id, login, game_id, game_name).await {
        return answer;
    }
    let place = format!("{login} in \"{game_name}\" ({game_id})");
    if !crate::services::drops_auth_service::DropsAuthService::is_authenticated().await {
        decided(&place, "drops not connected");
        return None;
    }
    let service = drops_service.lock().await;
    let campaigns = match service.get_all_active_campaigns_cached().await {
        Ok(c) => c,
        Err(e) => {
            decided(&place, &format!("campaign list unreadable: {e}"));
            return None;
        }
    };
    if game_id.is_empty() && game_name.is_empty() {
        decided(&place, "game not known yet");
        return None;
    }
    if !earnable_here(&campaigns, channel_id, login, game_id, game_name) {
        decided(&place, &format!("nothing earnable here among {} campaigns", campaigns.len()));
        return None;
    }
    let inventory = match service.fetch_inventory().await {
        Ok(inv) => inv,
        Err(e) => {
            log::warn!("[DropProgress] inventory read failed: {e}");
            return None;
        }
    };
    let bar = pick(&inventory.items, channel_id, login, game_id, game_name);
    if let Some(b) = &bar {
        decided(&place, &format!("showing {} ({})", b.name, b.trailing));
    }
    bar
}

/// One release-log line each time the bar's answer for a channel changes,
/// so "the bar never showed" can be read back without a debug build.
fn decided(place: &str, outcome: &str) {
    static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let line = format!("{place}: {outcome}");
    if let Ok(mut last) = LAST.lock() {
        if *last != line {
            log::info!("[DropProgress] {line}");
            *last = line;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::drops::TimeBasedDrop;
    use crate::models::drops::{AllowedChannel, CampaignStatus, DropBenefit, DropCampaign, DropProgress};
    use chrono::Utc;

    fn drop(id: &str, need: i32, have: i32, claimed: bool) -> TimeBasedDrop {
        TimeBasedDrop {
            id: id.into(),
            name: format!("drop {id}"),
            required_minutes_watched: need,
            benefit_edges: vec![DropBenefit {
                id: format!("b{id}"),
                name: format!("Reward {id}"),
                image_url: format!("img{id}"),
                distribution_type: None,
            }],
            progress: Some(DropProgress {
                campaign_id: "c".into(),
                drop_id: id.into(),
                current_minutes_watched: have,
                required_minutes_watched: need,
                is_claimed: claimed,
                last_updated: Utc::now(),
                drop_instance_id: Some(format!("inst-{id}")),
                twitch_progress: None,
            }),
            is_collectible: need > 0,
            required_subs: 0,
            required_days: 0,
            random_of: None,
            next_reward: None,
        }
    }

    fn item(id: &str, game: &str, channels: &[&str], drops: Vec<TimeBasedDrop>) -> InventoryItem {
        InventoryItem {
            campaign: DropCampaign {
                id: id.into(),
                name: format!("Campaign {id}"),
                game_id: format!("g-{game}"),
                game_name: game.into(),
                description: String::new(),
                image_url: String::new(),
                start_at: Utc::now(),
                // Still running: an ended campaign earns nothing and is not shown.
                end_at: Utc::now() + chrono::Duration::days(1),
                time_based_drops: drops,
                is_account_connected: true,
                allowed_channels: channels.iter().map(|c| AllowedChannel { id: format!("id-{c}"), name: c.to_string() }).collect(),
                is_acl_based: false,
                details_url: None,
                account_link: None,
                separate_progress: false,
                has_category: true,
                category_ids: vec![],
            },
            status: CampaignStatus::Active,
            progress_percentage: 0.0,
            total_drops: 0,
            claimed_drops: 0,
            drops_in_progress: 0,
        }
    }

    #[test]
    fn the_tier_with_the_fewest_minutes_left_in_this_game() {
        let items = vec![
            item("1", "Rust", &[], vec![drop("a", 60, 60, true), drop("b", 120, 90, false), drop("c", 180, 90, false)]),
            item("2", "Apex", &[], vec![drop("z", 60, 59, false)]),
        ];
        let bar = pick(&items, "", "shroud", "", "rust").unwrap();
        assert_eq!(bar.drop_id, "b");
        assert_eq!(bar.trailing, "30m to watch");
        assert_eq!(bar.line, "Campaign 1 · 2 of 3");
        assert_eq!(bar.instance_id.as_deref(), Some("inst-b"));
        assert!(!bar.complete);
        assert_eq!(bar.spoken, "Reward b, 30 minutes left");
    }

    #[test]
    fn a_finished_drop_waiting_on_a_claim_comes_first() {
        let items = vec![item("1", "Rust", &[], vec![drop("b", 120, 119, false), drop("c", 60, 60, false)])];
        let bar = pick(&items, "", "x", "g-Rust", "").unwrap();
        assert_eq!((bar.drop_id.as_str(), bar.complete, bar.trailing.as_str()), ("c", true, ""));
        assert_eq!(bar.claim_label, "Claim");
    }

    #[test]
    fn a_restricted_campaign_counts_only_on_its_channels() {
        let items = vec![item("1", "Rust", &["Hjune"], vec![drop("b", 120, 30, false)])];
        assert!(pick(&items, "", "shroud", "", "Rust").is_none());
        assert!(pick(&items, "", "hjune", "", "Rust").is_some());
        // No game known: nothing, not even a campaign naming this channel.
        assert!(pick(&items, "", "hjune", "", "").is_none());
        let other_game = vec![item("2", "League of Legends", &["Hjune"], vec![drop("b", 120, 30, false)])];
        assert!(pick(&other_game, "", "hjune", "", "Rust").is_none());
    }

    #[test]
    fn nothing_left_shows_nothing() {
        let items = vec![item("1", "Rust", &[], vec![drop("a", 60, 60, true), drop("e", 0, 0, false)])];
        assert!(pick(&items, "", "x", "", "Rust").is_none());
    }

    #[test]
    fn the_golden_cases_hold_through_pick() {
        for case in crate::services::watch_drops::golden::cases().into_iter().filter(|c| c.via_pick) {
            let lead = pick(&case.inventory.items, "id-shroud", "shroud", "", "Rust");
            assert_eq!(lead.as_ref().map(|l| l.drop_id.as_str()), case.rust["lead"].as_str(), "{}", case.name);
        }
    }

    #[test]
    fn the_phones_json_keeps_its_keys() {
        let drop = WatchDrop {
            campaign_id: String::new(),
            drop_id: String::new(),
            instance_id: None,
            name: String::new(),
            image: String::new(),
            trailing: String::new(),
            status: String::new(),
            line: String::new(),
            percent: 0.0,
            complete: false,
            claim_label: String::new(),
            spoken: String::new(),
        };
        let mut keys: Vec<String> = serde_json::to_value(drop).unwrap().as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["campaign_id", "claim_label", "complete", "drop_id", "image", "instance_id", "line", "name", "percent", "spoken", "status", "trailing"]
        );
    }
}
