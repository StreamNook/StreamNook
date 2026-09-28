//! Drops that Twitch lists outside its standard campaign list: the ones built
//! on containers and daily watch progress (the Pokémon chat badges, for one),
//! and a growing number of ordinary game drops. Twitch serves them from
//! `rewardCampaignsAvailableToUser` and `dropsCampaign(id:)`; this module reads
//! them and translates them into the ordinary drop model, so every drops
//! surface shows them like any other drop.
//!
//! Facts the translation rests on, read from the live API:
//! - A reward group is one reward tier. `WATCH` groups need minutes, repeated
//!   on `repeatableTimes` separate rolling 24-hour windows; `SUB` groups need
//!   subscriptions.
//! - A group's rewards can be containers holding a pool; the pool is what the
//!   viewer ends up with, drawn at random when it holds more than one.
//! - `self.status` is `IN_PROGRESS`, `CLAIMABLE` (earned, waiting to be
//!   claimed) or `CLAIMED`. `earnedReward` is set on a claimable container
//!   too, so only the status says whether it is claimed.
//! - A claim is the ordinary `claimDropRewards` call with the drop instance
//!   `userID#campaignID#rewardGroupID` (what twitch.tv's own chat panel sends).
//! - The viewer's fullest progress is under
//!   `currentUser.inventory.viewerRewardDropCampaignsInProgress`.

use crate::models::drops::{DropBenefit, DropCampaign, DropProgress, TimeBasedDrop, TwitchProgress};
use crate::services::drops_service::DropsService;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use log::debug;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

const GQL_URL: &str = "https://gql.twitch.tv/gql";
/// A fetch this recent is reused rather than repeated. Matches the watch
/// loop's refresh, which is what keeps progress live while a stream plays.
const CACHE_SECS: u64 = 110;
/// The inventory reuses a fetch this recent instead of asking again.
pub const REUSE_FOR_INVENTORY_SECS: u64 = 300;
/// Minutes that rose within this long mean the drop is being earned now.
const ACCRUING_MINUTES: i64 = 5;
/// Prefix of the display id a campaign with no game is grouped under. It can
/// never collide with a Twitch category id, which is numeric.
pub const NO_CATEGORY_PREFIX: &str = "twitch-rewards-";

const LIST_QUERY: &str = "query { rewardCampaignsAvailableToUser { id name brand summary startsAt endsAt aboutURL game { id displayName boxArtURL } } }";

const SELF_FIELDS: &str = "self { status currentMinutesWatched currentSubs grantCount currentWindow { startedAt expiresAt } earnedReward { id name thumbnailURL } }";

static CACHE: Lazy<Mutex<Option<(Instant, Vec<DropCampaign>)>>> = Lazy::new(|| Mutex::new(None));
static TRACKER: Lazy<Mutex<HashMap<String, Rise>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// The last minutes seen for a reward tier and when they last went up.
#[derive(Clone, Debug, Default)]
pub struct Rise {
    last: i32,
    rose_at: Option<DateTime<Utc>>,
}

/// How a campaign is grouped on the drops page: under its game, or, for one
/// that runs across categories, under its owner, brand or its own name.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayGroup {
    pub game_id: String,
    pub game_name: String,
    pub image_url: String,
    pub has_category: bool,
}

fn slug(label: &str) -> String {
    let mut out = String::new();
    for ch in label.to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// Grouping for a standard-list campaign JSON (`game`, `owner`, `name`).
/// `None` only when there is nothing at all to label it with.
pub fn display_group(campaign: &Value) -> Option<DisplayGroup> {
    let game = &campaign["game"];
    if let (Some(id), Some(name)) = (
        text(game, "id"),
        text(game, "displayName").or_else(|| text(game, "name")),
    ) {
        return Some(DisplayGroup {
            game_id: id.to_string(),
            game_name: name.to_string(),
            image_url: text(game, "boxArtURL").unwrap_or_default().to_string(),
            has_category: true,
        });
    }
    let label = text(&campaign["owner"], "name").or_else(|| text(campaign, "name"))?;
    Some(no_category_group(label))
}

fn no_category_group(label: &str) -> DisplayGroup {
    DisplayGroup {
        game_id: format!("{NO_CATEGORY_PREFIX}{}", slug(label)),
        game_name: label.to_string(),
        image_url: String::new(),
        has_category: false,
    }
}

/// The first reward image of a campaign's drops, for a campaign with no box art.
pub fn first_reward_image(drops: &[TimeBasedDrop]) -> String {
    drops
        .iter()
        .flat_map(|d| &d.benefit_edges)
        .map(|b| b.image_url.as_str())
        .find(|u| !u.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn parse_time(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn uint(v: &Value) -> u32 {
    v.as_u64().unwrap_or(0) as u32
}

/// Who is asking and what they already hold, for claims and ownership.
#[derive(Default)]
pub struct Viewer<'a> {
    /// The account's Twitch user id: a claim names `user#campaign#rewardGroup`.
    pub user_id: Option<&'a str>,
    /// Reward ids the account has claimed (`earnedDropRewards`), so a pool
    /// whose every reward is already held reads as done: Twitch never repeats one.
    pub claimed_items: HashSet<String>,
}

/// One reward tier as a drop, with Twitch's own progress.
fn group_drop(
    campaign_id: &str,
    group: &Value,
    inventory_self: Option<&Value>,
    viewer: &Viewer,
    tracker: &mut HashMap<String, Rise>,
    now: DateTime<Utc>,
) -> TimeBasedDrop {
    let id = group["id"].as_str().unwrap_or_default().to_string();
    let criteria = &group["progressCriteria"];
    let per_day = uint(&criteria["requirements"]["minutesWatched"]);
    let subs = uint(&criteria["requirements"]["subs"]);
    let repeats = uint(&criteria["repeatableConfig"]["repeatableTimes"]).max(1);
    let is_sub = criteria["requirementType"].as_str() == Some("SUB") || (per_day == 0 && subs > 0);
    let (required_minutes, required_subs, required_days) = if is_sub {
        (0, subs.max(1), 0)
    } else {
        (per_day * repeats, 0, if repeats > 1 { repeats } else { 0 })
    };

    // The rewards the viewer ends up with: a container's pool, else the reward.
    let mut benefit_edges = Vec::new();
    let mut random_of = None;
    let mut pool_ids: Vec<String> = Vec::new();
    let mut unheld: Vec<&Value> = Vec::new();
    for reward in group["rewards"].as_array().into_iter().flatten() {
        let pool: Vec<&Value> = reward["pool"]["rewards"].as_array().into_iter().flatten().collect();
        if pool.len() > 1 {
            random_of = Some(pool.len() as u32);
            unheld.extend(pool.iter().copied().filter(|p| {
                p["id"].as_str().is_some_and(|id| !viewer.claimed_items.contains(id))
            }));
        }
        pool_ids.extend(pool.iter().filter_map(|p| p["id"].as_str().map(str::to_string)));
        let benefit = |item: &Value, distribution_type: Option<&str>| DropBenefit {
            id: item["id"].as_str().unwrap_or_default().to_string(),
            name: item["name"].as_str().unwrap_or_default().to_string(),
            image_url: item["thumbnailURL"].as_str().unwrap_or_default().to_string(),
            distribution_type: distribution_type.map(str::to_string),
        };
        // A random draw shows its container (the Great Ball) first, until it
        // is opened, then the pool it draws from. `POOL` is Twitch's own name
        // for such a container. A pool of one is simply that reward.
        if pool.len() > 1 {
            benefit_edges.push(benefit(reward, Some("POOL")));
        }
        if pool.is_empty() {
            benefit_edges.push(benefit(reward, None));
        } else {
            benefit_edges.extend(pool.iter().map(|item| benefit(item, None)));
        }
    }

    let own = inventory_self
        .filter(|s| s.is_object())
        .or_else(|| Some(&group["self"]).filter(|s| s.is_object()));
    let progress = own.and_then(|s| {
        let status = s["status"].as_str().unwrap_or_default();
        let minutes = uint(&s["currentMinutesWatched"]);
        let subs_done = uint(&s["currentSubs"]);
        let days_done = uint(&s["grantCount"]);
        // A subscription tier is how a container is obtained (a Special Great
        // Ball). Twitch marks it CLAIMABLE once the subs are in, but the ball
        // is opened through its watch tier: here it simply means "you have it".
        let claimed = matches!(status, "CLAIMED" | "FULFILLED") || (is_sub && status == "CLAIMABLE");
        let ready = status == "CLAIMABLE" && !is_sub;
        if minutes == 0 && subs_done == 0 && days_done == 0 && !claimed && !ready {
            return None; // not started: the requirement says it all
        }
        let expires = parse_time(&s["currentWindow"]["expiresAt"]);
        let lapsed = expires.is_some_and(|e| e <= now);
        let minutes_today = if required_days > 1 {
            if lapsed { 0 } else { minutes.min(per_day) }
        } else {
            minutes
        };
        // `grantCount` counts today as soon as today's minutes are in (Twitch
        // shows "20/20m (Day 2)" at grantCount 2), so today's minutes add only
        // while the day is still short.
        let mut current = if required_days > 1 {
            days_done * per_day + if minutes_today < per_day { minutes_today } else { 0 }
        } else if required_minutes > 0 {
            minutes
        } else {
            0
        };
        if claimed || ready {
            current = required_minutes;
        }
        let current = current.min(required_minutes) as i32;

        let rise = tracker.entry(id.clone()).or_insert(Rise { last: current, rose_at: None });
        if current > rise.last {
            rise.rose_at = Some(now);
        }
        rise.last = current;
        let accruing = !claimed
            && !ready
            && rise.rose_at.is_some_and(|t| now - t <= Duration::minutes(ACCRUING_MINUTES));

        Some(DropProgress {
            campaign_id: campaign_id.to_string(),
            drop_id: id.clone(),
            current_minutes_watched: current,
            required_minutes_watched: required_minutes as i32,
            is_claimed: claimed,
            last_updated: now,
            // Claimed like any drop, through claimDropRewards: Twitch's own
            // chat panel names the instance `user#campaign#rewardGroup`.
            drop_instance_id: viewer
                .user_id
                .filter(|_| ready)
                .map(|user| format!("{user}#{campaign_id}#{id}")),
            twitch_progress: Some(TwitchProgress {
                days_done,
                minutes_today,
                window_expires_at: expires.filter(|_| !lapsed),
                accruing,
                subs_done,
                ready_to_claim: ready,
                earned: text(&s["earnedReward"], "name").map(|name| {
                    let id = s["earnedReward"]["id"].as_str().unwrap_or_default().to_string();
                    // A container earned but not yet opened is still a draw.
                    let distribution_type = benefit_edges
                        .iter()
                        .find(|b| b.id == id)
                        .and_then(|b| b.distribution_type.clone());
                    DropBenefit {
                        id,
                        name: name.to_string(),
                        image_url: s["earnedReward"]["thumbnailURL"].as_str().unwrap_or_default().to_string(),
                        distribution_type,
                    }
                }),
            }),
        })
    });

    // Every reward this tier could give is already held (Pichu, once claimed,
    // is the whole Poké Ball pool): nothing is left to earn here.
    let pool_held = !pool_ids.is_empty() && pool_ids.iter().all(|p| viewer.claimed_items.contains(p));
    let progress = match progress {
        Some(p) if p.is_claimed || p.twitch_progress.as_ref().is_some_and(|t| t.ready_to_claim) => Some(p),
        _ if pool_held => Some(DropProgress {
            campaign_id: campaign_id.to_string(),
            drop_id: id.clone(),
            current_minutes_watched: required_minutes as i32,
            required_minutes_watched: required_minutes as i32,
            is_claimed: true,
            last_updated: now,
            drop_instance_id: None,
            twitch_progress: Some(TwitchProgress {
                days_done: required_days,
                minutes_today: 0,
                window_expires_at: None,
                accruing: false,
                subs_done: required_subs,
                ready_to_claim: false,
                earned: (pool_ids.len() == 1).then(|| benefit_edges[0].clone()),
            }),
        }),
        other => other,
    };

    TimeBasedDrop {
        id,
        name: group["name"].as_str().unwrap_or_default().to_string(),
        required_minutes_watched: required_minutes as i32,
        benefit_edges,
        progress,
        is_collectible: required_minutes > 0,
        required_subs,
        required_days,
        random_of,
        // Twitch never repeats a reward from a pool, so one left unheld is what
        // this draw will give (Squirtle once Bulbasaur and Charmander are out).
        next_reward: (unheld.len() == 1).then(|| DropBenefit {
            id: unheld[0]["id"].as_str().unwrap_or_default().to_string(),
            name: unheld[0]["name"].as_str().unwrap_or_default().to_string(),
            image_url: unheld[0]["thumbnailURL"].as_str().unwrap_or_default().to_string(),
            distribution_type: None,
        }),
    }
}

/// Translates the campaign list and its details into ordinary drops. Pure:
/// `list` is `rewardCampaignsAvailableToUser`, `details` the response data
/// holding `c0..cN` (`dropsCampaign`) and `currentUser` (its id, the in-progress
/// list and `earnedDropRewards`).
pub fn build(
    list: &[Value],
    details: &Value,
    tracker: &mut HashMap<String, Rise>,
    now: DateTime<Utc>,
) -> Vec<DropCampaign> {
    let user = &details["currentUser"];
    let viewer = Viewer {
        user_id: text(user, "id"),
        claimed_items: user["inventory"]["earnedDropRewards"]["edges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| matches!(e["node"]["status"].as_str(), Some("CLAIMED" | "FULFILLED")))
            .filter_map(|e| e["node"]["item"]["id"].as_str().map(str::to_string))
            .collect(),
    };
    let mut inventory: HashMap<&str, &Value> = HashMap::new();
    for campaign in details["currentUser"]["inventory"]["viewerRewardDropCampaignsInProgress"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for group in campaign["rewardGroups"].as_array().into_iter().flatten() {
            if let Some(id) = group["id"].as_str() {
                inventory.insert(id, &group["self"]);
            }
        }
    }

    let mut out = Vec::new();
    let mut firsts: Vec<String> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let detail = &details[format!("c{i}")];
        let Some(id) = text(item, "id") else { continue };
        if detail.is_null() {
            continue;
        }
        let (Some(start_at), Some(end_at)) = (parse_time(&item["startsAt"]), parse_time(&item["endsAt"]))
        else {
            continue;
        };
        if start_at > now || end_at < now {
            continue;
        }
        let name = text(item, "name").unwrap_or(id);
        let group = display_group(item).filter(|g| g.has_category).unwrap_or_else(|| {
            no_category_group(text(item, "brand").unwrap_or(name))
        });

        let mut drops: Vec<TimeBasedDrop> = detail["rewardGroups"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| {
                let own = g["id"].as_str().and_then(|gid| inventory.get(gid).copied());
                group_drop(id, g, own, &viewer, tracker, now)
            })
            .collect();
        if drops.is_empty() {
            continue;
        }
        // Tiers in the order you earn them, and repeats of one reward (the
        // three Special Balls) numbered so each reads as its own tier.
        drops.sort_by_key(|d| (d.required_subs, d.required_minutes_watched));
        let first_tier = drops[0].name.clone();
        number_repeats(&mut drops);

        let image_url = if group.has_category && !group.image_url.is_empty() {
            group.image_url.clone()
        } else {
            text(detail, "imageURL")
                .map(str::to_string)
                .unwrap_or_else(|| first_reward_image(&drops))
        };
        firsts.push(first_tier);
        // Where it counts: every category any tier names, in Twitch's order.
        let mut category_ids: Vec<String> = Vec::new();
        for g in detail["rewardGroups"].as_array().into_iter().flatten() {
            for cat in g["progressCriteria"]["categories"].as_array().into_iter().flatten() {
                if let Some(cid) = cat["id"].as_str() {
                    if !category_ids.iter().any(|c| c == cid) {
                        category_ids.push(cid.to_string());
                    }
                }
            }
        }
        out.push(DropCampaign {
            id: id.to_string(),
            name: name.to_string(),
            game_id: group.game_id,
            game_name: group.game_name,
            description: text(item, "summary").filter(|s| *s != name).unwrap_or_default().to_string(),
            image_url,
            start_at,
            end_at,
            time_based_drops: drops,
            is_account_connected: true,
            allowed_channels: Vec::new(),
            is_acl_based: false,
            details_url: text(item, "aboutURL").map(str::to_string),
            account_link: None,
            separate_progress: true,
            has_category: group.has_category,
            category_ids,
        });
    }
    disambiguate_names(&mut out, &firsts);
    out
}

/// "Great Ball", "Great Ball", "Great Ball" become "Great Ball 1", "2", "3".
fn number_repeats(drops: &mut [TimeBasedDrop]) {
    let mut totals: HashMap<String, usize> = HashMap::new();
    for d in drops.iter() {
        *totals.entry(d.name.clone()).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for d in drops.iter_mut() {
        if totals.get(&d.name).copied().unwrap_or(0) > 1 {
            let n = seen.entry(d.name.clone()).or_default();
            *n += 1;
            d.name = format!("{} {}", d.name, n);
        }
    }
}

/// Campaigns in one group that share a name (Twitch names all three Pokémon
/// campaigns "First Partners Collection") take their first reward as the
/// name, and a subscription-only one says so, so each card reads differently.
fn disambiguate_names(campaigns: &mut [DropCampaign], firsts: &[String]) {
    let key = |c: &DropCampaign| (c.game_id.clone(), c.name.clone());
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for c in campaigns.iter() {
        *counts.entry(key(c)).or_default() += 1;
    }
    for (c, first) in campaigns.iter_mut().zip(firsts) {
        if counts.get(&key(c)).copied().unwrap_or(0) < 2 {
            continue;
        }
        let sub_only = c.time_based_drops.iter().all(|d| d.required_subs > 0);
        if c.description.is_empty() {
            c.description = c.name.clone();
        }
        c.name = if sub_only { format!("{first} · Subscribe") } else { first.clone() };
    }
}

fn details_query(count: usize) -> String {
    let group = format!(
        "rewardGroups {{ id name progressCriteria {{ requirementType requirements {{ minutesWatched subs }} repeatableConfig {{ repeatableTimes }} categories {{ id }} }} {SELF_FIELDS} rewards {{ id name thumbnailURL pool {{ rewards {{ id name thumbnailURL }} }} }} }}"
    );
    let vars: Vec<String> = (0..count).map(|i| format!("$id{i}: ID!")).collect();
    let fields: Vec<String> = (0..count)
        .map(|i| format!("c{i}: dropsCampaign(id: $id{i}) {{ id imageURL {group} }}"))
        .collect();
    format!(
        "query({}) {{ {} currentUser {{ id inventory {{ viewerRewardDropCampaignsInProgress {{ id rewardGroups {{ id {SELF_FIELDS} }} }} earnedDropRewards(first: 100) {{ edges {{ node {{ status item {{ id }} }} }} }} }} }} }}",
        vars.join(", "),
        fields.join(" ")
    )
}

async fn post(client: &reqwest::Client, token: &str, device_id: &str, session_id: &str, body: Value) -> Result<Value> {
    Ok(client
        .post(GQL_URL)
        .headers(DropsService::gql_headers(token, device_id, session_id))
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?
        .json()
        .await?)
}

/// The drops in Twitch's second list, as ordinary drops. Two requests: the
/// list, then every campaign's tiers and the viewer's progress in one go.
pub async fn fetch(client: &reqwest::Client, token: &str, device_id: &str, session_id: &str) -> Result<Vec<DropCampaign>> {
    if let Some((at, campaigns)) = CACHE.lock().ok().and_then(|c| c.clone()) {
        if at.elapsed().as_secs() < CACHE_SECS {
            return Ok(campaigns);
        }
    }
    let list = post(client, token, device_id, session_id, json!({ "query": LIST_QUERY })).await?;
    let Some(items) = list["data"]["rewardCampaignsAvailableToUser"].as_array().cloned() else {
        return Err(anyhow!("no list in the answer: {}", list["errors"]));
    };
    let campaigns = if items.is_empty() {
        Vec::new()
    } else {
        let mut variables = serde_json::Map::new();
        for (i, item) in items.iter().enumerate() {
            variables.insert(format!("id{i}"), item["id"].clone());
        }
        let details = post(
            client,
            token,
            device_id,
            session_id,
            json!({ "query": details_query(items.len()), "variables": variables }),
        )
        .await?;
        if details["data"].is_null() {
            return Err(anyhow!("no details in the answer: {}", details["errors"]));
        }
        let now = Utc::now();
        let mut tracker = TRACKER.lock().map_err(|_| anyhow!("tracker poisoned"))?;
        build(&items, &details["data"], &mut tracker, now)
    };
    debug!("[RewardDrops] {} campaigns from Twitch's second list", campaigns.len());
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((Instant::now(), campaigns.clone()));
    }
    Ok(campaigns)
}

/// Inventory rows for the campaigns the viewer has started, the same shape the
/// standard inventory builds, so the Inventory tab and the phone list them.
pub fn inventory_items(campaigns: &[DropCampaign]) -> Vec<crate::models::drops::InventoryItem> {
    campaigns
        .iter()
        .filter(|c| c.time_based_drops.iter().any(|d| d.progress.is_some()))
        .map(|c| {
            let total = c.time_based_drops.len() as i32;
            let claimed = c
                .time_based_drops
                .iter()
                .filter(|d| d.progress.as_ref().is_some_and(|p| p.is_claimed))
                .count() as i32;
            let in_progress = c
                .time_based_drops
                .iter()
                .filter(|d| d.progress.as_ref().is_some_and(|p| !p.is_claimed))
                .count() as i32;
            // A tier counts in full when claimed or held for claiming, else by
            // its share of minutes; a subscription tier has no minutes to share.
            let done: f32 = c
                .time_based_drops
                .iter()
                .map(|d| match &d.progress {
                    Some(p) if p.is_claimed => 1.0,
                    Some(p) if p.twitch_progress.as_ref().is_some_and(|t| t.ready_to_claim) => 1.0,
                    Some(p) if p.required_minutes_watched > 0 => {
                        (p.current_minutes_watched as f32 / p.required_minutes_watched as f32).min(1.0)
                    }
                    _ => 0.0,
                })
                .sum();
            crate::models::drops::InventoryItem {
                campaign: c.clone(),
                status: crate::models::drops::CampaignStatus::Active,
                progress_percentage: if total > 0 { done / total as f32 * 100.0 } else { 0.0 },
                total_drops: total,
                claimed_drops: claimed,
                drops_in_progress: in_progress,
            }
        })
        .collect()
}

/// The last fetched campaigns when fetched within `secs`.
pub fn cached_within(secs: u64) -> Option<Vec<DropCampaign>> {
    CACHE
        .lock()
        .ok()
        .and_then(|c| c.clone())
        .filter(|(at, _)| at.elapsed().as_secs() < secs)
        .map(|(_, v)| v)
}

/// The last fetched campaigns, at any age; empty before the first fetch.
pub fn cached() -> Vec<DropCampaign> {
    CACHE
        .lock()
        .ok()
        .and_then(|c| c.as_ref().map(|(_, v)| v.clone()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-25T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    /// The Pokémon list items and details as Twitch answered them on 2026-09-25,
    /// trimmed to the fields read.
    fn list() -> Vec<Value> {
        vec![
            json!({ "id": "poke", "name": "First Partners Collection", "brand": "Pokemon",
                    "summary": "First Partners Collection", "startsAt": "2026-08-24T17:00:00Z",
                    "endsAt": "2026-10-01T07:00:00Z", "aboutURL": "https://help.twitch.tv/s/article/pokemon-chat-badges",
                    "game": null }),
            json!({ "id": "great-sub", "name": "First Partners Collection", "brand": "Pokemon",
                    "summary": "First Partners Collection", "startsAt": "2026-08-24T17:00:00Z",
                    "endsAt": "2026-10-01T07:00:00Z", "aboutURL": "https://help.twitch.tv/s/article/pokemon-chat-badges",
                    "game": null }),
            json!({ "id": "control", "name": "CONTROL Resonant launch", "brand": "",
                    "summary": "Celebrate the launch", "startsAt": "2026-09-22T14:00:00Z",
                    "endsAt": "2026-10-13T13:59:59.999Z", "aboutURL": "https://www.twitch.tv/drops/campaigns",
                    "game": { "id": "1338428218", "displayName": "CONTROL Resonant", "boxArtURL": "https://box/control.jpg" } }),
        ]
    }

    fn pool(names: &[&str]) -> Value {
        json!({ "rewards": names.iter().map(|n| json!({ "id": format!("id-{n}"), "name": n, "thumbnailURL": format!("https://art/{n}.png") })).collect::<Vec<_>>() })
    }

    fn details(poke_self: Value, inventory: Value) -> Value {
        json!({
            "c0": { "id": "poke", "imageURL": "https://art/campaign.png", "rewardGroups": [
                { "id": "g-poke", "name": "Poké Ball",
                  "progressCriteria": { "requirementType": "WATCH", "requirements": { "minutesWatched": 20, "subs": null },
                                        "repeatableConfig": { "repeatableTimes": 3 } },
                  "self": poke_self,
                  "rewards": [ { "id": "ball", "name": "Poké Ball", "thumbnailURL": "https://art/ball.png", "pool": pool(&["Pichu"]) } ] }
            ] },
            "c1": { "id": "great-sub", "imageURL": "https://art/campaign.png", "rewardGroups": [
                { "id": "g-sub2", "name": "Great Ball",
                  "progressCriteria": { "requirementType": "SUB", "requirements": { "minutesWatched": null, "subs": 2 }, "repeatableConfig": null },
                  "self": null,
                  "rewards": [ { "id": "great", "name": "Great Ball", "thumbnailURL": "https://art/great.png",
                                 "pool": pool(&["Charmander", "Bulbasaur", "Squirtle"]) } ] }
            ] },
            "c2": { "id": "control", "imageURL": "https://art/control-campaign.png", "rewardGroups": [
                { "id": "g-helmet", "name": "Sierra Helmet",
                  "progressCriteria": { "requirementType": "WATCH", "requirements": { "minutesWatched": 240, "subs": null }, "repeatableConfig": null },
                  "self": null,
                  "rewards": [ { "id": "helmet", "name": "Sierra Helmet", "thumbnailURL": "https://art/helmet.png", "pool": null } ] }
            ] },
            "currentUser": { "id": "u1", "inventory": { "viewerRewardDropCampaignsInProgress": inventory } }
        })
    }

    fn built(poke_self: Value, inventory: Value) -> Vec<DropCampaign> {
        build(&list(), &details(poke_self, inventory), &mut HashMap::new(), now())
    }

    fn inv(group: &str, own: Value) -> Value {
        json!([ { "id": "x", "rewardGroups": [ { "id": group, "self": own } ] } ])
    }

    #[test]
    fn a_watch_group_becomes_a_multi_day_drop() {
        let c = built(Value::Null, json!([]));
        let poke = &c[0];
        assert_eq!(poke.game_name, "Pokemon");
        assert_eq!(poke.game_id, "twitch-rewards-pokemon");
        assert!(!poke.has_category && poke.separate_progress);
        assert_eq!(poke.image_url, "https://art/campaign.png");
        let d = &poke.time_based_drops[0];
        assert_eq!((d.required_minutes_watched, d.required_days, d.required_subs), (60, 3, 0));
        assert_eq!(d.benefit_edges[0].name, "Pichu");
        assert_eq!(d.benefit_edges[0].image_url, "https://art/Pichu.png");
        assert_eq!(d.random_of, None);
    }

    #[test]
    fn a_sub_group_becomes_a_subscribe_drop_drawn_at_random() {
        let c = built(Value::Null, json!([]));
        let d = &c[1].time_based_drops[0];
        assert_eq!((d.required_minutes_watched, d.required_subs), (0, 2));
        assert!(!d.is_collectible);
        assert_eq!(d.random_of, Some(3));
        // The container first (what an unopened draw looks like), then the pool.
        let names: Vec<&str> = d.benefit_edges.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["Great Ball", "Charmander", "Bulbasaur", "Squirtle"]);
        assert_eq!(d.benefit_edges[0].distribution_type.as_deref(), Some("POOL"));
        assert_eq!(d.benefit_edges[0].image_url, "https://art/great.png");
    }

    #[test]
    fn a_campaign_with_a_game_joins_that_game() {
        let c = built(Value::Null, json!([]));
        let control = &c[2];
        assert_eq!((control.game_id.as_str(), control.game_name.as_str()), ("1338428218", "CONTROL Resonant"));
        assert!(control.has_category && control.separate_progress);
        assert_eq!(control.image_url, "https://box/control.jpg");
        assert_eq!(control.time_based_drops[0].required_minutes_watched, 240);
    }

    #[test]
    fn progress_is_twitchs_own_and_the_inventory_wins() {
        let own = json!({ "status": "IN_PROGRESS", "currentMinutesWatched": 12, "currentSubs": 0, "grantCount": 1,
                          "currentWindow": { "startedAt": "2026-09-25T08:00:00Z", "expiresAt": "2026-09-26T08:00:00Z" } });
        let stale = json!({ "status": "IN_PROGRESS", "currentMinutesWatched": null, "grantCount": 0 });
        let c = built(stale, inv("g-poke", own));
        let p = c[0].time_based_drops[0].progress.clone().unwrap();
        assert_eq!((p.current_minutes_watched, p.required_minutes_watched, p.is_claimed), (32, 60, false));
        let t = p.twitch_progress.unwrap();
        assert_eq!((t.days_done, t.minutes_today), (1, 12));
        assert!(t.window_expires_at.is_some());
    }

    #[test]
    fn a_met_day_is_already_in_the_day_count() {
        // Twitch at the time of this state: "20/20m (Day 2)", the bar two thirds.
        let own = json!({ "status": "IN_PROGRESS", "currentMinutesWatched": 20, "grantCount": 2,
                          "currentWindow": { "expiresAt": "2026-09-26T09:00:00Z" } });
        let c = built(Value::Null, inv("g-poke", own));
        let p = c[0].time_based_drops[0].progress.clone().unwrap();
        assert_eq!((p.current_minutes_watched, p.required_minutes_watched), (40, 60));
        let t = p.twitch_progress.unwrap();
        assert_eq!((t.days_done, t.minutes_today), (2, 20));
    }

    #[test]
    fn a_lapsed_day_window_starts_today_at_zero() {
        let own = json!({ "status": "IN_PROGRESS", "currentMinutesWatched": 15, "grantCount": 1,
                          "currentWindow": { "expiresAt": "2026-09-25T04:39:54Z" } });
        let c = built(Value::Null, inv("g-poke", own));
        let p = c[0].time_based_drops[0].progress.clone().unwrap();
        assert_eq!(p.current_minutes_watched, 20);
        let t = p.twitch_progress.unwrap();
        assert_eq!(t.minutes_today, 0);
        assert!(t.window_expires_at.is_none());
    }

    #[test]
    fn not_started_means_requirement_only() {
        let untouched = json!({ "status": "IN_PROGRESS", "currentMinutesWatched": null, "currentSubs": null, "grantCount": 0 });
        let c = built(untouched, json!([]));
        assert!(c[0].time_based_drops[0].progress.is_none());
    }

    #[test]
    fn a_subscription_tier_means_you_have_the_ball_and_an_unlock_is_ready_to_open() {
        // Twitch marks a Special Great Ball's subscription tier CLAIMABLE once
        // the subs are in; the ball is opened through its watch tier, so this
        // reads as held, with nothing to claim.
        let earned = json!({ "status": "CLAIMABLE", "currentMinutesWatched": 0, "currentSubs": 4, "grantCount": 0,
                             "earnedReward": { "name": "Great Ball" } });
        let c = build(&list(), &{
            let mut d = details(Value::Null, json!([]));
            d["c1"]["rewardGroups"][0]["self"] = earned;
            d
        }, &mut HashMap::new(), now());
        let p = c[1].time_based_drops[0].progress.clone().unwrap();
        assert!(p.is_claimed);
        assert!(p.drop_instance_id.is_none());
        let t = p.twitch_progress.unwrap();
        assert!(!t.ready_to_claim);
        assert_eq!(t.subs_done, 4);

        // A watch tier Twitch marks CLAIMABLE is a ball ready to open.
        let unlocked = json!({ "status": "CLAIMABLE", "currentMinutesWatched": 20, "grantCount": 3 });
        let c = built(Value::Null, inv("g-poke", unlocked));
        let p = c[0].time_based_drops[0].progress.clone().unwrap();
        assert!(!p.is_claimed);
        assert!(p.twitch_progress.unwrap().ready_to_claim);

        let claimed = json!({ "status": "CLAIMED", "currentMinutesWatched": 20, "grantCount": 3 });
        let c = built(Value::Null, inv("g-poke", claimed));
        let p = c[0].time_based_drops[0].progress.clone().unwrap();
        assert!(p.is_claimed);
        assert_eq!(p.current_minutes_watched, 60);
    }

    #[test]
    fn accruing_only_while_minutes_rise() {
        let mut tracker = HashMap::new();
        let at = |m: u32| inv("g-poke", json!({ "status": "IN_PROGRESS", "currentMinutesWatched": m, "grantCount": 0,
                                               "currentWindow": { "expiresAt": "2026-09-26T08:00:00Z" } }));
        let accruing = |c: &[DropCampaign]| {
            c[0].time_based_drops[0].progress.as_ref().unwrap().twitch_progress.as_ref().unwrap().accruing
        };
        let t0 = now();
        assert!(!accruing(&build(&list(), &details(Value::Null, at(5)), &mut tracker, t0)));
        assert!(accruing(&build(&list(), &details(Value::Null, at(7)), &mut tracker, t0 + Duration::minutes(2))));
        // A second fetch seconds later with the same minutes keeps it lit.
        assert!(accruing(&build(&list(), &details(Value::Null, at(7)), &mut tracker, t0 + Duration::minutes(2) + Duration::seconds(20))));
        assert!(!accruing(&build(&list(), &details(Value::Null, at(7)), &mut tracker, t0 + Duration::minutes(9))));
    }

    #[test]
    fn an_ended_or_upcoming_campaign_is_skipped() {
        let late = DateTime::parse_from_rfc3339("2026-10-05T00:00:00Z").unwrap().with_timezone(&Utc);
        let c = build(&list(), &details(Value::Null, json!([])), &mut HashMap::new(), late);
        assert_eq!(c.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["control"]);
    }

    #[test]
    fn a_standard_campaign_without_a_game_is_grouped_not_dropped() {
        let g = display_group(&json!({ "name": "Sitewide Thing", "owner": { "name": "Twitch" }, "game": null })).unwrap();
        assert_eq!((g.game_id.as_str(), g.game_name.as_str(), g.has_category), ("twitch-rewards-twitch", "Twitch", false));
        let g = display_group(&json!({ "name": "Sitewide Thing", "game": null })).unwrap();
        assert_eq!(g.game_id, "twitch-rewards-sitewide-thing");
        assert!(display_group(&json!({ "game": null })).is_none());
        let real = display_group(&json!({ "game": { "id": "1", "displayName": "Rust", "boxArtURL": "b" } })).unwrap();
        assert!(real.has_category);
    }

    #[test]
    fn a_claimable_reward_is_claimed_like_any_drop() {
        let unlocked = json!({ "status": "CLAIMABLE", "currentMinutesWatched": 20, "grantCount": 3 });
        let c = built(Value::Null, inv("g-poke", unlocked));
        let p = c[0].time_based_drops[0].progress.clone().unwrap();
        // What twitch.tv's own chat panel sends to claimDropRewards to open it.
        assert_eq!(p.drop_instance_id.as_deref(), Some("u1#poke#g-poke"));

        // Nothing to claim, no instance.
        let own = json!({ "status": "IN_PROGRESS", "currentMinutesWatched": 12, "grantCount": 1 });
        let c = built(Value::Null, inv("g-poke", own));
        assert!(c[0].time_based_drops[0].progress.clone().unwrap().drop_instance_id.is_none());
    }

    #[test]
    fn the_last_unheld_badge_is_what_the_draw_gives() {
        let mut d = details(Value::Null, json!([]));
        d["currentUser"]["inventory"]["earnedDropRewards"] = json!({ "edges": [
            { "node": { "status": "CLAIMED", "item": { "id": "id-Charmander" } } },
            { "node": { "status": "CLAIMED", "item": { "id": "id-Bulbasaur" } } } ] });
        let c = build(&list(), &d, &mut HashMap::new(), now());
        let next = c[1].time_based_drops[0].next_reward.clone().expect("only Squirtle left");
        assert_eq!((next.name.as_str(), next.image_url.as_str()), ("Squirtle", "https://art/Squirtle.png"));

        // Two still open: a real draw, nothing named.
        let c = built(Value::Null, json!([]));
        assert!(c[1].time_based_drops[0].next_reward.is_none());
    }

    #[test]
    fn a_campaign_lists_every_category_it_counts_in() {
        let mut d = details(Value::Null, json!([]));
        d["c0"]["rewardGroups"][0]["progressCriteria"]["categories"] =
            json!([ { "id": "670867987" }, { "id": "509658" } ]);
        let c = build(&list(), &d, &mut HashMap::new(), now());
        assert_eq!(c[0].category_ids, vec!["670867987", "509658"]);
    }

    #[test]
    fn a_pool_already_held_reads_as_done() {
        let mut d = details(json!({ "status": "IN_PROGRESS", "grantCount": 0 }), json!([]));
        d["currentUser"]["inventory"]["earnedDropRewards"] =
            json!({ "edges": [ { "node": { "status": "CLAIMED", "item": { "id": "id-Pichu" } } } ] });
        let c = build(&list(), &d, &mut HashMap::new(), now());
        let p = c[0].time_based_drops[0].progress.clone().unwrap();
        assert!(p.is_claimed);
        assert_eq!(p.twitch_progress.unwrap().earned.unwrap().name, "Pichu");

        // Two of three held: the draw is still open.
        d["currentUser"]["inventory"]["earnedDropRewards"] = json!({ "edges": [
            { "node": { "status": "CLAIMED", "item": { "id": "id-Charmander" } } },
            { "node": { "status": "CLAIMED", "item": { "id": "id-Bulbasaur" } } } ] });
        let c = build(&list(), &d, &mut HashMap::new(), now());
        assert!(c[1].time_based_drops[0].progress.is_none());
    }

    #[test]
    fn same_named_campaigns_and_repeated_tiers_read_differently() {
        let mut d = details(Value::Null, json!([]));
        let tier = d["c1"]["rewardGroups"][0].clone();
        let mut sub1 = tier.clone();
        sub1["id"] = json!("g-sub1");
        sub1["progressCriteria"]["requirements"]["subs"] = json!(1);
        d["c1"]["rewardGroups"] = json!([tier, sub1]);
        let c = build(&list(), &d, &mut HashMap::new(), now());
        assert_eq!(c[0].name, "Poké Ball");
        assert_eq!(c[0].description, "First Partners Collection");
        assert_eq!(c[1].name, "Great Ball · Subscribe");
        let tiers: Vec<(&str, u32)> =
            c[1].time_based_drops.iter().map(|t| (t.name.as_str(), t.required_subs)).collect();
        assert_eq!(tiers, vec![("Great Ball 1", 1), ("Great Ball 2", 2)]);
        assert_eq!(c[2].name, "CONTROL Resonant launch");
    }

    #[test]
    fn an_opened_draw_names_the_badge_that_came_out() {
        let claimed = json!({ "status": "CLAIMED", "currentMinutesWatched": 20, "grantCount": 3,
                              "earnedReward": { "id": "b", "name": "Bulbasaur", "thumbnailURL": "https://art/b.png" } });
        let c = built(Value::Null, inv("g-poke", claimed));
        let earned = c[0].time_based_drops[0].progress.clone().unwrap().twitch_progress.unwrap().earned.unwrap();
        assert_eq!((earned.name.as_str(), earned.image_url.as_str()), ("Bulbasaur", "https://art/b.png"));
    }

    #[test]
    fn only_started_campaigns_reach_the_inventory() {
        let own = json!({ "status": "IN_PROGRESS", "currentMinutesWatched": 12, "grantCount": 1,
                          "currentWindow": { "expiresAt": "2026-09-26T08:00:00Z" } });
        let c = built(Value::Null, inv("g-poke", own));
        let items = inventory_items(&c);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].campaign.id, "poke");
        assert_eq!((items[0].total_drops, items[0].claimed_drops, items[0].drops_in_progress), (1, 0, 1));
        assert!((items[0].progress_percentage - 32.0 / 60.0 * 100.0).abs() < 0.01);
    }

    #[test]
    fn the_details_query_uses_variables() {
        let q = details_query(2);
        assert!(q.starts_with("query($id0: ID!, $id1: ID!)"));
        assert!(q.contains("c1: dropsCampaign(id: $id1)"));
    }
}

#[cfg(test)]
mod live_probe {
    use crate::services::drops_auth_service::DropsAuthService;
    use crate::services::drops_service::DropsService;
    use serde_json::{json, Value};

    async fn gql(client: &reqwest::Client, token: &str, body: Value) -> Value {
        let headers = DropsService::gql_headers(token, "probe0device0id", "probe0session0id");
        client
            .post("https://gql.twitch.tv/gql")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .expect("request")
            .json()
            .await
            .expect("json")
    }

    /// The real fetch and translation against the signed-in account: what the
    /// drops pages will show.
    #[tokio::test]
    #[ignore = "live read-only fetch with the local Drops sign-in; run with --ignored --nocapture"]
    async fn live_translate() {
        let token = DropsAuthService::get_token().await.expect("drops sign-in");
        let client = crate::services::http::client_unbounded();
        let campaigns = super::fetch(&client, &token, "probe0device0id", "probe0session0id")
            .await
            .expect("fetch");
        for c in &campaigns {
            println!(
                "CAMPAIGN {} | {} [{}] category={} image={}",
                c.name, c.game_name, c.game_id, c.has_category, !c.image_url.is_empty()
            );
            for d in &c.time_based_drops {
                let rewards: Vec<&str> = d.benefit_edges.iter().map(|b| b.name.as_str()).collect();
                let p = d.progress.as_ref().map(|p| {
                    let t = p.twitch_progress.as_ref().unwrap();
                    format!(
                        "{}/{} claimed={} ready={} days={} today={} subs={} window={:?}",
                        p.current_minutes_watched, p.required_minutes_watched, p.is_claimed,
                        t.ready_to_claim, t.days_done, t.minutes_today, t.subs_done, t.window_expires_at
                    )
                });
                println!(
                    "  TIER {} | {} min, {} days, {} subs, random_of={:?} | {:?} | {}",
                    d.name, d.required_minutes_watched, d.required_days, d.required_subs,
                    d.random_of, rewards, p.unwrap_or_else(|| "not started".into())
                );
            }
        }
        for item in super::inventory_items(&campaigns) {
            println!("INVENTORY {} {:.0}%", item.campaign.name, item.progress_percentage);
        }
    }

    /// Which categories and channels count toward each reward tier, and what
    /// Twitch says earns on one channel: what automation needs to pick a stream.
    #[tokio::test]
    #[ignore = "live read-only eligibility read with the local Drops sign-in; run with --ignored --nocapture"]
    async fn live_eligibility() {
        let token = DropsAuthService::get_token().await.expect("drops sign-in");
        let client = crate::services::http::client_unbounded();
        let list = gql(&client, &token, json!({ "query": "query { rewardCampaignsAvailableToUser { id name } }" })).await;
        for item in list["data"]["rewardCampaignsAvailableToUser"].as_array().into_iter().flatten() {
            let r = gql(&client, &token, json!({
                "query": "query($id: ID!) { dropsCampaign(id: $id) { id name game { id name } rewardGroups { id name progressCriteria { requirementType isAutoProgressable channels { id } categories { id displayName } } } } }",
                "variables": { "id": item["id"] }
            })).await;
            println!("CAMPAIGN {} errors={}", item["name"], r["errors"]);
            for g in r["data"]["dropsCampaign"]["rewardGroups"].as_array().into_iter().flatten() {
                let cats: Vec<String> = g["progressCriteria"]["categories"].as_array().into_iter().flatten()
                    .map(|c| format!("{}:{}", c["id"].as_str().unwrap_or(""), c["displayName"].as_str().unwrap_or(""))).collect();
                let chans = g["progressCriteria"]["channels"].as_array().map(|a| a.len());
                println!("  GROUP {} | {} | auto={} | channels={:?} | categories({})={:?}",
                    g["name"], g["progressCriteria"]["requirementType"], g["progressCriteria"]["isAutoProgressable"], chans, cats.len(), cats.iter().take(12).collect::<Vec<_>>());
            }
        }
        // Twitch's own answer for one channel (the "pokemon" channel).
        let user = gql(&client, &token, json!({ "query": "query { user(login: \"pokemon\") { id stream { id game { id displayName } } } }" })).await;
        let id = user["data"]["user"]["id"].clone();
        println!("POKEMON CHANNEL {} live={}", id, user["data"]["user"]["stream"]);
        let per_channel = gql(&client, &token, json!({
            "query": "query($c: ID!) { channelDropCampaigns(channelID: $c) { id name } channelDropCampaignsProgress(channelID: $c) { id name } }",
            "variables": { "c": id }
        })).await;
        println!("PER_CHANNEL {}", per_channel);
    }

    /// Wall time of each request the drops page now waits on.
    #[tokio::test]
    #[ignore = "live read-only timing with the local Drops sign-in; run with --ignored --nocapture"]
    async fn live_timing() {
        let token = DropsAuthService::get_token().await.expect("drops sign-in");
        let client = crate::services::http::client_unbounded();
        for round in 0..2 {
            let t = std::time::Instant::now();
            let list = gql(&client, &token, json!({ "query": super::LIST_QUERY })).await;
            let list_ms = t.elapsed().as_millis();
            let items = list["data"]["rewardCampaignsAvailableToUser"].as_array().cloned().unwrap_or_default();
            let mut vars = serde_json::Map::new();
            for (i, item) in items.iter().enumerate() {
                vars.insert(format!("id{i}"), item["id"].clone());
            }
            let t = std::time::Instant::now();
            let _ = gql(&client, &token, json!({ "query": super::details_query(items.len()), "variables": vars })).await;
            let details_ms = t.elapsed().as_millis();
            let t = std::time::Instant::now();
            let _ = gql(&client, &token, json!({ "query": "query { currentUser { inventory { earnedDropRewards(first: 100) { edges { node { status item { id } } } } } } }" })).await;
            let earned_ms = t.elapsed().as_millis();
            let t = std::time::Instant::now();
            let _ = crate::services::drops_service::DropsService::fetch_active_campaigns(&client, "probe0device0id", "probe0session0id").await;
            let all_ms = t.elapsed().as_millis();
            println!("TIMING round {round}: list {list_ms} ms, details {details_ms} ms (earned alone {earned_ms} ms), whole campaign fetch {all_ms} ms");
        }
    }

    /// What Twitch's own Inventory query says about the same campaigns: its
    /// in-progress list is what twitch.tv's inventory page renders and claims
    /// from (by dropInstanceID).
    #[tokio::test]
    #[ignore = "live read-only inventory read with the local Drops sign-in; run with --ignored --nocapture"]
    async fn live_inventory_view() {
        let token = DropsAuthService::get_token().await.expect("drops sign-in");
        let client = crate::services::http::client_unbounded();
        let inv = gql(
            &client,
            &token,
            json!({
                "operationName": "Inventory",
                "variables": { "fetchRewardCampaigns": false },
                "extensions": { "persistedQuery": { "version": 1, "sha256Hash": crate::services::drops_service::INVENTORY_QUERY_HASH } }
            }),
        )
        .await;
        let list = inv["data"]["currentUser"]["inventory"]["dropCampaignsInProgress"].as_array().cloned().unwrap_or_default();
        println!("INVENTORY {} campaigns in progress", list.len());
        for c in &list {
            println!("CAMPAIGN {} | {} | game={}", c["id"], c["name"], c["game"]);
            for d in c["timeBasedDrops"].as_array().into_iter().flatten() {
                let benefits: Vec<String> = d["benefitEdges"].as_array().into_iter().flatten()
                    .map(|e| format!("{}:{}", e["benefit"]["id"], e["benefit"]["name"])).collect();
                println!("  DROP {} | {} | req={} subs={} | self={} | {:?}",
                    d["id"], d["name"], d["requiredMinutesWatched"], d["requiredSubs"], d["self"], benefits);
            }
        }
    }

    /// The viewer's earned rewards from the second list: what twitch.tv reads
    /// to offer a claim (status + id).
    #[tokio::test]
    #[ignore = "live read-only earned-rewards read with the local Drops sign-in; run with --ignored --nocapture"]
    async fn live_earned_rewards() {
        let token = DropsAuthService::get_token().await.expect("drops sign-in");
        let client = crate::services::http::client_unbounded();
        let r = gql(
            &client,
            &token,
            json!({ "query": "query { currentUser { inventory { earnedDropRewards(first: 50) { edges { node { id status earnedAt isConnected item { id name distributionType } campaign { id brandName } } } } } } }" }),
        )
        .await;
        println!("ERRORS {}", r["errors"]);
        for e in r["data"]["currentUser"]["inventory"]["earnedDropRewards"]["edges"].as_array().into_iter().flatten() {
            println!("EARNED {}", e["node"]);
        }
    }

    /// Read-only field discovery against the signed-in account. Twitch strips
    /// introspection, but reports every invalid field by name; the canary
    /// `zzNotARealFieldZz` proves the parent type was reached.
    #[tokio::test]
    #[ignore = "live read-only probe with the local Drops sign-in; run with --ignored --nocapture"]
    async fn live_probe_reward_fields() {
        let token = DropsAuthService::get_token().await.expect("drops sign-in");
        let client = crate::services::http::client_unbounded();

        let list = gql(
            &client,
            &token,
            json!({ "query": "query { rewardCampaignsAvailableToUser { id name brand summary status startsAt endsAt aboutURL externalURL isSitewide game { id } unlockRequirements { minuteWatchedGoal subsGoal } rewards { id name } } }" }),
        )
        .await;
        println!("LIST {}", serde_json::to_string_pretty(&list).unwrap());

        let image_probe = gql(
            &client,
            &token,
            json!({ "query": "query { rewardCampaignsAvailableToUser { id image { zzNotARealFieldZz image1xURL image2xURL image3xURL url imageURL } rewards { id zzNotARealFieldZz thumbnailImage imageURL thumbnailURL bannerImage } } }" }),
        )
        .await;
        println!("IMAGE_PROBE {}", serde_json::to_string_pretty(&image_probe).unwrap());

        let Some(first) = list["data"]["rewardCampaignsAvailableToUser"][0]["id"].as_str() else {
            println!("no reward campaigns listed");
            return;
        };
        let detail = gql(
            &client,
            &token,
            json!({
                "query": "query($id: ID!) { dropsCampaign(id: $id) { id name brandName status startAt endAt rewardGroups { id name progressCriteria { requirementType requirements { minutesWatched subs } repeatableConfig { repeatableTimes resetIntervalSeconds shouldCompleteAllRepeatsToEarn } } self { status currentMinutesWatched currentSubs grantCount nextEligibleAt currentWindow { startedAt expiresAt } hasPreconditionsMet earnedReward { id name thumbnailURL } } rewards { id name pool { id rewards { id name } } } } } currentUser { inventory { viewerRewardDropCampaignsInProgress { id name rewardGroups { id self { status currentMinutesWatched currentSubs grantCount currentWindow { startedAt expiresAt } earnedReward { id name thumbnailURL } } } } } } }",
                "variables": { "id": first }
            }),
        )
        .await;
        println!("DETAIL {}", serde_json::to_string_pretty(&detail).unwrap());

        // Twitch now reports only the first invalid field, so each candidate
        // goes in its own request.
        let candidates = [
            ("campaign image1xURL", "query { rewardCampaignsAvailableToUser { id image { image1xURL } } }"),
            ("campaign image2xURL", "query { rewardCampaignsAvailableToUser { id image { image2xURL } } }"),
            ("campaign image url", "query { rewardCampaignsAvailableToUser { id image { url } } }"),
            ("campaign reward thumbnailURL", "query { rewardCampaignsAvailableToUser { id rewards { id thumbnailURL } } }"),
            ("campaign reward thumbnailImage", "query { rewardCampaignsAvailableToUser { id rewards { id thumbnailImage { image1xURL } } } }"),
        ];
        for (label, q) in candidates {
            let r = gql(&client, &token, json!({ "query": q })).await;
            let verdict = r["errors"][0]["message"].as_str().unwrap_or("OK");
            println!("FIELD {label}: {verdict}");
            if verdict == "OK" {
                println!("  sample {}", r["data"]["rewardCampaignsAvailableToUser"][0]);
            }
        }
        let group_candidates = [
            ("group reward thumbnailURL", "query($id: ID!) { dropsCampaign(id: $id) { rewardGroups { rewards { id thumbnailURL } } } }"),
            ("pool reward thumbnailURL", "query($id: ID!) { dropsCampaign(id: $id) { rewardGroups { rewards { pool { rewards { id name thumbnailURL } } } } } }"),
            ("campaign imageURL", "query($id: ID!) { dropsCampaign(id: $id) { id imageURL } }"),
        ];
        for (label, q) in group_candidates {
            let r = gql(&client, &token, json!({ "query": q, "variables": { "id": first } })).await;
            let verdict = r["errors"][0]["message"].as_str().unwrap_or("OK");
            println!("FIELD {label}: {verdict}");
            if verdict == "OK" {
                println!("  sample {}", r["data"]["dropsCampaign"]);
            }
        }
        let reward_probe = gql(
            &client,
            &token,
            json!({
                "query": "query($id: ID!) { dropsCampaign(id: $id) { zzNotARealFieldZz imageURL thumbnailURL image { url } rewardGroups { id rewards { zzNotARealFieldZz thumbnailURL imageURL image { url } pool { rewards { zzNotARealFieldZz thumbnailURL imageURL image { url } } } } } } }",
                "variables": { "id": first }
            }),
        )
        .await;
        println!("REWARD_PROBE {}", serde_json::to_string_pretty(&reward_probe).unwrap());
    }
}
