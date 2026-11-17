import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// docs/specs/ios-traffic-reduction.md §5 against a real daemon: what the phone engine is sent when it comes to the
/// foreground, and while working sessions it does not watch take their steps. The Mac says what it sent in
/// `trafficBreakdown` (`channel ios status --json`), so the assertions count messages and bytes exactly and leave room
/// where a size depends on the content.
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=ios/Tools/local-broker swift test --filter TrafficReductionE2ETests
final class TrafficReductionE2ETests: E2ECase {
    private lazy var project: URL = {
        let url = mac.home.appendingPathComponent("project")
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }()

    @discardableResult
    private func hook(_ event: String, session: String, extra: [String: Any] = [:]) async throws -> CmdResult {
        var payload: [String: Any] = ["session_id": session, "cwd": project.path]
        payload.merge(extra) { _, new in new }
        let data = try JSONSerialization.data(withJSONObject: payload)
        let result = try await mac.cli(["__agent-hook", "claude", event], stdin: String(decoding: data, as: UTF8.self), timeout: 30)
        XCTAssertEqual(result.status, 0, result.stderr)
        return result
    }

    private func toolStep(_ command: String) -> [String: Any] {
        ["hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": ["command": command]]
    }

    private struct Kind: Equatable {
        var msgs = 0
        var bytes = 0
    }

    /// The Mac's own account of what it sent and received by message type this month.
    private func breakdown() async throws -> [String: Kind] {
        let status = try await mac.status()
        let kinds = (status["trafficBreakdown"] as? [String: Any])?["kinds"] as? [String: [String: Any]] ?? [:]
        return kinds.mapValues {
            Kind(msgs: ($0["msgs"] as? NSNumber)?.intValue ?? 0, bytes: ($0["bytes"] as? NSNumber)?.intValue ?? 0)
        }
    }

    private func grew(_ key: String, from before: [String: Kind], to after: [String: Kind]) -> Kind {
        let old = before[key] ?? Kind(), new = after[key] ?? Kind()
        return Kind(msgs: new.msgs - old.msgs, bytes: new.bytes - old.bytes)
    }

    private func phoneLeft() async {
        await eventually("the Mac to see the phone leave") {
            let phones = (try? await self.mac.jsonArray(["channel", "ios", "devices", "--json"])) ?? []
            return phones.first?["online"] as? Bool == false
        }
    }

    func testTheForegroundCostsOneCompressedRoundAndUnwatchedStepsCostNothing() async throws {
        let sessions = 12, working = 4
        try await pair()
        let macState = await phone.state().macs.first
        XCTAssertEqual(macState?.caps.leanAgents, true, "the Mac offers the lean list to a phone that asked for it")

        for number in 0..<sessions {
            try await hook("session-start", session: "reduce-\(number)")
            if number < working {
                try await hook("turn-start", session: "reduce-\(number)", extra: ["prompt": "Work on item \(number)"])
            }
        }
        await eventually("the Agent list", timeout: 60) { await self.phone.state().agents.count == sessions }
        try await Task.sleep(nanoseconds: 2_000_000_000)

        // 1. Coming to the foreground: one round of snapshots, the list compressed, no second round.
        await phone.engine.setForeground(false)
        await phoneLeft()
        let beforeEntry = try await breakdown()
        await phone.engine.setForeground(true)
        await eventually("the phone synced again", timeout: 30) {
            let state = await self.phone.state()
            return state.macs.first?.link == .online && state.agents.count == sessions
        }
        // The foreground presence reaches the Mac a moment after the snapshots; give a second round time to show.
        try await Task.sleep(nanoseconds: 3_000_000_000)
        let afterEntry = try await breakdown()

        let snapshot = grew("out:agents.snapshot", from: beforeEntry, to: afterEntry)
        XCTAssertEqual(snapshot.msgs, 1, "one snapshot of the Agents, not the two of the old foreground round")
        let plainBytes = await phone.state().agents.map { (try? AHJSON.encoder.encode($0.view).count) ?? 0 }.reduce(0, +)
        XCTAssertLessThan(snapshot.bytes * 2, plainBytes, "the list went compressed: \(snapshot.bytes) B for \(plainBytes) B of Agents")
        XCTAssertEqual(grew("out:inbox.snapshot", from: beforeEntry, to: afterEntry).msgs, 1)
        XCTAssertEqual(grew("out:agents.delta", from: beforeEntry, to: afterEntry).msgs, 0, "nothing changed in between")
        let entered = afterEntry.filter { $0.key.hasPrefix("out:") }.reduce(0) { $0 + $1.value.bytes }
            - beforeEntry.filter { $0.key.hasPrefix("out:") }.reduce(0) { $0 + $1.value.bytes }

        // 2. The sessions that are working take steps nobody watches: nothing is sent for them.
        let beforeSteps = try await breakdown()
        for step in 0..<12 {
            try await hook("activity", session: "reduce-\(step % working)", extra: toolStep("cargo test step \(step)"))
        }
        try await Task.sleep(nanoseconds: 3_000_000_000)
        let afterSteps = try await breakdown()
        XCTAssertEqual(grew("out:agents.delta", from: beforeSteps, to: afterSteps).msgs, 0, "12 steps of unwatched sessions")
        let held = await phone.state().agents.first { $0.agentKey == "claude:reduce-0" }
        XCTAssertEqual(held?.view.state, "working")

        // 3. A session the phone watches still gets its tool and steps in the list, and frames.
        let item = try unwrap(held)
        await phone.engine.watch(macId: item.macId, agentKey: item.agentKey, open: true)
        await eventually("the first frame") { await self.phone.state().watches[item.id]?.frame != nil }
        let beforeWatch = try await breakdown()
        try await hook("activity", session: "reduce-0", extra: toolStep("cargo build --release"))
        await eventually("the tool of the watched session in the list") {
            await self.phone.state().agents.first { $0.id == item.id }?.view.currentTool?.contains("cargo build") == true
        }
        // Frames go out at most every two seconds.
        await eventually("a frame after that step", timeout: 30) { (await self.phone.state().watches[item.id]?.seq ?? 0) >= 2 }
        let afterWatch = try await breakdown()
        XCTAssertGreaterThanOrEqual(grew("out:agents.delta", from: beforeWatch, to: afterWatch).msgs, 1)
        XCTAssertGreaterThanOrEqual(grew("out:watch.frame", from: beforeWatch, to: afterWatch).msgs, 1)

        // 4. A state change of an unwatched session is a stable field: it is sent.
        await phone.engine.watch(macId: item.macId, agentKey: item.agentKey, open: false)
        try await hook("turn-end", session: "reduce-1")
        await eventually("the session idle on the phone") {
            await self.phone.state().agents.first { $0.agentKey == "claude:reduce-1" }?.view.state == "idle"
        }

        report(["enteredBytes": entered, "agentSnapshotBytes": snapshot.bytes, "agentsPlainBytes": plainBytes])
        XCTAssertLessThan(entered, 6_000, "a foreground entry of \(sessions) sessions sent \(entered) B")
    }

    func testAWatchOpenedRightAfterComingBackGetsItsFramesAndItsListActivity() async throws {
        try await pair()
        try await hook("session-start", session: "reduce-watch")
        try await hook("turn-start", session: "reduce-watch")
        await eventually("the session working") { await self.phone.state().agents.first?.view.state == "working" }
        let item = try unwrap(await phone.state().agents.first)

        // Leave, come back and watch at once: the watch is registered before the Mac has the foreground presence, and
        // it must outlive that presence (the Mac clears watches at a new `hello`, not at a presence).
        await phone.engine.setForeground(false)
        await phoneLeft()
        await phone.engine.setForeground(true)
        await phone.engine.watch(macId: item.macId, agentKey: item.agentKey, open: true)
        await eventually("a frame after coming back") { await self.phone.state().watches[item.id]?.frame != nil }

        try await hook("activity", session: "reduce-watch", extra: toolStep("cargo clippy"))
        await eventually("the tool in the list of a watched session") {
            await self.phone.state().agents.first?.view.currentTool?.contains("cargo clippy") == true
        }
        let frames = await phone.state().watches[item.id]?.seq ?? 0
        XCTAssertGreaterThanOrEqual(frames, 1)
    }
}
