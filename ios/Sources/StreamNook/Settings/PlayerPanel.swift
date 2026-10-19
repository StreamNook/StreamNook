import SNKit
import SwiftUI

/// Settings > Player: what happens when a stream ends, how the stream is fetched, how it starts,
/// how close to live it rides, and song identification. Volume, quality and speed live on the
/// player itself.
struct PlayerPanel: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        PanelIntro(text: "Most player controls (volume, quality, playback speed) are available directly in the video player. These settings control advanced streaming behavior.")

        SettingsBlock(title: "Auto-Switch", detail: "What happens when the stream you are watching ends, and where StreamNook takes you next.") {
            ToggleRow(title: "Stream switching", path: "auto_switch.enabled", fallback: true,
                      detail: "When the channel you are watching goes offline, StreamNook picks a new live stream and starts it for you.",
                      help: "Destination: Same Category picks the most-watched live stream in the same game or category; Followed Streams picks one of your followed channels that is live. Notice shows a toast naming the new channel each time it switches.") {
                NestedChoice(title: "Destination", path: "auto_switch.mode",
                             choices: [Choice("Same Category", "same_category"), Choice("Followed Streams", "followed_streams")],
                             fallback: "same_category")
                NestedToggle(title: "Notice", path: "auto_switch.show_notification", fallback: true)
            }
            ToggleRow(title: "Follow raids", path: "auto_switch.auto_redirect_on_raid", fallback: true,
                      detail: "When the streamer raids another channel, StreamNook jumps there with them (you need to be signed in).")
        }

        SettingsBlock(title: "Offline Chat", detail: "A channel's chat while it is not streaming: its offline screen, its latest broadcast, and the stream itself when it starts.") {
            ToggleRow(title: "Stay in chat", path: "auto_switch.stay_in_offline_chat",
                      detail: "When the stream you are watching ends, you stay in the channel's chat instead of being switched away.")
        }

        SettingsBlock(title: "Streaming", detail: "How StreamNook fetches the stream from Twitch, and how patient it is when a channel is slow to start.") {
            // Rust re-applies the phone's codec list on every save, so the switch needs no call.
            ToggleRow(title: "AV1 & h265", path: "streamlink.enhanced_codecs", fallback: true,
                      detail: "Asks Twitch for AV1 and h265 (HEVC) versions of a stream alongside h264.",
                      help: "Some channels offer them at better quality for the same bandwidth. Turn this off if you see decode errors or a black picture on older hardware. The change applies the next time a stream loads, no restart needed.")
            SliderRow(title: { "Start timeout: \(Int($0))s" }, path: "streamlink.stream_timeout", range: 30...120, step: 5,
                      fallback: 60,
                      detail: "How long StreamNook keeps trying to open a stream before giving up, which helps when a channel has only just gone live.",
                      accessibility: "Start timeout")
            SliderRow(title: { "Retry delay: \(Int($0))s" }, path: "streamlink.retry_streams", range: 0...5, step: 1, fallback: 3,
                      detail: "How long to wait between attempts while a stream is not available yet (0 means a single attempt).",
                      accessibility: "Retry delay")
        }

        SettingsBlock(title: "Playback", detail: "How every stream starts; you can still change most of this from the player itself.") {
            ToggleRow(title: "Autoplay", path: "video_player.autoplay", fallback: true,
                      detail: "A stream starts playing as soon as it opens, with no need to press play.")
            ToggleRow(title: "Background play", path: "video_player.background_mode",
                      detailFor: { audio in
                          audio ? "Leaving the app floats the video in Picture in Picture. Close the window and the audio keeps playing, with its controls on the Lock Screen."
                                : "Leaving the app floats the video in Picture in Picture. Closing the window ends the stream."
                      },
                      write: { ["video_player.background_mode": $0 ? "audio" : "pip"] },
                      isOn: { $0.string("video_player.background_mode", "pip") == "audio" })
            ChoiceRow(title: "Start quality", path: "quality", choices: Self.qualities, fallback: "best", menu: true,
                      detail: "Every stream opens at this quality, and you can change it anytime from the player controls.",
                      accessibility: "Default stream quality")
            ToggleRow(title: "Start muted", path: "video_player.muted",
                      detail: "Every stream opens silent until you unmute it.")
            SliderRow(title: { "Start volume: \(Int(($0 * 100).rounded()))%" }, path: "video_player.volume", range: 0...1,
                      step: 0.01, fallback: 1, detail: "The volume every stream opens at before you adjust it.",
                      integer: false, accessibility: "Start volume")
            ToggleRow(title: "Resume VODs", path: "video_player.resume_vod_playback", fallback: true,
                      detail: "Reopening a past broadcast picks up where you left off.",
                      help: "Off starts every VOD from the beginning. Only past broadcasts you open yourself are remembered, never a live stream you were watching. Device sync keeps your place in each VOD the same on every StreamNook you sign in to, and saves it to your Twitch account for Twitch's own apps.") {
                if store.bool("video_player.resume_vod_playback", true) {
                    NestedToggle(title: "Device sync", path: "video_player.sync_vod_progress", fallback: true)
                }
            }
        }

        // Buffer is hidden on iPhone, so the section shows as Latency; its anchor and catalog name stay.
        SettingsBlock(title: "Latency & Buffering", shownTitle: "Latency", detail: "How close to live the player rides.") {
            TargetLatencyRow()
            ToggleRow(title: "Low latency", path: "video_player.experimental_low_latency", fallback: true,
                      detail: "Rides as close to live as the Twitch site on channels that support it. On by default.",
                      help: "Off falls back to the wider whole-segment path. If a stream stutters or refuses to play, turn this off first.")
        }

        SettingsBlock(title: "Discover", detail: "What Discover and Kick browse show you.") {
            LanguagesRow()
        }
    }

    static let qualities = [
        Choice("Auto (Source)", "best"), Choice("1440p60", "1440p60"), Choice("1080p60", "1080p60"),
        Choice("720p60", "720p60"), Choice("480p30", "480p30"), Choice("360p30", "360p30"),
        Choice("160p30", "160p30"), Choice("Audio Only", "audio_only"),
    ]
}

/// Target latency: a slider whose readout says "Auto" until a number is chosen; the reset clears
/// it back to Auto. While Auto the thumb rests at 1.5 s.
private struct TargetLatencyRow: View {
    @Environment(SettingsStore.self) private var store
    @State private var live: Double?

    var body: some View {
        let path = "video_player.ll_target_latency"
        let stored = store.value(path).flatMap(SettingsPath.double)
        let shown = live.map { "\($0.fixed(1))s" } ?? stored.map { "\($0.fixed(1))s" } ?? "Auto"
        SettingRow(title: "Target latency: \(shown)",
                   detail: "How far behind the live edge the player rides; lower is closer to live. Auto picks a gap per channel.",
                   help: "Reopen the stream to apply a change. Auto rides 1.5s behind on channels the low-latency engine serves, 3.0s on other low-latency broadcasts and 5.0s on normal-latency ones. Set your own number to override all three; the lowest gaps need a solid connection to stay smooth.",
                   paths: [path],
                   trailing: { EmptyView() },
                   below: {
                       Slider(value: Binding(get: { live ?? stored ?? 1.5 }, set: { live = ($0 * 10).rounded() / 10 }),
                              in: 1...10, step: 0.1) { editing in
                           guard !editing, let value = live else { return }
                           Task {
                               await store.set(path, value)
                               live = nil
                           }
                       }
                       .tint(SN.accent)
                       .accessibilityLabel("Target latency")
                       .padding(.top, 10)
                   },
                   nested: { EmptyView() })
    }
}

/// Discover > Languages: which languages Discover and Kick browse show. Nothing picked is the
/// device language, "any" alone is no filter, otherwise the picked codes; Rust reads the list on
/// every browse (`browse_languages.rs`).
private struct LanguagesRow: View {
    @Environment(SettingsStore.self) private var store
    @State private var picking = false

    var body: some View {
        let summary = DiscoveryLanguages.summary(store.strings(DiscoveryLanguages.path))
        SettingRow(title: "Languages", detail: "Streams in Discover and Kick browse. Your device language until you pick some.",
                   paths: [DiscoveryLanguages.path]) {
            Button { picking = true } label: {
                HStack(spacing: 4) {
                    Text(summary).font(.sn(13, 500)).foregroundStyle(SN.textSecondary).lineLimit(1)
                    Image(systemName: "chevron.right").font(.system(size: 11, weight: .semibold)).foregroundStyle(SN.textMuted)
                }
                .frame(minHeight: 32)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .frame(maxWidth: 160, alignment: .trailing)
            .accessibilityLabel("Languages")
            .accessibilityValue(summary)
        }
        .sheet(isPresented: $picking) { LanguagesSheet() }
    }
}

/// The language list: "Any language" first and on its own, then every language in the desktop's
/// order. Picking a language drops "Any language"; picking none goes back to the device language.
private struct LanguagesSheet: View {
    @Environment(SettingsStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let picked = store.strings(DiscoveryLanguages.path)
        NavigationStack {
            List {
                Section {
                    Text(picked.isEmpty ? "Showing your device language. Pick languages to choose your own."
                                        : "Pick none to go back to your device language.")
                        .font(.sn(12.5)).foregroundStyle(SN.textSecondary)
                        .listRowBackground(Color.clear)
                }
                Section {
                    row("Any language", on: picked == [DiscoveryLanguages.any]) {
                        [DiscoveryLanguages.any]
                    }
                    ForEach(DiscoveryLanguages.options, id: \.code) { option in
                        row(option.name, on: picked.contains(option.code)) {
                            let rest = picked.filter { $0 != DiscoveryLanguages.any }
                            return rest.contains(option.code) ? rest.filter { $0 != option.code } : rest + [option.code]
                        }
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .background(SN.background)
            .navigationTitle("Languages")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        .presentationDetents([.large])
    }

    private func row(_ name: String, on: Bool, next: @escaping () -> [String]) -> some View {
        Button {
            let value = next()
            Task { await store.set(DiscoveryLanguages.path, value) }
        } label: {
            HStack {
                Text(name).font(.sn(14)).foregroundStyle(SN.textPrimary)
                Spacer()
                if on { Image(systemName: "checkmark").font(.system(size: 14, weight: .semibold)).foregroundStyle(SN.accent) }
            }
            .frame(minHeight: 36)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowBackground(SN.backgroundSecondary)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// The Discover languages, in the desktop's order (src/utils/discoveryLanguages.ts).
enum DiscoveryLanguages {
    static let path = "discovery_languages"
    static let any = "any"
    static let options: [(code: String, name: String)] = [
        ("en", "English"), ("es", "Español"), ("fr", "Français"), ("de", "Deutsch"), ("it", "Italiano"),
        ("pt", "Português"), ("ru", "Русский"), ("ja", "日本語"), ("ko", "한국어"), ("zh", "中文"), ("zh-hk", "粵語"),
        ("ar", "العربية"), ("tr", "Türkçe"), ("pl", "Polski"), ("nl", "Nederlands"), ("sv", "Svenska"), ("no", "Norsk"),
        ("da", "Dansk"), ("fi", "Suomi"), ("cs", "Čeština"), ("sk", "Slovenčina"), ("hu", "Magyar"), ("ro", "Română"),
        ("bg", "Български"), ("el", "Ελληνικά"), ("th", "ไทย"), ("vi", "Tiếng Việt"), ("id", "Bahasa Indonesia"),
        ("ms", "Bahasa Melayu"), ("hi", "हिन्दी"), ("tl", "Filipino"), ("uk", "Українська"), ("ca", "Català"),
        ("other", "Other"),
    ]

    /// "Device language", "Any language", or the first two picked names and how many more.
    static func summary(_ picked: [String]) -> String {
        if picked.isEmpty { return "Device language" }
        if picked == [any] { return "Any language" }
        let names = picked.map { code in options.first { $0.code == code }?.name ?? code }
        return names.count <= 2 ? names.joined(separator: ", ") : "\(names.prefix(2).joined(separator: ", ")) +\(names.count - 2)"
    }
}
