import CryptoKit
import Foundation
import AHProtocol

// What the Agents are doing in the projects of the Todos tab (docs/specs/ios-todos.md I18). The dots are
// not a second source of truth: they are the states the Agents tab already shows, laid over the project rows.

/// What the Agents of a project say about it, most urgent last. An ended session says nothing.
public enum TodoActivity: Int, Comparable, Sendable {
    case idle = 0
    case working = 1
    /// An Agent has asked a question and waits for the answer.
    case waiting = 2

    public static func < (lhs: TodoActivity, rhs: TodoActivity) -> Bool { lhs.rawValue < rhs.rawValue }

    /// The activity of an Agent in `state` (`AgentView.state`); `nil` for a session that has ended.
    public init?(agentState state: String) {
        switch state {
        case "waiting": self = .waiting
        case "working": self = .working
        case "ended": return nil
        default: self = .idle
        }
    }
}

/// How a project row is told from the path an Agent works in. The Mac names a row by a hash of its project
/// key and never sends the path (`todos_view::project_id`); the same hash lets the phone recognize the
/// Agents of a row that has no repository to go by.
public enum TodoProjectId {
    public static func of(key: String) -> String {
        SHA256.hash(data: Data(key.utf8)).prefix(8).map { String(format: "%02x", $0) }.joined()
    }
}

/// The live Agents, indexed for the project rows. An Agent belongs to a row when they name the same
/// repository (whichever Mac and worktree each is in), or when the Agent works in the very directory the
/// row stands for on the same Mac (projects without a repository). The most urgent state of a project wins:
/// waiting, then working, then idle.
public struct TodoActivityIndex: Equatable, Sendable {
    private var byRepo: [String: TodoActivity] = [:]
    /// By `macId|projectId`.
    private var byProject: [String: TodoActivity] = [:]

    public init() {}

    public init(agents: [AgentItem]) {
        for item in agents {
            guard let activity = TodoActivity(agentState: item.view.state) else { continue }
            let project = item.view.project
            if let repo = project.repo, !repo.isEmpty {
                Self.raise(&byRepo, repo, to: activity)
            }
            if !project.key.isEmpty {
                Self.raise(&byProject, TodosStore.key(item.macId, TodoProjectId.of(key: project.key)), to: activity)
            }
        }
    }

    public var isEmpty: Bool { byRepo.isEmpty && byProject.isEmpty }

    /// The activity in a project the screen lists, or `nil` when no Agent is live in it.
    public func activity(of project: TodoListedProject) -> TodoActivity? {
        let byRepo = project.row.repo.flatMap { self.byRepo[$0] }
        let byDirectory = byProject[project.key]
        switch (byRepo, byDirectory) {
        case (let a?, let b?): return max(a, b)
        case (let a?, nil): return a
        case (nil, let b?): return b
        case (nil, nil): return nil
        }
    }

    private static func raise(_ map: inout [String: TodoActivity], _ key: String, to activity: TodoActivity) {
        if let known = map[key], known >= activity { return }
        map[key] = activity
    }
}
