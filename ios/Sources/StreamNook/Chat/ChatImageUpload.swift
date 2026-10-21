import ImageIO
import PhotosUI
import SNKit
import SwiftUI
import UIKit
// PhotosPicker lives in PhotosUI's SwiftUI overlay, which this cross-compiled SDK does not import on
// its own.
import _PhotosUI_SwiftUI

/// One image host StreamNook can upload to (Rust `UploadHost`).
struct UploadHostData: Decodable, Sendable, Hashable, Identifiable {
    var id: String
    var label: String
    var note: String
    var maxMB: Int

    enum CodingKeys: String, CodingKey { case id, label, note, maxMB = "max_mb" }
}

/// A picture that went up (Rust `UploadedImage`).
struct UploadedImageData: Decodable, Sendable, Hashable {
    var url: String
    var width: Int
    var height: Int
}

/// The emote panel's Images section as the core reads it (`image_upload_panel`): the host the
/// pictures go to, whether the viewer was asked about it yet, and the pictures sent lately.
struct ImageUploadPanelData: Decodable, Sendable {
    var host: UploadHostData
    var needsConsent: Bool
    var recents: [UploadedImageData]

    enum CodingKeys: String, CodingKey { case host, recents, needsConsent = "needs_consent" }
}

/// The Images section's state. The core owns the host, the checks and the post (Rust
/// `services/image_upload.rs`); this only gets the picture there and runs the one-time question
/// about the host. Lives in the emote picker's model, so an upload survives a section switch.
@Observable @MainActor
final class ImageUploadModel {
    private(set) var panel: ImageUploadPanelData?
    /// The host's name while a picture is on its way there.
    private(set) var uploadingTo: String?
    /// Why the last upload did not go through, under the tile.
    private(set) var error = ""
    /// Recent pictures that did not load; their tiles hide.
    private(set) var failed: Set<String> = []
    /// The host question is up; its buttons answer it through `answer(_:)`.
    private(set) var asking = false
    /// The host the question is about.
    private(set) var askingHost = ""
    @ObservationIgnored private var waiting: CheckedContinuation<Bool, Never>?
    @ObservationIgnored private let core: CoreBridge

    init(core: CoreBridge) {
        self.core = core
    }

    var busy: Bool { uploadingTo != nil }

    /// The recent pictures that still load.
    var recents: [UploadedImageData] { (panel?.recents ?? []).filter { !failed.contains($0.url) } }

    func load() async {
        if let fresh = try? await core.invoke("image_upload_panel", args: [:], as: ImageUploadPanelData.self) {
            panel = fresh
        }
    }

    func hide(_ url: String) {
        failed.insert(url)
    }

    /// The host question's answer (Upload or Cancel, or the pane going away).
    func answer(_ yes: Bool) {
        asking = false
        waiting?.resume(returning: yes)
        waiting = nil
    }

    /// Asks once per host before the first picture goes to it.
    private func confirm(_ panel: ImageUploadPanelData) async -> Bool {
        guard panel.needsConsent else { return true }
        answer(false)
        askingHost = panel.host.label
        let yes = await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            self.waiting = continuation
            self.asking = true
        }
        guard yes else { return false }
        do {
            _ = try await core.invoke("accept_image_upload_host", args: [:])
            return true
        } catch {
            self.error = Self.words(error)
            return false
        }
    }

    /// Sends the picked photo and returns its link. Asks about the host first when it is new to
    /// the viewer, and asks again (once) when the host changed while the panel was open.
    func send(_ item: PhotosPickerItem) async -> String? {
        guard !busy else { return nil }
        error = ""
        defer { uploadingTo = nil }
        var body: String?
        for attempt in 0..<2 {
            await load()
            guard let panel else {
                error = "Image upload failed."
                return nil
            }
            guard await confirm(panel) else { return nil }
            uploadingTo = panel.host.label
            if body == nil {
                guard let picked = try? await item.loadTransferable(type: Data.self) else {
                    error = "That image could not be opened."
                    return nil
                }
                // Off the main thread: a big photo is decoded and shrunk here.
                body = await Task.detached(priority: .userInitiated) {
                    ChatUploadImage.prepare(picked)?.base64EncodedString()
                }.value
                guard body != nil else {
                    error = "That image could not be opened."
                    return nil
                }
            }
            do {
                let image = try await core.invoke("upload_chat_image", args: ["dataBase64": body],
                                                  as: UploadedImageData.self)
                await load()
                return image.url
            } catch {
                let words = Self.words(error)
                if words == "consent-required", attempt == 0 {
                    uploadingTo = nil
                    continue
                }
                self.error = words == "consent-required" || words.isEmpty ? "Image upload failed." : words
                return nil
            }
        }
        error = "Image upload failed."
        return nil
    }

    /// A core error in words: the core's own sentence when it sent one.
    static func words(_ error: Error) -> String {
        (error as? CoreFailure)?.description ?? ""
    }
}

/// What the core is sent: GIF always as it is; PNG, JPEG and WebP as they are at 4096 px or
/// less; anything else (HEIC, a 48 MP photo) through ImageIO at 4096 px on its long edge as JPEG,
/// upright, in its own color space and with no metadata. The core removes what is left (EXIF in a
/// JPEG or PNG).
enum ChatUploadImage {
    static func prepare(_ data: Data) -> Data? {
        guard let src = CGImageSourceCreateWithData(data as CFData, nil),
              let type = CGImageSourceGetType(src) as String? else { return nil }
        let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any]
        let w = props?[kCGImagePropertyPixelWidth] as? Int ?? 0
        let h = props?[kCGImagePropertyPixelHeight] as? Int ?? 0
        // Type identifiers as text, so this needs no UniformTypeIdentifiers overlay.
        if type == "com.compuserve.gif" { return data }
        let asIs: Set<String> = ["public.png", "public.jpeg", "org.webmproject.webp"]
        if asIs.contains(type), max(w, h) > 0, max(w, h) <= 4096 { return data }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 4096,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { return nil }
        let out = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(out as CFMutableData, "public.jpeg" as CFString, 1, nil) else {
            return nil
        }
        CGImageDestinationAddImage(dest, image, [kCGImageDestinationLossyCompressionQuality: 0.92] as CFDictionary)
        return CGImageDestinationFinalize(dest) ? out as Data : nil
    }
}

/// The emote panel's Images section: choose a picture, it goes to the host picked in Settings,
/// and its link lands in the message. Below, the pictures sent lately, one tap to use again.
struct ChatImageUploadPane: View {
    let model: ChatComposerModel
    @State private var item: PhotosPickerItem?
    private var images: ImageUploadModel { model.picker.images }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    chooseTile
                    if !images.error.isEmpty {
                        Text(images.error)
                            .font(.sn(12))
                            .foregroundStyle(SN.error)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 4)
                    }
                }
                if !images.recents.isEmpty { recentGrid }
            }
            .padding(4)
        }
        .mask(ChatEmotePicker.edgeFade)
        .task { await images.load() }
        .onChange(of: item) { _, picked in
            guard let picked else { return }
            item = nil
            let composer = model
            Task {
                if let link = await composer.picker.images.send(picked) { composer.insert(link) }
            }
        }
        .alert("Send images to this host?", isPresented: Binding(
            get: { images.asking }, set: { if !$0 { images.answer(false) } }
        )) {
            Button("Upload") { images.answer(true) }
            Button("Cancel", role: .cancel) { images.answer(false) }
        } message: {
            Text("Images you upload go to \(images.askingHost), and anyone with the link can see them.")
        }
        .onDisappear { if images.asking { images.answer(false) } }
    }

    private var chooseTile: some View {
        // Read here: the picker's label is not main-actor, so it takes plain values.
        let uploading = images.uploadingTo
        let host = images.panel?.host.label
        return PhotosPicker(selection: $item, matching: .images, photoLibrary: .shared()) {
            ChooseImageTile(uploadingTo: uploading, host: host)
        }
        .buttonStyle(.plain)
        .disabled(images.busy || images.panel == nil)
        .accessibilityLabel(uploading.map { "Uploading to \($0)" } ?? "Choose an image")
    }

    private var recentGrid: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("RECENT")
                .font(.system(size: 10, weight: .bold))
                .tracking(1)
                .foregroundStyle(SN.textSecondary)
                .padding(.horizontal, 4)
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 3), spacing: 8) {
                ForEach(images.recents, id: \.url) { recent in recentTile(recent) }
            }
        }
    }

    private func recentTile(_ recent: UploadedImageData) -> some View {
        let url = recent.url
        return Button {
            model.insert(url)
        } label: {
            Color.clear
                .aspectRatio(1, contentMode: .fit)
                .overlay {
                    CosmeticImage(url, animate: model.session.style.animatesEmotes, fill: true, pointHeight: 120,
                                  onFailure: { images.hide(url) })
                }
                .background(Color.white.opacity(0.04))
                .clipShape(.rect(cornerRadius: 10))
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(SN.borderSubtle))
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Insert this image's link")
    }
}

/// The Choose an image tile: a glass tile with the formats and the host under it, a spinner and
/// the host's name while a picture uploads. Its own view, so the photo picker's label can hold it.
private struct ChooseImageTile: View {
    let uploadingTo: String?
    let host: String?

    var body: some View {
        VStack(spacing: 6) {
            if uploadingTo != nil {
                ProgressView().tint(SN.textSecondary).frame(height: 24)
            } else {
                Image(systemName: "photo.badge.plus")
                    .font(.system(size: 20, weight: .medium))
                    .foregroundStyle(SN.textSecondary)
                    .frame(height: 24)
            }
            Text(uploadingTo.map { "Uploading to \($0)\u{2026}" } ?? "Choose an image")
                .font(.sn(13, 600))
                .foregroundStyle(SN.textPrimary)
            if uploadingTo == nil {
                Text("JPG, PNG, GIF or WebP" + (host.map { ", to \($0)" } ?? ""))
                    .font(.sn(11))
                    .foregroundStyle(SN.textSecondary)
            }
        }
        .multilineTextAlignment(.center)
        .padding(.horizontal, 16)
        .frame(maxWidth: .infinity, minHeight: 112)
        .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous), interactive: true)
        .contentShape(.rect)
    }
}
