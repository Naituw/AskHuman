import Foundation

/// One device's cumulative counters for a broker and UTC month. Relays preserve the original owner.
public struct TrafficRecord: Codable, Equatable, Sendable {
    public var deviceKind: String?
    public var recordedFrom: Int64?
    @OmitFalse public var uncertain: Bool
    public var broker: String
    public var month: String
    public var macId: String
    public var bytesIn: UInt64
    public var bytesOut: UInt64
    public var msgsIn: UInt64
    public var msgsOut: UInt64
    public var updatedAt: Int64
    public var macName: String?

    public init(broker: String, month: String, macId: String, bytesIn: UInt64 = 0, bytesOut: UInt64 = 0,
                msgsIn: UInt64 = 0, msgsOut: UInt64 = 0, updatedAt: Int64, macName: String? = nil, deviceKind: String? = "mac", recordedFrom: Int64? = nil, uncertain: Bool = false) {
        self.deviceKind = deviceKind
        self.recordedFrom = recordedFrom
        self.uncertain = uncertain
        self.broker = broker
        self.month = month
        self.macId = macId
        self.bytesIn = bytesIn
        self.bytesOut = bytesOut
        self.msgsIn = msgsIn
        self.msgsOut = msgsOut
        self.updatedAt = updatedAt
        self.macName = macName
    }

    public var kind: String { deviceKind ?? "mac" }
    public var key: String { broker + "|" + month + "|" + kind + "|" + macId }

    public static func monthStart(_ month: String) -> Int64? {
        ISO8601DateFormatter().date(from: month + "-01T00:00:00Z").map { Int64($0.timeIntervalSince1970 * 1000) }
    }

    public static func brokerId(_ host: String) -> String {
        host.trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(in: CharacterSet(charactersIn: ".")).lowercased()
    }
}

public struct TrafficSync: WireMessage {
    public static let type = "traffic.usage"
    public var records: [TrafficRecord]
    public init(records: [TrafficRecord]) { self.records = records }
}
