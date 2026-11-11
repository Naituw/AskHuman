import SwiftUI
import UIKit
import AHCore

// The scroll mechanics of the earlier exchanges on a request page (docs/plans/ios-session-history.md, P1 conclusion).
//
// A pull on the page loads a page of earlier exchanges above the request, and what the person is looking at does not
// move: the request's top is recorded, the thread is put in, and every layout that follows puts the request's top back
// where it was, in the same layout pass (so there is no frame in which it is out of place). The menu way in puts the
// thread in the same way and then scrolls up to its newest exchange.

/// What the request page and its scroll view share for one request: the thread, and the way the page is scrolled.
/// The page owns it (it is dropped with the page, S14); the scroll view's attachment registers itself in `scroller`.
@MainActor @Observable
final class RequestHistory {
    let thread: SessionThread
    /// The page is more than a screen above the request: "Back to the current request" is offered.
    private(set) var farAbove = false
    /// The menu is loading the thread (a pull has the spinner of its own).
    private(set) var menuLoading = false
    @ObservationIgnored weak var scroller: ThreadScrollCoordinator?
    @ObservationIgnored private let engaged: () -> Void

    /// `engaged` is told whenever the person asks for the history: on a request that has ended it stops the page's
    /// countdown (S13), as the strip's "Stay" does.
    init(thread: SessionThread, engaged: @escaping () -> Void) {
        self.thread = thread
        self.engaged = engaged
    }

    func setFarAbove(_ value: Bool) {
        guard farAbove != value else { return }
        farAbove = value
    }

    /// Something is above the request: exchanges, or the row that says why there are none.
    var showsThread: Bool {
        if !thread.items.isEmpty || thread.reachedStart { return true }
        if case .failed = thread.phase { return true }
        return false
    }

    /// The person pulled the page down: the next older page goes in above, and the page stays where it is.
    func pull() async {
        engaged()
        await loadOlder()
        scroller?.loadFinished()
    }

    /// The menu: the history is loaded when it was not, and the page scrolls to its newest exchange (to the top, where
    /// the row that says there is nothing earlier, or that the Mac could not be reached, is, when there is none).
    func reveal() async {
        guard !menuLoading, thread.phase != .loading, let scroller else { return }
        engaged()
        if thread.items.isEmpty, !thread.reachedStart {
            menuLoading = true
            await thread.loadMore(beforeInsert: { scroller.prepareInsertion() })
            menuLoading = false
            // The page is laid out once more before the place to scroll to is known.
            try? await Task.sleep(for: .milliseconds(120))
        } else {
            thread.asked()
            try? await Task.sleep(for: .milliseconds(60))
        }
        scroller.endCompensation()
        scroller.loadFinished()
        let animated = !UIAccessibility.isReduceMotionEnabled
        if thread.items.isEmpty { scroller.scrollToTop(animated: animated) } else { scroller.scrollToNewest(animated: animated) }
    }

    /// "Try again" on the row that says the Mac could not be reached.
    func retry() async {
        engaged()
        if thread.items.isEmpty { await reveal() } else { await loadOlder() }
    }

    /// "Back to the current request".
    func returnToCurrent() {
        scroller?.scrollToCurrent(animated: !UIAccessibility.isReduceMotionEnabled)
    }

    private func loadOlder() async {
        guard let scroller else { return }
        await thread.loadMore(beforeInsert: { await scroller.collapseAndPrepare() })
    }
}

/// Keeps the page's one `RequestHistory` while it is offered, in a place that can be written to while the page's body is
/// worked out (it is a plain object: nothing observes it).
@MainActor
final class RequestHistoryHolder {
    private var current: RequestHistory?
    private var currentId: String?

    /// The history of `item`'s page: made the first time the request qualifies, and dropped when it no longer does.
    func history(for item: InboxItem?, store: AppStore) -> RequestHistory? {
        guard let item, store.sessionHistoryAvailable(for: item) else {
            current = nil
            currentId = nil
            return nil
        }
        if currentId != item.id || current == nil {
            guard let thread = store.makeSessionThread(for: item) else { return nil }
            let id = item.id
            current = RequestHistory(thread: thread) { [weak store] in store?.stayOnEnded(id) }
            currentId = id
        }
        return current
    }
}

/// A zero-size UIKit marker, like `RequestReadingAnchor`, but of the thread: it is not one of the places the reading
/// position is kept as.
struct SessionThreadAnchor: UIViewRepresentable {
    let name: String
    func makeUIView(context: Context) -> SessionThreadAnchorView { SessionThreadAnchorView(name: name) }
    func updateUIView(_ view: SessionThreadAnchorView, context: Context) { view.name = name }
}

final class SessionThreadAnchorView: UIView {
    var name: String
    init(name: String) { self.name = name; super.init(frame: .zero); isUserInteractionEnabled = false }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

/// Attaches to the scroll view of a request page: the pull-down control that loads the thread, and the compensation that
/// keeps the request where it is when the thread grows above it.
struct ThreadScrollHook: UIViewRepresentable {
    let history: RequestHistory

    func makeCoordinator() -> ThreadScrollCoordinator { ThreadScrollCoordinator(history: history) }
    func makeUIView(context: Context) -> RequestScrollAttachment {
        let view = RequestScrollAttachment(frame: .zero)
        view.changed = { [weak coordinator = context.coordinator] view in
            coordinator?.attach(view)
            // The attachment is laid out after the content around it has been placed: the place to put the page right.
            coordinator?.reconcile()
        }
        return view
    }
    func updateUIView(_ view: RequestScrollAttachment, context: Context) { context.coordinator.attach(view) }
    static func dismantleUIView(_ view: RequestScrollAttachment, coordinator: ThreadScrollCoordinator) {
        view.changed = nil
        coordinator.detach()
    }
}

@MainActor
final class ThreadScrollCoordinator: NSObject {
    private let history: RequestHistory
    private weak var scroll: UIScrollView?
    private var refresh: UIRefreshControl?
    private var sizeObservation: NSKeyValueObservation?
    private var offsetObservation: NSKeyValueObservation?
    private var bounceObservation: NSKeyValueObservation?
    private var refreshObservation: NSKeyValueObservation?
    private weak var cachedAnchor: RequestReadingAnchorView?
    private var pending: Pending?
    private var ancestorObservations: [NSKeyValueObservation] = []

    private struct Pending {
        /// Where the request's top was in the viewport before the thread grew.
        let anchorViewportY: CGFloat
        let until: CFTimeInterval
    }

    init(history: RequestHistory) {
        self.history = history
        super.init()
        history.scroller = self
    }

    func attach(_ view: UIView) {
        var ancestor = view.superview
        while let parent = ancestor, !(parent is UIScrollView) { ancestor = parent.superview }
        guard let found = ancestor as? UIScrollView else { return }
        if scroll !== found {
            detach()
            scroll = found
            sizeObservation = found.observe(\.contentSize, options: []) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.reconcile() }
            }
            // Only a crossing of the threshold is written to the model, never every frame of a scroll.
            offsetObservation = found.observe(\.contentOffset, options: []) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.updateFarAbove() }
            }
        }
        installRefresh()
    }

    func detach() {
        sizeObservation = nil
        offsetObservation = nil
        bounceObservation = nil
        refreshObservation = nil
        ancestorObservations = []
        removeRefresh()
        scroll = nil
        history.setFarAbove(false)
    }

    // MARK: pulling

    /// The control is on the page while there is something older to ask for: a page that has reached its start only bounces.
    ///
    /// SwiftUI owns two properties of its scroll view that this control needs: it takes `refreshControl` away on its own
    /// updates (the page has no `.refreshable`), and sets `alwaysBounceVertical` by the size of the content (a short page,
    /// which is most of them, is then not pulled at all). Both are put back the moment they are taken away.
    private func installRefresh() {
        guard let scroll, !history.thread.reachedStart else { return }
        let control: UIRefreshControl
        if let refresh {
            control = refresh
        } else {
            control = UIRefreshControl()
            control.addTarget(self, action: #selector(pulled), for: .valueChanged)
            refresh = control
        }
        if scroll.refreshControl !== control { scroll.refreshControl = control }
        if !scroll.alwaysBounceVertical { scroll.alwaysBounceVertical = true }
        if refreshObservation == nil {
            refreshObservation = scroll.observe(\.refreshControl, options: []) { [weak self] scroll, _ in
                MainActor.assumeIsolated {
                    guard let refresh = self?.refresh, scroll.refreshControl !== refresh else { return }
                    scroll.refreshControl = refresh
                }
            }
        }
        if bounceObservation == nil {
            bounceObservation = scroll.observe(\.alwaysBounceVertical, options: []) { [weak self] scroll, _ in
                MainActor.assumeIsolated {
                    if self?.refresh != nil, !scroll.alwaysBounceVertical { scroll.alwaysBounceVertical = true }
                }
            }
        }
    }

    private func removeRefresh() {
        let control = refresh
        refresh = nil
        refreshObservation = nil
        bounceObservation = nil
        if let scroll, let control, scroll.refreshControl === control { scroll.refreshControl = nil }
    }

    @objc private func pulled() {
        let history = history
        Task { @MainActor in await history.pull() }
    }

    /// A load is over, whatever came of it: the control goes back (for good once there is nothing older to pull for).
    func loadFinished() {
        refresh?.endRefreshing()
        if history.thread.reachedStart { removeRefresh() }
    }

    /// The control goes back first; the thread is put in once the page is at rest again.
    ///
    /// A pull takes effect while the finger is still down, and the finger may stay down a good while after that. The wait
    /// is for the page to come to rest once the finger is up: the spring that brings it back has a long tail, and a
    /// thread put in before the tail is over would keep the request a few points off the place it had before the pull.
    /// How long the spring gets is counted from the lift, not from the start, so that a slow pull does not eat it.
    func collapseAndPrepare() async {
        refresh?.endRefreshing()
        let start = CACurrentMediaTime()
        var lifted: CFTimeInterval?
        while let scroll, !Task.isCancelled, CACurrentMediaTime() - start < 10 {
            let now = CACurrentMediaTime()
            if scroll.isTracking {
                lifted = nil
            } else {
                let since = lifted ?? now
                lifted = since
                if !scroll.isDecelerating, scroll.contentOffset.y >= -scroll.adjustedContentInset.top - 0.5 { break }
                if now - since > 1.5 { break }
            }
            try? await Task.sleep(for: .milliseconds(16))
        }
        prepareInsertion()
    }

    // MARK: keeping the request where it is

    /// Records where the request's top is on screen; the next layouts put it back there.
    func prepareInsertion() {
        guard let scroll, let anchor = requestAnchor() else { return }
        let y = anchor.convert(CGPoint.zero, to: scroll).y
        let relative = y - scroll.contentOffset.y
        pending = Pending(anchorViewportY: relative, until: CACurrentMediaTime() + 0.5)
        // SwiftUI moves the host of a representable, not the view inside it: every view between the anchor and the
        // scroll view is watched, so whichever of them it moves, the page is put right.
        ancestorObservations = []
        var view: UIView? = anchor
        while let current = view, current !== scroll {
            ancestorObservations.append(current.layer.observe(\.position, options: []) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.reconcile() }
            })
            ancestorObservations.append(current.layer.observe(\.bounds, options: []) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.reconcile() }
            })
            view = current.superview
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.55) { [weak self] in self?.endCompensation() }
    }

    func reconcile() {
        guard let scroll, let pending else { return }
        guard CACurrentMediaTime() < pending.until, let anchor = requestAnchor() else {
            self.pending = nil
            return
        }
        let y = anchor.convert(CGPoint.zero, to: scroll).y
        let desired = y - pending.anchorViewportY
        if abs(scroll.contentOffset.y - desired) > 0.25 {
            scroll.contentOffset = CGPoint(x: scroll.contentOffset.x, y: desired)
        }
    }

    /// The page is where it should be: from here on a change of layout is not made up for.
    func endCompensation() {
        pending = nil
        ancestorObservations = []
    }

    // MARK: going somewhere

    /// Scrolls to the top of the newest exchange of the thread.
    func scrollToNewest(animated: Bool) {
        guard let scroll, let anchor = threadAnchor("thread-newest") else { return }
        let y = anchor.convert(CGPoint.zero, to: scroll).y
        let target = max(-scroll.adjustedContentInset.top, y - scroll.adjustedContentInset.top - 8)
        scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: target), animated: animated)
    }

    func scrollToTop(animated: Bool) {
        guard let scroll else { return }
        scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: -scroll.adjustedContentInset.top), animated: animated)
    }

    /// Scrolls to where the request starts.
    func scrollToCurrent(animated: Bool) {
        guard let scroll, let anchor = requestAnchor() else { return }
        let y = anchor.convert(CGPoint.zero, to: scroll).y
        let target = max(-scroll.adjustedContentInset.top, y - scroll.adjustedContentInset.top)
        scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: target), animated: animated)
    }

    // MARK: "back to the current request"

    /// More than a screen above the request: offered. A little hysteresis keeps it from flickering at the threshold.
    private func updateFarAbove() {
        guard let scroll else { return }
        guard !history.thread.items.isEmpty, let anchor = requestAnchor() else {
            history.setFarAbove(false)
            return
        }
        let requestTop = anchor.convert(CGPoint.zero, to: scroll).y
        let viewportTop = scroll.contentOffset.y + scroll.adjustedContentInset.top
        let screen = scroll.bounds.height - scroll.adjustedContentInset.top - scroll.adjustedContentInset.bottom
        let above = requestTop - viewportTop
        history.setFarAbove(history.farAbove ? above > screen * 0.85 : above > screen)
    }

    // MARK: finding the markers

    private func requestAnchor() -> RequestReadingAnchorView? {
        if let cachedAnchor, cachedAnchor.window != nil { return cachedAnchor }
        guard let scroll else { return nil }
        func find(_ view: UIView) -> RequestReadingAnchorView? {
            if let anchor = view as? RequestReadingAnchorView, anchor.name == "request-top" { return anchor }
            for child in view.subviews { if let found = find(child) { return found } }
            return nil
        }
        cachedAnchor = find(scroll)
        return cachedAnchor
    }

    private func threadAnchor(_ name: String) -> SessionThreadAnchorView? {
        guard let scroll else { return nil }
        func find(_ view: UIView) -> SessionThreadAnchorView? {
            if let anchor = view as? SessionThreadAnchorView, anchor.name == name { return anchor }
            for child in view.subviews { if let found = find(child) { return found } }
            return nil
        }
        return find(scroll)
    }
}
