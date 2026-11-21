import SwiftUI
import AHCore
import AHProtocol

/// The pictures that came with a request, in one sheet: swipe sideways for the next one, pinch to zoom
/// each (`AssetPage`). It opens on the one that was tapped.
///
/// A picture the Mac sends a preview of is shown as that; the bar then offers "Original", and sharing it fetches the
/// original first (docs/specs/ios-attachment-traffic.md §7.2).
struct ImageGalleryViewer: View {
    let macId: String
    let images: [AssetRef]
    /// The `assetId` of the picture the sheet opens on.
    let start: String

    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(AppStore.self) private var store
    /// The `assetId` of the page the scroll view is at.
    @State private var current: String?
    /// What each page shows, and what the bar offers for the one on show.
    @State private var viewings = AssetViewings()
    /// The `assetId` of the picture that is being fetched to be shared: the share sheet opens when it has come.
    @State private var sharing: String?

    init(macId: String, images: [AssetRef], start: String) {
        self.macId = macId
        self.images = images
        self.start = start
        _current = State(initialValue: start)
    }

    private var position: Int { images.firstIndex { $0.assetId == current } ?? 0 }

    private func viewing(_ asset: AssetRef) -> AssetViewing { viewings.viewing(macId: macId, asset: asset) }

    /// The picture on show.
    private var shown: AssetViewing? { images.indices.contains(position) ? viewing(images[position]) : nil }

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
            // A picture rests clear of what lies over the sheet, so that its top is not behind the bar (or the status
            // bar and the island, where the sheet runs up to them): a tall picture would otherwise reach them. The page
            // is still the whole sheet, so a zoomed picture can be dragged out under the bar instead of being cut off at
            // its lower edge: the insets are handed to the picture (`AssetPage.clear`), which keeps them as the content
            // inset of its scroll view. The insets are read from the outer reader, which stays inside the safe area; the
            // inner one ignores it (and so is told there is none), and it is the one that sizes the pages. Only the top
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
                                    AssetPage(viewing: viewing(asset), near: abs(index - position) <= 1, clear: clear)
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
            .modifier(BarOverPicture())
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    CloseToolbarButton { dismiss() }
                        .accessibilityIdentifier("gallery-close")
                }
                ToolbarItem(placement: .principal) { heading }
                if let shown, shown.isReady {
                    ToolbarItemGroup(placement: .primaryAction) {
                        if shown.isPreview { originalButton(shown) }
                        shareButton(shown)
                    }
                }
            }
            .task(id: sharing) { await shareWhenFetched() }
            .alert(
                "Could not download the original",
                isPresented: Binding(get: { shown?.problem != nil }, set: { if !$0 { shown?.problem = nil } }),
                presenting: shown?.problem
            ) { problem in
                Button("Try again") { retry(problem) }
                Button("Cancel", role: .cancel) {}
            } message: { problem in
                Text(problem.message)
            }
            .onDisappear { viewings.cancelAll() }
        }
    }

    /// "Original · 7.1 MB", or how far its download is. At the accessibility text sizes the bar has no room for the heading
    /// next to the whole label (the heading would be dropped), so the size is left to VoiceOver there.
    @ViewBuilder private func originalButton(_ viewing: AssetViewing) -> some View {
        switch viewing.original {
        case .idle:
            let label = String(localized: "Original · \(Int(viewing.asset.size).formatted(.file))")
            Button {
                viewing.fetchOriginal(store: store)
            } label: {
                if dynamicTypeSize.isAccessibilitySize { Text("Original") } else { Text(label) }
            }
            .accessibilityLabel(label)
            .accessibilityIdentifier("gallery-original")
        case .loading(let fraction):
            ProgressRing(fraction: fraction)
                .accessibilityElement()
                .accessibilityLabel(Text("Downloading the original"))
                .accessibilityValue(Text(fraction, format: .percent.precision(.fractionLength(0))))
                .accessibilityIdentifier("gallery-original-progress")
        }
    }

    /// What is shared is the file itself, never the preview: when the phone has only the preview, the share sheet opens once
    /// the file has come.
    @ViewBuilder private func shareButton(_ viewing: AssetViewing) -> some View {
        if let file = viewing.file {
            ShareLink(item: file)
                .accessibilityIdentifier("gallery-share")
        } else {
            Button {
                sharing = viewing.asset.assetId
            } label: {
                Label("Share", systemImage: "square.and.arrow.up")
            }
            .disabled(sharing != nil || viewing.isFetchingOriginal)
            .accessibilityIdentifier("gallery-share")
        }
    }

    /// Fetches the picture that was asked to be shared and opens the share sheet, if that is still the picture on show.
    private func shareWhenFetched() async {
        guard let id = sharing, let asset = images.first(where: { $0.assetId == id }) else { return }
        let viewing = viewing(asset)
        await viewing.fetchOriginalAndWait(store: store)
        if !Task.isCancelled, current == id, let file = viewing.file { SharePanel.present(file) }
        if sharing == id { sharing = nil }
    }

    private func retry(_ problem: AssetViewing.Problem) {
        guard let shown else { return }
        if problem.forSharing {
            sharing = shown.asset.assetId
        } else {
            shown.fetchOriginal(store: store)
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

/// How far a download is, in the size of a bar button.
private struct ProgressRing: View {
    let fraction: Double

    var body: some View {
        ZStack {
            Circle().stroke(.tertiary, lineWidth: 2.5)
            Circle()
                .trim(from: 0, to: max(0.03, min(1, fraction)))
                .stroke(.tint, style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .frame(width: 20, height: 20)
        .padding(.horizontal, 8)
        .animation(.linear(duration: 0.15), value: fraction)
    }
}

/// The top bar over a picture that has been dragged under it. From iOS 26 the system softens the picture where it runs
/// under the bar (the scroll edge effect) and puts glass buttons on top. Before, the bar is clear over whatever lies
/// behind it, which would print the name and the buttons straight over the picture: it gets the bar's material.
private struct BarOverPicture: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
        } else {
            content.toolbarBackground(.visible, for: .navigationBar)
        }
    }
}
