import SwiftUI

@main
struct StreamNookApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var model = AppModel()
    @Environment(\.scenePhase) private var phase

    init() {
        Typeface.register()
        Appearance.apply()
    }

    var body: some Scene {
        WindowGroup {
            AppRoot(model: model)
                .task {
                    await model.boot()
                    #if DEBUG
                    await Drive.run(model: model)
                    #endif
                }
                .onReceive(NotificationCenter.default.publisher(for: UIApplication.didReceiveMemoryWarningNotification)) { _ in
                    model.lifecycle("memory_warning")
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
