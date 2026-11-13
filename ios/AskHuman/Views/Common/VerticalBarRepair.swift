import UIKit
import AHCore

/// Puts the back button of a pushed page back into the vertical strip when the system left it out.
///
/// What goes wrong (iOS 27.x on the Duo, seen on the simulator and on a device): the inner screen is held upright, so the
/// system lays its bars out horizontally, and the phone folds. SwiftUI swaps the split view for a new `NavigationStack`
/// in the middle of that change, and the strip of the outer screen is built for a navigation controller that has only
/// just been created. The page's buttons are declared (the navigation item has them, the vertical bar edge is the
/// strip's) but never reach the strip: it shows the tabs and nothing else, and the page has no way back. It stays like
/// that until something makes the system register the navigation item again. In the simulator about half of the folds
/// from the upright screen end this way; folds from the sideways screen, or right after the turn, never do.
///
/// Changing the navigation item registers it again, but only once the strip has settled (about two seconds after the
/// fold in the simulator; tries before that do nothing). So after each change of the container's size this looks at the
/// strip a few times and, as long as a pushed page has a back button the strip does not show, flips `hidesBackButton`
/// for one turn of the run loop. When the strip is fine, which is almost always, nothing is touched.
///
/// Things that look like they should work and do not: `setNeedsUpdateOfVerticalBarConfiguration`, assigning the same item
/// groups again, `setViewControllers` with the same stack, laying out again and hiding the tab bar for a moment. Giving
/// the navigation item new `UIBarButtonItemGroup`s that hold the same items does bring the buttons back but crashes soon
/// after: SwiftUI hands over its own subclass of the group, keeps books on the items it put in, and its next toolbar
/// update traps on them. Do not replace SwiftUI's groups.
@available(iOS 27.1, *)
@MainActor
final class VerticalBarRepair {
    private weak var container: UIViewController?
    private weak var root: UIViewController?
    private var lastSize = CGSize.zero
    private var pending: DispatchWorkItem?
    private var tries = 0

    /// Seconds after the last change of size at which the strip is looked at.
    private static let checks: [TimeInterval] = [0.5, 1, 1.5, 2, 2.5, 3, 4, 5, 6.5, 8]
    /// How many views one look at the strip goes through at most.
    private static let viewLimit = 4000

    /// `container` owns the strip; the navigation controllers of the pages are below `root`.
    init(container: UIViewController, root: UIViewController) {
        self.container = container
        self.root = root
    }

    /// Call whenever the container has laid out its views.
    func containerDidLayout() {
        guard let size = container?.view.bounds.size, size != lastSize else { return }
        lastSize = size
        pending?.cancel()
        tries = 0
        schedule(0, since: .now())
    }

    private func schedule(_ index: Int, since start: DispatchTime) {
        guard index < Self.checks.count else {
            if tries > 0 {
                Diagnostics.note("bars", "still no back button in the strip \(Self.checks[index - 1]) s after a change of size (\(tries) tries)")
            }
            return
        }
        let work = DispatchWorkItem { [weak self] in self?.check(index, since: start) }
        pending = work
        DispatchQueue.main.asyncAfter(deadline: start + Self.checks[index], execute: work)
    }

    private func check(_ index: Int, since start: DispatchTime) {
        let stranded = itemsWithoutBackButton()
        if stranded.isEmpty {
            if tries > 0 {
                Diagnostics.note("bars", "the strip has its back button again \(Self.checks[index]) s after a change of size (\(tries) tries)")
                tries = 0
            }
        } else {
            tries += 1
            for item in stranded {
                let hides = item.hidesBackButton
                item.hidesBackButton = !hides
                DispatchQueue.main.async { item.hidesBackButton = hides }
            }
            Diagnostics.note("bars", "no back button in the strip \(Self.checks[index]) s after a change of size; registered \(stranded.count) navigation item(s) again (try \(tries))")
        }
        // Keep looking until the end of the schedule even when the strip is fine: it can lose the buttons late in the transition.
        schedule(index + 1, since: start)
    }

    /// The navigation items of the pages on screen that are pushed and keep their back button, while the strip is there and
    /// shows no button at all. Empty when the strip is fine or its views are not found the way they are expected.
    private func itemsWithoutBackButton() -> [UINavigationItem] {
        guard let container, let root, container.view.window != nil,
              container.traitCollection.verticalBarEdge != .unspecified else { return [] }
        var items: [UINavigationItem] = []
        func visit(_ controller: UIViewController) {
            if let navigation = controller as? UINavigationController, navigation.isViewLoaded, navigation.view.window != nil,
               navigation.viewControllers.count > 1, let item = navigation.topViewController?.navigationItem,
               !item.hidesBackButton {
                items.append(item)
            }
            controller.children.forEach(visit)
        }
        visit(root)
        guard !items.isEmpty, shownBarButtons(in: container.view) == 0 else { return [] }
        return items
    }

    // MARK: the views of the strip

    private enum Kind { case page, bar, button, other }

    private static var kinds: [ObjectIdentifier: Kind] = [:]

    /// The system's views are told apart by the names of their classes. A name that changes makes the strip look
    /// unfindable, never empty, so the worst that can happen is that nothing is repaired.
    private static func kind(of view: UIView) -> Kind {
        let metatype = type(of: view)
        if let known = kinds[ObjectIdentifier(metatype)] { return known }
        let name = String(describing: metatype)
        let made: Kind =
            name == "HostingView" ? .page
            : name.hasPrefix("_UIFloatingBarContainerView") ? .bar
            : name.hasPrefix("_UIButtonBarButton") ? .button
            : .other
        kinds[ObjectIdentifier(metatype)] = made
        return made
    }

    /// How many bar buttons the floating bars below `view` show; `nil` when there is no floating bar to look at. The inside of
    /// a page (SwiftUI's own view) holds no bars and is not entered.
    private func shownBarButtons(in view: UIView) -> Int? {
        var bars = 0, shown = 0, looked = 0
        func visit(_ node: UIView, insideBar: Bool, visible: Bool) {
            for child in node.subviews {
                looked += 1
                if looked > Self.viewLimit { return }
                let kind = Self.kind(of: child)
                if kind == .page { continue }
                let isVisible = visible && !child.isHidden && child.alpha > 0.01
                if kind == .bar { bars += 1 }
                if kind == .button, insideBar, isVisible { shown += 1 }
                visit(child, insideBar: insideBar || kind == .bar, visible: isVisible)
            }
        }
        visit(view, insideBar: false, visible: true)
        return bars > 0 ? shown : nil
    }
}
