import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHCore

final class LiveUpdateTests: XCTestCase {
    func testAWaitingSubmissionResumesWithItsOriginalIdentityAfterTheMacUpdates() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("request") { await h.state().inbox.count == 1 }
        h.mac.ignoreAnswers = true
        let id = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "send", answers: [AnswerItem(q: 0, sel: [0])])
        await eventually("first send") { h.mac.count("answer.submit") == 1 }
        await h.mac.publishPresence(online: false, update: "updating")
        await eventually("updating") { await h.state().requestConnection(macId: h.mac.macId) == .updating }
        await h.pass(seconds: 40)
        var state = await h.state()
        guard case .sending(let waiting, _) = state.inbox.first?.state else { return XCTFail("the submission must keep waiting") }
        XCTAssertEqual(waiting, id)

        h.mac.holdSnapshots = true
        h.mac.epoch = "updated-epoch"
        await h.mac.publishPresence(online: true, update: "recovering")
        await eventually("new handshake") { h.mac.count(Wire.T.hello) == 2 }
        state = await h.state()
        XCTAssertEqual(state.inbox.first?.requestId, request.id)
        h.mac.ignoreAnswers = false
        await h.mac.publishPresence(online: true)
        await h.mac.sendSnapshots(to: h.phoneId)
        await eventually("accepted after update") { await h.state().inbox.isEmpty }
        let submissions = try h.mac.received("answer.submit").map { try $0.body(as: AnswerSubmit.self) }
        XCTAssertGreaterThanOrEqual(submissions.count, 2)
        XCTAssertTrue(submissions.allSatisfy { $0.clientMsgId == id })
    }

    func testAnInstalledUpdateKeepsTheRequestReadyUntilHandoffStarts() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.publishPresence(online: true, update: "pending")
        await eventually("pending update") { await h.state().macs.first?.update == "pending" }
        let state = await h.state()
        XCTAssertEqual(state.requestConnection(macId: h.mac.macId), .ready)
    }
}
