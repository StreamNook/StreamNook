import Foundation

/// The badge wall as `get_badge_wall` builds it (services/badge_wall.rs): tiles in the asked
/// sort with their sheet copy, the count, and the "Missing, earnable now" strip.
public struct BadgeWall: Decodable, Sendable, Equatable {
    public var loading: Bool
    public var tiles: [BadgeTile]
    public var countLabel: String
    public var syncLeft: Int
    public var missing: MissingNow
    public var nextChangeMS: Int64?

    enum CodingKeys: String, CodingKey {
        case loading, tiles, missing
        case countLabel = "count_label"
        case syncLeft = "sync_left"
        case nextChangeMS = "next_change_ms"
    }

    /// The tile standing for a `set/version` key (a missing row's).
    public func tile(holding key: String) -> BadgeTile? {
        tiles.first { $0.keys.contains(key) }
    }
}

/// A run of text, or an instant the phone writes in its own time zone.
public struct TextPart: Decodable, Sendable, Equatable {
    public var text: String?
    public var ms: Int64?
}

public struct BadgeTile: Decodable, Sendable, Equatable, Identifiable {
    public var key: String
    public var keys: [String]
    public var title: String
    public var image: String
    public var owned: Bool
    /// `available`, `coming-soon`, `expired`, or nil.
    public var status: String?
    /// "OWNED", "AVAILABLE", "SOON", "ENDED" or empty.
    public var word: String
    /// "OWNED", "AVAILABLE NOW", "COMING SOON", "NO LONGER EARNABLE" or empty.
    public var sheetWord: String
    public var prose: [TextPart]
    public var window: [TextPart]
    public var caveat: String?
    public var usageLabel: String?
    public var infoURL: String?

    public var id: String { key }
    /// Owned or earnable now: drawn in full color.
    public var lit: Bool { owned || status == "available" }

    enum CodingKeys: String, CodingKey {
        case key, keys, title, image, owned, status, word, prose, window, caveat
        case sheetWord = "sheet_word"
        case usageLabel = "usage_label"
        case infoURL = "info_url"
    }
}

public struct MissingNow: Decodable, Sendable, Equatable {
    /// `hidden`, `loading`, `failed`, `none`, or `list`.
    public var state: String
    public var refreshing: Bool
    public var closes: String
    public var catchUp: CatchUpView?
    public var rows: [MissingRow]

    enum CodingKeys: String, CodingKey {
        case state, refreshing, closes, rows
        case catchUp = "catch_up"
    }
}

public struct CatchUpView: Decodable, Sendable, Equatable {
    public var label: String
    public var stats: [CatchUpStat]
    public var notes: [String]
}

public struct CatchUpStat: Decodable, Sendable, Equatable, Hashable {
    public var value: String
    public var label: String
}

public struct MissingRow: Decodable, Sendable, Equatable, Identifiable {
    public var key: String
    public var title: String
    public var image: String
    public var chips: [EarnChip]
    public var iconsOnly: Bool
    public var randomNote: String?
    public var timeLeft: String

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key, title, image, chips
        case iconsOnly = "icons_only"
        case randomNote = "random_note"
        case timeLeft = "time_left"
    }
}

public struct EarnChip: Decodable, Sendable, Equatable, Hashable {
    public var kind: String
    public var ticket: Bool
    public var text: String
    public var hint: String
    public var iconURL: String?

    enum CodingKeys: String, CodingKey {
        case kind, ticket, text, hint
        case iconURL = "icon_url"
    }
}

/// The wall's sorts, in the row's order.
public enum BadgeSort: String, CaseIterable, Sendable, Hashable {
    case newest, available, soon, usage, oldest

    public var label: String {
        switch self {
        case .newest: "Newest"
        case .available: "Available"
        case .soon: "Coming soon"
        case .usage: "Most used"
        case .oldest: "Oldest"
        }
    }
}
