import SNKit
import SwiftUI

/// The spike's first page: the core's status, then the Browse list.
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
        }
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
