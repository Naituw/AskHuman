import Foundation

/// Append-only text log with size-based rotation. It records WHAT happened (event names, counts,
/// durations, error codes) and never todo text, record names, tokens or account identifiers.
final class HelperLog {
    private let queue = DispatchQueue(label: "todosync.log")
    private var url: URL?
    private var maxBytes = 1 << 20
    private var keep = 3
    private var handle: FileHandle?
    private var written = 0

    static let shared = HelperLog()

    func configure(url: URL, maxBytes: Int = 1 << 20, keep: Int = 3) {
        queue.sync {
            self.url = url
            self.maxBytes = maxBytes
            self.keep = keep
            try? FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            self.openHandle()
        }
    }

    func write(_ event: String, _ fields: [String: Any] = [:]) {
        let stamp = Self.timestamp()
        queue.async {
            guard self.url != nil else { return }
            var line = "\(stamp) \(event)"
            for key in fields.keys.sorted() {
                line += " \(key)=\(Self.render(fields[key]))"
            }
            line += "\n"
            let data = Data(line.utf8)
            if self.written + data.count > self.maxBytes { self.rotate() }
            if self.handle == nil { self.openHandle() }
            try? self.handle?.write(contentsOf: data)
            self.written += data.count
        }
    }

    /// Blocks until everything queued so far is on disk (tests, process exit).
    func flush() {
        queue.sync {}
    }

    private func openHandle() {
        guard let url else { return }
        if !FileManager.default.fileExists(atPath: url.path) {
            FileManager.default.createFile(atPath: url.path, contents: nil, attributes: [.posixPermissions: 0o600])
        }
        handle = try? FileHandle(forWritingTo: url)
        written = Int((try? handle?.seekToEnd()) ?? 0)
    }

    private func rotate() {
        guard let url else { return }
        try? handle?.close()
        handle = nil
        let fm = FileManager.default
        for index in stride(from: keep, through: 1, by: -1) {
            let from = index == 1 ? url : url.appendingPathExtension("\(index - 1)")
            let to = url.appendingPathExtension("\(index)")
            try? fm.removeItem(at: to)
            if fm.fileExists(atPath: from.path) { try? fm.moveItem(at: from, to: to) }
        }
        openHandle()
    }

    private static func render(_ value: Any?) -> String {
        switch value {
        case let s as String: return s.contains(" ") ? "\"\(s)\"" : s
        case let n as NSNumber: return n.stringValue
        case nil: return "nil"
        default: return String(describing: value!)
        }
    }

    private static func timestamp() -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.string(from: Date())
    }
}
