import UIKit
import AHCore

// What the developer mode draws: the round button, which is a system button with the system's menu, and the
// note that follows a finished action. Neither draws a menu of its own: the press-and-slide, the highlight, the
// haptics and VoiceOver's menu are what `UIMenu` does (docs/specs/ios-developer-mode.md §5).

/// The round button.
///
/// A finger that rests on it for a moment makes the system open the menu under the finger, which slides to a
/// row and lets go to choose it; a quick tap opens the menu and leaves it open. A finger that moves right
/// away drags the button instead, and no menu opens in that touch: the pan recognizer that the owning view
/// adds is recognized after a few points of movement, well before the menu is ready.
///
/// On iOS 26 and later it is Liquid Glass, and the menu grows out of it; before that, a blur material.
final class DeveloperButton: UIButton {
    /// A finger came down on the button.
    var onTouchDown: (() -> Void)?
    /// The finger is off the button, whatever took the touch: it was let go, the menu or a drag took it over,
    /// or the system took it away.
    var onTouchEnd: (() -> Void)?
    /// An entry of the menu was chosen.
    var onInvoke: ((DeveloperMenuItem) -> Void)?

    init() {
        let size = DeveloperMetrics.buttonSize
        super.init(frame: CGRect(x: 0, y: 0, width: size, height: size))

        var configuration: UIButton.Configuration
        if #available(iOS 26, *) {
            configuration = .glass()
        } else {
            configuration = .plain()
            configuration.background.visualEffect = UIBlurEffect(style: .systemThinMaterial)
            configuration.background.cornerRadius = DeveloperMetrics.buttonRadius
            configuration.background.strokeColor = .separator
            configuration.background.strokeWidth = 0.5
            layer.shadowColor = UIColor.black.cgColor
            layer.shadowOpacity = Self.restingShadow
            layer.shadowRadius = 8
            layer.shadowOffset = CGSize(width: 0, height: 2)
            layer.shadowPath = UIBezierPath(ovalIn: bounds).cgPath
        }
        configuration.image = UIImage(systemName: "hammer.fill")
        configuration.baseForegroundColor = .label
        configuration.contentInsets = .zero
        configuration.preferredSymbolConfigurationForImage = UIImage.SymbolConfiguration(pointSize: 18, weight: .semibold)
        self.configuration = configuration

        menu = UIMenu(children: DeveloperMenuItem.allCases.map { item in
            UIAction(title: item.title, image: UIImage(systemName: item.symbol)) { [weak self] _ in
                self?.onInvoke?(item)
            }
        })
        showsMenuAsPrimaryAction = true

        accessibilityIdentifier = "dev-button"
        accessibilityLabel = String(localized: "Developer tools")
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    private static let restingShadow: Float = 0.18

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        onTouchDown?()
        super.touchesBegan(touches, with: event)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesEnded(touches, with: event)
        onTouchEnd?()
    }

    // The menu and a drag both take the touch over: that is a cancel, not an end.
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesCancelled(touches, with: event)
        onTouchEnd?()
    }

    /// Picked up by a finger that is moving it: a little larger (and, without the glass, a deeper shadow).
    func setLifted(_ lifted: Bool, reduceMotion: Bool) {
        let apply = {
            self.transform = lifted ? CGAffineTransform(scaleX: 1.12, y: 1.12) : .identity
            if #unavailable(iOS 26) { self.layer.shadowOpacity = lifted ? 0.3 : Self.restingShadow }
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

/// "Added to AskHuman", for a moment below the top of the screen. Liquid Glass on iOS 26 and later, a blur
/// material before that.
final class DeveloperToastView: UIView {
    private let blur: UIVisualEffectView = {
        if #available(iOS 26, *) { return UIVisualEffectView(effect: UIGlassEffect(style: .regular)) }
        return UIVisualEffectView(effect: UIBlurEffect(style: .systemThickMaterial))
    }()
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
        if #available(iOS 26, *) {
            // The glass draws its own edge and shadow.
            blur.cornerConfiguration = .capsule()
        } else {
            layer.shadowColor = UIColor.black.cgColor
            layer.shadowOpacity = 0.15
            layer.shadowRadius = 10
            layer.shadowOffset = CGSize(width: 0, height: 3)
            blur.clipsToBounds = true
        }
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
        if #unavailable(iOS 26) { blur.layer.cornerRadius = size.height / 2 }
        layer.shadowPath = UIBezierPath(roundedRect: bounds, cornerRadius: size.height / 2).cgPath
        icon.frame = CGRect(x: Self.padding, y: 0, width: Self.iconWidth, height: size.height)
        label.frame = CGRect(
            x: Self.padding + Self.iconWidth + Self.gap, y: 0,
            width: size.width - Self.padding * 2 - Self.iconWidth - Self.gap, height: size.height
        )
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
