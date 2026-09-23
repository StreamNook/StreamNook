use crate::structs::proto::messages::{
    WebcastCaptionMessage, WebcastChatMessage, WebcastControlMessage, WebcastEnvelopeMessage,
    WebcastGiftMessage, WebcastGiftPanelUpdateMessage, WebcastGoalUpdateMessage,
    WebcastGuideMessage, WebcastImDeleteMessage, WebcastInRoomBannerMessage, WebcastLikeMessage,
    WebcastLinkLayerMessage, WebcastLinkMessage, WebcastLinkMicArmies, WebcastLinkMicBattle,
    WebcastLinkMicLayoutStateMessage, WebcastLinkMicMethod, WebcastLiveIntroMessage,
    WebcastMemberMessage, WebcastPollMessage, WebcastRankUpdateMessage, WebcastRoomMessage,
    WebcastRoomPinMessage, WebcastRoomUserSeqMessage, WebcastSocialMessage,
    WebcastUnauthorizedMemberMessage,
};
use crate::structs::proto::messages_ext::{
    WebcastAccessControlMessage, WebcastAccessRecallMessage, WebcastAlertBoxAuditResultMessage,
    WebcastBarrageMessage, WebcastBindingGiftMessage, WebcastBoostCardMessage,
    WebcastBottomMessage, WebcastEmoteChatMessage, WebcastGameRankNotifyMessage,
    WebcastGiftBroadcastMessage, WebcastGiftDynamicRestrictionMessage, WebcastGiftPromptMessage,
    WebcastHourlyRankMessage, WebcastLinkMicBattlePunishFinish, WebcastLinkMicFanTicketMethod,
    WebcastLinkStateMessage, WebcastLinkmicBattleTaskMessage, WebcastLiveGameIntroMessage,
    WebcastMarqueeAnnouncementMessage, WebcastMsgDetectMessage, WebcastNoticeMessage,
    WebcastNotifyMessage, WebcastOecLiveShoppingMessage, WebcastPartnershipDropsUpdateMessage,
    WebcastPartnershipGameOfflineMessage, WebcastPartnershipPunishMessage,
    WebcastPerceptionMessage, WebcastQuestionNewMessage, WebcastRankTextMessage,
    WebcastRoomVerifyMessage, WebcastSpeakerMessage, WebcastSubCapsuleMessage,
    WebcastSubNotifyMessage, WebcastSubPinEventMessage, WebcastSubscriptionNotifyMessage,
    WebcastSystemMessage, WebcastToastMessage, WebcastViewerPicksUpdateMessage,
};

/// Events received from a TikTok Live stream.
///
/// ## Gift streaks
///
/// Some gifts are "combo" gifts that fire multiple events during a streak.
/// Use [`WebcastGiftMessage::is_combo_gift`] and [`WebcastGiftMessage::is_streak_over`]
/// to handle them correctly. See the `gift_tracker` example for a complete pattern.
///
/// ## Convenience events
///
/// `Follow`, `Share`, `Join`, `LiveEnded` are sub-routed from raw events.
/// Both the raw event AND the convenience event fire for the same message.
#[derive(Clone, Debug)]
pub enum TikTokLiveEvent {
    // lifecycle
    Connected { room_id: String },
    Reconnecting { attempt: u32, max_retries: u32, delay_secs: u64 },
    Disconnected,

    // core events
    Chat(WebcastChatMessage),
    Gift(WebcastGiftMessage),
    Like(WebcastLikeMessage),
    Member(WebcastMemberMessage),
    Social(WebcastSocialMessage),
    RoomUserSeq(WebcastRoomUserSeqMessage),
    Control(WebcastControlMessage),

    // sub-routed convenience events (also fire raw event above)
    Follow(WebcastSocialMessage),
    Share(WebcastSocialMessage),
    Join(WebcastMemberMessage),
    LiveEnded(WebcastControlMessage),

    // useful events
    LiveIntro(WebcastLiveIntroMessage),
    RoomMessage(WebcastRoomMessage),
    Caption(WebcastCaptionMessage),
    GoalUpdate(WebcastGoalUpdateMessage),
    ImDelete(WebcastImDeleteMessage),

    // niche events
    RankUpdate(WebcastRankUpdateMessage),
    Poll(WebcastPollMessage),
    Envelope(WebcastEnvelopeMessage),
    RoomPin(WebcastRoomPinMessage),
    UnauthorizedMember(WebcastUnauthorizedMemberMessage),
    LinkMicMethod(WebcastLinkMicMethod),
    LinkMicBattle(WebcastLinkMicBattle),
    LinkMicArmies(WebcastLinkMicArmies),
    LinkMessage(WebcastLinkMessage),
    LinkLayer(WebcastLinkLayerMessage),
    LinkMicLayoutState(WebcastLinkMicLayoutStateMessage),
    GiftPanelUpdate(WebcastGiftPanelUpdateMessage),
    InRoomBanner(WebcastInRoomBannerMessage),
    Guide(WebcastGuideMessage),

    // extended events
    EmoteChat(WebcastEmoteChatMessage),
    QuestionNew(WebcastQuestionNewMessage),
    SubNotify(WebcastSubNotifyMessage),
    Barrage(WebcastBarrageMessage),
    HourlyRank(WebcastHourlyRankMessage),
    MsgDetect(WebcastMsgDetectMessage),
    LinkMicFanTicket(WebcastLinkMicFanTicketMethod),
    RoomVerify(WebcastRoomVerifyMessage),
    OecLiveShopping(WebcastOecLiveShoppingMessage),
    GiftBroadcast(WebcastGiftBroadcastMessage),
    RankText(WebcastRankTextMessage),
    GiftDynamicRestriction(WebcastGiftDynamicRestrictionMessage),
    ViewerPicksUpdate(WebcastViewerPicksUpdateMessage),

    // secondary events
    SystemMessage(WebcastSystemMessage),
    LiveGameIntro(WebcastLiveGameIntroMessage),
    AccessControl(WebcastAccessControlMessage),
    AccessRecall(WebcastAccessRecallMessage),
    AlertBoxAuditResult(WebcastAlertBoxAuditResultMessage),
    BindingGift(WebcastBindingGiftMessage),
    BoostCard(WebcastBoostCardMessage),
    BottomMessage(WebcastBottomMessage),
    GameRankNotify(WebcastGameRankNotifyMessage),
    GiftPrompt(WebcastGiftPromptMessage),
    LinkState(WebcastLinkStateMessage),
    LinkMicBattlePunishFinish(WebcastLinkMicBattlePunishFinish),
    LinkmicBattleTask(WebcastLinkmicBattleTaskMessage),
    MarqueeAnnouncement(WebcastMarqueeAnnouncementMessage),
    Notice(WebcastNoticeMessage),
    Notify(WebcastNotifyMessage),
    PartnershipDropsUpdate(WebcastPartnershipDropsUpdateMessage),
    PartnershipGameOffline(WebcastPartnershipGameOfflineMessage),
    PartnershipPunish(WebcastPartnershipPunishMessage),
    Perception(WebcastPerceptionMessage),
    Speaker(WebcastSpeakerMessage),
    SubCapsule(WebcastSubCapsuleMessage),
    SubPinEvent(WebcastSubPinEventMessage),
    SubscriptionNotify(WebcastSubscriptionNotifyMessage),
    Toast(WebcastToastMessage),

    // unknown passthrough
    Unknown { method: String, payload: Vec<u8> },
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct RoomInfo {
    pub title: String,
    pub viewers: i64,
    pub likes: i64,
    pub total_viewers: i64,
    pub stream_url: Option<StreamUrl>,
    pub raw_json: String,
}

/// One pullable tier of a LIVE room.
///
/// TikTok publishes several packagings of the same tier side by side, and which
/// of them a room carries varies by room, region and encoder. An absent
/// packaging is usually an EMPTY STRING rather than a missing key, so every URL
/// here is `None` unless it is a non-empty `http(s)` string.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct TikTokRendition {
    /// The `sdk_key`: "origin", "uhd", "hd", "sd", "ld", "ao".
    pub tier: String,
    /// TikTok's own display name for the tier, falling back to `tier`.
    pub label: String,
    /// Ordering value from the room's qualities ladder. Higher is better.
    pub level: i64,
    pub flv: Option<String>,
    pub hls: Option<String>,
    /// fMP4-packaged HLS. Plays natively wherever plain HLS does, and is the
    /// lowest-latency packaging TikTok offers.
    pub cmaf: Option<String>,
    pub dash: Option<String>,
    /// Decoded from `sdk_params`, which is itself a JSON string.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub vbitrate: Option<u64>,
    /// "h264", "bytevc1" (HEVC), "bytevc2". A tier whose codec the player
    /// cannot decode has to be demoted before it is selected, not after.
    pub vcodec: Option<String>,
}

impl TikTokRendition {
    /// True when nothing playable was published for this tier.
    pub fn is_empty(&self) -> bool {
        self.flv.is_none() && self.hls.is_none() && self.cmaf.is_none() && self.dash.is_none()
    }

    /// The short side of the frame. TikTok is portrait, so `height` is the LONG
    /// side and comparing a requested "720p" against it selects the wrong tier.
    pub fn short_side(&self) -> Option<u32> {
        match (self.width, self.height) {
            (Some(w), Some(h)) => Some(w.min(h)),
            (Some(w), None) => Some(w),
            (None, Some(h)) => Some(h),
            (None, None) => None,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct StreamUrl {
    pub flv_origin: Option<String>,
    pub flv_hd: Option<String>,
    pub flv_sd: Option<String>,
    pub flv_ld: Option<String>,
    pub flv_ao: Option<String>,
    /// Every published tier, best first. The five fields above are DERIVED from
    /// this, so there is one parser rather than two that can disagree.
    #[serde(default)]
    pub renditions: Vec<TikTokRendition>,
    /// The tier TikTok's own player would open on.
    #[serde(default)]
    pub default_tier: Option<String>,
}

impl WebcastGiftMessage {
    pub fn is_combo_gift(&self) -> bool {
        match &self.gift_details {
            Some(g) => g.gift_type == 1,
            None => false,
        }
    }

    pub fn is_streak_over(&self) -> bool {
        if !self.is_combo_gift() {
            return true;
        }
        self.repeat_end == 1
    }

    pub fn diamond_total(&self) -> i64 {
        let per_gift = match &self.gift_details {
            Some(g) => g.diamond_count as i64,
            None => 0,
        };
        per_gift * (self.repeat_count as i64).max(1)
    }
}
