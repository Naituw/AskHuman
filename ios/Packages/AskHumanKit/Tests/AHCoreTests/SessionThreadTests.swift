import XCTest
@testable import AHProtocol
@testable import AHCore

/// The thread of earlier exchanges (docs/specs/ios-session-history.md §7.2): paging with the cursor, order, one request
/// at a time, and what a failure or a cancellation leaves behind.
@MainActor
final class SessionThreadTests: XCTestCase {
    /// The Mac, scripted by the test: it pages `held` with the real cursor, unless a page or an error is queued.
    private final class Script: @unchecked Sendable {
        struct Call: Equatable {
            var ms: Int64
            var id: String?
            var limit: Int
        }

        private let lock = NSLock()
        private let held: [SessionExchange]
        private var recorded: [Call] = []
        private var errors: [Error] = []
        private var pages: [SessionHistory] = []
        private var holding = false
        private var waiting: [CheckedContinuation<Void, Never>] = []

        init(held: [SessionExchange]) { self.held = held }

        var calls: [Call] { lock.withLock { recorded } }
        func fail(with error: Error) { lock.withLock { errors.append(error) } }
        func answer(with page: SessionHistory) { lock.withLock { pages.append(page) } }
        func hold() { lock.withLock { holding = true } }

        func release() {
            let blocked = lock.withLock { () -> [CheckedContinuation<Void, Never>] in
                holding = false
                defer { waiting = [] }
                return waiting
            }
            blocked.forEach { $0.resume() }
        }

        func load(ms: Int64, id: String?, limit: Int) async throws -> SessionHistory {
            let (error, page, mustWait) = lock.withLock { () -> (Error?, SessionHistory?, Bool) in
                recorded.append(Call(ms: ms, id: id, limit: limit))
                return (errors.isEmpty ? nil : errors.removeFirst(), pages.isEmpty ? nil : pages.removeFirst(), holding)
            }
            if mustWait {
                await withCheckedContinuation { continuation in
                    lock.withLock { waiting.append(continuation) }
                }
            }
            if let error { throw error }
            if let page { return page }
            let get = SessionHistoryGet(reqId: "t", agentKey: "k", beforeMs: ms, beforeId: id, limit: limit)
            return TestMac.page(for: get, in: held)
        }
    }

    /// Every exchange of `TestMac.exchanges` is older than this.
    private let anchor: Int64 = 1_800_000_001_000

    private func thread(_ script: Script, pageSize: Int = 8) -> SessionThread {
        SessionThread(anchorMs: anchor, pageSize: pageSize) { ms, id, limit in
            try await script.load(ms: ms, id: id, limit: limit)
        }
    }

    private func ids(_ thread: SessionThread) -> [String] { thread.items.map(\.id) }

    // MARK: paging

    func testTheFirstPageStartsAtTheRequestAndIsShownOldestFirst() async {
        let script = Script(held: TestMac.exchanges(12))
        let thread = thread(script)
        XCTAssertEqual(thread.phase, .idle)
        XCTAssertFalse(thread.wasOpened)

        let added = await thread.loadMore()
        XCTAssertTrue(added)
        XCTAssertEqual(ids(thread), (5...12).map { "e\($0)" }, "a page comes newest first and the thread reads oldest first")
        XCTAssertEqual(thread.phase, .ready)
        XCTAssertFalse(thread.reachedStart)
        XCTAssertTrue(thread.wasOpened)
        XCTAssertEqual(script.calls, [Script.Call(ms: anchor, id: nil, limit: 8)])
    }

    func testTheNextPageIsPutInFrontAndStartsAtTheOldestExchangeShown() async {
        let script = Script(held: TestMac.exchanges(12))
        let thread = thread(script)
        await thread.loadMore()
        let oldest = thread.items[0]
        await thread.loadMore()

        XCTAssertEqual(ids(thread), (1...12).map { "e\($0)" })
        XCTAssertTrue(thread.reachedStart, "the Mac said there is nothing older")
        XCTAssertEqual(script.calls[1], Script.Call(ms: oldest.at, id: oldest.id, limit: 8))
    }

    func testOnceTheStartIsReachedNothingIsAskedAgain() async {
        let script = Script(held: TestMac.exchanges(3))
        let thread = thread(script)
        await thread.loadMore()
        XCTAssertTrue(thread.reachedStart)

        let added = await thread.loadMore()
        XCTAssertFalse(added)
        XCTAssertEqual(script.calls.count, 1)
    }

    func testASessionWithoutRecordsIsReadyAndAtItsStart() async {
        let script = Script(held: [])
        let thread = thread(script)
        let added = await thread.loadMore()
        XCTAssertFalse(added)
        XCTAssertTrue(thread.items.isEmpty)
        XCTAssertEqual(thread.phase, .ready)
        XCTAssertTrue(thread.reachedStart, "the page says there is nothing earlier")
        XCTAssertTrue(thread.wasOpened)
    }

    func testAnswersInTheSameMillisecondAreNeitherSkippedNorRepeated() async {
        let same = ["e", "d", "c", "b", "a"].map {
            SessionExchange(id: $0, at: 5_000, channel: "popup", message: "m", qs: [])
        }
        let script = Script(held: same)
        let thread = thread(script, pageSize: 2)
        while !thread.reachedStart {
            guard await thread.loadMore() else { break }
        }
        XCTAssertEqual(ids(thread), ["a", "b", "c", "d", "e"])
        XCTAssertEqual(script.calls.count, 3)
    }

    func testAnExchangeThatIsShownAlreadyIsNotShownTwice() async {
        let all = TestMac.exchanges(12)
        let script = Script(held: all)
        let thread = thread(script)
        await thread.loadMore()
        let again = thread.items[0]
        XCTAssertEqual(again.id, "e5")
        // A page that repeats the oldest exchange shown, with one older than it (e4) after it.
        script.answer(with: SessionHistory(reqId: "t", agentKey: "k", items: [again, all[8]], more: false))
        let added = await thread.loadMore()
        XCTAssertTrue(added)
        XCTAssertEqual(thread.items.filter { $0.id == again.id }.count, 1)
        XCTAssertEqual(ids(thread), (4...12).map { "e\($0)" })
    }

    // MARK: one at a time

    func testAnAskWhileOneIsInFlightIsIgnored() async {
        let script = Script(held: TestMac.exchanges(12))
        script.hold()
        let thread = thread(script)
        let first = Task { await thread.loadMore() }
        while script.calls.isEmpty { try? await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(thread.phase, .loading)

        let ignored = await thread.loadMore()
        XCTAssertFalse(ignored)
        XCTAssertEqual(script.calls.count, 1, "only one request is out")
        XCTAssertEqual(thread.intent, 1, "an ignored ask is not an intent")

        script.release()
        let added = await first.value
        XCTAssertTrue(added)
        XCTAssertEqual(thread.items.count, 8)
        XCTAssertEqual(script.calls.count, 1)
    }

    func testTheShowPrepareStepRunsWhenThePageIsInHandAndBeforeItIsShown() async {
        let script = Script(held: TestMac.exchanges(12))
        let thread = thread(script)
        var seen: (count: Int, phase: SessionThread.Phase)?
        await thread.loadMore(beforeInsert: { seen = (thread.items.count, thread.phase) })
        XCTAssertEqual(seen?.count, 0, "the page is not shown yet")
        XCTAssertEqual(seen?.phase, .loading)
        XCTAssertEqual(thread.items.count, 8)
    }

    // MARK: failing

    func testAFailureKeepsWhatIsShownAndAskingAgainStartsFromTheSameCursor() async {
        let script = Script(held: TestMac.exchanges(12))
        let thread = thread(script)
        await thread.loadMore()
        let before = ids(thread)

        script.fail(with: BackendError.timedOut)
        let failed = await thread.loadMore()
        XCTAssertFalse(failed)
        XCTAssertEqual(thread.phase, .failed(.timedOut))
        XCTAssertEqual(ids(thread), before, "what was loaded stays")
        XCTAssertFalse(thread.reachedStart)

        let retried = await thread.loadMore()
        XCTAssertTrue(retried)
        XCTAssertEqual(thread.phase, .ready)
        XCTAssertEqual(ids(thread).count, 12)
        XCTAssertEqual(script.calls[1], script.calls[2], "the retry asks what the failed call asked")
    }

    func testAFirstPageThatFailsCanBeAskedForAgain() async {
        let script = Script(held: TestMac.exchanges(5))
        let thread = thread(script)
        script.fail(with: BackendError.macOffline)
        await thread.loadMore()
        XCTAssertEqual(thread.phase, .failed(.macOffline))
        XCTAssertTrue(thread.wasOpened)

        await thread.loadMore()
        XCTAssertEqual(ids(thread), (1...5).map { "e\($0)" })
    }

    func testACancelledAskLeavesTheThreadAsItWas() async {
        let script = Script(held: TestMac.exchanges(12))
        let thread = thread(script)
        script.fail(with: CancellationError())
        let added = await thread.loadMore()
        XCTAssertFalse(added)
        XCTAssertEqual(thread.phase, .idle, "nothing was shown, so nothing was opened")
        XCTAssertFalse(thread.wasOpened)

        await thread.loadMore()
        XCTAssertEqual(thread.items.count, 8)
        script.fail(with: BackendError.cancelled)
        await thread.loadMore()
        XCTAssertEqual(thread.phase, .ready, "a later page that was cancelled leaves what is shown")
        XCTAssertEqual(thread.items.count, 8)
    }

    // MARK: intent

    func testEveryAskIsAnIntentAndTheMenuCanCountOneWithoutLoading() async {
        let script = Script(held: TestMac.exchanges(12))
        let thread = thread(script)
        XCTAssertEqual(thread.intent, 0)
        await thread.loadMore()
        XCTAssertEqual(thread.intent, 1)
        thread.asked()
        XCTAssertEqual(thread.intent, 2)
        XCTAssertEqual(script.calls.count, 1)
    }
}
