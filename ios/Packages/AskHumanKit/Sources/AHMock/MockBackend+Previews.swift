import Foundation
import AHProtocol
import AHCore

// What a Mac that makes previews sends to an image viewer (docs/specs/ios-attachment-traffic.md §7.1), and what a UI test can
// do to watch the file come after it.

extension MockBackend {
    /// As the engine does it: the preview of a picture that offers one, from a Mac that declares them, while the file is not
    /// on the phone; the file in every other case. The bytes of a preview are those of `MockData.previewContents`.
    public func downloadForViewing(
        macId: String, asset: AssetRef, progress: @escaping @Sendable (Double) -> Void
    ) async throws -> ViewableAsset {
        let makesPreviews = state.macs.first { $0.id == macId }?.caps.assetPreview == true
        guard asset.preview, makesPreviews, !originalsOnPhone.contains(asset.assetId) else {
            return ViewableAsset(url: try await download(macId: macId, asset: asset, progress: progress), isPreview: false)
        }
        for step in 1...2 {
            await pause(0.1)
            progress(Double(step) / 2)
        }
        let url = try mockFile(named: asset.assetId + ".preview", data: MockData.previewContents(of: asset))
        return ViewableAsset(url: url, isPreview: true)
    }

    /// Holds the download of the file of every picture that has a preview back until the file at `path` exists.
    public func setOriginalGate(path: String) {
        originalGate = path
    }

    /// Waits for the gate a UI test may have set: it decides when the file is ready, instead of racing a timer against a test
    /// runner that can be seconds late on a busy machine.
    func waitForOriginalGate() async {
        guard let gate = originalGate else { return }
        let giveUp = Date().addingTimeInterval(60)
        while !FileManager.default.fileExists(atPath: gate), Date() < giveUp {
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
    }
}
