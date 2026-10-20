import SNKit
import SwiftUI
import UIKit

/// A channel's videos and clips, from the player's avatar, the offline room and the chatter card.
/// Two tabs: Videos (past broadcasts, highlights or uploads, recent or popular) and Clips (by
/// period). The core words every row; a video plays in the watch layer with its chat replay, a
/// clip in the clip sheet.
struct ChannelSheet: View {
    let channel: TwitchStream
    let model: AppModel
    @State private var tab = Tab.videos
    @State private var kind = MediaShelf.VideoKind.archive
    @State private var sort = MediaShelf.VideoSort.recent
    @State private var period = MediaShelf.ClipPeriod.month
    @State private var rows: [MediaRow] = []
    @State private var cursor: String?
    @State private var loading = false
    @State private var failed = false
    @Environment(\.dismiss) private var dismiss

    enum Tab: Hashable { case videos, clips }

    private var shelf: MediaShelf {
        tab == .videos ? .videos(kind, sort) : .clips(period)
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            GlazeSegments([Segment(value: Tab.videos, label: "Videos"), Segment(value: Tab.clips, label: "Clips")],
                          selection: Binding(get: { tab }, set: { if let t = $0 { tab = t } }), fill: true)
                .padding(.horizontal, 16)
            filters
            list
        }
        .background(SN.background)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .task(id: shelf) { await load(reset: true) }
    }

    private var header: some View {
        HStack(spacing: 12) {
            Avatar(url: channel.profileImageURL, size: 44)
            VStack(alignment: .leading, spacing: 2) {
                Text(channel.userName).font(.sn(20, 700)).foregroundStyle(SN.textPrimary).lineLimit(1)
                Text(isLive ? "Live now" : "Offline").font(.sn(13)).foregroundStyle(SN.textSecondary)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 18)
        .padding(.top, 22)
        .padding(.bottom, 14)
    }

    private var isLive: Bool {
        if model.watch.stream?.userLogin.lowercased() == channel.userLogin.lowercased(),
           model.watch.room == nil, model.watch.playingVod == nil {
            return true
        }
        return model.home.lists.followingLive.contains { $0.userLogin.lowercased() == channel.userLogin.lowercased() }
    }

    @ViewBuilder
    private var filters: some View {
        if tab == .videos {
            VStack(alignment: .leading, spacing: 6) {
                GlazeSegments(MediaShelf.VideoKind.allCases.map { Segment(value: $0, label: $0.title) },
                              selection: Binding(get: { kind }, set: { if let k = $0 { kind = k } }))
                Menu {
                    ForEach(MediaShelf.VideoSort.allCases, id: \.self) { option in
                        Button {
                            sort = option
                        } label: {
                            if option == sort { Label(option.title, systemImage: "checkmark") } else { Text(option.title) }
                        }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Text(sort.title)
                        Image(systemName: "chevron.down").font(.system(size: 10, weight: .semibold))
                    }
                    .font(.sn(13, 500))
                    .foregroundStyle(SN.textSecondary)
                    .frame(minHeight: 32)
                    .contentShape(Rectangle())
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 12)
        } else {
            GlazeSegments(MediaShelf.ClipPeriod.allCases.map { Segment(value: $0, label: $0.title) },
                          selection: Binding(get: { period }, set: { if let p = $0 { period = p } }))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.top, 12)
                .padding(.bottom, 8)
        }
    }

    private var list: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                ForEach(rows) { row in
                    Button { open(row) } label: {
                        MediaRowLabel(thumbnail: row.thumbnailUrl, title: row.title, lines: [row.metaLine, row.countLine],
                                      progress: row.progress, watched: row.watched)
                    }
                    .buttonStyle(CardPress())
                    .contextMenu { menu(row) }
                    .onAppear {
                        if row.id == rows.last?.id, cursor != nil { Task { await load(reset: false) } }
                    }
                }
                if loading {
                    PenroseMark(width: 28).frame(maxWidth: .infinity).padding(.vertical, 16)
                } else if failed {
                    EmptyNote(text: "Could not load these", hint: "Pull down to try again.")
                } else if rows.isEmpty {
                    EmptyNote(text: tab == .videos ? "No \(kind.title.lowercased()) yet" : "No clips in this period",
                              hint: nil)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
        }
        .refreshable { await load(reset: true) }
    }

    @ViewBuilder
    private func menu(_ row: MediaRow) -> some View {
        Button("Copy link", systemImage: "link") { UIPasteboard.general.string = row.url }
        if row.isClip, let vodID = row.vodId {
            Button("Watch full broadcast from here", systemImage: "play.rectangle") {
                model.openVod(videoID: vodID, title: row.title, thumbnail: row.thumbnailUrl, channel: channel,
                              at: row.vodOffset.map(Double.init))
            }
        }
    }

    private func open(_ row: MediaRow) {
        if row.isClip {
            guard let url = URL(string: row.url) else { return }
            dismiss()
            model.clip = ClipLink(url: url)
        } else {
            model.openVod(videoID: row.id, title: row.title, thumbnail: row.thumbnailUrl, channel: channel)
        }
    }

    private func load(reset: Bool) async {
        guard let id = channel.twitchID, !id.isEmpty else { failed = true; return }
        guard !loading || reset else { return }
        let asked = shelf
        if reset {
            rows = []
            cursor = nil
        }
        loading = true
        failed = false
        defer { if asked == shelf { loading = false } }
        do {
            let page = try await model.core.invoke("channel_media", args: asked.args(channelID: id, cursor: reset ? nil : cursor),
                                                   as: MediaPage.self)
            guard asked == shelf, !Task.isCancelled else { return }
            let known = Set(rows.map(\.id))
            rows += page.rows.filter { !known.contains($0.id) }
            cursor = page.cursor
        } catch {
            guard asked == shelf, !Task.isCancelled else { return }
            failed = true
        }
    }
}
