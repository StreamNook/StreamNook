import SNKit
import SwiftUI

/// The spike's first page: the core's status, the Twitch account, then the Browse list.
struct RootView: View {
    @Bindable var model: AppModel

    var body: some View {
        NavigationStack(path: $model.path) {
            List {
                Section("Core") {
                    LabeledContent("StreamNook", value: model.version)
                    LabeledContent("Core", value: model.core.kind)
                    LabeledContent("Status", value: status)
                    LabeledContent("Events", value: events)
                }
                Section("Twitch") {
                    TwitchAccountRow(model: model)
                }
                Section("Live now") {
                    if model.loading && model.streams.isEmpty {
                        ProgressView().frame(maxWidth: .infinity)
                    }
                    if let error = model.browseError {
                        Text(error).foregroundStyle(.red)
                    }
                    ForEach(model.streams) { stream in
                        NavigationLink(value: stream) { StreamRow(stream: stream) }
                    }
                }
            }
            .navigationTitle("Browse")
            .refreshable { await model.browse() }
            .navigationDestination(for: TwitchStream.self) { stream in
                WatchView(model: model, stream: stream)
            }
            .sheet(item: signInPage, onDismiss: { model.signIn.sheetClosed() }) { page in
                SignInSheet(signIn: model.signIn, page: page)
            }
        }
    }

    private var signInPage: Binding<SignIn.Page?> {
        Binding(get: { model.signIn.page }, set: { model.signIn.page = $0 })
    }

    private var status: String {
        if let error = model.startError { return error }
        if let ms = model.startMS { return "Started in \(ms) ms" }
        return "Starting"
    }

    private var events: String {
        guard let last = model.lastEvent else { return "None yet" }
        return "\(model.eventCount), last \(last)"
    }
}

private struct StreamRow: View {
    let stream: TwitchStream

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(stream.userName).font(.headline)
                Spacer()
                Label(stream.viewerCount.formatted(.number.notation(.compactName)), systemImage: "person.fill")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(stream.title).font(.subheadline).lineLimit(1)
            Text(stream.gameName).font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// Who is signed in to Twitch, or the button that signs in.
private struct TwitchAccountRow: View {
    let model: AppModel

    private var signIn: SignIn { model.signIn }

    var body: some View {
        if let user = signIn.user {
            LabeledContent("Signed in", value: user.displayName)
        } else if signIn.busy && signIn.page == nil {
            LabeledContent("Signing in") { ProgressView() }
        } else {
            Button("Sign in to Twitch", systemImage: "person.crop.circle.badge.plus") {
                Task { await signIn.begin(core: model.core) }
            }
            .disabled(!model.started || signIn.busy)
        }
        if let error = signIn.error {
            Text(error).font(.caption).foregroundStyle(.red)
        }
    }
}
