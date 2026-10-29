import Foundation
import AHProtocol
import AHStore

/// What the UI layer needs from the world behind the phone: the real session engine
/// (`PhoneEngine`) or the in-memory mock (`AHMock`). Everything is `async`; results flow back as
/// `EngineState` snapshots and `EngineEvent`s.
public protocol RemoteBackend: AnyObject, Sendable {
    /// Conflated snapshots: a slow consumer only ever sees the latest one.
    var states: AsyncStream<EngineState> { get }
    var events: AsyncStream<EngineEvent> { get }

    /// Loads persisted data and publishes the first state.
    func start() async
    /// `true` connects to the broker; `false` says goodbye and disconnects.
    func setForeground(_ foreground: Bool) async
    /// The app is on screen but not receiving events (Control Center, the notification center, the app
    /// switcher, a system alert): not a departure. The connection stays for a grace period and only a
    /// stay that outlasts it says goodbye, as `setForeground(false)` does at once. `setForeground(true)`
    /// in the meantime keeps the connection.
    func setInactive() async
    /// Pull to refresh: reconnects when the link is down and asks every reachable Mac for an
    /// authoritative inbox snapshot (or, for a Mac whose handshake never finished, starts it over).
    func refresh() async
    /// A notification about a request this phone does not hold reached the open app. A Mac pushes
    /// only when it got no acknowledgement for the request it sent live, so the link or the session
    /// to that Mac is not what both sides think it is. Probes the broker connection (a dead one is
    /// replaced at once), then starts the Mac's handshake over or asks for its inbox again.
    func checkLink(macId: String) async

    /// The name the Macs show for this phone (used by pairings made and sessions opened from now on).
    func setDeviceName(_ name: String) async

    // pairing
    func startPairing(code: String) async
    func cancelPairing() async
    /// Dismisses a finished (done or failed) pairing from the state.
    func finishPairing() async
    func unpair(macId: String) async
    func renameMac(macId: String, alias: String?) async
    /// Removes every pairing and the broker account (the phone is as after a fresh install).
    func forgetAll() async

    // requests
    /// The request was opened in the UI: clears the unread flag and tells the Mac.
    func markSeen(macId: String, requestId: String) async
    /// Sends an answer and returns its client message id. Throws `.macOffline` when the Mac cannot
    /// be reached (nothing is queued for later).
    func submitAnswer(macId: String, requestId: String, action: String, answers: [AnswerItem]) async throws -> String
    func submitConfirm(macId: String, requestId: String, wireIndex: Int?, dismiss: Bool, input: String?) async throws -> String
    /// Sends a failed submission again under the same client message id.
    func retrySubmission(clientMsgId: String) async throws
    /// Forgets a failed submission and makes the request answerable again.
    func discardSubmission(clientMsgId: String) async
    /// Uploads a reply attachment; returns the upload id to put into the answer.
    func upload(macId: String, requestId: String, source: UploadSource, progress: @escaping @Sendable (Double) -> Void) async throws -> String
    /// Downloads an attachment of a request into the cache.
    func download(macId: String, asset: AssetRef, progress: @escaping @Sendable (Double) -> Void) async throws -> URL

    // agents
    /// Follows an Agent (the Mac tells the phone the first time it finishes or fails, and then drops the
    /// follow); at most `Wire.maxFollow` per Mac. Throws `.rejected("limit")` when the list is full. The
    /// app does not use it for a button any more: the Macs follow the tasks the phone starts.
    func setFollow(macId: String, agentKey: String, on: Bool) async throws
    /// Opens or closes the live frames of one Agent. A watch the user keeps open is asked for again
    /// after a reconnect.
    func watch(macId: String, agentKey: String, open: Bool) async
    /// Sends a message to a working Agent. `clientMsgId` makes a retry of a failed attempt idempotent
    /// (pass the same one again). Throws `.rejected(reason)` when the Mac refused it.
    func sendInterject(
        macId: String, agentKey: String, text: String, files: [UploadSource], clientMsgId: String?,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> InterjectOutcome
    /// Withdraws the messages still waiting for an Agent; returns how many are left.
    func clearInterject(macId: String, agentKey: String) async throws -> Int
    /// The messages still waiting for an Agent, with their text (to take them back into the editor).
    func queryInterject(macId: String, agentKey: String) async throws -> InterjectInfo

    // tasks
    /// Asks a Mac for the workspaces and Agents a new task may use (answer lands in the state).
    func loadTaskOptions(macId: String) async
    func loadTaskTodos(macId: String, workspaceId: String) async
    /// Sends a new task; returns its id (the progress is in `EngineState.tasks`).
    func createTask(macId: String, draft: TaskDraft) async throws -> String
    /// Sends a task whose outcome is unknown again, under the same id.
    func retryTask(id: String) async throws
    func dismissTask(id: String) async
    /// Makes this phone the one that receives the Mac's questions now.
    func takeSlot(macId: String) async throws

    // todos
    /// The projects a Mac lists for its todos (`mode`, sync facts, rows).
    func todoProjects(macId: String) async throws -> TodoProjects
    /// The todos of one project.
    func todoList(macId: String, projectId: String) async throws -> ProjectTodos
    /// Applies a write. A refusal comes back as an acknowledgement (with the project's current list); only
    /// the lack of an answer throws, and then the same `operation` can be sent again safely.
    func todoApply(macId: String, operation: TodoOperation) async throws -> TodoAck
    /// Subscribes to (or leaves) the Mac's `todo.changed` nudges; the wish survives reconnects.
    func todoWatch(macId: String, on: Bool) async

    // history
    func history() async -> [HistoryEntry]
    func deleteHistory(id: String) async
    func clearHistory() async

    // push
    func updatePushToken(_ token: Data?) async
}
