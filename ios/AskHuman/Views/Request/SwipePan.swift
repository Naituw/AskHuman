import SwiftUI
import UIKit
import UIKit.UIGestureRecognizerSubclass

// The finger that swipes a row sideways (docs/specs/todo-list-actions.md §5.3).
//
// It is a UIKit pan recognizer on the page's scroll view, not a SwiftUI `DragGesture` on the row. On iOS 27 a drag attached
// inside a scroll view takes the page's own scroll away from a finger that starts on it, although the drag does nothing with
// a vertical movement (a real iPhone could not scroll the answer page from a todo row). The recognizer here makes its
// decision in the first few points of the movement: a vertical start fails at once, and the page scrolls as if the row had
// no gesture at all; a sideways start begins, and the page's scroll is kept out of its way by `SwipeScrollLock`.

/// What a swiped row hears of the finger. `translation` is the finger's sideways distance since the touch began.
@MainActor
struct SwipePanEvents {
    let began: (_ translation: CGFloat) -> Void
    let changed: (_ translation: CGFloat) -> Void
    /// `velocity` is in points per second.
    let ended: (_ translation: CGFloat, _ velocity: CGFloat) -> Void
    /// The system took the touch, or the row was switched off while the finger was on it.
    let cancelled: () -> Void
}

/// Listens, for the row it is the background of, to the finger that swipes it sideways. It adds nothing to the screen and
/// takes no touch: its recognizer sits on the page's scroll view (the window, when the row is not in one) and only
/// answers to a touch that begins inside the row.
struct SwipePan: UIViewRepresentable {
    let isEnabled: Bool
    let events: SwipePanEvents
    /// Screen points from the left edge that belong to the back gesture.
    var backEdge: CGFloat = 20

    func makeUIView(context: Context) -> SwipePanAttachment { SwipePanAttachment() }

    func updateUIView(_ view: SwipePanAttachment, context: Context) {
        view.events = events
        view.backEdge = backEdge
        view.isSwipeEnabled = isEnabled
    }
}

/// Each attachment keeps its own recognizer on the scroll view it is in and takes it away when it leaves (SwiftUI may build
/// the background more than once, and one that goes must not unplug another that stays).
final class SwipePanAttachment: UIView {
    var events: SwipePanEvents?
    var backEdge: CGFloat = 20 {
        didSet { recognizer?.backEdge = backEdge }
    }
    var isSwipeEnabled = true {
        didSet { recognizer?.isEnabled = isSwipeEnabled }
    }

    private var recognizer: SwipePanRecognizer?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        isAccessibilityElement = false
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        attach()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        attach()
    }

    private func attach() {
        guard window != nil else {
            detach()
            return
        }
        var ancestor = superview
        while let parent = ancestor, !(parent is UIScrollView) { ancestor = parent.superview }
        guard let host = ancestor ?? window else { return }
        if let recognizer, recognizer.view === host { return }
        detach()
        let recognizer = SwipePanRecognizer(anchor: self)
        recognizer.backEdge = backEdge
        recognizer.isEnabled = isSwipeEnabled
        recognizer.addTarget(self, action: #selector(panned(_:)))
        host.addGestureRecognizer(recognizer)
        self.recognizer = recognizer
    }

    private func detach() {
        if let recognizer {
            recognizer.view?.removeGestureRecognizer(recognizer)
            self.recognizer = nil
        }
    }

    @objc private func panned(_ pan: SwipePanRecognizer) {
        let translation = pan.translation(in: nil).x
        switch pan.state {
        case .began: events?.began(translation)
        case .changed: events?.changed(translation)
        case .ended: events?.ended(translation, pan.velocity(in: nil).x)
        case .cancelled: events?.cancelled()
        default: break
        }
    }
}

/// A pan that is the row's only when the touch begins inside it, away from the screen's left edge, and its first movement is
/// clearly sideways. Anything else fails at once, so that it never takes part in what the touch means to the page.
final class SwipePanRecognizer: UIPanGestureRecognizer {
    weak var anchor: UIView?
    var backEdge: CGFloat = 20

    private var origin: CGPoint?
    private var decided = false
    /// How far the finger goes before it is told apart: sideways, or the page's.
    private static let decision: CGFloat = 8

    init(anchor: UIView) {
        self.anchor = anchor
        super.init(target: nil, action: nil)
        maximumNumberOfTouches = 1
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        origin = nil
        decided = false
        guard let touch = touches.first, let anchor, anchor.window != nil,
              anchor.bounds.contains(touch.location(in: anchor)) else {
            state = .failed
            return
        }
        let point = touch.location(in: nil)
        // The edge swipe that goes back owns the first points of the screen.
        guard point.x > backEdge else {
            state = .failed
            return
        }
        origin = point
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        if state == .possible, !decided, let origin, let touch = touches.first {
            let point = touch.location(in: nil)
            let across = abs(point.x - origin.x)
            let along = abs(point.y - origin.y)
            if hypot(across, along) >= Self.decision {
                decided = true
                // Only a clearly sideways start is the row's.
                if !(across > along * 1.2) {
                    state = .failed
                    return
                }
            }
        }
        super.touchesMoved(touches, with: event)
    }

    override func reset() {
        super.reset()
        origin = nil
        decided = false
    }
}
