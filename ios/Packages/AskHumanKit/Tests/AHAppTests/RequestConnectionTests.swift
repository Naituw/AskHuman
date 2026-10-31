import XCTest
import AHCore
import AHMock
import AHStore

@MainActor
final class RequestConnectionTests: XCTestCase {
    func testPhoneConnectionTakesPrecedenceOverCachedMacPresence() {
        let phases: [(ConnectionPhase, RequestConnection)] = [
            (.idle, .connecting), (.connecting(via: "tcp"), .connecting),
            (.backoff(retryAt: Date(), reason: "network"), .reconnecting),
            (.refused("credentials"), .refused),
        ]
        for link in [MacLink.online, .connecting, .macOffline, .brokerDown] {
            for (phase, expected) in phases {
                let state = EngineState(phase: phase, macs: MockData.macs(link: link))
                XCTAssertEqual(state.requestConnection(macId: MockData.homeMacId), expected, "\(phase), cached \(link)")
            }
        }
    }

    func testOnlyEstablishedPhoneConnectionCanReportAnOfflineMac() {
        let links: [(MacLink, RequestConnection)] = [
            (.online, .ready), (.connecting, .connecting), (.macOffline, .macOffline), (.brokerDown, .reconnecting),
        ]
        for (link, expected) in links {
            let state = EngineState(phase: .connected(via: "tcp"), macs: MockData.macs(link: link))
            XCTAssertEqual(state.requestConnection(macId: MockData.homeMacId), expected)
        }
        XCTAssertEqual(EngineState().requestConnection(macId: "unpaired"), .unavailable)
    }

    func testAnotherMacConnectingDoesNotBlockTheRequestWhoseMacIsReady() {
        var state = EngineState(phase: .connected(via: "tcp"), macs: MockData.macs())
        state.macs[1].link = .connecting
        XCTAssertEqual(state.syncStatus, .updating)
        XCTAssertEqual(state.requestConnection(macId: MockData.homeMacId), .ready)
        XCTAssertEqual(state.requestConnection(macId: MockData.officeMacId), .connecting)
    }

    func testCachedRequestRecoversWithoutReplacingItsDraftOrRoute() async throws {
        let backend = MockBackend(scenario: .syncing, latency: 0, cachedInbox: true)
        let store = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: UserDefaults(suiteName: UUID().uuidString)!)
        await store.start()
        await eventually { store.pendingCount == 5 }
        let item = try XCTUnwrap(store.state.inbox.first { $0.requestId == "req-migration" })
        store.inboxPath = [item.id]
        let composer = try XCTUnwrap(store.composer(for: item))
        composer.setText("Keep this draft during reconnection", question: 2)
        XCTAssertEqual(store.state.requestConnection(macId: item.macId), .connecting)
        await backend.finishSync()
        await eventually { store.state.requestConnection(macId: item.macId) == .ready }
        XCTAssertEqual(store.inboxPath, [item.id])
        XCTAssertTrue(store.composer(for: item) === composer)
        XCTAssertEqual(composer.answers[2].text, "Keep this draft during reconnection")
        await backend.setLink(.macOffline, for: item.macId)
        await eventually { store.state.requestConnection(macId: item.macId) == .macOffline }
        XCTAssertEqual(composer.answers[2].text, "Keep this draft during reconnection")
    }
}
