#if canImport(SNCore)
import Foundation
@preconcurrency import SNCore
import SNKit

/// The Rust core through the bindings UniFFI generates for it.
final class RustCore: CoreBridge, @unchecked Sendable {
    let events = CoreEvents()
    let kind = "Rust core"

    func start(dirs: SNKit.CoreDirs, version: String, vaultKey: Data, codecs: [String]) throws {
        do {
            try SNCore.start(
                sink: Sink(events: events),
                dirs: SNCore.HostDirs(data: dirs.data, cache: dirs.cache, logs: dirs.logs),
                version: version, vaultKey: vaultKey, codecs: codecs
            )
        } catch let error as CoreError {
            throw error.failure
        }
    }

    func invoke(_ cmd: String, _ argsJSON: String) async throws -> String {
        do {
            return try await SNCore.invoke(cmd: cmd, argsJson: argsJSON)
        } catch let error as CoreError {
            throw error.failure
        }
    }

    func setWebCookies(origin: String, cookiesJSON: String) throws {
        SNCore.setWebCookies(origin: origin, cookiesJson: cookiesJSON)
    }

    func lifecycle(_ state: String) {
        SNCore.lifecycle(state: state)
    }

    /// Where the core calls back into the app. Rust calls it on its own threads.
    private final class Sink: HostSink, @unchecked Sendable {
        let events: CoreEvents

        init(events: CoreEvents) {
            self.events = events
        }

        func event(name: String, payloadJson: String, target: String?) {
            events.emit(name: name, payloadJSON: payloadJson, target: target)
        }

        func hostRequest(kind: String, payloadJson: String) {
            events.hostRequest(kind: kind, payloadJSON: payloadJson)
        }
    }
}

extension CoreError {
    var failure: CoreFailure {
        switch self {
        case .Command(let json): return .command(json: json)
        case .Start(let message): return .start(message: message)
        }
    }
}
#endif
