import SNKit
import SwiftUI
import UIKit

/// The landscape chat column's width handle: a quiet pill straddling the column's inner edge,
/// half over the picture. Dragging resizes the column live and saves the width on release; a
/// tap opens or closes the background strip.
struct ChatWidthHandle: View {
    let chat: LandscapeChatLayout
    let width: Double
    let screenWidth: Double
    let onDrag: (Double) -> Void
    let onCommit: (Double) -> Void
    let onTap: () -> Void
    @State private var start: Double?
    @State private var last: Double?
    @GestureState private var active = false

    var body: some View {
        Capsule()
            .fill(Color.white.opacity(0.35))
            .frame(width: 3, height: 32)
            .frame(width: 28, height: 80)
            .contentShape(Rectangle())
            .onTapGesture {
                Haptics.tick()
                onTap()
            }
            .gesture(
                DragGesture(minimumDistance: 5, coordinateSpace: .global)
                    .updating($active) { _, state, _ in state = true }
                    .onChanged { value in
                        let base = start ?? width
                        if start == nil { start = width }
                        let next = chat.dragged(from: base, by: value.translation.width, screenWidth: screenWidth)
                        last = next
                        onDrag(next)
                    }
                    .onEnded { _ in finish() }
            )
            // The system took the touch after it moved (an edge swipe, a call): the width it
            // reached is kept, as a release would.
            .onChange(of: active) { _, now in if !now, start != nil { finish() } }
            .accessibilityElement()
            .accessibilityLabel("Chat width")
            .accessibilityValue("\(Int(width)) points")
            .accessibilityAdjustableAction { direction in
                let step: Double = direction == .increment ? 20 : -20
                onCommit(LandscapeChatLayout.clamp(width + step, screenWidth: screenWidth))
            }
    }

    private func finish() {
        if let last { onCommit(last) }
        start = nil
        last = nil
    }
}

/// The strip at the foot of the overlay column: the inbox (the column has no message bar to hold
/// its tray), how see-through the chat is, and a button to hide it. Any touch on it keeps it open.
struct ChatBackgroundStrip: View {
    let opacity: Double
    let onChange: (Double) -> Void
    let onHide: () -> Void
    let onTouch: () -> Void
    var inbox: MentionsStore?

    var body: some View {
        HStack(spacing: 10) {
            if let inbox {
                InboxTray(store: inbox, surface: .watch, plain: true)
                    .padding(.leading, -8)
                Rectangle().fill(Color.white.opacity(0.15)).frame(width: 1, height: 20)
                    .accessibilityHidden(true)
            }
            Image(systemName: Symbol.chatBackground)
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(.white.opacity(0.8))
                .accessibilityHidden(true)
            Slider(value: Binding(get: { opacity }, set: { value in
                onTouch()
                onChange(value)
            }), in: 0...100, step: 5)
            .tint(SN.accent)
            .accessibilityLabel("Chat background")
            Text("\(Int(opacity))%")
                .font(.sn(12, 600))
                .monospacedDigit()
                .foregroundStyle(.white)
                .frame(width: 36, alignment: .trailing)
            Button(action: onHide) {
                Image(systemName: Symbol.close)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.8))
                    .frame(width: 36, height: 36)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Hide chat")
        }
        .padding(.leading, 12)
        .padding(.trailing, 4)
        .frame(height: 44)
        .snGlass(Capsule())
        .simultaneousGesture(TapGesture().onEnded(onTouch))
    }
}

/// Which landscape dial a swipe turns.
enum DialAxis: Equatable {
    case brightness, volume

    var label: String { self == .brightness ? "Brightness" : "Volume" }

    func symbol(_ level: Double) -> String {
        switch self {
        case .brightness: level < 0.5 ? "sun.min.fill" : "sun.max.fill"
        case .volume: level <= 0 ? "speaker.slash.fill" : (level < 0.5 ? "speaker.wave.1.fill" : "speaker.wave.3.fill")
        }
    }
}

/// A dial's readout in the middle of the picture: the icon, a track filled to the level, and the
/// percent. It never takes touches.
struct DialReadout: View {
    let axis: DialAxis
    let level: Double

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: axis.symbol(level))
                .font(.system(size: 18, weight: .medium))
                .foregroundStyle(.white)
                .frame(width: 22)
                .contentTransition(.symbolEffect(.replace))
            ZStack(alignment: .leading) {
                Capsule().fill(Color.white.opacity(0.25))
                Capsule().fill(SN.accent).frame(width: 112 * level)
            }
            .frame(width: 112, height: 3)
            Text("\(Dial.hundredth(level))%")
                .font(.sn(13, 600))
                .monospacedDigit()
                .foregroundStyle(.white)
                .frame(width: 40, alignment: .trailing)
        }
        .padding(.horizontal, 16)
        .frame(height: 44)
        .snGlass(Capsule())
        .allowsHitTesting(false)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(axis.label) \(Dial.hundredth(level))%")
    }
}

/// The screen brightness the dial borrows. iOS brightness is global, so the level the viewer had
/// is kept from the first touch and handed back when watching ends or the app leaves the front.
@MainActor
enum ScreenBrightness {
    private static var original: Double?
    private static var observer: NSObjectProtocol?

    private static var screen: UIScreen? {
        UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.screen }.first
    }

    static var level: Double { Double(screen?.brightness ?? 0.5) }

    static func set(_ level: Double) {
        guard let screen else { return }
        if original == nil {
            original = Double(screen.brightness)
            observer = NotificationCenter.default.addObserver(forName: UIApplication.willResignActiveNotification,
                                                              object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { restore() }
            }
        }
        screen.brightness = CGFloat(max(level, Dial.brightnessFloor))
    }

    /// Hands the brightness back, only if a dial changed it.
    static func restore() {
        guard let original else { return }
        screen?.brightness = CGFloat(original)
        self.original = nil
        if let observer { NotificationCenter.default.removeObserver(observer) }
        observer = nil
    }
}
