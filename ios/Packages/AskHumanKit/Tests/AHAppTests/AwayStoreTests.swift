import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The screens' view of away mode (docs/specs/special-replies.md D31–D36), on the mock Macs.
@MainActor
final class AwayStoreTests: XCTestCase {
    private var backend: MockBackend!
    private var store: AppStore!

    private func make(_ scenario: MockBackend.Scenario) async {
        backend = MockBackend(scenario: scenario, latency: 0)
        let defaults = UserDefaults(suiteName: "ah-test-" + UUID().uuidString)!
        store = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await store.start()
        await eventually("first state") { self.store.state.foreground }
    }

    private func agent(_ key: String, mac: String = MockData.homeMacId) throws -> AgentItem {
        try XCTUnwrap(store.agent(id: AgentItem.key(mac, key)))
    }

    func testAMacThatHasAwayModeOffersQuickRepliesWithItsQuestions() async throws {
        await make(.away)
        let plain = try XCTUnwrap(store.state.inbox.first { $0.requestId == "req-migration" })
        XCTAssertEqual(plain.view?.ask?.quickReplies, MockData.quickReplies)
        let strict = store.state.inbox.compactMap(\.view?.ask).filter { $0.selectOnly || $0.whatsNext }
        XCTAssertTrue(strict.allSatisfy { $0.quickReplies.isEmpty }, "none for What's next or a strict choice")
        await make(.demo)
        XCTAssertTrue(store.state.inbox.compactMap(\.view?.ask).allSatisfy { $0.quickReplies.isEmpty })
    }

    func testAQuickReplyGoesOutAsTheAnswerOfTheQuestionsNobodyAnswered() async throws {
        await make(.away)
        let item = try XCTUnwrap(store.state.inbox.first { $0.requestId == "req-migration" })
        let form = try XCTUnwrap(store.composer(for: item))
        XCTAssertEqual(form.quickReplies, MockData.quickReplies)
        form.toggle(question: 0, option: 1)
        form.directive = .mute
        try await store.send(form, item: item, quickReply: form.quickReplies[0])

        let submissions = await backend.submissions
        let sent = try XCTUnwrap(submissions.last)
        XCTAssertEqual(sent.answers, [
            AnswerItem(q: 0, sel: [1]),
            AnswerItem(q: 1, text: "Continue"),
            AnswerItem(q: 2, text: "Continue"),
        ])
        XCTAssertEqual(sent.directive?.kind, "mute", "an instruction armed beforehand goes with the reply")
    }

    func testMacsWithoutAwayModeOfferNothing() async throws {
        await make(.demo)
        XCTAssertFalse(store.offersAway)
        XCTAssertFalse(store.away.canStart)
        let item = try XCTUnwrap(store.state.inbox.first { $0.view?.ask != nil })
        XCTAssertFalse(store.offersDirectives(for: item), "an older Mac would drop the directive")
        let working = try agent(MockData.workingAgentKey)
        XCTAssertFalse(store.canSetSessionAway(working))
    }

    func testStartingAndEndingAwayModeOnBothMacs() async throws {
        await make(.away)
        XCTAssertTrue(store.offersAway)
        XCTAssertFalse(store.away.isGlobal)
        XCTAssertEqual(store.away.startable.count, 2)

        let failed = await store.startAway(macIds: [MockData.homeMacId, MockData.officeMacId], note: "  in a meeting ")
        XCTAssertTrue(failed.isEmpty)
        await eventually { self.store.away.globalMacs.count == 2 }
        XCTAssertEqual(store.away.note, "in a meeting")
        let calls = await backend.awayCalls
        XCTAssertEqual(calls.map(\.setting), ["on", "on"])

        let ended = await store.endAway()
        XCTAssertTrue(ended.isEmpty)
        await eventually { !self.store.away.isGlobal }
        let all = await backend.awayCalls
        XCTAssertEqual(all.suffix(2).map(\.setting), ["off", "off"], "one tap ends them all")
    }

    func testAMacThatCannotBeReachedIsReportedAndTheOthersAreOn() async throws {
        await make(.away)
        await backend.setUnreachable([MockData.officeMacId])
        let failed = await store.startAway(macIds: [MockData.homeMacId, MockData.officeMacId], note: "")
        XCTAssertEqual(failed, [MockData.officeMacId])
        await eventually { self.store.away.globalMacs.map(\.id) == [MockData.homeMacId] }
    }

    func testTheEscalatedScenarioShowsTheStripFactsAndTheTags() async throws {
        await make(.awayOn)
        XCTAssertTrue(store.away.isGlobal)
        XCTAssertEqual(store.away.replies, 7)
        XCTAssertEqual(store.away.escalatedWaiting, 1)
        let waiting = try agent(MockData.waitingAgentKey)
        XCTAssertTrue(store.isEscalatedWaiting(waiting), "the migration question came as an escalation")
        let item = try XCTUnwrap(store.state.inbox.first { $0.requestId == MockData.escalatedRequestId })
        XCTAssertTrue(store.isEscalated(item))

        XCTAssertEqual(store.sessionMode(of: try agent(MockData.idleAgentKey)), .mute)
        XCTAssertEqual(store.sessionMode(of: try agent(MockData.workingAgentKey)), .normal)
        XCTAssertTrue(store.isAway(waiting), "the others follow the global switch")
        XCTAssertFalse(store.isAway(try agent(MockData.workingAgentKey)))
        XCTAssertFalse(store.isEscalatedWaiting(try agent(MockData.workingAgentKey)))
    }

    func testASessionIsSetOnItsOwnAndTheRowFollows() async throws {
        await make(.away)
        let working = try agent(MockData.workingAgentKey)
        XCTAssertTrue(store.canSetSessionAway(working))
        XCTAssertEqual(store.sessionMode(of: working), .follow)

        try await store.setSessionAway(.mute, for: working)
        await eventually { self.store.sessionMode(of: working) == .mute }
        try await store.setSessionAway(.follow, for: working)
        await eventually { self.store.sessionMode(of: working) == .follow }
        let calls = await backend.awayCalls
        XCTAssertEqual(calls.map(\.sessionId), [working.view.sessionId, working.view.sessionId])
    }

    func testTheArmedDirectiveGoesWithTheAnswerAndTakesEffect() async throws {
        await make(.away)
        let item = try XCTUnwrap(store.state.inbox.first { $0.requestId == "req-migration" })
        XCTAssertTrue(store.offersDirectives(for: item))
        let composer = try XCTUnwrap(store.composer(for: item))
        composer.toggle(question: 0, option: 0)
        composer.directive = .mute

        try await store.send(composer, item: item)
        let submissions = await backend.submissions
        XCTAssertEqual(submissions.last?.directive, AnswerDirective(kind: "mute", scope: "session"))
        await eventually { self.store.sessionMode(of: try! self.agent(MockData.waitingAgentKey)) == .mute }
    }

    func testAnswerWithoutAnArmedDirectiveCarriesNone() async throws {
        await make(.away)
        let item = try XCTUnwrap(store.state.inbox.first { $0.requestId == "req-migration" })
        let composer = try XCTUnwrap(store.composer(for: item))
        composer.toggle(question: 0, option: 0)
        try await store.send(composer, item: item)
        let submissions = await backend.submissions
        XCTAssertNil(submissions.last?.directive)
    }

    func testAnArmedDirectiveIsNotPartOfTheDraft() async throws {
        await make(.away)
        let item = try XCTUnwrap(store.state.inbox.first { $0.requestId == "req-migration" })
        let composer = try XCTUnwrap(store.composer(for: item))
        composer.directive = .away
        let again = try XCTUnwrap(store.composer(for: item))
        XCTAssertEqual(again.directive, .away, "the same page keeps it while it is open")
        let fresh = AnswerComposer(key: "other", ask: try XCTUnwrap(item.view?.ask), drafts: MemoryDraftStore())
        XCTAssertNil(fresh.directive, "a draft that comes back later does not turn anything on by itself")
    }

    func testTheDurationOfAwayModeIsWorkedOutFromTheEarliestSwitch() async throws {
        await make(.awayOn)
        let since = try XCTUnwrap(store.away.since)
        XCTAssertEqual(Date().timeIntervalSince(since), 2_400, accuracy: 30)
    }
}
