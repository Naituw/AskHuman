import Foundation
import AHCore

// The words and pictures of the developer mode. The logic that decides when they show is in AHCore.

extension DeveloperMenuItem {
    var title: String {
        switch self {
        case .quickTodo: String(localized: "Quick todo")
        }
    }

    var symbol: String {
        switch self {
        case .quickTodo: "text.badge.plus"
        }
    }
}

extension DeveloperToast {
    var text: String {
        switch message {
        case .todoAdded(let project): String(localized: "Added to \(project)")
        }
    }
}

// The quick-add page. What the Mac refused or failed to save is said as the Todos tab says it.

extension QuickTodoUnavailable {
    /// Why "Add" is dimmed (an empty text is not among the reasons: it needs no explaining).
    var text: String {
        switch self {
        case .noProject: String(localized: "Choose a project first.")
        case .macOffline(let mac): String(localized: "\(mac) is not reachable right now.")
        case .macNeedsUpdate(let mac): String(localized: "Update AskHuman on \(mac) to add todos from here.")
        case .icloudOffline: String(localized: "No Mac with iCloud sync is reachable right now.")
        case .tooLong(let count, let limit): String(localized: "That is too long: \(count) of \(limit) characters.")
        }
    }
}

extension QuickTodoFailure {
    /// Why the last "Add" did not make it.
    var text: String {
        switch self {
        case .unreachable(let mac): String(localized: "\(mac) is not reachable right now.")
        case .icloudUnreachable: String(localized: "No Mac with iCloud sync is reachable right now.")
        case .timedOut(let mac): String(localized: "\(mac) did not answer. Try again.")
        case .projectGone(let mac): String(localized: "This project is no longer on \(mac ?? String(localized: "iCloud")). Choose another.")
        case .notSaved: TodoNotice.notSaved.message
        case .refused(let reason): TodoNotice.refused(reason).message
        }
    }
}
