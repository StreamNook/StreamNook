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
                      detail: "Asks Twitch for AV1 and h265 (HEVC) versions of the stream alongside h264, which some channels offer at better quality for the same bandwidth.",
                      help: "Turn this off if you see decode errors or a black picture on older hardware. The change applies the next time a stream loads, no restart needed.")
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
                          audio ? "Leaving the app keeps the audio playing, with its controls on the Lock Screen. Tap them to come back."
                                : "Leaving the app floats the video in a picture-in-picture window."
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
        }

        SettingsBlock(title: "Latency & Buffering", detail: "How close to live the player rides, and how much it keeps loaded ahead.") {
            TargetLatencyRow()
            ToggleRow(title: "Low latency", path: "video_player.experimental_low_latency", fallback: true,
                      detail: "Rides as close to live as the Twitch site on channels that support it. On by default.",
                      help: "Off falls back to the wider whole-segment path. If a stream stutters or refuses to play, turn this off first.")
            SliderRow(title: { "Buffer: \(Int($0))s" }, path: "video_player.max_buffer_length", range: 3...300, step: 1,
                      fallback: 120,
                      detail: "How much video the player keeps loaded ahead of playback; more is steadier on a shaky connection but adds delay.",
                      accessibility: "Buffer")
        }

        SettingsBlock(title: "Song Identification", detail: "The /song command and the player's music button listen to a few seconds of the stream and name the track.") {
            SliderRow(title: { "Listen time: \(Int($0))s" }, path: "video_player.song_id.capture_seconds", range: 3...20, step: 1,
                      fallback: 10,
                      detail: "How many seconds of audio to fingerprint; longer matches more reliably over talking or noise, but the result takes a little longer to appear.",
                      accessibility: "Listen time")
            SliderRow(title: { "Retries: \(Int($0))" }, path: "video_player.song_id.retries", range: 0...3, step: 1, fallback: 1,
                      detail: "If the first listen matches nothing, StreamNook listens again this many times.",
                      help: "Each retry listens to a fresh window, so an ad break or quiet moment gets another shot.",
                      accessibility: "Retries")
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
        SettingRow(title: "Target latency",
                   detail: "How close to live the player stays: how far behind the live edge it rides; lower is closer to live. Auto picks a gap that suits each channel (reopen the stream to apply).",
                   help: "Auto rides 1.5s behind on channels the low-latency engine serves, 3.0s on other low-latency broadcasts and 5.0s on normal-latency ones. Set your own number to override all three; the lowest gaps need a solid connection to stay smooth.",
                   paths: [path],
                   trailing: { SliderReadout(text: live.map { "\($0.fixed(1))s" } ?? stored.map { "\($0.fixed(1))s" } ?? "Auto") },
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
