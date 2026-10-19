import Foundation

/// The drop being earned on the watched stream, as `get_watch_drop` answers it
/// (services/watch_drop.rs). Nil from the core means there is nothing to show.
public struct WatchDrop: Decodable, Sendable, Equatable {
    public var campaignID: String
    public var dropID: String
    public var instanceID: String?
    public var name: String
    public var image: String
    /// "12m left" or "Ready to claim".
    public var trailing: String
    /// The strip's few words: "12m to watch", or "Ready" once the minutes are in.
    public var status: String
    /// "Campaign · 2 of 5".
    public var line: String
    public var percent: Double
    public var complete: Bool
    /// "Claim" or "Open".
    public var claimLabel: String
    public var spoken: String

    enum CodingKeys: String, CodingKey {
        case name, image, trailing, status, line, percent, complete, spoken
        case campaignID = "campaign_id"
        case dropID = "drop_id"
        case instanceID = "instance_id"
        case claimLabel = "claim_label"
    }
}
