import SNKit
import SwiftUI

/// The watch page: the video with how far behind the relay's live edge it plays, and the chat socket's last frames under it.
/// Landscape is immersive: the video alone, no system bars.
struct WatchView: View {
    let model: AppModel
    let stream: TwitchStream
    @Environment(\.verticalSizeClass) private var vertical

    private var watch: Watch { model.watch }
    private var landscape: Bool { vertical == .compact }

    var body: some View {
        Group {
            if landscape {
                video
                    .ignoresSafeArea()
            } else {
                VStack(spacing: 0) {
                    video
                        .aspectRatio(16 / 9, contentMode: .fit)
                    details
                    frameList
                }
            }
        }
        .background(.black)
        .navigationTitle(stream.userName)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(landscape ? .hidden : .visible, for: .navigationBar)
        .statusBarHidden(landscape)
        .persistentSystemOverlays(landscape ? .hidden : .automatic)
        .onAppear { watch.open(stream, core: model.core) }
        .onDisappear { watch.close(core: model.core) }
    }

    private var video: some View {
        PlayerView(player: watch.player)
            .overlay(alignment: .topLeading) {
                Text(edgeText)
                    .font(.caption.monospacedDigit())
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .glassEffect()
                    .padding(10)
            }
    }

    private var edgeText: String {
        guard let behind = watch.behindMS else { return watch.status }
        return String(format: "Behind %.1f s", Double(behind) / 1000)
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(stream.title).font(.subheadline).lineLimit(2)
            HStack(spacing: 12) {
                Label(watch.status, systemImage: "play.circle")
                if let quality = watch.quality { Label(quality, systemImage: "sparkles.tv") }
                if let first = watch.firstPlayMS { Label("\(first) ms", systemImage: "timer") }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            Label(watch.chatState, systemImage: "bubble.left.and.bubble.right")
                .font(.caption)
                .foregroundStyle(.secondary)
            if let error = watch.error {
                Text(error).font(.caption).foregroundStyle(.red)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(Color(.systemBackground))
    }

    private var frameList: some View {
        ScrollViewReader { reader in
            List(Array(watch.frames.frames.enumerated()), id: \.offset) { index, frame in
                Text(frame.summary)
                    .font(.caption.monospaced())
                    .foregroundStyle(frame.kind == .message ? .primary : .secondary)
                    .lineLimit(2)
                    .id(index)
            }
            .listStyle(.plain)
            .onChange(of: watch.frames.frames.count) { _, count in
                reader.scrollTo(count - 1, anchor: .bottom)
            }
        }
    }
}
