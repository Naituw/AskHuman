import Foundation
import Observation
import AHProtocol

// The project todos in a request's answer form (docs/specs/special-replies.md D28/D29, docs/specs/todo-list-actions.md):
// the queue as the Mac lists it, which of them are picked, and what the user does with the list in place: complete a
// todo, delete it, add one. The picks belong to the answer form (`AnswerComposer.todoIds`, part of the draft): the picker
// reads the queue and moves the picks, the Mac does the rest (puts the text into the answer, takes the todo off the
// queue, delivers its attachments).
//
// Writes work as on the Todos tab (`TodosStore`): what is on screen is always *what the Mac last said* plus *the writes
// it has not answered yet*, so a refused or lost write disappears by itself, and an answer that arrives while a write is
// under way cannot make the list jump back. The request page does not subscribe to `todo.changed`: the Mac's answer to a
// write (the whole project after it) is what the list catches up with, and a refusal reads the queue again.

/// A todo the user just completed from the list, which they can still take back for a few seconds.
public struct CompletedTodo: Identifiable, Equatable, Sendable {
    /// Counts the completions of the picker, so that the next one is a note of its own.
    public var id: Int
    public var todoId: String
    public var text: String
    /// The queue as it was just before the completion: `restore` puts a todo at the end of the queue, and `reorder`
    /// by this puts it back where it was.
    var order: [String]
}

@MainActor
@Observable
public final class RequestTodoPicker {
    /// How many todos What's next lists before "Show all".
    public static let collapsedCount = 5
    /// How long a todo that was just added stays lit.
    public static let litDuration = Duration.milliseconds(1200)
    /// The ids of todos the phone makes up before the Mac has given them one.
    static let localPrefix = "local-"

    public enum Phase: Equatable, Sendable {
        case idle
        case loading
        case loaded
        /// The Mac did not answer. Nothing is shown, and a later look tries again.
        case failed
    }

    /// What `add` did with a text.
    public enum AddResult: Equatable, Sendable {
        /// Taken: it is in the list (sending) and on its way to the Mac.
        case added
        /// Not a todo: empty, or longer than the Mac keeps.
        case invalid(TodoTextError)
        /// The Mac cannot take it now (the notice says so): the text stays where it was written.
        case unavailable
    }

    /// The Mac the request came from.
    public let macId: String
    /// The id the Mac knows the request's project by: the hash of the project key, which the Mac works out itself
    /// and finds among the requests it is still waiting on.
    public let projectId: String
    public private(set) var phase: Phase = .idle
    /// What the Mac last said of the project; `nil` until it has said anything.
    public private(set) var confirmed: ProjectTodos?
    /// The card of an ask is open (a folded card is a single line).
    public var isExpanded = false
    /// What's next lists the whole queue (not only its first `collapsedCount`).
    public var showsAll = false

    // MARK: the add row

    /// What is being written in the add row. Memory only, kept while the request page is: collapsing the row (or the
    /// keyboard) does not lose it.
    public var addText = ""
    /// The add row is an input (and not the dashed line that opens it).
    public private(set) var isAdding = false
    /// The input has the keyboard.
    public var addFocused = false
    /// Why the last text was not taken (the Mac keeps `Wire.todoTextMaxChars` of it).
    public private(set) var addProblem: TodoTextError?
    /// The row of the todo that was just added, lit for a moment (`rowKey`: the same row before and after the Mac has
    /// given the todo its own id).
    public private(set) var litKey: String?
    /// The row the list is to scroll to (the one just added), and how many times it was asked to.
    public private(set) var revealKey: String?
    public private(set) var revealSerial = 0

    // MARK: what the screen says about writes

    /// The last completion, until it is taken back or goes by itself.
    public private(set) var undo: CompletedTodo?
    /// A write that did not go as expected, for a moment.
    public private(set) var notice: TodoNoticeItem?

    struct PendingWrite: Equatable, Sendable {
        var id: String
        var mutation: TodoMutation
    }

    /// The user's writes the Mac has not answered, oldest first.
    private var overlay: [PendingWrite] = []
    private let composer: AnswerComposer
    @ObservationIgnored private let backend: RemoteBackend
    @ObservationIgnored private let now: @Sendable () -> Date
    private var mac: MacState?
    @ObservationIgnored private var loading: Task<Void, Never>?
    @ObservationIgnored private var queue: [PendingWrite] = []
    @ObservationIgnored private var drainer: Task<Void, Never>?
    @ObservationIgnored private var litTask: Task<Void, Never>?
    /// The local id of each todo that was added here, by the id the Mac gave it.
    private var aliases: [String: String] = [:]
    /// Numbers the completions (`CompletedTodo.id`) and the notices (`TodoNoticeItem.id`).
    @ObservationIgnored private var serial = 0
    @ObservationIgnored private var noticeSerial = 0

    public init(
        backend: RemoteBackend, composer: AnswerComposer, macId: String, projectKey: String, mac: MacState? = nil,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.backend = backend
        self.composer = composer
        self.macId = macId
        self.projectId = TodoProjectId.of(key: projectKey)
        self.mac = mac
        self.now = now
    }

    /// The Macs have been heard from: what can be picked follows them. A Mac that came back, or that has just
    /// told what it can do, is asked again.
    public func update(mac next: MacState?) {
        let couldBefore = canLoad
        mac = next
        if !couldBefore, canLoad { reload() }
    }

    // MARK: what is offered

    /// Whether the Mac takes the todos picked with an answer (a Mac from before that would drop their ids).
    public var macTakesPicks: Bool {
        guard let caps = mac?.caps else { return false }
        return caps.todos && caps.answerTodos
    }

    /// The request takes todos at all: not a select-only form, and its Mac knows how.
    private var takesTodos: Bool { !composer.selectOnly && macTakesPicks }

    /// The queue can be asked for now.
    private var canLoad: Bool { takesTodos && mac?.link == .online }

    /// The queue is the Mac's to list: it can be had, or was, and stays on screen while the Mac is away so that
    /// the draft keeps its picks. Not so when the Mac did not answer: What's next then shows the todos its
    /// question carries, as it always has.
    public var listsQueue: Bool { takesTodos && phase != .failed }

    /// The list is on screen: its rows, or, with none, the way to add the first. (Nothing is drawn before the queue was
    /// read, so that a card does not blink in and out.)
    public var isVisible: Bool { listsQueue && phase == .loaded }

    /// The queue as the screen shows it, the todo the Agent takes first at the top: what the Mac last said, with the
    /// user's writes it has not answered yet laid over it.
    public var todos: [TodoItem] {
        guard var list = confirmed else { return [] }
        let at = now()
        for write in overlay { write.mutation.apply(to: &list, now: at) }
        return list.pending
    }

    public var count: Int { todos.count }

    /// A row of the list. The screen keeps its identity when the Mac answers the add that made it: the todo has another
    /// id then, and a row that is made again would blink.
    public struct Row: Identifiable, Equatable, Sendable {
        public let id: String
        public let todo: TodoItem
    }

    /// The identity a todo's row has on screen: the local id it was added under, or its own.
    public func rowKey(_ id: String) -> String { aliases[id] ?? id }

    /// Whether the todo's row is the one that was just added.
    public func isLit(_ id: String) -> Bool { litKey != nil && rowKey(id) == litKey }

    /// What What's next shows (`listed`), as rows.
    public var rows: [Row] { listed.map { Row(id: rowKey($0.id), todo: $0) } }

    /// The whole queue as rows (the card of an ordinary ask lists all of it).
    public var allRows: [Row] { todos.map { Row(id: rowKey($0.id), todo: $0) } }

    /// A project without todos: the ordinary card is a single line that opens the add row.
    public var showsEntry: Bool { isVisible && todos.isEmpty && !isAdding }

    public var pickedCount: Int { composer.todoIds.count }
    public var singleSelection: Bool { composer.ask.listsTodoQueue && composer.single }

    public func isPicked(_ id: String) -> Bool { composer.isTodoPicked(id) }

    /// What What's next shows of the queue: the first few (and the one picked, wherever it is), or all of it
    /// once asked for.
    public var listed: [TodoItem] {
        let all = todos
        if showsAll { return all }
        return all.enumerated().filter { $0.offset < Self.collapsedCount || isPicked($0.element.id) }.map(\.element)
    }

    /// The queue is longer than What's next shows at first, so there is a "Show all".
    public var canFold: Bool { count > Self.collapsedCount }

    /// Picks or lets go of a todo, retaining older Macs' single-choice behavior. A todo that has not been given
    /// its id by the Mac yet cannot be picked: the answer could not name it.
    public func toggle(_ id: String) {
        guard listsQueue, todos.contains(where: { $0.id == id }), !isSending(id) else { return }
        var picked = Set(composer.todoIds)
        if picked.contains(id) {
            picked.remove(id)
        } else if singleSelection {
            picked = [id]
        } else {
            picked.insert(id)
        }
        // The Mac merges them in this order: the queue's, as the popup does.
        composer.setTodoIds(todos.map(\.id).filter(picked.contains))
    }

    // MARK: reading

    /// Asks the Mac for the queue. Once at a time; a look while one is under way waits for it.
    public func load() async {
        if let loading { return await loading.value }
        guard canLoad else { return }
        let task = Task { await fetch() }
        loading = task
        await task.value
        loading = nil
    }

    /// The queue as it is now (the screen came back, or the Mac refused a pick that is gone).
    public func reload() {
        Task { await load() }
    }

    private func fetch() async {
        // A queue that is on screen stays there while it is asked for again.
        if phase != .loaded { phase = .loading }
        do {
            let list = try await backend.todoList(macId: macId, projectId: projectId)
            guard !Task.isCancelled else { return }
            confirmed = Self.known(list, projectId: projectId)
            phase = .loaded
            // A pick whose todo is gone (finished on another device, removed) cannot be sent.
            composer.keepTodos(in: Set(todos.map(\.id)))
            // A live queue (What's next, Stop) lists the queue itself from here on, not the todos its question carried.
            composer.dropTodoOptionSelections()
        } catch is CancellationError {
            if phase != .loaded { phase = .idle }
        } catch {
            if phase != .loaded {
                confirmed = nil
                phase = .failed
            }
        }
    }

    /// A project the Mac does not know (yet) is an empty queue that writes can start: the Mac finds the project among the
    /// requests it is still waiting on.
    private static func known(_ list: ProjectTodos, projectId: String) -> ProjectTodos {
        list.found ? list : ProjectTodos(id: projectId, rev: list.rev, name: "", canAuto: false)
    }

    // MARK: writing

    /// Whether the Mac can take a write now: it can be reached, knows todos, and has said what the queue holds (so there
    /// is something to show the write on). Writes are not queued for later.
    public var canWrite: Bool {
        guard takesTodos, let mac, mac.link == .online, mac.caps.todos else { return false }
        return confirmed != nil
    }

    /// The list stays on screen while the Mac is away, but nothing can be written to it.
    public var isOffline: Bool { listsQueue && phase == .loaded && !canWrite }

    /// A todo the user added that the Mac has not confirmed: it has no id of the Mac's yet, so nothing can be done to it.
    public func isSending(_ id: String) -> Bool {
        overlay.contains { $0.mutation.localId == id }
    }

    /// Whether a row takes the actions: the Mac can take them, and the todo is the Mac's.
    public func canAct(on id: String) -> Bool { canWrite && !isSending(id) }

    /// Marks a todo done: it leaves the queue for the history, as it does on the Todos tab, and is let go of if it was
    /// picked. The note that follows offers to take it back.
    public func complete(_ id: String) {
        guard let todo = actionable(id) else { return }
        let order = todos.map(\.id).filter { !$0.hasPrefix(Self.localPrefix) }
        guard write(.complete(id: id)) else { return }
        unpick(id)
        serial += 1
        undo = CompletedTodo(id: serial, todoId: todo.id, text: todo.text, order: order)
    }

    /// Takes a todo off the queue for good (no history, no undo: the screen asks nothing first, the swipe that opens
    /// the button is the confirmation), and lets go of it if it was picked.
    public func remove(_ id: String) {
        guard actionable(id) != nil, write(.remove(id: id)) else { return }
        unpick(id)
    }

    /// Takes the last completion back: the todo returns from the history (at the end of the queue), and goes to the
    /// place it had.
    public func undoComplete() {
        guard let undo else { return }
        self.undo = nil
        let order = undo.order
        guard write(.restore(id: undo.todoId)) else { return }
        if order.count > 1 { write(.reorder(ids: order)) }
    }

    public func dismissUndo(_ id: Int) {
        if undo?.id == id { undo = nil }
    }

    public func dismissNotice(_ id: Int) {
        if notice?.id == id { notice = nil }
    }

    /// The page goes away: what was only meant to be seen for a moment is not shown again when it comes back.
    public func clearTransient() {
        undo = nil
        notice = nil
    }

    /// Adds `raw` at the end of the queue, without ⚡ and attachments. The new row is lit for a moment, the list opens to
    /// show it, and it is never picked: adding is not answering.
    @discardableResult
    public func add(_ raw: String) -> AddResult {
        switch TodoText.validate(raw) {
        case .failure(let error):
            return .invalid(error)
        case .success(let text):
            let localId = Self.localPrefix + Bytes.newMsgId()
            guard write(.add(localId: localId, text: text, auto: false)) else { return .unavailable }
            isExpanded = true
            if count > Self.collapsedCount { showsAll = true }
            light(localId)
            revealKey = localId
            revealSerial += 1
            return .added
        }
    }

    /// The input's own send: adds what is written, and clears it for the next one. The row stays an input.
    public func submitAdd() {
        switch add(addText) {
        case .added:
            addText = ""
            addProblem = nil
        case .invalid(let error):
            // Nothing written is not worth a word.
            addProblem = error == .empty ? nil : error
        case .unavailable:
            break
        }
    }

    /// Opens the add row for typing: the dashed line (or the single-line entry of a project without todos) was tapped.
    public func beginAdd() {
        isExpanded = true
        isAdding = true
        addFocused = true
    }

    /// The input lost the keyboard (or got it). With nothing written it goes back to being a line; with a draft it stays,
    /// so that a draft is never hidden.
    public func addFocusChanged(_ focused: Bool) {
        addFocused = focused
        if !focused, addText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            isAdding = false
            addProblem = nil
        }
    }

    /// The text changed: whatever was said about the last one is stale.
    public func addTextChanged() {
        if addProblem != nil { addProblem = nil }
    }

    // MARK: writing, inside

    /// The todo an action is about, when it can be done to it now. Says so, when it cannot be done because of the Mac.
    private func actionable(_ id: String) -> TodoItem? {
        guard let todo = todos.first(where: { $0.id == id }), !isSending(id) else { return nil }
        guard canWrite else {
            post(.offline)
            return nil
        }
        return todo
    }

    private func unpick(_ id: String) {
        guard composer.isTodoPicked(id) else { return }
        composer.setTodoIds(composer.todoIds.filter { $0 != id })
    }

    private func light(_ key: String) {
        litKey = key
        litTask?.cancel()
        litTask = Task { [weak self] in
            try? await Task.sleep(for: Self.litDuration)
            guard !Task.isCancelled else { return }
            self?.litKey = nil
        }
    }

    /// Shows the write at once (on top of what the Mac said) and sends it, after the writes before it.
    @discardableResult
    private func write(_ mutation: TodoMutation) -> Bool {
        guard canWrite else {
            post(.offline)
            return false
        }
        let pending = PendingWrite(id: Bytes.newMsgId(), mutation: mutation)
        overlay.append(pending)
        queue.append(pending)
        if drainer == nil { drainer = Task { [weak self] in await self?.drain() } }
        return true
    }

    private func drain() async {
        while !queue.isEmpty {
            let item = queue.removeFirst()
            await send(item)
        }
        drainer = nil
    }

    private func send(_ item: PendingWrite) async {
        let operation = item.mutation.operation(clientMsgId: item.id, projectId: projectId)
        var ack: TodoAck?
        var failure: Error?
        // A lost answer is asked for again once; the id makes the repeat harmless.
        for _ in 0..<2 {
            do {
                ack = try await backend.todoApply(macId: macId, operation: operation)
                break
            } catch BackendError.timedOut {
                failure = BackendError.timedOut
            } catch {
                failure = error
                break
            }
        }
        settle(item, ack: ack)
        guard let ack else {
            let offline = (failure as? BackendError).map { $0 == .macOffline || $0 == .unknownMac } ?? false
            forgetUndo(of: item)
            post(offline ? .offline : .unconfirmed)
            if offline { dropQueued() }
            reload()
            return
        }
        switch ack.status {
        case "ok":
            // The todo the Mac made of an add has its own id: its row stays the one it was.
            if let localId = item.mutation.localId, let created = ack.todoId { aliases[created] = localId }
        case "notFound":
            forgetUndo(of: item)
            // A project the Mac does not know any more is not "handled elsewhere".
            post(ack.reason == "unknownProject" ? .refused("unknownProject") : .changedElsewhere)
        case "rejected":
            forgetUndo(of: item)
            post(.refused(ack.reason ?? ""))
        default:
            forgetUndo(of: item)
            post(.notSaved)
        }
        if ack.list == nil { reload() }
    }

    /// The Mac answered (or not): the write leaves the overlay and the Mac's list takes its place.
    private func settle(_ item: PendingWrite, ack: TodoAck?) {
        overlay.removeAll { $0.id == item.id }
        guard let list = ack?.list else { return }
        confirmed = Self.known(list, projectId: projectId)
        phase = .loaded
        // What the Mac says is gone (changed elsewhere) cannot stay picked.
        composer.keepTodos(in: Set(todos.map(\.id)))
    }

    /// The Mac cannot be reached: nothing queued will go out.
    private func dropQueued() {
        let dropped = Set(queue.map(\.id))
        overlay.removeAll { dropped.contains($0.id) }
        queue.removeAll()
    }

    /// A completion that did not take cannot be taken back.
    private func forgetUndo(of item: PendingWrite) {
        if case .complete(let id) = item.mutation, undo?.todoId == id { undo = nil }
    }

    private func post(_ notice: TodoNotice) {
        noticeSerial += 1
        self.notice = TodoNoticeItem(id: noticeSerial, notice: notice)
    }
}
