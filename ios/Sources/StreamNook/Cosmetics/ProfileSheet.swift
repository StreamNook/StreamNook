import Observation
import SNKit
import SwiftUI

/// What one open profile sheet holds: the core's sheet answer (seeded from its cache so a reopen
/// paints at once), the sub standing in this room, and the person's recent messages kept live
/// while the sheet is open. Every read fails soft; a closed sheet drops late answers.
@Observable @MainActor
final class ProfileSheetModel {
    let target: ProfileTarget
    private(set) var sheet: ProfileSheetData?
    private(set) var standing: SubStanding?
    /// nil until the store answers, so "nothing yet" is never shown over a list on its way.
    private(set) var history: [ProfileHistoryRow]?
    var picked: ProfileBadge?
    /// Badges whose every image failed; a group left with none is not shown.
    var failed: Set<String> = []
    @ObservationIgnored private let core: CoreBridge
    @ObservationIgnored private var tasks: [Task<Void, Never>] = []
    /// The user key whose history is being followed: the target's id, or for a login-only
    /// target (a mention, the long-press Profile item) the id the core resolved.
    @ObservationIgnored private var watchedKey: String?

    /// The signed-in viewer's Twitch id: your own sheet shows your picture without the medal.
    let ownID: String?
    /// A section to scroll to (drive steps: `member`, `recent`, or a badge group's title).
    var scrollTarget: String?
    @ObservationIgnored private weak var store: CosmeticsStore?

    static let historyLimit = 50

    init(target: ProfileTarget, core: CoreBridge, ownID: String? = nil, store: CosmeticsStore? = nil) {
        self.target = target
        self.core = core
        self.ownID = ownID
        self.store = store
    }

    /// Another person's StreamNook badge rides their picture; your own picture carries no marks.
    var medal: SNMemberBadge? {
        guard let badge = sheet?.streamnook else { return nil }
        let id = sheet?.userID.isEmpty == false ? sheet?.userID : target.userID
        if let ownID, !ownID.isEmpty, ownID == id { return nil }
        return badge
    }

    func start() {
        store?.activeProfile = self
        let core = core
        let target = target
        tasks.append(Task { [weak self] in
            if let cached = try? await core.invoke("user_profile_sheet_cached",
                                                   args: ["userId": target.userID, "channelId": target.roomID],
                                                   as: ProfileSheetData?.self) {
                self?.sheet = cached
            }
            do {
                let fresh = try await core.invoke("user_profile_sheet", args: [
                    "userId": target.userID, "login": target.login, "displayName": target.displayName,
                    "color": target.color, "channelId": target.roomID, "channelLogin": target.roomLogin,
                ], as: ProfileSheetData.self)
                guard !Task.isCancelled else { return }
                self?.sheet = fresh
                if target.userID.isEmpty, !fresh.userID.isEmpty {
                    await self?.followHistory(key: fresh.userID)
                }
            } catch {
                log.debug("profile sheet unavailable: \(String(describing: error), privacy: .public)")
            }
        })
        if let room = target.roomLogin, !room.isEmpty {
            tasks.append(Task { [weak self] in
                do {
                    let standing = try await core.invoke("get_sub_standing", args: [
                        "login": target.login, "channel": room, "wornBadges": target.badgeKeys,
                    ], as: SubStanding?.self)
                    guard !Task.isCancelled else { return }
                    self?.standing = standing
                } catch {
                    log.debug("sub standing unavailable: \(String(describing: error), privacy: .public)")
                }
            })
        }
        if !target.userID.isEmpty {
            tasks.append(Task { [weak self] in
                await self?.followHistory(key: target.userID)
            })
        }
    }

    func stop() {
        tasks.forEach { $0.cancel() }
        tasks.removeAll()
        guard let key = watchedKey else { return }
        watchedKey = nil
        let core = core
        Task { _ = try? await core.invoke("unwatch_user_history", args: ["userKey": key]) }
    }

    /// The session's messages from this person: the store's latest 50, then each new one pushed
    /// while the sheet is open (deduped by id, trimmed to the newest 50).
    private func followHistory(key: String) async {
        watchedKey = key
        let events = core.events.stream("user-history-message")
        _ = try? await core.invoke("watch_user_history", args: ["userKey": key])
        do {
            let rows = try await core.invoke("get_user_message_history_limited", args: ["userId": key, "limit": Self.historyLimit],
                                             as: [ProfileHistoryRow].self)
            history = rows
        } catch {
            history = []
        }
        for await event in events {
            guard !Task.isCancelled else { return }
            guard let push = try? JSONDecoder().decode(HistoryPush.self, from: Data(event.payloadJSON.utf8)),
                  push.userKey == key, let id = push.message.id else { continue }
            var rows = history ?? []
            guard !rows.contains(where: { $0.id == id }) else { continue }
            rows.append(ProfileHistoryRow(id: id, content: push.message.content ?? "", timestamp: push.message.timestamp))
            if rows.count > Self.historyLimit { rows.removeFirst(rows.count - Self.historyLimit) }
            history = rows
        }
    }

    private struct HistoryPush: Decodable {
        struct Message: Decodable {
            var id: String?
            var content: String?
            var timestamp: String?
        }
        var userKey: String
        var message: Message
        enum CodingKeys: String, CodingKey { case userKey = "user_key", message }
    }
}

/// The profile sheet a name tap opens, in the same shape as your own card: their Atmosphere
/// glowing behind them, their picture with their StreamNook badge on it as a medal (never on your own; it shows
/// nowhere else here), their name in their paint, their sub in this room, every badge they have
/// under its platform's mark, and what they said this session. No actions; the system sheet sized
/// to its content, at most 80% of the screen.
struct ProfileSheet: View {
    @State var model: ProfileSheetModel

    private var target: ProfileTarget { model.target }
    private var sheet: ProfileSheetData? { model.sheet }

    static let glow: CGFloat = 200
    static let avatar: CGFloat = 76

    var body: some View {
        ScrollViewReader { proxy in
            content
                .onChange(of: model.scrollTarget) { _, section in
                    guard let section else { return }
                    withAnimation { proxy.scrollTo(section, anchor: .top) }
                    model.scrollTarget = nil
                }
        }
        .fittedSheet(maxFraction: 0.8)
        .onAppear { model.start() }
        .onDisappear { model.stop() }
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 12) {
                CutoutAvatar(platformURL: sheet?.avatarURL, picture: sheet?.picture, size: Self.avatar,
                             medal: model.medal?.imageURL, medalLabel: model.medal?.label ?? "")
                Spacer(minLength: 0)
                if let rank = sheet?.member {
                    MemberRankMark(rank: rank).padding(.top, 6)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 24)
            VStack(alignment: .leading, spacing: 0) {
                identity.padding(.top, 8)
                if let standing = model.standing {
                    SubStandingTile(standing: standing, wornArt: wornSubArt)
                        .padding(.top, 16)
                }
                if let picked = model.picked {
                    pickedPanel(picked).padding(.top, 12)
                }
                ForEach(sheet?.groups ?? []) { group in
                    badgeGroup(group).padding(.top, 20).id(group.title)
                }
                if sheet?.hasNoBadges == true && model.medal == nil {
                    Text("No badges to show for this user.")
                        .font(.snRowDetail)
                        .foregroundStyle(SN.textMuted)
                        .padding(.top, 16)
                }
                if let member = sheet?.memberProfile {
                    MemberSections(profile: member).padding(.top, 20).id("member")
                }
                recent.padding(.top, 20).id("recent")
            }
            .padding(.horizontal, 16)
        }
        .padding(.bottom, 8)
        .background(alignment: .top) {
            AtmosphereGlow(look: sheet?.atmosphere?.look, tint: nameColor(sheet?.nameColor ?? target.color))
                .frame(height: Self.glow)
        }
    }

    // MARK: Identity

    private var identity: some View {
        VStack(alignment: .leading, spacing: 0) {
            let name = sheet?.displayName.isEmpty == false ? sheet!.displayName : target.displayName
            PaintedText(text: name, font: .sn(OwnIdentityCard.nameSize(name) - 2, 700), paint: sheet?.paint,
                        fallback: nameColor(sheet?.nameColor ?? target.color), shadows: .all, lineLimit: 2)
                .fixedSize(horizontal: false, vertical: true)
            let login = sheet?.shownLogin ?? fallbackLogin
            // What they have on, in words: the Atmosphere, then the paint.
            let parts = [sheet?.atmosphere?.name, sheet?.paint.flatMap { $0.name.isEmpty ? nil : "\($0.name) paint" }]
                .compactMap { $0 }.filter { !$0.isEmpty }
            let wearing: String? = parts.isEmpty ? nil : parts.joined(separator: " · ")
            if login != nil || wearing != nil {
                HStack(spacing: 6) {
                    if let login {
                        Text("@\(login)").font(.sn(13)).foregroundStyle(SN.textMuted).lineLimit(1)
                    }
                    if login != nil && wearing != nil {
                        Text("·").font(.sn(13)).foregroundStyle(SN.textMuted)
                    }
                    if let wearing {
                        Text(wearing).font(.snRowDetail).foregroundStyle(SN.textSecondary).lineLimit(1)
                    }
                }
                .padding(.top, 2)
            }
        }
    }

    /// Before the core answers: the login line from what the row carried.
    private var fallbackLogin: String? {
        guard sheet == nil else { return nil }
        return target.login.lowercased() == target.displayName.lowercased() ? nil : target.login
    }

    /// The sub or founder badge the opening row wore, for the tile when Rust has no art.
    private var wornSubArt: String? { target.subBadgeArt }

    // MARK: Badges

    private func badgeGroup(_ group: ProfileBadgeGroup) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            // A brand's group sits under its real mark (StreamNook's is the Penrose); "Relics"
            // and "Chat clients" keep their words.
            let platform = PlatformMark.Platform(named: group.title)
            let streamNook = group.title == "StreamNook"
            ProfileSectionHeader(platform == nil && !streamNook ? group.title : "Badges", platform: platform,
                                 streamNook: streamNook, detail: "\(group.badges.count)")
            FlowLayout(spacing: 8) {
                ForEach(group.badges) { badge in
                    ProfileBadgeButton(badge: badge) { model.picked = badge }
                }
            }
        }
    }

    private func pickedPanel(_ badge: ProfileBadge) -> some View {
        HStack(alignment: .top, spacing: 10) {
            CosmeticImage(badge.url, fallbacks: badge.fallbacks, animate: true).frame(width: 28, height: 28)
            VStack(alignment: .leading, spacing: 2) {
                Text(badge.title.isEmpty ? (badge.name ?? "") : badge.title)
                    .font(.sn(13.5, 600))
                    .foregroundStyle(SN.textPrimary)
                if let description = badge.description, !description.isEmpty {
                    Text(description).font(.snRowDetail).foregroundStyle(SN.textSecondary)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .snPanel()
    }

    // MARK: Recent messages

    private var recent: some View {
        VStack(alignment: .leading, spacing: 8) {
            ProfileSectionHeader("Recent messages", detail: model.history.flatMap { $0.isEmpty ? nil : "\($0.count)" })
            if let history = model.history {
                if history.isEmpty {
                    Text("Nothing from them in this chat yet.")
                        .font(.snRowDetail)
                        .foregroundStyle(SN.textMuted)
                } else {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(history) { row in
                            HStack(alignment: .firstTextBaseline, spacing: 10) {
                                Text(row.date.map { $0.formatted(.dateTime.hour(.defaultDigits(amPM: .omitted)).minute()) } ?? "")
                                    .font(.sn(12).monospacedDigit())
                                    .foregroundStyle(SN.textMuted)
                                    .frame(width: 42, alignment: .leading)
                                Text(row.content)
                                    .font(.sn(14))
                                    .foregroundStyle(SN.textPrimary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One badge on the sheet: a 32 pt button holding its 28 pt image, pressed at 90%, named for
/// VoiceOver. A badge whose every URL fails takes itself out.
private struct ProfileBadgeButton: View {
    let badge: ProfileBadge
    var onFailure: () -> Void = {}
    let action: () -> Void
    @State private var failed = false

    var body: some View {
        if !failed {
            Button(action: action) {
                CosmeticImage(badge.url, fallbacks: badge.fallbacks, animate: true, onFailure: {
                    failed = true
                    onFailure()
                })
                    .frame(width: 28, height: 28)
                    .frame(width: 32, height: 32)
                    .contentShape(Rectangle())
            }
            .buttonStyle(BadgePress())
            .accessibilityLabel(badge.label)
        }
    }
}

/// Scales to 90% while pressed.
struct BadgePress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.9 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

/// The one picture a person shows in StreamNook chat, in a box that never changes size: their 7TV
/// picture when they have one, else their platform picture, never one over the other (a 7TV
/// picture with transparent parts let the Twitch photo show through and read as one blended
/// image). It sits on a black disc (Brandon: black behind a clear picture, not gray), which also
/// shows until the picture decodes; the platform picture takes over only if the 7TV one fails.
struct SevenTvAvatar: View {
    let platformURL: String?
    let picture: AvatarPick?
    var size: CGFloat = 56
    @State private var sevenTvFailed = false

    var body: some View {
        ZStack {
            Circle().fill(Color.black)
            if let picture, !sevenTvFailed {
                CosmeticImage(picture.animated ?? picture.still, fallbacks: picture.animated == nil ? [] : [picture.still],
                              animate: picture.animated != nil, alwaysMove: true, fill: true,
                              onFailure: { sevenTvFailed = true })
            } else if let platformURL {
                CosmeticImage(platformURL, animate: false, fill: true)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .overlay { Circle().strokeBorder(Color.white.opacity(0.1), lineWidth: 0.5) }
        .accessibilityHidden(true)
    }
}
