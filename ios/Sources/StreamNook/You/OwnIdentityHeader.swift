import SNKit
import SwiftUI

/// Who you are, at the top of the You tab: your Atmosphere as the card's band, your 7TV picture
/// cut into its edge with your StreamNook badge on it, your name in your 7TV paint below the art,
/// the badges you wear and what you have on, a line of chat as everyone sees it, and the way into
/// Edit profile. The core saves the identity for the next cold launch, so the card paints on the
/// first frame, then rebuilds it fresh.
struct OwnIdentityCard: View {
    let model: AppModel
    let user: TwitchUser
    /// Your picks (badge, Atmosphere, paint, name color), read through the Edit profile model.
    let picks: CosmeticsScreenModel
    @State private var identity: OwnIdentityData?

    static let band: CGFloat = 104
    static let avatar: CGFloat = 80
    static let radius: CGFloat = 22

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            AtmosphereBand(look: look, tint: nameColor(picks.ext?.chatColor))
                .frame(height: Self.band)
            CutoutAvatar(platformURL: user.profileImageURL, picture: identity?.picture, size: Self.avatar,
                         medal: identity?.streamnook?.imageURL, medalLabel: identity?.streamnook?.label ?? "")
                .padding(.leading, 12)
                .padding(.top, -(Self.avatar / 2 + 4))
            details
                .padding(.horizontal, 16)
                .padding(.top, 8)
            if picks.sn != nil || picks.ext != nil {
                ProfilePreviewRow(name: user.displayName, picks: picks, framed: false)
                    .padding(.horizontal, 12)
                    .padding(.top, 14)
            }
            Rectangle().fill(SN.borderSubtle).frame(height: 0.5).padding(.top, 12)
            NavigationLink(value: Route.cosmetics) {
                HStack(spacing: 10) {
                    Text("Edit profile").font(.snRowStrong).foregroundStyle(SN.textPrimary)
                    Text("Atmosphere, badges, paint, color")
                        .font(.snRowDetail)
                        .foregroundStyle(SN.textMuted)
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    Image(systemName: Symbol.disclosure)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(SN.textMuted)
                }
                .padding(.horizontal, 16)
                .frame(height: 50)
                .contentShape(Rectangle())
            }
            .buttonStyle(RowPress())
        }
        .clipShape(RoundedRectangle(cornerRadius: Self.radius, style: .continuous))
        .snPanel(radius: Self.radius)
        .task(id: user.id) { await load() }
        .task(id: model.cosmetics.registryVersion) { if identity != nil { await refresh() } }
    }

    /// Your Atmosphere, from the theme you wear.
    private var look: AtmosphereLook? {
        guard let theme = picks.sn?.theme, theme != "tier" else { return nil }
        return AtmosphereCatalog.shared.look(theme)
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 0) {
            PaintedText(text: user.displayName, font: .sn(Self.nameSize(user.displayName), 700), paint: wornPaint,
                        fallback: nameColor(picks.ext?.chatColor), shadows: .all, lineLimit: 2)
                .fixedSize(horizontal: false, vertical: true)
            if user.login.lowercased() != user.displayName.lowercased() {
                Text("@\(user.login)")
                    .font(.sn(13))
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
                    .padding(.top, 1)
            }
            let badges = identity?.badges ?? []
            let wearing = wearingLine
            if !badges.isEmpty || wearing != nil {
                HStack(spacing: 6) {
                    ForEach(badges.prefix(6)) { badge in
                        WornBadge(badge: badge)
                    }
                    if !badges.isEmpty && wearing != nil {
                        Rectangle().fill(SN.borderLight).frame(width: 1, height: 14).padding(.horizontal, 3)
                    }
                    if let wearing {
                        Text(wearing).font(.snRowDetail).foregroundStyle(SN.textSecondary).lineLimit(1)
                    }
                }
                .padding(.top, 8)
            }
        }
    }

    private var wornPaint: ChatPaint? { picks.ext?.paints.first(where: \.worn)?.paint ?? identity?.paint }

    /// What you have on, in words: the Atmosphere, then the paint.
    private var wearingLine: String? {
        let atmosphere = picks.sn?.atmospheres.first(where: { $0.worn && $0.id != "tier" })?.name
        let paint = wornPaint.map(\.name).flatMap { $0.isEmpty ? nil : "\($0) paint" }
        let parts = [atmosphere, paint].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// Twitch caps names at 25 characters, so a size step keeps even the longest to two lines
    /// rather than cutting your own name short.
    static func nameSize(_ name: String) -> CGFloat {
        switch name.count {
        case 21...: 19
        case 17...: 22
        case 13...: 24
        default: 26
        }
    }

    private func load() async {
        if identity == nil {
            identity = try? await model.core.invoke("own_identity_header_cached", args: ["userId": user.id],
                                                    as: OwnIdentityData?.self)
        }
        await refresh()
    }

    private func refresh() async {
        do {
            identity = try await model.core.invoke("own_identity_header", args: [
                "userId": user.id, "login": user.login, "displayName": user.displayName,
            ], as: OwnIdentityData.self)
        } catch {
            log.debug("own identity unavailable: \(String(describing: error), privacy: .public)")
        }
    }
}

/// A badge you wear, 18 points; one whose every URL fails takes itself out rather than leave a gap.
private struct WornBadge: View {
    let badge: ProfileBadge
    @State private var failed = false

    var body: some View {
        if !failed {
            CosmeticImage(badge.url, fallbacks: badge.fallbacks, animate: true, onFailure: { failed = true })
                .frame(width: 18, height: 18)
                .accessibilityLabel(badge.label)
        }
    }
}
