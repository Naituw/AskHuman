import Foundation
import AVFoundation
import UniformTypeIdentifiers
import AHProtocol

public enum VideoAttachmentError: Error, Sendable, Equatable {
    case unreadable
    case cannotFit
    case compressionFailed
}

/// Prepares a complete video for the existing bounded file-attachment transport.
@MainActor public enum VideoAttachmentPreparer {
    public enum Stage: Sendable, Equatable {
        case inspecting
        case compressing(Double)
    }

    public struct Output: Sendable, Equatable {
        public let name: String
        public let mime: String
        public let data: Data
        public let compressed: Bool
    }

    struct Estimate: Equatable {
        let preset: String
        let bytes: Int64
    }

    // Estimates precede encoding. Require 20% headroom even for the lowest-quality preset;
    // never fall back to an estimate near the transport limit or encode without an estimate.
    nonisolated static func choose(_ estimates: [Estimate], limit: Int) throws -> Estimate {
        let known = estimates.filter { $0.bytes > 0 }
        guard !known.isEmpty else { throw VideoAttachmentError.unreadable }
        guard let chosen = known.first(where: { $0.bytes <= Int64(Double(limit) * 0.8) }) else {
            throw VideoAttachmentError.cannotFit
        }
        return chosen
    }

    public static func prepare(
        url: URL, name: String? = nil, limit: Int = Wire.maxAssetBytes,
        progress: @escaping @MainActor @Sendable (Stage) -> Void = { _ in }
    ) async throws -> Output {
        try Task.checkCancellation()
        let originalName = name ?? url.lastPathComponent
        let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard size > 0, limit > 0, limit < Int.max else { throw VideoAttachmentError.unreadable }
        if size <= limit {
            return Output(name: originalName,
                          mime: UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "video/quicktime",
                          data: try await read(url, limit: limit), compressed: false)
        }

        progress(.inspecting)
        let asset = AVURLAsset(url: url)
        let duration = try await asset.load(.duration)
        guard duration.seconds.isFinite, duration.seconds > 0,
              !(try await asset.loadTracks(withMediaType: .video)).isEmpty else {
            throw VideoAttachmentError.unreadable
        }
        let presets = [AVAssetExportPreset1920x1080, AVAssetExportPreset1280x720,
                       AVAssetExportPreset640x480, AVAssetExportPresetLowQuality]
        var sessions: [String: AVAssetExportSession] = [:]
        var estimates: [Estimate] = []
        for preset in presets {
            try Task.checkCancellation()
            guard await AVAssetExportSession.compatibility(ofExportPreset: preset, with: asset, outputFileType: .mp4),
                  let session = AVAssetExportSession(asset: asset, presetName: preset) else { continue }
            session.outputFileType = .mp4
            session.timeRange = CMTimeRange(start: .zero, duration: duration)
            let bytes: Int64? = try? await withCheckedThrowingContinuation { continuation in
                session.estimateOutputFileLength { bytes, error in
                    if let error { continuation.resume(throwing: error) }
                    else { continuation.resume(returning: bytes) }
                }
            }
            if let bytes, bytes > 0 {
                sessions[preset] = session
                estimates.append(Estimate(preset: preset, bytes: bytes))
            }
        }
        let chosen = try choose(estimates, limit: limit)
        guard let session = sessions[chosen.preset] else { throw VideoAttachmentError.unreadable }
        try Task.checkCancellation()

        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ah-video-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let output = directory.appendingPathComponent("video.mp4")
        session.outputURL = output
        session.fileLengthLimit = Int64(limit)
        session.shouldOptimizeForNetworkUse = true
        let export = VideoExport(session)
        let monitor = Task {
            while !Task.isCancelled {
                progress(.compressing(Double(session.progress)))
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
        defer { monitor.cancel() }
        try await export.run()
        try Task.checkCancellation()
        let data = try await read(output, limit: limit)
        // A size limit must never silently turn the selected video into a shorter clip.
        let exportedDuration = try await AVURLAsset(url: output).load(.duration)
        guard exportedDuration.seconds.isFinite,
              abs(exportedDuration.seconds - duration.seconds) < 0.25 else {
            throw VideoAttachmentError.compressionFailed
        }
        progress(.compressing(1))
        let base = (originalName as NSString).deletingPathExtension
        return Output(name: (base.isEmpty ? "Video" : base) + ".mp4", mime: "video/mp4", data: data, compressed: true)
    }

    private nonisolated static func read(_ url: URL, limit: Int) async throws -> Data {
        let file = try FileHandle(forReadingFrom: url)
        defer { try? file.close() }
        var data = Data()
        while data.count <= limit {
            try Task.checkCancellation()
            let chunk = try file.read(upToCount: min(1024 * 1024, limit + 1 - data.count)) ?? Data()
            if chunk.isEmpty { break }
            data.append(chunk)
        }
        guard !data.isEmpty else { throw VideoAttachmentError.unreadable }
        guard data.count <= limit else { throw VideoAttachmentError.cannotFit }
        return data
    }
}

/// AVAssetExportSession is not Sendable. Confine all of its state and cancellation to one actor.
@MainActor private final class VideoExport {
    private let session: AVAssetExportSession
    init(_ session: AVAssetExportSession) { self.session = session }

    func run() async throws {
        try await withTaskCancellationHandler {
            try Task.checkCancellation()
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                session.exportAsynchronously {
                    Task { @MainActor in
                        switch self.session.status {
                        case .completed: continuation.resume()
                        case .cancelled: continuation.resume(throwing: CancellationError())
                        default: continuation.resume(throwing: VideoAttachmentError.compressionFailed)
                        }
                    }
                }
            }
        } onCancel: {
            Task { @MainActor in self.session.cancelExport() }
        }
    }
}
