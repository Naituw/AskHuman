import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// Notes written on the message reach the asking process: the Swift phone engine against the real Rust gateway in a real
/// daemon, over a local broker (docs/specs/message-annotations.md).
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=ios/Tools/local-broker swift test --filter AnnotationE2ETests
final class AnnotationE2ETests: E2ECase {
    private let message = "We could use Redis or Memcached for the session cache."
    private let note = AnnotationItem(quote: "Memcached", note: "Why not this one?")

    private func submit(
        _ item: InboxItem, action: String = "send", answers: [AnswerItem], annotations: [AnnotationItem]
    ) async throws {
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: action, answers: answers, todoIds: [], directive: nil,
            annotations: annotations
        )
    }

    func testNotesGoWithTheAnswerToTheAgent() async throws {
        try await pair()
        let ask = try mac.ask([message, "-q", "Which one?", "-o", "Redis", "-o", "Memcached"])
        let item = try await phone.request()
        XCTAssertEqual(item.view?.ask?.annotatable, true, "an Agent's own question takes notes on its message")

        try await submit(item, answers: [AnswerItem(q: 0, sel: [0])], annotations: [note])
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("[selected_options]\nRedis"), ask.stdout)
        XCTAssertTrue(ask.stdout.contains("[user_annotations]\n> Memcached\nWhy not this one?"), ask.stdout)

        await eventually("the history entry") { await self.phone.engine.history().first != nil }
        let entry = try unwrap(await phone.engine.history().first)
        XCTAssertEqual(entry.annotations, [note], "the phone keeps what it sent")
    }

    func testNotesAloneAreAnAnswer() async throws {
        try await pair()
        let ask = try mac.ask([message, "-q", "Which one?", "-o", "Redis", "-o", "Memcached"])
        let item = try await phone.request()

        try await submit(item, answers: [AnswerItem(q: 0, sel: [])], annotations: [note])
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.hasPrefix("[user_annotations]\n> Memcached\nWhy not this one?"), ask.stdout)
        XCTAssertFalse(ask.stdout.contains("[selected_options]"), "nothing was chosen: \(ask.stdout)")
    }

    /// Ending the turn from What's next, with a note: the end is not approved, the Agent is only given the note to answer.
    func testEndingTheTurnWithANoteOnlyDeliversTheNote() async throws {
        try await pair()
        let ask = try mac.ask(["--whats-next", message, "-o", "Write the docs"])
        let item = try await phone.request()
        let view = try unwrap(item.view?.ask)
        XCTAssertTrue(view.whatsNext)
        let end = try XCTUnwrap(view.questions.first?.options.indices.last)

        try await submit(item, answers: [AnswerItem(q: 0, sel: [end])], annotations: [note])
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("[user_annotations]\n> Memcached\nWhy not this one?"), ask.stdout)
        XCTAssertFalse(ask.stdout.contains("[selected_options]"), "the end was not approved: \(ask.stdout)")
    }

    func testCancellingDropsTheNotes() async throws {
        try await pair()
        let ask = try mac.ask([message, "-q", "Which one?", "-o", "Redis", "-o", "Memcached"])
        let item = try await phone.request()

        try await submit(item, action: "cancel", answers: [], annotations: [note])
        _ = await ask.wait(timeout: 20)
        XCTAssertTrue(ask.stdout.contains("canceled"), ask.stdout)
        XCTAssertFalse(ask.stdout.contains("[user_annotations]"), ask.stdout)
    }

    /// A strict choice takes the listed options only: the Mac does not offer notes for it.
    func testAStrictChoiceIsNotOfferedNotes() async throws {
        try await pair()
        let ask = try mac.ask(["--select-only", message, "-q", "Which one?", "-o", "Redis", "-o", "Memcached"])
        let item = try await phone.request()
        XCTAssertEqual(item.view?.ask?.annotatable, false)
        ask.terminate()
    }
}
