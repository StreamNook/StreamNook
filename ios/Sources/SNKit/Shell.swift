import Foundation

/// The four tabs, in the bar's order. The raw value is what the start tab pref stores.
public enum MobileTab: String, CaseIterable, Sendable, Hashable {
    case following, browse, rewards, you

    /// A stored start tab; anything unknown or missing opens Following.
    public static func stored(_ raw: String?) -> MobileTab {
        raw.flatMap(MobileTab.init(rawValue:)) ?? .following
    }

    public var title: String {
        switch self {
        case .following: "Following"
        case .browse: "Browse"
        case .rewards: "Rewards"
        case .you: "You"
        }
    }
}

/// How stream lists draw their rows: one shared choice for Following and Browse.
public enum StreamView: String, Sendable {
    case cards, list

    /// A stored choice; anything but "list" is cards.
    public static func stored(_ raw: String?) -> StreamView { raw == "list" ? .list : .cards }
}

/// The app-wide motion preference (`motion_mode`).
public enum MotionMode: String, Sendable {
    case full, reduced, off

    /// Unset or unknown is full.
    public init(setting: String?) {
        self = setting.flatMap(MotionMode.init(rawValue:)) ?? .full
    }

    /// Movement (offsets, scales, slides) is dropped; fades stay.
    public var dropsMovement: Bool { self != .full }
    /// Looping decorations (pings, pulses, shimmers) stop.
    public var stopsLoops: Bool { self != .full }
}

/// The settings the look reads, out of `load_settings`. Every field is optional so a settings
/// file from any version decodes.
public struct SettingsLook: Decodable, Sendable, Equatable {
    public var theme: String?
    public var glassTransparency: Int?
    public var glassBlur: Bool?
    public var setupComplete: Bool?
    public var font: String?
    public var motionMode: String?

    enum CodingKeys: String, CodingKey {
        case theme
        case glassTransparency = "glass_transparency"
        case glassBlur = "glass_blur"
        case setupComplete = "setup_complete"
        case font
        case motionMode = "motion_mode"
    }

    public init(theme: String? = nil, glassTransparency: Int? = nil, glassBlur: Bool? = nil,
                setupComplete: Bool? = nil, font: String? = nil, motionMode: String? = nil) {
        self.theme = theme
        self.glassTransparency = glassTransparency
        self.glassBlur = glassBlur
        self.setupComplete = setupComplete
        self.font = font
        self.motionMode = motionMode
    }

    /// Glassiness as the slider stores it, 0 to 100, unset meaning 100.
    public var glassiness: Int { min(100, max(0, glassTransparency ?? 100)) }
    public var motion: MotionMode { MotionMode(setting: motionMode) }
}

/// The window's shape as the phone layouts read it: buckets by width, so an iPhone in portrait is
/// compact and on its side medium or expanded.
public struct WindowShape: Sendable, Equatable {
    public enum SizeClass: Sendable, Equatable { case compact, medium, expanded }
    public enum Variant: Sendable { case card, row }

    public var width: Double
    public var height: Double

    public init(width: Double, height: Double) {
        self.width = width
        self.height = height
    }

    public var sizeClass: SizeClass {
        width < 600 ? .compact : width < 840 ? .medium : .expanded
    }

    public var landscape: Bool { width > height }
    /// Two panes fit: expanded, or medium on its side.
    public var twoPanes: Bool { sizeClass == .expanded || (sizeClass == .medium && landscape) }
    /// The smallest side is tablet sized. A phone on its side is still a phone.
    public var largeScreen: Bool { min(width, height) >= 600 }
    /// Where the default two-pane split falls: the player gets 62 percent.
    public var splitX: Double { (width * 0.62).rounded() }

    /// Columns for a stream list.
    public func columns(_ variant: Variant) -> Int {
        switch sizeClass {
        case .compact: variant == .row && landscape ? 2 : 1
        case .medium: 2
        case .expanded: variant == .card ? 3 : 2
        }
    }
}

/// A hype train as the home snapshot lists it, by Twitch channel id.
public struct HypeTrain: Decodable, Sendable, Equatable {
    public var channelID: String
    public var isActive: Bool
    public var level: Int
    public var isGolden: Bool

    enum CodingKeys: String, CodingKey {
        case channelID = "channel_id"
        case isActive = "is_active"
        case level
        case isGolden = "is_golden_kappa"
    }

    public init(channelID: String, isActive: Bool, level: Int, isGolden: Bool) {
        self.channelID = channelID
        self.isActive = isActive
        self.level = level
        self.isGolden = isGolden
    }
}

/// The sections of the core's home snapshot the tab screens read. Missing sections decode empty.
public struct HomeLists: Decodable, Sendable, Equatable {
    public var followedLive: [TwitchStream]
    public var recommended: [TwitchStream]
    public var hypeTrains: [HypeTrain]
    public var watchStreaks: [String: Int]

    enum CodingKeys: String, CodingKey {
        case followedLive = "followed_live"
        case recommended
        case hypeTrains = "hype_trains"
        case watchStreaks = "watch_streaks"
    }

    public init(followedLive: [TwitchStream] = [], recommended: [TwitchStream] = [],
                hypeTrains: [HypeTrain] = [], watchStreaks: [String: Int] = [:]) {
        self.followedLive = followedLive
        self.recommended = recommended
        self.hypeTrains = hypeTrains
        self.watchStreaks = watchStreaks
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        followedLive = try c.decodeIfPresent([TwitchStream].self, forKey: .followedLive) ?? []
        recommended = try c.decodeIfPresent([TwitchStream].self, forKey: .recommended) ?? []
        hypeTrains = try c.decodeIfPresent([HypeTrain].self, forKey: .hypeTrains) ?? []
        watchStreaks = try c.decodeIfPresent([String: Int].self, forKey: .watchStreaks) ?? [:]
    }

    /// The active train on a channel, by its Twitch id.
    public func hypeTrain(for channelID: String?) -> HypeTrain? {
        guard let channelID else { return nil }
        return hypeTrains.first { $0.channelID == channelID && $0.isActive }
    }

    /// The watch streak on a channel, by its Twitch id, when above zero.
    public func streak(for channelID: String?) -> Int? {
        guard let channelID, let days = watchStreaks[channelID], days > 0 else { return nil }
        return days
    }
}

/// Twitch image addresses carry `{width}` and `{height}` (or `-{w}x{h}`) placeholders.
public enum TwitchImage {
    /// A stream preview at the size asked for. `stamp` changes when a list is refreshed, so a
    /// refresh shows new frames while ordinary redraws fetch nothing new.
    public static func preview(_ template: String, width: Int = 640, height: Int = 360, stamp: Int = 0) -> URL? {
        guard !template.isEmpty else { return nil }
        let base = fill(template, width: width, height: height)
        let joined = base + (base.contains("?") ? "&" : "?") + "sn=\(stamp)"
        return URL(string: joined)
    }

    /// Category box art at the size asked for.
    public static func boxArt(_ template: String?, width: Int = 285, height: Int = 380) -> URL? {
        guard let template, !template.isEmpty else { return nil }
        return URL(string: fill(template, width: width, height: height))
    }

    static func fill(_ template: String, width: Int, height: Int) -> String {
        template
            .replacingOccurrences(of: "{width}", with: "\(width)")
            .replacingOccurrences(of: "{height}", with: "\(height)")
            .replacingOccurrences(of: "-52x72", with: "-\(width)x\(height)")
    }
}

/// What a toast is about, which decides whether the phone shows it at all.
public enum ToastKind: String, Sendable {
    case info, success, warning, error

    /// The phone shows only errors and toasts that carry an action; the rest are dropped.
    public func shows(hasAction: Bool) -> Bool { self == .error || hasAction }

    /// How long it stays: 5 s normally, 8 s for a live alert, never for a sticky one.
    public static func seconds(live: Bool = false, sticky: Bool = false) -> Double? {
        sticky ? nil : live ? 8 : 5
    }
}
