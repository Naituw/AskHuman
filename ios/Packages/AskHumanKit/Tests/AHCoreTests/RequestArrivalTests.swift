import XCTest
import AHProtocol
import AHStore
@testable import AHCore

/// Exercises the real engine-to-screen path; the loopback Mac sends actual snapshots and deltas.
@MainActor
final class RequestArrivalTests: XCTestCase {
    private func make(preloaded: [RequestView] = []) async -> (Harness, AppStore) {
        let h = Harness(collectEvents: false)
        for request in preloaded { await h.mac.deliver(request) }
        let store = AppStore(
            backend: h.engine, drafts: MemoryDraftStore(),
            defaults: UserDefaults(suiteName: "ah-arrival-" + UUID().uuidString)!
        )
        await store.start()
        await store.setForeground(true)
        await h.pair()
        await eventually("the screen to finish syncing") { store.state.macs.first?.link == .online }
        return (h, store)
    }

    private func assertOnlyTheNextLiveRequestAnimates(
        _ h: Harness, _ store: AppStore, silent: RequestView,
        file: StaticString = #filePath, line: UInt = #line
    ) async {
        let live = sampleAsk()
        await h.mac.deliver(live)
        await eventually("the next live arrival") { store.requestArrivals.contains { $0.requestId == live.id } }
        XCTAssertEqual(store.requestArrivals.map(\.requestId), [live.id], file: file, line: line)
        XCTAssertEqual(store.arrivalTick, 1, "sync must not add a haptic", file: file, line: line)
        XCTAssertTrue(store.state.inbox.first { $0.requestId == silent.id }?.unread == true, file: file, line: line)
        XCTAssertNotNil(store.state.inbox.first { $0.requestId == silent.id }?.view, file: file, line: line)
        XCTAssertEqual(store.tab, .inbox, "live arrivals must still animate in the inbox", file: file, line: line)
    }

    func testInitialSnapshotIsSilentThenLiveArrivalAnimatesInTheInbox() async {
        let backlog = sampleAsk()
        let (h, store) = await make(preloaded: [backlog])
        await assertOnlyTheNextLiveRequestAnimates(h, store, silent: backlog)
    }

    func testReturningFromTheBackgroundDoesNotAnimateTheMissedRequests() async {
        let (h, store) = await make()
        await store.setForeground(false)
        let backlog = sampleAsk()
        await h.mac.deliver(backlog)
        await store.setForeground(true)
        await eventually("the return snapshot") { store.pendingCount == 1 && store.state.macs.first?.link == .online }
        await assertOnlyTheNextLiveRequestAnimates(h, store, silent: backlog)
    }

    func testPullToRefreshAddsMissingRequestsSilently() async {
        let (h, store) = await make()
        let missed = sampleAsk()
        // Store the request on the Mac without delivering its delta to this phone.
        await h.mac.deliver(missed, to: "not-paired")
        await h.engine.refresh()
        await eventually("the refresh snapshot") { store.pendingCount == 1 }
        await assertOnlyTheNextLiveRequestAnimates(h, store, silent: missed)
    }

    func testAGapInTheLiveStreamResyncsSilently() async {
        let (h, store) = await make()
        h.mac.skipRevision()
        let missed = sampleAsk()
        await h.mac.deliver(missed)
        await eventually("the gap-repair snapshot") { store.pendingCount == 1 }
        await assertOnlyTheNextLiveRequestAnimates(h, store, silent: missed)
    }

    func testAMacComingBackOnlineDoesNotReplayItsBacklog() async {
        let (h, store) = await make()
        await h.mac.publishPresence(online: false)
        await eventually("the Mac offline") { store.state.macs.first?.link == .macOffline }
        let backlog = sampleAsk()
        await h.mac.deliver(backlog, to: "not-paired")
        await h.mac.publishPresence(online: true)
        await eventually("the reconnect snapshot") { store.pendingCount == 1 && store.state.macs.first?.link == .online }
        await assertOnlyTheNextLiveRequestAnimates(h, store, silent: backlog)
    }
}
