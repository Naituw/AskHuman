import XCTest
import AHProtocol
@testable import AHCore

/// Which requests the phone shows as one question in the message area (docs/specs/message-annotations.md N1): three things
/// decide it, and nothing else does. Files attached to the message are not one of them (D2, 2026-11-16).
final class DirectQuestionTests: XCTestCase {
    private func ask(
        message: String = "", files: [AssetRef] = [], questions: [QuestionView] = [question("Ship it?", "Yes", "No")],
        whatsNext: Bool = false, selectOnly: Bool = false, markdown: Bool = true, annotatable: Bool = true
    ) -> AskView {
        AskView(
            isMarkdown: markdown, message: MessageView(text: message, files: files), questions: questions,
            selectOnly: selectOnly, single: false, whatsNext: whatsNext, annotatable: annotatable
        )
    }

    private let file = AssetRef(assetId: "a", name: "plan.md", size: 10, mime: "text/markdown", isImage: false)

    func testOneQuestionAndNothingElseIsADirectAsk() {
        XCTAssertTrue(ask().isDirectAsk)
    }

    func testAMessageMakesItAnOrdinaryAsk() {
        XCTAssertFalse(ask(message: "Staging is healthy.").isDirectAsk)
    }

    func testAMessageOfOnlyBlanksDoesNot() {
        XCTAssertTrue(ask(message: "  \n\t ").isDirectAsk, "what is trimmed away is not a message")
    }

    func testFilesAreNotAMessage() {
        XCTAssertTrue(ask(files: [file]).isDirectAsk, "files are not words: the question is what there is to read")
    }

    func testAMessageBesideFilesIsStillAMessage() {
        XCTAssertFalse(ask(message: "Staging is healthy.", files: [file]).isDirectAsk)
    }

    func testFilesDoNotMakeSeveralQuestionsOrWhatsNextADirectAsk() {
        XCTAssertFalse(ask(files: [file], questions: [question("First?", "a"), question("Second?", "b")]).isDirectAsk)
        XCTAssertFalse(ask(files: [file], whatsNext: true).isDirectAsk)
    }

    func testTwoQuestionsAreNotOne() {
        XCTAssertFalse(ask(questions: [question("First?", "a"), question("Second?", "b")]).isDirectAsk)
    }

    func testNoQuestionIsNotOne() {
        XCTAssertFalse(ask(questions: []).isDirectAsk)
    }

    func testWhatsNextKeepsItsQuestionWhereItWas() {
        XCTAssertFalse(ask(whatsNext: true).isDirectAsk)
    }

    // MARK: what does not matter

    func testAStrictChoiceIsOneToo() {
        XCTAssertTrue(ask(selectOnly: true, annotatable: false).isDirectAsk)
    }

    func testPlainTextIsOneToo() {
        XCTAssertTrue(ask(markdown: false).isDirectAsk)
    }

    func testAMacThatTakesNoNotesMakesNoDifference() {
        XCTAssertTrue(ask(annotatable: false).isDirectAsk)
        XCTAssertEqual(ask(annotatable: false).isDirectAsk, ask(annotatable: true).isDirectAsk)
    }

    // MARK: the text of the message area

    func testTheMessageAreaHasTheMessage() {
        XCTAssertEqual(ask(message: "  Staging is healthy.\n").messageAreaText, "Staging is healthy.")
    }

    func testTheMessageAreaOfADirectAskHasItsQuestion() {
        XCTAssertEqual(ask(questions: [question("  Ship it?\n", "Yes")]).messageAreaText, "Ship it?")
    }

    func testTheMessageAreaOfAMessageIsNotTheQuestion() {
        XCTAssertEqual(ask(message: "Staging is healthy.").messageAreaText, "Staging is healthy.", "the question stays in its card")
    }

    func testTheMessageAreaOfWhatsNextWithNoMessageIsEmpty() {
        XCTAssertEqual(ask(whatsNext: true).messageAreaText, "")
    }

    func testTheMessageAreaOfTwoQuestionsWithNoMessageIsEmpty() {
        XCTAssertEqual(ask(questions: [question("First?", "a"), question("Second?", "b")]).messageAreaText, "")
    }

    func testFilesBesideALoneQuestionLeaveItsTextInTheMessageArea() {
        XCTAssertEqual(ask(files: [file]).messageAreaText, "Ship it?", "the area lists the files under this text")
    }

    func testFilesAloneLeaveTheMessageAreaWithoutText() {
        XCTAssertEqual(ask(files: [file], whatsNext: true).messageAreaText, "", "there are files to show, and no words")
        XCTAssertEqual(
            ask(files: [file], questions: [question("First?", "a"), question("Second?", "b")]).messageAreaText, "",
            "two questions keep their cards"
        )
    }
}
