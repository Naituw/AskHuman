import Foundation
import Observation
import AHProtocol

/// How the user lets a new task act when the Mac asks every time.
public enum PermissionChoice: String, Sendable {
    /// The Agent asks before it acts.
    case standard = "default"
    /// The Agent skips its permission prompts.
    case yolo
}

/// The new task form: which Mac, workspace and Agent, how it may act, and what it should do.
@MainActor
@Observable
public final class TaskComposer {
    /// The longest task text the Mac accepts.
    public static let maxChars = 3000

    public var macId: String
    public var workspaceId: String?
    public var agentKind: String?
    /// What the user chose when the Mac lets the phone choose: nothing is preselected.
    public var permissionChoice: PermissionChoice?
    public var todoId: String?
    public var text = ""

    public init(macId: String) {
        self.macId = macId
    }

    // MARK: choices

    /// Picks what the form starts with once a Mac's choices are known: the workspace and Agent used
    /// last time when they are still offered, else the first sensible ones.
    public func applyDefaults(options: TaskOptions, defaults: UserDefaults) {
        if workspaceId == nil || !options.workspaces.contains(where: { $0.id == workspaceId }) {
            let last = defaults.string(forKey: Self.workspaceKey(macId))
            workspaceId = options.workspaces.first { $0.id == last }?.id
                ?? options.workspaces.first(where: \.pinned)?.id
                ?? options.workspaces.first?.id
        }
        if agentKind == nil || !options.agents.contains(where: { $0.kind == agentKind }) {
            let last = defaults.string(forKey: Self.agentKey(macId))
            agentKind = options.agents.first { $0.kind == last && $0.ready }?.kind
                ?? options.agents.first(where: \.ready)?.kind
                ?? options.agents.first?.kind
        }
    }

    /// Forgets the choices of the previous Mac.
    public func switchMac(to macId: String) {
        guard macId != self.macId else { return }
        self.macId = macId
        workspaceId = nil
        agentKind = nil
        todoId = nil
        permissionChoice = nil
    }

    /// Picks the Agent. The permission choice is made again for another Agent.
    public func selectAgent(_ kind: String) {
        guard kind != agentKind else { return }
        agentKind = kind
        permissionChoice = nil
    }

    public func remember(defaults: UserDefaults) {
        if let workspaceId { defaults.set(workspaceId, forKey: Self.workspaceKey(macId)) }
        if let agentKind { defaults.set(agentKind, forKey: Self.agentKey(macId)) }
    }

    private static func workspaceKey(_ macId: String) -> String { "ah.task.workspace." + macId }
    private static func agentKey(_ macId: String) -> String { "ah.task.agent." + macId }

    // MARK: derived

    public func workspace(in options: TaskOptions) -> WorkspaceOption? {
        options.workspaces.first { $0.id == workspaceId }
    }

    public func agent(in options: TaskOptions) -> AgentOption? {
        options.agents.first { $0.kind == agentKind }
    }

    /// The phone may pick the permission mode only when the Mac asks every time and the Agent has one.
    public func offersPermissionChoice(in options: TaskOptions) -> Bool {
        options.permissionMode == "ask" && agent(in: options)?.permissionSelectable == true
    }

    /// `default` or `yolo`, as the Mac's setting and the choice of the user decide.
    public func permission(in options: TaskOptions) -> String {
        switch options.permissionMode {
        case "yolo": return "yolo"
        case "agentDefault": return "default"
        default: return offersPermissionChoice(in: options) ? (permissionChoice ?? .standard).rawValue : "default"
        }
    }

    public var trimmedText: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }
    public var charCount: Int { trimmedText.count }
    public var overLimit: Bool { charCount > Self.maxChars }

    public func canSubmit(options: TaskOptions) -> Bool {
        guard options.enabled, workspace(in: options) != nil, let agent = agent(in: options), agent.ready else { return false }
        guard !overLimit else { return false }
        if offersPermissionChoice(in: options), permissionChoice == nil { return false }
        return !trimmedText.isEmpty || todoId != nil
    }

    /// What to send, or `nil` while the form is incomplete.
    public func draft(options: TaskOptions) -> TaskDraft? {
        guard canSubmit(options: options), let workspace = workspace(in: options), let agent = agent(in: options) else { return nil }
        return TaskDraft(
            workspaceId: workspace.id, workspaceName: workspace.label, agentKind: agent.kind, agentLabel: agent.label,
            permission: permission(in: options), todoId: todoId, text: trimmedText
        )
    }
}
