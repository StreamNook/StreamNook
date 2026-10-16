import SNKit
import SwiftUI

/// One badge: its art, where it stands, how it is earned, when, and how many people have it.
/// Sized to its content.
struct BadgeSheet: View {
    let model: AppModel
    let tile: BadgeTile
    let close: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            RewardArt(url: tile.image, width: 80, height: 80, radius: 0, fit: true, placeholder: nil)
                .padding(.top, 28)
            Text(tile.title)
                .font(.sn(16, 600))
                .foregroundStyle(SN.textPrimary)
                .multilineTextAlignment(.center)
                .padding(.top, 10)
            if !tile.sheetWord.isEmpty {
                Text(tile.sheetWord)
                    .font(.sn(11, 600))
                    .tracking(0.3)
                    .foregroundStyle(BadgeWords.color(tile))
                    .padding(.top, 4)
            }
            VStack(alignment: .leading, spacing: 12) {
                let prose = BlurbText.string(tile.prose)
                if !prose.isEmpty {
                    Text(prose)
                        .font(.sn(13))
                        .foregroundStyle(SN.textSecondary)
                        .lineSpacing(3)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let caveat = tile.caveat { WarningRow(text: caveat) }
                let window = BlurbText.string(tile.window)
                if !window.isEmpty { windowTile(window) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 12)
            if let usage = tile.usageLabel {
                Text(usage)
                    .font(.snCaption)
                    .foregroundStyle(SN.textMuted)
                    .padding(.top, 12)
            }
            if let text = tile.infoURL, let url = URL(string: text) {
                Button("More info") { Task { await UIApplication.shared.open(url) } }
                    .buttonStyle(.snSecondary)
                    .padding(.top, 16)
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 16)
        .fittedSheet(maxFraction: 0.8)
    }

    private func windowTile(_ text: String) -> some View {
        HStack(spacing: 10) {
            Image(systemName: Symbol.calendar)
                .font(.system(size: 15))
                .foregroundStyle(SN.accent)
            VStack(alignment: .leading, spacing: 3) {
                Text("EVENT DURATION")
                    .font(.sn(10.5, 600))
                    .tracking(0.5)
                    .foregroundStyle(SN.textSecondary)
                Text(text)
                    .font(.sn(12.5))
                    .foregroundStyle(SN.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .snPanel()
    }
}

/// Badge copy the core split into text and instants, with each instant in the phone's own time
/// zone: "Mar 4, 3:00 PM", the year only when it is not this one.
enum BlurbText {
    static func string(_ parts: [TextPart], now: Date = .now) -> String {
        parts.map { part in
            if let ms = part.ms {
                let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
                let sameYear = Calendar.current.component(.year, from: date) == Calendar.current.component(.year, from: now)
                var style = Date.FormatStyle().month(.abbreviated).day().hour().minute()
                if !sameYear { style = style.year() }
                return date.formatted(style)
            }
            return part.text ?? ""
        }
        .joined()
        .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
