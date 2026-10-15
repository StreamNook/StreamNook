import CoreMedia
import Foundation
import Security
import SNKit
import VideoToolbox

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
        // `proxy:http://HOST:PORT` sends the core's upstream traffic through a proxy, so a
        // shaping proxy on the desk can play a cellular link for the playback tests. Set before
        // the core builds its first HTTP client, which is when it reads the variable.
        if let proxy = steps.first(where: { $0.hasPrefix("proxy:") })?.dropFirst("proxy:".count) {
            setenv("HTTPS_PROXY", String(proxy), 1)
            setenv("NO_PROXY", "127.0.0.1,localhost", 1)
        }
        #endif
        #if canImport(SNCore)
        return RustCore()
        #else
        return FakeCore(fixtures: FakeFixtures(signedIn: false))
        #endif
    }

    static func startInputs() throws -> (CoreDirs, Data, [String]) {
        (try dirs(), try VaultKey.load(), codecs())
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

/// The 32-byte key the core seals its token vault with, made once and kept in the keychain.
enum VaultKey {
    private static let service = "app.streamnook.vault"
    private static let account = "core"

    enum Failure: Error {
        case keychain(OSStatus)
        case random(OSStatus)
    }

    static func load() throws -> Data {
        if let key = read(), key.count == 32 { return key }
        var bytes = [UInt8](repeating: 0, count: 32)
        let made = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        guard made == errSecSuccess else { throw Failure.random(made) }
        let key = Data(bytes)
        try save(key)
        return key
    }

    private static func read() -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess else { return nil }
        return result as? Data
    }

    private static func save(_ key: Data) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
        var item = query
        item[kSecValueData as String] = key
        // Readable after the first unlock, so the core can open its vault while the phone is
        // locked and a stream plays in the background.
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw Failure.keychain(status) }
    }
}
