import Foundation

/// The look a chat room draws with, kept current: read from the core when the room opens, then
/// again each time the core announces a settings change, so a choice made on the Chat page
/// restyles the rooms already open instead of waiting for them to be opened again.
public enum ChatStyleFeed {
    /// What the core emits after every settings write (field edits, resets, list edits, restores).
    public static let settingsChanged = "streamnook-settings-updated"

    /// The room's style now, then again after each settings change that alters it.
    public static func styles(core: CoreBridge) -> AsyncStream<ChatStyle> {
        // Listening before the first read, so a change that lands during it is not missed.
        let changes = core.events.stream(settingsChanged)
        return AsyncStream { continuation in
            let task = Task {
                var last: ChatStyle?
                func read() async {
                    guard let json = try? await core.invoke("get_settings", "{}") else { return }
                    let style = ChatStyle(settingsJSON: Data(json.utf8))
                    guard style != last else { return }
                    last = style
                    continuation.yield(style)
                }
                await read()
                for await _ in changes {
                    await read()
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
