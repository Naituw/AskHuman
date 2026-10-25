import Foundation
import CryptoKit
import AHProtocol
import AHStore

// Attachments (plan §3.7): files an Agent attached are pulled from the Mac in windows of up to
// eight chunks; files the user attaches to a reply are pushed in chunks with a rolling
// acknowledgement. Both verify a SHA-256 over the whole file.

struct Download {
    var macId: String
    var assetId: String
    var total: UInt64?
    var size: UInt64?
    var sha256: String?
    var chunks: [UInt64: Data] = [:]
    /// First index of the window currently requested.
    var windowStart: UInt64 = 0
    var waiters: [UUID: CheckedContinuation<URL, Error>] = [:]
    var progress: [UUID: @Sendable (Double) -> Void] = [:]
    var retried = false
}

/// Acknowledgements of one upload, queued until the sender asks for them.
struct UploadMailbox {
    var acks: [UploadAck] = []
    var waiter: CheckedContinuation<UploadAck, Error>?
}

extension PhoneEngine {
    static let windowTimeout: TimeInterval = 20
    static let uploadAckTimeout: TimeInterval = 15
    static let uploadWindow: UInt64 = 4

    // MARK: downloads

    func cachedAssetURL(_ assetId: String) -> URL {
        config.assetDirectory.appendingPathComponent(assetId, isDirectory: false)
    }

    public func download(macId: String, asset: AssetRef, progress: @escaping @Sendable (Double) -> Void) async throws -> URL {
        guard Bytes.isValidId(asset.assetId) else { throw BackendError.invalid("Bad asset id.") }
        if asset.tooLarge { throw BackendError.tooLarge }
        let url = cachedAssetURL(asset.assetId)
        if let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
           (attributes[.size] as? NSNumber)?.uint64Value == asset.size {
            progress(1)
            return url
        }
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }

        let waiter = UUID()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<URL, Error>) in
                if downloads[asset.assetId] == nil {
                    downloads[asset.assetId] = Download(macId: macId, assetId: asset.assetId)
                    downloads[asset.assetId]?.size = asset.size
                    Task { await self.requestWindow(asset.assetId, from: 0) }
                }
                downloads[asset.assetId]?.waiters[waiter] = cont
                downloads[asset.assetId]?.progress[waiter] = progress
            }
        } onCancel: {
            Task { await self.cancelDownloadWaiter(asset.assetId, waiter) }
        }
    }

    func cancelDownloadWaiter(_ assetId: String, _ waiter: UUID) {
        guard var download = downloads[assetId] else { return }
        download.waiters.removeValue(forKey: waiter)?.resume(throwing: BackendError.cancelled)
        download.progress[waiter] = nil
        downloads[assetId] = download
        if download.waiters.isEmpty {
            downloads[assetId] = nil
            cancelTimer("asset." + assetId)
        }
    }

    func requestWindow(_ assetId: String, from index: UInt64) async {
        guard var download = downloads[assetId] else { return }
        download.windowStart = index
        downloads[assetId] = download
        await send(AssetGet(assetId: assetId, from: index, count: UInt64(Wire.maxAssetChunksPerGet)), to: download.macId)
        after("asset." + assetId, Self.windowTimeout) { engine in await engine.windowTimedOut(assetId) }
    }

    func windowTimedOut(_ assetId: String) async {
        guard var download = downloads[assetId] else { return }
        if download.retried {
            failDownload(assetId, with: BackendError.timedOut)
            return
        }
        download.retried = true
        downloads[assetId] = download
        // Ask again from the first chunk we still miss.
        var next = download.windowStart
        while download.chunks[next] != nil { next += 1 }
        await requestWindow(assetId, from: next)
    }

    func onAssetChunk(_ macId: String, _ chunk: AssetChunk) async {
        guard var download = downloads[chunk.assetId], download.macId == macId else { return }
        guard let bytes = Bytes.b64uDecode(chunk.data), bytes.count <= Wire.assetChunkBytes, chunk.index < chunk.total,
              chunk.size <= UInt64(Wire.maxAssetBytes)
        else {
            failDownload(chunk.assetId, with: BackendError.invalid("Bad chunk."))
            return
        }
        download.total = chunk.total
        download.size = chunk.size
        if let sha = chunk.sha256 { download.sha256 = sha.lowercased() }
        download.chunks[chunk.index] = bytes
        downloads[chunk.assetId] = download

        let fraction = Double(download.chunks.count) / Double(max(1, chunk.total))
        for report in download.progress.values { report(min(0.99, fraction)) }

        if UInt64(download.chunks.count) == chunk.total {
            finishDownload(chunk.assetId)
            return
        }
        // The window is complete once all of its chunks are in: ask for the next one.
        let end = min(download.windowStart + UInt64(Wire.maxAssetChunksPerGet), chunk.total)
        if (download.windowStart..<end).allSatisfy({ download.chunks[$0] != nil }) {
            cancelTimer("asset." + chunk.assetId)
            downloads[chunk.assetId]?.retried = false
            await requestWindow(chunk.assetId, from: end)
        }
    }

    func onAssetError(_ macId: String, _ error: AssetError) {
        guard let download = downloads[error.assetId], download.macId == macId else { return }
        failDownload(error.assetId, with: error.reason == "tooLarge" ? BackendError.tooLarge : BackendError.unavailable(error.reason))
    }

    func finishDownload(_ assetId: String) {
        guard let download = downloads[assetId], let total = download.total else { return }
        var data = Data()
        for index in 0..<total {
            guard let part = download.chunks[index] else { return failDownload(assetId, with: BackendError.invalid("Missing chunk.")) }
            data.append(part)
        }
        guard UInt64(data.count) == download.size else { return failDownload(assetId, with: BackendError.invalid("Size mismatch.")) }
        if let expected = download.sha256 {
            let actual = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            guard actual == expected else { return failDownload(assetId, with: BackendError.invalid("Checksum mismatch.")) }
        }
        let url = cachedAssetURL(assetId)
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: url, options: .atomic)
        } catch {
            return failDownload(assetId, with: BackendError.unavailable("Could not store the file."))
        }
        cancelTimer("asset." + assetId)
        downloads[assetId] = nil
        for report in download.progress.values { report(1) }
        for waiter in download.waiters.values { waiter.resume(returning: url) }
    }

    func failDownload(_ assetId: String, with error: Error) {
        guard let download = downloads.removeValue(forKey: assetId) else { return }
        cancelTimer("asset." + assetId)
        for waiter in download.waiters.values { waiter.resume(throwing: error) }
    }

    /// Fails the transfers of one Mac, or all of them when `macId` is `nil`.
    func failTransfers(for macId: String? = nil, with error: BackendError) {
        for (assetId, download) in downloads where macId == nil || download.macId == macId {
            failDownload(assetId, with: error)
        }
        for (uploadId, mailbox) in uploadMailboxes {
            if macId == nil || uploadOwners[uploadId] == macId {
                mailbox.waiter?.resume(throwing: error)
                uploadMailboxes[uploadId] = nil
            }
        }
    }

    func failTransfers(with error: BackendError) { failTransfers(for: nil, with: error) }

    // MARK: uploads

    public func upload(
        macId: String, requestId: String, source: UploadSource, progress: @escaping @Sendable (Double) -> Void
    ) async throws -> String {
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        guard requests[CachedRequest.key(macId, requestId)] != nil else { throw BackendError.unknownRequest }
        guard !source.data.isEmpty, source.data.count <= Wire.maxAssetBytes else { throw BackendError.tooLarge }

        let uploadId = Bytes.newId()
        uploadOwners[uploadId] = macId
        uploadMailboxes[uploadId] = UploadMailbox()
        defer {
            uploadMailboxes[uploadId] = nil
            uploadOwners[uploadId] = nil
        }
        let size = UInt64(source.data.count)
        let chunkSize = Wire.assetChunkBytes
        let chunks = max(1, (source.data.count + chunkSize - 1) / chunkSize)
        let digest = SHA256.hash(data: source.data).map { String(format: "%02x", $0) }.joined()

        func fail(_ ack: UploadAck) -> BackendError {
            switch ack.reason {
            case "tooLarge": return .tooLarge
            case "gone": return .unknownRequest
            case .some(let reason): return .unavailable(reason)
            case .none: return .unavailable("upload failed")
            }
        }

        try Task.checkCancellation()
        await send(
            UploadBegin(
                uploadId: uploadId, name: source.name, mime: source.mime, size: size, chunks: UInt64(chunks), sha256: digest,
                scope: UploadScope(requestId: requestId)
            ),
            to: macId
        )
        let begun = try await nextUploadAck(uploadId)
        guard begun.status != "error" else { throw fail(begun) }

        var next = Int(begun.nextIndex)
        var sinceAck = 0
        while next < chunks {
            try Task.checkCancellation()
            let start = next * chunkSize
            let end = min(start + chunkSize, source.data.count)
            let part = source.data.subdata(in: start..<end)
            let sent = await send(UploadChunk(uploadId: uploadId, index: UInt64(next), data: Bytes.b64u(part)), to: macId)
            guard sent else { throw BackendError.macOffline }
            progress(Double(next + 1) / Double(chunks) * 0.98)
            next += 1
            sinceAck += 1
            if sinceAck >= Int(Self.uploadWindow) || next == chunks {
                sinceAck = 0
                let ack = try await nextUploadAck(uploadId)
                if ack.status == "error" { throw fail(ack) }
                if ack.status == "complete" { break }
                // The Mac asks for a different chunk (one got lost): continue where it says.
                next = Int(ack.nextIndex)
            }
        }
        progress(1)
        return uploadId
    }

    /// Waits for the next `upload.ack` of an upload; throws on timeout or cancellation.
    func nextUploadAck(_ uploadId: String) async throws -> UploadAck {
        if var mailbox = uploadMailboxes[uploadId], !mailbox.acks.isEmpty {
            let ack = mailbox.acks.removeFirst()
            uploadMailboxes[uploadId] = mailbox
            return ack
        }
        after("upload." + uploadId, Self.uploadAckTimeout) { engine in await engine.uploadAckTimedOut(uploadId) }
        defer { cancelTimer("upload." + uploadId) }
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<UploadAck, Error>) in
                if var mailbox = uploadMailboxes[uploadId] {
                    mailbox.waiter = cont
                    uploadMailboxes[uploadId] = mailbox
                } else {
                    cont.resume(throwing: BackendError.cancelled)
                }
            }
        } onCancel: {
            Task { await self.cancelUploadWait(uploadId) }
        }
    }

    func cancelUploadWait(_ uploadId: String) {
        guard var mailbox = uploadMailboxes[uploadId], let waiter = mailbox.waiter else { return }
        mailbox.waiter = nil
        uploadMailboxes[uploadId] = mailbox
        waiter.resume(throwing: BackendError.cancelled)
    }

    func uploadAckTimedOut(_ uploadId: String) {
        guard var mailbox = uploadMailboxes[uploadId], let waiter = mailbox.waiter else { return }
        mailbox.waiter = nil
        uploadMailboxes[uploadId] = mailbox
        waiter.resume(throwing: BackendError.timedOut)
    }

    func onUploadAck(_ macId: String, _ ack: UploadAck) {
        guard uploadOwners[ack.uploadId] == macId, var mailbox = uploadMailboxes[ack.uploadId] else { return }
        if let waiter = mailbox.waiter {
            mailbox.waiter = nil
            uploadMailboxes[ack.uploadId] = mailbox
            waiter.resume(returning: ack)
        } else {
            mailbox.acks.append(ack)
            uploadMailboxes[ack.uploadId] = mailbox
        }
    }
}
