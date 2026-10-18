import SNKit
import SwiftUI

/// Subscribe beside the channel's name in the player, for Twitch and Kick: "Subscribe",
/// "Resubscribe" after a past sub, "Gift Subs" while subscribed, with the viewer's own sub badge
/// (else a star). It opens the platform's own checkout in the web sheet, as the signed-in account.
/// Short on room it drops out whole, never truncated, and the name keeps its place.
struct SubscribePill: View {
    let model: AppModel
    let stream: TwitchStream
    /// Asked for only when no open chat holds this channel's relationship.
    @State private var fetched: ChannelRelationship?

    /// Twitch's answer for this channel (subscribed, past subs, the badge), as the stream's chat
    /// holds it; Kick's comes from the Kick follow list.
    private var relationship: ChannelRelationship? {
        model.chat.relationship(for: stream.userLogin) ?? fetched
    }

    private var subscribed: Bool {
        stream.isTwitch ? relationship?.subscribed == true : model.kick.subscribed(to: stream.userLogin)
    }

    private var label: String {
        if subscribed { return "Gift Subs" }
        if stream.isTwitch, relationship?.hasSubHistory == true { return "Resubscribe" }
        return "Subscribe"
    }

    var body: some View {
        // The whole pill or nothing: a pill cut to "Subsc..." says less than none.
        ViewThatFits(in: .horizontal) {
            pill
            Color.clear.frame(width: 0, height: 0)
        }
        .task(id: stream.rowKey) { await load() }
    }

    private var pill: some View {
        Button(action: subscribe) {
            HStack(spacing: 4) {
                if stream.isTwitch, let badge = relationship?.badgeURL, !badge.isEmpty {
                    EmoteThumb(url: badge, height: 14)
                        .accessibilityHidden(true)
                } else {
                    Image(systemName: "star.fill")
                        .font(.system(size: 10, weight: .semibold))
                        .frame(width: 12, height: 12)
                        .accessibilityHidden(true)
                }
                Text(label)
                    .font(.sn(11.5, 600))
                    .lineLimit(1)
            }
            .foregroundStyle(.white)
            .padding(.leading, 6)
            .padding(.trailing, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(.white.opacity(0.15)))
            .fixedSize()
            // A finger-sized target around the small pill; the tap is the pill's alone.
            .padding(.vertical, 8)
            .contentShape(Rectangle())
            .padding(.vertical, -8)
        }
        .buttonStyle(PillPress())
        .accessibilityLabel("\(label), \(stream.userName)")
    }

    /// Twitch's relationship to the channel when the stream's chat does not hold it yet.
    private func load() async {
        fetched = nil
        guard stream.isTwitch, model.signedIn, let id = stream.twitchID, !id.isEmpty,
              model.chat.relationship(for: stream.userLogin) == nil else { return }
        fetched = try? await model.core.invoke("channel_relationship", args: [
            "channelId": id, "channelLogin": stream.userLogin,
        ], as: ChannelRelationship.self)
    }

    private func subscribe() {
        let core = model.core
        let (login, title, provider) = (stream.userLogin, "Subscribe to \(stream.userName)", stream.platform)
        Task {
            do {
                _ = try await core.invoke("open_subscribe_window", args: [
                    "channelLogin": login, "title": title, "provider": provider,
                ])
            } catch {
                log.error("open_subscribe_window failed: \(String(describing: error), privacy: .public)")
            }
        }
    }
}

/// Presses to 95%, as the phone's pill does.
private struct PillPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.95 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}
