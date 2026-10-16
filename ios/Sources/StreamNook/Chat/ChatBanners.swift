import SNKit
import SwiftUI

/// The surface of a banner over (or under) moving chat: the popover tint at 90 percent, solid
/// at 0 percent glassiness, with no blur, since a blur over repainting chat costs every frame.
/// Paragraphs (an unrolled pin) sit on a near-solid tint so chat never reads through them.
private struct StaticPopover: ViewModifier {
    var radius: CGFloat = 12
    var tint = 0.9

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        content.background {
            shape.fill(SN.backgroundTertiary.opacity(tint + (1 - Look.shared.glassiness) * (1 - tint)))
                .overlay { shape.stroke(SN.borderSubtle, lineWidth: 1) }
        }
    }
}

// MARK: Pinned messages

/// The room's pinned messages as one container that grows downward: a header with the sender
/// and a one-line preview, which unrolls into every pin. It always starts collapsed; tapping a
/// pin never jumps to the message and there is no unpin here.
struct PinnedBanner: View {
    let pins: [PinnedChat]
    @Binding var expanded: Bool

    var body: some View {
        if let first = pins.first {
            VStack(alignment: .leading, spacing: 0) {
                Button {
                    withAnimation(.timingCurve(0.16, 1, 0.3, 1, duration: 0.2)) { expanded.toggle() }
                } label: {
                    header(first)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Pinned: \(first.shownName): \(first.text)")
                .accessibilityHint(expanded ? "Collapses the pinned messages" : "Shows the pinned messages")
                if expanded {
                    pinsBody(first)
                        .transition(.opacity.combined(with: .move(edge: .top)))
                }
            }
            .clipped()
            .modifier(StaticPopover(tint: expanded ? 0.97 : 0.9))
        }
    }

    private func header(_ first: PinnedChat) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "pin.fill")
                .font(.system(size: 12))
                .foregroundStyle(SN.accent)
                // On the tab strip's face column above it.
                .padding(.leading, 2)
                .accessibilityHidden(true)
            // The name hugs its preview; a long one gives way once the preview is down to 120 pt.
            PinnedSender(pin: first, allBadges: expanded)
                .layoutPriority(1)
            Text(expanded ? "" : first.text)
                .font(.sn(12))
                .foregroundStyle(SN.textPrimary.opacity(0.85))
                .lineLimit(1)
                .frame(minWidth: expanded ? 0 : 120, maxWidth: .infinity, alignment: .leading)
            if pins.count > 1 {
                Text("+\(pins.count - 1)")
                    .font(.sn(11, 600))
                    .monospacedDigit()
                    .foregroundStyle(SN.textSecondary)
            }
            Image(systemName: "chevron.down")
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(SN.textSecondary)
                .rotationEffect(.degrees(expanded ? 180 : 0))
                .animation(.timingCurve(0.2, 0.8, 0.2, 1, duration: 0.18), value: expanded)
                .accessibilityHidden(true)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
    }

    private func pinsBody(_ first: PinnedChat) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(first.text)
                .font(.sn(13))
                .lineSpacing(3)
                .foregroundStyle(SN.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            if pins.count > 1 {
                SN.borderSubtle.frame(height: 1).accessibilityHidden(true)
                ForEach(pins.dropFirst()) { pin in
                    HStack(alignment: .firstTextBaseline, spacing: 0) {
                        PinnedSender(pin: pin, allBadges: true)
                        Text(": \(pin.text)")
                            .font(.sn(13))
                            .foregroundStyle(SN.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        }
        .padding(.horizontal, 10)
        .padding(.top, 2)
        .padding(.bottom, 8)
    }
}

/// A pin's sender as their chat rows draw them: their badges (only the role badge when the
/// banner is collapsed) and their name in their paint or readable color. A badge takes room only
/// once its art has loaded, so the name never jumps over an empty slot.
private struct PinnedSender: View {
    let pin: PinnedChat
    let allBadges: Bool

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(badges.enumerated()), id: \.offset) { _, badge in
                AsyncImage(url: URL(string: badge.url)) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFit()
                            .frame(width: 14, height: 14)
                            .padding(.trailing, 4)
                    }
                }
                .accessibilityHidden(true)
            }
            PaintedName(name: pin.shownName, color: pin.senderView?.nameColor ?? pin.senderColor,
                        paint: pin.senderView?.paint, size: 13)
        }
    }

    private var badges: [ChatViewBadge] {
        guard let view = pin.senderView else { return [] }
        if allBadges { return view.badges }
        return view.roleBadge.map { [$0] } ?? []
    }
}

/// A chatter's name in bold: their 7TV paint as a gradient when they wear one, else their color
/// with a dark shadow to lift it off the banner.
struct PaintedName: View {
    let name: String
    let color: String
    let paint: ChatPaint?
    var size: CGFloat = 12

    var body: some View {
        let text = Text(name).font(.sn(size, 700))
        if let paint, paint.stops.count >= 2, paint.kind == "linear" || paint.kind == "radial" {
            text
                .foregroundStyle(gradient(paint))
                .lineLimit(1)
        } else {
            text
                .foregroundStyle(Color(ChatTheme.color(color) ?? ChatTheme.fallbackName))
                .shadow(color: .black.opacity(0.8), radius: 1, y: 1)
                .lineLimit(1)
        }
    }

    private func gradient(_ paint: ChatPaint) -> LinearGradient {
        let stops = paint.stops.map { Gradient.Stop(color: Color(ChatTheme.color($0.color) ?? ChatTheme.fallbackName), location: $0.at) }
        // CSS angles: 0 points up and turns clockwise.
        let radians = (paint.angle - 90) * .pi / 180
        let dx = cos(radians) / 2
        let dy = sin(radians) / 2
        return LinearGradient(stops: stops, startPoint: UnitPoint(x: 0.5 - dx, y: 0.5 - dy),
                              endPoint: UnitPoint(x: 0.5 + dx, y: 0.5 + dy))
    }
}

// MARK: Streaks above the composer

/// A resub and a watch-streak milestone the viewer can share, directly above the composer,
/// resub first (it is rarer and expires). Sharing posts the composer's text with it, consumes
/// the token and hides the banner; dismissals hold for the session.
struct StreakBanners: View {
    let rooms: ChatRooms
    let session: ChatSession

    private static let amber = Color(red: 0xF5 / 255, green: 0x9E / 255, blue: 0x0B / 255)
    private static let amberLight = Color(red: 0xFB / 255, green: 0xBF / 255, blue: 0x24 / 255)

    var body: some View {
        let streaks = rooms.activeStreaks
        if streaks.watch != nil || streaks.resub != nil {
            VStack(spacing: 4) {
                if let resub = streaks.resub { resubBanner(resub) }
                if let watch = streaks.watch { watchBanner(watch) }
            }
            .padding(.horizontal, 10)
            .padding(.top, 6)
        }
    }

    /// What the composer holds, shared with the streak.
    private var composer: ChatComposerModel? { ComposerRegistry.shared.models[ObjectIdentifier(session)] }

    private func resubBanner(_ resub: ResubNotice) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "star.fill")
                .font(.system(size: 13))
                .foregroundStyle(SN.accent)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text(resub.title)
                    .font(.sn(12))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                if resub.streakMonths > 1 {
                    Button {
                        rooms.includeStreak.toggle()
                    } label: {
                        Text("\(rooms.includeStreak ? "✓ " : "")Include \(resub.streakMonths)-month streak")
                            .font(.sn(10.5))
                            .foregroundStyle(SN.textMuted)
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(rooms.includeStreak ? .isSelected : [])
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button {
                Task {
                    if await rooms.shareResub(message: composer?.draft ?? "") { composer?.draft = "" }
                }
            } label: {
                Text("Share")
                    .font(.sn(11.5, 600))
                    .foregroundStyle(SN.accent)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .background(Capsule().fill(SN.accent.opacity(0.2)))
            }
            .buttonStyle(.plain)
            .disabled(rooms.sharing)
            .opacity(rooms.sharing ? 0.5 : 1)
            Button {
                rooms.dismiss("resub")
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(SN.textMuted)
                    .padding(4)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Dismiss resub")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .modifier(StaticPopover())
    }

    private func watchBanner(_ milestone: WatchStreakMilestone) -> some View {
        HStack(spacing: 8) {
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(Self.amber.opacity(0.1))
                .frame(width: 24, height: 24)
                .overlay {
                    Image(systemName: "flame.fill")
                        .font(.system(size: 13, weight: .bold))
                        .foregroundStyle(Self.amberLight)
                }
                .shadow(color: SN.warning.opacity(0.2), radius: 4)
                .accessibilityHidden(true)
            Text(milestone.title)
                .font(.sn(12))
                .foregroundStyle(SN.textPrimary)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button {
                Task {
                    if await rooms.shareWatchStreak(message: composer?.draft ?? "") { composer?.draft = "" }
                }
            } label: {
                Text("Share")
                    .font(.sn(11.5, 600))
                    .foregroundStyle(.black)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .background(Capsule().fill(Self.amber))
                    .shadow(color: SN.warning.opacity(0.2), radius: 5)
            }
            .buttonStyle(ShrinkPress())
            .disabled(rooms.sharing)
            .opacity(rooms.sharing ? 0.5 : 1)
            Button {
                rooms.dismiss("watch")
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(Self.amberLight.opacity(0.6))
                    .padding(4)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Dismiss watch streak")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Self.amber.opacity(0.05))
                .overlay { RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(Self.amber.opacity(0.2), lineWidth: 1) }
        }
    }
}

/// A chip that scales to 0.95 while pressed.
private struct ShrinkPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.95 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}
