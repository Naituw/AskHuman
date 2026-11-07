import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The developer mode's switch, what it remembers, and how a quick-add ends (docs/specs/ios-developer-mode.md).
@MainActor
final class DeveloperToolsTests: XCTestCase {
    private var backend: MockBackend!
    private var app: AppStore!
    private var defaults: UserDefaults!
    private var developer: DeveloperTools { app.developer }

    private func make(defaults existing: UserDefaults? = nil) async {
        backend = MockBackend(scenario: .demo, latency: 0)
        defaults = existing ?? UserDefaults(suiteName: "ah-developer-test-" + UUID().uuidString)!
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await app.start()
        await eventually("the Macs") { self.app.state.foreground && self.app.state.macs.count == 2 }
    }

    private func chooseOfficeWork() async {
        await developer.quickTodo.loadProjects()
        let office = developer.quickTodo.sources.first { $0.source == .mac(MockData.officeMacId) }
        guard case .projects(let rows)? = office?.state, let project = rows.first(where: { $0.id == MockData.todoOfficeWorkProject }) else {
            return XCTFail("the Office iMac lists no AskHuman · work")
        }
        developer.quickTodo.choose(project, in: .mac(MockData.officeMacId))
    }

    // MARK: the switch

    func testItIsOffUntilTheSwitchIsTurnedOnAndTheSwitchIsRemembered() async {
        await make()
        XCTAssertFalse(developer.enabled)
        XCTAssertFalse(app.developerMode)

        app.developerMode = true
        XCTAssertTrue(developer.enabled, "the settings toggle and the mode are one switch")

        let defaults = self.defaults!
        await make(defaults: defaults)
        XCTAssertTrue(developer.enabled, "the next launch starts the way this one ended")

        developer.enabled = false
        await make(defaults: defaults)
        XCTAssertFalse(developer.enabled)
    }

    func testTurningTheModeOffClosesWhatItHadOpen() async {
        await make()
        developer.enabled = true
        developer.presentQuickTodo()
        developer.showToast(.todoAdded(project: "AskHuman"))
        XCTAssertTrue(developer.quickTodoPresented)
        XCTAssertNotNil(developer.toast)

        developer.enabled = false
        XCTAssertFalse(developer.quickTodoPresented)
        XCTAssertNil(developer.toast)
    }

    func testTheQuickAddPageOpensOnlyWhileTheModeIsOn() async {
        await make()
        developer.invoke(.quickTodo)
        XCTAssertFalse(developer.quickTodoPresented)

        developer.enabled = true
        developer.invoke(.quickTodo)
        XCTAssertTrue(developer.quickTodoPresented)

        developer.dismissQuickTodo()
        XCTAssertFalse(developer.quickTodoPresented)
    }

    // MARK: the diagnostics (docs/specs/ios-developer-mode.md §12)

    func testTheMenuListsTheQuickTodoAndTheDiagnosticsInThatOrder() {
        XCTAssertEqual(DeveloperMenuItem.allCases, [.quickTodo, .copyDiagnostics])
    }

    func testCopyingDiagnosticsHandsTheReportToTheSinkAndSaysSo() async {
        await make()
        var copied: [String] = []
        developer.diagnosticsSink = { copied.append($0) }

        developer.invoke(.copyDiagnostics)
        XCTAssertTrue(copied.isEmpty, "with the mode off nothing is copied")
        XCTAssertNil(developer.toast)

        developer.enabled = true
        developer.invoke(.copyDiagnostics)
        XCTAssertEqual(copied.count, 1)
        XCTAssertTrue(copied.first?.hasPrefix("AskHuman diagnostics") == true)
        XCTAssertEqual(developer.toast?.message, .diagnosticsCopied)
        XCTAssertTrue(
            Diagnostics.shared.entries().contains { $0.category == "dev" && $0.text.hasPrefix("diagnostics copied") },
            "the copy is itself in the log, so that the next report shows where this one was taken"
        )
    }

    func testCopyingDiagnosticsWithNowhereToPutThemChangesNothing() async {
        await make()
        developer.enabled = true
        developer.invoke(.copyDiagnostics)
        XCTAssertNil(developer.toast)
    }

    func testTheStoreWritesWhereTheUserWentAndWhatEndedUnderThem() async {
        await make()
        app.tab = .agents
        app.tab = .inbox
        XCTAssertTrue(Diagnostics.shared.entries().contains { $0.category == "nav" && $0.text == "tab inbox → agents" })
        XCTAssertTrue(Diagnostics.shared.entries().contains { $0.category == "nav" && $0.text == "tab agents → inbox" })

        let before = Diagnostics.shared.entries().count
        app.tab = .inbox
        XCTAssertEqual(Diagnostics.shared.entries().count, before, "choosing the tab that is already chosen says nothing")
    }

    func testTheSummaryOfTheStateHasCountsAndStatesAndNoNames() async {
        await make()
        let summary = app.diagnosticSummary()
        XCTAssertTrue(summary.contains("tab inbox; inbox stack 0, agents stack 0"), summary)
        XCTAssertTrue(summary.contains("macs 2"), summary)
        XCTAssertTrue(summary.contains("developer mode false"), summary)
        for name in app.state.macs.map(\.name) + app.state.macs.map(\.macName) + app.state.inbox.map(\.id) {
            XCTAssertFalse(summary.contains(name), "the summary must not carry \(name)")
        }
    }

    // MARK: the button's place

    func testTheButtonStaysWhereItWasLeftAndAPlaceThatCannotBeReadIsTheStandardOne() async {
        await make()
        XCTAssertEqual(developer.placement, .standard)

        developer.placement = DeveloperPlacement(edge: .left, y: 0.7)
        let defaults = self.defaults!
        await make(defaults: defaults)
        XCTAssertEqual(developer.placement, DeveloperPlacement(edge: .left, y: 0.7))

        defaults.set(Data("garbage".utf8), forKey: "ah.dev.placement")
        await make(defaults: defaults)
        XCTAssertEqual(developer.placement, .standard)
    }

    func testThePlaceIsKeptWhileTheModeIsOff() async {
        await make()
        developer.enabled = true
        developer.placement = DeveloperPlacement(edge: .left, y: 0.2)
        developer.enabled = false
        developer.enabled = true
        XCTAssertEqual(developer.placement, DeveloperPlacement(edge: .left, y: 0.2))
    }

    // MARK: how a quick-add ends

    func testWhenTheMacTakesTheTodoThePageGoesAndANoteSaysWhere() async {
        await make()
        developer.enabled = true
        await chooseOfficeWork()
        developer.presentQuickTodo()
        developer.quickTodo.text = "From the couch"

        await developer.submitQuickTodo()

        XCTAssertFalse(developer.quickTodoPresented)
        XCTAssertEqual(developer.toast?.message, .todoAdded(project: "AskHuman"))
        XCTAssertEqual(developer.quickTodo.text, "")
    }

    func testWhenItDoesNotThePageStaysWithItsTextAndThereIsNoNote() async {
        await make()
        developer.enabled = true
        await chooseOfficeWork()
        developer.presentQuickTodo()
        developer.quickTodo.text = "Will not be saved"
        await backend.setFailNextTodoWrite("persist")

        await developer.submitQuickTodo()

        XCTAssertTrue(developer.quickTodoPresented)
        XCTAssertNil(developer.toast)
        XCTAssertEqual(developer.quickTodo.text, "Will not be saved")
        XCTAssertEqual(developer.quickTodo.failure, .notSaved)
    }

    func testSubmittingWhatCannotBeSentChangesNothing() async {
        await make()
        developer.enabled = true
        developer.presentQuickTodo()
        developer.quickTodo.text = "No project chosen yet"

        await developer.submitQuickTodo()

        XCTAssertTrue(developer.quickTodoPresented)
        XCTAssertNil(developer.toast)
    }

    // MARK: the note

    func testANoteIsClearedByItsOwnIdAndNotByAnOlderOne() async throws {
        await make()
        developer.showToast(.todoAdded(project: "first"))
        let first = try XCTUnwrap(developer.toast?.id)
        developer.showToast(.todoAdded(project: "second"))
        let second = try XCTUnwrap(developer.toast?.id)
        XCTAssertNotEqual(first, second)

        developer.clearToast(id: first)
        XCTAssertEqual(developer.toast?.message, .todoAdded(project: "second"), "the first note's timer must not remove the second")

        developer.clearToast(id: second)
        XCTAssertNil(developer.toast)
    }

    func testANoteStaysForTheUsualMomentUnlessTheToolsAreToldToHoldItLonger() async throws {
        await make()
        developer.showToast(.diagnosticsCopied)
        XCTAssertEqual(try XCTUnwrap(developer.toast).hold, DeveloperMetrics.toastDuration)

        developer.toastHold = 6
        developer.showToast(.diagnosticsCopied)
        XCTAssertEqual(try XCTUnwrap(developer.toast).hold, 6, "UI tests stretch it for a busy simulator")
    }
}
