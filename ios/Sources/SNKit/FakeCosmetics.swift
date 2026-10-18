import Foundation
import Synchronization

/// FakeCore's answers for profiles and cosmetics, shaped exactly as the core answers them. Writes
/// change the fixture's own state (never an account), so equip, unequip and rollback can be shown
/// and tested: equipping the badge slug `streamnook-refused` or the paint `refused` fails the way
/// a refused write does.
final class FakeCosmetics: Sendable {
    private struct State {
        var wornBadge: String? = "streamnook-mobius"
        var theme = "aurora"
        var chatAtmosphere = true
        var paint: String? = "p-sunset"
        var sevenTvBadge: String? = "b-sub"
        var twitchBadge = "glhf-pledge"
        var sevenTvSignedIn = true
        var chatColor = "#8a2be2"
        var bio = "Variety streamer, mostly late-night co-op."
        var displayName = "NookViewer"
        var uploads: [String] = []
    }

    private let state = Mutex(State())

    static let auroraLook = """
    {"accent":"#2dd4bfff","swatch":{"layers":[{"type":"linear","angle":135,"repeating":false,"stops":[\
    {"color":"#10b981ff","at":0},{"color":"#2dd4bfff","at":0.4},{"color":"#22d3eeff","at":0.7},{"color":"#8b5cf6ff","at":1}]}]},\
    "base_color":"#04070cff","base_layers":{"layers":[{"type":"radial","circle":false,"size":[1.2,0.6],"center":[0.5,0],\
    "repeating":false,"stops":[{"color":"#2dd4bf1a","at":0},{"color":"#2dd4bf00","at":0.65}]}]},\
    "layers":{"layers":[{"type":"linear","angle":95,"repeating":true,"stops":[{"color":"#10b98100","at":0,"px":true},\
    {"color":"#2dd4bf2e","at":120,"px":true},{"color":"#22d3ee1a","at":200,"px":true},{"color":"#10b98100","at":320,"px":true}]}]},\
    "layers2":{"layers":[{"type":"linear","angle":88,"repeating":true,"stops":[{"color":"#22d3ee00","at":0,"px":true},\
    {"color":"#22d3ee1f","at":90,"px":true},{"color":"#8b5cf61a","at":160,"px":true},{"color":"#22d3ee00","at":240,"px":true}]}]},\
    "motion":"aurora","chat_edge":{"layers":[{"type":"linear","angle":180,"repeating":false,"stops":[{"color":"#2dd4bf00","at":0},\
    {"color":"#2dd4bfd9","at":0.22},{"color":"#22d3eebf","at":0.52},{"color":"#8b5cf699","at":0.8},{"color":"#8b5cf600","at":1}]}]},\
    "chat_frost":false}
    """

    static let midnightLook = """
    {"accent":"#4ade80ff","swatch":{"layers":[{"type":"image","url":"https://cdn.streamnook.app/atmospheres/midnight.webp","cover":true}]},\
    "base_color":"#05060dff","image":"https://cdn.streamnook.app/atmospheres/midnight.webp","motion":"aurora",\
    "chat_edge":{"layers":[{"type":"linear","angle":180,"repeating":false,"stops":[{"color":"#4ade8000","at":0},\
    {"color":"#4ade80d9","at":0.25},{"color":"#22c55eb3","at":0.6},{"color":"#22c55e00","at":1}]}]},"chat_frost":true,"chat_blur":8}
    """

    static let paintSunset = """
    {"id":"p-sunset","kind":"linear","angle":90,"repeating":false,"stops":[{"at":0,"color":"#ff7a18ff"},{"at":1,"color":"#af002dff"}],\
    "shadows":[{"x":0,"y":0,"blur":2,"color":"#ff7a1899"}],"name":"Sunset","layers":[{"kind":"linear","angle":90,"repeating":false,\
    "stops":[{"at":0,"color":"#ff7a18ff"},{"at":0.5,"color":"#ffd200ff"},{"at":1,"color":"#af002dff"}]}],"base_color":"#ff7a18ff"}
    """

    static let paintAqua = """
    {"id":"p-aqua","kind":"radial","stops":[{"at":0,"color":"#a1ffceff"},{"at":1,"color":"#00b4dbff"}],"name":"Aqua Glow",\
    "layers":[{"kind":"radial","circle":true,"stops":[{"at":0,"color":"#a1ffceff"},{"at":1,"color":"#00b4dbff"}]}],\
    "shadows":[{"x":0,"y":1,"blur":3,"color":"#00000099"}]}
    """

    private static let house = "streamnook-house:"

    func answer(_ cmd: String, _ args: [String: Any], userID: String) throws -> String? {
        switch cmd {
        case "sn_atmospheres":
            return """
            [{"id":"aurora","name":"Aurora","look":\(Self.auroraLook)},{"id":"midnight","name":"Midnight","look":\(Self.midnightLook)}]
            """
        case "cosmetics_picker_state":
            let s = state.withLock { $0 }
            let badges: [(String, String, String?)] = [
                ("streamnook-default", "StreamNook", "\(Self.house)streamnook-default"),
                ("streamnook-subscriber", "Subscriber", "\(Self.house)streamnook-subscriber"),
                ("streamnook-mobius", "Möbius", "https://cdn.streamnook.app/badges/streamnook-mobius.webp?v=2"),
                ("streamnook-refused", "Refused", nil),
            ]
            let badgeJSON = badges.map { slug, name, url in
                let image = url.map { #","image_url":"\#($0)""# } ?? ""
                return #"{"slug":"\#(slug)","name":"\#(name)"\#(image),"worn":\#(s.wornBadge == slug)}"#
            }.joined(separator: ",")
            let auroraSwatch = ##"{"layers":[{"type":"linear","angle":135,"repeating":false,"stops":[{"color":"#10b981ff","at":0},{"color":"#2dd4bfff","at":0.4},{"color":"#22d3eeff","at":0.7},{"color":"#8b5cf6ff","at":1}]}]}"##
            let midnightSwatch = #"{"layers":[{"type":"image","url":"https://cdn.streamnook.app/atmospheres/midnight.webp","cover":true}]}"#
            let dispersionSwatch = #"{"layers":[{"type":"image","url":"https://cdn.streamnook.app/atmospheres/dispersion.webp","cover":true}]}"#
            let atmospheres = [
                #"{"id":"tier","name":"Default","swatch":{"layers":[]},"allowed":true,"worn":\#(s.theme == "tier")}"#,
                #"{"id":"aurora","name":"Aurora","swatch":\#(auroraSwatch),"allowed":true,"worn":\#(s.theme == "aurora")}"#,
                #"{"id":"midnight","name":"Midnight","swatch":\#(midnightSwatch),"allowed":true,"worn":\#(s.theme == "midnight")}"#,
                #"{"id":"dispersion","name":"Dispersion","swatch":\#(dispersionSwatch),"allowed":false,"worn":false}"#,
            ].joined(separator: ",")
            let toggle = s.theme != "tier" && s.theme != "paint"
            return #"{"badges":[\#(badgeJSON)],"atmospheres":[\#(atmospheres)],"theme":"\#(s.theme)","show_chat_toggle":\#(toggle),"chat_atmosphere":\#(s.chatAtmosphere)}"#
        case "external_cosmetics_state":
            let s = state.withLock { $0 }
            func paint(_ json: String, id: String) -> String {
                let name = id == "p-sunset" ? "Sunset" : "Aqua Glow"
                return #"{"id":"\#(id)","name":"\#(name)","paint":\#(json),"worn":\#(s.paint == id)}"#
            }
            let badge = { (id: String, title: String) -> String in
                #"{"badge":{"id":"\#(id)","url":"https://cdn.7tv.app/badge/\#(id)/4x.webp","title":"\#(title)","name":"\#(title)"},"worn":\#(s.sevenTvBadge == id)}"#
            }
            let twitch = [("glhf-pledge", "GLHF Pledge"), ("premium", "Prime Gaming"), ("twitchcon-2026", "TwitchCon 2026")].map { id, title in
                #"{"id":"\#(id)","version":"1","title":"\#(title)","image_url":"https://static-cdn.jtvnw.net/badges/v1/\#(id)/3","is_selected":\#(s.twitchBadge == id)}"#
            }.joined(separator: ",")
            return ##"{"seventv_signed_in":\##(s.sevenTvSignedIn),"seventv_user_id":"01FAKE7TV","paints":[\##(paint(Self.paintSunset, id: "p-sunset")),\##(paint(Self.paintAqua, id: "p-aqua"))],"seventv_badges":[\##(badge("b-sub", "7TV Subscriber")),\##(badge("b-early", "Early Supporter"))],"drops_signed_in":true,"twitch_selectable":[\##(twitch)],"twitch_earned":[],"chat_color":"\##(s.chatColor)","named_colors":[{"label":"Blue Violet","value":"blue_violet","hex":"#8a2be2"},{"label":"Coral","value":"coral","hex":"#ff7f50"},{"label":"Dodger Blue","value":"dodger_blue","hex":"#1e90ff"},{"label":"Spring Green","value":"spring_green","hex":"#00ff7f"}]}"##
        case "equip_streamnook_badge":
            let slug = args["slug"] as? String
            if slug == "streamnook-refused" { throw CoreFailure.command(json: #""You do not own that badge.""#) }
            state.withLock { $0.wornBadge = slug }
            return "null"
        case "set_profile_theme":
            state.withLock { $0.theme = args["theme"] as? String ?? "tier" }
            return "null"
        case "set_chat_atmosphere":
            state.withLock { $0.chatAtmosphere = args["show"] as? Bool ?? true }
            return "true"
        case "set_seventv_paint":
            let id = args["paintId"] as? String
            if id == "refused" { throw CoreFailure.command(json: #""SESSION_EXPIRED""#) }
            state.withLock { $0.paint = id }
            return #"{"success":true,"message":"ok"}"#
        case "set_seventv_badge":
            state.withLock { $0.sevenTvBadge = args["badgeId"] as? String }
            return #"{"success":true,"message":"ok"}"#
        case "get_twitch_bio":
            return String(decoding: try JSONEncoder().encode(state.withLock { $0.bio }), as: UTF8.self)
        case "update_twitch_bio":
            let text = (args["description"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            if text.count > 300 { throw CoreFailure.command(json: #""A bio can be at most 300 characters.""#) }
            state.withLock { $0.bio = text }
            return String(decoding: try JSONEncoder().encode(text), as: UTF8.self)
        case "update_twitch_display_name":
            let name = (args["displayName"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            guard !name.isEmpty, name.lowercased() == "nookviewer" else {
                throw CoreFailure.command(json: #""Only the capitalization of your name can change.""#)
            }
            let bio = state.withLock { s -> String in
                s.displayName = name
                return s.bio
            }
            let answer = ["display_name": name, "description": bio]
            return String(decoding: try JSONEncoder().encode(answer), as: UTF8.self)
        case "upload_seventv_profile_picture":
            guard state.withLock({ $0.sevenTvSignedIn }) else {
                throw CoreFailure.command(json: #""SESSION_EXPIRED: 7TV session has expired. Please reconnect your 7TV account.""#)
            }
            guard let data = Data(base64Encoded: args["dataBase64"] as? String ?? ""), !data.isEmpty else {
                throw CoreFailure.command(json: #""That image could not be read.""#)
            }
            state.withLock { $0.uploads.append("seventv") }
            return #""01FAKEPENDINGPICTURE""#
        case "upload_twitch_profile_image":
            let kind = args["kind"] as? String ?? ""
            guard kind == "picture" || kind == "banner" else { throw CoreFailure.command(json: #""Unknown image kind.""#) }
            guard let data = Data(base64Encoded: args["dataBase64"] as? String ?? ""), !data.isEmpty else {
                throw CoreFailure.command(json: #""That image could not be read.""#)
            }
            state.withLock { $0.uploads.append(kind) }
            return "null"
        case "update_user_chat_color":
            let value = args["color"] as? String ?? ""
            let hex = ["blue_violet": "#8a2be2", "coral": "#ff7f50", "dodger_blue": "#1e90ff", "spring_green": "#00ff7f"][value] ?? value
            state.withLock { $0.chatColor = hex }
            return "null"
        case "update_chat_identity_gql":
            state.withLock { $0.twitchBadge = args["badgeId"] as? String ?? "" }
            return "null"
        case "get_seventv_auth_status":
            let on = state.withLock { $0.sevenTvSignedIn }
            return #"{"is_authenticated":\#(on),"user_id":"01FAKE7TV","twitch_id":"\#(userID)","expires_at":null}"#
        case "logout_seventv":
            state.withLock { $0.sevenTvSignedIn = false }
            return "true"
        case "own_identity_header", "own_identity_header_cached":
            let s = state.withLock { $0 }
            let paint = s.paint == "p-sunset" ? Self.paintSunset : s.paint == "p-aqua" ? Self.paintAqua : "null"
            let member = s.wornBadge.map { slug -> String in
                let url = slug.hasPrefix("streamnook-mobius") ? "https://cdn.streamnook.app/badges/streamnook-mobius.webp?v=2"
                    : "\(Self.house)\(slug)"
                return #"{"slug":"\#(slug)","image_url":"\#(url)","label":"StreamNook Möbius"}"#
            } ?? #"{"image_url":"\#(Self.house)streamnook-default","label":"StreamNook"}"#
            let worn = s.theme == "aurora" ? #"{"id":"aurora","name":"Aurora","look":\#(Self.auroraLook)}"#
                : s.theme == "midnight" ? #"{"id":"midnight","name":"Midnight","look":\#(Self.midnightLook)}"# : "null"
            return #"{"user_id":"\#(userID)","login":"nookviewer","display_name":"\#(s.displayName)","banner_url":"https://static-cdn.jtvnw.net/jtv_user_pictures/c51fdef0-593a-4cae-a308-2e8e7ce39e51-profile_banner-480.png","atmosphere":\#(worn),"paint":\#(paint),"badges":[{"id":"glhf-pledge","url":"https://static-cdn.jtvnw.net/badges/v1/3158e758-3cb4-43c5-94b3-7639810451c5/3","title":"GLHF Pledge"},{"id":"b-sub","url":"https://cdn.7tv.app/badge/01F37R3RFR0000K96678WEQT01/4x.webp","title":"7TV Subscriber"}],"streamnook":\#(member)}"#
        case "user_profile_sheet", "user_profile_sheet_cached":
            let id = args["userId"] as? String ?? "1"
            let login = args["login"] as? String ?? "chatter"
            let name = args["displayName"] as? String ?? login
            return ##"{"user_id":"\##(id)","login":"\##(login)","display_name":"\##(name)","atmosphere":{"id":"aurora","name":"Aurora","look":\##(Self.auroraLook)},"avatar_url":"https://static-cdn.jtvnw.net/user-default-pictures-uv/ebe4cd89-b4f4-4cd9-adac-2f30151b4209-profile_image-300x300.png","name_color":"#ff7a18","paint":\##(Self.paintSunset),"groups":[{"title":"Twitch","badges":[{"id":"premium","url":"https://static-cdn.jtvnw.net/badges/v1/bbbe0db0-a598-423e-86d0-f9fb98ca1933/3","title":"Prime Gaming","description":"Prime Gaming member"}]},{"title":"7TV","badges":[{"id":"b-sub","url":"https://cdn.7tv.app/badge/01F37R3RFR0000K96678WEQT01/4x.webp","title":"7TV Subscriber","name":"7TV Subscriber"}]},{"title":"StreamNook","badges":[{"id":"streamnook-mobius","url":"https://cdn.streamnook.app/badges/streamnook-mobius.webp?v=2","title":"StreamNook Möbius","provider":"streamnook"},{"id":"streamnook-default","url":"streamnook-house:streamnook-default","title":"StreamNook","provider":"streamnook"}]}],"streamnook":{"slug":"streamnook-mobius","image_url":"https://cdn.streamnook.app/badges/streamnook-mobius.webp?v=2","label":"StreamNook Möbius"},"member":{"number":69,"tier":"ascendant","label":"Nice","accent":"#22D3EE"},"member_profile":{"stats":[{"key":"hours","label":"Hours watched","value":"1,300"},{"key":"messages","label":"Messages sent","value":"10,100"},{"key":"streams","label":"Streams watched","value":"300"},{"key":"favorite_channel","label":"Favorite channel","value":"NookCaster","caption":"313h watched"},{"key":"twitch_age","label":"Twitch age","value":"12 yrs, 6 mos","caption":"Cake day in 154 days"},{"key":"followers","label":"Followers","value":"1,204"},{"key":"views","label":"Profile views","value":"4,321"}],"accolades":[{"id":"chatterbox","label":"Chatterbox","icon":"message-circle","colors":["#6aa9ff","#3b6fd4"]},{"id":"marathoner","label":"Marathoner","icon":"timer","colors":["#f59e0b","#b45309"]},{"id":"og","label":"OG","icon":"crown","colors":["#fbbf24","#d97706"]},{"id":"nice","label":"Nice","icon":"smile","colors":["#facc15","#ca8a04"]}],"top_emotes":[{"id":"25","name":"Kappa","provider":"twitch","url":"https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/3.0","count":412}]}}"##
        case "get_sub_standing":
            return #"{"state":"current","founder":false,"kind":"paid","tier":1,"months":14,"streak_months":6,"title":"Subscribed to NookCaster","detail":"Tier 1 · 14 months","gift_label":"Gift a sub","gift_url":"https://www.twitch.tv/subs/nookcaster?giftrecipient=chatter","months_unit":"months","tier_label":"Tier 1","facts":[{"kind":"streak","text":"6-month streak","art":null}],"badge_art":null,"next_badge":{"months":18,"months_left":4,"progress":0.66,"art":null,"text":"4 months to the 18-month badge"}}"#
        case "get_user_message_history_limited":
            let now = Int(Date().timeIntervalSince1970 * 1000)
            return #"[{"id":"h1","content":"that clip was insane","timestamp":"\#(now - 600_000)"},{"id":"h2","content":"KEKW","timestamp":"\#(now - 120_000)"}]"#
        case "watch_user_history", "unwatch_user_history", "register_own_member_themes", "sn_refresh_registries":
            return "null"
        case "get_user_by_login":
            let login = (args["login"] as? String ?? "chatter").lowercased()
            return ##"{"id":"777\##(login.count)","login":"\##(login)","display_name":"\##(login.capitalized)","profile_image_url":"https://static-cdn.jtvnw.net/user-default-pictures-uv/ebe4cd89-b4f4-4cd9-adac-2f30151b4209-profile_image-300x300.png"}"##
        case "get_seventv_login_url":
            return #""https://api.7tv.app/v4/auth/login?platform=twitch""#
        default:
            return nil
        }
    }
}
