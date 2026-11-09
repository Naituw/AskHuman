import Foundation
import ServiceManagement

/// Registration of the embedded LaunchAgent (`Contents/Library/LaunchAgents/<bundle id>.agent.plist`)
/// as a login item. S0 measured this as the launch path that works with CloudKit: the system starts the
/// app itself (not a bare binary), `AssociatedBundleIdentifiers` ties it to the bundle.
enum LoginItem {
    static var bundleID: String { Bundle.main.bundleIdentifier ?? "com.naituw.humaninloop.todosync" }
    static var plistName: String { "\(bundleID).agent.plist" }
    static var service: SMAppService { SMAppService.agent(plistName: plistName) }

    static func statusName(_ status: SMAppService.Status = service.status) -> String {
        switch status {
        case .notRegistered: return "notRegistered"
        case .enabled: return "enabled"
        case .requiresApproval: return "requiresApproval"
        case .notFound: return "notFound"
        @unknown default: return "unknown"
        }
    }
}

/// `AskHumanTodoSync --register | --unregister | --login-item-status | --version`: what the installer
/// script (and later the core) runs. Only a validated run invocation starts the application.
enum CommandLineTool {
    static func run(_ invocation: HelperInvocation) -> Int32? {
        switch invocation {
        case .version:
            print(Self.json(["version": Self.version, "systemFixture": 2]))
            return 0
        case .unmanaged(let command):
            var result: [String: Any] = ["loginItem": "unmanaged", "systemFixture": 2]
            if command != "--login-item-status" {
                result["error"] = "fixture helpers do not register or unregister shared login items"
            }
            print(Self.json(result))
            return command == "--login-item-status" ? 0 : 1
        case .loginItemStatus:
            print(Self.json(["loginItem": LoginItem.statusName()]))
            return 0
        case .register:
            var failure: String?
            do {
                // Re-registering an enabled agent throws; that is not a failure for the caller.
                if LoginItem.service.status != .enabled { try LoginItem.service.register() }
            } catch let thrown {
                // Approval pending is reported through the status below, not as an error.
                if LoginItem.service.status != .requiresApproval { failure = "\(thrown)" }
            }
            let status = LoginItem.service.status
            // The settings card provides an explicit link. Background maintenance must not open
            // System Settings merely because an approval is still pending.
            var out: [String: Any] = ["loginItem": LoginItem.statusName(status)]
            if let failure { out["error"] = failure }
            print(Self.json(out))
            return status == .enabled ? 0 : (status == .requiresApproval ? 3 : 1)
        case .unregister:
            var failure: String?
            do {
                if LoginItem.service.status != .notRegistered && LoginItem.service.status != .notFound {
                    try LoginItem.service.unregister()
                }
            } catch let thrown {
                failure = "\(thrown)"
            }
            var out: [String: Any] = ["loginItem": LoginItem.statusName()]
            if let failure { out["error"] = failure }
            print(Self.json(out))
            return failure == nil ? 0 : 1
        case .run:
            return nil
        }
    }

    static var version: String {
        (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String) ?? "0"
    }

    private static func json(_ object: [String: Any]) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]),
            let text = String(data: data, encoding: .utf8)
        else { return "{}" }
        return text
    }
}
