import SwiftUI
import UIKit

// Desktop popovers, dropdowns and context menus become the system's own on the iPhone: Menu for
// a short list of picks, a popover for a small form beside its control, a sheet sized to its
// content for anything longer, and a confirmation dialog for a question.

/// One pick in a system menu: its name, a line under it, a check when it is the one chosen, and
/// dimmed when it cannot be picked now.
struct MenuPick: View {
    let label: String
    var detail: String?
    var symbol: String?
    var checked = false
    var enabled = true
    var role: ButtonRole?
    let action: () -> Void

    init(_ label: String, detail: String? = nil, symbol: String? = nil, checked: Bool = false, enabled: Bool = true,
         role: ButtonRole? = nil, action: @escaping () -> Void) {
        self.label = label
        self.detail = detail
        self.symbol = symbol
        self.checked = checked
        self.enabled = enabled
        self.role = role
        self.action = action
    }

    var body: some View {
        Button(role: role, action: action) {
            if checked || symbol != nil {
                Label {
                    Text(label)
                    if let detail { Text(detail) }
                } icon: {
                    Image(systemName: checked ? Symbol.check : symbol ?? "")
                }
            } else {
                Text(label)
                if let detail { Text(detail) }
            }
        }
        .disabled(!enabled)
    }
}

extension View {
    /// A popover holding `content`, kept a popover on the iPhone too, so it draws as Liquid Glass
    /// beside the control it came from instead of turning into a sheet.
    func glassPopover<Content: View>(isPresented: Binding<Bool>, arrowEdge: Edge? = nil,
                                     @ViewBuilder content: @escaping () -> Content) -> some View {
        popover(isPresented: isPresented, arrowEdge: arrowEdge) {
            content().presentationCompactAdaptation(.popover)
        }
    }

    /// A hide-keyboard button on the keyboard's bar, for a field whose own Return key does not
    /// close the keyboard: several lines (Return adds a line) or a number pad (no Return at all).
    /// Scrolling never closes a keyboard (people scroll chat while they type), so this button and
    /// a field's Done or Search key are the ways down.
    func keyboardHideButton() -> some View {
        toolbar {
            ToolbarItemGroup(placement: .keyboard) {
                Spacer()
                Button {
                    UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
                } label: {
                    Image(systemName: "keyboard.chevron.compact.down")
                }
                .accessibilityLabel("Hide keyboard")
            }
        }
    }

    /// Sizes a sheet to its content, never taller than `maxFraction` of the screen, with one
    /// detent (it cannot be pulled taller) and its handle showing.
    func fittedSheet(maxFraction: CGFloat = 0.72) -> some View {
        modifier(FittedSheet(maxFraction: maxFraction))
    }
}

private struct FittedSheet: ViewModifier {
    let maxFraction: CGFloat
    @State private var height: CGFloat?
    /// The content has scrolled under the handle.
    @State private var scrolled = false

    /// How far content fades out under the handle once it scrolls there, so nothing runs into the
    /// grabber with a hard edge. At rest there is no fade: a sheet's own top padding clears it.
    static let edgeFade: CGFloat = 24

    func body(content: Content) -> some View {
        let cap = Screen.height * maxFraction
        ScrollView {
            content
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
        }
        .scrollBounceBehavior(.basedOnSize)
        .scrollIndicators(.hidden)
        .onScrollGeometryChange(for: Bool.self) { $0.contentOffset.y + $0.contentInsets.top > 1 } action: { _, now in
            scrolled = now
        }
        .mask {
            VStack(spacing: 0) {
                LinearGradient(colors: [.black.opacity(scrolled ? 0 : 1), .black], startPoint: .top, endPoint: .bottom)
                    .frame(height: Self.edgeFade)
                Color.black
            }
            .animation(.easeOut(duration: 0.15), value: scrolled)
        }
        .presentationDetents([.height(min((height ?? cap) + 16, cap))])
        .presentationDragIndicator(.visible)
    }
}

/// The screen the app is on.
@MainActor
enum Screen {
    static var height: CGFloat {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        return scene?.screen.bounds.height ?? 874
    }
}

/// A sheet's title under the handle: 14 semibold, 8 points below the grabber.
struct SheetTitle: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.snSheetTitle)
            .foregroundStyle(SN.textPrimary)
            .frame(maxWidth: .infinity)
            .padding(.top, 22)
            .padding(.bottom, 8)
            .accessibilityAddTraits(.isHeader)
    }
}
