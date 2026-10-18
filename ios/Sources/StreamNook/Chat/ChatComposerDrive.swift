#if DEBUG
import Foundation
import SNKit

/// Composer drive steps (see Drive.swift), on the chat stage or the watch layer's room:
///
/// - `chatpicktab:ID` selects a picker tab (`favorites`, `emoji`, `twitch`, `bttv`, `7tv`, `ffz`)
/// - `chatpicksearch:TEXT` searches the picker
/// - `chatpickclose` closes the picker
/// - `chatgate:KIND` shows the room as the core words it when a mode blocks the viewer
///   (`subs`, `emote`, `follow`, `slow`, or `off`), by feeding the room a ROOMSTATE frame
/// - `chatslow` starts the slow-mode countdown as if a message had just gone out
/// - `chatfail[:WHY]` shows the draft as a message Twitch did not take
/// - `chatnotice:TEXT` prints a line only the viewer sees
/// - `chatpickerlog` logs the picker's tabs and sections
/// - `chatfocus[:off]` gives the box the keyboard (or takes it away)
/// - `chatdraft:TEXT` types TEXT into the box (`_` for spaces); `cheerlog` logs the cheer bar's
///   state. Send is ChatDrive's `chatsend`, which runs only on FakeCore (`chatlab:core:LOGIN`
///   under `fake:` gives the stage the fake wallet core)
@MainActor
enum ComposerDrive {
    static func run(_ name: String, _ value: String, session: ChatSession?) async -> Bool {
        guard let session else { return false }
        let composer = ComposerRegistry.shared.models[ObjectIdentifier(session)]
        switch name {
        case "chatpicktab":
            composer?.picker.select(value)
        case "chatpicksearch":
            composer?.picker.query = value
        case "chatpickclose":
            composer?.panel = .none
        case "chatgate":
            session.pipeline.ingest(roomState(value, channel: session.channel))
        case "chatslow":
            composer?.debugSlow()
        case "chatfail":
            composer?.debugFailure(value.isEmpty ? "same as your last message" : value)
        case "chatnotice":
            composer?.notice(value.isEmpty ? "Your message was not sent: your message was blocked by AutoMod" : value)
        case "chatfocus":
            composer?.requestFocus(value != "off")
            try? await Task.sleep(for: .milliseconds(800))
        case "chatdraft":
            composer?.draft = value.replacingOccurrences(of: "_", with: " ")
            try? await Task.sleep(for: .milliseconds(600))
        case "cheerlog":
            let flow = composer?.cheer
            log.notice("cheer draft \(flow?.draft?.total ?? -1) problems \(flow?.draft?.problems.count ?? -1) status \(String(describing: flow?.status.map { "\($0)" }), privacy: .public)")
        case "chatpickerlog":
            guard let picker = composer?.picker else { return true }
            let tabs = picker.tabs.map { "\($0)=\(picker.count($0).map(String.init) ?? "-")" }.joined(separator: ",")
            let sections = picker.sections.map { "\($0.title)(\($0.emotes.count),\($0.columns))" }.joined(separator: ",")
            log.notice("picker tab \(picker.tab, privacy: .public) tabs \(tabs, privacy: .public) sections \(sections, privacy: .public)")
        default:
            return false
        }
        return true
    }

    /// A ROOMSTATE frame worded the way the core words it for a viewer the mode holds back.
    private static func roomState(_ kind: String, channel: String) -> String {
        func mode(_ kind: String, _ label: String, _ compact: String, _ short: String, _ detail: String) -> [String: Any] {
            ["kind": kind, "label": label, "compact": compact, "short": short, "detail": detail, "applies_to_you": true]
        }
        var modes: [String: Any] = ["modes": [Any](), "slow_seconds": 0]
        switch kind {
        case "subs":
            modes = ["modes": [mode("subs_only", "Subscriber-only chat", "Subscriber-only", "Sub-only", "Only subscribers can chat.")],
                     "prompt": "Subscriber-only chat", "prompt_short": "Sub-only", "slow_seconds": 0,
                     "blocked": "subs_only", "reason": "Subscribers only"]
        case "emote":
            modes = ["modes": [mode("emote_only", "Emote-only chat", "Emote-only", "Emote-only", "Messages can only contain emotes.")],
                     "prompt": "Emote-only chat", "prompt_short": "Emote-only", "slow_seconds": 0,
                     "blocked": "emote_only", "reason": "Emote-only: pick an emote"]
        case "follow":
            modes = ["modes": [mode("followers_only", "Followers-only chat (10 minutes)", "Followers-only (10m)", "Followers 10m",
                                    "Only accounts that have followed for 10 minutes can chat.")],
                     "prompt": "Follow to chat", "prompt_short": "Follow to chat", "slow_seconds": 0,
                     "blocked": "followers_only", "reason": "Follow to chat"]
        case "slow":
            modes = ["modes": [mode("slow", "Slow mode (30 seconds)", "Slow mode (30s)", "Slow 30s", "Chatters wait 30 seconds between messages."),
                               mode("unique", "Unique chat", "Unique chat", "Unique", "Messages that repeat an earlier one are not allowed.")],
                     "prompt": "Slow mode (30s)", "prompt_short": "Slow 30s", "slow_seconds": 30]
        default:
            break
        }
        let frame: [String: Any] = ["type": "ROOMSTATE", "channel": channel, "modes": modes]
        let data = (try? JSONSerialization.data(withJSONObject: frame)) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }
}
#endif
