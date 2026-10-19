import Foundation

/// The look a chat room draws with, kept current: read from the core when the room opens, then
/// again each time the core announces a settings change, so a choice made on the Chat page
/// restyles the rooms already open instead of waiting for them to be opened again.
public enum ChatStyleFeed {
    /// What the core emits after every settings write (field edits, resets, list edits, restores).
    public static let settingsChanged = "streamnook-settings-updated"

    /// The room's style now, then again after each settings change that alters it.
    public static func styles(core: CoreBridge) -> AsyncStream<ChatStyle> {
        // Listening before the first read, so a change that lands during it is not missed.
        let changes = core.events.stream(settingsChanged)
        return AsyncStream { continuation in
            let task = Task {
                var last: ChatStyle?
                func read() async {
                    guard let json = try? await core.invoke("get_settings", "{}") else { return }
                    let style = ChatStyle(settingsJSON: Data(json.utf8))
                    guard style != last else { return }
                    last = style
                    continuation.yield(style)
                }
                await read()
                for await _ in changes {
                    await read()
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

/// Chat events as one vocabulary across the apps: the category a notice belongs to, so the
/// categories the viewer hid per platform (`chat_events.hidden_provider_events`, entries like
/// "twitch:gift") apply here as they do on the desktop and Android. The same map as the
/// desktop's chat events.
public enum ChatEventCategory {
    static let byMsgType: [String: String] = [
        "sub": "subscription", "resub": "subscription", "primepaidupgrade": "subscription",
        "giftpaidupgrade": "subscription", "anongiftpaidupgrade": "subscription",
        "standardpayforward": "subscription", "communitypayforward": "subscription",
        "membership": "subscription", "sharedchatnotice": "subscription",
        "subgift": "gift", "submysterygift": "gift", "anonsubgift": "gift", "anonsubmysterygift": "gift",
        "membergift": "gift", "giftedsub": "gift", "tiktok_gift": "gift", "kick_gift": "gift", "kick_gifted": "gift",
        "raid": "raid", "unraid": "raid",
        "announcement": "announcement", "ritual": "announcement",
        "viewermilestone": "milestone", "watchstreak": "milestone", "bitsbadgetier": "milestone",
        "charitydonation": "cheer", "cheer": "cheer", "bits": "cheer", "superchat": "cheer",
        "superticker": "cheer", "supersticker": "cheer",
        "tiktok_follow": "follow", "tiktok_share": "follow", "follow": "follow", "kick_follow": "follow",
    ]

    /// "provider:category" for an event row, nil for an ordinary message. A Twitch cheer is an
    /// ordinary message carrying a bit count, so it counts as one here.
    public static func key(of message: ChatMessage) -> String? {
        let provider = message.provider.isEmpty ? "twitch" : message.provider
        let type = message.metadata.msgType ?? message.tags["msg-id"]
        var category = type.flatMap { byMsgType[$0] }
        if category == nil, provider == "twitch" {
            let bits = message.metadata.bitsAmount ?? Int(message.tags["bits"] ?? "") ?? 0
            if bits > 0 { category = "cheer" }
        }
        return category.map { "\(provider):\($0)" }
    }

    /// Whether the viewer hid this row's kind of event on its platform.
    public static func isHidden(_ message: ChatMessage, _ hidden: [String]) -> Bool {
        guard !hidden.isEmpty, let key = key(of: message) else { return false }
        return hidden.contains(key)
    }
}
