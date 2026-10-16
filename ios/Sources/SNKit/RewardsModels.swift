import Foundation

/// The Rewards tab's Drops segment as `get_rewards_drops` builds it (services/rewards_drops.rs):
/// every line is worked out in the core; the screen only draws it.
public struct RewardsDrops: Decodable, Sendable, Equatable {
    public var connected: Bool
    public var query: String
    public var inProgress: [DropsGroup]
    public var available: [DropsGroup]
    public var earned: [EarnedDrop]
    public var inProgressEmpty: String
    /// `Nothing matches "rust".` when a search left every list empty; empty or nil otherwise.
    public var noMatch: String?

    enum CodingKeys: String, CodingKey {
        case connected, query, available, earned
        case inProgress = "in_progress"
        case inProgressEmpty = "in_progress_empty"
        case noMatch = "no_match"
    }

    public init(connected: Bool, query: String = "", inProgress: [DropsGroup] = [], available: [DropsGroup] = [],
                earned: [EarnedDrop] = [], inProgressEmpty: String = "") {
        self.connected = connected
        self.query = query
        self.inProgress = inProgress
        self.available = available
        self.earned = earned
        self.inProgressEmpty = inProgressEmpty
    }

    /// The group holding a campaign, for a jump from the player's drop bar.
    public func group(holding campaignID: String) -> DropsGroup? {
        (inProgress + available).first { $0.campaigns.contains { $0.id == campaignID } }
    }
}

/// One game's campaigns under a section.
public struct DropsGroup: Decodable, Sendable, Equatable, Identifiable {
    /// `progress:<game>` or `available:<game>`.
    public var key: String
    public var game: String
    public var art: String
    public var countLabel: String
    public var campaigns: [CampaignCard]

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key, game, art, campaigns
        case countLabel = "count_label"
    }
}

/// A campaign row and its sheet.
public struct CampaignCard: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var name: String
    public var gameID: String
    public var gameName: String
    public var art: String
    /// "2/5" in progress; empty for a campaign not started.
    public var claimedLabel: String
    public var percent: Double
    /// "3 rewards · 2d left".
    public var detail: String
    public var needsLink: Bool
    public var linkURL: String?
    public var metaLabel: String
    public var channels: [CampaignChannel]
    public var channelsLabel: String
    public var findStream: TwitchCategory?
    public var drops: [DropRow]

    enum CodingKeys: String, CodingKey {
        case id, name, art, percent, detail, channels, drops
        case gameID = "game_id"
        case gameName = "game_name"
        case claimedLabel = "claimed_label"
        case needsLink = "needs_link"
        case linkURL = "link_url"
        case metaLabel = "meta_label"
        case channelsLabel = "channels_label"
        case findStream = "find_stream"
    }

    public var started: Bool { !claimedLabel.isEmpty }
}

/// A channel a restricted campaign credits on.
public struct CampaignChannel: Decodable, Sendable, Equatable, Identifiable, Hashable {
    /// The broadcaster's Twitch user id.
    public var id: String
    public var name: String

    public var login: String { name.lowercased() }
}

/// One reward in the campaign sheet.
public struct DropRow: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var name: String
    public var image: String
    public var claimed: Bool
    public var done: Bool
    /// "Claimed", "Ready", "12m left", or empty.
    public var status: String
    public var detail: String?
    /// 0 to 100; nil for a drop watch time does not earn.
    public var bar: Double?
}

/// A drop earned lately.
public struct EarnedDrop: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var name: String
    public var image: String
    public var game: String?
}
