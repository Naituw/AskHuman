import SwiftUI
import AHProtocol

/// The pictures that came with a request, in one sheet: swipe sideways for the next one, pinch to zoom
/// each (`AssetPage`). It opens on the one that was tapped.
struct ImageGalleryViewer: View {
    let macId: String
    let images: [AssetRef]
    /// The `assetId` of the picture the sheet opens on.
    let start: String

    @Environment(\.dismiss) private var dismiss
    /// The `assetId` of the page the scroll view is at.
    @State private var current: String?
    /// Where the pictures that have arrived are, for sharing the one on show.
    @State private var arrived: [String: URL] = [:]

    init(macId: String, images: [AssetRef], start: String) {
        self.macId = macId
        self.images = images
        self.start = start
        _current = State(initialValue: start)
    }

    private var position: Int { images.firstIndex { $0.assetId == current } ?? 0 }

    /// The scroll view reports no page while it is between two; the heading keeps the last one then.
    private var page: Binding<String?> {
        Binding(get: { current }, set: { if let id = $0 { current = id } })
    }

    var body: some View {
        NavigationStack {
            // Every page is there from the start (a page that is not near the one on show is only a
            // placeholder, it takes nothing to hold), so a swipe always has a page to reveal and the
            // ones beside the one on show are already coming from the Mac when one swipes to them.
            ScrollViewReader { proxy in
                ScrollView(.horizontal) {
                    HStack(spacing: 0) {
                        ForEach(Array(images.enumerated()), id: \.element.assetId) { index, asset in
                            AssetPage(macId: macId, asset: asset, near: abs(index - position) <= 1) { arrived[asset.assetId] = $0 }
                                .containerRelativeFrame([.horizontal, .vertical])
                                .id(asset.assetId)
                        }
                    }
                    .scrollTargetLayout()
                }
                .scrollTargetBehavior(.paging)
                .scrollPosition(id: page)
                .scrollIndicators(.hidden)
                .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
                // The binding alone does not move a stack that is not lazy to its first value.
                .onAppear { proxy.scrollTo(start, anchor: .leading) }
            }
            .ignoresSafeArea(edges: .bottom)
            .accessibilityIdentifier("image-gallery")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                ToolbarItem(placement: .principal) { heading }
                if let url = images.indices.contains(position) ? arrived[images[position].assetId] : nil {
                    ToolbarItem(placement: .primaryAction) { ShareLink(item: url) }
                }
            }
        }
    }

    /// The name of the picture, and which one of how many it is.
    private var heading: some View {
        VStack(spacing: 0) {
            Text(images[position].name)
                .font(.headline)
                .lineLimit(1)
                .truncationMode(.middle)
                .accessibilityIdentifier("gallery-name")
            if images.count > 1 {
                Text("\(position + 1) / \(images.count)")
                    .font(.caption)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .accessibilityLabel(String(localized: "Image \(position + 1) of \(images.count)"))
                    .accessibilityIdentifier("gallery-position")
            }
        }
    }
}
