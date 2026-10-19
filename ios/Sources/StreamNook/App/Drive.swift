#if DEBUG
import Foundation
import MediaPlayer
import SNKit
import SwiftUI

/// Steps named in the SN_DRIVE launch variable, run one after another once the core has
/// started, to bring the app to a given state from outside. Steps are separated by ";":
///
/// - `fake`, `fake:signedin` run the build on FakeCore's fixtures, signed out or signed in (read
///   when the core is made, so put them anywhere in the list)
/// - `setup` shows the first-run wizard (without it a drive run skips the wizard for the session);
///   `setup:PAGE` opens it on `welcome`, `accounts`, `drops`, `kick`, `notifications` or `done`;
///   `setupalert:ID` turns an alert type on in its notifications page, as a tap does (asking for
///   permission the first time; with notifications off in iPhone Settings the page shows its
///   refusal line)
/// - `tab:NAME` picks a tab: `following`, `browse`, `rewards`, `you`
/// - `browse` loads the Browse list again and waits for it; `browse:categories`, `browse:live`
///   switch its mode; `search:TEXT` searches it
/// - `open:watch:INDEX` (or `play:INDEX`) opens the watch layer on the Browse list's stream at INDEX;
///   `login:NAME` opens it on a bare login, the way a notification tap or a raid seed does
/// - `nowplaying` logs the lock screen card (title, channel, artwork, live, rate, seek)
/// - `room:LOGIN` opens LOGIN's offline room (or its stream when it is live); `roomvod` plays the
///   room's latest broadcast, `roomback` returns to the room; `roomstate` logs what the room holds
/// - `clip` opens the watched channel's top clip the way a chat link does (the clip sheet)
/// - `channelsheet` opens the watched channel's videos and clips; `vodof:INDEX` plays the watched
///   channel's past broadcast at INDEX (as its Channel sheet does), `clipof:INDEX` its clip;
///   `seek:SECONDS`, `skip:SECONDS`, `rate:SPEED` move a VOD; `replay:on|off` picks the replay or
///   the live chat; `vodstate` logs the VOD and its replay; `clipsheet` opens Create clip (never
///   creates one); `open:myclips` pushes You > Your clips
/// - `corelog:TEXT` copies the core log's lines holding TEXT (`_` for spaces, the last 12) into
///   the app log, where a drive run reads them
/// - `open:category:INDEX` pushes the category of the Browse list's stream at INDEX
/// - `open:settings:NAME` pushes a settings section (`theme`, `support`, ...); `open:cosmetics`
/// - `mini`, `full` shrink or expand the watch layer; `back` or `close` closes it
/// - `rotate:landscape`, `rotate:portrait` turn the screen; `chat:on`, `chat:off` the landscape chat;
///   `controls` shows the player's controls
/// - `menu:card:INDEX` lists the Browse card's long-press menu in a sheet (a system menu cannot
///   be opened from code)
/// - `try:PATH=JSON` writes one setting for a look (the value it replaced is kept); `restore`
///   writes every replaced value back exactly as it was (absent stays absent); `ladd:PATH=TEXT`
///   and `lremove:PATH=TEXT` add or remove one entry of a settings list; `reset:PATH` is the reset
///   arrow; `ssearch:TEXT` types into the You tab's settings search (`_` for spaces)
/// - `scroll:SECTION` scrolls the open settings page to a section by title (`_` for spaces);
///   `scroll:SECTION@end` shows the section's last rows
/// - `glass:PERCENT`, `motion:full|reduced|off` preview a look without saving it; `theme:ID` (or
///   `ptheme:ID`) paints a built-in theme, its light included, without saving it; `theme:` paints
///   the saved one again
/// - `toast:TEXT` an error toast; `undo:TEXT` a toast with an Undo action
/// - `view:cards`, `view:list` the stream lists' card or row view (saved, like the menu)
/// - `resume` sends the app to the background and back as the scene phase does; `snapshot` logs
///   what the home snapshot carries (groups, drops stamps, hype trains); `category:drops` pushes
///   the category of the first drop campaign the core holds
/// - `wait:SECONDS`, `pause` (pauses the player), `signin` (opens the Twitch sign-in sheet)
/// - `fakelogin`, `fakelogout` sign FakeCore's account in or out the way the core announces it;
///   `signinstate` logs who the app thinks is signed in
/// - `quality:NAME` the quality the next stream asks for (`audio_only`, a variant, `best`)
/// - `qsheet` opens the quality sheet; `pickq:NAME` switches the playing stream's quality;
///   `restart` restarts it; `toggle` plays or pauses; `mute` mutes or unmutes; `state` logs
///   the player's state (paused, muted, quality, the offered qualities, viewers, PiP)
/// - `edge` shows how far behind the live edge the player sits
/// - `route:NAME`, `offset:SECONDS`, `preserve`, `catchup:MS`, `hold:PARTS`: the player's
///   low-latency knobs for the next stream (see `Watch`)
/// - the chat steps in `ChatDrive` (`chatlab`, `chatperf`, ...) and the profile and cosmetics
///   steps in `CosmeticsDrive` (`profile:LOGIN`, `cosmetics:dump`, `wash:find`, ...)
/// - the poll and prediction steps in `LiveVotesDrive` (`pollfake:open`, `predfake:locked`,
///   `predbet:yes:250`, `predconfirm`, `liveVotesLog`, ...)
/// - the Kick steps in `KickDrive` (`platform:kick`, `kickplay:INDEX`, `kicklogin:SLUG`, ...)
/// - the Mentions steps in `MentionsDrive` (`mentions`, `mentions:fake`, `mentionnotify`, `mentioncard`, ...)
/// - `shot` marks the moment a screenshot is due (the host takes it) and holds still for five
///   seconds so it lands on a settled screen
@MainActor
enum Drive {
    private static var steps: [String] {
        (ProcessInfo.processInfo.environment["SN_DRIVE"] ?? "")
            .split(separator: ";").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    /// A drive run lands on the tabs unless it asks for the wizard.
    static var skipsSetup: Bool { !steps.isEmpty && !steps.contains { $0.hasPrefix("setup") } }

    /// A run that shows the controls keeps them up, so a screenshot taken seconds later still
    /// has them.
    static var pinsControls: Bool { steps.contains("controls") }

    /// The edge readout shows on drive runs that measure latency.
    static var showsEdge: Bool { steps.contains {
        ["edge", "route", "offset", "preserve", "catchup", "hold"].contains(split($0).0)
    } }

    static func run(model: AppModel) async {
        let steps = steps
        guard !steps.isEmpty else { return }
        guard model.started else {
            log.error("drive skipped: the core did not start")
            return
        }
        log.notice("drive steps \(steps.joined(separator: ";"), privacy: .public)")
        for step in steps {
            let (name, value) = split(step)
            switch name {
            case "fake": break
            case "setup":
                model.showSetup()
                if let page = SetupWizard.Page.named(value) { model.nav.setupPage = page }
            case "setupalert": model.nav.setupAlert = value
            case "tab": if let tab = MobileTab(rawValue: value) { model.nav.tab = tab }
            case "browse":
                switch value {
                case "categories": model.browseState.mode = .categories
                case "live": model.browseState.mode = .live
                default: await model.browse()
                }
            case "search": model.browseState.query = value
            case "open": await open(value, model: model)
            case "play": await open("watch:\(value)", model: model)
            case "login":
                model.startWatching(TwitchStream(id: "", userLogin: value, userName: value, title: "", viewerCount: 0,
                                                 gameName: ""))
            case "corelog": coreLog(value.replacingOccurrences(of: "_", with: " "))
            case "loginof":
                // A Browse stream opened by its bare login, as a notification tap would.
                guard await waitForStreams(model), let index = Int(value), model.streams.indices.contains(index) else { continue }
                let login = model.streams[index].userLogin
                model.startWatching(TwitchStream(id: "", userLogin: login, userName: login, title: "", viewerCount: 0, gameName: ""))
            case "clip": await openTopClip(model)
            case "room":
                // An offline card for LOGIN (a live channel starts its stream instead).
                model.startWatching(TwitchStream(id: "", userLogin: value, userName: value, title: "", viewerCount: 0,
                                                 gameName: "", isLive: false))
            case "roomof":
                // A live Browse channel opened as a stale offline card: the core starts its stream.
                guard await waitForStreams(model), let index = Int(value), model.streams.indices.contains(index) else { continue }
                let login = model.streams[index].userLogin
                model.startWatching(TwitchStream(id: "", userLogin: login, userName: login, title: "", viewerCount: 0,
                                                 gameName: "", isLive: false))
            case "roomvod": model.watch.playRoomVod()
            case "roomback": model.watch.leaveRoomVod()
            case "channelsheet": model.channelSheet = model.watch.stream ?? model.nav.playing
            case "vodof": await openMedia(model, clips: false, index: Int(value) ?? 0)
            case "clipof": await openMedia(model, clips: true, index: Int(value) ?? 0)
            case "seek": model.watch.seekVod(to: Double(value) ?? 0)
            case "skip": model.watch.skipVod(by: Double(value) ?? 10)
            case "rate": model.watch.setVodRate(Float(value) ?? 1)
            case "replay": model.replayShown = value != "off"
            case "vodstate":
                let w = model.watch
                let vod = w.playingVod
                log.notice("drive vod \(vod?.videoID ?? "-", privacy: .public) play \(vod?.playID ?? 0) status \(vod?.status ?? "-", privacy: .public) at \(Int(w.vodPosition)) of \(Int(w.vodDuration)) rate \(w.vodRate) muted \(vod?.mutedSegments.count ?? 0) chapters \(vod?.chapters.count ?? 0) title \(w.vodTitle ?? "-", privacy: .public) replay \(model.replay?.rowCount ?? -1) shown \(model.replayShown) paused \(w.paused)")
            case "clipsheet":
                if let s = model.watch.stream { model.createClip(login: s.userLogin, channelID: s.userID ?? "") }
            case "roomstate":
                let room = model.watch.room
                log.notice("drive room \(room?.login ?? "-", privacy: .public) last \(room?.lastLiveAt ?? "-", privacy: .public) title \(room?.lastTitle ?? "-", privacy: .public) vod \(room?.latestVod?.id ?? "-", privacy: .public) next \(room?.nextStream?.startAt ?? "-", privacy: .public) waiting \(room?.chatters ?? -1) vodPlaying \(model.watch.roomVod) playing \(model.watch.stream?.userLogin ?? "-", privacy: .public)")
            case "nowplaying":
                let info = MPNowPlayingInfoCenter.default().nowPlayingInfo ?? [:]
                let rate = info[MPNowPlayingInfoPropertyPlaybackRate] as? Double ?? -1
                let live = info[MPNowPlayingInfoPropertyIsLiveStream] as? Bool ?? false
                log.notice("drive nowplaying title \(info[MPMediaItemPropertyTitle] as? String ?? "-", privacy: .public) artist \(info[MPMediaItemPropertyArtist] as? String ?? "-", privacy: .public) artwork \(info[MPMediaItemPropertyArtwork] != nil) live \(live) rate \(rate) seek \(MPRemoteCommandCenter.shared().changePlaybackPositionCommand.isEnabled)")
            case "mini": withAnimation(SNMotion.settleBack) { model.nav.playerMode = .mini }
            case "full": withAnimation(SNMotion.settleBack) { model.nav.playerMode = .full }
            case "back", "close": model.stopWatching()
            case "rotate": Orientation.turn(landscape: value == "landscape")
            case "controls": model.nav.controlsPing += 1
            case "qsheet": model.nav.qualityPing += 1
            case "pickq": await model.changeQuality(value.isEmpty ? "best" : value)
            case "restart": await model.restartStream()
            case "toggle": model.watch.togglePlay()
            case "mute": model.watch.toggleMute()
            case "row":
                let row = model.nav.playing
                log.notice("drive row \(row?.userLogin ?? "-", privacy: .public) id \(row?.userID ?? "-", privacy: .public) name \(row?.userName ?? "-", privacy: .public) title \(row?.title ?? "-", privacy: .public) game \(row?.gameName ?? "-", privacy: .public) started \(row?.startedAt ?? "-", privacy: .public) avatar \(row?.profileImageURL != nil) mode \(model.nav.playerMode == .mini ? "mini" : "full", privacy: .public)")
            case "state": log.notice("drive state paused \(model.watch.paused) muted \(model.watch.muted) quality \(model.watch.quality ?? "-", privacy: .public) available \(model.watch.available.joined(separator: ","), privacy: .public) viewers \(model.watch.live?.viewerCount ?? -1) pip \(model.watch.pip.active)")
            case "chat": model.nav.landscapeChat = value != "off"
            case "menu":
                let parts = value.split(separator: ":")
                let index = parts.count > 1 ? Int(parts[1]) ?? 0 : 0
                guard await waitForStreams(model), model.streams.indices.contains(index) else { continue }
                let stream = model.streams[index]
                model.nav.menuSample = MenuSample(title: stream.userName, items: StreamMenu.items(stream, category: true))
            case "try": await trySetting(value, model: model)
            case "restore": await restoreSettings(model: model)
            case "reset": await model.settingsStore.reset([value])
            case "ssearch": SettingsSearch.shared.query = value.replacingOccurrences(of: "_", with: " ")
            case "ladd", "lremove":
                guard let eq = value.firstIndex(of: "=") else { continue }
                let path = String(value[..<eq]), item = String(value[value.index(after: eq)...])
                do {
                    _ = try await model.settingsStore.edit(path, name == "ladd" ? .add(item) : .removeValue(item))
                    log.notice("drive \(name, privacy: .public) \(path, privacy: .public) ok")
                } catch {
                    log.notice("drive \(name, privacy: .public) refused: \(String(describing: error), privacy: .public)")
                }
            case "scroll": SettingsAnchor.shared.target = value.replacingOccurrences(of: "_", with: " ")
            case "glass": model.look.preview(glassiness: (Double(value) ?? 100) / 100)
            case "theme": await previewTheme(value, model: model)
            case "motion": model.look.preview(motion: MotionMode(setting: value))
            case "ptheme": await previewTheme(value, model: model)
            case "toast": model.toasts.show(value.isEmpty ? "Could not load your Kick follows. Try again in a moment." : value)
            case "undo": model.toasts.show(value.isEmpty ? "Message deleted" : value, .info, action: "Undo") {}
            case "wait": try? await Task.sleep(for: .seconds(Double(value) ?? 1))
            case "pause": model.watch.pause()
            case "edge": break
            case "route": model.watch.route = PlayRoute(rawValue: value) ?? .plain
            case "offset": model.watch.offsetFromLive = Double(value)
            case "preserve": model.watch.preservesOffset = true
            case "catchup": model.watch.catchUpTargetMS = Int(value)
            case "hold": model.watch.holdParts = Int(value)
            case "signin": await model.signIn.begin(core: model.core)
            case "fakelogin", "fakelogout":
                guard let fake = model.core as? FakeCore else {
                    log.error("drive: \(name, privacy: .public) needs FakeCore")
                    continue
                }
                fake.simulateTwitchSignIn(name == "fakelogin")
            case "signinstate":
                await model.signIn.refresh()
                log.notice("sign-in state: signed in \(model.signedIn) user \(model.signIn.user?.login ?? "none", privacy: .public) busy \(model.signIn.busy)")
            case "quality": model.watch.requestedQuality = value.isEmpty ? "best" : value
            case "view": UserDefaults.standard.set(StreamView.stored(value).rawValue, forKey: StreamView.key)            case "resume":
                model.lifecycle("background")
                try? await Task.sleep(for: .seconds(1))
                model.lifecycle("foreground")
            case "snapshot": await logSnapshot(model)
            case "category" where value == "drops": await openDropsCategory(model)
            case "together": await searchTogether(model)
            case "shot":
                log.notice("drive shot")
                try? await Task.sleep(for: .seconds(5))
            default:
                if !(await ChatDrive.run(name, value, model: model)),
                   !(await LiveVotesDrive.run(name, value, model: model)),
                   !(await CosmeticsDrive.run(name, value, model: model)),
                   !(await RewardsDrive.run(name, value, model: model)),
                   !(await KickDrive.run(name, value, model: model)),
                   !(await MentionsDrive.run(name, value, model: model)) {
                    log.error("drive: unknown step \(name, privacy: .public)")
                }
            }
            try? await Task.sleep(for: .milliseconds(400))
        }
        log.notice("drive done")
    }

    /// The watched channel's most viewed clip, opened as a tapped chat link.
    /// Plays the watched channel's video (or clip) at `index` from its Channel sheet's first page.
    private static func openMedia(_ model: AppModel, clips: Bool, index: Int) async {
        guard let channel = model.watch.stream ?? model.nav.playing, let id = channel.twitchID else {
            log.notice("drive media: nothing watched")
            return
        }
        let shelf: MediaShelf = clips ? .clips(.all) : .videos(.archive, .recent)
        guard let page = try? await model.core.invoke("channel_media", args: shelf.args(channelID: id, cursor: nil),
                                                       as: MediaPage.self),
              page.rows.indices.contains(index) else {
            log.notice("drive media: no row \(index)")
            return
        }
        let row = page.rows[index]
        log.notice("drive media \(row.kind, privacy: .public) \(row.id, privacy: .public) \(row.metaLine, privacy: .public) | \(row.countLine, privacy: .public) progress \(row.progress ?? -1)")
        if clips {
            if let url = URL(string: row.url) { model.clip = ClipLink(url: url) }
        } else {
            model.openVod(videoID: row.id, title: row.title, thumbnail: row.thumbnailUrl, channel: channel)
        }
    }

    private static func openTopClip(_ model: AppModel) async {
        struct Clip: Decodable { let url: String }
        guard let id = model.watch.stream?.userID,
              let json = try? await model.core.invoke("get_clips_by_broadcaster",
                                                      args: ["broadcasterId": id, "limit": 1, "cursor": nil, "period": "all"]),
              let page = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [Any],
              let first = (page.first as? [[String: Any]])?.first, let raw = first["url"] as? String,
              let url = URL(string: raw) else {
            log.error("drive clip: no clip for the watched channel")
            return
        }
        log.notice("drive clip \(raw, privacy: .public)")
        LinkRouter.open(url)
    }

    /// The core log's last lines holding `text`, copied into the app log. The core writes its
    /// `.log` files somewhere under the app's caches and data folders.
    private static func coreLog(_ text: String) {
        guard let dirs = try? CoreHost.dirs() else { return }
        let files = FileManager.default
        var logs: [String] = []
        for root in [dirs.cache, dirs.data, dirs.logs] {
            let found = files.enumerator(atPath: root)?.compactMap { $0 as? String }.filter { $0.hasSuffix(".log") } ?? []
            logs += found.map { root + "/" + $0 }
        }
        if logs.isEmpty { log.notice("corelog: no .log files under \(dirs.cache, privacy: .public)") }
        for path in Set(logs).sorted() {
            guard let body = try? String(contentsOfFile: path, encoding: .utf8) else { continue }
            let name = (path as NSString).lastPathComponent
            for line in body.split(separator: "\n").filter({ $0.contains(text) }).suffix(12) {
                log.notice("corelog \(name, privacy: .public): \(String(line.suffix(400)), privacy: .public)")
            }
        }
    }

    /// Values `try` replaced, by path, as the raw settings file held them (nil = absent).
    private static var replaced: [(String, Any?)] = []

    private static func trySetting(_ value: String, model: AppModel) async {
        guard let eq = value.firstIndex(of: "=") else { return }
        let path = String(value[..<eq])
        let json = String(value[value.index(after: eq)...])
        let raw = (try? await model.core.invoke("load_settings", args: [:]))
            .flatMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] } ?? [:]
        replaced.append((path, SettingsPath.value(in: raw, path)))
        // JSON when it parses (numbers, booleans), else the text itself.
        let next = (try? JSONSerialization.jsonObject(with: Data(json.utf8), options: [.fragmentsAllowed])) ?? json
        await model.settingsStore.set(path, next)
        log.notice("drive try \(path, privacy: .public) = \(json, privacy: .public)")
    }

    /// Paints a built-in theme from the core's catalog, palette and light, without saving it; an
    /// empty id paints the saved theme again.
    private static func previewTheme(_ id: String, model: AppModel) async {
        guard !id.isEmpty else { await model.loadTheme(); return }
        let raw = (try? await model.core.invoke("get_theme_catalog", args: [:]))
            .flatMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] } ?? [:]
        guard let theme = (raw["themes"] as? [[String: Any]])?.first(where: { $0["id"] as? String == id }),
              let tokens = theme["palette"] as? [String: Any] else {
            log.error("drive theme: no built-in theme \(id, privacy: .public)")
            return
        }
        model.look.apply(palette: Palette(tokens: tokens), themeID: id,
                         light: (theme["light"] as? [String: Any]).flatMap(ThemeLight.init(json:)))
        log.notice("drive theme \(id, privacy: .public) light \(model.look.light != nil)")
    }

    private static func restoreSettings(model: AppModel) async {
        for (path, old) in replaced.reversed() {
            await model.settingsStore.set(path, old)
            log.notice("drive restore \(path, privacy: .public)")
        }
        replaced = []
    }

    private static func open(_ value: String, model: AppModel) async {
        let (what, rest) = split(value)
        switch what {
        case "watch", "category":
            let index = Int(rest) ?? 0
            guard await waitForStreams(model), model.streams.indices.contains(index) else {
                log.error("drive open: no stream at \(index)")
                return
            }
            let stream = model.streams[index]
            if what == "watch" {
                model.startWatching(stream)
            } else if let id = stream.gameID, !id.isEmpty {
                model.nav.tab = .browse
                model.nav.push(.category(TwitchCategory(id: id, name: stream.gameName)))
            }
        case "settings":
            guard let section = SettingsSection.allCases.first(where: { $0.rawValue.lowercased() == rest.lowercased() }) else { return }
            model.nav.tab = .you
            model.nav.push(.settings(section))
        case "cosmetics":
            model.nav.tab = .you
            model.nav.push(.cosmetics)
        case "myclips":
            model.nav.tab = .you
            model.nav.push(.myClips)
        default:
            log.error("drive open: unknown \(what, privacy: .public)")
        }
    }

    /// What the snapshot carries for the cards: groups, drops stamps and trains.
    private static func logSnapshot(_ model: AppModel) async {
        guard let lists = try? await model.core.invoke("get_home_snapshot", args: [:], as: HomeLists.self) else {
            log.error("drive snapshot: no snapshot")
            return
        }
        let rows = lists.followedLive + lists.recommended
        let drops = rows.filter { $0.dropsEarnable == true }.map(\.userLogin)
        let groups = lists.collaborations.values.map(\.label) + lists.sharedChats.values.map(\.label)
        log.notice("drive snapshot followed \(lists.followedLive.count) recommended \(lists.recommended.count) trains \(lists.hypeTrains.filter(\.isActive).count) together \(lists.collaborations.count) shared chats \(lists.sharedChats.count) drops \(drops.joined(separator: ","), privacy: .public)")
        for label in groups.prefix(6) { log.notice("drive snapshot group: \(label, privacy: .public)") }
        // Campaigns that count in several categories, the ones that put DROPS on unexpected games.
        if let json = try? await model.core.invoke("get_home_snapshot", args: [:]),
           let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
           let campaigns = object["drops_campaigns"] as? [[String: Any]] {
            for c in campaigns {
                let ids = c["category_ids"] as? [String] ?? []
                guard !ids.isEmpty else { continue }
                let restricted = (c["is_acl_based"] as? Bool ?? false) || !((c["allowed_channels"] as? [Any] ?? []).isEmpty)
                log.notice("drive snapshot campaign \(c["name"] as? String ?? "?", privacy: .public) game \(c["game_name"] as? String ?? "?", privacy: .public) in \(ids.count) categories, just chatting \(ids.contains("509658")) restricted \(restricted)")
            }
        }
    }

    /// Back on Browse's root, searches the channel of the first Shared Viewership group (or Shared
    /// Chat session) the snapshot holds, so its card shows the tag.
    /// Groups come and go, so it walks the top categories (each reports its streams, which brings
    /// their groups into the snapshot) until one has a group.
    private static func searchTogether(_ model: AppModel) async {
        let games = (try? await model.core.invoke("get_top_games_paginated", args: ["cursor": nil, "limit": 9],
                                                  as: Page<TwitchCategory>.self).items) ?? []
        for attempt in 0..<(games.count + 1) * 8 {
            if attempt % 8 == 0, attempt / 8 < games.count {
                model.nav.tab = .browse
                model.nav.paths[.browse] = [.category(games[attempt / 8])]
            }
            if let lists = try? await model.core.invoke("get_home_snapshot", args: [:], as: HomeLists.self),
               let group = (lists.collaborations.values.first ?? lists.sharedChats.values.first),
               let me = group.members.first(where: \.isSelf) {
                log.notice("drive together: \(me.login, privacy: .public), \(group.label, privacy: .public)")
                model.nav.paths[.browse] = []
                model.nav.tab = .browse
                model.browseState.mode = .live
                model.browseState.query = me.login
                return
            }
            try? await Task.sleep(for: .seconds(1))
        }
        log.error("drive together: no group in the top \(games.count) categories")
    }

    /// Pushes the category of the first drop campaign the snapshot holds that names a category.
    private static func openDropsCategory(_ model: AppModel) async {
        for _ in 0..<60 {
            if let json = try? await model.core.invoke("get_home_snapshot", args: [:]),
               let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
               let campaigns = object["drops_campaigns"] as? [[String: Any]],
               let first = campaigns.first(where: { ($0["has_category"] as? Bool ?? true) && !($0["game_id"] as? String ?? "").isEmpty }),
               let id = first["game_id"] as? String, let name = first["game_name"] as? String {
                log.notice("drive drops category \(name, privacy: .public) of \(campaigns.count) campaigns")
                model.nav.tab = .browse
                model.nav.push(.category(TwitchCategory(id: id, name: name)))
                return
            }
            try? await Task.sleep(for: .seconds(1))
        }
        log.error("drive drops category: no campaigns in 60 s")
    }

    /// Waits up to ten seconds for the Browse list, loading it if nobody has.
    private static func waitForStreams(_ model: AppModel) async -> Bool {
        if model.streams.isEmpty && !model.home.recommendedLoaded { await model.browse() }
        for _ in 0..<40 where model.streams.isEmpty { try? await Task.sleep(for: .milliseconds(250)) }
        return !model.streams.isEmpty
    }

    private static func split(_ step: String) -> (String, String) {
        guard let colon = step.firstIndex(of: ":") else { return (step, "") }
        return (String(step[..<colon]), String(step[step.index(after: colon)...]))
    }
}

extension SetupWizard.Page {
    static func named(_ name: String) -> Self? {
        switch name {
        case "welcome": .welcome
        case "accounts": .accounts
        case "drops": .drops
        case "kick": .kick
        case "notifications": .notifications
        case "done": .done
        default: nil
        }
    }
}
#endif
