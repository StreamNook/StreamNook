import SwiftUI

@main
struct StreamNookApp: App {
    @State private var model = AppModel()
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            RootView(model: model)
                .task {
                    await model.boot()
                    #if DEBUG
                    await Drive.run(model: model)
                    #endif
                }
        }
        .onChange(of: phase) { _, phase in
            switch phase {
            case .active: model.lifecycle("foreground")
            case .background: model.lifecycle("background")
            default: break
            }
        }
    }
}
