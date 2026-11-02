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
            store: store,
            enabled: developer.enabled,
            placement: developer.placement,
            toast: developer.toast,
            quickTodoPresented: developer.quickTodoPresented,
            place: { developer.placement = $0 },
            invoke: { developer.invoke($0) },
            toastFinished: { developer.clearToast(id: $0) },
            quickTodoDismissed: { developer.dismissQuickTodo() }
        )
        .frame(width: 0, height: 0)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// A zero-size attachment to the real window, with the button's own window above it (see
/// `DeveloperOverlayController`).
struct DeveloperOverlayHost: UIViewRepresentable {
    var store: AppStore
    var enabled: Bool
    var placement: DeveloperPlacement
    var toast: DeveloperToast?
    var quickTodoPresented: Bool
    var place: (DeveloperPlacement) -> Void
    var invoke: (DeveloperMenuItem) -> Void
    var toastFinished: (Int) -> Void
    var quickTodoDismissed: () -> Void

    func makeCoordinator() -> DeveloperOverlayController { DeveloperOverlayController() }

    func makeUIView(context: Context) -> DeveloperSceneAttachment {
        let view = DeveloperSceneAttachment()
        view.isUserInteractionEnabled = false
        view.attached = { [weak controller = context.coordinator] window in controller?.attach(to: window) }
        return view
    }

    func updateUIView(_ view: DeveloperSceneAttachment, context: Context) {
        context.coordinator.synchronize(
            store: store, enabled: enabled, placement: placement, toast: toast, quickTodoPresented: quickTodoPresented,
            place: place, invoke: invoke, toastFinished: toastFinished, quickTodoDismissed: quickTodoDismissed
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

/// Owns the windows of the developer mode: the one of the floating button, which exists while the mode is on
/// and the app's window is in a scene, and the one of the quick-add page, which exists while the page is up.
/// Turning the mode off, or losing the scene, takes them down.
@MainActor
final class DeveloperOverlayController: NSObject {
    private weak var source: UIWindow?
    private var window: DeveloperButtonWindow?
    private var overlay: DeveloperOverlayView?
    private let quickTodo = QuickTodoPresenter()

    private var store: AppStore?
    private var enabled = false
    private var placement = DeveloperPlacement.standard
    private var toast: DeveloperToast?
    private var quickTodoPresented = false
    private var place: ((DeveloperPlacement) -> Void)?
    private var invoke: ((DeveloperMenuItem) -> Void)?
    private var toastFinished: ((Int) -> Void)?
    private var quickTodoDismissed: (() -> Void)?

    override init() {
        super.init()
        // The system took the page away: what the app believes follows, here first so that nothing
        // brings the page back before the app has heard of it.
        quickTodo.onDismissedBySystem = { [weak self] in
            self?.quickTodoPresented = false
            self?.quickTodoDismissed?()
        }
        // The note that waited for the page to go can show now.
        quickTodo.onFinished = { [weak self] in self?.refresh() }
    }

    func attach(to window: UIWindow?) {
        guard source !== window else { return }
        removeWindow()
        quickTodo.dismiss(animated: false)
        source = window
        refresh()
    }

    /// The values the views read arrive together, each time any of them changes.
    func synchronize(
        store: AppStore, enabled: Bool, placement: DeveloperPlacement, toast: DeveloperToast?, quickTodoPresented: Bool,
        place: @escaping (DeveloperPlacement) -> Void,
        invoke: @escaping (DeveloperMenuItem) -> Void,
        toastFinished: @escaping (Int) -> Void,
        quickTodoDismissed: @escaping () -> Void
    ) {
        self.store = store
        self.enabled = enabled
        self.placement = placement
        self.toast = toast
        self.quickTodoPresented = quickTodoPresented
        self.place = place
        self.invoke = invoke
        self.toastFinished = toastFinished
        self.quickTodoDismissed = quickTodoDismissed
        refresh()
    }

    func detach() {
        removeWindow()
        quickTodo.dismiss(animated: false)
        source = nil
        store = nil
        place = nil
        invoke = nil
        toastFinished = nil
        quickTodoDismissed = nil
    }

    private func refresh() {
        defer { reconcileQuickTodo() }
        guard enabled, let source, let scene = source.windowScene else {
            removeWindow()
            return
        }
        if window == nil { makeWindow(over: source, in: scene) }
        overlay?.apply(placement: placement)
        // A note that comes with the page's going waits until the page is gone, so that it is not seen through it.
        let note = quickTodo.isShowing ? nil : toast
        overlay?.apply(toast: note, text: note?.text)
    }

    /// Brings the page in line with what the app wants. Done a turn later: this runs inside the update of a
    /// SwiftUI view, and a window with a sheet is not made in the middle of that. It looks at what is wanted
    /// by then, so it can be asked for any number of times.
    private func reconcileQuickTodo() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            if self.enabled, self.quickTodoPresented, let source = self.source, let store = self.store {
                self.quickTodo.present(over: source, store: store)
            } else {
                self.quickTodo.dismiss()
            }
        }
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
