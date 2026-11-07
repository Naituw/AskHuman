import Foundation
import Observation

/// A line the floating button's window shows for a moment, after something it started has finished.
public struct DeveloperToast: Identifiable, Equatable, Sendable {
    public enum Message: Equatable, Sendable {
        /// A todo was added to this project.
        case todoAdded(project: String)
        /// The diagnostics are on the pasteboard.
        case diagnosticsCopied
    }

    public var id: Int
    public var message: Message
    /// How long it stays up, in seconds.
    public var hold: TimeInterval = DeveloperMetrics.toastDuration
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

    /// Where "Copy diagnostics" puts the text: the app gives it the pasteboard. Nothing is copied, and nothing is said,
    /// while there is none.
    @ObservationIgnored public var diagnosticsSink: (@MainActor (String) -> Void)?
    /// How long a note stays up. UI tests stretch it: a busy simulator can take longer than the usual moment to look.
    @ObservationIgnored public var toastHold = DeveloperMetrics.toastDuration

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
        case .copyDiagnostics: copyDiagnostics()
        }
    }

    /// Makes the report of `Diagnostics` and hands it to the sink. The copy is itself written to the log, so that a
    /// later report shows where the earlier one was taken.
    public func copyDiagnostics() {
        guard enabled, let sink = diagnosticsSink else { return }
        let text = Diagnostics.shared.report()
        sink(text)
        Diagnostics.note("dev", "diagnostics copied (\(text.count) characters)")
        showToast(.diagnosticsCopied)
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
        toast = DeveloperToast(id: toastSerial, message: message, hold: toastHold)
    }

    /// The view is done showing the note. A note that has been replaced meanwhile stays.
    public func clearToast(id: Int) {
        if toast?.id == id { toast = nil }
    }
}
