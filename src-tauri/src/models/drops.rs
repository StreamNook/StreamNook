use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropCampaign {
    pub id: String,
    pub name: String,
    pub game_id: String,
    pub game_name: String,
    pub description: String,
    pub image_url: String,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub time_based_drops: Vec<TimeBasedDrop>,
    #[serde(default)]
    pub is_account_connected: bool,
    #[serde(default)]
    pub allowed_channels: Vec<AllowedChannel>,
    #[serde(default)]
    pub is_acl_based: bool,
    #[serde(default)]
    pub details_url: Option<String>, // "About this drop" link
    #[serde(default)]
    pub account_link: Option<String>, // publisher "connect account" URL (Twitch accountLinkURL)
    /// Each reward keeps its own progress instead of sharing one running
    /// watch-time count (the container and daily-watch drops Twitch lists
    /// apart from its standard campaign list).
    #[serde(default)]
    pub separate_progress: bool,
    /// `game_id` is a real Twitch category. False for a campaign that runs
    /// across categories, whose `game_id` only groups it for display; such an
    /// id must never reach a category lookup.
    #[serde(default = "default_true")]
    pub has_category: bool,
    /// Every category a stream can be live in for this campaign to credit:
    /// Twitch's own list for a drop that counts in several (the Pokémon balls
    /// count in 21). Empty means `game_id` alone.
    #[serde(default)]
    pub category_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedChannel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeBasedDrop {
    pub id: String,
    pub name: String,
    /// Watch time required in minutes. Twitch API returns as "requiredMinutesWatched" (camelCase)
    #[serde(alias = "requiredMinutesWatched", default)]
    pub required_minutes_watched: i32,
    /// Benefit info. Twitch API returns as "benefitEdges" (camelCase)
    #[serde(alias = "benefitEdges", default)]
    pub benefit_edges: Vec<DropBenefit>,
    /// Progress data for this drop - NOT renamed to "self" so frontend can access as "progress"
    #[serde(default)]
    pub progress: Option<DropProgress>,
    /// Whether this drop can be auto-collected (time-based drops with required_minutes > 0)
    /// Drops with required_minutes_watched == 0 are event-based, badge-based, or require
    /// special actions (subscriptions, purchases, etc.) and cannot be auto-collected
    #[serde(default = "default_collectible", alias = "is_mineable")]
    pub is_collectible: bool,
    /// Subscriptions (or gifted subs) the reward needs. 0 = none.
    #[serde(default)]
    pub required_subs: u32,
    /// Separate days the watch time must be met on (Twitch's rolling 24-hour
    /// windows). 0 or 1 = once; `required_minutes_watched` is the total.
    #[serde(default)]
    pub required_days: u32,
    /// The reward is one drawn at random from this many.
    #[serde(default)]
    pub random_of: Option<u32>,
    /// A draw with exactly one reward left the viewer does not hold: what it
    /// will give. Twitch never repeats a reward, so it is certain.
    #[serde(default)]
    pub next_reward: Option<DropBenefit>,
}

fn default_collectible() -> bool {
    true
}

impl TimeBasedDrop {
    /// Calculate if this drop is collectible based on required minutes
    pub fn calculate_is_collectible(&self) -> bool {
        self.required_minutes_watched > 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DropBenefit {
    pub id: String,
    pub name: String,
    /// Image URL for the benefit. Twitch API returns as "imageAssetURL" (camelCase)
    #[serde(alias = "imageAssetURL", default)]
    pub image_url: String,
    /// Distribution type - "BADGE" for Twitch chat badges, "DIRECT_ENTITLEMENT" for in-game items
    /// Badge drops cannot be verified via gameEventDrops API, only in-game items can
    #[serde(default)]
    pub distribution_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropProgress {
    pub campaign_id: String,
    pub drop_id: String,
    pub current_minutes_watched: i32,
    pub required_minutes_watched: i32,
    pub is_claimed: bool,
    pub last_updated: DateTime<Utc>,
    /// The dropInstanceID from Twitch API - required for claiming drops
    /// Format is typically: "UserID#CampaignID#DropID" or a unique ID
    #[serde(default)]
    pub drop_instance_id: Option<String>,
    /// Twitch's own progress detail for multi-day and subscription drops.
    #[serde(default)]
    pub twitch_progress: Option<TwitchProgress>,
}

/// Progress Twitch reports for a drop beyond plain minutes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TwitchProgress {
    pub days_done: u32,
    /// Minutes in the current day window; 0 once that window has lapsed.
    pub minutes_today: u32,
    pub window_expires_at: Option<DateTime<Utc>>,
    /// Minutes rose within the last few minutes: this is being earned now.
    pub accruing: bool,
    pub subs_done: u32,
    /// Earned and waiting to be claimed (Twitch's `CLAIMABLE`).
    pub ready_to_claim: bool,
    /// What the viewer got (or holds to claim): for a random draw, the one
    /// reward that came out.
    #[serde(default)]
    pub earned: Option<DropBenefit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedDrop {
    pub id: String,
    pub campaign_id: String,
    pub drop_id: String,
    pub drop_name: String,
    pub game_name: String,
    pub benefit_name: String,
    pub benefit_image_url: String,
    pub claimed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPointsBalance {
    pub channel_id: String,
    pub channel_name: String,
    pub balance: i32,
    pub last_updated: DateTime<Utc>,
    /// Custom channel points name (e.g., "Kisses" for Hamlinz). None = default "Channel Points"
    #[serde(default)]
    pub points_name: Option<String>,
    /// Custom channel points icon URL. None = uses default Twitch icon
    #[serde(default)]
    pub points_icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPointsClaim {
    pub id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub points_earned: i32,
    pub claimed_at: DateTime<Utc>,
    pub claim_type: ChannelPointsClaimType,
}

/// Result of claiming a bonus chest. `points_earned` is the exact amount the
/// claim credited (`claim.pointsEarnedTotal` in the mutation response, which
/// includes any active multipliers), and `new_balance` is the post-claim
/// total. Both come straight from the `ClaimCommunityPoints` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BonusClaimResult {
    pub new_balance: i32,
    pub points_earned: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelPointsClaimType {
    Watch,      // Regular watch time bonus
    Raid,       // Participated in raid
    Prediction, // Prediction reward
    Bonus,      // Bonus chest claim
    Other,
}

/// A custom channel reward that users can redeem with channel points
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelReward {
    pub id: String,
    pub title: String,
    pub cost: i32,
    pub prompt: Option<String>,
    pub image_url: Option<String>,
    pub background_color: String,
    pub is_enabled: bool,
    pub is_paused: bool,
    pub is_in_stock: bool,
    pub is_user_input_required: bool,
    pub cooldown_expires_at: Option<String>,
    pub max_per_stream: Option<i32>,
    pub max_per_user_per_stream: Option<i32>,
    pub global_cooldown_seconds: Option<i32>,
}

/// Result of attempting to redeem a channel reward
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedemptionResult {
    pub success: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub new_balance: Option<i32>,
    /// For emote unlock rewards - contains info about the unlocked emote
    #[serde(default)]
    pub unlocked_emote: Option<UnlockedEmote>,
}

/// Information about an unlocked emote (for random emote unlock rewards)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnlockedEmote {
    pub id: String,
    pub name: String,
    pub image_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropsStatistics {
    pub total_drops_claimed: i32,
    pub total_channel_points_earned: i32,
    pub active_campaigns: i32,
    pub drops_in_progress: i32,
    pub recent_claims: Vec<ClaimedDrop>,
    pub channel_points_history: Vec<ChannelPointsClaim>,
}

/// Represents a reserved watch slot for the current stream (in-memory, not persisted)
/// Used to ensure one watch token always goes to the stream the user is actively watching
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReservedStreamSlot {
    pub channel_id: Option<String>,
    pub channel_login: Option<String>,
    pub reserved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityChannel {
    pub channel_id: String,
    pub channel_login: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropsSettings {
    pub auto_claim_drops: bool,
    pub auto_claim_channel_points: bool,
    pub notify_on_drop_available: bool,
    pub notify_on_drop_claimed: bool,
    pub notify_on_points_claimed: bool,
    pub check_interval_seconds: u64,
    // Automation-specific settings
    #[serde(alias = "auto_mining_enabled")]
    pub automation_enabled: bool,
    pub priority_games: Vec<String>,
    pub excluded_games: HashSet<String>,
    pub priority_mode: PriorityMode,
    pub watch_interval_seconds: u64,
    // UI-only settings (not used for automation logic)
    /// Games the user has favorited for visual tracking - sorts them to top of list
    /// This is separate from priority_games which affects automation behavior
    #[serde(default)]
    pub favorite_games: Vec<String>,
    // Watch token allocation settings
    /// When TRUE (default), one watch token is always reserved for the currently-watched stream
    /// This ensures presence in chat for gifted sub eligibility
    /// Power users can set this FALSE to reclaim the token for more efficient channel points collection
    #[serde(default = "default_true")]
    pub reserve_token_for_current_stream: bool,
    /// When TRUE (default), automatically reserves token when user starts watching a stream
    /// When FALSE, user must manually trigger reservation
    #[serde(default = "default_true")]
    pub auto_reserve_on_watch: bool,
    /// Channels the user wants to prioritize for background channel-points collection.
    /// When non-empty, rotation only selects from this list (falls back to all if none are live)
    #[serde(default, alias = "priority_farm_channels")]
    pub priority_channels: Vec<PriorityChannel>,
    /// When true, background collection ignores `priority_channels` and instead
    /// rotates through the channels the user has favorited that are currently
    /// live. The favorites themselves live in `AppSettings.favorite_streamers`;
    /// the host tags each live channel it hands the plugin with a `favorite`
    /// flag, so the plugin just filters to those.
    #[serde(default, alias = "farm_from_favorites")]
    pub prefer_favorites: bool,
    /// When true, drops automation works first on campaigns that award a
    /// global badge the account is missing and can earn by watching now.
    #[serde(default = "default_true")]
    pub prefer_missing_badges: bool,
    // Recovery settings
    #[serde(default)]
    pub recovery_settings: RecoverySettings,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PriorityMode {
    PriorityOnly,  // Only collect priority games
    EndingSoonest, // Prioritize campaigns ending soon
    LowAvailFirst, // Prioritize low availability campaigns
}

impl Default for DropsSettings {
    fn default() -> Self {
        Self {
            auto_claim_drops: true,
            auto_claim_channel_points: true,
            notify_on_drop_available: true,
            notify_on_drop_claimed: true,
            notify_on_points_claimed: false,
            check_interval_seconds: 60,
            // Automation defaults
            automation_enabled: false,
            priority_games: Vec::new(),
            excluded_games: HashSet::new(),
            priority_mode: PriorityMode::PriorityOnly,
            watch_interval_seconds: 20,
            // UI defaults
            favorite_games: Vec::new(),
            // Watch token allocation defaults (ON by default - matches Twitch native behavior)
            reserve_token_for_current_stream: true,
            auto_reserve_on_watch: true,
            // Priority channels default (empty = rotate all followed)
            priority_channels: Vec::new(),
            // Prefer-favorites off by default (uses the priority list / all followed)
            prefer_favorites: false,
            // Missing badges first, on by default: it only reorders campaigns
            prefer_missing_badges: true,
            // Recovery defaults
            recovery_settings: RecoverySettings::default(),
        }
    }
}

// ============================================
// RECOVERY SYSTEM MODELS
// ============================================

/// Settings for automatic collection recovery behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverySettings {
    /// How long without progress before considering collection "stale" (in seconds)
    /// Default: 420 (7 minutes)
    pub stale_progress_threshold_seconds: u64,
    /// How long to blacklist a streamer after issues (in seconds)
    /// Default: 600 (10 minutes)
    pub streamer_blacklist_duration_seconds: u64,
    /// How long to deprioritize a stuck campaign (in seconds)
    /// Default: 1800 (30 minutes)
    pub campaign_deprioritize_duration_seconds: u64,
    /// Interval for checking stream status (separate from watch payload) (in seconds)
    /// Default: 180 (3 minutes)
    pub stream_status_check_interval_seconds: u64,
    /// Recovery behavior mode
    pub recovery_mode: RecoveryMode,
    /// Whether to notify user when recovery actions are taken
    pub notify_on_recovery_action: bool,
    /// Whether to detect and handle game category changes
    pub detect_game_category_change: bool,
}

impl Default for RecoverySettings {
    fn default() -> Self {
        Self {
            stale_progress_threshold_seconds: 420,        // 7 minutes
            streamer_blacklist_duration_seconds: 600,     // 10 minutes
            campaign_deprioritize_duration_seconds: 1800, // 30 minutes
            stream_status_check_interval_seconds: 180,    // 3 minutes
            recovery_mode: RecoveryMode::Automatic,
            notify_on_recovery_action: true,
            detect_game_category_change: true,
        }
    }
}

/// Recovery behavior mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecoveryMode {
    /// Automatically switch streamers/campaigns when issues detected (5-7 min threshold)
    Automatic,
    /// More relaxed thresholds (15 min), notify first before auto-switching
    Relaxed,
    /// Only notify user, never auto-switch
    ManualOnly,
}

/// A temporarily blacklisted streamer (in-memory, not persisted)
#[derive(Debug, Clone)]
pub struct BlacklistedStreamer {
    pub channel_id: String,
    pub channel_name: String,
    pub reason: BlacklistReason,
    pub blacklisted_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Reason why a streamer was blacklisted
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlacklistReason {
    /// Multiple watch payload failures
    WatchPayloadFailures,
    /// Streamer went offline
    WentOffline,
    /// Progress stalled while watching
    StaleProgress,
    /// Streamer changed to non-drops game category
    GameCategoryChanged,
}

/// A temporarily deprioritized campaign
#[derive(Debug, Clone)]
pub struct DeprioritizedCampaign {
    pub campaign_id: String,
    pub campaign_name: String,
    pub game_name: String,
    pub reason: DeprioritizeReason,
    pub deprioritized_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Reason why a campaign was deprioritized
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeprioritizeReason {
    /// No online streamers available
    NoStreamersAvailable,
    /// All streamers for this campaign failed
    AllStreamersFailed,
    /// Progress stalled across multiple streamers
    StaleProgressPersistent,
}

/// Event emitted when a recovery action is taken
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryEvent {
    pub event_type: RecoveryEventType,
    pub timestamp: DateTime<Utc>,
    pub details: RecoveryEventDetails,
}

/// Types of recovery events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryEventType {
    StreamerSwitched,
    StreamerBlacklisted,
    CampaignDeprioritized,
    CampaignRotated,
    StaleProgressDetected,
    GameCategoryChanged,
    StreamerWentOffline,
}

/// Details about a recovery event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryEventDetails {
    pub from_channel: Option<String>,
    pub to_channel: Option<String>,
    pub from_campaign: Option<String>,
    pub to_campaign: Option<String>,
    pub reason: String,
}

/// Extended automation status with recovery tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationStatusExtended {
    /// Base automation status
    #[serde(flatten)]
    pub base: AutomationStatus,
    /// When progress last increased (for stale detection)
    pub last_progress_increase_at: Option<DateTime<Utc>>,
    /// Last known progress value (for detecting increases)
    pub last_known_progress_minutes: i32,
    /// Current streamer's game category (for change detection)
    pub current_streamer_game: Option<String>,
    /// Recent recovery events (for UI display)
    pub recent_recovery_events: Vec<RecoveryEvent>,
}

impl Default for AutomationStatusExtended {
    fn default() -> Self {
        Self {
            base: AutomationStatus::default(),
            last_progress_increase_at: None,
            last_known_progress_minutes: 0,
            current_streamer_game: None,
            recent_recovery_events: Vec::new(),
        }
    }
}

/// Tracking state for the recovery watchdog (in-memory)
#[derive(Debug, Clone)]
pub struct RecoveryWatchdogState {
    /// Blacklisted streamers (temporary, in-memory)
    pub blacklisted_streamers: HashMap<String, BlacklistedStreamer>,
    /// Deprioritized campaigns (temporary, in-memory)
    pub deprioritized_campaigns: HashMap<String, DeprioritizedCampaign>,
    /// When the last stream status check was performed
    pub last_stream_status_check: Option<DateTime<Utc>>,
    /// When progress last increased
    pub last_progress_increase_at: Option<DateTime<Utc>>,
    /// Last known progress value
    pub last_known_progress_minutes: i32,
    /// The expected game category for the current campaign
    pub expected_game_category: Option<String>,
}

impl Default for RecoveryWatchdogState {
    fn default() -> Self {
        Self {
            blacklisted_streamers: HashMap::new(),
            deprioritized_campaigns: HashMap::new(),
            last_stream_status_check: None,
            last_progress_increase_at: None,
            last_known_progress_minutes: 0,
            expected_game_category: None,
        }
    }
}

impl RecoveryWatchdogState {
    /// Check if a streamer is currently blacklisted
    pub fn is_streamer_blacklisted(&self, channel_id: &str) -> bool {
        if let Some(blacklisted) = self.blacklisted_streamers.get(channel_id) {
            Utc::now() < blacklisted.expires_at
        } else {
            false
        }
    }

    /// Check if a campaign is currently deprioritized
    pub fn is_campaign_deprioritized(&self, campaign_id: &str) -> bool {
        if let Some(deprioritized) = self.deprioritized_campaigns.get(campaign_id) {
            Utc::now() < deprioritized.expires_at
        } else {
            false
        }
    }

    /// Add a streamer to the blacklist
    pub fn blacklist_streamer(
        &mut self,
        channel_id: String,
        channel_name: String,
        reason: BlacklistReason,
        duration_seconds: u64,
    ) {
        let now = Utc::now();
        self.blacklisted_streamers.insert(
            channel_id.clone(),
            BlacklistedStreamer {
                channel_id,
                channel_name,
                reason,
                blacklisted_at: now,
                expires_at: now + chrono::Duration::seconds(duration_seconds as i64),
            },
        );
    }

    /// Deprioritize a campaign
    pub fn deprioritize_campaign(
        &mut self,
        campaign_id: String,
        campaign_name: String,
        game_name: String,
        reason: DeprioritizeReason,
        duration_seconds: u64,
    ) {
        let now = Utc::now();
        self.deprioritized_campaigns.insert(
            campaign_id.clone(),
            DeprioritizedCampaign {
                campaign_id,
                campaign_name,
                game_name,
                reason,
                deprioritized_at: now,
                expires_at: now + chrono::Duration::seconds(duration_seconds as i64),
            },
        );
    }

    /// Clean up expired blacklist/deprioritize entries
    pub fn cleanup_expired(&mut self) {
        let now = Utc::now();
        self.blacklisted_streamers.retain(|_, v| v.expires_at > now);
        self.deprioritized_campaigns
            .retain(|_, v| v.expires_at > now);
    }

    /// Update progress tracking - returns true if progress increased
    pub fn update_progress(&mut self, new_progress_minutes: i32) -> bool {
        let increased = new_progress_minutes > self.last_known_progress_minutes;
        if increased {
            self.last_progress_increase_at = Some(Utc::now());
        }
        self.last_known_progress_minutes = new_progress_minutes;
        increased
    }

    /// Check if progress is stale based on threshold
    pub fn is_progress_stale(&self, threshold_seconds: u64) -> bool {
        if let Some(last_increase) = self.last_progress_increase_at {
            let stale_threshold = chrono::Duration::seconds(threshold_seconds as i64);
            Utc::now() - last_increase > stale_threshold
        } else {
            // If we've never seen progress increase but have been running, consider it stale
            // after the threshold
            false
        }
    }
}

// Channel information for automation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveChannel {
    pub id: String,
    #[serde(rename = "display_name")]
    pub name: String,
    pub game_id: String,
    pub game_name: String,
    #[serde(rename = "viewer_count")]
    pub viewers: i32,
    pub drops_enabled: bool,
    #[serde(rename = "is_live")]
    pub is_online: bool,
    pub is_acl_based: bool,
}

// Automation status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationStatus {
    pub is_running: bool,
    pub current_channel: Option<ActiveChannel>,
    pub current_campaign: Option<String>,
    pub current_drop: Option<CurrentDropInfo>,
    pub eligible_channels: Vec<ActiveChannel>,
    pub last_update: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentDropInfo {
    pub drop_id: String,
    pub drop_name: String,
    pub drop_image: Option<String>, // Benefit image URL for display
    pub campaign_name: String,
    pub game_name: String,
    pub current_minutes: i32,
    pub required_minutes: i32,
    pub progress_percentage: f32,
    pub estimated_completion: Option<DateTime<Utc>>,
}

impl Default for AutomationStatus {
    fn default() -> Self {
        Self {
            is_running: false,
            current_channel: None,
            current_campaign: None,
            current_drop: None,
            eligible_channels: Vec::new(),
            last_update: Utc::now(),
        }
    }
}

// GQL Models for fetching inventory
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GqlInventoryResponse {
    pub data: GqlData,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GqlData {
    pub current_user: CurrentUser,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CurrentUser {
    pub inventory: Inventory,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub drop_campaigns_in_progress: Option<Vec<DropCampaignInProgress>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DropCampaignInProgress {
    pub id: String,
    pub name: String,
    pub status: String,
    pub game: Game,
    #[serde(rename = "self")]
    pub account_info: AccountInfo,
    pub time_based_drops: Vec<GqlTimeBasedDrop>,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub is_account_connected: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: String,
    pub name: String,
    pub box_art_url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GqlTimeBasedDrop {
    pub id: String,
    pub name: String,
    pub required_minutes_watched: i32,
    #[serde(rename = "self")]
    pub drop_instance: Option<DropInstance>,
    pub benefit_edges: Vec<BenefitEdge>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DropInstance {
    pub id: String,
    pub current_minutes_watched: i32,
    pub is_claimed: bool,
    pub drop_instance_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BenefitEdge {
    pub benefit: Benefit,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Benefit {
    pub id: String,
    pub name: String,
    pub image_asset_url: String,
}

// Inventory-specific models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryItem {
    pub campaign: DropCampaign,
    pub status: CampaignStatus,
    pub progress_percentage: f32,
    pub total_drops: i32,
    pub claimed_drops: i32,
    pub drops_in_progress: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CampaignStatus {
    Active,
    Upcoming,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryResponse {
    pub items: Vec<InventoryItem>,
    pub total_campaigns: i32,
    pub active_campaigns: i32,
    pub upcoming_campaigns: i32,
    pub expired_campaigns: i32,
    pub completed_drops: Vec<CompletedDrop>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GameEventDrop {
    pub id: String,
    pub last_awarded_at: DateTime<Utc>,
}

/// Represents a completed/claimed drop from the user's permanent inventory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedDrop {
    pub id: String,
    pub name: String,
    pub image_url: String,
    pub game_name: Option<String>,
    pub is_connected: bool,
    pub required_account_link: Option<String>,
    pub last_awarded_at: DateTime<Utc>,
    pub total_count: i32,
}
