import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// Checks that only mean something against the real services: the push relay, and the round trips
/// over the internet. Every other E2E test runs against the real broker too when `AH_E2E_REAL_FROM`
/// is set; these are skipped without it.
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_REAL_FROM=<ASKHUMAN_HOME of a configured Mac> \
///   AH_E2E_BUNDLE_ID=<the app's bundle id> swift test --filter RealServicesTests
///
/// The throwaway Macs take over the broker and relay settings of the configured Mac, so the checks
/// need nothing else. Nothing secret is ever printed; the numbers are (and go to `AH_E2E_REPORT`,
/// a JSON file, when that is set).
final class RealServicesTests: E2ECase {
    override func setUp() async throws {
        guard try E2EEnvironment.current().usesRealServices else {
            throw XCTSkip("AH_E2E_REAL_FROM is not set: these checks need the real broker and relay")
        }
        try await super.setUp()
    }

    // MARK: push relay

    /// A made-up device token goes through the whole push path: the phone registers it with the
    /// real relay and hands the handle to the Mac; a request that finds the phone away makes the
    /// Mac push through the relay to APNs, which refuses the token; the Mac learns the handle is
    /// dead. A new token (a reinstalled app) then gets a fresh handle.
    func testAPushForAMadeUpTokenIsRefusedByAPNsAndAFreshTokenRecovers() async throws {
        try XCTSkipIf(environment.phoneBundleId == "com.example.e2e", "AH_E2E_BUNDLE_ID must be the app's bundle id")
        try await pair()

        await phone.engine.updatePushToken(Data(repeating: 0xAB, count: 32))
        await eventually("the phone to hold a relay handle", timeout: 30) {
            if case .registered = await self.phone.state().push { return true }
            return false
        }
        await eventually("the Mac to hold the handle") { await self.pushState() == "ok" }

        await phone.engine.setForeground(false)
        await eventually("the phone to leave") { await self.isOnline() == false }

        let asked = Date()
        let ask = try mac.ask(["-q", "Pushed?", "-o", "A"])
        await eventually("APNs to refuse the made-up token", timeout: 30) { await self.pushState() == "gone" }
        let refused = Date().timeIntervalSince(asked)
        report(["pushRefusedSeconds": refused])
        XCTAssertLessThan(refused, 10, "relay and APNs answered slowly")

        // The phone comes back and still finds its request.
        await phone.engine.setForeground(true)
        await eventually("the request after coming back") { await self.phone.state().inbox.count == 1 }
        ask.terminate()

        await phone.engine.updatePushToken(Data(repeating: 0xCD, count: 32))
        await eventually("a fresh handle at the Mac", timeout: 30) { await self.pushState() == "ok" }
    }

    /// The relay only serves the app it was set up for: a token of another bundle id is refused,
    /// and the app shows that instead of pretending it will get pushes.
    func testTheRelayRefusesATokenOfAnotherApp() async throws {
        await phone.close()
        phone = E2EPhone(bundleId: "com.example.another-app")
        try await pair()
        await phone.engine.updatePushToken(Data(repeating: 0xEF, count: 32))
        await eventually("the relay to refuse", timeout: 30) {
            if case .failed = await self.phone.state().push { return true }
            return false
        }
        let devices = try await mac.jsonArray(["channel", "ios", "devices", "--json"])
        XCTAssertEqual(devices.count, 1)
        XCTAssertNil(devices.first?["push"], "the Mac never got a handle")
    }

    // MARK: latency

    /// Foreground round trips over the internet, against the targets of plan D10:
    /// (3) the AskHuman call until the phone shows the request, p95 <= 1.5 s;
    /// (1) the answer until the Mac has accepted it, p95 <= 1 s.
    func testForegroundRoundTripsMeetTheLatencyTargets() async throws {
        try await pair()
        let rounds = Int(ProcessInfo.processInfo.environment["AH_E2E_ROUNDS"] ?? "") ?? 20
        var shown: [Double] = []
        var accepted: [Double] = []
        var finished: [Double] = []
        for round in 0..<rounds {
            let asked = Date()
            let ask = try mac.ask(["-q", "Round \(round)", "-o", "A", "-o", "B"])
            guard await measure(timeout: 15, { await self.phone.state().inbox.count == 1 }) != nil else {
                return XCTFail("round \(round): the request never reached the phone")
            }
            shown.append(Date().timeIntervalSince(asked))
            let item = try unwrap(await phone.state().inbox.first)

            let answered = Date()
            _ = try await phone.engine.submitAnswer(
                macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [round % 2])]
            )
            guard await measure(timeout: 15, { await self.phone.state().inbox.isEmpty }) != nil else {
                return XCTFail("round \(round): the Mac never accepted the answer")
            }
            accepted.append(Date().timeIntervalSince(answered))
            guard await ask.wait(timeout: 15) == 0 else { return XCTFail("round \(round): AskHuman did not finish: \(ask.stderr)") }
            finished.append(Date().timeIntervalSince(answered))
        }

        let summary: [String: Any] = [
            "rounds": rounds,
            "callToPhoneShowsMs": Self.stats(shown),
            "answerToMacAcceptsMs": Self.stats(accepted),
            "answerToCallerReturnsMs": Self.stats(finished),
        ]
        report(summary)
        XCTAssertLessThanOrEqual(Self.percentile(shown, 0.95), 1.5, "call -> phone shows request, p95")
        XCTAssertLessThanOrEqual(Self.percentile(accepted, 0.95), 1.0, "answer -> Mac accepts, p95")
    }

    // MARK: helpers

    private func devices() async -> [[String: Any]] {
        (try? await mac.jsonArray(["channel", "ios", "devices", "--json"])) ?? []
    }

    private func pushState() async -> String? { await devices().first?["push"] as? String }

    private func isOnline() async -> Bool? { await devices().first?["online"] as? Bool }

    /// Seconds until `check` holds (polled every 5 ms), `nil` after `timeout`.
    private func measure(timeout: TimeInterval, _ check: () async -> Bool) async -> TimeInterval? {
        let started = Date()
        while Date().timeIntervalSince(started) < timeout {
            if await check() { return Date().timeIntervalSince(started) }
            try? await Task.sleep(nanoseconds: 5_000_000)
        }
        return nil
    }

    /// Nearest-rank percentile.
    static func percentile(_ values: [Double], _ p: Double) -> Double {
        let sorted = values.sorted()
        guard !sorted.isEmpty else { return .nan }
        let rank = Int((Double(sorted.count) * p).rounded(.up)) - 1
        return sorted[min(max(rank, 0), sorted.count - 1)]
    }

    static func stats(_ seconds: [Double]) -> [String: Double] {
        func ms(_ value: Double) -> Double { (value * 1000).rounded() }
        return [
            "min": ms(seconds.min() ?? .nan), "p50": ms(percentile(seconds, 0.5)),
            "p95": ms(percentile(seconds, 0.95)), "max": ms(seconds.max() ?? .nan),
        ]
    }
}
