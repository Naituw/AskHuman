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
                    Diagnostics.note("app", "scenePhase \(scenePhase)")
                    switch scenePhase {
                    case .active:
                        await environment.store.setForeground(true)
                        await environment.push.refreshAuthorization()
                    case .inactive:
                        // Control Center, the notification center, the app switcher and system alerts
                        // leave the app on screen: the connection stays for a grace period.
                        await environment.store.setInactive()
                    default:
                        // Only a file actively requested by a drag receiver can briefly keep the transport alive.
                        await environment.store.setForeground(false)
                    }
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
