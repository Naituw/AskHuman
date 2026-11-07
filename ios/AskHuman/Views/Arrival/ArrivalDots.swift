import SwiftUI
import UIKit
import Observation

/// The unread dots of the inbox list, for the arrival animation to land on (`docs/specs/ios-arrival-motion.md`).
///
/// In a window that shows the inbox as a list beside the request (the wide layout), the list is what the user is looking at
/// when a request arrives, so the bubble flies to the unread dot of the request's row, as the Mac's flies to the dot in its
/// sidebar. Each dot of such a list has a marker view behind it (`ArrivalDotMarker`): what the animation follows is UIKit's own
/// layout, like everything else it measures, and a row that is scrolled out of sight or not on screen has no dot to land on.
///
/// The dot of a request is held back while its bubble is on its way, and shows when the bubble reaches it (`InboxRow`).
@MainActor
@Observable
final class ArrivalDots {
    static let shared = ArrivalDots()

    /// The requests whose bubble has reached the dot. The dot of a request that has an arrival under way and is not in this
    /// set is held back.
    private(set) var reached: Set<String> = []

    @ObservationIgnored private var markers: [String: WeakMarker] = [:]

    private struct WeakMarker { weak var view: ArrivalDotMarkerView? }

    fileprivate func register(_ view: ArrivalDotMarkerView, for key: String) {
        markers[key] = WeakMarker(view: view)
    }

    fileprivate func unregister(_ view: ArrivalDotMarkerView, for key: String) {
        if markers[key]?.view === view { markers[key] = nil }
    }

    /// The bubble of `key` has arrived: its dot shows.
    func reach(_ key: String) {
        guard !reached.contains(key) else { return }
        // Not from inside a view update: the controller is also told to stop from there, and the rows read this.
        DispatchQueue.main.async { [self] in
            if !reached.contains(key) { reached.insert(key) }
        }
    }

    /// Forgets the requests that no longer have an arrival (not while the arrival is still on the store: the dot would be
    /// held back again for a moment).
    func keep(only live: Set<String>) {
        let kept = reached.intersection(live)
        if kept != reached { reached = kept }
    }

    /// The middle of the dot of `key`, in the coordinates of `window`, when it is on screen: the row is in a list that is
    /// showing, and not under a bar or off the edge of it.
    func center(of key: String, in window: UIWindow) -> CGPoint? {
        guard let marker = markers[key]?.view, ArrivalLayout.visible(marker, in: window) else { return nil }
        let rect = marker.convert(marker.bounds, to: window)
        let point = CGPoint(x: rect.midX, y: rect.midY)
        var ancestor = marker.superview
        while let current = ancestor, !(current is UIScrollView) { ancestor = current.superview }
        if let scroll = ancestor as? UIScrollView {
            // What the list shows is its frame less what the bars take of it.
            let shown = scroll.convert(scroll.bounds.inset(by: scroll.adjustedContentInset), to: window)
            guard shown.insetBy(dx: 0, dy: 2).contains(point) else { return nil }
        }
        return point
    }
}

/// The frame of an unread dot, for `ArrivalDots`: put behind the dot, it is as big as it is, and shows nothing.
struct ArrivalDotMarker: UIViewRepresentable {
    let key: String

    func makeUIView(context: Context) -> ArrivalDotMarkerView {
        let view = ArrivalDotMarkerView()
        view.isUserInteractionEnabled = false
        view.isAccessibilityElement = false
        view.backgroundColor = .clear
        return view
    }

    func updateUIView(_ view: ArrivalDotMarkerView, context: Context) { view.key = key }

    static func dismantleUIView(_ view: ArrivalDotMarkerView, coordinator: ()) { view.key = nil }
}

final class ArrivalDotMarkerView: UIView {
    /// The request whose dot this is. A list reuses its rows, so the dot of another request can be put on this view.
    var key: String? {
        didSet {
            guard key != oldValue else { return }
            if let oldValue { ArrivalDots.shared.unregister(self, for: oldValue) }
            if let key { ArrivalDots.shared.register(self, for: key) }
        }
    }
}
