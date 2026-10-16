import Foundation

/// The channel wallet's faces as the core's `channel_state` carries them (desktop dev
/// services/channel_state.rs, `ChannelPoints`, `ChannelBitsFace`, `WalletGain`). Every "+N" comes
/// from the core's `last_gain`, which counts a gain once whether the socket, a claim or the
/// minute poll saw it first; the app never works out a gain from two balances itself.
public struct WalletGain: Decodable, Sendable, Equatable {
    /// Grows with every gain, so two equal amounts in a row still float twice.
    public var seq: UInt64
    public var amount: Int64
    /// "for watching", "from the bonus chest"; nil when Twitch did not say.
    public var reason: String?
    /// A bonus chest.
    public var chest: Bool
}

/// Channel points on the watched channel.
public struct ChannelPoints: Decodable, Sendable, Equatable {
    /// False when the channel has points off or the account cannot earn here.
    public var enabled: Bool
    public var balance: Int64?
    /// The channel's own name for its points.
    public var name: String?
    public var iconURL: String?
    /// A bonus chest waiting to be claimed (only with auto-collect off).
    public var availableClaimID: String?
    public var lastGain: WalletGain?

    enum CodingKeys: String, CodingKey {
        case enabled, balance, name
        case iconURL = "icon_url"
        case availableClaimID = "available_claim_id"
        case lastGain = "last_gain"
    }

    /// The button's accessibility label: the channel's points name, else "Channel points".
    public var label: String { name.flatMap { $0.isEmpty ? nil : $0 } ?? "Channel points" }
}

/// Bits on the watched channel, for the wallet's face.
public struct ChannelBitsFace: Decodable, Sendable, Equatable {
    public var enabled: Bool
    /// The smallest single cheer here.
    public var pickMin: UInt32
    public var balance: Int64?
    public var gemURL: String?
    public var lastGain: WalletGain?

    enum CodingKeys: String, CodingKey {
        case enabled, balance
        case pickMin = "pick_min"
        case gemURL = "gem_url"
        case lastGain = "last_gain"
    }
}
