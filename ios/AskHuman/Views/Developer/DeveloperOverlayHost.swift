import SwiftUI
import UIKit
import AHCore

/// Reads the developer tools off the store, so that only this small view is redone when they change, not the
/// whole tab view.
struct DeveloperOverlayAnchor: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let developer = store.developer
        DeveloperOverlayHost(
            enabled: developer.enabled,
            placement: developer.placement,
            toast: developer.toast,
            place: { developer.placement = $0 },
            invoke: { developer.invoke($0) },
            toastFinished: { developer.clearToast(id: $0) }
        )
        .frame(width: 0, height: 0)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// A zero-size attachment to the real window, with the button's own window above it (see
/// `DeveloperOverlayController`).
struct DeveloperOverlayHost: UIViewRepresentable {
    var enabled: Bool
    var placement: DeveloperPlacement
    var toast: DeveloperToast?
    var place: (DeveloperPlacement) -> Void
    var invoke: (DeveloperMenuItem) -> Void
    var toastFinished: (Int) -> Void

    func makeCoordinator() -> DeveloperOverlayController { DeveloperOverlayController() }

    func makeUIView(context: Context) -> DeveloperSceneAttachment {
        let view = DeveloperSceneAttachment()
        view.isUserInteractionEnabled = false
        view.attached = { [weak controller = context.coordinator] window in controller?.attach(to: window) }
        return view
    }

    func updateUIView(_ view: DeveloperSceneAttachment, context: Context) {
        context.coordinator.synchronize(
            enabled: enabled, placement: placement, toast: toast,
            place: place, invoke: invoke, toastFinished: toastFinished
        )
    }

    static func dismantleUIView(_ view: DeveloperSceneAttachment, coordinator: DeveloperOverlayController) {
        view.attached = nil
        coordinator.detach()
    }
}

final class DeveloperSceneAttachment: UIView {
    var attached: ((UIWindow?) -> Void)?
    override func didMoveToWindow() { super.didMoveToWindow(); attached?(window) }
}

/// Owns the window of the floating button. The window exists while the developer mode is on and the app's
/// window is in a scene; turning the mode off, or losing the scene, takes it down.
@MainActor
final class DeveloperOverlayController: NSObject {
    private weak var source: UIWindow?
    private var window: DeveloperButtonWindow?
    private var overlay: DeveloperOverlayView?

    private var enabled = false
    private var placement = DeveloperPlacement.standard
    private var toast: DeveloperToast?
    private var place: ((DeveloperPlacement) -> Void)?
    private var invoke: ((DeveloperMenuItem) -> Void)?
    private var toastFinished: ((Int) -> Void)?

    func attach(to window: UIWindow?) {
        guard source !== window else { return }
        removeWindow()
        source = window
        refresh()
    }

    func synchronize(
        enabled: Bool, placement: DeveloperPlacement, toast: DeveloperToast?,
        place: @escaping (DeveloperPlacement) -> Void,
        invoke: @escaping (DeveloperMenuItem) -> Void,
        toastFinished: @escaping (Int) -> Void
    ) {
        self.enabled = enabled
        self.placement = placement
        self.toast = toast
        self.place = place
        self.invoke = invoke
        self.toastFinished = toastFinished
        refresh()
    }

    func detach() {
        removeWindow()
        source = nil
        place = nil
        invoke = nil
        toastFinished = nil
    }

    private func refresh() {
        guard enabled, let source, let scene = source.windowScene else {
            removeWindow()
            return
        }
        if window == nil { makeWindow(over: source, in: scene) }
        overlay?.apply(placement: placement)
        overlay?.apply(toast: toast, text: toast?.text)
    }

    private func makeWindow(over source: UIWindow, in scene: UIWindowScene) {
        let window = DeveloperButtonWindow(windowScene: scene)
        window.frame = source.frame
        // Above the app's window (+0) and the arrival animation (+1).
        window.windowLevel = UIWindow.Level(rawValue: source.windowLevel.rawValue + 2)
        window.backgroundColor = .clear
        let overlay = DeveloperOverlayView(frame: source.bounds)
        overlay.apply(placement: placement, animated: false)
        overlay.onInvoke = { [weak self] item in self?.invoke?(item) }
        overlay.onPlace = { [weak self] placement in self?.place?(placement) }
        overlay.onToastFinished = { [weak self] id in self?.toastFinished?(id) }
        let controller = UIViewController()
        controller.view = overlay
        window.rootViewController = controller
        self.window = window
        self.overlay = overlay
        // Setting visibility is intentional: makeKeyAndVisible would dismiss the editing responder.
        window.isHidden = false
    }

    private func removeWindow() {
        window?.isHidden = true
        window?.rootViewController = nil
        window = nil
        overlay = nil
    }
}
