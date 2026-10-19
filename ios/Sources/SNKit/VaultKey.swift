import Foundation

/// The rules for the 32-byte key the core seals its token vault with, which the app keeps in the
/// Keychain. Kept apart from the Keychain itself so they can be tested without one.
///
/// Only a key the Keychain says is not there is ever made. Before the first unlock after a
/// restart the Keychain answers "not allowed" for a key that is there; making a new one then
/// would leave every sealed sign-in unopenable, and the core deletes what no key can open. So a
/// locked Keychain is waited out, and any other failure stops the start instead.
public enum VaultKeyPolicy {
    /// Keychain result codes the policy reads (the `OSStatus` values from Security).
    public enum Status {
        public static let success: Int32 = 0
        public static let itemNotFound: Int32 = -25300
        public static let interactionNotAllowed: Int32 = -25308
    }

    public enum Failure: Error, Equatable {
        /// The phone has not been unlocked since it started: try again once it has.
        case locked
        case keychain(Int32)
        case random
    }

    /// What one Keychain read found.
    public struct Read: Sendable {
        public var status: Int32
        public var key: Data?
        /// The item is readable only while the phone is unlocked (or some other class than
        /// after-first-unlock, this device only), so it moves to that class.
        public var wrongAccessibility: Bool

        public init(status: Int32, key: Data? = nil, wrongAccessibility: Bool = false) {
            self.status = status
            self.key = key
            self.wrongAccessibility = wrongAccessibility
        }
    }

    /// The Keychain item as the policy uses it.
    public protocol Store {
        func read() -> Read
        /// Writes `key` as the only vault key, readable after the first unlock on this device only.
        func add(_ key: Data) -> Int32
        /// Moves the existing item to after-first-unlock, this device only.
        func fixAccessibility() -> Int32
    }

    /// The key to start the core with: the one kept, or a new one when none is kept.
    public static func load(_ store: some Store, random: () -> Data?) throws -> Data {
        let found = store.read()
        switch found.status {
        case Status.success:
            guard let key = found.key, key.count == 32 else {
                // A damaged item opens nothing it sealed; a new key is the only way on.
                return try make(store, random: random)
            }
            if found.wrongAccessibility { _ = store.fixAccessibility() }
            return key
        case Status.itemNotFound:
            return try make(store, random: random)
        case Status.interactionNotAllowed:
            throw Failure.locked
        default:
            throw Failure.keychain(found.status)
        }
    }

    private static func make(_ store: some Store, random: () -> Data?) throws -> Data {
        guard let key = random(), key.count == 32 else { throw Failure.random }
        let status = store.add(key)
        guard status == Status.success else { throw Failure.keychain(status) }
        return key
    }

    /// Whether a file's first bytes are a sealed credential's header (the core's vault format).
    public static func isSealed(_ head: Data) -> Bool {
        head.count >= 4 && head.prefix(4) == Data([0xA7, 0x53, 0x4E, 0x56])
    }
}
