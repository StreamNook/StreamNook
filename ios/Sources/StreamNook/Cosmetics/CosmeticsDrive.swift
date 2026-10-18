#if DEBUG
import Foundation
import SNKit
import SwiftUI

/// Drive steps for the profile and cosmetics screens:
///
/// - `profile:LOGIN` opens the profile sheet for a Twitch login (in the watched room when there is
///   one); `profile:row` opens it for the newest chatter in the watched room's chat, carrying the
///   row's colour and badges as a name tap does; `profile:close` closes it
/// - `cosmetics:open:KEY` opens a collapsible section (`paints`, `seventvBadges`, `twitch`);
///   `cosmetics:scroll:ID` scrolls the screen to `badge`, `atmosphere`, `seventv` or `twitch`
/// - `cosmetics:dump` logs what the core answers for the screen (read only)
/// - `wash:find` logs the chatters in the watched room whose rows carry an Atmosphere or a
///   StreamNook badge, and the catalog the rows draw from
/// - `cosmetics:equip:badge:SLUG`, `cosmetics:equip:theme:ID`, `cosmetics:chat:on|off`,
///   `cosmetics:equip:paint:ID`, `cosmetics:equip:7tvbadge:ID`, `cosmetics:equip:twitch:SET`
///   change what is worn through the screen, exactly as a tap does. FakeCore only: these never
///   run against a real account.
@MainActor
enum CosmeticsDrive {
    static func run(_ name: String, _ value: String, model: AppModel) async -> Bool {
        switch name {
        case "profile": await profile(value, model: model)
        case "cosmetics": await cosmetics(value, model: model)
        case "wash" where value == "find": await findWashes(model)
        case "wash" where value == "paints": countDecorated(model)
        default: return false
        }
        return true
    }

    private static func profile(_ value: String, model: AppModel) async {
        let session = model.chat.activeSession
        switch value {
        case "close":
            model.cosmetics.profile = nil
        case let scroll where scroll.hasPrefix("scroll:"):
            model.cosmetics.activeProfile?.scrollTarget = String(scroll.dropFirst("scroll:".count))
        case "row":
            guard let session, let message = session.pipeline.recentMessages(60).last(where: { !$0.isSystem }) else {
                log.error("drive profile: no chat row")
                return
            }
            model.cosmetics.open(ProfileTarget(userID: message.userID, login: message.username, displayName: message.displayName,
                                               color: message.metadata.view?.nameColor ?? message.color,
                                               badgeKeys: message.badges.map { "\($0.name)/\($0.version)" },
                                               roomID: session.roomID, roomLogin: session.channel))
            log.notice("drive profile row \(message.username, privacy: .public)")
        default:
            model.cosmetics.open(ProfileTarget(userID: "", login: value, displayName: value, roomID: session?.roomID,
                                               roomLogin: session?.channel))
        }
    }

    private static func cosmetics(_ value: String, model: AppModel) async {
        let parts = value.split(separator: ":", maxSplits: 2).map(String.init)
        switch parts.first ?? "" {
        case "open":
            if parts.count > 1 { model.cosmetics.openSections.insert(parts[1]) }
        case "scroll":
            if parts.count > 1 { model.cosmetics.scrollTarget = parts[1] }
        case "dump":
            await dump(model)
        case "equip", "chat":
            guard model.core is FakeCore else {
                log.error("drive cosmetics: writes run on FakeCore only")
                return
            }
            guard let screen = model.cosmetics.activeScreen else {
                log.error("drive cosmetics: the Cosmetics screen is not open")
                return
            }
            await equip(parts, screen: screen)
        default:
            log.error("drive cosmetics: unknown \(value, privacy: .public)")
        }
    }

    private static func equip(_ parts: [String], screen: CosmeticsScreenModel) async {
        if parts.first == "chat" {
            await screen.setChatAtmosphere(parts.count > 1 && parts[1] == "on")
            return
        }
        guard parts.count > 2 else { return }
        let id = parts[2]
        switch parts[1] {
        case "badge":
            if let badge = screen.sn?.badges.first(where: { $0.slug == id }) { await screen.equipBadge(badge) }
        case "theme":
            if let atmosphere = screen.sn?.atmospheres.first(where: { $0.id == id }) { await screen.equipTheme(atmosphere) }
        case "paint":
            if let paint = screen.ext?.paints.first(where: { $0.id == id }) { await screen.equipPaint(paint) }
        case "7tvbadge":
            if let badge = screen.ext?.sevenTvBadges.first(where: { $0.id == id }) { await screen.equipSevenTvBadge(badge) }
        case "twitch":
            if let badge = screen.ext?.twitchSelectable.first(where: { $0.setID == id }) { await screen.equipTwitchBadge(badge) }
        case "color":
            if let color = screen.ext?.namedColors.first(where: { $0.value == id }) { await screen.setChatColor(color) }
        case "bio":
            _ = await screen.saveBio(id.replacingOccurrences(of: "_", with: " "))
        default:
            log.error("drive cosmetics: unknown equip \(parts[1], privacy: .public)")
        }
    }

    private static func dump(_ model: AppModel) async {
        guard let user = model.signIn.user else {
            log.error("drive cosmetics dump: signed out")
            return
        }
        let sn = try? await model.core.invoke("cosmetics_picker_state", args: ["userId": user.id], as: StreamNookPickerState.self)
        let ext = try? await model.core.invoke("external_cosmetics_state", args: ["userId": user.id, "login": user.login],
                                               as: ExternalPickerState.self)
        let me = try? await model.core.invoke("own_identity_header", args: ["userId": user.id, "login": user.login,
                                                                           "displayName": user.displayName],
                                              as: OwnIdentityData.self)
        log.notice("drive cosmetics badges \(sn?.badges.count ?? -1) worn \(sn?.badges.first(where: \.worn)?.slug ?? "-", privacy: .public) theme \(sn?.theme ?? "-", privacy: .public) atmospheres \(sn?.atmospheres.map { "\($0.id)\($0.allowed ? "" : "(locked)")\($0.worn ? "*" : "")" }.joined(separator: ",") ?? "-", privacy: .public) chat \(sn?.chatAtmosphere ?? false)")
        log.notice("drive cosmetics 7tv signed in \(ext?.sevenTvSignedIn ?? false) paints \(ext?.paints.count ?? -1) worn \(ext?.paints.first(where: \.worn)?.name ?? "-", privacy: .public) badges \(ext?.sevenTvBadges.count ?? -1) drops \(ext?.dropsSignedIn ?? false) twitch selectable \(ext?.twitchSelectable.count ?? -1) earned \(ext?.twitchEarned.count ?? -1)")
        log.notice("drive cosmetics header badges \(me?.badges.count ?? -1) paint \(me?.paint?.name ?? "-", privacy: .public) layers \(me?.paint?.layers.count ?? -1) member \(me?.streamnook?.label ?? "-", privacy: .public) picture \(me?.picture != nil)")
    }

    private static func findWashes(_ model: AppModel) async {
        log.notice("drive wash catalog \(model.cosmetics.atmospheres.map(\.id).joined(separator: ","), privacy: .public)")
        guard let session = model.chat.activeSession else {
            log.error("drive wash: no chat")
            return
        }
        var seen = Set<String>()
        for message in session.pipeline.recentMessages(500) {
            guard let view = message.metadata.view, !seen.contains(message.userID) else { continue }
            let member = view.badges.first { $0.provider == "streamnook" }
            guard view.atmosphere != nil || view.cologne != nil || member != nil else { continue }
            seen.insert(message.userID)
            log.notice("drive wash \(message.username, privacy: .public) atmosphere \(view.atmosphere ?? "-", privacy: .public) cologne \(view.cologne != nil) member badge \(member?.title ?? "-", privacy: .public)")
        }
        log.notice("drive wash members on screen \(seen.count)")
    }

    /// How many rows wear 7TV and StreamNook cosmetics, history and live apart.
    private static func countDecorated(_ model: AppModel) {
        guard let session = model.chat.activeSession else {
            log.error("drive paints: no chat")
            return
        }
        var tally: [Bool: (rows: Int, paint: Int, sevenTv: Int, member: Int)] = [:]
        for message in session.pipeline.recentMessages(500) where !message.isSystem {
            let history = message.metadata.fromBackfill
            var count = tally[history] ?? (0, 0, 0, 0)
            count.rows += 1
            if let view = message.metadata.view {
                if view.paint != nil { count.paint += 1 }
                if view.badges.contains(where: { $0.provider == "7tv" }) { count.sevenTv += 1 }
                if view.badges.contains(where: { $0.provider == "streamnook" }) { count.member += 1 }
            }
            tally[history] = count
        }
        for (history, count) in tally {
            log.notice("drive paints \(history ? "history" : "live", privacy: .public) rows \(count.rows) paint \(count.paint) 7tv badge \(count.sevenTv) member badge \(count.member)")
        }
    }
}
#endif
