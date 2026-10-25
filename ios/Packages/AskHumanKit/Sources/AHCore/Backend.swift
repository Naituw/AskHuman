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
    /// Pull to refresh: reconnects when the link is down and asks every reachable Mac for an
    /// authoritative inbox snapshot.
    func refresh() async

    // pairing
    func startPairing(code: String) async
    func cancelPairing() async
    /// Dismisses a finished (done or failed) pairing from the state.
    func finishPairing() async
    func unpair(macId: String) async
    func renameMac(macId: String, alias: String?) async

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

    // history
    func history() async -> [HistoryEntry]
    func deleteHistory(id: String) async
    func clearHistory() async

    // push
    func updatePushToken(_ token: Data?) async
}
