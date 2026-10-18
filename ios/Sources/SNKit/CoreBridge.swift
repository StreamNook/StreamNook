import Foundation

/// The folders the core keeps its files in.
public struct CoreDirs: Sendable, Equatable {
    /// Settings, accounts and the token vault.
    public var data: String
    /// Image and playlist caches the system may clear.
    public var cache: String
    /// The core's log files.
    public var logs: String

    public init(data: String, cache: String, logs: String) {
        self.data = data
        self.cache = cache
        self.logs = logs
    }
}

/// What a core call failed with.
public enum CoreFailure: Error, Equatable, CustomStringConvertible {
    /// A command answered with an error; `json` is the error the command returned.
    case command(json: String)
    /// The core could not start.
    case start(message: String)

    public var description: String {
        switch self {
        case .command(let json): return Self.readable(json)
        case .start(let message): return "Core did not start: \(message)"
        }
    }

    /// A command error as a person reads it: a JSON string unquoted, an object's `error` or
    /// `message`, or the raw text.
    static func readable(_ json: String) -> String {
        guard let data = json.data(using: .utf8),
              let value = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
        else { return json }
        if let text = value as? String { return text }
        if let object = value as? [String: Any] {
            for key in ["error", "message"] {
                if let text = object[key] as? String { return text }
            }
        }
        return json
    }
}

/// The Rust core as the app sees it: the same commands the desktop and Android apps invoke, and
/// the events it emits. The real core and FakeCore both answer it.
public protocol CoreBridge: AnyObject, Sendable {
    /// Where the core's events and host requests arrive.
    var events: CoreEvents { get }
    /// A short name for the status line: "Rust core" or "FakeCore".
    var kind: String { get }

    /// Starts the core once, before any other call. `codecs` lists the video codecs this phone
    /// decodes in hardware.
    func start(dirs: CoreDirs, version: String, vaultKey: Data, codecs: [String]) throws
    /// Runs a command by its name with JSON arguments (named as the TypeScript call sites name
    /// them) and returns its JSON answer.
    func invoke(_ cmd: String, _ argsJSON: String) async throws -> String
    /// Hands the core the cookies a web sign-in left for `origin`.
    func setWebCookies(origin: String, cookiesJSON: String) throws
    /// Tells the core the app moved to "foreground", "background" or "terminating".
    func lifecycle(_ state: String)
}

extension CoreBridge {
    /// Runs a command with arguments encoded from a dictionary.
    public func invoke(_ cmd: String, args: [String: Any?]) async throws -> String {
        let clean = args.mapValues { $0 ?? NSNull() }
        let data = try JSONSerialization.data(withJSONObject: clean, options: [.sortedKeys])
        return try await invoke(cmd, String(decoding: data, as: UTF8.self))
    }

    /// Runs a command and decodes its answer.
    public func invoke<T: Decodable>(_ cmd: String, args: [String: Any?], as type: T.Type) async throws -> T {
        let json = try await invoke(cmd, args: args)
        return try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }
}
