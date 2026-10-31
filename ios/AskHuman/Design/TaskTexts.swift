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
        case "modelUnavailable": String(localized: "The Agent on the Mac no longer offers that model. Open the form again and choose another.")
        case "effortUnavailable": String(localized: "That effort is not available for the chosen model. Open the form again and choose another.")
        case "attachment": String(localized: "The task attachments are not available. Your task was not started. Please try again.")
        case "attachmentsUnsupported": String(localized: "Update AskHuman on this Mac to start tasks with attachments.")
        case "speedUnavailable": String(localized: "That speed is no longer available on the Mac. Open the form again and choose another.")
        case "terminalFailed": String(localized: "The Mac could not open a terminal for the Agent.")
        case "invalid": String(localized: "The Mac did not accept this task.")
        case .some(let text) where !text.isEmpty: text
        default: String(localized: "The Agent could not be started.")
        }
    }

    // MARK: model and effort

    /// The name of a reasoning effort: the levels the Agents document, else the code as the Mac gave it.
    static func effort(_ id: String) -> String {
        switch id {
        case "none": String(localized: "None")
        case "minimal": String(localized: "Minimal")
        case "low": String(localized: "Low")
        case "medium": String(localized: "Medium")
        case "high": String(localized: "High")
        case "xhigh": String(localized: "Extra high")
        case "max": String(localized: "Max")
        default: id.prefix(1).uppercased() + id.dropFirst()
        }
    }

    /// What the new task form calls "Default" for the effort: with the level it stands for, when known.
    static func defaultEffort(_ level: String?) -> String {
        guard let level else { return String(localized: "Default") }
        return String(localized: "Default (\(effort(level)))")
    }

    /// "Model: Opus · Effort: High" for what was picked in the form; "Default" stands for what was left alone.
    static func pick(modelLabel: String?, effort id: String?) -> String {
        var parts = [String(localized: "Model: \(modelLabel ?? String(localized: "Default"))")]
        if let id { parts.append(String(localized: "Effort: \(effort(id))")) }
        return parts.joined(separator: " · ")
    }

    /// "Model: x · Effort: y" for what an Agent session runs with; `nil` when the Mac told neither.
    static func running(model: String?, reasoning: String?) -> String? {
        var parts: [String] = []
        if let model { parts.append(String(localized: "Model: \(model)")) }
        if let reasoning { parts.append(String(localized: "Effort: \(effort(reasoning))")) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}
