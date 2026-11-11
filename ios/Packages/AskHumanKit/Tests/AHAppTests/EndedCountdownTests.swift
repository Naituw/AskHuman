import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The strip of a request page whose request ended somewhere else: its button, and the countdown that presses it
/// (docs/specs/ios-answer-flow.md §3.8).
@MainActor
final class EndedCountdownTests: XCTestCase {
    private var backend: MockBackend!
    private var store: AppStore!
    private var defaults: UserDefaults!

    /// `length` is long by default: a countdown that must not run out during a test.
    private func make(_ scenario: MockBackend.Scenario = .demo, length: Duration = .seconds(60)) async {
        backend = MockBackend(scenario: scenario, latency: 0)
        defaults = UserDefaults(suiteName: "ah-test-" + UUID().uuidString)!
        store = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        store.advanceTiming = .immediate
        store.endedCountdownLength = length
        await store.start()
        await eventually("first state") { self.store.state.foreground }
    }

    override func tearDown() async throws {
        store?.stopEndedCountdown()
    }

    private func item(_ requestId: String) throws -> InboxItem {
        try XCTUnwrap(store.state.inbox.first { $0.requestId == requestId })
    }

    /// Opens a request and has its Mac report that it ended somewhere else.
    @discardableResult
    private func openAndEnd(_ requestId: String, how: HistoryOutcome = .answeredElsewhere, showNotice: Bool = true) async throws -> InboxItem {
        let item = try item(requestId)
        store.open(macId: item.macId, requestId: item.requestId)
        await backend.simulateFinished(
            requestId: item.requestId, macId: item.macId, outcome: how, by: how == .answeredElsewhere ? "Popup" : nil
        )
        await eventually("ended") { self.store.endedNotice(for: item.id) != nil }
        if showNotice { store.endedNoticeDisplayed(item.id) }
        return item
    }

    // MARK: the button

    func testTheButtonMovesOnWhileAnotherRequestWaits() async throws {
        await make()
        let select = try await openAndEnd("req-select")
        let expected = try XCTUnwrap(store.nextPending(after: select.id))
        XCTAssertEqual(store.endedAction(for: select.id), .next(expected.id), "the same request as after a send: oldest unread, else newest")

        store.performEndedAction(for: select.id)
        XCTAssertEqual(store.inboxPath, [expected.id])
        XCTAssertNil(store.finished[select.id], "the ended page is closed behind the reader")
        XCTAssertNil(store.endedCountdown)
        XCTAssertNil(store.receipt, "nothing was sent: no receipt, no note")
        XCTAssertNil(store.sentNote)
        let neighbours = [store.adjacentRequest(to: expected.id, direction: -1), store.adjacentRequest(to: expected.id, direction: 1)]
        XCTAssertTrue(neighbours.contains { $0 != nil }, "the arrows walk the whole list from the new page")
    }

    func testTheButtonClosesThePageWhenNothingElseWaits() async throws {
        await make(.single)
        let select = try await openAndEnd("req-select")
        XCTAssertEqual(store.endedAction(for: select.id), .close)

        store.performEndedAction(for: select.id)
        XCTAssertTrue(store.inboxPath.isEmpty, "back to the list")
        XCTAssertNil(store.finished[select.id])
        XCTAssertNil(store.endedCountdown)
    }

    func testARequestThatArrivesMeanwhileTurnsCloseIntoNext() async throws {
        await make(.single)
        let select = try await openAndEnd("req-select")
        XCTAssertEqual(store.endedAction(for: select.id), .close)

        await backend.simulateArrival()
        await eventually("arrival in the inbox") { self.store.state.inbox.count == 1 }
        let arrived = try XCTUnwrap(store.state.inbox.first)
        XCTAssertEqual(store.endedAction(for: select.id), .next(arrived.id))
        store.performEndedAction(for: select.id)
        XCTAssertEqual(store.inboxPath, [arrived.id])
    }

    func testTheButtonIsForThePageThatIsUp() async throws {
        await make()
        let select = try await openAndEnd("req-select")
        let other = try item("req-migration")
        store.performEndedAction(for: other.id)
        XCTAssertEqual(store.inboxPath, [select.id], "another request's page is not this strip's to move")
        XCTAssertNotNil(store.endedNotice(for: select.id))
    }

    func testAnOpenRequestHasNoButtonToPress() async throws {
        await make()
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        store.performEndedAction(for: select.id)
        XCTAssertEqual(store.inboxPath, [select.id], "no strip, no way on: the request is still asking")
    }

    func testTheStripsMoveGoesThroughTheAppTransition() async throws {
        await make()
        var moves = 0
        store.animatePageMove = { change in
            moves += 1
            change()
        }
        let select = try await openAndEnd("req-select")
        store.performEndedAction(for: select.id)
        XCTAssertEqual(moves, 1)
        XCTAssertNotEqual(store.inboxPath, [select.id])
    }

    // MARK: the countdown

    func testTheCountdownStartsWhenTheEndedStripIsDisplayed() async throws {
        for how: HistoryOutcome in [.answeredElsewhere, .cancelled, .expired, .ended] {
            await make()
            XCTAssertNil(store.endedCountdown, "nothing counts down while the request is open")
            let select = try await openAndEnd("req-select", how: how)
            let countdown = try XCTUnwrap(store.endedCountdown, "\(how)")
            XCTAssertEqual(countdown.id, select.id)
            XCTAssertEqual(countdown.seconds, 60, accuracy: 0.001)
            XCTAssertEqual(countdown.deadline.timeIntervalSince(countdown.began), 60, accuracy: 0.001)
        }
    }

    func testTheCountdownIsThreeSecondsUnlessToldOtherwise() async throws {
        await make()
        let fresh = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        XCTAssertEqual(fresh.endedCountdownLength, .seconds(3))
    }

    func testAnUnseenStripDoesNotSpendTheCountdown() async throws {
        await make(length: .milliseconds(150))
        let select = try await openAndEnd("req-select", how: .cancelled, showNotice: false)
        try await Task.sleep(for: .milliseconds(350))
        XCTAssertNil(store.endedCountdown)
        XCTAssertEqual(store.inboxPath, [select.id])
        XCTAssertTrue(store.monitorsEndedNotice(select.id))

        store.endedNoticeDisplayed(select.id)
        XCTAssertEqual(store.endedCountdown?.id, select.id)
        await eventually("moves after the visible countdown") { self.store.inboxPath.last != select.id }
    }

    func testAnEndInTheBackgroundWaitsForTheStripToBeVisible() async throws {
        await make(length: .milliseconds(150))
        var visible = false
        var covered = true
        store.pageIsVisible = { visible }
        store.pageIsCovered = { covered }
        let select = try await openAndEnd("req-select", how: .cancelled)
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertNil(store.endedCountdown, "background presentation and editing state cannot consume or suppress the timer")
        XCTAssertEqual(store.inboxPath, [select.id])

        visible = true
        store.endedNoticeDisplayed(select.id)
        XCTAssertNil(store.endedCountdown, "the strip is still behind a cover")
        covered = false
        store.endedNoticeDisplayed(select.id)
        XCTAssertEqual(store.endedCountdown?.id, select.id)
        await eventually("moves after it was seen") { self.store.inboxPath.last != select.id }
    }

    func testReturningToTheForegroundDoesNotResetAnActiveCountdown() async throws {
        await make(length: .milliseconds(300))
        var visible = true
        store.pageIsVisible = { visible }
        let select = try await openAndEnd("req-select")
        let first = try XCTUnwrap(store.endedCountdown)
        visible = false
        await store.setForeground(false)
        try await Task.sleep(for: .milliseconds(100))
        visible = true
        store.endedNoticeDisplayed(select.id)
        store.endedNoticeDisplayed(select.id)
        XCTAssertEqual(store.endedCountdown, first, "returning and repeated layout callbacks preserve the original deadline")
        await eventually("the original deadline wins") { self.store.inboxPath.last != select.id }
    }

    func testGoingToTheBackgroundDoesNotInterruptACountdownAlreadySeen() async throws {
        await make(length: .milliseconds(200))
        var visible = true
        store.pageIsVisible = { visible }
        let select = try await openAndEnd("req-select")
        visible = false
        await store.setForeground(false)
        await eventually("moves while the app is not visible") { self.store.inboxPath.last != select.id }
    }

    func testLeavingAnUnseenEndedPageDisarmsItsCountdown() async throws {
        await make(length: .milliseconds(100))
        let select = try await openAndEnd("req-select", showNotice: false)
        let other = try item("req-migration")
        store.open(macId: other.macId, requestId: other.requestId)
        store.endedNoticeDisplayed(select.id)
        try await Task.sleep(for: .milliseconds(250))
        XCTAssertNil(store.endedCountdown)
        XCTAssertFalse(store.monitorsEndedNotice(select.id))
        XCTAssertEqual(store.inboxPath, [other.id])
    }

    func testWhenTheCountdownRunsOutThePageMovesOn() async throws {
        await make(length: .milliseconds(150))
        let select = try await openAndEnd("req-select")
        let expected = try XCTUnwrap(store.nextPending(after: select.id))
        await eventually("moved on") { self.store.inboxPath.last != select.id }
        XCTAssertEqual(store.inboxPath, [expected.id])
        XCTAssertNil(store.endedCountdown)
        XCTAssertNil(store.finished[select.id])
    }

    func testWhenTheCountdownRunsOutOnTheLastRequestThePageCloses() async throws {
        await make(.single, length: .milliseconds(150))
        try await openAndEnd("req-select")
        await eventually("closed") { self.store.inboxPath.isEmpty }
        XCTAssertNil(store.endedCountdown)
    }

    func testTheCountdownDoesWhatTheButtonSaysWhenItRunsOutNotWhenItBegan() async throws {
        await make(.single, length: .milliseconds(600))
        let select = try await openAndEnd("req-select")
        XCTAssertEqual(store.endedAction(for: select.id), .close, "when it began, nothing else waited")
        await backend.simulateArrival()
        await eventually("arrival in the inbox") { self.store.state.inbox.count == 1 }
        let arrived = try XCTUnwrap(store.state.inbox.first)

        await eventually(timeout: 5, "moved on") { self.store.inboxPath.last != select.id }
        XCTAssertEqual(store.inboxPath, [arrived.id], "the countdown does not close a page with a request waiting")
    }

    func testStayingKeepsThePageAndDropsTheCountdown() async throws {
        await make(length: .milliseconds(250))
        let select = try await openAndEnd("req-select")
        XCTAssertNotNil(store.endedCountdown)

        store.stayOnEnded(select.id)
        store.endedNoticeDisplayed(select.id)
        XCTAssertNil(store.endedCountdown)
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertEqual(store.inboxPath, [select.id], "the page stays for as long as the reader wants")
        XCTAssertNotNil(store.endedNotice(for: select.id), "and still says the request ended")

        // The button is still there.
        store.performEndedAction(for: select.id)
        XCTAssertNotEqual(store.inboxPath, [select.id])
    }

    func testStayingBeforeTheCountdownBeganKeepsItFromBeginning() async throws {
        await make(length: .milliseconds(250))
        let select = try await openAndEnd("req-select", showNotice: false)
        XCTAssertTrue(store.monitorsEndedNotice(select.id), "the countdown waits for its strip")
        XCTAssertNil(store.endedCountdown)

        store.stayOnEnded(select.id)
        XCTAssertFalse(store.monitorsEndedNotice(select.id))
        store.endedNoticeDisplayed(select.id)
        XCTAssertNil(store.endedCountdown, "the strip's showing does not start what the reader already turned down")
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertEqual(store.inboxPath, [select.id])
    }

    func testStayingIsForTheRequestItWasPressedOn() async throws {
        await make()
        let select = try await openAndEnd("req-select")
        store.stayOnEnded(try item("req-migration").id)
        XCTAssertEqual(store.endedCountdown?.id, select.id, "another request's stay does not stop this countdown")
    }

    func testWithAutoAdvanceOffThereIsOnlyTheButton() async throws {
        await make(length: .milliseconds(100))
        store.autoAdvance = false
        let select = try await openAndEnd("req-select")
        XCTAssertNil(store.endedCountdown, "a reader who turned moving on off is not moved on")
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(store.inboxPath, [select.id])
        XCTAssertNotEqual(store.endedAction(for: select.id), .close, "the button is there all the same")
        store.performEndedAction(for: select.id)
        XCTAssertNotEqual(store.inboxPath, [select.id])
    }

    func testAPageThatIsCoveredWhenItsRequestEndsGetsNoCountdown() async throws {
        await make(length: .milliseconds(100))
        store.pageIsCovered = { true }
        let select = try await openAndEnd("req-select")
        XCTAssertNil(store.endedCountdown, "a sheet or the keyboard is over the page: what is written there is not moved away")
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(store.inboxPath, [select.id])
        XCTAssertNotNil(store.endedNotice(for: select.id), "the page still says the request ended")
        XCTAssertNotEqual(store.endedAction(for: select.id), .close, "and the button is there")
        store.performEndedAction(for: select.id)
        XCTAssertNotEqual(store.inboxPath, [select.id])
    }

    func testTheCoverIsLookedAtWhenTheRequestEndsAndNotBefore() async throws {
        await make()
        var looks = 0
        var covered = false
        store.pageIsCovered = {
            looks += 1
            return covered
        }
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        XCTAssertEqual(looks, 0, "an open request has nothing to count down")
        await backend.simulateFinished(requestId: select.requestId, macId: select.macId, outcome: .cancelled)
        await eventually("ended") { self.store.endedNotice(for: select.id) != nil }
        XCTAssertEqual(looks, 1)
        store.endedNoticeDisplayed(select.id)
        XCTAssertEqual(looks, 2, "the visible strip also checks that no new cover is hiding it")
        XCTAssertNotNil(store.endedCountdown, "nothing was over the page")

        // Whatever comes over the page afterwards is the strip's to notice: the store looks once.
        covered = true
        XCTAssertNotNil(store.endedCountdown)
        XCTAssertEqual(looks, 2)
    }

    func testTurningAutoAdvanceOffStopsTheCountdown() async throws {
        await make()
        try await openAndEnd("req-select")
        XCTAssertNotNil(store.endedCountdown)
        store.autoAdvance = false
        XCTAssertNil(store.endedCountdown)
    }

    func testOurOwnAnswerIsNotAnEndingToCountDownOn() async throws {
        await make()
        let select = try item("req-select")
        store.open(macId: select.macId, requestId: select.requestId)
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: select)
        await eventually("moved on") { self.store.inboxPath.last != select.id && self.store.inboxPath.count == 1 }
        XCTAssertNil(store.endedCountdown, "the receipt and the move after a send are the app's own")
    }

    func testLeavingThePageStopsTheCountdown() async throws {
        await make(length: .milliseconds(200))
        let select = try await openAndEnd("req-select")
        XCTAssertNotNil(store.endedCountdown)
        store.inboxPath = []
        XCTAssertNil(store.endedCountdown)
        try await Task.sleep(for: .milliseconds(450))
        XCTAssertTrue(store.inboxPath.isEmpty, "nothing opens by itself over the list afterwards")
        XCTAssertNil(store.finished[select.id])
    }

    func testOpeningAnotherRequestStopsTheCountdown() async throws {
        await make(length: .milliseconds(200))
        try await openAndEnd("req-select")
        let other = try item("req-migration")
        store.open(macId: other.macId, requestId: other.requestId)
        XCTAssertNil(store.endedCountdown)
        try await Task.sleep(for: .milliseconds(450))
        XCTAssertEqual(store.inboxPath, [other.id], "the reader's own choice is not undone")
    }

    func testAnotherTabStopsTheCountdown() async throws {
        await make(length: .milliseconds(200))
        let select = try await openAndEnd("req-select")
        store.tab = .macs
        XCTAssertNil(store.endedCountdown)
        try await Task.sleep(for: .milliseconds(450))
        XCTAssertEqual(store.inboxPath, [select.id], "the page waits where it was")
    }

    func testARequestThatEndsWithoutItsPageOpenCountsNothing() async throws {
        await make()
        let select = try item("req-select")
        await backend.simulateFinished(requestId: select.requestId, macId: select.macId, outcome: .answeredElsewhere, by: "Popup")
        await eventually("left the inbox") { self.store.item(id: select.id) == nil }
        XCTAssertNil(store.endedCountdown)
        XCTAssertTrue(store.inboxPath.isEmpty)
    }

    func testTheCountdownOfAnEndedPageIsNotRestartedByTheNextOneEnding() async throws {
        await make()
        let select = try await openAndEnd("req-select")
        let first = try XCTUnwrap(store.endedCountdown)
        // Some other request ends too, behind the page.
        let other = try item("req-migration")
        await backend.simulateFinished(requestId: other.requestId, macId: other.macId, outcome: .cancelled)
        await eventually("left the inbox") { self.store.item(id: other.id) == nil }
        XCTAssertEqual(store.endedCountdown, first, "a request that is not on the page changes nothing about its countdown")
        XCTAssertEqual(store.endedCountdown?.id, select.id)
    }
}
