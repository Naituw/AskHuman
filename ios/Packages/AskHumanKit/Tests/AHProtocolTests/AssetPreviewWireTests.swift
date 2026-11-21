import XCTest
@testable import AHProtocol

/// The preview additions on the wire (docs/specs/ios-attachment-traffic.md §5.4). Everything is additive: a Mac and a
/// phone of different ages exchange exactly what each of them knows, and what is not there is not sent.
final class AssetPreviewWireTests: XCTestCase {
    private let digest = String(repeating: "ab", count: 32)

    private func caps(assetPreview: Bool) -> Caps {
        Caps(newTask: true, interject: true, watch: true, todos: true, answerTodos: true, away: true, binary: true, assetPreview: assetPreview)
    }

    // MARK: capability

    func testCapsLeaveThePreviewBitOutWhenFalse() throws {
        XCTAssertNil(try AHJSON.tree(caps(assetPreview: false))["assetPreview"], "what predates it sees what it always saw")
        XCTAssertEqual(try AHJSON.tree(caps(assetPreview: true))["assetPreview"], .bool(true))
    }

    func testAMissingPreviewBitReadsAsNo() throws {
        let old = try AHJSON.decoder.decode(Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true,"binary":true}"#.utf8))
        XCTAssertFalse(old.assetPreview)
        let new = try AHJSON.decoder.decode(Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true,"assetPreview":true}"#.utf8))
        XCTAssertTrue(new.assetPreview)
        for flag in [false, true] {
            XCTAssertEqual(try AHJSON.value(Caps.self, from: try AHJSON.tree(caps(assetPreview: flag))), caps(assetPreview: flag))
        }
    }

    // MARK: a file of a request

    func testAFileLeavesOutThePreviewAndTheHashWhenItHasNone() throws {
        let plain = AssetRef(assetId: "a1", name: "log.txt", size: 10, mime: "text/plain", isImage: false)
        let tree = try AHJSON.tree(plain)
        XCTAssertNil(tree["preview"])
        XCTAssertNil(tree["sha256"])

        let picture = AssetRef(
            assetId: "a2", name: "shot.png", size: 3_145_728, mime: "image/png", isImage: true, thumb: "/9j/4AAQ",
            preview: true, sha256: digest
        )
        let shown = try AHJSON.tree(picture)
        XCTAssertEqual(shown["preview"], .bool(true))
        XCTAssertEqual(shown["sha256"], .string(digest))
        XCTAssertEqual(try AHJSON.value(AssetRef.self, from: shown), picture)
    }

    func testAFileFromAMacThatPredatesThemReadsAsHavingNeither() throws {
        let old = try AHJSON.decoder.decode(
            AssetRef.self,
            from: Data(#"{"assetId":"a1","name":"a.png","size":1,"mime":"image/png","isImage":true}"#.utf8)
        )
        XCTAssertFalse(old.preview)
        XCTAssertNil(old.sha256)
    }

    // MARK: the ask and the answer

    func testTheAskNamesAVariantOnlyWhenItWantsOne() throws {
        let file = AssetGet(assetId: "a1", from: 0, count: 4)
        XCTAssertNil(try AHJSON.tree(file)["variant"], "an older Mac is not sent a field it never read")
        let preview = AssetGet(assetId: "a1", from: 0, count: 4, variant: "preview")
        XCTAssertEqual(try AHJSON.tree(preview)["variant"], .string("preview"))
        XCTAssertEqual(try AHJSON.value(AssetGet.self, from: try AHJSON.tree(preview)), preview)
        let old = try AHJSON.decoder.decode(AssetGet.self, from: Data(#"{"assetId":"a1","from":0,"count":4}"#.utf8))
        XCTAssertNil(old.variant)
    }

    func testTheChunksNameTheirVariantOnlyWhenTheMacSaidSo() throws {
        let file = AssetChunk(assetId: "a1", index: 0, total: 3, size: 500_000, sha256: nil, data: "AAEC")
        XCTAssertNil(try AHJSON.tree(file)["variant"], "the chunks of a request without a variant are what they always were")
        let original = AssetChunk(assetId: "a1", index: 0, total: 3, size: 500_000, sha256: digest, variant: "original", data: "AAEC")
        XCTAssertEqual(try AHJSON.tree(original)["variant"], .string("original"))
        XCTAssertEqual(try AHJSON.value(AssetChunk.self, from: try AHJSON.tree(original)), original)
        let old = try AHJSON.decoder.decode(
            AssetChunk.self, from: Data(#"{"assetId":"a1","index":0,"total":1,"size":3,"data":"AAEC"}"#.utf8)
        )
        XCTAssertNil(old.variant)
        // The header of a blob frame has no `data`, and the variant stays in it.
        let head = AssetChunk(assetId: "a1", index: 1, total: 1, size: 3, sha256: nil, variant: "preview", data: "")
        XCTAssertEqual(try Plain(blobHeader: head, nowMs: 5).b["variant"], .string("preview"))
    }
}
