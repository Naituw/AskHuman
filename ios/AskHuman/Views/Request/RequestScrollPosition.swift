import SwiftUI
import UIKit
import AHCore

/// Public UIKit geometry markers; no scroll delegate is replaced and no editor is focused.
struct RequestReadingAnchor: UIViewRepresentable {
    let name: String
    func makeUIView(context: Context) -> RequestReadingAnchorView { RequestReadingAnchorView(name: name) }
    func updateUIView(_ view: RequestReadingAnchorView, context: Context) { view.name = name }
}

final class RequestReadingAnchorView: UIView {
    var name: String
    init(name: String) { self.name = name; super.init(frame: .zero); isUserInteractionEnabled = false }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

struct RequestScrollPosition: UIViewRepresentable {
    let id: String
    let store: AppStore
    var ready = true

    func makeCoordinator() -> Coordinator { Coordinator(id: id, store: store) }
    func makeUIView(context: Context) -> RequestScrollAttachment {
        let view = RequestScrollAttachment(frame: .zero)
        view.changed = { [weak coordinator = context.coordinator] view in coordinator?.attach(view) }
        return view
    }
    func updateUIView(_ view: RequestScrollAttachment, context: Context) {
        context.coordinator.setReady(ready)
        context.coordinator.attach(view)
    }
    static func dismantleUIView(_ view: RequestScrollAttachment, coordinator: Coordinator) {
        view.changed = nil
        coordinator.detach()
    }

    @MainActor
    final class Coordinator {
        let id: String
        let store: AppStore
        private weak var scroll: UIScrollView?
        private var observations: [NSKeyValueObservation] = []
        private var restoring: RequestReadPosition?
        private var ready = false
        private var readyAt = 0.0
        private var expectedOffset: CGFloat?
        private var restoredSize = CGSize.zero
        private var queued = false

        init(id: String, store: AppStore) {
            self.id = id
            self.store = store
            restoring = store.requestReadPosition(for: id)
        }

        func setReady(_ value: Bool) {
            guard ready != value else { return }
            ready = value
            readyAt = CACurrentMediaTime() + 0.15
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.16) { [weak self] in self?.update() }
        }

        func attach(_ view: UIView) {
            var ancestor = view.superview
            while let parent = ancestor, !(parent is UIScrollView) { ancestor = parent.superview }
            guard let found = ancestor as? UIScrollView else { return }
            if scroll !== found {
                detach()
                scroll = found
                // Both properties are main-thread UIKit values; delivery is coalesced onto the actor.
                observations.append(found.observe(\.contentOffset) { [weak self] _, _ in
                    Task { @MainActor in self?.enqueue() }
                })
                observations.append(found.observe(\.contentSize) { [weak self] _, _ in
                    Task { @MainActor in self?.enqueue() }
                })
            }
            enqueue()
        }

        func detach() {
            observations.removeAll()
            scroll = nil
        }

        private func enqueue() {
            guard !queued else { return }
            queued = true
            DispatchQueue.main.async { [weak self] in
                self?.queued = false
                self?.update()
            }
        }

        private func anchors(in view: UIView) -> [(String, CGFloat)] {
            guard let scroll else { return [] }
            var result: [(String, CGFloat)] = []
            func visit(_ view: UIView) {
                if let anchor = view as? RequestReadingAnchorView {
                    result.append((anchor.name, anchor.convert(CGPoint.zero, to: scroll).y))
                }
                for child in view.subviews { visit(child) }
            }
            visit(view)
            return result
        }

        private func update() {
            guard let scroll, store.inboxPath.last == id else { return }
            if scroll.isTracking || scroll.isDragging || scroll.isDecelerating { restoring = nil }
            guard ready, CACurrentMediaTime() >= readyAt else { return }
            let topInset = scroll.adjustedContentInset.top
            let marks = anchors(in: scroll)
            if scroll.isTracking || scroll.isDragging || scroll.isDecelerating {
                restoring = nil
            } else if let expectedOffset, restoredSize == scroll.contentSize,
                      abs(scroll.contentOffset.y - expectedOffset) > 2 {
                // A question jump, accessibility scroll, or editor reveal is a new reading intention.
                restoring = nil
            }
            if let position = restoring {
                let top = marks.first(where: { $0.0 == position.anchor }).map { $0.1 + CGFloat(position.distance) } ?? CGFloat(position.fallback)
                let minimum = -topInset
                let maximum = max(minimum, scroll.contentSize.height - scroll.bounds.height + scroll.adjustedContentInset.bottom)
                let target = min(maximum, max(minimum, top - topInset))
                expectedOffset = target
                restoredSize = scroll.contentSize
                if abs(scroll.contentOffset.y - target) > 0.5 {
                    scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: target), animated: false)
                }
            }
            let top = scroll.contentOffset.y + topInset
            let anchor = marks.filter { $0.1 <= top + 1 }.max { $0.1 < $1.1 } ?? ("request-top", 0)
            store.saveRequestReadPosition(.init(anchor: anchor.0, distance: Double(top - anchor.1), fallback: Double(top)), for: id)
        }
    }
}

final class RequestScrollAttachment: UIView {
    var changed: ((UIView) -> Void)?
    override init(frame: CGRect) { super.init(frame: frame); isUserInteractionEnabled = false }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() { super.didMoveToWindow(); changed?(self) }
    override func layoutSubviews() { super.layoutSubviews(); changed?(self) }
}
