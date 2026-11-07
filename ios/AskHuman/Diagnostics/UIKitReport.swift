import UIKit
import os
import AHCore

/// What the diagnostics read off UIKit (docs/specs/ios-developer-mode.md §12): the device, the scenes and the windows
/// in them, the view controllers that hold the screens, and the bars along their edges.
///
/// It only looks. No view is loaded or laid out on its behalf, and nothing a person wrote comes out of it: the words
/// here are classes, frames, flags and counts (a title is "has one, N characters", never the title).
@MainActor
enum UIKitReport {
    // MARK: device and app

    static func device() -> String {
        let info = Bundle.main.infoDictionary ?? [:]
        func text(_ key: String) -> String { info[key] as? String ?? "?" }
        let process = ProcessInfo.processInfo
        let app = UIApplication.shared
        var lines = [
            "app \(text("CFBundleShortVersionString")) (\(text("CFBundleVersion"))), \(Bundle.main.bundleIdentifier ?? "?")",
            "built with Xcode \(text("DTXcode")) (\(text("DTXcodeBuild"))), SDK \(text("DTSDKName")) (\(text("DTSDKBuild"))), minimum iOS \(text("MinimumOSVersion"))",
            "device \(machine()), iOS \(UIDevice.current.systemVersion) (\(systemControl("kern.osversion") ?? "?")), idiom \(idiom(UIDevice.current.userInterfaceIdiom))",
            "application \(applicationState(app.applicationState)); protected data \(yesNo(app.isProtectedDataAvailable)); background refresh \(backgroundRefresh(app.backgroundRefreshStatus))",
            "low power \(yesNo(process.isLowPowerModeEnabled)); thermal \(thermal(process.thermalState)); memory left \(memoryLeft())",
            "content size \(app.preferredContentSizeCategory.rawValue.replacingOccurrences(of: "UICTContentSizeCategory", with: "")); language \(Bundle.main.preferredLocalizations.first ?? "?"); layout \(app.userInterfaceLayoutDirection == .leftToRight ? "left-to-right" : "right-to-left")",
            "reduce motion \(yesNo(UIAccessibility.isReduceMotionEnabled)); reduce transparency \(yesNo(UIAccessibility.isReduceTransparencyEnabled)); bold text \(yesNo(UIAccessibility.isBoldTextEnabled)); VoiceOver \(yesNo(UIAccessibility.isVoiceOverRunning)); switch control \(yesNo(UIAccessibility.isSwitchControlRunning))",
        ]
        let simulator = process.environment["SIMULATOR_MODEL_IDENTIFIER"]
        if let simulator { lines.append("simulator of \(simulator)") }
        return lines.joined(separator: "\n")
    }

    // MARK: scenes and windows

    static func scenes() -> String {
        let app = UIApplication.shared
        let all = app.connectedScenes.sorted { $0.session.persistentIdentifier < $1.session.persistentIdentifier }
        var lines = ["scenes \(all.count); sessions \(app.openSessions.count)"]
        for (index, scene) in all.enumerated() {
            guard let windowScene = scene as? UIWindowScene else {
                lines.append("scene \(index + 1): \(short(scene)) \(sceneState(scene.activationState))")
                continue
            }
            lines.append("scene \(index + 1): \(short(windowScene)), \(sceneState(windowScene.activationState)), orientation \(orientation(windowScene.interfaceOrientation))")
            let screen = windowScene.screen
            lines.append("  screen \(rect(screen.bounds)) @\(num(screen.scale))x, native \(num(screen.nativeBounds.width))×\(num(screen.nativeBounds.height)), space \(rect(windowScene.coordinateSpace.bounds))")
            let geometry = windowScene.effectiveGeometry
            var geometryLine = "  geometry: orientation \(orientation(geometry.interfaceOrientation))"
            if #available(iOS 26.0, *) {
                geometryLine += ", orientation locked \(yesNo(geometry.isInterfaceOrientationLocked)), resizing \(yesNo(geometry.isInteractivelyResizing))"
            }
            lines.append(geometryLine)
            if let bar = windowScene.statusBarManager {
                lines.append("  status bar: \(yesNo(bar.isStatusBarHidden)) hidden, frame \(rect(bar.statusBarFrame))")
            }
            lines.append("  traits: \(traits(windowScene.traitCollection))")
            lines.append("  windows, front to back:")
            for window in windows(of: windowScene) {
                lines.append("    " + describe(window))
            }
        }
        return lines.joined(separator: "\n")
    }

    // MARK: view controllers

    static func controllers() -> String {
        var lines: [String] = []
        var seen = Set<ObjectIdentifier>()
        for window in visibleWindows() {
            lines.append("\(short(window)) (level \(num(window.windowLevel.rawValue)))")
            guard let root = window.rootViewController else {
                lines.append("  (no root view controller)")
                continue
            }
            describe(controller: root, indent: 1, into: &lines, seen: &seen)
        }
        return lines.isEmpty ? "(no windows)" : lines.joined(separator: "\n")
    }

    private static func describe(controller: UIViewController, indent: Int, into lines: inout [String], seen: inout Set<ObjectIdentifier>) {
        guard seen.insert(ObjectIdentifier(controller)).inserted else { return }
        let pad = String(repeating: "  ", count: indent)
        var head = "\(pad)\(short(controller))"
        if controller.isViewLoaded {
            let view = controller.view!
            head += " view \(rect(view.frame))"
            if view.window == nil { head += " not-in-window" }
            if view.isHidden { head += " hidden" }
            if view.alpha < 1 { head += " alpha \(num(view.alpha))" }
            if controller.view.safeAreaInsets != .zero { head += " safe \(insets(view.safeAreaInsets))" }
        } else {
            head += " view not loaded"
        }
        for (flag, on) in [
            ("being-presented", controller.isBeingPresented), ("being-dismissed", controller.isBeingDismissed),
            ("moving-to-parent", controller.isMovingToParent), ("moving-from-parent", controller.isMovingFromParent),
        ] where on { head += " \(flag)" }
        if controller.transitionCoordinator != nil { head += " IN-TRANSITION" }
        lines.append(head)

        if let navigation = controller as? UINavigationController { describe(navigation: navigation, pad: pad, into: &lines) }
        if let tabs = controller as? UITabBarController { describe(tabs: tabs, pad: pad, into: &lines) }
        if let split = controller as? UISplitViewController {
            lines.append("\(pad)  split: collapsed \(yesNo(split.isCollapsed)), display mode \(split.displayMode.rawValue), columns \(split.viewControllers.count)")
        }

        for child in controller.children {
            describe(controller: child, indent: indent + 1, into: &lines, seen: &seen)
        }
        if let presented = controller.presentedViewController, presented.presentingViewController === controller {
            lines.append("\(pad)  presents (style \(presented.modalPresentationStyle.rawValue)):")
            describe(controller: presented, indent: indent + 2, into: &lines, seen: &seen)
        }
    }

    private static func describe(navigation: UINavigationController, pad: String, into lines: inout [String]) {
        let stack = navigation.viewControllers
        var line = "\(pad)  stack \(stack.count): \(stack.map { short($0, limit: 40) }.joined(separator: " > "))"
        line += "; bar hidden \(yesNo(navigation.isNavigationBarHidden))"
        if navigation.isViewLoaded {
            let bar = navigation.navigationBar
            line += ", frame \(rect(bar.frame)), alpha \(num(bar.alpha)), view hidden \(yesNo(bar.isHidden))"
        }
        lines.append(line)
        if navigation.isViewLoaded, let pop = navigation.interactivePopGestureRecognizer {
            lines.append("\(pad)  pop gesture: enabled \(yesNo(pop.isEnabled)), state \(gestureState(pop.state)), delegate \(pop.delegate == nil ? "none" : "set")")
        }
        if let top = navigation.topViewController {
            lines.append("\(pad)  top item: \(describe(item: top.navigationItem))")
        }
        if let bar = navigation.isViewLoaded ? navigation.navigationBar : nil {
            lines.append("\(pad)  bar items stack: \(bar.items?.count ?? 0), back item \(bar.backItem == nil ? "none" : "yes")")
        }
    }

    private static func describe(item: UINavigationItem) -> String {
        var parts = [
            "title \(item.title.map { "\($0.count) chars" } ?? "none")",
            "title view \(yesNo(item.titleView != nil))",
            "hides back \(yesNo(item.hidesBackButton))",
            "back button title \(item.backButtonTitle.map { "\($0.count) chars" } ?? "none")",
            "left \(item.leftBarButtonItems?.count ?? 0)", "right \(item.rightBarButtonItems?.count ?? 0)",
            "leading groups \(item.leadingItemGroups.count)", "trailing groups \(item.trailingItemGroups.count)",
            "style \(item.style.rawValue)", "large title \(item.largeTitleDisplayMode.rawValue)",
        ]
        if item.leftItemsSupplementBackButton { parts.append("left supplements back") }
        if item.searchController != nil { parts.append("search") }
        return parts.joined(separator: ", ")
    }

    private static func describe(tabs: UITabBarController, pad: String, into lines: inout [String]) {
        var line = "\(pad)  tabs \(tabs.viewControllers?.count ?? 0) (children \(tabs.children.count)), selected \(tabs.selectedIndex)"
        if #available(iOS 18.0, *) {
            line += ", mode \(tabs.mode.rawValue), tab bar hidden \(yesNo(tabs.isTabBarHidden)), sidebar hidden \(yesNo(tabs.sidebar.isHidden))"
        }
        if tabs.isViewLoaded {
            line += ", bar \(rect(tabs.tabBar.frame)) \(tabs.tabBar.isHidden ? "hidden" : "shown") alpha \(num(tabs.tabBar.alpha))"
        }
        lines.append(line)
    }

    // MARK: the view tree

    /// The view tree of each visible window, in window coordinates, without the insides of a SwiftUI page (`HostingView`): those
    /// are most of the views and are not where the bars are. What UIKit puts around them is: the navigation bar, the tab bar and
    /// the containers the system keeps its floating bars in (on the Duo's cover screen the back button lives in one).
    static func tree() -> String {
        var lines: [String] = []
        for window in visibleWindows() {
            lines.append("\(short(window)) \(flags(window))")
            var budget = window.isKeyWindow ? 450 : 150
            dump(window, depth: 1, maxDepth: 30, budget: &budget, into: &lines)
        }
        return lines.isEmpty ? "(no windows)" : lines.joined(separator: "\n")
    }

    private static func dump(_ view: UIView, depth: Int, maxDepth: Int, budget: inout Int, into lines: inout [String]) {
        for child in view.subviews {
            guard budget > 0 else {
                lines.append(String(repeating: "  ", count: depth) + "… (more views not shown)")
                return
            }
            budget -= 1
            var line = String(repeating: "  ", count: depth) + short(child) + " " + rect(child.convert(child.bounds, to: nil))
            let marks = flags(child)
            if !marks.isEmpty { line += " " + marks }
            let leaf = isPage(child) || child is UITabBar || depth >= maxDepth
            if leaf, !child.subviews.isEmpty { line += " (\(child.subviews.count) subviews not shown)" }
            lines.append(line)
            if !leaf { dump(child, depth: depth + 1, maxDepth: maxDepth, budget: &budget, into: &lines) }
        }
    }

    /// What the walks over a window need to know about a kind of view. It is worked out once for each class: making the name
    /// of a type is slow, a window holds far more views than there are kinds of view, and `BarWatch` walks every second.
    private struct Kind {
        let name: String
        /// The SwiftUI body of a page: the view SwiftUI calls `HostingView`, as opposed to the `_UIHostingView` that holds the
        /// app, the tab view and the navigation stack together.
        let isPage: Bool
        /// Where the system keeps its floating bars.
        let isFloatingBar: Bool
        let isBarButton: Bool
        /// Part of the back button's look (a mask view).
        let isBackMark: Bool
        /// The column along the edge of the floating bars' container.
        let isPocket: Bool
    }

    private static var kinds: [ObjectIdentifier: Kind] = [:]

    private static func kind(of view: UIView) -> Kind {
        let metatype = type(of: view)
        if let known = kinds[ObjectIdentifier(metatype)] { return known }
        let name = String(describing: metatype)
        let made = Kind(
            name: name, isPage: name == "HostingView", isFloatingBar: name.hasPrefix("_UIFloatingBarContainerView"),
            isBarButton: name.hasPrefix("_UIButtonBarButton"), isBackMark: name.contains("BackButton"),
            isPocket: name.hasPrefix("VerticalBarPocketView")
        )
        kinds[ObjectIdentifier(metatype)] = made
        return made
    }

    private static func isPage(_ view: UIView) -> Bool { kind(of: view).isPage }

    /// The views below `view` (not inside a page) that `matches`, at most `limit` looked at.
    private static func descendants(of view: UIView, limit: Int = 3000, where matches: (UIView) -> Bool) -> [UIView] {
        var found: [UIView] = []
        var looked = 0
        func visit(_ node: UIView) {
            for child in node.subviews where looked < limit {
                looked += 1
                if matches(child) { found.append(child) }
                if !isPage(child) { visit(child) }
            }
        }
        visit(view)
        return found
    }

    // MARK: the floating bars

    /// The containers the system keeps its floating bars in, and the buttons in them. On the Duo's cover screen the navigation
    /// bar's buttons (the back button above all) are not in the navigation bar: they are in the one of the tab bar controller, in
    /// the column at the right edge, and when that column is empty the page has no way back.
    /// `detailed`: for the report, with the number of views and where the buttons are; the log leaves those out (they change with
    /// every animation, and only whether the buttons are there matters to its lines).
    static func floatingBars(detailed: Bool) -> [String] {
        var out: [String] = []
        for window in visibleWindows() {
            for container in descendants(of: window, where: { kind(of: $0).isFloatingBar }) {
                out.append(describe(floatingBar: container, detailed: detailed))
            }
        }
        return out
    }

    private static func describe(floatingBar container: UIView, detailed: Bool) -> String {
        let all = descendants(of: container) { _ in true }
        let buttons = all.filter { kind(of: $0).isBarButton }
        let shown = buttons.filter { isShown($0, inside: container) }
        let backs = buttons.filter { button in button.subviews.contains { kind(of: $0).isBackMark } }
        var text = "\(owner(of: container)): buttons \(shown.count) shown of \(buttons.count) (back \(backs.count))"
        if detailed { text += ", \(all.count) views" }
        if let pocket = container.subviews.first(where: { kind(of: $0).isPocket }) {
            text += ", column \(pocket.isHidden ? "hidden" : "shown")"
            if detailed { text += " \(rect(pocket.convert(pocket.bounds, to: nil)))" }
        }
        if detailed, !shown.isEmpty {
            text += ", buttons at " + shown.map { rect($0.convert($0.bounds, to: nil)) }.joined(separator: " ")
        }
        return text
    }

    /// The view controller a view belongs to.
    private static func owner(of view: UIView) -> String {
        var responder: UIResponder? = view.next
        while let current = responder {
            if let controller = current as? UIViewController { return short(controller, limit: 40) }
            responder = current.next
        }
        return "no controller"
    }

    /// Whether neither the view nor any view above it, up to `container`, is hidden or clear.
    private static func isShown(_ view: UIView, inside container: UIView) -> Bool {
        var node: UIView? = view
        while let current = node {
            if current.isHidden || current.alpha <= 0.01 { return false }
            if current === container { return true }
            node = current.superview
        }
        return true
    }

    // MARK: what is watched

    /// A few short lines that say how the screen is laid out and what the bars look like. `BarWatch` takes them again and again
    /// and writes the ones that changed. Nothing here may be long: they go into the log.
    static func watched() -> [(key: String, value: String)] {
        var out: [(String, String)] = []
        let app = UIApplication.shared
        out.append(("application", applicationState(app.applicationState)))
        let all = app.connectedScenes.compactMap { $0 as? UIWindowScene }.sorted { $0.session.persistentIdentifier < $1.session.persistentIdentifier }
        for (index, scene) in all.enumerated() {
            let tag = all.count > 1 ? "#\(index + 1) " : ""
            out.append(("\(tag)scene", "\(sceneState(scene.activationState)) \(orientation(scene.interfaceOrientation)) \(rect(scene.coordinateSpace.bounds)) \(traits(scene.traitCollection))"))
            out.append(("\(tag)windows", windows(of: scene).map { describeBriefly($0) }.joined(separator: " | ")))
        }
        guard let window = visibleWindows().first(where: { $0.isKeyWindow }) ?? visibleWindows().first else { return out }
        out.append(("safe area", "\(insets(window.safeAreaInsets)) in \(rect(window.frame))"))
        guard let root = window.rootViewController else { return out }

        var navigations: [UINavigationController] = []
        var tabsFound: [UITabBarController] = []
        var transitioning = false
        gather(root, navigations: &navigations, tabs: &tabsFound, transitioning: &transitioning)
        if transitioning {
            out.append(("navigation", "a transition is running"))
        } else if let front = frontNavigation(from: root) {
            out.append(("navigation", watched(navigation: front, of: navigations.count)))
        } else {
            out.append(("navigation", "no navigation controller"))
        }
        let floating = floatingBars(detailed: false)
        out.append(("floating bars", floating.isEmpty ? "none" : floating.joined(separator: " | ")))
        for (index, tabs) in tabsFound.enumerated() {
            var text = "selected \(tabs.selectedIndex) of \(tabs.children.count)"
            if tabs.isViewLoaded {
                text += ", bar \(tabs.tabBar.isHidden ? "hidden" : "shown") \(rect(tabs.tabBar.frame))"
            }
            if #available(iOS 18.0, *) { text += ", mode \(tabs.mode.rawValue), bar hidden \(yesNo(tabs.isTabBarHidden))" }
            out.append(("tabs\(tabsFound.count > 1 ? " #\(index + 1)" : "")", text))
        }
        return out
    }

    private static func watched(navigation: UINavigationController, of count: Int) -> String {
        var text = "stack \(navigation.viewControllers.count) (of \(count) navigation controllers)"
        guard navigation.isViewLoaded else { return text + ", view not loaded" }
        let bar = navigation.navigationBar
        text += ", bar \(navigation.isNavigationBarHidden ? "hidden" : "shown") \(rect(bar.frame)) alpha \(num(bar.alpha))"
        if let item = navigation.topViewController?.navigationItem {
            text += ", item: back \(item.hidesBackButton ? "hidden" : "allowed") left \(item.leftBarButtonItems?.count ?? 0) right \(item.rightBarButtonItems?.count ?? 0) title \(item.title == nil ? "none" : "set")"
        }
        return text
    }

    private static func gather(
        _ controller: UIViewController, navigations: inout [UINavigationController], tabs: inout [UITabBarController],
        transitioning: inout Bool
    ) {
        if let navigation = controller as? UINavigationController { navigations.append(navigation) }
        if let bar = controller as? UITabBarController { tabs.append(bar) }
        if controller.transitionCoordinator != nil { transitioning = true }
        for child in controller.children {
            gather(child, navigations: &navigations, tabs: &tabs, transitioning: &transitioning)
        }
        if let presented = controller.presentedViewController, presented.presentingViewController === controller {
            gather(presented, navigations: &navigations, tabs: &tabs, transitioning: &transitioning)
        }
    }

    /// The navigation controller the user is looking at: the last one on the way down through what is presented,
    /// the selected tab and the top of each stack.
    private static func frontNavigation(from root: UIViewController) -> UINavigationController? {
        var current: UIViewController? = root
        var found: UINavigationController?
        var guardCount = 0
        while let controller = current, guardCount < 40 {
            guardCount += 1
            if let presented = controller.presentedViewController, presented.presentingViewController === controller {
                current = presented
                continue
            }
            if let tabs = controller as? UITabBarController {
                current = tabs.selectedViewController
            } else if let navigation = controller as? UINavigationController {
                found = navigation
                current = navigation.topViewController
            } else if let split = controller as? UISplitViewController {
                current = split.viewControllers.last
            } else {
                current = controller.children.first
            }
        }
        return found
    }

    // MARK: windows

    /// The windows of a scene, the front one first.
    private static func windows(of scene: UIWindowScene) -> [UIWindow] {
        scene.windows.sorted {
            $0.windowLevel.rawValue == $1.windowLevel.rawValue
                ? String(describing: type(of: $0)) < String(describing: type(of: $1))
                : $0.windowLevel.rawValue > $1.windowLevel.rawValue
        }
    }

    /// The windows that are up, in every scene, the key one of the foreground scene first.
    private static func visibleWindows() -> [UIWindow] {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .sorted { $0.session.persistentIdentifier < $1.session.persistentIdentifier }
        var out: [UIWindow] = []
        for scene in scenes {
            out += windows(of: scene).filter { !$0.isHidden }.sorted { ($0.isKeyWindow ? 0 : 1) < ($1.isKeyWindow ? 0 : 1) }
        }
        return out
    }

    private static func describe(_ window: UIWindow) -> String {
        var line = "\(short(window)) level \(num(window.windowLevel.rawValue))"
        if window.isKeyWindow { line += " KEY" }
        if window.isHidden { line += " hidden" }
        if window.alpha < 1 { line += " alpha \(num(window.alpha))" }
        line += " frame \(rect(window.frame))"
        if window.safeAreaInsets != .zero { line += " safe \(insets(window.safeAreaInsets))" }
        line += " traits \(traits(window.traitCollection))"
        line += " root \(window.rootViewController.map { short($0, limit: 50) } ?? "none")"
        if !window.canBecomeKey { line += " cannot-be-key" }
        return line
    }

    private static func describeBriefly(_ window: UIWindow) -> String {
        "\(short(window, limit: 32)) L\(num(window.windowLevel.rawValue))\(window.isKeyWindow ? " key" : "")\(window.isHidden ? " hidden" : "")"
    }

    // MARK: words and numbers

    /// The name of a type, without its module and cut short (SwiftUI's generic names run on).
    static func short(_ object: AnyObject, limit: Int = 90) -> String {
        let name = String(describing: type(of: object))
        return name.count > limit ? String(name.prefix(limit - 1)) + "…" : name
    }

    private static func flags(_ view: UIView) -> String {
        var marks: [String] = []
        if view.isHidden { marks.append("hidden") }
        if view.alpha < 1 { marks.append("alpha \(num(view.alpha))") }
        if !view.isUserInteractionEnabled { marks.append("no-touch") }
        if !view.transform.isIdentity { marks.append("transformed") }
        if let control = view as? UIControl, !control.isEnabled { marks.append("disabled") }
        return marks.joined(separator: " ")
    }

    private static func traits(_ traits: UITraitCollection) -> String {
        "h=\(sizeClass(traits.horizontalSizeClass)) v=\(sizeClass(traits.verticalSizeClass)) idiom=\(idiom(traits.userInterfaceIdiom)) \(traits.userInterfaceStyle == .dark ? "dark" : "light")"
    }

    static func num(_ value: CGFloat) -> String {
        guard value.isFinite else { return "\(value)" }
        let rounded = (value * 10).rounded() / 10
        return rounded == rounded.rounded() ? String(Int(rounded)) : "\(rounded)"
    }

    static func rect(_ rect: CGRect) -> String {
        "(\(num(rect.origin.x)), \(num(rect.origin.y)), \(num(rect.size.width))×\(num(rect.size.height)))"
    }

    static func insets(_ insets: UIEdgeInsets) -> String {
        "(top \(num(insets.top)), left \(num(insets.left)), bottom \(num(insets.bottom)), right \(num(insets.right)))"
    }

    private static func yesNo(_ value: Bool) -> String { value ? "yes" : "no" }

    private static func idiom(_ idiom: UIUserInterfaceIdiom) -> String {
        switch idiom {
        case .phone: "phone"
        case .pad: "pad"
        case .tv: "tv"
        case .carPlay: "carPlay"
        case .mac: "mac"
        case .vision: "vision"
        default: "unspecified"
        }
    }

    private static func sizeClass(_ value: UIUserInterfaceSizeClass) -> String {
        switch value {
        case .compact: "compact"
        case .regular: "regular"
        default: "unspecified"
        }
    }

    static func applicationState(_ state: UIApplication.State) -> String {
        switch state {
        case .active: "active"
        case .inactive: "inactive"
        case .background: "background"
        @unknown default: "unknown"
        }
    }

    static func sceneState(_ state: UIScene.ActivationState) -> String {
        switch state {
        case .unattached: "unattached"
        case .foregroundActive: "foregroundActive"
        case .foregroundInactive: "foregroundInactive"
        case .background: "background"
        @unknown default: "unknown"
        }
    }

    private static func orientation(_ value: UIInterfaceOrientation) -> String {
        switch value {
        case .portrait: "portrait"
        case .portraitUpsideDown: "portraitUpsideDown"
        case .landscapeLeft: "landscapeLeft"
        case .landscapeRight: "landscapeRight"
        default: "unknown"
        }
    }

    private static func gestureState(_ state: UIGestureRecognizer.State) -> String {
        switch state {
        case .possible: "possible"
        case .began: "began"
        case .changed: "changed"
        case .ended: "ended"
        case .cancelled: "cancelled"
        case .failed: "failed"
        @unknown default: "unknown"
        }
    }

    private static func thermal(_ state: ProcessInfo.ThermalState) -> String {
        switch state {
        case .nominal: "nominal"
        case .fair: "fair"
        case .serious: "serious"
        case .critical: "critical"
        @unknown default: "unknown"
        }
    }

    private static func backgroundRefresh(_ status: UIBackgroundRefreshStatus) -> String {
        switch status {
        case .available: "available"
        case .denied: "denied"
        case .restricted: "restricted"
        @unknown default: "unknown"
        }
    }

    /// What the app may still allocate before the system ends it (the simulator does not say).
    static func memoryLeft() -> String {
        let bytes = os_proc_available_memory()
        return bytes > 0 ? "\(bytes / 1_048_576) MB" : "unknown"
    }

    private static func machine() -> String {
        systemControl("hw.machine") ?? "?"
    }

    private static func systemControl(_ name: String) -> String? {
        var size = 0
        guard sysctlbyname(name, nil, &size, nil, 0) == 0, size > 0 else { return nil }
        var buffer = [CChar](repeating: 0, count: size)
        guard sysctlbyname(name, &buffer, &size, nil, 0) == 0 else { return nil }
        return String(cString: buffer)
    }
}
