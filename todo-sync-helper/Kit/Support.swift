import Darwin
import Foundation

/// One helper per state directory. `flock` is released by the kernel when the process dies, so a crash
/// never leaves a stale lock behind.
final class InstanceLock {
    private var fd: Int32

    private init(fd: Int32) {
        self.fd = fd
    }

    static func acquire(at url: URL) -> InstanceLock? {
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])
        let fd = open(url.path, O_CREAT | O_RDWR, 0o600)
        guard fd >= 0 else { return nil }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else {
            close(fd)
            return nil
        }
        return InstanceLock(fd: fd)
    }

    func release() {
        guard fd >= 0 else { return }
        flock(fd, LOCK_UN)
        close(fd)
        fd = -1
    }

    deinit { release() }
}

/// Used when the process cannot talk to CloudKit at all (missing entitlement, unsigned build). The RPC
/// server still answers `hello` and `status`, so the core can show a precise reason instead of a timeout.
final class UnavailableBackend: CloudBackend {
    private let environment: String
    private let container: String
    private let problem: String

    init(environment: String, container: String, problem: String) {
        self.environment = environment
        self.container = container
        self.problem = problem
    }

    func hello() async -> HelloInfo {
        HelloInfo(environment: environment, container: container, accountStatus: "unknown", accountHash: nil, problem: problem)
    }

    func ensure() async throws { throw unavailable() }
    func fetch(token: String?) async throws -> FetchResult { throw unavailable() }
    func save(_ records: [OutRecord]) async throws -> [SaveOutcome] { throw unavailable() }
    func deleteZone() async throws { throw unavailable() }

    private func unavailable() -> RpcFailure {
        RpcFailure(code: RpcCode.other, message: problem)
    }
}

/// Watches the directory that holds a file and reports when that file changed (identity, size or
/// modification time). `todos.json` is replaced by rename, which changes the directory, not the old inode,
/// so the directory is the right thing to watch.
final class FileChangeWatcher {
    struct Signature: Equatable {
        var inode: UInt64
        var size: Int64
        var mtimeNs: Int64
    }

    private let fileURL: URL
    private let queue: DispatchQueue
    private let onChange: () -> Void
    private var source: DispatchSourceFileSystemObject?
    private var last: Signature?

    init(fileURL: URL, queue: DispatchQueue = DispatchQueue(label: "todosync.filewatch"), onChange: @escaping () -> Void) {
        self.fileURL = fileURL
        self.queue = queue
        self.onChange = onChange
    }

    /// False when the directory does not exist (yet).
    @discardableResult
    func start() -> Bool {
        stop()
        let directory = fileURL.deletingLastPathComponent().path
        let fd = open(directory, O_EVTONLY)
        guard fd >= 0 else { return false }
        last = Self.signature(of: fileURL)
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fd, eventMask: [.write, .extend, .attrib, .link, .rename, .delete], queue: queue)
        source.setEventHandler { [weak self] in self?.check() }
        source.setCancelHandler { close(fd) }
        self.source = source
        source.resume()
        return true
    }

    func stop() {
        source?.cancel()
        source = nil
    }

    private func check() {
        let now = Self.signature(of: fileURL)
        guard now != last else { return }
        last = now
        onChange()
    }

    static func signature(of url: URL) -> Signature? {
        var info = stat()
        guard stat(url.path, &info) == 0 else { return nil }
        let ns = Int64(info.st_mtimespec.tv_sec) * 1_000_000_000 + Int64(info.st_mtimespec.tv_nsec)
        return Signature(inode: UInt64(info.st_ino), size: Int64(info.st_size), mtimeNs: ns)
    }

    deinit { stop() }
}

enum Clock {
    static func nowMs() -> Int64 {
        Int64(Date().timeIntervalSince1970 * 1000)
    }
}
