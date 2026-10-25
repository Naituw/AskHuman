import XCTest
import AHProtocol
@testable import AHCore

@MainActor
final class ConfirmComposerTests: XCTestCase {
    private func confirm(input: ConfirmInputView?, dismissIndex: Int? = 2) -> ConfirmView {
        ConfirmView(
            title: "Allow?", context: [], detail: ConfirmDetailView(summary: "s", bodyMd: "b"),
            choices: [
                ConfirmChoiceView(wireIndex: 0, label: "Allow", description: "", role: "primary", recommended: true),
                ConfirmChoiceView(wireIndex: 2, label: "Deny", description: "", role: "destructive", recommended: false),
            ],
            input: input, dismissIndex: dismissIndex, submitLabel: "Submit", expiresAt: 4_102_444_800_000
        )
    }

    func testNothingIsPreselectedAndSubmitNeedsAChoice() {
        let composer = ConfirmComposer(confirm: confirm(input: nil))
        XCTAssertNil(composer.selectedWireIndex)
        XCTAssertFalse(composer.canSubmit)
        composer.select(wireIndex: 0)
        XCTAssertTrue(composer.canSubmit)
        XCTAssertEqual(composer.selectedChoice?.label, "Allow")
    }

    func testUnknownChoicesAreIgnored() {
        let composer = ConfirmComposer(confirm: confirm(input: nil))
        composer.select(wireIndex: 7)
        XCTAssertNil(composer.selectedWireIndex)
    }

    func testTheInputOnlyAppearsForItsChoice() {
        let field = ConfirmInputView(id: "reason", label: "Reason", placeholder: "", maxChars: 20, visibleWhenIndex: 2, required: false)
        let composer = ConfirmComposer(confirm: confirm(input: field))
        XCTAssertNil(composer.visibleInput)
        composer.select(wireIndex: 0)
        XCTAssertNil(composer.visibleInput)
        composer.select(wireIndex: 2)
        XCTAssertEqual(composer.visibleInput?.id, "reason")
    }

    func testAnAlwaysVisibleInputShowsWithoutAChoice() {
        let field = ConfirmInputView(id: "note", label: "Note", placeholder: "", maxChars: 20)
        let composer = ConfirmComposer(confirm: confirm(input: field))
        XCTAssertNotNil(composer.visibleInput)
    }

    func testARequiredInputBlocksSubmittingUntilFilled() {
        let field = ConfirmInputView(id: "reason", label: "Reason", placeholder: "", maxChars: 20, visibleWhenIndex: 2, required: true)
        let composer = ConfirmComposer(confirm: confirm(input: field))
        composer.select(wireIndex: 0)
        XCTAssertTrue(composer.canSubmit, "the input is not shown for Allow, so it cannot be required")
        composer.select(wireIndex: 2)
        XCTAssertFalse(composer.canSubmit)
        composer.input = "  \n "
        XCTAssertFalse(composer.canSubmit)
        composer.input = " too risky "
        XCTAssertTrue(composer.canSubmit)
        XCTAssertEqual(composer.inputToSend, "too risky")
    }

    func testInputIsTrimmedCappedAndDroppedWhenHidden() {
        let field = ConfirmInputView(id: "reason", label: "Reason", placeholder: "", maxChars: 5, visibleWhenIndex: 2, required: false)
        let composer = ConfirmComposer(confirm: confirm(input: field))
        composer.input = "abcdefgh"
        composer.select(wireIndex: 0)
        XCTAssertNil(composer.inputToSend, "typed text is not sent along with another choice")
        composer.select(wireIndex: 2)
        XCTAssertEqual(composer.inputToSend, "abcde")
        composer.input = "   "
        XCTAssertNil(composer.inputToSend)
    }

    func testDismissingIsOnlyOfferedWhenTheMacMapsIt() {
        XCTAssertTrue(ConfirmComposer(confirm: confirm(input: nil)).canDismiss)
        XCTAssertFalse(ConfirmComposer(confirm: confirm(input: nil, dismissIndex: nil)).canDismiss)
    }

    func testSendingBlocksASecondSubmit() {
        let composer = ConfirmComposer(confirm: confirm(input: nil))
        composer.select(wireIndex: 0)
        composer.isSending = true
        XCTAssertFalse(composer.canSubmit)
    }
}
