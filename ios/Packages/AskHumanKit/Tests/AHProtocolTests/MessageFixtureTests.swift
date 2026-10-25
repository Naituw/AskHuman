import XCTest
@testable import AHProtocol

final class MessageFixtureTests: XCTestCase {
    /// Every sample decodes into its typed model and encodes back to the same JSON value
    /// (keys unordered, absent optionals omitted).
    func testEverySampleRoundTrips() throws {
        let fx = try Fixtures.json("messages.json")
        let samples = fx["samples"] as! [[String: Any]]
        XCTAssertGreaterThanOrEqual(samples.count, 60)
        var seen = Set<String>()
        for sample in samples {
            let type = sample["type"] as! String
            let name = (sample["name"] as? String).map { "\(type)#\($0)" } ?? type
            seen.insert(type)
            let body = try AHJSON.decoder.decode(
                JSONValue.self, from: JSONSerialization.data(withJSONObject: sample["body"] as Any)
            )
            guard let result = MessageRegistry.normalize(type: type, body: body) else {
                XCTFail("unknown message type \(type)")
                continue
            }
            switch result {
            case .success(let normalized):
                XCTAssertEqual(normalized, body, name)
            case .failure(let error):
                XCTFail("\(name): \(error)")
            }
        }
        // The fixture covers the whole catalog.
        XCTAssertEqual(seen, Set(MessageRegistry.all.map { $0.type }))
    }

    func testUnknownTypesAndBadShapesAreRejected() {
        XCTAssertNil(MessageRegistry.normalize(type: "nope", body: .object([:])))
        guard case .failure = MessageRegistry.normalize(type: "answer.ack", body: .object(["status": .string("accepted")]))! else {
            return XCTFail("a missing required field must fail")
        }
    }

    func testOptionalFieldsAreOmittedOnTheWire() throws {
        let ack = AnswerAck(clientMsgId: "c", requestId: "r", status: "accepted", reason: nil)
        XCTAssertEqual(
            try AHJSON.tree(ack),
            .object(["clientMsgId": .string("c"), "requestId": .string("r"), "status": .string("accepted")])
        )
        let option = OptionView(text: "a", recommended: false)
        XCTAssertEqual(try AHJSON.tree(option), .object(["text": .string("a"), "recommended": .bool(false)]))
        let todo = OptionView(text: "a", recommended: true, todo: true)
        XCTAssertEqual(try AHJSON.tree(todo)["todo"], .bool(true))
    }

    func testPlainMessagesCarryTypedBodies() throws {
        let plain = try Plain(RequestSeen(requestId: "r1"), nowMs: 1_700_000_000_000)
        XCTAssertEqual(plain.t, "request.seen")
        XCTAssertEqual(try plain.body(as: RequestSeen.self).requestId, "r1")
        XCTAssertThrowsError(try plain.body(as: RequestAck.self), "the type string must match")
    }

    func testPairCodeRoundTrip() {
        let code = PairCode(
            v: 1, h: "broker.example", p: 8883, w: 8084, u: "user1", pw: "pw", r: "https://relay.example",
            m: String(repeating: "m", count: 26), mn: "Home Mac", t: String(repeating: "t", count: 26),
            ts: Bytes.b64u(Data(repeating: 1, count: 32)), ek: Bytes.b64u(Data(repeating: 2, count: 32)), x: 1_800_000_000
        )
        let text = code.encode()
        XCTAssertTrue(text.hasPrefix("ahpair1."))
        XCTAssertEqual(PairCode.decode(text), code)
        XCTAssertEqual(PairCode.decode("  \(text)\n"), code)
        XCTAssertNil(PairCode.decode("ahpair1.@@@"))
        XCTAssertNil(PairCode.decode("other"))
    }

    func testTopicsAndPresence() {
        XCTAssertEqual(Wire.topicDown(user: "u1", phoneId: "p"), "ah/u1/d/p")
        XCTAssertEqual(Wire.topicUp(user: "u1", macId: "m"), "ah/u1/u/m")
        XCTAssertEqual(Wire.topicPresenceFilter(user: "u1"), "ah/u1/s/+")
        XCTAssertEqual(Wire.presenceDevice(user: "u1", topic: "ah/u1/s/x"), "x")
        XCTAssertNil(Wire.presenceDevice(user: "u1", topic: "ah/u2/s/x"))
        XCTAssertNil(Wire.presenceDevice(user: "u1", topic: "ah/u1/s/x/y"))
        for bad in ["", "+", "#", "+/#", "a/b", "a b", "名字"] {
            XCTAssertFalse(Wire.isValidBrokerUser(bad), bad)
        }
        XCTAssertTrue(Wire.isValidBrokerUser("askhuman01"))
        let presence = try? AHJSON.decoder.decode(Presence.self, from: Data(#"{"o":1,"t":5,"r":"mac","e":"abc"}"#.utf8))
        XCTAssertEqual(presence, Presence(o: 1, t: 5, r: "mac", fg: 0, e: "abc"))
    }
}
