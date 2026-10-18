import SNKit
import SwiftUI

/// Settings > Chat: the phone's sections in the phone's order, every row bound to the core's
/// settings. Sections that only hold a list editor live in ChatListSettings.
struct ChatSettingsPanel: View {
    @Environment(SettingsStore.self) private var store
    @AppStorage("sn-mention-haptic") private var mentionHaptic = "on"

    var body: some View {
        landscape
        pollsAndPredictions
        chatsTab
        events
        layout
        names
        mentions
        LinkSettings()
        emotes
        emoteEffects
        input
        tabCompletion
        points
        behavior
        HiddenMessagesSettings()
        highlightAppearance
        HighlightPhrasesSettings()
        EventHighlightsSettings()
        UserHighlightsSettings()
        BadgeHighlightsSettings()
        spam
        CustomCommandsSettings()
        ModerationSettings()
        NicknamesSettings()
    }

    // MARK: Landscape

    private var landscape: some View {
        SettingsBlock(title: "Landscape Chat", detail: "How chat shares the screen when the phone is on its side.") {
            ChoiceRow(title: "Layout", path: "fullscreen_chat.phone_layout",
                      choices: [Choice("Over the video", "overlay"), Choice("Beside the video", "beside")], fallback: "overlay",
                      detail: "Turn the phone sideways and tap the chat button on the player. Chat floats over the video, or takes a column beside it.",
                      help: "Drag the column's edge to resize it either way. Floating chat is read-only; tap its edge for the background slider. A lower background lets more of the video show through. Width never goes past half the screen.") {
                if store.string("fullscreen_chat.phone_layout", "overlay") == "overlay" {
                    NestedSlider(title: "Background", path: "fullscreen_chat.opacity", range: 0...100, step: 5, fallback: 55,
                                 readout: { "\(Int($0))%" })
                }
                NestedSlider(title: "Width", path: "fullscreen_chat.width", range: 240...480, step: 20, fallback: 340,
                             readout: { "\(Int($0))px" })
                // Desktop's `auto` shows as Right; the arrow only shows while the side is Left.
                NestedChoice(title: "Side", path: "fullscreen_chat.side",
                             choices: [Choice("Left", "left"), Choice("Right", "right")], fallback: "auto",
                             resetPaths: store.string("fullscreen_chat.side", "auto") == "left" ? ["fullscreen_chat.side"] : [],
                             shown: { $0 == "left" ? "left" : "right" })
            }
        }
    }

    // MARK: Polls & Predictions

    private var pollsAndPredictions: some View {
        SettingsBlock(title: "Polls & Predictions", detail: "The live cards at the top of chat while the streamer runs a poll or a prediction.") {
            ToggleRow(title: "Polls", path: "show_polls", fallback: true, detail: "A live poll card with the running vote tally.",
                      help: "Collapsed opens a poll as its header bar, so it never takes over the top of chat; tap the header to expand it. Collapsing a poll sticks: it no longer reopens every time somebody votes.") {
                NestedToggle(title: "Collapsed", path: "chat_design.polls_start_collapsed")
            }
            ToggleRow(title: "Predictions", path: "show_predictions", fallback: true,
                      detail: "A live prediction card with the outcomes and how points are stacking up.",
                      help: "Top card picks which one is open when a poll and a prediction run at the same time. The other folds to a strip under it; tap the strip to open it.") {
                if store.bool("show_polls", true) {
                    NestedChoice(title: "Top card", path: "chat_overlay_order",
                                 choices: [Choice("Prediction", "prediction-first"), Choice("Poll", "poll-first")],
                                 fallback: "prediction-first")
                }
            }
        }
    }

    // MARK: Chats tab

    private var chatsTab: some View {
        SettingsBlock(title: "Chats Tab", detail: "Reading your chats with no stream playing.") {
            ToggleRow(title: "Keep screen on", path: "chats_keep_screen_on", fallback: true,
                      detail: "The screen stays on while the Chats tab is open, until 30 minutes pass without a touch.")
        }
    }

    // MARK: Events

    private var events: some View {
        SettingsBlock(title: "Chat Events", detail: "What live channel activity shows while you watch. Turn any of these off to keep chat clean.") {
            ToggleRow(title: "Redemptions", path: "show_channel_point_redemptions", fallback: true,
                      detail: "A chat row when someone redeems a channel point reward that does not post its own message, like a no-input reward.",
                      help: "Rewards that already post to chat are unaffected.")
            WideRow(title: "Platforms", detail: "Turn event kinds off per platform. Lit means shown.",
                    paths: ["chat_events.hidden_provider_events"]) {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Self.eventKinds, id: \.0) { platform, label, kinds in
                        HStack(alignment: .top, spacing: 8) {
                            HStack(spacing: 4) {
                                BrandLogo(id: platform, size: 13)
                                Text(label).font(.sn(12)).foregroundStyle(SN.textSecondary)
                            }
                            .frame(width: 84, alignment: .leading)
                            .padding(.top, 7)
                            SettingChips(path: "chat_events.hidden_provider_events",
                                         chips: kinds.map { Choice($0.1, "\(platform):\($0.0)") })
                        }
                    }
                }
                .padding(.top, 12)
            }
        }
    }

    /// Twitch and Kick only: the iPhone has no YouTube or TikTok chat.
    static let eventKinds: [(String, String, [(String, String)])] = [
        ("twitch", "Twitch", [("subscriptions", "Subscriptions"), ("gifts", "Gifts"), ("bits", "Bits"), ("raids", "Raids"),
                             ("milestones", "Milestones"), ("channel_points", "Channel points"), ("announcements", "Announcements")]),
        ("kick", "Kick", [("subscriptions", "Subscriptions"), ("gifts", "Gifts"), ("follows", "Follows"), ("raids", "Raids")]),
    ]

    // MARK: Layout, names, mentions

    private var layout: some View {
        SettingsBlock(title: "Message Layout", detail: "Spacing, text size, animation and timestamps.") {
            ToggleRow(title: "Dividers", path: "chat_design.show_dividers",
                      detail: "A thin line between messages so a fast chat is easier to scan.")
            ToggleRow(title: "Striped rows", path: "chat_design.alternating_backgrounds",
                      detail: "Gives every other message a slightly different background, in your theme's colors, so rows are easier to follow.")
            SliderRow(title: { "Spacing: \(Int($0))px" }, path: "chat_design.message_spacing", range: 0...20, step: 1, fallback: 8,
                      detail: "Blank space between messages; more room means fewer messages on screen.", accessibility: "Spacing")
            SliderRow(title: { "Text size: \(Int($0))px" }, path: "chat_design.font_size", range: 10...48, step: 1, fallback: 14,
                      detail: "Size of message text.", accessibility: "Text size")
            ChoiceRow(title: "Text weight", path: "chat_design.font_weight",
                      choices: [Choice("Light (300)", "300"), Choice("Normal (400)", "400"), Choice("Medium (500)", "500"),
                                Choice("Semi-Bold (600)", "600"), Choice("Bold (700)", "700")],
                      fallback: "400", menu: true, detail: "How heavy the message text is, from light to bold.", accessibility: "Font weight")
            ChoiceRow(title: "Message animation", path: "chat_design.message_entrance",
                      choices: [Choice("Instant", "none"), Choice("Fade", "fade"), Choice("Slide", "slide"), Choice("Rise", "rise")],
                      fallback: "slide",
                      detail: "How a new message arrives: a short fade or slide as each one lands. History loaded on join never animates, and it is skipped when motion is reduced.")
            SliderRow(title: { "History opacity: \(Int($0))%" }, path: "chat_design.backfill_opacity", range: 30...100, step: 5,
                      fallback: 100, detail: "Dim the scrollback that loads when you join a chat, so live messages stand out.",
                      accessibility: "History opacity")
            ToggleRow(title: "Timestamps", path: "chat_design.show_timestamps",
                      detail: "Shows the time each message was sent, next to the name.",
                      help: "Clock is 12-hour (7:42 PM) or 24-hour (19:42). Seconds turns 7:42 PM into 7:42:30 PM.") {
                NestedChoice(title: "Clock", path: "chat_design.timestamp_format", choices: [Choice("12h", "12h"), Choice("24h", "24h")],
                             fallback: "12h")
                NestedToggle(title: "Seconds", path: "chat_design.show_timestamp_seconds")
            }
        }
    }

    private var names: some View {
        SettingsBlock(title: "Names & Badges", detail: "How chatter names, badges and 7TV paints look.") {
            ChoiceRow(title: "Name style", path: "chat_design.username_style",
                      choices: [Choice("Plain", "plain"), Choice("Accent bar", "bar"), Choice("Chip / tag", "chip"),
                                Choice("Brackets   [name]", "brackets"), Choice("Color dot", "dot")],
                      fallback: "plain", menu: true,
                      detail: "How names stand out from the message: plain, or with a bar, chip, brackets, or dot.",
                      help: "Separator is the mark between a name and its message, like a colon or an arrow; /me messages never get one. Prefix color paints the separator, bar, dot, brackets, or chip with the chatter's own color or your theme accent.",
                      accessibility: "Name style") {
                NestedLine(title: "Separator", paths: ["chat_design.username_separator"]) {
                    SettingMenu(path: "chat_design.username_separator",
                                choices: [Choice("None", "none"), Choice("Colon   name:", "colon"), Choice("Dot   name ·", "dot"),
                                          Choice("Arrow   name ›", "arrow"), Choice("Pipe   name |", "pipe"), Choice("Dash   name \u{2013}", "dash")],
                                fallback: "none", accessibility: "Name separator", fullWidth: false)
                }
                if store.string("chat_design.username_separator", "none") != "none" || store.string("chat_design.username_style", "plain") != "plain" {
                    NestedChoice(title: "Prefix color", path: "chat_design.username_accent_source",
                                 choices: [Choice("User color", "user"), Choice("Theme accent", "accent")], fallback: "user")
                }
            }
            ToggleRow(title: "Readable colors", path: "chat_design.name_color_adjustment",
                      detail: "Nudges a chatter's color lighter or darker until it stands out from your theme.",
                      help: "Lighter on a dark theme, darker on a light one. The hue stays theirs. Off shows colors exactly as they set them.",
                      write: { ["chat_design.name_color_adjustment": $0 ? "hsl_loop" : "off"] },
                      isOn: { $0.string("chat_design.name_color_adjustment", "hsl_loop") != "off" })
            ToggleRow(title: "Badges", path: "chat_design.show_badges", fallback: true,
                      detail: "The platform's own badges next to names: moderator, subscriber, VIP and the rest.",
                      help: "A channel that uploads its own moderator and VIP badges to FrankerFaceZ shows them in its chat instead of the standard ones. Who is a moderator or VIP still comes from Twitch. Turn it off to see the standard badges everywhere.") {
                NestedToggle(title: "Custom mod & VIP badges", path: "chat_design.ffz_custom_role_badges", fallback: true, logo: "ffz")
            }
            BadgeScaleRow(enabled: store.bool("chat_design.show_badges", true) || store.bool("chat_design.show_third_party_badges", true))
            ToggleRow(title: "Add-on badges", path: "chat_design.show_third_party_badges", fallback: true,
                      detail: "Badges from 7TV, FFZ, Chatterino, Homies and the other badge services, plus StreamNook membership badges.",
                      help: "Services turns individual badge services off. Lit means shown.") {
                NestedLine(title: "Services", paths: ["chat_design.hidden_badge_providers"], stacked: true) {
                    SettingChips(path: "chat_design.hidden_badge_providers", chips: Self.badgeServices, logos: true)
                }
            }
            // The picture source (7TV or Twitch) is not read on the iPhone yet, so only Animation shows.
            ChoiceRow(title: "Profile pictures", path: "cosmetics.profile_picture_animation",
                      choices: [Choice("Animated", "always"), Choice("Still", "never")], fallback: "always",
                      detail: "Animated 7TV profile pictures play, or hold their first frame.",
                      shown: { $0 == "hover" ? "never" : $0 })
            ToggleRow(title: "@ prefix", path: "chat_design.show_at_sign", detail: "Writes every name as @name.")
            ChoiceRow(title: "Paint shadows", path: "cosmetics.paint_shadows",
                      choices: [Choice("All", "all"), Choice("One", "one"), Choice("None", "none")], fallback: "all",
                      detail: "Some paints stack several drop shadows for readability; keep them all, just one, or none if names look too noisy.")
        }
    }

    static let badgeServices = [
        Choice("StreamNook", "streamnook"), Choice("7TV", "7tv"), Choice("FFZ", "ffz"), Choice("BTTV", "bttv"),
        Choice("Chatterino", "chatterino"), Choice("Homies", "homies"), Choice("Homies custom", "homies-custom"),
        Choice("Moltorino", "moltorino"), Choice("Chatsen", "chatsen"), Choice("Chatty", "chatty"), Choice("DankChat", "dankchat"),
    ]

    private var mentions: some View {
        SettingsBlock(title: "Mentions & Replies", detail: "How a message that mentions you, and a reply, stand out, and how an @name reads in chat.") {
            SettingRow(title: "@name style", detail: "How an @mention reads in chat.",
                       help: "A /me message is italic as a whole: Like the message keeps a mention in it italic too, Never keeps it upright.") {
                EmptyView()
            } nested: {
                NestedSettings {
                    NestedChoice(title: "Weight", path: "chat_design.mention_weight",
                                 choices: [Choice("Regular", "regular"), Choice("Medium", "medium"), Choice("Bold", "bold")], fallback: "medium")
                    NestedChoice(title: "Italic", path: "chat_design.mention_italic",
                                 choices: [Choice("Like the message", "inherit"), Choice("Never", "never"), Choice("Always", "always")],
                                 fallback: "inherit")
                    NestedChoice(title: "Shape", path: "chat_design.mention_style",
                                 choices: [Choice("Plain", "plain"), Choice("Pill", "pill")], fallback: "plain")
                    MentionColorLine()
                }
            }
            ColorRow(title: "Highlight", path: "chat_design.mention_color", unset: "#ff4444",
                     detail: "The color that marks a message mentioning or replying to you.",
                     help: "Flash briefly lights the message up as it lands, so you spot it in a fast chat. The color edge stays either way.") {
                NestedToggle(title: "Flash", path: "chat_design.mention_animation", fallback: true)
            }
            SettingRow(title: "Vibration", detail: "A short buzz when a message says your name, on top of the highlight.") {
                Toggle("Vibration", isOn: Binding(get: { mentionHaptic != "off" }, set: { mentionHaptic = $0 ? "on" : "off" }))
                    .labelsHidden()
                    .tint(SN.accent)
            }
            ToggleRow(title: "Paint", path: "chat_design.paint_mentions_in_body", fallback: true,
                      detail: "Draws a mentioned name in that person's 7TV paint instead of a flat color.",
                      help: "Off shows mentions in the chatter's plain name color.", logo: "7tv")
            ColorRow(title: "Thread color", path: "chat_design.reply_color", unset: "#ff6b6b",
                     detail: "The color that marks replies in a thread.")
            ChoiceRow(title: "Context", path: "chat_design.reply_style",
                      choices: [Choice("Context line", "full"), Choice("@name", "mention"), Choice("Off", "off")], fallback: "full",
                      detail: "How a reply shows the message it answers: a context line above it, an @name at the start, or nothing.")
        }
    }

    // MARK: Emotes, input, completion, points, behavior

    private var emotes: some View {
        SettingsBlock(title: "Emotes", detail: "How big emotes are, how they animate, and how much they grow when you press and hold one.") {
            ChoiceRow(title: "Emoji style", path: "chat_design.emoji_style",
                      choices: [Choice("Apple", "apple"), Choice("Google", "google"), Choice("Twitter", "twitter"),
                                Choice("Facebook", "facebook"), Choice("System", "system")],
                      fallback: "apple", menu: true, detail: "Which set draws the emoji in messages. System uses your device's own.",
                      accessibility: "Emoji style")
            ToggleRow(title: "Personal emotes", path: "chat_design.show_personal_emotes", fallback: true,
                      detail: "Emotes from a chatter's own 7TV personal set. Off shows the text they typed instead.", logo: "7tv")
            // Touch has no hover: a saved `hover` reads as Never until another choice is picked.
            ChoiceRow(title: "Animation", path: "chat_design.animate_emotes",
                      choices: [Choice("Always", "always"), Choice("Never", "never")], fallback: "always",
                      detail: "Play animated emotes, or show only their first frame. Never is the lightest on the GPU in a fast chat.",
                      shown: { $0 == "hover" ? "never" : $0 })
            ToggleRow(title: "GIFs", path: "chat_design.show_chat_gifs", fallback: true,
                      detail: "Twitch lets Tier 2 and Tier 3 subscribers post GIFs. Off swaps each one for a small chip you can tap to reveal.",
                      help: "GIFs also follow Animation: Never shows the chip instead.", showsNestedWhenOff: true) {
                NestedSlider(title: "Size", path: "chat_design.gif_scale", range: 0.5...2, step: 0.05, fallback: 1,
                             readout: { "\($0.fixed(2))x" }, integer: false)
            }
            SliderRow(title: { "Size: \($0.fixed(2))x" }, path: "chat_design.emote_scale", range: 0.5...3, step: 0.05, fallback: 1,
                      detail: "Scales emotes in chat relative to the text, with 1.00x being the default size.", integer: false,
                      accessibility: "Emote size")
            SliderRow(title: { "Spacing: \($0.fixed(3))rem" }, path: "chat_design.emote_margin", range: -0.5...0.5, step: 0.025,
                      fallback: 0.125, detail: "Space on each side of an emote; go negative to let neighboring emotes overlap.",
                      integer: false, accessibility: "Emote spacing")
        }
    }

    private var emoteEffects: some View {
        SettingsBlock(title: "Emote Effects", detail: "Emotes that change the emote beside them, and Twitch's giant power-up emotes.") {
            ToggleRow(title: "Effects", path: "chat_design.ffz_emote_effects", fallback: true,
                      detail: "Applies FrankerFaceZ modifiers (wide, flips, rainbow, shake) to the emote before them, the way FFZ does.",
                      help: "Off shows modifier emotes as plain overlay emotes.", logo: "ffz")
            ToggleRow(title: "Modifiers", path: "chat_design.bttv_emote_modifiers", fallback: true,
                      detail: "Applies BetterTTV modifiers (w! wide, h! and v! flips, c! cursed, p! party, s! shake) to the emote after them, the way BetterTTV does.",
                      help: "Off shows the modifiers as plain emotes.", logo: "bttv")
            ToggleRow(title: "Giant emotes", path: "chat_design.giant_emotes", fallback: true,
                      detail: "Draws the last emote of a \"Gigantify an Emote\" power-up message at 4x below the message, like Twitch does.",
                      help: "Off shows the emote inline at its normal size. Position puts it under the message on the left, centered or on the right, or keeps it in the text at its normal size.") {
                NestedChoice(title: "Position", path: "chat_design.giant_emote_align",
                             choices: [Choice("Left", "left"), Choice("Center", "center"), Choice("Right", "right"), Choice("In the text", "inline")],
                             fallback: "center")
            }
        }
    }

    private var input: some View {
        SettingsBlock(title: "Chat Input", detail: "Small conveniences in the box where you type, and which buttons sit around it.") {
            ToggleRow(title: "Duplicate sends", path: "chat_input.bypass_duplicate",
                      detail: "Lets you send the same message twice in a row by adding an invisible character, so Twitch does not reject the second send.",
                      help: "Twitch normally blocks identical messages sent back to back. Handy for repeating an emote.")
            ToggleRow(title: "Spellcheck", path: "chat_input.spellcheck_enabled", fallback: true,
                      detail: "Underlines misspelled words as you type and offers corrections when you tap one. Emotes, chatters, commands and links are left alone.")
            ToggleRow(title: "Chat modes", path: "chat_input.show_room_modes", fallback: true,
                      detail: "Shows when a channel limits who can chat, like emote-only or slow mode.",
                      help: "The message box names the mode and keeps a small icon for it while you type. Tap the icons for what each mode means and whether you can still chat, for example as a moderator. During slow mode the send button counts down until you can send again. Shown for Twitch and Kick.")
            ToggleRow(title: "Hide placeholder", path: "chat_input.hide_placeholder",
                      detail: "Leaves the message box empty instead of prompting you to send a message. Notices you can act on, like read-only or subscriber-only mode, still show.")
            ToggleRow(title: "Hide emote button", path: "chat_input.hide_emote_button",
                      detail: "Removes the smiley from inside the message box. The emote picker still opens from the emote suggestions as you type.")
        }
    }

    private var tabCompletion: some View {
        SettingsBlock(title: "Tab Completion", shownTitle: "Suggestions", detail: "Type part of an emote name in chat to see matching emotes above the input. Swipe the strip to see more, tap one to use it.") {
            ToggleRow(title: "Emotes", path: "chat_input.emote_tab_complete_enabled", fallback: true,
                      detail: "Suggest matching emotes as you type.",
                      help: "Matching: Starts With needs the emote to begin with what you typed; Contains matches it anywhere in the name.") {
                NestedChoice(title: "Matching", path: "chat_input.emote_tab_complete_match_mode",
                             choices: [Choice("Starts With", "starts_with"), Choice("Contains", "includes")], fallback: "starts_with")
            }
            ToggleRow(title: "Chatter names", path: "chat_input.emote_tab_complete_include_chatters", fallback: true,
                      detail: "Also suggest display names of users currently in chat.")
        }
    }

    private var points: some View {
        SettingsBlock(title: "Channel Points", detail: "Bonus chest pickup on the channel you are watching.") {
            ToggleRow(title: "Bonus chests", path: "auto_claim_points_watching", fallback: true,
                      detail: "Collects the bonus chest on the stream you are watching the moment it appears.",
                      help: "When this is off, a claim button appears on the points icon so you can grab it yourself. Claiming on channels you are not watching is a separate opt-in plugin.")
        }
    }

    private var behavior: some View {
        SettingsBlock(title: "Chat Behavior", detail: "Deleted messages, shared chat, scrolling, and how much chat is kept.") {
            // Five segments do not fit a phone's width at 13 pt, so this one is a menu.
            ChoiceRow(title: "Deleted messages", path: "chat_design.deleted_message_style",
                      choices: [Choice("Strikethrough", "strikethrough"), Choice("Dimmed", "dimmed"), Choice("Italic", "italic"),
                                Choice("Keep", "keep"), Choice("Hidden", "hidden")],
                      fallback: "italic", menu: true,
                      detail: "How a message looks once a moderator deletes it or times out or bans its sender.")
            ToggleRow(title: "Hide shared chat", path: "chat_design.hide_shared_chat",
                      detail: "Hides messages that came from the other channel in a Twitch Shared Chat, so you only see this channel's own chatters.")
            ToggleRow(title: "Smooth resume", path: "chat_render.smooth_scroll_on_resume", fallback: true,
                      detail: "Animates the scroll back to the bottom when you tap Resume; auto-scroll for new messages stays instant.")
            SliderRow(title: { "Scrollback: \(Int($0)) messages" }, path: "chat_render.message_buffer_cap", range: 50...300, step: 10,
                      fallback: 100,
                      detail: "How many messages each chat keeps to scroll back through. Phones stop at 300: more than that costs memory and smoothness with nothing extra to see.",
                      accessibility: "Scrollback")
        }
    }

    private var highlightAppearance: some View {
        SettingsBlock(title: "Highlight Appearance", detail: "Applies to every highlight type below: phrases, usernames, badges, and events.") {
            SliderRow(title: { "Opacity: \(Int($0))%" }, path: "chat_highlights.appearance.opacity", range: 0...100, step: 5,
                      fallback: 20, detail: "How bright a highlighted row's tint is; 20% matches the original look.",
                      accessibility: "Highlight opacity")
        }
    }

    private var spam: some View {
        SettingsBlock(title: "Spam & Pyramid", detail: "How /spam and /pyramid send their lines.") {
            SpamSpeedRow()
            ToggleRow(title: "Vary repeats", path: "chat_burst.vary_repeats", fallback: true,
                      detail: "Adds invisible characters to a line that repeats the one before it, so Twitch doesn't drop it as a duplicate.",
                      help: "Twitch drops a message identical to your previous one sent less than 30 seconds earlier. With this off, /spam sends your text exactly as typed and repeats may be dropped.")
        }
    }
}

/// Badge size: the "1.00x" readout on the title line, the slider full width below; dimmed while
/// neither kind of badge shows.
private struct BadgeScaleRow: View {
    @Environment(SettingsStore.self) private var store
    let enabled: Bool
    @State private var live: Double?

    var body: some View {
        let value = live ?? min(max(store.double("chat_design.badge_scale", 1), 0.5), 2.5)
        SettingRow(title: "Badge size", detail: "How big badges draw, relative to the text.", paths: ["chat_design.badge_scale"],
                   trailing: { SliderReadout(text: "\(value.fixed(2))x") },
                   below: {
                       SettingSlider(path: "chat_design.badge_scale", range: 0.5...2.5, step: 0.05, fallback: 1,
                                     accessibility: "Badge size", integer: false, live: $live)
                           .padding(.top, 10)
                           .disabled(!enabled)
                           .opacity(enabled ? 1 : 0.4)
                   },
                   nested: { EmptyView() })
    }
}

/// /spam speed: the description carries the pause between lines.
private struct SpamSpeedRow: View {
    @Environment(SettingsStore.self) private var store
    @State private var live: Double?

    var body: some View {
        let ms = live ?? store.double("chat_burst.delay_ms", 1000)
        WideRow(title: "Speed",
                detail: "One line every \(Self.delay(ms)). Add --delay <ms> to a command to change it for one run.",
                help: "1 second is the pace Twitch allows a regular viewer in one channel. Moderators, VIPs and the broadcaster may go faster. Going faster than your role allows gets lines dropped and can silence you for an hour, and moderators may time you out for spam. A run stops by itself if Twitch drops three lines in a row or says you're timed out.",
                paths: ["chat_burst.delay_ms"]) {
            SettingSlider(path: "chat_burst.delay_ms", range: 10...5000, step: 10, fallback: 1000,
                          accessibility: "Milliseconds between lines", live: $live)
                .padding(.top, 10)
        }
    }

    static func delay(_ ms: Double) -> String {
        if ms < 1000 { return "\(Int(ms)) ms" }
        let seconds = ms / 1000
        return seconds.rounded() == seconds ? "\(Int(seconds)) s" : "\(seconds.fixed(2)) s"
    }
}

/// Mention Color: their color, or one color with a swatch (picking One color seeds #bf94ff).
private struct MentionColorLine: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let path = "chat_design.mention_text_color"
        let one = !store.string(path).isEmpty
        NestedLine(title: "Color", paths: [path], stacked: true) {
            HStack(spacing: 10) {
                if one { SettingColor(path: path, unset: "#bf94ff", label: "Mention color") }
                SettingSegments(choices: [Choice("Their color", "their"), Choice("One color", "one")],
                                selected: one ? "one" : "their", accessibility: "Mention color") { value in
                    Task { await store.set(path, value == "one" ? "#bf94ff" : nil) }
                }
            }
        }
    }
}

/// A one-line text field bound to a string setting: typing stays local and is saved a moment
/// after it stops, and on leaving the field, so no keystroke is lost to a round trip.
struct SettingTextField: View {
    @Environment(SettingsStore.self) private var store
    let path: String
    let placeholder: String
    var limit = 200
    var monospaced = false
    var multiline = false
    @State private var text: String?
    @State private var save: Task<Void, Never>?
    @FocusState private var focused: Bool

    var body: some View {
        let shown = Binding(get: { text ?? store.string(path) }, set: { value in
            let capped = String(value.prefix(limit))
            text = capped
            save?.cancel()
            save = Task {
                try? await Task.sleep(for: .milliseconds(400))
                guard !Task.isCancelled else { return }
                await store.set(path, capped)
            }
        })
        Group {
            if multiline {
                TextField("", text: shown, prompt: Text(placeholder).foregroundStyle(SN.textMuted), axis: .vertical).lineLimit(3...6)
            } else {
                TextField("", text: shown, prompt: Text(placeholder).foregroundStyle(SN.textMuted)).lineLimit(1)
            }
        }
        .font(monospaced ? .system(size: 13, design: .monospaced) : .sn(13))
        .foregroundStyle(SN.textPrimary)
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .focused($focused)
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background { RoundedRectangle(cornerRadius: 8, style: .continuous).fill(SN.surface) }
        .onChange(of: focused) { _, now in
            if !now, let text, text != store.string(path) {
                save?.cancel()
                Task { await store.set(path, text) }
            }
        }
    }
}
