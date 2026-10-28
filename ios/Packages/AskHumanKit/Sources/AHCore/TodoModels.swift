import Foundation
import AHProtocol

// The pure parts of the todo screens (docs/specs/ios-todos.md §4, §7): which Mac serves a source,
// how the project rows are grouped and searched, what the sync status line says and how a write
// changes a list before the Mac has answered. `TodosStore` drives them; they know no backend.

/// Where the todos on screen come from.
public enum TodoSource: Hashable, Sendable {
    /// Everything that syncs through iCloud, served by whichever Mac with sync is online.
    case icloud
    /// What one Mac has: its projects, its directories.
    case mac(String)

    /// The form kept in `UserDefaults`.
    public var stored: String {
        switch self {
        case .icloud: return "icloud"
        case .mac(let id): return "mac:" + id
        }
    }

    public init?(stored: String) {
        if stored == "icloud" {
            self = .icloud
        } else if stored.hasPrefix("mac:"), stored.count > 4 {
            self = .mac(String(stored.dropFirst(4)))
        } else {
            return nil
        }
    }
}

/// What a Mac does with the todos it serves.
public enum TodoMode: String, Sendable, Equatable {
    /// Its own `todos.json` only.
    case local
    /// iCloud sync is on: the todos of every Mac that syncs.
    case icloud
}

/// Why the todos cannot be shown.
public enum TodoFailure: Equatable, Sendable {
    /// No Mac is paired.
    case noMacs
    /// The Mac behind the source cannot be reached.
    case offline
    case timedOut
    /// The Mac is from before the todo screen.
    case needsUpdate
    /// iCloud was asked for, but no Mac with sync is online. `anyOnline` says whether some other Mac is.
    case noSyncMac(anyOnline: Bool)
    case other(String)

    static func of(_ error: Error) -> TodoFailure {
        switch error as? BackendError {
        case .macOffline?, .unknownMac?: return .offline
        case .timedOut?: return .timedOut
        case .rejected("unsupported")?: return .needsUpdate
        case .rejected(let reason)?, .unavailable(let reason)?, .invalid(let reason)?: return .other(reason)
        default: return .other(error.localizedDescription)
        }
    }
}

public enum TodoLoadState: Equatable, Sendable {
    case idle
    case loading
    case loaded
    case failed(TodoFailure)

    public var isFailed: Bool {
        if case .failed = self { return true }
        return false
    }
}

// MARK: sources

/// One entry of the source menu.
public struct TodoSourceChoice: Identifiable, Equatable, Sendable {
    public enum Availability: Equatable, Sendable {
        case available
        case offline
        /// The Mac is from before the todo screen.
        case needsUpdate
        /// iCloud: no paired Mac is known to sync.
        case noSync
    }

    public var source: TodoSource
    /// The Mac's name; `nil` for iCloud.
    public var title: String?
    public var availability: Availability
    /// A Mac that is known to sync (it shows a cloud).
    public var syncs: Bool

    public var id: String { source.stored }
}

public enum TodoSources {
    /// The menu: iCloud first, then every paired Mac.
    public static func choices(macs: [MacState], modes: [String: TodoMode]) -> [TodoSourceChoice] {
        let known = macs.contains { modes[$0.id] == .icloud }
        var out = [
            TodoSourceChoice(source: .icloud, title: nil, availability: known ? .available : .noSync, syncs: true)
        ]
        for mac in macs {
            let availability: TodoSourceChoice.Availability =
                !mac.caps.todos ? .needsUpdate : (mac.link == .online ? .available : .offline)
            out.append(
                TodoSourceChoice(source: .mac(mac.id), title: mac.name, availability: availability, syncs: modes[mac.id] == .icloud)
            )
        }
        return out
    }

    /// The source to start with: the user's last choice when it is still valid, else the only Mac, else the
    /// first Mac that is online (or simply the first).
    public static func initial(stored: TodoSource?, macs: [MacState], modes: [String: TodoMode]) -> TodoSource? {
        guard !macs.isEmpty else { return nil }
        switch stored {
        case .icloud?:
            // Kept until a Mac says otherwise: the modes are only learned once the screen has loaded.
            if macs.contains(where: { $0.caps.todos && modes[$0.id] != .local }) { return .icloud }
        case .mac(let id)?:
            if macs.contains(where: { $0.id == id }) { return .mac(id) }
        case nil:
            break
        }
        if macs.count == 1 { return .mac(macs[0].id) }
        let online = macs.first { $0.link == .online && $0.caps.todos }
        return .mac((online ?? macs[0]).id)
    }

    /// The Macs that can serve the iCloud view, best first: the one used last, then those known to sync,
    /// then those whose mode is not known yet. A Mac that is known not to sync, is offline, is too old or
    /// already failed is left out.
    public static func icloudCandidates(
        macs: [MacState], modes: [String: TodoMode], lastUsed: String?, failed: Set<String> = []
    ) -> [String] {
        let usable = macs.filter {
            $0.link == .online && $0.caps.todos && modes[$0.id] != .local && !failed.contains($0.id)
        }
        let rank: (MacState) -> Int = { mac in
            if mac.id == lastUsed, modes[mac.id] == .icloud { return 0 }
            if modes[mac.id] == .icloud { return 1 }
            if mac.id == lastUsed { return 2 }
            return 3
        }
        // A stable sort: Macs of equal rank stay in the order they were paired.
        return usable.enumerated()
            .sorted { (rank($0.element), $0.offset) < (rank($1.element), $1.offset) }
            .map(\.element.id)
    }
}

// MARK: project rows

/// A group of project rows.
public struct TodoSection: Identifiable, Equatable, Sendable {
    public enum Kind: String, Sendable {
        case withTodos, synced, recent, empty, localOnly
    }

    public var kind: Kind
    public var rows: [TodoProjectRow]
    /// How many rows the section has before the search narrows them.
    public var total: Int
    /// The rows are hidden until the user opens the section (projects without todos; a search opens it).
    public var collapsed: Bool

    public var id: String { kind.rawValue }
}

public enum TodoRows {
    /// Rows that match every word of `query` in their name, hint, repository or Mac names.
    public static func matches(_ row: TodoProjectRow, query: String) -> Bool {
        let words = query.lowercased().split(whereSeparator: \.isWhitespace)
        if words.isEmpty { return true }
        let haystack = ([row.name, row.hint ?? "", row.repo ?? ""] + row.macs).joined(separator: "\n").lowercased()
        return words.allSatisfy { haystack.contains($0) }
    }

    /// Groups the rows a source shows. A Mac's own view leaves out the projects that exist only in iCloud
    /// (they have no directory there).
    public static func sections(
        rows: [TodoProjectRow], source: TodoSource, query: String, emptyExpanded: Bool
    ) -> [TodoSection] {
        let visible = rows.filter { row in
            if case .mac = source, row.scope == "cloudOnly" { return false }
            return true
        }
        let searching = !query.trimmingCharacters(in: .whitespaces).isEmpty
        var out: [TodoSection] = []
        for kind in [TodoSection.Kind.withTodos, .synced, .recent, .empty, .localOnly] {
            let inKind = visible.filter { $0.section == kind.rawValue }
            let matched = searching ? inKind.filter { matches($0, query: query) } : inKind
            guard !matched.isEmpty else { continue }
            let collapsed = kind == .empty && !emptyExpanded && !searching
            out.append(TodoSection(kind: kind, rows: collapsed ? [] : matched, total: inKind.count, collapsed: collapsed))
        }
        return out
    }
}

// MARK: sync status

/// What the iCloud status line says, derived from facts the Mac reports.
public enum TodoSyncLine: Equatable, Sendable {
    case notYet
    case synced(at: Date)
    case syncing
    /// Changes wait for the network.
    case waiting(unsynced: Int)
    case notSignedIn
    case accountChanged
    case zoneMissing
    case quota
    case helperNotRunning
    case helperIncompatible
    case helperUnsupported

    /// Whether the line is an error the user may want to look into (it turns red).
    public var isProblem: Bool {
        switch self {
        case .notYet, .synced, .syncing, .waiting: return false
        default: return true
        }
    }

    public static func derive(_ info: TodoSyncInfo) -> TodoSyncLine {
        switch info.helper {
        case "unsupported": return .helperUnsupported
        case "incompatible": return .helperIncompatible
        case "notRunning": return .helperNotRunning
        default: break
        }
        switch info.paused {
        case "notSignedIn"?: return .notSignedIn
        case "accountChanged"?: return .accountChanged
        case "zoneMissing"?: return .zoneMissing
        case "quota"?: return .quota
        default: break
        }
        if info.syncing { return .syncing }
        if info.unsynced > 0 { return .waiting(unsynced: Int(info.unsynced)) }
        let last = max(info.lastPullMs, info.lastPushMs)
        if last == 0 { return .notYet }
        return .synced(at: Date(timeIntervalSince1970: Double(last) / 1000))
    }
}

// MARK: writes

public enum TodoTextError: Error, Equatable, Sendable {
    case empty
    case tooLong
}

public enum TodoText {
    /// The text as it will be stored, or why it cannot be.
    public static func validate(_ raw: String) -> Result<String, TodoTextError> {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty { return .failure(.empty) }
        if text.count > Wire.todoTextMaxChars { return .failure(.tooLong) }
        return .success(text)
    }
}

/// A change the user made to a project's todos. It is shown at once (`apply`) and sent to the Mac
/// (`operation`); the Mac's answer replaces whatever was shown.
public enum TodoMutation: Equatable, Sendable {
    case add(localId: String, text: String, auto: Bool)
    case edit(id: String, text: String)
    case complete(id: String)
    case restore(id: String)
    case remove(id: String)
    case reorder(ids: [String])
    case setAuto(id: String, auto: Bool)

    /// Whether the change creates an entry that has no Mac id yet.
    public var localId: String? {
        if case .add(let id, _, _) = self { return id }
        return nil
    }

    public func operation(clientMsgId: String, projectId: String) -> TodoOperation {
        func op(_ kind: String, todoId: String? = nil, text: String? = nil, auto: Bool? = nil, order: [String]? = nil) -> TodoOperation {
            TodoOperation(clientMsgId: clientMsgId, id: projectId, op: kind, todoId: todoId, text: text, auto: auto, order: order)
        }
        switch self {
        case .add(_, let text, let auto): return op("add", text: text, auto: auto ? true : nil)
        case .edit(let id, let text): return op("edit", todoId: id, text: text)
        case .complete(let id): return op("complete", todoId: id)
        case .restore(let id): return op("restore", todoId: id)
        case .remove(let id): return op("remove", todoId: id)
        case .reorder(let ids): return op("reorder", order: ids)
        case .setAuto(let id, let auto): return op("auto", todoId: id, auto: auto)
        }
    }

    public func apply(to list: inout ProjectTodos, now: Date) {
        let nowMs = UInt64(max(0, now.timeIntervalSince1970 * 1000))
        switch self {
        case .add(let localId, let text, let auto):
            list.pending.append(TodoItem(id: localId, text: text, createdAtMs: nowMs, auto: auto))
            list.pendingTotal += 1
        case .edit(let id, let text):
            if let index = list.pending.firstIndex(where: { $0.id == id }) { list.pending[index].text = text }
        case .complete(let id):
            guard let index = list.pending.firstIndex(where: { $0.id == id }) else { return }
            let item = list.pending.remove(at: index)
            list.pendingTotal = list.pendingTotal > 0 ? list.pendingTotal - 1 : 0
            list.history.insert(
                TodoDoneItem(id: item.id, text: item.text, createdAtMs: item.createdAtMs, doneAtMs: nowMs, agent: item.agent, att: item.att),
                at: 0
            )
            if list.history.count > Wire.todoMaxHistory { list.history.removeLast(list.history.count - Wire.todoMaxHistory) }
            list.historyTotal += 1
        case .restore(let id):
            guard let index = list.history.firstIndex(where: { $0.id == id }) else { return }
            let done = list.history.remove(at: index)
            list.historyTotal = list.historyTotal > 0 ? list.historyTotal - 1 : 0
            list.pending.append(TodoItem(id: done.id, text: done.text, createdAtMs: done.createdAtMs, agent: done.agent, att: done.att))
            list.pendingTotal += 1
        case .remove(let id):
            guard let index = list.pending.firstIndex(where: { $0.id == id }) else { return }
            list.pending.remove(at: index)
            list.pendingTotal = list.pendingTotal > 0 ? list.pendingTotal - 1 : 0
        case .reorder(let ids):
            // Listed ones first in the given order; the ones not listed keep their order after them.
            var rest = list.pending
            var listed: [TodoItem] = []
            for id in ids {
                if let index = rest.firstIndex(where: { $0.id == id }) { listed.append(rest.remove(at: index)) }
            }
            list.pending = listed + rest
        case .setAuto(let id, let auto):
            if let index = list.pending.firstIndex(where: { $0.id == id }) { list.pending[index].auto = auto }
        }
    }
}

/// What the screen says about a write that did not go as the user expected.
public enum TodoNotice: Equatable, Sendable {
    /// The todo was done, removed or edited elsewhere in the meantime.
    case changedElsewhere
    /// The Mac refused: `empty`, `tooLong`, `cloudOnly`, `unsupported`, …
    case refused(String)
    /// The Mac could not store it.
    case notSaved
    /// The Mac cannot be reached.
    case offline
    /// No answer came: the change may or may not have happened; the list shown is the Mac's.
    case unconfirmed
}
