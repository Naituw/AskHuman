import XCTest
import AHProtocol
import AHStore
@testable import AHCore

/// What the Markdown page and the app say to each other about notes (`NotesPage`): script messages come out of
/// JavaScript as plain values and are taken only when they are well formed; script calls are written as text that
/// the page can read whatever the quoted text holds.
@MainActor
final class NotesPageTests: XCTestCase {
    // MARK: the page's selection

    func testASelectionIsReadWithItsPlaceAndTheNoteItTouches() {
        let body: [String: Any] = ["quote": "users first", "start": NSNumber(value: 12), "end": NSNumber(value: 23), "overlapId": NSNull()]
        XCTAssertEqual(NotesPage.selection(from: body), NotesPage.Selection(quote: "users first", start: 12, end: 23, overlapId: nil))

        let touching: [String: Any] = ["quote": "users", "start": NSNumber(value: 12), "end": NSNumber(value: 17), "overlapId": NSNumber(value: 4)]
        XCTAssertEqual(NotesPage.selection(from: touching)?.overlapId, 4)
    }

    func testNoSelectionIsNothing() {
        XCTAssertNil(NotesPage.selection(from: NSNull()), "the page sends null when nothing is selected")
        XCTAssertNil(NotesPage.selection(from: nil))
        XCTAssertNil(NotesPage.selection(from: "users first"))
        XCTAssertNil(NotesPage.selection(from: [Any]()))
    }

    func testASelectionThatIsNotWellFormedIsNotTaken() {
        func body(quote: Any? = "q", start: Any? = NSNumber(value: 1), end: Any? = NSNumber(value: 2)) -> [String: Any] {
            var fields: [String: Any] = [:]
            if let quote { fields["quote"] = quote }
            if let start { fields["start"] = start }
            if let end { fields["end"] = end }
            return fields
        }
        XCTAssertNotNil(NotesPage.selection(from: body()))
        XCTAssertNil(NotesPage.selection(from: body(quote: "")), "no passage")
        XCTAssertNil(NotesPage.selection(from: body(quote: NSNumber(value: 3))))
        XCTAssertNil(NotesPage.selection(from: body(quote: nil)))
        XCTAssertNil(NotesPage.selection(from: body(start: nil)))
        XCTAssertNil(NotesPage.selection(from: body(end: nil)))
        XCTAssertNil(NotesPage.selection(from: body(start: NSNumber(value: -1))))
        XCTAssertNil(NotesPage.selection(from: body(start: NSNumber(value: 5), end: NSNumber(value: 5))), "an empty passage")
        XCTAssertNil(NotesPage.selection(from: body(start: NSNumber(value: 6), end: NSNumber(value: 5))))
        XCTAssertNil(NotesPage.selection(from: body(start: NSNumber(value: 1.5))), "offsets are whole numbers")
        XCTAssertNil(NotesPage.selection(from: body(end: NSNumber(value: Double.nan))))
        XCTAssertNil(NotesPage.selection(from: body(end: NSNumber(value: Double.infinity))))
        XCTAssertNil(NotesPage.selection(from: body(end: NSNumber(value: 1e12))), "nothing like a text offset")
        XCTAssertNil(NotesPage.selection(from: body(start: NSNumber(value: true))), "a flag is not a number")
        XCTAssertNil(NotesPage.selection(from: body(start: "1")))
    }

    func testATappedNoteIsNamedByItsNumber() {
        XCTAssertEqual(NotesPage.noteID(from: NSNumber(value: 7)), 7)
        XCTAssertNil(NotesPage.noteID(from: NSNull()))
        XCTAssertNil(NotesPage.noteID(from: "7"))
        XCTAssertNil(NotesPage.noteID(from: NSNumber(value: true)))
        XCTAssertNil(NotesPage.noteID(from: NSNumber(value: 2.5)))
    }

    // MARK: where the page put the notes

    func testPositionsAreReadAndWhatIsNotWellFormedIsLeftOut() {
        let body: [Any] = [
            ["id": NSNumber(value: 1), "start": NSNumber(value: 5), "end": NSNumber(value: 9), "located": NSNumber(value: true)] as [String: Any],
            ["id": NSNumber(value: 2), "start": NSNumber(value: 0), "end": NSNumber(value: 4), "located": NSNumber(value: false)] as [String: Any],
            ["id": NSNumber(value: 3), "start": NSNumber(value: 0), "end": NSNumber(value: 4)] as [String: Any],
            ["id": NSNumber(value: 4), "start": NSNumber(value: 0), "end": NSNumber(value: 4), "located": NSNumber(value: 1)] as [String: Any],
            ["id": "5", "start": NSNumber(value: 0), "end": NSNumber(value: 4), "located": NSNumber(value: true)] as [String: Any],
            "nonsense",
            NSNull(),
        ]
        XCTAssertEqual(NotesPage.positions(from: body), [
            AnswerComposer.AnnotationPosition(id: 1, start: 5, end: 9, located: true),
            AnswerComposer.AnnotationPosition(id: 2, start: 0, end: 4, located: false),
        ])
        XCTAssertEqual(NotesPage.positions(from: NSNull()), [])
        XCTAssertEqual(NotesPage.positions(from: ["id": 1]), [])
        XCTAssertEqual(NotesPage.positions(from: [Any]()), [])
    }

    func testWhatThePageReportsGoesIntoTheForm() {
        let form = AnswerComposer(key: "mac/req", ask: annotatable(), drafts: MemoryDraftStore(), debounce: .milliseconds(5))
        form.addAnnotation(start: 30, end: 36, quote: "orders", note: "second")
        form.addAnnotation(start: 5, end: 11, quote: "Migrate", note: "first")
        let report: [Any] = [
            ["id": NSNumber(value: 1), "start": NSNumber(value: 2), "end": NSNumber(value: 8), "located": NSNumber(value: true)] as [String: Any],
            ["id": NSNumber(value: 2), "start": NSNumber(value: 5), "end": NSNumber(value: 11), "located": NSNumber(value: false)] as [String: Any],
        ]
        form.applyAnnotationPositions(NotesPage.positions(from: report))
        XCTAssertEqual(form.annotations.map(\.note), ["second", "first"], "the one the page found goes first, the one it lost last")
        XCTAssertEqual(form.annotation(id: 1)?.start, 2)
        XCTAssertFalse(form.isLocated(2))
    }

    // MARK: what the page is told

    /// The argument of a call such as `AH.setNotes([…])`, read as JSON.
    private func argument(of script: String, call: String) throws -> Any {
        let prefix = "\(call)("
        XCTAssertTrue(script.hasPrefix(prefix), script)
        XCTAssertTrue(script.hasSuffix(")"), script)
        let text = String(script.dropFirst(prefix.count).dropLast())
        return try JSONSerialization.jsonObject(with: Data(text.utf8), options: [.fragmentsAllowed])
    }

    func testTheNotesAreHandedOverAsAList() throws {
        let notes = [
            NotesPage.Note(id: 2, start: 5, end: 16, quote: "users first"),
            NotesPage.Note(id: 7, start: 30, end: 36, quote: "orders"),
        ]
        let list = try XCTUnwrap(try argument(of: NotesPage.setNotes(notes), call: "AH.setNotes") as? [[String: Any]])
        XCTAssertEqual(list.count, 2)
        XCTAssertEqual(list[0]["id"] as? Int, 2)
        XCTAssertEqual(list[0]["start"] as? Int, 5)
        XCTAssertEqual(list[0]["end"] as? Int, 16)
        XCTAssertEqual(list[0]["quote"] as? String, "users first")
        XCTAssertEqual(list[1]["quote"] as? String, "orders")
        XCTAssertEqual(NotesPage.setNotes([]), "AH.setNotes([])")
    }

    func testAQuoteWithAnythingInItStillReadsAsOneString() throws {
        let nasty = "say \"hi\" \\ and\nthen\ttab </script> 🙂 \u{2028} 中文 'single'"
        let script = NotesPage.setNotes([NotesPage.Note(id: 1, start: 0, end: 4, quote: nasty)])
        XCTAssertFalse(script.contains("\n"), "a line break inside the call would end the statement")
        let list = try XCTUnwrap(try argument(of: script, call: "AH.setNotes") as? [[String: Any]])
        XCTAssertEqual(list.first?["quote"] as? String, nasty)
    }

    func testThePassageBeingWrittenIsNamedOrTakenAway() throws {
        XCTAssertEqual(NotesPage.setPending(nil), "AH.setPending(null)")
        let anchor = try XCTUnwrap(try argument(of: NotesPage.setPending(NotesPage.Anchor(start: 3, end: 9)), call: "AH.setPending") as? [String: Int])
        XCTAssertEqual(anchor, ["start": 3, "end": 9])
    }

    func testTheCallsThatTakeANoteNumber() {
        XCTAssertEqual(NotesPage.locate(4), "AH.locate(4)")
        XCTAssertEqual(NotesPage.flash(4), "AH.flash(4)")
        XCTAssertEqual(NotesPage.clearSelection, "AH.clearSelection()")
    }

    // MARK: what the menu offers

    private func annotatable(_ yes: Bool = true) -> AskView {
        AskView(
            isMarkdown: true, message: MessageView(text: "the message", files: []),
            questions: [QuestionView(text: "q", options: [OptionView(text: "a", recommended: false)])],
            selectOnly: false, single: false, annotatable: yes
        )
    }

    private func form(_ view: AskView) -> AnswerComposer {
        AnswerComposer(key: "mac/req", ask: view, drafts: MemoryDraftStore(), debounce: .milliseconds(5))
    }

    private let selected = NotesPage.Selection(quote: "users first", start: 12, end: 23)

    func testTheMenuOffersToAddWhereNotesAreTaken() {
        XCTAssertEqual(form(annotatable()).annotationOffer(for: selected), .add(selected))
    }

    func testTheMenuOffersNothingWithoutASelectionOrForARequestThatTakesNoNotes() {
        XCTAssertNil(form(annotatable()).annotationOffer(for: nil))
        XCTAssertNil(form(annotatable(false)).annotationOffer(for: selected))
    }

    func testAPassageThatIsANoteIsOfferedForEditing() throws {
        let notes = form(annotatable())
        let note = try XCTUnwrap(notes.addAnnotation(start: 12, end: 17, quote: "users", note: "why"))
        var touching = selected
        touching.overlapId = note.id
        XCTAssertEqual(notes.annotationOffer(for: touching), .edit(id: note.id))

        // A page that is behind (it names a note that is gone) does not stop a new one.
        touching.overlapId = 99
        XCTAssertEqual(notes.annotationOffer(for: touching), .add(touching))
    }

    func testAFullFormOffersNoMoreButStillEdits() throws {
        let notes = form(annotatable())
        for index in 0..<Wire.maxAnnotations {
            XCTAssertNotNil(notes.addAnnotation(start: index * 10, end: index * 10 + 5, quote: "note \(index)", note: "n"))
        }
        XCTAssertEqual(notes.annotationOffer(for: selected), .full)
        var touching = selected
        touching.overlapId = try XCTUnwrap(notes.annotations.first).id
        XCTAssertEqual(notes.annotationOffer(for: touching), .edit(id: touching.overlapId!))
    }

    func testTheNotesAreGivenToThePageInReadingOrder() {
        let notes = form(annotatable())
        notes.addAnnotation(start: 30, end: 36, quote: "orders", note: "second")
        notes.addAnnotation(start: 5, end: 12, quote: "Migrate", note: "first")
        XCTAssertEqual(notes.pageNotes, [
            NotesPage.Note(id: 2, start: 5, end: 12, quote: "Migrate"),
            NotesPage.Note(id: 1, start: 30, end: 36, quote: "orders"),
        ])
    }
}
