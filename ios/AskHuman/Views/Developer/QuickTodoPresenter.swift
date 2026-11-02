import SwiftUI
import UIKit
import AHCore

/// A window that may be key, unlike the others of the developer mode: the quick-add page has a text field, and
/// the keyboard goes to the key window.
final class QuickTodoWindow: UIWindow {}

/// Shows the quick-add page over everything, in a window of its own (docs/specs/ios-developer-mode.md §5.5):
/// the app's window may have a sheet, a full-screen cover or Safari up, and none of that has to be found or
/// waited for. The page is a sheet of that window.
///
/// Becoming key takes the keyboard from whatever the app's window was typing in, so the first responder is
/// noted before and given back after. There is one way out (`finish`), whoever asks for it: the app's state
/// (the Cancel button, a todo that was taken, the mode turned off), the system (a swipe down) or the end of
/// the scene.
@MainActor
final class QuickTodoPresenter: NSObject, UIAdaptivePresentationControllerDelegate {
    private weak var source: UIWindow?
    private var window: QuickTodoWindow?
    private weak var savedResponder: UIResponder?
    /// The sheet is on screen (a dismissal asked for earlier waits for it).
    private var shown = false
    private var dismissWhenShown = false
    private var finishing = false

    /// The system took the page away (a swipe down): the app's state has to follow.
    var onDismissedBySystem: (() -> Void)?
    /// The window is gone and the app's window is key again.
    var onFinished: (() -> Void)?

    var isShowing: Bool { window != nil }

    func present(over source: UIWindow, store: AppStore) {
        guard window == nil, let scene = source.windowScene else { return }
        self.source = source
        savedResponder = Self.firstResponder(in: source)

        let window = QuickTodoWindow(windowScene: scene)
        window.frame = source.frame
        // Above the app's window (+0), the arrival animation (+1) and the button (+2).
        window.windowLevel = UIWindow.Level(rawValue: source.windowLevel.rawValue + 3)
        window.backgroundColor = .clear
        let root = UIViewController()
        root.view.backgroundColor = .clear
        window.rootViewController = root
        self.window = window
        shown = false
        dismissWhenShown = false
        window.makeKeyAndVisible()

        let page = UIHostingController(rootView: QuickTodoView().environment(store))
        page.modalPresentationStyle = .pageSheet
        if let sheet = page.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        page.presentationController?.delegate = self
        root.present(page, animated: true) { [weak self] in
            guard let self else { return }
            self.shown = true
            if self.dismissWhenShown { self.finish(animated: true, bySystem: false) }
        }
    }

    /// The app wants the page gone.
    func dismiss(animated: Bool = true) {
        guard window != nil else { return }
        if shown { finish(animated: animated, bySystem: false) } else { dismissWhenShown = true }
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        finish(animated: false, bySystem: true)
    }

    /// The one way out: takes the window down, makes the app's window key and gives the keyboard back.
    private func finish(animated: Bool, bySystem: Bool) {
        guard let window, !finishing else { return }
        finishing = true
        let done = { [weak self] in
            guard let self else { return }
            window.isHidden = true
            window.rootViewController = nil
            self.window = nil
            self.shown = false
            self.finishing = false
            self.source?.makeKey()
            if let responder = self.savedResponder, responder.canBecomeFirstResponder {
                responder.becomeFirstResponder()
            }
            self.savedResponder = nil
            if bySystem { self.onDismissedBySystem?() }
            self.onFinished?()
        }
        if !bySystem, let page = window.rootViewController?.presentedViewController {
            page.dismiss(animated: animated, completion: done)
        } else {
            done()
        }
    }

    /// The view of `root` that is being typed in, if any.
    private static func firstResponder(in root: UIView) -> UIResponder? {
        if root.isFirstResponder { return root }
        for view in root.subviews {
            if let found = firstResponder(in: view) { return found }
        }
        return nil
    }
}
