import Foundation
import Observation
import SNKit
import Synchronization

/// The Atmosphere catalog the core serves (each look as layers), readable from any thread: chat
/// rows are laid out on the chat queue and look their author's Atmosphere up here by id.
final class AtmosphereCatalog: Sendable {
    static let shared = AtmosphereCatalog()

    private let looks = Mutex<[String: AtmosphereLook]>([:])

    /// The look for an Atmosphere id; a Cologne id with add-on suffixes finds its base row.
    func look(_ id: String?) -> AtmosphereLook? {
        guard let id, !id.isEmpty else { return nil }
        return looks.withLock { $0[id] ?? $0[String(id.split(separator: "+").first ?? "")] }
    }

    func replace(_ atmospheres: [Atmosphere]) {
        let map = Dictionary(atmospheres.map { ($0.id, $0.look) }, uniquingKeysWith: { a, _ in a })
        looks.withLock { $0 = map }
    }
}

/// Profiles and cosmetics state that outlives any one screen: the Atmosphere catalog, the Cosmetics
/// screen's open sections (kept for the session, as the phone keeps its screen mounted), and the
/// profile sheet being shown.
@Observable @MainActor
final class CosmeticsStore {
    private(set) var atmospheres: [Atmosphere] = []
    /// The Cosmetics screen's collapsible sections that are open.
    var openSections: Set<String> = []
    /// The profile sheet on screen, nil when none.
    var profile: ProfileTarget?
    /// A section of the Cosmetics screen to scroll to (drive steps).
    var scrollTarget: String?
    /// The Cosmetics screen while it is open (drive steps reach its taps through it).
    @ObservationIgnored weak var activeScreen: CosmeticsScreenModel?
    /// Bumped when the registries change, so open surfaces re-read.
    private(set) var registryVersion = 0
    @ObservationIgnored private var core: CoreBridge?
    @ObservationIgnored private var started = false

    /// Reads the catalog and follows the core's registry changes. Once, after the core starts.
    func start(core: CoreBridge) {
        guard !started else { return }
        started = true
        self.core = core
        Task { await loadAtmospheres() }
        Task { [weak self] in
            for await _ in core.events.stream("sn-registry-changed") {
                guard let self else { return }
                self.registryVersion += 1
                await self.loadAtmospheres()
            }
        }
    }

    func loadAtmospheres() async {
        guard let core else { return }
        do {
            let list = try await core.invoke("sn_atmospheres", args: [:], as: [Atmosphere].self)
            AtmosphereCatalog.shared.replace(list)
            atmospheres = list
        } catch {
            log.error("sn_atmospheres failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// Shows the profile sheet for `target` (Twitch rooms only; the caller checks). A target known
    /// only by login (an `@mention`) is resolved to its id first; one that cannot be resolved opens
    /// nothing.
    func open(_ target: ProfileTarget) {
        guard target.userID.isEmpty else {
            profile = target
            return
        }
        guard let core else { return }
        Task {
            do {
                let user = try await core.invoke("get_user_by_login", args: ["login": target.login], as: TwitchUser.self)
                var resolved = target
                resolved.userID = user.id
                resolved.displayName = user.displayName
                profile = resolved
            } catch {
                log.warning("mention \(target.login, privacy: .public) did not resolve: \(String(describing: error), privacy: .public)")
            }
        }
    }
}
