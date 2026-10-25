import Foundation
import XCTest

/// One child process with captured output. Output is collected from background handlers so that a
/// chatty child never blocks on a full pipe.
final class Cmd: @unchecked Sendable {
    let process = Process()
    private let outPipe = Pipe()
    private let errPipe = Pipe()
    private let inPipe = Pipe()
    private let lock = NSLock()
    private var outData = Data()
    private var errData = Data()

    init(_ executable: String, _ arguments: [String], env: [String: String], cwd: URL, stdin: String? = nil) {
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        process.environment = env
        process.currentDirectoryURL = cwd
        process.standardOutput = outPipe
        process.standardError = errPipe
        process.standardInput = inPipe
        // At end of file the handler must remove itself: a pipe that reached EOF stays readable, so
        // a handler left in place is called again and again, and every finished child would keep a
        // core busy until the test run ends.
        outPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let chunk = handle.availableData
            guard !chunk.isEmpty, let self else {
                handle.readabilityHandler = nil
                return
            }
            self.lock.withLock { self.outData.append(chunk) }
        }
        errPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let chunk = handle.availableData
            guard !chunk.isEmpty, let self else {
                handle.readabilityHandler = nil
                return
            }
            self.lock.withLock { self.errData.append(chunk) }
        }
        pendingStdin = stdin
    }

    private var pendingStdin: String?

    func start() throws {
        try process.run()
        if let text = pendingStdin { inPipe.fileHandleForWriting.write(Data(text.utf8)) }
        try? inPipe.fileHandleForWriting.close()
    }

    var stdout: String { lock.withLock { String(decoding: outData, as: UTF8.self) } }
    var stderr: String { lock.withLock { String(decoding: errData, as: UTF8.self) } }
    var isRunning: Bool { process.isRunning }

    /// Waits for the child to exit; `nil` when it is still running after `timeout`.
    @discardableResult
    func wait(timeout: TimeInterval) async -> Int32? {
        let deadline = Date().addingTimeInterval(timeout)
        while process.isRunning {
            if Date() > deadline { return nil }
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        // Let the output handlers drain.
        try? await Task.sleep(nanoseconds: 60_000_000)
        return process.terminationStatus
    }

    func terminate() {
        if process.isRunning { process.terminate() }
    }

    func interrupt() {
        if process.isRunning { process.interrupt() }
    }

    /// Waits until a line of stdout satisfies `match` and returns it.
    func waitForLine(timeout: TimeInterval, _ match: (String) -> Bool) async -> String? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let line = stdout.split(separator: "\n").map(String.init).first(where: match) { return line }
            if !process.isRunning && stdout.isEmpty { break }
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        return nil
    }
}

struct CmdResult {
    var status: Int32
    var stdout: String
    var stderr: String
}

/// Runs a command to completion.
func run(_ executable: String, _ arguments: [String], env: [String: String], cwd: URL, stdin: String? = nil, timeout: TimeInterval = 60) async throws -> CmdResult {
    let cmd = Cmd(executable, arguments, env: env, cwd: cwd, stdin: stdin)
    try cmd.start()
    guard let status = await cmd.wait(timeout: timeout) else {
        cmd.terminate()
        throw E2EError("timed out: \(URL(fileURLWithPath: executable).lastPathComponent) \(arguments.prefix(3).joined(separator: " "))")
    }
    return CmdResult(status: status, stdout: cmd.stdout, stderr: cmd.stderr)
}

struct E2EError: Error, CustomStringConvertible {
    var description: String
    init(_ description: String) { self.description = description }
}
