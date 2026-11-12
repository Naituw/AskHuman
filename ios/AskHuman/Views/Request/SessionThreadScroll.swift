import SwiftUI
import UIKit
import AHCore

// The scroll mechanics of the earlier exchanges on a request page (docs/plans/ios-session-history.md, P1 conclusion, R1).
//
// A pull on the page asks for a page of earlier exchanges (S18): the header above the page is drawn here. A pull let go past
// its threshold puts a row of the loading line (S19) at the top of the page's content and keeps the request where it was on
// screen while it does (the page then springs to rest with the row in view). When a page is in hand, the request's top is
// recorded and the thread is put in in place of the row, and every layout that follows puts the request's top back where it
// was, in the same layout pass (so there is no frame in which it is out of place). The menu way in puts the thread in the same
// way and then scrolls up to its newest exchange.
//
// (The room for the line is content, not `contentInset`: SwiftUI owns the scroll view's inset and puts its own value back
// within a frame of any other.)

/// What the request page and its scroll view share for one request: the thread, and the way the page is scrolled.
/// The page owns it (it is dropped with the page, S14); the scroll view's attachment registers itself in `scroller`.
@MainActor @Observable
final class RequestHistory {
    let thread: SessionThread
    /// The Mac the thread is read from: the pictures of its exchanges are fetched from it.
    let macId: String
    /// The page is more than a screen above the request: "Back to the current request" is offered.
    private(set) var farAbove = false
    /// The menu is loading the thread (a pull has its own room on the page).
    private(set) var menuLoading = false
    /// A pull is loading the thread: its row of the loading line is at the top of the page (S19). What the load brings (a row
    /// that says why there is nothing, or that the Mac could not be reached) takes its place when the load is over.
    private(set) var holding = false
    /// What the pull is doing (idle, pulling, armed, loading): the UI tests read it.
    private(set) var pullState = "idle"
    @ObservationIgnored weak var scroller: ThreadScrollCoordinator?
    @ObservationIgnored private let engaged: () -> Void

    /// `engaged` is told whenever the person asks for the history: on a request that has ended it stops the page's
    /// countdown (S13), as the strip's "Stay" does.
    init(thread: SessionThread, macId: String, engaged: @escaping () -> Void) {
        self.thread = thread
        self.macId = macId
        self.engaged = engaged
    }

    func setFarAbove(_ value: Bool) {
        guard farAbove != value else { return }
        farAbove = value
    }

    func setHolding(_ value: Bool) {
        guard holding != value else { return }
        holding = value
    }

    func setPullState(_ value: String) {
        guard pullState != value else { return }
        pullState = value
    }

    /// The row above the thread's first exchange, when there is one.
    enum StateRow: Equatable {
        /// A pull's load is on its way.
        case loading
        /// The Mac could not be reached.
        case failed
        /// There is nothing earlier.
        case start
    }

    var stateRow: StateRow? {
        if holding { return .loading }
        if case .failed = thread.phase { return .failed }
        if thread.reachedStart { return .start }
        return nil
    }

    /// Something is above the request: exchanges, or the row that says why there are none.
    var showsThread: Bool {
        !thread.items.isEmpty || stateRow != nil
    }

    /// The person pulled the page down and let go: the loading line has its row, and the next older page goes in above the
    /// request when it comes.
    func pull() async {
        await loadShowingRow()
    }

    /// Loads the next older page with the loading line in the row at the top of the page; whatever comes of it, the row
    /// then says how it ended.
    private func loadShowingRow() async {
        engaged()
        await loadOlder()
        await scroller?.loadFinished()
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
            await scroller.waitForThreadLayout(expectingExchanges: !thread.items.isEmpty)
        } else {
            thread.asked()
            try? await Task.sleep(for: .milliseconds(60))
        }
        scroller.endCompensation()
        let animated = !UIAccessibility.isReduceMotionEnabled
        if thread.items.isEmpty { scroller.scrollToTop(animated: animated) } else { scroller.scrollToNewest(animated: animated) }
    }

    /// "Try again" on the row that says the Mac could not be reached (S19): the same row goes back to the loading line, and
    /// the page stays where it is.
    func retry() async {
        scroller?.retryStarts()
        await loadShowingRow()
    }

    /// "Back to the current request".
    func returnToCurrent() {
        scroller?.scrollToCurrent(animated: !UIAccessibility.isReduceMotionEnabled)
    }

    private func loadOlder() async {
        guard let scroller else { return }
        await thread.loadMore(beforeInsert: { await scroller.releaseAndPrepare() })
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
            current = RequestHistory(thread: thread, macId: item.macId) { [weak store] in store?.stayOnEnded(id) }
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

/// Attaches to the scroll view of a request page: the pull-down that loads the thread, and the compensation that keeps the
/// request where it is when the thread grows above it.
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
    private weak var pan: UIPanGestureRecognizer?
    private var sizeObservation: NSKeyValueObservation?
    private var offsetObservation: NSKeyValueObservation?
    private var bounceObservation: NSKeyValueObservation?
    private weak var cachedAnchor: RequestReadingAnchorView?
    private var pending: Pending?
    private var ancestorObservations: [NSKeyValueObservation] = []
    /// Tells a timer of an earlier `prepareInsertion` from the one of the current.
    private var generation = 0

    // The pull (S18).
    private var pull = HistoryPull()
    private var header: PullHeaderView?
    private var haptic: UIImpactFeedbackGenerator?
    private var holdStart: CFTimeInterval = 0
    /// The row of the loading line is on its way in, and the page is yet to spring to rest with it.
    private var settlePending = false
    /// The least time the loading line is on the page, so that an answer that is already there does not make it flicker.
    private static let minimumHold = 0.6
    /// How long the layout of what was put in above the request is waited for, at most. A main thread that is starved
    /// (a device short of memory, a machine that is swapping) can be a second late with it, and a window counted from the
    /// moment the insertion was asked for would be over by then.
    private static let layoutPatience = 4.0
    /// How long the passes that follow the first layout of the insertion (a card measuring itself again) are made up for.
    private static let followUp = 0.5

    private struct Pending {
        /// Where the request's top was in the viewport before the thread grew.
        let anchorViewportY: CGFloat
        /// Where the request's top was in the content before the thread grew.
        let anchorContentY: CGFloat
        /// Layouts up to this time are made up for. For a thread coming in it starts far off, and is brought to a short
        /// while ahead once the layout of the insertion has come (`applied`).
        var until: CFTimeInterval
        /// The insertion has been laid out: the request has been moved by it.
        var applied = false
        /// The row of a pull's loading line is being put in: once the request has been moved by it, the page springs to rest.
        let settling: Bool
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
            // The pull is followed on every change of the offset; the model is only written to on a crossing.
            offsetObservation = found.observe(\.contentOffset, options: []) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.offsetChanged() }
            }
            found.panGestureRecognizer.addTarget(self, action: #selector(panned(_:)))
            pan = found.panGestureRecognizer
            #if DEBUG
            PullProbe.shared?.run { [weak self] in self?.probeLine() ?? "" }
            #endif
        }
        keepBouncing()
        layoutHeader()
    }

    func detach() {
        sizeObservation = nil
        offsetObservation = nil
        bounceObservation = nil
        ancestorObservations = []
        pan?.removeTarget(self, action: nil)
        pan = nil
        header?.removeFromSuperview()
        header = nil
        pull.cancel()
        scroll = nil
        pending = nil
        settlePending = false
        history.setHolding(false)
        history.setPullState("idle")
        history.setFarAbove(false)
    }

    // MARK: pulling

    /// A page that has reached its start only bounces, one that is loading is not asked again, and one whose row of the
    /// loading line is up has asked already.
    private var pullEnabled: Bool {
        !history.thread.reachedStart && history.thread.phase != .loading && !history.menuLoading && !history.holding
    }

    /// SwiftUI sets `alwaysBounceVertical` of its scroll view by the size of the content (a short page, which is most of
    /// them, is then not pulled at all): it is put back the moment it is taken away.
    private func keepBouncing() {
        guard let scroll else { return }
        if pullEnabled, !scroll.alwaysBounceVertical { scroll.alwaysBounceVertical = true }
        if bounceObservation == nil {
            bounceObservation = scroll.observe(\.alwaysBounceVertical, options: []) { [weak self] scroll, _ in
                MainActor.assumeIsolated {
                    guard let self, self.pullEnabled || self.history.holding, !scroll.alwaysBounceVertical else { return }
                    scroll.alwaysBounceVertical = true
                }
            }
        }
    }

    private func offsetChanged() {
        updateFarAbove()
        trackPull()
    }

    /// Feeds the state machine how far past its top the page is and whether a finger is moving it.
    private func trackPull() {
        guard let scroll else { return }
        let over = -(scroll.contentOffset.y + scroll.adjustedContentInset.top)
        let outcome = pull.update(overscroll: over, dragging: scroll.isDragging, enabled: pullEnabled)
        switch outcome {
        case .armed:
            haptic?.impactOccurred()
            haptic?.prepare()
        case .load:
            startLoading()
        case .nothing:
            break
        }
        showHeader()
    }

    @objc private func panned(_ gesture: UIPanGestureRecognizer) {
        switch gesture.state {
        case .began:
            // Where the threshold is depends on how tall the row that takes the header's place will be: the pull has to
            // be let go further out than that, or the row would have to hold the page open less than it is tall.
            pull = HistoryPull(threshold: max(HistoryPull.defaultThreshold, PullMetrics.holdHeight + 8))
            haptic = UIImpactFeedbackGenerator(style: .light)
            haptic?.prepare()
        case .ended:
            // Whichever of this and the offset's own change sees the lift first decides; the other finds nothing to do.
            if pull.release() { startLoading() } else { showHeader() }
        case .cancelled, .failed:
            pull.cancel()
            showHeader()
        default:
            break
        }
    }

    /// What the header shows while a finger pulls. Once the pull is let go and the loading line has its row, the header goes.
    private func showHeader() {
        if pull.phase == .idle || scroll == nil {
            // A pull that was let go loading leaves the header to the moment its row is in (see `reconcile`).
            if !settlePending, let header, !header.isHidden { header.isHidden = true }
            if !history.holding { history.setPullState("idle") }
            return
        }
        let header = ensureHeader()
        header.isHidden = false
        header.show(pull, older: !history.thread.items.isEmpty)
        history.setPullState(pull.isArmed ? "armed" : "pulling")
    }

    private func ensureHeader() -> PullHeaderView {
        if let header { return header }
        let made = PullHeaderView(frame: .zero)
        made.autoresizingMask = [.flexibleWidth]
        made.isHidden = true
        scroll?.addSubview(made)
        header = made
        layoutHeader()
        return made
    }

    /// The header is a subview of the scroll view just above its content, where the loading row will stand: it moves with
    /// the content, frame for frame.
    private func layoutHeader() {
        guard let scroll, let header else { return }
        let room = PullMetrics.holdHeight
        header.frame = CGRect(x: 0, y: -room, width: scroll.bounds.width, height: room)
    }

    // MARK: loading

    /// The pull was let go while armed: the loading line gets its row at the top of the page and the load starts. The
    /// request's top is recorded first, so that the row coming in above it does not move it; the page, still pulled out
    /// by the finger, then springs to rest with the row in view.
    private func startLoading() {
        guard scroll != nil, !history.holding else { return }
        holdStart = CACurrentMediaTime()
        settlePending = true
        prepareInsertion(settling: true)
        history.setHolding(true)
        history.setPullState("loading")
        #if DEBUG
        PullProbe.shared?.note("loading starts")
        #endif
        UIAccessibility.post(notification: .announcement, argument: String(localized: "Loading history…"))
        let history = history
        Task { @MainActor in await history.pull() }
    }

    /// "Try again": the row that said the Mac could not be reached goes back to the loading line. It has the room of the
    /// loading line already (a row of each kind is as tall as the others), so nothing moves.
    func retryStarts() {
        holdStart = CACurrentMediaTime()
        history.setHolding(true)
        history.setPullState("loading")
        UIAccessibility.post(notification: .announcement, argument: String(localized: "Loading history…"))
    }

    /// The row of the loading line is in, and the request is where it was: the page springs to rest, where the row is in full
    /// view above the request.
    ///
    /// UIKit's own spring back from the pull was stopped by putting the request right (a write of the offset that is not
    /// an animation stops it), and an offset out of range stands until the next touch: the spring is started here.
    private func settleLoadingRow() {
        guard settlePending else { return }
        settlePending = false
        header?.isHidden = true
        guard let scroll else { return }
        endCompensation()
        scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: -scroll.adjustedContentInset.top), animated: true)
        #if DEBUG
        PullProbe.shared?.note("page springs to rest")
        #endif
    }

    /// A page is in hand (or the load failed): once the loading line has been on the page for its least time and the page is
    /// at rest, the request's top is recorded and the thread takes the row's place.
    func releaseAndPrepare() async {
        if history.holding {
            let shown = CACurrentMediaTime() - holdStart
            if shown < Self.minimumHold { try? await Task.sleep(for: .seconds(Self.minimumHold - shown)) }
        }
        await waitForRest()
        prepareInsertion()
        #if DEBUG
        PullProbe.shared?.note("thread comes in")
        PullProbe.shared?.stallIfAsked()
        #endif
        history.setHolding(false)
        history.setPullState("idle")
    }

    /// A load is over, whatever came of it: a row that is still the loading line (the load brought nothing to put in) gives
    /// way to whatever says how the load ended.
    func loadFinished() async {
        if history.holding { await releaseAndPrepare() }
    }

    /// A pull takes effect while the finger is still down, and the finger may stay down a good while after that. The wait
    /// is for the page to come to rest once the finger is up: the spring that brings it back has a long tail, and a thread
    /// put in before the tail is over would keep the request a few points off the place it had before the pull. How long
    /// the spring gets is counted from the lift, not from the start, so that a slow pull does not eat it.
    private func waitForRest() async {
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
    }

    // MARK: keeping the request where it is

    /// Records where the request's top is on screen; the next layouts put it back there.
    func prepareInsertion(settling: Bool = false) {
        guard let scroll, let anchor = requestAnchor() else { return }
        let y = anchor.convert(CGPoint.zero, to: scroll).y
        let relative = y - scroll.contentOffset.y
        generation += 1
        let mine = generation
        pending = Pending(
            anchorViewportY: relative, anchorContentY: y,
            until: CACurrentMediaTime() + (settling ? Self.followUp : Self.layoutPatience), settling: settling
        )
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
        watchInsertion(generation: mine, settling: settling, after: settling ? 0.3 : 0.55)
    }

    /// Ends what an insertion asked of the page once its time is up. A row of a pull that did not come in the time it should
    /// have is not waited for any longer, and the page springs to rest. A thread's window is as long as `until` says: far
    /// off while its layout is yet to come, a short while once it has (checked every tenth of a second).
    private func watchInsertion(generation mine: Int, settling: Bool, after delay: TimeInterval) {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, self.generation == mine else { return }
            if settling { self.settleLoadingRow(); return }
            if let pending = self.pending, pending.until > CACurrentMediaTime() {
                self.watchInsertion(generation: mine, settling: false, after: 0.1)
                return
            }
            self.endCompensation()
        }
    }

    func reconcile() {
        guard let scroll, var pending else { return }
        guard CACurrentMediaTime() < pending.until, let anchor = requestAnchor() else {
            #if DEBUG
            PullProbe.shared?.note("compensation window over")
            #endif
            self.pending = nil
            return
        }
        let y = anchor.convert(CGPoint.zero, to: scroll).y
        let desired = y - pending.anchorViewportY
        let moved = abs(y - pending.anchorContentY) > 0.5
        if !pending.settling {
            // Until the layout of the insertion has come there is nothing to put right. Once it has, the request goes back to
            // where it was in the viewport when the insertion was asked for.
            if !moved, !pending.applied { return }
            if !pending.applied {
                pending.applied = true
                pending.until = CACurrentMediaTime() + Self.followUp
                self.pending = pending
            }
        }
        if abs(scroll.contentOffset.y - desired) > 0.25 {
            #if DEBUG
            PullProbe.shared?.note(String(format: "request put right by %.1f", desired - scroll.contentOffset.y))
            #endif
            if pending.settling, moved {
                // UIKit's own spring (the page coming back from the pull) is stopped: a plain write would be overwritten by
                // its next frame.
                scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: desired), animated: false)
            } else {
                scroll.contentOffset = CGPoint(x: scroll.contentOffset.x, y: desired)
            }
        }
        if pending.settling, moved, settlePending {
            // The row has taken the header's place on screen (the page was moved by exactly its height to keep the request
            // where it was): the header, now a row's height too high, goes in this very pass. The spring starts with the
            // next turn of the run loop, once every layout that the row brought is done and put right.
            header?.isHidden = true
            DispatchQueue.main.async { [weak self] in self?.settleLoadingRow() }
        }
    }

    /// The page is where it should be: from here on a change of layout is not made up for.
    func endCompensation() {
        pending = nil
        ancestorObservations = []
    }

    // MARK: going somewhere

    /// Waits until the page has laid out what was put in above the request: the newest exchange has its view in the scroll
    /// view and the content has stopped changing size. A fixed wait is not enough: on a busy device the frame that lays the
    /// thread out can come later than any time chosen for it, and the place to scroll to is then not there to be found. At
    /// most two seconds; what is not there by then is scrolled to as well as it can be.
    func waitForThreadLayout(expectingExchanges: Bool) async {
        var size = scroll?.contentSize.height ?? 0
        try? await Task.sleep(for: .milliseconds(120))
        guard expectingExchanges else { return }
        let begin = CACurrentMediaTime()
        // The content counts as still for the wait just over, unless it changed meanwhile.
        var changedAt = begin - 0.1
        while let scroll, CACurrentMediaTime() - begin < 2 {
            let now = CACurrentMediaTime()
            if abs(scroll.contentSize.height - size) > 0.5 {
                size = scroll.contentSize.height
                changedAt = now
            }
            if threadAnchor("thread-newest") != nil, now - changedAt >= 0.1 { return }
            try? await Task.sleep(for: .milliseconds(30))
        }
    }

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

    // MARK: prototype probe

    #if DEBUG
    /// One line of what the page does this frame: where the page is, where the request is, what the pull is doing.
    private func probeLine() -> String {
        guard let scroll else { return "" }
        let request = requestAnchor().map { String(format: "%.2f", $0.convert(CGPoint.zero, to: scroll).y - scroll.contentOffset.y) } ?? "nil"
        // The header as it is drawn: p or r (pull down, release), 1 for the wording of an older page, and how strong it is.
        var hdr = "-"
        var alpha = 0.0
        var turns = 0.0
        if let header, !header.isHidden {
            alpha = header.alpha
            turns = header.iconTurns
            switch header.look {
            case .pull(let older): hdr = older ? "p1" : "p0"
            case .release(let older): hdr = older ? "r1" : "r0"
            }
        }
        return String(
            format: "off=%.2f adj=%.2f size=%.1f req=%@ drag=%ld trk=%ld dec=%ld bv=%ld hold=%ld pull=%@ hdr=%@ ha=%.2f turn=%.2f rm=%ld",
            scroll.contentOffset.y, scroll.adjustedContentInset.top, scroll.contentSize.height, request,
            scroll.isDragging ? 1 : 0, scroll.isTracking ? 1 : 0, scroll.isDecelerating ? 1 : 0,
            scroll.alwaysBounceVertical ? 1 : 0,
            history.holding ? 1 : 0, history.pullState, hdr, alpha, turns, UIAccessibility.isReduceMotionEnabled ? 1 : 0
        )
    }
    #endif
}
