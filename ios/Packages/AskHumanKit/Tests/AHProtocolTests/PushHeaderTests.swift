import XCTest
@testable import AHProtocol

/// The two lines above the body of a notification, which have little room: what leads, and what a
/// narrow line cuts off first.
final class PushHeaderTests: XCTestCase {
    private func plain(agent: String = "Claude Code", project: String = "relay", mac: String = "Home Mac", session: String? = nil) -> PushPlain {
        PushPlain(k: "ask", rid: "r1", mac: mac, agent: agent, project: project, session: session, text: "Pick a database", ts: 1)
    }

    func testWithoutASessionTitleTheAgentAndProjectLeadAndTheMacFollows() {
        let lines = plain().headerLines
        XCTAssertEqual(lines.title, "Claude Code · relay")
        XCTAssertEqual(lines.subtitle, "Home Mac")
    }

    func testASessionTitleLeadsAndTheMacComesLast() {
        let lines = plain(session: "Fix the flaky login test").headerLines
        XCTAssertEqual(lines.title, "Fix the flaky login test")
        XCTAssertEqual(lines.subtitle, "Claude Code · relay · Home Mac", "the Mac is last: a narrow line cuts it first")
    }

    func testABlankSessionTitleIsNoTitle() {
        XCTAssertEqual(plain(session: "  \n ").headerLines.title, "Claude Code · relay")
        XCTAssertEqual(plain(session: "").headerLines.subtitle, "Home Mac")
    }

    func testMissingPartsLeaveNoDanglingSeparators() {
        let noProject = plain(project: "", session: "Fix it").headerLines
        XCTAssertEqual(noProject.subtitle, "Claude Code · Home Mac")
        // A push that names neither Agent nor project (such as a notice from the Mac itself).
        let bare = plain(agent: "", project: "").headerLines
        XCTAssertEqual(bare.title, "Home Mac")
        XCTAssertEqual(bare.subtitle, "")
        let bareWithTitle = plain(agent: "", project: "", session: "Fix it").headerLines
        XCTAssertEqual(bareWithTitle.title, "Fix it")
        XCTAssertEqual(bareWithTitle.subtitle, "Home Mac")
    }

    func testThePlainTextOfAPushWithoutASessionOmitsTheField() throws {
        let data = try JSONEncoder().encode(plain())
        let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertNil(object["session"], "older phones and Macs see exactly what they always saw")
        let titled = try JSONEncoder().encode(plain(session: "Fix it"))
        XCTAssertEqual((try JSONSerialization.jsonObject(with: titled) as? [String: Any])?["session"] as? String, "Fix it")
        // And a plain text from an older Mac decodes with no session.
        let old = try JSONDecoder().decode(
            PushPlain.self, from: Data(#"{"k":"ask","mac":"Home","agent":"A","project":"p","text":"t","ts":1}"#.utf8)
        )
        XCTAssertNil(old.session)
    }
}
