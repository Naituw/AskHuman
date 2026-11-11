import SwiftUI
import UIKit
import AHCore
import AHProtocol
import OSLog

enum AttachmentDrag {
    @MainActor
    static func provider(store: AppStore, macId: String, asset: AssetRef) -> NSItemProvider {
        let promise = AttachmentFilePromise(asset: asset, activity: { cancel in
            let lease = ExportExecution(cancel: cancel)
            return { lease.finish() }
        }) { progress in
            try await store.downloadForExport(macId: macId, asset: asset, progress: progress)
        }
        Logger(subsystem: "com.naituw.humaninloop.ios", category: "attachment-drag")
            .info("Offering file drag type \(promise.typeIdentifier, privacy: .public)")
        #if DEBUG
        // The host switches apps after the real lift, rather than guessing XCTest's event scheduling delay.
        if AppEnvironment.current?.launch.mock == true, ProcessInfo.processInfo.arguments.contains("-AHDragReceiverTest") {
            let signal = FileManager.default.temporaryDirectory.appendingPathComponent("askhuman-drag-ready")
            try? promise.typeIdentifier.write(to: signal, atomically: true, encoding: .utf8)
        }
        #endif
        return promise.provider()
    }
}

/// Only a receiver requesting a promised file acquires background time; an abandoned lift does not.
@MainActor
private final class ExportExecution {
    private var token = UIBackgroundTaskIdentifier.invalid

    init(cancel: @escaping @Sendable () -> Void) {
        token = UIApplication.shared.beginBackgroundTask(withName: "Attachment export") { [weak self] in
            cancel()
            Task { @MainActor in self?.finish() }
        }
        if token == .invalid { cancel() }
    }

    func finish() {
        guard token != .invalid else { return }
        let ending = token
        token = .invalid
        UIApplication.shared.endBackgroundTask(ending)
    }
}

private struct AttachmentDragModifier: ViewModifier {
    let macId: String
    let asset: AssetRef
    @Environment(AppStore.self) private var store

    @ViewBuilder func body(content: Content) -> some View {
        if asset.tooLarge {
            content
        } else {
            content.onDrag { AttachmentDrag.provider(store: store, macId: macId, asset: asset) }
        }
    }
}

extension View {
    func attachmentDrag(macId: String, asset: AssetRef) -> some View {
        modifier(AttachmentDragModifier(macId: macId, asset: asset))
    }
}
