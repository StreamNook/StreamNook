import Foundation

/// A channel's wallet faces as the core holds them (`get_channel_state` and the `channel-state`
/// pushes tagged `points` / `bits`): the parts the composer's wallet reads.
public struct WalletFaces: Sendable, Equatable {
    public var points: ChannelPoints?
    public var bits: ChannelBitsFace?

    public init(points: ChannelPoints? = nil, bits: ChannelBitsFace? = nil) {
        self.points = points
        self.bits = bits
    }

    /// Points on here: a balance to show and rewards to spend it on.
    public var pointsOn: ChannelPoints? { points?.enabled == true ? points : nil }
    /// Bits on here.
    public var bitsOn: ChannelBitsFace? { bits?.enabled == true ? bits : nil }
    /// Anything to spend in this channel.
    public var hasAny: Bool { pointsOn != nil || bitsOn != nil }

    private struct State: Decodable {
        var points: ChannelPoints?
        var bits: ChannelBitsFace?
    }

    /// `get_channel_state`'s answer (null before the channel is watched).
    public static func parseState(_ json: String) -> WalletFaces? {
        // `try?` flattens the optional: nil for a failed decode and for a JSON null alike.
        guard let state = try? JSONDecoder().decode(State?.self, from: Data(json.utf8)) else { return nil }
        return WalletFaces(points: state.points, bits: state.bits)
    }

    /// One `channel-state` push: the login and what it changes, or nil for other sections.
    public enum Push: Sendable, Equatable {
        case points(login: String, ChannelPoints?)
        case bits(login: String, ChannelBitsFace?)
    }

    private struct Head: Decodable { var section: String; var login: String }
    private struct PointsPush: Decodable { var points: ChannelPoints? }
    private struct BitsPush: Decodable { var bits: ChannelBitsFace? }

    public static func parsePush(_ json: String) -> Push? {
        let data = Data(json.utf8)
        guard let head = try? JSONDecoder().decode(Head.self, from: data) else { return nil }
        let login = head.login.lowercased()
        switch head.section {
        case "points": return (try? JSONDecoder().decode(PointsPush.self, from: data)).map { .points(login: login, $0.points) }
        case "bits": return (try? JSONDecoder().decode(BitsPush.self, from: data)).map { .bits(login: login, $0.bits) }
        default: return nil
        }
    }
}

/// `get_channel_rewards`, `get_cached_channel_rewards` and the `channel-rewards` push: the
/// rewards with Rust's verdict and words on each.
public struct ChannelRewardsSnapshot: Decodable, Sendable, Equatable {
    public var login: String
    public var rewards: [ChannelReward]
    public var balance: Int64?
    public var fetchedAtMS: Int64

    enum CodingKeys: String, CodingKey {
        case login, rewards, balance
        case fetchedAtMS = "fetched_at_ms"
    }
}

public struct ChannelReward: Decodable, Sendable, Equatable, Identifiable, Hashable {
    public var id: String
    public var title: String
    public var cost: Int
    public var prompt: String?
    public var imageURL: String?
    public var backgroundColor: String
    public var description: String?
    public var availability: RewardAvailability
    public var view: RewardView

    enum CodingKeys: String, CodingKey {
        case id, title, cost, prompt, description, availability, view
        case imageURL = "image_url"
        case backgroundColor = "background_color"
    }

    /// What the detail view says it does: the streamer's prompt, else Rust's description.
    public var about: String? {
        [prompt, description].compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }.first { !$0.isEmpty }
    }

    /// A reward gone from the latest list: it cannot be redeemed.
    public func gone() -> ChannelReward {
        var copy = self
        copy.availability = RewardAvailability(redeemable: false, reason: "disabled", cooldownUntilMS: nil,
                                               affordable: availability.affordable, label: "Gone",
                                               detail: "This reward is no longer offered")
        return copy
    }
}

/// Rust's verdict for one reward.
public struct RewardAvailability: Decodable, Sendable, Equatable, Hashable {
    public var redeemable: Bool
    /// `not_enough_points`, `on_cooldown`, `paused`, ... (models/drops.rs `RewardBlock`).
    public var reason: String?
    public var cooldownUntilMS: Int64?
    public var affordable: Bool
    /// A few words for the list row.
    public var label: String?
    /// The full sentence for the detail view.
    public var detail: String?

    enum CodingKeys: String, CodingKey {
        case redeemable, reason, affordable, label, detail
        case cooldownUntilMS = "cooldown_until_ms"
    }

    public init(redeemable: Bool, reason: String?, cooldownUntilMS: Int64?, affordable: Bool, label: String?, detail: String?) {
        self.redeemable = redeemable
        self.reason = reason
        self.cooldownUntilMS = cooldownUntilMS
        self.affordable = affordable
        self.label = label
        self.detail = detail
    }

    /// A running cooldown's end.
    public var cooldownEnd: Date? {
        guard reason == "on_cooldown", let ms = cooldownUntilMS else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
    }
}

/// Rust's words for a reward's controls.
public struct RewardView: Decodable, Sendable, Equatable, Hashable {
    /// `highlight`, `sub_message`, `random_emote`, `choose_emote`, `modify_emote`, `input`, `standard`.
    public var kind: String
    public var takesText: Bool
    public var costLabel: String
    public var action: String
    public var busy: String
    public var placeholder: String?
    public var hint: String?

    enum CodingKeys: String, CodingKey {
        case kind, action, busy, placeholder, hint
        case takesText = "takes_text"
        case costLabel = "cost_label"
    }

    /// It opens an emote picker before anything is spent.
    public var picksEmote: Bool { kind == "choose_emote" || kind == "modify_emote" }
}

/// `redeem_reward`: what a redeem came to, with the toast to show.
public struct RedeemOutcome: Decodable, Sendable, Equatable {
    public var success: Bool
    public var message: String
    public var newBalance: Int?
    public var unlockedEmote: UnlockedEmote?

    enum CodingKeys: String, CodingKey {
        case success, message
        case newBalance = "new_balance"
        case unlockedEmote = "unlocked_emote"
    }
}

public struct UnlockedEmote: Decodable, Sendable, Equatable {
    public var id: String
    public var name: String
    public var imageURL: String

    enum CodingKeys: String, CodingKey {
        case id, name
        case imageURL = "image_url"
    }
}

/// `get_modifiable_emotes`: the channel's sub emotes and their modified versions.
public struct ModifiableEmote: Decodable, Sendable, Equatable, Identifiable, Hashable {
    public var id: String
    public var token: String
    public var modifications: [EmoteModification]

    public var imageURL: String { "https://static-cdn.jtvnw.net/emoticons/v2/\(id)/default/dark/2.0" }
}

public struct EmoteModification: Decodable, Sendable, Equatable, Identifiable, Hashable {
    public var id: String
    /// `MOD_BW`, `MOD_HF`, ...
    public var modifierID: String
    public var token: String

    enum CodingKeys: String, CodingKey {
        case id, token
        case modifierID = "modifier_id"
    }

    public var imageURL: String { "https://static-cdn.jtvnw.net/emoticons/v2/\(id)/default/dark/2.0" }

    /// The modifier's name and what it does, as the desktop picker words them.
    public var words: (name: String, detail: String) {
        switch modifierID {
        case "MOD_BW": ("Black & White", "Grayscale version")
        case "MOD_HF": ("Flipped", "Horizontally mirrored")
        case "MOD_SG": ("Sunglasses", "Wearing cool shades")
        case "MOD_SQ": ("Squashed", "Vertically squished")
        case "MOD_TK": ("Thinking", "With thinking hand")
        case "MOD_CV": ("Cursed", "Spooky distortion")
        default: (modifierID, "")
        }
    }
}
