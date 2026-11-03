import UIKit
import AHCore

/// The window of the floating button. Transparent, above the app's window and the arrival animation, and
/// never made key: making it key would take the keyboard from whatever is being typed in (see
/// `ArrivalSceneController`; the window is only ever un-hidden). A touch that is not on the button falls
/// through to the windows below. The button's menu is not drawn here: the system presents it, above every
/// window.
final class DeveloperButtonWindow: UIWindow {
    override var canBecomeKey: Bool { false }

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        let hit = super.hitTest(point, with: event)
        return hit === self ? nil : hit
    }
}

/// What the button's window shows: the button, which takes touches only where it is, and the note that
/// follows a finished action. This view places the button (where the user left it, above the keyboard while
/// one is up, dimmed when it has not been touched for a while) and lets a finger drag it; the menu and what a
/// finger does in it are the system's, on the button itself.
final class DeveloperOverlayView: UIView {
    /// The user chose an entry of the button's menu.
    var onInvoke: ((DeveloperMenuItem) -> Void)?
    /// The button was dropped: this is where it stays.
    var onPlace: ((DeveloperPlacement) -> Void)?
    /// The note with this id has been up long enough.
    var onToastFinished: ((Int) -> Void)?

    private var placement = DeveloperPlacement.standard
    private let button = DeveloperButton()
    private let toastView = DeveloperToastView()
    private var keyboardTop: CGFloat?
    private var buttonCenter = CGPoint.zero
    /// The button's center when the finger that is dragging it took hold.
    private var grabbedAt = CGPoint.zero
    private var dragging = false
    private var shownToast: Int?
    private var idleTask: Task<Void, Never>?
    private var toastTask: Task<Void, Never>?

    private var reduceMotion: Bool { UIAccessibility.isReduceMotionEnabled }

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        isOpaque = false
        isMultipleTouchEnabled = false
        addSubview(button)
        addSubview(toastView)
        isAccessibilityElement = false
        accessibilityElements = [button]

        button.onInvoke = { [weak self] item in self?.onInvoke?(item) }
        button.onTouchDown = { [weak self] in self?.wake() }
        button.onTouchEnd = { [weak self] in self?.scheduleIdle() }
        // A finger that moves right away is recognized as a pan before the menu has opened, and then the
        // button is the pan's: it follows the finger. One that rests gets the menu, and the pan never starts.
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        pan.maximumNumberOfTouches = 1
        button.addGestureRecognizer(pan)

        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(keyboardChanged), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        center.addObserver(self, selector: #selector(keyboardHidden), name: UIResponder.keyboardWillHideNotification, object: nil)
        scheduleIdle()
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    // The tasks hold the view weakly: they end on their own once it is gone.
    deinit { NotificationCenter.default.removeObserver(self) }

    // MARK: Geometry

    private var layout: DeveloperLayout {
        DeveloperLayout(
            size: bounds.size,
            safeArea: DeveloperInsets(
                top: safeAreaInsets.top, left: safeAreaInsets.left, bottom: safeAreaInsets.bottom, right: safeAreaInsets.right
            ),
            keyboardTop: keyboardTop
        )
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard bounds.width > 0, bounds.height > 0 else { return }
        // A finger that is dragging the button owns its center; at any other time it rests where the
        // placement says (and above the keyboard, if one is up).
        if !dragging {
            buttonCenter = placement.displayedCenter(in: layout)
            button.center = buttonCenter
        }
        toastView.place(in: bounds, safeTop: safeAreaInsets.top)
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        setNeedsLayout()
    }

    /// The placement the app remembers (the button follows it unless a finger is moving it). The first one,
    /// as the window comes up, is taken without a flight from the corner.
    func apply(placement new: DeveloperPlacement, animated: Bool = true) {
        guard new != placement else { return }
        placement = new
        if !dragging { moveButton(animated: animated) }
    }

    private func moveButton(animated: Bool) {
        buttonCenter = placement.displayedCenter(in: layout)
        let move = { self.button.center = self.buttonCenter }
        if !animated {
            move()
        } else if reduceMotion {
            move()
            button.alpha = 0.3
            UIView.animate(withDuration: 0.2) { self.button.alpha = 1 }
        } else {
            UIView.animate(
                withDuration: 0.45, delay: 0, usingSpringWithDamping: 0.78, initialSpringVelocity: 0,
                options: [.beginFromCurrentState, .allowUserInteraction], animations: move
            )
        }
    }

    // MARK: Touches

    /// Only the button takes a touch; the rest of the window is not there.
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        guard !isHidden, alpha > 0.01, isUserInteractionEnabled, button.frame.contains(point) else { return nil }
        return button.hitTest(convert(point, to: button), with: event)
    }

    @objc private func panned(_ pan: UIPanGestureRecognizer) {
        switch pan.state {
        case .began:
            wake()
            dragging = true
            grabbedAt = button.center
            button.setLifted(true, reduceMotion: reduceMotion)
        case .changed:
            let move = pan.translation(in: self)
            buttonCenter = DeveloperPlacement.clampedDragCenter(
                CGPoint(x: grabbedAt.x + move.x, y: grabbedAt.y + move.y), in: layout
            )
            button.center = buttonCenter
        case .ended:
            dragging = false
            placement = DeveloperPlacement.snapped(from: buttonCenter, in: layout)
            onPlace?(placement)
            button.setLifted(false, reduceMotion: reduceMotion)
            moveButton(animated: true)
            scheduleIdle()
        case .cancelled, .failed:
            // The system took the touch away (a call, a notification, the app losing the screen): the button
            // goes back to where it was before this touch began.
            dragging = false
            button.setLifted(false, reduceMotion: reduceMotion)
            moveButton(animated: true)
            scheduleIdle()
        default:
            break
        }
    }

    // MARK: Idle

    private func wake() {
        idleTask?.cancel()
        idleTask = nil
        guard button.alpha < 1 else { return }
        UIView.animate(withDuration: 0.15, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction]) {
            self.button.alpha = 1
        }
    }

    /// The button fades back a moment after the last touch, so it covers less of what is behind it.
    private func scheduleIdle() {
        idleTask?.cancel()
        idleTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(DeveloperMetrics.idleDelay))
            guard !Task.isCancelled, let self, !self.dragging else { return }
            UIView.animate(withDuration: 0.4, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction]) {
                self.button.alpha = DeveloperMetrics.idleAlpha
            }
        }
    }

    // MARK: Keyboard

    @objc private func keyboardChanged(_ note: Notification) {
        guard let window, let end = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect else { return }
        let frame = window.convert(end, from: nil)
        keyboardTop = frame.minY < bounds.height - 0.5 ? frame.minY : nil
        animateLayout(note)
    }

    @objc private func keyboardHidden(_ note: Notification) {
        keyboardTop = nil
        animateLayout(note)
    }

    /// The button rides up with the keyboard and back down with it.
    private func animateLayout(_ note: Notification) {
        let info = note.userInfo
        let duration = (info?[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double) ?? 0.25
        let curve = (info?[UIResponder.keyboardAnimationCurveUserInfoKey] as? Int) ?? UIView.AnimationCurve.easeInOut.rawValue
        setNeedsLayout()
        UIView.animate(
            withDuration: duration, delay: 0,
            options: [UIView.AnimationOptions(rawValue: UInt(curve) << 16), .beginFromCurrentState]
        ) { self.layoutIfNeeded() }
    }

    // MARK: Toast

    /// Shows the note with this text (once per id), or takes it down when there is none.
    func apply(toast: DeveloperToast?, text: String?) {
        guard toast?.id != shownToast else { return }
        shownToast = toast?.id
        toastTask?.cancel()
        toastTask = nil
        guard let toast, let text else {
            toastView.hide()
            updateAccessibilityElements()
            return
        }
        toastView.show(text, in: bounds, safeTop: safeAreaInsets.top, reduceMotion: reduceMotion)
        updateAccessibilityElements()
        UIAccessibility.post(notification: .announcement, argument: text)
        let id = toast.id
        toastTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(DeveloperMetrics.toastDuration))
            guard !Task.isCancelled, let self else { return }
            self.toastTask = nil
            self.toastView.hide()
            self.updateAccessibilityElements()
            self.onToastFinished?(id)
        }
    }

    // MARK: Accessibility

    private func updateAccessibilityElements() {
        var elements: [Any] = [button]
        if toastView.isShowing { elements.append(toastView) }
        accessibilityElements = elements
    }
}
