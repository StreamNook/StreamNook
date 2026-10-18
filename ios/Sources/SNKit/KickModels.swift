import Foundation

/// One account row of the core's roster (`get_account_roster`, `account-roster-changed`).
public struct RosterAccount: Decodable, Sendable, Equatable {
    /// `twitch`, `kick`, `youtube`, `tiktok`, or an extra (`twitch_drops`, `seventv`).
    public var id: String
    /// `connected`, `expired` or `not_connected`.
    public var status: String
    public var name: String?
    public var handle: String?
    public var avatarURL: String?
    /// The id that platform's chat stamps on this account's messages (Kick's numeric account id).
    public var accountID: String?

    enum CodingKeys: String, CodingKey {
        case id, status, name, handle
        case avatarURL = "avatar_url"
        case accountID = "account_id"
    }

    public init(id: String, status: String, name: String? = nil, handle: String? = nil, avatarURL: String? = nil,
                accountID: String? = nil) {
        self.id = id
        self.status = status
        self.name = name
        self.handle = handle
        self.avatarURL = avatarURL
        self.accountID = accountID
    }

    /// Connected means the core holds a usable token; an expired row reads as signed out.
    public var connected: Bool { status == "connected" }
}

/// The platform accounts and the feature credentials, as the core lists them.
public struct AccountRoster: Decodable, Sendable, Equatable {
    public var main: [RosterAccount]
    public var extras: [RosterAccount]

    public init(main: [RosterAccount] = [], extras: [RosterAccount] = []) {
        self.main = main
        self.extras = extras
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        main = (try? c.decodeIfPresent([RosterAccount].self, forKey: .main)) ?? []
        extras = (try? c.decodeIfPresent([RosterAccount].self, forKey: .extras)) ?? []
    }

    enum CodingKeys: String, CodingKey { case main, extras }

    public func account(_ id: String) -> RosterAccount? { main.first { $0.id == id } }
}

/// A channel on the follow list the core keeps for platforms other than Twitch
/// (`get_provider_follows`).
public struct ProviderFollow: Decodable, Sendable, Equatable {
    public var provider: String
    public var channel: String
    public var displayName: String?
    public var subscribed: Bool
    public var imported: Bool

    enum CodingKeys: String, CodingKey {
        case provider, channel, subscribed, imported
        case displayName = "display_name"
    }

    public init(provider: String, channel: String, displayName: String? = nil, subscribed: Bool = false,
                imported: Bool = false) {
        self.provider = provider
        self.channel = channel
        self.displayName = displayName
        self.subscribed = subscribed
        self.imported = imported
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        provider = try c.decode(String.self, forKey: .provider)
        channel = try c.decode(String.self, forKey: .channel)
        displayName = try c.decodeIfPresent(String.self, forKey: .displayName)
        subscribed = (try? c.decodeIfPresent(Bool.self, forKey: .subscribed)) ?? false
        imported = (try? c.decodeIfPresent(Bool.self, forKey: .imported)) ?? false
    }
}

/// `kick_account_sync`'s answer: how many follows came in.
public struct KickSync: Decodable, Sendable, Equatable {
    public var imported: Int
    public var subscribed: Int

    public init(imported: Int, subscribed: Int = 0) {
        self.imported = imported
        self.subscribed = subscribed
    }
}

/// Opening Kick's login form in the web sheet.
public enum KickLogin {
    /// The page Kick's mobile site lands on for `/login` (its home page, form closed).
    public static func opensForm(_ url: URL) -> Bool {
        let host = url.host?.lowercased() ?? ""
        return (host == "kick.com" || host == "www.kick.com") && url.path.lowercased().hasPrefix("/login")
    }

    /// Presses Kick's own Log In button until a password field shows (20 tries, 500 ms apart).
    public static let script = """
    (function () {
      var tries = 0;
      var timer = setInterval(function () {
        tries += 1;
        if (document.querySelector('input[type="password"]') || tries > 20) { clearInterval(timer); return; }
        var button = document.querySelector('button[data-testid="login"]');
        if (button) { button.click(); }
      }, 500);
    })();
    """
}

/// The words the Kick account row and its toasts use.
public enum KickWords {
    /// While the sign-in sheet is up.
    public static let waiting = "Waiting for you to sign in…"

    /// What a connect that did not finish says: the core's own sentence, capitalized; anything
    /// about a cancel reads as one; an empty or overlong answer reads as not completed.
    public static func outcome(_ raw: String) -> String {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("\""), text.hasSuffix("\""), text.count >= 2 { text = String(text.dropFirst().dropLast()) }
        if text.isEmpty || text == "undefined" || text.count > 160 { return "Sign-in was not completed" }
        if text.range(of: "cancel", options: .caseInsensitive) != nil { return "Sign-in was cancelled" }
        return text.prefix(1).uppercased() + text.dropFirst()
    }

    /// The row's second line when no step is showing.
    public static func status(connected: Bool, follows: Int) -> String {
        guard connected else { return "Not signed in" }
        guard follows > 0 else { return "Signed in" }
        return follows == 1 ? "1 follow" : "\(follows) follows"
    }

    /// The refresh toast.
    public static func found(_ count: Int) -> String {
        count == 1 ? "Found 1 Kick channel you follow" : "Found \(count) Kick channels you follow"
    }

    public static let refreshFailed = "Could not load your Kick follows. Try again in a moment."
}

/// A page of another platform's streams (`provider_directory`, `provider_search`).
public struct StreamPage: Decodable, Sendable {
    public var streams: [TwitchStream]
    public var cursor: String?

    public init(streams: [TwitchStream], cursor: String? = nil) {
        self.streams = streams
        self.cursor = cursor
    }
}

/// A category on another platform (Rust `ProviderCategory`).
public struct ProviderCategory: Decodable, Sendable, Hashable, Identifiable {
    public var provider: String
    public var id: String
    public var name: String
    public var thumbnail: String
    public var viewerCount: Int
    /// The tile's words for `viewerCount`, from the core's one compact format.
    public var viewersLine: String?

    enum CodingKeys: String, CodingKey {
        case provider, id, name, thumbnail
        case viewerCount = "viewer_count"
        case viewersLine = "viewers_line"
    }

    public init(provider: String = "kick", id: String, name: String, thumbnail: String = "", viewerCount: Int = 0) {
        self.provider = provider
        self.id = id
        self.name = name
        self.thumbnail = thumbnail
        self.viewerCount = viewerCount
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        provider = (try? c.decodeIfPresent(String.self, forKey: .provider)) ?? "kick"
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        thumbnail = (try? c.decodeIfPresent(String.self, forKey: .thumbnail)) ?? ""
        viewerCount = (try? c.decodeIfPresent(Int.self, forKey: .viewerCount)) ?? 0
        viewersLine = try? c.decodeIfPresent(String.self, forKey: .viewersLine)
    }

    /// The same tile Twitch's categories use: the thumbnail as the art (none: a quiet block).
    public var tile: TwitchCategory {
        TwitchCategory(id: id, name: name, boxArtURL: thumbnail.isEmpty ? nil : thumbnail, viewerCount: viewerCount,
                       viewersLine: viewersLine)
    }

    /// Kick has no category search: the loaded list filtered by name.
    public static func filter(_ list: [ProviderCategory], _ query: String) -> [ProviderCategory] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return list }
        return list.filter { $0.name.lowercased().contains(q) }
    }
}

/// `provider_categories`' answer.
public struct CategoryPage: Decodable, Sendable {
    public var categories: [ProviderCategory]
}

extension TwitchStream {
    /// A row from a Kick list, stamped Kick even when the answer left it unstamped.
    public var asKick: TwitchStream {
        var row = self
        if row.provider == nil || row.provider?.isEmpty == true { row.provider = "kick" }
        if row.key == nil { row.key = "kick:\(row.userLogin.lowercased())" }
        if row.watchURL == nil { row.watchURL = "https://kick.com/\(row.userLogin)" }
        return row
    }
}
