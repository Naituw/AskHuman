import CloudKit
import CryptoKit
import Foundation

/// The production [`CloudBackend`]: plain CloudKit calls against one private-database zone. No
/// `CKSyncEngine` (it cannot "pull now"; see docs/specs/todo-sync.md §6) and no local database: the
/// core owns all state, the helper only moves records.
///
/// Every record has one field, `payload`, stored through `encryptedValues` (end-to-end encrypted).
final class CloudKitBackend: CloudBackend {
    static let zoneName = "Todos"
    static let subscriptionID = "todos-zone"
    static let payloadKey = "payload"
    /// Records per `modifyRecords` call (CloudKit's own limit is 400).
    static let saveChunk = 200

    private let containerID: String
    private let environment: String
    private let container: CKContainer
    private let zoneID: CKRecordZone.ID
    private let systemFields = SystemFieldsCache()
    private let lock = NSLock()
    private var accountHashCache: String?

    private var database: CKDatabase { container.privateCloudDatabase }

    /// Call only when the process holds the CloudKit entitlement for `containerID`: touching
    /// `CKContainer` without it traps.
    init(containerID: String, environment: String) {
        self.containerID = containerID
        self.environment = environment
        self.container = CKContainer(identifier: containerID)
        self.zoneID = CKRecordZone.ID(zoneName: Self.zoneName, ownerName: CKCurrentUserDefaultName)
    }

    // MARK: - hello

    func hello() async -> HelloInfo {
        var info = HelloInfo(environment: environment, container: containerID, accountStatus: "unknown", accountHash: nil)
        let status: CKAccountStatus
        do {
            status = try await container.accountStatus()
        } catch {
            return info
        }
        switch status {
        case .available:
            info.accountStatus = "available"
            info.accountHash = await accountHash()
            if info.accountHash == nil { info.accountStatus = "unknown" }
        case .noAccount: info.accountStatus = "noAccount"
        case .restricted: info.accountStatus = "restricted"
        case .couldNotDetermine, .temporarilyUnavailable: info.accountStatus = "unknown"
        @unknown default: info.accountStatus = "unknown"
        }
        return info
    }

    /// Stable, non-reversible handle of the signed-in iCloud account (so the core can tell "the account
    /// changed" without ever learning who it is).
    private func accountHash() async -> String? {
        lock.lock()
        let cached = accountHashCache
        lock.unlock()
        if let cached { return cached }
        do {
            let user = try await container.userRecordID()
            let digest = SHA256.hash(data: Data(("todosync-account-v1:" + user.recordName).utf8))
            let hash = digest.map { String(format: "%02x", $0) }.joined().prefix(16).description
            lock.lock()
            accountHashCache = hash
            lock.unlock()
            return hash
        } catch {
            return nil
        }
    }

    /// The account can change under a running helper (`CKAccountChanged`); forget the cached hash then.
    func accountChanged() {
        lock.lock()
        accountHashCache = nil
        lock.unlock()
        systemFields.removeAll()
    }

    // MARK: - ensure / deleteZone

    func ensure() async throws {
        do {
            let status = try await container.accountStatus()
            guard status == .available else {
                throw RpcFailure(
                    code: status == .noAccount || status == .restricted ? RpcCode.notAuthenticated : RpcCode.offline,
                    message: "account not available")
            }
            let zone = CKRecordZone(zoneID: zoneID)
            let zoneResult = try await database.modifyRecordZones(saving: [zone], deleting: [])
            for (_, result) in zoneResult.saveResults {
                if case .failure(let error) = result { throw error }
            }
            let subscription = CKRecordZoneSubscription(zoneID: zoneID, subscriptionID: Self.subscriptionID)
            let info = CKSubscription.NotificationInfo()
            // Silent push: it only tells an idle helper "something changed, go and look".
            info.shouldSendContentAvailable = true
            subscription.notificationInfo = info
            let subResult = try await database.modifySubscriptions(saving: [subscription], deleting: [])
            for (_, result) in subResult.saveResults {
                if case .failure(let error) = result { throw error }
            }
        } catch {
            throw CloudErrors.failure(from: error)
        }
    }

    func deleteZone() async throws {
        do {
            _ = try await database.modifyRecordZones(saving: [], deleting: [zoneID])
            systemFields.removeAll()
        } catch {
            let failure = CloudErrors.failure(from: error)
            // Already gone is success: "clear iCloud data" is idempotent.
            if failure.code == RpcCode.zoneNotFound { systemFields.removeAll(); return }
            throw failure
        }
    }

    // MARK: - fetch

    func fetch(token: String?) async throws -> FetchResult {
        let since: CKServerChangeToken?
        if let token {
            guard let data = Data(base64Encoded: token),
                let decoded = try? NSKeyedUnarchiver.unarchivedObject(ofClass: CKServerChangeToken.self, from: data)
            else {
                throw RpcFailure(code: RpcCode.tokenExpired, message: "undecodable token")
            }
            since = decoded
        } else {
            since = nil
        }
        do {
            let result = try await database.recordZoneChanges(inZoneWith: zoneID, since: since)
            var changes: [FetchedRecord] = []
            changes.reserveCapacity(result.modificationResultsByID.count)
            for (id, outcome) in result.modificationResultsByID {
                switch outcome {
                case .success(let modification):
                    let record = modification.record
                    systemFields.put(record)
                    changes.append(
                        FetchedRecord(
                            name: id.recordName, type: record.recordType,
                            payload: Self.payload(of: record), changeTag: record.recordChangeTag ?? ""))
                case .failure(let error):
                    throw error
                }
            }
            var deletions: [FetchedDeletion] = []
            for deletion in result.deletions {
                systemFields.remove(deletion.recordID.recordName)
                deletions.append(FetchedDeletion(name: deletion.recordID.recordName, type: deletion.recordType))
            }
            // Oldest first: a stable order makes replays and logs comparable.
            changes.sort { $0.name < $1.name }
            let archived = try NSKeyedArchiver.archivedData(
                withRootObject: result.changeToken, requiringSecureCoding: true)
            return FetchResult(
                changes: changes, deletions: deletions, token: archived.base64EncodedString(),
                moreComing: result.moreComing)
        } catch {
            throw CloudErrors.failure(from: error)
        }
    }

    // MARK: - save

    func save(_ records: [OutRecord]) async throws -> [SaveOutcome] {
        var outcomes: [String: SaveOutcome] = [:]
        var bases: [String: CKRecord] = [:]
        var wantsFetch: [String: String] = [:]  // record name -> tag the caller believes is current
        for record in records {
            guard let tag = record.changeTag else { continue }
            if let base = systemFields.record(name: record.name, tag: tag) {
                bases[record.name] = base
            } else {
                wantsFetch[record.name] = tag
            }
        }
        do {
            // After a helper restart the caller's tags are unknown to us: look the records up once.
            if !wantsFetch.isEmpty {
                let ids = wantsFetch.keys.map { CKRecord.ID(recordName: $0, zoneID: zoneID) }
                let fetched = try await database.records(for: ids)
                for (id, outcome) in fetched {
                    let name = id.recordName
                    switch outcome {
                    case .success(let server):
                        systemFields.put(server)
                        if server.recordChangeTag == wantsFetch[name] {
                            bases[name] = server
                        } else {
                            outcomes[name] = .conflict(
                                name: name, serverPayload: Self.payload(of: server),
                                serverChangeTag: server.recordChangeTag ?? "")
                        }
                    case .failure(let error):
                        // Gone from the server: write it again as a new record.
                        if let ck = error as? CKError, ck.code == .unknownItem { continue }
                        throw error
                    }
                }
            }

            var toSave: [CKRecord] = []
            for record in records where outcomes[record.name] == nil {
                let id = CKRecord.ID(recordName: record.name, zoneID: zoneID)
                let ck = bases[record.name] ?? CKRecord(recordType: record.type, recordID: id)
                ck.encryptedValues[Self.payloadKey] = record.payload
                toSave.append(ck)
            }

            var index = 0
            while index < toSave.count {
                let chunk = Array(toSave[index..<min(index + Self.saveChunk, toSave.count)])
                index += Self.saveChunk
                let result = try await database.modifyRecords(
                    saving: chunk, deleting: [], savePolicy: .ifServerRecordUnchanged, atomically: false)
                for (id, outcome) in result.saveResults {
                    let name = id.recordName
                    switch outcome {
                    case .success(let saved):
                        systemFields.put(saved)
                        outcomes[name] = .saved(name: name, changeTag: saved.recordChangeTag ?? "")
                    case .failure(let error):
                        if let ck = error as? CKError, ck.code == .serverRecordChanged, let server = ck.serverRecord {
                            systemFields.put(server)
                            outcomes[name] = .conflict(
                                name: name, serverPayload: Self.payload(of: server),
                                serverChangeTag: server.recordChangeTag ?? "")
                        } else {
                            outcomes[name] = .error(name: name, code: CloudErrors.recordCode(from: error))
                        }
                    }
                }
            }
        } catch {
            throw CloudErrors.failure(from: error)
        }
        // Reply in the caller's order; a record we heard nothing about is an error the core retries.
        return records.map { outcomes[$0.name] ?? .error(name: $0.name, code: "noResult") }
    }

    // MARK: - helpers

    private static func payload(of record: CKRecord) -> Data {
        (record.encryptedValues[payloadKey] as? Data) ?? Data()
    }
}

/// Remembers the CloudKit "system fields" (record id, change tag, ...) of records seen so far, keyed by
/// name and change tag. A save of an existing record has to be built on them. Purely an optimisation:
/// without an entry the backend looks the record up again.
final class SystemFieldsCache {
    private struct Entry {
        var tag: String
        var data: Data
    }

    private let lock = NSLock()
    private var entries: [String: Entry] = [:]
    /// Bound on memory: far above the number of todos a person keeps.
    private let limit = 20_000

    func put(_ record: CKRecord) {
        guard let tag = record.recordChangeTag else { return }
        let archiver = NSKeyedArchiver(requiringSecureCoding: true)
        record.encodeSystemFields(with: archiver)
        archiver.finishEncoding()
        lock.lock()
        if entries.count >= limit { entries.removeAll() }
        entries[record.recordID.recordName] = Entry(tag: tag, data: archiver.encodedData)
        lock.unlock()
    }

    /// A record carrying the cached system fields, if the cache holds exactly `tag` for `name`.
    func record(name: String, tag: String) -> CKRecord? {
        lock.lock()
        let entry = entries[name]
        lock.unlock()
        guard let entry, entry.tag == tag,
            let coder = try? NSKeyedUnarchiver(forReadingFrom: entry.data)
        else { return nil }
        coder.requiresSecureCoding = true
        defer { coder.finishDecoding() }
        return CKRecord(coder: coder)
    }

    func remove(_ name: String) {
        lock.lock()
        entries[name] = nil
        lock.unlock()
    }

    func removeAll() {
        lock.lock()
        entries.removeAll()
        lock.unlock()
    }
}
