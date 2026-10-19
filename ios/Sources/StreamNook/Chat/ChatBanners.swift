import SNKit
import SwiftUI

/// The surface of a banner over (or under) moving chat: the popover tint at 90 percent, solid
/// at 0 percent glassiness, with no blur, since a blur over repainting chat costs every frame.
/// Paragraphs (an unrolled pin) sit on a near-solid tint so chat never reads through them. A
/// popover's share of the theme's light lies on the tint, under the edge. The poll and
/// prediction cards share it.
struct StaticPopover: ViewModifier {
    var radius: CGFloat = 12
    var tint = 0.9

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        content.background {
            shape.fill(SN.backgroundTertiary.opacity(tint + (1 - Look.shared.glassiness) * (1 - tint)))
                .overlay { ThemeLightLayer(surface: .popover).clipShape(shape) }
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
///
/// They wear the Liquid Glass the rooms strip's capsules wear (solid at Glassiness 0), a 12 point
/// glyph in the leading column, what it is in bold with the detail quieter beside it, then the
/// one action (Share, Liquid Glass tinted with the accent) and a quieter dismiss. They rise out of
/// the composer's edge and fold back.
struct StreakBanners: View {
    let rooms: ChatRooms
    let session: ChatSession

    private static let fold = Animation.timingCurve(0.2, 0.8, 0.2, 1, duration: 0.2)

    var body: some View {
        let streaks = rooms.activeStreaks
        VStack(spacing: 0) {
            if let resub = streaks.resub {
                resubBanner(resub)
                    .padding(.horizontal, 10)
                    .padding(.top, 6)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            if let watch = streaks.watch {
                watchBanner(watch)
                    .padding(.horizontal, 10)
                    .padding(.top, 6)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .clipped()
        .animation(SNMotion.run(Self.fold), value: streaks.resub?.id)
        .animation(SNMotion.run(Self.fold), value: streaks.watch?.milestoneID)
        .animation(SNMotion.run(Self.fold), value: rooms.shareSteps)
    }

    /// What the composer holds, shared with the streak.
    private var composer: ChatComposerModel? { ComposerRegistry.shared.models[ObjectIdentifier(session)] }

    /// Armed or sending: what happens next, and a way out until it is sent.
    private func sharingRow<Icon: View>(_ what: String, title: String, step: ChatRooms.ShareStep,
                                       @ViewBuilder icon: () -> Icon) -> some View {
        HStack(spacing: 8) {
            icon()
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.sn(12, 600)).foregroundStyle(SN.textPrimary)
                Text(step == .sending ? "Sending\u{2026}" : "It sends with your next message")
                    .font(.sn(11)).foregroundStyle(SN.textMuted)
            }
            .lineLimit(1)
            .padding(.vertical, 2)
            .frame(maxWidth: .infinity, alignment: .leading)
            dismissButton(label: "Cancel share") { rooms.cancelShare(what) }
                .disabled(step == .sending)
                .opacity(step == .sending ? 0.4 : 1)
        }
        .padding(.leading, 10)
        .padding(.trailing, 4)
        .padding(.vertical, 4)
        .snGlass(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .contain)
    }

    /// Twitch took it: said once, then the banner goes.
    private func sharedRow(_ title: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 13))
                .foregroundStyle(SN.success)
                .frame(width: 14)
                .accessibilityHidden(true)
            Text(title).font(.sn(12, 600)).foregroundStyle(SN.textPrimary).lineLimit(1)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .snGlass(RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    /// Twitch did not take it: one line under the banner's own words.
    @ViewBuilder private func failedLine(_ what: String) -> some View {
        if rooms.shareStep(what) == .failed {
            Text("That didn't go through. Try again.")
                .font(.sn(11, 500))
                .foregroundStyle(SN.error)
                .lineLimit(1)
        }
    }

    /// Share pressed: the next send carries it, so the field is where the viewer goes next.
    private func arm(_ what: String) {
        rooms.armShare(what)
        composer?.requestFocus(true)
    }

    @ViewBuilder private func resubBanner(_ resub: ResubNotice) -> some View {
        let title = resub.isGift ? "gift sub" : "resub"
        let star = Image(systemName: "star.fill").font(.system(size: 12)).foregroundStyle(SN.accent)
            .frame(width: 14).accessibilityHidden(true)
        switch rooms.shareStep("resub") {
        case .shared:
            sharedRow("Your \(title) is shared")
        case .some(let step) where step == .armed || step == .sending:
            sharingRow("resub", title: "Sharing your \(title)", step: step) { star }
        default:
            resubOffer(resub)
        }
    }

    private func resubOffer(_ resub: ResubNotice) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "star.fill")
                .font(.system(size: 12))
                .foregroundStyle(SN.accent)
                .frame(width: 14)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                (Text(resub.isGift ? "Gift sub" : "Resub").font(.sn(12, 600)).foregroundStyle(SN.textPrimary)
                    + Text(resubDetail(resub).map { " · \($0)" } ?? "")
                    .font(.sn(12)).foregroundStyle(SN.textSecondary))
                    .lineLimit(1)
                if resub.streakMonths > 1 {
                    Button {
                        rooms.includeStreak.toggle()
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: rooms.includeStreak ? "checkmark.square.fill" : "square")
                                .font(.system(size: 11))
                                .foregroundStyle(rooms.includeStreak ? SN.accent : SN.textMuted)
                            Text("Include the \(resub.streakMonths)-month streak")
                                .font(.sn(11))
                                .foregroundStyle(SN.textMuted)
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(rooms.includeStreak ? .isSelected : [])
                }
                failedLine("resub")
            }
            .padding(.vertical, 2)
            .frame(maxWidth: .infinity, alignment: .leading)
            shareButton { arm("resub") }
            dismissButton(label: "Dismiss resub") { rooms.dismiss("resub") }
        }
        .padding(.leading, 10)
        .padding(.trailing, 4)
        .padding(.vertical, 4)
        .snGlass(RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    /// Twitch's streak flame, in the theme's orange like the chat's streak rows.
    private var flame: some View {
        Image(systemName: Symbol.streak)
            .font(.system(size: 13))
            .foregroundStyle(SN.highlightOrange)
            .frame(width: 14)
            .accessibilityHidden(true)
    }

    @ViewBuilder private func watchBanner(_ milestone: WatchStreakMilestone) -> some View {
        switch rooms.shareStep("watch") {
        case .shared:
            sharedRow("Your watch streak is shared")
        case .some(let step) where step == .armed || step == .sending:
            sharingRow("watch", title: "Sharing your watch streak", step: step) { flame }
        default:
            watchOffer(milestone)
        }
    }

    private func watchOffer(_ milestone: WatchStreakMilestone) -> some View {
        HStack(spacing: 8) {
            flame
            VStack(alignment: .leading, spacing: 1) {
                (Text("Watch streak").font(.sn(12, 600)).foregroundStyle(SN.textPrimary)
                    + Text(milestone.streakCount.map { " · \($0) stream\($0 == 1 ? "" : "s")" } ?? "")
                    .font(.sn(12)).foregroundStyle(SN.textSecondary))
                    .lineLimit(1)
                failedLine("watch")
            }
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            shareButton { arm("watch") }
            dismissButton(label: "Dismiss watch streak") { rooms.dismiss("watch") }
        }
        .padding(.leading, 10)
        .padding(.trailing, 4)
        .padding(.vertical, 4)
        .snGlass(RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    /// The banner's one action: Share, on Liquid Glass tinted with the theme's accent.
    private func shareButton(_ action: @escaping () -> Void) -> some View {
        Button {
            action()
        } label: {
            Text("Share")
                .font(.sn(12, 600))
                .padding(.horizontal, 14)
                .frame(height: 28)
        }
        .buttonStyle(AccentGlass())
        .disabled(rooms.sharing)
        .opacity(rooms.sharing ? 0.5 : 1)
    }

    /// A step quieter than Share: a light glyph in muted text, with a 28 point target.
    private func dismissButton(label: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: "xmark")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(SN.textMuted)
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }

    /// The quieter half of the resub line: who gifted it, or how many months.
    private func resubDetail(_ resub: ResubNotice) -> String? {
        if resub.isGift { return resub.gifter.map { "from \($0)" } }
        let n = resub.cumulativeMonths
        return "\(n) month\(n == 1 ? "" : "s")"
    }
}

/// The one action on a glass banner: Liquid Glass tinted with the theme's accent, interactive under
/// the finger (the desktop's glazed accent button), a solid accent capsule at Glassiness 0. Its
/// text is white on a dark accent and near-black on a pale one.
private struct AccentGlass: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(Look.shared.palette.accent.luminance > 0.5 ? Color.black.opacity(0.88) : .white)
            .snGlass(Capsule(), tint: SN.accent, interactive: true)
            .contentShape(Capsule())
            .opacity(configuration.isPressed ? 0.85 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}
