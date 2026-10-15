import Foundation

// What the core answers for profiles and cosmetics: StreamNook member badges, Atmospheres (CSS
// read into layers by the core), 7TV paints, and the profile surfaces built from them. Every
// rule is the core's; these only decode.

// MARK: Fills (Rust `css_fill`)

/// A CSS background as layers, top first as CSS stacks them.
public struct SNFill: Codable, Sendable, Equatable, Hashable {
    public var layers: [Layer]

    public init(layers: [Layer] = []) {
        self.layers = layers
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        layers = try c.decodeIfPresent([Layer].self, forKey: .layers) ?? []
    }

    public var isEmpty: Bool { layers.isEmpty }

    public struct Stop: Codable, Sendable, Equatable, Hashable {
        /// `#rrggbbaa`.
        public var color: String
        /// A fraction of the gradient line, or CSS pixels when `px`.
        public var at: Double
        public var px: Bool

        public init(color: String, at: Double, px: Bool = false) {
            self.color = color
            self.at = at
            self.px = px
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            color = try c.decode(String.self, forKey: .color)
            at = try c.decodeIfPresent(Double.self, forKey: .at) ?? 0
            px = try c.decodeIfPresent(Bool.self, forKey: .px) ?? false
        }
    }

    public enum Layer: Codable, Sendable, Equatable, Hashable {
        /// `angle` in CSS degrees (0 up, 90 right).
        case linear(angle: Double, repeating: Bool, stops: [Stop])
        /// `size` as fractions of the box (nil: the farthest corner); `center` a fraction.
        case radial(circle: Bool, size: [Double]?, center: [Double], repeating: Bool, stops: [Stop])
        case image(url: String, cover: Bool)
        case solid(color: String)

        enum CodingKeys: String, CodingKey { case type, angle, repeating, stops, circle, size, center, url, cover, color }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            switch try c.decodeIfPresent(String.self, forKey: .type) ?? "solid" {
            case "linear":
                self = .linear(angle: try c.decodeIfPresent(Double.self, forKey: .angle) ?? 180,
                               repeating: try c.decodeIfPresent(Bool.self, forKey: .repeating) ?? false,
                               stops: try c.decodeIfPresent([Stop].self, forKey: .stops) ?? [])
            case "radial":
                self = .radial(circle: try c.decodeIfPresent(Bool.self, forKey: .circle) ?? false,
                               size: try c.decodeIfPresent([Double].self, forKey: .size),
                               center: try c.decodeIfPresent([Double].self, forKey: .center) ?? [0.5, 0.5],
                               repeating: try c.decodeIfPresent(Bool.self, forKey: .repeating) ?? false,
                               stops: try c.decodeIfPresent([Stop].self, forKey: .stops) ?? [])
            case "image":
                self = .image(url: try c.decode(String.self, forKey: .url),
                              cover: try c.decodeIfPresent(Bool.self, forKey: .cover) ?? false)
            default:
                self = .solid(color: try c.decodeIfPresent(String.self, forKey: .color) ?? "#00000000")
            }
        }

        public func encode(to encoder: Encoder) throws {
            var c = encoder.container(keyedBy: CodingKeys.self)
            switch self {
            case .linear(let angle, let repeating, let stops):
                try c.encode("linear", forKey: .type)
                try c.encode(angle, forKey: .angle)
                try c.encode(repeating, forKey: .repeating)
                try c.encode(stops, forKey: .stops)
            case .radial(let circle, let size, let center, let repeating, let stops):
                try c.encode("radial", forKey: .type)
                try c.encode(circle, forKey: .circle)
                try c.encodeIfPresent(size, forKey: .size)
                try c.encode(center, forKey: .center)
                try c.encode(repeating, forKey: .repeating)
                try c.encode(stops, forKey: .stops)
            case .image(let url, let cover):
                try c.encode("image", forKey: .type)
                try c.encode(url, forKey: .url)
                try c.encode(cover, forKey: .cover)
            case .solid(let color):
                try c.encode("solid", forKey: .type)
                try c.encode(color, forKey: .color)
            }
        }
    }
}

// MARK: Atmospheres (Rust `cosmetics_registry::Atmosphere`)

/// An Atmosphere's look as layers (Rust `AtmosphereLook`).
public struct AtmosphereLook: Codable, Sendable, Equatable, Hashable {
    public var accent: String
    public var swatch: SNFill
    public var baseColor: String
    public var baseLayers: SNFill
    public var image: String?
    public var layers: SNFill
    public var layers2: SNFill
    /// "aurora" or "drift".
    public var motion: String
    /// The 1 pt edge down a chat row's left; nil draws none.
    public var chatEdge: SNFill?
    public var chatFrost: Bool
    public var chatBlur: Double?
    public var chatRim: SNFill?
    /// "cologne-chrome" for the Cologne look.
    public var kind: String?
    public var chromeTexture: String?
    public var chromeCoin: String?
    public var chromeFrame: String?

    enum CodingKeys: String, CodingKey {
        case accent, swatch, baseColor = "base_color", baseLayers = "base_layers", image, layers, layers2, motion
        case chatEdge = "chat_edge", chatFrost = "chat_frost", chatBlur = "chat_blur", chatRim = "chat_rim", kind
        case chromeTexture = "chrome_texture", chromeCoin = "chrome_coin", chromeFrame = "chrome_frame"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        accent = try c.decodeIfPresent(String.self, forKey: .accent) ?? "#ffffffff"
        swatch = try c.decodeIfPresent(SNFill.self, forKey: .swatch) ?? SNFill()
        baseColor = try c.decodeIfPresent(String.self, forKey: .baseColor) ?? "#000000ff"
        baseLayers = try c.decodeIfPresent(SNFill.self, forKey: .baseLayers) ?? SNFill()
        image = try c.decodeIfPresent(String.self, forKey: .image)
        layers = try c.decodeIfPresent(SNFill.self, forKey: .layers) ?? SNFill()
        layers2 = try c.decodeIfPresent(SNFill.self, forKey: .layers2) ?? SNFill()
        motion = try c.decodeIfPresent(String.self, forKey: .motion) ?? "aurora"
        chatEdge = try c.decodeIfPresent(SNFill.self, forKey: .chatEdge)
        chatFrost = try c.decodeIfPresent(Bool.self, forKey: .chatFrost) ?? false
        chatBlur = try c.decodeIfPresent(Double.self, forKey: .chatBlur)
        chatRim = try c.decodeIfPresent(SNFill.self, forKey: .chatRim)
        kind = try c.decodeIfPresent(String.self, forKey: .kind)
        chromeTexture = try c.decodeIfPresent(String.self, forKey: .chromeTexture)
        chromeCoin = try c.decodeIfPresent(String.self, forKey: .chromeCoin)
        chromeFrame = try c.decodeIfPresent(String.self, forKey: .chromeFrame)
    }
}

public struct Atmosphere: Codable, Sendable, Equatable, Identifiable {
    public var id: String
    public var name: String
    public var look: AtmosphereLook

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        look = try c.decode(AtmosphereLook.self, forKey: .look)
    }
}

// MARK: Member badges and themes

/// A member's StreamNook badge (Rust `SnBadge`). `imageURL` may use the `streamnook-house:` scheme
/// for art the app ships.
public struct SNMemberBadge: Codable, Sendable, Equatable, Hashable {
    public var slug: String?
    public var imageURL: String
    public var label: String

    enum CodingKeys: String, CodingKey { case slug, imageURL = "image_url", label }
}

/// The CS2 Major Cologne look's add-ons.
public struct CologneLook: Codable, Sendable, Equatable, Hashable {
    public var coin: Bool
    public var frame: Bool

    public init(coin: Bool, frame: Bool) {
        self.coin = coin
        self.frame = frame
    }
}

/// A member's resolved theme (Rust `MemberTheme`, camelCase).
public struct MemberTheme: Codable, Sendable, Equatable {
    public var memberID: String
    public var theme: String?
    public var atmosphereID: String?
    public var cologne: CologneLook?
    public var hiddenInChat: Bool

    enum CodingKeys: String, CodingKey {
        case memberID = "memberId", theme, atmosphereID = "atmosphereId", cologne, hiddenInChat
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        memberID = try c.decode(String.self, forKey: .memberID)
        theme = try c.decodeIfPresent(String.self, forKey: .theme)
        atmosphereID = try c.decodeIfPresent(String.self, forKey: .atmosphereID)
        cologne = try c.decodeIfPresent(CologneLook.self, forKey: .cologne)
        hiddenInChat = try c.decodeIfPresent(Bool.self, forKey: .hiddenInChat) ?? false
    }
}

// MARK: Profile surfaces (Rust `profile_view`)

/// One badge on a profile surface.
public struct ProfileBadge: Codable, Sendable, Equatable, Hashable, Identifiable {
    public var id: String
    public var url: String
    public var fallbacks: [String]
    public var title: String
    public var name: String?
    public var description: String?
    public var provider: String?

    public init(id: String, url: String, fallbacks: [String] = [], title: String = "", name: String? = nil,
                description: String? = nil, provider: String? = nil) {
        self.id = id
        self.url = url
        self.fallbacks = fallbacks
        self.title = title
        self.name = name
        self.description = description
        self.provider = provider
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        url = try c.decode(String.self, forKey: .url)
        fallbacks = try c.decodeIfPresent([String].self, forKey: .fallbacks) ?? []
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        provider = try c.decodeIfPresent(String.self, forKey: .provider)
    }

    /// What it is called aloud: its title, else its name, else "Badge".
    public var label: String {
        if !title.isEmpty { return title }
        if let name, !name.isEmpty { return name }
        return "Badge"
    }

    /// Every URL to try, best first.
    public var urls: [String] { [url] + fallbacks }
}

public struct ProfileBadgeGroup: Codable, Sendable, Equatable, Identifiable {
    public var title: String
    public var badges: [ProfileBadge]
    public var id: String { title }
}

/// The 7TV picture renditions one surface draws.
public struct AvatarPick: Codable, Sendable, Equatable, Hashable {
    public var still: String
    public var animated: String?
}

/// The user profile sheet (Rust `ProfileSheet`).
public struct ProfileSheetData: Codable, Sendable, Equatable {
    public var userID: String
    public var login: String
    public var displayName: String
    public var shownLogin: String?
    public var avatarURL: String?
    public var picture: AvatarPick?
    public var nameColor: String
    public var paint: ChatPaint?
    public var groups: [ProfileBadgeGroup]
    public var streamnook: SNMemberBadge?
    /// The Atmosphere worn on the profile (wear-gated; shown even when kept out of chat).
    public var atmosphere: Atmosphere?
    /// The Cologne event look instead of an Atmosphere.
    public var cologne: CologneLook?

    enum CodingKeys: String, CodingKey {
        case userID = "user_id", login, displayName = "display_name", shownLogin = "shown_login"
        case avatarURL = "avatar_url", picture, nameColor = "name_color", paint, groups, streamnook, atmosphere, cologne
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        userID = try c.decode(String.self, forKey: .userID)
        login = try c.decodeIfPresent(String.self, forKey: .login) ?? ""
        displayName = try c.decodeIfPresent(String.self, forKey: .displayName) ?? ""
        shownLogin = try c.decodeIfPresent(String.self, forKey: .shownLogin)
        avatarURL = try c.decodeIfPresent(String.self, forKey: .avatarURL)
        picture = try c.decodeIfPresent(AvatarPick.self, forKey: .picture)
        nameColor = try c.decodeIfPresent(String.self, forKey: .nameColor) ?? "#9147FF"
        paint = try c.decodeIfPresent(ChatPaint.self, forKey: .paint)
        groups = try c.decodeIfPresent([ProfileBadgeGroup].self, forKey: .groups) ?? []
        streamnook = try c.decodeIfPresent(SNMemberBadge.self, forKey: .streamnook)
        atmosphere = try c.decodeIfPresent(Atmosphere.self, forKey: .atmosphere)
        cologne = try c.decodeIfPresent(CologneLook.self, forKey: .cologne)
    }

    /// No badge in any group and not a member.
    public var hasNoBadges: Bool { groups.allSatisfy { $0.badges.isEmpty } && streamnook == nil }
}

/// The viewer's own identity header (Rust `OwnIdentity`).
public struct OwnIdentityData: Codable, Sendable, Equatable {
    public var userID: String
    public var login: String
    public var displayName: String
    public var shownLogin: String?
    public var paint: ChatPaint?
    public var picture: AvatarPick?
    public var badges: [ProfileBadge]
    public var streamnook: SNMemberBadge?
    /// The Atmosphere worn on the profile (wear-gated; shown even when kept out of chat).
    public var atmosphere: Atmosphere?
    /// The Cologne event look instead of an Atmosphere.
    public var cologne: CologneLook?

    enum CodingKeys: String, CodingKey {
        case userID = "user_id", login, displayName = "display_name", shownLogin = "shown_login", paint, picture, badges
        case streamnook, atmosphere, cologne
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        userID = try c.decode(String.self, forKey: .userID)
        login = try c.decodeIfPresent(String.self, forKey: .login) ?? ""
        displayName = try c.decodeIfPresent(String.self, forKey: .displayName) ?? ""
        shownLogin = try c.decodeIfPresent(String.self, forKey: .shownLogin)
        paint = try c.decodeIfPresent(ChatPaint.self, forKey: .paint)
        picture = try c.decodeIfPresent(AvatarPick.self, forKey: .picture)
        badges = try c.decodeIfPresent([ProfileBadge].self, forKey: .badges) ?? []
        streamnook = try c.decodeIfPresent(SNMemberBadge.self, forKey: .streamnook)
        atmosphere = try c.decodeIfPresent(Atmosphere.self, forKey: .atmosphere)
        cologne = try c.decodeIfPresent(CologneLook.self, forKey: .cologne)
    }
}

// MARK: The Cosmetics screen

public struct PickerBadge: Codable, Sendable, Equatable, Identifiable {
    public var slug: String
    public var name: String
    public var imageURL: String?
    public var worn: Bool
    public var id: String { slug }

    enum CodingKeys: String, CodingKey { case slug, name, imageURL = "image_url", worn }
}

public struct PickerAtmosphere: Codable, Sendable, Equatable, Identifiable {
    /// `tier` for the Default tile.
    public var id: String
    public var name: String
    public var swatch: SNFill
    public var allowed: Bool
    public var worn: Bool
}

/// The StreamNook half of the Cosmetics screen (Rust `StreamNookPicker`).
public struct StreamNookPickerState: Codable, Sendable, Equatable {
    public var badges: [PickerBadge]
    public var atmospheres: [PickerAtmosphere]
    public var theme: String
    public var showChatToggle: Bool
    public var chatAtmosphere: Bool

    enum CodingKeys: String, CodingKey {
        case badges, atmospheres, theme, showChatToggle = "show_chat_toggle", chatAtmosphere = "chat_atmosphere"
    }
}

public struct PickerPaint: Codable, Sendable, Equatable, Identifiable {
    public var id: String
    public var name: String
    public var paint: ChatPaint
    public var worn: Bool
}

public struct PickerSevenTvBadge: Codable, Sendable, Equatable, Identifiable {
    public var badge: ProfileBadge
    public var worn: Bool
    public var id: String { badge.id }
}

/// One of Twitch's global badges (Rust `ChatIdentityBadge`).
public struct TwitchIdentityBadge: Codable, Sendable, Equatable, Identifiable {
    public var setID: String
    public var version: String
    public var title: String
    public var imageURL: String
    public var isSelected: Bool
    public var id: String { "\(setID)/\(version)" }

    enum CodingKeys: String, CodingKey {
        case setID = "id", version, title, imageURL = "image_url", isSelected = "is_selected"
    }
}

/// The 7TV and Twitch half of the Cosmetics screen (Rust `ExternalPicker`).
public struct ExternalPickerState: Codable, Sendable, Equatable {
    public var sevenTvSignedIn: Bool
    public var sevenTvUserID: String?
    public var paints: [PickerPaint]
    public var sevenTvBadges: [PickerSevenTvBadge]
    public var dropsSignedIn: Bool
    public var twitchSelectable: [TwitchIdentityBadge]
    public var twitchEarned: [ProfileBadge]
    /// The account's Twitch name colour, `#rrggbb`.
    public var chatColor: String?
    public var namedColors: [NamedChatColor]

    enum CodingKeys: String, CodingKey {
        case sevenTvSignedIn = "seventv_signed_in", sevenTvUserID = "seventv_user_id", paints
        case sevenTvBadges = "seventv_badges", dropsSignedIn = "drops_signed_in"
        case twitchSelectable = "twitch_selectable", twitchEarned = "twitch_earned"
        case chatColor = "chat_color", namedColors = "named_colors"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sevenTvSignedIn = try c.decodeIfPresent(Bool.self, forKey: .sevenTvSignedIn) ?? false
        sevenTvUserID = try c.decodeIfPresent(String.self, forKey: .sevenTvUserID)
        paints = try c.decodeIfPresent([PickerPaint].self, forKey: .paints) ?? []
        sevenTvBadges = try c.decodeIfPresent([PickerSevenTvBadge].self, forKey: .sevenTvBadges) ?? []
        dropsSignedIn = try c.decodeIfPresent(Bool.self, forKey: .dropsSignedIn) ?? false
        twitchSelectable = try c.decodeIfPresent([TwitchIdentityBadge].self, forKey: .twitchSelectable) ?? []
        twitchEarned = try c.decodeIfPresent([ProfileBadge].self, forKey: .twitchEarned) ?? []
        chatColor = try c.decodeIfPresent(String.self, forKey: .chatColor)
        namedColors = try c.decodeIfPresent([NamedChatColor].self, forKey: .namedColors) ?? []
    }
}

/// One of Twitch's named chat colours (Rust `NamedColor`).
public struct NamedChatColor: Codable, Sendable, Equatable, Identifiable {
    public var label: String
    /// What Helix takes (`blue_violet`).
    public var value: String
    public var hex: String
    public var id: String { value }
}

// MARK: Sub standing (Rust `sub_standing::SubStanding`)

public struct SubStanding: Codable, Sendable, Equatable {
    public struct Fact: Codable, Sendable, Equatable, Hashable {
        /// "founder" | "prime" | "streak" | "gift"
        public var kind: String
        public var text: String
        public var art: String?
    }

    public struct NextBadge: Codable, Sendable, Equatable {
        public var months: Int
        public var monthsLeft: Int
        public var progress: Double
        public var art: String?
        public var text: String

        enum CodingKeys: String, CodingKey { case months, monthsLeft = "months_left", progress, art, text }
    }

    /// "current" | "past" | "never" | "unknown"
    public var state: String
    public var founder: Bool
    /// "paid" | "prime" | "gift" (Rust `SubKind`)
    public var kind: String?
    public var tier: Int?
    public var months: Int?
    public var title: String?
    public var detail: String?
    public var giftLabel: String?
    public var giftURL: String?
    public var monthsUnit: String?
    public var tierLabel: String?
    public var facts: [Fact]
    public var badgeArt: String?
    public var nextBadge: NextBadge?

    enum CodingKeys: String, CodingKey {
        case state, founder, kind, tier, months, title, detail, giftLabel = "gift_label", giftURL = "gift_url"
        case monthsUnit = "months_unit", tierLabel = "tier_label", facts, badgeArt = "badge_art", nextBadge = "next_badge"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        state = try c.decodeIfPresent(String.self, forKey: .state) ?? "unknown"
        founder = try c.decodeIfPresent(Bool.self, forKey: .founder) ?? false
        kind = try c.decodeIfPresent(String.self, forKey: .kind)
        tier = try c.decodeIfPresent(Int.self, forKey: .tier)
        months = try c.decodeIfPresent(Int.self, forKey: .months)
        title = try c.decodeIfPresent(String.self, forKey: .title)
        detail = try c.decodeIfPresent(String.self, forKey: .detail)
        giftLabel = try c.decodeIfPresent(String.self, forKey: .giftLabel)
        giftURL = try c.decodeIfPresent(String.self, forKey: .giftURL)
        monthsUnit = try c.decodeIfPresent(String.self, forKey: .monthsUnit)
        tierLabel = try c.decodeIfPresent(String.self, forKey: .tierLabel)
        facts = try c.decodeIfPresent([Fact].self, forKey: .facts) ?? []
        badgeArt = try c.decodeIfPresent(String.self, forKey: .badgeArt)
        nextBadge = try c.decodeIfPresent(NextBadge.self, forKey: .nextBadge)
    }

    /// The tier's tint: Prime blue, gold for tier 3, pale purple for tier 2, Twitch purple.
    public var tintHex: String {
        if kind == "prime" { return "#00a8e1" }
        if tier == 3 { return "#f2c14e" }
        if tier == 2 { return "#bf94ff" }
        return "#a970ff"
    }
}

// MARK: Recent messages (Rust `UserMessageSummary`)

public struct ProfileHistoryRow: Codable, Sendable, Equatable, Identifiable {
    public var id: String
    public var content: String
    public var timestamp: String?

    public init(id: String, content: String, timestamp: String?) {
        self.id = id
        self.content = content
        self.timestamp = timestamp
    }

    /// The stamp as a date: unix ms or seconds as a string, or RFC 3339.
    public var date: Date? {
        guard let timestamp, !timestamp.isEmpty else { return nil }
        if let n = Double(timestamp) {
            return Date(timeIntervalSince1970: n < 1e12 ? n : n / 1000)
        }
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = iso.date(from: timestamp) { return d }
        iso.formatOptions = [.withInternetDateTime]
        return iso.date(from: timestamp)
    }
}

/// Who a profile sheet opens for, and in which room.
public struct ProfileTarget: Sendable, Equatable, Identifiable, Hashable {
    /// The bare Twitch user id; empty when only the login is known (the core resolves it).
    public var userID: String
    public var login: String
    public var displayName: String
    /// The row's colour; nil uses Twitch purple.
    public var color: String?
    /// The opening row's badge keys (`subscriber/3012`), for the sub standing tile.
    public var badgeKeys: [String]
    /// The room's broadcaster id and login, so channel badges and the sub tile resolve there.
    public var roomID: String?
    public var roomLogin: String?
    /// The sub or founder badge art the row wore, for the sub tile when the core has none.
    public var subBadgeArt: String?

    public var id: String { userID.isEmpty ? "login:\(login.lowercased())" : userID }

    public init(userID: String, login: String, displayName: String, color: String? = nil, badgeKeys: [String] = [],
                roomID: String? = nil, roomLogin: String? = nil, subBadgeArt: String? = nil) {
        self.userID = userID
        self.login = login
        self.displayName = displayName
        self.color = color
        self.badgeKeys = badgeKeys
        self.roomID = roomID
        self.roomLogin = roomLogin
        self.subBadgeArt = subBadgeArt
    }
}
