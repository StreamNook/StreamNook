import SNKit
import SwiftUI

/// The wallet's Bits tab: the balance, then two steps. Choose how many bits (a ready-made amount
/// or any amount typed), then pick a cheermote, drawn at that amount, to put the word
/// ("Cheer250") in the message box. Nothing is spent here: Send and the cheer bar ask first.
/// Rust (`get_bits_state`) decides what is cheerable here, the range and the ready-made amounts.
struct BitsPane: View {
    let login: String
    let channelID: String?
    let core: CoreBridge
    /// The cheer word chosen.
    let insert: (String) -> Void
    @State private var state: BitsState?
    @State private var failed = false
    @State private var text = ""
    @FocusState private var amountFocused: Bool

    private var amount: Int? {
        guard let state, let n = Int(text), n >= state.pickMin, n <= state.pickMax else { return nil }
        return n
    }

    var body: some View {
        Group {
            if let state {
                content(state)
            } else if failed {
                Text("Bits aren't available here right now.")
                    .font(.sn(13)).foregroundStyle(SN.textMuted)
                    .frame(maxWidth: .infinity).padding(.vertical, 32)
            } else {
                ProgressView().frame(maxWidth: .infinity).padding(.vertical, 40)
            }
        }
        .task(id: login) {
            guard let channelID else { failed = true; return }
            do {
                let next = try await core.invoke("get_bits_state", args: ["channelLogin": login, "channelId": channelID], as: BitsState.self)
                state = next
                if text.isEmpty { text = String(next.pickMin) }
            } catch {
                failed = true
            }
        }
    }

    private func content(_ state: BitsState) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                if let gem = gem(state, for: Int(state.balance ?? 1)) {
                    RewardArt(url: gem, width: 20, height: 20, radius: 0, fit: true, placeholder: nil)
                }
                if let balance = state.balance {
                    (Text(balance.formatted()).fontWeight(.semibold).monospacedDigit() + Text(" bits"))
                        .font(.sn(14)).foregroundStyle(SN.textPrimary)
                } else {
                    Text("Sign in to Twitch to see your bits").font(.sn(13)).foregroundStyle(SN.textMuted)
                }
            }

            HStack(spacing: 8) {
                HStack(spacing: 5) {
                    if let gem = gem(state, for: amount ?? state.pickMin) {
                        RewardArt(url: gem, width: 16, height: 16, radius: 0, fit: true, placeholder: nil)
                    }
                    TextField("", text: $text)
                        .keyboardType(.numberPad)
                        .font(.sn(13.5, 600))
                        .monospacedDigit()
                        .frame(width: 58)
                        .focused($amountFocused)
                        .accessibilityLabel("Bits per cheer")
                    Text("bits").font(.sn(12)).foregroundStyle(SN.textSecondary)
                }
                .padding(.horizontal, 10)
                .frame(height: 34)
                .snPanel(radius: 10)
                ScrollView(.horizontal) {
                    HStack(spacing: 6) {
                        ForEach(state.quickAmounts, id: \.self) { n in
                            Button { text = String(n); amountFocused = false } label: {
                                Text(n >= 1000 && n % 1000 == 0 ? "\(n / 1000)K" : n.formatted())
                                    .font(.sn(12, n == amount ? 600 : 500))
                                    .monospacedDigit()
                                    .foregroundStyle(n == amount ? SN.textPrimary : SN.textSecondary)
                                    .padding(.horizontal, 10)
                                    .frame(height: 30)
                                    .background(Capsule().fill(n == amount ? SN.accent.opacity(0.25) : SN.surface))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .scrollIndicators(.hidden)
            }

            Text(amount == nil ? "Choose \(state.pickMin.formatted()) to \(state.pickMax.formatted()) bits."
                 : state.pickMin > 1 ? "Cheers here start at \(state.pickMin.formatted()) bits. Pick a cheermote to add it; you'll confirm before anything is spent."
                 : "Pick a cheermote to add it. You'll confirm before anything is spent.")
                .font(.sn(11.5))
                .foregroundStyle(amount == nil ? SN.warning : SN.textMuted)
                .fixedSize(horizontal: false, vertical: true)

            if state.cheermotes.isEmpty {
                Text("This channel's cheermotes haven't loaded yet.").font(.sn(12.5)).foregroundStyle(SN.textMuted)
            } else {
                let custom = state.cheermotes.filter(\.custom)
                if !custom.isEmpty {
                    SectionLabel(text: "This channel")
                    grid(custom)
                    SectionLabel(text: "Twitch").padding(.top, 4)
                }
                grid(state.cheermotes.filter { !$0.custom })
            }
        }
    }

    private func grid(_ list: [CheermoteChoice]) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 64), spacing: 6)], spacing: 6) {
            ForEach(list) { choice in
                Button {
                    guard let amount else { return }
                    insert("\(choice.prefix)\(amount)")
                } label: {
                    VStack(spacing: 4) {
                        if let tier = choice.tier(for: amount ?? state?.pickMin ?? 1) {
                            RewardArt(url: tier.url, width: 32, height: 32, radius: 0, fit: true, placeholder: nil)
                        }
                        Text(choice.prefix).font(.sn(10.5)).foregroundStyle(SN.textSecondary).lineLimit(1)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .contentShape(Rectangle())
                }
                .buttonStyle(RowPress())
                .disabled(amount == nil)
                .opacity(amount == nil ? 0.4 : 1)
                .accessibilityLabel(amount.map { "Add \(choice.prefix)\($0)" } ?? choice.prefix)
            }
        }
    }

    private func gem(_ state: BitsState, for bits: Int) -> String? {
        let n = max(1, bits)
        return (state.icons.last { $0.bits <= n } ?? state.icons.first)?.url
    }
}
