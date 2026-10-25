import SwiftUI
import AHCore

@main
struct AskHumanApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @Environment(\.scenePhase) private var scenePhase
    @State private var environment = AppEnvironment()

    var body: some Scene {
        WindowGroup {
            RootView(leftoverPairings: environment.leftoverPairings)
                .environment(environment.store)
                .environment(environment.push)
                .environment(\.launchOptions, environment.launch)
                .task { await environment.start() }
                .task(id: scenePhase) {
                    await environment.store.setForeground(scenePhase == .active)
                    if scenePhase == .active { await environment.push.refreshAuthorization() }
                }
        }
    }
}

private struct LaunchOptionsKey: EnvironmentKey {
    static let defaultValue = LaunchOptions()
}

extension EnvironmentValues {
    var launchOptions: LaunchOptions {
        get { self[LaunchOptionsKey.self] }
        set { self[LaunchOptionsKey.self] = newValue }
    }
}
