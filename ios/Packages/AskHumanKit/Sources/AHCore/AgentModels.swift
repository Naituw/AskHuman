import Foundation
import AHProtocol

// What the UI shows about Agents, live watching and new tasks (plan §4.4 – §4.7).

// MARK: agents

/// One Agent session of one Mac as the phone lists it.
public struct AgentItem: Identifiable, Sendable, Equatable {
    /// `macId|agentKey`.
    public var id: String
    public var macId: String
    public var macName: String
    public var view: AgentView

    public init(macId: String, macName: String, view: AgentView) {
        self.id = Self.key(macId, view.key)
        self.macId = macId
        self.macName = macName
        self.view = view
    }

    public static func key(_ macId: String, _ agentKey: String) -> String { macId + "|" + agentKey }

    /// The Mac id and Agent key an `id` was made of.
    public static func split(_ id: String) -> (macId: String, agentKey: String)? {
        guard let bar = id.firstIndex(of: "|") else { return nil }
        return (String(id[..<bar]), String(id[id.index(after: bar)...]))
    }

    public var agentKey: String { view.key }
    public var isWorking: Bool { view.state == "working" }
    public var isEnded: Bool { view.state == "ended" }
}

// MARK: watching

public struct WatchStep: Sendable, Equatable {
    /// `run`, `read`, `write` or `other`.
    public var kind: String
    /// Name of the tool when `kind` is `other`.
    public var name: String?
    public var object: String?
    /// `running`, `done` or `failed`.
    public var state: String

    public init(kind: String, name: String? = nil, object: String? = nil, state: String) {
        self.kind = kind
        self.name = name
        self.object = object
        self.state = state
    }
}

public struct WatchTodo: Sendable, Equatable {
    public var content: String
    /// `inProgress`, `completed` or `pending`.
    public var state: String

    public init(content: String, state: String) {
        self.content = content
        self.state = state
    }
}

/// The content of one `watch.frame`, decoded leniently: a missing field is an empty one.
public struct WatchFrameData: Sendable, Equatable {
    /// `working`, `idle`, `waiting` or `ended`.
    public var phase: String
    public var title: String?
    public var project: String?
    /// The latest words of the Agent (Markdown).
    public var text: String?
    public var steps: [WatchStep]
    public var stepsOmitted: Int
    public var todos: [WatchTodo]
    public var activeElapsedSecs: UInt64?
    /// Unix seconds of the last activity.
    public var at: Int64?
    /// The model the session runs with and its reasoning effort, when the Mac knows them.
    public var model: String?
    public var reasoning: String?

    public init(
        phase: String, title: String? = nil, project: String? = nil, text: String? = nil, steps: [WatchStep] = [],
        stepsOmitted: Int = 0, todos: [WatchTodo] = [], activeElapsedSecs: UInt64? = nil, at: Int64? = nil,
        model: String? = nil, reasoning: String? = nil
    ) {
        self.phase = phase
        self.title = title
        self.project = project
        self.text = text
        self.steps = steps
        self.stepsOmitted = stepsOmitted
        self.todos = todos
        self.activeElapsedSecs = activeElapsedSecs
        self.at = at
        self.model = model
        self.reasoning = reasoning
    }

    /// `nil` when the value is not a frame object.
    public init?(json: JSONValue) {
        guard case .object = json else { return nil }
        func string(_ key: String) -> String? {
            guard let value = json[key]?.stringValue, !value.isEmpty else { return nil }
            return value
        }
        let steps = (json["steps"]?.arrayValue ?? []).map { step in
            WatchStep(
                kind: step["kind"]?.stringValue ?? "other", name: step["name"]?.stringValue,
                object: step["object"]?.stringValue, state: step["state"]?.stringValue ?? "done"
            )
        }
        let todos = (json["todos"]?.arrayValue ?? []).compactMap { todo -> WatchTodo? in
            guard let content = todo["content"]?.stringValue else { return nil }
            return WatchTodo(content: content, state: todo["state"]?.stringValue ?? "pending")
        }
        self.init(
            phase: json["phase"]?.stringValue ?? "working", title: string("title"), project: string("project"),
            text: string("text"), steps: steps, stepsOmitted: Int(json["stepsOmitted"]?.intValue ?? 0), todos: todos,
            activeElapsedSecs: json["activeElapsedSecs"]?.intValue.map { UInt64(max(0, $0)) }, at: json["at"]?.intValue,
            model: string("model"), reasoning: string("reasoning")
        )
    }
}

/// A live watch of one Agent session: the latest frame, or why the stream ended.
public struct WatchState: Sendable, Equatable {
    public var macId: String
    public var agentKey: String
    public var seq: UInt64
    public var frame: WatchFrameData?
    /// Why the Mac stopped sending frames: `ended`, `limit` or `notFound`.
    public var closed: String?
    public var receivedAt: Date

    public init(macId: String, agentKey: String, seq: UInt64 = 0, frame: WatchFrameData? = nil, closed: String? = nil, receivedAt: Date) {
        self.macId = macId
        self.agentKey = agentKey
        self.seq = seq
        self.frame = frame
        self.closed = closed
        self.receivedAt = receivedAt
    }
}

// MARK: interjections

/// What the Mac did with a message sent to an Agent.
public struct InterjectOutcome: Sendable, Equatable {
    /// `queued` (the Agent reads it at its next step) or `delivered`.
    public var status: String
    /// Messages still waiting for the Agent, including this one when queued.
    public var pending: Int

    public init(status: String, pending: Int) {
        self.status = status
        self.pending = pending
    }
}

/// The messages waiting for an Agent.
public struct InterjectInfo: Sendable, Equatable {
    public var pending: Int
    /// Their full text, when asked for.
    public var text: String?

    public init(pending: Int, text: String?) {
        self.pending = pending
        self.text = text
    }
}

// MARK: new tasks

public enum TaskOptionsState: Sendable, Equatable {
    case loading
    case ready(TaskOptions)
    case failed(String)
}

/// A task created from the phone, from the moment it was sent until its Agent shows up.
public struct TaskProgress: Identifiable, Sendable, Equatable {
    /// The client message id.
    public var id: String
    public var macId: String
    /// What the user chose, for display (workspace name).
    public var workspace: String
    public var agentLabel: String
    public var text: String
    /// `sending`, `accepted`, `launching`, `launched`, `watching`, `unmatched` or `failed`.
    public var stage: String
    public var reason: String?
    public var agentKey: String?
    public var createdAt: Date
    /// The model the user picked (its display name; `nil` = "Default") and the effort (its id; `nil` = "Default").
    public var modelLabel: String?
    public var effort: String?

    public init(
        id: String, macId: String, workspace: String, agentLabel: String, text: String, stage: String, reason: String? = nil,
        agentKey: String? = nil, createdAt: Date, modelLabel: String? = nil, effort: String? = nil
    ) {
        self.id = id
        self.macId = macId
        self.workspace = workspace
        self.agentLabel = agentLabel
        self.text = text
        self.stage = stage
        self.reason = reason
        self.agentKey = agentKey
        self.createdAt = createdAt
        self.modelLabel = modelLabel
        self.effort = effort
    }

    /// The user picked a model or an effort (rather than leaving both to the Agent).
    public var hasModelPick: Bool { modelLabel != nil || effort != nil }

    /// Nothing more will change: the Agent was found (followed or not), or the task is lost.
    public var isFinal: Bool {
        switch stage {
        case "watching", "unmatched", "failed": return true
        case "launched": return agentKey != nil
        default: return false
        }
    }

    /// The outcome is unknown (the connection dropped) and sending the task again is safe.
    public var canRetry: Bool { stage == "lost" || stage == "failed" && reason == "connection" }

    /// The Agent session this task became, once it showed up.
    public var foundAgent: Bool { agentKey != nil && (stage == "watching" || stage == "launched") }

    /// Something is still going on (a spinner is right).
    public var isActive: Bool { !isFinal && stage != "lost" }
}

/// What the user entered in the new task form.
public struct TaskDraft: Sendable, Equatable {
    public var workspaceId: String
    public var workspaceName: String
    public var agentKind: String
    public var agentLabel: String
    /// `default` or `yolo`.
    public var permission: String
    public var todoId: String?
    public var text: String
    /// The model id and effort the user picked; `nil` = "Default", which sends nothing.
    public var model: String?
    public var effort: String?
    /// The display name of the picked model (the progress page shows it).
    public var modelLabel: String?

    public init(
        workspaceId: String, workspaceName: String, agentKind: String, agentLabel: String, permission: String, todoId: String? = nil,
        model: String? = nil, effort: String? = nil, modelLabel: String? = nil, text: String
    ) {
        self.workspaceId = workspaceId
        self.workspaceName = workspaceName
        self.agentKind = agentKind
        self.agentLabel = agentLabel
        self.permission = permission
        self.todoId = todoId
        self.text = text
        self.model = model
        self.effort = effort
        self.modelLabel = modelLabel
    }
}

extension EngineState {
    public static func todosKey(macId: String, workspaceId: String) -> String { macId + "|" + workspaceId }

    /// The TODOs a Mac offered for a workspace; `nil` until they arrived.
    public func todos(macId: String, workspaceId: String) -> [TodoOption]? {
        taskTodos[Self.todosKey(macId: macId, workspaceId: workspaceId)]
    }

    public func agent(macId: String, agentKey: String) -> AgentItem? {
        agents.first { $0.macId == macId && $0.agentKey == agentKey }
    }
}
