import AppKit
import Carbon
import ObjectiveC

typealias SessionEndCallback = @convention(c) () -> Void

func isSystemSessionQuit(_ event: NSAppleEventDescriptor?) -> Bool {
    guard let event, event.eventClass == AEEventClass(kCoreEventClass),
        event.eventID == AEEventID(kAEQuitApplication)
    else { return false }
    let reason = event.paramDescriptor(forKeyword: AEKeyword(kAEQuitReason))
        ?? event.attributeDescriptor(forKeyword: AEKeyword(kAEQuitReason))
    switch reason?.enumCodeValue {
    case OSType(kAEShutDown), OSType(kAERestart), OSType(kAEReallyLogOut): return true
    default: return false
    }
}

private var sessionEndObserver: NSObjectProtocol?

private func interceptSessionQuit(_ ending: @escaping SessionEndCallback) {
    guard let delegate = NSApplication.shared.delegate,
        let delegateClass = object_getClass(delegate)
    else { return }
    let selector = #selector(NSApplicationDelegate.applicationShouldTerminate(_:))
    typealias Original = @convention(c) (AnyObject, Selector, NSApplication) -> UInt
    let original = class_getInstanceMethod(delegateClass, selector).map {
        unsafeBitCast(method_getImplementation($0), to: Original.self)
    }
    let handler: @convention(block) (AnyObject, NSApplication) -> UInt = { receiver, application in
        // AppKit clears the Apple Event before willTerminate and Tauri's final exit callback.
        if isSystemSessionQuit(NSAppleEventManager.shared().currentAppleEvent) {
            ending()
            return NSApplication.TerminateReply.terminateNow.rawValue
        }
        return original?(receiver, selector, application)
            ?? NSApplication.TerminateReply.terminateNow.rawValue
    }
    let implementation = imp_implementationWithBlock(handler)
    if !class_addMethod(delegateClass, selector, implementation, "Q@:@") {
        class_replaceMethod(delegateClass, selector, implementation, "Q@:@")
    }
}

// Main thread only. The workspace notification covers session termination before AppKit quits.
@_cdecl("ah_session_end_observe")
func observeSessionEnd(_ ending: @escaping SessionEndCallback) {
    guard sessionEndObserver == nil else { return }
    interceptSessionQuit(ending)
    sessionEndObserver = NSWorkspace.shared.notificationCenter.addObserver(
        forName: NSWorkspace.willPowerOffNotification, object: nil, queue: .main
    ) { _ in ending() }
}
