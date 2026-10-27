import Foundation
import Observation
import UIKit
import UserNotifications
import AHCore

/// Notification permission and the APNs device token. The token goes to the engine, which registers
/// it with the push relay and tells every paired Mac.
@MainActor
@Observable
final class PushCoordinator {
    enum Authorization: Equatable {
        case unknown, notDetermined, denied, authorized
    }

    private(set) var authorization: Authorization
    private(set) var registrationError: String?

    @ObservationIgnored private let store: AppStore
    @ObservationIgnored private let simulated: Bool

    init(store: AppStore, simulated: Bool) {
        self.store = store
        self.simulated = simulated
        authorization = simulated ? .authorized : .unknown
    }

    /// Reads the current permission; registers for remote notifications when it is granted.
    func refreshAuthorization() async {
        guard !simulated else { return }
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        switch settings.authorizationStatus {
        case .notDetermined: authorization = .notDetermined
        case .denied: authorization = .denied
        default:
            authorization = .authorized
            UIApplication.shared.registerForRemoteNotifications()
        }
    }

    /// Asks for permission (the first time only the system dialog appears).
    func requestAuthorization() async {
        guard !simulated else { return }
        _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound])
        await refreshAuthorization()
    }

    func openSystemSettings() {
        guard let url = URL(string: UIApplication.openNotificationSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }

    func didRegister(token: Data) {
        registrationError = nil
        Task { await store.updatePushToken(token) }
    }

    func didFail(error: Error) {
        registrationError = error.localizedDescription
        Task { await store.updatePushToken(nil) }
    }

    /// The badge shows how many requests wait for an answer.
    func updateBadge(_ count: Int) {
        guard !simulated, authorization == .authorized else { return }
        UNUserNotificationCenter.current().setBadgeCount(count) { _ in }
    }
}
