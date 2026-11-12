import XCTest
import AHProtocol
import AHStore
import AHCore

final class TrafficE2ETests: E2ECase {
    func testTwoRealDaemonsExchangeTheirLedgerThroughThePhone() async throws {
        try await pair()
        let other = try await MacInstance(environment: environment, broker: broker, macName: "Office Mac")
        addTeardownBlock { await other.stop() }
        await phone.engine.finishPairing()
        await phone.engine.startPairing(code: try await other.startPairing())
        await eventually("second Mac pairing request") {
            let pairing = (try? await other.status())?["pairing"] as? [String: Any]
            return pairing?["stage"] as? String == "pending"
        }
        _ = try await other.cli(["channel", "ios", "allow"])
        await eventually("second pairing finishes") {
            if case .done = await self.phone.state().pairing?.stage { return true }
            return false
        }
        await phone.engine.finishPairing()
        await eventually("both Mac counters on phone") { await self.phone.state().brokerTraffic?.records.count == 3 }
        await phone.engine.setForeground(false)
        await phone.engine.setForeground(true)
        await eventually("both daemons hold the union") {
            let first = (try? await self.mac.status())?["usageRecords"] as? [[String: Any]]
            let second = (try? await other.status())?["usageRecords"] as? [[String: Any]]
            return first?.count == 3 && second?.count == 3
        }
        let firstMac = try XCTUnwrap(mac)
        for instance in [firstMac, other] {
            let status = try await instance.status()
            let counters = try XCTUnwrap(status["traffic"] as? [String: Any])
            let confirmed = try XCTUnwrap(counters["bytesOut"] as? NSNumber)
            XCTAssertGreaterThan(confirmed.uint64Value, 0)
        }
        let current = await phone.state()
        XCTAssertGreaterThan(current.traffic?.sentBytes ?? 0, 0)
        XCTAssertFalse(current.brokerTraffic?.hasCompleteHistory ?? true)
        let before = current.brokerTraffic?.records.count
        let paired = try XCTUnwrap(phone.vault.pairings().first)
        await phone.engine.unpair(macId: paired.macId)
        XCTAssertEqual(before, 3)
        let after = await phone.state().brokerTraffic?.records.count
        XCTAssertEqual(after, 3)
    }
}
