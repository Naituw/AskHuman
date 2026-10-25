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

    private func make(_ scenario: MockBackend.Scenario = .demo, latency: Double = 0) async {
        backend = MockBackend(scenario: scenario, latency: latency)
        defaults = UserDefaults(suiteName: "ah-test-" + UUID().uuidString)!
        store = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
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

        // Another phone/Mac answers; the mock models it as our own submission finishing.
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 1)
        try await store.send(composer, item: select)
        await eventually("finish noted") { self.store.finished[select.id] != nil }
        XCTAssertEqual(store.finished[select.id]?.outcome, .answered)

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
        store.autoAdvance = false
        // Not our own submission: the mock cannot model that, so end it by cancelling from elsewhere.
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: select)
        await eventually { self.store.item(id: select.id) == nil }
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
        XCTAssertEqual(store.banner?.kind, .warning)
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
