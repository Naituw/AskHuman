import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// Notes on the message through the engine (docs/specs/message-annotations.md): they travel with the answer under its
/// client message id, are carried again by a retry and by a restart, and end up in the local history.
final class AnnotationEngineTests: XCTestCase {
    private let notes = [
        AnnotationItem(quote: "passage", note: "why?"),
        AnnotationItem(quote: "another one", note: "and this? \u{4E3A}\u{4EC0}\u{4E48}"),
    ]
    private let answers = [AnswerItem(q: 0, sel: [1], text: "because")]

    private func delivered(_ h: Harness) async throws -> RequestView {
        let request = sampleAsk(annotatable: true)
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        return request
    }

    func testTheCapabilityComesWithTheHandshake() async throws {
        let withNotes = Harness()
        withNotes.mac.caps = Caps(newTask: false, interject: false, watch: false, annotations: true)
        await withNotes.pair()
        let state = await withNotes.state()
        XCTAssertTrue(state.macs[0].caps.annotations)

        let without = Harness()
        await without.pair()
        let old = await without.state()
        XCTAssertFalse(old.macs[0].caps.annotations, "a Mac that does not say so takes no notes")
    }

    func testTheMacSendsTheAnnotatableFlagThroughToTheRequest() async throws {
        let h = Harness()
        await h.pair()
        let request = try await delivered(h)
        let view = try unwrap(await h.state().firstInbox?.view)
        XCTAssertEqual(view.ask?.annotatable, true)
        XCTAssertEqual(view, request)
    }

    func testNotesTravelWithTheAnswerAndAreKeptInTheHistory() async throws {
        let h = Harness()
        await h.pair()
        let request = try await delivered(h)

        let id = try await h.engine.submitAnswer(
            macId: h.mac.macId, requestId: request.id, action: "send", answers: answers, todoIds: [], directive: nil,
            annotations: notes
        )
        await eventually("finished") { await h.state().inbox.isEmpty }
        let seen = try XCTUnwrap(h.mac.answer(for: request.id))
        XCTAssertEqual(seen.clientMsgId, id)
        XCTAssertEqual(seen.annotations, notes)
        XCTAssertEqual(seen.answers, answers)

        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.outcome, .answered)
        XCTAssertEqual(entry.annotations, notes, "the history keeps what was sent with the answer")
        XCTAssertEqual(entry.answers, answers)
    }

    func testNotesWithoutAnyAnswerAreAnAnswerToo() async throws {
        let h = Harness()
        await h.pair()
        let request = try await delivered(h)
        _ = try await h.engine.submitAnswer(
            macId: h.mac.macId, requestId: request.id, action: "send", answers: [], todoIds: [], directive: nil,
            annotations: notes
        )
        await eventually("finished") { await h.state().inbox.isEmpty }
        let seen = try XCTUnwrap(h.mac.answer(for: request.id))
        XCTAssertTrue(seen.answers.isEmpty)
        XCTAssertEqual(seen.annotations, notes)
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.annotations, notes)
    }

    func testAnAnswerWithoutNotesHasNoNotesField() async throws {
        let h = Harness()
        await h.pair()
        let request = try await delivered(h)
        _ = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "send", answers: answers)
        await eventually("answer at the Mac") { h.mac.count(AnswerSubmit.type) == 1 }
        guard case .object(let fields) = h.mac.received(AnswerSubmit.type)[0].b else { return XCTFail("an object") }
        XCTAssertNil(fields["annotations"], "an older Mac must not see a field it does not know")
        await eventually("finished") { await h.state().inbox.isEmpty }
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertTrue(entry.annotations.isEmpty)
    }

    func testTheHistoryKeepsTheNotesWhenTheResolutionComesBeforeTheAcknowledgement() async throws {
        let h = Harness()
        await h.pair()
        let request = try await delivered(h)
        h.mac.ignoreAnswers = true
        _ = try await h.engine.submitAnswer(
            macId: h.mac.macId, requestId: request.id, action: "send", answers: answers, todoIds: [], directive: nil,
            annotations: notes
        )
        await eventually("answer at the Mac") { h.mac.count(AnswerSubmit.type) == 1 }

        await h.mac.resolve(request.id, how: "answered", by: nil)
        await eventually("finished") { await h.state().inbox.isEmpty }
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.outcome, .answered)
        XCTAssertEqual(entry.answers, answers)
        XCTAssertEqual(entry.annotations, notes, "the same rule as for the answers: our own submission decided it")
    }

    func testAnswersDecidedElsewhereKeepNoNotes() async throws {
        let h = Harness()
        await h.pair()
        let request = try await delivered(h)
        h.mac.ignoreAnswers = true
        _ = try await h.engine.submitAnswer(
            macId: h.mac.macId, requestId: request.id, action: "send", answers: answers, todoIds: [], directive: nil,
            annotations: notes
        )
        await eventually("answer at the Mac") { h.mac.count(AnswerSubmit.type) == 1 }
        await h.mac.resolve(request.id, how: "answeredElsewhere", by: "Telegram")
        await eventually("finished") { await h.state().inbox.isEmpty }
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.outcome, .answeredElsewhere)
        XCTAssertTrue(entry.annotations.isEmpty, "what this phone wrote did not reach the Agent")
    }

    func testTheNotesSurviveARestartInTheOutbox() async throws {
        let h = Harness()
        await h.pair()
        let request = try await delivered(h)
        h.mac.ignoreAnswers = true
        let id = try await h.engine.submitAnswer(
            macId: h.mac.macId, requestId: request.id, action: "send", answers: answers, todoIds: [], directive: nil,
            annotations: notes
        )
        await eventually("answer at the Mac") { h.mac.count(AnswerSubmit.type) == 1 }
        await h.engine.setForeground(false)

        h.mac.ignoreAnswers = false
        let engine2 = PhoneEngine(
            vault: h.vault, store: h.store, broker: h.broker.makeClient(), clock: h.clock, relay: FakeRelay(),
            config: EngineConfig(deviceName: "Test iPhone", appVersion: "1.0", bundleId: "b", pushEnv: "dev", allowPlainLoopback: false, assetDirectory: h.root.appendingPathComponent("assets2"))
        )
        await engine2.start()
        await engine2.setForeground(true)
        await eventually("the answer is accepted after the retry") { await engine2.snapshot().inbox.isEmpty }
        let again = try unwrap(h.mac.answer(for: request.id))
        XCTAssertEqual(again.clientMsgId, id)
        XCTAssertEqual(again.annotations, notes, "the retry carries what the first try did")
        let entry = try unwrap(await engine2.history().first)
        XCTAssertEqual(entry.annotations, notes)
    }
}
