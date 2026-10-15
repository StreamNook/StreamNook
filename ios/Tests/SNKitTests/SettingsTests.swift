import Foundation
import Testing
@testable import SNKit

@Suite struct SettingsPathTests {
    @Test func readsNestedValuesAndTreatsNullAsAbsent() {
        let root: [String: Any] = ["chat_design": ["font_size": 14, "mention_sound": NSNull()], "quality": "best"]
        #expect(SettingsPath.value(in: root, "chat_design.font_size") as? Int == 14)
        #expect(SettingsPath.value(in: root, "chat_design.mention_sound") == nil)
        #expect(SettingsPath.value(in: root, "chat_design.missing.deeper") == nil)
        #expect(SettingsPath.value(in: root, "quality") as? String == "best")
    }

    @Test func aRowIsChangedByItsLeafAGroupBelowItOrAListAboveIt() {
        let changed: Set<String> = ["chat_design.font_size", "chat_blend.platforms.kick", "chat_design.scrollbar_mark_kinds"]
        #expect(SettingsPath.isChanged(changed, ["chat_design.font_size"]))
        #expect(SettingsPath.isChanged(changed, ["chat_blend.platforms"]))
        #expect(SettingsPath.isChanged(changed, ["chat_design.scrollbar_mark_kinds"]))
        #expect(!SettingsPath.isChanged(changed, ["chat_design.font"]))
        #expect(!SettingsPath.isChanged(changed, []))
    }

    @Test func numbersReadAsDoublesButBooleansDoNot() {
        #expect(SettingsPath.double(NSNumber(value: 3)) == 3)
        #expect(SettingsPath.double(1.5) == 1.5)
        #expect(SettingsPath.double(NSNumber(value: true)) == nil)
        #expect(SettingsPath.double("3") == nil)
    }

    @Test func listEditsTakeTheCoresShape() {
        #expect(SettingsListEdit.add(nil).json["op"] as? String == "add")
        #expect(SettingsListEdit.add(nil).json["item"] == nil)
        let update = SettingsListEdit.update(id: "a", ["sound_id": nil]).json
        #expect(update["id"] as? String == "a")
        #expect((update["patch"] as? [String: Any])?["sound_id"] is NSNull)
        #expect(SettingsListEdit.move(id: "a", to: 2).json["to"] as? Int == 2)
        #expect(SettingsListEdit.toggle("kick:follows").json["value"] as? String == "kick:follows")
    }
}

/// A core that answers the settings commands from one dictionary.
private final class SettingsCore: CoreBridge, @unchecked Sendable {
    let events = CoreEvents()
    let kind = "test"
    var values: [String: Any] = ["chat_design": ["font_size": 14]]
    var calls: [String] = []

    func start(dirs: CoreDirs, version: String, vaultKey: Data, codecs: [String]) throws {}
    func setWebCookies(origin: String, cookiesJSON: String) throws {}
    func lifecycle(_ state: String) {}

    func invoke(_ cmd: String, _ argsJSON: String) async throws -> String {
        calls.append(cmd)
        let args = (try? JSONSerialization.jsonObject(with: Data(argsJSON.utf8))) as? [String: Any] ?? [:]
        switch cmd {
        case "get_settings_view":
            let size = SettingsPath.double(SettingsPath.value(in: values, "chat_design.font_size")) ?? 14
            let view: [String: Any] = ["values": values, "changed": size == 14 ? [] : ["chat_design.font_size"]]
            return String(decoding: try JSONSerialization.data(withJSONObject: view), as: UTF8.self)
        case "set_settings_fields":
            if let size = (args["changes"] as? [String: Any])?["chat_design.font_size"] { values["chat_design"] = ["font_size": size] }
            return "null"
        case "reset_settings":
            values["chat_design"] = ["font_size": 14]
            return "null"
        case "edit_settings_list":
            throw CoreFailure.command(json: #""That is not a site address.""#)
        default:
            throw CoreFailure.command(json: #""unknown""#)
        }
    }
}

@Suite struct SettingsStoreTests {
    @MainActor @Test func writesShowAtOnceAndArrowsFollowTheCore() async {
        let core = SettingsCore()
        let store = SettingsStore(core: core)
        await store.refresh()
        #expect(store.loaded)
        #expect(store.int("chat_design.font_size") == 14)
        #expect(!store.isChanged(["chat_design.font_size"]))

        await store.set("chat_design.font_size", 18)
        #expect(store.int("chat_design.font_size") == 18)
        #expect(store.isChanged(["chat_design.font_size"]))

        await store.reset(["chat_design.font_size"])
        #expect(store.int("chat_design.font_size") == 14)
        #expect(!store.isChanged(["chat_design.font_size"]))
        #expect(core.calls.contains("set_settings_fields") && core.calls.contains("reset_settings"))
    }

    @MainActor @Test func aRefusedListEditThrowsTheCoresSentence() async {
        let store = SettingsStore(core: SettingsCore())
        do {
            _ = try await store.edit("chat_design.link_preview_trusted_domains", .add("not a host"))
            Issue.record("the edit should have been refused")
        } catch {
            #expect("\(error)" == "That is not a site address.")
        }
    }
}
