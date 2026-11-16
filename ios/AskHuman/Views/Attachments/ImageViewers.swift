import SwiftUI
import UIKit
import QuickLook

/// Pinch to zoom, drag to pan, double-tap to toggle.
///
/// The view may run under what lies over it (the bar of the viewer's sheet). `clear` is how far the picture keeps
/// clear of those edges when it is not zoomed: it is fitted into the view less `clear`, and it is a content inset of
/// the scroll view, so a zoomed picture can be dragged out under the bar (as in the system's Photos app) instead of
/// being cut off at the bar's lower edge.
struct ZoomableImage: UIViewRepresentable {
    let image: UIImage
    var clear: NSDirectionalEdgeInsets = .zero
    /// Read by VoiceOver for the picture, and the name UI tests find it by (`accessibilityIdentifier`).
    var label: String?
    var identifier: String?
    var dragProvider: (() -> NSItemProvider)?

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> PictureScrollView {
        let scroll = PictureScrollView()
        scroll.delegate = context.coordinator
        scroll.minimumZoomScale = 1
        scroll.maximumZoomScale = 6
        scroll.showsVerticalScrollIndicator = false
        scroll.showsHorizontalScrollIndicator = false
        scroll.bouncesZoom = true
        // The insets are the viewer's own (`clear`); the system must not add the safe area on top of them.
        scroll.contentInsetAdjustmentBehavior = .never
        scroll.clear = clear
        let imageView = scroll.imageView
        imageView.image = image
        imageView.isUserInteractionEnabled = true
        let drag = UIDragInteraction(delegate: context.coordinator)
        drag.isEnabled = true
        imageView.addInteraction(drag)
        context.coordinator.dragProvider = dragProvider
        imageView.contentMode = .scaleAspectFit
        imageView.isAccessibilityElement = true
        imageView.accessibilityTraits = .image
        imageView.accessibilityLabel = label
        imageView.accessibilityIdentifier = identifier
        scroll.addSubview(imageView)
        context.coordinator.imageView = imageView
        context.coordinator.scroll = scroll
        let doubleTap = UITapGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.doubleTapped(_:)))
        doubleTap.numberOfTapsRequired = 2
        scroll.addGestureRecognizer(doubleTap)
        return scroll
    }

    func updateUIView(_ scroll: PictureScrollView, context: Context) {
        context.coordinator.dragProvider = dragProvider
        scroll.clear = clear
        // Leave the image view alone unless the image itself changed.
        if let imageView = context.coordinator.imageView, imageView.image !== image {
            imageView.image = image
        }
    }

    /// A scroll view that keeps one picture, as big as the view less `clear` when it is not zoomed, where `clear` is
    /// the content inset: the picture rests inside the clear area and can be moved out of it only by zooming.
    final class PictureScrollView: UIScrollView {
        let imageView = UIImageView()
        var clear = NSDirectionalEdgeInsets.zero {
            didSet { if clear != oldValue { setNeedsLayout() } }
        }
        /// What the picture was last placed for: a change in either places it again (and drops the zoom).
        private var placedFor: (size: CGSize, clear: NSDirectionalEdgeInsets)?

        override func layoutSubviews() {
            super.layoutSubviews()
            place()
        }

        private func place() {
            guard bounds.width > 0, bounds.height > 0 else { return }
            if let placedFor, placedFor.clear == clear,
               abs(placedFor.size.width - bounds.width) < 0.5, abs(placedFor.size.height - bounds.height) < 0.5 {
                return
            }
            placedFor = (bounds.size, clear)
            let rightToLeft = effectiveUserInterfaceLayoutDirection == .rightToLeft
            let inset = UIEdgeInsets(
                top: clear.top, left: rightToLeft ? clear.trailing : clear.leading,
                bottom: clear.bottom, right: rightToLeft ? clear.leading : clear.trailing
            )
            let fitted = CGSize(
                width: max(1, bounds.width - inset.left - inset.right),
                height: max(1, bounds.height - inset.top - inset.bottom)
            )
            zoomScale = 1
            contentInset = inset
            imageView.frame = CGRect(origin: .zero, size: fitted)
            contentSize = fitted
            contentOffset = CGPoint(x: -inset.left, y: -inset.top)
        }
    }

    final class Coordinator: NSObject, UIScrollViewDelegate, UIDragInteractionDelegate {
        var dragProvider: (() -> NSItemProvider)?
        var imageView: UIImageView?
        weak var scroll: UIScrollView?

        func viewForZooming(in scrollView: UIScrollView) -> UIView? { imageView }

        func dragInteraction(_ interaction: UIDragInteraction, itemsForBeginning session: UIDragSession) -> [UIDragItem] {
            dragProvider.map { [UIDragItem(itemProvider: $0())] } ?? []
        }

        func dragInteraction(_ interaction: UIDragInteraction, sessionAllowsMoveOperation session: UIDragSession) -> Bool { false }

        @objc func doubleTapped(_ gesture: UITapGestureRecognizer) {
            guard let scroll else { return }
            if scroll.zoomScale > 1.01 {
                scroll.setZoomScale(1, animated: true)
            } else {
                let point = gesture.location(in: imageView)
                let size = CGSize(width: scroll.bounds.width / 2.5, height: scroll.bounds.height / 2.5)
                scroll.zoom(to: CGRect(x: point.x - size.width / 2, y: point.y - size.height / 2, width: size.width, height: size.height), animated: true)
            }
        }
    }
}

/// System preview for types the app does not render itself (PDF, Office documents, …).
struct QuickLookPreview: UIViewControllerRepresentable {
    let url: URL

    func makeCoordinator() -> Coordinator { Coordinator(url: url) }

    func makeUIViewController(context: Context) -> QLPreviewController {
        let controller = QLPreviewController()
        controller.dataSource = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: QLPreviewController, context: Context) {}

    final class Coordinator: NSObject, QLPreviewControllerDataSource {
        let url: URL
        init(url: URL) { self.url = url }
        func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }
        func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem { url as NSURL }
    }
}
