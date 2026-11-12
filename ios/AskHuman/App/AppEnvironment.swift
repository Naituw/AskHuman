import SwiftUI
import UIKit
import AHCore
import AHMock
import AHProtocol
import AHStore
import AHTransport

/// Options for automated runs (UI tests, screenshots): `-AHMock YES` swaps the engine for the mock
/// backend; the others pick the scenario and the first screen.
struct LaunchOptions {
    var mock = false
    var scenario: MockBackend.Scenario = .demo
    var latency = 1.0
    var tab: AppTab?
    var openRequest: String?
    /// An Agent (its key, or `macId|key`) whose page opens first.
    var openAgent: String?
    /// An Agent (its key, or `macId|key`) to follow as soon as a Mac lists it, in a run on the real engine:
    /// the app has no follow button (the Macs follow the tasks the phone starts), the live UI test asks.
    var followAgent: String?
    /// Opens the new task sheet first.
    var newTask = false
    var taskAttachmentDemo = false
    var noTaskAttachments = false
    var failTaskUpload = false
    /// The mock Agents never pick up the messages sent to them (UI tests that withdraw one).
    var holdInterjects = false
    /// The mock Macs list no models, like an older Mac: the new task form has no model or effort row.
    var noModels = false
    /// The mock Mac controls Cursor's Max mode (UI tests of the Allow Max switch).
    var maxControl = false
    /// The next new task of the mock fails with this reason (the texts of the progress page).
    var failTask: String?
    /// A followed Agent reports `done`, `failed` or `waiting` a moment after launch (the follow banner).
    var followEvent: String?
    /// A request arrives a moment after launch (the arrival animation), from a session with this title;
    /// `none` for one without a title.
    var arrival: String?
    var arrivalCount = 1
    var arrivalDelay = 2.0
    /// The request `-AHOpen` names ends this many seconds after launch, as if it had been settled somewhere else
    /// (the ended request page). `-AHEndHow` says how (`answeredElsewhere`, `cancelled`, `expired`, `ended`) and
    /// `-AHEndBy` who answered it (a channel's name, as a Mac reports it: "Popup", "Slack").
    var endAfter: Double?
    /// Resolves the mock after entering the background, with a bounded background execution lease.
    var endInBackground = false
    var endHow: HistoryOutcome = .answeredElsewhere
    var endBy: String?
    var arrivalMotionDiagnostics = false
    var inputCaretDiagnostics = false
    var arrivalMotionSlowdown = 1.0
    var arrivalFullscreenFixture = false
    var arrivalMarkedTextFixture = false
    var arrivalReducedMotion = false
    /// Forces "open the next request after sending" on or off (UI tests need a fixed value).
    var autoAdvance: Bool?
    /// Forces the developer mode (the floating button) on or off.
    var developerMode: Bool?
    /// Seconds (real time) after which the `syncing` scenario turns into the demo; unset keeps it syncing.
    var syncDelay: Double?
    /// The `syncing` scenario starts with the requests the phone kept from the last run.
    var cachedInbox = false
    /// Seconds the receipt of a sent request stays up, and the next page takes to settle (UI tests
    /// stretch them to have time to look; the app's own values are made for the eye).
    var advanceHold: Double?
    var advanceSettle: Double?
    /// Seconds the countdown on the strip of an ended request page takes (the app's own 5 is too short for a UI test
    /// that has to look at the strip and press a button on a busy simulator).
    var endedCountdown: Double?
    /// Seconds the "Added to <project>" note stays over a request page (the app's own 2 is too short for a
    /// UI test that polls a busy simulator).
    var todoNoteHold: Double?
    /// Seconds the developer button's note ("Added to …", "Diagnostics copied") stays up (the app's own 1.8 is too
    /// short for a UI test that polls a busy simulator).
    var developerToastHold: Double?
    /// A request (on the first Mac) whose notification was tapped: its page opens at once, whether or
    /// not the Macs have answered yet, with a made-up notification text.
    var notifyRequest: String?
    /// `-AHWebMarkdown NO` draws every Markdown body with the native renderer, as the app does when its web
    /// page cannot be used (UI tests take that road on purpose).
    var webMarkdown = true
    /// Seconds a Markdown page may take to draw before the native renderer takes over: 5 on a phone. The
    /// simulator needs seconds to start web content the first time (more than 10 on a busy Mac), and
    /// falling back there would hide the page from whoever is looking at it, so it waits longer.
    #if targetEnvironment(simulator)
    var webTimeout = 30.0
    #else
    var webTimeout = 5.0
    #endif
    /// `-AHAnnotate YES`: every request of the mock takes notes on its message, as the newest Macs allow (UI tests of the
    /// answer bar, whose forms are not in the `annotations` scenario).
    var annotate = false
    /// `-AHNotes 3`: the request `-AHOpen` names starts with this many notes written on its message (the number on the bar's
    /// "Annotations" button). It has to take notes: with `-AHScenario annotations`, or `-AHAnnotate YES`.
    var notes = 0
    /// `-AHHistoryFail 1`: the next reads of a session's history get no answer from the (mock) Mac, that many times.
    var historyFail = 0
    /// `-AHHistoryDelay 1.5`: a read of a session's history takes this many real seconds (the pull's UI tests).
    var historyDelay: Double?
    /// `-AHHistoryGate <path>`: a read of a session's history is held back until that file exists.
    var historyGate: String?

    init(defaults: UserDefaults = .standard) {
        mock = defaults.bool(forKey: "AHMock")
        if let name = defaults.string(forKey: "AHScenario"), let scenario = MockBackend.Scenario(rawValue: name) {
            self.scenario = scenario
        }
        if defaults.object(forKey: "AHLatency") != nil { latency = defaults.double(forKey: "AHLatency") }
        tab = defaults.string(forKey: "AHTab").flatMap(AppTab.init(rawValue:))
        openRequest = defaults.string(forKey: "AHOpen")
        openAgent = defaults.string(forKey: "AHOpenAgent")
        followAgent = defaults.string(forKey: "AHFollowAgent")
        newTask = defaults.bool(forKey: "AHNewTask")
        taskAttachmentDemo = defaults.bool(forKey: "AHTaskAttachments")
        noTaskAttachments = defaults.bool(forKey: "AHNoTaskAttachments")
        failTaskUpload = defaults.bool(forKey: "AHFailTaskUpload")
        holdInterjects = defaults.bool(forKey: "AHHoldInterjects")
        noModels = defaults.bool(forKey: "AHNoModels")
        maxControl = defaults.bool(forKey: "AHMaxControl")
        failTask = defaults.string(forKey: "AHFailTask")
        followEvent = defaults.string(forKey: "AHFollowEvent")
        arrival = defaults.string(forKey: "AHArrival")
        arrivalCount = min(20, max(1, defaults.integer(forKey: "AHArrivalCount")))
        if defaults.object(forKey: "AHArrivalDelay") != nil { arrivalDelay = max(0, defaults.double(forKey: "AHArrivalDelay")) }
        if defaults.object(forKey: "AHEndAfter") != nil { endAfter = max(0, defaults.double(forKey: "AHEndAfter")) }
        endInBackground = defaults.bool(forKey: "AHEndInBackground")
        if let how = defaults.string(forKey: "AHEndHow").flatMap(HistoryOutcome.init(rawValue:)) { endHow = how }
        endBy = defaults.string(forKey: "AHEndBy")
        arrivalMotionDiagnostics = defaults.bool(forKey: "AHArrivalMotionDiagnostics")
        inputCaretDiagnostics = defaults.bool(forKey: "AHInputCaretDiagnostics")
        arrivalFullscreenFixture = defaults.bool(forKey: "AHArrivalFullscreenFixture")
        arrivalMarkedTextFixture = defaults.bool(forKey: "AHArrivalMarkedTextFixture")
        arrivalReducedMotion = defaults.bool(forKey: "AHArrivalReducedMotion")
        if defaults.object(forKey: "AHArrivalMotionSlowdown") != nil { arrivalMotionSlowdown = max(1, defaults.double(forKey: "AHArrivalMotionSlowdown")) }
        if defaults.object(forKey: "AHAutoAdvance") != nil { autoAdvance = defaults.bool(forKey: "AHAutoAdvance") }
        if defaults.object(forKey: "AHDeveloperMode") != nil { developerMode = defaults.bool(forKey: "AHDeveloperMode") }
        if defaults.object(forKey: "AHSyncDelay") != nil { syncDelay = defaults.double(forKey: "AHSyncDelay") }
        cachedInbox = defaults.bool(forKey: "AHCached")
        if defaults.object(forKey: "AHAdvanceHold") != nil { advanceHold = defaults.double(forKey: "AHAdvanceHold") }
        if defaults.object(forKey: "AHAdvanceSettle") != nil { advanceSettle = defaults.double(forKey: "AHAdvanceSettle") }
        if defaults.object(forKey: "AHEndedCountdown") != nil { endedCountdown = max(0.1, defaults.double(forKey: "AHEndedCountdown")) }
        if defaults.object(forKey: "AHTodoNoteHold") != nil { todoNoteHold = max(0, defaults.double(forKey: "AHTodoNoteHold")) }
        if defaults.object(forKey: "AHDeveloperToastHold") != nil { developerToastHold = max(0, defaults.double(forKey: "AHDeveloperToastHold")) }
        notifyRequest = defaults.string(forKey: "AHNotify")
        if defaults.object(forKey: "AHWebMarkdown") != nil { webMarkdown = defaults.bool(forKey: "AHWebMarkdown") }
        if defaults.object(forKey: "AHWebTimeout") != nil { webTimeout = defaults.double(forKey: "AHWebTimeout") }
        annotate = defaults.bool(forKey: "AHAnnotate")
        notes = min(Wire.maxAnnotations, max(0, defaults.integer(forKey: "AHNotes")))
        historyFail = max(0, defaults.integer(forKey: "AHHistoryFail"))
        if defaults.object(forKey: "AHHistoryDelay") != nil { historyDelay = max(0, defaults.double(forKey: "AHHistoryDelay")) }
        historyGate = defaults.string(forKey: "AHHistoryGate")
    }
}

/// Wires the app together: the real engine over the keychain and the broker, or the mock backend
/// with sample data.
@MainActor
final class AppEnvironment {
    static var current: AppEnvironment?

    let store: AppStore
    let push: PushCoordinator
    let launch: LaunchOptions
    let mock: MockBackend?
    /// Pairings found in the keychain on the first launch after an install (they survive deleting the app).
    let leftoverPairings: Int

    init(launch: LaunchOptions = LaunchOptions()) {
        self.launch = launch
        let defaults = UserDefaults.standard
        let drafts: DraftStore
        if launch.mock {
            let backend = MockBackend(
                scenario: launch.scenario, latency: launch.latency, syncDelay: launch.syncDelay, cachedInbox: launch.cachedInbox
            )
            mock = backend
            if launch.holdInterjects { Task { await backend.setInterjectConsumeDelay(nil) } }
            if launch.historyFail > 0 { Task { await backend.setFailNextSessionHistory(launch.historyFail) } }
            if let delay = launch.historyDelay { Task { await backend.setSessionHistoryDelay(realSeconds: delay) } }
            if let gate = launch.historyGate { Task { await backend.setSessionHistoryGate(path: gate) } }
            if let kind = launch.followEvent {
                Task {
                    try? await Task.sleep(for: .seconds(2))
                    await backend.simulateFollowEvent(kind: kind)
                }
            }
            if let title = launch.arrival {
                Task {
                    try? await Task.sleep(for: .seconds(launch.arrivalDelay))
                    for index in 0..<launch.arrivalCount {
                        await backend.simulateArrival(sessionTitle: title == "none" ? nil : title,
                                                      kind: ["ask", "permission", "stop"][index % 3])
                        try? await Task.sleep(for: .milliseconds(110))
                    }
                }
            }
            drafts = MemoryDraftStore()
            // Every mock run starts from the same preferences, whatever an earlier run left behind.
            let suite = UserDefaults(suiteName: "ah.mock") ?? defaults
            suite.removePersistentDomain(forName: "ah.mock")
            if let autoAdvance = launch.autoAdvance { suite.set(autoAdvance, forKey: "ah.autoAdvance") }
            if let developerMode = launch.developerMode { suite.set(developerMode, forKey: "ah.developerMode") }
            store = AppStore(backend: backend, drafts: drafts, defaults: suite)
            if let hold = launch.advanceHold { store.advanceTiming.hold = .seconds(hold) }
            if let settle = launch.advanceSettle { store.advanceTiming.settle = .seconds(settle) }
            if let seconds = launch.endedCountdown { store.endedCountdownLength = .seconds(seconds) }
            if let hold = launch.todoNoteHold { store.todoNoteHold = .seconds(hold) }
            if let hold = launch.developerToastHold { store.developer.toastHold = hold }
            leftoverPairings = 0
        } else {
            let group = Bundle.main.object(forInfoDictionaryKey: "AHKeychainGroup") as? String
            let secrets = KeychainSecretStore(service: "com.naituw.humaninloop.ios", accessGroup: group)
            let vault = Vault(secrets: secrets)
            let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("AskHuman", isDirectory: true)
            let local = LocalStore(root: support)
            let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("AskHumanAssets", isDirectory: true)
            let pushEnv = Bundle.main.object(forInfoDictionaryKey: "AHPushEnvironment") as? String
            #if DEBUG
            let allowLoopback = true
            #else
            let allowLoopback = false
            #endif
            let engine = PhoneEngine(
                vault: vault, store: local, broker: MQTTBrokerClient(),
                config: EngineConfig(
                    deviceName: DeviceName.current,
                    appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0",
                    bundleId: Bundle.main.bundleIdentifier ?? "com.naituw.humaninloop.ios",
                    pushEnv: pushEnv == "prod" ? "prod" : "dev", allowPlainLoopback: allowLoopback, assetDirectory: caches
                )
            )
            mock = nil
            drafts = local
            store = AppStore(backend: engine, drafts: drafts)
            // After a reinstall the keychain still holds the pairings but the app has never run.
            leftoverPairings = defaults.bool(forKey: "ah.hasLaunched") ? 0 : vault.pairings().count
        }
        defaults.set(true, forKey: "ah.hasLaunched")
        // Both the ended strip's button and its countdown use the same slide as advancing after a send.
        store.animatePageMove = { [weak store = store] change in
            if let store { RequestPageTransition.shared.move(in: store, change: change) } else { change() }
        }
        store.pageIsCovered = { PageEditing.isCovered }
        store.pageIsVisible = { PageEditing.isVisible }
        push = PushCoordinator(store: store, simulated: launch.mock)
        if let tab = launch.tab { store.tab = tab }
        Self.current = self
        AppDiagnostics.install(store: store)
    }

    func start() async {
        // These two are in place before anything can ask the mock for a list or start a task.
        if let backend = mock {
            if launch.noModels { await backend.setOffersModels(false) }
            if launch.maxControl { await backend.setOffersMaxControl(true) }
            if let reason = launch.failTask { await backend.setFailNextTask(reason) }
            await backend.setTaskAttachmentOptions(supported: !launch.noTaskAttachments, failUpload: launch.failTaskUpload)
            if launch.annotate { await backend.allowAnnotations() }
        }
        await store.start()
        await push.refreshAuthorization()
        if let request = launch.notifyRequest {
            // What the notification extension would have made of this push, lines and all.
            let plain = PushPlain(
                k: "ask", rid: request, mac: "Home MacBook Pro", agent: "Cursor", project: "HumanInLoop",
                session: "Plan the users table migration", text: "Which database should the migration target?", ts: 0
            )
            let lines = plain.headerLines
            let hint = NotificationHint(title: lines.title, subtitle: lines.subtitle, body: plain.text)
            store.openNotification(macId: MockData.homeMacId, rid: request, kind: "ask", hint: hint)
        }
        if let request = launch.openRequest {
            // Wait for the first state, then open the request.
            for _ in 0..<50 where store.state.inbox.isEmpty { try? await Task.sleep(for: .milliseconds(20)) }
            if let item = store.state.inbox.first(where: { $0.requestId == request || $0.id == request }) {
                store.open(macId: item.macId, requestId: item.requestId)
                if launch.mock, launch.notes > 0, let composer = store.composer(for: item) {
                    Self.seedNotes(launch.notes, in: composer)
                }
                // The page is up: the request ends while it is being looked at.
                if let seconds = launch.endAfter, let backend = mock {
                    let (macId, requestId, outcome, by) = (item.macId, item.requestId, launch.endHow, launch.endBy)
                    let lease = launch.endInBackground ? MockResolutionLease() : nil
                    Task {
                        defer { lease?.finish() }
                        if lease != nil {
                            while UIApplication.shared.applicationState != .background {
                                try? await Task.sleep(for: .milliseconds(50))
                            }
                        }
                        try? await Task.sleep(for: .seconds(seconds))
                        await backend.simulateFinished(requestId: requestId, macId: macId, outcome: outcome, by: by)
                    }
                }
            }
        }
        if let agent = launch.openAgent {
            for _ in 0..<50 where store.state.agents.isEmpty { try? await Task.sleep(for: .milliseconds(20)) }
            if let item = store.state.agents.first(where: { $0.agentKey == agent || $0.id == agent }) {
                store.openAgent(macId: item.macId, agentKey: item.agentKey)
            }
        }
        if let agent = launch.followAgent, !launch.mock {
            Task { @MainActor [store] in
                // The session may not exist yet (the run pairs and answers questions first): wait up to 20 minutes.
                for _ in 0..<6000 {
                    if let item = store.state.agents.first(where: { $0.agentKey == agent || $0.id == agent }) {
                        await store.setFollow(item, on: true)
                        return
                    }
                    try? await Task.sleep(for: .milliseconds(200))
                }
            }
        }
        if launch.newTask {
            for _ in 0..<50 where store.taskMacs.isEmpty { try? await Task.sleep(for: .milliseconds(20)) }
            store.tab = .agents
            store.presentNewTask()
            if launch.mock, launch.taskAttachmentDemo, let composer = store.taskComposer {
                try? composer.addAttachment(name: "Notes.txt", mime: "text/plain", data: Data("Task notes".utf8), isImage: false)
                if let png = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=") {
                    try? composer.addAttachment(name: "Screen.png", mime: "image/png", data: png, isImage: true)
                }
            }
        }
    }

    /// Writes `count` notes on the message of a request, for the UI tests of the bar. They sit at made-up places, so the
    /// page finds the passage of some of them by their words and not of others; all of them count all the same.
    private static func seedNotes(_ count: Int, in composer: AnswerComposer) {
        for index in 0..<count {
            composer.addAnnotation(start: index * 12, end: index * 12 + 5, quote: "users", note: "Note \(index + 1)")
        }
    }
}

/// Only the mock resolution fixture requests background time; real sessions keep their existing lifecycle.
@MainActor
private final class MockResolutionLease {
    private var token = UIBackgroundTaskIdentifier.invalid

    init() {
        token = UIApplication.shared.beginBackgroundTask(withName: "Mock request resolution") { [weak self] in
            Task { @MainActor in self?.finish() }
        }
    }

    func finish() {
        guard token != .invalid else { return }
        let ending = token
        token = .invalid
        UIApplication.shared.endBackgroundTask(ending)
    }
}

/// The name shown on the Macs for this phone: what the user chose, else the device name (which iOS
/// reduces to the model name unless the user allows more).
@MainActor
enum DeviceName {
    private static let key = "ah.deviceName"

    static var current: String {
        get {
            let stored = UserDefaults.standard.string(forKey: key)?.trimmingCharacters(in: .whitespacesAndNewlines)
            if let stored, !stored.isEmpty { return stored }
            return UIDevice.current.name
        }
        set {
            let trimmed = newValue.trimmingCharacters(in: .whitespacesAndNewlines)
            UserDefaults.standard.set(trimmed.isEmpty ? nil : trimmed, forKey: key)
        }
    }
}
