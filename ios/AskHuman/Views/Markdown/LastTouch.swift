import SwiftUI
import UIKit
import UIKit.UIGestureRecognizerSubclass

/// Where the screen was last touched, in window coordinates.
///
/// A tapped link reaches the app as a bare URL: SwiftUI's `openURL` and WebKit's navigation policy say what was
/// tapped, not where. The confirmation that follows is a popover on a wide screen, and a popover needs a spot to
/// point at, so the app's window remembers where each touch began (the link's own handling comes when the finger
/// lifts, which is later).
@MainActor
final class LastTouch {
    static let shared = LastTouch()

    private var point: CGPoint?
    private var time: TimeInterval = 0

    /// Where the last touch began, if that was a while ago at most. A link that was not tapped (chosen from a
    /// keyboard or by an assistive technology) has no touch behind it, and the last one belongs to something else.
    /// The window is long because a link is handled when the page has decided what the tap was, which on a busy
    /// phone can be seconds after the finger touched it.
    func recent(within seconds: TimeInterval = 15) -> CGPoint? {
        guard let point, ProcessInfo.processInfo.systemUptime - time <= seconds else { return nil }
        return point
    }

    fileprivate func record(_ point: CGPoint) {
        self.point = point
        time = ProcessInfo.processInfo.systemUptime
    }
}

/// A recognizer that only watches. It fails the moment a touch begins, so that it never takes part in what the
/// touch means to anything else (a link, a scroll, a selection): it neither waits for nor cancels them.
@MainActor
private final class TouchObserver: UIGestureRecognizer, UIGestureRecognizerDelegate {
    private let began: @MainActor (CGPoint) -> Void

    init(began: @escaping @MainActor (CGPoint) -> Void) {
        self.began = began
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
        delegate = self
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        if let touch = touches.first { began(touch.location(in: nil)) }
        state = .failed
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        true
    }
}

/// A zero-size attachment to the app's window that has `LastTouch` watch it.
struct LastTouchAnchor: UIViewRepresentable {
    func makeUIView(context: Context) -> LastTouchAttachment {
        let view = LastTouchAttachment()
        view.isUserInteractionEnabled = false
        return view
    }

    func updateUIView(_ view: LastTouchAttachment, context: Context) {}
}

/// Each attachment keeps its own recognizer on the window it is in, and takes it away when it leaves (SwiftUI may
/// build the background more than once, and one that goes must not unplug another that stays).
final class LastTouchAttachment: UIView {
    private var observer: UIGestureRecognizer?

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if let observer {
            observer.view?.removeGestureRecognizer(observer)
            self.observer = nil
        }
        guard let window else { return }
        let recognizer = TouchObserver { point in LastTouch.shared.record(point) }
        window.addGestureRecognizer(recognizer)
        observer = recognizer
    }
}
