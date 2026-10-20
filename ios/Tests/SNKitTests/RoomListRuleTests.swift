import Testing
@testable import SNKit

/// The chat screens as the rule sees them: lists on rooms, which list holds each room, and what
/// each list was handed. A list mounts into the window and claims its room unless covered (as
/// `ChatListView.didMoveToWindow` does); every frame each list asks the rule.
private struct Screens {
    struct List {
        var room: String
        var inWindow = true
        var covered: Bool
        var drawn = false
    }

    var lists: [Int: List] = [:]
    var holder: [String: Int] = [:]
    private var next = 0

    mutating func mount(_ room: String, covered: Bool) -> Int {
        next += 1
        lists[next] = List(room: room, covered: covered)
        if !covered { claim(next) }
        return next
    }

    mutating func unmount(_ id: Int) {
        lists[id] = nil
        // The holder is held weakly: a list that went away holds nothing.
        for (room, held) in holder where held == id { holder[room] = nil }
    }

    mutating func cover(_ id: Int, _ covered: Bool) {
        lists[id]?.covered = covered
        if !covered, let list = lists[id], list.inWindow, holder[list.room] != id { claim(id) }
    }

    private mutating func claim(_ id: Int) {
        guard let room = lists[id]?.room else { return }
        holder[room] = id
        lists[id]?.drawn = true
    }

    mutating func frame() {
        for id in lists.keys.sorted() {
            guard let list = lists[id] else { continue }
            let held = holder[list.room].flatMap { lists[$0] }
                .map { RoomListRule.Holder(inWindow: $0.inWindow, covered: $0.covered) }
            switch RoomListRule.step(inWindow: list.inWindow, covered: list.covered,
                                     holds: holder[list.room] == id, holder: held) {
            case .draw: lists[id]?.drawn = true
            case .claim: claim(id)
            case .wait: break
            }
        }
    }

    /// The list a person sees on `room` holds it and was handed its rows.
    func draws(_ id: Int) -> Bool {
        guard let list = lists[id] else { return false }
        return holder[list.room] == id && list.drawn
    }
}

@Suite struct RoomListRuleTests {
    @Test func aCoveredListOrOneOffScreenTakesNothing() {
        #expect(RoomListRule.step(inWindow: true, covered: true, holds: true, holder: nil) == .wait)
        #expect(RoomListRule.step(inWindow: false, covered: false, holds: true, holder: nil) == .wait)
    }

    @Test func aVisibleHolderKeepsTheRoomAndAGoneOrCoveredOneGivesItUp() {
        let visible = RoomListRule.Holder(inWindow: true, covered: false)
        #expect(RoomListRule.step(inWindow: true, covered: false, holds: true, holder: visible) == .draw)
        #expect(RoomListRule.step(inWindow: true, covered: false, holds: false, holder: visible) == .wait)
        #expect(RoomListRule.step(inWindow: true, covered: false, holds: false, holder: nil) == .claim)
        #expect(RoomListRule.step(inWindow: true, covered: false, holds: false,
                                  holder: .init(inWindow: true, covered: true)) == .claim)
        #expect(RoomListRule.step(inWindow: true, covered: false, holds: false,
                                  holder: .init(inWindow: false, covered: false)) == .claim)
    }

    /// Chats with rooms A and B, Live on A, then A to B to A under the player, then the player
    /// closes: the list a person sees always draws its room.
    @Test func aRoomFollowsTheListOnTopThroughTheHandOffAndEverySwitch() {
        var s = Screens()
        // Chats shows A.
        var chats = s.mount("A", covered: false)
        s.frame()
        #expect(s.draws(chats))

        // Live on A: the player covers Chats and its chat shows A.
        s.cover(chats, true)
        var player = s.mount("A", covered: false)
        s.frame()
        #expect(s.draws(player))
        #expect(!s.draws(chats))

        // Switch to B under the player: both screens rebuild their list for B (either order).
        for order in [true, false] {
            var t = s
            let c: Int
            let p: Int
            if order {
                t.unmount(chats); c = t.mount("B", covered: true)
                t.unmount(player); p = t.mount("B", covered: false)
            } else {
                t.unmount(player); p = t.mount("B", covered: false)
                t.unmount(chats); c = t.mount("B", covered: true)
            }
            t.frame()
            #expect(t.draws(p), "player draws B, order \(order)")
            #expect(!t.draws(c))
            if order { s = t; chats = c; player = p }
        }

        // And back to A.
        s.unmount(chats); chats = s.mount("A", covered: true)
        s.unmount(player); player = s.mount("A", covered: false)
        s.frame()
        #expect(s.draws(player))
        #expect(!s.draws(chats))

        // The player closes: Chats is on top again and draws A, handed every row A kept.
        s.unmount(player)
        s.cover(chats, false)
        s.frame()
        #expect(s.draws(chats))

        // And B, once Chats shows it.
        s.unmount(chats); chats = s.mount("B", covered: false)
        s.frame()
        #expect(s.draws(chats))
    }

    /// The player's chat mounts before the player covers Chats (one SwiftUI pass, either order):
    /// the next frame still settles on the player's list.
    @Test func theListOnTopWinsWhateverOrderTheScreensUpdateIn() {
        var s = Screens()
        let chats = s.mount("A", covered: false)
        let player = s.mount("A", covered: false)
        s.cover(chats, true)
        s.frame()
        #expect(s.draws(player))
        #expect(!s.draws(chats))
    }
}
