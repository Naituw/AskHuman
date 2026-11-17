import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// `request.seen` tells the Mac that the user has read a request, which is what keeps it from pushing
/// that request to a phone that is not looking. The user opens a request once, so the phone owns what
/// was seen and has to tell the Mac again whenever the Mac may not have heard: the link went away with
/// the message, the Mac was not there, or it started over.
final class RequestSeenTests: XCTestCase {
    private func deliveredAndSeen(_ h: Harness) async throws -> RequestView {
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        await h.engine.markSeen(macId: h.mac.macId, requestId: request.id)
        return request
    }

    /// A new session starts from nothing the Mac was told, and tells it once.
    func testWhatWasSeenIsToldAgainInTheNextSession() async throws {
        let h = Harness()
        await h.pair()
        _ = try await deliveredAndSeen(h)
        await eventually("request.seen") { h.mac.count(RequestSeen.type) == 1 }

        h.phone.drop()
        await eventually("broker down") { await h.state().macs.first?.link == .brokerDown }
        await h.advance(until: { await h.state().macs.first?.link == .online }, "reconnect and resync")
        await eventually("told again") { h.mac.count(RequestSeen.type) == 2 }
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertEqual(h.mac.count(RequestSeen.type), 2, "once in a session")
    }

    /// The link still looks connected but takes nothing: the Mac was not told, and nothing says it was.
    func testASeenMarkThatCouldNotBeSentIsNotRemembered() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }

        h.phone.failsPublishes = true
        await h.engine.markSeen(macId: h.mac.macId, requestId: request.id)
        XCTAssertEqual(h.mac.count(RequestSeen.type), 0, "the message went nowhere")

        h.phone.failsPublishes = false
        // What lists the Mac's requests again (here a snapshot) tells it what was seen.
        await h.mac.sendInboxSnapshot(to: h.phoneId)
        await eventually("told") { h.mac.count(RequestSeen.type) == 1 }
    }

    /// The user reads a request while the Mac is away: there is nobody to tell yet.
    func testWhatWasSeenWhileTheMacWasAwayIsToldWhenItIsBack() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }

        h.mac.client.drop()
        await eventually("macOffline") { await h.state().macs.first?.link == .macOffline }
        await h.engine.markSeen(macId: h.mac.macId, requestId: request.id)
        XCTAssertEqual(h.mac.count(RequestSeen.type), 0)

        await h.mac.start(password: "pw")
        await eventually("online again") { await h.state().macs.first?.link == .online }
        await eventually("told") { h.mac.count(RequestSeen.type) == 1 }
    }

    /// A request nobody has opened has nothing to tell.
    func testAnUnreadRequestIsNotReportedAsSeenByANewSession() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.deliver(sampleAsk())
        await eventually("inbox") { await h.state().inbox.count == 1 }

        h.phone.drop()
        await eventually("broker down") { await h.state().macs.first?.link == .brokerDown }
        await h.advance(until: { await h.state().macs.first?.link == .online }, "reconnect and resync")
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertEqual(h.mac.count(RequestSeen.type), 0)
    }
}
