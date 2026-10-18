import SNKit
import SwiftUI

/// The drop being earned on the stream being watched, as a small glass card over the chat: the
/// reward, how far along, and a Claim once the minutes are in. A tap anywhere else shrinks the
/// player and shows the campaign in Rewards. It reads the core once a minute while the app is in
/// front; the core only asks Twitch when the channel can earn something.
struct WatchDropBar: View {
    let model: AppModel
    /// False while another chat room is on screen: progress belongs to the stream.
    var visible = true
    @State private var drop: WatchDrop?
    @State private var claiming = false
    /// The first answer for a stream is logged even when it is "nothing".
    @State private var reported: String?
    @Environment(\.scenePhase) private var phase

    private var stream: TwitchStream? { model.watch.stream }

    var body: some View {
        // A ZStack, not a Group: with no drop a Group has no child for `.task` to attach to, so
        // the read never ran and the bar could never appear. The zero-size clear base keeps the
        // reader alive while the bar takes no room.
        ZStack(alignment: .top) {
            Color.clear.frame(width: 0, height: 0)
            if visible, let drop {
                // The slot adds no padding, so an empty bar takes no room; the gutter is the card's.
                card(drop)
                    .padding(.horizontal, 10)
                    .padding(.top, 6)
                    .transition(.opacity.combined(with: .offset(y: -6)))
            }
        }
        .animation(SNMotion.run(SNMotion.settle), value: drop?.dropID)
        .task(id: "\(stream?.userLogin ?? "")|\(stream?.gameName ?? "")|\(phase == .active)") {
            drop = nil
            guard stream != nil else { return }
            while !Task.isCancelled {
                if phase == .active { await read() }
                try? await Task.sleep(for: .seconds(60))
            }
        }
    }

    private func read() async {
        guard let stream else { return }
        do {
            let next = try await model.core.invoke(
                "get_watch_drop",
                args: ["channelId": stream.userID ?? "", "login": stream.userLogin,
                       "gameId": stream.gameID ?? "", "gameName": stream.gameName],
                as: WatchDrop?.self)
            guard model.watch.stream?.userLogin == stream.userLogin else { return }
            if next != drop || reported != stream.userLogin {
                reported = stream.userLogin
                log.notice("drop bar \(stream.userLogin, privacy: .public) game '\(stream.gameName, privacy: .public)' (\(stream.gameID ?? "-", privacy: .public)): \(next.map { "\($0.name) \($0.trailing)" } ?? "nothing", privacy: .public), visible \(visible)")
            }
            drop = next
        } catch {
            log.error("get_watch_drop failed: \(String(describing: error), privacy: .public)")
        }
    }

    private func card(_ drop: WatchDrop) -> some View {
        HStack(spacing: 10) {
            RewardArt(url: drop.image, width: 36, height: 36, radius: 6, ring: true)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(drop.name)
                        .font(.sn(12, 600))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Text(drop.trailing)
                        .font(.sn(11, drop.complete ? 600 : 400))
                        .monospacedDigit()
                        .foregroundStyle(drop.complete ? SN.success : SN.textMuted)
                }
                Text(drop.line)
                    .font(.sn(10.5))
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
                ProgressTrack(percent: drop.percent, height: 4, color: drop.complete ? SN.success : SN.accent)
                    .padding(.top, 1)
            }
            if drop.complete {
                Button { Task { await claim(drop) } } label: {
                    Text(claiming ? "…" : drop.claimLabel)
                        .font(.sn(11, 600))
                        .foregroundStyle(SN.success)
                        .padding(.horizontal, 10)
                        .frame(height: 24)
                        .background(Capsule().fill(SN.success.opacity(0.2)))
                }
                .buttonStyle(.plain)
                .disabled(claiming)
                .opacity(claiming ? 0.6 : 1)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous), light: .popover)
        .contentShape(Rectangle())
        .onTapGesture { jump(to: drop.campaignID) }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(drop.spoken)
        .accessibilityHint("Shows the campaign in Rewards")
        .accessibilityAddTraits(.isButton)
    }

    /// The claim stays on the player; only a tap on the card goes to Rewards.
    private func claim(_ drop: WatchDrop) async {
        claiming = true
        defer { claiming = false }
        do {
            _ = try await model.core.invoke("claim_drop", args: ["dropId": drop.dropID, "dropInstanceId": drop.instanceID])
            model.toasts.show("Drop claimed", .success)
            await read()
        } catch {
            model.toasts.show("Could not claim that drop.")
            log.error("claim_drop failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The player shrinks to the mini window and Rewards shows the campaign.
    private func jump(to campaignID: String) {
        withAnimation(SNMotion.run(SNMotion.settleBack)) { model.nav.playerMode = .mini }
        model.rewards.focusCampaign = campaignID
        model.nav.tab = .rewards
    }
}
