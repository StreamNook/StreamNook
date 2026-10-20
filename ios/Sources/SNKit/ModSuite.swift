import Foundation

/// One step of the swipe slider (Rust `touch_mod_slider::TouchStep`).
public struct SwipeStep: Decodable, Sendable, Equatable {
    /// "delete" | "timeout" | "ban"
    public var action: String
    public var seconds: Int?
    /// "Delete", "10m", "Ban".
    public var label: String
    /// What the chip says while armed: "Delete message", "Time out 10m", "Ban".
    public var chip: String
    /// Between the theme's warning (0) and error (1); nil is Delete's calm gray.
    public var tone: Double?

    public var isBan: Bool { action == "ban" }

    public init(action: String, seconds: Int?, label: String, chip: String, tone: Double?) {
        self.action = action
        self.seconds = seconds
        self.label = label
        self.chip = chip
        self.tone = tone
    }

    /// What `moderate_chatter` takes for this step on a message.
    public func action(messageID: String) -> [String: Any] {
        switch action {
        case "delete": return ["kind": "delete", "messageId": messageID]
        case "ban": return ["kind": "ban"]
        default: return ["kind": "timeout", "seconds": seconds ?? 600]
        }
    }
}

/// The swipe slider for one row width (Rust `touch_mod_slider::TouchSlider`): the ruler, the
/// rules and every word the chip shows. The phone only follows the finger.
public struct SwipeSlider: Decodable, Sendable, Equatable {
    public var enabled: Bool
    /// "preview" | "rail" | "capsule"
    public var style: String
    public var steps: [SwipeStep]
    public var dead: Double
    public var spacing: Double
    public var starts: [Double]
    public var end: Double
    public var banDwellMs: Int
    public var banStill: Double
    public var hysteresis: Double
    public var leash: Double
    public var engage: Double
    public var cancelWords: String
    public var holdBanWords: String
    public var nudgeWords: String
    /// Where the thumb landed, from the row's left edge: the ruler starts here.
    public var start: Double
    public var quickSeconds: Int

    enum CodingKeys: String, CodingKey {
        case enabled, style, steps, dead, spacing, starts, end, banDwellMs = "ban_dwell_ms", banStill = "ban_still",
             hysteresis, leash, engage, cancelWords = "cancel_words", holdBanWords = "hold_ban_words",
             nudgeWords = "nudge_words", start, quickSeconds = "quick_seconds"
    }

    /// The step under a pull of `slide` points, nil in the cancel zone. The gap before Ban keeps
    /// showing the step before it (Rust `step_at`).
    public func step(at slide: Double) -> Int? {
        guard slide >= dead, !steps.isEmpty, spacing > 0 else { return nil }
        let unit = Int(((slide - dead) / spacing).rounded(.down))
        let last = steps.count - 1
        if !steps[last].isBan { return min(unit, last) }
        if unit >= last + 1 { return last }
        if last == 0 { return nil }
        return min(unit, last - 1)
    }

    /// The step to show, keeping `current` until the pull is `hysteresis` past its edge (Rust
    /// `with_hysteresis`).
    public func step(at slide: Double, keeping current: Int?) -> Int? {
        let next = step(at: slide)
        guard let cur = current, let nx = next, nx != cur else { return next }
        if nx > cur { return slide >= starts[nx] + hysteresis ? nx : cur }
        return slide <= starts[cur] - hysteresis ? nx : cur
    }

    /// The pull the finger is allowed: never past the end of the last zone.
    public func clamp(_ slide: Double) -> Double { min(max(0, slide), end - 0.5) }
}

/// One moderator action in a room (Rust `room_mod_log::LogEntry`).
public struct RoomModAction: Codable, Sendable, Equatable, Identifiable {
    public var id: String
    public var channel: String
    /// Twitch's action name: "ban", "timeout", "delete", "slow", ...
    public var action: String
    public var moderator: String
    public var target: String?
    public var targetID: String?
    /// "kay_lee timed out gus for 10m".
    public var line: String
    /// The reason or the deleted message.
    public var quote: String?
    public var atMs: Int64
    /// A ban or timeout Unban would lift.
    public var liftable: Bool

    enum CodingKeys: String, CodingKey {
        case id, channel, action, moderator, target, targetID = "target_id", line, quote, atMs = "at_ms", liftable
    }
}

extension CoreBridge {
    /// The swipe slider for a press `startX` points from the left of a row `width` points wide, on
    /// `name`'s message (the chip names them).
    public func swipeSlider(width: Double, startX: Double = 0, name: String = "", targetID: String? = nil,
                            badges: [String]? = nil, channel: String? = nil) async throws -> SwipeSlider {
        try await invoke("get_touch_mod_slider", args: [
            "provider": "twitch", "rowWidth": width, "startX": startX, "targetName": name,
            "targetId": targetID, "targetBadges": badges, "channel": channel,
        ], as: SwipeSlider.self)
    }

    /// A room's recent moderator actions, newest first.
    public func roomModLog(channel: String) async throws -> [RoomModAction] {
        try await invoke("get_room_mod_log", args: ["channel": channel], as: [RoomModAction].self)
    }

    /// Messages AutoMod is holding in a room.
    public func heldQueue(channel: String) async throws -> [HeldMessage] {
        try await invoke("get_automod_queue", args: ["channel": channel], as: [HeldMessage].self)
    }

    /// Adds or removes one of the swipe slider's timeout lengths; answers the list as stored.
    public func editSliderTimeouts(_ op: String, seconds: Int) async throws -> [Int] {
        try await invoke("edit_mod_slider_timeouts", args: ["op": op, "seconds": seconds], as: [Int].self)
    }

    /// A typed length ("45m", "1h30m", "2d") as a step, or the core's words for why not.
    public func parseTimeout(_ text: String) async throws -> SwipeStep {
        let raw = try await invoke("parse_mod_timeout", args: ["text": text])
        let step = try JSONDecoder().decode(ParsedStep.self, from: Data(raw.utf8))
        return SwipeStep(action: step.action, seconds: step.seconds, label: step.label, chip: step.label, tone: nil)
    }

    /// Runs one action on a chatter through the core's mod door.
    public func moderate(action: [String: Any], broadcasterID: String, targetUserID: String, targetName: String,
                         useSavedReason: Bool) async throws -> ModOutcome {
        let request: [String: Any] = [
            "provider": "twitch", "broadcasterId": broadcasterID, "targetUserId": targetUserID,
            "targetName": targetName, "action": action, "useSavedReason": useSavedReason,
        ]
        return try await invoke("moderate_chatter", args: ["request": request], as: ModOutcome.self)
    }
}

/// A typed length as the core answers it (`ModStep`).
private struct ParsedStep: Decodable {
    var action: String
    var seconds: Int?
    var label: String
}

/// What the viewer may do to one message, by Twitch's rules: the core's
/// `moderation::message_reach`, mirrored here because the hold menu decides while the finger
/// rests and cannot wait on the core. Helix refuses deleting the broadcaster's messages or
/// another moderator's, and banning or timing out the broadcaster, yourself, or (unless the
/// broadcaster does it) a moderator.
public enum MessageReach: Sendable, Equatable {
    case all, deleteOnly, noDelete, nothing

    public var canDelete: Bool { self == .all || self == .deleteOnly }
    public var canTimeout: Bool { self == .all || self == .noDelete }

    /// `badges` as names ("moderator") or with versions ("moderator/1").
    public static func of(viewerModerates: Bool, viewerIsBroadcaster: Bool, viewerID: String,
                          targetID: String, badges: [String]) -> MessageReach {
        guard viewerModerates || viewerIsBroadcaster, !targetID.isEmpty else { return .nothing }
        func has(_ name: String) -> Bool { badges.contains { $0 == name || $0.hasPrefix(name + "/") } }
        let own = !viewerID.isEmpty && targetID == viewerID
        if has("broadcaster") || (own && viewerIsBroadcaster) || has("staff") || has("admin") || has("global_mod") {
            return .nothing
        }
        if own { return .deleteOnly }
        if has("moderator") || has("lead_moderator") { return viewerIsBroadcaster ? .noDelete : .nothing }
        return .all
    }
}
