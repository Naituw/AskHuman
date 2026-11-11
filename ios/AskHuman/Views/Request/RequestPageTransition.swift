import SwiftUI
import UIKit
import Observation
import AHCore

/// A frozen copy of the ended page lasts only for its slide into the next request. It never goes to disk.
@MainActor @Observable
final class RequestPageTransition {
    static let shared = RequestPageTransition()

    struct Snapshot: Identifiable {
        let id = UUID()
        let destination: String
        let view: UIView
    }

    private(set) var snapshot: Snapshot?
    @ObservationIgnored private weak var surface: RequestPageCapture.Reader?
    @ObservationIgnored private var expiration: Task<Void, Never>?

    func register(_ view: RequestPageCapture.Reader) { surface = view }

    func snapshot(for id: String) -> Snapshot? {
        snapshot.flatMap { $0.destination == id ? $0 : nil }
    }

    func complete(_ id: UUID) {
        guard snapshot?.id == id else { return }
        expiration?.cancel()
        expiration = nil
        snapshot = nil
    }

    func move(in store: AppStore, change: () -> Void) {
        PageEditing.commit()
        guard let from = store.inboxPath.last, case .next = store.endedAction(for: from) else {
            withAnimation(.easeOut(duration: 0.18), change)
            return
        }
        let frozen = PageEditing.isVisible && surface?.requestId == from ? surface?.capture() : nil
        change()
        guard let frozen, let next = store.inboxPath.last, next != from else {
            if let snapshot { complete(snapshot.id) }
            return
        }
        if let snapshot { complete(snapshot.id) }
        let departure = Snapshot(destination: next, view: frozen)
        snapshot = departure
        // A tab change can prevent the destination view from mounting and running its own cleanup.
        expiration = Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            self?.complete(departure.id)
        }
        Diagnostics.note("nav", "ended request prepares the next page slide")
    }
}

/// Measures the content area of the current request, excluding the app's navigation and tab bars.
struct RequestPageCapture: UIViewRepresentable {
    let id: String

    func makeUIView(context: Context) -> Reader {
        let view = Reader()
        view.requestId = id
        view.isUserInteractionEnabled = false
        view.isAccessibilityElement = false
        return view
    }

    func updateUIView(_ view: Reader, context: Context) {
        view.requestId = id
        if view.window != nil { RequestPageTransition.shared.register(view) }
    }

    final class Reader: UIView {
        var requestId = ""

        override func didMoveToWindow() {
            super.didMoveToWindow()
            if window != nil { RequestPageTransition.shared.register(self) }
        }

        func capture() -> UIView? {
            guard let window, !bounds.isEmpty else { return nil }
            let area = convert(bounds, to: window).intersection(window.bounds)
            guard !area.isEmpty else { return nil }
            return window.resizableSnapshotView(from: area, afterScreenUpdates: false, withCapInsets: .zero)
        }
    }
}

/// The departing page is an inert native snapshot: its editors, drafts and web view are not recreated.
struct RequestPageSnapshot: UIViewRepresentable {
    let snapshot: RequestPageTransition.Snapshot

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.isUserInteractionEnabled = false
        view.isAccessibilityElement = false
        view.accessibilityElementsHidden = true
        view.clipsToBounds = true
        snapshot.view.frame = view.bounds
        snapshot.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(snapshot.view)
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        snapshot.view.frame = view.bounds
    }
}
