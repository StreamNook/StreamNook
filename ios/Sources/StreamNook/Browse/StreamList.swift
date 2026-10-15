import SNKit
import SwiftUI

/// A list of live streams as cards or rows, in as many columns as the window's width allows,
/// settling in the first time it shows. Shared by Following, Browse and a category.
struct StreamGrid: View {
    let model: AppModel
    let streams: [TwitchStream]
    let view: StreamView
    /// Names the list, so a new search or another view mode settles in again.
    let settleKey: String
    let latch: SettleLatch
    /// Off where the screen already names the category.
    var showsCategory = true
    @State private var width: CGFloat = 402

    private var home: HomeStore { model.home }

    var body: some View {
        let shape = WindowShape(width: width, height: 874)
        let count = shape.columns(view == .list ? .row : .card)
        let gap: CGFloat = view == .list ? 8 : 12
        let playing = latch.playing(settleKey)
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: gap, alignment: .top), count: count), spacing: gap) {
            ForEach(Array(streams.enumerated()), id: \.element.id) { index, stream in
                StreamCard(stream: stream, variant: view, showsCategory: showsCategory,
                           hypeTrain: home.lists.hypeTrain(for: stream.userID),
                           streak: home.lists.streak(for: stream.userID), group: home.lists.group(for: stream.userID),
                           stamp: home.previewStamp,
                           open: { model.startWatching(stream) },
                           openCategory: category(of: stream).map { category in { model.nav.push(.category(category)) } })
                    .settleIn(index: index, active: playing)
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 24)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .settles(latch, key: settleKey, ready: !streams.isEmpty)
    }

    private func category(of stream: TwitchStream) -> TwitchCategory? {
        guard let id = stream.gameID, !id.isEmpty, !stream.gameName.isEmpty else { return nil }
        return TwitchCategory(id: id, name: stream.gameName)
    }
}

/// The two toolbar buttons that pick cards or rows, one choice shared by every stream list: the
/// chosen one in the accent color (cards filled, the list bold), the other muted.
struct StreamViewButtons: View {
    @Binding var view: StreamView

    var body: some View {
        HStack(spacing: 0) {
            pick(.cards, symbol: view == .cards ? "\(Symbol.cardView).fill" : Symbol.cardView, label: "Card view")
            pick(.list, symbol: Symbol.listView, label: "List view")
        }
    }

    private func pick(_ choice: StreamView, symbol: String, label: String) -> some View {
        Button { view = choice } label: {
            Image(systemName: symbol)
                .font(.system(size: 18, weight: view == choice ? .bold : .regular))
                .foregroundStyle(view == choice ? SN.accent : SN.textMuted)
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .accessibilityAddTraits(view == choice ? .isSelected : [])
    }
}

extension StreamView {
    /// The stored choice for every stream list.
    static let key = "sn-stream-view"
}

extension View {
    /// Hands the core the Twitch ids of the rows a screen shows beyond the followed and
    /// recommended lists (search results, a category), so hype trains and Shared Viewership cover
    /// them: 2 s after the set last changed, never an empty set, and a refetch of the same
    /// channels sends nothing. Leaving the screen drops a send still waiting.
    func reportsChannels(_ streams: [TwitchStream], to core: CoreBridge) -> some View {
        let ids = HomeLists.channelIDs(of: streams)
        return task(id: ids) {
            guard !ids.isEmpty else { return }
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            _ = try? await core.invoke("set_home_extra_channels", args: ["channelIds": ids])
            log.notice("reported \(ids.count) channels on screen")
        }
    }
}
