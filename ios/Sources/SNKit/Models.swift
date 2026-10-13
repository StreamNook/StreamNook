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

/// The Browse list: the most watched categories' live streams, until there are enough.
public enum Browse {
    public static func load(_ core: CoreBridge, count: Int = 20) async throws -> [TwitchStream] {
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
