import Darwin
import Foundation

/// A line-oriented unix-socket server: every `\n`-terminated line is a request, the reply is one line.
/// Blocking POSIX I/O on dedicated threads (a handful of short conversations a day), the async handler
/// is bridged with a semaphore so replies stay in request order on a connection.
final class RpcSocketServer {
    enum StartError: Error, Equatable {
        case pathTooLong
        case alreadyRunning
        case system(String, Int32)
    }

    typealias Handler = (Data) async -> Data

    private let path: String
    private let handler: Handler
    private let log: (String, [String: Any]) -> Void
    private var listenFD: Int32 = -1
    private var stopped = false
    private let lock = NSLock()

    init(path: String, log: @escaping (String, [String: Any]) -> Void = { _, _ in }, handler: @escaping Handler) {
        self.path = path
        self.log = log
        self.handler = handler
    }

    func start() throws {
        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        guard path.utf8.count < capacity else { throw StartError.pathTooLong }
        withUnsafeMutablePointer(to: &address.sun_path) { pointer in
            pointer.withMemoryRebound(to: CChar.self, capacity: capacity) { dst in
                _ = path.withCString { strncpy(dst, $0, capacity - 1) }
            }
        }

        let directory = (path as NSString).deletingLastPathComponent
        try? FileManager.default.createDirectory(
            atPath: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])

        // A socket file left behind by a crash must not block us; a live one means another helper.
        if FileManager.default.fileExists(atPath: path) {
            if Self.probe(address) { throw StartError.alreadyRunning }
            unlink(path)
        }

        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw StartError.system("socket", errno) }
        var off: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &off, socklen_t(MemoryLayout<Int32>.size))

        let previousMask = umask(0o177)  // the socket file is created 0600
        let bound = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        let bindError = errno
        umask(previousMask)
        guard bound == 0 else {
            close(fd)
            throw StartError.system("bind", bindError)
        }
        guard listen(fd, 8) == 0 else {
            let e = errno
            close(fd)
            unlink(path)
            throw StartError.system("listen", e)
        }
        lock.lock()
        listenFD = fd
        stopped = false
        lock.unlock()

        let thread = Thread { [weak self] in self?.acceptLoop(fd) }
        thread.name = "todosync.accept"
        thread.start()
    }

    func stop() {
        lock.lock()
        stopped = true
        let fd = listenFD
        listenFD = -1
        lock.unlock()
        if fd >= 0 {
            shutdown(fd, SHUT_RDWR)
            close(fd)
            unlink(path)
        }
    }

    private var isStopped: Bool {
        lock.lock()
        defer { lock.unlock() }
        return stopped
    }

    /// True when something answers on the socket.
    private static func probe(_ address: sockaddr_un) -> Bool {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        var copy = address
        let result = withUnsafePointer(to: &copy) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        return result == 0
    }

    private func acceptLoop(_ fd: Int32) {
        while !isStopped {
            let client = accept(fd, nil, nil)
            if client < 0 {
                if isStopped { return }
                if errno == EINTR { continue }
                log("accept_failed", ["errno": Int(errno)])
                return
            }
            var off: Int32 = 1
            setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &off, socklen_t(MemoryLayout<Int32>.size))
            guard Self.peerIsCurrentUser(client) else {
                log("peer_rejected", [:])
                close(client)
                continue
            }
            let thread = Thread { [weak self] in self?.serve(client) }
            thread.name = "todosync.conn"
            thread.start()
        }
    }

    private static func peerIsCurrentUser(_ fd: Int32) -> Bool {
        var uid: uid_t = 0
        var gid: gid_t = 0
        return getpeereid(fd, &uid, &gid) == 0 && uid == geteuid()
    }

    private func serve(_ fd: Int32) {
        defer { close(fd) }
        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 64 * 1024)
        while true {
            while let newline = buffer.firstIndex(of: 0x0A) {
                let line = buffer.subdata(in: buffer.startIndex..<newline)
                buffer.removeSubrange(buffer.startIndex...newline)
                if line.isEmpty { continue }
                let reply = respond(to: line)
                guard Self.writeAll(fd, reply + Data([0x0A])) else { return }
            }
            if buffer.count > Wire.maxLineBytes {
                _ = Self.writeAll(fd, WireCoder.encode(FailureReply(id: 0, error: WireError(code: RpcCode.badRequest, retryAfterMs: nil, message: "line too long"))) + Data([0x0A]))
                return
            }
            let n = chunk.withUnsafeMutableBytes { read(fd, $0.baseAddress, $0.count) }
            if n > 0 {
                buffer.append(contentsOf: chunk[0..<n])
            } else if n == 0 {
                return
            } else if errno != EINTR {
                return
            }
        }
    }

    private func respond(to line: Data) -> Data {
        final class Box: @unchecked Sendable { var value = Data() }
        let box = Box()
        let done = DispatchSemaphore(value: 0)
        let handler = self.handler
        Task {
            box.value = await handler(line)
            done.signal()
        }
        done.wait()
        return box.value
    }

    private static func writeAll(_ fd: Int32, _ data: Data) -> Bool {
        var offset = 0
        return data.withUnsafeBytes { raw -> Bool in
            while offset < raw.count {
                let n = write(fd, raw.baseAddress! + offset, raw.count - offset)
                if n > 0 {
                    offset += n
                } else if n < 0 && errno == EINTR {
                    continue
                } else {
                    return false
                }
            }
            return true
        }
    }
}
