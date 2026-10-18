//! The phone's Drops segment (Rewards tab on iPhone and Android): campaigns in
//! progress, running campaigns not started yet, and drops earned lately, each
//! list grouped by game, with every line the rows and the campaign sheet print
//! worked out here. The phones only draw it.
//!
//! The last inventory and campaign list are kept, so a new search re-filters
//! them without asking Twitch again; mount and pull-to-refresh re-read.

use crate::models::drops::{
    AllowedChannel, CampaignStatus, CompletedDrop, DropCampaign, InventoryItem, InventoryResponse,
    TimeBasedDrop,
};
use crate::services::drop_words;
use crate::services::game_suggest::sized_box_art;
use serde::Serialize;
use std::sync::Mutex;

/// Drops earned lately: the first this many, in Twitch's order.
const EARNED_SHOWN: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct RewardsDrops {
    /// A Drops sign-in is stored. False: the phone shows the connect card and
    /// nothing else.
    pub connected: bool,
    /// The trimmed search the lists were filtered by.
    pub query: String,
    pub in_progress: Vec<DropsGroup>,
    pub available: Vec<DropsGroup>,
    pub earned: Vec<EarnedDrop>,
    /// "No drops in progress match that search." or the plain empty line,
    /// for when `in_progress` is empty.
    pub in_progress_empty: String,
    /// `Nothing matches “rust”.` when a search left all three lists empty
    /// (the sections are not drawn then); empty otherwise.
    pub no_match: String,
}

/// One game's campaigns under a section.
#[derive(Debug, Clone, Serialize)]
pub struct DropsGroup {
    /// `progress:<game>` or `available:<game>`: what the open/closed choice is
    /// remembered by, so one game opens apart under each section.
    pub key: String,
    pub game: String,
    /// Box art at 144x192, or empty.
    pub art: String,
    /// "1 campaign", "3 campaigns".
    pub count_label: String,
    pub campaigns: Vec<CampaignCard>,
}

/// A campaign row and everything its sheet shows.
#[derive(Debug, Clone, Serialize)]
pub struct CampaignCard {
    pub id: String,
    pub name: String,
    pub game_id: String,
    pub game_name: String,
    /// The row picture for a campaign not started: the first tier's reward
    /// art, else the game's box art, else empty.
    pub art: String,
    /// In progress: "2 of 5 claimed". Empty for a campaign not started.
    pub claimed_label: String,
    /// In progress: 0 to 100.
    pub percent: f32,
    /// Not started: "3 rewards · 2d left".
    pub detail: String,
    /// The game account is not linked, so watching earns nothing yet.
    pub needs_link: bool,
    /// The publisher's account-link page, when Twitch gave one.
    pub link_url: Option<String>,
    /// Sheet meta, right side: "2/5 claimed" or "3 rewards".
    pub meta_label: String,
    /// Channels the campaign credits on; empty for one that counts on any
    /// channel in its game.
    pub channels: Vec<AllowedChannel>,
    /// "Earn on 3 channels"; empty with no list.
    pub channels_label: String,
    /// The category to find a stream in, when the campaign's game is a real one.
    pub find_stream: Option<CategoryRef>,
    /// Every time-based drop, fewest minutes first.
    pub drops: Vec<DropRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryRef {
    pub id: String,
    pub name: String,
    pub box_art_url: String,
}

/// One reward in the campaign sheet.
#[derive(Debug, Clone, Serialize)]
pub struct DropRow {
    pub id: String,
    pub name: String,
    pub image: String,
    pub claimed: bool,
    /// Claimed, or Twitch says it is ready, or the minutes are all in.
    pub done: bool,
    /// "Claimed", "Ready", "12m left", or empty for a drop with no minutes.
    pub status: String,
    /// Twitch's progress line, else the requirement; None once claimed.
    pub detail: Option<String>,
    /// Bar fill 0 to 100; None for a drop watch time does not earn.
    pub bar: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EarnedDrop {
    pub id: String,
    pub name: String,
    pub image: String,
    pub game: Option<String>,
}

/// The last inventory and campaigns read, for re-filtering on a new search.
struct Source {
    inventory: Option<InventoryResponse>,
    campaigns: Vec<DropCampaign>,
}

static LAST: Mutex<Option<Source>> = Mutex::new(None);

/// What a drops connection that is not there answers.
fn disconnected(query: &str) -> RewardsDrops {
    RewardsDrops {
        connected: false,
        query: query.to_string(),
        in_progress: Vec::new(),
        available: Vec::new(),
        earned: Vec::new(),
        in_progress_empty: String::new(),
        no_match: String::new(),
    }
}

/// The Drops segment. `refresh` reads Twitch again (mount, pull to refresh);
/// otherwise the last read is filtered by `query`, reading only when there
/// is none yet. A failed read is an empty list, never an error.
pub async fn get(
    drops_service: &tokio::sync::Mutex<crate::services::drops_service::DropsService>,
    query: &str,
    refresh: bool,
) -> RewardsDrops {
    let query = query.trim();
    if !crate::services::drops_auth_service::DropsAuthService::is_authenticated().await {
        if let Ok(mut last) = LAST.lock() {
            *last = None;
        }
        return disconnected(query);
    }
    let cached = !refresh && LAST.lock().map(|l| l.is_some()).unwrap_or(false);
    if !cached {
        let service = drops_service.lock().await;
        let (inventory, campaigns) =
            tokio::join!(service.fetch_inventory(), service.get_all_active_campaigns_cached());
        if let Err(e) = &inventory {
            log::warn!("[Rewards] inventory read failed: {e}");
        }
        if let Err(e) = &campaigns {
            log::warn!("[Rewards] campaign list read failed: {e}");
        }
        let source = Source { inventory: inventory.ok(), campaigns: campaigns.unwrap_or_default() };
        if let Ok(mut last) = LAST.lock() {
            *last = Some(source);
        }
    }
    let now = chrono::Utc::now().timestamp_millis();
    let last = LAST.lock().ok();
    match last.as_ref().and_then(|l| l.as_ref()) {
        Some(source) => build(source.inventory.as_ref(), &source.campaigns, query, now),
        None => build(None, &[], query, now),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "<m>m left" under an hour (at least 1), "<h>h left" under 48 hours,
/// otherwise "<d>d left"; None once over.
pub fn ends_in(end_ms: i64, now_ms: i64) -> Option<String> {
    let left = end_ms - now_ms;
    if left <= 0 {
        return None;
    }
    let hours = left / 3_600_000;
    Some(if hours < 1 {
        format!("{}m left", ((left as f64 / 60_000.0).round() as i64).max(1))
    } else if hours < 48 {
        format!("{hours}h left")
    } else {
        format!("{}d left", hours / 24)
    })
}

fn matches(query: &str, game: &str, name: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    game.to_lowercase().contains(&q) || name.to_lowercase().contains(&q)
}

fn game_label(campaign: &DropCampaign) -> String {
    if campaign.game_name.is_empty() {
        "Other".to_string()
    } else {
        campaign.game_name.clone()
    }
}

/// Group rows by game: busiest game first, then by name, so the list never
/// reshuffles between reads.
fn group(section: &str, rows: Vec<(String, String, CampaignCard)>) -> Vec<DropsGroup> {
    let mut groups: Vec<DropsGroup> = Vec::new();
    for (game, art, card) in rows {
        match groups.iter_mut().find(|g| g.game == game) {
            Some(g) => g.campaigns.push(card),
            None => groups.push(DropsGroup {
                key: format!("{section}:{game}"),
                game,
                art: if art.is_empty() { String::new() } else { sized_box_art(&art) },
                count_label: String::new(),
                campaigns: vec![card],
            }),
        }
    }
    groups.sort_by(|a, b| b.campaigns.len().cmp(&a.campaigns.len()).then_with(|| a.game.cmp(&b.game)));
    for g in &mut groups {
        g.count_label = plural(g.campaigns.len(), "campaign", "campaigns");
    }
    groups
}

fn drop_row(drop: &TimeBasedDrop, now_ms: i64) -> DropRow {
    let need = drop.required_minutes_watched;
    let progress = drop.progress.as_ref();
    let have = progress.map(|p| p.current_minutes_watched).unwrap_or(0).min(need).max(0);
    let claimed = progress.is_some_and(|p| p.is_claimed);
    let twitch = progress.and_then(|p| p.twitch_progress.as_ref());
    let done = claimed
        || twitch.is_some_and(|t| t.ready_to_claim)
        || (twitch.is_none() && need > 0 && have >= need);
    // What it turned into once Twitch says; an unopened draw's first reward is
    // its container.
    let benefit = twitch
        .and_then(|t| t.earned.as_ref())
        .or(drop.next_reward.as_ref())
        .or(drop.benefit_edges.first());
    let status = if claimed {
        "Claimed".to_string()
    } else if done {
        "Ready".to_string()
    } else if need > 0 {
        drop_words::to_watch(need - have)
    } else {
        String::new()
    };
    let detail = if claimed {
        None
    } else {
        drop_words::progress_line(drop, now_ms).or_else(|| drop_words::requirement_text(drop))
    };
    let bar = (need > 0).then(|| if claimed { 100.0 } else { (have as f32 / need as f32 * 100.0).min(100.0) });
    DropRow {
        id: drop.id.clone(),
        name: benefit.map(|b| b.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| drop.name.clone()),
        image: benefit.map(|b| b.image_url.clone()).unwrap_or_default(),
        claimed,
        done,
        status,
        detail,
        bar,
    }
}

fn card(campaign: &DropCampaign, item: Option<&InventoryItem>, now_ms: i64) -> CampaignCard {
    let rewards = campaign.time_based_drops.len();
    let mut drops: Vec<&TimeBasedDrop> = campaign.time_based_drops.iter().collect();
    drops.sort_by_key(|d| d.required_minutes_watched);
    let reward_art = campaign
        .time_based_drops
        .first()
        .and_then(|d| d.benefit_edges.first())
        .map(|b| b.image_url.clone())
        .filter(|u| !u.is_empty());
    let art = reward_art.unwrap_or_else(|| {
        if campaign.image_url.is_empty() {
            String::new()
        } else {
            sized_box_art(&campaign.image_url)
        }
    });
    let rewards_label = plural(rewards, "reward", "rewards");
    let detail = match ends_in(campaign.end_at.timestamp_millis(), now_ms) {
        Some(left) => format!("{rewards_label} · {left}"),
        None => rewards_label.clone(),
    };
    let channels_label = if campaign.allowed_channels.is_empty() {
        String::new()
    } else {
        format!("Earn on {}", plural(campaign.allowed_channels.len(), "channel", "channels"))
    };
    let find_stream = (!campaign.game_id.is_empty() && !campaign.game_name.is_empty() && campaign.has_category)
        .then(|| CategoryRef {
            id: campaign.game_id.clone(),
            name: campaign.game_name.clone(),
            box_art_url: campaign.image_url.clone(),
        });
    CampaignCard {
        id: campaign.id.clone(),
        name: campaign.name.clone(),
        game_id: campaign.game_id.clone(),
        game_name: campaign.game_name.clone(),
        art,
        claimed_label: item.map(|i| format!("{} of {} claimed", i.claimed_drops, i.total_drops)).unwrap_or_default(),
        percent: item.map(|i| i.progress_percentage.clamp(0.0, 100.0)).unwrap_or(0.0),
        detail,
        needs_link: !campaign.is_account_connected,
        link_url: campaign.account_link.clone().filter(|u| !u.is_empty()),
        meta_label: match item {
            Some(i) => format!("{} of {} claimed", i.claimed_drops, i.total_drops),
            None => rewards_label,
        },
        channels: campaign.allowed_channels.clone(),
        channels_label,
        find_stream,
        drops: drops.into_iter().map(|d| drop_row(d, now_ms)).collect(),
    }
}

/// The segment from an inventory and the running campaigns.
pub fn build(
    inventory: Option<&InventoryResponse>,
    campaigns: &[DropCampaign],
    query: &str,
    now_ms: i64,
) -> RewardsDrops {
    let items: &[InventoryItem] = inventory.map(|i| i.items.as_slice()).unwrap_or_default();
    // In progress: Twitch calls it active, or a drop has started.
    let in_progress: Vec<&InventoryItem> = items
        .iter()
        .filter(|i| i.status == CampaignStatus::Active || i.drops_in_progress > 0)
        .collect();
    let started: std::collections::HashSet<&str> = in_progress.iter().map(|i| i.campaign.id.as_str()).collect();
    // Running and not started, soonest to end first.
    let mut available: Vec<&DropCampaign> =
        campaigns.iter().filter(|c| !started.contains(c.id.as_str())).collect();
    available.sort_by_key(|c| c.end_at);

    let progress_rows = in_progress
        .iter()
        .filter(|i| matches(query, &i.campaign.game_name, &i.campaign.name))
        .map(|i| (game_label(&i.campaign), i.campaign.image_url.clone(), card(&i.campaign, Some(i), now_ms)))
        .collect();
    let available_rows = available
        .iter()
        .filter(|c| matches(query, &c.game_name, &c.name))
        .map(|c| (game_label(c), c.image_url.clone(), card(c, None, now_ms)))
        .collect();
    let completed: &[CompletedDrop] = inventory.map(|i| i.completed_drops.as_slice()).unwrap_or_default();
    // A search narrows what was earned too, so nothing unrelated reads as a match.
    let earned: Vec<EarnedDrop> = completed
        .iter()
        .filter(|d| matches(query, d.game_name.as_deref().unwrap_or(""), &d.name))
        .take(EARNED_SHOWN)
        .map(|d| EarnedDrop {
            id: d.id.clone(),
            name: d.name.clone(),
            image: d.image_url.clone(),
            game: d.game_name.clone().filter(|g| !g.is_empty()),
        })
        .collect();
    let in_progress = group("progress", progress_rows);
    let available = group("available", available_rows);
    let nothing = in_progress.is_empty() && available.is_empty() && earned.is_empty();
    RewardsDrops {
        connected: true,
        query: query.to_string(),
        no_match: if !query.is_empty() && nothing { format!("Nothing matches \u{201C}{query}\u{201D}.") } else { String::new() },
        in_progress,
        available,
        earned,
        in_progress_empty: if query.is_empty() {
            "No drop campaigns in progress. Watch a drops-enabled stream to start earning.".to_string()
        } else {
            "No drops in progress match that search.".to_string()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::drops::{DropBenefit, DropProgress};
    use chrono::{TimeZone, Utc};

    fn drop(id: &str, req: i32, cur: Option<i32>, claimed: bool) -> TimeBasedDrop {
        TimeBasedDrop {
            id: id.into(),
            name: format!("drop {id}"),
            required_minutes_watched: req,
            benefit_edges: vec![DropBenefit {
                id: format!("b-{id}"),
                name: format!("reward {id}"),
                image_url: format!("img-{id}"),
                distribution_type: None,
            }],
            progress: cur.map(|c| DropProgress {
                campaign_id: "c".into(),
                drop_id: id.into(),
                current_minutes_watched: c,
                required_minutes_watched: req,
                is_claimed: claimed,
                last_updated: Utc::now(),
                drop_instance_id: None,
                twitch_progress: None,
            }),
            is_collectible: req > 0,
            required_subs: 0,
            required_days: 0,
            random_of: None,
            next_reward: None,
        }
    }

    fn campaign(id: &str, game: &str, end_day: u32, drops: Vec<TimeBasedDrop>) -> DropCampaign {
        DropCampaign {
            id: id.into(),
            name: format!("Campaign {id}"),
            game_id: if game.is_empty() { String::new() } else { format!("g-{game}") },
            game_name: game.into(),
            description: String::new(),
            image_url: format!("https://x/ttv-boxart/{game}-{{width}}x{{height}}.jpg"),
            start_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
            end_at: Utc.with_ymd_and_hms(2026, 10, end_day, 0, 0, 0).unwrap(),
            time_based_drops: drops,
            is_account_connected: true,
            allowed_channels: vec![],
            is_acl_based: false,
            details_url: None,
            account_link: None,
            separate_progress: false,
            has_category: true,
            category_ids: vec![],
        }
    }

    fn item(c: DropCampaign, status: CampaignStatus, in_progress: i32) -> InventoryItem {
        InventoryItem {
            total_drops: c.time_based_drops.len() as i32,
            campaign: c,
            status,
            progress_percentage: 40.0,
            claimed_drops: 1,
            drops_in_progress: in_progress,
        }
    }

    fn inv(items: Vec<InventoryItem>, completed: usize) -> InventoryResponse {
        InventoryResponse {
            items,
            total_campaigns: 0,
            active_campaigns: 0,
            upcoming_campaigns: 0,
            expired_campaigns: 0,
            completed_drops: (0..completed)
                .map(|i| CompletedDrop {
                    id: format!("e{i}"),
                    name: format!("earned {i}"),
                    image_url: String::new(),
                    game_name: Some(String::new()),
                    is_connected: true,
                    required_account_link: None,
                    last_awarded_at: Utc::now(),
                    total_count: 1,
                })
                .collect(),
        }
    }

    fn now() -> i64 {
        Utc.with_ymd_and_hms(2026, 10, 16, 0, 0, 0).unwrap().timestamp_millis()
    }

    #[test]
    fn sections_split_dedupe_and_order() {
        let started = campaign("1", "Rust", 20, vec![drop("a", 60, Some(30), false)]);
        let inventory = inv(
            vec![
                item(started.clone(), CampaignStatus::Active, 1),
                item(campaign("9", "Old", 20, vec![]), CampaignStatus::Expired, 0),
            ],
            12,
        );
        let campaigns = vec![
            started,
            campaign("2", "Valorant", 25, vec![]),
            campaign("3", "Valorant", 18, vec![]),
            campaign("4", "", 19, vec![]),
            campaign("5", "Apex", 30, vec![]),
        ];
        let view = build(Some(&inventory), &campaigns, "", now());
        assert_eq!(view.in_progress.len(), 1);
        assert_eq!(view.in_progress[0].key, "progress:Rust");
        assert_eq!(view.in_progress[0].campaigns[0].claimed_label, "1 of 1 claimed");
        // Started campaign 1 is not listed again; Valorant has two so it leads,
        // then Apex and Other alphabetically.
        let games: Vec<&str> = view.available.iter().map(|g| g.game.as_str()).collect();
        assert_eq!(games, ["Valorant", "Apex", "Other"]);
        // Inside a game, soonest-ending first.
        let ids: Vec<&str> = view.available[0].campaigns.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["3", "2"]);
        assert_eq!(view.available[0].count_label, "2 campaigns");
        assert_eq!(view.available[0].art, "https://x/ttv-boxart/Valorant-144x192.jpg");
        assert_eq!(view.earned.len(), 10);
        assert_eq!(view.earned[0].game, None);
    }

    #[test]
    fn search_matches_game_or_campaign_name() {
        let campaigns = vec![campaign("1", "Rust", 20, vec![]), campaign("2", "Apex", 20, vec![])];
        let view = build(None, &campaigns, "rUs", now());
        assert_eq!(view.available.len(), 1);
        let view = build(None, &campaigns, "campaign 2", now());
        assert_eq!(view.available[0].game, "Apex");
        assert_eq!(view.in_progress_empty, "No drops in progress match that search.");
        assert_eq!(view.no_match, "");
        assert_eq!(build(None, &campaigns, "zzz", now()).no_match, "Nothing matches “zzz”.");
    }

    #[test]
    fn time_left_words() {
        let n = now();
        assert_eq!(ends_in(n + 20_000, n).unwrap(), "1m left");
        assert_eq!(ends_in(n + 30 * 60_000, n).unwrap(), "30m left");
        assert_eq!(ends_in(n + 47 * 3_600_000, n).unwrap(), "47h left");
        assert_eq!(ends_in(n + 72 * 3_600_000, n).unwrap(), "3d left");
        assert_eq!(ends_in(n - 1, n), None);
    }

    #[test]
    fn card_rows_and_sheet_lines() {
        let mut c = campaign(
            "1",
            "Rust",
            20,
            vec![drop("late", 120, Some(130), false), drop("first", 60, Some(20), false), drop("got", 30, Some(30), true), drop("event", 0, None, false)],
        );
        c.allowed_channels = vec![AllowedChannel { id: "1".into(), name: "Shroud".into() }];
        c.is_account_connected = false;
        c.account_link = Some("https://link".into());
        let card = card(&c, None, now());
        assert_eq!(card.detail, "4 rewards · 4d left");
        assert_eq!(card.meta_label, "4 rewards");
        assert_eq!(card.channels_label, "Earn on 1 channel");
        assert!(card.needs_link);
        assert_eq!(card.art, "img-late");
        let ids: Vec<&str> = card.drops.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["event", "got", "first", "late"]);
        assert_eq!((card.drops[0].status.as_str(), card.drops[0].bar), ("", None));
        assert_eq!((card.drops[1].status.as_str(), card.drops[1].bar, card.drops[1].detail.clone()), ("Claimed", Some(100.0), None));
        assert_eq!(card.drops[2].status, "40m to watch");
        assert_eq!(card.drops[3].status, "Ready");
        assert_eq!(card.drops[3].bar, Some(100.0));
        assert_eq!(card.drops[2].name, "reward first");
    }

    #[test]
    fn a_campaign_with_no_category_offers_no_stream_search() {
        let mut c = campaign("1", "Rust", 20, vec![]);
        c.has_category = false;
        assert!(card(&c, None, now()).find_stream.is_none());
    }
}
