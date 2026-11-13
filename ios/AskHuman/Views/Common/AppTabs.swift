import SwiftUI
import UIKit
import AHCore

/// Keeps tab and navigation ownership inside one UIKit container across column/stack transitions.
@available(iOS 27.1, *)
struct AppTabs<Content: View, Overlay: View>: UIViewControllerRepresentable {
    @Binding var selection: AppTab
    let pendingCount: Int
    let isOverlayInteractive: Bool
    let content: (AppTab) -> Content
    let overlay: () -> Overlay

    func makeCoordinator() -> Coordinator { Coordinator(selection: $selection) }

    func makeUIViewController(context: Context) -> AppTabContainerController {
        let host = AppTabContainerController()
        let banner = UIHostingController(rootView: overlay())
        banner.sizingOptions = .intrinsicContentSize
        host.overlay = banner
        host.tabs.mode = .tabBar
        host.tabs.tabs = AppTab.allCases.map { tab in
            let page = UIHostingController(rootView: content(tab))
            let item = UITab(title: tab.title, image: UIImage(systemName: tab.symbol), identifier: tab.rawValue) { _ in page }
            context.coordinator.tabs[tab] = item
            return item
        }
        synchronize(host, context: context)
        return host
    }

    func updateUIViewController(_ host: AppTabContainerController, context: Context) {
        synchronize(host, context: context)
    }

    private func synchronize(_ host: AppTabContainerController, context: Context) {
        let controller = host.tabs
        let coordinator = context.coordinator
        coordinator.selection = $selection
        // Programmatic synchronization must not become a user selection or sidebar reveal request.
        controller.delegate = nil
        defer { controller.delegate = coordinator }
        for tab in AppTab.allCases {
            if let item = coordinator.tabs[tab], item.title != tab.title { item.title = tab.title }
        }
        let badge = pendingCount > 0 ? String(pendingCount) : nil
        if let inbox = coordinator.tabs[.inbox], inbox.badgeValue != badge {
            inbox.badgeValue = badge
        }
        if controller.selectedTab?.identifier != selection.rawValue {
            controller.selectedTab = coordinator.tabs[selection]
        }
        host.isOverlayInteractive = isOverlayInteractive
    }

    final class Coordinator: NSObject, UITabBarControllerDelegate {
        var selection: Binding<AppTab>
        var tabs: [AppTab: UITab] = [:]

        init(selection: Binding<AppTab>) { self.selection = selection }

        func tabBarController(_ tabBarController: UITabBarController, shouldSelectTab tab: UITab) -> Bool {
            if tabBarController.selectedTab === tab, let id = AppTab(rawValue: tab.identifier) {
                selection.wrappedValue = id
            }
            return true
        }

        func tabBarController(_ tabBarController: UITabBarController, didSelectTab selectedTab: UITab, previousTab: UITab?) {
            guard let id = AppTab(rawValue: selectedTab.identifier), id != selection.wrappedValue else { return }
            selection.wrappedValue = id
        }
    }
}

/// Provides UIKit containment before the tab controller participates in SwiftUI's view hierarchy.
@available(iOS 27.1, *)
final class AppTabContainerController: UIViewController {
    let tabs = UITabBarController()
    var overlay: UIViewController?
    var isOverlayInteractive = false {
        didSet { overlay?.viewIfLoaded?.isUserInteractionEnabled = isOverlayInteractive }
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        addChild(tabs)
        view.addSubview(tabs.view)
        tabs.view.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            tabs.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            tabs.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            tabs.view.topAnchor.constraint(equalTo: view.topAnchor),
            tabs.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        tabs.didMove(toParent: self)
        if let overlay {
            // Keep banners beside the tab controller, inside the same bar-owning container.
            // The intrinsic height is zero when empty, leaving the page's touch routing intact.
            addChild(overlay)
            overlay.view.backgroundColor = .clear
            overlay.view.isUserInteractionEnabled = isOverlayInteractive
            view.addSubview(overlay.view)
            overlay.view.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([
                overlay.view.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
                overlay.view.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
                overlay.view.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            ])
            overlay.didMove(toParent: self)
        }
    }

    override var childForPreferredVerticalBarBehavior: UIViewController? { tabs }
}

extension View {
    /// Where the system lays the tab bar and a page's toolbar buttons out in one vertical strip (the outer
    /// screen of a folding phone) and the strip is too short for both, the tab bar gives way: it folds into
    /// the button of the tab that is open, which shows the whole bar again when it is tapped, and the page
    /// keeps all of its buttons. The system's own choice is the other way round: it moves the buttons into
    /// an overflow menu and leaves the five tabs where they were.
    ///
    /// Put it on the root content of a tab's `NavigationStack`, after its `navigationDestination`s: the
    /// pages pushed over that root are covered by it. Outside the stack (the tab, the stack itself) it is
    /// not heard.
    @ViewBuilder
    func prefersToolbarItemsOverTabBar() -> some View {
        if #available(iOS 27.1, *) {
            toolbarVerticalCompressionBehavior(.prefersToolbarItems)
        } else {
            self
        }
    }
}

extension AppTab {
    var title: String {
        switch self {
        case .inbox: String(localized: "Inbox")
        case .agents: String(localized: "Agents")
        case .todos: String(localized: "Todos")
        case .history: String(localized: "History")
        case .macs: String(localized: "Macs")
        }
    }

    var symbol: String {
        switch self {
        case .inbox: "tray.full"
        case .agents: "person.2.wave.2"
        case .todos: "checklist"
        case .history: "clock.arrow.circlepath"
        case .macs: "desktopcomputer"
        }
    }
}
