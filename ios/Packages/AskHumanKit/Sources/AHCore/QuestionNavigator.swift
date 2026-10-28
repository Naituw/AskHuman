import CoreGraphics

/// Where the "Next" button of a form of several questions leads, worked out from what is on screen.
///
/// The Mac popup moves a "current question" pointer one step down and brings that question into view
/// (spec `multi-question-vertical.md` N1–N4). On the phone the pointer is implicit: it is where the
/// screen is. The target is the first question whose bottom is not on screen yet, which is the one
/// after the last question the person has reached, however far down they went before and whatever was
/// seen or answered on the way. (An earlier version led to "the first question not read yet", which
/// jumped over the questions that were looked at but not answered once the person had scrolled back.)
public enum QuestionNavigator {
    /// Which edge of the visible area the question is lined up with.
    public enum Anchor: Equatable, Sendable {
        case top
        case bottom
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
            if fits(frame, in: viewport, margin: margin) { return Target(question: question, anchor: .bottom) }
            return Target(question: question, anchor: frame.minY > viewport.top + tolerance ? .top : .bottom)
        }
        return nil
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
