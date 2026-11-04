import Foundation

// Tests for NotesRules.swift. Build and run (no XCTest, no credentials, no network):
//   xcrun swiftc -parse-as-library NotesRules.swift NotesRulesTests.swift -o /tmp/notes-rules-tests
//   /tmp/notes-rules-tests notes-rules-cases.json
// scripts/ios-testflight-changes.node-test.mjs does exactly this.

@main
enum NotesRulesTests {
    static var checks = 0
    static var failures = 0

    static func check(_ condition: Bool, _ message: @autoclosure () -> String) {
        checks += 1
        if !condition {
            failures += 1
            print("FAIL: \(message())")
        }
    }

    static func expectReason(_ reason: String, _ name: String, _ body: () throws -> Void) {
        do {
            try body()
            check(false, "\(name): expected rejection \(reason), but it succeeded")
        } catch let error as NotesRejection {
            check(error.reason == reason, "\(name): expected \(reason), got \(error.reason)")
        } catch {
            check(false, "\(name): unexpected error \(error)")
        }
    }

    /// Replaces {"$repeat": text, "$count": n, "$suffix": text?} with the repeated string, anywhere in a JSON value.
    static func expand(_ value: Any) -> Any {
        if let object = value as? [String: Any] {
            if let unit = object["$repeat"] as? String, let count = object["$count"] as? Int {
                return String(repeating: unit, count: count) + (object["$suffix"] as? String ?? "")
            }
            return object.mapValues(expand)
        }
        if let array = value as? [Any] { return array.map(expand) }
        return value
    }

    static func sharedCases(_ path: String) throws {
        guard let cases = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: path))) as? [String: Any] else {
            check(false, "cases file is not a JSON object")
            return
        }
        for item in cases["canonicalize"] as? [[String: Any]] ?? [] {
            let name = "canonicalize: \(item["name"] as? String ?? "?")"
            guard let input = expand(item["input"] as Any) as? String else { check(false, "\(name): no input"); continue }
            if let reason = item["error"] as? String {
                expectReason(reason, name) { _ = try NotesRules.canonicalize(input) }
            } else {
                let expected = expand(item["expect"] as Any) as? String
                check((try? NotesRules.canonicalize(input)) == expected, "\(name): wrong result")
            }
        }
        for item in cases["build"] as? [[String: Any]] ?? [] {
            let value = item["value"] as? String ?? ""
            let accepted = (try? NotesRules.validateBuild(value)) != nil
            check(accepted == (item["ok"] as? Bool), "build \(value.debugDescription): expected ok=\(item["ok"] as? Bool ?? false)")
        }
        for item in cases["request"] as? [[String: Any]] ?? [] {
            let name = "request: \(item["name"] as? String ?? "?")"
            guard let data = try? JSONSerialization.data(withJSONObject: expand(item["request"] as Any)) else { check(false, "\(name): cannot encode"); continue }
            if let reason = item["error"] as? String {
                expectReason(reason, name) { _ = try NotesRules.parseRequest(data) }
            } else if let expect = expand(item["expect"] as Any) as? [String: Any], let request = try? NotesRules.parseRequest(data) {
                check(request.mode.rawValue == expect["mode"] as? String, "\(name): mode")
                check(request.build == expect["build"] as? String, "\(name): build")
                check(request.texts == (expect["texts"] as? [String: String] ?? [:]), "\(name): texts")
            } else {
                check(false, "\(name): expected a parsed request")
            }
        }
    }

    static func gateRules() {
        func allowed(_ gate: NotesGate, _ method: String, _ path: String) -> Bool {
            (try? gate.authorize(method: method, path: path)) != nil
        }
        let collection = NotesGate.collection
        let read = NotesGate(mode: .read)
        check(allowed(read, "GET", "/v1/builds"), "read may list builds")
        check(allowed(read, "GET", collection), "read may list localizations")
        check(!allowed(read, "POST", collection), "read may not create")
        try? read.learn("abc")
        check(!allowed(read, "PATCH", collection + "/abc"), "read may not patch")
        check(!allowed(read, "DELETE", collection + "/abc"), "read may not delete")

        let write = NotesGate(mode: .write)
        check(allowed(write, "POST", collection), "write may create")
        check(!allowed(write, "PATCH", collection + "/abc"), "write may not patch an ID it has not seen")
        try? write.learn("abc")
        check(allowed(write, "PATCH", collection + "/abc"), "write may patch a listed ID")
        try? write.recordCreated("new-1")
        check(allowed(write, "PATCH", collection + "/new-1"), "write may patch what it created")
        check(!allowed(write, "DELETE", collection + "/new-1"), "write never deletes")

        let probe = NotesGate(mode: .probe)
        check(allowed(probe, "POST", collection), "probe may create")
        try? probe.learn("listed")
        check(!allowed(probe, "DELETE", collection + "/listed"), "probe may not delete a listed entry")
        check(!allowed(probe, "DELETE", collection + "/mine"), "probe may not delete an ID it did not create")
        try? probe.recordCreated("mine")
        check(allowed(probe, "DELETE", collection + "/mine"), "probe may delete what it created")
        check(!allowed(probe, "PATCH", collection + "/mine"), "probe never patches")
        check(!allowed(probe, "DELETE", collection + "/mine/extra"), "no nested paths")
        check(!allowed(probe, "DELETE", collection + "/../apps/1"), "no path traversal")
        check(!allowed(probe, "DELETE", collection), "no delete on the collection")

        for gate in [read, write, probe] {
            check(!allowed(gate, "GET", "/v1/apps"), "no other collections")
            check(!allowed(gate, "GET", "/v1/builds/123"), "no single-build path")
            check(!allowed(gate, "GET", "/v1/betaGroups"), "no beta groups")
            check(!allowed(gate, "PUT", "/v1/builds"), "no PUT")
            check(!allowed(gate, "DELETE", "/v1/builds"), "no build delete")
            check(!allowed(gate, "GET", "v1/builds"), "path must be absolute")
        }
        expectReason("badIdentifier", "learn rejects a path-like ID") { try NotesGate(mode: .write).learn("a/b") }
        expectReason("badIdentifier", "learn rejects an empty ID") { try NotesGate(mode: .write).learn("") }
        check(NotesGate.isIdentifier("0b5c0a2e-7c1e-4a4e-9a5e-1d6f8a2b3c4d"), "UUIDs are identifiers")
        check(!NotesGate.isIdentifier(String(repeating: "a", count: 65)), "identifiers are at most 64 bytes")
    }

    static func plans() throws {
        let existing = ["en-US": RemoteNote(id: "e1", text: "Build 7")]
        check(NotesRules.writePlan(existing: existing, texts: ["en-US": "Build 7", "zh-Hans": "构建 7"])
            == [NotesStep(locale: "en-US", action: .unchanged), NotesStep(locale: "zh-Hans", action: .create)], "unchanged and create")
        check(NotesRules.writePlan(existing: existing, texts: ["en-US": "Build 8"])
            == [NotesStep(locale: "en-US", action: .update(id: "e1"))], "update")
        check(NotesRules.writePlan(existing: [:], texts: [:]).isEmpty, "empty plan")

        check(try NotesRules.probeLocales(existing: []) == ["en-US", "zh-Hans"], "probe uses both free locales")
        check(try NotesRules.probeLocales(existing: ["en-US"]) == ["zh-Hans"], "probe skips an existing locale")
        check(try NotesRules.probeLocales(existing: ["ja"]) == ["en-US", "zh-Hans"], "unrelated locales do not block the probe")
        expectReason("probeRefused", "probe refuses when both locales have entries") { _ = try NotesRules.probeLocales(existing: ["en-US", "zh-Hans"]) }

        let canonicalProbe = try NotesRules.canonicalize(NotesRules.probeText)
        check(canonicalProbe == NotesRules.probeText, "the probe text is already canonical")
        check(canonicalProbe.contains("\n") && canonicalProbe.contains("•") && canonicalProbe.unicodeScalars.contains { $0.value > 0x2E80 }, "the probe text covers newline, bullet and CJK")
    }

    static func responses() throws {
        let body: [String: Any] = ["data": [
            ["id": "a1", "attributes": ["locale": "en-US", "whatsNew": "Hello"]],
            ["id": "b2", "attributes": ["locale": "zh-Hans", "whatsNew": NSNull()]],
        ]]
        let parsed = try NotesRules.parseLocalizations(body)
        check(parsed == ["en-US": RemoteNote(id: "a1", text: "Hello"), "zh-Hans": RemoteNote(id: "b2", text: "")], "parses entries, null text becomes empty")
        check((try NotesRules.parseLocalizations(["data": []])).isEmpty, "an empty list is valid")
        expectReason("badResponse", "missing data") { _ = try NotesRules.parseLocalizations([:]) }
        expectReason("badResponse", "missing locale") { _ = try NotesRules.parseLocalizations(["data": [["id": "x", "attributes": [:]]]]) }
        expectReason("badResponse", "duplicate locale") {
            _ = try NotesRules.parseLocalizations(["data": [
                ["id": "x", "attributes": ["locale": "en-US"]], ["id": "y", "attributes": ["locale": "en-US"]],
            ]])
        }

        let long = String(repeating: "d", count: 1000)
        let forbidden = Data("{\"errors\":[{\"status\":\"403\",\"code\":\"FORBIDDEN_ERROR\",\"title\":\"Forbidden\",\"detail\":\"\(long)\"}]}".utf8)
        let summary = NotesRules.appleErrorSummary(status: 403, body: forbidden)
        check(summary.contains("HTTP 403") && summary.contains("FORBIDDEN_ERROR") && summary.contains("authorization problem"), "403 summary")
        check(!summary.contains(String(repeating: "d", count: 301)), "details are cut at 300 characters")
        check(!summary.contains("Bearer"), "no credentials in a summary")
        check(NotesRules.appleErrorSummary(status: 500, body: Data("not json".utf8)) == "Apple rejected the notes request (HTTP 500).", "non-JSON body")
        check(!NotesRules.appleErrorSummary(status: 409, body: Data()).contains("authorization"), "only 401 and 403 mention authorization")
    }

    static func requestFiles() throws {
        let folder = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("notes-rules-tests-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: folder) }
        func file(_ name: String, _ contents: Data, mode: mode_t) -> String {
            let path = folder.appendingPathComponent(name).path
            FileManager.default.createFile(atPath: path, contents: contents)
            chmod(path, mode)
            return path
        }
        let valid = Data("{\"mode\":\"read\",\"build\":\"6\"}".utf8)
        check((try? NotesRules.loadRequest(path: file("ok.json", valid, mode: 0o600))) == NotesRequest(mode: .read, build: "6", texts: [:]), "an owner-only file loads")
        expectReason("badRequestFile", "group-readable") { _ = try NotesRules.loadRequest(path: file("group.json", valid, mode: 0o640)) }
        expectReason("badRequestFile", "world-readable") { _ = try NotesRules.loadRequest(path: file("world.json", valid, mode: 0o644)) }
        expectReason("badRequestFile", "missing") { _ = try NotesRules.loadRequest(path: folder.appendingPathComponent("absent.json").path) }
        expectReason("badRequestFile", "directory") { _ = try NotesRules.loadRequest(path: folder.path) }
        let target = file("target.json", valid, mode: 0o600)
        let link = folder.appendingPathComponent("link.json").path
        symlink(target, link)
        expectReason("badRequestFile", "symlink") { _ = try NotesRules.loadRequest(path: link) }
        let oversized = Data("{\"mode\":\"read\",\"build\":\"6\"}".utf8) + Data(repeating: 0x20, count: NotesRules.maxRequestBytes)
        expectReason("badRequestFile", "oversized") { _ = try NotesRules.loadRequest(path: file("big.json", oversized, mode: 0o600)) }
        expectReason("badRequest", "not JSON") { _ = try NotesRules.loadRequest(path: file("junk.json", Data("nope".utf8), mode: 0o600)) }
    }

    static func main() {
        guard CommandLine.arguments.count == 2 else {
            print("Usage: notes-rules-tests <notes-rules-cases.json>")
            exit(2)
        }
        do {
            try sharedCases(CommandLine.arguments[1])
            gateRules()
            try plans()
            try responses()
            try requestFiles()
        } catch {
            check(false, "unexpected error: \(error)")
        }
        print("NotesRulesTests: \(checks) checks, \(failures) failed")
        exit(failures == 0 ? 0 : 1)
    }
}
