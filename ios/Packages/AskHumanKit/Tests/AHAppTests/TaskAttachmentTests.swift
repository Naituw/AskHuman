import XCTest
import AHCore
import AHProtocol
import AHStore

@MainActor
final class TaskAttachmentTests: XCTestCase {
    private func options(supported: Bool? = true) -> TaskOptions {
        TaskOptions(enabled: true, permissionMode: "agentDefault", workspaces: [
            WorkspaceOption(id: "w", label: "Project", subtitle: "", pinned: true)
        ], agents: [AgentOption(kind: "claude", label: "Claude", ready: true, subtitle: "", permissionSelectable: false)],
        attachmentsSupported: supported)
    }

    private func composer() -> TaskComposer {
        let form = TaskComposer(macId: "mac")
        form.applyDefaults(options: options(), defaults: UserDefaults(suiteName: UUID().uuidString)!)
        return form
    }

    func testAttachmentsDoNotReplaceInstructionsAndOldMacCannotSilentlyDropThem() throws {
        let form = composer()
        let file = try form.addAttachment(name: "Notes.txt", mime: "text/plain", data: Data("notes".utf8), isImage: false)
        XCTAssertTrue(form.hasContent)
        XCTAssertFalse(form.canSubmit(options: options()))
        form.todoId = "todo"
        XCTAssertTrue(form.canSubmit(options: options()))
        XCTAssertFalse(form.canSubmit(options: options(supported: nil)))
        form.todoId = nil
        form.text = "Review it"
        XCTAssertTrue(form.canSubmit(options: options()))
        form.removeAttachment(id: file.id)
        XCTAssertTrue(form.canSubmit(options: options(supported: nil)))
    }

    func testPreparingAndSendingGateCreationUntilEveryPreparationFinishes() {
        let form = composer()
        form.text = "Do it"
        let first = UUID(), second = UUID()
        form.setAttachmentPreparation(first, active: true)
        form.setAttachmentPreparation(second, active: true)
        form.setAttachmentPreparation(first, active: false)
        XCTAssertFalse(form.canSubmit(options: options()))
        form.setAttachmentPreparation(second, active: false)
        XCTAssertTrue(form.canSubmit(options: options()))
        form.isSending = true
        XCTAssertFalse(form.canSubmit(options: options()))
    }

    func testFilesSurviveMacAndAgentChangesAndAreRemovedWithTheDraft() throws {
        var form: TaskComposer? = composer()
        let file = try form!.addAttachment(name: "../Notes.txt", mime: "text/plain", data: Data("notes".utf8), isImage: false)
        let url = form!.url(of: file)
        XCTAssertFalse(url.lastPathComponent.contains("/"))
        form!.switchMac(to: "other")
        form!.selectAgent("codex")
        XCTAssertEqual(try form!.sources().first?.data, Data("notes".utf8))
        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path))
        form = nil
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
    }

    func testFileLimitsAndRemovalAreEnforcedLocally() throws {
        let form = composer()
        XCTAssertThrowsError(try form.addAttachment(name: "big", mime: "application/octet-stream", data: Data(count: Wire.maxAssetBytes + 1), isImage: false))
        for index in 0..<Wire.maxReplyAttachments {
            try form.addAttachment(name: "\(index).mp4", mime: "video/mp4", data: Data([1]), isImage: false)
        }
        XCTAssertFalse(form.canAttachMore)
        XCTAssertThrowsError(try form.addAttachment(name: "extra", mime: "text/plain", data: Data([1]), isImage: false))
        let removed = form.attachments[0]
        let url = form.url(of: removed)
        form.removeAttachment(id: removed.id)
        XCTAssertTrue(form.canAttachMore)
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
        XCTAssertEqual(try form.sources().map(\.isImage), [false, false, false, false])
    }
}
