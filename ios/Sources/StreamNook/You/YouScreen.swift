import SNKit
import SwiftUI

/// You: your identity card as the screen's header (no title above it: the card says whose tab
/// this is), the accounts you are signed in with (Twitch, 7TV, Kick, each with its own sign in or
/// out), where the app opens, then the settings sections.
struct YouScreen: View {
    let model: AppModel
    @AppStorage("sn-start-tab") private var startTab = MobileTab.following.rawValue
    @State private var picks: CosmeticsScreenModel?
    @State private var ownIdentity: OwnIdentityData?

    private var user: TwitchUser? { model.signIn.user }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                identity
                    .padding(.horizontal, 16)
                    .padding(.top, 8)

                // Signed out, the card above already offers the sign-in; the accounts wait for it.
                if user != nil {
                    label("Accounts")
                    AccountsCard(model: model, picks: picks, sevenTvPicture: ownIdentity?.picture)
                        .padding(.horizontal, 16)
                }

                label("Open the app on")
                GlazeSegments(MobileTab.allCases.map { Segment(value: $0, label: $0.title) },
                              selection: Binding(get: { MobileTab(rawValue: startTab) },
                                                 set: { if let tab = $0 { startTab = tab.rawValue } }), fill: true)
                    .padding(.horizontal, 16)

                label("Settings")
                SettingsDirectory(model: model)
            }
            .padding(.bottom, 24)
        }
        .scrollIndicators(.hidden)
        .background(SN.background)
        .navigationTitle("You")
        .toolbar(.hidden, for: .navigationBar)
        // Every visit re-reads your picks, so a change made in Edit profile shows on return.
        .task(id: user?.id) {
            guard user != nil else { picks = nil; return }
            let next = picks ?? CosmeticsScreenModel(app: model)
            picks = next
            await next.load()
        }
        .onAppear { if let picks { Task { await picks.load() } } }
        .sheet(isPresented: Binding(get: { picks?.showSevenTvSignIn ?? false },
                                    set: { picks?.showSevenTvSignIn = $0 })) {
            SevenTvSignInSheet(core: model.core) { stored in
                Task {
                    await picks?.sevenTvSignInFinished(stored)
                    await picks?.load()
                }
            }
        }
    }

    @ViewBuilder
    private var identity: some View {
        if let user, let picks {
            OwnIdentityCard(model: model, user: user, picks: picks, identity: $ownIdentity)
        } else {
            SignedOutIdentityCard(model: model)
        }
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .snSectionStyle()
            .padding(.horizontal, 16)
            .padding(.top, 24)
            .padding(.bottom, 8)
    }
}

/// Signed out, the card keeps its shape: the quiet band, an empty avatar, why to sign in, and the
/// Twitch sign-in.
private struct SignedOutIdentityCard: View {
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            AtmosphereBand(look: nil).frame(height: OwnIdentityCard.band)
            Circle().fill(SN.surface)
                .frame(width: OwnIdentityCard.avatar, height: OwnIdentityCard.avatar)
                .overlay { Image(systemName: "person.fill").font(.system(size: 30)).foregroundStyle(SN.textMuted) }
                .padding(4)
                .background { Circle().fill(SN.background).overlay { Circle().fill(SN.floor) } }
                .padding(.leading, 12)
                .padding(.top, -(OwnIdentityCard.avatar / 2 + 4))
            VStack(alignment: .leading, spacing: 4) {
                Text("Not signed in").font(.sn(24, 700)).foregroundStyle(SN.textPrimary)
                Text("Sign in to show your badges, Atmosphere and 7TV paint, and to follow channels and chat.")
                    .font(.snBody)
                    .foregroundStyle(SN.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)
            SignInButton(model: model, maxWidth: .infinity)
                .padding(16)
        }
        .clipShape(RoundedRectangle(cornerRadius: OwnIdentityCard.radius, style: .continuous))
        .snPanel(radius: OwnIdentityCard.radius)
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

/// One row per account: Twitch (the account the whole app runs on), 7TV (what lets you wear a
/// paint and a 7TV badge) and Kick. Each row is the platform's mark, its name, one line of state,
/// and its own sign in or out.
struct AccountsCard: View {
    let model: AppModel
    let picks: CosmeticsScreenModel?
    /// Your 7TV picture, for the 7TV row; the Twitch row shows your Twitch picture.
    let sevenTvPicture: AvatarPick?
    @State private var twitchArmed = false
    @State private var sevenTvArmed = false
    @State private var kickArmed = false

    private var user: TwitchUser? { model.signIn.user }
    private var sevenTvSignedIn: Bool { picks?.ext?.sevenTvSignedIn ?? false }

    var body: some View {
        VStack(spacing: 0) {
            row(picture: AnyView(AccountAvatar(url: user?.profileImageURL, platform: .twitch)), title: "Twitch",
                detail: user.map { twitchArmed ? "Signs you out of StreamNook" : "@\($0.login)" } ?? "Not signed in") {
                if user != nil {
                    ArmedPill(label: "Sign out", armed: $twitchArmed) {
                        Task { await model.signOut() }
                    }
                } else {
                    BrandPill(label: model.signIn.busy ? "Signing in" : "Sign in", fill: SN.twitch, text: .white,
                              enabled: model.started && !model.signIn.busy) {
                        Task { await model.signIn.begin(core: model.core) }
                    }
                }
            }
            if user != nil, let picks {
                divider
                row(picture: AnyView(SevenTvAccountPicture(picture: sevenTvPicture)), title: "7TV",
                    detail: sevenTvSignedIn ? (sevenTvArmed ? "Your paint and badge stay on 7TV" : "Connected")
                                            : "Paints and badges") {
                    if sevenTvSignedIn {
                        ArmedPill(label: "Disconnect", armed: $sevenTvArmed) {
                            Task {
                                await picks.signOutOfSevenTv()
                                await picks.load()
                            }
                        }
                    } else {
                        BrandPill(label: picks.signingIn ? "Connecting" : "Connect", fill: FixedColor.sevenTV.color,
                                  text: Color(red: 0.02, green: 0.13, blue: 0.18), enabled: !picks.signingIn) {
                            picks.signInToSevenTv()
                        }
                    }
                }
            }
            divider
            kickRow
        }
        .snPanel()
    }

    /// Kick: one "Sign in with Kick" brings both the kick.com session (your follows) and Kick's
    /// own authorization (chat). Signed in, the follows can be read again and it signs out.
    private var kickRow: some View {
        let kick = model.kick
        return row(picture: AnyView(AccountAvatar(url: kick.avatarURL, platform: .kick)),
                   title: kick.name ?? "Kick", detail: kick.detail) {
            if kick.connected {
                HStack(spacing: 0) {
                    Button { Task { await kick.resync() } } label: {
                        Image(systemName: "arrow.clockwise")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(SN.textMuted)
                            .symbolEffect(.rotate, options: .repeat(.continuous), isActive: kick.syncing)
                            .frame(width: 44, height: 44)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(kick.syncing || kick.busy)
                    .opacity(kick.syncing || kick.busy ? 0.5 : 1)
                    .accessibilityLabel("Update your Kick follows")
                    ArmedPill(label: "Sign out", enabled: !kick.busy, armed: $kickArmed) {
                        Task { await kick.disconnect() }
                    }
                }
                // The buttons keep their words; a long name gives way instead.
                .fixedSize()
            } else {
                BrandPill(label: kick.busy ? "Signing in" : "Sign in with Kick", fill: SN.kick, text: .black,
                          enabled: model.started && !kick.busy) {
                    Task { await kick.connect() }
                }
            }
        }
    }

    private var divider: some View {
        Rectangle().fill(SN.borderSubtle).frame(height: 0.5).padding(.leading, 70)
    }

    private func row<Action: View>(picture: AnyView, title: String, detail: String,
                                   @ViewBuilder action: () -> Action) -> some View {
        HStack(spacing: 12) {
            picture
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.snRowStrong).foregroundStyle(SN.textPrimary).lineLimit(1)
                Text(detail).font(.snRowDetail).foregroundStyle(SN.textMuted).lineLimit(1)
            }
            Spacer(minLength: 8)
            action()
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
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

/// A sign-in in the platform's own colors, so it reads as signing in with that platform.
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

/// The 7TV row's picture: your 7TV picture with 7TV's mark on its corner, or the mark alone in its
/// tile when you have none.
private struct SevenTvAccountPicture: View {
    let picture: AvatarPick?

    var body: some View {
        if let picture {
            CosmeticImage(picture.animated ?? picture.still, fallbacks: picture.animated == nil ? [] : [picture.still],
                          animate: picture.animated != nil, fill: true)
                .frame(width: 44, height: 44)
                .clipShape(Circle())
                .overlay(alignment: .bottomTrailing) {
                    PlatformMark(platform: .sevenTV, size: 9)
                        .padding(3)
                        .background(Circle().fill(SN.backgroundTertiary))
                        .offset(x: 4, y: 2)
                }
                .accessibilityHidden(true)
        } else {
            BrandTile(platform: .sevenTV)
        }
    }
}
