import SwiftUI
import UIKit

// A row that is swiped sideways to show the buttons under it (docs/specs/todo-list-actions.md D12, §6.1). The todo list of
// a request page lives in a `ScrollView` among cards, where a `List`'s own swipe actions are not to be had, so this is the
// same thing made of a drag: right shows "Complete" (all the way across does it), left shows "Delete" (it never deletes
// by itself).
//
// The drag has to share the finger with three other things: the page's vertical scroll (the drag only takes over when the
// first movement is clearly sideways, and the page stops scrolling for as long as it holds), the row's own tap (a tap is
// not a drag), and the edge swipe that goes back (a drag that starts at the very edge of the screen is left alone).

/// One button under a row.
struct SwipeRowAction {
    let title: LocalizedStringKey
    let symbol: String
    let tint: Color
    /// Of the button that shows under the row (the UI tests look for it).
    let identifier: String
    /// A swipe all the way across carries the action out by itself.
    var fullSwipe = false
    let perform: () -> Void
}

/// What the rows of a page agree on: at most one is open, and the page does not scroll while one is being dragged.
@MainActor
@Observable
final class SwipeCoordinator {
    /// The row that is open (its button shows) or being dragged.
    private(set) var openId: String?
    /// A row is being dragged sideways.
    private(set) var isSwiping = false
    /// When the last sideways drag let go.
    @ObservationIgnored private var lastEnd: ContinuousClock.Instant?

    func began(_ id: String) {
        openId = id
        isSwiping = true
    }

    func ended() {
        isSwiping = false
        lastEnd = .now
    }

    /// The finger that is lifting (or just lifted) was dragging a row sideways. A `Button` fires when a touch ends on it,
    /// however far the finger took the row along, so the row's own tap has to ask: a swipe is not a pick.
    var swallowsTap: Bool {
        isSwiping || (lastEnd.map { $0.duration(to: .now) < .milliseconds(350) } ?? false)
    }

    func opened(_ id: String) { openId = id }

    func closed(_ id: String) {
        if openId == id { openId = nil }
    }
}

/// Keeps a scroll view still while a row is dragged sideways: the page would follow the vertical part of the finger.
struct SwipeScrollLock: ViewModifier {
    let coordinator: SwipeCoordinator

    func body(content: Content) -> some View {
        content.scrollDisabled(coordinator.isSwiping)
    }
}

struct SwipeActionRow<Content: View>: View {
    /// What the row is known by to the page's coordinator.
    let id: String
    /// Whether the row can be swiped (a disabled form, or a todo the Mac cannot be asked about).
    var isEnabled = true
    var leading: SwipeRowAction?
    var trailing: SwipeRowAction?
    @ViewBuilder let content: () -> Content

    @Environment(SwipeCoordinator.self) private var coordinator: SwipeCoordinator?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// A form that is disabled (an answer on its way, a request that ended) does not stop a gesture by itself.
    @Environment(\.isEnabled) private var formEnabled

    /// Where the row is: 0 at rest, positive when it was swiped right (the leading button shows).
    @State private var offset: CGFloat = 0
    /// Where the row was when the finger came down.
    @State private var origin: CGFloat = 0
    /// What the finger is doing, decided by its first movement and kept until it lifts.
    @State private var axis: Axis?
    @State private var width: CGFloat = 0
    /// The row is far enough across that letting go carries out the leading action.
    @State private var armed = false
    @State private var ticks = 0
    @State private var finishes = 0
    @GestureState private var touching = false

    /// How much of the row a button takes when it is shown and left alone.
    private let reveal: CGFloat = 84
    private let corner: CGFloat = 12
    /// Screen points from the left edge that belong to the back gesture.
    private let backEdge: CGFloat = 20

    private var fullThreshold: CGFloat { max(reveal * 1.8, width * 0.45) }

    private var swipeable: Bool { isEnabled && formEnabled }

    var body: some View {
        content()
            .overlay {
                // A tap on a row that is open closes it, and is not the row's own tap.
                if offset != 0 {
                    Color.clear
                        .contentShape(Rectangle())
                        .onTapGesture { settle(0) }
                }
            }
            .offset(x: offset)
            // The buttons sit behind the row and take its height; they are never part of its layout, so a row that is
            // swiped open is as tall as it was and the rows under it stay where they are.
            .background { tiles }
            .clipShape(RoundedRectangle(cornerRadius: corner, style: .continuous))
        .background {
            GeometryReader { proxy in
                Color.clear
                    .onAppear { width = proxy.size.width }
                    .onChange(of: proxy.size.width) { _, new in width = new }
            }
        }
        .simultaneousGesture(drag, including: swipeable ? .all : .subviews)
        .sensoryFeedback(.impact(weight: .light), trigger: ticks)
        .sensoryFeedback(.success, trigger: finishes)
        .onChange(of: coordinator?.openId) { _, open in
            // Another row was touched: this one closes.
            if open != id, offset != 0, axis == nil { settle(0) }
        }
        .onChange(of: swipeable) { _, now in
            if !now, offset != 0 { settle(0) }
        }
        .onChange(of: touching) { _, now in
            // The finger went without the drag ending (the page's scroll, or the system, took it): rest where it is nearest.
            if !now, let held = axis {
                axis = nil
                if held == .horizontal {
                    coordinator?.ended()
                    armed = false
                    release(projected: offset)
                }
            }
        }
        .onDisappear {
            if axis == .horizontal { coordinator?.ended() }
            coordinator?.closed(id)
        }
    }

    // MARK: the buttons under the row

    @ViewBuilder
    private var tiles: some View {
        if offset > 0, let leading {
            tile(leading, alignment: .leading, extent: offset)
        } else if offset < 0, let trailing {
            tile(trailing, alignment: .trailing, extent: -offset)
        }
    }

    private func tile(_ action: SwipeRowAction, alignment: Alignment, extent: CGFloat) -> some View {
        Button {
            runOpen(action)
        } label: {
            Rectangle()
                .fill(action.tint)
                .frame(width: extent)
                .overlay(alignment: alignment) {
                    VStack(spacing: 3) {
                        Image(systemName: action.symbol)
                            .font(.body.weight(.semibold))
                            .scaleEffect(armed && action.fullSwipe ? 1.2 : 1)
                            .animation(.snappy(duration: 0.15), value: armed)
                        Text(action.title)
                            .font(.caption.weight(.semibold))
                            .lineLimit(1)
                    }
                    .foregroundStyle(.white)
                    // The label keeps its place as the button grows and takes the middle of it once it is wider than the label.
                    .frame(width: max(reveal, extent))
                }
                .clipped()
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(action.identifier)
        .frame(maxWidth: .infinity, alignment: alignment)
    }

    // MARK: the drag

    private var drag: some Gesture {
        DragGesture(minimumDistance: 12, coordinateSpace: .global)
            .updating($touching) { _, state, _ in state = true }
            .onChanged { value in
                if axis == nil {
                    let across = abs(value.translation.width)
                    let along = abs(value.translation.height)
                    // Only a clearly sideways start is the row's; one that starts at the screen's edge is the back gesture's.
                    if across > along * 1.2, value.startLocation.x > backEdge {
                        axis = .horizontal
                        origin = offset
                        coordinator?.began(id)
                    } else {
                        axis = .vertical
                    }
                }
                guard axis == .horizontal else { return }
                follow(origin + value.translation.width)
            }
            .onEnded { value in
                let held = axis
                axis = nil
                guard held == .horizontal else { return }
                coordinator?.ended()
                release(projected: origin + value.predictedEndTranslation.width)
            }
    }

    /// The row goes where the finger goes, held back where there is nothing to show.
    private func follow(_ proposed: CGFloat) {
        let x = clamped(proposed)
        withTransaction(Transaction(animation: nil)) { offset = x }
        let was = armed
        armed = leading?.fullSwipe == true && x >= fullThreshold
        if armed != was { ticks += 1 }
    }

    private func clamped(_ x: CGFloat) -> CGFloat {
        if x > 0 {
            guard let leading else { return 0 }
            if leading.fullSwipe { return min(x, width) }
            return x > reveal ? reveal + (x - reveal) * 0.3 : x
        }
        if x < 0 {
            guard trailing != nil else { return 0 }
            return -x > reveal ? -(reveal + (-x - reveal) * 0.3) : x
        }
        return 0
    }

    /// The finger lifted: the leading action is carried out when the row was swiped all the way across, otherwise the
    /// button stays open when the row was taken far enough, and the row closes when it was not.
    private func release(projected: CGFloat) {
        if armed, let leading {
            commit(leading)
            return
        }
        if offset > 0, leading != nil {
            settle(offset > reveal * 0.45 || projected > reveal * 1.1 ? reveal : 0)
        } else if offset < 0, trailing != nil {
            settle(-offset > reveal * 0.45 || -projected > reveal * 1.1 ? -reveal : 0)
        } else {
            settle(0)
        }
    }

    private func settle(_ target: CGFloat) {
        armed = false
        withAnimation(reduceMotion ? .easeOut(duration: 0.12) : .snappy(duration: 0.28)) { offset = target }
        if target == 0 { coordinator?.closed(id) } else { coordinator?.opened(id) }
    }

    /// Swiped all the way across: the row leaves the way it was going, and then the action is carried out. A row that is
    /// still there afterwards (the action was refused) comes back.
    private func commit(_ action: SwipeRowAction) {
        armed = false
        finishes += 1
        let distance = max(width, reveal)
        withAnimation(.easeOut(duration: reduceMotion ? 0.01 : 0.16)) {
            offset = distance
        } completion: {
            action.perform()
            withAnimation(.snappy(duration: 0.28)) { offset = 0 }
            coordinator?.closed(id)
        }
    }

    /// A button that was left showing was pressed.
    private func runOpen(_ action: SwipeRowAction) {
        finishes += 1
        action.perform()
        settle(0)
    }
}
