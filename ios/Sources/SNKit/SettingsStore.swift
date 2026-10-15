import Foundation
import Observation

/// Reading a settings object by dotted path (`chat_design.font_size`), and whether a row's paths
/// are among the ones the core says differ from a fresh install.
public enum SettingsPath {
    /// The value at `path`, or nil when a step is missing or null.
    public static func value(in root: [String: Any], _ path: String) -> Any? {
        var node: Any = root
        for step in path.split(separator: ".") {
            guard let object = node as? [String: Any], let next = object[String(step)] else { return nil }
            node = next
        }
        return node is NSNull ? nil : node
    }

    /// A row differs from the default when one of its paths is a changed leaf, or holds one
    /// (a group of toggles), or sits inside one (a list stored as a single leaf).
    public static func isChanged(_ changed: Set<String>, _ paths: [String]) -> Bool {
        paths.contains { path in
            changed.contains(path) || changed.contains { $0.hasPrefix(path + ".") || path.hasPrefix($0 + ".") }
        }
    }

    /// A number read as a Double, whatever JSON gave it.
    public static func double(_ value: Any?) -> Double? {
        // JSON booleans arrive as NSNumbers too; their type code is "c".
        guard let number = value as? NSNumber, String(cString: number.objCType) != "c" else { return nil }
        return number.doubleValue
    }
}

/// One edit to a settings list, in the shape the core's `edit_settings_list` reads.
public enum SettingsListEdit {
    case add(Any? = nil)
    case remove(id: String)
    case removeAt(Int)
    case removeValue(Any)
    case update(id: String, [String: Any?])
    case updateAt(Int, [String: Any?])
    case move(id: String, to: Int)
    case toggle(Any)

    var json: [String: Any] {
        switch self {
        case .add(let item): item.map { ["op": "add", "item": $0] } ?? ["op": "add"]
        case .remove(let id): ["op": "remove", "id": id]
        case .removeAt(let index): ["op": "remove", "index": index]
        case .removeValue(let value): ["op": "remove", "value": value]
        case .update(let id, let patch): ["op": "update", "id": id, "patch": patch.mapValues { $0 ?? NSNull() }]
        case .updateAt(let index, let patch): ["op": "update", "index": index, "patch": patch.mapValues { $0 ?? NSNull() }]
        case .move(let id, let to): ["op": "move", "id": id, "to": to]
        case .toggle(let value): ["op": "toggle", "value": value]
        }
    }
}

/// The settings the settings pages draw from and write through. Values come from the core's
/// `get_settings_view` (the frontend-owned groups already filled with their defaults), every
/// write is a field or list edit the core applies to the copy it holds, and the reset arrows
/// follow the core's list of paths that differ from a fresh install. A write shows at once
/// and is replaced by the core's answer when it lands.
@Observable @MainActor
public final class SettingsStore {
    public private(set) var values: [String: Any] = [:]
    public private(set) var changed: Set<String> = []
    public private(set) var loaded = false
    /// What a write in flight shows until the core's view replaces it.
    private var pending: [String: Any] = [:]
    @ObservationIgnored private let core: CoreBridge
    @ObservationIgnored private var refreshing: Task<Void, Never>?
    /// Called with the readable error when a write fails.
    @ObservationIgnored public var onError: ((String) -> Void)?

    public init(core: CoreBridge) {
        self.core = core
    }

    /// Reads the core's view again. Concurrent asks share one read.
    public func refresh() async {
        if let refreshing {
            await refreshing.value
            return
        }
        let task = Task { await self.read() }
        refreshing = task
        await task.value
        refreshing = nil
    }

    private func read() async {
        do {
            let json = try await core.invoke("get_settings_view", "{}")
            guard let view = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else { return }
            apply(view)
        } catch {
            onError?("\(error)")
        }
    }

    /// Takes a `get_settings_view` answer.
    public func apply(_ view: [String: Any]) {
        values = view["values"] as? [String: Any] ?? [:]
        changed = Set(view["changed"] as? [String] ?? [])
        loaded = true
    }

    public func value(_ path: String) -> Any? {
        if let shown = pending[path] { return shown is NSNull ? nil : shown }
        return SettingsPath.value(in: values, path)
    }

    public func bool(_ path: String, _ fallback: Bool = false) -> Bool { value(path) as? Bool ?? fallback }
    public func double(_ path: String, _ fallback: Double = 0) -> Double { SettingsPath.double(value(path)) ?? fallback }
    public func int(_ path: String, _ fallback: Int = 0) -> Int { SettingsPath.double(value(path)).map { Int($0.rounded()) } ?? fallback }
    public func string(_ path: String, _ fallback: String = "") -> String { value(path) as? String ?? fallback }
    public func strings(_ path: String) -> [String] { value(path) as? [String] ?? [] }
    public func list(_ path: String) -> [[String: Any]] { value(path) as? [[String: Any]] ?? [] }

    /// Whether any of a row's paths differs from a fresh install.
    public func isChanged(_ paths: [String]) -> Bool { SettingsPath.isChanged(changed, paths) }

    /// Writes one setting.
    public func set(_ path: String, _ value: Any?) async {
        await set([path: value])
    }

    /// Writes several settings as one save.
    public func set(_ changes: [String: Any?]) async {
        for (path, value) in changes { pending[path] = value ?? NSNull() }
        do {
            _ = try await send("set_settings_fields", ["changes": changes.mapValues { $0 ?? NSNull() }])
        } catch {
            onError?("\(error)")
        }
        await refresh()
        for path in changes.keys { pending[path] = nil }
    }

    /// Puts a row's paths back to a fresh install's values.
    public func reset(_ paths: [String]) async {
        do {
            _ = try await send("reset_settings", ["paths": paths])
        } catch {
            onError?("\(error)")
        }
        await refresh()
    }

    /// Encodes the arguments here, so only a string crosses to the core.
    private func send(_ cmd: String, _ args: [String: Any]) async throws -> String {
        let data = try JSONSerialization.data(withJSONObject: args, options: [.sortedKeys])
        return try await core.invoke(cmd, String(decoding: data, as: UTF8.self))
    }

    /// One edit to a list. Throws the core's sentence when it refuses the value.
    @discardableResult
    public func edit(_ path: String, _ edit: SettingsListEdit) async throws -> Any? {
        defer { Task { await self.refresh() } }
        let json = try await send("edit_settings_list", ["path": path, "edit": edit.json])
        return try? JSONSerialization.jsonObject(with: Data(json.utf8), options: [.fragmentsAllowed])
    }
}
