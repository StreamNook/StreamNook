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
    /// The name on the Images tab's host tabs.
    var short: String
    var note: String
    var maxMB: Int
    /// Deleting an upload deletes the file there, not only the list entry.
    var deletesFromHost: Bool
    /// Its links stop working after a while.
    var expires: Bool

    enum CodingKeys: String, CodingKey {
        case id, label, short, note, expires
        case maxMB = "max_mb"
        case deletesFromHost = "deletes_from_host"
    }

    /// The small line under a host in the host list: "up to 20 MB · links expire · you can delete
    /// uploads", each part only when it is true.
    var facts: String {
        var parts = ["up to \(maxMB) MB"]
        if expires { parts.append("links expire") }
        if deletesFromHost { parts.append("you can delete uploads") }
        return parts.joined(separator: " \u{00B7} ")
    }
}

/// One upload (Rust `UploadedImage`): what `upload_chat_image` and `reupload_image` answer, and
/// what the panel's groups hold.
struct UploadedImageData: Decodable, Sendable, Hashable {
    var url: String
    var width: Int
    var height: Int
    /// The host key it went to.
    var host: String
    /// "2 days left", "Expired"; nil for a link that lasts.
    var timeLeft: String?
    var expired: Bool
    /// A kept copy can be sent again for a fresh link.
    var canReupload: Bool
    /// The kept copy on this phone, drawn in place of a dead link.
    var copyPath: String?

    enum CodingKeys: String, CodingKey {
        case url, width, height, host, expired
        case timeLeft = "time_left"
        case canReupload = "can_reupload"
        case copyPath = "copy_path"
    }

    /// The kept copy as an address the image pipeline reads from disk.
    var copyURL: String? { copyPath.map { URL(fileURLWithPath: $0).absoluteString } }

    /// Width over height; 1 when the size is unknown.
    var shape: CGFloat { width > 0 && height > 0 ? CGFloat(width) / CGFloat(height) : 1 }
}

/// The uploads to one host, newest first (Rust `UploadGroup`).
struct UploadGroupData: Decodable, Sendable {
    var host: UploadHostData
    var items: [UploadedImageData]
}

/// The emote panel's Images section as the core reads it (`image_upload_panel`): the host the next
/// picture goes to, the hosts it can switch to, whether the viewer was asked about it yet, and the
/// uploads by host, the host used most recently first.
struct ImageUploadPanelData: Decodable, Sendable {
    var host: UploadHostData
    var hosts: [UploadHostData]
    var needsConsent: Bool
    var groups: [UploadGroupData]

    enum CodingKeys: String, CodingKey { case host, hosts, groups, needsConsent = "needs_consent" }
}

/// The Images section's state. The core owns the host, the checks, the post, deleting and sending
/// again (Rust `services/image_upload.rs`); this only gets the picture there, runs the one-time
/// question about the host and keeps what the pane shows. Lives in the emote picker's model, so an
/// upload survives a section switch.
@Observable @MainActor
final class ImageUploadModel {
    private(set) var panel: ImageUploadPanelData?
    /// The host's name while a picture is on its way there.
    private(set) var uploadingTo: String?
    /// Why the last upload, delete or send-again did not go through, under the tile.
    private(set) var error = ""
    /// Uploads whose picture loaded from nowhere (link and kept copy); their tiles hide.
    private(set) var failed: Set<String> = []
    /// Uploads being deleted or sent again; their tiles show a spinner.
    private(set) var working: Set<String> = []
    /// "all", or the host id whose uploads show.
    var filter = "all"
    /// Edit mode: a tap deletes. It turns itself off once nothing is left.
    var editing = false
    /// The host list under "Uploads go to" is open.
    var hostsOpen = false
    /// The host question is up; its buttons answer it through `answer(_:)`.
    private(set) var asking = false
    /// The host the question is about.
    private(set) var askingHost = ""
    @ObservationIgnored private var waiting: CheckedContinuation<Bool, Never>?
    @ObservationIgnored private let core: CoreBridge
    /// The app's settings, where the upload host is written.
    @ObservationIgnored private let settings: @MainActor () -> SettingsStore?

    /// The upload host, the same setting as Settings > Image Uploads > Where images go.
    static let presetPath = "chat_input.image_uploader.preset"

    init(core: CoreBridge, settings: @escaping @MainActor () -> SettingsStore?) {
        self.core = core
        self.settings = settings
    }

    var busy: Bool { uploadingTo != nil }

    var groups: [UploadGroupData] { panel?.groups ?? [] }

    /// Every upload, across hosts.
    var total: Int { groups.reduce(0) { $0 + $1.items.count } }

    /// The filter, back on All once its host has no uploads left.
    var activeFilter: String { groups.contains { $0.host.id == filter } ? filter : "all" }

    var shownGroups: [UploadGroupData] {
        let shown = activeFilter
        return shown == "all" ? groups : groups.filter { $0.host.id == shown }
    }

    var isEditing: Bool { editing && total > 0 }

    /// The line under the heading in Edit, by what a delete does on the hosts shown.
    var editHint: String {
        let deleting = shownGroups.map(\.host.deletesFromHost)
        if deleting.allSatisfy({ $0 }) { return "Tap a picture to delete it." }
        if deleting.contains(true) {
            return "Tap a picture to delete it. Hosts marked with a bin delete the file; the others keep theirs, so their links still work."
        }
        return "Tap a picture to remove it from this list. The host keeps the file, so its link still works."
    }

    /// A group's uploads that have something to draw.
    func visible(_ group: UploadGroupData) -> [UploadedImageData] {
        group.items.filter { !failed.contains($0.url) }
    }

    func load() async {
        if let fresh = try? await core.invoke("image_upload_panel", args: [:], as: ImageUploadPanelData.self) {
            panel = fresh
            if fresh.groups.allSatisfy({ $0.items.isEmpty }) { editing = false }
        }
    }

    func hide(_ url: String) {
        failed.insert(url)
    }

    /// Where the next picture goes: writes the setting, closes the list and reads the panel again
    /// (the new host may still need its question).
    func pickHost(_ id: String) async {
        hostsOpen = false
        guard id != panel?.host.id, let store = settings() else { return }
        await store.set(Self.presetPath, id)
        await load()
    }

    /// Sends a kept copy again for a fresh link, which takes the old one's place in the list.
    /// Returns the new link, or nil with the core's words under the tile.
    func reupload(_ item: UploadedImageData) async -> String? {
        let url = item.url
        guard !working.contains(url) else { return nil }
        error = ""
        working.insert(url)
        defer { working.remove(url) }
        do {
            let fresh = try await core.invoke("reupload_image", args: ["url": url], as: UploadedImageData.self)
            await load()
            return fresh.url
        } catch {
            self.error = Self.words(error, or: "Sending it again failed.")
            return nil
        }
    }

    /// Takes an upload off the list, deleting the file on its host where the host allows that.
    func remove(_ item: UploadedImageData) async {
        let url = item.url
        guard !working.contains(url) else { return }
        error = ""
        working.insert(url)
        defer { working.remove(url) }
        do {
            _ = try await core.invoke("delete_image_upload", args: ["url": url])
            await load()
        } catch {
            self.error = Self.words(error, or: "Deleting it failed.")
        }
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

    /// A core error in words: the core's own sentence when it sent one, else `fallback`.
    static func words(_ error: Error, or fallback: String = "") -> String {
        let said = (error as? CoreFailure)?.description ?? ""
        return said.isEmpty ? fallback : said
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

/// The emote panel's Images section, as on the desktop and Android: where pictures go (switchable
/// here), Choose an image, and the uploads, grouped by host with a tab per host. A tap puts an
/// upload's link in the message; an expired one is sent again first. Edit deletes uploads (on the
/// host too where it allows that).
struct ChatImageUploadPane: View {
    let model: ChatComposerModel
    @State private var item: PhotosPickerItem?
    private var images: ImageUploadModel { model.picker.images }

    /// A row's height before it stretches to fill the width.
    private static let rowHeight: CGFloat = 92

    /// A tile's shape: the picture's own, held between 0.8 and 4.5 (past these it draws whole
    /// inside its tile instead of filling it).
    static func tileAspect(_ shape: CGFloat) -> CGFloat { min(4.5, max(0.8, shape)) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 6) {
                    hostLine
                    if images.hostsOpen, let panel = images.panel { hostList(panel) }
                    chooseTile
                    if !images.error.isEmpty {
                        Text(images.error)
                            .font(.sn(12))
                            .foregroundStyle(SN.error)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 4)
                    }
                }
                if images.total > 0 { uploads }
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
        .onDisappear {
            if images.asking { images.answer(false) }
            // Every visit starts out of Edit (where a tap deletes), with the host list shut.
            images.editing = false
            images.hostsOpen = false
        }
    }

    // MARK: Where uploads go

    private var hostLine: some View {
        let open = images.hostsOpen
        let label = images.panel?.host.label
        return HStack(spacing: 2) {
            Text("Uploads go to")
                .font(.sn(11))
                .foregroundStyle(SN.textSecondary)
            Button {
                withAnimation(.easeOut(duration: 0.18)) { images.hostsOpen.toggle() }
            } label: {
                HStack(spacing: 3) {
                    Text(label ?? "\u{2026}")
                        .font(.sn(11, 600))
                        .foregroundStyle(SN.textPrimary)
                    Image(systemName: "chevron.down")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(SN.textSecondary)
                        .rotationEffect(.degrees(open ? 180 : 0))
                }
                .padding(.horizontal, 4)
                .frame(minHeight: 28)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(label == nil)
            .accessibilityLabel("Uploads go to \(label ?? "")")
            .accessibilityHint("Changes where pictures are sent")
        }
        .padding(.horizontal, 4)
    }

    private func hostList(_ panel: ImageUploadPanelData) -> some View {
        let current = panel.host.id
        return VStack(spacing: 0) {
            ForEach(Array(panel.hosts.enumerated()), id: \.element.id) { index, host in
                if index > 0 { Rectangle().fill(SN.borderSubtle).frame(height: 1) }
                hostRow(host, on: host.id == current)
            }
        }
        .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .transition(.opacity)
    }

    private func hostRow(_ host: UploadHostData, on: Bool) -> some View {
        let images = images
        let id = host.id
        return Button {
            Task { await images.pickHost(id) }
        } label: {
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(host.label)
                        .font(.sn(13, 500))
                        .foregroundStyle(SN.textPrimary)
                    Text(host.note)
                        .font(.sn(11))
                        .foregroundStyle(SN.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(host.facts)
                        .font(.sn(10))
                        .foregroundStyle(SN.textMuted)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                if on {
                    Image(systemName: "checkmark")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(SN.accent)
                        .padding(.top, 2)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .frame(minHeight: 44)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }

    private var chooseTile: some View {
        // Read here: the picker's label is not main-actor, so it takes plain values.
        let uploading = images.uploadingTo
        return PhotosPicker(selection: $item, matching: .images, photoLibrary: .shared()) {
            ChooseImageTile(uploadingTo: uploading)
        }
        .buttonStyle(.plain)
        .disabled(images.busy || images.panel == nil)
        .accessibilityLabel(uploading.map { "Uploading to \($0)" } ?? "Choose an image")
    }

    // MARK: Uploads

    private struct HostTab: Identifiable {
        var id: String
        var short: String
        var count: Int
    }

    private var uploads: some View {
        let editing = images.isEditing
        let groups = images.groups
        let headed = images.activeFilter == "all" && groups.count > 1
        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("UPLOADS")
                    .font(.system(size: 10, weight: .bold))
                    .tracking(1)
                    .foregroundStyle(SN.textSecondary)
                Spacer(minLength: 8)
                Button {
                    images.editing = !editing
                } label: {
                    Text(editing ? "Done" : "Edit")
                        .font(.sn(11, 500))
                        .foregroundStyle(editing ? SN.accent : SN.textSecondary)
                        .padding(.horizontal, 6)
                        .frame(minHeight: 28)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
            }
            .padding(.leading, 4)
            if groups.count > 1 { hostTabs(groups) }
            if editing {
                Text(images.editHint)
                    .font(.sn(11))
                    .foregroundStyle(SN.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 4)
            }
            ForEach(images.shownGroups, id: \.host.id) { group in
                groupBlock(group, heading: headed, editing: editing)
            }
        }
    }

    /// All, then one tab per host with uploads, in the emote picker's tab style.
    private func hostTabs(_ groups: [UploadGroupData]) -> some View {
        let active = images.activeFilter
        let tabs = [HostTab(id: "all", short: "All", count: images.total)]
            + groups.map { HostTab(id: $0.host.id, short: $0.host.short, count: $0.items.count) }
        return HStack(spacing: 4) {
            ForEach(tabs) { tab in
                let on = active == tab.id
                Button {
                    images.filter = tab.id
                } label: {
                    VStack(spacing: 2) {
                        Text(tab.short)
                            .font(.sn(12, 600))
                            .lineLimit(1)
                            .frame(height: 16)
                        Text("\(tab.count)")
                            .font(.sn(10).monospacedDigit())
                    }
                    .foregroundStyle(on ? Color.white : ComposerInk.quiet)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
                    .background(on ? Color.white.opacity(0.08) : .clear, in: .rect(cornerRadius: 8))
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("\(tab.short), \(tab.count)")
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
    }

    @ViewBuilder
    private func groupBlock(_ group: UploadGroupData, heading: Bool, editing: Bool) -> some View {
        let items = images.visible(group)
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                if heading {
                    HStack(alignment: .firstTextBaseline) {
                        Text(group.host.label.uppercased())
                            .font(.system(size: 10, weight: .bold))
                            .tracking(1)
                            .foregroundStyle(SN.textSecondary)
                            .lineLimit(1)
                        Spacer(minLength: 8)
                        if group.host.expires {
                            Text("links expire")
                                .font(.sn(10))
                                .foregroundStyle(SN.textMuted)
                        }
                    }
                    .padding(.horizontal, 4)
                    .padding(.top, 4)
                }
                JustifiedRows(rowHeight: Self.rowHeight, spacing: 8) {
                    ForEach(items, id: \.url) { item in
                        tile(item, deletes: group.host.deletesFromHost, editing: editing)
                            .layoutValue(key: TileShape.self, value: Self.tileAspect(item.shape))
                    }
                }
            }
        }
    }

    /// One upload: the picture at its own shape, a round glyph on its corner (delete in Edit, send
    /// again where a kept copy allows it, a spinner while either runs), and how long its link has
    /// left under it. Never words over the picture.
    private func tile(_ item: UploadedImageData, deletes: Bool, editing: Bool) -> some View {
        let shape = item.shape
        let aspect = Self.tileAspect(shape)
        let fits = aspect == shape
        let working = images.working.contains(item.url)
        let copy = item.copyURL
        // An expired link draws its kept copy; a live one falls back to the copy if it fails.
        let source = (item.expired ? copy : nil) ?? item.url
        let fallbacks = item.expired ? [] : (copy.map { [$0] } ?? [])
        let url = item.url
        let images = images
        let label = editing
            ? (deletes ? "Delete this picture" : "Remove this picture from the list")
            : (item.expired && item.canReupload ? "Send this picture again and insert its new link" : "Insert this image's link")
        return VStack(alignment: .leading, spacing: 4) {
            Button {
                pick(item)
            } label: {
                Color.clear
                    .aspectRatio(aspect, contentMode: .fit)
                    .overlay {
                        CosmeticImage(source, fallbacks: fallbacks, animate: model.session.style.animatesEmotes,
                                      fill: fits, pointHeight: 180, onFailure: { images.hide(url) })
                            .opacity(item.expired ? 0.5 : 1)
                    }
                    .background(fits ? Color.white.opacity(0.04) : Color.black.opacity(0.3))
                    .clipShape(.rect(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(SN.borderSubtle))
                    .overlay(alignment: .topTrailing) {
                        if working {
                            CornerGlyph { ProgressView().tint(.white).scaleEffect(0.6) }
                                .padding(6)
                        } else if editing {
                            CornerGlyph { Image(systemName: deletes ? "trash" : "xmark") }
                                .padding(6)
                        }
                    }
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(working)
            .accessibilityLabel(label)
            .overlay(alignment: .topTrailing) {
                if item.canReupload, !editing, !working {
                    Button {
                        Task { _ = await images.reupload(item) }
                    } label: {
                        // The padding is the glyph's inset and widens what a finger can hit.
                        CornerGlyph { Image(systemName: "arrow.clockwise") }
                            .padding(6)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Send it again for a fresh link")
                }
            }
            if let left = item.timeLeft {
                Text(left)
                    .font(.sn(10))
                    .foregroundStyle(item.expired ? SN.warning : SN.textMuted)
                    .lineLimit(1)
                    .padding(.horizontal, 2)
            }
        }
    }

    /// A tap: deletes in Edit; an expired link with a kept copy is sent again and its new link
    /// inserted; anything else inserts its link.
    private func pick(_ item: UploadedImageData) {
        let images = images
        let composer = model
        guard !images.working.contains(item.url) else { return }
        if images.isEditing {
            Task { await images.remove(item) }
        } else if item.expired, item.canReupload {
            Task {
                if let link = await images.reupload(item) { composer.insert(link) }
            }
        } else {
            composer.insert(item.url)
        }
    }
}

/// The round glyph on a tile's corner (delete, send again, a spinner).
private struct CornerGlyph<Content: View>: View {
    @ViewBuilder let content: () -> Content

    var body: some View {
        content()
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 24, height: 24)
            .background(Color.black.opacity(0.62), in: Circle())
    }
}

/// A tile's width over its height, read by `JustifiedRows`.
private struct TileShape: LayoutValueKey {
    static let defaultValue: CGFloat = 1
}

/// Justified rows, as the desktop's wrapping flex rows: a row takes tiles while they fit side by
/// side at the base height, then every tile in it grows in proportion to its shape until the row
/// fills the width, so the row shares one picture height. The last row keeps the base height. A
/// lone tile wider than the pane shrinks to fit it. Each tile is offered its width and sizes its
/// own height (the picture, plus a line under it where it has one).
struct JustifiedRows: Layout {
    var rowHeight: CGFloat
    var spacing: CGFloat

    private struct Row {
        var range: Range<Int>
        /// The pictures' height in this row.
        var height: CGFloat
    }

    private func rows(_ shapes: [CGFloat], width: CGFloat) -> [Row] {
        var out: [Row] = []
        var start = 0
        while start < shapes.count {
            var end = start + 1
            var used = shapes[start] * rowHeight
            while end < shapes.count, used + spacing + shapes[end] * rowHeight <= width + 0.5 {
                used += spacing + shapes[end] * rowHeight
                end += 1
            }
            let gaps = spacing * CGFloat(end - start - 1)
            let fill = max(width - gaps, 1) / max(shapes[start..<end].reduce(0, +), 0.01)
            out.append(Row(range: start..<end, height: end == shapes.count ? min(rowHeight, fill) : fill))
            start = end
        }
        return out
    }

    private func rowWidth(_ proposal: ProposedViewSize, _ shapes: [CGFloat]) -> CGFloat {
        if let width = proposal.width, width.isFinite { return width }
        let natural = shapes.reduce(0) { $0 + $1 * rowHeight }
        return natural + spacing * CGFloat(max(shapes.count - 1, 0))
    }

    /// The tallest tile in a row at its width.
    private func extent(_ row: Row, _ shapes: [CGFloat], _ subviews: Subviews) -> CGFloat {
        row.range.map { i in
            subviews[i].sizeThatFits(ProposedViewSize(width: shapes[i] * row.height, height: nil)).height
        }.max() ?? 0
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let shapes = subviews.map { $0[TileShape.self] }
        let width = rowWidth(proposal, shapes)
        var height: CGFloat = 0
        for (n, row) in rows(shapes, width: width).enumerated() {
            if n > 0 { height += spacing }
            height += extent(row, shapes, subviews)
        }
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let shapes = subviews.map { $0[TileShape.self] }
        var y = bounds.minY
        for row in rows(shapes, width: bounds.width) {
            var x = bounds.minX
            for i in row.range {
                let w = shapes[i] * row.height
                subviews[i].place(at: CGPoint(x: x, y: y), anchor: .topLeading,
                                  proposal: ProposedViewSize(width: w, height: nil))
                x += w + spacing
            }
            y += extent(row, shapes, subviews) + spacing
        }
    }
}

/// The Choose an image tile: a glass tile with the formats under it, a spinner and the host's name
/// while a picture uploads (the formats line kept, only hidden, so nothing moves). Its own view, so
/// the photo picker's label can hold it.
private struct ChooseImageTile: View {
    let uploadingTo: String?

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
            Text("JPG, PNG, GIF or WebP")
                .font(.sn(11))
                .foregroundStyle(SN.textSecondary)
                .opacity(uploadingTo == nil ? 1 : 0)
                .accessibilityHidden(uploadingTo != nil)
        }
        .multilineTextAlignment(.center)
        .padding(.horizontal, 16)
        .frame(maxWidth: .infinity, minHeight: 112)
        .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous), interactive: true)
        .contentShape(.rect)
    }
}
