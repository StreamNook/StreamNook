import SNKit
import SwiftUI

/// You: who you are, the accounts you are signed in with (each with its own sign in or sign out),
/// where the app opens, then the settings sections, with no menu in between.
struct YouScreen: View {
    let model: AppModel
    @AppStorage("sn-start-tab") private var startTab = MobileTab.following.rawValue

    private var user: TwitchUser? { model.signIn.user }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                identity
                NavigationLink(value: Route.cosmetics) {
                    RowLabel(symbol: Symbol.cosmetics, title: "Cosmetics", detail: "Your badges, atmosphere, and 7TV paint",
                             symbolTint: SN.accent)
                }
                .buttonStyle(RowPress())
                .padding(.horizontal, 12)
                .padding(.bottom, 8)

                label("Accounts")
                AccountsCard(model: model)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 16)

                label("Open the app on")
                GlazeSegments(MobileTab.allCases.map { Segment(value: $0, label: $0.title) },
                              selection: Binding(get: { MobileTab(rawValue: startTab) },
                                                 set: { if let tab = $0 { startTab = tab.rawValue } }))
                    .padding(.horizontal, 16)
                .padding(.bottom, 16)

                label("Settings")
                VStack(spacing: 0) {
                    ForEach(SettingsSection.allCases) { section in
                        NavigationLink(value: Route.settings(section)) {
                            RowLabel(symbol: section.symbol, title: section.title, detail: section.detail)
                        }
                        .buttonStyle(RowPress())
                    }
                }
                .padding(.horizontal, 12)
            }
            .padding(.bottom, 24)
        }
        .scrollIndicators(.hidden)
        .background(SN.background)
        .navigationTitle("You")
        .toolbarTitleDisplayMode(.inlineLarge)
    }

    @ViewBuilder
    private var identity: some View {
        if let user {
            OwnIdentityHeader(model: model, user: user)
                .padding(.horizontal, 16)
                .padding(.top, 16)
                .padding(.bottom, 12)
        } else {
            signedOutIdentity
        }
    }

    private var signedOutIdentity: some View {
        HStack(alignment: .top, spacing: 12) {
            Avatar(url: user?.profileImageURL, size: 56)
            VStack(alignment: .leading, spacing: 2) {
                Text(user?.displayName ?? "Not signed in")
                    .font(.sn(18, 700))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                Text(user.map { "@\($0.login)" } ?? "Sign in with Twitch below to follow channels and chat.")
                    .font(.snRowDetail)
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(2)
            }
            .padding(.top, user == nil ? 6 : 8)
        }
        .padding(.horizontal, 16)
        .padding(.top, 16)
        .padding(.bottom, 12)
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .snSectionStyle()
            .padding(.horizontal, 16)
            .padding(.bottom, 6)
    }
}

/// A round profile picture, or the theme's surface while there is none.
struct Avatar: View {
    let url: String?
    var size: CGFloat = 44

    var body: some View {
        AsyncImage(url: url.flatMap(URL.init(string:))) { phase in
            if let image = phase.image {
                image.resizable().scaledToFill()
            } else {
                SN.surface
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .accessibilityHidden(true)
    }
}

/// One row per platform the phone signs in to. Twitch is the account the whole app runs on; Kick
/// is optional.
struct AccountsCard: View {
    let model: AppModel
    @State private var twitchArmed = false

    private var user: TwitchUser? { model.signIn.user }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                AccountAvatar(url: user?.profileImageURL, platform: .twitch)
                VStack(alignment: .leading, spacing: 2) {
                    // The header above already names you; this row names the platform.
                    Text("Twitch").font(.snRowStrong).foregroundStyle(SN.textPrimary).lineLimit(1)
                    Text(user.map { twitchArmed ? "Signs you out of StreamNook" : "@\($0.login)" } ?? "Not signed in")
                        .font(.snRowDetail)
                        .foregroundStyle(SN.textMuted)
                }
                Spacer(minLength: 8)
                if user != nil {
                    ArmedPill(label: "Sign out", armed: $twitchArmed) {
                        Task { await model.signOut() }
                    }
                } else {
                    BrandPill(label: model.signIn.busy ? "Signing in" : "Sign in with Twitch", fill: SN.twitch, text: .white,
                              enabled: model.started && !model.signIn.busy) {
                        Task { await model.signIn.begin(core: model.core) }
                    }
                }
            }
            .padding(14)
            Divider().overlay(SN.borderSubtle)
            HStack(spacing: 12) {
                AccountAvatar(url: nil, platform: .kick)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Kick").font(.snRowStrong).foregroundStyle(SN.textPrimary)
                    Text("Not signed in").font(.snRowDetail).foregroundStyle(SN.textMuted)
                }
                Spacer(minLength: 8)
                BrandPill(label: "Sign in with Kick", fill: SN.kick, text: .white, enabled: false) {}
            }
            .padding(14)
        }
        .snPanel()
    }
}

/// An account's avatar with the platform's mark on its corner, lifted by a shadow; without a
/// picture, the surface with the mark in the middle.
private struct AccountAvatar: View {
    let url: String?
    let platform: PlatformMark.Platform

    var body: some View {
        if url != nil {
            Avatar(url: url, size: 44)
                .overlay(alignment: .bottomTrailing) {
                    PlatformMark(platform: platform, size: 15)
                        .shadow(color: .black.opacity(0.7), radius: 2, y: 1)
                        .offset(x: 4, y: 2)
                }
        } else {
            Circle().fill(SN.surface).frame(width: 44, height: 44)
                .overlay { PlatformMark(platform: platform, size: 20) }
        }
    }
}

/// A sign-in in the platform's own colours, so it reads as signing in with that platform.
private struct BrandPill: View {
    let label: String
    let fill: Color
    let text: Color
    var enabled = true
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.sn(12.5, 700))
                .foregroundStyle(text)
                .lineLimit(1)
                .padding(.horizontal, 14)
                .padding(.vertical, 6)
                .background(Capsule().fill(fill))
                .frame(minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.6)
    }
}
