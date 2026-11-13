import SwiftUI
import UIKit

/// A way to bring a view back into sight on the page it is in, for when something inside it gets shorter and the page, left
/// where it was, would end up showing something else.
@MainActor
final class PageAnchor {
    fileprivate weak var view: UIView?

    /// When the top of the view is above what the page shows, scrolls the page until that top is back in sight, with `margin`
    /// points of what is above it. Meant for just before the view gets shorter, so that the page does not first stop at its
    /// new end and then jump.
    func bringTopIntoSight(margin: CGFloat) {
        guard let view, view.window != nil else { return }
        var ancestor = view.superview
        while let parent = ancestor, !(parent is UIScrollView) { ancestor = parent.superview }
        guard let scroll = ancestor as? UIScrollView else { return }
        let inset = scroll.adjustedContentInset.top
        let top = view.convert(CGPoint.zero, to: scroll).y - margin
        guard top < scroll.contentOffset.y + inset else { return }
        scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: max(-inset, top - inset)), animated: true)
    }
}

/// Marks the view it is in the `.background` of as the one a `PageAnchor` brings back into sight.
struct PageAnchorMark: UIViewRepresentable {
    let anchor: PageAnchor

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.isUserInteractionEnabled = false
        view.isAccessibilityElement = false
        anchor.view = view
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        anchor.view = view
    }
}
