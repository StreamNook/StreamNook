// Which on-screen list draws a chat room. A room's rows are made once and taken once, so a room
// shown by two lists at once (Chats under the full player, the player's chat on top) is drawn by
// one of them: the list a person can see. Foundation only, so it is tested on Linux; the chat list
// asks it every frame.

/// One list's view of the room it shows.
public enum RoomListRule {
    /// What a list does this frame.
    public enum Step: Equatable, Sendable {
        /// It holds the room: it takes the room's new rows.
        case draw
        /// It takes the room over, and is handed every row the room kept.
        case claim
        /// Another list draws the room, or this one cannot be seen: it takes nothing.
        case wait
    }

    /// The list that holds a room, as the rule needs to know it.
    public struct Holder: Equatable, Sendable {
        public var inWindow: Bool
        public var covered: Bool
        public init(inWindow: Bool, covered: Bool) {
            self.inWindow = inWindow
            self.covered = covered
        }
    }

    /// Only a list in the window and not covered by another screen may hold a room. It keeps the
    /// room while it holds it; it takes it over when the room has no holder, or its holder has
    /// left the window or been covered (the player opened over Chats).
    public static func step(inWindow: Bool, covered: Bool, holds: Bool, holder: Holder?) -> Step {
        guard inWindow, !covered else { return .wait }
        if holds { return .draw }
        if let holder, holder.inWindow, !holder.covered { return .wait }
        return .claim
    }
}
