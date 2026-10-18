import Observation
import SNKit
import SwiftUI

/// The Cosmetics screen's state: the core's two halves (StreamNook's own badge and Atmosphere; 7TV
/// and Twitch), with each change shown at once and put back if the core refuses it.
@Observable @MainActor
final class CosmeticsScreenModel {
    private(set) var sn: StreamNookPickerState?
    private(set) var ext: ExternalPickerState?
    var signingIn = false
    var showSevenTvSignIn = false
    /// The account's Twitch bio; nil until read.
    private(set) var bio: String?
    var editingBio = false
    @ObservationIgnored private var afterSignIn: ((String) async -> Void)?
    @ObservationIgnored let app: AppModel

    init(app: AppModel) {
        self.app = app
    }

    private var core: CoreBridge { app.core }
    private var user: TwitchUser? { app.signIn.user }

    /// Every open re-reads everything; each half lands on its own, with no spinners.
    func load() async {
        guard let user else { return }
        async let snRead: StreamNookPickerState? = try? core.invoke("cosmetics_picker_state", args: ["userId": user.id],
                                                                     as: StreamNookPickerState.self)
        async let extRead: ExternalPickerState? = try? core.invoke("external_cosmetics_state",
                                                                    args: ["userId": user.id, "login": user.login],
                                                                    as: ExternalPickerState.self)
        async let bioRead: String? = try? core.invoke("get_twitch_bio", args: ["userId": user.id], as: String.self)
        if let value = await snRead { sn = value }
        if let value = await extRead { ext = value }
        if let value = await bioRead { bio = value }
    }

    /// Saves the Twitch bio; answers whether Twitch took it.
    func saveBio(_ text: String) async -> Bool {
        do {
            bio = try await core.invoke("update_twitch_bio", args: ["description": text], as: String.self)
            return true
        } catch {
            let reason = (error as? CoreFailure)?.description ?? ""
            app.toasts.show(reason.isEmpty ? "Could not save your bio." : reason)
            return false
        }
    }

    // MARK: StreamNook

    /// Wears a badge; tapping the one worn takes it off (back to the plain mark).
    func equipBadge(_ badge: PickerBadge) async {
        guard let user, var state = sn else { return }
        let next: String? = badge.worn ? nil : badge.slug
        let before = state
        for i in state.badges.indices { state.badges[i].worn = state.badges[i].slug == next }
        sn = state
        do {
            _ = try await core.invoke("equip_streamnook_badge", args: ["userId": user.id, "slug": next])
        } catch {
            sn = before
            let text = (error as? CoreFailure)?.description ?? ""
            app.toasts.show(text.isEmpty ? "Could not equip that badge." : text)
        }
    }

    func equipTheme(_ atmosphere: PickerAtmosphere) async {
        guard let user, var state = sn, atmosphere.allowed else { return }
        let before = state
        state.theme = atmosphere.id
        for i in state.atmospheres.indices { state.atmospheres[i].worn = state.atmospheres[i].id == atmosphere.id }
        state.showChatToggle = atmosphere.id != "tier" && atmosphere.id != "paint"
        sn = state
        do {
            _ = try await core.invoke("set_profile_theme", args: ["userId": user.id, "theme": atmosphere.id])
        } catch {
            sn = before
            app.toasts.show("Could not apply that atmosphere.")
        }
    }

    func setChatAtmosphere(_ show: Bool) async {
        guard let user, var state = sn else { return }
        state.chatAtmosphere = show
        sn = state
        let saved = (try? await core.invoke("set_chat_atmosphere", args: ["userId": user.id, "show": show],
                                            as: Bool.self)) ?? false
        if !saved {
            state.chatAtmosphere = !show
            sn = state
            app.toasts.show("Could not save that. Try again in a moment.")
        }
    }

    // MARK: 7TV

    func signInToSevenTv() {
        guard !signingIn else { return }
        afterSignIn = nil
        signingIn = true
        showSevenTvSignIn = true
    }

    func signOutOfSevenTv() async {
        do {
            _ = try await core.invoke("logout_seventv", args: [:])
            ext?.sevenTvSignedIn = false
        } catch {
            app.toasts.show("Could not sign out of 7TV.")
        }
    }

    /// The sign-in sheet closed: stored or not, then read the status back.
    func sevenTvSignInFinished(_ stored: Bool) async {
        defer { signingIn = false }
        let next = afterSignIn
        afterSignIn = nil
        guard stored else { return }
        guard let status = try? await core.invoke("get_seventv_auth_status", args: [:], as: SevenTvStatus.self) else {
            app.toasts.show("Could not sign in to 7TV.")
            return
        }
        ext?.sevenTvSignedIn = status.isAuthenticated
        if let id = status.userID { ext?.sevenTvUserID = id }
        guard status.isAuthenticated else {
            app.toasts.show("7TV did not finish signing you in. Try again.")
            return
        }
        guard let id = status.userID ?? ext?.sevenTvUserID else {
            app.toasts.show("7TV did not identify your account.")
            return
        }
        await next?(id)
    }

    /// Runs `apply` with the 7TV account id, signing in first when needed; a tap while signing in
    /// is ignored.
    private func withSevenTv(_ apply: @escaping (String) async -> Void) async {
        guard let ext else { return }
        if !ext.sevenTvSignedIn {
            guard !signingIn else { return }
            afterSignIn = apply
            signingIn = true
            showSevenTvSignIn = true
            return
        }
        guard let id = ext.sevenTvUserID else {
            app.toasts.show("7TV did not identify your account.")
            return
        }
        await apply(id)
    }

    func equipPaint(_ paint: PickerPaint) async {
        await withSevenTv { [weak self] sevenTvID in
            guard let self, var state = self.ext else { return }
            let current = state.paints.first(where: \.worn)?.id
            let next: String? = current == paint.id ? nil : paint.id
            let before = state
            for i in state.paints.indices { state.paints[i].worn = state.paints[i].id == next }
            self.ext = state
            do {
                let result = try await self.core.invoke("set_seventv_paint", args: ["userId": sevenTvID, "paintId": next],
                                                        as: SevenTvWrite.self)
                if !result.success { throw CoreFailure.command(json: "\"rejected\"") }
            } catch {
                self.ext = before
                self.sevenTvWriteFailed(error, what: "paint")
            }
        }
    }

    func equipSevenTvBadge(_ badge: PickerSevenTvBadge) async {
        await withSevenTv { [weak self] sevenTvID in
            guard let self, var state = self.ext else { return }
            let current = state.sevenTvBadges.first(where: \.worn)?.id
            let next: String? = current == badge.id ? nil : badge.id
            let before = state
            for i in state.sevenTvBadges.indices { state.sevenTvBadges[i].worn = state.sevenTvBadges[i].id == next }
            self.ext = state
            do {
                let result = try await self.core.invoke("set_seventv_badge", args: ["userId": sevenTvID, "badgeId": next],
                                                        as: SevenTvWrite.self)
                if !result.success { throw CoreFailure.command(json: "\"rejected\"") }
            } catch {
                self.ext = before
                self.sevenTvWriteFailed(error, what: "badge")
            }
        }
    }

    private func sevenTvWriteFailed(_ error: Error, what: String) {
        if "\(error)".contains("SESSION_EXPIRED") {
            ext?.sevenTvSignedIn = false
            app.toasts.show("Your 7TV sign-in expired. Tap again to sign back in.")
        } else {
            app.toasts.show("Could not equip that \(what).")
        }
    }

    // MARK: Name colour

    /// Sets the account's Twitch name colour to one of Twitch's named colours.
    func setChatColor(_ color: NamedChatColor) async {
        guard let user, var state = ext, state.chatColor != color.hex else { return }
        let before = state
        state.chatColor = color.hex
        ext = state
        do {
            _ = try await core.invoke("update_user_chat_color", args: ["targetUserId": user.id, "color": color.value])
        } catch {
            ext = before
            app.toasts.show("Could not change your name color.")
        }
    }

    // MARK: Twitch

    /// Wears a Twitch global badge. There is no taking one off: Twitch only swaps which is worn.
    func equipTwitchBadge(_ badge: TwitchIdentityBadge) async {
        guard !badge.isSelected, var state = ext else { return }
        let before = state
        for i in state.twitchSelectable.indices {
            state.twitchSelectable[i].isSelected = state.twitchSelectable[i].setID == badge.setID
        }
        ext = state
        do {
            _ = try await core.invoke("update_chat_identity_gql", args: ["badgeId": badge.setID, "badgeVersion": badge.version])
        } catch {
            ext = before
            app.toasts.show("\(error)".contains("NEEDS_DROPS_AUTH") ? "Sign in to Drops to change your Twitch badge."
                                                                     : "Could not equip that badge.")
        }
    }

    private struct SevenTvStatus: Decodable {
        var isAuthenticated: Bool
        var userID: String?
        enum CodingKeys: String, CodingKey { case isAuthenticated = "is_authenticated", userID = "user_id" }
    }

    private struct SevenTvWrite: Decodable {
        var success: Bool
    }
}
