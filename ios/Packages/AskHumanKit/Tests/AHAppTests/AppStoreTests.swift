import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

@MainActor
final class AppStoreTests: XCTestCase {
    private var backend: MockBackend!
    private var store: AppStore!
    private var defaults: UserDefaults!

    private func make(
        _ scenario: MockBackend.Scenario = .demo, latency: Double = 0, syncDelay: Double? = nil, cachedInbox: Bool = false
    ) async {
        backend = MockBackend(scenario: scenario, latency: latency, syncDelay: syncDelay, cachedInbox: cachedInbox)
        defaults = UserDefaults(suiteName: "ah-test-" + UUID().uuidString)!
        store = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        store.advanceTiming = .immediate
        await store.start()
        await eventually("first state") { self.store.state.foreground }
    }

    private func item(_ requestId: String) throws -> InboxItem {
        try XCTUnwrap(store.state.inbox.first { $0.requestId == requestId })
    }

    // MARK: inbox lists

    func testTheInboxIsGroupedByProjectOnEachMac() async {
        await make()
        let sections = store.inboxSections()
        XCTAssertEqual(sections.count, 2)
        let home = sections.first { $0.macId == MockData.homeMacId }
        let office = sections.first { $0.macId == MockData.officeMacId }
        XCTAssertEqual(home?.projectName, "HumanInLoop")
        XCTAssertEqual(home?.items.count, 3)
        XCTAssertEqual(office?.projectName, "relay")
        XCTAssertEqual(office?.items.count, 2)
        // Newest first inside a group; the most recently active group comes first.
        XCTAssertEqual(home?.items.map(\.requestId), ["req-permission", "req-migration", "req-next"])
        XCTAssertEqual(sections.first?.id, home?.id)
    }

    func testTheSameProjectOnTwoMacsStaysSeparate() async {
        await make()
        await backend.simulateArrival(macId: MockData.officeMacId)
        await eventually { self.store.state.inbox.count == 6 }
        let sections = store.inboxSections()
        XCTAssertEqual(sections.count, 3)
        XCTAssertEqual(Set(sections.map(\.id)).count, 3)
    }

    func testFiltersNarrowTheInbox() async {
        await make()
        XCTAssertEqual(store.inboxItems(filter: .all).count, 5)
        XCTAssertEqual(store.inboxItems(filter: .mac(MockData.officeMacId)).map(\.requestId).sorted(), ["req-select", "req-stop"])
        XCTAssertEqual(store.inboxItems(filter: .asks).count, 3)
        XCTAssertEqual(store.inboxItems(filter: .confirms).count, 2)
        store.filter = .confirms
        XCTAssertEqual(store.inboxItems().count, 2)
    }

    func testCountsAndDrafts() async throws {
        await make()
        XCTAssertEqual(store.pendingCount, 5)
        XCTAssertEqual(store.unreadCount, 2)
        let migration = try item("req-migration")
        XCTAssertFalse(store.hasDraft(for: migration))
        let composer = try XCTUnwrap(store.composer(for: migration))
        composer.toggle(question: 0, option: 0)
        XCTAssertTrue(store.hasDraft(for: migration))
        XCTAssertTrue(store.composer(for: migration) === composer, "the same form is reused while the request is around")
    }

    func testNextPendingPrefersTheOldestUnreadRequest() async throws {
        await make()
        let next = store.nextPending(after: try item("req-permission").id)
        XCTAssertEqual(next?.requestId, "req-migration")
        XCTAssertNil(store.nextPending(after: nil) == nil ? "" : nil, "there is always something while requests are pending")
        // Without unread requests the newest one is offered.
        for pending in store.state.inbox { await backend.markSeen(macId: pending.macId, requestId: pending.requestId) }
        await eventually { self.store.unreadCount == 0 }
        XCTAssertEqual(store.nextPending(after: try item("req-permission").id)?.requestId, "req-migration")
    }

    func testOpeningMarksTheRequestSeen() async throws {
        await make()
        let migration = try item("req-migration")
        XCTAssertTrue(migration.unread)
        await store.opened(migration)
        await eventually { self.store.unreadCount == 1 }
        let seen = await backend.seen
        XCTAssertEqual(seen, ["req-migration"])
    }

    // MARK: answering

    func testSendingAnAnswerMovesTheRequestIntoTheHistory() async throws {
        await make()
        let migration = try item("req-migration")
        let composer = try XCTUnwrap(store.composer(for: migration))
        composer.toggle(question: 0, option: 0)
        composer.setText("watch the replica lag", question: 2)
        try await store.send(composer, item: migration)

        await eventually("request leaves the inbox") { self.store.state.inbox.contains { $0.requestId == "req-migration" } == false }
        await eventually("history updated") { self.store.history.first?.id == migration.id }
        let submissions = await backend.submissions
        XCTAssertEqual(submissions.count, 1)
        XCTAssertEqual(submissions[0].action, "send")
        XCTAssertEqual(submissions[0].answers, [AnswerItem(q: 0, sel: [0]), AnswerItem(q: 2, text: "watch the replica lag")])
        XCTAssertEqual(store.history.first?.outcome, .answered)
        XCTAssertFalse(composer.isSending)
    }

    func testAttachmentsAreUploadedBeforeTheAnswer() async throws {
        await make()
        let migration = try item("req-migration")
        let composer = try XCTUnwrap(store.composer(for: migration))
        try composer.addAttachment(name: "a.png", mime: "image/png", data: Data([1, 2]), isImage: true, question: 2)
        try composer.addAttachment(name: "b.txt", mime: "text/plain", data: Data([3]), isImage: false, question: 2)
        try await store.send(composer, item: migration)

        let uploads = await backend.uploads
        XCTAssertEqual(uploads.map(\.name), ["a.png", "b.txt"])
        let answer = await backend.submissions.first?.answers.first
        XCTAssertEqual(answer?.q, 2)
        XCTAssertEqual(answer?.images.count, 1)
        XCTAssertEqual(answer?.files.count, 1)
        XCTAssertNotEqual(answer?.images, answer?.files)
    }

    func testCancellingSendsTheCancelAction() async throws {
        await make()
        let migration = try item("req-migration")
        try await store.cancel(migration)
        await eventually { self.store.history.first?.outcome == .cancelled }
        let action = await backend.submissions.first?.action
        XCTAssertEqual(action, "cancel")
    }

    func testAnUnreachableMacRejectsTheAnswerAndNothingIsQueued() async throws {
        await make()
        await backend.setUnreachable([MockData.homeMacId])
        let migration = try item("req-migration")
        let composer = try XCTUnwrap(store.composer(for: migration))
        do {
            try await store.send(composer, item: migration)
            XCTFail("sending to an unreachable Mac must throw")
        } catch {
            XCTAssertEqual(error as? BackendError, .macOffline)
        }
        XCTAssertFalse(composer.isSending, "the form is usable again")
        let submissions = await backend.submissions
        XCTAssertTrue(submissions.isEmpty)
    }

    func testAFailedSubmissionCanBeRetriedOrDiscarded() async throws {
        await make()
        await backend.setFailNextSubmission(true)
        let select = try item("req-select")
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: select)
        await eventually("failed state shows") {
            if case .failed = self.store.item(id: select.id)?.state { return true }
            return false
        }
        // Discarding makes it answerable again.
        await store.discardFailure(try item("req-select"))
        await eventually { self.store.item(id: select.id)?.state == .pending }

        await backend.setFailNextSubmission(true)
        try await store.send(composer, item: try item("req-select"))
        await eventually { if case .failed = self.store.item(id: select.id)?.state { return true } else { return false } }
        try await store.retry(try item("req-select"))
        await eventually("retry goes through") { self.store.state.inbox.contains { $0.requestId == "req-select" } == false }
    }

    // MARK: confirming

    func testConfirmDecisionsCarryTheChoiceAndInput() async throws {
        await make()
        let permission = try item("req-permission")
        let composer = try XCTUnwrap(store.confirmComposer(for: permission))
        composer.select(wireIndex: 2)
        composer.input = "run the tests first"
        try await store.decide(composer, item: permission)

        await eventually { self.store.history.first?.id == permission.id }
        let submission = await backend.submissions.first
        XCTAssertEqual(submission?.wireIndex, 2)
        XCTAssertEqual(submission?.input, "run the tests first")
        XCTAssertEqual(submission?.dismiss, false)
        XCTAssertEqual(store.history.first?.confirmChoice, "拒绝")
    }

    func testDismissingAConfirmationSendsDismiss() async throws {
        await make()
        let stop = try item("req-stop")
        let composer = try XCTUnwrap(store.confirmComposer(for: stop))
        try await store.dismiss(composer, item: stop)
        await eventually { self.store.history.first?.outcome == .dismissed }
        let submission = await backend.submissions.first
        XCTAssertEqual(submission?.dismiss, true)
        XCTAssertNil(submission?.wireIndex)
    }

    func testComposersMatchTheKindOfTheRequest() async throws {
        await make()
        XCTAssertNil(store.composer(for: try item("req-permission")))
        XCTAssertNil(store.confirmComposer(for: try item("req-migration")))
    }

    // MARK: sync status

    private func engineState(_ phase: ConnectionPhase, _ links: [MacLink]) -> EngineState {
        var macs = Array(MockData.macs().prefix(links.count))
        for index in macs.indices { macs[index].link = links[index] }
        return EngineState(phase: phase, foreground: true, macs: macs)
    }

    func testSyncStatusFollowsTheBrokerAndTheMacs() {
        let connected = ConnectionPhase.connected(via: "tcp")
        XCTAssertEqual(engineState(connected, [.online, .online]).syncStatus, .upToDate)
        XCTAssertEqual(engineState(connected, [.online, .connecting]).syncStatus, .updating)
        XCTAssertEqual(engineState(connected, [.online, .macOffline]).syncStatus, .upToDate, "an offline Mac is an answer")
        XCTAssertEqual(engineState(.connecting(via: "tcp"), [.connecting]).syncStatus, .updating)
        XCTAssertEqual(engineState(.idle, [.connecting]).syncStatus, .updating)
        XCTAssertEqual(engineState(.backoff(retryAt: Date(), reason: "network"), [.brokerDown]).syncStatus, .reconnecting)
        XCTAssertEqual(engineState(.refused("no"), [.brokerDown]).syncStatus, .refused)
        XCTAssertEqual(engineState(.idle, []).syncStatus, .upToDate, "nothing paired, nothing to wait for")
    }

    func testAnEmptyListIsOnlyTrustedOnceTheMacsAnswered() {
        let connected = ConnectionPhase.connected(via: "tcp")
        XCTAssertEqual(engineState(connected, [.online]).listReadiness, .ready)
        XCTAssertEqual(engineState(connected, [.online, .connecting]).listReadiness, .loading, "one Mac is still on its way")
        XCTAssertEqual(engineState(connected, [.online, .macOffline]).listReadiness, .ready)
        XCTAssertEqual(engineState(connected, [.macOffline, .macOffline]).listReadiness, .unreachable)
        XCTAssertEqual(engineState(.backoff(retryAt: Date(), reason: "network"), [.brokerDown, .brokerDown]).listReadiness, .unreachable)
        XCTAssertEqual(engineState(.connecting(via: "tcp"), [.connecting]).listReadiness, .loading)
        XCTAssertEqual(engineState(.idle, []).listReadiness, .ready)
    }

    func testAConnectedDemoIsUpToDate() async {
        await make()
        XCTAssertEqual(store.syncStatus, .upToDate)
        XCTAssertEqual(store.listReadiness, .ready)
    }

    func testFreshlyOpenedListsAreLoadingNotEmpty() async {
        await make(.syncing)
        XCTAssertTrue(store.state.inbox.isEmpty)
        XCTAssertTrue(store.state.agents.isEmpty)
        XCTAssertEqual(store.syncStatus, .updating)
        XCTAssertEqual(store.listReadiness, .loading, "an empty list that has not heard from the Macs says nothing")
        await backend.finishSync()
        await eventually("synced") { self.store.syncStatus == .upToDate }
        XCTAssertEqual(store.listReadiness, .ready)
        XCTAssertEqual(store.state.inbox.count, 5)
    }

    func testRequestsKeptFromTheLastRunShowWhileTheMacsAreStillAnswering() async {
        await make(.syncing, cachedInbox: true)
        XCTAssertEqual(store.state.inbox.count, 5)
        XCTAssertEqual(store.syncStatus, .updating)
        XCTAssertEqual(store.listReadiness, .loading)
    }

    func testADeadBrokerIsAnAnswerNotALoad() async {
        await make(.offline)
        XCTAssertEqual(store.syncStatus, .reconnecting)
        XCTAssertEqual(store.listReadiness, .unreachable)
    }

    func testNothingPairedHasNothingToWaitFor() async {
        await make(.empty)
        XCTAssertEqual(store.syncStatus, .upToDate)
        XCTAssertEqual(store.listReadiness, .ready)
    }

    func testPullToRefreshStaysUpWhileTheListsCatchUp() async {
        await make(.syncing, syncDelay: 1.5)
        let began = Date()
        await store.refresh()
        XCTAssertGreaterThanOrEqual(Date().timeIntervalSince(began), 1.2, "the spinner waits for the Macs")
        XCTAssertEqual(store.syncStatus, .upToDate)
    }

    func testPullToRefreshNeverVanishesBeforeItWasSeen() async {
        await make()
        let began = Date()
        await store.refresh()
        XCTAssertGreaterThanOrEqual(Date().timeIntervalSince(began), 0.55)
    }

    // MARK: a request that is not here (yet)

    func testARequestThatIsNotHereYetIsLoadingNotUnavailable() async {
        await make(.syncing)
        let id = CachedRequest.key(MockData.homeMacId, "req-migration")
        XCTAssertNil(store.displayItem(id: id))
        XCTAssertEqual(store.missingRequest(id: id), .loading, "the Macs have not answered: it may be on its way")
        await backend.finishSync()
        await eventually("synced") { self.store.syncStatus == .upToDate }
        XCTAssertNotNil(store.displayItem(id: id), "and there it is")
        XCTAssertEqual(
            store.missingRequest(id: CachedRequest.key(MockData.homeMacId, "req-nope")), .gone,
            "the Mac has answered and does not have it"
        )
    }

    func testNothingIsSaidAboutARequestBeforeTheFirstState() {
        let idle = AppStore(
            backend: MockBackend(), drafts: MemoryDraftStore(), defaults: UserDefaults(suiteName: "ah-test-" + UUID().uuidString)!
        )
        XCTAssertFalse(idle.hasState)
        XCTAssertEqual(idle.missingRequest(id: CachedRequest.key(MockData.homeMacId, "req-migration")), .loading)
    }

    func testARequestOfAMacThatCannotBeReachedIsNeitherLoadingNorGone() async {
        await make(.offline)
        XCTAssertEqual(store.missingRequest(id: CachedRequest.key(MockData.homeMacId, "req-late")), .unreachable)
    }

    func testARequestOfAMacThatIsNotPairedIsGone() async {
        await make()
        XCTAssertEqual(store.missingRequest(id: CachedRequest.key("unknown-mac", "req-x")), .gone)
    }

    func testATappedNotificationKeepsWhatItSaidUntilItsPageCloses() async {
        await make(.syncing)
        let hint = NotificationHint(title: "Claude Code · relay", subtitle: "Office iMac", body: "Pin the matrix?")
        store.openNotification(macId: MockData.officeMacId, rid: "req-select", kind: "ask", hint: hint)
        let id = CachedRequest.key(MockData.officeMacId, "req-select")
        XCTAssertEqual(store.inboxPath, [id], "the page opens at once, request or not")
        XCTAssertEqual(store.hint(for: id), hint)
        store.closed(id: id)
        XCTAssertNil(store.hint(for: id))
    }

    // MARK: data allowance

    func testTheDemoIsFarFromTheFreeAllowanceAndTheQuotaScenarioIsNearIt() async {
        await make()
        XCTAssertEqual(store.state.traffic?.level, .ok)
        await make(.quota)
        XCTAssertEqual(store.state.traffic?.level, .warning)
    }

    // MARK: lifecycle of open requests

    func testAnOpenRequestThatFinishesElsewhereIsRemembered() async throws {
        await make()
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        XCTAssertEqual(store.inboxPath, [select.id])
        XCTAssertEqual(store.tab, .inbox)

        // Another phone answers it while the page is open. Not our own submission: that makes the app
        // move on and forget the notice, depending on whether the state update or the event comes first.
        await backend.simulateFinishedElsewhere(requestId: select.requestId, macId: select.macId, by: "Another phone")
        await eventually("finish noted") { self.store.finished[select.id] != nil }
        XCTAssertEqual(store.finished[select.id]?.outcome, .answered)
        XCTAssertEqual(store.finished[select.id]?.by, "Another phone")
        XCTAssertEqual(store.endedNotice(for: select.id)?.by, "Another phone", "its page says so")
        XCTAssertEqual(store.inboxPath, [select.id], "the page is not closed under the reader")

        // The state update and the event are separate streams: wait for the request to be gone as well.
        await eventually("request left the inbox") { self.store.item(id: select.id) == nil }
        store.closed(id: select.id)
        XCTAssertNil(store.finished[select.id], "closing the page forgets the notice")
    }

    func testAfterOurAnswerTheNextPendingRequestOpens() async throws {
        await make()
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: select)
        await eventually("moved on") { self.store.inboxPath.last != select.id && self.store.inboxPath.count == 1 }
        let next = try XCTUnwrap(store.inboxPath.last)
        XCTAssertNotEqual(next, select.id)
        XCTAssertEqual(store.item(id: next)?.state, .pending)
    }

    func testTheNextRequestIsPushedOnTopOfTheSentOneBeforeThatIsDropped() async throws {
        await make()
        // Slow enough to look at each step.
        store.advanceTiming = AdvanceTiming(hold: .milliseconds(150), settle: .milliseconds(150), note: .milliseconds(400))
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        let ticks = store.sentTick
        try await store.send(composer, item: select)

        await eventually("receipt") { self.store.receipt?.from == select.id }
        XCTAssertEqual(store.receipt?.outcome, .answered)
        XCTAssertEqual(store.sentTick, ticks + 1, "the haptic comes with the receipt, not with the tap")
        XCTAssertEqual(store.inboxPath, [select.id], "the sent page stays up while its receipt shows")

        await eventually("pushed") { self.store.inboxPath.count == 2 }
        XCTAssertEqual(store.inboxPath.first, select.id, "the next page slides in over the sent one")
        let next = try XCTUnwrap(store.inboxPath.last)
        XCTAssertEqual(store.sentNote?.remaining, store.pendingCount)
        XCTAssertNotNil(store.receipt, "the sent page still shows its receipt while it slides away")

        await eventually("dropped") { self.store.inboxPath == [next] }
        await eventually("receipt gone") { self.store.receipt == nil }
        await eventually("note gone") { self.store.sentNote == nil }
    }

    func testTheFormUnderOurOwnReceiptDoesNotSayTheRequestEnded() async throws {
        await make()
        store.advanceTiming = AdvanceTiming(hold: .milliseconds(300), settle: .milliseconds(100), note: .milliseconds(100))
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: select)

        // The request has finished, but the page answers with its receipt. While that fades in over the
        // form, the form must keep saying "waiting for your Mac", not flip to "This request has ended."
        await eventually("receipt") { self.store.receipt?.from == select.id }
        XCTAssertNotNil(store.finished[select.id], "the request did finish")
        XCTAssertNil(store.endedNotice(for: select.id), "but our own submission is not an ended page")
    }

    func testLeavingThePageDuringTheReceiptCancelsTheMove() async throws {
        await make()
        store.advanceTiming = AdvanceTiming(hold: .milliseconds(200), settle: .milliseconds(50), note: .milliseconds(50))
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: select)
        await eventually("receipt") { self.store.receipt != nil }

        let other = try item("req-migration")
        store.open(macId: other.macId, requestId: other.requestId)
        XCTAssertNil(store.receipt, "going elsewhere on purpose ends the show")
        try await Task.sleep(for: .milliseconds(350))
        XCTAssertEqual(store.inboxPath, [other.id], "the pending move must not drag the reader away again")
        XCTAssertNil(store.sentNote)
    }

    func testWithAutoAdvanceOffTheListComesBack() async throws {
        await make()
        store.autoAdvance = false
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: select)
        await eventually("popped") { self.store.inboxPath.isEmpty }
    }

    func testAnEndedPageKeepsShowingTheRequestWithANotice() async throws {
        await make()
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        // Not our own submission (that makes the app move on): another phone answers it.
        await backend.simulateFinishedElsewhere(requestId: select.requestId, macId: select.macId)
        await eventually("request left the inbox") { self.store.item(id: select.id) == nil }
        await eventually("finish noted") { self.store.finished[select.id] != nil }
        XCTAssertEqual(store.displayItem(id: select.id)?.requestId, "req-select", "the page still has its request")
    }

    func testClosingAFormSavesTheDraftButKeepsItWhileThePendingRequestExists() async throws {
        await make()
        let migration = try item("req-migration")
        let composer = try XCTUnwrap(store.composer(for: migration))
        composer.setText("draft", question: 2)
        store.closed(id: migration.id)
        XCTAssertEqual(store.drafts.draft(key: migration.id)?.answers[2].text, "draft")
    }

    func testANewRequestRaisesABannerAndAHapticTick() async throws {
        await make()
        let before = store.arrivalTick
        await backend.simulateArrival()
        await eventually("tick") { self.store.arrivalTick == before + 1 }
        guard case .request(let macId, _)? = store.banner?.kind else { return XCTFail("expected a request banner") }
        XCTAssertEqual(macId, MockData.homeMacId)
        XCTAssertEqual(store.banner?.title, "Cursor · HumanInLoop")
    }

    func testTheBannerOfARequestNamesItsSession() async throws {
        await make()
        let before = store.arrivalTick
        await backend.simulateArrival(sessionTitle: "Review the release notes")
        await eventually("tick") { self.store.arrivalTick == before + 1 }
        XCTAssertEqual(store.banner?.title, "Review the release notes", "the title replaces the Agent and project")
        XCTAssertEqual(store.banner?.subtitle, "测试已通过，需要我提交这些改动吗？")
    }

    func testTheInboxNamesARequestBySessionTitleElseByAgent() async throws {
        await make()
        let titled = try item("req-migration")
        XCTAssertEqual(titled.sessionTitle, "Plan the users table migration")
        XCTAssertEqual(titled.displayTitle, "Plan the users table migration")
        let plain = try item("req-select")
        XCTAssertNil(plain.sessionTitle)
        XCTAssertEqual(plain.displayTitle, "Claude Code")
        // What the engine keeps for the list wins nothing over the full view: they say the same.
        XCTAssertEqual(titled.summary.sessionTitle, titled.view?.sessionTitle)
    }

    func testNoBannerWhileTheAppIsInTheBackground() async throws {
        await make()
        await store.setForeground(false)
        await eventually { self.store.state.foreground == false }
        let before = store.arrivalTick
        await backend.simulateArrival()
        await eventually { self.store.state.inbox.count == 6 }
        XCTAssertEqual(store.arrivalTick, before)
        XCTAssertNil(store.banner)
    }

    func testTheBannerDisappearsWhenItsRequestIsAnsweredElsewhere() async throws {
        await make()
        await backend.simulateArrival()
        await eventually { self.store.banner != nil }
        let arrived = try XCTUnwrap(store.state.inbox.first { $0.requestId.hasPrefix("req-live") })
        let composer = try XCTUnwrap(store.composer(for: arrived))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: arrived)
        await eventually("banner cleared") { self.store.banner == nil }
    }

    // MARK: Macs and pairing

    func testPairingWalksThroughTheStages() async throws {
        await make(.empty)
        XCTAssertFalse(store.hasPairedMacs)
        await store.startPairing(code: "ahpair1.eyJ9")
        await eventually(timeout: 5, "done") {
            if case .done? = self.store.state.pairing?.stage { return true }
            return false
        }
        XCTAssertEqual(store.state.pairing?.sas, "428 913")
        XCTAssertEqual(store.macs.count, 1)
        await store.finishPairing()
        await eventually { self.store.state.pairing == nil }
    }

    func testAGarbledPairingCodeFailsRightAway() async {
        await make(.empty)
        await store.startPairing(code: "hello")
        await eventually { self.store.state.pairing?.stage == .failed(.invalidCode) }
        XCTAssertFalse(store.hasPairedMacs)
    }

    func testCancellingPairingClearsTheState() async {
        await make(.empty, latency: 0.05)
        await store.startPairing(code: "ahpair1.eyJ9")
        await eventually { self.store.state.pairing != nil }
        await store.cancelPairing()
        await eventually { self.store.state.pairing == nil }
        try? await Task.sleep(for: .milliseconds(500))
        XCTAssertFalse(store.hasPairedMacs, "a cancelled pairing never completes")
        XCTAssertNil(store.state.pairing)
    }

    func testUnpairingRemovesTheMacAndItsRequests() async throws {
        await make()
        let office = try XCTUnwrap(store.macs.first { $0.id == MockData.officeMacId })
        await store.unpair(office)
        await eventually { self.store.macs.count == 1 }
        XCTAssertEqual(store.state.inbox.count, 3)
        // The banner comes from the event that follows the state update.
        await eventually("warning banner") { self.store.banner?.kind == .warning }
    }

    func testRenamingAMacShowsTheAliasInTheInbox() async throws {
        await make()
        let home = try XCTUnwrap(store.macs.first { $0.id == MockData.homeMacId })
        await store.rename(home, to: "Desk")
        await eventually { self.store.macs.first { $0.id == MockData.homeMacId }?.name == "Desk" }
        XCTAssertEqual(try item("req-migration").macName, "Desk")
        await store.rename(try XCTUnwrap(store.macs.first { $0.id == MockData.homeMacId }), to: "  ")
        await eventually { self.store.macs.first { $0.id == MockData.homeMacId }?.name == "Home MacBook Pro" }
    }

    // MARK: history and settings

    func testHistoryCanBeDeletedAndCleared() async throws {
        await make()
        await eventually { self.store.history.count == 2 }
        let first = try XCTUnwrap(store.history.first)
        await store.deleteHistory(id: first.id)
        await eventually { self.store.history.count == 1 }
        await store.clearHistory()
        await eventually { self.store.history.isEmpty }
    }

    func testAutoAdvanceIsRemembered() async {
        await make()
        XCTAssertTrue(store.autoAdvance)
        store.autoAdvance = false
        let again = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        XCTAssertFalse(again.autoAdvance)
    }
}
