import Foundation
import Testing
@testable import SNKit

@Suite struct VaultKeyTests {
    /// A Keychain that answers one status and records what was written.
    final class FakeItem: VaultKeyPolicy.Store, @unchecked Sendable {
        var answer: VaultKeyPolicy.Read
        var added: [Data] = []
        var fixes = 0
        var addStatus: Int32 = VaultKeyPolicy.Status.success

        init(_ answer: VaultKeyPolicy.Read) { self.answer = answer }
        func read() -> VaultKeyPolicy.Read { answer }
        func add(_ key: Data) -> Int32 {
            added.append(key)
            return addStatus
        }
        func fixAccessibility() -> Int32 {
            fixes += 1
            return VaultKeyPolicy.Status.success
        }
    }

    private let kept = Data(repeating: 7, count: 32)
    private let fresh = Data(repeating: 9, count: 32)

    @Test func aKeptKeyIsUsedAndNothingIsWritten() throws {
        let item = FakeItem(.init(status: VaultKeyPolicy.Status.success, key: kept))
        #expect(try VaultKeyPolicy.load(item) { fresh } == kept)
        #expect(item.added.isEmpty)
        #expect(item.fixes == 0)
    }

    @Test func beforeTheFirstUnlockTheKeyIsWaitedForNeverReplaced() {
        let item = FakeItem(.init(status: VaultKeyPolicy.Status.interactionNotAllowed))
        #expect(throws: VaultKeyPolicy.Failure.locked) { try VaultKeyPolicy.load(item) { fresh } }
        #expect(item.added.isEmpty, "every sealed sign-in would be orphaned")
    }

    @Test func anyOtherKeychainFailureStopsTheStartWithoutWriting() {
        let item = FakeItem(.init(status: -34018))
        #expect(throws: VaultKeyPolicy.Failure.keychain(-34018)) { try VaultKeyPolicy.load(item) { fresh } }
        #expect(item.added.isEmpty)
    }

    @Test func onlyAKeyTheKeychainDoesNotHaveIsMade() throws {
        let item = FakeItem(.init(status: VaultKeyPolicy.Status.itemNotFound))
        #expect(try VaultKeyPolicy.load(item) { fresh } == fresh)
        #expect(item.added == [fresh])
    }

    @Test func aNewKeyTheKeychainRefusesIsNotUsed() {
        let item = FakeItem(.init(status: VaultKeyPolicy.Status.itemNotFound))
        item.addStatus = -25299
        #expect(throws: VaultKeyPolicy.Failure.keychain(-25299)) { try VaultKeyPolicy.load(item) { fresh } }
    }

    @Test func aKeyKeptUnderAnotherAccessClassMovesToAfterFirstUnlock() throws {
        let item = FakeItem(.init(status: VaultKeyPolicy.Status.success, key: kept, wrongAccessibility: true))
        #expect(try VaultKeyPolicy.load(item) { fresh } == kept)
        #expect(item.fixes == 1)
        #expect(item.added.isEmpty)
    }

    @Test func noRandomnessMeansNoKey() {
        let item = FakeItem(.init(status: VaultKeyPolicy.Status.itemNotFound))
        #expect(throws: VaultKeyPolicy.Failure.random) { try VaultKeyPolicy.load(item) { nil } }
        #expect(item.added.isEmpty)
    }

    @Test func sealedFilesAreToldByTheirHeader() {
        #expect(VaultKeyPolicy.isSealed(Data([0xA7, 0x53, 0x4E, 0x56, 1, 1])))
        #expect(!VaultKeyPolicy.isSealed(Data("{\"theme\":1}".utf8)))
        #expect(!VaultKeyPolicy.isSealed(Data([0xA7, 0x53])))
    }
}
