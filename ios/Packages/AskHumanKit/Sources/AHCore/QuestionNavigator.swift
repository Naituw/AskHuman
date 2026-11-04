import CoreGraphics

/// Where the "Next" button of a form of several questions leads, worked out from what is on screen.
///
/// The Mac popup moves a "current question" pointer one step down and brings that question into view
/// (spec `multi-question-vertical.md` N1–N4). On the phone the pointer is implicit: it is where the
/// screen is. The target is the first question whose bottom is not on screen yet, which is the one
/// after the last question the person has reached, however far down they went before and whatever was
/// seen or answered on the way. (An earlier version led to "the first question not read yet", which
/// jumped over the questions that were looked at but not answered once the person had scrolled back.)
///
/// One thing sets the pointer by hand, as it does in the popup (H4: typing in a question makes it the
/// current one): the keyboard. A person who is typing is in the question that has it, and "Next" goes
/// to the one after, whatever the screen shows (`next(after:in:count:margin:)`).
public enum QuestionNavigator {
    /// Which edge of the visible area the question is lined up with.
    public enum Anchor: Equatable, Sendable {
        case top
        case bottom
        /// The question is all on screen already: nothing scrolls, it is only pointed at.
        case stay
    }

    public struct Target: Equatable, Sendable {
        public let question: Int
        public let anchor: Anchor

        public init(question: Int, anchor: Anchor) {
            self.question = question
            self.anchor = anchor
        }
    }

    /// What the screen shows of the form, all in one coordinate space.
    public struct Viewport: Equatable, Sendable {
        /// The frame of every question card.
        public var cards: [Int: CGRect]
        /// Where the visible area starts (below the navigation bar).
        public var top: CGFloat
        /// Where the visible area ends (above the bottom bar).
        public var bottom: CGFloat

        public init(cards: [Int: CGRect], top: CGFloat, bottom: CGFloat) {
            self.cards = cards
            self.top = top
            self.bottom = bottom
        }

        public var height: CGFloat { bottom - top }
    }

    /// Slack for the rounding of layout numbers.
    private static let tolerance: CGFloat = 1

    /// The target of "Next": the first question whose bottom is below the visible area, or `nil` when
    /// every question has been reached (or there is nothing to go through).
    ///
    /// A question that fits on screen is lined up with the bottom edge, so that all of it shows. One
    /// taller than the screen is shown from its top first; once its top is at the edge, the next press
    /// shows its bottom, and only then does "Next" move on.
    public static func next(in viewport: Viewport, count: Int, margin: CGFloat = 16) -> Target? {
        guard count > 1 else { return nil }
        for question in 0..<count {
            guard let frame = viewport.cards[question] else { continue }
            if frame.maxY <= viewport.bottom + tolerance { continue }
            return lead(to: question, frame, in: viewport, margin: margin)
        }
        return nil
    }

    /// The target of "Next" for a person typing in `question`: the question after it, wherever it is.
    ///
    /// The position cannot say where a person is while the keyboard hides half the screen, and the card
    /// typed in reaches down to the bar, a point or two below its edge. The keyboard can: it is in
    /// `question`. The target is brought into view as `next(in:count:margin:)` does (all of it when it
    /// fits, a taller one from its top and then from its bottom); one that is all on screen is not
    /// scrolled, only pointed at (`Anchor.stay`), as the popup's "next" does. `nil` when `question` is
    /// the last one, or the place of the one after it is not known.
    public static func next(after question: Int, in viewport: Viewport, count: Int, margin: CGFloat = 16) -> Target? {
        let target = question + 1
        guard count > 1, (0..<count).contains(question), target < count, let frame = viewport.cards[target] else { return nil }
        let belowTheTop = frame.minY >= viewport.top - tolerance
        let aboveTheBottom = frame.maxY <= viewport.bottom + tolerance
        if belowTheTop, aboveTheBottom { return Target(question: target, anchor: .stay) }
        // Its bottom is on screen and its top is cut off: show it from the top.
        if aboveTheBottom { return Target(question: target, anchor: .top) }
        return lead(to: target, frame, in: viewport, margin: margin)
    }

    /// How to bring a question whose bottom is below the visible area into view: all of it, lined up
    /// with the bottom, when it fits; else its top, and once the top is at the edge its bottom.
    private static func lead(to question: Int, _ frame: CGRect, in viewport: Viewport, margin: CGFloat) -> Target {
        if fits(frame, in: viewport, margin: margin) { return Target(question: question, anchor: .bottom) }
        return Target(question: question, anchor: frame.minY > viewport.top + tolerance ? .top : .bottom)
    }

    /// How to bring a given question into view (to point at it, as the hint of a select-only form
    /// does), or `nil` when all of it is on screen already or it is not known where it is.
    ///
    /// A question above the screen is shown from its top; one below it from its bottom when it fits.
    public static func reveal(_ question: Int, in viewport: Viewport, margin: CGFloat = 16) -> Target? {
        guard let frame = viewport.cards[question] else { return nil }
        let onScreen = frame.minY >= viewport.top - tolerance && frame.maxY <= viewport.bottom + tolerance
        if onScreen { return nil }
        if frame.minY < viewport.top - tolerance { return Target(question: question, anchor: .top) }
        return Target(question: question, anchor: fits(frame, in: viewport, margin: margin) ? .bottom : .top)
    }

    /// Whether the whole question fits in the visible area, with a gap below it.
    private static func fits(_ frame: CGRect, in viewport: Viewport, margin: CGFloat) -> Bool {
        frame.height <= viewport.height - margin
    }
}
