import Foundation
import Testing
@testable import SNKit

/// A core holding one settings object, which announces a write the way the real core does.
private final class LiveSettingsCore: CoreBridge, @unchecked Sendable {
    let events = CoreEvents()
    let kind = "test"
    private let lock = NSLock()
    private var settings: [String: Any]

    init(_ settings: [String: Any]) { self.settings = settings }

    func start(dirs: CoreDirs, version: String, vaultKey: Data, codecs: [String]) throws {}
    func setWebCookies(origin: String, cookiesJSON: String) throws {}
    func lifecycle(_ state: String) {}

    func invoke(_ cmd: String, _ argsJSON: String) async throws -> String {
        guard cmd == "get_settings" else { throw CoreFailure.command(json: #""unknown""#) }
        return String(decoding: snapshot(), as: UTF8.self)
    }

    private func snapshot() -> Data {
        lock.lock()
        defer { lock.unlock() }
        return (try? JSONSerialization.data(withJSONObject: settings)) ?? Data("{}".utf8)
    }

    /// Saves one group key and announces it, as `set_settings_fields` does.
    func write(_ group: String, _ key: String, _ value: Any) {
        lock.lock()
        var block = settings[group] as? [String: Any] ?? [:]
        block[key] = value
        settings[group] = block
        lock.unlock()
        events.emit(name: ChatStyleFeed.settingsChanged, payloadJSON: #"{"source":null,"keys":["\#(group)"]}"#, target: nil)
    }
}

@Suite struct ChatSettingsApplyTests {
    /// Every Chat page choice a room draws with, set away from its default, reaches the style.
    @Test func everyChatPageChoiceReachesTheStyle() throws {
        let settings: [String: Any] = [
            "chat_design": [
                "show_dividers": true, "alternating_backgrounds": true, "message_spacing": 12, "font_size": 18,
                "font_weight": 600, "message_entrance": "rise", "backfill_opacity": 60, "show_timestamps": true,
                "show_timestamp_seconds": true, "username_style": "chip", "username_separator": "colon",
                "username_accent_source": "theme", "show_at_sign": true, "mention_weight": "bold",
                "mention_italic": "always", "mention_style": "pill", "mention_color": "#00ff00",
                "mention_animation": false, "paint_mentions_in_body": false, "reply_color": "#0000ff",
                "reply_style": "mention", "emoji_style": "google", "show_personal_emotes": false,
                "animate_emotes": "never", "show_chat_gifs": false, "gif_scale": 1.5, "emote_scale": 2,
                "emote_margin": 0.25, "giant_emotes": false, "giant_emote_align": "left",
                "deleted_message_style": "hidden", "hide_shared_chat": true, "badge_scale": 1.5,
                "mention_text_color": "#123456", "link_color": "#abcdef", "link_underline": false,
            ] as [String: Any],
            "chat_input": [
                "bypass_duplicate": true, "spellcheck_enabled": false, "show_room_modes": false,
                "hide_placeholder": true, "hide_emote_button": true, "emote_tab_complete_enabled": false,
            ],
            "chat_render": ["smooth_scroll_on_resume": false, "message_buffer_cap": 200] as [String: Any],
            "chat_events": ["hidden_provider_events": ["twitch:gift", "kick:follow"]],
            "cosmetics": ["paint_shadows": "none"],
        ]
        let s = ChatStyle(settingsJSON: try JSONSerialization.data(withJSONObject: settings))
        #expect(s.showDividers && s.alternatingBackgrounds && s.messageSpacing == 12 && s.fontSize == 18)
        #expect(s.fontWeight == 600 && s.messageEntrance == "rise" && s.backfillOpacity == 60)
        #expect(s.showTimestamps && s.showTimestampSeconds && s.showAtSign)
        #expect(s.usernameStyle == "chip" && s.usernameSeparator == "colon" && s.usernameAccentSource == "theme")
        #expect(s.mentionWeight == "bold" && s.mentionItalic == "always" && s.mentionStyle == "pill")
        #expect(s.mentionColor == "#00ff00" && !s.mentionAnimation && !s.paintMentionsInBody)
        #expect(s.replyColor == "#0000ff" && s.replyStyle == "mention" && s.mentionTextColor == "#123456")
        #expect(s.emojiStyle == "google" && !s.showPersonalEmotes && s.animateEmotes == "never")
        #expect(!s.showChatGifs && s.gifScale == 1.5 && s.emoteScale == 2 && s.emoteMargin == 0.25)
        #expect(!s.giantEmotes && s.giantEmoteAlign == "left" && s.badgeScale == 1.5)
        #expect(s.deletedMessageStyle == "hidden" && s.hideSharedChat)
        #expect(s.linkColor == "#abcdef" && !s.linkUnderline && s.paintShadows == "none")
        #expect(s.bypassDuplicate && !s.spellcheck && !s.showRoomModes && s.hidePlaceholder && s.hideEmoteButton)
        #expect(!s.suggestionsEnabled && !s.smoothScrollOnResume && s.bufferCap == 200)
        #expect(s.hiddenEvents == ["twitch:gift", "kick:follow"])
    }

    /// A change saved while a room is open restyles it, with no reopening.
    @Test func aSavedChangeRestylesAnOpenRoom() async throws {
        let core = LiveSettingsCore(["chat_design": ["show_dividers": false, "message_entrance": "slide"]])
        var styles = ChatStyleFeed.styles(core: core).makeAsyncIterator()

        let opened = try #require(await styles.next())
        #expect(!opened.showDividers && opened.messageEntrance == "slide")

        core.write("chat_design", "show_dividers", true)
        let divided = try #require(await styles.next())
        #expect(divided.showDividers && divided.messageEntrance == "slide")

        core.write("chat_design", "message_entrance", "fade")
        let faded = try #require(await styles.next())
        #expect(faded.showDividers && faded.messageEntrance == "fade")

        core.write("chat_events", "hidden_provider_events", ["twitch:raid"])
        let filtered = try #require(await styles.next())
        #expect(filtered.hiddenEvents == ["twitch:raid"])
    }

    /// A numeric choice ("Text weight", the alert check interval) saves as the number the core's
    /// field holds: a whole number as an Int, since the core refuses `300.0` for a `u32` and with it
    /// the whole save.
    @Test func aNumericChoiceSavesAsAWholeNumber() throws {
        let weight = try #require(SettingsPath.number("300"))
        #expect(weight as? Int == 300)
        let encoded = try JSONSerialization.data(withJSONObject: ["chat_design.font_weight": weight])
        #expect(String(decoding: encoded, as: UTF8.self) == #"{"chat_design.font_weight":300}"#)
        #expect(SettingsPath.number("1.5") as? Double == 1.5)
        #expect(SettingsPath.number("slide") == nil)
    }
}
