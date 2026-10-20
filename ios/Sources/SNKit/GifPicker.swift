import Foundation

// Twitch chat GIFs, as the core answers them (commands/gifs.rs): whether this account may post GIFs
// in a channel, GIPHY results (the key never leaves the core), and a send through Twitch's own
// GIF message. The same shapes and words as Android's picker (src/services/gifService.ts).

/// `get_gif_picker_status`: the channel's answer.
public struct GifPickerStatus: Decodable, Sendable, Equatable {
    /// The broadcaster allows GIFs in this channel.
    public var isEnabled: Bool
    /// This account may send them here (Twitch's server-side Tier 2/3 gate).
    public var isAllowlisted: Bool
    /// Enabled, allowlisted and a usable search key: the section can do its job.
    public var canUse: Bool

    enum CodingKeys: String, CodingKey {
        case isEnabled = "is_enabled", isAllowlisted = "is_allowlisted", canUse = "can_use"
    }

    public init(isEnabled: Bool, isAllowlisted: Bool, canUse: Bool) {
        self.isEnabled = isEnabled
        self.isAllowlisted = isAllowlisted
        self.canUse = canUse
    }

    /// Why the section cannot be used here, as Android's pane says it.
    public var unavailableLine: String? {
        if canUse { return nil }
        return isEnabled ? "Tier 2 and Tier 3 subscribers can post GIFs in this channel."
            : "This channel has GIFs turned off."
    }
}

/// `search_gifs`: one result.
public struct GifItem: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var title: String
    /// Small looping preview for the grid.
    public var previewURL: String
    /// The full asset, sent to Twitch verbatim.
    public var url: String

    enum CodingKeys: String, CodingKey {
        case id, title, url, previewURL = "preview_url"
    }

    public init(id: String, title: String, previewURL: String, url: String) {
        self.id = id
        self.title = title
        self.previewURL = previewURL
        self.url = url
    }
}

/// `send_gif_message`: Twitch's answer.
public struct SendGifOutcome: Decodable, Sendable, Equatable {
    public var sent: Bool
    /// Twitch's own error enum, e.g. TEMPORARILY_UNAVAILABLE.
    public var error: String?
    /// Cooldown in seconds. Honored; a refused send is never retried on its own.
    public var secondsUntilCanSend: Int

    enum CodingKeys: String, CodingKey {
        case sent, error, secondsUntilCanSend = "seconds_until_can_send"
    }

    public init(sent: Bool, error: String? = nil, secondsUntilCanSend: Int = 0) {
        self.sent = sent
        self.error = error
        self.secondsUntilCanSend = secondsUntilCanSend
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sent = try c.decodeIfPresent(Bool.self, forKey: .sent) ?? false
        error = try c.decodeIfPresent(String.self, forKey: .error)
        secondsUntilCanSend = try c.decodeIfPresent(Int.self, forKey: .secondsUntilCanSend) ?? 0
    }

    /// Why a send was refused, in Android's words (`gifSendMessage`); empty when it went out.
    public var notSentLine: String {
        if sent { return "" }
        if error == "TEMPORARILY_UNAVAILABLE" { return "GIFs are temporarily unavailable. Try again in a moment." }
        if secondsUntilCanSend > 0 { return "Slow down a moment, you can send another GIF in \(secondsUntilCanSend)s." }
        return "That GIF could not be sent."
    }
}
