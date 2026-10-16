import SNKit
import SwiftUI

/// A channel's offline room in the player band: who, that they are offline and since when, what
/// they last streamed, the next scheduled stream, how many wait in chat, and the latest broadcast
/// as a plain button. Facts as a dot and words; no art under the text. Compact in the mini box.
struct OfflineRoomView: View {
    let model: AppModel
    let compact: Bool

    private var watch: Watch { model.watch }

    var body: some View {
        if let room = watch.room {
            ZStack {
                LinearGradient(colors: [SN.surface, Color.black], startPoint: .top, endPoint: .bottom)
                if compact { mini(room) } else { full(room) }
            }
            .contentShape(Rectangle())
            .accessibilityElement(children: .contain)
        }
    }

    private func mini(_ room: OfflineRoom) -> some View {
        VStack(spacing: 6) {
            Face(url: room.avatarURL, name: room.displayName, size: 32)
            OfflineDot()
        }
        .allowsHitTesting(false)
    }

    private func full(_ room: OfflineRoom) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                Face(url: room.avatarURL, name: room.displayName, size: 44)
                VStack(alignment: .leading, spacing: 3) {
                    Text(room.displayName)
                        .font(.sn(16, 600))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                    HStack(spacing: 6) {
                        OfflineDot()
                        if let since = RoomText.lastLive(room.lastLiveAt) {
                            Text(since)
                                .font(.sn(12))
                                .foregroundStyle(.white.opacity(0.6))
                                .lineLimit(1)
                        }
                    }
                }
                Spacer(minLength: 0)
            }
            if let title = room.lastTitle {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.sn(14))
                        .foregroundStyle(.white.opacity(0.85))
                        .lineLimit(2)
                    if let category = room.lastCategory {
                        Text(category)
                            .font(.sn(12))
                            .foregroundStyle(.white.opacity(0.6))
                            .lineLimit(1)
                    }
                }
            }
            if let next = RoomText.next(room.nextStream) {
                Label(next, systemImage: "calendar")
                    .font(.sn(12, 500))
                    .foregroundStyle(.white.opacity(0.75))
                    .lineLimit(1)
            }
            HStack(spacing: 8) {
                if let waiting = RoomText.waiting(room.chatters) {
                    if let sad = room.sadEmoteURL.flatMap(URL.init(string:)) {
                        AsyncImage(url: sad) { $0.resizable().scaledToFit() } placeholder: { Color.clear }
                            .frame(width: 22, height: 22)
                            .accessibilityHidden(true)
                    }
                    Text(waiting)
                        .font(.sn(12, 500))
                        .monospacedDigit()
                        .foregroundStyle(.white.opacity(0.75))
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                if room.latestVod != nil {
                    Button { watch.playRoomVod() } label: {
                        Label("Latest broadcast", systemImage: Symbol.play)
                            .font(.sn(13, 600))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                            .fixedSize()
                            .padding(.horizontal, 14)
                            .frame(height: 34)
                            .snGlass(Capsule(), interactive: true)
                    }
                    .buttonStyle(.plain)
                    .layoutPriority(1)
                    .accessibilityHint(room.latestVod?.title ?? "")
                }
            }
        }
        .padding(16)
        .frame(maxWidth: 520, maxHeight: .infinity, alignment: .center)
    }
}

/// "Offline": a hollow ring and the word.
private struct OfflineDot: View {
    var body: some View {
        HStack(spacing: 5) {
            Circle()
                .stroke(Color.white.opacity(0.6), lineWidth: 1.5)
                .frame(width: 7, height: 7)
            Text("Offline")
                .font(.sn(12, 600))
                .foregroundStyle(.white.opacity(0.8))
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Offline")
    }
}

/// The channel's face with a quiet ring (no live red), or its first letter.
private struct Face: View {
    let url: String?
    let name: String
    let size: CGFloat

    var body: some View {
        AsyncImage(url: url.flatMap(URL.init(string:))) { image in
            image.resizable().scaledToFill()
        } placeholder: {
            ZStack {
                Circle().fill(Color.white.opacity(0.15))
                Text(name.prefix(1).uppercased()).font(.sn(size * 0.4, 700)).foregroundStyle(.white)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .overlay(Circle().stroke(Color.white.opacity(0.25), lineWidth: 2))
        .accessibilityHidden(true)
    }
}
