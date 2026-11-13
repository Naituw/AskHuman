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

    func testFilesWithTheSameNameAreToldApartAndNothingIsOverwritten() throws {
        let form = composer()
        let first = try form.addAttachment(name: "Photo.jpg", mime: "image/jpeg", data: Data([1]), isImage: true)
        let second = try form.addAttachment(name: "Photo.jpg", mime: "image/jpeg", data: Data([2]), isImage: true)
        // Letter case does not make a name another one: the Mac's disk would not tell them apart.
        let third = try form.addAttachment(name: "photo.JPG", mime: "image/jpeg", data: Data([3]), isImage: true)
        XCTAssertEqual(form.attachments.map(\.name), ["Photo.jpg", "Photo-2.jpg", "photo-3.JPG"])
        XCTAssertEqual(Set([first, second, third].map { form.url(of: $0) }).count, 3, "each has a file of its own")
        // What goes up is what was picked, in the order it was picked, under the names the list shows.
        XCTAssertEqual(try form.sources().map(\.name), ["Photo.jpg", "Photo-2.jpg", "photo-3.JPG"])
        XCTAssertEqual(try form.sources().map(\.data), [Data([1]), Data([2]), Data([3])])
    }

    func testANameThatWasFreedIsGivenOutAgainAndOddNamesStayTheirOwn() throws {
        let form = composer()
        try form.addAttachment(name: "notes", mime: "text/plain", data: Data([1]), isImage: false)
        let second = try form.addAttachment(name: "notes", mime: "text/plain", data: Data([2]), isImage: false)
        XCTAssertEqual(second.name, "notes-2", "a name without an extension")
        form.removeAttachment(id: second.id)
        XCTAssertEqual(try form.addAttachment(name: "notes", mime: "text/plain", data: Data([3]), isImage: false).name, "notes-2")
        try form.addAttachment(name: "archive.tar.gz", mime: "application/gzip", data: Data([4]), isImage: false)
        XCTAssertEqual(try form.addAttachment(name: "archive.tar.gz", mime: "application/gzip", data: Data([5]), isImage: false).name, "archive.tar-2.gz")
    }

    func testClosingTheFormStopsWhatIsStillBeingPrepared() {
        let form = composer()
        let operation = Task<Void, Never> { try? await Task.sleep(for: .seconds(60)) }
        form.attachmentPreparation.operation = operation
        form.discard()
        XCTAssertTrue(operation.isCancelled)
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
