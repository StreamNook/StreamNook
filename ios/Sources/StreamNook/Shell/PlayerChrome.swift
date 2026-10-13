import SNKit
import SwiftUI

/// What sits over the full player: a quiet loader until the first frame, the failure if there is
/// one, and on a tap the stream's name and title with the minimize control (and, on its side, the
/// chat button). The controls fade after three seconds without a touch. The player's own controls
/// (quality, volume, sharing) belong to the watch screen's builders.
struct PlayerChrome: View {
    let model: AppModel
    let landscape: Bool
    let insets: EdgeInsets
    /// Room the landscape chat column takes on the trailing side, so no control sits under it.
    let chatWidth: CGFloat
    let shown: Bool
    let onMinimize: () -> Void
    let onTouch: () -> Void

    private var watch: Watch { model.watch }
    private var stream: TwitchStream? { model.nav.playing }

    var body: some View {
        ZStack {
            if let error = watch.error {
                Text(error)
                    .font(.snCaption)
                    .foregroundStyle(.white.opacity(0.85))
                    .multilineTextAlignment(.center)
                    .padding(24)
                    .allowsHitTesting(false)
            } else if watch.firstPlayMS == nil {
                PenroseMark(width: 30).allowsHitTesting(false)
            }
            if shown {
                LinearGradient(stops: [.init(color: .black.opacity(0.6), location: 0), .init(color: .clear, location: 0.45)],
                               startPoint: .top, endPoint: .bottom)
                    .allowsHitTesting(false)
                    .transition(.opacity)
                header
                    .padding(.top, landscape ? insets.top + 8 : 4)
                    .padding(.leading, landscape ? insets.leading + 4 : 4)
                    .padding(.trailing, (landscape ? insets.trailing + 4 : 4) + chatWidth)
                    .frame(maxHeight: .infinity, alignment: .top)
                    .transition(.opacity)
            }
            #if DEBUG
            if Drive.showsEdge {
                Text(edgeText)
                    .font(.caption.monospacedDigit())
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .glassEffect()
                    .padding(10)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                    .allowsHitTesting(false)
            }
            #endif
        }
        .clipped()
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 4) {
            IconButton(symbol: Symbol.minimize, label: "Minimize", size: 18, tint: .white) {
                onTouch()
                onMinimize()
            }
            if let stream {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(stream.userName)
                            .font(.snRowStrong)
                            .foregroundStyle(.white)
                            .lineLimit(1)
                        LiveDot()
                        Text(stream.viewerCount.formatted())
                            .font(.sn(12.5, 500).monospacedDigit())
                            .foregroundStyle(.white.opacity(0.8))
                    }
                    Text(stream.title)
                        .font(.snRowDetail)
                        .foregroundStyle(.white.opacity(0.75))
                        .lineLimit(1)
                }
                .padding(.top, 5)
                .frame(maxWidth: .infinity, alignment: .leading)
                .allowsHitTesting(false)
            }
            if landscape {
                let on = model.nav.landscapeChat
                IconButton(symbol: on ? Symbol.chatShown : Symbol.chatShow, label: on ? "Hide chat" : "Show chat",
                           size: 18, tint: .white) {
                    onTouch()
                    withAnimation(SNMotion.run(.easeOut(duration: 0.18))) { model.nav.landscapeChat.toggle() }
                }
            }
        }
    }

    #if DEBUG
    private var edgeText: String {
        guard let behind = watch.behindMS else { return watch.status }
        return String(format: "Behind %.1f s", Double(behind) / 1000)
    }
    #endif
}

/// A live status: a solid live-red dot.
struct LiveDot: View {
    var size: CGFloat = 6

    var body: some View {
        Circle()
            .fill(SN.live)
            .frame(width: size, height: size)
            .shadow(color: SN.live.opacity(0.6), radius: 1.5)
            .accessibilityLabel("Live")
    }
}
