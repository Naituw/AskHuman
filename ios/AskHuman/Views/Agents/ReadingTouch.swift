import SwiftUI
import UIKit
import UIKit.UIGestureRecognizerSubclass

// A finger that stays on, or moves over, the latest text of a watched Agent is someone reading it, and the text does not
// change under it (docs/specs/agent-observation-latest-text.md §4). The finger is watched by a recognizer that never takes
// part in what the touch means to anything else, instead of a SwiftUI `DragGesture` / `LongPressGesture` on the text: on
// iOS 27 a drag attached to something inside a scroll view takes the page's scroll away from a finger that starts on it
// (see `SwipePan`), and the text is most of what a finger lands on in the live card.

/// Calls `onReading` when a finger that began on this view stays for a moment or moves. Put it in the `.background` of
/// the view whose touches count.
struct ReadingTouch: UIViewRepresentable {
    let onReading: () -> Void

    func makeUIView(context: Context) -> ReadingTouchAttachment { ReadingTouchAttachment() }

    func updateUIView(_ view: ReadingTouchAttachment, context: Context) {
        view.onReading = onReading
    }
}

/// A zero-interaction view that keeps a recognizer on the scroll view it is in (the window when it is in none), and takes
/// it away when it leaves.
final class ReadingTouchAttachment: UIView {
    var onReading: (() -> Void)?
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
        let recognizer = ReadingRecognizer(anchor: self) { [weak self] in self?.onReading?() }
        host.addGestureRecognizer(recognizer)
        self.recognizer = recognizer
    }

    private func detach() {
        if let recognizer {
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
    private let reading: () -> Void
    private var origin: CGPoint?
    private var lingering: Task<Void, Never>?

    /// How long a finger has to stay before it counts, and how far it has to move.
    private static let linger: Duration = .milliseconds(150)
    private static let slop: CGFloat = 1

    init(anchor: UIView, reading: @escaping () -> Void) {
        self.anchor = anchor
        self.reading = reading
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
        delegate = self
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        guard origin == nil, let touch = touches.first, let anchor, anchor.window != nil,
              anchor.bounds.contains(touch.location(in: anchor)) else { state = .failed; return }
        origin = touch.location(in: nil)
        lingering = Task { @MainActor [weak self] in
            try? await Task.sleep(for: Self.linger)
            guard !Task.isCancelled else { return }
            self?.report()
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let origin, let touch = touches.first else { return }
        let point = touch.location(in: nil)
        if hypot(point.x - origin.x, point.y - origin.y) >= Self.slop { report() }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) { state = .failed }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) { state = .failed }

    override func reset() {
        super.reset()
        origin = nil
        lingering?.cancel()
        lingering = nil
    }

    private func report() {
        lingering?.cancel()
        lingering = nil
        reading()
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        true
    }
}
