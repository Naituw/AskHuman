import Foundation
import AHProtocol

public struct TrafficLedger: Codable, Equatable, Sendable {
    public var records: [TrafficRecord] = []
    public var syncedAt: [String: Int64]?
    public var localPhoneId: String?
    public var observedSince: [String: Int64]?

    public init() {}

    public func records(broker: String, month: String) -> [TrafficRecord] {
        records.filter { $0.broker == broker && $0.month == month }.sorted { $0.key < $1.key }
    }

    @discardableResult
    public mutating func merge(_ incoming: [TrafficRecord], broker: String, month: String) -> Bool {
        var changed = false
        for record in incoming {
            guard record.broker == broker, record.month == month, record.updatedAt >= 0, ["mac", "phone"].contains(record.kind),
                  record.recordedFrom.map({ $0 >= 0 }) ?? true,
                  record.macId.utf8.count == 26,
                  record.macId.utf8.allSatisfy({ (97...122).contains($0) || (50...55).contains($0) }) else { continue }
            if let index = records.firstIndex(where: { $0.broker == broker && $0.month == month && $0.macId == record.macId && $0.kind == record.kind }) {
                let before = records[index]
                records[index].bytesIn = max(before.bytesIn, record.bytesIn)
                records[index].bytesOut = max(before.bytesOut, record.bytesOut)
                records[index].msgsIn = max(before.msgsIn, record.msgsIn)
                records[index].msgsOut = max(before.msgsOut, record.msgsOut)
                if record.updatedAt >= before.updatedAt, let name = record.macName { records[index].macName = name }
                records[index].updatedAt = max(before.updatedAt, record.updatedAt)
                if let a = before.recordedFrom, let b = record.recordedFrom {
                    records[index].recordedFrom = min(a, b)
                } else { records[index].recordedFrom = nil }
                records[index].uncertain = before.uncertain || record.uncertain
                changed = changed || records[index] != before
            } else {
                records.append(record)
                changed = true
            }
        }
        return changed
    }
}

public struct BrokerTraffic: Equatable, Sendable {
    public var month: String
    public var syncedAt: [String: Int64]
    public var records: [TrafficRecord]

    public init(month: String, records: [TrafficRecord], syncedAt: [String: Int64] = [:]) {
        self.month = month
        self.syncedAt = syncedAt
        self.records = records
    }

    public var hasCompleteHistory: Bool {
        guard let start = TrafficRecord.monthStart(month), records.contains(where: { $0.kind == "phone" }) else { return false }
        return records.allSatisfy { !$0.uncertain && ($0.recordedFrom.map { $0 <= start } ?? false) }
    }

    public var recordedBytes: UInt64 { Self.add(stats.sentBytes, stats.receivedBytes) }

    public var stats: TrafficStats {
        var result = TrafficStats(month: month)
        for record in records {
            result.sentBytes = Self.add(result.sentBytes, record.bytesOut)
            result.receivedBytes = Self.add(result.receivedBytes, record.bytesIn)
            result.sentMessages = Self.add(result.sentMessages, record.msgsOut)
            result.receivedMessages = Self.add(result.receivedMessages, record.msgsIn)
        }
        return result
    }

    public static func add(_ left: UInt64, _ right: UInt64) -> UInt64 {
        let (sum, overflow) = left.addingReportingOverflow(right)
        return overflow ? .max : sum
    }
}
