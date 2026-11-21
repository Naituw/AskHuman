import XCTest
import AHProtocol
import AHStore
@testable import AHMock
@testable import AHCore

/// "Remove for now" as the screens see it (docs/specs/ios-offline-remove.md): when it is offered, what it does to the lists, the
/// page and the draft, and what brings the request back.
@MainActor
final class RemoveForNowTests: XCTestCase {
    private var backend: MockBackend!
    private var store: AppStore!
    private var drafts: MemoryDraftStore!

    private func make() async {
        backend = MockBackend(scenario: .demo, latency: 0)
        drafts = MemoryDraftStore()
        store = AppStore(backend: backend, drafts: drafts, defaults: UserDefaults(suiteName: "ah-test-" + UUID().uuidString)!)
        store.advanceTiming = .immediate
        await store.start()
        await eventually("first state") { self.store.state.foreground && self.store.pendingCount == 5 }
    }

    private func item(_ requestId: String) throws -> InboxItem {
        try XCTUnwrap(store.state.inbox.first { $0.requestId == requestId })
    }

    private func link(_ link: MacLink, of macId: String = MockData.homeMacId) async {
        await backend.setLink(link, for: macId)
        await eventually("the link shows") { self.store.state.macs.first { $0.id == macId }?.link == link }
    }

    private func pendingCount(of macId: String) -> Int? { store.state.macs.first { $0.id == macId }?.pendingCount }

    func testItIsOfferedOnlyWhileTheMacIsOffline() async throws {
        await make()
        let request = try item("req-migration")
        let expected: [(MacLink, Bool)] = [(.online, false), (.connecting, false), (.brokerDown, false), (.macOffline, true)]
        for (state, offered) in expected {
            await link(state)
            XCTAssertEqual(store.canRemoveForNow(request), offered, "\(state)")
        }
        XCTAssertFalse(store.canRemoveForNow(try item("req-select")), "the other Mac is online")
        await link(.online)
        XCTAssertFalse(store.canRemoveForNow(request))
    }

    func testRemovingTakesTheRequestOffTheListsAndTheOpenPageGivesWayToTheList() async throws {
        await make()
        let request = try item("req-migration")
        XCTAssertEqual(pendingCount(of: MockData.homeMacId), 3)
        store.openFromList(request.id)
        XCTAssertEqual(store.inboxPath, [request.id])
        await link(.macOffline)

        await store.removeForNow(request)
        await eventually("off the list") { !self.store.state.inbox.contains { $0.id == request.id } }

        XCTAssertEqual(store.pendingCount, 4)
        XCTAssertEqual(pendingCount(of: MockData.homeMacId), 2, "the count of its Mac is down")
        XCTAssertTrue(store.inboxPath.isEmpty, "back at the list, not on the next request")
        XCTAssertEqual(store.state.removedForNowIds, [request.id])
        XCTAssertNil(store.displayItem(id: request.id), "nothing keeps showing a request that was taken away")
        XCTAssertFalse(store.inboxSections().flatMap(\.items).contains { $0.id == request.id })
    }

    func testARemovedRequestIsNotANeighborOfTheOthers() async throws {
        await make()
        await link(.macOffline, of: MockData.homeMacId)
        await link(.macOffline, of: MockData.officeMacId)
        let open = try item("req-permission")
        store.openFromList(open.id)
        let neighbor = try XCTUnwrap(store.adjacentRequest(to: open.id, direction: 1))

        await store.removeForNow(try XCTUnwrap(store.item(id: neighbor)))
        await eventually("the neighbor is off") { self.store.item(id: neighbor) == nil }

        XCTAssertNotEqual(store.adjacentRequest(to: open.id, direction: 1), neighbor)
        XCTAssertEqual(store.inboxPath, [open.id], "the page that was open stays: it was another request that went")
    }

    func testNothingIsRemovedWhileTheMacIsReachable() async throws {
        await make()
        let request = try item("req-migration")

        await store.removeForNow(request)

        XCTAssertEqual(store.pendingCount, 5)
        XCTAssertTrue(store.state.removedForNowIds.isEmpty)
    }

    func testTheDraftOfARemovedRequestIsKeptAndTheRequestReturnsWithTheMac() async throws {
        await make()
        let request = try item("req-migration")
        store.openFromList(request.id)
        let composer = try XCTUnwrap(store.composer(for: request))
        composer.setText("Half way through", question: 2)
        await link(.macOffline)

        await store.removeForNow(request)
        await eventually("off the list") { self.store.pendingCount == 4 }
        XCTAssertEqual(drafts.draft(key: request.id)?.answers[2].text, "Half way through", "saved when the page closed")
        XCTAssertTrue(store.composer(for: request) === composer, "the form is still there for when it returns")

        await link(.online)
        await eventually("it returns") { self.store.pendingCount == 5 }
        let back = try item("req-migration")
        XCTAssertTrue(back.unread, "new to the list again")
        XCTAssertTrue(store.state.removedForNowIds.isEmpty)
        XCTAssertEqual(store.composer(for: back)?.answers[2].text, "Half way through")
        XCTAssertEqual(drafts.draft(key: request.id)?.answers[2].text, "Half way through")
    }

    /// A form closed after its request was taken off the lists (what `closed(id:)` does for a request that is gone) is not a
    /// form of a request that is over: its draft and the form stay.
    func testClosingTheFormOfARemovedRequestKeepsItsDraft() async throws {
        await make()
        let request = try item("req-migration")
        let composer = try XCTUnwrap(store.composer(for: request))
        composer.setText("Still mine", question: 2)
        await link(.macOffline)
        await store.removeForNow(request)
        await eventually("off the list") { self.store.pendingCount == 4 }

        store.closed(id: request.id)

        XCTAssertEqual(drafts.draft(key: request.id)?.answers[2].text, "Still mine")
        XCTAssertEqual(composer.answers[2].text, "Still mine")
        await link(.online)
        await eventually("it returns") { self.store.pendingCount == 5 }
        XCTAssertEqual(store.composer(for: try item("req-migration"))?.answers[2].text, "Still mine")
    }

    /// One Mac coming back brings back its own requests only.
    func testAMacBringsBackItsOwnRequestsOnly() async throws {
        await make()
        let home = try item("req-migration")
        let office = try item("req-select")
        await link(.macOffline, of: MockData.homeMacId)
        await link(.macOffline, of: MockData.officeMacId)
        await store.removeForNow(home)
        await store.removeForNow(office)
        await eventually("both off") { self.store.pendingCount == 3 }

        await link(.online, of: MockData.officeMacId)
        await eventually("the office one returns") { self.store.pendingCount == 4 }

        XCTAssertEqual(store.state.removedForNowIds, [home.id])
        XCTAssertNotNil(store.state.inbox.first { $0.id == office.id })
    }

    /// With every request removed and no Mac reachable the list has a reason for being empty, and it is not "nothing waiting".
    func testTheListSaysItCannotReachTheMacsWhenEverythingWasRemoved() async throws {
        await make()
        await link(.macOffline, of: MockData.homeMacId)
        await link(.macOffline, of: MockData.officeMacId)
        for request in store.state.inbox { await store.removeForNow(request) }
        await eventually("all off") { self.store.pendingCount == 0 }

        XCTAssertTrue(store.inboxSections().isEmpty)
        XCTAssertEqual(store.listReadiness, .unreachable)
        XCTAssertEqual(store.state.removedForNowIds.count, 5)
    }

    /// A tapped notification of a request that was removed opens a page that says the Mac cannot be reached, not "unavailable":
    /// the request is not gone, and it shows up when the Mac is back.
    func testTheNotificationOfARemovedRequestFindsAMacThatCannotBeReached() async throws {
        await make()
        let request = try item("req-migration")
        await link(.macOffline)
        await store.removeForNow(request)
        await eventually("off the list") { self.store.pendingCount == 4 }

        XCTAssertNil(store.displayItem(id: request.id))
        XCTAssertEqual(store.missingRequest(id: request.id), .unreachable)

        await link(.online)
        await eventually("it returns") { self.store.item(id: request.id) != nil }
    }

    func testTheQuestionGoesWithTheOfflineMacAndWithThePage() async throws {
        await make()
        let request = try item("req-migration")
        await link(.macOffline)
        store.removalPrompt = request.id

        await link(.online)
        await eventually("the question is gone") { self.store.removalPrompt == nil }

        await link(.macOffline)
        store.openFromList(request.id)
        store.removalPrompt = request.id
        store.inboxPath = []
        XCTAssertNil(store.removalPrompt, "closing the page withdraws it")
    }
}
