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
    @State private var width: CGFloat = 402

    private var home: HomeStore { model.home }

    var body: some View {
        let shape = WindowShape(width: width, height: 874)
        let count = shape.columns(view == .list ? .row : .card)
        let gap: CGFloat = view == .list ? 8 : 12
        let playing = latch.playing(settleKey)
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: gap, alignment: .top), count: count), spacing: gap) {
            ForEach(Array(streams.enumerated()), id: \.element.id) { index, stream in
                StreamCard(stream: stream, variant: view, hypeTrain: home.lists.hypeTrain(for: stream.userID),
                           streak: home.lists.streak(for: stream.userID), stamp: home.previewStamp,
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

/// The toolbar menu that picks cards or rows, one choice shared by every stream list.
struct StreamViewMenu: View {
    @Binding var view: StreamView

    var body: some View {
        Menu {
            Picker("View", selection: $view) {
                Label("Cards", systemImage: Symbol.cardView).tag(StreamView.cards)
                Label("List", systemImage: Symbol.listView).tag(StreamView.list)
            }
        } label: {
            Image(systemName: view == .list ? Symbol.listView : Symbol.cardView)
                .accessibilityLabel(view == .list ? "List view" : "Card view")
        }
    }
}

extension StreamView {
    /// The stored choice for every stream list.
    static let key = "sn-stream-view"
}
