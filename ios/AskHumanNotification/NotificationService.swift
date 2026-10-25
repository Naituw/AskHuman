import UserNotifications
import AHProtocol
import AHStore

/// Turns the generic push into one that names the Mac, the Agent and the question, by decrypting the
/// preview with the pairing key. Anything that goes wrong leaves the system text (the `loc-key`
/// fallback such as "Your Agent has a question") in place.
final class NotificationService: UNNotificationServiceExtension {
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
        if case .success(let push) = PushDecryptor.decrypt(userInfo: request.content.userInfo, vault: vault) {
            NotificationText.apply(push, to: content)
        }
        contentHandler(content)
    }

    /// Out of time: deliver whatever is ready (the generic text if decryption did not finish).
    override func serviceExtensionTimeWillExpire() {
        if let contentHandler, let bestAttempt { contentHandler(bestAttempt) }
    }
}
