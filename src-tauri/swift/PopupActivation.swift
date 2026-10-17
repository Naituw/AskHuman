import AppKit

typealias PopupActivationChanged = @convention(c) (Int32) -> Void

// Reports when this process becomes or stops being the active application. A window's key state is the wrong signal
// for "the user left AskHuman": it also changes for the sheets, file panels and Quick Look panel that belong to the
// process, while the application's activation does not.
private final class PopupActivationObserver {
    private let center = NotificationCenter.default
    private var observers: [NSObjectProtocol] = []

    init(changed: @escaping PopupActivationChanged) {
        observers.append(center.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in changed(1) })
        observers.append(center.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { _ in changed(0) })
    }
    deinit {
        for observer in observers { center.removeObserver(observer) }
    }
}
private var popupActivationObserver: PopupActivationObserver?

// Main thread only. Starts reporting (once) and returns whether the process is the active application right now.
@_cdecl("ah_popup_activation_observe")
func observePopupActivation(_ changed: @escaping PopupActivationChanged) -> Int32 {
    if popupActivationObserver == nil { popupActivationObserver = PopupActivationObserver(changed: changed) }
    return NSApplication.shared.isActive ? 1 : 0
}
