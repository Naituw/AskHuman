import Foundation
import Observation

/// A line the floating button's window shows for a moment, after something it started has finished.
public struct DeveloperToast: Identifiable, Equatable, Sendable {
    public enum Message: Equatable, Sendable {
        /// A todo was added to this project.
        case todoAdded(project: String)
    }

    public var id: Int
    public var message: Message
}

/// The developer mode of the app (docs/specs/ios-developer-mode.md): a switch that puts a floating button
/// above every screen, with a menu of shortcuts. Off, the app is the app it always was.
///
/// This holds what the views read and write: whether the mode is on, where the button was left, the note
/// that follows a finished action, and whether the quick-add page is up. How a finger moves the button, and the
/// menu it opens, belong to the button's own view; the windows themselves are the app's business.
@MainActor
@Observable
public final class DeveloperTools {
    public var enabled: Bool {
        didSet {
            defaults.set(enabled, forKey: Self.enabledKey)
            if !enabled {
                toast = nil
                quickTodoPresented = false
            }
        }
    }

    /// Where the button was left. Kept while the mode is off, so it comes back to the same place.
    public var placement: DeveloperPlacement {
        didSet { defaults.set(try? JSONEncoder().encode(placement), forKey: Self.placementKey) }
    }

    public private(set) var toast: DeveloperToast?
    /// The quick-add page should be on screen.
    public private(set) var quickTodoPresented = false

    public let quickTodo: QuickTodoComposer

    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private var toastSerial = 0

    private static let enabledKey = "ah.developerMode"
    private static let placementKey = "ah.dev.placement"

    public init(backend: RemoteBackend, defaults: UserDefaults = .standard) {
        self.defaults = defaults
        enabled = defaults.bool(forKey: Self.enabledKey)
        placement = defaults.data(forKey: Self.placementKey).flatMap { try? JSONDecoder().decode(DeveloperPlacement.self, from: $0) }
            ?? .standard
        quickTodo = QuickTodoComposer(backend: backend, defaults: defaults)
    }

    /// The Macs, as the engine last said (so the quick-add page knows which can be written to).
    public func update(macs: [MacState]) {
        quickTodo.update(macs: macs)
    }

    // MARK: the menu

    /// The user chose an entry of the menu.
    public func invoke(_ item: DeveloperMenuItem) {
        switch item {
        case .quickTodo: presentQuickTodo()
        }
    }

    public func presentQuickTodo() {
        guard enabled else { return }
        quickTodoPresented = true
    }

    public func dismissQuickTodo() {
        quickTodoPresented = false
    }

    /// Sends the quick-add text. When the Mac takes it, the page goes and the note says where it went; when it
    /// does not, the page stays with the reason on it.
    public func submitQuickTodo() async {
        guard let target = await quickTodo.send() else { return }
        showToast(.todoAdded(project: target.projectName))
        quickTodoPresented = false
    }

    // MARK: the note

    public func showToast(_ message: DeveloperToast.Message) {
        toastSerial += 1
        toast = DeveloperToast(id: toastSerial, message: message)
    }

    /// The view is done showing the note. A note that has been replaced meanwhile stays.
    public func clearToast(id: Int) {
        if toast?.id == id { toast = nil }
    }
}
