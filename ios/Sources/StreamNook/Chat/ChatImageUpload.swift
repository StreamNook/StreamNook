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
    /// How long its links last, when the host says (Litterbox 72, uguu 3).
    var lifetimeHours: Int?

    enum CodingKeys: String, CodingKey {
        case id, label, short, note, expires
        case maxMB = "max_mb"
        case deletesFromHost = "deletes_from_host"
        case lifetimeHours = "lifetime_hours"
    }

    /// How long its links last, as its chip says: "Links last 72 h", or "Links expire" when the
    /// host does not say. Nil for a host that keeps them.
    var lifetimeChip: String? {
        guard expires else { return nil }
        return lifetimeHours.map { "Links last \($0) h" } ?? "Links expire"
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
    /// When its link stops working (ms since the epoch), if it does.
    var expiresMs: Int64?
    /// "2 days left", "Expired"; nil for a link that lasts.
    var timeLeft: String?
    var expired: Bool
    /// A kept copy can be sent again for a fresh link.
    var canReupload: Bool
    /// The kept file on this phone, drawn in place of a dead link.
    var copyPath: String?
    /// The small picture on this phone (320 px), which is what every tile and row draws, so a
    /// full-size upload never sits decoded in memory.
    var thumbPath: String?
    /// Less than a day left on the link.
    var urgent: Bool
    var uploadedMs: Int64
    /// "2 hours ago".
    var uploaded: String
    /// "Oct 21, 3:40 PM".
    var uploadedAt: String
    /// "Expires Oct 23, 3:40 PM (2 days left)", "Expired ...", "Kept indefinitely", or the host
    /// not saying when.
    var expiry: String
    /// "1920 × 1080 · PNG · 1.4 MB".
    var details: String

    enum CodingKeys: String, CodingKey {
        case url, width, height, host, expired, urgent, uploaded, expiry, details
        case expiresMs = "expires_ms"
        case timeLeft = "time_left"
        case canReupload = "can_reupload"
        case copyPath = "copy_path"
        case thumbPath = "thumb_path"
        case uploadedMs = "uploaded_ms"
        case uploadedAt = "uploaded_at"
    }

    /// What a picture of it draws, in order: the small picture, the kept file once the link is
    /// dead, the link while it works, then the kept file. Files as addresses the image pipeline
    /// reads from disk.
    var sources: [String] {
        let copy = copyPath.map(Self.fileAddress)
        let all = [thumbPath.map(Self.fileAddress), expired ? copy : nil, expired ? nil : url, copy].compactMap { $0 }
        var seen = Set<String>()
        return all.filter { seen.insert($0).inserted }
    }

    /// Sending again is offered where the link runs out or ran out, as on the desktop.
    var offersSendAgain: Bool { canReupload && (expiresMs != nil || expired) }

    /// Width over height; 1 when the size is unknown.
    var shape: CGFloat { width > 0 && height > 0 ? CGFloat(width) / CGFloat(height) : 1 }

    private static func fileAddress(_ path: String) -> String { URL(fileURLWithPath: path).absoluteString }
}

/// The uploads to one host, newest first (Rust `UploadGroup`).
struct UploadGroupData: Decodable, Sendable {
    var host: UploadHostData
    var items: [UploadedImageData]
}

/// The emote panel's Images section as the core reads it (`image_upload_panel`): the host the next
/// picture goes to, the hosts it can switch to, whether the viewer was asked about it yet, the
/// uploads by host (the host used most recently first) and what the kept files take on the phone.
/// The phone has no file manager, so the panel's folder button is never drawn here.
struct ImageUploadPanelData: Decodable, Sendable {
    var host: UploadHostData
    var hosts: [UploadHostData]
    var needsConsent: Bool
    var groups: [UploadGroupData]
    /// "48 MB on this device".
    var storage: String

    enum CodingKeys: String, CodingKey { case host, hosts, groups, storage, needsConsent = "needs_consent" }
}

/// The Images section's state. The core owns the host, the checks, the post, deleting and sending
/// again (Rust `services/image_upload.rs`); this only gets the picture there, runs the one-time
/// question about the host and keeps what the pane shows. Lives in the emote picker's model, so an
/// upload survives a section switch, and the uploads manager sheet shares it with the pane.
@Observable @MainActor
final class ImageUploadModel {
    private(set) var panel: ImageUploadPanelData?
    /// The host's name while a picture is on its way there.
    private(set) var uploadingTo: String?
    /// Why the last upload, delete or send-again did not go through, under the card.
    private(set) var error = ""
    /// Uploads whose picture loaded from nowhere (small picture, link and kept file); they hide.
    private(set) var failed: Set<String> = []
    /// Uploads being deleted or sent again; they show a spinner.
    private(set) var working: Set<String> = []
    /// The link just copied, which shows a check for a moment.
    private(set) var copied: String?
    /// "all", or the host id whose uploads show.
    var filter = "all"
    /// Edit mode (the grid only): a tap deletes. It turns itself off once nothing is left.
    var editing = false
    /// The host list inside the send card is open.
    var hostsOpen = false
    /// The uploads manager sheet is up.
    var managerOpen = false
    /// The host question is up; its buttons answer it through `answer(_:)`.
    private(set) var asking = false
    /// The host the question is about.
    private(set) var askingHost = ""
    @ObservationIgnored private var waiting: CheckedContinuation<Bool, Never>?
    @ObservationIgnored private let core: CoreBridge
    /// The app's settings, where the upload host and the view are written.
    @ObservationIgnored private let settings: @MainActor () -> SettingsStore?

    /// The upload host, the same setting as Settings > Image Uploads > Where images go.
    static let presetPath = "chat_input.image_uploader.preset"
    /// Grid or list, the same setting as the desktop's toggle.
    static let viewPath = "chat_input.image_uploader.view"

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

    /// The uploads as a list with each one's details, else the grid of pictures (the default).
    var isList: Bool { settings()?.string(Self.viewPath, "grid") == "list" }

    var isEditing: Bool { editing && total > 0 && !isList }

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

    /// Grid or list, written to the setting so it sticks. The list has no Edit.
    func setView(list: Bool) async {
        guard list != isList, let store = settings() else { return }
        editing = false
        await store.set(Self.viewPath, list ? "list" : "grid")
    }

    /// Puts a link in the message; from the uploads manager, the sheet closes too.
    func insert(_ link: String, into composer: ChatComposerModel, closingManager: Bool) {
        composer.insert(link)
        if closingManager { managerOpen = false }
    }

    /// Copies a working link, with a check on its button for a moment.
    func copy(_ item: UploadedImageData) {
        guard !item.expired else { return }
        let url = item.url
        UIPasteboard.general.string = url
        copied = url
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(1400))
            if self?.copied == url { self?.copied = nil }
        }
    }

    /// Sends a kept copy again for a fresh link, which takes the old one's place in the list.
    /// Returns the new link, or nil with the core's words under the card.
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

/// The emote panel's Images section, as on the desktop and Android: one glass card to send a
/// picture (and to see or switch where it goes), then the uploads, grouped by host with a tab per
/// host, as a grid of whole pictures or as a list with each one's details and actions. A tap puts
/// an upload's link in the message; an expired one is sent again first. Edit (the grid) deletes
/// uploads, on the host too where it allows that. With `full` it is the uploads manager, drawn in
/// a native sheet with bigger pictures and the storage line. Every picture drawn is the small copy
/// the core keeps on the phone. The panel is read again every minute while shown, since its time
/// labels go stale.
struct ChatImageUploadPane: View {
    let model: ChatComposerModel
    /// The uploads manager: bigger pictures, the storage line, and inserting closes the sheet.
    var full = false
    @State private var item: PhotosPickerItem?
    private var images: ImageUploadModel { model.picker.images }

    /// How often the panel is read again while the pane shows.
    private static let refreshEvery: Duration = .seconds(60)

    /// A grid row's height before it stretches to fill the width.
    private var rowHeight: CGFloat { full ? 112 : 92 }
    /// A list row's picture.
    private var thumbSide: CGFloat { full ? 88 : 76 }

    /// A tile's shape: the picture's own, held between 0.8 and 4.5 (past these it draws whole
    /// inside its tile instead of filling it).
    static func tileAspect(_ shape: CGFloat) -> CGFloat { min(4.5, max(0.8, shape)) }

    var body: some View {
        let images = images
        let full = full
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 8) {
                    sendCard
                    if !images.error.isEmpty {
                        Text(images.error)
                            .font(.sn(12))
                            .foregroundStyle(SN.error)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 4)
                    }
                }
                if images.total > 0 { uploads }
                if full, let storage = images.panel?.storage { storageLine(storage) }
            }
            .padding(full ? 16 : 4)
        }
        .mask {
            if full { Color.black } else { ChatEmotePicker.edgeFade }
        }
        .task {
            while !Task.isCancelled {
                await images.load()
                try? await Task.sleep(for: Self.refreshEvery)
            }
        }
        .onChange(of: item) { _, picked in
            guard let picked else { return }
            item = nil
            let composer = model
            Task {
                if let link = await images.send(picked) { images.insert(link, into: composer, closingManager: full) }
            }
        }
        // One question at a time: the manager asks while it is up, the pane otherwise.
        .alert("Send images to this host?", isPresented: Binding(
            get: { images.asking && images.managerOpen == full }, set: { if !$0 { images.answer(false) } }
        )) {
            Button("Upload") { images.answer(true) }
            Button("Cancel", role: .cancel) { images.answer(false) }
        } message: {
            Text("Images you upload go to \(images.askingHost), and anyone with the link can see them.")
        }
        .sheet(isPresented: Binding(get: { !full && images.managerOpen }, set: { images.managerOpen = $0 }),
               onDismiss: { Task { await images.load() } }) {
            ImageUploadsManager(model: model)
        }
        .onDisappear {
            if images.asking { images.answer(false) }
            // Every visit starts out of Edit (where a tap deletes), with the host list shut.
            images.editing = false
            images.hostsOpen = false
        }
    }

    private func openManager() {
        images.editing = false
        images.hostsOpen = false
        images.managerOpen = true
    }

    // MARK: Send card

    /// Choose an image on top; under a hairline, the host it goes to with its facts and Change,
    /// which opens the host list inside the card.
    private var sendCard: some View {
        let host = images.panel?.host
        return VStack(spacing: 0) {
            chooseRow
            Hairline()
            HStack(spacing: 10) {
                if let host {
                    HostMark(host: host, size: 30)
                } else {
                    Color.clear.frame(width: 30, height: 30)
                }
                VStack(alignment: .leading, spacing: 4) {
                    Text(host?.label ?? "\u{2026}")
                        .font(.sn(13, 600))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    if let host { HostChips(host: host) }
                }
                Spacer(minLength: 8)
                changeButton(label: host?.label)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            if images.hostsOpen, let panel = images.panel {
                Hairline()
                hostList(panel)
                    .transition(.opacity)
            }
        }
        .snGlass(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    private var chooseRow: some View {
        // Read here: the picker's label is not main-actor, so it takes plain values.
        let uploading = images.uploadingTo
        let maxMB = images.panel?.host.maxMB
        return PhotosPicker(selection: $item, matching: .images, photoLibrary: .shared()) {
            ChooseImageRow(uploadingTo: uploading, maxMB: maxMB)
        }
        .buttonStyle(.plain)
        .disabled(images.busy || images.panel == nil)
        .accessibilityLabel(uploading.map { "Uploading to \($0)" } ?? "Choose an image")
    }

    private func changeButton(label: String?) -> some View {
        let open = images.hostsOpen
        let images = images
        return Button {
            withAnimation(.easeOut(duration: 0.18)) { images.hostsOpen.toggle() }
        } label: {
            HStack(spacing: 4) {
                Text("Change")
                    .font(.sn(12, 500))
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .bold))
                    .rotationEffect(.degrees(open ? 180 : 0))
            }
            .foregroundStyle(SN.textPrimary)
            .padding(.horizontal, 12)
            .frame(height: 30)
            .snGlass(Capsule(), interactive: true)
            .frame(minHeight: 44)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(label == nil)
        .accessibilityLabel("Change where uploads go")
        .accessibilityValue(label ?? "")
    }

    private func hostList(_ panel: ImageUploadPanelData) -> some View {
        let current = panel.host.id
        return VStack(spacing: 2) {
            ForEach(panel.hosts) { host in hostRow(host, on: host.id == current) }
        }
        .padding(6)
    }

    /// One host: its mark, its name (checked when current), its note and its facts. The current
    /// one sits on a soft accent glass.
    private func hostRow(_ host: UploadHostData, on: Bool) -> some View {
        let images = images
        let id = host.id
        return Button {
            Task { await images.pickHost(id) }
        } label: {
            HStack(alignment: .top, spacing: 12) {
                HostMark(host: host, size: 34)
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        Text(host.label)
                            .font(.sn(13, 600))
                            .foregroundStyle(SN.textPrimary)
                        if on {
                            Image(systemName: "checkmark")
                                .font(.system(size: 11, weight: .bold))
                                .foregroundStyle(SN.accent)
                        }
                    }
                    Text(host.note)
                        .font(.sn(11))
                        .foregroundStyle(SN.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    HostChips(host: host)
                }
                Spacer(minLength: 0)
            }
            .padding(10)
            .frame(minHeight: 44)
            .background {
                if on {
                    Color.clear
                        .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous), tint: SN.accent.opacity(0.25),
                                 interactive: true)
                }
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }

    // MARK: Uploads

    private struct HostTab: Identifiable {
        var id: String
        var short: String
        var count: Int
    }

    private var uploads: some View {
        let editing = images.isEditing
        let list = images.isList
        let groups = images.groups
        let headed = images.activeFilter == "all" && groups.count > 1
        return VStack(alignment: .leading, spacing: 10) {
            uploadsHeader(editing: editing, list: list)
            if groups.count > 1 { hostTabs(groups) }
            if editing {
                Text(images.editHint)
                    .font(.sn(11))
                    .foregroundStyle(SN.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 4)
            }
            ForEach(images.shownGroups, id: \.host.id) { group in
                groupBlock(group, heading: headed, editing: editing, list: list)
            }
        }
    }

    /// UPLOADS and the count; the manager, Grid/List and (the grid) Edit on the right.
    private func uploadsHeader(editing: Bool, list: Bool) -> some View {
        let images = images
        return HStack(spacing: 8) {
            Text("UPLOADS")
                .font(.system(size: 10, weight: .bold))
                .tracking(1)
                .foregroundStyle(SN.textSecondary)
            Text("\(images.total)")
                .font(.sn(10).monospacedDigit())
                .foregroundStyle(SN.textMuted)
            Spacer(minLength: 8)
            if !full {
                Button {
                    openManager()
                } label: {
                    Image(systemName: "arrow.up.left.and.arrow.down.right")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(SN.textSecondary)
                        .frame(width: 30, height: 30)
                        .snGlass(Circle(), interactive: true)
                        .frame(minHeight: 44)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Open the uploads manager")
            }
            ViewToggle(list: list) { next in
                Task { await images.setView(list: next) }
            }
            if !list {
                Button {
                    images.editing = !editing
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: editing ? "checkmark" : "pencil")
                            .font(.system(size: 10, weight: .bold))
                        Text(editing ? "Done" : "Edit")
                            .font(.sn(12, 600))
                    }
                    .foregroundStyle(editing ? UploadInk.onAccent : SN.textSecondary)
                    .padding(.horizontal, 11)
                    .frame(height: 30)
                    .snGlass(Capsule(), tint: editing ? SN.accent : nil, interactive: true)
                    .frame(minHeight: 44)
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.leading, 4)
    }

    /// All, then one tab per host with uploads, on a glass track with the chosen tab's pill.
    private func hostTabs(_ groups: [UploadGroupData]) -> some View {
        let active = images.activeFilter
        let images = images
        let tabs = [HostTab(id: "all", short: "All", count: images.total)]
            + groups.map { HostTab(id: $0.host.id, short: $0.host.short, count: $0.items.count) }
        return HStack(spacing: 2) {
            ForEach(tabs) { tab in
                let on = active == tab.id
                Button {
                    images.filter = tab.id
                } label: {
                    VStack(spacing: 2) {
                        Text(tab.short)
                            .font(.sn(12, on ? 700 : 500))
                            .lineLimit(1)
                            .frame(height: 16)
                        Text("\(tab.count)")
                            .font(.sn(10).monospacedDigit())
                            .opacity(0.75)
                    }
                    .foregroundStyle(on ? SN.textPrimary : SN.textSecondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 5)
                    .background {
                        if on { RoundedRectangle(cornerRadius: 9, style: .continuous).fill(SN.glazeSelected) }
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("\(tab.short), \(tab.count)")
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
        .padding(3)
        .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    @ViewBuilder
    private func groupBlock(_ group: UploadGroupData, heading: Bool, editing: Bool, list: Bool) -> some View {
        let items = images.visible(group)
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                if heading {
                    GroupHeading(host: group.host)
                        .padding(.horizontal, 4)
                        .padding(.top, 4)
                }
                if list {
                    VStack(spacing: 0) {
                        ForEach(Array(items.enumerated()), id: \.element.url) { index, item in
                            if index > 0 { Hairline() }
                            row(item, host: group.host)
                        }
                    }
                    .snGlass(RoundedRectangle(cornerRadius: 16, style: .continuous))
                } else {
                    JustifiedRows(rowHeight: rowHeight, spacing: 8) {
                        ForEach(items, id: \.url) { item in
                            tile(item, deletes: group.host.deletesFromHost, editing: editing)
                                .layoutValue(key: TileShape.self, value: Self.tileAspect(item.shape))
                        }
                    }
                }
            }
        }
    }

    /// One upload in the grid: the small picture at its own shape, a round glyph on its corner
    /// (delete in Edit, send again where the link runs out, a spinner while either runs), and
    /// under it its time left (amber under a day, red once dead) or when it went up. Never words
    /// over the picture.
    private func tile(_ item: UploadedImageData, deletes: Bool, editing: Bool) -> some View {
        let shape = item.shape
        let aspect = Self.tileAspect(shape)
        let fits = aspect == shape
        let working = images.working.contains(item.url)
        let sources = item.sources
        let url = item.url
        let images = images
        let label = editing
            ? (deletes ? "Delete this picture" : "Remove this picture from the list")
            : (item.expired && item.canReupload ? "Send this picture again and insert its new link" : "Insert this picture's link")
        return VStack(alignment: .leading, spacing: 4) {
            Button {
                pick(item)
            } label: {
                Color.clear
                    .aspectRatio(aspect, contentMode: .fit)
                    .overlay {
                        CosmeticImage(sources.first, fallbacks: Array(sources.dropFirst()),
                                      animate: model.session.style.animatesEmotes, fill: fits, pointHeight: 180,
                                      onFailure: { images.hide(url) })
                            .opacity(item.expired ? 0.5 : 1)
                    }
                    .background(fits ? Color.white.opacity(0.04) : Color.black.opacity(0.3))
                    .clipShape(.rect(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(SN.borderSubtle))
                    .overlay(alignment: .topTrailing) {
                        if working {
                            CornerGlyph { ProgressView().tint(SN.textPrimary).scaleEffect(0.6) }
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
            .accessibilityHint(item.details)
            .overlay(alignment: .topTrailing) {
                if item.offersSendAgain, !editing, !working {
                    Button {
                        Task { _ = await images.reupload(item) }
                    } label: {
                        // The padding is the glyph's inset and widens what a finger can hit.
                        CornerGlyph(interactive: true) { Image(systemName: "arrow.clockwise") }
                            .padding(6)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Send it again for a fresh link")
                }
            }
            HStack(spacing: 3) {
                if item.timeLeft != nil {
                    Image(systemName: "clock")
                        .font(.system(size: 8.5, weight: .semibold))
                }
                Text(item.timeLeft ?? item.uploaded)
                    .font(.sn(10))
                    .lineLimit(1)
            }
            .foregroundStyle(Self.timeInk(item))
            .padding(.horizontal, 2)
        }
    }

    /// One upload in the list: the small picture whole on a dark backing (a tap inserts), the
    /// host and when it went up, its expiry, its size, then insert, copy, send again (where the
    /// link runs out) and, on the right, delete.
    private func row(_ item: UploadedImageData, host: UploadHostData) -> some View {
        let working = images.working.contains(item.url)
        let copied = images.copied == item.url
        let sources = item.sources
        let url = item.url
        let images = images
        let side = thumbSide
        return HStack(alignment: .top, spacing: 12) {
            Button {
                pick(item)
            } label: {
                CosmeticImage(sources.first, fallbacks: Array(sources.dropFirst()),
                              animate: model.session.style.animatesEmotes, pointHeight: side,
                              onFailure: { images.hide(url) })
                    .opacity(item.expired ? 0.5 : 1)
                    .frame(width: side, height: side)
                    .background(Color.black.opacity(0.3))
                    .clipShape(.rect(cornerRadius: 10, style: .continuous))
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(working)
            .accessibilityLabel(item.expired && item.canReupload ? "Send this picture again and insert its new link"
                                                                 : "Insert this picture's link")
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(host.label)
                        .font(.sn(13, 600))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    Text(item.uploaded)
                        .font(.sn(11))
                        .foregroundStyle(SN.textMuted)
                        .lineLimit(1)
                        .layoutPriority(1)
                }
                Self.expiryChip(item, host: host)
                if !item.details.isEmpty {
                    Text(item.details)
                        .font(.sn(11))
                        .foregroundStyle(SN.textMuted)
                        .lineLimit(1)
                        .minimumScaleFactor(0.85)
                }
                HStack(spacing: 0) {
                    RoundAction(symbol: "plus.message", label: item.expired ? "Send again and insert" : "Insert link",
                                disabled: working) { pick(item) }
                    RoundAction(symbol: copied ? "checkmark" : "doc.on.doc", label: copied ? "Copied" : "Copy link",
                                ink: copied ? SN.accent : nil, disabled: working || item.expired) { images.copy(item) }
                    if item.offersSendAgain {
                        RoundAction(symbol: "arrow.clockwise", label: "Send again for a fresh link", busy: working,
                                    disabled: working) { Task { _ = await images.reupload(item) } }
                    }
                    Spacer(minLength: 0)
                    RoundAction(symbol: host.deletesFromHost ? "trash" : "xmark",
                                label: host.deletesFromHost ? "Delete from \(host.label)" : "Remove from this list",
                                disabled: working) { Task { await images.remove(item) } }
                }
                // The buttons' hit area reaches past their glass; this lines the glass up with the text.
                .padding(.horizontal, -6)
            }
        }
        .padding(12)
    }

    /// An upload's expiry as a chip: a clock for a link that runs out (amber under a day, red once
    /// dead); for the rest, infinity, green where the host keeps it, amber where it does not say.
    @MainActor
    private static func expiryChip(_ item: UploadedImageData, host: UploadHostData) -> FactChip {
        if item.expiresMs != nil {
            return FactChip(symbol: "clock", text: item.expiry, tone: item.expired ? .bad : item.urgent ? .warn : .plain)
        }
        return FactChip(symbol: "infinity", text: item.expiry, tone: host.expires ? .warn : .good)
    }

    /// The line under a tile: amber under a day, red once dead, muted otherwise.
    @MainActor
    private static func timeInk(_ item: UploadedImageData) -> Color {
        guard item.timeLeft != nil else { return SN.textMuted }
        return item.expired ? SN.error : item.urgent ? SN.warning : SN.textMuted
    }

    /// The manager's footer: what the kept files take on the phone.
    private func storageLine(_ storage: String) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "internaldrive")
                .font(.system(size: 11, weight: .medium))
            Text(storage)
                .font(.sn(11))
        }
        .foregroundStyle(SN.textMuted)
        .padding(.horizontal, 4)
        .padding(.top, 4)
        .accessibilityElement(children: .combine)
    }

    /// A tap: deletes in Edit; an expired link with a kept copy is sent again and its new link
    /// inserted; anything else inserts its link. From the manager, inserting closes it.
    private func pick(_ item: UploadedImageData) {
        let images = images
        let composer = model
        let closes = full
        guard !images.working.contains(item.url) else { return }
        if images.isEditing {
            Task { await images.remove(item) }
        } else if item.expired, item.canReupload {
            Task {
                if let link = await images.reupload(item) { images.insert(link, into: composer, closingManager: closes) }
            }
        } else {
            images.insert(item.url, into: composer, closingManager: closes)
        }
    }
}

/// The uploads manager: the Images section full screen in the phone's own sheet. Closing it reads
/// the pane's panel again.
private struct ImageUploadsManager: View {
    let model: ChatComposerModel

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: "Image uploads")
            ChatImageUploadPane(model: model, full: true)
        }
        .snSheetLight()
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        // Nothing is see-through at 0 percent glassiness.
        .presentationBackground {
            if !Look.shared.frosted { SN.backgroundTertiary }
        }
    }
}

/// Text and glyphs on the accent glass: white on a dark accent, near-black on a pale one.
@MainActor
private enum UploadInk {
    static var onAccent: Color { Look.shared.palette.accent.luminance > 0.5 ? Color.black.opacity(0.88) : .white }
}

/// The 1 point line between a card's parts.
private struct Hairline: View {
    var body: some View {
        Rectangle().fill(SN.borderSubtle).frame(height: 1)
    }
}

/// A host's mark: a tinted rounded square with its initials, so showing the hosts never contacts
/// one. Unknown hosts take the first two letters of their short name in gray.
private struct HostMark: View {
    let host: UploadHostData
    let size: CGFloat

    private struct Mark {
        var text: String
        var color: Color
    }

    private static let known: [String: (text: String, rgb: UInt32)] = [
        "nuuls": ("N", 0x5B8DEF),
        "kappa": ("K", 0x9B6BFF),
        "imgdrip": ("ID", 0x22B8CF),
        "catbox": ("CB", 0xF08C4B),
        "litterbox": ("LB", 0xE3A92F),
        "uguu": ("U", 0xF06595),
    ]

    private static func mark(_ host: UploadHostData) -> Mark {
        if let known = known[host.id] { return Mark(text: known.text, color: color(known.rgb)) }
        return Mark(text: String(host.short.prefix(2)).uppercased(), color: color(0x868E96))
    }

    private static func color(_ rgb: UInt32) -> Color {
        Color(red: Double((rgb >> 16) & 0xFF) / 255, green: Double((rgb >> 8) & 0xFF) / 255, blue: Double(rgb & 0xFF) / 255)
    }

    var body: some View {
        let mark = Self.mark(host)
        let shape = RoundedRectangle(cornerRadius: size * 0.27, style: .continuous)
        Text(mark.text)
            .font(.system(size: (size * (mark.text.count > 1 ? 0.36 : 0.46)).rounded(), weight: .heavy))
            .tracking(-0.3)
            .foregroundStyle(mark.color)
            .lineLimit(1)
            .frame(width: size, height: size)
            .background(shape.fill(mark.color.opacity(0.16)))
            .overlay(shape.strokeBorder(mark.color.opacity(0.3), lineWidth: 1))
            .accessibilityHidden(true)
    }
}

/// A small fact as a glass chip: a symbol and words in the fact's color, on the chip's glass
/// tinted faintly with that color.
private struct FactChip: View {
    enum Tone { case plain, good, warn, bad }

    let symbol: String
    let text: String
    var tone: Tone = .plain

    @MainActor
    private static func color(_ tone: Tone) -> Color {
        switch tone {
        case .plain: SN.textSecondary
        case .good: SN.success
        case .warn: SN.warning
        case .bad: SN.error
        }
    }

    var body: some View {
        let color = Self.color(tone)
        HStack(spacing: 3) {
            Image(systemName: symbol)
                .font(.system(size: 8.5, weight: .bold))
            Text(text)
                .font(.sn(10, 500))
                .lineLimit(1)
        }
        .foregroundStyle(color)
        .padding(.horizontal, 7)
        .padding(.vertical, 2.5)
        .snGlass(Capsule(), tint: tone == .plain ? nil : color.opacity(0.12))
    }
}

/// A host's facts: its size limit, how long its links last, and whether deleting reaches it.
private struct HostChips: View {
    let host: UploadHostData

    var body: some View {
        FlowLayout(spacing: 4) {
            FactChip(symbol: "internaldrive", text: "Up to \(host.maxMB) MB")
            HostLifetimeChip(host: host)
            if host.deletesFromHost { FactChip(symbol: "trash", text: "Deletable") }
        }
    }
}

/// Kept (green) or how long links last (amber).
private struct HostLifetimeChip: View {
    let host: UploadHostData

    var body: some View {
        if let life = host.lifetimeChip {
            FactChip(symbol: "clock", text: life, tone: .warn)
        } else {
            FactChip(symbol: "infinity", text: "Kept", tone: .good)
        }
    }
}

/// A host's heading under All: its mark, its name, and how long its links last on the right.
private struct GroupHeading: View {
    let host: UploadHostData

    var body: some View {
        HStack(spacing: 8) {
            HostMark(host: host, size: 18)
            Text(host.label.uppercased())
                .font(.system(size: 10, weight: .bold))
                .tracking(1)
                .foregroundStyle(SN.textSecondary)
                .lineLimit(1)
            Spacer(minLength: 8)
            HostLifetimeChip(host: host)
        }
    }
}

/// Grid or List: two glyphs on a glass capsule, the chosen one on the glaze pill.
private struct ViewToggle: View {
    let list: Bool
    let pick: (Bool) -> Void

    var body: some View {
        HStack(spacing: 2) {
            option(list: false, symbol: "square.grid.2x2", label: "Pictures")
            option(list: true, symbol: "list.bullet", label: "Details")
        }
        .padding(3)
        .snGlass(Capsule())
    }

    private func option(list value: Bool, symbol: String, label: String) -> some View {
        let on = list == value
        return Button {
            if !on { pick(value) }
        } label: {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(on ? SN.textPrimary : SN.textSecondary)
                .frame(width: 32, height: 26)
                .background { if on { GlazeSelected() } }
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// A list row's round icon button: a glyph on round glass, a spinner while it runs. The padding
/// takes what a finger can hit to 44 points.
private struct RoundAction: View {
    let symbol: String
    let label: String
    var ink: Color?
    var busy = false
    var disabled = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            ZStack {
                if busy {
                    ProgressView().tint(SN.textSecondary).scaleEffect(0.65)
                } else {
                    Image(systemName: symbol)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(ink ?? SN.textSecondary)
                }
            }
            .frame(width: 32, height: 32)
            .snGlass(Circle(), interactive: true)
            .padding(6)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled && !busy ? 0.45 : 1)
        .accessibilityLabel(label)
    }
}

/// The round glyph on a tile's corner (delete, send again, a spinner), on round glass.
private struct CornerGlyph<Content: View>: View {
    var interactive = false
    @ViewBuilder let content: () -> Content

    var body: some View {
        content()
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(SN.textPrimary)
            .frame(width: 26, height: 26)
            .snGlass(Circle(), interactive: interactive)
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

/// The send card's top: an accent glass square with the upload glyph (a spinner while a picture
/// is on its way), Choose an image and the formats with the host's limit. While uploading, the
/// formats line stays, only hidden, so nothing moves. Its own view, so the photo picker's label
/// can hold it.
private struct ChooseImageRow: View {
    let uploadingTo: String?
    let maxMB: Int?

    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                if uploadingTo != nil {
                    ProgressView().tint(UploadInk.onAccent)
                } else {
                    Image(systemName: "photo.badge.plus")
                        .font(.system(size: 19, weight: .semibold))
                        .foregroundStyle(UploadInk.onAccent)
                }
            }
            .frame(width: 44, height: 44)
            .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous), tint: SN.accent, interactive: true)
            VStack(alignment: .leading, spacing: 2) {
                Text(uploadingTo.map { "Uploading to \($0)\u{2026}" } ?? "Choose an image")
                    .font(.sn(14, 600))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                Text("JPG, PNG, GIF or WebP" + (maxMB.map { ", up to \($0) MB" } ?? ""))
                    .font(.sn(11))
                    .foregroundStyle(SN.textSecondary)
                    .lineLimit(1)
                    .opacity(uploadingTo == nil ? 1 : 0)
                    .accessibilityHidden(uploadingTo != nil)
            }
            Spacer(minLength: 0)
        }
        .padding(14)
        .contentShape(.rect)
    }
}
