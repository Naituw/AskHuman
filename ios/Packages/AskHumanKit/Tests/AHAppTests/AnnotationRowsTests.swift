import XCTest
import AHProtocol
import AHStore
@testable import AHCore

/// The notes as the list on the answer screen shows them (docs/specs/message-annotations.md N3, §7.4): the numbers
/// are the ones the page draws on the passages.
@MainActor
final class AnnotationRowsTests: XCTestCase {
    private func composer() -> AnswerComposer {
        let view = AskView(
            isMarkdown: true, message: MessageView(text: "the message", files: []), questions: [question("q", "a", "b")],
            selectOnly: false, single: false, annotatable: true
        )
        return AnswerComposer(key: "mac/req", ask: view, drafts: MemoryDraftStore(), debounce: .milliseconds(5))
    }

    /// `number note` for each row, `-` for a note without a number.
    private func shown(_ form: AnswerComposer) -> [String] {
        form.annotationRows.map { "\($0.number.map(String.init) ?? "-") \($0.note)" }
    }

    func testNoNotesNoRows() {
        XCTAssertTrue(composer().annotationRows.isEmpty)
    }

    func testNotesAreNumberedInReadingOrderWhateverOrderTheyWereWrittenIn() {
        let form = composer()
        form.addAnnotation(start: 50, end: 54, quote: "late", note: "third")
        form.addAnnotation(start: 10, end: 15, quote: "early", note: "first")
        form.addAnnotation(start: 30, end: 36, quote: "middle", note: "second")
        XCTAssertEqual(shown(form), ["1 first", "2 second", "3 third"])
        XCTAssertEqual(form.annotationRows.map(\.quote), ["early", "middle", "late"])
    }

    func testARowKeepsTheIdOfItsNote() throws {
        let form = composer()
        let later = try XCTUnwrap(form.addAnnotation(start: 40, end: 44, quote: "late", note: "b"))
        let earlier = try XCTUnwrap(form.addAnnotation(start: 4, end: 9, quote: "early", note: "a"))
        XCTAssertEqual(form.annotationRows.map(\.id), [earlier.id, later.id], "the list edits and shows the note by its id")
    }

    func testANoteTheMessageCouldNotLocateHasNoNumberAndComesLast() throws {
        let form = composer()
        form.addAnnotation(start: 10, end: 15, quote: "early", note: "first")
        let lost = try XCTUnwrap(form.addAnnotation(start: 30, end: 36, quote: "middle", note: "second"))
        form.addAnnotation(start: 50, end: 54, quote: "late", note: "third")
        form.applyAnnotationPositions([.init(id: lost.id, start: 30, end: 36, located: false)])

        XCTAssertEqual(shown(form), ["1 first", "2 third", "- second"], "the page numbers what it found: the note it lost is skipped")
    }

    func testNumbersCloseRanksWhenANoteIsDeleted() throws {
        let form = composer()
        let first = try XCTUnwrap(form.addAnnotation(start: 10, end: 15, quote: "early", note: "first"))
        form.addAnnotation(start: 30, end: 36, quote: "middle", note: "second")
        form.addAnnotation(start: 50, end: 54, quote: "late", note: "third")
        form.removeAnnotation(id: first.id)
        XCTAssertEqual(shown(form), ["1 second", "2 third"])
    }

    func testANoteTheMessageFindsAgainGetsItsNumberBack() throws {
        let form = composer()
        let note = try XCTUnwrap(form.addAnnotation(start: 10, end: 15, quote: "early", note: "first"))
        form.addAnnotation(start: 30, end: 36, quote: "middle", note: "second")
        form.applyAnnotationPositions([.init(id: note.id, start: 10, end: 15, located: false)])
        XCTAssertEqual(shown(form), ["1 second", "- first"])
        form.applyAnnotationPositions([.init(id: note.id, start: 10, end: 15, located: true)])
        XCTAssertEqual(shown(form), ["1 first", "2 second"])
    }

    func testBeforeThePageReportsEveryNoteCountsAsLocated() {
        // A note is lost only when the page says so: until it has drawn the message, none is.
        let form = composer()
        form.addAnnotation(start: 10, end: 15, quote: "early", note: "first")
        XCTAssertEqual(shown(form), ["1 first"])
    }
}
