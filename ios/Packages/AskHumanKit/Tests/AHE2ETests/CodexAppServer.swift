import Foundation

/// A minimal client of `codex app-server`: JSON-RPC, one JSON object per line over stdio. The Codex end-to-end tests use
/// it for one thing, archiving the threads they created (`thread/archive` moves a thread to `archived_sessions`, so
/// it can be undone).
final class CodexAppServer: @unchecked Sendable {
    private let process = Process()
    private let input = Pipe()
    private let output = Pipe()
    private let lock = NSLock()
    private var pending = Data()
    private var replies: [Int: [String: Any]] = [:]
    private var nextId = 0

    init(executable: String, environment: [String: String]) {
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = ["app-server"]
        process.environment = environment
        process.currentDirectoryURL = FileManager.default.temporaryDirectory
        process.standardInput = input
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        // A pipe at end of file stays readable: the handler has to remove itself (see `Cmd`).
        output.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let chunk = handle.availableData
            guard !chunk.isEmpty, let self else {
                handle.readabilityHandler = nil
                return
            }
            self.receive(chunk)
        }
    }

    func start() throws { try process.run() }

    /// Keeps the replies (messages with an `id` and a `result` or an `error`); notifications and requests of the
    /// server are of no interest here.
    private func receive(_ chunk: Data) {
        lock.withLock {
            pending.append(chunk)
            while let newline = pending.firstIndex(of: 0x0A) {
                let line = pending.subdata(in: pending.startIndex..<newline)
                pending.removeSubrange(pending.startIndex...newline)
                guard let object = try? JSONSerialization.jsonObject(with: line) as? [String: Any],
                      let id = object["id"] as? Int,
                      object["result"] != nil || object["error"] != nil
                else { continue }
                replies[id] = object
            }
        }
    }

    private func send(_ message: [String: Any]) throws {
        let line = try JSONSerialization.data(withJSONObject: message) + Data([0x0A])
        try input.fileHandleForWriting.write(contentsOf: line)
    }

    private func request(_ method: String, _ params: [String: Any], timeout: TimeInterval = 30) async throws {
        let id = lock.withLock { () -> Int in
            defer { nextId += 1 }
            return nextId
        }
        try send(["method": method, "id": id, "params": params])
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let reply = lock.withLock({ replies.removeValue(forKey: id) }) {
                if let error = reply["error"] { throw E2EError("codex app-server refused \(method): \(error)") }
                return
            }
            guard process.isRunning else { throw E2EError("codex app-server exited before it answered \(method)") }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        throw E2EError("codex app-server did not answer \(method) within \(Int(timeout)) s")
    }

    func initialize() async throws {
        try await request("initialize", [
            "clientInfo": ["name": "askhuman-e2e", "title": "AskHuman end-to-end tests", "version": "0"],
            "capabilities": NSNull(),
        ])
        try send(["method": "initialized"])
    }

    func archive(thread id: String) async throws {
        try await request("thread/archive", ["threadId": id])
    }

    /// Closes its input (which ends it) and makes sure it is gone.
    func stop() async {
        try? input.fileHandleForWriting.close()
        let deadline = Date().addingTimeInterval(3)
        while process.isRunning, Date() < deadline { try? await Task.sleep(nanoseconds: 50_000_000) }
        if process.isRunning { process.terminate() }
        output.fileHandleForReading.readabilityHandler = nil
    }
}
