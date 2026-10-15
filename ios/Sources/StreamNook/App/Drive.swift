#if DEBUG
import Foundation
import SNKit
import SwiftUI

/// Steps named in the SN_DRIVE launch variable, run one after another once the core has
/// started, to bring the app to a given state from outside. Steps are separated by ";":
///
/// - `fake`, `fake:signedin` run the build on FakeCore's fixtures, signed out or signed in (read
///   when the core is made, so put them anywhere in the list)
/// - `setup` shows the first-run wizard (without it a drive run skips the wizard for the session);
///   `setup:PAGE` opens it on `welcome`, `accounts`, `drops`, `notifications` or `done`
/// - `tab:NAME` picks a tab: `following`, `browse`, `rewards`, `you`
/// - `browse` loads the Browse list again and waits for it; `browse:categories`, `browse:live`
///   switch its mode; `search:TEXT` searches it
/// - `open:watch:INDEX` (or `play:INDEX`) opens the watch layer on the Browse list's stream at INDEX
/// - `open:category:INDEX` pushes the category of the Browse list's stream at INDEX
/// - `open:settings:NAME` pushes a settings section (`theme`, `support`, ...); `open:cosmetics`
/// - `mini`, `full` shrink or expand the watch layer; `back` or `close` closes it
/// - `rotate:landscape`, `rotate:portrait` turn the screen; `chat:on`, `chat:off` the landscape chat;
///   `controls` shows the player's controls
/// - `menu:card:INDEX` lists the Browse card's long-press menu in a sheet (a system menu cannot
///   be opened from code)
/// - `glass:PERCENT`, `motion:full|reduced|off` preview a look without saving it
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
/// - the chat steps in `ChatDrive` (`chatlab`, `chatperf`, ...)
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
            case "state": log.notice("drive state paused \(model.watch.paused) muted \(model.watch.muted) quality \(model.watch.quality ?? "-", privacy: .public) available \(model.watch.available.joined(separator: ","), privacy: .public) viewers \(model.watch.live?.viewerCount ?? -1) pip \(model.watch.pip.active)")
            case "chat": model.nav.landscapeChat = value != "off"
            case "menu":
                let parts = value.split(separator: ":")
                let index = parts.count > 1 ? Int(parts[1]) ?? 0 : 0
                guard await waitForStreams(model), model.streams.indices.contains(index) else { continue }
                let stream = model.streams[index]
                model.nav.menuSample = MenuSample(title: stream.userName, items: StreamMenu.items(stream, category: true))
            case "glass": model.look.preview(glassiness: (Double(value) ?? 100) / 100)
            case "motion": model.look.preview(motion: MotionMode(setting: value))
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
            case "view": UserDefaults.standard.set(StreamView.stored(value).rawValue, forKey: StreamView.key)
            case "resume":
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
                if !(await ChatDrive.run(name, value, model: model)) {
                    log.error("drive: unknown step \(name, privacy: .public)")
                }
            }
            try? await Task.sleep(for: .milliseconds(400))
        }
        log.notice("drive done")
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
        case "notifications": .notifications
        case "done": .done
        default: nil
        }
    }
}
#endif
