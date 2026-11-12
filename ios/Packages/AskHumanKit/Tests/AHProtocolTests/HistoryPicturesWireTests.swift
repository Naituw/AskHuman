import XCTest
@testable import AHProtocol

/// The pictures of past exchanges on the wire (docs/specs/ios-history-pictures.md §5). Everything is additive: a Mac and a
/// phone of different ages exchange exactly what each of them knows, and what is not there is not sent.
final class HistoryPicturesWireTests: XCTestCase {
    private func caps(historyPictures: Bool) -> Caps {
        Caps(
            newTask: true, interject: true, watch: true, todos: true, answerTodos: true, away: true, sessionHistory: true,
            historyPictures: historyPictures
        )
    }

    private func picture(_ id: String, thumb: String? = "/9j/4AAQ", tooLarge: Bool = false, gone: Bool = false) -> AssetRef {
        AssetRef(
            assetId: id, name: id + ".png", size: gone ? 0 : 2_048, mime: "image/png", isImage: true, thumb: gone ? nil : thumb,
            tooLarge: tooLarge, gone: gone
        )
    }

    // MARK: capability

    func testCapsLeaveThePicturesBitOutWhenFalse() throws {
        XCTAssertNil(try AHJSON.tree(caps(historyPictures: false))["historyPictures"], "what predates it sees what it always saw")
        XCTAssertEqual(try AHJSON.tree(caps(historyPictures: true))["historyPictures"], .bool(true))
    }

    func testAMissingPicturesBitReadsAsNo() throws {
        let old = try AHJSON.decoder.decode(
            Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true,"sessionHistory":true}"#.utf8)
        )
        XCTAssertFalse(old.historyPictures)
        let new = try AHJSON.decoder.decode(
            Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true,"historyPictures":true}"#.utf8)
        )
        XCTAssertTrue(new.historyPictures)
        for flag in [false, true] {
            XCTAssertEqual(try AHJSON.value(Caps.self, from: try AHJSON.tree(caps(historyPictures: flag))), caps(historyPictures: flag))
        }
    }

    // MARK: the ask

    func testThePageAskForPicturesIsSentOnlyWhenWanted() throws {
        let plain = SessionHistoryGet(reqId: "h1", agentKey: "claude:s1", beforeMs: 5, limit: 8)
        XCTAssertNil(try AHJSON.tree(plain)["pictures"], "an older Mac is not sent a field it never read")
        let wanted = SessionHistoryGet(reqId: "h1", agentKey: "claude:s1", beforeMs: 5, limit: 8, pictures: true)
        XCTAssertEqual(try AHJSON.tree(wanted)["pictures"], .bool(true))
        XCTAssertEqual(try AHJSON.value(SessionHistoryGet.self, from: try AHJSON.tree(wanted)), wanted)
        // A read from an older phone has no such field, and means no pictures.
        let old = try AHJSON.decoder.decode(
            SessionHistoryGet.self, from: Data(#"{"reqId":"h1","agentKey":"k","beforeMs":5,"limit":8}"#.utf8)
        )
        XCTAssertFalse(old.pictures)
    }

    // MARK: a picture

    func testAPictureLeavesOutWhatIsNotSpecial() throws {
        let plain = try AHJSON.tree(picture("p1"))
        XCTAssertNil(plain["gone"])
        XCTAssertNil(plain["tooLarge"])
        XCTAssertEqual(plain["thumb"], .string("/9j/4AAQ"))

        let gone = picture("p2", gone: true)
        let tree = try AHJSON.tree(gone)
        XCTAssertEqual(tree["gone"], .bool(true))
        XCTAssertNil(tree["thumb"], "a file that is gone has no thumbnail")
        XCTAssertEqual(try AHJSON.value(AssetRef.self, from: tree), gone)

        let huge = picture("p3", thumb: nil, tooLarge: true)
        XCTAssertEqual(try AHJSON.tree(huge)["tooLarge"], .bool(true))
        XCTAssertNil(try AHJSON.tree(huge)["gone"])
        // A file of a request, from a Mac that does not know about `gone`.
        let old = try AHJSON.decoder.decode(
            AssetRef.self,
            from: Data(#"{"assetId":"a1","name":"a.png","size":1,"mime":"image/png","isImage":true}"#.utf8)
        )
        XCTAssertFalse(old.gone)
    }

    // MARK: exchanges and questions

    func testAnExchangeLeavesItsPicturesOutWhenThereAreNone() throws {
        let bare = SessionExchange(id: "r", at: 1, channel: "away", message: "", qs: [SessionQuestion(text: "ok?")])
        let tree = try AHJSON.tree(bare)
        XCTAssertNil(tree["pictures"])
        guard case .array(let questions)? = tree["qs"], case .object(let first)? = questions.first else { return XCTFail("a question") }
        XCTAssertNil(first["pictures"])
    }

    func testPicturesKeepTheirOrderAndRoundTrip() throws {
        let exchange = SessionExchange(
            id: "r2", at: 2, channel: "popup", message: "m", attachments: 4,
            pictures: [picture("a"), picture("b", gone: true), picture("c", thumb: nil, tooLarge: true)],
            qs: [
                SessionQuestion(text: "q", input: "words", images: 2, pictures: [picture("d"), picture("e", gone: true)]),
                SessionQuestion(text: "second"),
            ]
        )
        let tree = try AHJSON.tree(exchange)
        let back = try AHJSON.value(SessionExchange.self, from: tree)
        XCTAssertEqual(back, exchange)
        XCTAssertEqual(back.pictures.map(\.assetId), ["a", "b", "c"], "the order the files were attached in")
        XCTAssertEqual(back.qs[0].pictures.map(\.assetId), ["d", "e"])
        XCTAssertTrue(back.qs[1].pictures.isEmpty)
        // An exchange from an older Mac has none, and decodes to none.
        let decoded = try AHJSON.value(SessionExchange.self, from: try AHJSON.tree(SessionExchange(id: "o", at: 1, channel: "ios", message: "", qs: [])))
        XCTAssertTrue(decoded.pictures.isEmpty)
    }

    // MARK: one exchange

    func testTheExchangeAskCarriesTheIdOfTheRecord() throws {
        let get = HistoryExchangeGet(reqId: "x1", id: "req-10")
        XCTAssertEqual(try AHJSON.tree(get), .object(["reqId": .string("x1"), "id": .string("req-10")]))
        XCTAssertEqual(HistoryExchangeGet.type, "history.exchange.get")
        XCTAssertEqual(try AHJSON.value(HistoryExchangeGet.self, from: try AHJSON.tree(get)), get)
    }

    func testTheExchangeAnswerHasAnItemOnlyWhenTheMacHasTheRecord() throws {
        let missing = HistoryExchange(reqId: "x2", id: "req-gone")
        XCTAssertEqual(HistoryExchange.type, "history.exchange")
        XCTAssertEqual(try AHJSON.tree(missing), .object(["reqId": .string("x2"), "id": .string("req-gone")]), "no record is not an error")
        let none = try AHJSON.value(HistoryExchange.self, from: try AHJSON.tree(missing))
        XCTAssertNil(none.item)

        let held = SessionExchange(
            id: "req-10", at: 3, channel: "ios", message: "m", attachments: 1, pictures: [picture("a")],
            qs: [SessionQuestion(text: "q", selected: ["yes"])]
        )
        let found = HistoryExchange(reqId: "x1", id: "req-10", item: held)
        let back = try AHJSON.value(HistoryExchange.self, from: try AHJSON.tree(found))
        XCTAssertEqual(back, found)
        XCTAssertEqual(back.item?.pictures.map(\.assetId), ["a"])
    }

    func testTheNewMessagesAreRegistered() {
        let types = MessageRegistry.all.map { $0.type }
        XCTAssertTrue(types.contains(HistoryExchangeGet.type))
        XCTAssertTrue(types.contains(HistoryExchange.type))
    }

    func testTheLimitsAreTheMacs() {
        // `HISTORY_PICTURES_MAX` and `HISTORY_THUMB_MAX_BYTES` in `src-tauri/src/ios/wire.rs`.
        XCTAssertEqual(Wire.historyPicturesMax, 12)
        XCTAssertEqual(Wire.historyThumbMaxBytes, 6144)
    }
}
