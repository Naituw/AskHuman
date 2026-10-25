import os
import UserNotifications
import AHProtocol
import AHStore

/// Turns the generic push into one that names the Mac, the Agent and the question, by decrypting the
/// preview with the pairing key. Anything that goes wrong leaves the system text (the `loc-key`
/// fallback such as "Your Agent has a question") in place.
final class NotificationService: UNNotificationServiceExtension {
    private static let log = Logger(subsystem: "com.naituw.humaninloop.ios.notification", category: "push")

    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?

    override func didReceive(
        _ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        self.contentHandler = contentHandler
        let content = (request.content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        bestAttempt = content

        let group = Bundle.main.object(forInfoDictionaryKey: "AHKeychainGroup") as? String
        let vault = Vault(secrets: KeychainSecretStore(service: "com.naituw.humaninloop.ios", accessGroup: group))
        switch PushDecryptor.decrypt(userInfo: request.content.userInfo, vault: vault) {
        case .success(let push):
            NotificationText.apply(push, to: content)
        case .failure(let failure):
            // Only the reason and a count: nothing of the push or of the keys ends up in the log.
            Self.log.notice(
                "push left generic: \(String(describing: failure), privacy: .public), pairings readable here: \(vault.pairings().count, privacy: .public)"
            )
        }
        contentHandler(content)
    }

    /// Out of time: deliver whatever is ready (the generic text if decryption did not finish).
    override func serviceExtensionTimeWillExpire() {
        if let contentHandler, let bestAttempt { contentHandler(bestAttempt) }
    }
}
