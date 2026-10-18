import Foundation

/// The stream manager as the core's `get_stream_manager` answers it (services/stream_manager.rs,
/// `StreamManagerView`). Rust decides which rows a role gets and words every line; the app only
/// draws them and sends `ManagerAction`s through `stream_manager_act`.
public struct StreamManagerView: Decodable, Sendable, Equatable {
    public var channelID: String
    public var channelLogin: String
    public var role: String
    public var roleLine: String
    public var sections: [ManagerSectionRow]
    public var info: StreamInfo?
    public var poll: RunningPoll?
    public var prediction: RunningPrediction?
    public var raid: PendingRaid?
    public var ads: AdsInfo?
    public var chat: ManagerChat?
    public var queueCount: Int?
    public var limits: ManagerLimits
    public var problems: [String]

    enum CodingKeys: String, CodingKey {
        case role, sections, info, poll, prediction, raid, ads, chat, limits, problems
        case channelID = "channel_id", channelLogin = "channel_login", roleLine = "role_line", queueCount = "queue_count"
    }

    /// The Stream group: every row but Announce, which the Chat group carries.
    public var streamRows: [ManagerSectionRow] { sections.filter { $0.id != "announce" } }
    public var canAnnounce: Bool { sections.contains { $0.id == "announce" } }
}

public struct ManagerSectionRow: Decodable, Sendable, Equatable, Identifiable {
    /// "info", "poll", "prediction", "raid", "ads", "rewards", "announce".
    public var id: String
    public var label: String
    public var detail: String?
    public var badge: String?
}

public struct ManagerCategory: Codable, Sendable, Equatable, Hashable {
    public var id: String
    public var name: String
    public var boxArtURL: String

    public init(id: String, name: String, boxArtURL: String) {
        self.id = id
        self.name = name
        self.boxArtURL = boxArtURL
    }

    enum CodingKeys: String, CodingKey {
        case id, name, boxArtURL = "box_art_url"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        boxArtURL = try c.decodeIfPresent(String.self, forKey: .boxArtURL) ?? ""
    }

    /// The box art at 52x72 (Twitch's `{width}x{height}` template filled).
    public var boxArt: URL? {
        URL(string: boxArtURL.replacingOccurrences(of: "{width}", with: "52").replacingOccurrences(of: "{height}", with: "72"))
    }
}

public struct ManagerLanguage: Decodable, Sendable, Equatable, Hashable {
    public var code: String
    public var name: String
}

public struct StreamInfo: Decodable, Sendable, Equatable {
    public var title: String
    public var category: ManagerCategory?
    public var tags: [String]
    public var language: String
    public var languageName: String
    public var languages: [ManagerLanguage]
    public var summary: String

    enum CodingKeys: String, CodingKey {
        case title, category, tags, language, languages, summary, languageName = "language_name"
    }
}

public struct RunningPoll: Decodable, Sendable, Equatable {
    public var id: String
    public var title: String
}

public struct RunningPrediction: Decodable, Sendable, Equatable {
    public struct Outcome: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var title: String
        public var color: String
    }
    public var id: String
    public var title: String
    /// "active" (taking predictions) or "locked".
    public var status: String
    public var outcomes: [Outcome]
}

public struct PendingRaid: Decodable, Sendable, Equatable {
    public var targetID: String
    public var targetLogin: String
    public var targetName: String
    /// When the countdown ends by itself.
    public var deadlineMS: Int64

    enum CodingKeys: String, CodingKey {
        case targetID = "target_id", targetLogin = "target_login", targetName = "target_name", deadlineMS = "deadline_ms"
    }
}

public struct ManagerChoice: Decodable, Sendable, Equatable, Hashable {
    public var label: String
    public var value: UInt32
}

public struct AdsInfo: Decodable, Sendable, Equatable {
    public struct Schedule: Decodable, Sendable, Equatable {
        public var nextAdMS: Int64?
        public var snoozesLeft: UInt32
        public var snoozeLine: String
        public var prerollLine: String?

        enum CodingKeys: String, CodingKey {
            case nextAdMS = "next_ad_ms", snoozesLeft = "snoozes_left", snoozeLine = "snooze_line", prerollLine = "preroll_line"
        }
    }
    public var lengths: [ManagerChoice]
    public var schedule: Schedule?
    public var scheduleHint: String?

    enum CodingKeys: String, CodingKey {
        case lengths, schedule, scheduleHint = "schedule_hint"
    }
}

public struct ManagerChat: Decodable, Sendable, Equatable {
    public struct Mode: Decodable, Sendable, Equatable, Identifiable {
        /// "followers", "slow", "subs", "emotes", "unique".
        public var kind: String
        public var label: String
        public var on: Bool
        public var value: UInt32?
        public var valueLabel: String?
        public var options: [ManagerChoice]
        public var id: String { kind }

        enum CodingKeys: String, CodingKey {
            case kind, label, on, value, options, valueLabel = "value_label"
        }
    }
    public struct Color: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var label: String
    }
    public var modes: [Mode]
    public var shield: Bool?
    public var announceColors: [Color]

    enum CodingKeys: String, CodingKey {
        case modes, shield, announceColors = "announce_colors"
    }
}

public struct ManagerLimits: Decodable, Sendable, Equatable {
    public var title: Int
    public var tags: Int
    public var tag: Int
    public var pollTitle: Int
    public var pollChoice: Int
    public var pollChoicesMin: Int
    public var pollChoicesMax: Int
    public var predictionTitle: Int
    public var predictionOutcome: Int
    public var predictionOutcomesMin: Int
    public var predictionOutcomesMax: Int
    public var pollSecondsMin: UInt32
    public var predictionSecondsMin: UInt32
    public var durations: [ManagerChoice]

    enum CodingKeys: String, CodingKey {
        case title, tags, tag, durations
        case pollTitle = "poll_title", pollChoice = "poll_choice", pollChoicesMin = "poll_choices_min",
             pollChoicesMax = "poll_choices_max", predictionTitle = "prediction_title",
             predictionOutcome = "prediction_outcome", predictionOutcomesMin = "prediction_outcomes_min",
             predictionOutcomesMax = "prediction_outcomes_max", pollSecondsMin = "poll_seconds_min",
             predictionSecondsMin = "prediction_seconds_min"
    }
}

/// The stream info form as typed.
public struct StreamInfoDraft: Sendable, Equatable {
    public var title: String
    public var category: ManagerCategory?
    public var tags: [String]
    public var language: String

    public init(title: String = "", category: ManagerCategory? = nil, tags: [String] = [], language: String = "") {
        self.title = title
        self.category = category
        self.tags = tags
        self.language = language
    }

    public init(_ info: StreamInfo?) {
        self.init(title: info?.title ?? "", category: info?.category, tags: info?.tags ?? [], language: info?.language ?? "")
    }

    public var argument: [String: Any] {
        [
            "title": title,
            "category": category.map { ["id": $0.id, "name": $0.name, "box_art_url": $0.boxArtURL] as Any } ?? NSNull(),
            "tags": tags,
            "language": language,
        ]
    }
}

/// `check_stream_info`'s answer: the problems under the fields, the counts and the tile.
public struct InfoCheck: Decodable, Sendable, Equatable {
    public struct Problem: Decodable, Sendable, Equatable {
        /// "title", "tags", "tag:2", "language".
        public var field: String
        public var line: String
    }
    public struct Preview: Decodable, Sendable, Equatable {
        public var title: String
        public var channelName: String
        public var avatarURL: String?
        public var categoryName: String
        public var boxArtURL: String?
        public var tags: [String]
        public var thumbnailURL: String?

        enum CodingKeys: String, CodingKey {
            case title, tags
            case channelName = "channel_name", avatarURL = "avatar_url", categoryName = "category_name",
                 boxArtURL = "box_art_url", thumbnailURL = "thumbnail_url"
        }
    }
    public var ok: Bool
    public var problems: [Problem]
    public var titleCount: String
    public var tagsCount: String
    public var preview: Preview
    public var changed: Bool

    enum CodingKeys: String, CodingKey {
        case ok, problems, preview, changed, titleCount = "title_count", tagsCount = "tags_count"
    }

    public func problem(_ field: String) -> String? { problems.first { $0.field == field }?.line }
    public var tagProblem: String? { problems.first { $0.field.hasPrefix("tag") }?.line }
}

public struct RewardQueue: Decodable, Sendable, Equatable {
    public struct Row: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var userName: String
        public var avatarURL: String?
        public var input: String?
        public var line: String

        enum CodingKeys: String, CodingKey {
            case id, input, line, userName = "user_name", avatarURL = "avatar_url"
        }
    }
    public struct Group: Decodable, Sendable, Equatable, Identifiable {
        public var rewardID: String
        public var title: String
        public var paused: Bool
        public var enabled: Bool
        public var heading: String
        public var rows: [Row]
        public var id: String { rewardID }

        enum CodingKeys: String, CodingKey {
            case title, paused, enabled, heading, rows, rewardID = "reward_id"
        }
    }
    public var groups: [Group]
    public var count: Int
    public var emptyLine: String?
    public var more: Bool

    enum CodingKeys: String, CodingKey {
        case groups, count, more, emptyLine = "empty_line"
    }
}

public struct RaidTarget: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var login: String
    public var name: String
    public var avatarURL: String?
    public var live: Bool
    public var line: String

    enum CodingKeys: String, CodingKey {
        case id, login, name, live, line, avatarURL = "avatar_url"
    }
}

public struct ManagerOutcome: Decodable, Sendable, Equatable {
    public var ok: Bool
    public var message: String
    public var view: StreamManagerView?
    public var queue: RewardQueue?
}

/// Every write the manager makes, as `stream_manager_act`'s `action` argument.
public enum ManagerAction: Sendable, Equatable {
    case saveInfo(StreamInfoDraft)
    case createPoll(title: String, choices: [String], seconds: UInt32, pointsPerVote: UInt32?)
    case endPoll(showResults: Bool)
    case createPrediction(title: String, outcomes: [String], seconds: UInt32)
    case lockPrediction, cancelPrediction
    case resolvePrediction(outcomeID: String)
    case raid(RaidTarget)
    case cancelRaid, raidNow
    case runAd(seconds: UInt32)
    case snoozeAd
    case redemptions(ids: [String], fulfill: Bool)
    case rewardQueue(rewardID: String, fulfill: Bool)
    case reward(rewardID: String, paused: Bool?, enabled: Bool?)
    case chatMode(mode: String, on: Bool, value: UInt32?)
    case shield(on: Bool)
    case announce(text: String, color: String)

    public var argument: [String: Any] {
        func opt<T>(_ v: T?) -> Any { v.map { $0 as Any } ?? NSNull() }
        switch self {
        case .saveInfo(let d): return ["kind": "save_info", "draft": d.argument]
        case let .createPoll(t, c, s, p):
            return ["kind": "create_poll", "title": t, "choices": c, "seconds": s, "points_per_vote": opt(p)]
        case .endPoll(let show): return ["kind": "end_poll", "show_results": show]
        case let .createPrediction(t, o, s): return ["kind": "create_prediction", "title": t, "outcomes": o, "seconds": s]
        case .lockPrediction: return ["kind": "lock_prediction"]
        case .cancelPrediction: return ["kind": "cancel_prediction"]
        case .resolvePrediction(let id): return ["kind": "resolve_prediction", "outcome_id": id]
        case .raid(let t): return ["kind": "raid", "target_id": t.id, "target_login": t.login, "target_name": t.name]
        case .cancelRaid: return ["kind": "cancel_raid"]
        case .raidNow: return ["kind": "raid_now"]
        case .runAd(let s): return ["kind": "run_ad", "seconds": s]
        case .snoozeAd: return ["kind": "snooze_ad"]
        case let .redemptions(ids, f): return ["kind": "redemptions", "ids": ids, "fulfill": f]
        case let .rewardQueue(id, f): return ["kind": "reward_queue", "reward_id": id, "fulfill": f]
        case let .reward(id, p, e): return ["kind": "reward", "reward_id": id, "paused": opt(p), "enabled": opt(e)]
        case let .chatMode(m, on, v): return ["kind": "chat_mode", "mode": m, "on": on, "value": opt(v)]
        case .shield(let on): return ["kind": "shield", "on": on]
        case let .announce(t, c): return ["kind": "announce", "text": t, "color": c]
        }
    }
}

/// `m:ss` left until `deadlineMS`, never below zero.
public func managerCountdown(_ deadlineMS: Int64, now: Date = Date()) -> String {
    let left = max(0, Int((Double(deadlineMS) / 1000 - now.timeIntervalSince1970).rounded(.up)))
    return "\(left / 60):\(String(format: "%02d", left % 60))"
}
