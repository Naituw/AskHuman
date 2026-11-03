import UIKit
import AHCore

// What the developer mode draws: the round button, its menu, and the note that follows a finished action.
// None of them takes a touch (`DeveloperOverlayView` does, for all of them); they only look right and say
// what they are to VoiceOver and to UI tests.

/// The round button.
final class DeveloperButtonView: UIView {
    private let blur = UIVisualEffectView(effect: UIBlurEffect(style: .systemThinMaterial))
    private let icon = UIImageView(image: UIImage(systemName: "hammer.fill"))

    init() {
        let size = DeveloperMetrics.buttonSize
        super.init(frame: CGRect(x: 0, y: 0, width: size, height: size))
        isUserInteractionEnabled = false
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.18
        layer.shadowRadius = 8
        layer.shadowOffset = CGSize(width: 0, height: 2)
        layer.shadowPath = UIBezierPath(ovalIn: bounds).cgPath

        blur.frame = bounds
        blur.clipsToBounds = true
        blur.layer.cornerRadius = DeveloperMetrics.buttonRadius
        blur.layer.borderWidth = 0.5
        blur.layer.borderColor = UIColor.separator.cgColor
        addSubview(blur)

        icon.frame = bounds
        icon.contentMode = .center
        icon.tintColor = .label
        icon.preferredSymbolConfiguration = UIImage.SymbolConfiguration(pointSize: 18, weight: .semibold)
        addSubview(icon)

        isAccessibilityElement = true
        accessibilityIdentifier = "dev-button"
        accessibilityLabel = String(localized: "Developer tools")
        accessibilityTraits = .button
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (view: DeveloperButtonView, _) in
            view.blur.layer.borderColor = UIColor.separator.cgColor
        }
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    /// Picked up by a finger that is moving it: a little larger, a deeper shadow.
    func setLifted(_ lifted: Bool, reduceMotion: Bool) {
        let apply = {
            self.transform = lifted ? CGAffineTransform(scaleX: 1.12, y: 1.12) : .identity
            self.layer.shadowOpacity = lifted ? 0.3 : 0.18
        }
        if reduceMotion {
            apply()
        } else {
            UIView.animate(
                withDuration: 0.28, delay: 0, usingSpringWithDamping: 0.7, initialSpringVelocity: 0,
                options: [.allowUserInteraction, .beginFromCurrentState], animations: apply
            )
        }
    }
}

/// One line of the menu: an icon and a title.
final class DeveloperMenuRowView: UIView {
    let item: DeveloperMenuItem
    private let highlightView = UIView()
    private let icon = UIImageView()
    private let label = UILabel()

    private static let leading: CGFloat = 16
    private static let trailing: CGFloat = 20
    private static let iconWidth: CGFloat = 24
    private static let gap: CGFloat = 12
    /// The highlight stays inside the row, so that its corners follow the menu's own rounded corners instead of
    /// reaching out of them (the first and the last row sit right at the edge of the menu).
    private static let highlightInset = UIEdgeInsets(top: 4, left: 6, bottom: 4, right: 6)

    init(item: DeveloperMenuItem) {
        self.item = item
        super.init(frame: .zero)
        isUserInteractionEnabled = false

        highlightView.backgroundColor = UIColor.tintColor.withAlphaComponent(0.18)
        highlightView.layer.cornerRadius = DeveloperMetrics.menuCornerRadius - Self.highlightInset.left
        highlightView.layer.cornerCurve = .continuous
        highlightView.alpha = 0
        addSubview(highlightView)

        icon.image = UIImage(systemName: item.symbol)
        icon.tintColor = .label
        icon.contentMode = .center
        icon.preferredSymbolConfiguration = UIImage.SymbolConfiguration(pointSize: 17, weight: .regular)
        addSubview(icon)

        label.text = item.title
        label.font = Self.font
        label.textColor = .label
        label.numberOfLines = 1
        addSubview(label)

        isAccessibilityElement = true
        accessibilityIdentifier = "dev-menu-" + item.rawValue
        accessibilityLabel = item.title
        accessibilityTraits = .button
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    private static var font: UIFont { UIFont.preferredFont(forTextStyle: .body) }

    /// The width the row needs for its title.
    static func width(for item: DeveloperMenuItem) -> CGFloat {
        let text = (item.title as NSString).size(withAttributes: [.font: font]).width
        return leading + iconWidth + gap + ceil(text) + trailing
    }

    func setHighlighted(_ on: Bool) {
        UIView.animate(withDuration: 0.12, delay: 0, options: [.allowUserInteraction, .beginFromCurrentState]) {
            self.highlightView.alpha = on ? 1 : 0
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        highlightView.frame = bounds.inset(by: Self.highlightInset)
        icon.frame = CGRect(x: Self.leading, y: 0, width: Self.iconWidth, height: bounds.height)
        let labelX = Self.leading + Self.iconWidth + Self.gap
        label.frame = CGRect(x: labelX, y: 0, width: max(0, bounds.width - labelX - Self.trailing), height: bounds.height)
    }
}

/// The menu next to the button.
final class DeveloperMenuView: UIView {
    private let blur = UIVisualEffectView(effect: UIBlurEffect(style: .systemThickMaterial))
    private var rowViews: [DeveloperMenuRowView] = []
    /// Whether the menu is meant to be on screen (a hide that is still fading must not hide a menu shown again).
    private var wanted = false

    var rows: [DeveloperMenuRowView] { rowViews }

    init() {
        super.init(frame: .zero)
        isUserInteractionEnabled = false
        isHidden = true
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.2
        layer.shadowRadius = 14
        layer.shadowOffset = CGSize(width: 0, height: 4)
        blur.clipsToBounds = true
        blur.layer.cornerRadius = DeveloperMetrics.menuCornerRadius
        blur.layer.cornerCurve = .continuous
        addSubview(blur)
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    /// The width the rows need.
    func contentWidth(for items: [DeveloperMenuItem]) -> CGFloat {
        items.map(DeveloperMenuRowView.width(for:)).max() ?? 0
    }

    /// Puts the menu where `layout` says. It grows from the edge next to the button, which is `opensLeft`
    /// when the button is on the right.
    func apply(_ layout: DeveloperMenuLayout, opensLeft: Bool) {
        if rowViews.map(\.item) != layout.items {
            rowViews.forEach { $0.removeFromSuperview() }
            rowViews = layout.items.map(DeveloperMenuRowView.init(item:))
            rowViews.forEach(addSubview)
        }
        layer.anchorPoint = CGPoint(x: opensLeft ? 1 : 0, y: 0.5)
        bounds = CGRect(origin: .zero, size: layout.frame.size)
        center = CGPoint(x: opensLeft ? layout.frame.maxX : layout.frame.minX, y: layout.frame.midY)
        layer.shadowPath = UIBezierPath(
            roundedRect: bounds, cornerRadius: DeveloperMetrics.menuCornerRadius
        ).cgPath
        blur.frame = bounds
        for (index, row) in rowViews.enumerated() {
            row.frame = layout.rows[index].offsetBy(dx: -layout.frame.minX, dy: -layout.frame.minY)
        }
    }

    func setHighlighted(_ item: DeveloperMenuItem?) {
        for row in rowViews { row.setHighlighted(row.item == item) }
    }

    func show(reduceMotion: Bool) {
        wanted = true
        isHidden = false
        alpha = 0
        if reduceMotion {
            transform = .identity
            UIView.animate(withDuration: 0.15) { self.alpha = 1 }
        } else {
            transform = CGAffineTransform(scaleX: 0.7, y: 0.7)
            UIView.animate(
                withDuration: 0.38, delay: 0, usingSpringWithDamping: 0.78, initialSpringVelocity: 0,
                options: [.allowUserInteraction, .beginFromCurrentState]
            ) {
                self.alpha = 1
                self.transform = .identity
            }
        }
    }

    func hide(reduceMotion: Bool) {
        wanted = false
        setHighlighted(nil)
        UIView.animate(
            withDuration: 0.14, delay: 0, options: [.allowUserInteraction, .beginFromCurrentState],
            animations: {
                self.alpha = 0
                if !reduceMotion { self.transform = CGAffineTransform(scaleX: 0.9, y: 0.9) }
            },
            completion: { _ in
                if !self.wanted { self.isHidden = true }
            }
        )
    }
}

/// "Added to AskHuman", for a moment below the top of the screen.
final class DeveloperToastView: UIView {
    private let blur = UIVisualEffectView(effect: UIBlurEffect(style: .systemThickMaterial))
    private let icon = UIImageView(image: UIImage(systemName: "checkmark.circle.fill"))
    private let label = UILabel()
    private(set) var isShowing = false

    private static let padding: CGFloat = 16
    private static let iconWidth: CGFloat = 20
    private static let gap: CGFloat = 8

    init() {
        super.init(frame: .zero)
        isUserInteractionEnabled = false
        isHidden = true
        alpha = 0
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.15
        layer.shadowRadius = 10
        layer.shadowOffset = CGSize(width: 0, height: 3)

        blur.clipsToBounds = true
        addSubview(blur)
        icon.tintColor = .systemGreen
        icon.contentMode = .center
        addSubview(icon)
        label.font = UIFont.preferredFont(forTextStyle: .subheadline).withWeight(.semibold)
        label.textColor = .label
        label.numberOfLines = 2
        addSubview(label)

        isAccessibilityElement = true
        accessibilityIdentifier = "dev-toast"
        accessibilityTraits = .staticText
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    func show(_ text: String, in area: CGRect, safeTop: CGFloat, reduceMotion: Bool) {
        label.text = text
        accessibilityLabel = text
        isShowing = true
        isHidden = false
        place(in: area, safeTop: safeTop)
        alpha = 0
        transform = reduceMotion ? .identity : CGAffineTransform(translationX: 0, y: -14)
        UIView.animate(
            withDuration: reduceMotion ? 0.15 : 0.35, delay: 0, usingSpringWithDamping: 0.85, initialSpringVelocity: 0,
            options: [.beginFromCurrentState]
        ) {
            self.alpha = 1
            self.transform = .identity
        }
    }

    func hide() {
        guard isShowing else { return }
        isShowing = false
        UIView.animate(
            withDuration: 0.25, delay: 0, options: [.beginFromCurrentState],
            animations: { self.alpha = 0 },
            completion: { _ in
                if !self.isShowing { self.isHidden = true }
            }
        )
    }

    /// Centered under the top safe area.
    func place(in area: CGRect, safeTop: CGFloat) {
        guard isShowing else { return }
        let maxWidth = max(0, area.width - 48)
        let textRoom = max(0, maxWidth - Self.padding * 2 - Self.iconWidth - Self.gap)
        let text = label.sizeThatFits(CGSize(width: textRoom, height: .greatestFiniteMagnitude))
        let size = CGSize(
            width: min(maxWidth, Self.padding * 2 + Self.iconWidth + Self.gap + ceil(text.width)),
            height: max(40, ceil(text.height) + 20)
        )
        let transform = self.transform
        self.transform = .identity
        bounds = CGRect(origin: .zero, size: size)
        center = CGPoint(x: area.midX, y: safeTop + 8 + size.height / 2)
        self.transform = transform
        blur.frame = bounds
        blur.layer.cornerRadius = size.height / 2
        layer.shadowPath = UIBezierPath(roundedRect: bounds, cornerRadius: size.height / 2).cgPath
        icon.frame = CGRect(x: Self.padding, y: 0, width: Self.iconWidth, height: size.height)
        label.frame = CGRect(
            x: Self.padding + Self.iconWidth + Self.gap, y: 0,
            width: size.width - Self.padding * 2 - Self.iconWidth - Self.gap, height: size.height
        )
    }
}

/// The three kinds of tick the button gives.
@MainActor
final class DeveloperHaptics {
    private let light = UIImpactFeedbackGenerator(style: .light)
    private let firm = UIImpactFeedbackGenerator(style: .medium)
    private let selection = UISelectionFeedbackGenerator()

    func prepare() {
        light.prepare()
        selection.prepare()
    }

    func play(_ kind: DeveloperPress.Haptic) {
        switch kind {
        case .opened: light.impactOccurred()
        case .changed: selection.selectionChanged()
        case .invoked: firm.impactOccurred()
        }
    }
}

private extension UIFont {
    func withWeight(_ weight: UIFont.Weight) -> UIFont {
        let descriptor = fontDescriptor.addingAttributes([
            .traits: [UIFontDescriptor.TraitKey.weight: weight]
        ])
        return UIFont(descriptor: descriptor, size: pointSize)
    }
}
