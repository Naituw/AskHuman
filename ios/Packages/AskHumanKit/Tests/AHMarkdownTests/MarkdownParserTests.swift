import XCTest
@testable import AHMarkdown

final class MarkdownParserTests: XCTestCase {
    private func text(_ block: MarkdownBlock) -> String { block.plainText }

    func testHeadingsAndParagraphs() {
        let blocks = MarkdownParser.parse("# Title\n\nFirst paragraph.\n\n## Sub\n\nSecond one.")
        XCTAssertEqual(blocks.count, 4)
        guard case .heading(let level, let title) = blocks[0] else { return XCTFail("heading") }
        XCTAssertEqual(level, 1)
        XCTAssertEqual(String(title.characters), "Title")
        XCTAssertEqual(text(blocks[1]), "First paragraph.")
        guard case .heading(let sub, _) = blocks[2] else { return XCTFail("sub heading") }
        XCTAssertEqual(sub, 2)
        XCTAssertEqual(text(blocks[3]), "Second one.")
    }

    func testInlineFormattingStaysInTheAttributedText() {
        let blocks = MarkdownParser.parse("Some **bold**, *italic*, `code` and [a link](https://example.com).")
        guard case .paragraph(let paragraph) = blocks.first else { return XCTFail("paragraph") }
        XCTAssertEqual(String(paragraph.characters), "Some bold, italic, code and a link.")
        let intents = paragraph.runs.compactMap { $0.inlinePresentationIntent }
        XCTAssertTrue(intents.contains(.stronglyEmphasized))
        XCTAssertTrue(intents.contains(.emphasized))
        XCTAssertTrue(intents.contains(.code))
        XCTAssertEqual(paragraph.runs.compactMap(\.link).map(\.absoluteString), ["https://example.com"])
    }

    func testListsKeepOrderNestingAndStartNumbers() {
        let blocks = MarkdownParser.parse("- one\n- two\n  - nested\n- three\n\n3. third\n4. fourth")
        guard case .list(let ordered, _, let items) = blocks[0] else { return XCTFail("bullet list") }
        XCTAssertFalse(ordered)
        XCTAssertEqual(items.count, 3)
        XCTAssertEqual(items[0].blocks.map(text), ["one"])
        XCTAssertEqual(items[1].blocks.count, 2)
        guard case .list(_, _, let nested) = items[1].blocks[1] else { return XCTFail("nested list") }
        XCTAssertEqual(nested.first?.blocks.map(text), ["nested"])

        guard case .list(let numbered, let start, let second) = blocks[1] else { return XCTFail("numbered list") }
        XCTAssertTrue(numbered)
        XCTAssertEqual(start, 3)
        XCTAssertEqual(second.count, 2)
    }

    func testBlockQuotesCodeBlocksAndRules() {
        let blocks = MarkdownParser.parse("> quoted\n> text\n\n```swift\nlet x = 1\nprint(x)\n```\n\n---\n\nafter")
        guard case .quote(let inner) = blocks[0] else { return XCTFail("quote") }
        XCTAssertEqual(inner.map(text), ["quoted text"])
        XCTAssertEqual(blocks[1], .code(language: "swift", text: "let x = 1\nprint(x)"))
        XCTAssertEqual(blocks[2], .rule)
        XCTAssertEqual(text(blocks[3]), "after")
    }

    func testTablesCarryHeaderRowsAndAlignment() {
        let blocks = MarkdownParser.parse("| Name | Count | Note |\n|:--|--:|:-:|\n| a | 1 | x |\n| b | 22 | y |")
        guard case .table(let table) = blocks.first else { return XCTFail("table") }
        XCTAssertEqual(table.header.map { String($0.characters) }, ["Name", "Count", "Note"])
        XCTAssertEqual(table.rows.count, 2)
        XCTAssertEqual(table.rows[1].map { String($0.characters) }, ["b", "22", "y"])
        XCTAssertEqual(table.alignments, [.leading, .trailing, .center])
    }

    func testImagesBecomeBlocksOfTheirOwn() {
        let blocks = MarkdownParser.parse("Before ![diagram](ahasset://abc123) after")
        XCTAssertEqual(blocks.count, 3)
        XCTAssertEqual(text(blocks[0]).trimmingCharacters(in: .whitespaces), "Before")
        XCTAssertEqual(blocks[1], .image(source: "ahasset://abc123", alt: "diagram"))
        XCTAssertEqual(text(blocks[2]).trimmingCharacters(in: .whitespaces), "after")
    }

    func testEmptyAndPlainInput() {
        XCTAssertEqual(MarkdownParser.parse(""), [])
        XCTAssertEqual(MarkdownParser.parse("just text").map(text), ["just text"])
    }

    func testPlainTextDropsMarkupAndCollapsesWhitespace() {
        XCTAssertEqual(MarkdownParser.plainText("# Heading\n\nSome **bold** text\n\n- a\n- b"), "Heading Some bold text a b")
    }
}
