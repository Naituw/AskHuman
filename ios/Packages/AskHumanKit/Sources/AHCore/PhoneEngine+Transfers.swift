import Foundation
import CryptoKit
import AHProtocol
import AHStore

// Attachments (plan §3.7): files an Agent attached are pulled from the Mac in windows of up to
// eight chunks; files the user attaches to a reply are pushed in chunks with a rolling
// acknowledgement. Both verify a SHA-256 over the whole file.

/// What a download brings: the file itself, or the downscaled preview of a picture the Mac makes on request
/// (docs/specs/ios-attachment-traffic.md §5.4). The raw values are what the wire calls them (`AssetGet.variant`,
/// `AssetChunk.variant`).
enum AssetVariant: String, Sendable {
    case original
    case preview
}

/// A download is told apart by what it brings as well as by the asset: the preview and the file of one asset may be on
/// their way at the same time, and every chunk says which of the two it belongs to (§7.1).
struct DownloadKey: Hashable, Sendable {
    var assetId: String
    var variant: AssetVariant
}

struct Download {
    var macId: String
    var assetId: String
    var total: UInt64?
    var size: UInt64?
    var sha256: String?
    var chunks: [UInt64: Data] = [:]
    /// First index of the window currently requested.
    var windowStart: UInt64 = 0
    var waiters: [UUID: CheckedContinuation<ViewableAsset, Error>] = [:]
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

    /// The caller owns a bounded OS background execution task and cancels this operation on expiry.
    public func downloadForExport(macId: String, asset: AssetRef, progress: @escaping @Sendable (Double) -> Void) async throws -> URL {
        guard Bytes.isValidId(asset.assetId) else { throw BackendError.invalid("Bad asset id.") }
        if asset.tooLarge { throw BackendError.tooLarge }
        let cached = cachedAssetURL(asset.assetId)
        if let attributes = try? FileManager.default.attributesOfItem(atPath: cached.path),
           (attributes[.size] as? NSNumber)?.uint64Value == asset.size {
            return try await download(macId: macId, asset: asset, progress: progress)
        }
        guard macs[macId] != nil else { throw BackendError.macOffline }
        let token = UUID()
        assetExports.insert(token)
        startConnecting()
        do {
            // A receiver may ask for the promise only after the ordinary foreground connection closed.
            let deadline = clock.now().addingTimeInterval(15)
            while departing || macs[macId]?.sync != .synced {
                try Task.checkCancellation()
                if presenceSettled, macs[macId]?.online != true { throw BackendError.macOffline }
                if clock.now() >= deadline { throw BackendError.timedOut }
                try await clock.sleep(seconds: 0.1)
            }
            try Task.checkCancellation()
            let url = try await download(macId: macId, asset: asset, progress: progress)
            await finishAssetExport(token)
            return url
        } catch {
            await finishAssetExport(token)
            throw error
        }
    }

    private func finishAssetExport(_ token: UUID) async {
        assetExports.remove(token)
        if !foreground, assetExports.isEmpty, !departing {
            await goodbye()
            if foreground || !assetExports.isEmpty { startConnecting() }
        }
    }

    func exportBackgroundPresence() async {
        guard subscribed, !exportPresencePublished, let active = activeSettings else { return }
        exportPresencePublished = true
        let presence = Presence(o: 1, t: clock.nowMs() / 1000, r: "phone", fg: 0)
        if let payload = try? AHJSON.encoder.encode(presence) {
            _ = await publishPayload(topic: Wire.topicPresence(user: active.username, deviceId: vault.phoneId()), payload: payload, retain: true)
        }
        // Foreground may have returned while the publication was in flight.
        if foreground {
            foregroundPublished = false
            await ensureForegroundPresence()
        }
    }

    func cachedAssetURL(_ assetId: String) -> URL {
        config.assetDirectory.appendingPathComponent(assetId, isDirectory: false)
    }

    /// Where the preview of a picture is kept, apart from the file (`cachedAssetURL`), which may arrive later.
    func cachedPreviewURL(_ assetId: String) -> URL {
        config.assetDirectory.appendingPathComponent(assetId + ".preview", isDirectory: false)
    }

    /// The file of an attachment, whatever else is on the phone of it.
    public func download(macId: String, asset: AssetRef, progress: @escaping @Sendable (Double) -> Void) async throws -> URL {
        try await fetch(macId: macId, asset: asset, preferPreview: false, progress: progress).url
    }

    public func downloadForViewing(
        macId: String, asset: AssetRef, progress: @escaping @Sendable (Double) -> Void
    ) async throws -> ViewableAsset {
        try await fetch(macId: macId, asset: asset, preferPreview: true, progress: progress)
    }

    /// The one path of every download (docs/specs/ios-attachment-traffic.md §7.1). A file that is on the phone serves
    /// every purpose. Otherwise a preview is what comes when the caller prefers one, the asset offers it, and the Mac
    /// declared in this session that it makes them; in every other case it is the file.
    private func fetch(
        macId: String, asset: AssetRef, preferPreview: Bool, progress: @escaping @Sendable (Double) -> Void
    ) async throws -> ViewableAsset {
        guard Bytes.isValidId(asset.assetId) else { throw BackendError.invalid("Bad asset id.") }
        if asset.tooLarge { throw BackendError.tooLarge }
        let url = cachedAssetURL(asset.assetId)
        if let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
           (attributes[.size] as? NSNumber)?.uint64Value == asset.size {
            progress(1)
            return ViewableAsset(url: url, isPreview: false)
        }
        var variant = AssetVariant.original
        if preferPreview, asset.preview {
            // A preview that is here is shown whatever the link to the Mac is doing.
            let preview = cachedPreviewURL(asset.assetId)
            if FileManager.default.fileExists(atPath: preview.path) {
                progress(1)
                return ViewableAsset(url: preview, isPreview: true)
            }
            if macs[macId]?.caps?.assetPreview == true { variant = .preview }
        }
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }

        let key = DownloadKey(assetId: asset.assetId, variant: variant)
        let waiter = UUID()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<ViewableAsset, Error>) in
                if downloads[key] == nil {
                    downloads[key] = Download(macId: macId, assetId: asset.assetId)
                    // The size of a preview is not known before its first chunk says it; the file's is.
                    if variant == .original { downloads[key]?.size = asset.size }
                    Task { await self.requestWindow(key, from: 0) }
                }
                downloads[key]?.waiters[waiter] = cont
                downloads[key]?.progress[waiter] = progress
            }
        } onCancel: {
            Task { await self.cancelDownloadWaiter(waiter) }
        }
    }

    /// Takes a waiter out of the download it waits in, wherever that is by now (a waiter of a preview moves to the file's
    /// download when the Mac answers with the file), and drops the download when nobody waits for it any more.
    func cancelDownloadWaiter(_ waiter: UUID) {
        for (key, var download) in downloads where download.waiters[waiter] != nil {
            download.waiters.removeValue(forKey: waiter)?.resume(throwing: BackendError.cancelled)
            download.progress[waiter] = nil
            downloads[key] = download
            if download.waiters.isEmpty {
                downloads[key] = nil
                cancelTimer(timerKey(key))
            }
            return
        }
    }

    func timerKey(_ key: DownloadKey) -> String { "asset.\(key.assetId).\(key.variant.rawValue)" }

    func requestWindow(_ key: DownloadKey, from index: UInt64) async {
        guard var download = downloads[key] else { return }
        download.windowStart = index
        downloads[key] = download
        // Only a preview is asked for by name: nothing asks for the file but the plain request every Mac knows.
        let variant = key.variant == .preview ? AssetVariant.preview.rawValue : nil
        await send(
            AssetGet(assetId: key.assetId, from: index, count: UInt64(Wire.maxAssetChunksPerGet), variant: variant),
            to: download.macId
        )
        after(timerKey(key), Self.windowTimeout) { engine in await engine.windowTimedOut(key) }
    }

    func windowTimedOut(_ key: DownloadKey) async {
        guard var download = downloads[key] else { return }
        if macs[download.macId]?.recoveringUpdate == true {
            after(timerKey(key), Self.windowTimeout) { engine in await engine.windowTimedOut(key) }
            return
        }
        if download.retried {
            failDownload(key, with: BackendError.timedOut)
            return
        }
        download.retried = true
        downloads[key] = download
        // Ask again from the first chunk we still miss.
        var next = download.windowStart
        while download.chunks[next] != nil { next += 1 }
        await requestWindow(key, from: next)
    }

    /// One chunk of a download. Its bytes come raw in `blob` when it arrived as a blob frame, and in `chunk.data` as
    /// base64 when it arrived as JSON.
    func onAssetChunk(_ macId: String, _ chunk: AssetChunk, blob: Data?) async {
        guard let key = downloadKey(of: chunk, from: macId), var download = downloads[key], download.macId == macId else { return }
        guard let bytes = blob ?? Bytes.b64uDecode(chunk.data), bytes.count <= Wire.assetChunkBytes, chunk.index < chunk.total,
              chunk.size <= UInt64(Wire.maxAssetBytes)
        else {
            failDownload(key, with: BackendError.invalid("Bad chunk."))
            return
        }
        download.total = chunk.total
        download.size = chunk.size
        if let sha = chunk.sha256 { download.sha256 = sha.lowercased() }
        download.chunks[chunk.index] = bytes
        downloads[key] = download

        let fraction = Double(download.chunks.count) / Double(max(1, chunk.total))
        for report in download.progress.values { report(min(0.99, fraction)) }

        if UInt64(download.chunks.count) == chunk.total {
            finishDownload(key)
            return
        }
        // The window is complete once all of its chunks are in: ask for the next one.
        let end = min(download.windowStart + UInt64(Wire.maxAssetChunksPerGet), chunk.total)
        if (download.windowStart..<end).allSatisfy({ download.chunks[$0] != nil }) {
            cancelTimer(timerKey(key))
            downloads[key]?.retried = false
            await requestWindow(key, from: end)
        }
    }

    /// The download a chunk belongs to. A chunk with no `variant` is the file's (every Mac that does not know of previews
    /// sends those), `preview` is the preview's, and `original` is the answer to a request for a preview that is the file
    /// instead (the Mac cannot make one, or the file is not what the request described; §5.4). Anything else is not
    /// something this phone asked for.
    private func downloadKey(of chunk: AssetChunk, from macId: String) -> DownloadKey? {
        switch chunk.variant {
        case nil: return DownloadKey(assetId: chunk.assetId, variant: .original)
        case AssetVariant.preview.rawValue: return DownloadKey(assetId: chunk.assetId, variant: .preview)
        case AssetVariant.original.rawValue: return adoptOriginal(of: chunk.assetId, from: macId)
        default: return nil
        }
    }

    /// The Mac answered the request for the preview of `assetId` with the file. What is on its way now is the file, so
    /// the download of the preview becomes the download of the file (§7.1): it joins the one of the file that is already
    /// under way, or takes its place. Its later windows ask for the file, and the file satisfies everyone waiting.
    /// Returns the download the chunk belongs to, `nil` when there is none (the viewer left in the meantime).
    private func adoptOriginal(of assetId: String, from macId: String) -> DownloadKey? {
        let previewKey = DownloadKey(assetId: assetId, variant: .preview)
        let originalKey = DownloadKey(assetId: assetId, variant: .original)
        guard let preview = downloads[previewKey], preview.macId == macId else {
            // The rest of a window whose first chunks already moved the download over.
            return downloads[originalKey]?.macId == macId ? originalKey : nil
        }
        guard preview.chunks.isEmpty else {
            // Chunks of a preview came first and the file follows: the file changed under the request. They do not make one
            // file together.
            failDownload(previewKey, with: BackendError.invalid("The file changed."))
            return nil
        }
        downloads[previewKey] = nil
        cancelTimer(timerKey(previewKey))
        if var original = downloads[originalKey], original.macId == macId {
            original.waiters.merge(preview.waiters) { current, _ in current }
            original.progress.merge(preview.progress) { current, _ in current }
            downloads[originalKey] = original
        } else {
            downloads[originalKey] = preview
            after(timerKey(originalKey), Self.windowTimeout) { engine in await engine.windowTimedOut(originalKey) }
        }
        return originalKey
    }

    func onAssetError(_ macId: String, _ error: AssetError) {
        // An error is about the asset, whichever of its variants was asked for.
        let reason = error.reason == "tooLarge" ? BackendError.tooLarge : BackendError.unavailable(error.reason)
        for (key, download) in downloads where key.assetId == error.assetId && download.macId == macId {
            failDownload(key, with: reason)
        }
    }

    func finishDownload(_ key: DownloadKey) {
        guard let download = downloads[key], let total = download.total else { return }
        var data = Data()
        for index in 0..<total {
            guard let part = download.chunks[index] else { return failDownload(key, with: BackendError.invalid("Missing chunk.")) }
            data.append(part)
        }
        guard UInt64(data.count) == download.size else { return failDownload(key, with: BackendError.invalid("Size mismatch.")) }
        if let expected = download.sha256 {
            let actual = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            guard actual == expected else { return failDownload(key, with: BackendError.invalid("Checksum mismatch.")) }
        }
        let isPreview = key.variant == .preview
        let url = isPreview ? cachedPreviewURL(key.assetId) : cachedAssetURL(key.assetId)
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: url, options: .atomic)
        } catch {
            return failDownload(key, with: BackendError.unavailable("Could not store the file."))
        }
        cancelTimer(timerKey(key))
        downloads[key] = nil
        for report in download.progress.values { report(1) }
        for waiter in download.waiters.values { waiter.resume(returning: ViewableAsset(url: url, isPreview: isPreview)) }
        if !isPreview { satisfyPreviewWaiters(withFileOf: key.assetId, at: url, from: download.macId) }
    }

    /// The file is here, so a viewer still waiting for the preview of it has no use for one: it gets the file, and what
    /// the Mac may still send of the preview is not wanted (§7.1).
    private func satisfyPreviewWaiters(withFileOf assetId: String, at url: URL, from macId: String) {
        let previewKey = DownloadKey(assetId: assetId, variant: .preview)
        guard let preview = downloads[previewKey], preview.macId == macId else { return }
        downloads[previewKey] = nil
        cancelTimer(timerKey(previewKey))
        for report in preview.progress.values { report(1) }
        for waiter in preview.waiters.values { waiter.resume(returning: ViewableAsset(url: url, isPreview: false)) }
    }

    func failDownload(_ key: DownloadKey, with error: Error) {
        guard let download = downloads.removeValue(forKey: key) else { return }
        cancelTimer(timerKey(key))
        for waiter in download.waiters.values { waiter.resume(throwing: error) }
    }

    /// Fails the transfers of one Mac, or all of them when `macId` is `nil`.
    func failTransfers(for macId: String? = nil, with error: BackendError) {
        for (key, download) in downloads where macId == nil || download.macId == macId {
            failDownload(key, with: error)
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
        return try await performUpload(macId: macId, scope: UploadScope(requestId: requestId), source: source, progress: progress)
    }

    /// Pushes one file to a Mac for a question, an Agent or a new task, bound to that scope.
    func performUpload(
        macId: String, scope: UploadScope, source: UploadSource, progress: @escaping @Sendable (Double) -> Void
    ) async throws -> String {
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        guard !source.data.isEmpty, source.data.count <= Wire.maxAssetBytes else { throw BackendError.tooLarge }

        let uploadId = Bytes.newId()
        var completed = false
        uploadOwners[uploadId] = macId
        uploadMailboxes[uploadId] = UploadMailbox()
        defer {
            uploadMailboxes[uploadId] = nil
            uploadOwners[uploadId] = nil
            uploadBegins[uploadId] = nil
            if !completed, scope.taskId != nil { Task { await self.send(UploadCancel(uploadId: uploadId), to: macId) } }
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
        let begin = UploadBegin(
                uploadId: uploadId, name: source.name, mime: source.mime, size: size, chunks: UInt64(chunks), sha256: digest,
                scope: scope
            )
        uploadBegins[uploadId] = begin
        await send(begin, to: macId)
        let begun = try await nextUploadAck(uploadId)
        guard begun.status != "error" else { throw fail(begun) }

        var next = Int(begun.nextIndex)
        var sinceAck = 0
        while next < chunks {
            try Task.checkCancellation()
            let start = next * chunkSize
            let end = min(start + chunkSize, source.data.count)
            let part = source.data.subdata(in: start..<end)
            let sent = await sendUploadChunk(uploadId, index: UInt64(next), bytes: part, to: macId)
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
        completed = true
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

    func resumeTransfers(_ macId: String) async {
        for (id, begin) in uploadBegins where uploadOwners[id] == macId { await send(begin, to: macId) }
        for (id, download) in downloads where download.macId == macId { await requestWindow(id, from: download.windowStart) }
    }

    func uploadAckTimedOut(_ uploadId: String) {
        if let macId = uploadOwners[uploadId], macs[macId]?.recoveringUpdate == true {
            after("upload." + uploadId, Self.uploadAckTimeout) { engine in await engine.uploadAckTimedOut(uploadId) }
            return
        }
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
