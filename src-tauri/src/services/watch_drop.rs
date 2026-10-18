//! The drop being earned on the stream being watched, for the phones' bar in
//! the player: which reward, how far along, and whether it waits to be
//! claimed. Only a channel that can earn something is ever asked about: the
//! campaign list (cached) is checked first, and the inventory, a network
//! read, only when some campaign counts here.
//!
//! The progress is the inventory's own per-drop minutes: `get_drop_progress`
//! and the `drop-progress` events belong to the desktop automation and are
//! empty on a phone.

use crate::models::drops::{earnable_here, InventoryItem, TimeBasedDrop};
use crate::services::drop_words;
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

/// What the watched channel is earning, from the inventory: the unclaimed
/// drop whose minutes are all in comes first (it waits on a claim), then the
/// one with the fewest minutes left. Campaigns count only when earnable on
/// this channel AND in the game it is streaming; with the game not known yet
/// there is nothing to show.
pub fn pick(items: &[InventoryItem], channel_id: &str, login: &str, game_id: &str, game_name: &str) -> Option<WatchDrop> {
    // Twitch credits a campaign only in its own category, so with the game not
    // known yet nothing can honestly show. (Counting a campaign that names this
    // channel flashed another game's drop for the half second before the
    // stream's row resolved.)
    if game_id.is_empty() && game_name.is_empty() {
        return None;
    }
    let relevant: Vec<&InventoryItem> = items
        .iter()
        .filter(|it| it.campaign.earnable_on(channel_id, login) && it.campaign.counts_in(game_id, game_name))
        .collect();
    let candidates: Vec<(&InventoryItem, &TimeBasedDrop, i32, i32)> = relevant
        .iter()
        .flat_map(|it| it.campaign.time_based_drops.iter().map(move |d| (*it, d)))
        .filter(|(_, d)| d.required_minutes_watched > 0 && !d.progress.as_ref().is_some_and(|p| p.is_claimed))
        .map(|(it, d)| {
            let need = d.required_minutes_watched;
            let have = d.progress.as_ref().map(|p| p.current_minutes_watched).unwrap_or(0).clamp(0, need);
            (it, d, have, need)
        })
        .collect();
    if candidates.is_empty() {
        let drops: usize = relevant.iter().map(|it| it.campaign.time_based_drops.len()).sum();
        log::warn!(
            "[DropProgress] no match: game=\"{game_name}\" channel=\"{login}\" campaigns={} relevant={} drops={drops}",
            items.len(),
            relevant.len()
        );
        return None;
    }
    let ready = |have: i32, need: i32, d: &TimeBasedDrop| {
        have >= need || d.progress.as_ref().and_then(|p| p.twitch_progress.as_ref()).is_some_and(|t| t.ready_to_claim)
    };
    let (item, drop, have, need) = candidates
        .iter()
        .min_by_key(|(_, d, have, need)| (!ready(*have, *need, d), need - have))
        .copied()?;
    let complete = ready(have, need, drop);
    let mut tiers: Vec<&TimeBasedDrop> =
        item.campaign.time_based_drops.iter().filter(|d| d.required_minutes_watched > 0).collect();
    tiers.sort_by_key(|d| d.required_minutes_watched);
    let tier = tiers.iter().position(|d| d.id == drop.id);
    let line = match tier {
        Some(i) if tiers.len() > 1 => format!("{} · {} of {}", item.campaign.name, i + 1, tiers.len()),
        _ => item.campaign.name.clone(),
    };
    let benefit = drop.benefit_edges.first();
    let name = benefit
        .map(|b| b.name.clone())
        .filter(|n| !n.is_empty())
        .or_else(|| Some(drop.name.clone()).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| "Drop".to_string());
    let left = need - have;
    Some(WatchDrop {
        campaign_id: item.campaign.id.clone(),
        drop_id: drop.id.clone(),
        instance_id: drop.progress.as_ref().and_then(|p| p.drop_instance_id.clone()),
        // A finished drop is said once, by its Claim button.
        trailing: if complete { String::new() } else { drop_words::to_watch(left) },
        status: if complete { "Ready".to_string() } else { drop_words::to_watch(left) },
        spoken: if complete {
            format!("{name}, ready to claim")
        } else {
            format!("{name}, {left} minute{} left", if left == 1 { "" } else { "s" })
        },
        name,
        image: benefit.map(|b| b.image_url.clone()).unwrap_or_default(),
        line,
        percent: if complete { 100.0 } else { have as f32 / need as f32 * 100.0 },
        complete,
        claim_label: drop_words::claim_label(drop).into(),
    })
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
                end_at: Utc::now(),
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
}
