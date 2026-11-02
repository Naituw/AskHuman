import CoreGraphics
import Foundation

/// How long the finger must rest, and how far it may stray while it does.
public enum DeveloperPressTuning {
    /// A finger that stays on the button this long opens the menu.
    public static let menuDelay: TimeInterval = 0.12
    /// A finger that moves farther than this before the menu opens drags the button instead.
    public static let moveSlop: CGFloat = 10
}

/// What one finger does on the floating button (docs/specs/ios-developer-mode.md §5.2). The same finger
/// has to be able to open the menu the moment it lands and to drag the button, which cannot both be
/// instant, so it is decided by whether the finger moves:
///
/// - it moves right away: it drags the button, and no menu opens in this touch;
/// - it rests for `menuDelay`: the menu opens and the touch can no longer drag — the finger slides to a
///   row and lets go to choose it;
/// - it lets go before that: the menu opens and stays, to be tapped.
///
/// This is only the decision. It owns no timer (the caller arms one when told to and calls
/// `menuTimerFired()`), draws nothing and knows nothing of UIKit: it turns touches into effects, and the
/// view carries those out in order. Points are in the window of the button.
public struct DeveloperPress: Equatable, Sendable {
    public enum Phase: Equatable, Sendable {
        /// Nothing is showing and no finger is down.
        case idle
        /// The finger is on the button and has not decided yet. When it was the touch that closed the
        /// open menu, letting go quickly leaves the menu closed.
        case pending(quickReleaseOpens: Bool)
        /// The menu is showing and the finger is down: it chooses a row by being let go on it.
        case menuHeld
        /// The menu is showing and no finger is down.
        case menuOpen
        case dragging
        /// A touch that closed the menu from outside: it is used up, and the rest of it does nothing.
        case swallowing
    }

    public enum Haptic: Equatable, Sendable {
        case opened
        case changed
        case invoked
    }

    public enum Effect: Equatable, Sendable {
        case scheduleMenuTimer(after: TimeInterval)
        case cancelMenuTimer
        case showMenu
        case hideMenu
        case highlight(DeveloperMenuItem?)
        case beginDrag
        /// The button's center follows the finger, keeping the offset it was grabbed at.
        case drag(to: CGPoint)
        /// Let go while dragging: the button snaps from here.
        case endDrag(at: CGPoint)
        case invoke(DeveloperMenuItem)
        case haptic(Haptic)
        /// The touch was taken away: the button goes back to where it was before it began.
        case restorePosition
    }

    public private(set) var phase: Phase = .idle
    public private(set) var highlighted: DeveloperMenuItem?
    private var origin = CGPoint.zero
    private var grabOffset = CGSize.zero

    public init() {}

    /// The finger landed. `hits` says what is where right now.
    public mutating func down(at point: CGPoint, hits: DeveloperHitMap) -> [Effect] {
        var effects: [Effect] = []
        switch phase {
        case .idle, .menuOpen: break
        default: effects += cancel()  // a second finger, or the end of the last touch got lost
        }

        if phase == .menuOpen {
            if let item = hits.item(at: point) {
                phase = .menuHeld
                highlighted = item
                return effects + [.highlight(item), .haptic(.changed)]
            }
            guard hits.isOnButton(point) else {
                phase = .swallowing
                return effects + [.hideMenu]
            }
            // The button again: the menu closes, and this touch is a new one that can drag.
            begin(at: point, hits: hits, quickReleaseOpens: false)
            return effects + [.hideMenu, .scheduleMenuTimer(after: DeveloperPressTuning.menuDelay)]
        }

        guard hits.isOnButton(point) else { return effects }
        begin(at: point, hits: hits, quickReleaseOpens: true)
        return effects + [.scheduleMenuTimer(after: DeveloperPressTuning.menuDelay)]
    }

    public mutating func move(to point: CGPoint, hits: DeveloperHitMap) -> [Effect] {
        switch phase {
        case .pending:
            guard hypot(point.x - origin.x, point.y - origin.y) > DeveloperPressTuning.moveSlop else { return [] }
            phase = .dragging
            return [.cancelMenuTimer, .beginDrag, .drag(to: followed(point))]
        case .dragging:
            return [.drag(to: followed(point))]
        case .menuHeld:
            let item = hits.item(at: point)
            guard item != highlighted else { return [] }
            highlighted = item
            return item == nil ? [.highlight(nil)] : [.highlight(item), .haptic(.changed)]
        default:
            return []
        }
    }

    /// The timer armed by a `scheduleMenuTimer` ran out. One that ran out after the touch had moved on is
    /// ignored.
    public mutating func menuTimerFired() -> [Effect] {
        guard case .pending = phase else { return [] }
        phase = .menuHeld
        highlighted = nil
        return [.showMenu, .haptic(.opened)]
    }

    public mutating func up(at point: CGPoint, hits: DeveloperHitMap) -> [Effect] {
        switch phase {
        case .pending(let quickReleaseOpens):
            if quickReleaseOpens {
                phase = .menuOpen
                return [.cancelMenuTimer, .showMenu, .haptic(.opened)]
            }
            phase = .idle
            return [.cancelMenuTimer]
        case .menuHeld:
            let chosen = hits.item(at: point)
            let cleared: [Effect] = highlighted == nil ? [] : [.highlight(nil)]
            highlighted = nil
            if let chosen {
                phase = .idle
                return [.hideMenu, .haptic(.invoked), .invoke(chosen)]
            }
            if hits.isOnButton(point) {
                phase = .menuOpen
                return cleared
            }
            phase = .idle
            return cleared + [.hideMenu]
        case .dragging:
            phase = .idle
            return [.endDrag(at: followed(point))]
        case .swallowing:
            phase = .idle
            return []
        case .idle, .menuOpen:
            return []
        }
    }

    /// The system took the touch away (a call, a notification, the app losing the screen), or the menu has
    /// to go for some other reason: whatever is showing closes and the button stays where it was.
    public mutating func cancel() -> [Effect] {
        let before = phase
        phase = .idle
        highlighted = nil
        switch before {
        case .idle, .swallowing: return []
        case .pending: return [.cancelMenuTimer]
        case .menuHeld, .menuOpen: return [.hideMenu]
        case .dragging: return [.restorePosition]
        }
    }

    private mutating func begin(at point: CGPoint, hits: DeveloperHitMap, quickReleaseOpens: Bool) {
        origin = point
        grabOffset = CGSize(width: point.x - hits.button.midX, height: point.y - hits.button.midY)
        highlighted = nil
        phase = .pending(quickReleaseOpens: quickReleaseOpens)
    }

    private func followed(_ point: CGPoint) -> CGPoint {
        CGPoint(x: point.x - grabOffset.width, y: point.y - grabOffset.height)
    }
}
