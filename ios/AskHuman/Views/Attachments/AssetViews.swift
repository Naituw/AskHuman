import SwiftUI
import UIKit
import AHCore
import AHProtocol

/// The files that came with a request: the pictures as a grid of squares, the other files as rows.
/// A picture opens in a viewer that pages through all the pictures; a file opens on its own.
struct AssetList: View {
    let macId: String
    let assets: [AssetRef]
    @State private var shown: Shown?

    /// What the sheet holds.
    private enum Shown: Identifiable {
        case pictures(from: String)
        case file(AssetRef)

        var id: String {
            switch self {
            case .pictures(let start): "pictures-\(start)"
            case .file(let asset): "file-\(asset.assetId)"
            }
        }
    }

    var body: some View {
        let images = assets.filter { $0.kind == .image }
        let files = assets.filter { $0.kind != .image }
        VStack(alignment: .leading, spacing: 8) {
            if !images.isEmpty {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 96, maximum: 140), spacing: 8)], alignment: .leading, spacing: 8) {
                    ForEach(images, id: \.assetId) { asset in
                        Button { shown = .pictures(from: asset.assetId) } label: { ImageThumbnail(asset: asset) }
                            .buttonStyle(.plain)
                            .accessibilityLabel(asset.name)
                            .accessibilityIdentifier("asset-picture-\(asset.assetId)")
                    }
                }
            }
            ForEach(files, id: \.assetId) { asset in
                Button { shown = .file(asset) } label: { FileRow(asset: asset) }
                    .buttonStyle(.plain)
            }
        }
        .sheet(item: $shown) { shown in
            switch shown {
            case .pictures(let start): ImageGalleryViewer(macId: macId, images: images, start: start)
            case .file(let asset): AssetViewer(macId: macId, asset: asset)
            }
        }
    }
}

extension AssetRef: @retroactive Identifiable {
    public var id: String { assetId }
}

/// A square cell with the picture cropped to fill it. The square alone sets the size: a picture that is
/// wide or tall only fills it and is clipped, so it can neither spill out of its cell nor cover the next.
private struct ImageThumbnail: View {
    let asset: AssetRef

    var body: some View {
        Color(.tertiarySystemFill)
            .aspectRatio(1, contentMode: .fit)
            .overlay {
                if let data = asset.thumbnailData, let image = UIImage(data: data) {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                } else {
                    Image(systemName: "photo").foregroundStyle(.secondary)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(alignment: .bottomLeading) {
                if asset.tooLarge {
                    Chip(text: "Too large", systemImage: "exclamationmark.triangle.fill", tint: .orange).padding(4)
                }
            }
    }
}

struct FileRow: View {
    let asset: AssetRef

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: asset.kind.symbol)
                .font(.title3)
                .foregroundStyle(.tint)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 1) {
                Text(asset.name).font(.subheadline).lineLimit(1).truncationMode(.middle)
                Text(asset.tooLarge ? String(localized: "Too large to download") : Int(asset.size).formatted(.file))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

extension Int {
    func formatted(_ style: ByteCountFormatStyle) -> String { style.format(Int64(self)) }
}

// MARK: viewer

/// One file of any kind in a sheet of its own.
struct AssetViewer: View {
    let macId: String
    let asset: AssetRef

    @Environment(\.dismiss) private var dismiss
    /// The file once it is on the phone, for sharing.
    @State private var ready: URL?

    var body: some View {
        NavigationStack {
            AssetPage(macId: macId, asset: asset) { ready = $0 }
                .navigationTitle(asset.name)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                    if let ready {
                        ToolbarItem(placement: .primaryAction) { ShareLink(item: ready) }
                    }
                }
        }
    }
}

/// What a viewer shows of one attachment: it is downloaded from the Mac once it is on screen, then
/// previewed by type. `onReady` says where the file is when it has arrived.
struct AssetPage: View {
    let macId: String
    let asset: AssetRef
    /// Whether the page is on show or beside the one that is. Only such a page is fetched and drawn; a
    /// page further off is a placeholder (and lets go of its picture), as a gallery may hold many.
    var near = true
    var onReady: (URL) -> Void = { _ in }

    @Environment(AppStore.self) private var store
    @State private var phase: Phase = .loading(0)
    @State private var attempt = 0

    private enum Phase {
        case loading(Double)
        case ready(URL)
        case failed(String)
    }

    /// What starts a download: the first time the page is near, and every "Try again".
    private struct Fetch: Equatable {
        var attempt: Int
        var near: Bool
    }

    var body: some View {
        Group {
            switch phase {
            case .loading(let fraction):
                waiting(fraction)
            case .ready(let url):
                if near { AssetContent(url: url, asset: asset) } else { waiting(nil) }
            case .failed(let message):
                ContentUnavailableView {
                    Label("Could not open the file", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(message)
                } actions: {
                    Button("Try again") { attempt += 1 }.buttonStyle(.borderedProminent)
                }
            }
        }
        .task(id: Fetch(attempt: attempt, near: near)) {
            guard near else { return }
            if case .ready = phase { return }
            await load()
        }
    }

    /// The thumbnail, and how far the download is (when it is going on).
    private func waiting(_ fraction: Double?) -> some View {
        VStack(spacing: 14) {
            if let data = asset.thumbnailData, let image = UIImage(data: data) {
                Image(uiImage: image).resizable().scaledToFit().frame(maxHeight: 160).clipShape(RoundedRectangle(cornerRadius: 12))
            }
            if let fraction {
                ProgressView(value: fraction).frame(maxWidth: 220)
                Text("Downloading from your Mac…").font(.footnote).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func load() async {
        phase = .loading(0)
        do {
            let cached = try await store.download(macId: macId, asset: asset) { fraction in
                Task { @MainActor in
                    if case .loading = phase { phase = .loading(fraction) }
                }
            }
            let url = try Self.named(cached, as: asset)
            phase = .ready(url)
            onReady(url)
        } catch is CancellationError {
            return
        } catch BackendError.cancelled {
            // The engine reports a cancelled download this way (a page that is no longer near).
            return
        } catch {
            phase = .failed(error.userMessage)
        }
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

private struct AssetContent: View {
    let url: URL
    let asset: AssetRef

    var body: some View {
        switch asset.kind {
        case .image:
            PictureContent(url: url)
        case .markdown:
            TextFile(url: url) { text in
                ScrollView { MarkdownView(text: text).padding() }
            }
        case .diff:
            TextFile(url: url) { text in DiffView(text: text) }
        case .text:
            TextFile(url: url) { text in
                ScrollView([.vertical, .horizontal]) {
                    Text(text)
                        .font(.system(.footnote, design: .monospaced))
                        .textSelection(.enabled)
                        .padding()
                }
            }
        case .other:
            QuickLookPreview(url: url).ignoresSafeArea(edges: .bottom)
        }
    }
}

/// A picture that has arrived. It is opened once and then kept, so a gallery that re-evaluates its pages
/// while it scrolls does not hand the image view a new (and so newly decoded) image each time.
private struct PictureContent: View {
    @State private var picture: UIImage?

    init(url: URL) {
        _picture = State(initialValue: UIImage(contentsOfFile: url.path))
    }

    var body: some View {
        if let picture {
            ZoomableImage(image: picture).ignoresSafeArea(edges: .bottom)
        } else {
            ContentUnavailableView("Not a readable image", systemImage: "photo.badge.exclamationmark")
        }
    }
}

private let textPreviewLimit = 1_000_000

/// Reads the start of a text file (large logs are cut with a notice).
private struct TextFile<Content: View>: View {
    let url: URL
    @ViewBuilder let content: (String) -> Content
    @State private var loaded: (text: String, truncated: Bool)?

    var body: some View {
        Group {
            if let loaded {
                VStack(spacing: 0) {
                    if loaded.truncated {
                        Label("Showing the first 1 MB only.", systemImage: "scissors")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .padding(8)
                    }
                    content(loaded.text)
                }
            } else {
                ProgressView()
            }
        }
        .task {
            let url = url
            loaded = await Task.detached {
                guard let handle = try? FileHandle(forReadingFrom: url) else { return ("", false) }
                defer { try? handle.close() }
                let data = (try? handle.read(upToCount: textPreviewLimit + 1)) ?? Data()
                let truncated = data.count > textPreviewLimit
                return (String(decoding: data.prefix(textPreviewLimit), as: UTF8.self), truncated)
            }.value
        }
    }
}

struct DiffView: View {
    let text: String

    var body: some View {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        ScrollView([.vertical, .horizontal]) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    Text(line.isEmpty ? " " : line)
                        .font(.system(.footnote, design: .monospaced))
                        .textSelection(.enabled)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(background(for: line))
                }
            }
            .frame(minWidth: 320, alignment: .leading)
        }
    }

    private func background(for line: String) -> Color {
        if line.hasPrefix("+++") || line.hasPrefix("---") { return Color(.tertiarySystemFill) }
        if line.hasPrefix("@@") { return Color.blue.opacity(0.14) }
        if line.hasPrefix("+") { return Color.green.opacity(0.2) }
        if line.hasPrefix("-") { return Color.red.opacity(0.2) }
        return .clear
    }
}
