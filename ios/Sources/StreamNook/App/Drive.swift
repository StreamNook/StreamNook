#if DEBUG
import Foundation

/// Steps named in the SN_DRIVE launch variable, run one after another once the core has
/// started, to bring the app to a given state from outside. Steps are separated by ";":
///
/// - `browse` loads the Browse list again and waits for it
/// - `play:INDEX` opens the watch page of the Browse list's stream at INDEX
/// - `back` closes the watch page
/// - `wait:SECONDS`
/// - `pause` pauses the player
/// - `route:NAME` picks how the next stream reaches the relay (`plain`, `promote`, `ll`,
///   `llhttp`; see `PlayRoute`)
/// - `offset:SECONDS` sets the next player item's configured offset from live
/// - `preserve` makes the next player item keep its offset from live after a stall
/// - `catchup:MS` holds that distance behind the relay's edge with a gentle catch-up rate
/// - `signin` opens the Twitch sign-in sheet
/// - `shot` marks the moment a screenshot is due (the host takes it) and holds still for five
///   seconds so it lands on a settled screen
@MainActor
enum Drive {
    static func run(model: AppModel) async {
        guard let script = ProcessInfo.processInfo.environment["SN_DRIVE"], !script.isEmpty else { return }
        guard model.started else {
            log.error("drive skipped: the core did not start")
            return
        }
        log.notice("drive steps \(script, privacy: .public)")
        for step in script.split(separator: ";").map({ $0.trimmingCharacters(in: .whitespaces) }) where !step.isEmpty {
            let (name, value) = split(step)
            switch name {
            case "browse": await model.browse()
            case "play":
                let index = Int(value) ?? 0
                guard model.streams.indices.contains(index) else {
                    log.error("drive play: no stream at \(index)")
                    continue
                }
                model.path = [model.streams[index]]
            case "back": model.path = []
            case "wait": try? await Task.sleep(for: .seconds(Double(value) ?? 1))
            case "pause": model.watch.pause()
            case "route": model.watch.route = PlayRoute(rawValue: value) ?? .plain
            case "offset": model.watch.offsetFromLive = Double(value)
            case "preserve": model.watch.preservesOffset = true
            case "catchup": model.watch.catchUpTargetMS = Int(value)
            case "signin": await model.signIn.begin(core: model.core)
            case "shot":
                log.notice("drive shot")
                try? await Task.sleep(for: .seconds(5))
            default: log.error("drive: unknown step \(name, privacy: .public)")
            }
            try? await Task.sleep(for: .milliseconds(400))
        }
        log.notice("drive done")
    }

    private static func split(_ step: String) -> (String, String) {
        guard let colon = step.firstIndex(of: ":") else { return (step, "") }
        return (String(step[..<colon]), String(step[step.index(after: colon)...]))
    }
}
#endif
