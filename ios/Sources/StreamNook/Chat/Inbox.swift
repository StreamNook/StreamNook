import SNKit
import SwiftUI
import UIKit

// The inbox: one timeline, newest first, of every mention, reply and highlight (and whispers once
// their store joins the core's view), with chips across the top to narrow it. One panel, two ways
// in: the sheet that grows up out of the message bar's tray, and You > Mentions. The core builds
// the view (`get_inbox_view`): it filters by chip, folds identical messages in a row into one and
// attaches the senders' pictures it has. This draws it, relays taps, and marks what was on screen
// read when it closes. The same flow and words as Android's (src/mobile/mentions).

extension EnvironmentValues {
    /// Which chat a composer belongs to, for the inbox: the chat under the player or the Chats tab.
    @Entry var inboxSurface: InboxSurface = .watch
}

enum InboxMotion {
    /// The desktop Dynamic Island's spring: the sheet, the card, the tray leaving while you type.
    static let spring = Animation.interpolatingSpring(mass: 0.9, stiffness: 360, damping: 32)
    /// How long a new mention's card stays before it folds into the tray's count.
    static let arrivalSeconds = 4.0
}

/// The trays on screen, in window points, so a tap on one never reads as outside the sheet it
/// toggles (the tray closes it itself).
@MainActor
enum InboxTrayFrames {
    static var frames: [String: CGRect] = [:]
}

// MARK: The tray

/// The inbox's way in at one end of the message bar (Settings > Chat > Inbox button picks which),
/// the same glass circle as the chat menu at the other end: a small count on its rim while
/// something is new, and the X that closes the sheet while it is open.
struct InboxTray: View {
    let store: MentionsStore
    let surface: InboxSurface
    /// The landscape strip's variant: white ink on the strip's own glass.
    var plain = false
    /// The bar's end it sits at: the count rides the corner beside the field.
    var right = false
    @State private var key = UUID().uuidString

    var body: some View {
        let open = store.sheetIn == surface
        let unread = store.unreadTotal
        Button {
            withAnimation(SNMotion.run(InboxMotion.spring)) { store.toggleSheet(surface) }
        } label: {
            ZStack {
                Image(systemName: "tray").opacity(open ? 0 : 1)
                // On the landscape strip, beside its own hide-chat X, open reads as put away.
                Image(systemName: plain ? "chevron.down" : "xmark").opacity(open ? 1 : 0)
            }
            .font(.system(size: plain ? 15 : 17, weight: .semibold))
            .foregroundStyle(plain ? Color.white : (open ? SN.textPrimary : SN.textSecondary))
            .animation(.easeOut(duration: 0.15), value: open)
            .frame(width: plain ? 36 : 44, height: plain ? 36 : 44)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .modifier(TrayGlass(plain: plain))
        .overlay(alignment: right ? .topLeading : .topTrailing) {
            if !open && unread > 0 {
                InboxCount(count: unread, ring: plain ? Color.black.opacity(0.55) : SN.background)
                    .offset(x: right ? -4 : 4, y: -4)
                    .allowsHitTesting(false)
            }
        }
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { InboxTrayFrames.frames[key] = $0 }
        .onDisappear { InboxTrayFrames.frames[key] = nil }
        .accessibilityLabel(open ? "Close the inbox" : unread > 0 ? "Inbox, \(unread) new" : "Inbox")
    }
}

private struct TrayGlass: ViewModifier {
    let plain: Bool

    func body(content: Content) -> some View {
        if plain {
            content
        } else {
            content.glassEffect(.regular.interactive(), in: .circle)
        }
    }
}

/// The tray's count: the accent, ringed with what it sits on.
struct InboxCount: View {
    let count: Int
    var ring: Color = SN.background

    var body: some View {
        Text(count > 99 ? "99+" : "\(count)")
            .font(.system(size: 10.5, weight: .bold).monospacedDigit())
            .foregroundStyle(Look.shared.palette.accent.luminance > 0.5 ? Color.black.opacity(0.88) : .white)
            .padding(.horizontal, 5)
            .frame(minWidth: 17, minHeight: 17)
            .background(Capsule().fill(SN.accent))
            .overlay(Capsule().stroke(ring, lineWidth: 2))
    }
}

// MARK: Faces and words

/// The sender's picture (the core's cached lookup), or their initial on their own color while it
/// loads or when there is none, with the kind's mark on its corner.
struct InboxFace: View {
    let entry: MentionEntry
    let avatar: String?
    let size: CGFloat
    var dim = false

    var body: some View {
        let color = MentionColors.name(entry.senderColor)
        ZStack {
            Circle().fill(LinearGradient(colors: [color, color.mix(with: .black, by: 0.45)],
                                         startPoint: .topLeading, endPoint: .bottomTrailing))
            Text(String(entry.shownSender.prefix(1)).uppercased())
                .font(.system(size: size * 0.42, weight: .semibold))
                .foregroundStyle(.white)
            if let avatar, !avatar.isEmpty {
                ChatImageView(url: avatar, size: size)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .opacity(dim ? 0.6 : 1)
        .overlay(alignment: .bottomTrailing) {
            let chip = (size * 0.5).rounded()
            Image(systemName: Self.symbol(entry.kind))
                .font(.system(size: chip * 0.55, weight: .bold))
                .foregroundStyle(SN.accent)
                .frame(width: chip, height: chip)
                .background(Circle().fill(SN.backgroundTertiary))
                .overlay(Circle().stroke(SN.backgroundSecondary, lineWidth: 1.5))
                .offset(x: 4, y: 4)
        }
        .accessibilityHidden(true)
    }

    static func symbol(_ kind: MentionEntry.Kind) -> String {
        switch kind {
        case .mention: "at"
        case .reply: "arrowshape.turn.up.left.fill"
        case .word: "highlighter"
        }
    }
}

/// The message as the chat drew it: emotes inline at the text's height (the shared image
/// pipeline), the name standing in until one has loaded; the plain text for an entry filed
/// before the core kept its parts.
struct InboxMessageText: View {
    let entry: MentionEntry
    var emote: CGFloat = 21
    var lines = 2
    var color: Color = SN.textPrimary
    @Environment(\.displayScale) private var scale
    @State private var images: [String: CGImage] = [:]

    var body: some View {
        text
            .lineLimit(lines)
            .multilineTextAlignment(.leading)
            .task(id: entry.id) { await load() }
    }

    private var text: Text {
        if entry.removed {
            return Text("Message deleted by a moderator").italic().foregroundStyle(SN.textMuted)
        }
        guard !entry.parts.isEmpty else { return Text(entry.text).foregroundStyle(color) }
        var out = Text("")
        for part in entry.parts {
            switch part {
            case .text(let words):
                out = out + Text(words).foregroundStyle(color)
            case .emote(let name, let url):
                if let image = images[url] {
                    out = out + Text(Image(decorative: image, scale: scale)).baselineOffset(-emote * 0.22)
                } else {
                    out = out + Text(name).foregroundStyle(color)
                }
            }
        }
        return out
    }

    private func load() async {
        let pixels = Int(emote * scale)
        for case .emote(_, let url) in entry.parts where images[url] == nil {
            if let hit = ImagePipeline.shared.cached(url, pixelHeight: pixels, animated: false)?.first {
                images[url] = hit
                continue
            }
            let decoded: DecodedImage? = await withCheckedContinuation { continuation in
                ImagePipeline.shared.load(url, pixelHeight: pixels, animated: false) { continuation.resume(returning: $0) }
            }
            if let first = decoded?.first { images[url] = first }
        }
    }
}

// MARK: The panel

/// The inbox's content, for the sheet and for You > Mentions.
struct InboxPanel: View {
    enum Variant { case sheet, screen }

    let store: MentionsStore
    let variant: Variant
    /// Called before a row opens its message (a sheet around the panel closes first).
    var onOpenRow: (() -> Void)?
    /// The sheet's slide down, reported from its header band.
    var onDrag: ((CGFloat) -> Void)?
    var onDragEnd: ((CGFloat, CGFloat) -> Void)?

    @State private var filter = "all"
    @State private var view: InboxView?
    @State private var failed = false
    @State private var loadingMore = false
    /// Every message that was on screen, read as the inbox closes: the dots stay while you look,
    /// and the tray's count clears as you leave.
    @State private var seen: Set<String> = []
    @State private var expanded: Set<String> = []

    var body: some View {
        VStack(spacing: 0) {
            if variant == .sheet { header }
            chips
            list
        }
        .task(id: "\(filter)|\(store.revision)") { await load() }
        .onDisappear { store.markSeen(Array(seen)) }
        .toolbar {
            if variant == .screen, (view?.unreadTotal ?? 0) > 0 {
                ToolbarItem(placement: .topBarTrailing) {
                    Button { Task { await store.markRead(nil) } } label: {
                        Image(systemName: "checkmark.circle")
                    }
                    .accessibilityLabel("Mark all read")
                }
            }
        }
    }

    private var unread: Int { view?.unreadTotal ?? store.unreadTotal }

    /// The grabber, then the title with its count and the quiet mark-all-read. A slide down here
    /// closes the sheet.
    private var header: some View {
        VStack(spacing: 0) {
            Capsule().fill(SN.textMuted.opacity(0.5)).frame(width: 36, height: 4).padding(.top, 6)
            HStack(spacing: 0) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text("Inbox").font(.sn(16, 700)).foregroundStyle(SN.textPrimary)
                    if unread > 0 {
                        Text("\(unread) new").font(.sn(12.5, 500)).monospacedDigit().foregroundStyle(SN.textMuted)
                    }
                }
                Spacer(minLength: 8)
                if unread > 0 {
                    Button { Task { await store.markRead(nil) } } label: {
                        Image(systemName: "checkmark.circle")
                            .font(.system(size: 17, weight: .regular))
                            .foregroundStyle(SN.textMuted)
                            .frame(width: 36, height: 36)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Mark all read")
                }
            }
            .padding(.leading, 12)
            .padding(.trailing, 3)
            .frame(height: 40)
        }
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 6, coordinateSpace: .global)
                .onChanged { value in onDrag?(max(0, value.translation.height)) }
                .onEnded { value in
                    onDragEnd?(max(0, value.translation.height),
                               value.predictedEndTranslation.height - value.translation.height)
                }
        )
    }

    private var chips: some View {
        let list = view?.chips ?? ["all", "mentions", "highlights"].map { InboxChip(id: $0, unread: 0) }
        return HStack(spacing: 6) {
            ForEach(list) { chip in
                let on = chip.id == filter
                Button {
                    guard !on else { return }
                    filter = chip.id
                } label: {
                    // The bold label laid out invisibly underneath, so a chip keeps one width
                    // chosen or not and the row never shifts.
                    ZStack {
                        Text(InboxText.chip(chip.id)).font(.sn(11.5, 600)).hidden()
                        Text(InboxText.chip(chip.id)).font(.sn(11.5, on ? 600 : 400))
                    }
                        .foregroundStyle(on ? SN.textPrimary : SN.textSecondary)
                        .padding(.horizontal, 10)
                        .frame(height: 26)
                        .background(Capsule().fill(on ? SN.textPrimary.opacity(0.11) : .clear))
                        .overlay(Capsule().strokeBorder(on ? SN.textPrimary.opacity(0.18) : SN.borderSubtle, lineWidth: 1))
                        .overlay(alignment: .topTrailing) {
                            if chip.unread > 0, !on, chip.id != "all" {
                                Circle().fill(SN.accent).frame(width: 6, height: 6)
                                    .overlay(Circle().stroke(SN.backgroundTertiary.opacity(0.9), lineWidth: 2))
                                    .offset(x: -2)
                            }
                        }
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(on ? .isSelected : [])
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.top, variant == .screen ? 8 : 0)
        .padding(.bottom, 8)
    }

    @ViewBuilder
    private var list: some View {
        if let view, view.rows.isEmpty {
            let words = InboxText.empty(filter)
            VStack {
                EmptyNote(text: failed ? "The inbox did not load." : words.text,
                          hint: failed ? "Try again in a moment." : words.hint)
                    .padding(.top, 28)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity)
        } else if view == nil && failed {
            EmptyNote(text: "The inbox did not load.", hint: "Try again in a moment.")
                .padding(.top, 28)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        } else {
            let rows = view?.rows ?? []
            let mixed = view?.mixedPlatforms ?? false
            let muted = Set((store.settings?.strings("mentions.muted_channels") ?? []).map { $0.lowercased() })
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(rows) { row in
                        rowView(row, mixed: mixed, muted: muted.contains(row.entry.room))
                            .onAppear {
                                seen.formUnion(row.ids)
                                if row.id == rows.last?.id { Task { await loadMore() } }
                            }
                    }
                    if loadingMore {
                        ProgressView().padding(.vertical, 12)
                    }
                }
                .padding(.bottom, 16)
            }
            .scrollIndicators(.hidden)
            .mask {
                VStack(spacing: 0) {
                    Color.black
                    LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: 20)
                }
            }
        }
    }

    private func rowView(_ row: InboxRowView, mixed: Bool, muted: Bool) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                onOpenRow?()
                store.open(row: row)
            } label: {
                InboxRowLine(row: row, mixed: mixed)
            }
            .buttonStyle(RowPress())
            .contextMenu {
                if let settings = store.settings {
                    Button(muted ? "Unmute this chat" : "Mute this chat", systemImage: muted ? "bell" : "bell.slash") {
                        Task {
                            if muted {
                                await store.unmute(row.entry.room, store: settings)
                            } else {
                                await store.mute(row.entry.room, store: settings)
                            }
                        }
                    }
                }
                Button("Remove", systemImage: "trash", role: .destructive) {
                    self.view?.rows.removeAll { $0.id == row.id }
                    Task { for id in row.ids { await store.remove(id) } }
                }
            }
            if !row.folded.isEmpty { folded(row) }
        }
    }

    /// The fold's line under its row, and the folded messages once opened.
    private func folded(_ row: InboxRowView) -> some View {
        let open = expanded.contains(row.id)
        return VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.snappy(duration: 0.2)) {
                    if open { expanded.remove(row.id) } else { expanded.insert(row.id) }
                }
            } label: {
                HStack(spacing: 4) {
                    Text(InboxText.folded(row.entry, count: row.folded.count + 1))
                    Image(systemName: "chevron.down")
                        .font(.system(size: 10, weight: .bold))
                        .rotationEffect(.degrees(open ? 180 : 0))
                }
                .font(.sn(12, 500))
                .foregroundStyle(SN.textMuted)
                .frame(minHeight: 28)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if open {
                ForEach(row.folded) { one in
                    Button {
                        onOpenRow?()
                        store.sheetIn = nil
                        store.onOpen?(one.id)
                    } label: {
                        HStack(spacing: 8) {
                            Circle().fill(one.read ? SN.border : SN.accent).frame(width: 6, height: 6)
                            Text(MentionText.time(one.atMs, nowMs: Int64(Date().timeIntervalSince1970 * 1000)))
                                .monospacedDigit()
                                .foregroundStyle(SN.textMuted)
                                .frame(width: 48, alignment: .leading)
                            Text("The same message").foregroundStyle(SN.textSecondary)
                            Spacer(minLength: 0)
                        }
                        .font(.sn(12.5))
                        .frame(minHeight: 30)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(.leading, 54)
        .padding(.trailing, 12)
        .padding(.top, -6)
        .padding(.bottom, 8)
    }

    private func load() async {
        do {
            let limit = max(MentionList.pageSize, min(200, view?.rows.count ?? 0))
            let fresh = try await store.view(filter, limit: limit)
            view = fresh
            failed = false
        } catch {
            failed = true
            log.error("get_inbox_view failed: \(String(describing: error), privacy: .public)")
        }
    }

    private func loadMore() async {
        guard let current = view, current.hasMore, let before = current.beforeMs, !loadingMore else { return }
        loadingMore = true
        defer { loadingMore = false }
        let at = filter
        guard let page = try? await store.view(at, before: before), at == filter else { return }
        view?.append(page)
    }
}

/// One row: the face, the sender in their color and where they said it, the time and the unread
/// dot, then the message. Read rows keep their names and times at full strength; the message goes
/// a step quieter and the face dims, and the dot carries the rest.
struct InboxRowLine: View {
    let row: InboxRowView
    var mixed = false

    var body: some View {
        let entry = row.entry
        HStack(alignment: .top, spacing: 10) {
            InboxFace(entry: entry, avatar: row.avatar, size: 32, dim: !row.unread)
                .padding(.top, 4)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text(entry.shownSender)
                        .font(.sn(14, 700))
                        .foregroundStyle(MentionColors.name(entry.senderColor))
                        .lineLimit(1)
                        .layoutPriority(1)
                    Text("in").font(.sn(12.5)).foregroundStyle(SN.textMuted)
                    HStack(spacing: 3) {
                        if mixed { ProviderLogo(provider: entry.platform, size: 11, tinted: true) }
                        Text(InboxText.place(entry)).font(.sn(12.5)).foregroundStyle(SN.textSecondary).lineLimit(1)
                    }
                    Spacer(minLength: 6)
                    TimelineView(.everyMinute) { context in
                        Text(MentionText.time(entry.atMs, nowMs: Int64(context.date.timeIntervalSince1970 * 1000)))
                            .font(.sn(11.5))
                            .monospacedDigit()
                            .foregroundStyle(SN.textMuted)
                    }
                    Circle()
                        .fill(row.unread ? SN.accent : .clear)
                        .frame(width: 8, height: 8)
                        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 0.5 }
                        .padding(.leading, 2)
                }
                InboxMessageText(entry: entry, emote: 21, lines: 2, color: row.unread ? SN.textPrimary : SN.textSecondary)
                    .font(.sn(14))
                    .lineSpacing(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(row.unread ? "Unread. " : "")\(entry.shownSender) \(entry.kindPhrase) in \(InboxText.place(entry)): \(entry.shownText)")
        .accessibilityHint("Opens the chat at this message")
    }
}

// MARK: The sheet over the chat

/// The inbox sheet: it grows up out of the message bar from the tray's corner, flush on the bar,
/// over the lower two thirds of the chat; the chat above stays live. It closes the way it came on a
/// slide down, a tap anywhere outside it, the tray (its X), or focusing the message field.
struct InboxSheetOverlay: View {
    let store: MentionsStore
    let surface: InboxSurface
    /// The tray's end of the bar, where the sheet grows from.
    let leading: Bool
    @State private var drag: CGFloat = 0
    @State private var frame: CGRect = .zero

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .bottom) {
                if store.sheetIn == surface {
                    let shape = UnevenRoundedRectangle(topLeadingRadius: 22, topTrailingRadius: 22, style: .continuous)
                    InboxPanel(store: store, variant: .sheet,
                               onDrag: { drag = $0 },
                               onDragEnd: { distance, fling in
                                   if distance > 90 || fling > 160 {
                                       close()
                                   } else {
                                       withAnimation(.snappy(duration: 0.22)) { drag = 0 }
                                   }
                               })
                        .frame(height: max(220, min(geo.size.height, geo.size.height * 0.68)))
                        .snGlass(shape, light: .panel)
                        .clipShape(shape)
                        .shadow(color: .black.opacity(0.35), radius: 12, y: -6)
                        .padding(.horizontal, 10)
                        .offset(y: drag)
                        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame = $0 }
                        .background(OutsideTap(inside: { [frame] + Array(InboxTrayFrames.frames.values) }, onOutside: close))
                        .transition(.scale(scale: 0.12, anchor: leading ? .bottomLeading : .bottomTrailing)
                            .combined(with: .opacity))
                }
            }
            .frame(width: geo.size.width, height: geo.size.height, alignment: .bottom)
        }
        .animation(SNMotion.run(InboxMotion.spring), value: store.sheetIn == surface)
    }

    private func close() {
        withAnimation(SNMotion.run(InboxMotion.spring)) {
            store.sheetIn = nil
        } completion: {
            drag = 0
        }
    }
}

/// A new mention, reply or highlight, as a compact card risen out of the tray just above the
/// message bar: who, where, and a line of what they said. It stays a few seconds and folds back
/// into the tray's count. A tap jumps to the message (the field keeps the keyboard and the
/// draft); a slide down puts it away early. It waits while the composer's own risers are up.
struct InboxArrivalCard: View {
    let store: MentionsStore
    let leading: Bool
    @State private var drag: CGFloat = 0

    var body: some View {
        ZStack(alignment: .bottom) {
            if let card = store.arrival, !store.composerBusy, store.sheetIn == nil {
                content(card)
                    .id(card.seq)
                    .transition(.scale(scale: 0.2, anchor: leading ? .bottomLeading : .bottomTrailing).combined(with: .opacity))
                    .task(id: card.seq) {
                        try? await Task.sleep(for: .seconds(InboxMotion.arrivalSeconds))
                        withAnimation(SNMotion.run(InboxMotion.spring)) { store.dismissArrival(card.seq) }
                    }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
        .animation(SNMotion.run(InboxMotion.spring), value: store.arrival?.seq)
        .animation(SNMotion.run(InboxMotion.spring), value: store.composerBusy)
    }

    private func content(_ card: MentionsStore.ArrivalCard) -> some View {
        let entry = card.arrival.entry
        return Button {
            withAnimation(SNMotion.run(InboxMotion.spring)) { store.dismissArrival(card.seq) }
            store.onOpen?(entry.id)
        } label: {
            HStack(spacing: 10) {
                InboxFace(entry: entry, avatar: card.arrival.avatar, size: 36)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(alignment: .firstTextBaseline, spacing: 5) {
                        Text(entry.shownSender)
                            .font(.sn(14, 700))
                            .foregroundStyle(MentionColors.name(entry.senderColor))
                            .lineLimit(1)
                        Text("in \(InboxText.place(entry))").font(.sn(12.5)).foregroundStyle(SN.textMuted).lineLimit(1)
                        Spacer(minLength: 6)
                        Text("now").font(.sn(11.5)).foregroundStyle(SN.textMuted)
                    }
                    InboxMessageText(entry: entry, emote: 19, lines: 1, color: SN.textSecondary)
                        .font(.sn(13.5))
                        .fixedSize(horizontal: true, vertical: false)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .clipped()
                        .mask {
                            HStack(spacing: 0) {
                                Color.black
                                LinearGradient(colors: [.black, .clear], startPoint: .leading, endPoint: .trailing).frame(width: 28)
                            }
                        }
                }
            }
            .padding(.leading, 8)
            .padding(.trailing, 12)
            .frame(height: 56)
            .snGlass(RoundedRectangle(cornerRadius: 22, style: .continuous), interactive: true, light: .panel)
            .shadow(color: .black.opacity(0.3), radius: 10, y: -4)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 10)
        .padding(.bottom, 8)
        .offset(y: drag)
        .simultaneousGesture(
            DragGesture(minimumDistance: 8)
                .onChanged { drag = max(0, $0.translation.height) }
                .onEnded { value in
                    if value.translation.height > 24 {
                        withAnimation(SNMotion.run(InboxMotion.spring)) { store.dismissArrival(card.seq) }
                    }
                    withAnimation(.snappy(duration: 0.2)) { drag = 0 }
                }
        )
        .accessibilityLabel("\(entry.shownSender) \(entry.kindPhrase) in \(InboxText.place(entry)): \(entry.shownText)")
        .accessibilityHint("Opens the chat at this message")
    }
}

/// The chat list's mask while an inbox overlay is up: rows fade out over 24 points above it.
struct InboxClearMask: View {
    let sheet: Bool
    let card: Bool

    var body: some View {
        GeometryReader { geo in
            let clear = sheet ? max(220, geo.size.height * 0.68) : card ? 64 : 0
            VStack(spacing: 0) {
                Color.black
                if clear > 0 {
                    LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: 24)
                    Color.clear.frame(height: min(clear, geo.size.height))
                }
            }
        }
        .animation(SNMotion.run(InboxMotion.spring), value: sheet)
        .animation(SNMotion.run(InboxMotion.spring), value: card)
    }
}

// MARK: A tap outside

/// Hears every tap in the window, without taking any, and reports one that lands outside the
/// given frames. Until the shared tap-outside mechanism lands this is the inbox's own; switch to
/// that one then.
struct OutsideTap: UIViewRepresentable {
    let inside: () -> [CGRect]
    let onOutside: () -> Void

    func makeUIView(context: Context) -> Catcher {
        let view = Catcher()
        view.isUserInteractionEnabled = false
        return view
    }

    func updateUIView(_ view: Catcher, context: Context) {
        view.inside = inside
        view.onOutside = onOutside
    }

    static func dismantleUIView(_ view: Catcher, coordinator: ()) {
        view.detach()
    }

    final class Catcher: UIView, UIGestureRecognizerDelegate {
        var inside: () -> [CGRect] = { [] }
        var onOutside: () -> Void = {}
        private var tap: UITapGestureRecognizer?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            detach()
            guard let window else { return }
            let tap = UITapGestureRecognizer(target: self, action: #selector(tapped(_:)))
            tap.cancelsTouchesInView = false
            tap.delaysTouchesBegan = false
            tap.delaysTouchesEnded = false
            tap.delegate = self
            window.addGestureRecognizer(tap)
            self.tap = tap
        }

        func detach() {
            if let tap { tap.view?.removeGestureRecognizer(tap) }
            tap = nil
        }

        @objc private func tapped(_ recognizer: UITapGestureRecognizer) {
            guard let window else { return }
            let point = recognizer.location(in: window)
            guard !inside().contains(where: { $0.contains(point) }) else { return }
            onOutside()
        }

        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }
    }
}
