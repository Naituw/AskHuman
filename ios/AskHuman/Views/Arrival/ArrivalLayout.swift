import UIKit

/// All positions come from the source scene's current UIKit layout, never a device-size table.
struct ArrivalLayout {
    enum Destination: Equatable { case inbox, title }
    var origin: CGPoint
    var target: CGPoint
    var destination: Destination
    var receiverCenter: CGPoint
    var title: UIView?

    @MainActor
    static func measure(in window: UIWindow, keyboard: CGRect) -> ArrivalLayout {
        var content = window.rootViewController
        while let presented = content?.presentedViewController, !presented.isBeingDismissed { content = presented }
        let inPresentation = content !== window.rootViewController
        let root = content?.viewIfLoaded ?? window
        let bars = descendants(of: root).compactMap { $0 as? UINavigationBar }.filter { visible($0, in: window) }
        let bar = bars.min { $0.convert($0.bounds, to: window).minY < $1.convert($1.bounds, to: window).minY }
        let nav = bar.map { $0.convert($0.bounds, to: window) }
        let keyboard = keyboard.intersection(window.bounds)
        let hasKeyboard = !keyboard.isNull && keyboard.height > 20
        let tabBar = !inPresentation && !hasKeyboard
            ? descendants(of: window).compactMap { $0 as? UITabBar }.first { visible($0, in: window) } : nil

        let top = max(window.safeAreaInsets.top, nav?.maxY ?? window.safeAreaInsets.top)
        let bottom = hasKeyboard ? keyboard.minY : (tabBar.map { $0.convert($0.bounds, to: window).minY }
            ?? (window.bounds.maxY - window.safeAreaInsets.bottom))
        let low = min(top + 72, bottom - 48), high = max(low, bottom - 56)
        var origin = CGPoint(x: window.bounds.midX, y: (low + high) / 2)
        if let input = descendants(of: window).first(where: \.isFirstResponder) {
            let editing = input.convert(input.bounds, to: window).insetBy(dx: -44, dy: -44)
            if editing.contains(origin) {
                origin.y = editing.minY > low ? max(low, editing.minY - 12) : min(high, editing.maxY + 12)
            }
        }

        if let tabBar, let target = inboxAnchor(tabBar, in: window) {
            return ArrivalLayout(origin: origin, target: target, destination: .inbox, receiverCenter: .zero, title: nil)
        }

        var title = bar?.topItem?.titleView
        if title == nil, let bar, let text = bar.topItem?.title, !text.isEmpty {
            title = descendants(of: bar).compactMap { $0 as? UILabel }.first {
                $0.text == text && abs($0.convert($0.bounds, to: window).midX - window.bounds.midX) < window.bounds.width * 0.2
            }
        }
        var receiverCenter = CGPoint(x: nav?.midX ?? window.bounds.midX, y: (nav?.maxY ?? window.safeAreaInsets.top + 24) - 9)
        if title == nil, let bar, let nav {
            // A detail page can use most of the trailing bar for request navigation. Receive in the
            // remaining space, or just below the bar when even a compact hint would crowd buttons.
            let controls = descendants(of: bar).compactMap { $0 as? UIControl }.filter {
                visible($0, in: window) && $0.bounds.width > 20 && $0.bounds.width < bar.bounds.width * 0.7 && $0.bounds.height > 20
            }.map { $0.convert($0.bounds, to: window) }.sorted { $0.minX < $1.minX }
            var edge = nav.minX + 8
            var gaps: [ClosedRange<CGFloat>] = []
            for rect in controls {
                if rect.minX > edge { gaps.append(edge...rect.minX) }
                edge = max(edge, rect.maxX)
            }
            if edge < nav.maxX - 8 { gaps.append(edge...(nav.maxX - 8)) }
            if let gap = gaps.max(by: { $0.upperBound - $0.lowerBound < $1.upperBound - $1.lowerBound }), gap.upperBound - gap.lowerBound >= 160 {
                receiverCenter.x = (gap.lowerBound + gap.upperBound) / 2
            } else {
                receiverCenter.y = nav.maxY + 12
            }
        }
        if let title, title.bounds.height > 28 {
            // A gallery title may already contain a filename and page count. Leave both intact.
            receiverCenter.y = (nav?.maxY ?? window.safeAreaInsets.top) + 12
        }
        return ArrivalLayout(origin: origin, target: receiverCenter, destination: .title,
                             receiverCenter: receiverCenter, title: (title?.bounds.height ?? 0) <= 28 ? title : nil)
    }

    @MainActor
    private static func inboxAnchor(_ bar: UITabBar, in window: UIWindow) -> CGPoint? {
        guard let items = bar.items, !items.isEmpty else { return nil }
        let controls = descendants(of: bar).compactMap { $0 as? UIControl }.filter {
            let rect = $0.convert($0.bounds, to: window)
            return visible($0, in: window) && rect.width > 20 && rect.width < bar.bounds.width * 0.6 && rect.height > 20
        }
        let first = controls.min { $0.convert($0.bounds, to: window).midX < $1.convert($1.bounds, to: window).midX }
        if let first {
            let icon = descendants(of: first).compactMap { $0 as? UIImageView }.first {
                $0.image != nil && $0.bounds.width >= 16 && $0.bounds.width <= 50 && $0.bounds.height >= 16
            }
            let anchor = (icon as UIView?) ?? first
            let rect = anchor.convert(anchor.bounds, to: window)
            return CGPoint(x: rect.midX, y: rect.midY)
        }
        // Public item count and the actual bar frame remain a safe fallback during tab transitions.
        let frame = bar.convert(bar.bounds, to: window)
        return CGPoint(x: frame.minX + frame.width / CGFloat(items.count) / 2, y: frame.midY - 6)
    }

    @MainActor
    static func descendants(of view: UIView) -> [UIView] {
        [view] + view.subviews.flatMap { descendants(of: $0) }
    }

    @MainActor
    static func visible(_ view: UIView, in window: UIWindow) -> Bool {
        guard view.window === window, !view.bounds.isEmpty else { return false }
        var node: UIView? = view
        while let current = node {
            if current.isHidden || current.alpha < 0.01 { return false }
            node = current.superview
        }
        let rect = view.convert(view.bounds, to: window)
        return window.bounds.intersection(rect).width > 1 && window.bounds.intersection(rect).height > 1
    }
}
