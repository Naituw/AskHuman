import XCTest

@testable import TodoSyncHelperKit

final class WireTests: XCTestCase {
    private func parse(_ text: String) -> Result<ParsedRequest, RequestParseFailure> {
        RequestParser.parse(Data(text.utf8))
    }

    func testParsesEveryOperation() throws {
        XCTAssertEqual(try parse(#"{"id":1,"op":"hello"}"#).get(), ParsedRequest(id: 1, op: .hello))
        XCTAssertEqual(try parse(#"{"id":2,"op":"status"}"#).get(), ParsedRequest(id: 2, op: .status))
        XCTAssertEqual(try parse(#"{"id":3,"op":"ensure"}"#).get(), ParsedRequest(id: 3, op: .ensure))
        XCTAssertEqual(try parse(#"{"id":4,"op":"deleteZone"}"#).get(), ParsedRequest(id: 4, op: .deleteZone))
        XCTAssertEqual(try parse(#"{"id":5,"op":"fetch"}"#).get(), ParsedRequest(id: 5, op: .fetch(token: nil)))
        XCTAssertEqual(
            try parse(#"{"id":6,"op":"fetch","token":"abc"}"#).get(), ParsedRequest(id: 6, op: .fetch(token: "abc")))
    }

    func testEmptyOrNullTokenMeansFullFetch() throws {
        XCTAssertEqual(try parse(#"{"id":1,"op":"fetch","token":""}"#).get().op, .fetch(token: nil))
        XCTAssertEqual(try parse(#"{"id":1,"op":"fetch","token":null}"#).get().op, .fetch(token: nil))
    }

    func testParsesSaveRecordsWithAndWithoutTag() throws {
        let line = #"{"id":7,"op":"save","records":[{"recordName":"a","recordType":"Todo","payload":"aGk="},{"recordName":"b","recordType":"Device","payload":"","changeTag":"t"}]}"#
        guard case .save(let records) = try parse(line).get().op else { return XCTFail("not a save") }
        XCTAssertEqual(records.count, 2)
        XCTAssertEqual(records[0], OutRecord(name: "a", type: "Todo", payload: Data("hi".utf8), changeTag: nil))
        XCTAssertEqual(records[1], OutRecord(name: "b", type: "Device", payload: Data(), changeTag: "t"))
    }

    func testRejectsMalformedRequestsButKeepsTheId() {
        func failure(_ text: String) -> RequestParseFailure? {
            if case .failure(let f) = parse(text) { return f }
            return nil
        }
        XCTAssertEqual(failure("not json")?.id, 0)
        XCTAssertEqual(failure("[1,2]")?.id, 0)
        XCTAssertEqual(failure(#"{"id":4}"#)?.id, 4)
        XCTAssertEqual(failure(#"{"id":5,"op":"explode"}"#)?.message, "unknown op explode")
        XCTAssertEqual(failure(#"{"id":6,"op":"save"}"#)?.id, 6)
        XCTAssertEqual(failure(#"{"id":8,"op":"save","records":[{"recordName":"a","recordType":"Todo","payload":"%%%"}]}"#)?.id, 8)
        XCTAssertEqual(failure(#"{"id":9,"op":"fetch","token":5}"#)?.message, "token must be a string")
    }

    func testReplyEncodingIsStableAndOmitsNils() {
        let reply = WireCoder.encode(EmptyReply(id: 3))
        XCTAssertEqual(String(decoding: reply, as: UTF8.self), #"{"id":3,"ok":true}"#)

        let failure = WireCoder.encode(
            FailureReply(id: 1, error: WireError(code: "offline", retryAfterMs: nil, message: nil)))
        XCTAssertEqual(String(decoding: failure, as: UTF8.self), #"{"error":{"code":"offline"},"id":1,"ok":false}"#)
    }

    func testHelloUsesProtocolAsKey() throws {
        let data = WireCoder.encode(
            HelloReply(
                id: 1, protocolVersion: 1, helperVersion: "x", environment: "production", container: "c",
                accountStatus: "available", accountHash: nil, problem: nil))
        let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(object["protocol"] as? Int, Wire.protocolVersion)
        XCTAssertNil(object["protocolVersion"])
    }

    func testUnknownOpAndBadJsonBecomeBadRequestReplies() async throws {
        let dispatcher = RpcDispatcher(backend: FixtureBackend(response: [:]), helperVersion: "0")
        for line in ["nonsense", #"{"id":3,"op":"fly"}"#] {
            let reply = await dispatcher.handle(Data(line.utf8))
            let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: reply) as? [String: Any])
            XCTAssertEqual(object["ok"] as? Bool, false)
            XCTAssertEqual((object["error"] as? [String: Any])?["code"] as? String, "badRequest")
        }
    }

    func testBackendErrorsOutsideRpcFailureBecomeOther() async throws {
        final class Boom: CloudBackend {
            struct Failure: Error {}
            func hello() async -> HelloInfo { HelloInfo(environment: "", container: "", accountStatus: "unknown", accountHash: nil) }
            func ensure() async throws { throw Failure() }
            func fetch(token: String?) async throws -> FetchResult { throw Failure() }
            func save(_ records: [OutRecord]) async throws -> [SaveOutcome] { throw Failure() }
            func deleteZone() async throws { throw Failure() }
        }
        let dispatcher = RpcDispatcher(backend: Boom(), helperVersion: "0")
        let reply = await dispatcher.handle(Data(#"{"id":2,"op":"ensure"}"#.utf8))
        let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: reply) as? [String: Any])
        XCTAssertEqual(object["id"] as? Int, 2)
        XCTAssertEqual((object["error"] as? [String: Any])?["code"] as? String, "other")
    }

    func testLogLinesNeverCarryPayloadsOrRecordNames() async throws {
        var lines: [String] = []
        let dispatcher = RpcDispatcher(
            backend: FixtureBackend(response: [
                "ok": true, "results": [["recordName": "secret-name", "status": "saved", "changeTag": "t"]],
            ]),
            helperVersion: "0", log: { event, fields in lines.append("\(event) \(fields)") })
        _ = await dispatcher.handle(
            Data(#"{"id":1,"op":"save","records":[{"recordName":"secret-name","recordType":"Todo","payload":"c2VjcmV0LXRleHQ="}]}"#.utf8))
        XCTAssertFalse(lines.isEmpty)
        for line in lines {
            XCTAssertFalse(line.contains("secret"), line)
        }
    }
}
