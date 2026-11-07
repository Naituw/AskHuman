import XCTest
import AHProtocol
import AHStore
@testable import AHCore

/// Notes on passages of the message in the answer form (docs/specs/message-annotations.md): kept in the draft,
/// counted as content of the form (N8), never an answer of their own.
@MainActor
final class AnswerComposerAnnotationTests: XCTestCase {
    private var drafts: MemoryDraftStore!

    override func setUp() {
        drafts = MemoryDraftStore()
    }

    private func annotatable(
        _ questions: [QuestionView] = [question("q", "a", "b")], whatsNext: Bool = false, quickReplies: [String] = []
    ) -> AskView {
        AskView(
            isMarkdown: true, message: MessageView(text: "the message", files: []), questions: questions,
            selectOnly: false, single: false, whatsNext: whatsNext, quickReplies: quickReplies, annotatable: true
        )
    }

    private func composer(_ view: AskView, key: String = "mac/req", debounce: Duration = .milliseconds(5)) -> AnswerComposer {
        AnswerComposer(key: key, ask: view, drafts: drafts, debounce: debounce)
    }

    @discardableResult
    private func note(_ form: AnswerComposer, _ start: Int, quote: String = "passage", text: String = "why?") -> DraftAnnotation? {
        form.addAnnotation(start: start, end: start + quote.count, quote: quote, note: text)
    }

    // MARK: adding, editing, deleting

    func testANoteIsKeptWithItsPassageAndPosition() throws {
        let form = composer(annotatable())
        let added = try XCTUnwrap(form.addAnnotation(start: 12, end: 19, quote: "passage", note: "  why this?  "))
        XCTAssertEqual(added, DraftAnnotation(id: 1, start: 12, end: 19, quote: "passage", note: "why this?"))
        XCTAssertEqual(form.annotations, [added])
        XCTAssertEqual(form.annotation(id: 1), added)
        XCTAssertNil(form.annotation(id: 2))
        XCTAssertEqual(form.annotationItems, [AnnotationItem(quote: "passage", note: "why this?")])
    }

    func testNotesAreKeptInReadingOrder() {
        let form = composer(annotatable())
        note(form, 50, quote: "late", text: "third")
        note(form, 10, quote: "early", text: "first")
        note(form, 30, quote: "middle", text: "second")
        XCTAssertEqual(form.annotations.map(\.note), ["first", "second", "third"])
        XCTAssertEqual(form.annotations.map(\.id), [2, 3, 1], "numbers are made once and never change")
        XCTAssertEqual(form.annotationItems.map(\.note), ["first", "second", "third"], "the wire goes in reading order")
    }

    func testEditingAndDeletingANote() throws {
        let form = composer(annotatable())
        let first = try XCTUnwrap(note(form, 10))
        let second = try XCTUnwrap(note(form, 40, quote: "another", text: "and this?"))

        XCTAssertTrue(form.setAnnotationNote("  rewritten ", id: first.id))
        XCTAssertEqual(form.annotation(id: first.id)?.note, "rewritten")
        XCTAssertEqual(form.annotation(id: first.id)?.start, 10, "editing the text does not move the passage")
        XCTAssertFalse(form.setAnnotationNote("   \n", id: first.id), "taking a note away is deleting it")
        XCTAssertEqual(form.annotation(id: first.id)?.note, "rewritten")
        XCTAssertFalse(form.setAnnotationNote("x", id: 99))

        form.removeAnnotation(id: first.id)
        XCTAssertEqual(form.annotations.map(\.id), [second.id])
        XCTAssertTrue(form.hasContent)
        form.removeAnnotation(id: 99)
        form.removeAnnotation(id: second.id)
        XCTAssertTrue(form.annotations.isEmpty)
        XCTAssertFalse(form.hasContent, "nothing else was written")
    }

    func testTheFormTakesNoMoreThanTwentyNotes() {
        let form = composer(annotatable())
        for index in 0..<Wire.maxAnnotations {
            XCTAssertNotNil(note(form, index * 100, text: "n\(index)"))
        }
        XCTAssertTrue(form.annotationLimitReached)
        XCTAssertFalse(form.canAddAnnotation)
        XCTAssertNil(note(form, 5000, text: "one too many"))
        XCTAssertEqual(form.annotations.count, Wire.maxAnnotations)

        form.removeAnnotation(id: 3)
        XCTAssertTrue(form.canAddAnnotation)
        XCTAssertNotNil(note(form, 5000, text: "room again"))
    }

    func testNotesAreOfferedOnlyWhereTheMacSaidSo() {
        let plain = AskView(
            isMarkdown: true, message: MessageView(text: "m", files: []), questions: [question("q")],
            selectOnly: false, single: false
        )
        let form = composer(plain)
        XCTAssertFalse(form.canAddAnnotation, "a request the Mac did not mark would have its notes dropped")
        XCTAssertNil(note(form, 0))
        XCTAssertFalse(form.hasContent)
    }

    func testEmptyOrBackwardsPassagesAndEmptyNotesAreRefused() {
        let form = composer(annotatable())
        XCTAssertNil(form.addAnnotation(start: 0, end: 7, quote: "   ", note: "why?"))
        XCTAssertNil(form.addAnnotation(start: 0, end: 7, quote: "passage", note: " \n "))
        XCTAssertNil(form.addAnnotation(start: 7, end: 7, quote: "passage", note: "why?"))
        XCTAssertNil(form.addAnnotation(start: 9, end: 3, quote: "passage", note: "why?"))
        XCTAssertNil(form.addAnnotation(start: -1, end: 6, quote: "passage", note: "why?"))
        XCTAssertTrue(form.annotations.isEmpty)
    }

    func testAPassageIsFoldedToOneLineAndCutInTheMiddle() throws {
        let form = composer(annotatable())
        let folded = try XCTUnwrap(form.addAnnotation(start: 0, end: 30, quote: "  alpha\n\n beta\t gamma ", note: "x"))
        XCTAssertEqual(folded.quote, "alpha beta gamma")

        let long = String(repeating: "a", count: 1500) + String(repeating: "b", count: 1500)
        let cut = try XCTUnwrap(form.addAnnotation(start: 100, end: 3100, quote: long, note: "x"))
        XCTAssertEqual(cut.quote, String(repeating: "a", count: 999) + "…" + String(repeating: "b", count: 1000))
        XCTAssertEqual(cut.quote.unicodeScalars.count, Wire.maxQuoteChars, "the Mac keeps what is cut to this length as it is")
    }

    func testANoteIsCutToTheLimitTheMacKeeps() throws {
        let form = composer(annotatable())
        let added = try XCTUnwrap(note(form, 0, text: String(repeating: "x", count: Wire.maxNoteChars + 500)))
        XCTAssertEqual(added.note.count, Wire.maxNoteChars)
        XCTAssertTrue(form.setAnnotationNote(String(repeating: "y", count: Wire.maxNoteChars + 1), id: added.id))
        XCTAssertEqual(form.annotation(id: added.id)?.note.count, Wire.maxNoteChars)
    }

    // MARK: content, answered, sendable (N8)

    func testNotesCountAsContentButNeverAsAnAnswer() {
        let form = composer(annotatable([question("one", "a"), question("two")], quickReplies: ["Continue"]))
        XCTAssertFalse(form.hasContent)
        note(form, 4)
        XCTAssertTrue(form.hasContent, "an unsent note lights the draft mark")
        XCTAssertFalse(form.isAnswered(0))
        XCTAssertFalse(form.isAnswered(1))
        XCTAssertEqual(form.answeredCount, 0)
        XCTAssertTrue(form.canSend, "what Send needs is not changed")
        XCTAssertTrue(form.items(uploads: [:]).isEmpty, "the notes travel beside the items")
        XCTAssertEqual(form.quickReplyTargets, 2, "a quick reply still has the questions to answer")
    }

    func testNotesAndAnAnswerTravelSideBySide() {
        let form = composer(annotatable([question("one", "a", "b"), question("two")]))
        form.toggle(question: 0, option: 1)
        form.setText(" because ", question: 1)
        note(form, 8)
        XCTAssertEqual(form.items(uploads: [:]), [AnswerItem(q: 0, sel: [1]), AnswerItem(q: 1, text: "because")])
        XCTAssertEqual(form.annotationItems.count, 1)
    }

    func testWhatsNextTakesNotesToo() {
        let view = annotatable(
            [QuestionView(text: "What next?", options: [
                OptionView(text: "from the Agent", recommended: false),
                OptionView(text: "End this round", recommended: false),
            ])],
            whatsNext: true
        )
        let form = composer(view)
        note(form, 3)
        form.toggle(question: 0, option: 1)
        XCTAssertEqual(form.items(uploads: [:]), [AnswerItem(q: 0, sel: [1])])
        XCTAssertEqual(form.annotationItems.count, 1, "choosing 'End this round' does not drop the notes")
        form.setTodoIds(["t1"])
        XCTAssertEqual(form.annotationItems.count, 1, "nor does picking a todo")
    }

    // MARK: the draft

    func testNotesSurviveARestartInTheDraft() throws {
        let view = annotatable()
        let first = composer(view, debounce: .seconds(60))
        note(first, 40, quote: "second passage", text: "b")
        note(first, 5, quote: "first passage", text: "a")
        first.flush()

        let second = composer(view)
        XCTAssertEqual(second.annotations, first.annotations)
        XCTAssertEqual(second.annotations.map(\.note), ["a", "b"])
        XCTAssertTrue(second.hasContent)
        let added = try XCTUnwrap(note(second, 90, quote: "third passage", text: "c"))
        XCTAssertEqual(added.id, 3, "numbers go on from the highest of the draft")
    }

    func testEditsAreSavedAsADraftAfterTheDebounce() async throws {
        let form = composer(annotatable())
        let added = try XCTUnwrap(note(form, 4))
        await eventually { self.drafts.draft(key: "mac/req")?.annotations.count == 1 }
        form.setAnnotationNote("edited", id: added.id)
        await eventually { self.drafts.draft(key: "mac/req")?.annotations.first?.note == "edited" }
        form.removeAnnotation(id: added.id)
        await eventually("an all-empty draft is not kept") { self.drafts.draft(key: "mac/req") == nil }
    }

    func testADraftIsCleanedWhenItIsTakenBack() throws {
        var stored: [DraftAnnotation] = [
            DraftAnnotation(id: 1, start: 4, end: 9, quote: "kept", note: "fine"),
            DraftAnnotation(id: 2, start: 10, end: 14, quote: "  ", note: "no passage"),
            DraftAnnotation(id: 3, start: 20, end: 24, quote: "no note", note: " "),
            DraftAnnotation(id: 1, start: 30, end: 34, quote: "same id", note: "twice"),
            DraftAnnotation(id: 5, start: -8, end: -2, quote: "negative", note: "offsets"),
        ]
        for index in 10..<40 { stored.append(DraftAnnotation(id: index, start: index, end: index + 3, quote: "q\(index)", note: "n\(index)")) }
        drafts.save(Draft(answers: [DraftAnswer()], annotations: stored), key: "mac/req")

        let form = composer(annotatable())
        XCTAssertEqual(form.annotations.count, Wire.maxAnnotations, "no more than the limit")
        XCTAssertEqual(form.annotations.map(\.id).prefix(2), [1, 5])
        XCTAssertEqual(Set(form.annotations.map(\.id)).count, form.annotations.count, "every number once")
        XCTAssertTrue(form.annotations.allSatisfy { !$0.quote.isEmpty && !$0.note.isEmpty })
        let negative = try XCTUnwrap(form.annotation(id: 5))
        XCTAssertEqual(negative.start, 0)
        XCTAssertGreaterThan(negative.end, negative.start)
    }

    func testARequestThatOffersNoNotesTakesNoneFromADraft() {
        drafts.save(
            Draft(answers: [DraftAnswer()], annotations: [DraftAnnotation(id: 1, start: 0, end: 4, quote: "text", note: "n")]),
            key: "mac/req"
        )
        let plain = AskView(
            isMarkdown: true, message: MessageView(text: "m", files: []), questions: [question("q")],
            selectOnly: false, single: false
        )
        let form = composer(plain)
        XCTAssertTrue(form.annotations.isEmpty, "the Mac would drop them from this request")
        XCTAssertTrue(form.annotationItems.isEmpty)
    }

    func testDiscardForgetsTheNotesWithTheDraft() {
        let form = composer(annotatable(), debounce: .seconds(60))
        note(form, 4)
        form.flush()
        XCTAssertNotNil(drafts.draft(key: "mac/req"))
        form.discard()
        XCTAssertNil(drafts.draft(key: "mac/req"))
    }

    // MARK: what the page reports (the message page draws the highlights and says where it put them)

    func testThePositionsThePageFoundAreKeptInTheDraft() throws {
        let form = composer(annotatable(), debounce: .seconds(60))
        let first = try XCTUnwrap(note(form, 10, quote: "first passage"))
        let second = try XCTUnwrap(note(form, 40, quote: "second passage"))

        // The text moved (the message was drawn again, say with another theme): both passages sit elsewhere now.
        form.applyAnnotationPositions([
            .init(id: first.id, start: 12, end: 25, located: true),
            .init(id: second.id, start: 8, end: 22, located: true),
        ])
        XCTAssertEqual(form.annotations.map(\.id), [second.id, first.id], "reading order follows the new positions")
        XCTAssertEqual(form.annotation(id: first.id)?.start, 12)
        XCTAssertEqual(form.annotation(id: second.id)?.end, 22)
        form.flush()
        XCTAssertEqual(drafts.draft(key: "mac/req")?.annotations, form.annotations)
    }

    func testAReportThatChangesNothingWritesNothing() async throws {
        let counting = CountingDraftStore()
        let form = AnswerComposer(key: "mac/req", ask: annotatable(), drafts: counting, debounce: .milliseconds(1))
        let added = try XCTUnwrap(note(form, 10, quote: "first passage"))
        await eventually { counting.saves == 1 }

        let same = AnswerComposer.AnnotationPosition(id: added.id, start: 10, end: 23, located: true)
        form.applyAnnotationPositions([same])
        form.applyAnnotationPositions([same])
        try await Task.sleep(for: .milliseconds(60))
        XCTAssertEqual(counting.saves, 1, "the page reports after every drawing, and most of them say the same")

        form.applyAnnotationPositions([.init(id: added.id, start: 11, end: 24, located: true)])
        await eventually { counting.saves == 2 }
    }

    func testNotesThePageCannotFindGoLastButStillTravel() throws {
        let form = composer(annotatable())
        let first = try XCTUnwrap(note(form, 10, quote: "first passage", text: "a"))
        let second = try XCTUnwrap(note(form, 30, quote: "second passage", text: "b"))
        let third = try XCTUnwrap(note(form, 50, quote: "third passage", text: "c"))
        XCTAssertTrue(form.isLocated(first.id))

        form.applyAnnotationPositions([.init(id: first.id, start: 10, end: 23, located: false)])
        XCTAssertFalse(form.isLocated(first.id))
        XCTAssertEqual(form.annotations.map(\.id), [second.id, third.id, first.id], "no place, so after the placed ones (N3)")
        XCTAssertEqual(form.annotationItems.map(\.note), ["b", "c", "a"], "it is sent all the same: the user wrote it")
        XCTAssertEqual(form.annotation(id: first.id)?.start, 10, "its last known place stays in the draft")

        let added = try XCTUnwrap(note(form, 20, quote: "late arrival", text: "d"))
        XCTAssertEqual(form.annotations.map(\.id), [added.id, second.id, third.id, first.id], "a new note is placed before the lost ones")

        form.applyAnnotationPositions([.init(id: first.id, start: 5, end: 18, located: true)])
        XCTAssertTrue(form.isLocated(first.id))
        XCTAssertEqual(form.annotations.map(\.id), [first.id, added.id, second.id, third.id])
    }

    func testAReportAboutANoteThatIsGoneChangesNothing() throws {
        let form = composer(annotatable())
        let first = try XCTUnwrap(note(form, 10))
        form.removeAnnotation(id: first.id)
        form.applyAnnotationPositions([.init(id: first.id, start: 3, end: 9, located: true)])
        XCTAssertTrue(form.annotations.isEmpty)
        let again = try XCTUnwrap(note(form, 20))
        XCTAssertNotEqual(again.id, first.id, "a late report must not land on a new note that took the number")
        form.applyAnnotationPositions([.init(id: first.id, start: 3, end: 9, located: false)])
        XCTAssertTrue(form.isLocated(again.id))
    }

    func testAnImpossiblePositionFromThePageIsNotKept() throws {
        let form = composer(annotatable())
        let added = try XCTUnwrap(note(form, 10, quote: "first passage"))
        form.applyAnnotationPositions([.init(id: added.id, start: -4, end: 6, located: true)])
        form.applyAnnotationPositions([.init(id: added.id, start: 9, end: 9, located: true)])
        XCTAssertEqual(form.annotation(id: added.id)?.start, 10)
        XCTAssertEqual(form.annotation(id: added.id)?.end, 23)
    }
}

/// A draft store that counts the writes (the form must not save when nothing changed).
private final class CountingDraftStore: DraftStore, @unchecked Sendable {
    private let inner = MemoryDraftStore()
    private let lock = NSLock()
    private var count = 0
    var saves: Int { lock.withLock { count } }

    func draft(key: String) -> Draft? { inner.draft(key: key) }
    func save(_ draft: Draft, key: String) {
        lock.withLock { count += 1 }
        inner.save(draft, key: key)
    }
    func remove(key: String) { inner.remove(key: key) }
    func filesDirectory(key: String) -> URL { inner.filesDirectory(key: key) }
}
