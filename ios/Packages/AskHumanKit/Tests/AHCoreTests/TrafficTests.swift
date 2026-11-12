import XCTest
import AHProtocol
import AHStore
@testable import AHCore

final class EngineTrafficTests: XCTestCase {
    func testLiveCountersAreSavedAndMacRecordsSurviveRemoval() async throws {
        let h = Harness()
        h.mac.caps.trafficUsage = true
        await h.pair()
        let state = await h.state()
        let broker = TrafficRecord.brokerId(try XCTUnwrap(state.brokerHost))
        let month = PhoneEngine.month(h.clock.now())
        let local = try XCTUnwrap(state.traffic)
        XCTAssertGreaterThan(local.receivedBytes, 0)
        XCTAssertGreaterThanOrEqual(h.store.traffic(month: month, broker: broker).receivedBytes, local.receivedBytes)
        let a = TrafficRecord(broker: broker, month: month, macId: h.mac.macId, bytesOut: 100, updatedAt: h.clock.nowMs())
        let b = TrafficRecord(broker: broker, month: month, macId: Bytes.newId(), bytesOut: 50, updatedAt: h.clock.nowMs())
        await h.mac.sendDown(TrafficSync(records: [a, b]), to: h.phoneId)
        await eventually("two owners") { await h.state().brokerTraffic?.records.filter { $0.kind == "mac" }.count == 2 }
        await h.mac.sendDown(TrafficSync(records: [a, b]), to: h.phoneId)
        await eventually("remote records stay stable") {
            await h.state().brokerTraffic?.records.filter { $0.kind == "mac" }.reduce(UInt64(0)) { $0 + $1.bytesOut } == 150
        }
        await h.engine.unpair(macId: h.mac.macId)
        XCTAssertEqual(h.store.trafficLedger().records.count, 3)
        let expected = BrokerTraffic(month: month, records: h.store.trafficLedger().records).recordedBytes
        let reopened = PhoneEngine(
            vault: h.vault, store: LocalStore(root: h.store.root), broker: h.broker.makeClient(), clock: h.clock,
            relay: h.relay, config: EngineConfig(
                deviceName: "Restored phone", appVersion: "1", bundleId: "com.example.app", pushEnv: "dev",
                allowPlainLoopback: false, assetDirectory: h.root.appendingPathComponent("restored-assets")
            )
        )
        await reopened.start()
        let restored = await reopened.snapshot()
        XCTAssertEqual(restored.brokerTraffic?.stats.brokerEstimateBytes, expected)
        XCTAssertGreaterThan(restored.traffic?.receivedBytes ?? 0, 0)
    }

    func testRejectedPublicationDoesNotIncreaseTheSentCounter() async throws {
        let h = Harness()
        h.vault.broker = BrokerSettings(host: "broker.test", port: 8883, wsPort: 8084,
                                       username: "u1", password: "pw", relayUrl: "")
        await h.engine.start()
        let before = await h.state().traffic?.sentBytes
        h.phone.failsPublishes = true
        let sent = await h.engine.publishPayload(topic: "ah/u1/test", payload: Data([1, 2, 3]), retain: false)
        XCTAssertFalse(sent)
        let after = await h.state()
        XCTAssertEqual(after.traffic?.sentBytes, before)
        XCTAssertTrue(after.brokerTraffic?.records.first { $0.kind == "phone" }?.uncertain == true)
    }

    func testLegacyMonthStaysPartialAndTheNextMonthKeepsTheObservationStart() async throws {
        let h = Harness()
        h.vault.broker = BrokerSettings(host: "broker.test", port: 8883, wsPort: 8084,
                                       username: "u1", password: "pw", relayUrl: "")
        let oldMonth = PhoneEngine.month(h.clock.now())
        h.store.saveTraffic(TrafficStats(month: oldMonth, sentBytes: 10))
        await h.engine.start()
        await h.engine.recordTraffic(inbound: true, bytes: 5, broker: "broker.test", month: oldMonth, announce: false)
        let old = await h.state().brokerTraffic?.records.first
        XCTAssertNil(old?.recordedFrom)
        XCTAssertEqual(old?.bytesOut, 10)
        await h.clock.advance(by: 35 * 24 * 3600)
        let newMonth = PhoneEngine.month(h.clock.now())
        await h.engine.recordTraffic(inbound: true, bytes: 7, broker: "broker.test", month: newMonth, announce: false)
        let current = await h.state().brokerTraffic?.records.first
        XCTAssertEqual(current?.recordedFrom, TrafficRecord.monthStart(newMonth))
        XCTAssertEqual(current?.bytesIn, 7)
        XCTAssertEqual(current?.bytesOut, 0)
    }

    func testPhoneForwardsUnionToAnotherMacAndStopsWhenNothingChanges() async throws {
        let h = Harness()
        h.mac.caps.trafficUsage = true
        await h.pair()
        let other = TestMac(broker: h.broker, user: "u1", clock: h.clock, macName: "Office")
        defer { other.stop() }
        other.caps.trafficUsage = true
        await other.start(password: "pw")
        await h.engine.startPairing(code: other.startPairing(relayUrl: "", password: "pw"))
        await eventually("second pairing") { other.sas != nil }
        await other.allow()
        await eventually("second pairing completes") {
            if case .done = await h.state().pairing?.stage { return true }
            return false
        }
        await h.engine.finishPairing()
        await eventually("both online") { await h.state().macs.filter { $0.link == .online }.count == 2 }
        let current = await h.state()
        let broker = TrafficRecord.brokerId(try XCTUnwrap(current.brokerHost))
        let month = PhoneEngine.month(h.clock.now())
        let a = TrafficRecord(broker: broker, month: month, macId: h.mac.macId, bytesOut: 100, updatedAt: h.clock.nowMs())
        let b = TrafficRecord(broker: broker, month: month, macId: other.macId, bytesOut: 50, updatedAt: h.clock.nowMs())
        await h.mac.sendDown(TrafficSync(records: [a]), to: h.phoneId)
        await other.sendDown(TrafficSync(records: [b]), to: h.phoneId)
        await eventually("union on phone") { await h.state().brokerTraffic?.records.filter { $0.kind == "mac" }.count == 2 }
        await h.pass(seconds: 65)
        XCTAssertGreaterThan(h.mac.count(TrafficSync.type), 0)
        XCTAssertGreaterThan(other.count(TrafficSync.type), 0)
        let sent = h.mac.count(TrafficSync.type) + other.count(TrafficSync.type)
        await h.pass(seconds: 120)
        XCTAssertEqual(h.mac.count(TrafficSync.type) + other.count(TrafficSync.type), sent)
    }
}
