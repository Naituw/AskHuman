import SwiftUI
import AHCore
import AHProtocol

/// How the UI names and colors the state of an Agent session.
extension AgentItem {
    var stateTitle: LocalizedStringKey {
        switch view.state {
        case "working": "Working"
        case "waiting": "Waiting for you"
        case "ended": view.lastTurnFailed ? "Failed" : "Finished"
        default: "Idle"
        }
    }

    var stateColor: Color {
        switch view.state {
        case "working": .green
        case "waiting": .orange
        case "ended": view.lastTurnFailed ? .red : .gray
        default: .gray
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
