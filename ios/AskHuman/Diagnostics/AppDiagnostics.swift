import UIKit
import AHCore

/// Connects the diagnostics to the app (docs/specs/ios-developer-mode.md §12): the sections of the report that look at
/// UIKit, the pasteboard the copy goes to, and what is written to the log without anyone asking (what the system tells the
/// app about itself coming and going, and what `BarWatch` sees change).
///
/// To have something else in the report, register a section of its own: `Diagnostics.shared.register`. To have
/// something in the log, write a line: `Diagnostics.note`. Nothing here needs to change for either.
@MainActor
enum AppDiagnostics {
    /// A notification on its way from the observer, which is called on the main queue, to the main actor.
    private struct Delivered: @unchecked Sendable { let note: Notification }

    static let watch = BarWatch()
    private static var observers: [NSObjectProtocol] = []

    /// Called whenever an environment is made, which may be more than once: the sections look at the store they were
    /// last given, and the listening is set up the first time only.
    static func install(store: AppStore) {
        let diagnostics = Diagnostics.shared
        diagnostics.register("Device and app") { UIKitReport.device() }
        diagnostics.register("App state") { [weak store] in store?.diagnosticSummary() ?? "(the store is gone)" }
        diagnostics.register("Scenes and windows") { UIKitReport.scenes() }
        diagnostics.register("View controllers") { UIKitReport.controllers() }
        diagnostics.register("Floating bars") {
            let bars = UIKitReport.floatingBars(detailed: true)
            return bars.isEmpty ? "none" : bars.joined(separator: "\n")
        }
        diagnostics.register("View tree (window coordinates)") { UIKitReport.tree() }
        diagnostics.register("Cost of the diagnostics") {
            "\(watch.cost())\nlog \(Diagnostics.shared.entries().count) of \(Diagnostics.capacity) lines kept"
        }
        store.developer.diagnosticsSink = { UIPasteboard.general.string = $0 }

        guard observers.isEmpty else { return }
        Diagnostics.note("app", "diagnostics started")
        listen()
        // The application object is certain to exist a turn later.
        Task { @MainActor in watch.start() }
    }

    private static func listen() {
        func add(
            _ name: Notification.Name, _ category: String, resample: Bool = false,
            _ text: @escaping @MainActor @Sendable (Notification) -> String
        ) {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { note in
                // Delivered on the main queue, which the compiler cannot know: the box carries the notification across.
                let delivered = Delivered(note: note)
                MainActor.assumeIsolated {
                    Diagnostics.note(category, text(delivered.note))
                    if resample { watch.sample(soonAfter: [0.3, 1.5]) }
                }
            })
        }

        // The application: coming back, going away, and the phone locking (which takes the protected data with it).
        let application: [(Notification.Name, String)] = [
            (UIApplication.didBecomeActiveNotification, "didBecomeActive"),
            (UIApplication.willResignActiveNotification, "willResignActive"),
            (UIApplication.didEnterBackgroundNotification, "didEnterBackground"),
            (UIApplication.willEnterForegroundNotification, "willEnterForeground"),
            (UIApplication.protectedDataWillBecomeUnavailableNotification, "protected data will become unavailable (locking)"),
            (UIApplication.protectedDataDidBecomeAvailableNotification, "protected data available (unlocked)"),
        ]
        for (name, label) in application {
            add(name, "app", resample: true) { _ in "\(label); application \(UIKitReport.applicationState(UIApplication.shared.applicationState))" }
        }
        add(UIApplication.didReceiveMemoryWarningNotification, "app") { _ in "memory warning; memory left \(UIKitReport.memoryLeft())" }
        add(UIApplication.significantTimeChangeNotification, "app") { _ in "significant time change" }
        add(ProcessInfo.thermalStateDidChangeNotification, "app") { _ in "thermal state \(ProcessInfo.processInfo.thermalState.rawValue)" }
        add(.NSProcessInfoPowerStateDidChange, "app") { _ in "low power mode \(ProcessInfo.processInfo.isLowPowerModeEnabled)" }
        add(UIContentSizeCategory.didChangeNotification, "app") { note in
            let new = note.userInfo?[UIContentSizeCategory.newValueUserInfoKey] as? UIContentSizeCategory
            return "content size category \(new?.rawValue ?? "?")"
        }

        // The scenes.
        let scenes: [(Notification.Name, String)] = [
            (UIScene.willConnectNotification, "willConnect"),
            (UIScene.didDisconnectNotification, "didDisconnect"),
            (UIScene.willEnterForegroundNotification, "willEnterForeground"),
            (UIScene.didActivateNotification, "didActivate"),
            (UIScene.willDeactivateNotification, "willDeactivate"),
            (UIScene.didEnterBackgroundNotification, "didEnterBackground"),
        ]
        for (name, label) in scenes {
            add(name, "scene", resample: true) { note in
                guard let scene = note.object as? UIScene else { return label }
                return "\(label); \(UIKitReport.short(scene)) is \(UIKitReport.sceneState(scene.activationState))"
            }
        }

        // The windows: the ones the system adds (the keyboard, alerts) as well as the app's own.
        let windows: [(Notification.Name, String, Bool)] = [
            (UIWindow.didBecomeVisibleNotification, "visible", true),
            (UIWindow.didBecomeHiddenNotification, "hidden", true),
            (UIWindow.didBecomeKeyNotification, "key", false),
            (UIWindow.didResignKeyNotification, "not key any more", false),
        ]
        for (name, label, resample) in windows {
            add(name, "window", resample: resample) { note in
                guard let window = note.object as? UIWindow else { return label }
                return "\(UIKitReport.short(window, limit: 48)) level \(UIKitReport.num(window.windowLevel.rawValue)) is \(label)"
            }
        }

        // The screen and the keyboard.
        add(UIScreen.modeDidChangeNotification, "screen", resample: true) { note in
            guard let screen = note.object as? UIScreen else { return "mode changed" }
            return "mode changed; bounds \(UIKitReport.rect(screen.bounds)) @\(UIKitReport.num(screen.scale))x"
        }
        for (name, label) in [(UIResponder.keyboardWillShowNotification, "will show"), (UIResponder.keyboardWillHideNotification, "will hide")] {
            add(name, "keyboard") { note in
                let frame = (note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue
                let local = (note.userInfo?[UIResponder.keyboardIsLocalUserInfoKey] as? NSNumber)?.boolValue
                return "\(label); end frame \(frame.map { UIKitReport.rect($0) } ?? "?"), this app's \(local.map(String.init) ?? "?")"
            }
        }
    }
}
