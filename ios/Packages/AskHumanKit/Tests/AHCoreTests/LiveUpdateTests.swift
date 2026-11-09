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

    func testUploadsResumeWhenUpdatePresenceClearsWithoutAnOfflineMessage() async throws {
        for update: String? in [nil, "pending"] {
            for restarted in [false, true] {
                let h = Harness()
                await h.pair()
                let request = sampleAsk()
                await h.mac.deliver(request)
                await eventually("request") { await h.state().inbox.count == 1 }
                h.mac.ignoreUploads = true
                let transfer = Task {
                    try await h.engine.upload(macId: h.mac.macId, requestId: request.id,
                        source: UploadSource(name: "update.txt", mime: "text/plain", data: Data([1, 2, 3]), isImage: false)) { _ in }
                }
                defer { transfer.cancel() }
                await eventually("first upload begin") { h.mac.count(UploadBegin.type) == 1 }
                await h.mac.publishPresence(online: true, update: "updating")
                await eventually("updating") { await h.state().macs.first?.isUpdating == true }
                if restarted { h.mac.epoch = "replacement-epoch" }
                h.mac.ignoreUploads = false
                await h.mac.publishPresence(online: true, update: update)
                await eventually("resume upload after update, restarted=\(restarted), update=\(update ?? "none")") { h.mac.count(UploadBegin.type) >= 2 }
                guard h.mac.count(UploadBegin.type) >= 2 else { continue }
                let id = try await transfer.value
                XCTAssertEqual(h.mac.uploaded(id)?.data, Data([1, 2, 3]))
                let starts = try h.mac.received(UploadBegin.type).map { try $0.body(as: UploadBegin.self) }
                XCTAssertTrue(starts.allSatisfy { $0.uploadId == id })
            }
        }
    }
}
