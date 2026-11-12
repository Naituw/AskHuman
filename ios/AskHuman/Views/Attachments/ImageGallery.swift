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
            //
            // A page is as big as the sheet, taken from outside the scroll view. It is not sized by its
            // container (`containerRelativeFrame`): a keyboard that comes up over the sheet from another
            // window (the quick-add page) keeps resizing that container, the paging never settles, and the
            // app stops answering.
            //
            // The picture itself is laid out clear of what lies over the sheet, so that its top is not behind the bar
            // (or the status bar and the island, where the sheet runs up to them): a tall picture would otherwise
            // reach them. The insets are read from the outer reader, which stays inside the safe area; the inner
            // one ignores it (and so is told there is none), and it is the one that sizes the pages. Only the top
            // and the sides are read: the bottom is where a keyboard from another window shows up, and a page must
            // not follow it, so the outer reader ignores the bottom altogether (nothing in the size of a page
            // depends on the keyboard).
            GeometryReader { edges in
                let clear = EdgeInsets(
                    top: edges.safeAreaInsets.top, leading: edges.safeAreaInsets.leading,
                    bottom: 0, trailing: edges.safeAreaInsets.trailing
                )
                GeometryReader { viewport in
                    ScrollViewReader { proxy in
                        ScrollView(.horizontal) {
                            HStack(spacing: 0) {
                                ForEach(Array(images.enumerated()), id: \.element.assetId) { index, asset in
                                    AssetPage(macId: macId, asset: asset, near: abs(index - position) <= 1) { arrived[asset.assetId] = $0 }
                                        .padding(clear)
                                        .frame(width: viewport.size.width, height: viewport.size.height)
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
                }
                // The whole sheet, under the bars and under a keyboard alike, so nothing over it moves a page.
                .ignoresSafeArea()
                .accessibilityIdentifier("image-gallery")
            }
            .ignoresSafeArea(edges: .bottom)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    CloseToolbarButton { dismiss() }
                        .accessibilityIdentifier("gallery-close")
                }
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
