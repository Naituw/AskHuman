import AppKit

typealias PopupPulseCancel = @convention(c) () -> Void

// Cancel before AppKit starts moving/resizing the window. Events inside AppKit's nested
// drag loop do not reach a local event monitor, so the initial event is the boundary.
private final class PopupPulseInteraction {
    weak var window: NSWindow?
    var monitor: Any?
    var observers: [NSObjectProtocol] = []

    init(window: NSWindow, cancel: @escaping PopupPulseCancel) {
        self.window = window
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak window] event in
            if event.window === window { cancel() }
            return event
        }
        for name in [NSWindow.willMoveNotification, NSWindow.willStartLiveResizeNotification,
                     NSWindow.willMiniaturizeNotification, NSWindow.willEnterFullScreenNotification,
                     NSWindow.willCloseNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: window, queue: nil) { _ in cancel() })
        }
    }
    deinit {
        if let monitor { NSEvent.removeMonitor(monitor) }
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
    }
}
private var popupPulseInteraction: PopupPulseInteraction?

@_cdecl("ah_popup_pulse_watch_interaction")
func watchPopupPulseInteraction(_ pointer: UnsafeMutableRawPointer, _ cancel: @escaping PopupPulseCancel) {
    let window = Unmanaged<NSWindow>.fromOpaque(pointer).takeUnretainedValue()
    if popupPulseInteraction?.window !== window {
        popupPulseInteraction = PopupPulseInteraction(window: window, cancel: cancel)
    }
}
