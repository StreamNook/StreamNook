import Foundation
import Synchronization

/// One event the core emitted. `target` names the window it was meant for, nil for all.
public struct CoreEvent: Sendable, Equatable {
    public var name: String
    public var payloadJSON: String
    public var target: String?

    public init(name: String, payloadJSON: String, target: String? = nil) {
        self.name = name
        self.payloadJSON = payloadJSON
        self.target = target
    }
}

/// Something the core asks the app to do for it (open a link, copy text, pick a file).
public struct HostRequest: Sendable, Equatable {
    public var kind: String
    public var payloadJSON: String

    public init(kind: String, payloadJSON: String) {
        self.kind = kind
        self.payloadJSON = payloadJSON
    }
}

/// Fans the core's single event callback out to everyone listening for a given name. Each
/// `stream(_:)` call gets its own stream; it stops receiving when its task ends.
public final class CoreEvents: Sendable {
    private struct Listeners {
        var byName: [String: [UUID: AsyncStream<CoreEvent>.Continuation]] = [:]
        var all: [UUID: AsyncStream<CoreEvent>.Continuation] = [:]
        var requests: [UUID: AsyncStream<HostRequest>.Continuation] = [:]
    }

    private let listeners = Mutex(Listeners())

    public init() {}

    /// Events with this name, from now on.
    public func stream(_ name: String) -> AsyncStream<CoreEvent> {
        let (stream, continuation) = AsyncStream<CoreEvent>.makeStream(bufferingPolicy: .bufferingNewest(256))
        let id = UUID()
        listeners.withLock { $0.byName[name, default: [:]][id] = continuation }
        continuation.onTermination = { [weak self] _ in
            self?.listeners.withLock { state in
                state.byName[name]?[id] = nil
                if state.byName[name]?.isEmpty == true { state.byName[name] = nil }
            }
        }
        return stream
    }

    /// Every event, whatever its name.
    public func all() -> AsyncStream<CoreEvent> {
        let (stream, continuation) = AsyncStream<CoreEvent>.makeStream(bufferingPolicy: .bufferingNewest(256))
        let id = UUID()
        listeners.withLock { $0.all[id] = continuation }
        continuation.onTermination = { [weak self] _ in
            self?.listeners.withLock { $0.all[id] = nil }
        }
        return stream
    }

    /// The core's requests to the app.
    public func requests() -> AsyncStream<HostRequest> {
        let (stream, continuation) = AsyncStream<HostRequest>.makeStream(bufferingPolicy: .bufferingNewest(64))
        let id = UUID()
        listeners.withLock { $0.requests[id] = continuation }
        continuation.onTermination = { [weak self] _ in
            self?.listeners.withLock { $0.requests[id] = nil }
        }
        return stream
    }

    /// How many streams listen for this name (or for every name when nil).
    public func listenerCount(_ name: String? = nil) -> Int {
        listeners.withLock { state in
            guard let name else { return state.all.count }
            return state.byName[name]?.count ?? 0
        }
    }

    /// Called from the core's sink, on any thread.
    public func emit(name: String, payloadJSON: String, target: String?) {
        let event = CoreEvent(name: name, payloadJSON: payloadJSON, target: target)
        let receivers = listeners.withLock { state in
            Array((state.byName[name] ?? [:]).values) + Array(state.all.values)
        }
        for receiver in receivers { receiver.yield(event) }
    }

    /// Called from the core's sink, on any thread.
    public func hostRequest(kind: String, payloadJSON: String) {
        let request = HostRequest(kind: kind, payloadJSON: payloadJSON)
        let receivers = listeners.withLock { Array($0.requests.values) }
        for receiver in receivers { receiver.yield(request) }
    }

    /// Ends every stream, for when the core goes away.
    public func finish() {
        let (events, requests) = listeners.withLock { state -> ([AsyncStream<CoreEvent>.Continuation], [AsyncStream<HostRequest>.Continuation]) in
            let events = state.byName.values.flatMap { $0.values } + state.all.values
            let requests = Array(state.requests.values)
            state = Listeners()
            return (events, requests)
        }
        events.forEach { $0.finish() }
        requests.forEach { $0.finish() }
    }
}
