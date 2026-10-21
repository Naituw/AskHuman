import AppKit
import ObjectiveC

// Files dragged out of some places (the Dock's stacks, for one) can be opened by a drop target only if it is the
// frontmost application at the moment it first asks the pasteboard server for them: the server logs
// "Target frontmost: 1" for that request and hands out working read access; for a target that is not frontmost it
// logs "Target frontmost: 0", and opening the file afterwards fails with EPERM in the folders macOS guards
// (Downloads, Desktop, Documents). Activating the window after the drag has arrived is too late, because wry reads the
// dragged paths in `draggingEntered:`, the first call a drag makes into a window. So the application takes the front
// right before that first read, and only when the drag carries files.
//
// The same rule applies to whatever the files are dragged on to next, so a drag that leaves the window without
// dropping gives the front back to the application that had it.

private typealias DraggingEntered = @convention(c) (AnyObject, Selector, AnyObject) -> UInt
private typealias DraggingExited = @convention(c) (AnyObject, Selector, AnyObject?) -> Void
private typealias PerformDragOperation = @convention(c) (AnyObject, Selector, AnyObject) -> ObjCBool

private let enteredSelector = NSSelectorFromString("draggingEntered:")
private let exitedSelector = NSSelectorFromString("draggingExited:")
private let performSelector = NSSelectorFromString("performDragOperation:")
private let filenamesType = NSPasteboard.PasteboardType("NSFilenamesPboardType")

private var installed = false

// The web views that take the front for a file drag. The hooks are installed on the class shared by every wry web view
// in the process, so each call checks that its receiver was registered.
private let registeredViews = NSHashTable<AnyObject>.weakObjects()

// The application that was in front when the current drag took the front from it.
private var frontedFrom: NSRunningApplication?

// A fresh NSRunningApplication reads the live state; NSWorkspace.frontmostApplication is refreshed by the run loop,
// which must not turn inside a drag callback.
private func isActiveNow() -> Bool {
    NSRunningApplication(processIdentifier: ProcessInfo.processInfo.processIdentifier)?.isActive ?? false
}

private func draggedPasteboard(of info: AnyObject) -> NSPasteboard? {
    info.perform(NSSelectorFromString("draggingPasteboard"))?.takeUnretainedValue() as? NSPasteboard
}

private func takeFront(for info: AnyObject) {
    frontedFrom = nil
    let types = draggedPasteboard(of: info)?.types ?? []
    guard types.contains(.fileURL) || types.contains(filenamesType), !isActiveNow() else { return }
    let previous = NSWorkspace.shared.frontmostApplication
    NSApplication.shared.activate(ignoringOtherApps: true)
    var waited = 0
    while !isActiveNow(), waited < 150 {
        usleep(2_000)
        waited += 2
    }
    guard isActiveNow() else {
        NSLog("%@", "askhuman.dragfront: not in front after \(waited) ms")
        return
    }
    if let previous, previous.processIdentifier != ProcessInfo.processInfo.processIdentifier {
        frontedFrom = previous
    }
}

// The drag left without dropping: whoever had the front gets it back, unless the user has moved on from here already.
private func giveFrontBack() {
    guard let previous = frontedFrom else { return }
    frontedFrom = nil
    guard isActiveNow(), !previous.isTerminated else { return }
    if #available(macOS 14.0, *) {
        previous.activate(from: NSRunningApplication.current, options: [])
    } else {
        previous.activate(options: [.activateIgnoringOtherApps])
    }
}

// Main thread only. `webView` is the popup's live WKWebView (the wry subclass); its class is wrapped once.
@_cdecl("ah_popup_drag_front_install")
func installPopupDragFront(_ webView: UnsafeMutableRawPointer?) -> Int32 {
    guard let webView else { return 0 }
    let view = Unmanaged<AnyObject>.fromOpaque(webView).takeUnretainedValue()
    registeredViews.add(view)
    if installed { return 1 }
    guard let cls = object_getClass(view),
          let entered = class_getInstanceMethod(cls, enteredSelector),
          let exited = class_getInstanceMethod(cls, exitedSelector),
          let perform = class_getInstanceMethod(cls, performSelector) else {
        NSLog("%@", "askhuman.dragfront: install failed (drag methods missing on \(String(describing: type(of: view))))")
        return 0
    }
    installed = true

    let enteredOriginal = unsafeBitCast(method_getImplementation(entered), to: DraggingEntered.self)
    let enteredHook: @convention(block) (AnyObject, AnyObject) -> UInt = { this, info in
        if registeredViews.contains(this) { takeFront(for: info) }
        return enteredOriginal(this, enteredSelector, info)
    }
    method_setImplementation(entered, imp_implementationWithBlock(enteredHook))

    let exitedOriginal = unsafeBitCast(method_getImplementation(exited), to: DraggingExited.self)
    let exitedHook: @convention(block) (AnyObject, AnyObject?) -> Void = { this, info in
        exitedOriginal(this, exitedSelector, info)
        if registeredViews.contains(this) { giveFrontBack() }
    }
    method_setImplementation(exited, imp_implementationWithBlock(exitedHook))

    // A drop keeps the front: the user is working with the window.
    let performOriginal = unsafeBitCast(method_getImplementation(perform), to: PerformDragOperation.self)
    let performHook: @convention(block) (AnyObject, AnyObject) -> ObjCBool = { this, info in
        if registeredViews.contains(this) { frontedFrom = nil }
        return performOriginal(this, performSelector, info)
    }
    method_setImplementation(perform, imp_implementationWithBlock(performHook))
    return 1
}
