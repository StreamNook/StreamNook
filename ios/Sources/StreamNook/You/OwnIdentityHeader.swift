import SNKit
import SwiftUI

/// Who you are at the top of the You tab: your name in your 7TV paint (with the paint's name in a
/// chip beside it), `@login` when it says something the name does not, and the badges you WEAR
/// (Twitch, 7TV, the other chat apps, then StreamNook last, at most eight). The core saves the
/// header for the next cold launch, so it paints on the first frame, then rebuilds it fresh.
struct OwnIdentityHeader: View {
    let model: AppModel
    let user: TwitchUser
    @State private var identity: OwnIdentityData?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            SevenTvAvatar(platformURL: user.profileImageURL, picture: identity?.picture, size: 56)
            details.padding(.top, 8)
        }
        .task(id: user.id) { await load() }
        .task(id: model.cosmetics.registryVersion) { if identity != nil { await refresh() } }
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 8) {
                PaintedText(text: user.displayName, font: .sn(Self.nameSize(user.displayName), 700), paint: identity?.paint,
                            fallback: SN.textPrimary, shadows: .all, lineLimit: 2)
                    .fixedSize(horizontal: false, vertical: true)
                    .layoutPriority(1)
                if let paint = identity?.paint, !paint.name.isEmpty {
                    PaintedText(text: paint.name, font: .sn(11, 700), paint: paint,
                                fallback: Color(red: 0x91 / 255, green: 0x46 / 255, blue: 1), shadows: .all)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 1)
                        .snGlass(Capsule())
                        .layoutPriority(0)
                }
            }
            if user.login.lowercased() != user.displayName.lowercased() {
                Text("@\(user.login)")
                    .font(.sn(13))
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
            }
            let badges = identity?.badges ?? []
            if !badges.isEmpty || identity?.streamnook != nil {
                FlowLayout(spacing: 6) {
                    ForEach(badges) { badge in
                        CosmeticImage(badge.url, fallbacks: badge.fallbacks, animate: true)
                            .frame(width: 18, height: 18)
                            .accessibilityLabel(badge.label)
                    }
                    if let member = identity?.streamnook {
                        CosmeticImage(member.imageURL, animate: true)
                            .frame(width: 24, height: 24)
                            .accessibilityLabel(member.label)
                    }
                }
                .padding(.top, 6)
            }
        }
    }

    /// Twitch caps names at 25 characters, so a size step keeps even the longest to two lines
    /// rather than cutting your own name short.
    static func nameSize(_ name: String) -> CGFloat {
        switch name.count {
        case 21...: 14.5
        case 17...: 16
        case 13...: 17
        default: 18.5
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
