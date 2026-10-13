import Foundation

/// A live stream as `get_streams_by_game` returns it (the TypeScript `TwitchStream`).
public struct TwitchStream: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var userLogin: String
    public var userName: String
    public var title: String
    public var viewerCount: Int
    public var gameName: String
    public var thumbnailURL: String

    enum CodingKeys: String, CodingKey {
        case id
        case userLogin = "user_login"
        case userName = "user_name"
        case title
        case viewerCount = "viewer_count"
        case gameName = "game_name"
        case thumbnailURL = "thumbnail_url"
    }

    public init(id: String, userLogin: String, userName: String, title: String, viewerCount: Int,
                gameName: String, thumbnailURL: String = "") {
        self.id = id
        self.userLogin = userLogin
        self.userName = userName
        self.title = title
        self.viewerCount = viewerCount
        self.gameName = gameName
        self.thumbnailURL = thumbnailURL
    }
}

/// A category as `get_top_games_paginated` returns it (the TypeScript `TwitchCategory`).
public struct TwitchCategory: Codable, Sendable, Equatable, Identifiable {
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

/// What `start_stream` answers: the URL the player loads and the quality it serves.
public struct StreamStart: Decodable, Sendable, Equatable {
    public var url: String
    public var quality: String
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
