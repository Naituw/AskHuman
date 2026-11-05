import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// Away mode on the phone (docs/specs/special-replies.md D31–D36): the Mac's state, the commands, the directive.
final class AwayTests: XCTestCase {
    private func pairedWithAway(_ h: Harness, global: AwayGlobal? = nil, sessions: [AwaySession] = []) async {
        h.mac.caps = Caps(newTask: false, interject: false, watch: false, away: true)
        h.mac.awayGlobal = global
        h.mac.awaySessions = sessions
        await h.pair()
    }

    func testTheAwayStateArrivesWithTheHandshakeAndEveryChange() async throws {
        let h = Harness()
        await pairedWithAway(h, global: AwayGlobal(sinceMs: 1_000, note: "meeting", replies: 3))
        await eventually("the away state") { await h.state().macs.first?.away?.global?.note == "meeting" }
        var state = await h.state()
        XCTAssertEqual(state.macs[0].away?.global?.replies, 3)
        XCTAssertTrue(AwayOverview(macs: state.macs).isGlobal)

        await h.mac.changeAway { global, sessions in
            global = nil
            sessions = [AwaySession(sessionId: "s1", mode: "mute", sinceMs: 2_000)]
        }
        await eventually("the change") { await h.state().macs.first?.away?.global == nil }
        state = await h.state()
        XCTAssertEqual(state.macs[0].away?.sessions.map(\.sessionId), ["s1"])
        XCTAssertFalse(AwayOverview(macs: state.macs).isGlobal, "a session of its own is not the strip's business")
    }

    func testAnOldOrRepeatedStateIsDropped() async throws {
        let h = Harness()
        await pairedWithAway(h, global: AwayGlobal(sinceMs: 1_000, note: "first"))
        await eventually("the away state") { await h.state().macs.first?.away != nil }
        let phone = h.mac.pairedPhones[0]
        let rev = await h.state().macs[0].away?.rev ?? 0

        await h.mac.sendDown(AwayState(rev: rev, global: nil), to: phone)
        await h.mac.sendDown(AwayState(rev: rev - 1, global: nil), to: phone)
        try await Task.sleep(nanoseconds: 80_000_000)
        let kept = await h.state().macs[0].away?.global?.note
        XCTAssertEqual(kept, "first", "a state that is not newer changes nothing")

        await h.mac.sendDown(AwayState(rev: rev + 1, global: nil), to: phone)
        await eventually("the newer state") { await h.state().macs.first?.away?.global == nil }
    }

    func testAMacWithoutAwayModeHasNoStateAndTakesNoCommand() async throws {
        let h = Harness()
        await h.pair()
        let state = await h.state()
        XCTAssertNil(state.macs[0].away)
        XCTAssertFalse(state.macs[0].caps.away)
        XCTAssertFalse(AwayOverview(macs: state.macs).canStart)

        do {
            try await h.engine.setAway(macId: h.mac.macId, on: true, note: nil)
            XCTFail("an older Mac must not be sent a command it would drop")
        } catch {
            XCTAssertEqual(h.mac.count(AwaySet.type), 0)
        }
    }

    func testTurningAwayOnAsksTheMacAndTheNewStateComesBack() async throws {
        let h = Harness()
        await pairedWithAway(h)
        await eventually("the first state") { await h.state().macs.first?.away != nil }

        try await h.engine.setAway(macId: h.mac.macId, on: true, note: "  in a meeting  ")
        await eventually("away.set") { h.mac.count(AwaySet.type) == 1 }
        let sent = try h.mac.received(AwaySet.type)[0].body(as: AwaySet.self)
        XCTAssertTrue(sent.on)
        XCTAssertEqual(sent.note, "in a meeting", "the note is trimmed")
        await eventually("on") { await h.state().macs.first?.away?.global?.note == "in a meeting" }

        try await h.engine.setAway(macId: h.mac.macId, on: false, note: nil)
        await eventually("off") { await h.state().macs.first?.away?.global == nil }
        XCTAssertNil(try h.mac.received(AwaySet.type)[1].body(as: AwaySet.self).note)
    }

    func testABlankNoteIsNoNoteAndALongOneIsCut() {
        XCTAssertNil(AwayNote.clean(nil))
        XCTAssertNil(AwayNote.clean("  \n "))
        XCTAssertEqual(AwayNote.clean(" a "), "a")
        XCTAssertEqual(AwayNote.clean(String(repeating: "x", count: 900))?.count, AwayNote.maxCharacters)
    }

    func testASessionIsSetAndClearedOnItsOwn() async throws {
        let h = Harness()
        await pairedWithAway(h)
        await eventually("the first state") { await h.state().macs.first?.away != nil }

        try await h.engine.setSessionAway(macId: h.mac.macId, sessionId: "s1", setting: .mute, note: nil)
        await eventually("muted") { await h.state().macs.first?.away?.sessions.first?.mode == "mute" }
        let sent = try h.mac.received(AwaySessionSet.type)[0].body(as: AwaySessionSet.self)
        XCTAssertEqual(sent.sessionId, "s1")
        XCTAssertEqual(sent.setting, "mute")

        try await h.engine.setSessionAway(macId: h.mac.macId, sessionId: "s1", setting: .follow, note: nil)
        await eventually("cleared") { await h.state().macs.first?.away?.sessions.isEmpty == true }
    }

    func testNothingIsSentToAMacThatIsOffline() async throws {
        let h = Harness()
        await pairedWithAway(h)
        await eventually("the first state") { await h.state().macs.first?.away != nil }
        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link != .online }
        let state = await h.state()
        XCTAssertNil(state.macs[0].away, "what the Mac said is void once it is gone")

        do {
            try await h.engine.setAway(macId: h.mac.macId, on: true, note: nil)
            XCTFail("expected macOffline")
        } catch let error as BackendError {
            XCTAssertEqual(error, .macOffline)
        }
        XCTAssertEqual(h.mac.count(AwaySet.type), 0)
    }

    func testTheDirectiveGoesWithTheAnswer() async throws {
        let h = Harness()
        await pairedWithAway(h)
        let request = sampleAsk(escalated: true)
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        let delivered = await h.state()
        let item = try XCTUnwrap(delivered.inbox.first)
        XCTAssertTrue(item.view?.escalated == true)

        let choice = AnswerDirectiveChoice(kind: .away, sessionOnly: true, note: " back at five ")
        _ = try await h.engine.submitAnswer(
            macId: h.mac.macId, requestId: request.id, action: "send", answers: [AnswerItem(q: 0, sel: [0])], todoIds: [],
            directive: choice.wire
        )
        await eventually("finished") { await h.state().inbox.isEmpty }
        let seen = try XCTUnwrap(h.mac.answer(for: request.id))
        XCTAssertEqual(seen.directive, AnswerDirective(kind: "away", scope: "session", note: "back at five"))
    }

    func testTheWireFormOfTheChoices() {
        XCTAssertEqual(AnswerDirectiveChoice.away.wire, AnswerDirective(kind: "away", scope: "global", note: nil))
        XCTAssertEqual(AnswerDirectiveChoice.mute.wire, AnswerDirective(kind: "mute", scope: "session", note: nil))
        var choice = AnswerDirectiveChoice.mute
        choice.sessionOnly = false
        XCTAssertEqual(choice.wire.scope, "session", "mute is always about the asking session")
    }

    // MARK: the overview

    private func mac(_ id: String, link: MacLink = .online, away: Bool = true, state: AwayState? = nil) -> MacState {
        MacState(
            id: id, name: id.uppercased(), macName: id, alias: nil, link: link,
            caps: Caps(newTask: false, interject: false, watch: false, away: away), slot: nil, pairedAt: Date(timeIntervalSince1970: 0),
            lastSeenAt: nil, pendingCount: 0, pushStatus: nil, away: state
        )
    }

    func testTheOverviewAddsTheMacsUp() {
        let a = mac("a", state: AwayState(
            rev: 1, global: AwayGlobal(sinceMs: 5_000, note: "meeting", replies: 4),
            sessions: [AwaySession(sessionId: "s1", mode: "away", sinceMs: 1), AwaySession(sessionId: "s2", mode: "mute", sinceMs: 1)],
            escalatedWaiting: 1
        ))
        let b = mac("b", state: AwayState(rev: 1, global: AwayGlobal(sinceMs: 2_000, replies: 2)))
        let c = mac("c", link: .macOffline)
        let old = mac("d", away: false)
        let overview = AwayOverview(macs: [a, b, c, old])

        XCTAssertTrue(overview.isGlobal)
        XCTAssertEqual(overview.globalMacs.map(\.id), ["a", "b"])
        XCTAssertEqual(overview.replies, 6)
        XCTAssertEqual(overview.since, Date(timeIntervalSince1970: 2))
        XCTAssertEqual(overview.sessionAway, 1)
        XCTAssertEqual(overview.escalatedWaiting, 1)
        XCTAssertEqual(overview.note, "meeting")
        XCTAssertEqual(overview.startable.map(\.id), ["a", "b"], "offline Macs and Macs without away mode cannot be set")
        XCTAssertTrue(overview.canStart)
        XCTAssertFalse(AwayOverview(macs: [old]).canStart)
        XCTAssertFalse(AwayOverview(macs: [c]).isGlobal)
    }

    func testASessionsModeIsItsOwnSettingElseTheMacsSwitch() {
        let agent = sampleAgent("claude:s1")
        let off = mac("a", state: AwayState(rev: 1, sessions: [AwaySession(sessionId: "s1", mode: "mute", sinceMs: 1)]))
        XCTAssertEqual(AwayOverview.mode(of: agent, on: off), .mute)
        XCTAssertFalse(AwayOverview.isAway(agent: agent, on: off))

        let global = mac("a", state: AwayState(rev: 1, global: AwayGlobal(sinceMs: 1)))
        XCTAssertEqual(AwayOverview.mode(of: agent, on: global), .follow)
        XCTAssertTrue(AwayOverview.isAway(agent: agent, on: global), "follows the global switch")

        let exception = mac("a", state: AwayState(
            rev: 1, global: AwayGlobal(sinceMs: 1), sessions: [AwaySession(sessionId: "s1", mode: "normal", sinceMs: 1)]
        ))
        XCTAssertFalse(AwayOverview.isAway(agent: agent, on: exception), "asks normally anyway")
        XCTAssertEqual(AwayOverview.mode(of: agent, on: nil), .follow)
    }
}
