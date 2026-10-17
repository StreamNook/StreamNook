import Foundation
import Testing
@testable import SNKit

@Suite struct CosmeticsModelTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(type, from: Data(json.utf8))
    }

    @Test func fillsDecodeEveryLayerKind() throws {
        let fill = try decode(SNFill.self, """
        {"layers":[
          {"type":"linear","angle":95,"repeating":true,"stops":[{"color":"#10b98100","at":0,"px":true},{"color":"#2dd4bf2e","at":120,"px":true}]},
          {"type":"radial","circle":false,"size":[1.2,0.6],"center":[0.5,0],"repeating":false,"stops":[{"color":"#2dd4bf1a","at":0}]},
          {"type":"image","url":"https://cdn.streamnook.app/atmospheres/midnight.webp","cover":true},
          {"type":"solid","color":"#04070cff"}]}
        """)
        #expect(fill.layers.count == 4)
        guard case .linear(let angle, let repeating, let stops) = fill.layers[0] else { Issue.record("not linear"); return }
        #expect(angle == 95 && repeating && stops[1].px && stops[1].at == 120)
        guard case .radial(_, let size, let center, _, _) = fill.layers[1] else { Issue.record("not radial"); return }
        #expect(size == [1.2, 0.6] && center == [0.5, 0])
        #expect(fill.layers[2] == .image(url: "https://cdn.streamnook.app/atmospheres/midnight.webp", cover: true))
        #expect(fill.layers[3] == .solid(color: "#04070cff"))
    }

    @Test func atmosphereLooksDecodeFromTheCoresShape() throws {
        let aurora = try decode(AtmosphereLook.self, FakeCosmetics.auroraLook)
        #expect(aurora.motion == "aurora" && !aurora.chatFrost && aurora.chatEdge != nil && aurora.image == nil)
        #expect(aurora.layers.layers.count == 1 && aurora.layers2.layers.count == 1)
        let midnight = try decode(AtmosphereLook.self, FakeCosmetics.midnightLook)
        #expect(midnight.chatFrost && midnight.chatBlur == 8 && midnight.image != nil && midnight.layers.isEmpty)
    }

    @Test func paintsCarryEveryLayerAndOlderShapesStillDecode() throws {
        let sunset = try decode(ChatPaint.self, FakeCosmetics.paintSunset)
        #expect(sunset.name == "Sunset" && sunset.layers.count == 1 && sunset.layers[0].stops.count == 3)
        #expect(sunset.baseColor == "#ff7a18ff" && sunset.opacity == 1)
        let old = try decode(ChatPaint.self, ##"{"id":"x","kind":"solid","stops":[{"at":0,"color":"#ffffffff"}]}"##)
        #expect(old.layers.isEmpty && old.opacity == 1 && old.name.isEmpty)
    }

    @Test func historyStampsReadMillisecondsSecondsAndRFC3339() {
        #expect(ProfileHistoryRow(id: "a", content: "", timestamp: "1700000000000").date == Date(timeIntervalSince1970: 1_700_000_000))
        #expect(ProfileHistoryRow(id: "b", content: "", timestamp: "1700000000").date == Date(timeIntervalSince1970: 1_700_000_000))
        #expect(ProfileHistoryRow(id: "c", content: "", timestamp: "2023-11-14T22:13:20Z").date == Date(timeIntervalSince1970: 1_700_000_000))
        #expect(ProfileHistoryRow(id: "d", content: "", timestamp: "soon").date == nil)
        #expect(ProfileHistoryRow(id: "e", content: "", timestamp: nil).date == nil)
    }

    @Test func profileTargetsKeyOnTheIdOrTheLogin() {
        #expect(ProfileTarget(userID: "42", login: "a", displayName: "A").id == "42")
        #expect(ProfileTarget(userID: "", login: "Someone", displayName: "Someone").id == "login:someone")
    }

    @Test func badgesNameThemselves() {
        #expect(ProfileBadge(id: "1", url: "u", title: "Prime").label == "Prime")
        #expect(ProfileBadge(id: "1", url: "u", name: "Named").label == "Named")
        #expect(ProfileBadge(id: "1", url: "u").label == "Badge")
        #expect(ProfileBadge(id: "1", url: "u", fallbacks: ["v"]).urls == ["u", "v"])
    }

    @Test func subStandingTintsByTier() throws {
        let prime = try decode(SubStanding.self, #"{"state":"current","founder":false,"kind":"prime","facts":[]}"#)
        #expect(prime.tintHex == "#00a8e1")
        let gold = try decode(SubStanding.self, #"{"state":"current","founder":false,"kind":"paid","tier":3,"facts":[]}"#)
        #expect(gold.tintHex == "#f2c14e")
    }
}

@Suite struct MemberFrameTests {
    private func row(_ id: String, user: String, provider: String = "twitch") throws -> ChatSocketFrame {
        .row(try JSONDecoder().decode(ChatMessage.self, from: Data("""
        {"id":"\(id)","user_id":"\(user)","username":"u\(user)","display_name":"U\(user)","badges":[],"timestamp":"0",
         "content":"hi","provider":"\(provider)","channel":"room","tags":{},"segments":[],
         "metadata":{"view":{"name_color":"#ffffff","badges":[{"url":"x","provider":"twitch"},{"url":"old","provider":"streamnook"}],"spans":[]}}}
        """.utf8)))
    }

    @Test func aMemberNeverDressesAKickChatterWithTheSameNumber() throws {
        var buffer = ChatRoomBuffer(cap: 50)
        _ = buffer.apply([try row("k", user: "7", provider: "kick")])
        let member = ChatSocketFrame.decode(#"{"type":"ROW_MEMBER","user_id":"7","atmosphere":"aurora","badge":{"url":"new","provider":"streamnook"}}"#)
        let change = buffer.apply([member])
        #expect(change.updated.isEmpty)
        #expect(buffer.row("k")?.message.metadata.view?.atmosphere == nil)
        // The member who claimed that Kick account is named by it.
        let claimed = ChatSocketFrame.decode(#"{"type":"ROW_MEMBER","user_id":"kick:7","atmosphere":"aurora","badge":{"url":"new","provider":"streamnook"}}"#)
        #expect(buffer.apply([claimed]).updated.count == 1)
        #expect(buffer.row("k")?.message.metadata.view?.atmosphere == "aurora")
    }

    @Test func aMemberFrameDecodes() {
        let frame = ChatSocketFrame.decode(#"{"type":"ROW_MEMBER","user_id":"7","atmosphere":"aurora","cologne":null,"badge":{"url":"streamnook-house:streamnook-default","title":"StreamNook","provider":"streamnook"}}"#)
        guard case .member(let user, let atmosphere, let cologne, let badge) = frame else { Issue.record("\(frame)"); return }
        #expect(user == "7" && atmosphere == "aurora" && cologne == nil && badge?.provider == "streamnook")
    }

    @Test func aMemberFrameRepaintsOnlyThatMembersRows() throws {
        var buffer = ChatRoomBuffer(cap: 50)
        _ = buffer.apply([try row("a", user: "7"), try row("b", user: "8")])
        let member = ChatSocketFrame.decode(#"{"type":"ROW_MEMBER","user_id":"7","atmosphere":"aurora","badge":{"url":"new","provider":"streamnook"}}"#)
        let change = buffer.apply([member])
        #expect(change.updated.count == 1)
        let view = try #require(buffer.row("a")?.message.metadata.view)
        #expect(view.atmosphere == "aurora")
        #expect(view.badges.filter { $0.provider == "streamnook" }.map(\.url) == ["new"])
        #expect(buffer.row("b")?.message.metadata.view?.atmosphere == nil)
        // Taking the theme off clears it again.
        _ = buffer.apply([ChatSocketFrame.decode(#"{"type":"ROW_MEMBER","user_id":"7","atmosphere":null}"#)])
        #expect(buffer.row("a")?.message.metadata.view?.atmosphere == nil)
        #expect(buffer.row("a")?.message.metadata.view?.badges.contains { $0.provider == "streamnook" } == false)
    }
}

@Suite struct FakeCosmeticsTests {
    private func core() throws -> FakeCore {
        let core = FakeCore(fixtures: FakeFixtures(signedIn: true))
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "test", vaultKey: Data(count: 32), codecs: [])
        return core
    }

    @Test func everyCosmeticsAnswerDecodes() async throws {
        let core = try core()
        let atmospheres = try await core.invoke("sn_atmospheres", args: [:], as: [Atmosphere].self)
        #expect(atmospheres.map(\.id) == ["aurora", "midnight"])
        let sn = try await core.invoke("cosmetics_picker_state", args: ["userId": "424242"], as: StreamNookPickerState.self)
        #expect(sn.atmospheres.first?.id == "tier" && sn.showChatToggle)
        #expect(sn.badges.first { $0.worn }?.slug == "streamnook-mobius")
        let ext = try await core.invoke("external_cosmetics_state", args: ["userId": "424242", "login": "nookviewer"],
                                        as: ExternalPickerState.self)
        #expect(ext.paints.count == 2 && ext.sevenTvBadges.count == 2 && ext.twitchSelectable.count == 3)
        let sheet = try await core.invoke("user_profile_sheet", args: ["userId": "9", "login": "x", "displayName": "X"],
                                          as: ProfileSheetData.self)
        #expect(sheet.groups.map(\.title) == ["Twitch", "7TV"] && sheet.streamnook != nil && !sheet.hasNoBadges)
        let me = try await core.invoke("own_identity_header", args: ["userId": "424242"], as: OwnIdentityData.self)
        #expect(me.badges.count == 2 && me.paint?.name == "Sunset")
        let standing = try await core.invoke("get_sub_standing", args: [:], as: SubStanding?.self)
        #expect(standing?.nextBadge?.monthsLeft == 4)
    }

    @Test func writesChangeTheFixtureAndRefusalsThrow() async throws {
        let core = try core()
        _ = try await core.invoke("equip_streamnook_badge", args: ["userId": "424242", "slug": nil])
        var sn = try await core.invoke("cosmetics_picker_state", args: [:], as: StreamNookPickerState.self)
        #expect(!sn.badges.contains { $0.worn })
        await #expect(throws: CoreFailure.self) {
            _ = try await core.invoke("equip_streamnook_badge", args: ["userId": "424242", "slug": "streamnook-refused"])
        }
        _ = try await core.invoke("set_profile_theme", args: ["userId": "424242", "theme": "tier"])
        sn = try await core.invoke("cosmetics_picker_state", args: [:], as: StreamNookPickerState.self)
        #expect(sn.theme == "tier" && !sn.showChatToggle)
    }

    @Test func profileAnswersCarryTheWornAtmosphere() async throws {
        let core = try core()
        let sheet = try await core.invoke("user_profile_sheet", args: ["userId": "9", "login": "x", "displayName": "X"],
                                          as: ProfileSheetData.self)
        #expect(sheet.atmosphere?.id == "aurora" && sheet.atmosphere?.look.chatFrost == false && sheet.cologne == nil)
        var me = try await core.invoke("own_identity_header", args: ["userId": "424242"], as: OwnIdentityData.self)
        #expect(me.atmosphere?.name == "Aurora" && me.bannerURL?.hasPrefix("https://") == true)
        _ = try await core.invoke("set_profile_theme", args: ["userId": "424242", "theme": "tier"])
        me = try await core.invoke("own_identity_header", args: ["userId": "424242"], as: OwnIdentityData.self)
        #expect(me.atmosphere == nil)
    }

    @Test func twitchProfileWritesFollowTheCoresRules() async throws {
        let core = try core()
        struct Stored: Decodable { var display_name: String }
        let stored = try await core.invoke("update_twitch_display_name", args: ["displayName": "NOOKViewer"], as: Stored.self)
        #expect(stored.display_name == "NOOKViewer")
        let me = try await core.invoke("own_identity_header", args: ["userId": "424242"], as: OwnIdentityData.self)
        #expect(me.displayName == "NOOKViewer")
        await #expect(throws: CoreFailure.self) {
            _ = try await core.invoke("update_twitch_display_name", args: ["displayName": "SomeoneElse"])
        }
        _ = try await core.invoke("upload_twitch_profile_image", args: ["kind": "banner", "dataBase64": "iVBORw0KGgo="])
        let pending = try await core.invoke("upload_seventv_profile_picture", args: ["userId": "01FAKE7TV", "dataBase64": "iVBORw0KGgo="],
                                            as: String.self)
        #expect(!pending.isEmpty)
        _ = try await core.invoke("logout_seventv", args: [:])
        await #expect(throws: CoreFailure.self) {
            _ = try await core.invoke("upload_seventv_profile_picture", args: ["userId": "01FAKE7TV", "dataBase64": "iVBORw0KGgo="])
        }
        await #expect(throws: CoreFailure.self) {
            _ = try await core.invoke("upload_twitch_profile_image", args: ["kind": "poster", "dataBase64": "iVBORw0KGgo="])
        }
    }
}
