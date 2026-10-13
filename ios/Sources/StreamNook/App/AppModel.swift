import Foundation
import Observation
import os
import SNKit

let log = Logger(subsystem: "app.streamnook", category: "spike")

/// The app's state: the core, whether it started, and the Browse list.
@Observable @MainActor
final class AppModel {
    let core: CoreBridge = CoreHost.makeCore()
    let watch = Watch()

    private(set) var started = false
    private(set) var startError: String?
    private(set) var startMS: Int?
    private(set) var eventCount = 0
    private(set) var lastEvent: String?

    private(set) var streams: [TwitchStream] = []
    private(set) var loading = false
    private(set) var browseError: String?

    /// The pushed watch page, if any.
    var path: [TwitchStream] = []

    let version = CoreHost.appVersion

    /// Starts the core once, then loads Browse.
    func boot() async {
        guard !started, startError == nil else { return }
        listenForEvents()
        let core = core
        let clock = ContinuousClock.now
        do {
            let (dirs, key, codecs) = try CoreHost.startInputs()
            try await Task.detached(priority: .userInitiated) {
                try core.start(dirs: dirs, version: CoreHost.appVersion, vaultKey: key, codecs: codecs)
            }.value
            startMS = Int((ContinuousClock.now - clock) / .milliseconds(1))
            started = true
            log.notice("core started (\(core.kind, privacy: .public)) in \(self.startMS ?? 0) ms, codecs \(codecs.joined(separator: ","), privacy: .public)")
        } catch {
            startError = "\(error)"
            log.error("core start failed: \(String(describing: error), privacy: .public)")
            return
        }
        await browse()
    }

    func browse() async {
        guard started, !loading else { return }
        loading = true
        defer { loading = false }
        do {
            streams = try await Browse.load(core)
            browseError = nil
            log.notice("browse listed \(self.streams.count) streams")
        } catch {
            browseError = "\(error)"
            log.error("browse failed: \(String(describing: error), privacy: .public)")
        }
    }

    func lifecycle(_ state: String) {
        guard started else { return }
        core.lifecycle(state)
    }

    private func listenForEvents() {
        let events = core.events.all()
        Task { [weak self] in
            for await event in events {
                guard let self else { return }
                self.eventCount += 1
                self.lastEvent = event.name
            }
        }
    }
}
