#if DEBUG
import UIKit

/// A mock-only UI-test fixture for a real UITextInput marked range, not a simulated text binding.
@MainActor
final class ArrivalCompositionProbe {
    private weak var view: UIView?
    private var expectedSelection: [Int] = []
    private var prepared = false

    func inspect(in window: UIWindow) -> String {
        if !prepared {
            guard let field = ArrivalLayout.descendants(of: window).first(where: \.isFirstResponder),
                  let input = field as? UITextInput else { return "waiting" }
            input.setMarkedText("ceshi", selectedRange: NSRange(location: 5, length: 0))
            guard input.markedTextRange != nil else { return "waiting" }
            view = field
            expectedSelection = selection(input)
            prepared = true
        }
        guard let view, view.isFirstResponder, let input = view as? UITextInput,
              let marked = input.markedTextRange, input.text(in: marked) == "ceshi",
              selection(input) == expectedSelection else { return "changed" }
        return "preserved"
    }

    private func selection(_ input: UITextInput) -> [Int] {
        guard let range = input.selectedTextRange else { return [] }
        return [input.offset(from: input.beginningOfDocument, to: range.start),
                input.offset(from: input.beginningOfDocument, to: range.end)]
    }
}
#endif
