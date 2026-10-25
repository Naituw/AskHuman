import UIKit
import UserNotifications

/// UIKit entry points SwiftUI has no equivalent for: the APNs token callbacks and the notification
/// center delegate (taps, and foreground presentation).
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
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

    /// A notification arrives while the app is open: the in-app banner already covers new requests,
    /// so only show the system one for something the inbox does not know.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let info = notification.request.content.userInfo
        let rid = info["rid"] as? String
        let mac = info["mac"] as? String
        let kind = info["kind"] as? String
        let known = await MainActor.run { () -> Bool in
            guard let store = AppEnvironment.current?.store, let rid, let mac else { return false }
            return store.coversNotification(macId: mac, rid: rid, kind: kind)
        }
        return known ? [] : [.banner, .list, .sound]
    }

    /// The user tapped a notification: open the request or the Agent it is about.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        guard let rid = info["rid"] as? String, let mac = info["mac"] as? String else { return }
        let kind = info["kind"] as? String
        await MainActor.run { AppEnvironment.current?.store.openNotification(macId: mac, rid: rid, kind: kind) }
    }
}
