import SNKit
import SwiftUI
import UIKit

/// Where `/clip` goes: the app model opens Create clip for the room it was typed in.
@MainActor
enum ClipRouter {
    static var create: ((_ login: String, _ channelID: String, _ title: String) -> Void)?
}

/// What to clip: the channel, and for a VOD its id and the playhead (a live clip ends at the edge).
struct ClipTarget: Identifiable {
    let id = UUID()
    let channelID: String
    let login: String
    let name: String
    let title: String
    var vodID: String?
    var vodOffset: Double?
}

/// Create clip: the last 30 seconds, under a title (the stream's title to start with). The core
/// makes it and files it in "Your clips"; the sheet then offers Play (once Twitch has rendered it),
/// Copy link and Share.
struct CreateClipSheet: View {
    let target: ClipTarget
    let model: AppModel
    @State private var title: String
    @State private var phase = Phase.editing
    @State private var rendered = false
    @State private var copied = false
    @FocusState private var titleFocused: Bool
    @Environment(\.dismiss) private var dismiss

    enum Phase: Equatable {
        case editing, working
        case made(MadeClip)
        case failed(String)
    }

    init(target: ClipTarget, model: AppModel) {
        self.target = target
        self.model = model
        _title = State(initialValue: target.title)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetTitle(text: made == nil ? "Create clip" : "Clip made")
            Group {
                if let made { result(made) } else { form }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 16)
        }
        .fittedSheet()
        .animation(SNMotion.run(.easeOut(duration: 0.18)), value: phase)
    }

    private var made: MadeClip? {
        if case .made(let clip) = phase { return clip }
        return nil
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 0) {
            caption("Title")
            TextField("", text: $title, prompt: Text("Clip").foregroundStyle(SN.textMuted), axis: .vertical)
                .keyboardHideButton()
                .lineLimit(1...3)
                .font(.snBody)
                .foregroundStyle(SN.textPrimary)
                .focused($titleFocused)
                .submitLabel(.done)
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(RoundedRectangle(cornerRadius: SNRadius.panel, style: .continuous).fill(SN.surface))
            caption("Length")
            HStack {
                Text("Last 30 seconds").font(.snBody).foregroundStyle(SN.textPrimary)
                Spacer(minLength: 8)
                Text(target.vodID == nil ? "ends now" : "ends at the playhead")
                    .font(.sn(13)).foregroundStyle(SN.textSecondary)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .background(RoundedRectangle(cornerRadius: SNRadius.panel, style: .continuous).fill(SN.surface))
            if case .failed(let words) = phase {
                Text(words)
                    .font(.sn(13))
                    .foregroundStyle(SN.error)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 12)
            }
            Button(action: create) {
                if phase == .working {
                    ProgressView().tint(.white)
                } else {
                    Text("Create")
                }
            }
            .buttonStyle(.snPrimary)
            .disabled(phase == .working)
            .padding(.top, 18)
        }
    }

    private func result(_ clip: MadeClip) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 3) {
                Text(clip.title)
                    .font(.sn(15, 600))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(2)
                Text("\(clip.channelName) · 30s").font(.sn(13)).foregroundStyle(SN.textSecondary)
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: SNRadius.panel, style: .continuous).fill(SN.surface))
            HStack(spacing: 10) {
                Button {
                    guard let url = URL(string: clip.url) else { return }
                    dismiss()
                    model.clip = ClipLink(url: url)
                } label: {
                    Text(rendered ? "Play" : "Rendering")
                }
                .buttonStyle(.snPrimary)
                .disabled(!rendered)
                Button {
                    UIPasteboard.general.string = clip.url
                    copied = true
                } label: {
                    Text(copied ? "Copied" : "Copy link")
                }
                .buttonStyle(.snSecondary)
                if let url = URL(string: clip.url) {
                    ShareLink(item: url, subject: Text(clip.title)) { Text("Share") }
                        .buttonStyle(.snSecondary)
                }
            }
        }
        .task(id: clip.slug) { await waitForRender(clip.slug) }
    }

    private func caption(_ text: String) -> some View {
        Text(text).snSectionStyle().padding(.top, 14).padding(.bottom, 6)
    }

    private func create() {
        titleFocused = false
        phase = .working
        let target = target
        let title = title
        Task {
            do {
                let clip = try await model.core.invoke("clip_create", args: [
                    "channelId": target.channelID, "login": target.login, "name": target.name, "title": title,
                    "vodId": target.vodID, "vodOffset": target.vodOffset,
                ], as: MadeClip.self)
                phase = .made(clip)
            } catch {
                let words = (error as? CoreFailure)?.description ?? ""
                phase = .failed(words.isEmpty ? "Could not make a clip" : words)
            }
        }
    }

    /// Twitch renders a new clip for a few seconds; Play waits for it, so it never opens a black frame.
    private func waitForRender(_ slug: String) async {
        struct Status: Decodable { let ready: Bool }
        for _ in 0..<30 where !Task.isCancelled {
            if let status = try? await model.core.invoke("get_clip_render_status", args: ["slug": slug], as: Status.self),
               status.ready {
                rendered = true
                return
            }
            try? await Task.sleep(for: .milliseconds(1500))
        }
        rendered = true
    }
}
