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

    func testAQuestionWithoutOptionsIsABareInputOnlyWhereTextIsTaken() {
        let form = composer(ask([question("choose", "a", "b"), question("write")]))
        XCTAssertFalse(form.isTextOnly(0), "with options, the text is a note beside a choice")
        XCTAssertTrue(form.isTextOnly(1))
        XCTAssertFalse(form.isTextOnly(5), "not a question of the form")
        let strict = composer(ask([question("choose", "a", "b"), question("write")], selectOnly: true), key: "mac/strict")
        XCTAssertFalse(strict.isTextOnly(1), "a select-only form takes no text at all")
    }

    // MARK: reading progress (spec multi-question-vertical V1/V2)

    func testASingleQuestionIsReadFromTheStart() {
        let form = composer(ask([question("q", "a")]))
        XCTAssertTrue(form.hasReachedEnd, "nothing to scroll to, so nothing to hold Send back")
    }

    func testSeveralQuestionsCannotBeSentBeforeTheLastWasSeen() {
        let form = composer(ask([question("1", "a"), question("2", "b"), question("3", "c")]))
        XCTAssertFalse(form.hasReachedEnd)
        form.markSeen(0)
        XCTAssertFalse(form.hasReachedEnd, "seeing the first ones is not enough")
        form.markSeen(2)
        XCTAssertTrue(form.hasReachedEnd, "the last question was seen: Send appears, as in the popup")
        XCTAssertFalse(form.hasRead(1), "the one in between was not seen, and nothing skips it")
    }

    func testTheEndStaysReachedOnceItWasSeen() {
        let form = composer(ask([question("1", "a"), question("2", "b")]))
        form.markSeen(1)
        XCTAssertTrue(form.hasReachedEnd)
        // Scrolling back up does not take Send away: only what was seen is recorded, never what is on screen.
        XCTAssertTrue(form.hasReachedEnd)
        XCTAssertEqual(form.seen, [1])
    }

    func testAnAnsweredQuestionCountsAsRead() {
        let form = composer(ask([question("1", "a"), question("2", "b")]))
        form.toggle(question: 1, option: 0)
        XCTAssertTrue(form.hasReachedEnd, "an answer takes a look at the question")
        XCTAssertFalse(form.seen.contains(1), "reading is only inferred, never recorded by answering")
    }

    func testASelectOnlyFormPointsAtTheFirstQuestionWithoutASelection() {
        let form = composer(ask([question("1", "a", "b"), question("2", "c", "d"), question("3", "e", "f")], selectOnly: true))
        XCTAssertEqual(form.firstWithoutSelection, 0)
        form.toggle(question: 0, option: 1)
        form.toggle(question: 2, option: 0)
        XCTAssertEqual(form.firstWithoutSelection, 1, "the gap in the middle, not the end")
        form.toggle(question: 1, option: 0)
        XCTAssertNil(form.firstWithoutSelection, "nothing is missing any more")
    }

    func testAFormThatTakesTextHasNoQuestionToPointAt() {
        let form = composer(ask([question("1", "a"), question("2", "b")]))
        XCTAssertNil(form.firstWithoutSelection, "an unanswered question does not hold such a form back")
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

    // MARK: project todos (docs/specs/special-replies.md D28/D29)

    private func whatsNext() -> AskView {
        AskView(
            isMarkdown: true, message: MessageView(text: "done", files: []),
            questions: [QuestionView(text: "What next?", options: [
                OptionView(text: "from the Agent", recommended: false),
                OptionView(text: "⚡ a queued todo", recommended: false, todo: true),
                OptionView(text: "End this round", recommended: false),
            ])],
            selectOnly: false, single: true, whatsNext: true
        )
    }

    func testPickedTodosAnswerTheLastQuestionAndAreNotAnItemOfTheirOwn() {
        let form = composer(ask([question("1", "a"), question("2", "b")]))
        XCTAssertFalse(form.hasContent)
        form.setTodoIds(["t1", "t2"])
        XCTAssertEqual(form.todoIds, ["t1", "t2"], "in the order the picker lists them")
        XCTAssertTrue(form.hasContent)
        XCTAssertTrue(form.isAnswered(1), "they go into the last question's answer")
        XCTAssertFalse(form.isAnswered(0))
        XCTAssertEqual(form.answeredCount, 1)
        XCTAssertTrue(form.items(uploads: [:]).isEmpty, "the ids travel beside the items")
        XCTAssertTrue(form.isTodoPicked("t2"))
        XCTAssertFalse(form.isTodoPicked("t3"))
    }

    func testAnAnswerWithTodosAndTextKeepsBothAndTheTextOnItsOwnItem() {
        let form = composer(ask([question("1", "a")]))
        form.setTodoIds(["t1"])
        form.setText("  because  ", question: 0)
        let items = form.items(uploads: [:])
        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items[0].text, "because")
        XCTAssertEqual(form.todoIds, ["t1"])
    }

    func testASelectOnlyFormTakesNoTodos() {
        let form = composer(ask([question("1", "a", "b")], selectOnly: true))
        form.setTodoIds(["t1"])
        XCTAssertTrue(form.todoIds.isEmpty)
        XCTAssertFalse(form.hasContent)
    }

    func testPickedTodosSurviveARestartInTheDraft() {
        let view = ask([question("1", "a")])
        let first = composer(view, debounce: .seconds(60))
        first.setTodoIds(["t1", "t2"])
        first.flush()
        let second = composer(view)
        XCTAssertEqual(second.todoIds, ["t1", "t2"])
        XCTAssertTrue(second.hasContent)
    }

    func testADraftWithOnlyTodosIsNotEmptyAndOneFromBeforeTodosHasNone() throws {
        XCTAssertFalse(Draft(answers: [DraftAnswer()], todoIds: ["t1"]).isEmpty)
        XCTAssertTrue(Draft(answers: [DraftAnswer()]).isEmpty)
        let old = #"{"answers":[{"selected":[0],"text":"","attachments":[]}],"updatedAt":0}"#
        let draft = try JSONDecoder().decode(Draft.self, from: Data(old.utf8))
        XCTAssertEqual(draft.todoIds, [])
        XCTAssertEqual(draft.answers.count, 1)
    }

    func testLettingGoOfTheLastTodoEmptiesTheFormAgain() {
        let form = composer(ask([question("1", "a")]))
        form.setTodoIds(["t1"])
        form.setTodoIds([])
        XCTAssertFalse(form.hasContent)
        XCTAssertFalse(form.isAnswered(0))
    }

    func testTodosThatAreNoLongerInTheQueueAreLetGoOf() {
        let form = composer(ask([question("1", "a")]))
        form.setTodoIds(["t1", "t2", "t3"])
        form.keepTodos(in: ["t1", "t3", "t9"])
        XCTAssertEqual(form.todoIds, ["t1", "t3"])
    }

    func testWhatsNextTakesOneTodoAndALaterChoiceReplacesIt() {
        let form = composer(whatsNext())
        form.toggle(question: 0, option: 0)
        form.setTodoIds(["t1", "t2"])
        XCTAssertEqual(form.todoIds, ["t1"], "the next task is one thing")
        XCTAssertEqual(form.answers[0].selected, [], "a todo takes the place of the option that was chosen")
        form.toggle(question: 0, option: 2)
        XCTAssertEqual(form.todoIds, [], "an option takes the place of the todo")
        XCTAssertEqual(form.answers[0].selected, [2])
    }

    func testMultiChoiceWhatsNextKeepsOptionsAndSeveralTodosUntilEndIsChosen() {
        var view = whatsNext()
        view.single = false
        let form = composer(view)
        form.toggle(question: 0, option: 0)
        form.toggle(question: 0, option: 1)
        form.setTodoIds(["t1", "t2"])
        XCTAssertEqual(form.answers[0].selected, [0, 1])
        XCTAssertEqual(form.todoIds, ["t1", "t2"])
        form.toggle(question: 0, option: 2)
        XCTAssertEqual(form.answers[0].selected, [2])
        XCTAssertTrue(form.todoIds.isEmpty)
        form.toggle(question: 0, option: 0)
        XCTAssertEqual(form.answers[0].selected, [0], "a task replaces the end choice")
        form.toggle(question: 0, option: 2)
        form.setTodoIds(["t2", "t3"])
        XCTAssertTrue(form.answers[0].selected.isEmpty, "picked todos replace the end choice")
        XCTAssertEqual(form.todoIds, ["t2", "t3"])
    }

    func testMultiChoiceWhatsNextRestoresEveryPickedTaskFromItsDraft() {
        var view = whatsNext()
        view.single = false
        let first = composer(view, debounce: .seconds(60))
        first.toggle(question: 0, option: 0)
        first.toggle(question: 0, option: 1)
        first.setTodoIds(["t1", "t2"])
        first.flush()
        let restored = composer(view)
        XCTAssertEqual(restored.answers[0].selected, [0, 1])
        XCTAssertEqual(restored.todoIds, ["t1", "t2"])
    }

    func testOtherAsksTakeAsManyTodosAsTheQueueHolds() {
        let form = composer(ask([question("1", "a")]))
        let ids = (0..<Wire.maxPickedTodos + 5).map { "t\($0)" }
        form.setTodoIds(ids)
        XCTAssertEqual(form.todoIds.count, Wire.maxPickedTodos)
    }

    func testTheTodosAWhatsNextQuestionCarriesAreLetGoOfWhenTheQueueTakesTheirPlace() {
        let form = composer(whatsNext())
        form.toggle(question: 0, option: 1)
        form.toggle(question: 0, option: 0)
        XCTAssertEqual(form.answers[0].selected, [0], "single choice: the last one stays")
        form.toggle(question: 0, option: 1)
        XCTAssertEqual(form.answers[0].selected, [1])
        form.dropTodoOptionSelections()
        XCTAssertEqual(form.answers[0].selected, [], "a hidden choice would be sent without being seen")
        form.toggle(question: 0, option: 0)
        form.dropTodoOptionSelections()
        XCTAssertEqual(form.answers[0].selected, [0], "an option the Agent offered stays")
    }

    func testOnlyWhatsNextDropsTheOptionsThatStandForTodos() {
        let view = AskView(
            isMarkdown: true, message: MessageView(text: "m", files: []),
            questions: [QuestionView(text: "q", options: [OptionView(text: "a", recommended: false, todo: true)])],
            selectOnly: false, single: true
        )
        let form = composer(view)
        form.toggle(question: 0, option: 0)
        form.dropTodoOptionSelections()
        XCTAssertEqual(form.answers[0].selected, [0], "an ordinary question's options are its own")
    }

    // MARK: the Stop confirmation as a live queue (docs/specs/todo-list-actions.md D10)

    private func stopQueue() -> AskView {
        AskView(
            isMarkdown: true, message: MessageView(text: "wants to stop", files: []),
            questions: [QuestionView(text: "What next?", options: [
                OptionView(text: "a queued todo", recommended: false, todo: true),
                OptionView(text: "Continue the conversation", recommended: false),
                OptionView(text: "End the conversation", recommended: false),
            ])],
            selectOnly: false, single: true, todoQueue: true
        )
    }

    func testOnlyWhatsNextAndTheStopQueueListTheLiveQueue() {
        XCTAssertTrue(whatsNext().listsTodoQueue)
        XCTAssertTrue(stopQueue().listsTodoQueue)
        XCTAssertFalse(ask([question("1", "a")]).listsTodoQueue)
    }

    func testAStopQueueTakesOneTodoAndALaterChoiceReplacesIt() {
        let form = composer(stopQueue())
        form.setTodoIds(["t1", "t2"])
        XCTAssertEqual(form.todoIds, ["t1"], "what comes next is one thing, as in What's next")
        form.toggle(question: 0, option: 1)
        XCTAssertEqual(form.todoIds, [], "\"continue\" takes the todo's place")
        XCTAssertEqual(form.answers[0].selected, [1])
        form.setTodoIds(["t3"])
        XCTAssertEqual(form.answers[0].selected, [], "and a todo takes the place of the choice")
        form.toggle(question: 0, option: 2)
        XCTAssertEqual(form.todoIds, [], "so does \"end\"")
        XCTAssertEqual(form.answers[0].selected, [2])
    }

    func testAStopQueueLetsGoOfTheTodoOptionsItsQuestionCarried() {
        let form = composer(stopQueue())
        form.toggle(question: 0, option: 0)
        form.dropTodoOptionSelections()
        XCTAssertEqual(form.answers[0].selected, [], "its row is not on screen once the queue is listed")
        form.toggle(question: 0, option: 1)
        form.dropTodoOptionSelections()
        XCTAssertEqual(form.answers[0].selected, [1], "a choice the Agent offered stays")
    }

    func testAStopQueueRestoresThePickedTodoFromItsDraft() {
        let view = stopQueue()
        let first = composer(view, debounce: .seconds(60))
        first.setTodoIds(["t1"])
        first.flush()
        XCTAssertEqual(composer(view).todoIds, ["t1"])
    }

    // MARK: quick replies (docs/specs/special-replies.md D18/D27)

    private func repliesAsk(_ questions: [QuestionView], selectOnly: Bool = false, whatsNext: Bool = false) -> AskView {
        AskView(
            isMarkdown: true, message: MessageView(text: "m", files: []), questions: questions,
            selectOnly: selectOnly, single: false, whatsNext: whatsNext, quickReplies: ["Continue", "Skip this one"]
        )
    }

    func testAQuickReplyAnswersEveryQuestionThatHasNoAnswerYet() {
        let form = composer(repliesAsk([question("1", "a", "b"), question("2"), question("3")]))
        XCTAssertEqual(form.quickReplies, ["Continue", "Skip this one"])
        XCTAssertEqual(form.quickReplyTargets, 3)
        form.toggle(question: 0, option: 1)
        XCTAssertEqual(form.quickReplyTargets, 2)
        XCTAssertEqual(form.items(uploads: [:], fillingWith: "Continue"), [
            AnswerItem(q: 0, sel: [1]),
            AnswerItem(q: 1, text: "Continue"),
            AnswerItem(q: 2, text: "Continue"),
        ])
        XCTAssertEqual(form.answers[1].text, "", "the form itself is not changed: a failed send leaves the draft as it was")
    }

    func testAnAnsweredQuestionKeepsItsTextAndAttachmentsWhenAReplyGoesOut() throws {
        let form = composer(repliesAsk([question("1"), question("2")]))
        form.setText("  my own words  ", question: 0)
        let image = try form.addAttachment(name: "p.png", mime: "image/png", data: Data([1]), isImage: true, question: 0)
        XCTAssertEqual(form.items(uploads: [image.id: "up-1"], fillingWith: "Continue"), [
            AnswerItem(q: 0, text: "my own words", images: ["up-1"]),
            AnswerItem(q: 1, text: "Continue"),
        ])
    }

    func testWithEveryQuestionAnsweredThereIsNothingForAReplyToAnswer() {
        let form = composer(repliesAsk([question("1", "a"), question("2")]))
        form.toggle(question: 0, option: 0)
        form.setText("typed", question: 1)
        XCTAssertEqual(form.quickReplyTargets, 0, "the menu greys the replies out")
        XCTAssertEqual(form.items(uploads: [:], fillingWith: "Continue"), form.items(uploads: [:]))
    }

    func testAPickedTodoAnswersTheLastQuestionSoAReplyLeavesItAlone() {
        let form = composer(repliesAsk([question("1"), question("2")]))
        form.setTodoIds(["t1"])
        XCTAssertEqual(form.quickReplyTargets, 1)
        XCTAssertFalse(form.quickReplyFills(1))
        XCTAssertEqual(form.items(uploads: [:], fillingWith: "Continue"), [AnswerItem(q: 0, text: "Continue")])
    }

    func testQuickRepliesBelongToOrdinaryQuestionsOnly() {
        let strict = composer(repliesAsk([question("1", "a", "b")], selectOnly: true))
        XCTAssertTrue(strict.quickReplies.isEmpty, "a strict choice takes no free text")
        XCTAssertEqual(strict.quickReplyTargets, 0)
        let next = composer(repliesAsk([question("What next?", "a", "b")], whatsNext: true))
        XCTAssertTrue(next.quickReplies.isEmpty, "the next task is not answered with a sentence")
        let plain = composer(ask([question("1", "a")]))
        XCTAssertTrue(plain.quickReplies.isEmpty, "a Mac with no replies offers none")
        XCTAssertEqual(plain.quickReplyTargets, 0)
    }

    func testOnlyAReplyOfTheRequestFillsIn() {
        let form = composer(repliesAsk([question("1")]))
        XCTAssertTrue(form.items(uploads: [:], fillingWith: "Something else").isEmpty, "not one of the Mac's replies")
        XCTAssertTrue(form.items(uploads: [:], fillingWith: "   ").isEmpty)
        XCTAssertTrue(form.items(uploads: [:], fillingWith: nil).isEmpty)
    }
}
