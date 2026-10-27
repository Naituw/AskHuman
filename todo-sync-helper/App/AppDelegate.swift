import AppKit
import CloudKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var runtime: HelperRuntime!

    func applicationDidFinishLaunching(_ notification: Notification) {
        let paths = HelperPaths.resolve()
        let logs = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/AskHumanTodoSync/helper.log")
        HelperLog.shared.configure(url: logs)

        runtime = HelperRuntime(paths: paths)
        guard runtime.start() else {
            // Another helper owns this directory, or the socket cannot be served: nothing useful to do.
            HelperLog.shared.flush()
            exit(0)
        }
        if Entitlements.hasPushEntitlement {
            NSApplication.shared.registerForRemoteNotifications()
        } else {
            runtime.apnsChanged("unsupported")
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        runtime?.stop()
    }

    // MARK: - Remote notifications (CloudKit zone subscription: only a hint to go and look)

    func application(_ application: NSApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        runtime.apnsChanged("registered")
    }

    func application(_ application: NSApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        runtime.apnsChanged("failed")
    }

    func application(_ application: NSApplication, didReceiveRemoteNotification userInfo: [String: Any]) {
        // Never log the payload: only whether it is ours.
        let note = CKNotification(fromRemoteNotificationDictionary: userInfo)
        runtime.pushReceived(subscriptionID: note?.subscriptionID)
    }
}
