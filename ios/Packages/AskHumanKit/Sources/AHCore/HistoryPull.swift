import Foundation

/// The pull-down that asks for a session's earlier history (docs/specs/ios-session-history.md, S18): what the finger has
/// done so far, and how the header above the page should look. It knows nothing of UIKit. The page tells it how far past
/// its top it is and whether a finger is moving it, and asks what to show and whether to load.
///
/// A pull is only a pull while a finger drags the page: a bounce after letting go, the run of a flick past the top and a
/// scroll the app does by itself are none, and nothing is shown for them.
public struct HistoryPull: Equatable, Sendable {
    /// Past the top by no more than this, nothing is shown (a page that is touched and moved a little).
    public static let deadZone: CGFloat = 12
    /// From here on the header is seen in full.
    public static let fullSight: CGFloat = 36
    /// Pulled this far past the top, letting go loads (the page may ask for more where its room for the loading line is
    /// taller, see `init(threshold:)`).
    public static let defaultThreshold: CGFloat = 72
    /// Once armed, the pull has to come back this far under the threshold before it is disarmed, so that a finger
    /// resting on the threshold does not make the tap come again and again.
    public static let hysteresis: CGFloat = 4

    public enum Phase: Equatable, Sendable {
        case idle
        /// Pulled past the dead zone, short of the threshold.
        case pulling
        /// Past the threshold: letting go now loads.
        case armed
    }

    /// What the page is to do about the last position it gave.
    public enum Outcome: Equatable, Sendable {
        case nothing
        /// The pull crossed the threshold on its way out: a light tap under the finger.
        case armed
        /// The finger let go while the pull was armed: load.
        case load
    }

    public private(set) var phase: Phase = .idle
    /// Points past the top of the page; zero or less is at the top or inside the page.
    public private(set) var overscroll: CGFloat = 0
    private var dragging = false
    /// Pulled this far past the top, letting go loads.
    public let threshold: CGFloat

    public init(threshold: CGFloat = HistoryPull.defaultThreshold) {
        self.threshold = threshold
    }

    public var isArmed: Bool { phase == .armed }

    /// 0 at the dead zone to 1 at the threshold: how far the pull has come.
    public var progress: CGFloat {
        min(1, max(0, (overscroll - Self.deadZone) / (threshold - Self.deadZone)))
    }

    /// 0 until the dead zone is left, 1 once the header is in full sight.
    public var opacity: CGFloat {
        guard phase != .idle else { return 0 }
        return min(1, max(0, (overscroll - Self.deadZone) / (Self.fullSight - Self.deadZone)))
    }

    /// How far the icon has turned, counter-clockwise like a tape being rewound, in radians: a full turn at the threshold.
    public var turn: CGFloat { -2 * .pi * progress }

    /// The page moved. `overscroll` is how far past its top it is, `dragging` whether a finger is moving it and `enabled`
    /// whether the page can be pulled for history at all (it has an entry, there is something older, nothing is loading).
    ///
    /// A finger that stops dragging (`dragging` goes from true to false) lets go, so letting go is seen from here too and
    /// not only from `release()`: whichever comes first decides, the other finds nothing left to decide.
    @discardableResult
    public mutating func update(overscroll: CGFloat, dragging: Bool, enabled: Bool) -> Outcome {
        let wasDragging = self.dragging
        self.dragging = dragging
        self.overscroll = overscroll
        guard enabled else {
            phase = .idle
            return .nothing
        }
        if wasDragging, !dragging { return finish() }
        guard dragging else {
            phase = .idle
            return .nothing
        }
        let before = phase
        switch phase {
        case .idle, .pulling:
            if overscroll >= threshold {
                phase = .armed
            } else {
                phase = overscroll > Self.deadZone ? .pulling : .idle
            }
        case .armed:
            if overscroll < threshold - Self.hysteresis {
                phase = overscroll > Self.deadZone ? .pulling : .idle
            }
        }
        return before != .armed && phase == .armed ? .armed : .nothing
    }

    /// The finger lifted (the page's own pan gesture ended). True when the pull was armed, which is when to load.
    public mutating func release() -> Bool {
        dragging = false
        return finish() == .load
    }

    /// The pull is over without a decision (the page went away, or the entry did).
    public mutating func cancel() {
        dragging = false
        phase = .idle
        overscroll = 0
    }

    private mutating func finish() -> Outcome {
        let armed = phase == .armed
        phase = .idle
        return armed ? .load : .nothing
    }
}
