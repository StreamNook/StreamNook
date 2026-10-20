import SNKit
import SwiftUI
import UserNotifications

// Settings > Ad-free, Notifications, Cache and Support.

// MARK: Ad-free

/// Ad-free playback: the master switch, how the playing stream is served, and the relays.
struct AdFreePanel: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var relaysOpen = false

    var body: some View {
        let on = store.bool("video_player.ad_bypass_enabled", true)
        SettingsBlock(title: "Ad-free") {
            ToggleRow(title: "Ad-free playback", path: "video_player.ad_bypass_enabled", fallback: true,
                      detail: on ? "Routes the playlist through a public relay and removes ad segments before the player sees them."
                                 : "Streams play exactly as Twitch serves them.")
        }
        if on {
            let source = Self.describe(model.watch.lastStart)
            SettingsBlock(title: "This stream") {
                SettingRow(title: source.label, detail: source.detail, icon: "play.tv") { EmptyView() }
            }
            SettingsBlock(title: "Good to know") {
                Label {
                    Text("A Turbo subscription or a sub to the channel is already ad-free through Twitch itself, and gives you the highest quality tiers. Those streams skip the relay.")
                        .font(.sn(12.5)).foregroundStyle(SN.textSecondary)
                } icon: {
                    Image(systemName: "crown")
                }
                Label {
                    Text("Low Latency stays off while this is on. Both want to build the playlist the player reads, and only one of them can.")
                        .font(.sn(12.5)).foregroundStyle(SN.textSecondary)
                } icon: {
                    Image(systemName: "bolt.slash")
                }
            }
            // Still behind a disclosure: the row says which list is in use, and the box to
            // replace it only opens on purpose.
            SettingsBlock(title: "Relays") {
                let own = !store.string("video_player.ad_bypass_proxies").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                SettingRow(title: "Relays", detail: own ? "Your own list" : "The bundled list",
                           paths: ["video_player.ad_bypass_proxies"], icon: "point.3.connected.trianglepath.dotted") {
                    Button {
                        withAnimation(SNMotion.run(.easeOut(duration: 0.2))) { relaysOpen.toggle() }
                    } label: {
                        Image(systemName: "chevron.forward").font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(SN.textMuted)
                            .rotationEffect(.degrees(relaysOpen ? 90 : 0))
                            .frame(width: 44, height: 44)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .padding(.vertical, -12)
                    .padding(.trailing, -12)
                    .accessibilityLabel(relaysOpen ? "Hide the relay list" : "Show the relay list")
                } below: {
                    if relaysOpen {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("Leave this empty to use the bundled list, which is raced on every stream so the fastest one that answers wins. Enter your own addresses, one per line, only if you run or trust a specific relay.")
                            Text("Your own list is used on its own. If none of them answer, the stream plays directly from Twitch with ads rather than falling back to the bundled relays.")
                            SettingTextField(path: "video_player.ad_bypass_proxies", placeholder: "https://relay.example.com",
                                             limit: 4000, monospaced: true, multiline: true)
                        }
                        .font(.sn(12.5)).foregroundStyle(SN.textSecondary).fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 10)
                        .transition(.opacity)
                    }
                } nested: {
                    EmptyView()
                }
            }
        }
    }

    static func describe(_ start: StreamStart?) -> (label: String, detail: String) {
        switch start?.mode {
        case "turbo": ("Turbo", "Your Turbo subscription already serves this stream without ads, so no relay is used.")
        case "subscribed": ("Subscribed", "Your subscription to this channel already serves it without ads, so no relay is used.")
        case "proxy": (start?.proxyRegion.map { "Relay (\($0))" } ?? "Relay",
                       "The playlist is coming through a public relay, with ad segments stripped out.")
        case "auth-only": ("Direct", "No relay answered, so this is the stream Twitch serves you directly. Ads can appear.")
        default:
            start?.entitled == true ? ("Ad-free", "This stream is being served without ads.")
                                    : ("Not playing", "Start a stream to see how it is being served.")
        }
    }
}

/// A group label over a phone-only panel's card.
struct GroupLabel: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View { Text(text).snSectionStyle().padding(.bottom, -10) }
}

/// A phone-only panel's master switch: symbol, title, a line that follows the state.
struct MasterCard: View {
    let symbol: String
    let title: String
    let detail: String
    let path: String
    var fallback = true
    /// Runs after the switch is flipped (a permission to ask for).
    var onChange: ((Bool) async -> Void)?

    var body: some View {
        HStack(alignment: .settingTitle, spacing: 12) {
            Image(systemName: symbol).font(.system(size: 17)).foregroundStyle(SN.accent)
                .alignmentGuide(.settingTitle) { $0[VerticalAlignment.center] }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.sn(14.5, 600)).foregroundStyle(SN.textPrimary)
                    .alignmentGuide(.settingTitle) { $0[VerticalAlignment.center] }
                Text(detail).font(.sn(12)).foregroundStyle(SN.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            SettingToggle(path: path, fallback: fallback, label: title, onChange: onChange)
                .alignmentGuide(.settingTitle) { $0[VerticalAlignment.center] }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .snPanel()
    }
}

// MARK: Notifications

/// Settings > Notifications on the iPhone: only what this iPhone sends. "Notify me about" lists
/// the alert types the core says this build sends (`get_alert_types`), the same list and words
/// Android draws; today that is go-lives (pushed, app open or closed; Which channels narrows them),
/// mentions and replies, and whispers (both posted while StreamNook runs). Drops and badges join the list when
/// the core adds the iPhone to them; background checking and Instant alerts are Android's own and
/// stay hidden here (their saved values are kept for the other apps).
struct NotificationsPanel: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var phase
    @State private var status: UNAuthorizationStatus?

    var body: some View {
        let on = store.bool("live_notifications.enabled", true)
        VStack(alignment: .leading, spacing: 16) {
            MasterCard(symbol: "bell", title: "Notifications",
                       detail: on ? "StreamNook can alert you about the categories below." : "All StreamNook notifications are off.",
                       path: "live_notifications.enabled") { turnedOn in
                if turnedOn, model.alertTypes.contains(where: { store.bool($0.setting, true) }) { await ask() }
            }
            if on {
                permissionCard
                AlertTypeSettings(types: model.alertTypes, ask: ask)
                    .padding(.top, 12)
            }
            MentionRuleSettings()
                .padding(.top, 12)
            SettingButton(title: "Send a test notification", symbol: "bell.badge") { Task { await test() } }
        }
        .task { await readStatus() }
        .onChange(of: phase) { _, now in if now == .active { Task { await readStatus() } } }
    }

    @ViewBuilder private var permissionCard: some View {
        switch status {
        case .authorized, .provisional, .ephemeral:
            card(symbol: "bell", tint: SN.accent, title: "System notifications",
                 body: "StreamNook can send notifications to this iPhone.", allow: false, settingsLabel: "Sounds and banners in iPhone Settings")
        case .notDetermined:
            card(symbol: "exclamationmark.triangle.fill", tint: SN.warning, title: "Notifications are not allowed yet",
                 body: "Allow notifications so the alerts below can reach you.", allow: true, settingsLabel: "Open iPhone Settings")
        case .denied:
            card(symbol: "exclamationmark.triangle.fill", tint: SN.warning, title: "Notifications are not allowed yet",
                 body: "Your iPhone will not ask again, so turn notifications on for StreamNook in Settings.", allow: false,
                 settingsLabel: "Open iPhone Settings")
        default:
            EmptyView()
        }
    }

    private func card(symbol: String, tint: Color, title: String, body: String, allow: Bool, settingsLabel: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: symbol).font(.system(size: 17)).foregroundStyle(tint)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.sn(14.5, 600)).foregroundStyle(SN.textPrimary)
                    Text(body).font(.sn(12.5)).foregroundStyle(SN.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
            }
            if allow {
                SettingButton(title: "Allow notifications") {
                    Task {
                        _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
                        await readStatus()
                        await LiveAlerts.sync(model)
                    }
                }
            }
            SettingButton(title: settingsLabel, symbol: "arrow.up.right.square") { MentionAlerts.openSettings() }
        }
        .padding(14)
        .snPanel()
    }

    /// Turning a notification on asks for permission the first time; an answer already given stays.
    private func ask() async {
        await MentionAlerts.askIfUndecided()
        await readStatus()
        await LiveAlerts.sync(model)
    }

    private func readStatus() async {
        status = await MentionAlerts.status()
    }

    private func test() async {
        NotificationRouter.install()
        let content = UNMutableNotificationContent()
        content.title = "StreamNook"
        content.body = "Notifications are working."
        try? await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "sn-test", content: content, trigger: nil))
    }
}

/// Notifications > Notify me about: a switch per alert type the core lists for this iPhone, in its
/// order and with its words. Which channels sits nested under Live channels, shown whether or not
/// that switch is on so it never appears and disappears under the reader (as on Android). Turning
/// a type on asks for permission the first time.
struct AlertTypeSettings: View {
    let types: [AlertType]
    let ask: () async -> Void

    var body: some View {
        if !types.isEmpty {
            SettingsBlock(title: "Notify me about") {
                ForEach(types) { type in
                    if type.id == "live" {
                        ToggleRow(title: type.title, path: type.setting, fallback: true, detail: type.detail,
                                  onChange: { on in if on { await ask() } }, showsNestedWhenOff: true) {
                            WhichChannelsLine()
                        }
                    } else {
                        ToggleRow(title: type.title, path: type.setting, fallback: true, detail: type.detail,
                                  onChange: { on in if on { await ask() } })
                    }
                }
            }
        }
    }
}

/// Live channels > Which channels: every channel followed on Twitch, or all but the ones muted (in
/// the list this opens, or with the bell beside a playing channel's name), in Android's words. The
/// whole line under the name opens the list.
struct WhichChannelsLine: View {
    @Environment(SettingsStore.self) private var store
    @State private var open = false
    private static let path = "live_notifications.muted_live_channels"

    var body: some View {
        let muted = store.strings(Self.path).count
        let summary = muted > 0 ? "Everything you follow except \(muted)" : "Everything you follow"
        NestedLine(title: "Which channels", paths: [Self.path], stacked: true) {
            Button { open = true } label: {
                HStack(spacing: 8) {
                    Text(summary).font(.sn(12)).foregroundStyle(SN.textSecondary).lineLimit(1)
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.right").font(.system(size: 11, weight: .semibold)).foregroundStyle(SN.textMuted)
                }
                // A full touch target that keeps the line its own height beside the name.
                .padding(.vertical, 12)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.vertical, -12)
            .accessibilityLabel("Which channels")
            .accessibilityValue(summary)
        }
        .sheet(isPresented: $open) { NotifiedChannelsSheet() }
    }
}

/// Notifications > Mentions: what counts as a mention (which also decides what collects in
/// Mentions) and the chats that stay quiet. Whether a mention notifies is its Notify me about row.
struct MentionRuleSettings: View {
    var body: some View {
        SettingsBlock(title: "Mentions") {
            // The rules shape Mentions as well as notifications, so they stand on their own.
            SettingRow(title: "Counts as a mention",
                       detail: "Your @name always counts. These count too, in Mentions and for notifications.") {
                EmptyView()
            } nested: {
                NestedSettings {
                    NestedToggle(title: "Name without @", path: "mentions.match_bare_login", fallback: true)
                    NestedToggle(title: "Display name", path: "mentions.match_display_name", fallback: true)
                    NestedToggle(title: "Highlight words", path: "mentions.highlight_words", fallback: true)
                }
            }
            MutedChatsRow()
            // The core tells the server and pulls what it finds into Mentions
            // (services/mention_watch.rs).
            ToggleRow(title: "Mentions while you're away", path: "mentions.away_alerts",
                      detail: "StreamNook's server watches chat in channels you follow for your name and replies, and keeps matches for 7 days.")
        }
    }
}

/// The chats whose mentions never notify or collect in Mentions (they still highlight in chat),
/// each with its remove.
struct MutedChatsRow: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model

    var body: some View {
        let muted = store.strings("mentions.muted_channels")
        let mixed = Set(muted.map(MentionText.platform(ofRoom:))).count > 1
        WideRow(title: "Muted chats",
                detail: muted.isEmpty ? "None. Mute a chat from its row in Mentions."
                                      : "Mentions here never notify you or reach the Mentions list.",
                paths: ["mentions.muted_channels"]) {
            if !muted.isEmpty {
                FlowLayout(spacing: 6) {
                    ForEach(muted, id: \.self) { room in
                        MutedRoomChip(room: room, mark: mixed) {
                            Task { await model.mentions.unmute(room, store: store) }
                        }
                    }
                }
                .padding(.top, 10)
            }
        }
    }
}

/// A muted chat: its name (with its platform's logo where platforms mix) and an x to unmute.
private struct MutedRoomChip: View {
    let room: String
    let mark: Bool
    let remove: () -> Void

    var body: some View {
        let channel = MentionText.channel(ofRoom: room)
        HStack(spacing: 4) {
            if mark { ProviderLogo(provider: MentionText.platform(ofRoom: room), size: 11, tinted: true) }
            Text(channel).font(.sn(12)).foregroundStyle(SN.textPrimary)
            Button(action: remove) {
                Image(systemName: "xmark").font(.system(size: 10, weight: .semibold)).foregroundStyle(SN.textMuted)
                    .frame(width: 28, height: 28).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Unmute \(channel)")
        }
        .padding(.leading, 12)
        .background { Capsule().fill(SN.surface) }
    }
}

/// Every followed channel with a bell to mute or unmute its live alerts.
struct NotifiedChannelsSheet: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var channels: [TwitchStream] = []
    @State private var truncated = false
    @State private var loading = true
    @State private var query = ""

    private struct Answer: Decodable {
        let channels: [TwitchStream]
        let truncated: Bool
    }

    var body: some View {
        let muted = Set(store.strings("live_notifications.muted_live_channels"))
        let shown = query.isEmpty ? channels : channels.filter {
            $0.userName.localizedCaseInsensitiveContains(query) || $0.userLogin.localizedCaseInsensitiveContains(query)
        }
        NavigationStack {
            List {
                Section {
                    Text(muted.isEmpty ? "Alerting for every channel you follow. Turn off any you would rather not hear about."
                                       : "Alerting for every channel you follow except \(muted.count).")
                        .font(.sn(12.5)).foregroundStyle(SN.textSecondary)
                        .listRowBackground(Color.clear)
                }
                Section {
                    if loading && channels.isEmpty {
                        note("Loading your follows…")
                    } else if channels.isEmpty {
                        note("No followed channels found.")
                    } else if shown.isEmpty {
                        note("No channels match that.")
                    }
                    ForEach(shown, id: \.userLogin) { channel in
                        let off = muted.contains(channel.userLogin.lowercased())
                        HStack(spacing: 10) {
                            Avatar(url: channel.profileImageURL, size: 32)
                            Text(channel.userName).font(.sn(14)).foregroundStyle(SN.textPrimary).lineLimit(1)
                            Spacer()
                            Button {
                                Task { _ = try? await store.edit("live_notifications.muted_live_channels", .toggle(channel.userLogin)) }
                            } label: {
                                Image(systemName: off ? "bell.slash" : "bell.fill")
                                    .foregroundStyle(off ? SN.textMuted : SN.accent)
                                    .frame(width: 44, height: 44)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(off ? "Turn on alerts for \(channel.userName)" : "Turn off alerts for \(channel.userName)")
                        }
                        .listRowBackground(SN.backgroundSecondary)
                    }
                    if truncated {
                        Text("Showing the first 2000 channels you follow. Search covers only these.")
                            .font(.sn(12)).foregroundStyle(SN.textMuted).listRowBackground(Color.clear)
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .snSheetLight()
            .background(SN.background)
            .searchable(text: $query, prompt: "Search channels")
            .navigationTitle("Channels to notify me about")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        .presentationDetents([.large])
        .task { await load() }
    }

    private func note(_ text: String) -> some View {
        Text(text).font(.sn(13)).foregroundStyle(SN.textMuted).frame(maxWidth: .infinity).padding(.vertical, 24)
            .listRowBackground(Color.clear)
    }

    private func load() async {
        defer { loading = false }
        do {
            let answer = try await model.core.invoke("get_followed_channels_for_alerts", args: [:], as: Answer.self)
            channels = answer.channels
            truncated = answer.truncated
        } catch {
            log.error("follow list for alerts failed: \(String(describing: error), privacy: .public)")
        }
    }
}

// MARK: Cache

struct CachePanel: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var summary: String?

    var body: some View {
        SettingsBlock(title: "Cache", detail: "Keeps emotes, badges, and channel details on disk so chat loads without fetching them again. Come here to check how much is stored or start fresh.") {
            ToggleRow(title: "Disk cache", path: "cache.enabled", fallback: true,
                      detail: "Keeps emotes and badges on this iPhone so channels load near instantly.",
                      help: "A copy is stored after the first download; off fetches them again on every launch. Uses a little disk space in the app data folder and nothing else. Turning it off does not delete what is already stored; use Clear cache below for that.")
            SliderRow(title: { "Refresh after: \(Int($0)) days" }, path: "cache.expiry_days", range: 1...30, step: 1, fallback: 7,
                      detail: "Anything older than this is fetched again the next time it is needed, so new emotes and badge art show up on their own.",
                      help: "1 to 30 days, 7 by default. A shorter window keeps you current at the cost of more downloads; a longer one saves bandwidth.",
                      accessibility: "Refresh after")
            WideRow(title: "Storage",
                    detail: "View cache info shows a count of what is on disk, and Clear cache deletes every stored emote and badge so they download fresh.",
                    help: "Clearing is safe and does not touch your settings or login. Chat will feel slower for the first visit to each channel while the cache refills.") {
                VStack(alignment: .leading, spacing: 8) {
                    VStack(spacing: 8) {
                        SettingButton(title: "View cache info") {
                            Task { await stats() }
                        }
                        SettingButton(title: "Clear cache", role: .destructive, tint: Color(red: 0xdc / 255, green: 0x26 / 255, blue: 0x26 / 255)) {
                            Task { await clear() }
                        }
                    }
                    if let summary {
                        Text("Currently cached: \(summary)").font(.sn(13)).foregroundStyle(SN.textSecondary)
                    }
                }
                .padding(.top, 10)
            }
        }
        EmotePrefetchSection()
    }

    private struct Stats: Decodable { let entries_by_type: [String: Int] }

    private func stats() async {
        do {
            let stats = try await model.core.invoke("get_universal_cache_statistics", args: [:], as: Stats.self)
            let parts = ["emotes", "badges", "third-party-badges", "cosmetics"].compactMap { kind -> String? in
                guard let n = stats.entries_by_type[kind], n > 0 else { return nil }
                return "\(n) \(kind)"
            }
            summary = parts.isEmpty ? "empty" : parts.joined(separator: ", ")
        } catch {
            model.toasts.show("Failed to get cache statistics: \(error)")
        }
    }

    private func clear() async {
        do {
            _ = try await model.core.invoke("clear_cache", args: [:])
            _ = try await model.core.invoke("clear_all_universal_cache", args: [:])
            summary = "empty"
        } catch {
            model.toasts.show("Failed to clear cache: \(error)")
        }
    }
}

/// Emote prefetch: scan your follows, then download every missing emote in the background.
private struct EmotePrefetchSection: View {
    @Environment(AppModel.self) private var model
    @State private var progress: Progress?

    struct Progress: Decodable {
        var phase: String
        var channels_total: Int
        var channels_done: Int
        var total_emotes: Int
        var already_cached: Int
        var to_download: Int
        var downloaded: Int
        var failed: Int
        var estimated_bytes: Int
        var warning: String?
    }

    var body: some View {
        SettingsBlock(title: "Emote Prefetch", detail: "Download every emote from the channels you follow ahead of time, so the emote menu opens instantly in any of their chats.") {
            WideRow(title: "Followed channels",
                    detail: "Scan your follows to see how many emotes are missing and how much space they need, then download them in the background while you do something else.",
                    help: "1. Scan follows. 2. Check the count and size. 3. Download. Shared emotes are stored once and anything already cached is skipped, so a rerun only fetches what is new. Best started when you are away from your desk.") {
                content.padding(.top, 10)
            }
        }
        .task { await follow() }
    }

    @ViewBuilder private var content: some View {
        let p = progress
        VStack(alignment: .leading, spacing: 8) {
            if let warning = p?.warning {
                Label(warning, systemImage: "exclamationmark.triangle").font(.sn(12)).foregroundStyle(SN.warning)
                    .padding(8).background { RoundedRectangle(cornerRadius: 8).fill(SN.warning.opacity(0.12)) }
            }
            switch p?.phase ?? "idle" {
            case "scanning":
                Label("Scanning your follows…", systemImage: "arrow.triangle.2.circlepath").font(.sn(13)).foregroundStyle(SN.textSecondary)
                ProgressView(value: Double(p?.channels_done ?? 0), total: Double(max(1, p?.channels_total ?? 1))).tint(SN.accent)
                HStack {
                    Text("\(p?.channels_done ?? 0) / \(p?.channels_total ?? 0) channels")
                    Spacer()
                    Text("\(p?.total_emotes ?? 0) emotes found")
                }
                .font(.sn(11).monospacedDigit()).foregroundStyle(SN.textMuted)
            case "planned":
                if let p, p.to_download == 0 {
                    Label("All \(p.total_emotes) emotes across \(p.channels_total) channels are already cached.", systemImage: "checkmark")
                        .font(.sn(13)).foregroundStyle(SN.textSecondary)
                    SettingButton(title: "Rescan", symbol: "arrow.clockwise") { Task { await plan() } }
                } else if let p {
                    Text("\(p.to_download) emotes to download (~\(Self.size(p.estimated_bytes))) across \(p.channels_total) channels. \(p.already_cached) already cached.")
                        .font(.sn(13)).foregroundStyle(SN.textSecondary)
                    HStack(spacing: 8) {
                        SettingButton(title: "Download \(p.to_download) emotes", symbol: "icloud.and.arrow.down", tint: SN.accent) {
                            Task { _ = try? await model.core.invoke("emote_prefetch_start", args: [:]) }
                        }
                        SettingButton(title: "Rescan") { Task { await plan() } }
                    }
                }
            case "downloading":
                Label("Downloading emotes…", systemImage: "arrow.down.circle").font(.sn(13)).foregroundStyle(SN.textSecondary)
                ProgressView(value: Double(p?.downloaded ?? 0), total: Double(max(1, p?.to_download ?? 1))).tint(SN.accent)
                HStack {
                    Text("\(p?.downloaded ?? 0) / \(p?.to_download ?? 0)")
                    Spacer()
                    let pct = Int((Double(p?.downloaded ?? 0) / Double(max(1, p?.to_download ?? 1)) * 100).rounded())
                    Text((p?.failed ?? 0) > 0 ? "\(p?.failed ?? 0) failed · \(pct)%" : "\(pct)%")
                }
                .font(.sn(11).monospacedDigit()).foregroundStyle(SN.textMuted)
                SettingButton(title: "Stop") { Task { _ = try? await model.core.invoke("emote_prefetch_stop", args: [:]) } }
            case "complete", "cancelled":
                let done = p?.phase == "complete" ? "Prefetch complete." : "Stopped."
                let cached = (p?.already_cached ?? 0) > 0 ? ", \(p?.already_cached ?? 0) already cached" : ""
                let failed = (p?.failed ?? 0) > 0 ? " · \(p?.failed ?? 0) failed" : ""
                let n = p?.downloaded ?? 0
                Text("\(done) Downloaded \(n) emote\(n == 1 ? "" : "s")\(cached)\(failed)").font(.sn(13)).foregroundStyle(SN.textSecondary)
                SettingButton(title: "Scan again", symbol: "arrow.clockwise") { Task { await plan() } }
            default:
                SettingButton(title: "Scan follows", symbol: "arrow.clockwise", tint: SN.accent) { Task { await plan() } }
            }
        }
    }

    static func size(_ bytes: Int) -> String {
        if bytes <= 0 { return "0 MB" }
        if bytes < 1_048_576 { return "\(max(1, bytes / 1024)) KB" }
        return "\((Double(bytes) / 1_048_576).fixed(1)) MB"
    }

    private func plan() async {
        progress = try? await model.core.invoke("emote_prefetch_plan", args: ["tier": "4x"], as: Progress.self)
    }

    private func follow() async {
        progress = try? await model.core.invoke("emote_prefetch_status", args: [:], as: Progress.self)
        for await event in model.core.events.all() where event.name.hasPrefix("emote-prefetch-") {
            if let next = try? JSONDecoder().decode(Progress.self, from: Data(event.payloadJSON.utf8)) { progress = next }
        }
    }
}

// MARK: Support

struct SupportPanel: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var invite: Invite?
    @State private var bannerIndex = 0
    @State private var iconIndex = 0

    struct Invite: Decodable {
        let name: String?
        let description: String?
        let icon_url: String?
        /// The server icon to try in order (animated, then the same icon still).
        let icon_urls: [String]?
        let banner_urls: [String]
        let online: Int?
        let members: Int?
        let invite_url: String
        let invite_label: String
    }

    var body: some View {
        Section {
            community
                .padding(.vertical, 12)
                .task {
                    while !Task.isCancelled {
                        if let next = try? await model.core.invoke("get_community_invite", args: [:], as: Invite.self) { invite = next }
                        try? await Task.sleep(for: .seconds(60))
                    }
                }
        } header: {
            ListHeader(text: "Community")
        }
        // Which build this is, and the other platforms StreamNook runs on: what the desktop's
        // About popup says, here since the phone has no title bar. Not in settings search, as on
        // Android.
        SettingsBlock(title: "Devices") {
            // The row's detail line, with the platform's mark leading it as in the desktop's footer.
            WideRow(title: "This device", icon: "iphone") {
                HStack(spacing: 6) {
                    SystemMark(platform: .apple, size: 12)
                    Text("iPhone · Version \(model.version)")
                }
                .font(.sn(12))
                .foregroundStyle(SN.textSecondary)
                .padding(.top, 2)
            }
            WideRow(title: "Also on your other devices", icon: "laptopcomputer.and.iphone") {
                OtherPlatformLinks()
                    .padding(.top, 10)
            }
        }
        SettingsBlock(title: "Diagnostics", detail: "How much StreamNook writes to its log file, and where to find that file when someone asks you for it.") {
            ToggleRow(title: "Detailed log", path: "error_reporting_enabled", fallback: true,
                      detail: "Records connection, playback, and chat activity to streamnook.log on this device so a problem can be traced after the fact.",
                      help: "Leave it on if you might report a bug; with it off the log holds almost nothing worth sending. The file stays on your device until you choose to share it.")
            WideRow(title: "Log export",
                    detail: "Shares your logs as one zip, so you can save it to Files or send it straight to an app for a bug report.") {
                SettingButton(title: "Share", symbol: "square.and.arrow.up") { Task { await export() } }
                    .padding(.top, 10)
            }
        }
        SettingsBlock(title: "Account Data", detail: "Signed in, StreamNook keeps a few things against your account. Chat messages and anything you type are not among them.") {
            SettingRow(title: "Channels and emotes",
                       detail: "Counts which channels you watch and which emotes you use, which is what fills in your profile stats and unlocks accolades.",
                       help: "Stored as running totals per channel and per emote, not as a history of when you watched. Signed out, nothing is counted.") { EmptyView() }
            SettingRow(title: "Version and platform",
                       detail: "Records which build you are on, your operating system, and whether update checks are getting through.",
                       help: "This is how a broken update path becomes visible. Without it a client that quietly stopped updating looks identical to one that is already current.") { EmptyView() }
            SettingRow(title: "Linked accounts",
                       detail: "Records which other platforms you have connected, so they survive a reinstall.") { EmptyView() }
            SettingRow(title: "VOD positions",
                       detail: "With Device sync on, keeps where you left off in each VOD you open, so your place follows you to your other devices.",
                       help: "Only past broadcasts you open yourself, never live watching. Kept for 90 days. Turn off Device sync under Player > Resume VODs and nothing more is sent.") { EmptyView() }
        }
    }

    private var community: some View {
        VStack(spacing: 16) {
            VStack(spacing: 8) {
                PenroseMark(width: 36).accessibilityHidden(true)
                    .frame(width: 56, height: 56)
                    .background { RoundedRectangle(cornerRadius: 16, style: .continuous).fill(SN.accent.opacity(0.1)) }
                Text("Join the community").font(.sn(17, 600)).foregroundStyle(SN.textPrimary)
                Text("Get help, request features, hear about updates, and hang out with other StreamNook users.")
                    .font(.sn(13)).foregroundStyle(SN.textSecondary).multilineTextAlignment(.center)
            }
            .frame(maxWidth: 340)
            VStack(alignment: .leading, spacing: 0) {
                banner
                VStack(alignment: .leading, spacing: 6) {
                    serverIcon.padding(.top, -36)
                    Text(invite?.name ?? "StreamNook").font(.sn(15, 600)).foregroundStyle(SN.textPrimary).lineLimit(1)
                    // The invite code is case-sensitive, so it is never uppercased.
                    Text(invite?.invite_label ?? "discord.gg/2xvuF9TES7").font(.sn(12, 500)).foregroundStyle(SN.textMuted)
                    if let description = invite?.description {
                        Text(description).font(.sn(12.5)).foregroundStyle(SN.textSecondary).lineLimit(2)
                    }
                    if let online = invite?.online, let members = invite?.members {
                        HStack(spacing: 14) {
                            HStack(spacing: 5) {
                                Circle().fill(Color.green).frame(width: 8, height: 8)
                                Text("\(Text(online, format: .number).bold()) Online")
                            }
                            HStack(spacing: 5) {
                                Circle().fill(SN.textMuted).frame(width: 8, height: 8)
                                Text("\(Text(members, format: .number).bold()) Members")
                            }
                        }
                        .font(.sn(12)).foregroundStyle(SN.textSecondary)
                    }
                    SettingButton(title: "Join the Discord", tint: Color(red: 0x58 / 255, green: 0x65 / 255, blue: 0xF2 / 255)) {
                        if let url = URL(string: invite?.invite_url ?? "https://discord.gg/2xvuF9TES7") { UIApplication.shared.open(url) }
                    }
                    .padding(.top, 6)
                }
                .padding(14)
            }
            .snPanel(radius: 16)
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        }
    }

    private var banner: some View {
        let urls = invite?.banner_urls ?? ["https://cdn.streamnook.app/community-banner.gif"]
        return ZStack(alignment: .bottom) {
            Color(red: 215 / 255, green: 165 / 255, blue: 140 / 255).opacity(0.07)
            if bannerIndex < urls.count {
                AsyncImage(url: URL(string: urls[bannerIndex])) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFill()
                    } else if phase.error != nil {
                        Color.clear.onAppear { bannerIndex += 1 }
                    }
                }
            }
            LinearGradient(colors: [.black.opacity(0.55), .clear], startPoint: .bottom, endPoint: .top)
        }
        .aspectRatio(2, contentMode: .fit)
        .clipped()
    }

    /// The first icon that loads, then the Penrose mark; the same list Android tries.
    private var serverIcon: some View {
        let urls = invite?.icon_urls ?? invite?.icon_url.map { [$0] } ?? []
        let url = iconIndex < urls.count ? URL(string: urls[iconIndex]) : nil
        return AsyncImage(url: url) { phase in
            if let image = phase.image {
                image.resizable().scaledToFill()
            } else if phase.error != nil, iconIndex + 1 < urls.count {
                iconFallback.onAppear { iconIndex += 1 }
            } else {
                iconFallback
            }
        }
        .frame(width: 68, height: 68)
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay { RoundedRectangle(cornerRadius: 16, style: .continuous).stroke(SN.backgroundTertiary, lineWidth: 4) }
    }

    private var iconFallback: some View {
        PenroseMark(width: 44).accessibilityHidden(true)
            .frame(maxWidth: .infinity, maxHeight: .infinity).background(SN.backgroundTertiary)
    }

    private func export() async {
        do {
            let result = try await model.core.invoke("export_logs", args: ["to": "share"])
            if result.contains("empty") { model.toasts.show("Nothing has been logged yet.") }
        } catch {
            model.toasts.show("Could not get the logs. Try again.")
        }
    }
}

/// The other platforms StreamNook runs on, in the downloads page's order, each opening that page
/// (it has one card per platform and no anchors). The same pills as the desktop's About popup and
/// Android's Support page: the mark, then the name, on the left of the pill.
private struct OtherPlatformLinks: View {
    @Environment(\.openURL) private var openURL

    private static let downloads = URL(string: "https://streamnook.app/downloads")!
    private struct Entry {
        let mark: PlatformMark.Platform
        let name: String
    }

    private static let platforms = [
        Entry(mark: .windows, name: "Windows"), Entry(mark: .apple, name: "macOS"),
        Entry(mark: .linux, name: "Linux"), Entry(mark: .android, name: "Android"),
    ]

    var body: some View {
        EqualColumns(spacing: 8) {
            ForEach(Self.platforms, id: \.name) { platform in
                Button {
                    openURL(Self.downloads)
                } label: {
                    HStack(spacing: 8) {
                        SystemMark(platform: platform.mark, size: 15)
                        Text(platform.name).font(.sn(13, 500)).lineLimit(1)
                    }
                    .foregroundStyle(SN.textSecondary)
                    .padding(.horizontal, 14)
                    .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
                    .contentShape(Capsule())
                }
                .buttonStyle(PlatformPillPress())
                .accessibilityLabel("StreamNook for \(platform.name)")
                .accessibilityHint("Opens streamnook.app/downloads")
            }
        }
    }
}

/// A platform pill: the row's chip fill, a shade darker while pressed.
private struct PlatformPillPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background { Capsule().fill(configuration.isPressed ? SN.surfaceActive : SN.surface) }
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

/// Columns of one width, the widest item's, so four pills split two and two with their marks on
/// one line, never one left alone on a row.
private struct EqualColumns: Layout {
    var columns = 2
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let cell = cellSize(proposal, subviews)
        let rows = (subviews.count + columns - 1) / columns
        return CGSize(width: cell.width * CGFloat(columns) + spacing * CGFloat(columns - 1),
                      height: cell.height * CGFloat(rows) + spacing * CGFloat(max(0, rows - 1)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let cell = cellSize(proposal, subviews)
        for (i, subview) in subviews.enumerated() {
            let x = bounds.minX + CGFloat(i % columns) * (cell.width + spacing)
            let y = bounds.minY + CGFloat(i / columns) * (cell.height + spacing)
            subview.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(cell))
        }
    }

    /// The widest and tallest item, narrowed when the columns would not fit the width offered.
    private func cellSize(_ proposal: ProposedViewSize, _ subviews: Subviews) -> CGSize {
        let sizes = subviews.map { $0.sizeThatFits(.unspecified) }
        var width = sizes.map(\.width).max() ?? 0
        if let available = proposal.width {
            width = min(width, (available - spacing * CGFloat(columns - 1)) / CGFloat(columns))
        }
        return CGSize(width: width, height: sizes.map(\.height).max() ?? 0)
    }
}
