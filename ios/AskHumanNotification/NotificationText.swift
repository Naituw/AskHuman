import Foundation
import UserNotifications
import AHProtocol
import AHStore

/// What a decrypted push looks like on the lock screen: the session's title (else the Agent and project)
/// as the title, who asks and the Mac as the subtitle (see `PushPlain.headerLines`), the question (or
/// what happened to it) as the body.
enum NotificationText {
    static func apply(_ push: DecryptedPush, to content: UNMutableNotificationContent) {
        let plain = push.plain
        let lines = plain.headerLines
        content.title = lines.title
        content.subtitle = lines.subtitle
        content.body = body(of: push)
        // Group by Mac, and carry what the app needs to open the right request on a tap.
        content.threadIdentifier = push.macId
        var info: [String: String] = ["mac": push.macId, "kind": plain.k]
        if let rid = plain.rid { info["rid"] = rid }
        content.userInfo = info
        if push.kind == .resolved {
            // It only updates an alert that is already there.
            content.interruptionLevel = .passive
            content.sound = nil
        }
    }

    static func body(of push: DecryptedPush) -> String {
        let text = push.plain.text.trimmingCharacters(in: .whitespacesAndNewlines)
        switch push.kind {
        case .ask, .other:
            return text
        case .confirm:
            return text.isEmpty ? String(localized: "Needs your confirmation") : String(localized: "Needs your confirmation: \(text)")
        case .followDone:
            return text.isEmpty ? String(localized: "Finished") : String(localized: "Finished: \(text)")
        case .followWaiting:
            return text.isEmpty ? String(localized: "Waiting for you") : String(localized: "Waiting for you: \(text)")
        case .followFailed:
            return text.isEmpty ? String(localized: "Failed") : String(localized: "Failed: \(text)")
        case .resolved:
            return resolution(text)
        }
    }

    /// The Mac sends `how` or `how: by` (`answered`, `answeredElsewhere: Feishu`, `cancelled`, `expired`).
    private static func resolution(_ text: String) -> String {
        let parts = text.split(separator: ":", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        let by = parts.count > 1 && !parts[1].isEmpty ? parts[1] : nil
        switch parts.first ?? "" {
        case "answered": return String(localized: "Answered")
        case "answeredElsewhere":
            return by.map { String(localized: "Answered on \($0)") } ?? String(localized: "Answered elsewhere")
        case "cancelled": return String(localized: "Cancelled")
        case "expired": return String(localized: "Expired")
        default: return String(localized: "Handled elsewhere")
        }
    }
}
