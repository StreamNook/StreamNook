import SNKit
import SwiftUI

/// The drop being earned on the watched stream, shared by the reader that keeps it current and
/// the chat stack that draws it as one of its cards.
@Observable @MainActor
final class WatchDropModel {
    /// The watch layer's drop. The chat stage keeps its own.
    static let watched = WatchDropModel()

    private(set) var drop: WatchDrop?
    /// False while another chat room is on screen or chat floats over the video: progress
    /// belongs to the stream's own room.
    var visible = true
    private(set) var claiming = false

    /// Claims a drop (the reader's: the core, then a fresh read).
    @ObservationIgnored var claimer: (@MainActor (WatchDrop) async -> Void)?
    /// Shows a drop's campaign (the reader's: the player shrinks and Rewards opens).
    @ObservationIgnored var opener: (@MainActor (WatchDrop) -> Void)?

    /// The drop the chat stack shows, if any.
    var shown: WatchDrop? { visible ? drop : nil }

    func set(_ next: WatchDrop?) {
        if next != drop { drop = next }
    }

    func claim() async {
        guard let drop, !claiming, let claimer else { return }
        claiming = true
        defer { claiming = false }
        await claimer(drop)
    }

    func open() {
        if let drop { opener?(drop) }
    }
}

/// Keeps the watched stream's drop current: it reads the core once a minute while the app is in
/// front (the core only asks Twitch when the channel can earn something). It draws nothing; the
/// chat stack shows the drop.
struct WatchDropReader: View {
    let model: AppModel
    let drops: WatchDropModel
    /// The stream's own room is on screen, below the video.
    var visible = true
    /// The first answer for a stream is logged even when it is "nothing".
    @State private var reported: String?
    @Environment(\.scenePhase) private var phase

    private var stream: TwitchStream? { model.watch.stream }

    var body: some View {
        Color.clear
            .frame(width: 0, height: 0)
            .accessibilityHidden(true)
            .onChange(of: visible, initial: true) { _, visible in drops.visible = visible }
            .onAppear {
                drops.claimer = { [model, drops] drop in await Self.claim(drop, model: model, drops: drops) }
                drops.opener = { [model] drop in Self.jump(to: drop.campaignID, model: model) }
            }
            .task(id: "\(stream?.userLogin ?? "")|\(stream?.gameName ?? "")|\(phase == .active)") {
                drops.set(nil)
                guard stream != nil else { return }
                while !Task.isCancelled {
                    if phase == .active { await read() }
                    try? await Task.sleep(for: .seconds(60))
                }
            }
    }

    private func read() async {
        await Self.read(model: model, drops: drops, reported: reported) { reported = $0 }
    }

    private static func read(model: AppModel, drops: WatchDropModel, reported: String?,
                             report: (String) -> Void) async {
        guard let stream = model.watch.stream else { return }
        do {
            let next = try await model.core.invoke(
                "get_watch_drop",
                args: ["channelId": stream.userID ?? "", "login": stream.userLogin,
                       "gameId": stream.gameID ?? "", "gameName": stream.gameName],
                as: WatchDrop?.self)
            guard model.watch.stream?.userLogin == stream.userLogin else { return }
            if next != drops.drop || reported != stream.userLogin {
                report(stream.userLogin)
                log.notice("drop bar \(stream.userLogin, privacy: .public) game '\(stream.gameName, privacy: .public)' (\(stream.gameID ?? "-", privacy: .public)): \(next.map { "\($0.name) \($0.trailing)" } ?? "nothing", privacy: .public), visible \(drops.visible)")
            }
            drops.set(next)
        } catch {
            log.error("get_watch_drop failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The claim stays on the player; only the card's body goes to Rewards.
    private static func claim(_ drop: WatchDrop, model: AppModel, drops: WatchDropModel) async {
        do {
            _ = try await model.core.invoke("claim_drop", args: ["dropId": drop.dropID, "dropInstanceId": drop.instanceID])
            model.toasts.show("Drop claimed", .success)
            await read(model: model, drops: drops, reported: nil) { _ in }
        } catch {
            model.toasts.show("Could not claim that drop.")
            log.error("claim_drop failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The player shrinks to the mini window and Rewards shows the campaign.
    private static func jump(to campaignID: String, model: AppModel) {
        withAnimation(SNMotion.run(SNMotion.settleBack)) { model.nav.playerMode = .mini }
        model.rewards.focusCampaign = campaignID
        model.nav.tab = .rewards
    }
}

/// The drop as a card of the chat stack. Its head is the same row strip or open: the reward's art,
/// its name, its short status and the chevron, with a thin progress line along the strip's foot.
/// Open, the campaign line, the progress bar and Claim follow; a tap on that body shows the
/// campaign in Rewards, a tap on the head folds it.
struct DropCard: View {
    let model: LiveVotesModel
    let drops: WatchDropModel
    let drop: WatchDrop
    let open: Bool

    /// Rust words the minutes left; once the minutes are in it sends no words, so the strip
    /// says the drop is ready.
    private var status: String { drop.status }
    private var barColor: Color { drop.complete ? SN.success : SN.accentOnSurface }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            CardHead(open: open, label: "Drop", showsLabel: false, line: drop.name, lineWhenOpen: true,
                     action: { model.tap(.drop) }) {
                RewardArt(url: drop.image, width: 18, height: 18, radius: 4, ring: true)
            } status: {
                Text(status)
                    .font(.sn(12, drop.complete ? 600 : 500))
                    .monospacedDigit()
                    .foregroundStyle(drop.complete ? SN.success : SN.textSecondary)
                    .lineLimit(1)
            }
            CardBody(open: open) {
                details
            }
        }
        .modifier(VoteCardSurface())
        // The strip's progress: a thin line along its foot, under the head's text, never on it.
        .overlay(alignment: .bottom) {
            ProgressTrack(percent: drop.percent, height: 2, color: barColor)
                .padding(.horizontal, 12)
                .padding(.bottom, 3)
                .opacity(open ? 0 : 1)
                .allowsHitTesting(false)
        }
        .accessibilityElement(children: .contain)
    }

    private var details: some View {
        HStack(alignment: .center, spacing: 10) {
            Button {
                drops.open()
            } label: {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 4) {
                        Text(drop.line)
                            .font(.sn(12))
                            .foregroundStyle(SN.textMuted)
                            .lineLimit(1)
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(SN.textMuted)
                            .accessibilityHidden(true)
                        Spacer(minLength: 0)
                    }
                    ProgressTrack(percent: drop.percent, height: 4, color: barColor)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(drop.spoken)
            .accessibilityHint("Shows the campaign in Rewards")
            if drop.complete {
                Button {
                    Task { await drops.claim() }
                } label: {
                    Text(drops.claiming ? "…" : drop.claimLabel)
                        .font(.sn(12, 600))
                        .foregroundStyle(SN.success)
                        .padding(.horizontal, 12)
                        .frame(height: 26)
                        .background(Capsule().fill(SN.success.opacity(0.2)))
                }
                .buttonStyle(.plain)
                .disabled(drops.claiming)
                .opacity(drops.claiming ? 0.6 : 1)
            }
        }
    }
}
