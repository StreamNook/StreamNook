//! Pokémon badge evolution: Twitch's progression tracks.
//!
//! Evolving a Pokémon chat badge earns progress only for the line the viewer has
//! EQUIPPED. Twitch models that as a track (one for every evolution) of lanes
//! (one per line, shared by its stages: Haunter and Gengar are one lane), set
//! with `setTrackSelection` and read with `trackSelections`; twitch.tv's Drops
//! panel below chat is where a viewer picks one. A stage's day progress is its
//! reward group's ordinary progress, which `reward_drops` already reads.
//!
//! Twitch reports every evolution group as in progress with its preconditions
//! met, even one whose earlier form the viewer does not hold, so whether a line
//! can start comes from the badges themselves: the stage before in the same
//! lane, or, for a line's first stage, the badge it evolves from
//! (`badge_earn::evolves_from`).
//!
//! Nor does Twitch list a lane's stages in order: the Bulbasaur lane comes as
//! Venusaur, then Ivysaur. The order is the chain of what each stage evolves
//! from (`evolution_order`), with Twitch's order kept only where that is unknown.

use crate::models::drops::{DropCampaign, TimeBasedDrop};
use crate::services::drops_overview::badge_key;
use crate::services::reward_drops::TrackGroup;
use anyhow::{anyhow, Result};
use log::{debug, warn};
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

const SELECTIONS_QUERY: &str =
    "query ProgressionTrackSelections($tracks: [ID!]!) { trackSelections(tracks: $tracks) { id track lane } }";
const SET_SELECTION_MUTATION: &str =
    "mutation SetProgressionTrackSelection($input: SetTrackSelectionInput!) { setTrackSelection(input: $input) { error } }";

/// How long the equipped lanes are reused before Twitch is asked again.
/// An equip from StreamNook updates the copy at once.
const SELECTIONS_SECS: u64 = 60;
/// The second drops list is reused when this fresh.
const CAMPAIGNS_SECS: u64 = 300;

static SELECTIONS: Lazy<Mutex<Option<(Instant, HashMap<String, String>)>>> = Lazy::new(|| Mutex::new(None));

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct EvolutionBoard {
    /// Empty when there is nothing to evolve right now, or no Drops sign-in.
    pub tracks: Vec<EvolutionTrack>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EvolutionTrack {
    pub track_id: String,
    pub campaign_id: String,
    pub campaign_name: String,
    /// Evolving ends here.
    pub ends_ms: i64,
    pub equipped_lane: Option<String>,
    /// In Twitch's order.
    pub lanes: Vec<EvolutionLane>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EvolutionLane {
    pub lane_id: String,
    /// The badge the line starts from (Gastly), when known.
    pub base: Option<BadgeRef>,
    pub stages: Vec<EvolutionStage>,
    /// The stage being worked on: the first not yet earned. `None` = done.
    pub current: Option<usize>,
    pub equipped: bool,
    pub can_equip: bool,
    /// The badge that must be held first, when it is known not to be.
    pub blocked_by: Option<String>,
    /// What the current stage evolves from: the stage before it, or the base.
    pub from: Option<String>,
    /// The current stage's progress in words ("Day 1 of 3 · 5/20 min today").
    pub line: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EvolutionStage {
    pub group_id: String,
    pub title: String,
    pub image_url: String,
    pub earned: bool,
    pub days_done: u32,
    pub total_days: u32,
    pub ready_to_claim: bool,
    /// What `claim_drop` sends when the stage is ready.
    pub instance_id: Option<String>,
    pub claim_label: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BadgeRef {
    pub title: String,
    pub image_url: String,
    pub owned: bool,
}

/// What the board needs to know about global badges, by comparable title.
#[derive(Default, Debug, Clone)]
pub(crate) struct BadgeIndex {
    /// `badge_key(title)` to (title, image, whether any version is held).
    pub by_title: HashMap<String, (String, String, bool)>,
    /// `badge_key(title)` of an evolved badge to the title it evolves from.
    pub base_of: HashMap<String, String>,
}

impl BadgeIndex {
    fn badge(&self, title: &str) -> Option<BadgeRef> {
        self.by_title.get(&badge_key(title)).map(|(t, image, owned)| BadgeRef {
            title: t.clone(),
            image_url: image.clone(),
            owned: *owned,
        })
    }
}

/// The board from inputs already in hand. Pure.
pub(crate) fn build_board(
    campaigns: &[DropCampaign],
    groups: &[TrackGroup],
    selections: &HashMap<String, String>,
    badges: &BadgeIndex,
    now_ms: i64,
) -> EvolutionBoard {
    let drops: HashMap<&str, &TimeBasedDrop> = campaigns
        .iter()
        .flat_map(|c| c.time_based_drops.iter())
        .map(|d| (d.id.as_str(), d))
        .collect();

    // Tracks and their lanes in Twitch's order. A group with no lane (the
    // balls) needs no choice and is not shown.
    let mut tracks: Vec<(&TrackGroup, Vec<(String, Vec<&TrackGroup>)>)> = Vec::new();
    for g in groups {
        let Some(lane) = g.lane.as_deref() else { continue };
        let at = match tracks.iter().position(|(t, _)| t.track == g.track) {
            Some(i) => i,
            None => {
                tracks.push((g, Vec::new()));
                tracks.len() - 1
            }
        };
        let lanes = &mut tracks[at].1;
        match lanes.iter_mut().find(|(id, _)| id.as_str() == lane) {
            Some((_, stages)) => stages.push(g),
            None => lanes.push((lane.to_string(), vec![g])),
        }
    }

    let out = tracks
        .into_iter()
        .filter_map(|(first, lanes)| {
            let equipped_lane = selections.get(&first.track).cloned();
            let lanes: Vec<EvolutionLane> = lanes
                .into_iter()
                .filter_map(|(lane_id, members)| {
                    let listed: Vec<&TimeBasedDrop> =
                        members.iter().filter_map(|g| drops.get(g.group_id.as_str()).copied()).collect();
                    if listed.is_empty() {
                        return None;
                    }
                    let titles: Vec<String> = listed.iter().map(|d| stage_title(d)).collect();
                    let stage_drops: Vec<&TimeBasedDrop> =
                        evolution_order(&titles, |t| badges.base_of.get(&badge_key(t)).cloned())
                            .into_iter()
                            .map(|i| listed[i])
                            .collect();
                    let stages: Vec<EvolutionStage> = stage_drops.iter().map(|d| stage(d, badges)).collect();
                    let current = stages.iter().position(|s| !s.earned);
                    // A line's first stage evolves from a badge the copy names;
                    // every later one from the stage before it, which is earned
                    // by the time it is current.
                    let base = stages
                        .first()
                        .and_then(|s| badges.base_of.get(&badge_key(&s.title)))
                        .map(|title| badges.badge(title).unwrap_or(BadgeRef {
                            title: title.clone(),
                            image_url: String::new(),
                            owned: false,
                        }));
                    let blocked_by = match (current, &base) {
                        (Some(0), Some(b)) if !b.owned => Some(b.title.clone()),
                        _ => None,
                    };
                    let line = current.and_then(|i| crate::services::drop_words::progress_line(stage_drops[i], now_ms));
                    let from = match current {
                        Some(i) if i > 0 => Some(stages[i - 1].title.clone()),
                        Some(_) => base.as_ref().map(|b| b.title.clone()),
                        None => None,
                    };
                    Some(EvolutionLane {
                        equipped: equipped_lane.as_deref() == Some(lane_id.as_str()),
                        can_equip: current.is_some() && blocked_by.is_none(),
                        lane_id,
                        base,
                        stages,
                        current,
                        blocked_by,
                        from,
                        line,
                    })
                })
                .collect();
            if lanes.is_empty() {
                return None;
            }
            Some(EvolutionTrack {
                track_id: first.track.clone(),
                campaign_id: first.campaign_id.clone(),
                campaign_name: first.campaign_name.clone(),
                ends_ms: first.end_at.timestamp_millis(),
                equipped_lane,
                lanes,
            })
        })
        .collect();
    EvolutionBoard { tracks: out }
}

/// A line's stages in evolution order, as indices into `titles` (Twitch's
/// order): each comes after the stage it evolves from. A stage whose base is
/// unknown, or outside the lane, keeps its place in Twitch's order. Pure.
pub(crate) fn evolution_order(titles: &[String], base_of: impl Fn(&str) -> Option<String>) -> Vec<usize> {
    let keys: Vec<String> = titles.iter().map(|t| badge_key(t)).collect();
    let bases: Vec<Option<String>> = titles.iter().map(|t| base_of(t).map(|b| badge_key(&b))).collect();
    let mut placed = vec![false; titles.len()];
    let mut order = Vec::with_capacity(titles.len());
    while order.len() < titles.len() {
        // A stage waits while the stage it evolves from is still to be placed.
        let waits = |i: usize| {
            bases[i]
                .as_ref()
                .is_some_and(|b| (0..titles.len()).any(|j| j != i && !placed[j] && keys[j] == *b))
        };
        let Some(next) = (0..titles.len())
            .find(|&i| !placed[i] && !waits(i))
            .or_else(|| (0..titles.len()).find(|&i| !placed[i]))
        else {
            break;
        };
        placed[next] = true;
        order.push(next);
    }
    order
}

/// The badge a stage awards, by its reward's name.
fn stage_title(drop: &TimeBasedDrop) -> String {
    drop.benefit_edges
        .first()
        .map(|b| b.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| drop.name.clone())
}

fn stage(drop: &TimeBasedDrop, badges: &BadgeIndex) -> EvolutionStage {
    let reward = drop.benefit_edges.first();
    let title = stage_title(drop);
    let held = badges.badge(&title);
    let progress = drop.progress.as_ref();
    let twitch = progress.and_then(|p| p.twitch_progress.as_ref());
    let earned = held.as_ref().is_some_and(|b| b.owned) || progress.is_some_and(|p| p.is_claimed);
    let ready_to_claim = !earned && twitch.is_some_and(|t| t.ready_to_claim);
    EvolutionStage {
        group_id: drop.id.clone(),
        image_url: held
            .map(|b| b.image_url)
            .filter(|u| !u.is_empty())
            .or_else(|| reward.map(|b| b.image_url.clone()))
            .unwrap_or_default(),
        title,
        earned,
        days_done: twitch.map(|t| t.days_done).unwrap_or(0).min(drop.required_days.max(1)),
        total_days: drop.required_days.max(1),
        ready_to_claim,
        instance_id: progress.filter(|_| ready_to_claim).and_then(|p| p.drop_instance_id.clone()),
        claim_label: crate::services::drop_words::claim_label(drop).to_string(),
    }
}

/// The global badges the board reads, with what this account holds.
async fn badge_index(app: &crate::rt::AppHandle) -> BadgeIndex {
    let owned: HashSet<String> = crate::services::badge_standing::get_standing(app, false)
        .await
        .owned
        .into_iter()
        .collect();
    let catalogue = crate::services::badge_standing::load_catalogue().await;
    tokio::task::spawn_blocking(move || {
        let metadata = crate::services::badge_standing::load_metadata(&catalogue);
        let mut index = BadgeIndex::default();
        for b in &catalogue {
            let key = badge_key(&b.title);
            let held = owned.contains(&b.key());
            let entry = index
                .by_title
                .entry(key.clone())
                .or_insert_with(|| (b.title.clone(), b.image_url.clone(), false));
            entry.2 |= held;
            if let Some(meta) = metadata.get(&b.key()) {
                if let Some(base) =
                    crate::services::badge_earn::evolves_from(meta.enrichment.as_ref(), meta.more_info.as_deref())
                {
                    index.base_of.insert(key, base);
                }
            }
        }
        index
    })
    .await
    .unwrap_or_default()
}

/// Each evolved badge's title to the title it evolves from. Autopilot orders a
/// line's stages by it, as the board does, since it has no badge copy of its own.
pub(crate) async fn evolution_bases() -> HashMap<String, String> {
    let catalogue = crate::services::badge_standing::load_catalogue().await;
    tokio::task::spawn_blocking(move || {
        let metadata = crate::services::badge_standing::load_metadata(&catalogue);
        catalogue
            .iter()
            .filter_map(|b| {
                let meta = metadata.get(&b.key())?;
                let base =
                    crate::services::badge_earn::evolves_from(meta.enrichment.as_ref(), meta.more_info.as_deref())?;
                Some((b.title.clone(), base))
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

fn cached_selections(tracks: &[String]) -> Option<HashMap<String, String>> {
    let guard = SELECTIONS.lock().ok()?;
    let (at, map) = guard.as_ref()?;
    (at.elapsed().as_secs() < SELECTIONS_SECS).then(|| {
        map.iter().filter(|(t, _)| tracks.contains(t)).map(|(t, l)| (t.clone(), l.clone())).collect()
    })
}

/// The lane equipped on each track, by track id.
async fn selections(
    service: &crate::services::drops_service::DropsService,
    tracks: &[String],
    force: bool,
) -> Result<HashMap<String, String>> {
    if !force {
        if let Some(map) = cached_selections(tracks) {
            return Ok(map);
        }
    }
    let answer = service
        .post_gql(json!({ "query": SELECTIONS_QUERY, "variables": { "tracks": tracks } }))
        .await?;
    let list = answer
        .pointer("/data/trackSelections")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("no trackSelections in the answer: {}", answer["errors"]))?;
    let map: HashMap<String, String> = list
        .iter()
        .filter_map(|s| Some((s["track"].as_str()?.to_string(), s["lane"].as_str()?.to_string())))
        .collect();
    if let Ok(mut c) = SELECTIONS.lock() {
        *c = Some((Instant::now(), map.clone()));
    }
    Ok(map)
}

/// The board for the signed-in Drops account. Empty when nothing can be
/// evolved right now or there is no Drops sign-in, so a surface can hide.
pub async fn board(app: &crate::rt::AppHandle, force: bool) -> EvolutionBoard {
    use tauri::Manager;
    let Some(state) = app.try_state::<crate::models::settings::AppState>() else {
        return EvolutionBoard::default();
    };
    let (campaigns, groups, selected) = {
        let service = state.drops_service.lock().await;
        let campaigns = match service.reward_campaigns(if force { 0 } else { CAMPAIGNS_SECS }).await {
            Ok(c) => c,
            Err(e) => {
                debug!("[Evolution] no second drops list: {}", e);
                return EvolutionBoard::default();
            }
        };
        let groups = crate::services::reward_drops::track_groups();
        let mut tracks: Vec<String> = Vec::new();
        for g in groups.iter().filter(|g| g.lane.is_some()) {
            if !tracks.contains(&g.track) {
                tracks.push(g.track.clone());
            }
        }
        if tracks.is_empty() {
            return EvolutionBoard::default();
        }
        let selected = match selections(&service, &tracks, force).await {
            Ok(s) => s,
            Err(e) => {
                warn!("[Evolution] could not read the equipped lanes: {}", e);
                HashMap::new()
            }
        };
        (campaigns, groups, selected)
    };
    let badges = badge_index(app).await;
    build_board(&campaigns, &groups, &selected, &badges, chrono::Utc::now().timestamp_millis())
}

/// Equip `lane` on `track`, as twitch.tv's Drops panel does. Errors are in words.
pub async fn equip(app: &crate::rt::AppHandle, track: &str, lane: &str) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    let state = app
        .try_state::<crate::models::settings::AppState>()
        .ok_or_else(|| "Drops are still starting. Try again in a moment.".to_string())?;
    let answer = {
        let service = state.drops_service.lock().await;
        service
            .post_gql(json!({
                "query": SET_SELECTION_MUTATION,
                "variables": { "input": { "track": track, "lane": lane } }
            }))
            .await
            .map_err(|e| {
                warn!("[Evolution] equip failed: {}", e);
                "Couldn't reach Twitch. Try again in a moment.".to_string()
            })?
    };
    if answer["errors"].as_array().is_some_and(|e| !e.is_empty()) || answer["data"]["setTrackSelection"].is_null() {
        warn!("[Evolution] equip refused: {}", answer["errors"]);
        return Err("Twitch didn't accept that. Try again in a moment.".to_string());
    }
    let code = &answer["data"]["setTrackSelection"]["error"];
    if !code.is_null() {
        warn!("[Evolution] equip refused with {}", code);
        return Err("Twitch didn't accept that Pokémon right now.".to_string());
    }
    if let Ok(mut c) = SELECTIONS.lock() {
        let map = c.get_or_insert_with(|| (Instant::now(), HashMap::new()));
        map.1.insert(track.to_string(), lane.to_string());
    }
    let _ = app.emit("badge-standing-changed", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    const TRACK: &str = "pokemon#t";

    fn at(iso: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(iso).unwrap().with_timezone(&Utc)
    }

    fn group(id: &str, lane: Option<&str>) -> TrackGroup {
        TrackGroup {
            campaign_id: "evo".into(),
            campaign_name: "Pokemon Evolutions".into(),
            end_at: at("2027-01-30T22:59:59.999Z"),
            group_id: id.into(),
            track: if lane.is_some() { TRACK.into() } else { "balls#b".into() },
            lane: lane.map(str::to_string),
        }
    }

    /// One stage as `reward_drops` builds it: 20 minutes on 3 days.
    fn stage_drop(id: &str, reward: &str, progress: Value) -> Value {
        json!({ "id": id, "name": reward, "required_minutes_watched": 60, "required_days": 3,
                "benefit_edges": [{ "id": format!("r-{reward}"), "name": reward, "image_url": format!("https://art/{reward}.png") }],
                "progress": progress })
    }

    fn progress(id: &str, days: u32, today: u32, ready: bool, claimed: bool) -> Value {
        json!({ "campaign_id": "evo", "drop_id": id, "current_minutes_watched": days * 20 + today,
                "required_minutes_watched": 60, "is_claimed": claimed, "last_updated": "2026-10-21T12:00:00Z",
                "drop_instance_id": if ready { Some(format!("u#evo#{id}")) } else { None },
                "twitch_progress": { "days_done": days, "minutes_today": today, "window_expires_at": null,
                                     "accruing": false, "subs_done": 0, "ready_to_claim": ready, "earned": null } })
    }

    fn campaign(drops: Vec<Value>) -> DropCampaign {
        serde_json::from_value(json!({
            "id": "evo", "name": "Pokemon Evolutions", "game_id": "twitch-rewards-pokemon", "game_name": "Pokemon",
            "description": "", "image_url": "", "start_at": "2026-10-16T17:00:00Z", "end_at": "2027-01-30T22:59:59.999Z",
            "time_based_drops": drops
        }))
        .unwrap()
    }

    fn index(owned: &[&str]) -> BadgeIndex {
        let mut i = BadgeIndex::default();
        for t in ["Gastly", "Haunter", "Gengar", "Pichu", "Pikachu"] {
            i.by_title.insert(badge_key(t), (t.into(), format!("https://cdn/{t}"), owned.contains(&t)));
        }
        i.base_of.insert(badge_key("Haunter"), "Gastly".into());
        i.base_of.insert(badge_key("Gengar"), "Haunter".into());
        i.base_of.insert(badge_key("Pikachu"), "Pichu".into());
        i
    }

    fn board_for(owned: &[&str], selected: Option<&str>, haunter: Value) -> EvolutionBoard {
        let campaigns = vec![campaign(vec![
            stage_drop("g-haunter", "Haunter", haunter),
            stage_drop("g-gengar", "Gengar", Value::Null),
            stage_drop("g-pikachu", "Pikachu", Value::Null),
            stage_drop("g-ball", "Pokémon Night Out Themed Poke Ball", Value::Null),
        ])];
        let groups = vec![
            group("g-haunter", Some("lane-gastly")),
            group("g-gengar", Some("lane-gastly")),
            group("g-pikachu", Some("lane-pichu")),
            group("g-ball", None),
        ];
        let mut selections = HashMap::new();
        if let Some(lane) = selected {
            selections.insert(TRACK.to_string(), lane.to_string());
        }
        build_board(&campaigns, &groups, &selections, &index(owned), at("2026-10-21T12:00:00Z").timestamp_millis())
    }

    #[test]
    fn lanes_are_lines_and_the_balls_are_left_out() {
        let b = board_for(&["Gastly"], None, Value::Null);
        assert_eq!(b.tracks.len(), 1);
        let t = &b.tracks[0];
        assert_eq!(t.equipped_lane, None);
        assert_eq!(t.ends_ms, at("2027-01-30T22:59:59.999Z").timestamp_millis());
        let lanes: Vec<(&str, Vec<&str>)> = t
            .lanes
            .iter()
            .map(|l| (l.lane_id.as_str(), l.stages.iter().map(|s| s.title.as_str()).collect()))
            .collect();
        assert_eq!(lanes, vec![("lane-gastly", vec!["Haunter", "Gengar"]), ("lane-pichu", vec!["Pikachu"])]);
        assert!(t.lanes.iter().all(|l| !l.equipped));
    }

    #[test]
    fn a_line_starts_only_from_a_held_base() {
        let b = board_for(&["Gastly"], None, Value::Null);
        let gastly = &b.tracks[0].lanes[0];
        assert_eq!(gastly.current, Some(0));
        assert_eq!(gastly.base.as_ref().map(|b| (b.title.as_str(), b.owned)), Some(("Gastly", true)));
        assert_eq!(gastly.from.as_deref(), Some("Gastly"));
        assert!(gastly.can_equip && gastly.blocked_by.is_none());
        let pichu = &b.tracks[0].lanes[1];
        assert_eq!(pichu.blocked_by.as_deref(), Some("Pichu"));
        assert!(!pichu.can_equip);
    }

    #[test]
    fn an_unknown_base_lets_twitch_decide() {
        let campaigns = vec![campaign(vec![stage_drop("g-x", "Mew", Value::Null)])];
        let groups = vec![group("g-x", Some("lane-x"))];
        let b = build_board(&campaigns, &groups, &HashMap::new(), &BadgeIndex::default(), 0);
        let lane = &b.tracks[0].lanes[0];
        assert!(lane.base.is_none() && lane.can_equip);
    }

    #[test]
    fn the_current_stage_moves_on_once_the_first_is_held() {
        let b = board_for(&["Gastly", "Haunter"], Some("lane-gastly"), Value::Null);
        let gastly = &b.tracks[0].lanes[0];
        assert!(gastly.equipped);
        assert_eq!(gastly.current, Some(1));
        assert_eq!(gastly.from.as_deref(), Some("Haunter"));
        assert!(gastly.blocked_by.is_none() && gastly.can_equip);
        let done = board_for(&["Gastly", "Haunter", "Gengar"], None, Value::Null);
        let lane = &done.tracks[0].lanes[0];
        assert_eq!(lane.current, None);
        assert!(!lane.can_equip);
    }

    #[test]
    fn progress_reads_in_days_and_a_finished_stage_offers_its_claim() {
        let b = board_for(&["Gastly"], Some("lane-gastly"), progress("g-haunter", 1, 5, false, false));
        let lane = &b.tracks[0].lanes[0];
        assert_eq!((lane.stages[0].days_done, lane.stages[0].total_days), (1, 3));
        assert_eq!(lane.line.as_deref().map(|l| l.starts_with("Day 2 of 3 · 5/20 min today")), Some(true));
        let ready = board_for(&["Gastly"], Some("lane-gastly"), progress("g-haunter", 3, 20, true, false));
        let s = &ready.tracks[0].lanes[0].stages[0];
        assert!(s.ready_to_claim);
        assert_eq!(s.instance_id.as_deref(), Some("u#evo#g-haunter"));
    }

    #[test]
    fn a_line_is_ordered_by_what_each_stage_evolves_from() {
        // Twitch lists the Bulbasaur lane as Venusaur, then Ivysaur.
        let campaigns = vec![campaign(vec![
            stage_drop("g-venusaur", "Venusaur", Value::Null),
            stage_drop("g-ivysaur", "Ivysaur", Value::Null),
        ])];
        let groups = vec![group("g-venusaur", Some("lane-bulbasaur")), group("g-ivysaur", Some("lane-bulbasaur"))];
        let mut badges = BadgeIndex::default();
        for (t, owned) in [("Bulbasaur", true), ("Ivysaur", false), ("Venusaur", false)] {
            badges.by_title.insert(badge_key(t), (t.into(), format!("https://cdn/{t}"), owned));
        }
        badges.base_of.insert(badge_key("Ivysaur"), "Bulbasaur".into());
        badges.base_of.insert(badge_key("Venusaur"), "Ivysaur".into());
        let b = build_board(&campaigns, &groups, &HashMap::new(), &badges, 0);
        let lane = &b.tracks[0].lanes[0];
        let titles: Vec<&str> = lane.stages.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Ivysaur", "Venusaur"]);
        assert_eq!(lane.current, Some(0));
        assert_eq!(lane.base.as_ref().map(|b| (b.title.as_str(), b.owned)), Some(("Bulbasaur", true)));
        assert_eq!(lane.from.as_deref(), Some("Bulbasaur"));
        assert!(lane.can_equip && lane.blocked_by.is_none());
    }

    #[test]
    fn an_unknown_chain_keeps_twitchs_order() {
        let titles: Vec<String> = ["Gengar", "Haunter", "Mew"].iter().map(|s| s.to_string()).collect();
        let base = |t: &str| (t == "Gengar").then(|| "Haunter".to_string());
        assert_eq!(evolution_order(&titles, base), vec![1, 0, 2]);
        assert_eq!(evolution_order(&titles, |_| None), vec![0, 1, 2]);
        // Two stages naming each other cannot loop.
        let cycle = |t: &str| Some(if t == "Gengar" { "Haunter" } else { "Gengar" }.to_string());
        assert_eq!(evolution_order(&titles[..2], cycle).len(), 2);
    }

    #[test]
    fn nothing_to_evolve_is_an_empty_board() {
        let b = build_board(&[], &[group("g-ball", None)], &HashMap::new(), &BadgeIndex::default(), 0);
        assert!(b.tracks.is_empty());
    }
}
