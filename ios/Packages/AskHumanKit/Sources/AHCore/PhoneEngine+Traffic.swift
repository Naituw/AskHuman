import Foundation
import AHProtocol
import AHStore

extension PhoneEngine {
    func loadPhoneTraffic() {
        let broker = activeSettings.map { TrafficRecord.brokerId($0.host) }
        let month = Self.month(clock.now())
        if traffic == nil || trafficBroker != broker || traffic?.month != month {
            trafficBroker = broker
            traffic = store.traffic(month: month, broker: broker, phoneId: vault.phoneId())
        }
    }

    func brokerTraffic() -> BrokerTraffic? {
        guard let settings = activeSettings else { return nil }
        let month = Self.month(clock.now())
        let records = trafficLedger.records(broker: TrafficRecord.brokerId(settings.host), month: month)
        return records.isEmpty ? nil : BrokerTraffic(month: month, records: records, syncedAt: trafficLedger.syncedAt ?? [:])
    }

    func onTrafficSync(_ macId: String, _ sync: TrafficSync) {
        guard let mac = macs[macId], mac.sync.isHandshaken, mac.caps?.trafficUsage == true,
              let settings = activeSettings else { return }
        let broker = TrafficRecord.brokerId(settings.host)
        let month = Self.month(clock.now())
        let changed = trafficLedger.merge(sync.records, broker: broker, month: month)
        var syncedAt = trafficLedger.syncedAt ?? [:]
        for record in sync.records where record.broker == broker && record.month == month && Bytes.isValidId(record.macId) {
            syncedAt[record.key] = clock.nowMs()
        }
        trafficLedger.syncedAt = syncedAt
        trafficStorageIssue = !store.saveTrafficLedger(trafficLedger)
        if let own = trafficLedger.records(broker: broker, month: month).first(where: { $0.kind == "phone" && $0.macId == vault.phoneId() }) {
            cachePhoneTraffic(own)
        }
        if changed {
            for (id, peer) in macs where id != macId && peer.caps?.trafficUsage == true {
                trafficPending.insert(id)
                scheduleTraffic(id)
            }
        }
    }

    func scheduleTraffic(_ macId: String) {
        guard timers["traffic." + macId] == nil else { return }
        let delay = max(0, 60 - clock.now().timeIntervalSince(trafficSentAt[macId] ?? .distantPast))
        after("traffic." + macId, delay) { engine in await engine.flushTraffic(macId) }
    }

    func flushTraffic(_ macId: String) async {
        guard foreground, trafficPending.contains(macId), let mac = macs[macId],
              mac.sync.isHandshaken, mac.caps?.trafficUsage == true, let settings = activeSettings else { return }
        trafficPending.remove(macId)
        trafficSentAt[macId] = clock.now()
        let records = trafficLedger.records(broker: TrafficRecord.brokerId(settings.host), month: Self.month(clock.now())).map { record in
            if record.kind == "phone", record.macId == vault.phoneId() {
                return announcedPhoneTraffic[record.key] ?? record
            }
            return record
        }
        for start in stride(from: 0, to: records.count, by: 128) {
            let batch = Array(records[start..<min(start + 128, records.count)])
            if !(await send(TrafficSync(records: batch), to: macId)) {
                trafficPending.insert(macId)
                scheduleTraffic(macId)
                return
            }
        }
    }

    /// Counts a publication only after the broker acknowledged it. Failures keep the history partial.
    func publishPayload(topic: String, payload: Data, retain: Bool, announce: Bool = true) async -> Bool {
        guard let settings = activeSettings else { return false }
        let brokerId = TrafficRecord.brokerId(settings.host)
        let month = Self.month(clock.now())
        let sent = await broker.publish(topic: topic, payload: payload, retain: retain)
        if sent {
            recordTraffic(inbound: false, bytes: payload.count, broker: brokerId, month: month, announce: announce)
        } else {
            recordTraffic(inbound: false, bytes: 0, broker: brokerId, month: month, announce: true, uncertain: true)
        }
        return sent
    }

    func recordTraffic(inbound: Bool, bytes: Int, broker: String, month: String, announce: Bool, uncertain: Bool = false) {
        let phoneId = vault.phoneId()
        var observedSince = trafficLedger.observedSince ?? [:]
        let previousStart = observedSince[broker] ?? trafficLedger.records.filter {
            $0.broker == broker && $0.kind == "phone" && $0.macId == phoneId
        }.compactMap(\.recordedFrom).min() ?? clock.nowMs()
        observedSince[broker] = previousStart
        trafficLedger.observedSince = observedSince
        var own: TrafficRecord
        if let saved = trafficLedger.records(broker: broker, month: month).first(where: { $0.kind == "phone" && $0.macId == phoneId }) {
            own = saved
        } else {
            let legacy = store.traffic(month: month, broker: broker, phoneId: vault.phoneId())
            let hasLegacy = legacy.sentBytes > 0 || legacy.receivedBytes > 0
            own = TrafficRecord(
                broker: broker, month: month, macId: phoneId,
                bytesIn: legacy.receivedBytes, bytesOut: legacy.sentBytes,
                msgsIn: legacy.receivedMessages, msgsOut: legacy.sentMessages,
                updatedAt: clock.nowMs(), macName: config.deviceName, deviceKind: "phone",
                recordedFrom: hasLegacy ? nil : max(previousStart, TrafficRecord.monthStart(month) ?? clock.nowMs())
            )
        }
        if bytes > 0 || !uncertain {
            if inbound {
                own.bytesIn = BrokerTraffic.add(own.bytesIn, UInt64(bytes))
                own.msgsIn = BrokerTraffic.add(own.msgsIn, 1)
            } else {
                own.bytesOut = BrokerTraffic.add(own.bytesOut, UInt64(bytes))
                own.msgsOut = BrokerTraffic.add(own.msgsOut, 1)
            }
        }
        own.uncertain = own.uncertain || uncertain || macs.values.contains { !($0.caps ?? $0.record.caps).trafficUsage }
        own.updatedAt = clock.nowMs()
        own.macName = config.deviceName
        trafficLedger.merge([own], broker: broker, month: month)
        trafficLedger.localPhoneId = phoneId
        trafficStorageIssue = !store.saveTrafficLedger(trafficLedger)
        if trafficStorageIssue {
            if let index = trafficLedger.records.firstIndex(where: { $0.key == own.key }) {
                trafficLedger.records[index].uncertain = true
            }
        }
        cachePhoneTraffic(own)
        if announce { announcePhoneTraffic() }
    }

    private func cachePhoneTraffic(_ record: TrafficRecord) {
        guard activeSettings.map({ TrafficRecord.brokerId($0.host) }) == record.broker,
              Self.month(clock.now()) == record.month else { return }
        trafficBroker = record.broker
        traffic = TrafficStats(month: record.month, sentBytes: record.bytesOut, receivedBytes: record.bytesIn,
                               sentMessages: record.msgsOut, receivedMessages: record.msgsIn)
    }

    func announcePhoneTraffic() {
        guard let summary = brokerTraffic(), let own = summary.records.first(where: { $0.kind == "phone" && $0.macId == vault.phoneId() }) else { return }
        announcedPhoneTraffic[own.key] = own
        for (id, mac) in macs where mac.caps?.trafficUsage == true {
            trafficPending.insert(id)
            scheduleTraffic(id)
        }
    }
}
