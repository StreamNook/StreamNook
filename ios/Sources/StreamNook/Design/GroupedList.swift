import SwiftUI

// The app's grouped list: the system's inset-grouped cards (opaque, so nothing shows through at
// any glassiness) on the theme's background, each section named by a quiet label above its card
// and explained by a line under it. The chat menu and every settings page draw with this one
// helper, so their cards, rows, icons, separators and headers cannot drift apart.

extension View {
    /// An inset-grouped list on the theme background, with the chat menu's spacing and tint. Views
    /// inside it know they already sit on a card (`snInGroupedList`), so a panel does not draw a
    /// second card inside the row.
    @MainActor
    func snGroupedList() -> some View {
        listStyle(.insetGrouped)
            .listSectionSpacing(16)
            .contentMargins(.top, 4, for: .scrollContent)
            .scrollContentBackground(.hidden)
            .tint(SN.textPrimary)
            .environment(\.snInGroupedList, true)
    }
}

/// A grouped list's section name above its card: plain and quieter than the rows.
struct ListHeader: View {
    let text: String
    var logo: String?

    /// The header's ink, the same in every theme.
    static let ink = Color(red: 0x8C / 255, green: 0xA0 / 255, blue: 0xA7 / 255)

    var body: some View {
        HStack(spacing: 6) {
            if let logo { BrandLogo(id: logo, size: 13) }
            Text(text).foregroundStyle(Self.ink)
        }
    }
}

/// The line under a grouped list's card.
struct ListFooter: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.sn(12.5))
            .foregroundStyle(SN.textSecondary)
    }
}

private struct InGroupedListKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// True inside `snGroupedList`: the row already sits on a card.
    var snInGroupedList: Bool {
        get { self[InGroupedListKey.self] }
        set { self[InGroupedListKey.self] = newValue }
    }
}

/// The grouped card where a List cannot go (rows inside a page that scrolls with other content):
/// the system's grouped cell surface, rows split by hairlines inset after their symbols.
struct GroupedCard<Content: View>: View {
    /// Where a row's text starts: its padding, the 24 point symbol column and the gap after it.
    var separatorInset: CGFloat = 48
    @ViewBuilder let content: Content

    /// The corner of iOS's inset-grouped cards.
    static var radius: CGFloat { 26 }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Self.radius, style: .continuous)
        VStack(spacing: 0) {
            Group(subviews: content) { rows in
                ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                    if index > 0 {
                        Rectangle().fill(Color(uiColor: .separator)).frame(height: 0.5).padding(.leading, separatorInset)
                    }
                    row
                }
            }
        }
        .background(Color(uiColor: .secondarySystemGroupedBackground), in: shape)
        .clipShape(shape)
        // A form's rows: a switch or a button pressed while typing acts, and the keyboard goes.
        .tapsThroughKeyboard()
    }
}
