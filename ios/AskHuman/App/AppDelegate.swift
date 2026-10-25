import UIKit
import UserNotifications

/// UIKit entry points SwiftUI has no equivalent for: the APNs token callbacks and the notification
/// center delegate (taps, and foreground presentation).
final class AppDelegate: NSObject, UIApplicationDelegate, @preconcurrency UNUserNotificationCenterDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { @MainActor in AppEnvironment.current?.push.didRegister(token: deviceToken) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        Task { @MainActor in AppEnvironment.current?.push.didFail(error: error) }
    }

    // Both delegate methods below are main-actor methods on purpose: Swift calls the system's completion
    // handler from the executor the method ran on, and UIKit insists on the main thread for it. As
    // `nonisolated` methods they finished on the cooperative pool, and tapping a notification crashed
    // the app (an assertion in `UIApplication`, seen on the iOS 27 simulator).

    /// A notification arrives while the app is open: the in-app banner already covers new requests,
    /// so only show the system one for something the inbox does not know.
    @MainActor
    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let info = notification.request.content.userInfo
        guard let store = AppEnvironment.current?.store, let rid = info["rid"] as? String, let mac = info["mac"] as? String else {
            return [.banner, .list, .sound]
        }
        let known = store.coversNotification(macId: mac, rid: rid, kind: info["kind"] as? String)
        return known ? [] : [.banner, .list, .sound]
    }

    /// The user tapped a notification: open the request or the Agent it is about.
    @MainActor
    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        guard let rid = info["rid"] as? String, let mac = info["mac"] as? String else { return }
        AppEnvironment.current?.store.openNotification(macId: mac, rid: rid, kind: info["kind"] as? String)
    }
}
