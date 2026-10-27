import CloudKit
import Darwin
import XCTest

@testable import TodoSyncHelperKit

private func makeTempDirectory(_ prefix: String = "ahts") throws -> URL {
    // Unix socket paths are limited to ~100 bytes: keep the directory name short and under /tmp.
    let url = URL(fileURLWithPath: "/tmp/\(prefix)-\(UInt32.random(in: 0..<UInt32.max))", isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

final class HelperConfigTests: XCTestCase {
    func testParsesTheCoreHandOverFile() throws {
        let json = #"{"protocol":1,"askhuman":"/Applications/AskHuman.app/Contents/MacOS/AskHuman","stateDir":"/Users/x/.askhuman/state/todo-sync","todosPath":"/Users/x/.askhuman/state/todos.json","pollMinutes":5,"enabled":true}"#
        let config = try XCTUnwrap(HelperConfig.parse(Data(json.utf8)))
        XCTAssertEqual(config.askhuman, "/Applications/AskHuman.app/Contents/MacOS/AskHuman")
        XCTAssertEqual(config.pollMinutes, 5)
        XCTAssertTrue(config.enabled)
    }

    func testDefaultsAndClamping() throws {
        let base = #""askhuman":"/a","stateDir":"/s","todosPath":"/t""#
        let defaults = try XCTUnwrap(HelperConfig.parse(Data("{\(base)}".utf8)))
        XCTAssertEqual(defaults.pollMinutes, HelperConfig.defaultPollMinutes)
        XCTAssertTrue(defaults.enabled)
        XCTAssertEqual(try XCTUnwrap(HelperConfig.parse(Data("{\(base),\"pollMinutes\":0}".utf8))).pollMinutes, 1)
        XCTAssertEqual(try XCTUnwrap(HelperConfig.parse(Data("{\(base),\"pollMinutes\":999999}".utf8))).pollMinutes, 1_440)
        XCTAssertFalse(try XCTUnwrap(HelperConfig.parse(Data("{\(base),\"enabled\":false}".utf8))).enabled)
    }

    func testRejectsIncompleteOrBrokenFiles() {
        XCTAssertNil(HelperConfig.parse(Data("{}".utf8)))
        XCTAssertNil(HelperConfig.parse(Data(#"{"askhuman":"/a","stateDir":"/s"}"#.utf8)))
        XCTAssertNil(HelperConfig.parse(Data("garbage".utf8)))
    }

    func testPathsFollowTheEnvironmentOverride() {
        let overridden = HelperPaths.resolve(environment: ["ASKHUMAN_TODO_SYNC_DIR": "/tmp/x"], home: URL(fileURLWithPath: "/h"))
        XCTAssertEqual(overridden.socketURL.path, "/tmp/x/helper.sock")
        XCTAssertEqual(overridden.configURL.path, "/tmp/x/helper.json")
        let normal = HelperPaths.resolve(environment: [:], home: URL(fileURLWithPath: "/h"))
        XCTAssertEqual(normal.stateDir.path, "/h/.askhuman/state/todo-sync")
    }
}

final class CloudErrorsTests: XCTestCase {
    private func map(_ code: CKError.Code, _ info: [String: Any] = [:]) -> RpcFailure {
        CloudErrors.failure(from: CKError(code, userInfo: info))
    }

    func testMapsToTheVocabularyTheCoreUnderstands() {
        XCTAssertEqual(map(.networkUnavailable).code, "offline")
        XCTAssertEqual(map(.networkFailure).code, "offline")
        XCTAssertEqual(map(.notAuthenticated).code, "notAuthenticated")
        XCTAssertEqual(map(.zoneNotFound).code, "zoneNotFound")
        XCTAssertEqual(map(.userDeletedZone).code, "zoneNotFound")
        XCTAssertEqual(map(.changeTokenExpired).code, "tokenExpired")
        XCTAssertEqual(map(.quotaExceeded).code, "quota")
        XCTAssertEqual(map(.serverRejectedRequest).code, "other")
    }

    func testTransientAccountAndContainerStatesRetryInsteadOfPausing() {
        XCTAssertEqual(map(.accountTemporarilyUnavailable).code, "offline")
        XCTAssertEqual(map(.badContainer).code, "offline")
    }

    func testRateLimitUsesTheServersHintOrADefault() {
        let hinted = map(.requestRateLimited, [CKErrorRetryAfterKey: 12.5])
        XCTAssertEqual(hinted.code, "rateLimited")
        XCTAssertEqual(hinted.retryAfterMs, 12_500)
        XCTAssertEqual(map(.zoneBusy).retryAfterMs, CloudErrors.defaultRetryAfterMs)
    }

    func testPartialFailureReportsTheMostActionableMember() {
        let inner: [AnyHashable: Error] = [
            "a": CKError(.networkFailure), "b": CKError(.quotaExceeded), "c": CKError(.serverRejectedRequest),
        ]
        let error = CKError(.partialFailure, userInfo: [CKPartialErrorsByItemIDKey: inner])
        XCTAssertEqual(CloudErrors.failure(from: error).code, "quota")
    }

    func testForeignErrorsAreOfflineOrOther() {
        XCTAssertEqual(CloudErrors.failure(from: URLError(.notConnectedToInternet)).code, "offline")
        struct Odd: Error {}
        XCTAssertEqual(CloudErrors.failure(from: Odd()).code, "other")
        let already = RpcFailure(code: "zoneNotFound")
        XCTAssertEqual(CloudErrors.failure(from: already), already)
    }

    func testPerRecordCodes() {
        XCTAssertEqual(CloudErrors.recordCode(from: CKError(.quotaExceeded)), "quotaExceeded")
        XCTAssertEqual(CloudErrors.recordCode(from: CKError(.limitExceeded)), "limitExceeded")
        XCTAssertEqual(CloudErrors.recordCode(from: URLError(.timedOut)), "error")
    }
}

final class SocketServerTests: XCTestCase {
    /// A real unix-socket round trip: bytes in, dispatcher, bytes out.
    private func connect(_ path: String) throws -> Int32 {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        withUnsafeMutablePointer(to: &address.sun_path) { tuple in
            tuple.withMemoryRebound(to: CChar.self, capacity: capacity) { destination in
                _ = path.withCString { source in strncpy(destination, source, capacity - 1) }
            }
        }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        let result = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard result == 0 else { throw NSError(domain: "connect", code: Int(errno)) }
        return fd
    }

    private func exchange(_ fd: Int32, _ line: String) -> String {
        let bytes = Array((line + "\n").utf8)
        _ = bytes.withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
        var out = Data()
        var one = [UInt8](repeating: 0, count: 1)
        while true {
            let n = read(fd, &one, 1)
            if n <= 0 || one[0] == 0x0A { break }
            out.append(one[0])
        }
        return String(decoding: out, as: UTF8.self)
    }

    func testServesRequestsInOrderOnOneConnection() async throws {
        let dir = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let path = dir.appendingPathComponent("s.sock").path
        let dispatcher = RpcDispatcher(
            backend: FixtureBackend(response: ["environment": "production", "container": "c", "accountStatus": "available", "accountHash": "h"]),
            helperVersion: "9.9")
        let server = RpcSocketServer(path: path) { await dispatcher.handle($0) }
        try server.start()
        defer { server.stop() }

        let fd = try connect(path)
        defer { close(fd) }
        let first = exchange(fd, #"{"id":1,"op":"hello"}"#)
        XCTAssertTrue(first.contains(#""helperVersion":"9.9""#), first)
        XCTAssertTrue(first.contains(#""id":1"#))
        let second = exchange(fd, #"{"id":2,"op":"nope"}"#)
        XCTAssertTrue(second.contains(#""code":"badRequest""#), second)
        XCTAssertTrue(second.contains(#""id":2"#))
    }

    func testSocketIsPrivateAndStaleFilesAreReplaced() async throws {
        let dir = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let path = dir.appendingPathComponent("s.sock").path
        // a leftover regular file from a crashed run
        FileManager.default.createFile(atPath: path, contents: Data("x".utf8))
        let backend = FixtureBackend(response: [:])
        let dispatcher = RpcDispatcher(backend: backend, helperVersion: "0")
        let server = RpcSocketServer(path: path) { await dispatcher.handle($0) }
        try server.start()
        defer { server.stop() }

        var info = stat()
        XCTAssertEqual(stat(path, &info), 0)
        XCTAssertEqual(info.st_mode & 0o777, 0o600)

        // a second helper on the same path must give up instead of stealing the socket
        let other = RpcSocketServer(path: path) { await dispatcher.handle($0) }
        XCTAssertThrowsError(try other.start()) { error in
            XCTAssertEqual(error as? RpcSocketServer.StartError, .alreadyRunning)
        }
    }

    func testTooLongPathIsRejected() {
        let server = RpcSocketServer(path: "/tmp/" + String(repeating: "a", count: 120)) { _ in Data() }
        XCTAssertThrowsError(try server.start()) { error in
            XCTAssertEqual(error as? RpcSocketServer.StartError, .pathTooLong)
        }
    }

    func testStopRemovesTheSocketFile() throws {
        let dir = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let path = dir.appendingPathComponent("s.sock").path
        let server = RpcSocketServer(path: path) { _ in Data() }
        try server.start()
        XCTAssertTrue(FileManager.default.fileExists(atPath: path))
        server.stop()
        XCTAssertFalse(FileManager.default.fileExists(atPath: path))
    }
}

final class InstanceLockTests: XCTestCase {
    func testSecondAcquireFailsUntilReleased() throws {
        let dir = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("helper.lock")
        let first = try XCTUnwrap(InstanceLock.acquire(at: url))
        XCTAssertNil(InstanceLock.acquire(at: url))
        first.release()
        XCTAssertNotNil(InstanceLock.acquire(at: url))
    }
}

final class FileChangeWatcherTests: XCTestCase {
    func testAtomicReplaceIsReportedButUnrelatedFilesAreNot() async throws {
        let dir = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let target = dir.appendingPathComponent("todos.json")
        try Data("{}".utf8).write(to: target)

        let lock = NSLock()
        var changes = 0
        let watcher = FileChangeWatcher(fileURL: target) {
            lock.lock()
            changes += 1
            lock.unlock()
        }
        XCTAssertTrue(watcher.start())
        defer { watcher.stop() }

        func count() -> Int {
            lock.lock()
            defer { lock.unlock() }
            return changes
        }

        // unrelated file in the same directory
        try Data("x".utf8).write(to: dir.appendingPathComponent("other.txt"))
        try await Task.sleep(nanoseconds: 300_000_000)
        XCTAssertEqual(count(), 0)

        // the way the core writes: temp file + rename
        let temp = dir.appendingPathComponent("todos.json.tmp")
        try Data(#"{"a":1}"#.utf8).write(to: temp)
        XCTAssertEqual(rename(temp.path, target.path), 0)
        try await Task.sleep(nanoseconds: 400_000_000)
        XCTAssertEqual(count(), 1)

        // writing the same bytes to the same inode again is a change only if size or mtime moved
        try Data(#"{"a":22}"#.utf8).write(to: target)
        try await Task.sleep(nanoseconds: 400_000_000)
        XCTAssertGreaterThanOrEqual(count(), 1)
    }

    func testMissingDirectoryDoesNotStart() {
        let watcher = FileChangeWatcher(fileURL: URL(fileURLWithPath: "/tmp/ahts-no-such-dir/todos.json")) {}
        XCTAssertFalse(watcher.start())
    }
}

final class HelperLogTests: XCTestCase {
    func testRotatesBySizeAndKeepsAFewFiles() throws {
        let dir = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("helper.log")
        let log = HelperLog()
        log.configure(url: url, maxBytes: 600, keep: 2)
        for i in 0..<40 {
            log.write("event", ["i": i, "note": "has space"])
        }
        log.flush()
        let names = try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted()
        XCTAssertEqual(names, ["helper.log", "helper.log.1", "helper.log.2"])
        let current = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(current.contains("note=\"has space\""))
        XCTAssertLessThanOrEqual(current.utf8.count, 700)
    }
}
