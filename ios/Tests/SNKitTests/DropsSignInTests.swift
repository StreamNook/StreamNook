import Foundation
import Testing
@testable import SNKit

/// The first-run wizard's drops step and the Rewards tab sign in the same way: the core opens the
/// grant and announces `drops-login-complete`. The fake core must do both, or a FakeCore run of
/// the wizard sticks on "Waiting for Twitch".
@Suite struct DropsSignInTests {
    private func core() throws -> FakeCore {
        let core = FakeCore(fixtures: FakeFixtures(signedIn: true))
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "test", vaultKey: Data(count: 32), codecs: [])
        return core
    }

    @Test func aDropsSignInGrantsAndAnnouncesItself() async throws {
        let core = try core()
        // A fresh install: no drops sign-in yet.
        core.fixtures?.rewards.connected = false
        #expect(try await core.invoke("is_drops_authenticated", args: [:]) == "false")
        let outcomes = core.events.stream("drops-login-complete")
        _ = try await core.invoke("open_drops_sheet", args: [:])
        let announced = await withTaskGroup(of: Bool.self) { group in
            group.addTask {
                for await _ in outcomes { return true }
                return false
            }
            group.addTask {
                try? await Task.sleep(for: .seconds(3))
                return false
            }
            let first = await group.next() ?? false
            group.cancelAll()
            return first
        }
        #expect(announced, "the wizard and Rewards both wait for this event")
        #expect(try await core.invoke("is_drops_authenticated", args: [:]) == "true")
        let drops = try await core.invoke("get_rewards_drops", args: ["query": "", "refresh": false], as: RewardsDrops.self)
        #expect(drops.connected, "the wizard's done page reads this")
    }
}
