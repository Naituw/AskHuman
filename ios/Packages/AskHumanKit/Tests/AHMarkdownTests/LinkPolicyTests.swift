import XCTest
@testable import AHMarkdown

final class LinkPolicyTests: XCTestCase {
    private func action(_ text: String, file: StaticString = #filePath, line: UInt = #line) -> LinkAction? {
        guard let url = URL(string: text) else {
            XCTFail("not a URL: \(text)", file: file, line: line)
            return nil
        }
        return LinkPolicy.action(for: url)
    }

    func testWebAddressesOpenInTheBrowserAfterAskingTheDomain() {
        XCTAssertEqual(action("https://example.com/docs"), .browser)
        XCTAssertEqual(action("http://example.com"), .browser)
        XCTAssertEqual(action("HTTPS://Example.com"), .browser, "the scheme is not case-sensitive")
    }

    func testMailAndPhoneLinksGoStraightToTheSystem() {
        XCTAssertEqual(action("mailto:hi@example.com"), .system)
        XCTAssertEqual(action("tel:+8613800000000"), .system)
        XCTAssertEqual(action("TEL:123"), .system)
    }

    /// The point of the policy: nothing is listed, so a scheme nobody has heard of is treated like any other.
    func testAnySchemeThatIsNotRefusedLeadsToAnApp() {
        for link in [
            "myapp://debug/panel?id=42", "shortcuts://run-shortcut?name=Build", "maps://?q=AskHuman", "sms:123",
            "x-apple-reminderkit://", "cursor://file/Users/me/a.swift:12", "obsidian://open?vault=Notes",
            "ahdemo-unclaimed://open/panel?id=42&name=%E4%B8%AD%E6%96%87", "MYAPP://Upper", "myapp:", "a+b.c-d://x",
        ] {
            XCTAssertEqual(action(link), .app, link)
        }
    }

    func testSchemesThatOpenNothingAreIgnored() {
        for link in ["javascript:alert(1)", "JavaScript:alert(1)", "vbscript:x", "file:///etc/hosts", "data:text/html,hi"] {
            XCTAssertEqual(action(link), .ignore, link)
        }
    }

    func testALinkWithoutASchemeIsIgnored() {
        XCTAssertEqual(action("docs/readme.md"), .ignore, "a relative path has nothing to be relative to here")
        XCTAssertEqual(action("#section"), .ignore)
        XCTAssertEqual(action("/absolute/path"), .ignore)
    }

    /// The native renderer only asks the policy about links its parser kept: a scheme the parser dropped would never
    /// reach it, and the two renderers would differ again.
    func testTheNativeParserKeepsEverySchemeThePolicyCanFollow() {
        let blocks = MarkdownParser.parse("[a](myapp://debug/panel?id=42) [b](maps://?q=x) [c](https://example.com) [d](mailto:a@b.c)")
        guard case .paragraph(let paragraph) = blocks.first else { return XCTFail("paragraph") }
        XCTAssertEqual(
            paragraph.runs.compactMap(\.link).map(\.absoluteString),
            ["myapp://debug/panel?id=42", "maps://?q=x", "https://example.com", "mailto:a@b.c"]
        )
    }
}
