import Foundation

// The strip of a request page whose request ended somewhere else carries a button, and a countdown that presses it
// (docs/specs/ios-answer-flow.md §3.8). The state lives here, not in the view, so that one place decides when the page
// moves on and the tests can drive it.

/// What the strip's button does: move on, or close the page.
public enum EndedAction: Equatable, Sendable {
    /// Another request waits: the page moves on to it (`macId/requestId`).
    case next(String)
    /// Nothing else waits: the page is closed and the list is back.
    case close
}

/// The countdown on the strip of an ended request's page. When it runs out the page does what the button says, unless
/// the reader stayed (or something came over the page) before.
public struct EndedCountdown: Equatable, Sendable {
    /// The request whose page counts down (`macId/requestId`).
    public var id: String
    public var began: Date
    public var seconds: TimeInterval

    public var deadline: Date { began.addingTimeInterval(seconds) }

    public init(id: String, began: Date, seconds: TimeInterval) {
        self.id = id
        self.began = began
        self.seconds = seconds
    }
}

extension Duration {
    /// The length in seconds, for what is drawn from it (the countdown's label and fill).
    var timeInterval: TimeInterval {
        Double(components.seconds) + Double(components.attoseconds) / 1e18
    }
}

extension AppStore {
    /// What the strip's button does now. It is decided on every look and again when the button is pressed or the
    /// countdown runs out: a request that arrives meanwhile turns "Close" into "Next", and the one that was to come
    /// next may be gone by then. The same rule as after a send: the oldest unread request, else the newest.
    public func endedAction(for id: String) -> EndedAction {
        nextPending(after: id).map { .next($0.id) } ?? .close
    }

    /// Does what the strip's button says. Both the button and the countdown come here.
    public func performEndedAction(for id: String) {
        // Only the page that is up, and only once its request has ended: that is what offers the button.
        guard inboxPath.last == id, endedNotice(for: id) != nil else { return }
        stopEndedCountdown()
        switch endedAction(for: id) {
        case .next(let next):
            guard let item = item(id: next) else { return }
            Diagnostics.note("store", "ended page moves on to the next request")
            moveAnimated { self.open(macId: item.macId, requestId: item.requestId) }
        case .close:
            Diagnostics.note("store", "ended page closes")
            moveAnimated {
                self.cancelAdvance()
                self.inboxPath = []
            }
        }
    }

    /// A page change that comes from the strip, not from a tap on the page's own navigation, gets the light transition
    /// the arrows use (docs/specs/ios-request-switching.md): the app layer says what that is.
    private func moveAnimated(_ change: () -> Void) {
        if let animatePageMove { animatePageMove(change) } else { change() }
    }

    /// The reader stays: the countdown is dropped and only the button is left. For this request only.
    public func stayOnEnded(_ id: String) {
        guard endedCountdown?.id == id else { return }
        Diagnostics.note("store", "ended-page countdown stopped")
        stopEndedCountdown()
    }

    /// Starts the countdown on a page whose request has just ended elsewhere. There is none unless the page is the one
    /// in view and the reader wants to be moved on (the setting that moves on after a send), and nothing is over it.
    func beginEndedCountdown(for id: String) {
        stopEndedCountdown()
        guard autoAdvance, tab == .inbox, inboxPath.last == id, endedNotice(for: id) != nil else { return }
        // Asked here, in the turn the news came in: the page's form stops taking input a moment later, and the keyboard
        // of what was being written goes with it, so that a look from the strip would find nothing to protect.
        if pageIsCovered?() == true {
            Diagnostics.note("store", "ended page is covered: no countdown")
            return
        }
        let length = endedCountdownLength
        let countdown = EndedCountdown(id: id, began: Date(), seconds: length.timeInterval)
        endedCountdown = countdown
        Diagnostics.note("store", "ended-page countdown started (\(Int(countdown.seconds)) s)")
        endedCountdownTask = Task { [weak self] in
            try? await Task.sleep(for: length)
            guard !Task.isCancelled, let self, self.endedCountdown == countdown else { return }
            self.performEndedAction(for: id)
        }
    }

    func stopEndedCountdown() {
        endedCountdownTask?.cancel()
        endedCountdownTask = nil
        if endedCountdown != nil { endedCountdown = nil }
    }
}
