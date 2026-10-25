import Foundation

/// Words for the codes a Mac gives when it cannot start a task.
enum TaskTexts {
    /// Why a Mac offers no task form.
    static func blocked(_ reason: String?) -> String {
        switch reason {
        case "disabled": String(localized: "Agent tasks are turned off in AskHuman Settings on this Mac.")
        case "notReady": String(localized: "AskHuman on this Mac has to run as a login item before it can open terminals for tasks.")
        case "noTerminal": String(localized: "This Mac has no supported terminal or Codex App to start an Agent in.")
        case "noWorkspace": String(localized: "This Mac has no workspace to start a task in yet. Open a project with an Agent first.")
        case "noAgent": String(localized: "No Agent on this Mac is ready to start tasks.")
        case "timeout": String(localized: "The Mac did not answer in time.")
        default: String(localized: "This Mac cannot start tasks right now.")
        }
    }

    /// Why a task that was sent did not start.
    static func failure(_ reason: String?) -> String {
        switch reason {
        case "connection": String(localized: "The connection dropped. The Mac may or may not have received the task.")
        case "disabled", "notReady", "noTerminal": blocked(reason)
        case "workspaceGone": String(localized: "The workspace folder no longer exists on the Mac.")
        case "agentNotReady": String(localized: "The Agent is not ready on the Mac.")
        case "permissionChanged": String(localized: "The permission setting on the Mac changed. Open the form again.")
        case "todoGone": String(localized: "That TODO is gone from the Mac.")
        case "terminalFailed": String(localized: "The Mac could not open a terminal for the Agent.")
        case "invalid": String(localized: "The Mac did not accept this task.")
        case .some(let text) where !text.isEmpty: text
        default: String(localized: "The Agent could not be started.")
        }
    }
}
