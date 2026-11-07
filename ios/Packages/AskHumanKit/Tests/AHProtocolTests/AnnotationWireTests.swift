import XCTest
@testable import AHProtocol

/// Notes on the message on the wire (docs/specs/message-annotations.md). Every field is additive in both
/// directions, so a Mac and a phone of different ages exchange exactly what each of them knows.
final class AnnotationWireTests: XCTestCase {
    private func caps(annotations: Bool) -> Caps {
        Caps(newTask: true, interject: true, watch: true, todos: true, answerTodos: true, away: true, annotations: annotations)
    }

    private func ask(annotatable: Bool) -> AskView {
        AskView(
            isMarkdown: true, message: MessageView(text: "## Plan", files: []),
            questions: [QuestionView(text: "Which one?", options: [])], selectOnly: false, single: false,
            annotatable: annotatable
        )
    }

    // MARK: capability

    func testCapsLeaveTheNotesBitOutWhenFalse() throws {
        XCTAssertNil(try AHJSON.tree(caps(annotations: false))["annotations"], "what predates notes sees what it always saw")
        XCTAssertEqual(try AHJSON.tree(caps(annotations: true))["annotations"], .bool(true))
    }

    func testAMissingNotesBitReadsAsNo() throws {
        let old = try AHJSON.decoder.decode(
            Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true,"todos":true}"#.utf8)
        )
        XCTAssertFalse(old.annotations)
        let new = try AHJSON.decoder.decode(
            Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true,"annotations":true}"#.utf8)
        )
        XCTAssertTrue(new.annotations)
        // A pairing stored by a build that knew nothing about notes still opens.
        XCTAssertEqual(try AHJSON.value(Caps.self, from: try AHJSON.tree(caps(annotations: true))), caps(annotations: true))
        XCTAssertEqual(try AHJSON.value(Caps.self, from: try AHJSON.tree(caps(annotations: false))), caps(annotations: false))
    }

    // MARK: annotatable

    func testAnAskIsMarkedAnnotatableOnlyWhenTheMacSaysSo() throws {
        XCTAssertFalse(ask(annotatable: false).annotatable)
        XCTAssertNil(try AHJSON.tree(ask(annotatable: false))["annotatable"], "an old phone is not sent a flag it never read")
        XCTAssertEqual(try AHJSON.tree(ask(annotatable: true))["annotatable"], .bool(true))
        for annotatable in [false, true] {
            let view = ask(annotatable: annotatable)
            XCTAssertEqual(try AHJSON.value(AskView.self, from: try AHJSON.tree(view)), view)
        }
    }

    func testAnAskFromAMacThatPredatesNotesIsNotAnnotatable() throws {
        guard case .object(var fields) = try AHJSON.tree(ask(annotatable: true)) else { return XCTFail("an object") }
        fields["annotatable"] = nil
        XCTAssertFalse(try AHJSON.value(AskView.self, from: .object(fields)).annotatable)
    }

    // MARK: the answer

    func testAnAnswerCarriesNotesOnlyWhenThereAreSome() throws {
        let plain = AnswerSubmit(clientMsgId: "c", requestId: "r", action: "send", answers: [AnswerItem(q: 0, sel: [0])])
        XCTAssertNil(try AHJSON.tree(plain)["annotations"], "an older Mac must not see a field it does not know")

        let noted = AnswerSubmit(
            clientMsgId: "c", requestId: "r", action: "send", answers: [],
            annotations: [AnnotationItem(quote: "passage", note: "why?"), AnnotationItem(quote: "other", note: "and this")]
        )
        XCTAssertEqual(
            try AHJSON.tree(noted)["annotations"],
            .array([
                .object(["quote": .string("passage"), "note": .string("why?")]),
                .object(["quote": .string("other"), "note": .string("and this")]),
            ])
        )
        XCTAssertEqual(try AHJSON.value(AnswerSubmit.self, from: try AHJSON.tree(noted)), noted)
    }

    func testAnAnswerFromBeforeNotesHasNone() throws {
        let legacy: JSONValue = .object([
            "clientMsgId": .string("c"), "requestId": .string("r"), "action": .string("send"), "answers": .array([]),
        ])
        XCTAssertTrue(try AHJSON.value(AnswerSubmit.self, from: legacy).annotations.isEmpty)
    }

    func testTheNoteLimitsAreTheMacs() {
        // `MAX_ANNOTATIONS`, `MAX_NOTE_CHARS` and `MAX_QUOTE_CHARS` in `src-tauri/src/annotations.rs`.
        XCTAssertEqual(Wire.maxAnnotations, 20)
        XCTAssertEqual(Wire.maxNoteChars, 2000)
        XCTAssertEqual(Wire.maxQuoteChars, 2000)
    }
}
