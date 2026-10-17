#if DEBUG
import Foundation
import SNKit

/// Drive steps for Kick (see `Drive`):
///
/// - `platform:kick`, `platform:twitch` pick Browse's platform (saved, like the switch)
/// - `kickdir` loads Kick's live directory and logs its first rows; `kickplay:INDEX` opens the
///   directory's stream at INDEX; `kicklogin:SLUG` opens a bare Kick slug
/// - `kickcats` loads Kick's categories and logs them; `kickcat:INDEX` filters the directory to one
/// - `kicksearch:SLUG` runs Kick's exact channel search and logs the answer
/// - `kickstate` logs the Kick account (signed in, name, follows, the row's words)
/// - `kicksignin` starts "Sign in with Kick" (the sheet then needs a person)
/// - `kickfollowing` logs the Following tab's Kick rows and the platforms you follow on
/// - `kickchat` logs the room on screen (platform, rows, rows Rust laid out, badges with art)
@MainActor
enum KickDrive {
    static func run(_ name: String, _ value: String, model: AppModel) async -> Bool {
        let kick = model.browseState.kick
        switch name {
        case "platform":
            UserDefaults.standard.set(value == "kick" ? "kick" : "twitch", forKey: "sn-browse-platform")
            model.nav.tab = .browse
        case "kickdir":
            await kick.loadDirectory(model.core)
            let rows = kick.streams.prefix(5).map { "\($0.userLogin)(\($0.viewerCount))" }.joined(separator: ",")
            log.notice("drive kickdir \(kick.streams.count) streams: \(rows, privacy: .public)")
        case "kickplay":
            if kick.streams.isEmpty { await kick.loadDirectory(model.core) }
            let index = Int(value) ?? 0
            guard kick.streams.indices.contains(index) else {
                log.error("drive kickplay: no Kick stream at \(index)")
                return true
            }
            let stream = kick.streams[index]
            log.notice("drive kickplay \(stream.userLogin, privacy: .public) \(stream.viewerCount)")
            model.startWatching(stream)
        case "kicklogin":
            model.startWatching(TwitchStream(id: "", userLogin: value.lowercased(), userName: value, title: "", viewerCount: 0,
                                             gameName: "", provider: "kick").asKick)
        case "kickcats":
            await kick.loadCategories(model.core)
            let names = kick.categories.prefix(6).map(\.name).joined(separator: ",")
            log.notice("drive kickcats \(kick.categories.count): \(names, privacy: .public)")
        case "kickcat":
            if kick.categories.isEmpty { await kick.loadCategories(model.core) }
            let index = Int(value) ?? 0
            guard kick.categories.indices.contains(index) else { return true }
            model.browseState.query = ""
            kick.category = kick.categories[index]
            model.browseState.mode = .live
        case "kicksearch":
            await kick.search(value, core: model.core, immediate: true)
            let rows = (kick.results ?? []).map { "\($0.userLogin) live \($0.isLive ?? true)" }.joined(separator: ",")
            log.notice("drive kicksearch \(value, privacy: .public): \(rows, privacy: .public)")
        case "kickstate":
            await model.kick.refresh()
            let k = model.kick
            let raw = (try? await model.core.invoke("get_provider_follows", args: [:])) ?? "failed"
            log.notice("drive kickfollows raw \(String(raw.prefix(600)), privacy: .public)")
            log.notice("drive kickstate connected \(k.connected) name \(k.name ?? "-", privacy: .public) follows \(k.follows) busy \(k.busy) detail \(k.detail, privacy: .public)")
        case "kicksignin":
            Task { await model.kick.connect() }
        case "kickfollowing":
            let lists = model.home.lists
            let raw = (try? await model.core.invoke("get_provider_follows", args: [:])) ?? "failed"
            log.notice("drive kickfollows raw \(String(raw.prefix(600)), privacy: .public)")
            let rows = lists.followingLive.filter { !$0.isTwitch }.map { "\($0.userLogin)(\($0.viewerCount))" }
            log.notice("drive kickfollowing platforms \(lists.following.platforms.joined(separator: ","), privacy: .public) kick live \(rows.joined(separator: ","), privacy: .public) all \(lists.followingLive.count)")
        case "kickchat":
            guard let session = model.chat.activeSession else {
                log.notice("drive kickchat: no room on screen")
                return true
            }
            let rows = session.pipeline.recentMessages(40)
            let laidOut = rows.filter { $0.metadata.view != nil }.count
            let badges = rows.compactMap { $0.metadata.view?.badges.count }.reduce(0, +)
            log.notice("drive kickchat \(session.channel, privacy: .public) provider \(session.provider, privacy: .public) rows \(session.rowCount) recent \(rows.count) laid out \(laidOut) badges \(badges)")
        default:
            return false
        }
        return true
    }
}
#endif
