import Foundation
import Security

/// Reads the entitlements this process was signed with. `CKContainer(identifier:)` traps when the
/// entitlement for that container is missing, so the helper asks first and degrades to a "problem" reply.
enum Entitlements {
    static func value(_ key: String) -> Any? {
        guard let task = SecTaskCreateFromSelf(nil) else { return nil }
        return SecTaskCopyValueForEntitlement(task, key as CFString, nil)
    }

    static func grantsCloudKit(container: String) -> Bool {
        guard let ids = value("com.apple.developer.icloud-container-identifiers") as? [String] else { return false }
        guard let services = value("com.apple.developer.icloud-services") as? [String] else { return false }
        return ids.contains(container) && services.contains("CloudKit")
    }

    /// "production" for a Developer ID build (the profile pins the environment), else "development".
    static func cloudKitEnvironment() -> String {
        if let env = value("com.apple.developer.icloud-container-environment") as? String {
            return env.lowercased()
        }
        return "development"
    }

    static var hasPushEntitlement: Bool {
        value("com.apple.developer.aps-environment") != nil
    }
}
