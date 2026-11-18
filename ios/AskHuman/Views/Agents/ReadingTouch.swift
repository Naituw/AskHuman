import SwiftUI
import UIKit
import UIKit.UIGestureRecognizerSubclass

// A finger that stays on, or moves over, the latest text of a watched Agent is someone reading it, and the text does not
// change under it for as long as the finger is there and a moment after (docs/specs/agent-observation-latest-text.md §4).
// The finger is watched by a recognizer that never takes part in what the touch means to anything else, instead of a
// SwiftUI `DragGesture` / `LongPressGesture` on the text: on iOS 27 a drag attached to something inside a scroll view takes
// the page's scroll away from a finger that starts on it (see `SwipePan`), and the text is most of what a finger lands on
// in the live card.

/// Calls `onBegan` when a finger that began on this view stays for a moment or moves, and `onEnded` when that finger has
/// left. Put it in the `.background` of the view whose touches count.
struct ReadingTouch: UIViewRepresentable {
    let onBegan: () -> Void
    let onEnded: () -> Void

    func makeUIView(context: Context) -> ReadingTouchAttachment { ReadingTouchAttachment() }

    func updateUIView(_ view: ReadingTouchAttachment, context: Context) {
        view.onBegan = onBegan
        view.onEnded = onEnded
    }
}

/// A zero-interaction view that keeps a recognizer on the scroll view it is in (the window when it is in none), and takes
/// it away when it leaves.
final class ReadingTouchAttachment: UIView {
    var onBegan: (() -> Void)?
    var onEnded: (() -> Void)?
    private var recognizer: ReadingRecognizer?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        isAccessibilityElement = false
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func didMoveToWindow() { super.didMoveToWindow(); attach() }
    override func layoutSubviews() { super.layoutSubviews(); attach() }

    private func attach() {
        guard window != nil else { detach(); return }
        var ancestor = superview
        while let parent = ancestor, !(parent is UIScrollView) { ancestor = parent.superview }
        guard let host = ancestor ?? window else { return }
        if let recognizer, recognizer.view === host { return }
        detach()
        let recognizer = ReadingRecognizer(
            anchor: self, began: { [weak self] in self?.onBegan?() }, ended: { [weak self] in self?.onEnded?() }
        )
        host.addGestureRecognizer(recognizer)
        self.recognizer = recognizer
    }

    private func detach() {
        if let recognizer {
            // A finger that is on the text when the text goes away is not coming back to renew anything.
            recognizer.release()
            recognizer.view?.removeGestureRecognizer(recognizer)
            self.recognizer = nil
        }
    }
}

/// A recognizer that only watches: it never recognizes, never cancels a touch and never waits for or holds up another
/// recognizer, so a scroll, a selection or a tap on the same finger goes on as if it was not there.
@MainActor
final class ReadingRecognizer: UIGestureRecognizer, UIGestureRecognizerDelegate {
    private weak var anchor: UIView?
    private let began: () -> Void
    private let ended: () -> Void
    /// The finger that began on the anchor, and where.
    private weak var tracked: UITouch?
    private var origin: CGPoint?
    private var lingering: Task<Void, Never>?
    /// Whether `began` was called for the finger that is down, so that `ended` is called exactly once for it.
    private var reported = false

    /// How long a finger has to stay before it counts, and how far it has to move.
    private static let linger: Duration = .milliseconds(150)
    private static let slop: CGFloat = 1

    init(anchor: UIView, began: @escaping () -> Void, ended: @escaping () -> Void) {
        self.anchor = anchor
        self.began = began
        self.ended = ended
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
        delegate = self
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        guard origin == nil, let touch = touches.first, let anchor, anchor.window != nil,
              anchor.bounds.contains(touch.location(in: anchor)) else { state = .failed; return }
        tracked = touch
        origin = touch.location(in: nil)
        lingering = Task { @MainActor [weak self] in
            try? await Task.sleep(for: Self.linger)
            guard !Task.isCancelled else { return }
            self?.report()
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let origin, let tracked, touches.contains(tracked) else { return }
        let point = tracked.location(in: nil)
        if hypot(point.x - origin.x, point.y - origin.y) >= Self.slop { report() }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) { finger(left: touches) }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) { finger(left: touches) }

    override func reset() {
        super.reset()
        release()
    }

    /// Forgets the finger, telling the one that listens when it had been told about it.
    func release() {
        lingering?.cancel()
        lingering = nil
        origin = nil
        tracked = nil
        guard reported else { return }
        reported = false
        ended()
    }

    private func finger(left touches: Set<UITouch>) {
        // Another finger leaving does not end the reading of the one that is still on the text.
        guard let tracked, touches.contains(tracked) else { return }
        release()
        state = .failed
    }

    private func report() {
        lingering?.cancel()
        lingering = nil
        guard origin != nil, !reported else { return }
        reported = true
        began()
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        true
    }
}
