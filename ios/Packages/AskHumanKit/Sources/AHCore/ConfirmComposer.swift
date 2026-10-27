import Foundation
import Observation
import AHProtocol

/// The state of a confirmation page (permission prompt, stop hook): which button the user picked and
/// what they typed, with the Mac's visibility rules for the optional input. Nothing is preselected.
@MainActor
@Observable
public final class ConfirmComposer {
    public let confirm: ConfirmView
    public private(set) var selectedWireIndex: Int?
    public var input = ""
    public var isSending = false

    public init(confirm: ConfirmView) {
        self.confirm = confirm
    }

    public var selectedChoice: ConfirmChoiceView? {
        confirm.choices.first { $0.wireIndex == selectedWireIndex }
    }

    public func select(wireIndex: Int) {
        guard confirm.choices.contains(where: { $0.wireIndex == wireIndex }) else { return }
        selectedWireIndex = wireIndex
    }

    /// The input field, when it applies to the current choice.
    public var visibleInput: ConfirmInputView? {
        guard let field = confirm.input else { return nil }
        guard let required = field.visibleWhenIndex else { return field }
        return selectedWireIndex == required ? field : nil
    }

    private var trimmedInput: String { input.trimmingCharacters(in: .whitespacesAndNewlines) }

    public var canSubmit: Bool {
        guard selectedWireIndex != nil, !isSending else { return false }
        if let field = visibleInput, field.required, trimmedInput.isEmpty { return false }
        return true
    }

    /// Text to send along with the decision (`nil` when no input is visible or it is empty).
    public var inputToSend: String? {
        guard let field = visibleInput else { return nil }
        let text = String(trimmedInput.prefix(field.maxChars))
        return text.isEmpty ? nil : text
    }

    /// The Mac maps dismissing the page to a choice of its own (usually "deny").
    public var canDismiss: Bool { confirm.dismissIndex != nil }

    public var expiresAt: Date { Date(timeIntervalSince1970: TimeInterval(confirm.expiresAt) / 1000) }
}
