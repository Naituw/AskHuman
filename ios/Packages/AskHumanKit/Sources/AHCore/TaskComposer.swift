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

/// The reasoning efforts the form offers for the model chosen so far.
public struct EffortChoices: Equatable, Sendable {
    /// The levels, in the order the Mac gave them.
    public var levels: [String]
    /// The level "Default" stands for, when the Mac says (always one of `levels`).
    public var defaultLevel: String?

    public init(levels: [String], defaultLevel: String? = nil) {
        self.levels = levels
        self.defaultLevel = defaultLevel.flatMap { levels.contains($0) ? $0 : nil }
    }
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
    /// The model the user picked (`nil` = "Default"). Never remembered between forms, and only meaningful
    /// for the Agent it was picked for: read it through `model(in:)`, which checks it is still offered.
    public private(set) var modelId: String?
    /// The reasoning effort the user picked (`nil` = "Default"); read it through `effort(in:)`.
    public var effortId: String?
    public var todoId: String?
    public var text = ""

    public init(macId: String) {
        self.macId = macId
    }

    // MARK: choices

    /// Picks what the form starts with once a Mac's choices are known: the workspace and Agent used
    /// last time when they are still offered, else the first sensible ones. The model and the effort
    /// always start from "Default" (a pick that is no longer offered goes back to it).
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
        dropStalePicks(in: options)
    }

    /// Forgets the choices of the previous Mac.
    public func switchMac(to macId: String) {
        guard macId != self.macId else { return }
        self.macId = macId
        workspaceId = nil
        agentKind = nil
        todoId = nil
        permissionChoice = nil
        modelId = nil
        effortId = nil
    }

    /// Picks the Agent. The permission, the model and the effort are chosen again for another Agent.
    public func selectAgent(_ kind: String) {
        guard kind != agentKind else { return }
        agentKind = kind
        permissionChoice = nil
        modelId = nil
        effortId = nil
    }

    /// Picks the model (`nil` = "Default"). The effort stays when the new choice offers it, else it is
    /// "Default" again.
    public func selectModel(_ id: String?, in options: TaskOptions) {
        modelId = id
        dropStalePicks(in: options)
    }

    public func remember(defaults: UserDefaults) {
        if let workspaceId { defaults.set(workspaceId, forKey: Self.workspaceKey(macId)) }
        if let agentKind { defaults.set(agentKind, forKey: Self.agentKey(macId)) }
    }

    private static func workspaceKey(_ macId: String) -> String { "ah.task.workspace." + macId }
    private static func agentKey(_ macId: String) -> String { "ah.task.agent." + macId }

    /// A model or an effort the choices no longer offer is forgotten: what the form shows and what it
    /// sends always agree.
    private func dropStalePicks(in options: TaskOptions) {
        if modelId != nil, model(in: options) == nil { modelId = nil }
        if effortId != nil, effort(in: options) == nil { effortId = nil }
    }

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

    // MARK: model and effort

    /// The models of the picked Agent, in the Mac's order; empty when it gave none (an older Mac, or one
    /// that could not list them): then the form has neither a model row nor an effort row.
    public func models(in options: TaskOptions) -> [ModelOption] {
        agent(in: options)?.models ?? []
    }

    public func offersModelChoice(in options: TaskOptions) -> Bool {
        !models(in: options).isEmpty
    }

    /// The model picked, or `nil` for "Default".
    public func model(in options: TaskOptions) -> ModelOption? {
        guard let modelId else { return nil }
        return models(in: options).first { $0.id == modelId }
    }

    /// What "Default" stands for on the picked Agent, as far as the Mac told.
    public func defaultModel(in options: TaskOptions) -> DefaultModel? {
        agent(in: options)?.defaultModel
    }

    /// The efforts the form offers, or `nil` while it cannot tell: the picked model names them, or,
    /// with "Default", the model "Default" stands for does.
    public func effortChoices(in options: TaskOptions) -> EffortChoices? {
        guard offersModelChoice(in: options) else { return nil }
        let source: (efforts: [String]?, defaultEffort: String?)
        if let model = model(in: options) {
            source = (model.efforts, model.defaultEffort)
        } else {
            let standing = defaultModel(in: options)
            source = (standing?.efforts, standing?.defaultEffort)
        }
        guard let levels = source.efforts, !levels.isEmpty else { return nil }
        return EffortChoices(levels: levels, defaultLevel: source.defaultEffort)
    }

    /// The effort picked, or `nil` for "Default".
    public func effort(in options: TaskOptions) -> String? {
        guard let effortId, effortChoices(in: options)?.levels.contains(effortId) == true else { return nil }
        return effortId
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
        let model = model(in: options)
        return TaskDraft(
            workspaceId: workspace.id, workspaceName: workspace.label, agentKind: agent.kind, agentLabel: agent.label,
            permission: permission(in: options), todoId: todoId, model: model?.id, effort: effort(in: options),
            modelLabel: model?.label, text: trimmedText
        )
    }
}
