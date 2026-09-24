//! The Drops page's model: every active campaign joined with live progress and
//! the account's inventory, grouped by game, with what is claimable, in
//! progress, or already owned worked out once.
//!
//! This used to be built inside the React component, with the ownership rule
//! written twice (load and claim) and a nested progress lookup per drop. The
//! component now asks for the model and renders it; the one display ordering
//! that depends on live UI state (sort mode, favourites pin, the game being
//! collected right now) stays with the view.

use crate::commands::badges::{get_cached_global_badges, prefetch_global_badges};
use crate::models::drops::{
    CompletedDrop, DropCampaign, DropProgress, DropsStatistics, InventoryItem, InventoryResponse,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Earned badge titles need a GQL round trip; they change when a badge is
/// earned, which the Drops page is not where you watch for.
const KNOWN_TITLES_TTL: Duration = Duration::from_secs(10 * 60);

static BADGE_TITLES: Mutex<Option<(Instant, BadgeTitles)>> = Mutex::new(None);

/// Badge titles, lowercased: the ones this account has earned, and those plus
/// Twitch's global catalog.
#[derive(Debug, Clone, Default)]
pub struct BadgeTitles {
    pub earned: HashSet<String>,
    pub known: HashSet<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DropGame {
    pub id: String,
    pub name: String,
    pub box_art_url: String,
    pub active_campaigns: Vec<DropCampaign>,
    pub total_active_drops: usize,
    pub drops_in_progress: usize,
    pub inventory_items: Vec<InventoryItem>,
    pub total_claimed: i32,
    /// Whether this game is being collected right now: decided by the view,
    /// which holds the live automation status.
    pub active: bool,
    pub has_claimable: bool,
    pub all_drops_claimed: bool,
    /// The newest campaign start among the active campaigns (epoch ms), the
    /// "newest / oldest" sort key.
    pub release_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DropsOverview {
    pub games: Vec<DropGame>,
    pub inventory_items: Vec<InventoryItem>,
    pub completed_drops: Vec<CompletedDrop>,
    pub statistics: Option<DropsStatistics>,
    pub progress: Vec<DropProgress>,
    /// Badge titles this account has earned, lowercased.
    pub earned_badge_titles: Vec<String>,
    /// Earned plus globally catalogued badge titles, lowercased: how the reward
    /// list tells a badge from an in-game item when both look alike.
    pub known_badge_titles: Vec<String>,
}

fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

/// What the account already owns, from unambiguous sources only: the permanent
/// earned-drops list and any inventory drop explicitly flagged claimed. NOT
/// "claimed by index" or "100% watched", which over-matched in-progress drops.
struct Owned {
    drop_ids: HashSet<String>,
    benefit_ids: HashSet<String>,
    benefit_names: HashSet<String>,
}

impl Owned {
    fn from(inventory: Option<&InventoryResponse>) -> Self {
        let mut owned = Owned {
            drop_ids: HashSet::new(),
            benefit_ids: HashSet::new(),
            benefit_names: HashSet::new(),
        };
        let Some(inv) = inventory else { return owned };
        for d in &inv.completed_drops {
            owned.benefit_ids.insert(d.id.clone());
            let n = normalize(&d.name);
            if !n.is_empty() {
                owned.benefit_names.insert(n);
            }
        }
        for item in &inv.items {
            for drop in &item.campaign.time_based_drops {
                if drop.progress.as_ref().is_some_and(|p| p.is_claimed) {
                    owned.drop_ids.insert(drop.id.clone());
                    for b in &drop.benefit_edges {
                        owned.benefit_ids.insert(b.id.clone());
                        let n = normalize(&b.name);
                        if !n.is_empty() {
                            owned.benefit_names.insert(n);
                        }
                    }
                }
            }
        }
        owned
    }
}

/// Join campaigns, live progress and inventory into per-game rows.
///
/// `progress` is the live map (real-time); a drop missing from it falls back to
/// the matching inventory campaign's progress, then to what the campaign itself
/// carried.
pub fn build(
    campaigns: &[DropCampaign],
    progress: &[DropProgress],
    inventory: Option<&InventoryResponse>,
    known_badge_titles: &HashSet<String>,
) -> Vec<DropGame> {
    let live: HashMap<&str, &DropProgress> =
        progress.iter().map(|p| (p.drop_id.as_str(), p)).collect();
    let mut order: Vec<String> = Vec::new();
    let mut games: HashMap<String, DropGame> = HashMap::new();
    let mut game_for = |id: &str, name: &str, art: &str| -> String {
        if !games.contains_key(id) {
            order.push(id.to_string());
            games.insert(
                id.to_string(),
                DropGame {
                    id: id.to_string(),
                    name: name.to_string(),
                    box_art_url: art.to_string(),
                    active_campaigns: Vec::new(),
                    total_active_drops: 0,
                    drops_in_progress: 0,
                    inventory_items: Vec::new(),
                    total_claimed: 0,
                    active: false,
                    has_claimable: false,
                    all_drops_claimed: false,
                    release_ms: 0,
                },
            );
        }
        id.to_string()
    };
    let mut entries: Vec<(String, DropCampaign)> = Vec::new();
    let mut inventory_rows: Vec<(String, InventoryItem)> = Vec::new();

    for campaign in campaigns {
        let inventory_campaign = inventory.and_then(|inv| {
            inv.items
                .iter()
                .find(|it| it.campaign.id == campaign.id || it.campaign.name == campaign.name)
        });
        let mut merged = campaign.clone();
        for drop in &mut merged.time_based_drops {
            let from_inventory = || {
                inventory_campaign
                    .and_then(|it| it.campaign.time_based_drops.iter().find(|d| d.id == drop.id))
                    .and_then(|d| d.progress.clone())
            };
            let chosen = live.get(drop.id.as_str()).map(|p| (*p).clone()).or_else(from_inventory);
            if chosen.is_some() {
                drop.progress = chosen;
            }
        }
        let key = game_for(&campaign.game_id, &campaign.game_name, &campaign.image_url);
        entries.push((key, merged));
    }
    if let Some(inv) = inventory {
        for item in &inv.items {
            let name = if item.campaign.game_name.is_empty() {
                "Unknown Game"
            } else {
                item.campaign.game_name.as_str()
            };
            let id = if item.campaign.game_id.is_empty() {
                format!("generated-{}", name.to_lowercase().split_whitespace().collect::<Vec<_>>().join("-"))
            } else {
                item.campaign.game_id.clone()
            };
            let key = game_for(&id, name, &item.campaign.image_url);
            inventory_rows.push((key, item.clone()));
        }
    }

    let owned = Owned::from(inventory);
    let owned_by_name = |name: &str| {
        let n = normalize(name);
        !n.is_empty() && owned.benefit_names.contains(&n) && known_badge_titles.contains(&n)
    };

    for (key, campaign) in entries {
        let game = games.get_mut(&key).expect("created above");
        game.total_active_drops += campaign.time_based_drops.len();
        game.release_ms = game.release_ms.max(campaign.start_at.timestamp_millis());
        for drop in &campaign.time_based_drops {
            if let Some(p) = &drop.progress {
                if !p.is_claimed && p.current_minutes_watched > 0 {
                    game.drops_in_progress += 1;
                }
            }
        }
        game.active_campaigns.push(campaign);
    }
    for (key, item) in inventory_rows {
        let game = games.get_mut(&key).expect("created above");
        game.total_claimed += item.claimed_drops;
        game.inventory_items.push(item);
    }

    for game in games.values_mut() {
        let mut total = 0;
        let mut owned_count = 0;
        for drop in game.active_campaigns.iter().flat_map(|c| &c.time_based_drops) {
            total += 1;
            let p = drop.progress.as_ref();
            let watching = p.is_some_and(|p| p.current_minutes_watched > 0 || p.is_claimed);
            // A drop flagged claimed is a proven claim of THIS instance (reissues
            // mint new drop ids), so it counts even with stale watch minutes.
            // Benefit id/name matching only counts when the drop is not itself
            // being collected; a held BADGE (by name) cannot be re-earned, but a
            // consumable reissued under a new campaign can.
            let is_owned = p.is_some_and(|p| p.is_claimed)
                || owned.drop_ids.contains(&drop.id)
                || (!watching
                    && drop
                        .benefit_edges
                        .iter()
                        .any(|b| owned.benefit_ids.contains(&b.id) || owned_by_name(&b.name)));
            if is_owned {
                owned_count += 1;
            } else if p.is_some_and(|p| {
                p.required_minutes_watched > 0 && p.current_minutes_watched >= p.required_minutes_watched
            }) {
                game.has_claimable = true;
            }
        }
        game.all_drops_claimed = total > 0 && owned_count == total;
    }

    // Only games with a running campaign are actionable here; games known only
    // from earned inventory live in the Inventory tab.
    order
        .into_iter()
        .filter_map(|id| games.remove(&id))
        .filter(|g| !g.active_campaigns.is_empty())
        .collect()
}

/// Earned badge titles for the signed-in account, and those plus Twitch's
/// global badge catalog. Cached; an empty answer on failure is not cached.
pub async fn badge_titles() -> BadgeTitles {
    if let Ok(slot) = BADGE_TITLES.lock() {
        if let Some((at, titles)) = slot.as_ref() {
            if at.elapsed() < KNOWN_TITLES_TTL {
                return titles.clone();
            }
        }
    }
    let mut earned = HashSet::new();
    if let Ok(me) = crate::services::twitch_service::TwitchService::get_user_info().await {
        if let Ok(badges) = crate::commands::badge_service::get_user_badges_with_earned_unified(
            me.id.clone(),
            me.login.clone(),
            me.id,
            me.login,
        )
        .await
        {
            for b in badges.earned_badges.iter().chain(&badges.third_party_badges) {
                let t = normalize(&b.badge_info.title);
                if !t.is_empty() {
                    earned.insert(t);
                }
            }
        }
    }
    let mut global = get_cached_global_badges().await.ok().flatten();
    if global.is_none() {
        let _ = prefetch_global_badges().await;
        global = get_cached_global_badges().await.ok().flatten();
    }
    let mut known = earned.clone();
    for set in global.iter().flat_map(|g| &g.data) {
        for v in &set.versions {
            let t = normalize(&v.title);
            if !t.is_empty() {
                known.insert(t);
            }
        }
    }
    let titles = BadgeTitles { earned, known };
    if !titles.known.is_empty() {
        if let Ok(mut slot) = BADGE_TITLES.lock() {
            *slot = Some((Instant::now(), titles.clone()));
        }
    }
    titles
}

// ---- New campaigns in favourite games --------------------------------------

/// One favourite game that gained campaigns since the last look.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NewFavoriteDrops {
    pub game_name: String,
    pub game_image: String,
    pub new_count: usize,
    pub campaign_names: Vec<String>,
}

type SeenCampaigns = HashMap<String, Vec<String>>;

/// Compare favourite games' campaigns with what was seen last time. Returns the
/// games with new campaigns and the record to keep. `seen` of None means there
/// is no record yet, so this look only seeds it: without that, the first run
/// would announce every running campaign as new.
pub fn diff_favorite_campaigns(
    games: &[DropGame],
    favorites: &[String],
    seen: Option<&SeenCampaigns>,
) -> (Vec<NewFavoriteDrops>, SeenCampaigns) {
    let favorites: HashSet<String> = favorites.iter().map(|f| f.to_lowercase()).collect();
    let mut next = SeenCampaigns::new();
    let mut found = Vec::new();
    for game in games {
        let key = game.name.to_lowercase();
        if !favorites.contains(&key) {
            continue;
        }
        let ids: Vec<String> = game.active_campaigns.iter().map(|c| c.id.clone()).collect();
        if let Some(seen) = seen {
            let before = seen.get(&key).map(Vec::as_slice).unwrap_or_default();
            let new: Vec<&DropCampaign> =
                game.active_campaigns.iter().filter(|c| !before.contains(&c.id)).collect();
            if !new.is_empty() {
                found.push(NewFavoriteDrops {
                    game_name: game.name.clone(),
                    game_image: game.box_art_url.clone(),
                    new_count: new.len(),
                    campaign_names: new.iter().map(|c| c.name.clone()).collect(),
                });
            }
        }
        next.insert(key, ids);
    }
    (found, next)
}

fn seen_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("favorite_drop_campaigns.json"))
}

/// Announce new campaigns in favourite games (`new-favorite-drops`, one event
/// per game), and remember what was seen.
pub fn announce_new_favorite_campaigns(
    app: &tauri::AppHandle,
    games: &[DropGame],
    favorites: &[String],
    enabled: bool,
) {
    use tauri::Emitter;
    let Some(path) = seen_path(app) else { return };
    let seen: Option<SeenCampaigns> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let (found, next) = diff_favorite_campaigns(games, favorites, seen.as_ref());
    if enabled {
        for game in &found {
            let _ = app.emit("new-favorite-drops", game);
        }
    }
    if let Ok(json) = serde_json::to_string(&next) {
        let _ = std::fs::write(path, json);
    }
}

pub fn overview(
    campaigns: &[DropCampaign],
    progress: Vec<DropProgress>,
    inventory: Option<InventoryResponse>,
    statistics: Option<DropsStatistics>,
    titles: BadgeTitles,
) -> DropsOverview {
    let games = build(campaigns, &progress, inventory.as_ref(), &titles.known);
    let (inventory_items, completed_drops) = inventory
        .map(|inv| (inv.items, inv.completed_drops))
        .unwrap_or_default();
    let sorted = |set: HashSet<String>| {
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    };
    DropsOverview {
        games,
        inventory_items,
        completed_drops,
        statistics,
        progress,
        earned_badge_titles: sorted(titles.earned),
        known_badge_titles: sorted(titles.known),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::drops::{CampaignStatus, DropBenefit, TimeBasedDrop};
    use chrono::{TimeZone, Utc};

    fn progress(drop_id: &str, cur: i32, req: i32, claimed: bool) -> DropProgress {
        DropProgress {
            campaign_id: "c".into(),
            drop_id: drop_id.into(),
            current_minutes_watched: cur,
            required_minutes_watched: req,
            is_claimed: claimed,
            last_updated: Utc::now(),
            drop_instance_id: None,
        }
    }

    fn drop(id: &str, req: i32, benefit: &str) -> TimeBasedDrop {
        TimeBasedDrop {
            id: id.into(),
            name: id.into(),
            required_minutes_watched: req,
            benefit_edges: vec![DropBenefit {
                id: format!("b-{benefit}"),
                name: benefit.into(),
                image_url: String::new(),
                distribution_type: None,
            }],
            progress: None,
            is_collectible: req > 0,
        }
    }

    fn campaign(id: &str, game: &str, start_day: u32, drops: Vec<TimeBasedDrop>) -> DropCampaign {
        DropCampaign {
            id: id.into(),
            name: format!("campaign {id}"),
            game_id: format!("g-{game}"),
            game_name: game.into(),
            description: String::new(),
            image_url: format!("art-{game}"),
            start_at: Utc.with_ymd_and_hms(2026, 9, start_day, 0, 0, 0).unwrap(),
            end_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
            time_based_drops: drops,
            is_account_connected: true,
            allowed_channels: vec![],
            is_acl_based: false,
            details_url: None,
            account_link: None,
        }
    }

    fn inventory(items: Vec<InventoryItem>, completed: Vec<(&str, &str)>) -> InventoryResponse {
        InventoryResponse {
            items,
            total_campaigns: 0,
            active_campaigns: 0,
            upcoming_campaigns: 0,
            expired_campaigns: 0,
            completed_drops: completed
                .into_iter()
                .map(|(id, name)| CompletedDrop {
                    id: id.into(),
                    name: name.into(),
                    image_url: String::new(),
                    game_name: None,
                    is_connected: true,
                    required_account_link: None,
                    last_awarded_at: Utc::now(),
                    total_count: 1,
                })
                .collect(),
        }
    }

    fn item(c: DropCampaign, claimed: i32) -> InventoryItem {
        InventoryItem {
            campaign: c,
            status: CampaignStatus::Active,
            progress_percentage: 0.0,
            total_drops: 0,
            claimed_drops: claimed,
            drops_in_progress: 0,
        }
    }

    #[test]
    fn groups_campaigns_by_game_and_merges_live_progress() {
        let campaigns = vec![
            campaign("1", "Rust", 1, vec![drop("d1", 60, "hat"), drop("d2", 120, "cape")]),
            campaign("2", "Rust", 5, vec![drop("d3", 60, "boots")]),
            campaign("3", "Other", 2, vec![drop("d4", 30, "skin")]),
        ];
        let live = vec![progress("d1", 60, 60, false), progress("d2", 10, 120, false)];
        let games = build(&campaigns, &live, None, &HashSet::new());
        assert_eq!(games.len(), 2);
        let rust = &games[0];
        assert_eq!((rust.name.as_str(), rust.total_active_drops), ("Rust", 3));
        assert_eq!(rust.drops_in_progress, 2);
        assert!(rust.has_claimable);
        assert!(!rust.all_drops_claimed);
        assert_eq!(rust.release_ms, Utc.with_ymd_and_hms(2026, 9, 5, 0, 0, 0).unwrap().timestamp_millis());
        assert_eq!(rust.active_campaigns[0].time_based_drops[1].progress.as_ref().unwrap().current_minutes_watched, 10);
    }

    #[test]
    fn a_zero_minute_drop_is_never_claimable_by_watching() {
        let campaigns = vec![campaign("1", "Rust", 1, vec![drop("d1", 0, "event")])];
        let games = build(&campaigns, &[progress("d1", 0, 0, false)], None, &HashSet::new());
        assert!(!games[0].has_claimable);
    }

    #[test]
    fn inventory_progress_fills_a_drop_the_live_map_lacks() {
        let mut inv_campaign = campaign("1", "Rust", 1, vec![drop("d1", 60, "hat")]);
        inv_campaign.time_based_drops[0].progress = Some(progress("d1", 30, 60, false));
        let inv = inventory(vec![item(inv_campaign, 0)], vec![]);
        let games = build(&[campaign("1", "Rust", 1, vec![drop("d1", 60, "hat")])], &[], Some(&inv), &HashSet::new());
        assert_eq!(games[0].drops_in_progress, 1);
    }

    #[test]
    fn ownership_counts_claims_benefit_ids_and_badge_names() {
        let campaigns = vec![campaign(
            "1",
            "Rust",
            1,
            vec![drop("d1", 60, "hat"), drop("d2", 60, "Cool Badge"), drop("d3", 60, "potion")],
        )];
        let inv = inventory(vec![], vec![("b-hat", "hat"), ("other-id", "cool badge"), ("x", "potion")]);
        let known = HashSet::from(["cool badge".to_string()]);
        let games = build(&campaigns, &[], Some(&inv), &known);
        // hat by benefit id, the badge by name (it is a known badge), and the
        // potion by id only; a consumable is not owned just by sharing a name.
        assert!(!games[0].all_drops_claimed);
        let with_potion = inventory(vec![], vec![("b-hat", "hat"), ("other-id", "cool badge"), ("b-potion", "potion")]);
        assert!(build(&campaigns, &[], Some(&with_potion), &known)[0].all_drops_claimed);
    }

    #[test]
    fn a_drop_being_collected_is_not_owned_by_benefit() {
        let campaigns = vec![campaign("1", "Rust", 1, vec![drop("d1", 60, "hat")])];
        let inv = inventory(vec![], vec![("b-hat", "hat")]);
        let games = build(&campaigns, &[progress("d1", 20, 60, false)], Some(&inv), &HashSet::new());
        assert!(!games[0].all_drops_claimed);
        let claimed = build(&campaigns, &[progress("d1", 20, 60, true)], Some(&inv), &HashSet::new());
        assert!(claimed[0].all_drops_claimed);
    }

    #[test]
    fn inventory_only_games_are_left_to_the_inventory_tab() {
        let inv = inventory(vec![item(campaign("9", "Old", 1, vec![]), 3)], vec![]);
        let games = build(&[campaign("1", "Rust", 1, vec![drop("d1", 60, "hat")])], &[], Some(&inv), &HashSet::new());
        assert_eq!(games.len(), 1);
        let same_game_inv = inventory(vec![item(campaign("8", "Rust", 1, vec![]), 2)], vec![]);
        let games = build(&[campaign("1", "Rust", 1, vec![drop("d1", 60, "hat")])], &[], Some(&same_game_inv), &HashSet::new());
        assert_eq!(games[0].total_claimed, 2);
    }

    #[test]
    fn favorite_diff_seeds_silently_then_reports_only_new_campaigns() {
        let games = build(
            &[campaign("1", "Rust", 1, vec![]), campaign("2", "Other", 1, vec![])],
            &[],
            None,
            &HashSet::new(),
        );
        // The empty-drop campaigns keep the games listed.
        assert_eq!(games.len(), 2);
        let favorites = vec!["rust".to_string()];
        let (first, seen) = diff_favorite_campaigns(&games, &favorites, None);
        assert!(first.is_empty());
        assert_eq!(seen.get("rust").unwrap(), &vec!["1".to_string()]);

        let more = build(
            &[campaign("1", "Rust", 1, vec![]), campaign("3", "Rust", 2, vec![])],
            &[],
            None,
            &HashSet::new(),
        );
        let (found, _) = diff_favorite_campaigns(&more, &favorites, Some(&seen));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].new_count, 1);
        assert_eq!(found[0].campaign_names, vec!["campaign 3".to_string()]);
    }
}
