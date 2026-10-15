import PhotosUI
import SNKit
import SwiftUI
import UIKit
// PhotosPicker lives in PhotosUI's SwiftUI overlay, which this cross-compiled SDK does not import on
// its own.
import _PhotosUI_SwiftUI

/// Your profile, the first section of Edit profile: your two pictures (Twitch's and 7TV's, never
/// blended into one), your Twitch banner, your display name (its capitalization) and your bio,
/// each a row of one panel. The core writes Twitch through the twitch.tv session and 7TV through
/// the 7TV session, and answers in words when either says no.
struct TwitchProfileSection: View {
    let model: AppModel
    let screen: CosmeticsScreenModel
    let user: TwitchUser
    /// Your 7TV picture and Twitch banner, as the core reads them.
    let identity: OwnIdentityData?
    /// A Twitch picture just picked, shown before Twitch serves it.
    @Binding var picture: UIImage?
    /// The name as typed, shown in the preview while it is edited.
    @Binding var draftName: String
    @State private var banner: UIImage?
    @State private var twitchItem: PhotosPickerItem?
    @State private var sevenTvItem: PhotosPickerItem?
    @State private var bannerItem: PhotosPickerItem?
    @State private var uploading: ImageKind?
    /// A 7TV picture sent and waiting on 7TV's review.
    @State private var sevenTvPending = false
    @State private var savingName = false
    @FocusState private var nameFocused: Bool

    enum ImageKind: String { case picture, banner, sevenTV }

    private var sevenTvSignedIn: Bool { screen.ext?.sevenTvSignedIn ?? false }

    var body: some View {
        VStack(spacing: 0) {
            twitchPictureRow
            divider
            sevenTvPictureRow
            divider
            bannerRow
            divider
            nameRow
            if screen.bio != nil {
                divider
                bioRow
            }
        }
        .snPanel()
        .onChange(of: twitchItem) { _, item in if let item { Task { await upload(item, as: .picture) } } }
        .onChange(of: sevenTvItem) { _, item in if let item { Task { await upload(item, as: .sevenTV) } } }
        .onChange(of: bannerItem) { _, item in if let item { Task { await upload(item, as: .banner) } } }
        .onAppear { if draftName.isEmpty { draftName = user.displayName } }
    }

    private var divider: some View {
        Rectangle().fill(SN.borderSubtle).frame(height: 0.5).padding(.leading, 14)
    }

    // MARK: Pictures

    private var twitchPictureRow: some View {
        HStack(spacing: 12) {
            pictureFrame(mark: .twitch) {
                if let picture {
                    Image(uiImage: picture).resizable().scaledToFill()
                } else {
                    CosmeticImage(user.profileImageURL ?? "", animate: false, fill: true)
                }
            }
            rowText("Twitch picture", detail: "Shown on Twitch and in Twitch chat")
            Spacer(minLength: 8)
            changeButton(.picture, selection: $twitchItem)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    private var sevenTvPictureRow: some View {
        HStack(spacing: 12) {
            pictureFrame(mark: .sevenTV) {
                if let pick = identity?.picture {
                    CosmeticImage(pick.animated ?? pick.still, fallbacks: pick.animated == nil ? [] : [pick.still],
                                  animate: pick.animated != nil, fill: true)
                } else {
                    ZStack {
                        SN.surface
                        PlatformMark(platform: .sevenTV, size: 12, color: SN.textMuted)
                    }
                }
            }
            rowText("7TV picture", detail: sevenTvPending ? "Processing on 7TV"
                                          : identity?.picture == nil ? "Shown to StreamNook and 7TV viewers in chat"
                                                                      : "What StreamNook and 7TV viewers see in chat")
            Spacer(minLength: 8)
            if sevenTvSignedIn {
                changeButton(.sevenTV, selection: $sevenTvItem)
            } else {
                // Signed out of 7TV: sign in first, as the paint tiles do.
                Button { screen.signInToSevenTv() } label: { ChangePill(label: screen.signingIn ? "Connecting" : "Connect") }
                    .buttonStyle(.plain)
                    .disabled(screen.signingIn)
                    .accessibilityLabel("Connect 7TV to change your 7TV picture")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    /// A 44 point round picture with its platform's mark on the corner, so each picture says whose
    /// it is without words.
    private func pictureFrame<Content: View>(mark: PlatformMark.Platform, @ViewBuilder content: () -> Content) -> some View {
        content()
            .frame(width: 44, height: 44)
            .clipShape(Circle())
            .overlay { Circle().stroke(Color.white.opacity(0.1), lineWidth: 1) }
            .overlay(alignment: .bottomTrailing) {
                PlatformMark(platform: mark, size: mark == .sevenTV ? 9 : 13)
                    .padding(mark == .sevenTV ? 3 : 2)
                    .background(Circle().fill(SN.backgroundTertiary))
                    .offset(x: 4, y: 3)
            }
            .accessibilityHidden(true)
    }

    // MARK: Banner

    private var bannerRow: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                rowText("Banner", detail: "The wide image across your Twitch channel")
                Spacer(minLength: 8)
                changeButton(.banner, selection: $bannerItem)
            }
            Group {
                if let banner {
                    Image(uiImage: banner).resizable().scaledToFill()
                } else if let url = identity?.bannerURL {
                    CosmeticImage(url, animate: false, fill: true)
                }
            }
            .frame(height: 64)
            .frame(maxWidth: .infinity)
            .clipShape(RoundedRectangle(cornerRadius: SNRadius.thumb, style: .continuous))
            .opacity(banner == nil && identity?.bannerURL == nil ? 0 : 1)
            .frame(height: banner == nil && identity?.bannerURL == nil ? 0 : 64)
            .accessibilityHidden(true)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    // MARK: Name

    private var nameRow: some View {
        let changed = draftName != user.displayName && !draftName.isEmpty
        let sameName = draftName.lowercased() == user.login.lowercased()
        return VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Display name").font(.snRowDetail).foregroundStyle(SN.textMuted)
                    TextField("Display name", text: $draftName)
                        .font(.snRowStrong)
                        .foregroundStyle(SN.textPrimary)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .submitLabel(.done)
                        .focused($nameFocused)
                        .onSubmit { if changed && sameName { Task { await saveName() } } }
                }
                Spacer(minLength: 8)
                if changed {
                    Button { Task { await saveName() } } label: {
                        Text(savingName ? "Saving" : "Save")
                            .font(.sn(13, 700))
                            .foregroundStyle(SN.background)
                            .padding(.horizontal, 14)
                            .frame(height: 32)
                            .background(Capsule().fill(SN.accent))
                            .frame(minHeight: 44)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(!sameName || savingName)
                    .opacity(sameName ? 1 : 0.45)
                }
            }
            // Twitch lets you change only how your login is capitalized.
            if changed && !sameName {
                Text("Only the capitalization of your name can change.")
                    .font(.snCaption)
                    .foregroundStyle(SN.warning)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .animation(.easeOut(duration: 0.15), value: changed)
    }

    // MARK: Bio

    private var bioRow: some View {
        Button { screen.editingBio = true } label: {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Bio").font(.snRowDetail).foregroundStyle(SN.textMuted)
                    let bio = screen.bio ?? ""
                    Text(bio.isEmpty ? "Add a few words about you" : bio)
                        .font(.snBody)
                        .foregroundStyle(bio.isEmpty ? SN.textSecondary : SN.textPrimary)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                }
                Spacer(minLength: 8)
                Image(systemName: Symbol.disclosure)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(SN.textMuted)
                    .padding(.top, 14)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowPress())
    }

    private func rowText(_ title: String, detail: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.snRowStrong).foregroundStyle(SN.textPrimary)
            Text(detail).font(.snRowDetail).foregroundStyle(SN.textMuted).lineLimit(1)
        }
    }

    /// The row's Change: a glass pill that opens the photo picker, a spinner while it uploads.
    @ViewBuilder
    private func changeButton(_ kind: ImageKind, selection: Binding<PhotosPickerItem?>) -> some View {
        if uploading == kind {
            ProgressView().tint(SN.textSecondary).frame(width: 44, height: 44)
        } else {
            PhotosPicker(selection: selection, matching: .images, photoLibrary: .shared()) {
                ChangePill(label: "Change")
            }
            .disabled(uploading != nil)
            .accessibilityLabel(Self.changeLabel(kind))
        }
    }

    private static func changeLabel(_ kind: ImageKind) -> String {
        switch kind {
        case .picture: "Change your Twitch picture"
        case .sevenTV: "Change your 7TV picture"
        case .banner: "Change your banner"
        }
    }

    // MARK: Writes

    /// Crops the pick to the shape it is shown in (square pictures, a 5:2 banner), sizes it down,
    /// and hands the core the bytes. A Twitch pick shows at once and is dropped if Twitch says no;
    /// a 7TV pick is reviewed by 7TV first, so the row says so instead of showing it as live. An
    /// animated GIF goes to 7TV untouched, so it keeps moving.
    private func upload(_ item: PhotosPickerItem, as kind: ImageKind) async {
        defer {
            uploading = nil
            switch kind {
            case .picture: twitchItem = nil
            case .sevenTV: sevenTvItem = nil
            case .banner: bannerItem = nil
            }
        }
        guard let data = try? await item.loadTransferable(type: Data.self), let image = UIImage(data: data),
              let prepared = ProfileImage.prepare(image, kind: kind) else {
            model.toasts.show("Could not read that image.")
            return
        }
        uploading = kind
        if kind == .sevenTV {
            await uploadSevenTv(ProfileImage.isGIF(data) && data.count <= ProfileImage.sevenTvLimit ? data : prepared.jpeg)
            return
        }
        if kind == .picture { picture = prepared.image } else { banner = prepared.image }
        do {
            _ = try await model.core.invoke("upload_twitch_profile_image",
                                            args: ["kind": kind.rawValue, "dataBase64": prepared.jpeg.base64EncodedString()])
            // Twitch serves the new image a few seconds later; the header re-reads after it.
            try? await Task.sleep(for: .seconds(4))
            await model.signIn.refresh()
        } catch {
            if kind == .picture { picture = nil } else { banner = nil }
            let reason = (error as? CoreFailure)?.description ?? ""
            model.toasts.show(reason.isEmpty ? "Twitch did not take that image. Try again in a moment." : reason)
        }
    }

    private func uploadSevenTv(_ bytes: Data) async {
        guard let id = screen.ext?.sevenTvUserID else {
            model.toasts.show("7TV did not identify your account.")
            return
        }
        do {
            _ = try await model.core.invoke("upload_seventv_profile_picture",
                                            args: ["userId": id, "dataBase64": bytes.base64EncodedString()])
            sevenTvPending = true
        } catch {
            let text = "\(error)"
            if text.contains("SESSION_EXPIRED") {
                model.toasts.show("Your 7TV sign-in expired. Sign back in, then pick the picture again.")
                screen.signInToSevenTv()
            } else {
                let reason = (error as? CoreFailure)?.description ?? ""
                model.toasts.show(reason.isEmpty ? "7TV did not take that picture." : reason)
            }
        }
    }

    private func saveName() async {
        guard !savingName else { return }
        savingName = true
        defer { savingName = false }
        do {
            _ = try await model.core.invoke("update_twitch_display_name", args: ["displayName": draftName])
            nameFocused = false
            await model.signIn.refresh()
        } catch {
            let reason = (error as? CoreFailure)?.description ?? ""
            model.toasts.show(reason.isEmpty ? "Could not change your name." : reason)
        }
    }
}

/// A row's action: a glass pill at the rows' 32 point pill height inside a 44 point target. Its own
/// view, so a photo picker's label (not main-actor) can hold it.
private struct ChangePill: View {
    let label: String

    var body: some View {
        Text(label)
            .font(.sn(13, 600))
            .foregroundStyle(SN.textPrimary)
            .padding(.horizontal, 14)
            .frame(height: 32)
            .snGlass(Capsule(), interactive: true)
            .frame(minHeight: 44)
            .contentShape(Rectangle())
    }
}

/// An image cut to the shape it is shown in and sized for upload.
enum ProfileImage {
    struct Prepared {
        let image: UIImage
        let jpeg: Data
    }

    /// 7TV takes up to 7 MB.
    static let sevenTvLimit = 7 * 1024 * 1024

    static func isGIF(_ data: Data) -> Bool { data.starts(with: Array("GIF8".utf8)) }

    /// Pictures are square at 600 px, the banner 5:2 at 1200 by 480; all center-cropped.
    static func prepare(_ source: UIImage, kind: TwitchProfileSection.ImageKind) -> Prepared? {
        let target = kind == .banner ? CGSize(width: 1200, height: 480) : CGSize(width: 600, height: 600)
        let size = source.size
        guard size.width > 0, size.height > 0 else { return nil }
        let scale = max(target.width / size.width, target.height / size.height)
        let drawn = CGSize(width: size.width * scale, height: size.height * scale)
        let origin = CGPoint(x: (target.width - drawn.width) / 2, y: (target.height - drawn.height) / 2)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let image = UIGraphicsImageRenderer(size: target, format: format).image { _ in
            source.draw(in: CGRect(origin: origin, size: drawn))
        }
        guard let jpeg = image.jpegData(compressionQuality: 0.88) else { return nil }
        return Prepared(image: image, jpeg: jpeg)
    }
}
