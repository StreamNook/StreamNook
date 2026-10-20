import Foundation

/// The streamer's picture on a go-live banner. notify.streamnook.app sends go-live pushes with
/// `mutable-content` set and the channel's Twitch profile picture under `sn.avatar`; the
/// notification service extension fetches it and attaches it before iOS shows the banner. These
/// are the rules it fetches by, Foundation only, so the extension stays small (it runs in a tight
/// memory budget, without the core) and the rules test on Linux.
public enum LiveAvatar {
    /// Twitch serves profile pictures from this one host. Nothing else is fetched: the push
    /// payload is the server's, but the rule here does not depend on trusting it.
    public static let hosts: Set<String> = ["static-cdn.jtvnw.net"]

    /// Larger than any Twitch profile picture (300 px PNGs are well under 200 KB), far below the
    /// 10 MB iOS takes for an image attachment.
    public static let maxBytes = 1 << 20

    /// How long the fetch may take in all. iOS gives the extension about 30 seconds; a banner
    /// that waits more than a few for its picture is worse than one without it.
    public static let timeout: TimeInterval = 5

    /// The picture a go-live push asks for, or nil when it names none or one this never fetches.
    public static func url(in userInfo: [AnyHashable: Any]) -> URL? {
        guard let sn = userInfo["sn"] as? [String: Any], sn["type"] as? String == "live",
              let raw = sn["avatar"] as? String else { return nil }
        return allowed(raw)
    }

    /// The URL when it is HTTPS on a Twitch picture host (no other port, no user info), else nil.
    public static func allowed(_ raw: String) -> URL? {
        guard !raw.isEmpty, raw.utf8.count <= 2048, let parts = URLComponents(string: raw) else { return nil }
        return allowed(parts)
    }

    /// The same rule for a URL already in hand, such as where a redirect leads.
    public static func allowed(_ url: URL?) -> URL? {
        guard let url, let parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return nil }
        return allowed(parts)
    }

    private static func allowed(_ parts: URLComponents) -> URL? {
        guard parts.scheme?.lowercased() == "https", let host = parts.host?.lowercased(), hosts.contains(host),
              parts.user == nil, parts.password == nil, parts.port == nil || parts.port == 443 else { return nil }
        return parts.url
    }

    /// How the picture goes on the banner. `communication` draws it as the round sender picture on
    /// the left, the way Messages does; it needs the app signed with the Communication
    /// Notifications entitlement, which only release signing can give (xtool's development
    /// signing cannot carry it). `attachment` is the thumbnail on the right, which any build can
    /// show, and the fallback when the communication style fails.
    public enum Style: Equatable, Sendable {
        case attachment, communication
    }

    /// The extension's Info.plist key release signing sets to true when the app's profile grants
    /// `com.apple.developer.usernotifications.communication`. Without it iOS would draw a
    /// communication-style banner as a plain one with no picture at all, so the default is the
    /// attachment.
    public static let communicationKey = "SNCommunicationStyle"

    /// The style this build can draw, from the extension's Info.plist.
    public static func style(_ info: [String: Any]?) -> Style {
        info?[communicationKey] as? Bool == true ? .communication : .attachment
    }

    /// Who a go-live banner is from, as the communication style names the sender.
    public struct Sender: Equatable, Sendable {
        public let channelID: String
        public let login: String

        public init(channelID: String, login: String) {
            self.channelID = channelID
            self.login = login
        }

        /// Stable per channel, so iOS treats every go-live from one streamer as one sender.
        public var senderID: String { "twitch:\(channelID.isEmpty ? login.lowercased() : channelID)" }

        /// The same id as the push's `thread-id`, so banners group the same way in both styles.
        public var conversationID: String { "live-\(channelID.isEmpty ? login.lowercased() : channelID)" }
    }

    /// The sender of a go-live push, or nil for any other push.
    public static func sender(in userInfo: [AnyHashable: Any]) -> Sender? {
        guard let sn = userInfo["sn"] as? [String: Any], sn["type"] as? String == "live",
              let login = sn["login"] as? String, !login.isEmpty else { return nil }
        return Sender(channelID: sn["channel_id"] as? String ?? "", login: login)
    }

    /// The image kinds a banner attachment takes, with the file extension iOS reads the type from.
    public enum Kind: String, Equatable, Sendable {
        case png, jpg, gif
    }

    /// What the downloaded bytes are, read from their first bytes rather than from the server's
    /// word: a PNG, JPEG or GIF signature, else nil.
    public static func kind(of head: Data) -> Kind? {
        let b = [UInt8](head.prefix(8))
        if b.starts(with: [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) { return .png }
        if b.starts(with: [0xFF, 0xD8, 0xFF]) { return .jpg }
        if b.starts(with: Array("GIF87a".utf8)) || b.starts(with: Array("GIF89a".utf8)) { return .gif }
        return nil
    }

    /// Whether a finished download is one to attach: HTTP 200, an image type, within the size cap
    /// both as announced and as received, and bytes that really are an image. Returns its kind.
    public static func accept(status: Int, mimeType: String?, expectedLength: Int64, received: Int, head: Data) -> Kind? {
        guard status == 200, mimeType?.lowercased().hasPrefix("image/") == true,
              expectedLength <= Int64(maxBytes), received > 0, received <= maxBytes else { return nil }
        return kind(of: head)
    }
}
