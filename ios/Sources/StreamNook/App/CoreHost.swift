import CoreMedia
import Foundation
import os
import Security
import SNKit
import UIKit
import VideoToolbox

private let log = Logger(subsystem: "app.streamnook", category: "vault")

/// What the app hands the core when it starts: where its files go, the key that seals its
/// token vault, and the codecs this phone decodes in hardware.
enum CoreHost {
    static let appVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"

    /// The Rust core when it is linked into this build, FakeCore otherwise. A DEBUG build launched
    /// with SN_FAKE=1, or with a `fake` or `fake:signedin` drive step, uses FakeCore either way,
    /// answering the screens from fixtures.
    static func makeCore() -> CoreBridge {
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        let steps = (environment["SN_DRIVE"] ?? "").split(separator: ";").map { $0.trimmingCharacters(in: .whitespaces) }
        if environment["SN_FAKE"] == "1" || steps.contains("fake") || steps.contains("fake:signedin") {
            return FakeCore(fixtures: FakeFixtures(signedIn: steps.contains("fake:signedin")))
        }
        #endif
        #if canImport(SNCore)
        return RustCore()
        #else
        return FakeCore(fixtures: FakeFixtures(signedIn: false))
        #endif
    }

    /// Waits out a Keychain that is locked because the phone has not been unlocked since it
    /// started (a launch in the background then): the key is there, just unreadable until then.
    static func startInputs() async throws -> (CoreDirs, Data, [String]) {
        var waited = false
        while true {
            do {
                let key = try VaultKey.load()
                if waited { log.notice("vault key read after the first unlock") }
                return (try dirs(), key, codecs())
            } catch VaultKeyPolicy.Failure.locked {
                if !waited { log.notice("vault key locked until the first unlock; waiting") }
                waited = true
                await VaultKey.waitForUnlock()
            }
        }
    }

    /// Keeps sealed sign-ins out of iCloud and computer backups. The core flags each file as it
    /// writes it; this checks every one with the system's own call (a file an older build wrote,
    /// or a flag the system did not take) and flags any it missed.
    static func excludeSealedFromBackup() {
        guard let support = try? dirs().data else { return }
        let dir = URL(filePath: support, directoryHint: .isDirectory)
        let files = (try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: [.isExcludedFromBackupKey],
                                                                   options: [])) ?? []
        var sealed = 0, flagged = 0, failed = 0
        for file in files {
            guard let handle = try? FileHandle(forReadingFrom: file) else { continue }
            let head = (try? handle.read(upToCount: 4)) ?? Data()
            try? handle.close()
            guard VaultKeyPolicy.isSealed(head) || file.lastPathComponent == ".token_vault_key" else { continue }
            sealed += 1
            if (try? file.resourceValues(forKeys: [.isExcludedFromBackupKey]))?.isExcludedFromBackup == true { continue }
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            var url = file
            do {
                try url.setResourceValues(values)
                flagged += 1
            } catch {
                failed += 1
            }
        }
        if sealed > 0 {
            log.notice("backup: \(sealed) sealed files, \(sealed - flagged - failed) already excluded, \(flagged) excluded now, \(failed) failed")
        }
    }

    /// Data in Application Support (backed up), caches and logs in Caches (never backed up).
    static func dirs() throws -> CoreDirs {
        let files = FileManager.default
        let support = try files.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appending(path: "app.streamnook", directoryHint: .isDirectory)
        let caches = try files.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appending(path: "app.streamnook", directoryHint: .isDirectory)
        let logs = caches.appending(path: "logs", directoryHint: .isDirectory)
        for url in [support, caches, logs] {
            try files.createDirectory(at: url, withIntermediateDirectories: true)
        }
        return CoreDirs(data: support.path(percentEncoded: false), cache: caches.path(percentEncoded: false),
                        logs: logs.path(percentEncoded: false))
    }

    /// H.264 always; HEVC and AV1 only where the phone decodes them in hardware.
    static func codecs() -> [String] {
        var codecs = ["h264"]
        if VTIsHardwareDecodeSupported(kCMVideoCodecType_HEVC) { codecs.append("hevc") }
        if VTIsHardwareDecodeSupported(kCMVideoCodecType_AV1) { codecs.append("av1") }
        return codecs
    }
}

/// The 32-byte key the core seals its token vault with, made once and kept in the Keychain. When
/// one is made and when a read waits are `VaultKeyPolicy`'s rules.
enum VaultKey {
    static func load() throws -> Data {
        try VaultKeyPolicy.load(KeychainItem()) {
            var bytes = [UInt8](repeating: 0, count: 32)
            guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { return nil }
            return Data(bytes)
        }
    }

    /// Returns once the phone has been unlocked, or after a few seconds so the caller reads the
    /// Keychain again (a notice missed while suspended must not leave the start waiting forever).
    static func waitForUnlock() async {
        let gate = UnlockGate()
        await withCheckedContinuation { gate.arm($0) }
    }

    /// Resumes its waiter once: on the unlock notice or after three seconds, whichever is first.
    private final class UnlockGate: @unchecked Sendable {
        private let lock = NSLock()
        private var waiter: CheckedContinuation<Void, Never>?
        private var observer: NSObjectProtocol?

        func arm(_ waiter: CheckedContinuation<Void, Never>) {
            lock.withLock { self.waiter = waiter }
            let observer = NotificationCenter.default.addObserver(
                forName: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil, queue: nil
            ) { [self] _ in fire() }
            // A notice that landed before the observer was kept has already resumed the waiter.
            let fired = lock.withLock {
                if self.waiter == nil { return true }
                self.observer = observer
                return false
            }
            if fired { NotificationCenter.default.removeObserver(observer) }
            DispatchQueue.global().asyncAfter(deadline: .now() + 3) { [self] in fire() }
        }

        private func fire() {
            let (waiter, observer) = lock.withLock {
                defer { self.waiter = nil; self.observer = nil }
                return (self.waiter, self.observer)
            }
            if let observer { NotificationCenter.default.removeObserver(observer) }
            waiter?.resume()
        }
    }

    /// The one Keychain item, readable after the first unlock on this device only: the core opens
    /// its vault while the phone is locked and a stream plays in the background, and the key never
    /// leaves the phone in a backup.
    private struct KeychainItem: VaultKeyPolicy.Store {
        private var query: [String: Any] {
            [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: "app.streamnook.vault",
                kSecAttrAccount as String: "core",
            ]
        }

        func read() -> VaultKeyPolicy.Read {
            var ask = query
            ask[kSecReturnData as String] = true
            ask[kSecReturnAttributes as String] = true
            ask[kSecMatchLimit as String] = kSecMatchLimitOne
            var result: AnyObject?
            let status = SecItemCopyMatching(ask as CFDictionary, &result)
            guard status == errSecSuccess, let item = result as? [String: Any] else { return .init(status: status) }
            let accessible = item[kSecAttrAccessible as String] as? String
            return .init(status: status, key: item[kSecValueData as String] as? Data,
                         wrongAccessibility: accessible != (kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String))
        }

        func add(_ key: Data) -> Int32 {
            SecItemDelete(query as CFDictionary)
            var item = query
            item[kSecValueData as String] = key
            item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            return SecItemAdd(item as CFDictionary, nil)
        }

        func fixAccessibility() -> Int32 {
            let change = [kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
            return SecItemUpdate(query as CFDictionary, change as CFDictionary)
        }
    }
}
