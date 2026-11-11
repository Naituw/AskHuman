import Foundation
import UniformTypeIdentifiers
import AHProtocol

/// A cross-process file promise. Constructing it neither downloads a file nor opens a background task.
public final class AttachmentFilePromise: @unchecked Sendable {
    public typealias Loader = @Sendable (@escaping @Sendable (Double) -> Void) async throws -> URL
    public typealias Activity = @MainActor @Sendable (@escaping @Sendable () -> Void) -> (@MainActor @Sendable () -> Void)

    public let typeIdentifier: String
    public let filename: String
    private let load: Loader
    private let activity: Activity
    private let lock = NSLock()
    private var directories: [URL] = []

    public init(asset: AssetRef, activity: @escaping Activity = { _ in {} }, load: @escaping Loader) {
        let mime = UTType(mimeType: asset.mime)
        let ext = UTType(filenameExtension: (asset.name as NSString).pathExtension)
        let type = (mime == .data || mime == .content) ? (ext ?? mime) : (mime ?? ext)
        typeIdentifier = (type ?? .data).identifier
        let name = asset.name.replacingOccurrences(of: "\\", with: "_").replacingOccurrences(of: "/", with: "_")
            .components(separatedBy: .controlCharacters).joined()
        filename = name.isEmpty || name == "." || name == ".." ? "file" : name
        self.activity = activity
        self.load = load
    }

    deinit {
        for directory in directories { try? FileManager.default.removeItem(at: directory) }
    }

    public func provider() -> NSItemProvider {
        let provider = NSItemProvider()
        provider.suggestedName = filename
        register(typeIdentifier, on: provider)
        // A receiver may not know the specific file type (for example, a Markdown UTI).
        if typeIdentifier != UTType.data.identifier { register(UTType.data.identifier, on: provider) }
        return provider
    }

    private func register(_ type: String, on provider: NSItemProvider) {
        provider.registerFileRepresentation(forTypeIdentifier: type, fileOptions: [], visibility: .all) { [self] completion in
            let operation = Operation()
            let task = Task { @MainActor in
                let end = activity { operation.cancel() }
                defer { end(); operation.finished() }
                var exported: URL?
                do {
                    try Task.checkCancellation()
                    let source = try await load { operation.progress.completedUnitCount = Int64(min(1, max(0, $0)) * 1000) }
                    try Task.checkCancellation()
                    let copy = try await Task.detached { try self.ownedCopy(of: source) }.value
                    exported = copy
                    try Task.checkCancellation()
                    completion(copy, false, nil)
                } catch {
                    if let exported { self.discard(exported) }
                    completion(nil, false, error)
                }
            }
            operation.install(task)
            return operation.progress
        }
    }

    private func discard(_ file: URL) {
        let directory = file.deletingLastPathComponent()
        lock.withLock { directories.removeAll { $0 == directory } }
        try? FileManager.default.removeItem(at: directory)
    }

    private func ownedCopy(of source: URL) throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ah-export-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let target = directory.appendingPathComponent(filename)
        do {
            try FileManager.default.copyItem(at: source, to: target)
            // The promise retains its own file even if the original preview or cache disappears.
            lock.withLock { directories.append(directory) }
            return target
        } catch {
            try? FileManager.default.removeItem(at: directory)
            throw error
        }
    }

    /// Cancellation can arrive before the main-actor task has begun, or on the provider's loading queue.
    private final class Operation: @unchecked Sendable {
        let progress = Progress(totalUnitCount: 1000)
        private let lock = NSLock()
        private var task: Task<Void, Never>?
        private var cancelled = false
        private var completed = false

        init() { progress.cancellationHandler = { [weak self] in self?.cancel() } }

        func install(_ task: Task<Void, Never>) {
            let cancel = lock.withLock {
                if !completed { self.task = task }
                return cancelled
            }
            if cancel { task.cancel() }
        }

        func cancel() {
            let current = lock.withLock { cancelled = true; return task }
            current?.cancel()
        }

        func finished() {
            lock.withLock { task = nil; completed = true }
            progress.cancellationHandler = nil
        }
    }
}
