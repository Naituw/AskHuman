import SwiftUI
import AHCore
import AHProtocol
import AHStore

struct TrafficSection: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        Section {
            if let summary = store.state.brokerTraffic {
                LabeledContent("Recorded devices", value: "\(summary.records.count)")
                LabeledContent("Recorded message traffic", value: bytes(summary.stats.brokerEstimateBytes))
                if summary.hasCompleteHistory {
                    LabeledContent("Of the free allowance", value: summary.stats.quotaShare.formatted(.percent.precision(.fractionLength(0))))
                } else {
                    Text("This month's record is incomplete.").foregroundStyle(.secondary)
                }
                DisclosureGroup("Usage by device") {
                    ForEach(summary.records, id: \.key) { record in
                        VStack(alignment: .leading, spacing: 3) {
                            LabeledContent(name(record), value: bytes(estimate(record)))
                            Text("Updated \(Date(timeIntervalSince1970: Double(record.updatedAt) / 1000).formatted(date: .abbreviated, time: .shortened))")
                                .font(.caption).foregroundStyle(.secondary)
                            if let syncedAt = summary.syncedAt[record.key] {
                                Text("Synced \(Date(timeIntervalSince1970: Double(syncedAt) / 1000).formatted(date: .abbreviated, time: .shortened))")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            } else {
                Text("Waiting for Mac usage")
            }
            ForEach(store.macs.filter { !$0.caps.trafficUsage }) { mac in
                Text("Update \(mac.name) to include its usage.").foregroundStyle(.secondary)
            }
            if let traffic = store.state.traffic {
                DisclosureGroup("This phone") {
                    LabeledContent("Sent", value: bytes(traffic.sentBytes))
                    LabeledContent("Received", value: bytes(traffic.receivedBytes))
                    LabeledContent("Messages", value: "\(traffic.sentMessages + traffic.receivedMessages)")
                }
            }
            if store.state.trafficStorageIssue {
                Text("Traffic could not be saved. The estimate may be incomplete after a restart.")
                    .foregroundStyle(.orange)
            }
            Link("View in EMQX console", destination: URL(string: "https://cloud.emqx.com/console/")!)
        } header: {
            Text("This month")
        } footer: {
            Text("Adds each device's own sent and received messages. Earlier usage may be incomplete. Other clients, MQTT control packets and TCP/TLS overhead also consume the broker's 1 GB monthly allowance. Unpairing keeps recorded usage.")
        }
    }

    private func bytes(_ value: UInt64) -> String { Int(clamping: value).formatted(.file) }

    private func estimate(_ record: TrafficRecord) -> UInt64 {
        BrokerTraffic(month: record.month, records: [record]).stats.brokerEstimateBytes
    }

    private func name(_ record: TrafficRecord) -> String {
        if record.kind == "phone" { return record.macName ?? String(localized: "Mobile device") }
        return store.macs.first { $0.id == record.macId }?.name ?? record.macName ?? String(record.macId.prefix(8))
    }
}
