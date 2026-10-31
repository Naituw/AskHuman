import XCTest
@testable import AHCore
@testable import AHProtocol

final class TaskAttachmentEngineTests: XCTestCase {
    private func ready(_ h: Harness, supported: Bool? = true) async {
        h.mac.taskOptions.attachmentsSupported = supported
        await h.pair()
        await h.engine.loadTaskOptions(macId: h.mac.macId)
        await eventually("task options") {
            if case .ready? = await h.state().taskOptions[h.mac.macId] { return true }
            return false
        }
    }

    private func draft() -> TaskDraft {
        TaskDraft(workspaceId: "ws1", workspaceName: "Project", agentKind: "codex", agentLabel: "Codex", permission: "default",
            files: [UploadSource(name: "Notes.txt", mime: "text/plain", data: Data("notes".utf8), isImage: false)], text: "Review this")
    }

    func testFilesFinishUploadingBeforeCreationAndRetryReusesTheSameFileIds() async throws {
        let h = Harness()
        h.mac.ignoreTasks = true
        await ready(h)
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft())
        await eventually("task.create") { h.mac.count(TaskCreate.type) == 1 }
        let begin = try h.mac.received(UploadBegin.type)[0].body(as: UploadBegin.self)
        let create = try h.mac.received(TaskCreate.type)[0].body(as: TaskCreate.self)
        XCTAssertEqual(begin.scope.taskId, id)
        XCTAssertNil(begin.scope.requestId)
        XCTAssertNil(begin.scope.agentKey)
        XCTAssertEqual(create.files, [begin.uploadId])
        XCTAssertEqual(h.mac.uploaded(begin.uploadId)?.data, Data("notes".utf8))
        await h.advance(until: { await h.state().tasks.first?.stage == "failed" }, "accept timeout")
        h.mac.ignoreTasks = false
        try await h.engine.retryTask(id: id)
        await eventually("retry") { h.mac.count(TaskCreate.type) == 2 }
        let again = try h.mac.received(TaskCreate.type)[1].body(as: TaskCreate.self)
        XCTAssertEqual(create, again)
        XCTAssertEqual(h.mac.count(UploadBegin.type), 1)
        let names = await h.state().tasks.first?.attachmentNames
        XCTAssertEqual(names, ["Notes.txt"])
    }

    func testAnUploadFailureCreatesNoTaskAndTheSameDraftCanBeRetried() async throws {
        let h = Harness()
        await ready(h)
        h.mac.uploadError = "tooLarge"
        do {
            _ = try await h.engine.createTask(macId: h.mac.macId, draft: draft())
            XCTFail("upload should fail")
        } catch { }
        XCTAssertEqual(h.mac.count(TaskCreate.type), 0)
        let tasks = await h.state().tasks
        XCTAssertTrue(tasks.isEmpty)
        await eventually("failed upload cancelled") { h.mac.count(UploadCancel.type) == 1 }
        h.mac.uploadError = nil
        _ = try await h.engine.createTask(macId: h.mac.macId, draft: draft())
        await eventually("retry created") { h.mac.count(TaskCreate.type) == 1 }
    }

    func testAnOldMacGetsNoUploadOrTaskWhenFilesArePresent() async {
        let h = Harness()
        await ready(h, supported: nil)
        do {
            _ = try await h.engine.createTask(macId: h.mac.macId, draft: draft())
            XCTFail("old Mac must not silently ignore files")
        } catch { }
        XCTAssertEqual(h.mac.count(UploadBegin.type), 0)
        XCTAssertEqual(h.mac.count(TaskCreate.type), 0)
    }

    func testALaterUploadFailureCancelsTheWholeBatchBeforeCreation() async throws {
        let h = Harness()
        await ready(h)
        var batch = draft()
        batch.files.append(UploadSource(name: "Image.png", mime: "image/png", data: Data([1, 2]), isImage: true))
        do {
            _ = try await h.engine.createTask(macId: h.mac.macId, draft: batch, progress: { fraction in
                if fraction >= 0.5 { h.mac.uploadError = "busy" }
            })
            XCTFail("second upload should fail")
        } catch { }
        await eventually("whole upload batch cancelled") { h.mac.count(UploadCancel.type) == 2 }
        let uploads = try h.mac.received(UploadBegin.type).map { try $0.body(as: UploadBegin.self) }
        let cancelled = try h.mac.received(UploadCancel.type).map { try $0.body(as: UploadCancel.self).uploadId }
        XCTAssertEqual(Set(cancelled), Set(uploads.map(\.uploadId)))
        XCTAssertEqual(h.mac.count(TaskCreate.type), 0)
        let tasks = await h.state().tasks
        XCTAssertTrue(tasks.isEmpty)
    }

    func testAnUnconfirmedCreatePublishKeepsThePreparedTaskForRetry() async throws {
        let h = Harness()
        await ready(h)
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft(), progress: { fraction in
            if fraction >= 1 { h.phone.failsPublishes = true }
        })
        let task = await h.state().tasks.first
        XCTAssertEqual(task?.id, id)
        XCTAssertEqual(task?.reason, "connection")
        XCTAssertEqual(h.mac.count(TaskCreate.type), 0)
        h.phone.failsPublishes = false
        try await h.engine.retryTask(id: id)
        await eventually("prepared task retried") { h.mac.count(TaskCreate.type) == 1 }
        let sent = try h.mac.received(TaskCreate.type)[0].body(as: TaskCreate.self)
        let upload = try h.mac.received(UploadBegin.type)[0].body(as: UploadBegin.self)
        XCTAssertEqual(sent.clientMsgId, id)
        XCTAssertEqual(sent.files, [upload.uploadId])
        XCTAssertEqual(h.mac.count(UploadBegin.type), 1)
    }

    func testAMacReceiptIsNotOverwrittenByALostPublishAcknowledgement() async throws {
        let h = Harness()
        await ready(h)
        defer { h.phone.beforePublish = nil; h.phone.failsPublishes = false }
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft(), progress: { fraction in
            guard fraction >= 1 else { return }
            h.phone.beforePublish = { _ in
                let begin = try! h.mac.received(UploadBegin.type)[0].body(as: UploadBegin.self)
                await h.mac.sendDown(TaskStatus(clientMsgId: begin.scope.taskId!, stage: "launched", reason: nil, agentKey: "codex:receipt"), to: h.mac.pairedPhones[0])
                await eventually("Mac receipt") { await h.state().tasks.first?.stage == "launched" }
            }
            h.phone.failsPublishes = true
        })
        let task = await h.state().tasks.first
        XCTAssertEqual(task?.id, id)
        XCTAssertEqual(task?.stage, "launched")
        XCTAssertEqual(task?.agentKey, "codex:receipt")
        XCTAssertNil(task?.reason)
        XCTAssertEqual(h.mac.count(TaskCreate.type), 0)
    }

    func testALateMacReceiptResolvesAnUnconfirmedCreateWithoutRetry() async throws {
        let h = Harness()
        await ready(h)
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft(), progress: { fraction in
            if fraction >= 1 { h.phone.failsPublishes = true }
        })
        h.phone.failsPublishes = false
        await h.mac.sendDown(TaskStatus(clientMsgId: id, stage: "launched", reason: nil, agentKey: "codex:late"), to: h.mac.pairedPhones[0])
        await eventually("late Mac receipt") { await h.state().tasks.first?.stage == "launched" }
        let task = await h.state().tasks.first
        XCTAssertEqual(task?.agentKey, "codex:late")
        XCTAssertNil(task?.reason)
        XCTAssertEqual(h.mac.count(TaskCreate.type), 0)
    }
}
