import XCTest
import AHCore
import AHMock
import AHStore
import AHProtocol

@MainActor
final class RequestNavigationTests: XCTestCase {
    private var backend: MockBackend!
    private var store: AppStore!

    private func make(latency: Double = 0) async {
        backend = MockBackend(scenario: .demo, latency: latency)
        store = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: UserDefaults(suiteName: "ah-navigation-" + UUID().uuidString)!)
        store.advanceTiming = .immediate
        await store.start()
        await eventually { self.store.state.foreground && self.store.pendingCount == 5 }
    }

    private func item(_ rid: String) throws -> InboxItem {
        try XCTUnwrap(store.state.inbox.first { $0.requestId == rid })
    }

    func testQueueKeepsOrderAndSkipsEndedAnchorsWithoutWrapping() {
        var queue = RequestNavigation(filter: .all, ids: ["home/same", "office/same", "home/third"], current: "office/same")
        queue.append(["home/third", "home/new", "home/new"])
        XCTAssertEqual(queue.ids, ["home/same", "office/same", "home/third", "home/new"])
        let live: Set<String> = ["home/same", "home/new"]
        XCTAssertEqual(queue.neighbor(of: "office/same", direction: -1, live: live), "home/same")
        XCTAssertEqual(queue.neighbor(of: "office/same", direction: 1, live: live), "home/new")
        XCTAssertNil(queue.neighbor(of: "home/new", direction: 1, live: live))
        XCTAssertNil(queue.neighbor(of: "home/same", direction: -1, live: live))
    }

    func testListEntryCapturesFilterAndAppendsOnlyMatchingArrivals() async throws {
        await make()
        store.filter = .asks
        let original = store.inboxSections().flatMap { $0.items.map(\.id) }
        store.inboxPath = [try item("req-migration").id]
        XCTAssertEqual(store.requestNavigation?.filter, .asks)
        XCTAssertEqual(store.requestNavigation?.ids, original)
        await backend.simulateArrival(kind: "permission")
        await eventually { self.store.pendingCount == 6 }
        XCTAssertEqual(store.requestNavigation?.ids, original)
        await backend.simulateArrival(sessionTitle: "A new question")
        await eventually { self.store.requestNavigation?.ids.count == original.count + 1 }
        XCTAssertEqual(Array(try XCTUnwrap(store.requestNavigation).ids.prefix(original.count)), original)
        XCTAssertEqual(store.item(id: try XCTUnwrap(store.requestNavigation?.ids.last))?.sessionTitle, "A new question")
        let seen = await backend.seen
        XCTAssertTrue(seen.isEmpty, "assembling a queue never marks its neighbors read")
    }

    func testExplicitEntryUsesAllRequestsEvenWhenTheListIsFiltered() async throws {
        await make()
        store.filter = .mac(MockData.officeMacId)
        let request = try item("req-migration")
        store.openNotification(macId: request.macId, rid: request.requestId, kind: "ask")
        XCTAssertEqual(store.requestNavigation?.filter, .all)
        XCTAssertEqual(store.requestNavigation?.ids.count, 5)
    }

    func testSwitchingReplacesOnePageAndKeepsDraftsAttachmentsAndReadingPositions() async throws {
        await make()
        let migration = try item("req-migration")
        store.inboxPath = [migration.id]
        let composer = try XCTUnwrap(store.composer(for: migration))
        composer.setText("Keep this draft", question: 2)
        composer.toggle(question: 0, option: 0)
        let attachment = try composer.addAttachment(name: "notes.txt", mime: "text/plain", data: Data("original".utf8), isImage: false, question: 2)
        let position = RequestReadPosition(anchor: "question-2", distance: 12, fallback: 830)
        store.saveRequestReadPosition(position, for: migration.id)
        store.switchRequest(from: migration.id, direction: 1)
        let other = try XCTUnwrap(store.inboxPath.last)
        XCTAssertEqual(store.inboxPath.count, 1)
        XCTAssertNil(store.requestReadPosition(for: other))
        store.switchRequest(from: other, direction: -1)
        XCTAssertEqual(store.inboxPath, [migration.id])
        XCTAssertEqual(store.requestReadPosition(for: migration.id), position)
        XCTAssertTrue(store.composer(for: migration) === composer)
        XCTAssertEqual(composer.answers[2].text, "Keep this draft")
        XCTAssertTrue(composer.isSelected(question: 0, option: 0))
        XCTAssertEqual(try Data(contentsOf: composer.url(of: attachment)), Data("original".utf8))
        store.inboxPath = []
        XCTAssertNil(store.requestNavigation)
        XCTAssertNil(store.requestReadPosition(for: migration.id))
    }

    func testCurrentEndedRequestKeepsItsPlaceAndNavigationSkipsOtherEndedRequests() async throws {
        await make()
        let migration = try item("req-migration"), next = try item("req-next")
        store.inboxPath = [migration.id]
        await backend.simulateFinishedElsewhere(requestId: migration.requestId, macId: migration.macId)
        await backend.simulateFinishedElsewhere(requestId: next.requestId, macId: next.macId)
        await eventually { self.store.item(id: migration.id) == nil && self.store.item(id: next.id) == nil }
        XCTAssertEqual(store.inboxPath, [migration.id])
        XCTAssertNotNil(store.displayItem(id: migration.id))
        XCTAssertEqual(store.adjacentRequest(to: migration.id, direction: 1), try item("req-select").id)
        store.switchRequest(from: migration.id, direction: 1)
        XCTAssertEqual(store.inboxPath, [try item("req-select").id])
    }

    func testBackgroundUploadAndReturningToTheSameRequestCannotTriggerOldAutoAdvance() async throws {
        await make(latency: 0.5)
        let migration = try item("req-migration")
        store.inboxPath = [migration.id]
        let composer = try XCTUnwrap(store.composer(for: migration))
        composer.setText("Send from the original request", question: 2)
        try composer.addAttachment(name: "original.txt", mime: "text/plain", data: Data([1, 2, 3]), isImage: false, question: 2)
        let send = Task { try await store.send(composer, item: migration) }
        await eventually { composer.isSending }
        store.switchRequest(from: migration.id, direction: 1)
        let other = try XCTUnwrap(store.inboxPath.last)
        store.switchRequest(from: other, direction: -1)
        try await send.value
        await eventually { self.store.item(id: migration.id) == nil }
        XCTAssertEqual(store.inboxPath, [migration.id], "a late reply must not advance a page opened after a manual switch")
        XCTAssertNil(store.receipt)
        let submission = await backend.submissions.first
        XCTAssertEqual(submission?.requestId, migration.requestId)
        XCTAssertEqual(submission?.answers.first?.text, "Send from the original request")
        XCTAssertEqual(submission?.answers.first?.files.count, 1)
    }

    func testAutomaticAdvanceStillSelectsFromTheEntireInboxAndCreatesAnAllQueue() async throws {
        await make()
        store.filter = .mac(MockData.officeMacId)
        let select = try item("req-select")
        store.inboxPath = [select.id]
        XCTAssertEqual(store.requestNavigation?.filter, .mac(MockData.officeMacId))
        let expected = try XCTUnwrap(store.nextPending(after: select.id))
        XCTAssertNotEqual(expected.macId, select.macId)
        let composer = try XCTUnwrap(store.composer(for: select))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: select)
        await eventually { self.store.inboxPath == [expected.id] }
        XCTAssertEqual(store.requestNavigation?.filter, .all)
        XCTAssertTrue(store.requestNavigation?.ids.contains(expected.id) == true)
    }

    func testBackgroundErrorsStayWithTheirSourceRequest() async throws {
        await make()
        let migration = try item("req-migration")
        store.inboxPath = [migration.id]
        store.switchRequest(from: migration.id, direction: 1)
        let other = try XCTUnwrap(store.inboxPath.last)
        store.reportRequestError("Upload failed", for: migration.id)
        XCTAssertEqual(store.requestErrors[migration.id], "Upload failed")
        XCTAssertNil(store.requestErrors[other])
        XCTAssertEqual(store.inboxPath, [other])
        XCTAssertEqual(store.banner?.title, migration.displayTitle)
    }
}
