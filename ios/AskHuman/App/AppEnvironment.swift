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
    /// Forces "open the next request after sending" on or off (UI tests need a fixed value).
    var autoAdvance: Bool?

    init(defaults: UserDefaults = .standard) {
        mock = defaults.bool(forKey: "AHMock")
        if let name = defaults.string(forKey: "AHScenario"), let scenario = MockBackend.Scenario(rawValue: name) {
            self.scenario = scenario
        }
        if defaults.object(forKey: "AHLatency") != nil { latency = defaults.double(forKey: "AHLatency") }
        tab = defaults.string(forKey: "AHTab").flatMap(AppTab.init(rawValue:))
        openRequest = defaults.string(forKey: "AHOpen")
        if defaults.object(forKey: "AHAutoAdvance") != nil { autoAdvance = defaults.bool(forKey: "AHAutoAdvance") }
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
            let backend = MockBackend(scenario: launch.scenario, latency: launch.latency)
            mock = backend
            drafts = MemoryDraftStore()
            // Every mock run starts from the same preferences, whatever an earlier run left behind.
            let suite = UserDefaults(suiteName: "ah.mock") ?? defaults
            suite.removePersistentDomain(forName: "ah.mock")
            if let autoAdvance = launch.autoAdvance { suite.set(autoAdvance, forKey: "ah.autoAdvance") }
            store = AppStore(backend: backend, drafts: drafts, defaults: suite)
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
        await store.start()
        await push.refreshAuthorization()
        if let request = launch.openRequest {
            // Wait for the first state, then open the request.
            for _ in 0..<50 where store.state.inbox.isEmpty { try? await Task.sleep(for: .milliseconds(20)) }
            if let item = store.state.inbox.first(where: { $0.requestId == request || $0.id == request }) {
                store.open(macId: item.macId, requestId: item.requestId)
            }
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
