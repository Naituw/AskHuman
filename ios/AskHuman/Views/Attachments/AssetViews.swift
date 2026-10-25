import SwiftUI
import UIKit
import AHCore
import AHProtocol

/// The files that came with a request: image thumbnails as a grid, other files as rows.
struct AssetList: View {
    let macId: String
    let assets: [AssetRef]
    @State private var shown: AssetRef?

    var body: some View {
        let images = assets.filter { $0.kind == .image }
        let files = assets.filter { $0.kind != .image }
        VStack(alignment: .leading, spacing: 8) {
            if !images.isEmpty {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 96, maximum: 140), spacing: 8)], alignment: .leading, spacing: 8) {
                    ForEach(images, id: \.assetId) { asset in
                        Button { shown = asset } label: { ImageThumbnail(asset: asset) }
                            .buttonStyle(.plain)
                            .accessibilityLabel(asset.name)
                    }
                }
            }
            ForEach(files, id: \.assetId) { asset in
                Button { shown = asset } label: { FileRow(asset: asset) }
                    .buttonStyle(.plain)
            }
        }
        .sheet(item: $shown) { asset in
            AssetViewer(macId: macId, asset: asset)
        }
    }
}

extension AssetRef: @retroactive Identifiable {
    public var id: String { assetId }
}

private struct ImageThumbnail: View {
    let asset: AssetRef

    var body: some View {
        ZStack {
            if let data = asset.thumbnailData, let image = UIImage(data: data) {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else {
                Rectangle().fill(Color(.tertiarySystemFill))
                Image(systemName: "photo").foregroundStyle(.secondary)
            }
        }
        .frame(height: 96)
        .frame(maxWidth: .infinity)
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
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

/// Downloads an attachment from the Mac when opened and previews it by type.
struct AssetViewer: View {
    let macId: String
    let asset: AssetRef

    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var phase: Phase = .loading(0)
    @State private var attempt = 0

    private enum Phase {
        case loading(Double)
        case ready(URL)
        case failed(String)
    }

    var body: some View {
        NavigationStack {
            Group {
                switch phase {
                case .loading(let fraction):
                    VStack(spacing: 14) {
                        if let data = asset.thumbnailData, let image = UIImage(data: data) {
                            Image(uiImage: image).resizable().scaledToFit().frame(maxHeight: 160).clipShape(RoundedRectangle(cornerRadius: 12))
                        }
                        ProgressView(value: fraction).frame(maxWidth: 220)
                        Text("Downloading from your Mac…").font(.footnote).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                case .ready(let url):
                    AssetContent(url: url, asset: asset)
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
            .navigationTitle(asset.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                if case .ready(let url) = phase {
                    ToolbarItem(placement: .primaryAction) { ShareLink(item: url) }
                }
            }
        }
        .task(id: attempt) { await load() }
    }

    private func load() async {
        phase = .loading(0)
        do {
            let cached = try await store.download(macId: macId, asset: asset) { fraction in
                Task { @MainActor in
                    if case .loading = phase { phase = .loading(fraction) }
                }
            }
            phase = .ready(try Self.named(cached, as: asset))
        } catch is CancellationError {
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
            if let image = UIImage(contentsOfFile: url.path) {
                ZoomableImage(image: image).ignoresSafeArea(edges: .bottom)
            } else {
                ContentUnavailableView("Not a readable image", systemImage: "photo.badge.exclamationmark")
            }
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
