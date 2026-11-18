import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// A request the phone knows only as a line of the inbox snapshot is completed with `request.get`
/// (`PhoneEngine.fetchFull`). The wait for the answer belongs to the session it began in: once the
/// session is over, or the Mac says it has nothing to give, nobody is waiting any more, and the next
/// snapshot has to be able to ask again. Otherwise the request's page stays on "Loading the full
/// request…" until the app is restarted.
final class RequestFetchTests: XCTestCase {
    /// The answer goes the way of the connection that carried the question.
    func testARequestWhoseFullViewWasLostWithTheLinkIsFetchedAgainInTheNextSession() async throws {
        let h = Harness()
        await h.pair()
        h.mac.dropNextRequestGet = true
        await h.mac.deliver(sampleAsk(text: "long body"), full: false)
        await eventually("request.get") { h.mac.count(RequestGet.type) == 1 }
        let waiting = await h.state().inbox
        XCTAssertEqual(waiting.count, 1)
        XCTAssertNil(waiting.first?.view, "only the summary is here, and the answer is not coming")

        h.phone.drop()
        await eventually("broker down") { await h.state().macs.first?.link == .brokerDown }
        await h.advance(until: { await h.state().macs.first?.link == .online }, "reconnect and resync")
        await eventually("asked again") { h.mac.count(RequestGet.type) == 2 }
        await eventually("full view") { await h.state().firstInbox?.view != nil }
    }

    /// The Mac goes away (its last will says so) with the question unanswered, and comes back.
    func testARequestWhoseFullViewWasLostWithTheMacIsFetchedAgainWhenItIsBack() async throws {
        let h = Harness()
        await h.pair()
        h.mac.dropNextRequestGet = true
        await h.mac.deliver(sampleAsk(text: "long body"), full: false)
        await eventually("request.get") { h.mac.count(RequestGet.type) == 1 }

        h.mac.client.drop()
        await eventually("macOffline") { await h.state().macs.first?.link == .macOffline }
        await h.mac.start(password: "pw")
        await eventually("online again") { await h.state().macs.first?.link == .online }
        await eventually("asked again") { h.mac.count(RequestGet.type) == 2 }
        await eventually("full view") { await h.state().firstInbox?.view != nil }
    }

    /// The link still looks connected but takes nothing, as in the moments before a dropped
    /// connection is noticed: the question never went out, so there is nothing to wait for.
    func testAFetchThatCouldNotBeSentIsNotWaitedFor() async throws {
        let h = Harness()
        await h.pair()
        let up = Wire.topicUp(user: "u1", macId: h.mac.macId)
        let tally = Tally()
        h.phone.beforePublish = { topic in if topic == up { tally.add() } }
        h.phone.failsPublishes = true
        await h.mac.deliver(sampleAsk(text: "long body"), full: false)
        await eventually("the phone tried to fetch") { tally.count >= 1 }
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertEqual(h.mac.count(RequestGet.type), 0, "the question went nowhere")

        h.phone.failsPublishes = false
        h.phone.beforePublish = nil
        // The next thing that lists the request (here a snapshot) asks again.
        await h.mac.sendInboxSnapshot(to: h.phoneId)
        await eventually("asked") { h.mac.count(RequestGet.type) == 1 }
        await eventually("full view") { await h.state().firstInbox?.view != nil }
    }

    /// The Mac answers `request.get` for a request it has no trace of: what became of the request is
    /// for its inbox to say, so the phone asks for the snapshot instead of waiting for a full view.
    func testTheMacSayingItHasNoSuchRequestEndsTheWaitAndAsksForTheInbox() async throws {
        let h = Harness()
        await h.pair()
        h.mac.dropNextRequestGet = true
        let request = sampleAsk(text: "long body")
        await h.mac.deliver(request, full: false)
        await eventually("request.get") { h.mac.count(RequestGet.type) == 1 }

        _ = await h.mac.sendDown(ErrorBody(ref: request.id, code: "not_found", message: nil), to: h.phoneId)
        await eventually("the phone asks for the inbox") {
            h.mac.received(Resync.type).contains { (try? $0.body(as: Resync.self))?.stream == "inbox" }
        }
        // This Mac still lists the request in its snapshot, so the phone asks for it again and gets it.
        await eventually("asked again") { h.mac.count(RequestGet.type) == 2 }
        await eventually("full view") { await h.state().firstInbox?.view != nil }
    }
}

/// Counts the publishes a test hook sees.
private final class Tally: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0

    func add() { lock.lock(); value += 1; lock.unlock() }
    var count: Int { lock.lock(); defer { lock.unlock() }; return value }
}
