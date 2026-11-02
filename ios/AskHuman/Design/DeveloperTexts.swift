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
