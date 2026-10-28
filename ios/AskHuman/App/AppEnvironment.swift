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
    /// The mock Agents never pick up the messages sent to them (UI tests that withdraw one).
    var holdInterjects = false
    /// The mock Macs list no models, like an older Mac: the new task form has no model or effort row.
    var noModels = false
    /// The next new task of the mock fails with this reason (the texts of the progress page).
    var failTask: String?
    /// A followed Agent reports `done`, `failed` or `waiting` a moment after launch (the follow banner).
    var followEvent: String?
    /// A request arrives a moment after launch (the request banner), from a session with this title;
    /// `none` for one without a title.
    var arrival: String?
    /// Forces "open the next request after sending" on or off (UI tests need a fixed value).
    var autoAdvance: Bool?
    /// Seconds (real time) after which the `syncing` scenario turns into the demo; unset keeps it syncing.
    var syncDelay: Double?
    /// The `syncing` scenario starts with the requests the phone kept from the last run.
    var cachedInbox = false
    /// Seconds the receipt of a sent request stays up, and the next page takes to settle (UI tests
    /// stretch them to have time to look; the app's own values are made for the eye).
    var advanceHold: Double?
    var advanceSettle: Double?
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
        holdInterjects = defaults.bool(forKey: "AHHoldInterjects")
        noModels = defaults.bool(forKey: "AHNoModels")
        failTask = defaults.string(forKey: "AHFailTask")
        followEvent = defaults.string(forKey: "AHFollowEvent")
        arrival = defaults.string(forKey: "AHArrival")
        if defaults.object(forKey: "AHAutoAdvance") != nil { autoAdvance = defaults.bool(forKey: "AHAutoAdvance") }
        if defaults.object(forKey: "AHSyncDelay") != nil { syncDelay = defaults.double(forKey: "AHSyncDelay") }
        cachedInbox = defaults.bool(forKey: "AHCached")
        if defaults.object(forKey: "AHAdvanceHold") != nil { advanceHold = defaults.double(forKey: "AHAdvanceHold") }
        if defaults.object(forKey: "AHAdvanceSettle") != nil { advanceSettle = defaults.double(forKey: "AHAdvanceSettle") }
        notifyRequest = defaults.string(forKey: "AHNotify")
        if defaults.object(forKey: "AHWebMarkdown") != nil { webMarkdown = defaults.bool(forKey: "AHWebMarkdown") }
        if defaults.object(forKey: "AHWebTimeout") != nil { webTimeout = defaults.double(forKey: "AHWebTimeout") }
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
            if let kind = launch.followEvent {
                Task {
                    try? await Task.sleep(for: .seconds(2))
                    await backend.simulateFollowEvent(kind: kind)
                }
            }
            if let title = launch.arrival {
                Task {
                    try? await Task.sleep(for: .seconds(2))
                    await backend.simulateArrival(sessionTitle: title == "none" ? nil : title)
                }
            }
            drafts = MemoryDraftStore()
            // Every mock run starts from the same preferences, whatever an earlier run left behind.
            let suite = UserDefaults(suiteName: "ah.mock") ?? defaults
            suite.removePersistentDomain(forName: "ah.mock")
            if let autoAdvance = launch.autoAdvance { suite.set(autoAdvance, forKey: "ah.autoAdvance") }
            store = AppStore(backend: backend, drafts: drafts, defaults: suite)
            if let hold = launch.advanceHold { store.advanceTiming.hold = .seconds(hold) }
            if let settle = launch.advanceSettle { store.advanceTiming.settle = .seconds(settle) }
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
        push = PushCoordinator(store: store, simulated: launch.mock)
        if let tab = launch.tab { store.tab = tab }
        Self.current = self
    }

    func start() async {
        // These two are in place before anything can ask the mock for a list or start a task.
        if let backend = mock {
            if launch.noModels { await backend.setOffersModels(false) }
            if let reason = launch.failTask { await backend.setFailNextTask(reason) }
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
        }
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
