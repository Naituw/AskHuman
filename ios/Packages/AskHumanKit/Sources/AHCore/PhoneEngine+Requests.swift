import Foundation
import AHProtocol
import AHStore

// Requests: what the Macs send into the inbox, and how answers and confirmations travel back
// (plan §3.4, §3.5). Submissions go through the outbox: persisted, sent under a client message
// id, acknowledged by the Mac, and retried under the same id.

extension PhoneEngine {
    /// How long a submission waits for the Mac's acknowledgement before it is shown as failed.
    static let ackTimeout: TimeInterval = 30

    // MARK: inbound

    func onInboxSnapshot(_ macId: String, _ snapshot: InboxSnapshot) async {
        guard var mac = macs[macId], snapshot.epoch == mac.epoch else { return }
        mac.inboxRev = snapshot.rev
        mac.sync = .synced
        macs[macId] = mac
        let now = clock.now()

        let remote = Dictionary(snapshot.items.map { ($0.id, $0) }, uniquingKeysWith: { _, last in last })
        // Requests that ended while we were away. One we answered stays until the Mac's reply to
        // that answer arrives (its journal knows who won, even for a request that is gone).
        for (key, runtime) in requests where runtime.cached.macId == macId && remote[runtime.cached.requestId] == nil {
            let hasSubmission = outbox.values.contains { $0.macId == macId && $0.subject == runtime.cached.requestId }
            if !hasSubmission { finish(key, outcome: .ended, by: nil) }
        }
        var fetch: [String] = []
        for summary in snapshot.items {
            let key = CachedRequest.key(macId, summary.id)
            if var runtime = requests[key] {
                runtime.cached.summary = summary
                if runtime.cached.view?.contentHash != summary.contentHash { fetch.append(summary.id) }
                requests[key] = runtime
            } else {
                var cached = CachedRequest(macId: macId, requestId: summary.id, summary: summary, receivedAt: now)
                cached.unread = true
                requests[key] = RequestRuntime(cached: cached, state: .pending)
                store.saveRequest(cached)
                emit(.requestArrived(macId: macId, requestId: summary.id))
                fetch.append(summary.id)
            }
        }
        for id in fetch { await fetchFull(macId, requestId: id) }
        // Submissions nobody is waiting for any more (the app was restarted, or the link dropped)
        // go out again under their original client message ids.
        for item in outbox.values.sorted(by: { $0.createdAt < $1.createdAt }) where item.macId == macId && timers["ack." + item.clientMsgId] == nil {
            await transmit(item)
        }
        await ensureForegroundPresence()
    }

    func fetchFull(_ macId: String, requestId: String) async {
        let key = CachedRequest.key(macId, requestId)
        guard requests[key]?.fetching == false else { return }
        requests[key]?.fetching = true
        await send(RequestGet(requestId: requestId), to: macId)
    }

    func onRequestNew(_ macId: String, _ message: RequestNew) async {
        guard var mac = macs[macId], mac.epoch != nil else { return }
        // Deltas apply in order: a repeat is dropped, a gap asks for a fresh snapshot.
        guard let last = mac.inboxRev else {
            await requestResync(macId, stream: "inbox")
            return
        }
        if message.rev <= last { return }
        if message.rev != last + 1 {
            await requestResync(macId, stream: "inbox")
            return
        }
        mac.inboxRev = message.rev
        macs[macId] = mac

        let view = message.request
        let key = CachedRequest.key(macId, view.id)
        let summary = summarize(view)
        if var existing = requests[key] {
            existing.cached.summary = summary
            if message.full { existing.cached.view = view }
            requests[key] = existing
        } else {
            let cached = CachedRequest(
                macId: macId, requestId: view.id, summary: summary, view: message.full ? view : nil, receivedAt: clock.now()
            )
            requests[key] = RequestRuntime(cached: cached, state: .pending)
            emit(.requestArrived(macId: macId, requestId: view.id))
        }
        persistRequest(key)
        if message.full {
            await acknowledge(key)
        } else {
            await fetchFull(macId, requestId: view.id)
        }
    }

    func onRequestFull(_ macId: String, _ message: RequestFull) async {
        let view = message.request
        let key = CachedRequest.key(macId, view.id)
        guard var runtime = requests[key] else { return }
        runtime.fetching = false
        runtime.cached.view = view
        runtime.cached.summary = summarize(view, size: runtime.cached.summary.size)
        requests[key] = runtime
        persistRequest(key)
        await acknowledge(key)
    }

    func onRequestResolved(_ macId: String, _ message: RequestResolved) async {
        guard var mac = macs[macId], mac.epoch != nil else { return }
        guard let last = mac.inboxRev else {
            await requestResync(macId, stream: "inbox")
            return
        }
        if message.rev <= last { return }
        if message.rev != last + 1 {
            mac.inboxRev = last
            macs[macId] = mac
            await requestResync(macId, stream: "inbox")
            return
        }
        mac.inboxRev = message.rev
        macs[macId] = mac
        let key = CachedRequest.key(macId, message.requestId)
        guard requests[key] != nil else { return }
        let outcome: HistoryOutcome
        switch message.how {
        case "answered": outcome = .answered
        case "cancelled": outcome = .cancelled
        case "expired": outcome = .expired
        case "answeredElsewhere": outcome = .answeredElsewhere
        default: outcome = .ended
        }
        // If our own submission decided it, keep what we sent (the ack may arrive after this).
        var answers: [AnswerItem] = []
        var choice: String?
        var input: String?
        if outcome == .answered, let item = outbox.values.first(where: { $0.macId == macId && $0.subject == message.requestId }) {
            if let submit = try? AHJSON.value(AnswerSubmit.self, from: item.body) { answers = submit.answers }
            if let decide = try? AHJSON.value(ConfirmDecide.self, from: item.body) {
                input = decide.input
                if let index = decide.wireIndex, let confirm = requests[key]?.cached.view?.confirm {
                    choice = confirm.choices.first { $0.wireIndex == index }?.label
                }
            }
        }
        finish(key, outcome: outcome, by: message.by, answers: answers, confirmChoice: choice, confirmInput: input)
    }

    /// Tells the Mac the request is stored on the phone (so no push is needed).
    func acknowledge(_ key: String) async {
        guard var runtime = requests[key], !runtime.cached.acked else { return }
        runtime.cached.acked = true
        requests[key] = runtime
        persistRequest(key)
        await send(RequestAck(requestId: runtime.cached.requestId), to: runtime.cached.macId)
    }

    func persistRequest(_ key: String) {
        guard let runtime = requests[key] else { return }
        store.saveRequest(runtime.cached)
    }

    func summarize(_ view: RequestView, size: UInt64 = 0) -> RequestSummary {
        RequestSummary(
            id: view.id, kind: view.kind, project: view.project, agent: view.agent, sessionTitle: view.sessionTitle,
            createdAt: view.createdAt, preview: Self.preview(of: view), contentHash: view.contentHash, size: size
        )
    }

    static func preview(of view: RequestView) -> String {
        let source: String
        if let ask = view.ask {
            let message = ask.message.text.trimmingCharacters(in: .whitespacesAndNewlines)
            source = message.isEmpty ? (ask.questions.first?.text ?? "") : message
        } else if let confirm = view.confirm {
            source = confirm.detail.summary.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? confirm.title : confirm.detail.summary
        } else {
            source = ""
        }
        let line = source.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? ""
        return line.count > Wire.pushPreviewMaxChars ? String(line.prefix(Wire.pushPreviewMaxChars)) + "…" : line
    }

    // MARK: finishing

    /// Moves a request out of the inbox into the local history.
    func finish(
        _ key: String, outcome: HistoryOutcome, by: String?, answers: [AnswerItem] = [], confirmChoice: String? = nil,
        confirmInput: String? = nil
    ) {
        guard let runtime = requests.removeValue(forKey: key) else { return }
        let cached = runtime.cached
        for item in outbox.values where item.macId == cached.macId && item.subject == cached.requestId {
            outbox[item.clientMsgId] = nil
            store.removeOutbox(item.clientMsgId)
            cancelTimer("ack." + item.clientMsgId)
        }
        store.removeRequest(macId: cached.macId, requestId: cached.requestId)
        store.removeDraft(key: key)
        let name = macs[cached.macId]?.record.displayName ?? ""
        let view = cached.view ?? Self.placeholderView(for: cached.summary)
        store.appendHistory(
            HistoryEntry(
                id: key, macId: cached.macId, macName: name, request: view, outcome: outcome, by: by, answers: answers,
                confirmChoice: confirmChoice, confirmInput: confirmInput, receivedAt: cached.receivedAt, finishedAt: clock.now()
            ),
            now: clock.now()
        )
        historyRevision += 1
        emit(.requestFinished(macId: cached.macId, requestId: cached.requestId, outcome: outcome, by: by))
    }

    static func placeholderView(for summary: RequestSummary) -> RequestView {
        RequestView(
            id: summary.id, kind: summary.kind, createdAt: summary.createdAt, agent: summary.agent, project: summary.project,
            sessionTitle: summary.sessionTitle, source: "", contentHash: summary.contentHash,
            ask: summary.kind == "ask"
                ? AskView(isMarkdown: false, message: MessageView(text: summary.preview, files: []), questions: [], selectOnly: false, single: false)
                : nil
        )
    }

    // MARK: user actions

    public func markSeen(macId: String, requestId: String) async {
        let key = CachedRequest.key(macId, requestId)
        guard var runtime = requests[key] else { return }
        if runtime.cached.unread {
            runtime.cached.unread = false
            requests[key] = runtime
            persistRequest(key)
        }
        if !runtime.seenSent, macs[macId]?.sync == .synced {
            requests[key]?.seenSent = true
            await send(RequestSeen(requestId: requestId), to: macId)
        }
        publish()
    }

    public func submitAnswer(macId: String, requestId: String, action: String, answers: [AnswerItem]) async throws -> String {
        let key = CachedRequest.key(macId, requestId)
        guard let runtime = requests[key], runtime.cached.view?.ask != nil else { throw BackendError.unknownRequest }
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        if case .sending = runtime.state { throw BackendError.invalid("This request is already being sent.") }
        let clientMsgId = Bytes.newMsgId()
        let submit = AnswerSubmit(clientMsgId: clientMsgId, requestId: requestId, action: action, answers: answers)
        let body = try AHJSON.tree(submit)
        return await enqueue(
            OutboxItem(clientMsgId: clientMsgId, macId: macId, kind: .answer, subject: requestId, type: AnswerSubmit.type, body: body, createdAt: clock.now())
        )
    }

    public func submitConfirm(macId: String, requestId: String, wireIndex: Int?, dismiss: Bool, input: String?) async throws -> String {
        let key = CachedRequest.key(macId, requestId)
        guard let runtime = requests[key], runtime.cached.view?.confirm != nil else { throw BackendError.unknownRequest }
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        if case .sending = runtime.state { throw BackendError.invalid("This request is already being sent.") }
        let clientMsgId = Bytes.newMsgId()
        let decide = ConfirmDecide(clientMsgId: clientMsgId, requestId: requestId, wireIndex: wireIndex, dismiss: dismiss, input: input)
        let body = try AHJSON.tree(decide)
        return await enqueue(
            OutboxItem(clientMsgId: clientMsgId, macId: macId, kind: .confirm, subject: requestId, type: ConfirmDecide.type, body: body, createdAt: clock.now())
        )
    }

    func enqueue(_ item: OutboxItem) async -> String {
        outbox[item.clientMsgId] = item
        store.saveOutbox(item)
        if let subject = item.subject {
            let key = CachedRequest.key(item.macId, subject)
            requests[key]?.state = .sending(clientMsgId: item.clientMsgId, since: item.createdAt)
        }
        publish()
        await transmit(item)
        return item.clientMsgId
    }

    /// Sends (or re-sends) an outbox item and starts its acknowledgement timer.
    func transmit(_ item: OutboxItem) async {
        var current = item
        current.attempts += 1
        outbox[item.clientMsgId] = current
        store.saveOutbox(current)
        if let subject = item.subject {
            requests[CachedRequest.key(item.macId, subject)]?.state = .sending(clientMsgId: item.clientMsgId, since: clock.now())
        }
        let plain = Plain(t: item.type, ts: clock.nowMs(), b: item.body)
        let sent = await send(plain, to: item.macId)
        if sent {
            let id = item.clientMsgId
            after("ack." + id, Self.ackTimeout) { engine in await engine.ackTimedOut(id) }
        } else {
            markFailed(item.clientMsgId, reason: "not sent")
        }
        publish()
    }

    func ackTimedOut(_ clientMsgId: String) {
        markFailed(clientMsgId, reason: "timeout")
        publish()
    }

    func markFailed(_ clientMsgId: String, reason: String) {
        guard var item = outbox[clientMsgId] else { return }
        item.failure = reason
        outbox[clientMsgId] = item
        store.saveOutbox(item)
        if let subject = item.subject {
            let key = CachedRequest.key(item.macId, subject)
            if requests[key] != nil { requests[key]?.state = .failed(clientMsgId: clientMsgId, reason: reason) }
        }
    }

    public func retrySubmission(clientMsgId: String) async throws {
        guard let item = outbox[clientMsgId] else { throw BackendError.unknownRequest }
        guard macs[item.macId]?.sync == .synced else { throw BackendError.macOffline }
        await transmit(item)
    }

    public func discardSubmission(clientMsgId: String) async {
        if outbox.removeValue(forKey: clientMsgId) != nil {
            store.removeOutbox(clientMsgId)
            cancelTimer("ack." + clientMsgId)
        }
        // The request goes back to being answerable, whether or not the submission was still queued
        // (a rejected one is already gone from the outbox).
        for (key, runtime) in requests {
            switch runtime.state {
            case .sending(let id, _), .failed(let id, _):
                if id == clientMsgId { requests[key]?.state = .pending }
            case .pending:
                break
            }
        }
        publish()
    }

    // MARK: acknowledgements

    func onAnswerAck(_ macId: String, _ ack: AnswerAck) async {
        guard let item = outbox[ack.clientMsgId], item.macId == macId else { return }
        let submit = try? AHJSON.value(AnswerSubmit.self, from: item.body)
        resolveSubmission(item, status: ack.status, reason: ack.reason, answers: submit?.answers ?? [], cancelled: submit?.action == "cancel")
    }

    func onConfirmAck(_ macId: String, _ ack: ConfirmAck) async {
        guard let item = outbox[ack.clientMsgId], item.macId == macId else { return }
        let decide = try? AHJSON.value(ConfirmDecide.self, from: item.body)
        let key = CachedRequest.key(macId, item.subject ?? "")
        var choice: String?
        if let index = decide?.wireIndex, let confirm = requests[key]?.cached.view?.confirm {
            choice = confirm.choices.first { $0.wireIndex == index }?.label
        }
        resolveSubmission(
            item, status: ack.status, reason: ack.reason, answers: [], cancelled: decide?.dismiss == true,
            confirmChoice: choice, confirmInput: decide?.input
        )
    }

    func resolveSubmission(
        _ item: OutboxItem, status: String, reason: String?, answers: [AnswerItem], cancelled: Bool,
        confirmChoice: String? = nil, confirmInput: String? = nil
    ) {
        cancelTimer("ack." + item.clientMsgId)
        let key = CachedRequest.key(item.macId, item.subject ?? "")
        switch status {
        case "accepted":
            finish(key, outcome: cancelled ? .cancelled : .answered, by: nil, answers: answers, confirmChoice: confirmChoice, confirmInput: confirmInput)
        case "lost":
            finish(key, outcome: .answeredElsewhere, by: nil)
        case "expired":
            finish(key, outcome: .expired, by: nil)
        default:
            outbox[item.clientMsgId] = nil
            store.removeOutbox(item.clientMsgId)
            if requests[key] != nil {
                requests[key]?.state = .failed(clientMsgId: item.clientMsgId, reason: reason ?? status)
            }
        }
    }

    // MARK: history

    public func history() async -> [HistoryEntry] { store.history() }

    public func deleteHistory(id: String) async {
        store.removeHistory(id: id)
        historyRevision += 1
        publish()
    }

    public func clearHistory() async {
        store.clearHistory()
        historyRevision += 1
        publish()
    }
}
