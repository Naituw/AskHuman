import Foundation
import Observation
import AHProtocol

// The project todos a request can be answered with (docs/specs/special-replies.md D28/D29): the queue as the Mac
// lists it, which of them are picked, and whether there is anything to pick. The picks belong to the answer form
// (`AnswerComposer.todoIds`, part of the draft): the picker reads the queue and moves the picks, the Mac does the
// rest (puts the text into the answer, takes the todo off the queue, delivers its attachments).

@MainActor
@Observable
public final class RequestTodoPicker {
    /// How many todos What's next lists before "Show all".
    public static let collapsedCount = 5

    public enum Phase: Equatable, Sendable {
        case idle
        case loading
        case loaded
        /// The Mac did not answer. Nothing is shown, and a later look tries again.
        case failed
    }

    /// The Mac the request came from.
    public let macId: String
    /// The id the Mac knows the request's project by: the hash of the project key, which the Mac works out itself
    /// and finds among the requests it is still waiting on.
    public let projectId: String
    public private(set) var phase: Phase = .idle
    /// The queue, the todo the Agent takes first at the top. Only what is pending.
    public private(set) var todos: [TodoItem] = []
    /// The card of an ask is open (a folded card is a single line).
    public var isExpanded = false
    /// What's next lists the whole queue (not only its first `collapsedCount`).
    public var showsAll = false

    private let composer: AnswerComposer
    @ObservationIgnored private let backend: RemoteBackend
    private var mac: MacState?
    @ObservationIgnored private var loading: Task<Void, Never>?

    public init(backend: RemoteBackend, composer: AnswerComposer, macId: String, projectKey: String, mac: MacState? = nil) {
        self.backend = backend
        self.composer = composer
        self.macId = macId
        self.projectId = TodoProjectId.of(key: projectKey)
        self.mac = mac
    }

    /// The Macs have been heard from: what can be picked follows them. A Mac that came back, or that has just
    /// told what it can do, is asked again.
    public func update(mac next: MacState?) {
        let couldBefore = canLoad
        mac = next
        if !couldBefore, canLoad { reload() }
    }

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

    /// Something to pick: the card (or What's next's list) is shown only then.
    public var isVisible: Bool { listsQueue && phase == .loaded && !todos.isEmpty }

    public var count: Int { todos.count }

    public var pickedCount: Int { composer.todoIds.count }

    public func isPicked(_ id: String) -> Bool { composer.isTodoPicked(id) }

    /// What What's next shows of the queue: the first few (and the one picked, wherever it is), or all of it
    /// once asked for.
    public var listed: [TodoItem] {
        if showsAll { return todos }
        return todos.enumerated().filter { $0.offset < Self.collapsedCount || isPicked($0.element.id) }.map(\.element)
    }

    /// The queue is longer than What's next shows at first, so there is a "Show all".
    public var canFold: Bool { todos.count > Self.collapsedCount }

    /// Picks or lets go of a todo. In What's next a todo is the next task, one choice among the options, so
    /// picking another one replaces it.
    public func toggle(_ id: String) {
        guard listsQueue, todos.contains(where: { $0.id == id }) else { return }
        var picked = Set(composer.todoIds)
        if picked.contains(id) {
            picked.remove(id)
        } else if composer.ask.whatsNext {
            picked = [id]
        } else {
            picked.insert(id)
        }
        // The Mac merges them in this order: the queue's, as the popup does.
        composer.setTodoIds(todos.map(\.id).filter(picked.contains))
    }

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
            todos = list.found ? list.pending : []
            phase = .loaded
            // A pick whose todo is gone (finished on another device, removed) cannot be sent.
            composer.keepTodos(in: Set(todos.map(\.id)))
            // What's next lists the queue itself from here on, not the todos its question carried.
            composer.dropTodoOptionSelections()
        } catch is CancellationError {
            if phase != .loaded { phase = .idle }
        } catch {
            if phase != .loaded {
                todos = []
                phase = .failed
            }
        }
    }
}
