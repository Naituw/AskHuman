import SwiftUI
import AHCore
import AHProtocol

/// How the UI names and colors the state of an Agent session.
extension AgentItem {
    var stateTitle: LocalizedStringKey {
        switch view.state {
        case "working": "Working"
        case "waiting": "Waiting for you"
        case "ended": "Session ended"
        default: "Idle"
        }
    }

    var stateColor: Color {
        switch view.state {
        case "working": .green
        case "waiting": .orange
        case "ended": .gray
        default: .gray
        }
    }

    var resultTitle: LocalizedStringKey? {
        switch view.lastTurnResult?.outcome {
        case "completed": "Completed"
        case "failed": "Last turn failed"
        case "incomplete": "Incomplete"
        case "interrupted": "Interrupted"
        case "unknown": "Result unknown"
        default: view.lastTurnResult != nil ? "Result unknown" : view.lastTurnFailed ? "Last turn failed" : nil
        }
    }

    var resultColor: Color { view.lastTurnResult?.outcome == "incomplete" ? .orange : needsAttention ? .red : .gray }

    var resultReason: String? {
        if let summary = view.lastTurnResult?.summary, !summary.isEmpty { return summary }
        switch view.lastTurnResult?.reason {
        case "max_turns": return String(localized: "The Agent reached its run limit.")
        case "no_progress": return String(localized: "The Agent stopped because it was not making progress.")
        case "user_interrupt": return String(localized: "You stopped this turn.")
        case "permission_rejected", "permission_cancelled": return String(localized: "The permission request was declined or cancelled.")
        case "runtime_error": return String(localized: "The Agent ended this turn with an error.")
        default: return nil
        }
    }

    var badgeAgent: AgentRef { AgentRef(kind: view.kind, label: view.kindLabel) }

    /// The title of the session, or the name of its Agent when it has none yet.
    var displayTitle: String {
        let title = view.title.trimmingCharacters(in: .whitespacesAndNewlines)
        return title.isEmpty ? view.kindLabel : title
    }

    var lastActivity: Date { Date(timeIntervalSince1970: Double(view.lastActivityAt) / 1000) }
    var startedAt: Date { Date(timeIntervalSince1970: Double(view.startedAt) / 1000) }
}

extension TimeInterval {
    /// "3 min, 12 sec" in the user's language.
    var elapsedText: String {
        Duration.seconds(Int(self)).formatted(.units(allowed: [.hours, .minutes, .seconds], width: .abbreviated, maximumUnitCount: 2))
    }
}
