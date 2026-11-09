import Foundation
import XCTest

@testable import TodoSyncHelperKit

final class TickProcessTests: XCTestCase {
    private var tree: FixtureTestTree!
    private var fixture: SystemFixture!

    override func setUpWithError() throws {
        tree = try FixtureTestTree()
        fixture = try tree.fixture()
        try fixture.prepareStateDirectory()
    }

    override func tearDownWithError() throws { try tree?.remove() }

    private var environment: [String: String] {
        ["FIXTURE_TEST_ROOT": tree.root.path, "ASKHUMAN_HOME": "/must-not-propagate",
         SystemFixture.targetEnvironment: "inherited-context-must-not-reach-probe"]
    }

    private func runner(probeTimeout: TimeInterval = 3) -> TickProcessRunner {
        TickProcessRunner(fixture: fixture, environment: environment, probeTimeout: probeTimeout, log: { _, _ in })
    }

    private func run(_ runner: TickProcessRunner, config: HelperConfig? = nil) -> Int32? {
        let finished = expectation(description: "temporary stub tick finished")
        var result: Int32?
        runner.run(config: config ?? tree.config(fixture), reason: .wake) { status in
            result = status
            finished.fulfill()
        }
        wait(for: [finished], timeout: 5)
        return result
    }

    private func read(_ name: String) throws -> String {
        try String(contentsOf: tree.root.appendingPathComponent(name), encoding: .utf8)
    }

    private func writeStub(capabilities: String = #"{"systemFixture":2}"#, tickExit: Int = 0) throws {
        try tree.writeExecutable("""
        if [ "$1" = '--development-capabilities' ]; then
            printf '%s\\n' "${ASKHUMAN_RESOLVED_TARGET_V1-unset}" > "$FIXTURE_TEST_ROOT/probe-context"
            printf '%s\\n' '\(capabilities)'
            exit 0
        fi
        printf '%s\\n' "$ASKHUMAN_RESOLVED_TARGET_V1" > "$FIXTURE_TEST_ROOT/tick-context"
        printf '%s\\n' "$@" > "$FIXTURE_TEST_ROOT/tick-arguments"
        printf '%s\\n' "$HOME" "$ASKHUMAN_NO_KEYCHAIN" "${ASKHUMAN_HOME-unset}" > "$FIXTURE_TEST_ROOT/tick-environment"
        printf 'tick\\n' >> "$FIXTURE_TEST_ROOT/tick-count"
        exit \(tickExit)
        """)
    }

    func testProbeIsUnpinnedAndActualTickCarriesSystemV2() throws {
        try writeStub()
        XCTAssertEqual(run(runner()), 0)
        XCTAssertEqual(try read("probe-context"), "unset\n")
        XCTAssertEqual(try read("tick-arguments"), "todo-sync\ntick\n--reason\nwake\n")
        XCTAssertEqual(try read("tick-environment"), "\(fixture.accountHome.path)\n1\nunset\n")
        let raw = try read("tick-context")
        XCTAssertEqual(try SystemFixture.inheritedIdentity([SystemFixture.targetEnvironment: raw]), fixture.identity)
        let target = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
        XCTAssertEqual(target["binary"] as? String, tree.executable.path)
        XCTAssertEqual(try read("tick-count"), "tick\n")
        let remaining = try FileManager.default.contentsOfDirectory(atPath: fixture.paths.stateDir.path)
        XCTAssertFalse(remaining.contains { $0.hasPrefix("fixture-probe-") })
    }

    func testOldDevelopmentV2WithoutFixtureCapabilityCannotTick() throws {
        try writeStub(capabilities: #"{"developmentVerification":2}"#)
        XCTAssertNil(run(runner()))
        XCTAssertFalse(FileManager.default.fileExists(atPath: tree.root.appendingPathComponent("tick-context").path))
    }

    func testExecutableReplacementIsProbedAgain() throws {
        let processRunner = runner()
        try writeStub()
        XCTAssertEqual(run(processRunner), 0)
        try writeStub(capabilities: #"{"developmentVerification":2}"#)
        XCTAssertNil(run(processRunner))
        XCTAssertEqual(try read("tick-count"), "tick\n")
    }

    func testTickRejectionIsNonzeroWithoutUnpinnedRetry() throws {
        // Simulate replacement by an old program after a successful capability probe.
        try writeStub(tickExit: 7)
        XCTAssertEqual(run(runner()), 7)
        XCTAssertEqual(try read("tick-count"), "tick\n")
        XCTAssertEqual(try SystemFixture.inheritedIdentity([
            SystemFixture.targetEnvironment: read("tick-context"),
        ]), fixture.identity)
    }

    func testMismatchedConfigDoesNotEvenProbe() throws {
        try writeStub()
        var config = tree.config(fixture)
        config.todosPath = tree.realAccount.appendingPathComponent("todos.json").path
        XCTAssertNil(run(runner(), config: config))
        XCTAssertFalse(FileManager.default.fileExists(atPath: tree.root.appendingPathComponent("probe-context").path))
    }

    func testFailedAndHangingProbesDoNotTickOrLeakOutputFiles() throws {
        for body in ["printf '{\"systemFixture\":2}'\nexit 9\n", "while :; do :; done\n"] {
            try tree.writeExecutable(body)
            XCTAssertNil(run(runner(probeTimeout: 0.05)))
            XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: fixture.paths.stateDir.path), [])
        }
    }
}
