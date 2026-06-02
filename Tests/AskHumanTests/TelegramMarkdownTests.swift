import XCTest
@testable import AskHuman

final class TelegramMarkdownTests: XCTestCase {
    func testMarkdownProcessing() {
        let input = "# 标题\n\n**粗体文本**\n\n`代码`\n\n```rust\nfn main() {}\n```"
        let result = TelegramMarkdown.process(input)
        XCTAssertTrue(result.contains(">标题"))
        XCTAssertTrue(result.contains("*粗体文本*"))
        XCTAssertTrue(result.contains("```rust\nfn main() {}\n```"))
    }

    func testSpecialCharEscaping() {
        let input = "测试_下划线和[方括号]"
        let result = TelegramMarkdown.process(input)
        XCTAssertTrue(result.contains("测试\\_下划线和\\[方括号\\]"))
    }

    func testInlineCodePreserved() {
        let input = "执行 `ls -la` 命令"
        let result = TelegramMarkdown.process(input)
        XCTAssertTrue(result.contains("`ls -la`"))
    }
}
