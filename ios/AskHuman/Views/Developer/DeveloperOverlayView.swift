import UIKit
import AHCore

/// The window of the floating button. Transparent, above the app's window and the arrival animation, and
/// never made key: making it key would take the keyboard from whatever is being typed in (see
/// `ArrivalSceneController`; the window is only ever un-hidden). A touch that is not on the button or the
/// open menu falls through to the windows below.
final class DeveloperButtonWindow: UIWindow {
    override var canBecomeKey: Bool { false }

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        let hit = super.hitTest(point, with: event)
        return hit === self ? nil : hit
    }
}

/// What the button's window shows, and the one view in it that takes touches: on the button while the menu is
/// shut, anywhere while it is open (a touch outside closes it and goes no further). The decisions are
/// `DeveloperPress`'s (the same code the unit tests run); this carries out what it asks for.
final class DeveloperOverlayView: UIView {
    var onInvoke: ((DeveloperMenuItem) -> Void)?
    /// The button was dropped: this is where it stays.
    var onPlace: ((DeveloperPlacement) -> Void)?
    /// The note with this id has been up long enough.
    var onToastFinished: ((Int) -> Void)?

    private let items = DeveloperMenuItem.allCases
    private var placement = DeveloperPlacement.standard
    private var press = DeveloperPress()
    private let button = DeveloperButtonView()
    private let menu = DeveloperMenuView()
    private let toastView = DeveloperToastView()
    private let haptics = DeveloperHaptics()
    private var keyboardTop: CGFloat?
    private var buttonCenter = CGPoint.zero
    private var menuLayout: DeveloperMenuLayout?
    private var activeTouch: UITouch?
    private var shownToast: Int?
    private var menuTask: Task<Void, Never>?
    private var idleTask: Task<Void, Never>?
    private var toastTask: Task<Void, Never>?

    private var reduceMotion: Bool { UIAccessibility.isReduceMotionEnabled }

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        isOpaque = false
        isMultipleTouchEnabled = false
        addSubview(menu)
        addSubview(button)
        addSubview(toastView)
        isAccessibilityElement = false
        accessibilityElements = [button]
        button.accessibilityCustomActions = items.map { item in
            UIAccessibilityCustomAction(name: item.title) { [weak self] _ in
                self?.onInvoke?(item)
                return true
            }
        }
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(keyboardChanged), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        center.addObserver(self, selector: #selector(keyboardHidden), name: UIResponder.keyboardWillHideNotification, object: nil)
        center.addObserver(self, selector: #selector(interrupted), name: UIApplication.willResignActiveNotification, object: nil)
        center.addObserver(self, selector: #selector(sceneInterrupted), name: UIScene.willDeactivateNotification, object: nil)
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

    private var buttonRect: CGRect {
        let size = DeveloperMetrics.buttonSize
        return CGRect(x: buttonCenter.x - size / 2, y: buttonCenter.y - size / 2, width: size, height: size)
    }

    private var hitMap: DeveloperHitMap { DeveloperHitMap(button: buttonRect, menu: menuLayout) }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard bounds.width > 0, bounds.height > 0 else { return }
        // A finger that is dragging the button owns its center; at any other time it rests where the
        // placement says (and above the keyboard, if one is up).
        if press.phase != .dragging {
            buttonCenter = placement.displayedCenter(in: layout)
            button.center = buttonCenter
        }
        if menuLayout != nil { layOutMenu() }
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
        if press.phase == .idle { moveButton(animated: animated) }
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

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        guard !isHidden, alpha > 0.01, isUserInteractionEnabled else { return nil }
        if menuLayout != nil { return self }
        return buttonRect.contains(point) ? self : nil
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard activeTouch == nil, let touch = touches.first else { return }
        activeTouch = touch
        wake()
        haptics.prepare()
        run(press.down(at: touch.location(in: self), hits: hitMap))
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = activeTouch, touches.contains(touch) else { return }
        run(press.move(to: touch.location(in: self), hits: hitMap))
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = activeTouch, touches.contains(touch) else { return }
        activeTouch = nil
        run(press.up(at: touch.location(in: self), hits: hitMap))
        if press.phase == .idle { scheduleIdle() }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = activeTouch, touches.contains(touch) else { return }
        activeTouch = nil
        run(press.cancel())
        scheduleIdle()
    }

    @objc private func interrupted() {
        activeTouch = nil
        run(press.cancel())
        scheduleIdle()
    }

    @objc private func sceneInterrupted(_ note: Notification) {
        guard let scene = note.object as? UIScene, scene === window?.windowScene else { return }
        interrupted()
    }

    // MARK: Effects

    private func run(_ effects: [DeveloperPress.Effect]) {
        for effect in effects {
            switch effect {
            case .scheduleMenuTimer(let delay):
                menuTask?.cancel()
                menuTask = Task { [weak self] in
                    try? await Task.sleep(for: .seconds(delay))
                    guard !Task.isCancelled, let self else { return }
                    self.menuTask = nil
                    self.run(self.press.menuTimerFired())
                }
            case .cancelMenuTimer:
                menuTask?.cancel()
                menuTask = nil
            case .showMenu:
                showMenu()
            case .hideMenu:
                hideMenu()
            case .highlight(let item):
                menu.setHighlighted(item)
            case .beginDrag:
                button.setLifted(true, reduceMotion: reduceMotion)
            case .drag(let center):
                buttonCenter = DeveloperPlacement.clampedDragCenter(center, in: layout)
                button.center = buttonCenter
            case .endDrag(let center):
                let dropped = DeveloperPlacement.clampedDragCenter(center, in: layout)
                placement = DeveloperPlacement.snapped(from: dropped, in: layout)
                onPlace?(placement)
                button.setLifted(false, reduceMotion: reduceMotion)
                moveButton(animated: true)
            case .invoke(let item):
                onInvoke?(item)
            case .haptic(let kind):
                haptics.play(kind)
            case .restorePosition:
                button.setLifted(false, reduceMotion: reduceMotion)
                moveButton(animated: true)
            }
        }
    }

    private func showMenu() {
        layOutMenu()
        menu.show(reduceMotion: reduceMotion)
        updateAccessibilityElements()
    }

    private func hideMenu() {
        menuLayout = nil
        menu.hide(reduceMotion: reduceMotion)
        updateAccessibilityElements()
        scheduleIdle()
    }

    /// Puts the menu where the button now is (the window may have changed size or the keyboard come up).
    private func layOutMenu() {
        let made = DeveloperMenuLayout.make(
            items: items, width: menu.contentWidth(for: items), buttonCenter: buttonCenter, in: layout
        )
        menuLayout = made
        menu.apply(made, opensLeft: buttonCenter.x >= bounds.width / 2)
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
            guard !Task.isCancelled, let self, self.press.phase == .idle else { return }
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
        if menuLayout != nil { elements += menu.rows }
        if toastView.isShowing { elements.append(toastView) }
        accessibilityElements = elements
    }

    /// The two-finger scrub closes the open menu.
    override func accessibilityPerformEscape() -> Bool {
        guard menuLayout != nil else { return false }
        run(press.cancel())
        return true
    }
}
