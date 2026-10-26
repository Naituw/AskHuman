import XCTest
import AHProtocol
import AHStore
@testable import AHCore

@MainActor
final class AnswerComposerTests: XCTestCase {
    private var drafts: MemoryDraftStore!

    override func setUp() {
        drafts = MemoryDraftStore()
    }

    private func composer(_ view: AskView, key: String = "mac/req", debounce: Duration = .milliseconds(5)) -> AnswerComposer {
        AnswerComposer(key: key, ask: view, drafts: drafts, debounce: debounce)
    }

    func testMultipleChoiceKeepsSelectionsSorted() {
        let form = composer(ask([question("q", "a", "b", "c")]))
        form.toggle(question: 0, option: 2)
        form.toggle(question: 0, option: 0)
        XCTAssertEqual(form.answers[0].selected, [0, 2])
        form.toggle(question: 0, option: 2)
        XCTAssertEqual(form.answers[0].selected, [0])
    }

    func testSingleChoiceReplacesThePreviousSelection() {
        let form = composer(ask([question("q", "a", "b")], single: true))
        form.toggle(question: 0, option: 0)
        form.toggle(question: 0, option: 1)
        XCTAssertEqual(form.answers[0].selected, [1])
        form.toggle(question: 0, option: 1)
        XCTAssertEqual(form.answers[0].selected, [], "tapping the selected option clears it")
    }

    func testOutOfRangeTogglesAreIgnored() {
        let form = composer(ask([question("q", "a")]))
        form.toggle(question: 0, option: 5)
        form.toggle(question: 3, option: 0)
        XCTAssertTrue(form.answers[0].selected.isEmpty)
    }

    func testSelectOnlyNeedsAnOptionForEveryQuestionAndTakesNoText() throws {
        let form = composer(ask([question("one", "a", "b"), question("two", "c", "d")], selectOnly: true))
        XCTAssertFalse(form.canSend)
        XCTAssertEqual(form.blockedReason, .needsSelection)
        form.toggle(question: 0, option: 1)
        XCTAssertFalse(form.canSend)
        form.toggle(question: 1, option: 0)
        XCTAssertTrue(form.canSend)
        XCTAssertNil(form.blockedReason)

        form.setText("ignored", question: 0)
        XCTAssertEqual(form.answers[0].text, "")
        XCTAssertFalse(form.canAttachMore)
        XCTAssertThrowsError(try form.addAttachment(name: "a.txt", mime: "text/plain", data: Data("x".utf8), isImage: false, question: 0))
    }

    func testOrdinaryRequestsCanBeSentAsTheyAre() {
        let form = composer(ask([question("q", "a")]))
        XCTAssertTrue(form.canSend)
        XCTAssertFalse(form.hasContent)
        XCTAssertEqual(form.answeredCount, 0)
    }

    func testAQuestionCountsAsAnsweredWithAnOptionTextOrAttachment() throws {
        let form = composer(ask([question("1", "a"), question("2"), question("3"), question("4")]))
        form.toggle(question: 0, option: 0)
        form.setText("typed", question: 1)
        try form.addAttachment(name: "p.png", mime: "image/png", data: Data([1, 2, 3]), isImage: true, question: 2)
        XCTAssertEqual(form.answeredCount, 3)
        XCTAssertTrue(form.isAnswered(0) && form.isAnswered(1) && form.isAnswered(2) && !form.isAnswered(3))
    }

    func testAnswersBecomeWireItemsOnlyForAnsweredQuestions() throws {
        let form = composer(ask([question("1", "a", "b"), question("2"), question("3")]))
        form.toggle(question: 0, option: 1)
        form.toggle(question: 0, option: 0)
        form.setText("  hello  ", question: 2)
        let image = try form.addAttachment(name: "shot.png", mime: "image/png", data: Data([9]), isImage: true, question: 2)
        let file = try form.addAttachment(name: "log.txt", mime: "text/plain", data: Data([8]), isImage: false, question: 2)

        let items = form.items(uploads: [image.id: "up-1", file.id: "up-2"])
        XCTAssertEqual(items, [
            AnswerItem(q: 0, sel: [0, 1]),
            AnswerItem(q: 2, text: "hello", images: ["up-1"], files: ["up-2"]),
        ])
    }

    func testWhitespaceOnlyTextDoesNotBecomeAnAnswerText() {
        let form = composer(ask([question("1", "a")]))
        form.toggle(question: 0, option: 0)
        form.setText("   \n ", question: 0)
        XCTAssertEqual(form.items(uploads: [:]), [AnswerItem(q: 0, sel: [0])])
    }

    func testTextIsCappedAtTheMacLimit() {
        let form = composer(ask([question("1")]))
        form.setText(String(repeating: "x", count: Wire.maxAnswerTextChars + 500), question: 0)
        XCTAssertEqual(form.answers[0].text.count, Wire.maxAnswerTextChars)
    }

    func testAttachmentLimitsApplyToTheWholeForm() throws {
        let form = composer(ask([question("1"), question("2")]))
        for index in 0..<Wire.maxReplyAttachments {
            try form.addAttachment(name: "f\(index).txt", mime: "text/plain", data: Data([1]), isImage: false, question: index % 2)
        }
        XCTAssertFalse(form.canAttachMore)
        XCTAssertThrowsError(try form.addAttachment(name: "more.txt", mime: "text/plain", data: Data([1]), isImage: false, question: 0))

        let big = Data(count: Wire.maxAssetBytes + 1)
        let other = composer(ask([question("1")]), key: "mac/other")
        XCTAssertThrowsError(try other.addAttachment(name: "big.bin", mime: "application/octet-stream", data: big, isImage: false, question: 0)) {
            XCTAssertEqual($0 as? BackendError, .tooLarge)
        }
    }

    func testRemovingAnAttachmentDeletesItsFile() throws {
        let form = composer(ask([question("1")]))
        let attachment = try form.addAttachment(name: "a/b:c.txt", mime: "text/plain", data: Data([1]), isImage: false, question: 0)
        let url = form.url(of: attachment)
        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path))
        XCTAssertFalse(attachment.localName.contains("/"), "path separators never reach the file system")
        form.removeAttachment(id: attachment.id, question: 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
        XCTAssertTrue(form.answers[0].attachments.isEmpty)
    }

    func testEditsAreSavedAsADraftAfterTheDebounce() async {
        let form = composer(ask([question("1", "a")]))
        form.toggle(question: 0, option: 0)
        form.setText("half written", question: 0)
        await eventually { self.drafts.draft(key: "mac/req") != nil }
        XCTAssertEqual(drafts.draft(key: "mac/req")?.answers[0].text, "half written")
    }

    func testFlushSavesImmediatelyAndANewComposerPicksTheDraftUp() throws {
        let view = ask([question("1", "a", "b"), question("2")])
        let first = composer(view, debounce: .seconds(60))
        first.toggle(question: 0, option: 1)
        first.setText("note", question: 1)
        let attachment = try first.addAttachment(name: "n.txt", mime: "text/plain", data: Data([1]), isImage: false, question: 1)
        first.flush()

        let second = composer(view)
        XCTAssertEqual(second.answers[0].selected, [1])
        XCTAssertEqual(second.answers[1].text, "note")
        XCTAssertEqual(second.answers[1].attachments.map(\.id), [attachment.id])
        XCTAssertTrue(second.hasContent)
    }

    func testDraftsAreCleanedWhenTheRequestChanged() {
        let old = ask([question("1", "a", "b", "c"), question("2"), question("3")])
        let first = composer(old, debounce: .seconds(60))
        first.toggle(question: 0, option: 2)
        first.toggle(question: 0, option: 1)
        first.setText("kept", question: 2)
        first.flush()

        // The request now has fewer options and questions, and is single choice.
        let changed = ask([question("1", "a", "b")], single: true)
        let second = composer(changed)
        XCTAssertEqual(second.answers.count, 1)
        XCTAssertEqual(second.answers[0].selected, [1], "option 2 no longer exists and only one selection is allowed")

        let strict = ask([question("1", "a", "b"), question("2"), question("3")], selectOnly: true)
        let third = composer(strict)
        XCTAssertEqual(third.answers[2].text, "", "select-only forms take no text")
    }

    func testDiscardForgetsTheDraft() {
        let form = composer(ask([question("1", "a")]), debounce: .seconds(60))
        form.toggle(question: 0, option: 0)
        form.flush()
        XCTAssertNotNil(drafts.draft(key: "mac/req"))
        form.discard()
        XCTAssertNil(drafts.draft(key: "mac/req"))
    }

    func testEmptyDraftsAreNotKept() {
        let form = composer(ask([question("1", "a")]), debounce: .seconds(60))
        form.toggle(question: 0, option: 0)
        form.toggle(question: 0, option: 0)
        form.flush()
        XCTAssertNil(drafts.draft(key: "mac/req"))
    }

    // MARK: reading progress (spec multi-question-vertical V1/V2)

    func testASingleQuestionIsReadFromTheStart() {
        let form = composer(ask([question("q", "a")]))
        XCTAssertTrue(form.hasReachedEnd, "nothing to scroll to, so nothing to hold Send back")
        XCTAssertNil(form.nextUnread, "a single question has no 'next'")
    }

    func testSeveralQuestionsCannotBeSentBeforeTheLastWasSeen() {
        let form = composer(ask([question("1", "a"), question("2", "b"), question("3", "c")]))
        XCTAssertFalse(form.hasReachedEnd)
        XCTAssertEqual(form.nextUnread, 0)
        form.markSeen(0)
        XCTAssertEqual(form.nextUnread, 1, "Next leads to the first question not read yet")
        XCTAssertFalse(form.hasReachedEnd, "seeing the first ones is not enough")
        form.markSeen(2)
        XCTAssertTrue(form.hasReachedEnd, "the last question was seen: Send appears, as in the popup")
        XCTAssertEqual(form.nextUnread, 1, "the one skipped on the way is still pointed at")
    }

    func testAnAnsweredQuestionCountsAsRead() {
        let form = composer(ask([question("1", "a"), question("2", "b")]))
        form.toggle(question: 1, option: 0)
        XCTAssertTrue(form.hasReachedEnd, "an answer takes a look at the question")
        XCTAssertEqual(form.nextUnread, 0)
        XCTAssertFalse(form.seen.contains(1), "reading is only inferred, never recorded by answering")
    }

    func testAnAnswerFromADraftCountsAsReadToo() {
        let view = ask([question("1", "a"), question("2", "b")])
        let first = composer(view, debounce: .seconds(60))
        first.toggle(question: 1, option: 0)
        first.flush()
        XCTAssertTrue(composer(view).hasReachedEnd)
    }

    func testMarkingIsIgnoredOutsideTheQuestionsAndForASingleQuestion() {
        let form = composer(ask([question("1", "a"), question("2", "b")]))
        form.markSeen(-1)
        form.markSeen(2)
        XCTAssertTrue(form.seen.isEmpty)
        let single = composer(ask([question("1", "a")]), key: "mac/single")
        single.markSeen(0)
        XCTAssertTrue(single.seen.isEmpty)
    }

    func testReadingDoesNotChangeWhatMaySendByContent() {
        // Content rules and reading rules are separate: select-only still needs every selection.
        let form = composer(ask([question("1", "a"), question("2", "b")], selectOnly: true))
        form.markSeen(1)
        XCTAssertTrue(form.hasReachedEnd)
        XCTAssertFalse(form.canSend)
        XCTAssertEqual(form.blockedReason, .needsSelection)
    }
}
