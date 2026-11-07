import XCTest
import AHProtocol
@testable import AHCore

/// Which requests the phone shows as one question in the message area (docs/specs/message-annotations.md N1): four things
/// decide it, and nothing else does.
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

    func testAFileMakesItAnOrdinaryAsk() {
        XCTAssertFalse(ask(files: [file]).isDirectAsk, "the files are the message's, and the message area has them")
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

    func testOnlyFilesLeaveTheMessageAreaWithoutText() {
        XCTAssertEqual(ask(files: [file]).messageAreaText, "", "there are files to show, and no words")
    }
}
