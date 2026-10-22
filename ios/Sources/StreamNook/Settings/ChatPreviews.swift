import SNKit
import SwiftUI
import UIKit

// Live previews for the Chat settings whose look is their point: a few sample rows drawn by the
// chat's own row builder and cell, with the style the page holds right now, so a change on the
// page shows here at once and can never drift from what chat draws. Each sits as a row of its
// section, a quiet strip of chat background with nothing to tap.

// MARK: The style the page holds

/// A slider's value while a finger is on it, so a preview follows the drag before it is saved.
/// A slider sets its path while dragging and clears it on release.
@Observable @MainActor
final class ChatPreviewDraft {
    static let shared = ChatPreviewDraft()
    private(set) var values: [String: Double] = [:]

    func set(_ path: String, _ value: Double?) {
        if values[path] != value { values[path] = value }
    }
}

@MainActor
enum ChatPreviewStyle {
    /// The settings groups `ChatStyle` reads, with keys a saved file may leave out.
    private static let groups: [String: [String]] = [
        "chat_design": ["mention_text_color", "link_color"],
        "chat_events": ["hidden_provider_events"],
        "chat_input": [],
        "chat_render": [],
        "cosmetics": ["paint_shadows"],
    ]

    /// The chat style as the page shows it: the core's values with any write still in flight and
    /// any slider under a finger laid over them.
    static func style(_ store: SettingsStore) -> ChatStyle {
        var root = store.values
        let draft = ChatPreviewDraft.shared.values
        for (group, extras) in groups {
            var block = root[group] as? [String: Any] ?? [:]
            for key in Set(block.keys).union(extras) {
                let path = "\(group).\(key)"
                block[key] = draft[path].map { $0 as Any } ?? store.value(path)
            }
            root[group] = block
        }
        var style = ChatStyle()
        style.apply(root)
        return style
    }
}

// MARK: The previews

/// Message Layout: two rows of history under a new arrival, for dividers, spacing, text size,
/// timestamps and history opacity. The newest row plays its arrival again when that changes.
struct MessageLayoutPreview: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let style = ChatPreviewStyle.style(store)
        let s = ChatPreviewSamples(store)
        ChatPreviewRows(rows: [
            s.row("layout-1", .pixelfox, "that clutch was unreal", minutesAgo: 3, history: true),
            s.row("layout-2", .mossy, "gg, that boss had no chance", minutesAgo: 2, history: true),
            s.row("layout-3", .quietstorm, "first time catching this live, hi chat", minutesAgo: 0, live: true),
        ], style: style, arrival: style.messageEntrance)
        .chatPreviewSurface()
    }
}

/// Names & Badges: a moderator, a chatter with add-on badges and a 7TV paint, and a plain one,
/// for the name style, separator, prefix color, @ prefix, badges and paint shadows.
struct NamesPreview: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let s = ChatPreviewSamples(store)
        ChatPreviewRows(rows: [
            s.row("names-1", .pixelfox, "that clutch was unreal", minutesAgo: 3,
                  badges: [ChatPreviewSamples.moderator, ChatPreviewSamples.subscriber]),
            s.row("names-2", .quietstorm, "first time catching this live, hi chat", minutesAgo: 2,
                  badges: [ChatPreviewSamples.house, ChatPreviewSamples.sevenTV], paint: ChatPreviewSamples.paint),
            s.row("names-3", .mossy, "gg, that boss had no chance", minutesAgo: 1),
        ], style: ChatPreviewStyle.style(store))
        .chatPreviewSurface()
    }
}

/// Mentions & Replies: an @name inside a /me message, a message that mentions you and a reply
/// to you, for the @name style, both colors and the reply context. The two edges flash again
/// when a color or Flash changes.
struct MentionsPreview: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let style = ChatPreviewStyle.style(store)
        let s = ChatPreviewSamples(store)
        ChatPreviewRows(rows: [
            s.row("mentions-1", .lootgoblin, [.text("high-fives "), .mention(text: "@quietstorm", login: "quietstorm", color: "#DA70D6")],
                  minutesAgo: 3) { $0.isAction = true },
            s.row("mentions-2", .pixelfox, [.mention(text: "@you", login: "you", color: "#BF94FF"), .text(" that clutch was unreal")],
                  minutesAgo: 2, live: true) { $0.isMentioned = true },
            s.row("mentions-3", .mossy, [.text("yes, best play of the night")], minutesAgo: 1, live: true) { meta in
                meta.isReplyToMe = true
                meta.replyInfo = ChatPreviewSamples.decode(ChatReplyInfo.self, [
                    "parent_msg_id": "preview-parent", "parent_display_name": "you", "parent_msg_body": "did anyone else catch that?",
                    "parent_user_id": "0", "parent_user_login": "you",
                ])
            },
        ], style: style, flash: [style.mentionColor, style.replyColor, String(style.mentionAnimation)].joined(separator: "|"))
        .chatPreviewSurface()
    }
}

/// Chat Events: a gift sub between two messages. A kind hidden for Twitch takes the card out.
struct EventRowsPreview: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let s = ChatPreviewSamples(store)
        let style = ChatPreviewStyle.style(store)
        // A room's buffer drops hidden kinds before layout; the preview does the same.
        ChatPreviewRows(rows: [
            s.row("events-1", .pixelfox, "that clutch was unreal", minutesAgo: 2),
            s.giftRow(),
            s.row("events-3", .lootgoblin, "welcome to the club", minutesAgo: 1),
        ].filter { !ChatRoomBuffer.hides($0.message, style.hiddenEvents) }, style: style)
        .chatPreviewSurface()
    }
}

/// Chat Behavior: a timed-out message and a deleted one between two ordinary ones, drawn the
/// way Deleted messages asks; Hidden takes both out.
struct DeletedMessagePreview: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let s = ChatPreviewSamples(store)
        ChatPreviewRows(rows: [
            s.row("deleted-1", .pixelfox, "that clutch was unreal", minutesAgo: 3),
            s.row("deleted-2", .lootgoblin, "free subs at the link in my bio", minutesAgo: 2, moderation: .timedOut(seconds: 600)),
            s.row("deleted-3", .mossy, "gg, that boss had no chance", minutesAgo: 2),
            s.row("deleted-4", .quietstorm, "the ending is a twist, she was the villain", minutesAgo: 1, moderation: .deleted),
        ], style: ChatPreviewStyle.style(store))
        .chatPreviewSurface()
    }
}

/// Links: one message with a link, for its color, underline and short label.
struct LinksPreview: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let s = ChatPreviewSamples(store)
        let url = "https://www.twitch.tv/videos/2280391155?t=1h02m"
        let label = store.bool("chat_design.shorten_links", true) ? ChatPreviewSamples.linkLabel(url) : url
        ChatPreviewRows(rows: [
            s.row("links-1", .pixelfox, [.text("the clutch is at "), .link(label: label, url: url)], minutesAgo: 1),
        ], style: ChatPreviewStyle.style(store))
        .chatPreviewSurface()
    }
}

/// Profile pictures: with 7TV, a moving 7TV picture (StreamNook's gold badge) playing or held on
/// its first frame as Animation asks; with Twitch, the platform's own still picture (the
/// StreamNook logo). The same black disc a profile sheet draws.
struct ProfilePicturePreview: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let platform = store.string("cosmetics.profile_picture_source", "seventv") == "twitch"
        let moves = store.string("cosmetics.profile_picture_animation", "always") == "always"
        let art = "streamnook-house:streamnook-subscriber"
        let picture = ChatPreviewSamples.decode(AvatarPick.self, moves ? ["still": art, "animated": art] : ["still": art])
        HStack(spacing: 10) {
            if platform {
                ZStack {
                    Circle().fill(Color.black)
                    if let logo = BrandLogo.image("streamnook") {
                        Image(uiImage: logo).resizable().interpolation(.high).scaledToFill()
                    }
                }
                .frame(width: 36, height: 36)
                .clipShape(Circle())
                .overlay { Circle().strokeBorder(Color.white.opacity(0.1), lineWidth: 0.5) }
            } else {
                SevenTvAvatar(platformURL: nil, picture: picture, size: 36)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text("StreamNook").font(.sn(14, 500)).foregroundStyle(SN.textPrimary)
                Text(platform ? "Twitch picture" : "7TV picture").font(.sn(12)).foregroundStyle(SN.textSecondary)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .chatPreviewSurface()
    }
}

/// Chat Input: the message bar in a followers-only room in slow mode, with the mode's icon when
/// Chat modes is on, the placeholder and emote button unless they are hidden, and the inbox tray
/// at the end Inbox button picks (the chat menu at the other). Drawn as the composer draws it;
/// nothing in it can be typed into or pressed.
struct ChatInputPreview: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        let modes = store.bool("chat_input.show_room_modes", true)
        let placeholder = !store.bool("chat_input.hide_placeholder")
        let emote = !store.bool("chat_input.hide_emote_button")
        let right = store.string("chat_input.inbox_button_side", "left") == "right"
        HStack(alignment: .center, spacing: 6) {
            if right { menu } else { tray }
            HStack(spacing: 4) {
                if emote {
                    Text("😀").font(.sn(20)).frame(width: 36, height: 36)
                }
                // A space keeps the line's height while the placeholder is hidden.
                Text(placeholder ? "Send a message" : " ")
                    .font(.sn(15))
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
                    .padding(.vertical, 8)
                    .padding(.leading, emote ? 0 : 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if modes {
                    HStack(spacing: 4) {
                        Image(systemName: RoomModeSymbol.name("followers_only")).font(ComposerSymbol.font)
                        Text("+1")
                            .font(.system(size: 10, weight: .semibold).monospacedDigit())
                            .frame(minWidth: 13)
                    }
                    .foregroundStyle(Color(ChatTheme.textSecondary))
                    .padding(.horizontal, 4)
                    .frame(height: 36)
                }
                Image(systemName: "slash.circle")
                    .font(ComposerSymbol.font)
                    .foregroundStyle(SN.textSecondary)
                    .frame(width: 32, height: 36)
            }
            .padding(.leading, 4)
            .padding(.trailing, 5)
            .frame(minHeight: 40)
            .glassEffect(.regular, in: .rect(cornerRadius: 12))
            if right { tray } else { menu }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .chatPreviewSurface()
        .animation(.snappy(duration: 0.25), value: right)
    }

    private var menu: some View {
        Image(systemName: "ellipsis")
            .font(.system(size: 18, weight: .bold))
            .foregroundStyle(SN.textSecondary)
            .frame(width: 44, height: 44)
            .glassEffect(.regular, in: .circle)
    }

    private var tray: some View {
        let right = store.string("chat_input.inbox_button_side", "left") == "right"
        return Image(systemName: "tray")
            .font(.system(size: 17, weight: .semibold))
            .foregroundStyle(SN.textSecondary)
            .frame(width: 44, height: 44)
            .glassEffect(.regular, in: .circle)
            .overlay(alignment: right ? .topLeading : .topTrailing) { InboxCount(count: 2).offset(x: right ? -4 : 4, y: -4) }
    }
}

// MARK: The surface

extension View {
    /// A preview's place in its section: a strip of chat background whose edges line up with the
    /// rows' symbols and controls (the card's 16 point padding), rounded close to concentric with
    /// the card, with no separator under it and nothing to tap.
    func chatPreviewSurface() -> some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        return background(SN.background, in: shape)
            .clipShape(shape)
            .allowsHitTesting(false)
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Preview")
            .listRowInsets(EdgeInsets(top: 12, leading: 16, bottom: 12, trailing: 16))
            .listRowSeparator(.hidden)
    }
}

// MARK: Sample rows

/// The sample chatters and rows, the desktop preview's cast.
@MainActor
struct ChatPreviewSamples {
    enum Chatter {
        case pixelfox, mossy, lootgoblin, quietstorm

        var name: String { "\(self)" }

        var color: String {
            switch self {
            case .pixelfox: "#1E90FF"
            case .mossy: "#2E8B57"
            case .lootgoblin: "#DAA520"
            case .quietstorm: "#DA70D6"
            }
        }
    }

    private let h24: Bool
    private let platformBadges: Bool
    private let addOnBadges: Bool
    private let hiddenProviders: Set<String>

    init(_ store: SettingsStore) {
        h24 = store.string("chat_design.timestamp_format", "12h") == "24h"
        platformBadges = store.bool("chat_design.show_badges", true)
        addOnBadges = store.bool("chat_design.show_third_party_badges", true)
        hiddenProviders = Set(store.strings("chat_design.hidden_badge_providers").map { $0.lowercased() })
    }

    func row(_ id: String, _ who: Chatter, _ text: String, minutesAgo: Int, history: Bool = false, live: Bool = false,
             badges: [ChatViewBadge] = [], paint: ChatPaint? = nil, moderation: ChatModeration? = nil) -> ChatRowModel {
        row(id, who, [.text(text)], minutesAgo: minutesAgo, history: history, live: live, badges: badges, paint: paint,
            moderation: moderation)
    }

    func row(_ id: String, _ who: Chatter, _ spans: [ChatSpan], minutesAgo: Int, history: Bool = false, live: Bool = false,
             badges: [ChatViewBadge] = [], paint: ChatPaint? = nil, moderation: ChatModeration? = nil,
             edit: (inout ChatMetadata) -> Void = { _ in }) -> ChatRowModel {
        var meta = ChatMetadata()
        meta.shownName = who.name
        meta.formattedTimestamp = stamp(minutesAgo, seconds: false)
        meta.formattedTimestampWithSeconds = stamp(minutesAgo, seconds: true)
        meta.rulesEvaluated = true
        meta.fromBackfill = history
        meta.view = ChatRowView(nameColor: who.color, badges: shown(badges), paint: paint, spans: spans)
        edit(&meta)
        let message = ChatMessage(id: "preview-\(id)", username: who.name, displayName: who.name, color: who.color,
                                  content: spans.map(\.text).joined(), channel: "preview", metadata: meta)
        return ChatRowModel(message: message, moderation: moderation, live: live)
    }

    /// mossy gifting quietstorm a sub, as the core sends a USERNOTICE.
    func giftRow() -> ChatRowModel {
        var meta = ChatMetadata()
        meta.shownName = Chatter.mossy.name
        meta.msgType = "subgift"
        meta.systemMessage = "mossy gifted a subscription to quietstorm!"
        meta.formattedTimestamp = stamp(2, seconds: false)
        meta.formattedTimestampWithSeconds = stamp(2, seconds: true)
        // The event key the core stamps on a row, which is what hiding a kind matches.
        var view = ChatRowView(nameColor: Chatter.mossy.color)
        view.event = "twitch:gift"
        meta.view = view
        let message = ChatMessage(id: "preview-events-2", username: "mossy", displayName: "mossy", color: Chatter.mossy.color,
                                  content: "", channel: "preview",
                                  tags: ["msg-id": "subgift", "msg-param-recipient-display-name": "quietstorm"], metadata: meta)
        return ChatRowModel(message: message, live: false)
    }

    /// One fixed evening, 7:42:30 PM, so the sample clock never jumps while settings change.
    private func stamp(_ minutesAgo: Int, seconds: Bool) -> String {
        let total = 19 * 60 + 42 - minutesAgo
        let hour = total / 60
        let minute = String(format: "%02d", total % 60)
        let tail = seconds ? ":30" : ""
        if h24 { return "\(hour):\(minute)\(tail)" }
        return "\(hour % 12 == 0 ? 12 : hour % 12):\(minute)\(tail) \(hour < 12 ? "AM" : "PM")"
    }

    /// The badges the core would send for these settings: platform badges with Badges, the rest
    /// with Add-on badges unless their service is turned off.
    private func shown(_ badges: [ChatViewBadge]) -> [ChatViewBadge] {
        badges.filter { $0.provider == "twitch" ? platformBadges : addOnBadges && !hiddenProviders.contains($0.provider) }
    }

    static func twitchBadge(_ id: String, _ title: String) -> ChatViewBadge {
        ChatViewBadge(url: "https://static-cdn.jtvnw.net/badges/v1/\(id)/2", title: title, provider: "twitch")
    }

    static let moderator = twitchBadge("3267646d-33f0-4b17-b3df-f923a41db1d0", "Moderator")
    static let subscriber = twitchBadge("5d9f2208-5dd8-11e7-8513-2ff4adfae661", "1-Year Subscriber")
    static let house = ChatViewBadge(url: "streamnook-house:streamnook-subscriber@chat", title: "StreamNook Subscriber",
                                     provider: "streamnook")
    static let sevenTV = ChatViewBadge(url: "https://cdn.7tv.app/badge/01GXP5DNHR000CV9HPMT8GM9JZ/2x.webp",
                                       title: "7TV Subscriber", provider: "7tv")

    /// A soft pink to blue paint with two shadows, so All, One and None each look different.
    static let paint: ChatPaint = {
        let stops = [ChatPaint.Stop(at: 0, color: "#ff9ad5ff"), ChatPaint.Stop(at: 0.5, color: "#c4a1ffff"),
                     ChatPaint.Stop(at: 1, color: "#7cc4ffff")]
        return ChatPaint(id: "preview-paint", kind: "linear", angle: 90, stops: stops,
                         shadows: [ChatPaint.Shadow(x: 0, y: 0, blur: 0.5, color: "#000000ff"),
                                   ChatPaint.Shadow(x: 1, y: 1, blur: 0, color: "#4c1d95cc")],
                         name: "Preview", layers: [ChatPaint.Layer(kind: "linear", angle: 90, stops: stops)])
    }()

    /// A link's short label, as the core prints it with Short links on: the host without www.,
    /// then the path, query and fragment, cut to 40 characters.
    static func linkLabel(_ raw: String) -> String {
        let max = 40
        guard let parts = URLComponents(string: raw.hasPrefix("http") ? raw : "https://\(raw)"), let host = parts.host else {
            return raw.count > max ? String(raw.prefix(max - 1)) + "\u{2026}" : raw
        }
        let bare = host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
        let path = parts.percentEncodedPath == "/" ? "" : parts.percentEncodedPath
        let rest = path + (parts.percentEncodedQuery.map { "?\($0)" } ?? "") + (parts.percentEncodedFragment.map { "#\($0)" } ?? "")
        let full = bare + rest
        if !rest.isEmpty, full.count > max {
            return bare + String(rest.prefix(Swift.max(0, max - bare.count - 1))) + "\u{2026}"
        }
        let trimmed = full.hasSuffix("/") ? String(full.dropLast()) : full
        return trimmed.isEmpty ? bare : trimmed
    }

    /// A core type that has no public initializer, made from the JSON the core would send.
    nonisolated static func decode<T: Decodable>(_ type: T.Type, _ object: [String: Any]) -> T? {
        guard let data = try? JSONSerialization.data(withJSONObject: object) else { return nil }
        return try? JSONDecoder().decode(type, from: data)
    }
}

// MARK: The rows, drawn by chat's own cells

/// Sample rows laid out by `ChatRowLayoutBuilder` and drawn by `ChatRowCell`, exactly as the chat
/// list draws them, at the width SwiftUI offers.
struct ChatPreviewRows: UIViewRepresentable {
    let rows: [ChatRowModel]
    let style: ChatStyle
    /// The arrival the newest row plays again when it changes (Message animation).
    var arrival: String?
    /// Rows that flash play it again when this changes (a mention color or Flash).
    var flash: String?

    func makeUIView(context: Context) -> ChatPreviewRowsView {
        ChatPreviewRowsView()
    }

    func updateUIView(_ view: ChatPreviewRowsView, context: Context) {
        view.update(rows: rows, style: style, arrival: arrival, flash: flash)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: ChatPreviewRowsView, context: Context) -> CGSize? {
        guard let width = proposal.width, width > 0, width.isFinite else { return nil }
        return CGSize(width: width, height: uiView.height(for: width))
    }
}

@MainActor
final class ChatPreviewRowsView: UIView {
    /// The rows' own box. It clips, so the first row's top divider (there to part it from a row
    /// above that the preview does not have) is cut off, as desktop's preview leaves it out.
    private let rowsView = UIView()
    private var cells: [ChatRowCell] = []
    private var rows: [ChatRowModel] = []
    private var style = ChatStyle()
    private var built: (width: CGFloat, scale: CGFloat, layouts: [ChatRowLayout])?
    private var arrival: String??
    private var flashKey: String??
    private var replayArrival: String?
    private var replayFlash = false
    /// Space above the first row and under the last, as the chat column's own.
    private let inset: CGFloat = 4

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        isUserInteractionEnabled = false
        rowsView.clipsToBounds = true
        addSubview(rowsView)
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(redo), name: ChatTheme.changed, object: nil)
        center.addObserver(self, selector: #selector(redo), name: ImagePipeline.failedNotification, object: nil)
        center.addObserver(self, selector: #selector(redo), name: ImagePipeline.recoveredNotification, object: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    /// A new theme, or an image that failed to load or came back: the rows are laid out again (a
    /// failed badge gives its slot back, a failed emote reads as its name). Image changes post
    /// from any thread.
    @objc nonisolated private func redo() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            built = nil
            invalidateIntrinsicContentSize()
            setNeedsLayout()
        }
    }

    func update(rows: [ChatRowModel], style: ChatStyle, arrival: String?, flash: String?) {
        if rows != self.rows || style != self.style {
            self.rows = rows
            self.style = style
            built = nil
            invalidateIntrinsicContentSize()
            setNeedsLayout()
        }
        // The first value only records; a later change plays the look again.
        if let seen = self.arrival, seen != arrival { replayArrival = arrival; setNeedsLayout() }
        if let seen = flashKey, seen != flash { replayFlash = true; setNeedsLayout() }
        self.arrival = .some(arrival)
        flashKey = .some(flash)
    }

    private var scale: CGFloat {
        let scale = window?.screen.scale ?? traitCollection.displayScale
        return scale > 0 ? scale : 3
    }

    private func layouts(for width: CGFloat) -> [ChatRowLayout] {
        let scale = scale
        if let built, built.width == width, built.scale == scale { return built.layouts }
        let builder = ChatRowLayoutBuilder(style: style, width: width, scale: scale)
        let layouts = rows.map { builder.build($0) }
        built = (width, scale, layouts)
        return layouts
    }

    /// How much of the first shown row's top is cut: its divider, when it has one.
    private func cut(_ layouts: [ChatRowLayout]) -> CGFloat {
        layouts.first { $0.height > 0 }?.decor.topHairline == nil ? 0 : 1 / scale
    }

    func height(for width: CGFloat) -> CGFloat {
        let layouts = layouts(for: width)
        return ceil(layouts.reduce(0) { $0 + $1.height } - cut(layouts) + 2 * inset)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let width = bounds.width
        guard width > 0 else { return }
        let layouts = layouts(for: width)
        let cut = cut(layouts)
        rowsView.frame = CGRect(x: 0, y: inset, width: width, height: max(0, layouts.reduce(0) { $0 + $1.height } - cut))
        while cells.count < layouts.count {
            let cell = ChatRowCell()
            rowsView.addSubview(cell)
            cells.append(cell)
        }
        var y = -cut
        for (i, cell) in cells.enumerated() {
            guard i < layouts.count, layouts[i].height > 0 else {
                cell.isHidden = true
                continue
            }
            let layout = layouts[i]
            cell.isHidden = false
            cell.frame = CGRect(x: 0, y: y, width: width, height: layout.height)
            cell.configure(layout, scale: scale, animateImages: style.animatesEmotes)
            y += layout.height
        }
        playReplays(layouts)
    }

    private func playReplays(_ layouts: [ChatRowLayout]) {
        if let entrance = replayArrival {
            replayArrival = nil
            // As the list: no arrival under Reduce Motion or with motion turned down.
            if entrance != "none", !UIAccessibility.isReduceMotionEnabled, Look.shared.motion == .full,
               let newest = layouts.lastIndex(where: { $0.height > 0 }) {
                cells[newest].arrive(entrance)
            }
        }
        if replayFlash {
            replayFlash = false
            for (i, layout) in layouts.enumerated() where layout.height > 0 && layout.decor.flash != nil {
                cells[i].flash()
            }
        }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        // Off screen the images leave the shared clock; back on, they rejoin it.
        if window == nil {
            cells.forEach { $0.recycle() }
        } else {
            cells.forEach { $0.resume() }
        }
    }
}
