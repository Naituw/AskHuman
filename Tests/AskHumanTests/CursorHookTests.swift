import XCTest
@testable import AskHuman

final class CursorHookLogicTests: XCTestCase {
    func testUpsertIsIdempotentAndKeepsOthers() {
        let root: [String: Any] = [
            "version": 1,
            "hooks": [
                "preToolUse": [
                    ["command": "/other/app/hook.sh", "matcher": "Shell"]
                ]
            ]
        ]
        var updated = CursorHook.upsertEntry(into: root, scriptPath: "/home/.cursor/hooks/humaninloop-timeout.sh")
        XCTAssertTrue(CursorHook.rootHasMarker(updated))
        // 再次安装应幂等（数量不变）
        updated = CursorHook.upsertEntry(into: updated, scriptPath: "/home/.cursor/hooks/humaninloop-timeout.sh")
        let hooks = updated["hooks"] as! [String: Any]
        let pre = hooks["preToolUse"] as! [[String: Any]]
        XCTAssertEqual(pre.count, 2) // 其他应用 + 本应用
    }

    func testRemoveKeepsOthersAndClearsKeyWhenEmpty() {
        let root: [String: Any] = [
            "hooks": [
                "preToolUse": [
                    ["command": "/x/humaninloop-timeout.sh", "matcher": "Shell"]
                ]
            ]
        ]
        let removed = CursorHook.removeEntries(from: root)
        XCTAssertFalse(CursorHook.rootHasMarker(removed))
        let hooks = removed["hooks"] as! [String: Any]
        XCTAssertNil(hooks["preToolUse"]) // 空数组时删除键

        let root2: [String: Any] = [
            "hooks": [
                "preToolUse": [
                    ["command": "/other/hook.sh", "matcher": "Shell"],
                    ["command": "/x/humaninloop-timeout.sh", "matcher": "Shell"]
                ]
            ]
        ]
        let removed2 = CursorHook.removeEntries(from: root2)
        let pre2 = (removed2["hooks"] as! [String: Any])["preToolUse"] as! [[String: Any]]
        XCTAssertEqual(pre2.count, 1)
        XCTAssertEqual(pre2[0]["command"] as? String, "/other/hook.sh")
    }
}

final class CursorHookScriptTests: XCTestCase {
    private func runScript(command: String) throws -> String {
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("hook-test-\(UUID().uuidString).sh")
        try CursorHook.scriptContent.write(to: tmp, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tmp.path)
        defer { try? FileManager.default.removeItem(at: tmp) }

        let inputJSON = try String(
            data: JSONSerialization.data(withJSONObject: ["tool_input": ["command": command]]),
            encoding: .utf8
        ) ?? "{}"

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/bash")
        process.arguments = [tmp.path]
        let stdin = Pipe()
        let stdout = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        try process.run()
        stdin.fileHandleForWriting.write(inputJSON.data(using: .utf8)!)
        stdin.fileHandleForWriting.closeFile()
        process.waitUntilExit()
        let out = stdout.fileHandleForReading.readDataToEndOfFile()
        return String(data: out, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    func testMatches() throws {
        XCTAssertTrue(try runScript(command: "AskHuman \"你好\"").contains("86400000"))
        XCTAssertTrue(try runScript(command: "cmd1 && AskHuman --option a \"问题\" && cmd2").contains("86400000"))
        XCTAssertTrue(try runScript(command: "/usr/local/bin/AskHuman --settings").contains("86400000"))
    }

    func testDoesNotMatch() throws {
        XCTAssertEqual(try runScript(command: "echo AskHuman_log.txt"), "{}")
        XCTAssertEqual(try runScript(command: "AskHumanFoo arg"), "{}")
        XCTAssertEqual(try runScript(command: "ls -la"), "{}")
    }
}
