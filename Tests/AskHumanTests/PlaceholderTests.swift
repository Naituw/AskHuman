import XCTest
@testable import AskHuman

final class ArgumentParserTests: XCTestCase {
    func testMessageOnly() throws {
        let p = try ArgumentParser.parseAsk(["hello"])
        XCTAssertEqual(p.message, "hello")
        XCTAssertTrue(p.options.isEmpty)
        XCTAssertTrue(p.isMarkdown)
    }

    func testMultipleOptions() throws {
        let p = try ArgumentParser.parseAsk(["msg", "-o", "A", "--option", "B"])
        XCTAssertEqual(p.message, "msg")
        XCTAssertEqual(p.options, ["A", "B"])
    }

    func testNoMarkdown() throws {
        let p = try ArgumentParser.parseAsk(["msg", "--no-markdown"])
        XCTAssertFalse(p.isMarkdown)
    }

    func testUnknownFlag() {
        XCTAssertThrowsError(try ArgumentParser.parseAsk(["--foo"]))
    }

    func testMultipleMessages() {
        XCTAssertThrowsError(try ArgumentParser.parseAsk(["a", "b"]))
    }

    func testMissingOptionValue() {
        XCTAssertThrowsError(try ArgumentParser.parseAsk(["msg", "-o"]))
    }

    func testMissingMessage() {
        XCTAssertThrowsError(try ArgumentParser.parseAsk(["--no-markdown"]))
    }
}

final class OutputFormatterTests: XCTestCase {
    func testAllSections() {
        let out = OutputFormatter.sendOutput(
            selectedOptions: ["A", "B"],
            userInput: "hi",
            imagePaths: ["/tmp/a.png"]
        )
        XCTAssertEqual(out, "[选择的选项]\nA, B\n\n[用户输入]\nhi\n\n[图片]\n/tmp/a.png")
    }

    func testEmptyFallback() {
        let out = OutputFormatter.sendOutput(selectedOptions: [], userInput: "  ", imagePaths: [])
        XCTAssertEqual(out, "[用户输入]\n用户确认继续")
    }

    func testCancel() {
        XCTAssertTrue(OutputFormatter.cancelOutput().hasPrefix("[状态]\n"))
    }
}

final class ImageWriterTests: XCTestCase {
    func testSanitize() {
        XCTAssertEqual(ImageWriter.sanitize("/etc/passwd", fallbackExt: "png"), "passwd")
        XCTAssertEqual(ImageWriter.sanitize("../../foo.png", fallbackExt: "png"), "foo.png")
        XCTAssertEqual(ImageWriter.sanitize("", fallbackExt: "png"), "img.png")
        XCTAssertEqual(ImageWriter.sanitize(".....", fallbackExt: "png"), "img.png")
    }

    func testExtensionMapping() {
        XCTAssertEqual(ImageWriter.fileExtension(forMediaType: "image/png"), "png")
        XCTAssertEqual(ImageWriter.fileExtension(forMediaType: "image/JPEG"), "jpg")
        XCTAssertEqual(ImageWriter.fileExtension(forMediaType: "application/unknown"), "bin")
    }

    func testDecodeDataURI() {
        let b64 = "iVBORw0KGgo="
        let bytes = ImageWriter.decodeBase64("data:image/png;base64,\(b64)")
        XCTAssertEqual(bytes, Data(base64Encoded: b64))
    }

    func testConfigDefaults() {
        let c = AppConfig()
        XCTAssertEqual(c.general.theme, .system)
        XCTAssertTrue(c.channels.popup.enabled)
        XCTAssertFalse(c.channels.telegram.enabled)
    }
}
