import XCTest
import AHProtocol
@testable import AHStore

final class TrafficTests: XCTestCase {
    func testReplicationMergesOwnersAndSurvivesUnpairingAndReopen() {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = LocalStore(root: root)
        let a = String(repeating: "a", count: 26)
        let b = String(repeating: "b", count: 26)
        func record(_ id: String, _ bytes: UInt64) -> TrafficRecord {
            TrafficRecord(broker: "broker.test", month: "2026-11", macId: id, bytesOut: bytes, updatedAt: 1)
        }
        var ledger = TrafficLedger()
        XCTAssertTrue(ledger.merge([record(a, 100), record(b, 50)], broker: "broker.test", month: "2026-11"))
        XCTAssertFalse(ledger.merge([record(a, 80), record(b, 50)], broker: "broker.test", month: "2026-11"))
        XCTAssertFalse(ledger.merge([record(a, 200)], broker: "other.test", month: "2026-11"))
        XCTAssertFalse(ledger.merge([record(a, 200)], broker: "broker.test", month: "2026-12"))
        XCTAssertEqual(BrokerTraffic(month: "2026-11", records: ledger.records).stats.brokerEstimateBytes, 150)
        XCTAssertTrue(store.saveTrafficLedger(ledger))
        store.forget(macId: a)
        XCTAssertEqual(LocalStore(root: root).trafficLedger(), ledger)
        XCTAssertTrue(ledger.merge([record(a, 120)], broker: "broker.test", month: "2026-11"))
        XCTAssertEqual(ledger.records.count, 2)
    }

    func testPhoneCountersAreDurableAndScopedToBrokerAndMonth() {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = LocalStore(root: root)
        XCTAssertTrue(store.saveTraffic(TrafficStats(month: "2026-11", sentBytes: 7), broker: "a.test"))
        let reopened = LocalStore(root: root)
        XCTAssertEqual(reopened.traffic(month: "2026-11", broker: "a.test").sentBytes, 7)
        XCTAssertEqual(reopened.traffic(month: "2026-11", broker: "b.test").sentBytes, 0)
        XCTAssertEqual(reopened.traffic(month: "2026-12", broker: "a.test").sentBytes, 0)
    }

    func testEndpointLegsAreSummedOnceAndLegacyCoverageIsVisible() {
        let start = TrafficRecord.monthStart("2026-11")!
        let id = String(repeating: "a", count: 26)
        let mac = TrafficRecord(broker: "b", month: "2026-11", macId: id, bytesOut: 100, updatedAt: start, recordedFrom: start)
        var phone = TrafficRecord(broker: "b", month: "2026-11", macId: id, bytesIn: 100, updatedAt: start, deviceKind: "phone", recordedFrom: start)
        var ledger = TrafficLedger()
        ledger.merge([mac, phone, phone], broker: "b", month: "2026-11")
        let complete = BrokerTraffic(month: "2026-11", records: ledger.records)
        XCTAssertEqual(ledger.records.count, 2)
        XCTAssertEqual(complete.recordedBytes, 200)
        XCTAssertTrue(complete.hasCompleteHistory)
        phone.uncertain = true
        ledger.merge([phone], broker: "b", month: "2026-11")
        XCTAssertFalse(BrokerTraffic(month: "2026-11", records: ledger.records).hasCompleteHistory)
    }

    func testDurablePhoneCounterUsesItsOwnerIdentity() {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = LocalStore(root: root)
        let owner = String(repeating: "a", count: 26)
        var ledger = TrafficLedger()
        ledger.localPhoneId = owner
        ledger.records = [TrafficRecord(broker: "b", month: "2026-11", macId: owner, bytesOut: 100,
                                        updatedAt: 1, deviceKind: "phone")]
        XCTAssertTrue(store.saveTrafficLedger(ledger))
        let reopened = LocalStore(root: root)
        XCTAssertEqual(reopened.traffic(month: "2026-11", broker: "b", phoneId: owner).sentBytes, 100)
        XCTAssertEqual(reopened.traffic(month: "2026-11", broker: "b", phoneId: String(repeating: "b", count: 26)).sentBytes, 0)
    }

    func testSaveFailureIsReportedAndLargeCountersDoNotOverflow() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try Data().write(to: root)
        XCTAssertFalse(LocalStore(root: root).saveTraffic(TrafficStats(month: "2026-11"), broker: "a.test"))
        XCTAssertEqual(TrafficStats(month: "2026-11", sentBytes: .max, receivedBytes: 1).brokerEstimateBytes, .max)
    }
}
