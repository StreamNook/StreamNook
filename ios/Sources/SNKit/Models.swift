import Foundation

/// A live stream as `get_streams_by_game`, `search_channels` and the home snapshot return it
/// (the TypeScript `TwitchStream`).
public struct TwitchStream: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var userLogin: String
    public var userName: String
    public var title: String
    public var viewerCount: Int
    public var gameName: String
    public var thumbnailURL: String
    /// The broadcaster's Twitch numeric id, the key hype trains and watch streaks are filed under.
    public var userID: String?
    public var gameID: String?
    public var broadcasterType: String?
    public var profileImageURL: String?
    public var startedAt: String?
    /// Search answers offline channels too; a missing value counts as live.
    public var isLive: Bool?
    /// Watching here earns drops right now; the core stamps it on every Twitch row it serves.
    public var dropsEarnable: Bool?

    enum CodingKeys: String, CodingKey {
        case id
        case userLogin = "user_login"
        case userName = "user_name"
        case title
        case viewerCount = "viewer_count"
        case gameName = "game_name"
        case thumbnailURL = "thumbnail_url"
        case userID = "user_id"
        case gameID = "game_id"
        case broadcasterType = "broadcaster_type"
        case profileImageURL = "profile_image_url"
        case startedAt = "started_at"
        case isLive = "is_live"
        case dropsEarnable = "drops_earnable"
    }

    public init(id: String, userLogin: String, userName: String, title: String, viewerCount: Int,
                gameName: String, thumbnailURL: String = "", userID: String? = nil, gameID: String? = nil,
                broadcasterType: String? = nil, profileImageURL: String? = nil, startedAt: String? = nil,
                isLive: Bool? = nil, dropsEarnable: Bool? = nil) {
        self.id = id
        self.userLogin = userLogin
        self.userName = userName
        self.title = title
        self.viewerCount = viewerCount
        self.gameName = gameName
        self.thumbnailURL = thumbnailURL
        self.userID = userID
        self.gameID = gameID
        self.broadcasterType = broadcasterType
        self.profileImageURL = profileImageURL
        self.startedAt = startedAt
        self.isLive = isLive
        self.dropsEarnable = dropsEarnable
    }

    public var isPartner: Bool { broadcasterType == "partner" }
    /// Search lists offline channels as well; only an explicit `false` is offline.
    public var isOffline: Bool { isLive == false }
}

/// A category as `get_top_games_paginated` returns it (the TypeScript `TwitchCategory`).
public struct TwitchCategory: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var name: String
    public var boxArtURL: String?

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case boxArtURL = "box_art_url"
    }

    public init(id: String, name: String, boxArtURL: String? = nil) {
        self.id = id
        self.name = name
        self.boxArtURL = boxArtURL
    }
}

/// The `[items, cursor]` pair the paginated commands answer with.
public struct Page<Item: Decodable & Sendable>: Decodable, Sendable {
    public var items: [Item]
    public var cursor: String?

    public init(from decoder: Decoder) throws {
        var pair = try decoder.unkeyedContainer()
        items = try pair.decode([Item].self)
        cursor = try pair.isAtEnd ? nil : pair.decodeIfPresent(String.self)
    }
}

/// What `start_stream` and `change_stream_quality` answer: the URL the player loads, the quality
/// it serves, and every quality the stream offers in the core's order (`best` first).
public struct StreamStart: Decodable, Sendable, Equatable {
    public var url: String
    public var quality: String
    public var available: [String]
    /// How the resolver served a live stream: `turbo`, `subscribed`, `proxy` or `auth-only`.
    public var mode: String?
    /// Twitch's own ad-free entitlement is serving it, no relay in use.
    public var entitled: Bool
    /// The relay's region when one is in use.
    public var proxyRegion: String?

    enum CodingKeys: String, CodingKey {
        case url, quality, available, mode, entitled
        case proxyRegion = "proxy_region"
    }

    public init(url: String, quality: String, available: [String] = [], mode: String? = nil, entitled: Bool = false,
                proxyRegion: String? = nil) {
        self.url = url
        self.quality = quality
        self.available = available
        self.mode = mode
        self.entitled = entitled
        self.proxyRegion = proxyRegion
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        url = try c.decode(String.self, forKey: .url)
        quality = try c.decode(String.self, forKey: .quality)
        available = try c.decodeIfPresent([String].self, forKey: .available) ?? []
        mode = try c.decodeIfPresent(String.self, forKey: .mode)
        entitled = try c.decodeIfPresent(Bool.self, forKey: .entitled) ?? false
        proxyRegion = try c.decodeIfPresent(String.self, forKey: .proxyRegion)
    }
}

/// How far the player sits behind the relay's live edge (`get_relay_live_distance`), the
/// catch-up rate to run at, and the player's requests the relay saw since the last ask.
public struct LiveDistance: Decodable, Sendable, Equatable {
    public var behindMs: Int?
    public var behindNewestMs: Int?
    public var edgeSn: Int?
    public var edgePart: Int?
    public var edgeAgeMs: Int?
    public var rate: Double
    public var fetches: [String]
}

/// The part of the core's home snapshot the Browse list reads.
public struct HomeRecommended: Decodable, Sendable {
    public var recommended: [TwitchStream]
}

/// The Browse list: the home snapshot's recommended streams, which need no sign-in; without
/// them, the most watched categories' live streams until there are enough.
public enum Browse {
    public static func load(_ core: CoreBridge, count: Int = 20) async throws -> [TwitchStream] {
        if let recommended = await recommended(core), !recommended.isEmpty {
            return Array(recommended.prefix(count))
        }
        return try await topCategories(core, count: count)
    }

    /// Asks the core to refresh the recommended section, then reads the snapshot until the
    /// refresh lands (the core fills it in the background).
    static func recommended(_ core: CoreBridge) async -> [TwitchStream]? {
        let refresh: [String: Any?] = ["section": "recommended", "languages": [String](), "personalized": false]
        guard (try? await core.invoke("refresh_home_section", args: refresh)) != nil else { return nil }
        for _ in 0..<20 {
            if let snapshot = try? await core.invoke("get_home_snapshot", args: [:], as: HomeRecommended.self),
               !snapshot.recommended.isEmpty {
                return snapshot.recommended
            }
            try? await Task.sleep(nanoseconds: 250_000_000)
        }
        return nil
    }

    static func topCategories(_ core: CoreBridge, count: Int) async throws -> [TwitchStream] {
        let games = try await core.invoke(
            "get_top_games_paginated", args: ["cursor": nil, "limit": 20], as: Page<TwitchCategory>.self
        ).items
        var streams: [TwitchStream] = []
        for game in games where streams.count < count {
            let page = try await core.invoke(
                "get_streams_by_game", args: ["gameId": game.id, "cursor": nil, "limit": count],
                as: Page<TwitchStream>.self
            )
            let seen = Set(streams.map(\.id))
            streams += page.items.filter { !seen.contains($0.id) }
        }
        return Array(streams.prefix(count))
    }
}
