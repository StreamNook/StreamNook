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

    static let historyLimit = 50

    init(target: ProfileTarget, core: CoreBridge) {
        self.target = target
        self.core = core
    }

    func start() {
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

/// The profile sheet a name tap opens: who they are (7TV picture, painted name), their sub in this
/// room, every badge they have by provider, and what they said this session. No actions, no
/// title row; the system sheet sized to its content, at most 80% of the screen.
struct ProfileSheet: View {
    @State var model: ProfileSheetModel

    private var target: ProfileTarget { model.target }
    private var sheet: ProfileSheetData? { model.sheet }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            if let standing = model.standing {
                SubStandingTile(standing: standing, wornArt: wornSubArt)
                    .padding(.top, 12)
            }
            if let picked = model.picked {
                pickedPanel(picked).padding(.top, 12)
            }
            ForEach(sheet?.groups ?? []) { group in
                if group.badges.contains(where: { !model.failed.contains("\(group.title)/\($0.id)") }) {
                    badgeGroup(group).padding(.top, 12)
                }
            }
            if let member = sheet?.streamnook {
                VStack(alignment: .leading, spacing: 6) {
                    groupTitle("StreamNook")
                    CosmeticImage(member.imageURL, animate: true)
                        .frame(width: 24, height: 24)
                        .accessibilityLabel(member.label)
                }
                .padding(.top, 12)
            }
            if sheet?.hasNoBadges == true {
                Text("No badges to show for this user.")
                    .font(.sn(12.5))
                    .foregroundStyle(SN.textMuted)
                    .frame(maxWidth: .infinity)
                    .padding(.top, 16)
            }
            recent.padding(.top, 16)
        }
        .padding(.horizontal, 16)
        .padding(.top, 20)
        .padding(.bottom, 8)
        .fittedSheet(maxFraction: 0.8)
        .onAppear { model.start() }
        .onDisappear { model.stop() }
    }

    // MARK: Header

    private var header: some View {
        HStack(spacing: 12) {
            SevenTvAvatar(platformURL: sheet?.avatarURL, picture: sheet?.picture, size: 56)
            VStack(alignment: .leading, spacing: 0) {
                let name = sheet?.displayName.isEmpty == false ? sheet!.displayName : target.displayName
                PaintedText(text: name, font: .sn(18, 700), paint: sheet?.paint,
                            fallback: Color(cgColor: FillPainter.color(sheet?.nameColor ?? "#9147FF")
                                ?? CGColor(red: 0.57, green: 0.28, blue: 1, alpha: 1)),
                            shadows: .all)
                if let login = sheet?.shownLogin ?? fallbackLogin {
                    Text("@\(login)")
                        .font(.sn(13))
                        .foregroundStyle(SN.textMuted)
                        .lineLimit(1)
                }
                if let paint = sheet?.paint, !paint.name.isEmpty {
                    Text("Paint: \(paint.name)")
                        .font(.sn(11.5))
                        .foregroundStyle(SN.textSecondary)
                        .lineLimit(1)
                        .padding(.top, 2)
                }
            }
            Spacer(minLength: 0)
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

    private func groupTitle(_ text: String) -> some View {
        Text(text)
            .font(.sn(11, 600))
            .textCase(.uppercase)
            .tracking(0.6)
            .foregroundStyle(SN.textMuted)
    }

    private func badgeGroup(_ group: ProfileBadgeGroup) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            groupTitle(group.title)
            FlowLayout(spacing: 8) {
                ForEach(group.badges) { badge in
                    ProfileBadgeButton(badge: badge, onFailure: { model.failed.insert("\(group.title)/\(badge.id)") }) {
                        model.picked = badge
                    }
                }
            }
        }
    }

    private func pickedPanel(_ badge: ProfileBadge) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            (Text(badge.title.isEmpty ? (badge.name ?? "") : badge.title).font(.sn(12, 700)).foregroundColor(SN.textPrimary)
                + Text(badge.provider.map { " · \($0)" } ?? "").font(.sn(12)).foregroundColor(SN.textMuted))
            if let description = badge.description, !description.isEmpty {
                Text(description).font(.sn(11.5)).foregroundStyle(SN.textMuted)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(SN.surface.opacity(0.6)))
    }

    // MARK: Recent messages

    private var recent: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                groupTitle("Recent messages")
                if let history = model.history, !history.isEmpty {
                    Text("\(history.count)").font(.sn(11, 500)).foregroundStyle(SN.textMuted)
                }
            }
            if let history = model.history {
                if history.isEmpty {
                    Text("Nothing from them in this chat yet.")
                        .font(.sn(12.5))
                        .foregroundStyle(SN.textMuted)
                } else {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(history) { row in
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text(row.date.map { $0.formatted(.dateTime.hour().minute()) } ?? "")
                                    .font(.sn(11.5).monospacedDigit())
                                    .foregroundStyle(SN.textMuted)
                                    .lineLimit(1)
                                    .fixedSize()
                                    .frame(minWidth: 52, alignment: .leading)
                                Text(row.content)
                                    .font(.sn(13))
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

/// A profile picture whose box never changes size: the plain surface until the platform picture
/// is known, then that picture, with the 7TV picture over it once it has decoded (and the
/// platform picture back if the 7TV one fails).
struct SevenTvAvatar: View {
    let platformURL: String?
    let picture: AvatarPick?
    var size: CGFloat = 56
    @State private var sevenTvFailed = false

    var body: some View {
        ZStack {
            Circle().fill(SN.surface)
            if let platformURL {
                CosmeticImage(platformURL, animate: false, fill: true)
                if let picture, !sevenTvFailed {
                    CosmeticImage(picture.animated ?? picture.still, fallbacks: picture.animated == nil ? [] : [picture.still],
                                  animate: picture.animated != nil, alwaysMove: true, fill: true,
                                  onFailure: { sevenTvFailed = true })
                }
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .overlay { Circle().stroke(Color.white.opacity(0.1), lineWidth: 1) }
        .accessibilityHidden(true)
    }
}
