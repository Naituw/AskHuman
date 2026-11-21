import SwiftUI
import UIKit
import AHCore
import AHProtocol

/// What one attachment shows in a viewer and how it got there: the page draws it, and the bar over a gallery offers
/// "Original" and sharing from it, so both read the same object.
///
/// A picture a Mac makes a preview of arrives as that (docs/specs/ios-attachment-traffic.md §7.1, §7.2). The file follows
/// when the user asks for it, in the background of the page, which then swaps the picture in place.
@Observable @MainActor
final class AssetViewing {
    let macId: String
    let asset: AssetRef

    enum Phase {
        case loading(Double)
        case ready(Shown)
        case failed(String)
    }

    /// What is on show.
    struct Shown: Equatable {
        /// Where it is on the phone. A preview is read from the cache as it is; a file is a copy under its real name and
        /// extension, which previews and sharing want.
        var url: URL
        var isPreview: Bool
    }

    /// The download of the file of a picture whose preview is on show.
    enum Original: Equatable {
        case idle
        case loading(Double)
    }

    private(set) var phase: Phase = .loading(0)
    private(set) var original: Original = .idle
    /// The file under its real name, once it is on the phone (what is shared).
    private(set) var file: URL?
    /// Why the file of a picture could not come, for the alert the viewer shows; `forSharing` says what it was for.
    var problem: Problem?

    struct Problem: Equatable {
        var message: String
        var forSharing: Bool
    }

    @ObservationIgnored private var originalTask: Task<Void, Never>?
    /// Whether the download of the file under way was asked for by sharing (it may have been started by the button).
    @ObservationIgnored private var forSharing = false

    init(macId: String, asset: AssetRef) {
        self.macId = macId
        self.asset = asset
    }

    var isPreview: Bool {
        if case .ready(let shown) = phase { return shown.isPreview }
        return false
    }

    /// The page has something to show, so there is something to share.
    var isReady: Bool {
        if case .ready = phase { return true }
        return false
    }

    var isFetchingOriginal: Bool { original != .idle }

    // MARK: the page

    /// The page's download. A preview that the phone cannot open is replaced by the file without a word (AT18): the
    /// user asked for a picture, not for a preview.
    func load(store: AppStore) async {
        if case .ready = phase { return }
        phase = .loading(0)
        let report: @Sendable (Double) -> Void = { [weak self] fraction in
            Task { @MainActor in self?.progressed(fraction) }
        }
        do {
            var viewable = try await store.downloadForViewing(macId: macId, asset: asset, progress: report)
            if viewable.isPreview, !Self.opens(viewable.url) {
                viewable = ViewableAsset(url: try await store.download(macId: macId, asset: asset, progress: report), isPreview: false)
            }
            try show(viewable)
        } catch is CancellationError {
            return
        } catch BackendError.cancelled {
            // The engine reports a cancelled download this way (a page that is no longer near).
            return
        } catch {
            phase = .failed(error.userMessage)
        }
    }

    private func progressed(_ fraction: Double) {
        if case .loading = phase { phase = .loading(fraction) }
    }

    private func show(_ viewable: ViewableAsset) throws {
        if viewable.isPreview {
            phase = .ready(Shown(url: viewable.url, isPreview: true))
        } else {
            let named = try Self.named(viewable.url, as: asset)
            file = named
            phase = .ready(Shown(url: named, isPreview: false))
        }
    }

    // MARK: the file

    /// Starts the download of the file of the picture on show, unless it is on its way or here. Nothing happens to the page
    /// until the file has come: then the picture is swapped in place.
    func fetchOriginal(store: AppStore, forSharing sharing: Bool = false) {
        if sharing { forSharing = true }
        guard originalTask == nil, isPreview else { return }
        problem = nil
        original = .loading(0)
        originalTask = Task { [weak self, macId, asset] in
            do {
                let cached = try await store.download(macId: macId, asset: asset) { fraction in
                    Task { @MainActor in self?.originalProgressed(fraction) }
                }
                try self?.arrived(cached)
            } catch is CancellationError {
                self?.originalEnded(nil)
            } catch BackendError.cancelled {
                self?.originalEnded(nil)
            } catch {
                self?.originalEnded(error.userMessage)
            }
        }
    }

    /// Waits for the download of the file, starting it when there is none; whether it came is `file`.
    func fetchOriginalAndWait(store: AppStore) async {
        if file != nil { return }
        fetchOriginal(store: store, forSharing: true)
        await originalTask?.value
    }

    /// The viewer is going away: what is under way is not wanted any more.
    func cancelOriginal() {
        originalTask?.cancel()
    }

    private func originalProgressed(_ fraction: Double) {
        if case .loading = original { original = .loading(fraction) }
    }

    private func arrived(_ cached: URL) throws {
        let named = try Self.named(cached, as: asset)
        file = named
        phase = .ready(Shown(url: named, isPreview: false))
        originalEnded(nil)
    }

    private func originalEnded(_ failure: String?) {
        if let failure { problem = Problem(message: failure, forSharing: forSharing) }
        original = .idle
        forSharing = false
        originalTask = nil
    }

    // MARK: files

    /// Whether the phone can open the picture at `url`.
    private static func opens(_ url: URL) -> Bool {
        UIImage(contentsOfFile: url.path) != nil
    }

    /// The cache stores files by id; previews and sharing want the real name and extension.
    private static func named(_ cached: URL, as asset: AssetRef) throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("ah-preview", isDirectory: true)
            .appendingPathComponent(asset.assetId, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let name = asset.name.replacingOccurrences(of: "/", with: "_")
        let target = directory.appendingPathComponent(name.isEmpty ? "file" : name)
        if !FileManager.default.fileExists(atPath: target.path) {
            try FileManager.default.copyItem(at: cached, to: target)
        }
        return target
    }
}

/// The `AssetViewing` of each attachment of a viewer, made when it is first asked for.
@MainActor
final class AssetViewings {
    private var all: [String: AssetViewing] = [:]

    func viewing(macId: String, asset: AssetRef) -> AssetViewing {
        if let found = all[asset.assetId] { return found }
        let made = AssetViewing(macId: macId, asset: asset)
        all[asset.assetId] = made
        return made
    }

    /// The viewer is going away: the files still on their way are not wanted any more.
    func cancelAll() {
        for viewing in all.values { viewing.cancelOriginal() }
    }
}
