import XCTest
import AHProtocol
@testable import AHCore

/// The model and the effort of the new task form (`docs/specs/ios-task-model.md`): what the form offers,
/// how the two follow each other, and what reaches the draft.
@MainActor
final class TaskComposerModelTests: XCTestCase {
    private let five = ["low", "medium", "high", "xhigh", "max"]

    /// Claude (the same efforts for every model), Codex (efforts per model, one without), Cursor (models
    /// but no efforts) and Grok (what an older Mac sends: no models at all).
    private func options() -> TaskOptions {
        TaskOptions(
            enabled: true, permissionMode: "agentDefault",
            workspaces: [WorkspaceOption(id: "ws", label: "Project", subtitle: "~/p", pinned: true)],
            agents: [
                AgentOption(
                    kind: "claude", label: "Claude Code", ready: true, subtitle: "", permissionSelectable: false,
                    models: [ModelOption(id: "opus", label: "Opus", efforts: five), ModelOption(id: "haiku", label: "Haiku", efforts: five)],
                    defaultModel: DefaultModel(label: "opus", efforts: five)
                ),
                AgentOption(
                    kind: "codex", label: "Codex", ready: true, subtitle: "", permissionSelectable: false,
                    models: [
                        ModelOption(id: "big", label: "Big", efforts: ["low", "medium", "high", "xhigh"], defaultEffort: "medium"),
                        ModelOption(id: "small", label: "Small", efforts: ["medium", "high"], defaultEffort: "medium"),
                        ModelOption(id: "plain", label: "Plain"),
                    ],
                    defaultModel: DefaultModel(label: "Big", efforts: ["low", "medium", "high", "xhigh"], defaultEffort: "high")
                ),
                AgentOption(
                    kind: "cursor", label: "Cursor", ready: true, subtitle: "", permissionSelectable: false,
                    models: [ModelOption(id: "c1", label: "C1")], defaultModel: DefaultModel(label: "Auto")
                ),
                AgentOption(kind: "grok", label: "Grok", ready: true, subtitle: "", permissionSelectable: false),
            ]
        )
    }

    private func defaults() -> UserDefaults { UserDefaults(suiteName: "ah-test-" + UUID().uuidString)! }

    private func makeComposer(on kind: String, _ choices: TaskOptions? = nil) -> TaskComposer {
        let composer = TaskComposer(macId: "mac")
        composer.applyDefaults(options: choices ?? options(), defaults: defaults())
        composer.selectAgent(kind)
        composer.text = "Do it"
        return composer
    }

    // MARK: what the form offers

    func testAnAgentWithoutModelsHasNeitherRow() {
        let choices = options()
        let composer = makeComposer(on: "grok")
        XCTAssertFalse(composer.offersModelChoice(in: choices))
        XCTAssertTrue(composer.models(in: choices).isEmpty)
        XCTAssertNil(composer.effortChoices(in: choices))
        composer.selectModel("opus", in: choices)
        XCTAssertNil(composer.model(in: choices), "there is nothing to pick")
        let draft = composer.draft(options: choices)
        XCTAssertNotNil(draft)
        XCTAssertNil(draft?.model)
        XCTAssertNil(draft?.effort)
    }

    func testTheFormStartsFromDefault() throws {
        let choices = options()
        let composer = TaskComposer(macId: "mac")
        composer.applyDefaults(options: choices, defaults: defaults())
        XCTAssertEqual(composer.agentKind, "claude")
        XCTAssertNil(composer.modelId)
        XCTAssertNil(composer.model(in: choices))
        XCTAssertNil(composer.effort(in: choices))
        XCTAssertNil(composer.draft(options: choices), "no text yet")
        composer.text = "Go"
        let draft = try XCTUnwrap(composer.draft(options: choices))
        XCTAssertNil(draft.model)
        XCTAssertNil(draft.effort)
    }

    func testModelsWithoutEffortsGiveNoEffortRow() {
        let choices = options()
        let cursor = makeComposer(on: "cursor")
        XCTAssertTrue(cursor.offersModelChoice(in: choices))
        XCTAssertNil(cursor.effortChoices(in: choices), "Cursor folds the effort into the model id")
        cursor.selectModel("c1", in: choices)
        XCTAssertEqual(cursor.model(in: choices)?.label, "C1")
        XCTAssertNil(cursor.effortChoices(in: choices))

        let codex = makeComposer(on: "codex")
        codex.selectModel("plain", in: choices)
        XCTAssertNil(codex.effortChoices(in: choices), "a model that names no efforts has no effort row")
    }

    func testTheEffortsFollowTheModelThatIsPicked() {
        let choices = options()
        let composer = makeComposer(on: "codex")
        XCTAssertEqual(
            composer.effortChoices(in: choices), EffortChoices(levels: ["low", "medium", "high", "xhigh"], defaultLevel: "high"),
            "with Default: what the model Default stands for offers, and runs with"
        )
        composer.selectModel("small", in: choices)
        XCTAssertEqual(composer.effortChoices(in: choices), EffortChoices(levels: ["medium", "high"], defaultLevel: "medium"))
        composer.selectModel("plain", in: choices)
        XCTAssertNil(composer.effortChoices(in: choices))
        composer.selectModel(nil, in: choices)
        XCTAssertEqual(composer.effortChoices(in: choices)?.levels, ["low", "medium", "high", "xhigh"])
    }

    func testTheDefaultLevelIsOnlyKnownWhenItIsOneOfTheLevels() {
        XCTAssertEqual(EffortChoices(levels: ["low", "high"], defaultLevel: "high").defaultLevel, "high")
        XCTAssertNil(EffortChoices(levels: ["low", "high"], defaultLevel: "max").defaultLevel)
        XCTAssertNil(EffortChoices(levels: ["low", "high"]).defaultLevel)
    }

    // MARK: how the two follow each other

    func testAnEffortSurvivesAModelThatOffersItAndGoesBackToDefaultOtherwise() {
        let choices = options()
        let composer = makeComposer(on: "codex")
        composer.selectModel("big", in: choices)
        composer.effortId = "high"
        composer.selectModel("small", in: choices)
        XCTAssertEqual(composer.effort(in: choices), "high", "Small offers it too")

        composer.selectModel("big", in: choices)
        composer.effortId = "xhigh"
        composer.selectModel("small", in: choices)
        XCTAssertNil(composer.effort(in: choices), "Small has no xhigh")
        XCTAssertNil(composer.effortId, "and nothing is kept for later")
        composer.selectModel("big", in: choices)
        XCTAssertNil(composer.effort(in: choices), "going back does not bring it back")

        composer.effortId = "low"
        composer.selectModel("plain", in: choices)
        XCTAssertNil(composer.effortId, "a model without efforts leaves none")
    }

    func testPickingDefaultAgainKeepsAnEffortThatTheDefaultModelOffers() {
        let choices = options()
        let composer = makeComposer(on: "codex")
        composer.selectModel("big", in: choices)
        composer.effortId = "xhigh"
        composer.selectModel(nil, in: choices)
        XCTAssertEqual(composer.effort(in: choices), "xhigh", "the model Default stands for has it")
    }

    func testAnotherAgentStartsFromDefaultButTheSameAgentKeepsThePick() {
        let choices = options()
        let composer = makeComposer(on: "codex")
        composer.selectModel("small", in: choices)
        composer.effortId = "high"
        composer.selectAgent("codex")
        XCTAssertEqual(composer.model(in: choices)?.id, "small", "the same Agent keeps it")
        XCTAssertEqual(composer.effort(in: choices), "high")

        composer.selectAgent("claude")
        XCTAssertNil(composer.modelId)
        XCTAssertNil(composer.effortId)
        composer.selectAgent("codex")
        XCTAssertNil(composer.modelId, "and coming back does not restore it")
        XCTAssertNil(composer.effort(in: choices))
    }

    func testSwitchingTheMacForgetsThePick() {
        let choices = options()
        let composer = makeComposer(on: "claude")
        composer.selectModel("haiku", in: choices)
        composer.effortId = "max"
        composer.switchMac(to: "other")
        XCTAssertNil(composer.modelId)
        XCTAssertNil(composer.effortId)
    }

    // MARK: remembered preferences and stale choices

    func testTheSubmittedPickSurvivesANewDefaultsInstanceAndAnotherMac() throws {
        let choices = options()
        let suite = "ah-test-" + UUID().uuidString
        let store = UserDefaults(suiteName: suite)!
        defer { store.removePersistentDomain(forName: suite) }
        let first = TaskComposer(macId: "mac")
        first.applyDefaults(options: choices, defaults: store)
        first.selectAgent("codex")
        first.selectModel("big", in: choices)
        first.effortId = "xhigh"
        first.remember(options: choices, defaults: store)

        let second = TaskComposer(macId: "other-mac")
        second.applyDefaults(options: choices, defaults: UserDefaults(suiteName: suite)!)
        second.selectAgent("codex")
        second.applyDefaults(options: choices, defaults: UserDefaults(suiteName: suite)!)
        XCTAssertEqual(second.modelId, "big", "this phone shares the Agent's preference across Macs")
        XCTAssertEqual(second.effortId, "xhigh")
        second.text = "Go"
        let draft = try XCTUnwrap(second.draft(options: choices))
        XCTAssertEqual(draft.model, "big")
        XCTAssertEqual(draft.effort, "xhigh")
    }

    func testEachAgentHasItsOwnPreferenceAndReturningRestoresTheSubmittedPick() {
        let choices = options()
        let store = defaults()
        let composer = TaskComposer(macId: "mac")
        composer.applyDefaults(options: choices, defaults: store)
        composer.selectModel("haiku", in: choices)
        composer.effortId = "max"
        composer.remember(options: choices, defaults: store)

        composer.selectAgent("codex")
        composer.applyDefaults(options: choices, defaults: store)
        XCTAssertNil(composer.modelId, "Claude's model does not carry over")
        composer.selectModel("small", in: choices)
        composer.effortId = "high"
        composer.remember(options: choices, defaults: store)
        composer.selectModel("big", in: choices)

        composer.selectAgent("claude")
        composer.applyDefaults(options: choices, defaults: store)
        XCTAssertEqual(composer.modelId, "haiku")
        XCTAssertEqual(composer.effortId, "max")
        composer.selectAgent("codex")
        composer.applyDefaults(options: choices, defaults: store)
        XCTAssertEqual(composer.modelId, "small", "an unsubmitted edit did not replace the preference")
        XCTAssertEqual(composer.effortId, "high")
    }

    func testRefreshingOptionsDoesNotOverwriteEditsOrAnExplicitDefault() {
        let choices = options()
        let store = defaults()
        let composer = makeComposer(on: "codex")
        composer.selectModel("big", in: choices)
        composer.effortId = "xhigh"
        composer.remember(options: choices, defaults: store)

        let next = TaskComposer(macId: "mac")
        next.applyDefaults(options: choices, defaults: store)
        next.selectModel(nil, in: choices)
        next.effortId = nil
        next.applyDefaults(options: choices, defaults: store)
        XCTAssertNil(next.modelId)
        XCTAssertNil(next.effortId)
        next.remember(options: choices, defaults: store)

        let reopened = TaskComposer(macId: "mac")
        reopened.applyDefaults(options: choices, defaults: store)
        XCTAssertNil(reopened.modelId, "submitting Default clears the saved model")
        XCTAssertNil(reopened.effortId)
    }

    func testAnUnavailableSavedModelFallsBackWithoutErasingThePreference() {
        let choices = options()
        let store = defaults()
        let first = makeComposer(on: "codex")
        first.selectModel("big", in: choices)
        first.effortId = "high"
        first.remember(options: choices, defaults: store)

        var limited = choices
        limited.agents[1].models?.removeAll { $0.id == "big" }
        let other = TaskComposer(macId: "other-mac")
        other.selectAgent("codex")
        other.applyDefaults(options: limited, defaults: store)
        other.text = "Go"
        XCTAssertNil(other.draft(options: limited)?.model)
        XCTAssertNil(other.effortId, "the effort of the missing model also falls back")
        other.remember(options: limited, defaults: store)

        other.switchMac(to: "mac")
        other.applyDefaults(options: choices, defaults: store)
        XCTAssertEqual(other.modelId, "big", "the original Mac still offers this phone's preference")
        XCTAssertEqual(other.effortId, "high")

        other.switchMac(to: "other-mac")
        other.selectAgent("codex")
        other.applyDefaults(options: limited, defaults: store)
        other.selectModel(nil, in: limited)
        other.remember(options: limited, defaults: store)
        let next = TaskComposer(macId: "mac")
        next.applyDefaults(options: choices, defaults: store)
        XCTAssertNil(next.modelId, "explicitly choosing Default replaces even an unavailable preference")
    }

    func testAnOlderMacDoesNotEraseTheSavedModelOrEffort() {
        let choices = options()
        let store = defaults()
        let first = makeComposer(on: "codex")
        first.selectModel("small", in: choices)
        first.effortId = "high"
        first.remember(options: choices, defaults: store)
        var old = choices
        old.agents[1].models = nil
        old.agents[1].defaultModel = nil
        let other = TaskComposer(macId: "old-mac")
        other.selectAgent("codex")
        other.applyDefaults(options: old, defaults: store)
        other.remember(options: old, defaults: store)

        let next = TaskComposer(macId: "mac")
        next.applyDefaults(options: choices, defaults: store)
        XCTAssertEqual(next.modelId, "small")
        XCTAssertEqual(next.effortId, "high")
    }

    func testAnUnavailableSavedEffortKeepsTheModelAndUsesDefaultEffort() {
        let choices = options()
        let store = defaults()
        let first = makeComposer(on: "codex")
        first.selectModel("big", in: choices)
        first.effortId = "xhigh"
        first.remember(options: choices, defaults: store)
        var changed = choices
        changed.agents[1].models?[0].efforts = ["medium", "high"]
        let next = TaskComposer(macId: "mac")
        next.applyDefaults(options: changed, defaults: store)
        XCTAssertEqual(next.modelId, "big")
        XCTAssertNil(next.effortId)
    }

    func testAPickThatTheMacNoLongerOffersGoesBackToDefault() {
        let choices = options()
        let composer = makeComposer(on: "codex")
        composer.selectModel("big", in: choices)
        composer.effortId = "xhigh"

        var refreshed = choices
        refreshed.agents[1].models = [ModelOption(id: "small", label: "Small", efforts: ["medium", "high"], defaultEffort: "medium")]
        refreshed.agents[1].defaultModel = DefaultModel(label: "Small", efforts: ["medium", "high"], defaultEffort: "medium")
        composer.applyDefaults(options: refreshed, defaults: defaults())
        XCTAssertNil(composer.modelId, "Big is gone")
        XCTAssertNil(composer.effortId, "xhigh is not offered any more")

        composer.selectModel("small", in: refreshed)
        composer.effortId = "high"
        var thinner = refreshed
        thinner.agents[1].models = [ModelOption(id: "small", label: "Small")]
        composer.applyDefaults(options: thinner, defaults: defaults())
        XCTAssertEqual(composer.modelId, "small", "the model is still there")
        XCTAssertNil(composer.effortId, "but it names no efforts now")
    }

    func testAnEffortThatIsNotOfferedIsNeverSent() {
        let choices = options()
        let composer = makeComposer(on: "claude")
        composer.effortId = "bogus"
        XCTAssertNil(composer.effort(in: choices))
        XCTAssertNil(composer.draft(options: choices)?.effort)
        composer.effortId = "max"
        XCTAssertEqual(composer.draft(options: choices)?.effort, "max")
    }

    // MARK: the draft

    func testTheDraftCarriesWhatWasPickedAndTheNameOfTheModel() throws {
        let choices = options()
        let composer = makeComposer(on: "codex")
        composer.selectModel("small", in: choices)
        composer.effortId = "high"
        let draft = try XCTUnwrap(composer.draft(options: choices))
        XCTAssertEqual(draft.model, "small")
        XCTAssertEqual(draft.effort, "high")
        XCTAssertEqual(draft.modelLabel, "Small")
        XCTAssertEqual(draft.agentKind, "codex")
    }

    func testAnEffortAloneIsSentWithoutAModel() throws {
        let choices = options()
        let composer = makeComposer(on: "claude")
        composer.effortId = "max"
        let draft = try XCTUnwrap(composer.draft(options: choices))
        XCTAssertNil(draft.model, "Default sends no model")
        XCTAssertNil(draft.modelLabel)
        XCTAssertEqual(draft.effort, "max")
    }

    func testNothingPickedSendsNothing() throws {
        let choices = options()
        let composer = makeComposer(on: "claude")
        let draft = try XCTUnwrap(composer.draft(options: choices))
        XCTAssertNil(draft.model)
        XCTAssertNil(draft.effort)
        XCTAssertNil(draft.modelLabel)
    }

    func testAModelTheChoicesDoNotOfferNeverReachesTheDraft() throws {
        let choices = options()
        let composer = makeComposer(on: "codex")
        composer.selectModel("big", in: choices)
        composer.effortId = "xhigh"
        var changed = choices
        changed.agents[1].models = [ModelOption(id: "small", label: "Small")]
        changed.agents[1].defaultModel = DefaultModel(label: "Small")
        let draft = try XCTUnwrap(composer.draft(options: changed))
        XCTAssertNil(draft.model, "the form shows Default for it, so Default is what is sent")
        XCTAssertNil(draft.effort)
    }
}
