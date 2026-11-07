import CoreGraphics
import Foundation

// Where the floating developer button sits (docs/specs/ios-developer-mode.md §5). Only numbers and
// rectangles: the views feed in what the window looks like and draw what comes out, so all of it is tested
// without a screen. No UIKit here (this target builds on macOS as well).

/// The size of the floating button and the times around it, in one place so that the views and the tests agree.
public enum DeveloperMetrics {
    /// Diameter of the button.
    public static let buttonSize: CGFloat = 44
    public static var buttonRadius: CGFloat { buttonSize / 2 }
    /// Gap between the button and the edge of the screen once it has snapped.
    public static let edgeMargin: CGFloat = 6
    /// Gap kept between the button and the top and the bottom of the safe area.
    public static let verticalMargin: CGFloat = 8
    /// The button dims to `idleAlpha` after this long without a touch.
    public static let idleDelay: TimeInterval = 2.5
    public static let idleAlpha: CGFloat = 0.5
    /// How long the note about a finished action stays on screen.
    public static let toastDuration: TimeInterval = 1.8
}

/// Distances from the edges of the window to its safe area.
public struct DeveloperInsets: Equatable, Sendable {
    public var top: CGFloat
    public var left: CGFloat
    public var bottom: CGFloat
    public var right: CGFloat

    public init(top: CGFloat = 0, left: CGFloat = 0, bottom: CGFloat = 0, right: CGFloat = 0) {
        self.top = top
        self.left = left
        self.bottom = bottom
        self.right = right
    }

    public static let zero = DeveloperInsets()
}

/// What the window looks like right now.
public struct DeveloperLayout: Equatable, Sendable {
    public var size: CGSize
    public var safeArea: DeveloperInsets
    /// The top of the keyboard in the window's coordinates, while it is up.
    public var keyboardTop: CGFloat?

    public init(size: CGSize, safeArea: DeveloperInsets = .zero, keyboardTop: CGFloat? = nil) {
        self.size = size
        self.safeArea = safeArea
        self.keyboardTop = keyboardTop
    }
}

/// The side of the screen the button rests on. Physical sides, not reading direction: the button does not
/// move to the other side in a right-to-left language.
public enum DeveloperEdge: String, Codable, Sendable {
    case left
    case right
}

/// Where the user left the button: a side and a height. It is what is remembered; the point on the screen
/// follows from it and the window's layout.
public struct DeveloperPlacement: Codable, Equatable, Sendable {
    public var edge: DeveloperEdge
    /// Where the button sits on the free height of the safe area: 0 is the top, 1 the bottom.
    public var y: Double

    public static let standard = DeveloperPlacement(edge: .right, y: 0.4)

    public init(edge: DeveloperEdge, y: Double) {
        self.edge = edge
        self.y = Self.clamped(y)
    }

    private enum CodingKeys: String, CodingKey { case edge, y }

    /// Stored data that is out of range is pulled back in; data that is not readable at all throws.
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        self.init(edge: try values.decode(DeveloperEdge.self, forKey: .edge), y: try values.decode(Double.self, forKey: .y))
    }

    private static func clamped(_ y: Double) -> Double {
        y.isFinite ? min(1, max(0, y)) : standard.y
    }
}

extension DeveloperPlacement {
    /// The heights the button's center can have: the safe area, less the margins and the button itself.
    public static func verticalRange(in layout: DeveloperLayout) -> ClosedRange<CGFloat> {
        let low = layout.safeArea.top + DeveloperMetrics.verticalMargin + DeveloperMetrics.buttonRadius
        let high = layout.size.height - layout.safeArea.bottom - DeveloperMetrics.verticalMargin - DeveloperMetrics.buttonRadius
        if low <= high { return low...high }
        let middle = (low + high) / 2
        return middle...middle
    }

    /// The x of the button's center once it has snapped to `edge`.
    public static func edgeX(_ edge: DeveloperEdge, in layout: DeveloperLayout) -> CGFloat {
        switch edge {
        case .left:
            return layout.safeArea.left + DeveloperMetrics.edgeMargin + DeveloperMetrics.buttonRadius
        case .right:
            return layout.size.width - layout.safeArea.right - DeveloperMetrics.edgeMargin - DeveloperMetrics.buttonRadius
        }
    }

    /// Where the button rests when nothing gets in its way.
    public func restingCenter(in layout: DeveloperLayout) -> CGPoint {
        let range = Self.verticalRange(in: layout)
        let y = range.lowerBound + CGFloat(self.y) * (range.upperBound - range.lowerBound)
        return CGPoint(x: Self.edgeX(edge, in: layout), y: y)
    }

    /// Where the button is drawn: at rest, or just above the keyboard when the keyboard would cover it.
    /// The remembered placement does not change, so the button goes back when the keyboard does.
    public func displayedCenter(in layout: DeveloperLayout) -> CGPoint {
        var center = restingCenter(in: layout)
        if let keyboardTop = layout.keyboardTop {
            let limit = keyboardTop - DeveloperMetrics.verticalMargin - DeveloperMetrics.buttonRadius
            if center.y > limit { center.y = max(Self.verticalRange(in: layout).lowerBound, limit) }
        }
        return center
    }

    /// Where the button goes when the finger lets go with its center at `center`: the nearest side, the
    /// height it was let go at.
    public static func snapped(from center: CGPoint, in layout: DeveloperLayout) -> DeveloperPlacement {
        let range = verticalRange(in: layout)
        let span = range.upperBound - range.lowerBound
        let clamped = min(max(center.y, range.lowerBound), range.upperBound)
        let fraction = span > 0 ? Double((clamped - range.lowerBound) / span) : standard.y
        return DeveloperPlacement(edge: center.x < layout.size.width / 2 ? .left : .right, y: fraction)
    }

    /// Where the button's center may be while it is dragged: anywhere the button rests, and between.
    public static func clampedDragCenter(_ proposed: CGPoint, in layout: DeveloperLayout) -> CGPoint {
        let range = verticalRange(in: layout)
        let left = edgeX(.left, in: layout)
        let right = max(left, edgeX(.right, in: layout))
        return CGPoint(x: min(max(proposed.x, left), right), y: min(max(proposed.y, range.lowerBound), range.upperBound))
    }
}

// MARK: menu

/// The things the developer menu offers: the structure is what makes more of them cheap. The menu itself is the
/// system's (`UIMenu` on the button): this is only what it lists, in the order it lists it.
public enum DeveloperMenuItem: String, CaseIterable, Identifiable, Sendable {
    case quickTodo
    /// Puts what `Diagnostics` has on the pasteboard (docs/specs/ios-developer-mode.md §12).
    case copyDiagnostics

    public var id: String { rawValue }
}
