import XCTest

@testable import TodoSyncHelperKit

/// Drives the dispatcher with every `Fixtures/rpc-v1/*.json`: the request line goes in, a backend that
/// returns exactly what the fixture's response describes is behind it, and the reply line must equal the
/// fixture's response, key for key. The Rust client tests against the same files.
final class FixtureTests: XCTestCase {
    static let fixturesDirectory: URL = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Fixtures/rpc-v1", isDirectory: true)

    struct Fixture {
        var file: String
        var request: [String: Any]
        var response: [String: Any]
    }

    static func loadFixtures() throws -> [Fixture] {
        let names = try FileManager.default.contentsOfDirectory(atPath: fixturesDirectory.path)
            .filter { $0.hasSuffix(".json") }.sorted()
        return try names.map { name in
            let data = try Data(contentsOf: fixturesDirectory.appendingPathComponent(name))
            let doc = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any], name)
            return Fixture(
                file: name, request: try XCTUnwrap(doc["request"] as? [String: Any], name),
                response: try XCTUnwrap(doc["response"] as? [String: Any], name))
        }
    }

    static func canonical(_ object: Any) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes])
        return String(decoding: data, as: UTF8.self)
    }

    func testFixturesExist() throws {
        XCTAssertGreaterThanOrEqual(try Self.loadFixtures().count, 15)
    }

    func testEveryFixtureRoundTripsThroughTheDispatcher() async throws {
        for fixture in try Self.loadFixtures() {
            let backend = FixtureBackend(response: fixture.response)
            let status = FixtureStatus(response: fixture.response)  // the dispatcher only holds it weakly
            let dispatcher = RpcDispatcher(backend: backend, helperVersion: "0.1.0", statusSource: status)
            let line = try JSONSerialization.data(withJSONObject: fixture.request)
            let reply = await dispatcher.handle(line)
            withExtendedLifetime(status) {}
            let parsed = try JSONSerialization.jsonObject(with: reply)
            XCTAssertEqual(
                try Self.canonical(parsed), try Self.canonical(fixture.response), "fixture \(fixture.file)")
        }
    }

    func testRequestsReachTheBackendAsDescribed() async throws {
        let fixtures = Dictionary(uniqueKeysWithValues: try Self.loadFixtures().map { ($0.file, $0) })

        let next = try XCTUnwrap(fixtures["fetch-next.json"])
        let fetchBackend = FixtureBackend(response: next.response)
        _ = await RpcDispatcher(backend: fetchBackend, helperVersion: "0")
            .handle(try JSONSerialization.data(withJSONObject: next.request))
        XCTAssertEqual(fetchBackend.fetchTokens, [next.request["token"] as? String])

        let first = try XCTUnwrap(fixtures["fetch-first.json"])
        let firstBackend = FixtureBackend(response: first.response)
        _ = await RpcDispatcher(backend: firstBackend, helperVersion: "0")
            .handle(try JSONSerialization.data(withJSONObject: first.request))
        XCTAssertEqual(firstBackend.fetchTokens, [nil], "no token field means a full fetch")

        let save = try XCTUnwrap(fixtures["save-mixed.json"])
        let saveBackend = FixtureBackend(response: save.response)
        _ = await RpcDispatcher(backend: saveBackend, helperVersion: "0")
            .handle(try JSONSerialization.data(withJSONObject: save.request))
        let records = try XCTUnwrap(saveBackend.saved.first)
        XCTAssertEqual(records.map(\.name), ["todo-2", "todo-1", "todo-3", "device-1"])
        XCTAssertEqual(records.map(\.type), ["Todo", "Todo", "Todo", "Device"])
        XCTAssertEqual(records.map(\.changeTag), [nil, "tag-a1", "tag-old", "tag-d1"])
        XCTAssertTrue(String(decoding: records[0].payload, as: UTF8.self).contains("review the spec"))
    }
}

/// A backend whose answers are read out of a fixture's `response`.
final class FixtureBackend: CloudBackend {
    private let response: [String: Any]
    private(set) var fetchTokens: [String?] = []
    private(set) var saved: [[OutRecord]] = []

    init(response: [String: Any]) {
        self.response = response
    }

    private func failIfNeeded() throws {
        guard (response["ok"] as? Bool) == false, let error = response["error"] as? [String: Any] else { return }
        throw RpcFailure(
            code: error["code"] as? String ?? "other",
            retryAfterMs: (error["retryAfterMs"] as? NSNumber)?.intValue,
            message: error["message"] as? String)
    }

    func hello() async -> HelloInfo {
        HelloInfo(
            environment: response["environment"] as? String ?? "",
            container: response["container"] as? String ?? "",
            accountStatus: response["accountStatus"] as? String ?? "unknown",
            accountHash: response["accountHash"] as? String,
            problem: response["problem"] as? String)
    }

    func ensure() async throws { try failIfNeeded() }
    func deleteZone() async throws { try failIfNeeded() }

    func fetch(token: String?) async throws -> FetchResult {
        fetchTokens.append(token)
        try failIfNeeded()
        let changes = (response["changes"] as? [[String: Any]] ?? []).map {
            FetchedRecord(
                name: $0["recordName"] as! String, type: $0["recordType"] as! String,
                payload: Data(base64Encoded: $0["payload"] as! String)!, changeTag: $0["changeTag"] as! String)
        }
        let deletions = (response["deletions"] as? [[String: Any]] ?? []).map {
            FetchedDeletion(name: $0["recordName"] as! String, type: $0["recordType"] as! String)
        }
        return FetchResult(
            changes: changes, deletions: deletions, token: response["token"] as? String ?? "",
            moreComing: response["moreComing"] as? Bool ?? false)
    }

    func save(_ records: [OutRecord]) async throws -> [SaveOutcome] {
        saved.append(records)
        try failIfNeeded()
        return (response["results"] as? [[String: Any]] ?? []).map { item in
            let name = item["recordName"] as! String
            switch item["status"] as! String {
            case "saved": return .saved(name: name, changeTag: item["changeTag"] as! String)
            case "conflict":
                return .conflict(
                    name: name, serverPayload: Data(base64Encoded: item["serverPayload"] as! String)!,
                    serverChangeTag: item["serverChangeTag"] as! String)
            default: return .error(name: name, code: item["code"] as! String)
            }
        }
    }
}

final class FixtureStatus: StatusSource {
    private let response: [String: Any]

    init(response: [String: Any]) {
        self.response = response
    }

    func statusExtras() -> StatusExtras {
        StatusExtras(
            apns: response["apns"] as? String ?? "pending",
            lastPushMs: (response["lastPushMs"] as? NSNumber)?.int64Value ?? 0,
            lastTickMs: (response["lastTickMs"] as? NSNumber)?.int64Value ?? 0,
            lastTickReason: response["lastTickReason"] as? String,
            lastTickExit: (response["lastTickExit"] as? NSNumber)?.intValue,
            ticksRun: (response["ticksRun"] as? NSNumber)?.intValue ?? 0,
            enabled: response["enabled"] as? Bool ?? false,
            pollMinutes: (response["pollMinutes"] as? NSNumber)?.intValue ?? 10,
            pid: (response["pid"] as? NSNumber)?.int32Value ?? 0,
            uptimeMs: (response["uptimeMs"] as? NSNumber)?.int64Value ?? 0)
    }
}
