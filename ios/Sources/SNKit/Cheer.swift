import Foundation

/// A typed message that cheers, as Rust checks it (`check_cheer`, and the `cheer` a held
/// `send_chat_message` hands back): what it spends and anything in the way.
public struct CheerCheck: Decodable, Sendable, Equatable {
    /// Bare lowercase login.
    public var channel: String
    public var channelID: String
    public var text: String
    public var total: Int
    /// Nil when the twitch.tv session could not be read.
    public var balance: Int64?
    /// Why it cannot go out as it is. Empty: ready to cheer.
    public var problems: [String]
    /// The balance is what stands in the way.
    public var needsBits: Bool
    /// Twitch's animated Cheer gem for this total.
    public var iconURL: String?
    /// The message with its cheers taken out.
    public var plainText: String

    enum CodingKeys: String, CodingKey {
        case channel, text, total, balance, problems
        case channelID = "channel_id"
        case needsBits = "needs_bits"
        case iconURL = "icon_url"
        case plainText = "plain_text"
    }

    public init(channel: String, channelID: String, text: String, total: Int, balance: Int64?, problems: [String] = [],
                needsBits: Bool = false, iconURL: String? = nil, plainText: String = "") {
        self.channel = channel
        self.channelID = channelID
        self.text = text
        self.total = total
        self.balance = balance
        self.problems = problems
        self.needsBits = needsBits
        self.iconURL = iconURL
        self.plainText = plainText
    }

    /// Bits left after it, when the balance is known and covers it.
    public var left: Int64? {
        guard let balance, balance >= Int64(total) else { return nil }
        return balance - Int64(total)
    }
}

/// What became of a cheer (`send_cheer`).
public struct CheerOutcome: Decodable, Sendable, Equatable {
    public var sent: Bool
    public var balance: Int64?
    public var problem: String?
    /// Twitch would send it without the message: the viewer may continue or edit.
    public var canContinue: Bool
    public var needsBits: Bool

    enum CodingKeys: String, CodingKey {
        case sent, balance, problem
        case canContinue = "can_continue"
        case needsBits = "needs_bits"
    }

    public init(sent: Bool, balance: Int64? = nil, problem: String? = nil, canContinue: Bool = false, needsBits: Bool = false) {
        self.sent = sent
        self.balance = balance
        self.problem = problem
        self.canContinue = canContinue
        self.needsBits = needsBits
    }
}

/// One cheer from the box to Twitch. While typing, `draft` is Rust's live check. Send goes
/// through Rust, which holds any message that cheers; when the bar already showed that very
/// message with nothing in the way, the hold is the viewer's yes and it goes out at once.
/// Otherwise the bar asks once, or says what stopped it and gives the text back.
public struct CheerFlow: Sendable, Equatable {
    public enum Status: Sendable, Equatable {
        /// Waiting for the viewer's yes.
        case held(CheerCheck)
        case sending(CheerCheck)
        /// It did not go out: why, and whether Twitch would take it without the message.
        case refused(CheerCheck, problem: String, canContinue: Bool, needsBits: Bool)

        public var check: CheerCheck {
            switch self {
            case .held(let c), .sending(let c): c
            case .refused(let c, _, _, _): c
            }
        }
    }

    /// What to do with a held message.
    public enum Next: Sendable, Equatable {
        /// Send it now (the bar already showed it).
        case send(CheerCheck)
        /// Ask first.
        case ask
        /// It cannot go out: put this text back in the box.
        case restore(String)
    }

    public var draft: CheerCheck?
    public var status: Status?

    public init() {}

    /// The bar's cheer: the status when there is one, else the live check.
    public var shown: CheerCheck? { status?.check ?? draft }

    /// Rust held `check` on Send.
    public mutating func held(_ check: CheerCheck) -> Next {
        if let problem = check.problems.first {
            status = .refused(check, problem: problem, canContinue: false, needsBits: check.needsBits)
            return .restore(check.text)
        }
        if let seen = draft, seen.text.trimmingCharacters(in: .whitespacesAndNewlines)
            == check.text.trimmingCharacters(in: .whitespacesAndNewlines), seen.problems.isEmpty {
            status = .sending(check)
            return .send(check)
        }
        status = .held(check)
        return .ask
    }

    /// The viewer said yes (or continue without the message).
    public mutating func sending(_ check: CheerCheck) {
        status = .sending(check)
    }

    /// Twitch answered. Returns the text to put back in the box when it did not go out.
    public mutating func answered(_ outcome: CheerOutcome, for check: CheerCheck) -> String? {
        if outcome.sent {
            status = nil
            draft = nil
            return nil
        }
        var shown = check
        if let balance = outcome.balance { shown.balance = balance }
        status = .refused(shown, problem: outcome.problem ?? "Your cheer of \(check.total.formatted()) bits didn't go out.",
                          canContinue: outcome.canContinue, needsBits: outcome.needsBits)
        return check.text
    }

    public mutating func dismiss() {
        status = nil
    }
}

/// The Bits tab's picker for a channel (`get_bits_state`): the amounts one cheermote can carry
/// here, ready-made amounts, the balance, and the cheermotes with their tiers.
public struct BitsState: Decodable, Sendable, Equatable {
    public var enabled: Bool
    public var pickMin: Int
    public var pickMax: Int
    public var quickAmounts: [Int]
    public var balance: Int64?
    public var cheermotes: [CheermoteChoice]
    public var icons: [CheermoteAmount]

    enum CodingKeys: String, CodingKey {
        case enabled, balance, cheermotes, icons
        case pickMin = "pick_min"
        case pickMax = "pick_max"
        case quickAmounts = "quick_amounts"
    }
}

public struct CheermoteChoice: Decodable, Sendable, Equatable, Identifiable {
    /// As Twitch writes it ("Cheer"); the inserted word is prefix + amount.
    public var prefix: String
    public var custom: Bool
    public var tiers: [CheermoteAmount]

    public var id: String { prefix }

    /// The tier Twitch draws for an amount: the highest one it reaches.
    public func tier(for bits: Int) -> CheermoteAmount? {
        let n = max(1, bits)
        return tiers.last { $0.bits <= n } ?? tiers.first
    }
}

public struct CheermoteAmount: Decodable, Sendable, Equatable {
    public var bits: Int
    public var url: String
    public var color: String
}
