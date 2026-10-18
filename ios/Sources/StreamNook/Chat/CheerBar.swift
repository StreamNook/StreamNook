import SNKit
import SwiftUI

/// One line above the field while the message cheers: the gem, what Send will spend and what is
/// left, or what stands in the way. A held cheer asks here ("Cheer 250"); a refused one says why,
/// and offers to go without the message when Twitch would take that. The x takes the cheer out
/// of a live message, or sets a held or refused one aside.
struct CheerBar: View {
    let model: ChatComposerModel

    var body: some View {
        if let shown = model.cheer.shown {
            HStack(spacing: 8) {
                if let gem = shown.iconURL {
                    RewardArt(url: gem, width: 18, height: 18, radius: 0, fit: true, placeholder: nil)
                }
                words(shown)
                    .font(.sn(12.5))
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                actions(shown)
            }
            .padding(.leading, 14)
            .padding(.trailing, 8)
            .padding(.top, 6)
            .accessibilityElement(children: .contain)
            .accessibilityAddTraits(.updatesFrequently)
        }
    }

    private static func bits(_ n: Int) -> String { "\(n.formatted()) \(n == 1 ? "bit" : "bits")" }

    @ViewBuilder
    private func words(_ shown: CheerCheck) -> some View {
        switch model.cheer.status {
        case .sending?:
            Text("Cheering \(Self.bits(shown.total))...").foregroundStyle(SN.textSecondary)
        case .refused(_, let problem, _, _)?:
            Text(problem).foregroundStyle(ComposerInk.warning)
        case .held?, nil:
            if model.cheer.status == nil, let problem = shown.problems.first {
                Text(problem).foregroundStyle(ComposerInk.warning)
            } else {
                (Text("Cheering \(Self.bits(shown.total))").fontWeight(.semibold).foregroundStyle(SN.textPrimary)
                    + Text(shown.left.map { " · \($0.formatted()) left" } ?? "").foregroundStyle(SN.textSecondary))
            }
        }
    }

    @ViewBuilder
    private func actions(_ shown: CheerCheck) -> some View {
        switch model.cheer.status {
        case .held(let check)?:
            Button("Cheer \(check.total.formatted())") { Task { await model.sendCheer(check) } }
                .font(.sn(12.5, 600))
                .foregroundStyle(SN.accent)
                .accessibilityLabel("Cheer \(Self.bits(check.total))")
        case .refused(let check, _, true, _)?:
            Button("Cheer without the message") { Task { await model.sendCheer(check, continueAnyway: true) } }
                .font(.sn(12.5, 600))
                .foregroundStyle(SN.accent)
        default:
            EmptyView()
        }
        if case .sending? = model.cheer.status {
            ProgressView().controlSize(.small)
        } else {
            Button { model.closeCheer() } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(ComposerInk.quiet)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(model.cheer.status == nil ? "Remove the cheer" : "Close")
        }
    }
}
