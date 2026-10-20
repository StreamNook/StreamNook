import SNKit
import SwiftUI
import UIKit

/// You › Your clips: the clips made in StreamNook on this account, newest first, as the core keeps
/// them. A tap plays one in the clip sheet; the menu copies its link or takes it off the list.
struct MyClipsScreen: View {
    let model: AppModel
    @State private var clips: [MyClip]?

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                if let clips, clips.isEmpty {
                    EmptyNote(text: "No clips yet", hint: "Clips you make while watching show up here.")
                }
                ForEach(clips ?? []) { clip in
                    Button {
                        guard let url = URL(string: clip.url) else { return }
                        model.clip = ClipLink(url: url)
                    } label: {
                        MediaRowLabel(thumbnail: clip.thumbnailUrl, title: clip.title,
                                      lines: ["\(clip.channelName) · \(clip.ageLabel)", clip.lengthLabel])
                    }
                    .buttonStyle(CardPress())
                    .contextMenu {
                        Button("Copy link", systemImage: "link") { UIPasteboard.general.string = clip.url }
                        Button("Remove from Your clips", systemImage: "minus.circle", role: .destructive) {
                            Task { await forget(clip) }
                        }
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
        }
        .background(SN.background)
        .navigationTitle("Your clips")
        .task { await load() }
        // A clip made elsewhere, or its thumbnail arriving after the render, shows without a refresh.
        .task {
            for await _ in model.core.events.stream("my-clips://changed") { await load() }
        }
    }

    private func load() async {
        clips = (try? await model.core.invoke("my_clips_list", args: [:], as: [MyClip].self)) ?? clips ?? []
    }

    private func forget(_ clip: MyClip) async {
        _ = try? await model.core.invoke("my_clips_forget", args: ["slug": clip.slug])
        await load()
    }
}

/// A video or clip row: the thumbnail with no words on it, the resume bar under it when there
/// is one, then the title and up to two quiet lines.
struct MediaRowLabel: View {
    let thumbnail: String?
    let title: String
    let lines: [String]
    var progress: Double?
    var watched = false

    static let thumbWidth: CGFloat = 144

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(spacing: 5) {
                AsyncImage(url: thumbnail.flatMap(URL.init(string:))) { phase in
                    if let image = phase.image { image.resizable().scaledToFill() } else { SN.surface }
                }
                .frame(width: Self.thumbWidth, height: Self.thumbWidth * 9 / 16)
                .clipShape(RoundedRectangle(cornerRadius: SNRadius.thumb, style: .continuous))
                .opacity(watched ? 0.6 : 1)
                if let progress {
                    Capsule()
                        .fill(Color.white.opacity(0.12))
                        .frame(height: 3)
                        .overlay(alignment: .leading) {
                            Capsule()
                                .fill(watched ? Color.white.opacity(0.4) : SN.accent)
                                .frame(width: Self.thumbWidth * max(0.04, min(1, progress)), height: 3)
                        }
                }
            }
            .frame(width: Self.thumbWidth)
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.sn(14, 600))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    Text(line).font(.sn(12.5)).foregroundStyle(SN.textSecondary).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}
