import Foundation

// Domain-level model between the RPC layer and the cloud. The dispatcher translates wire <-> domain; the
// backend (CloudKit in production, a fake in tests) never sees JSON or base64.

struct HelloInfo: Equatable {
    var environment: String
    var container: String
    var accountStatus: String
    var accountHash: String?
    var problem: String? = nil
}

struct FetchedRecord: Equatable {
    var name: String
    var type: String
    var payload: Data
    var changeTag: String
}

struct FetchedDeletion: Equatable {
    var name: String
    var type: String
}

struct FetchResult: Equatable {
    var changes: [FetchedRecord]
    var deletions: [FetchedDeletion]
    var token: String
    var moreComing: Bool
}

enum SaveOutcome: Equatable {
    case saved(name: String, changeTag: String)
    /// Somebody else saved first: here is their version.
    case conflict(name: String, serverPayload: Data, serverChangeTag: String)
    case error(name: String, code: String)
}

protocol CloudBackend: AnyObject {
    /// Never throws: "could not tell" is `accountStatus: "unknown"`.
    func hello() async -> HelloInfo
    /// Create the zone and its subscription; idempotent.
    func ensure() async throws
    func fetch(token: String?) async throws -> FetchResult
    func save(_ records: [OutRecord]) async throws -> [SaveOutcome]
    func deleteZone() async throws
}

/// The part of `status` that only the app knows (push registration, tick bookkeeping, process).
struct StatusExtras: Equatable {
    var apns: String
    var lastPushMs: Int64
    var lastTickMs: Int64
    var lastTickReason: String?
    var lastTickExit: Int?
    var ticksRun: Int
    var enabled: Bool
    var pollMinutes: Int
    var pid: Int32
    var uptimeMs: Int64
}

protocol StatusSource: AnyObject {
    func statusExtras() -> StatusExtras
}

/// Parses one request line, runs it against the backend and returns the reply line (no newline).
/// Every failure becomes a well-formed `{"ok":false,...}` reply: the socket never sees a thrown error.
final class RpcDispatcher {
    private let backend: CloudBackend
    private let helperVersion: String
    private weak var statusSource: StatusSource?
    private let log: (String, [String: Any]) -> Void

    init(
        backend: CloudBackend,
        helperVersion: String,
        statusSource: StatusSource? = nil,
        log: @escaping (String, [String: Any]) -> Void = { _, _ in }
    ) {
        self.backend = backend
        self.helperVersion = helperVersion
        self.statusSource = statusSource
        self.log = log
    }

    func handle(_ line: Data) async -> Data {
        switch RequestParser.parse(line) {
        case .failure(let failure):
            log("rpc_bad_request", ["message": failure.message])
            return failureReply(id: failure.id, RpcFailure(code: RpcCode.badRequest, message: failure.message))
        case .success(let request):
            return await run(request)
        }
    }

    private func run(_ request: ParsedRequest) async -> Data {
        let id = request.id
        let started = DispatchTime.now().uptimeNanoseconds
        func elapsedMs() -> Int { Int((DispatchTime.now().uptimeNanoseconds - started) / 1_000_000) }
        do {
            switch request.op {
            case .hello:
                let info = await backend.hello()
                return WireCoder.encode(
                    HelloReply(
                        id: id, protocolVersion: Wire.protocolVersion, helperVersion: helperVersion,
                        environment: info.environment, container: info.container,
                        accountStatus: info.accountStatus, accountHash: info.accountHash, problem: info.problem))
            case .status:
                let info = await backend.hello()
                guard let extras = statusSource?.statusExtras() else {
                    throw RpcFailure(code: RpcCode.other, message: "status unavailable")
                }
                return WireCoder.encode(
                    StatusReply(
                        id: id, protocolVersion: Wire.protocolVersion, helperVersion: helperVersion,
                        environment: info.environment, container: info.container,
                        accountStatus: info.accountStatus, accountHash: info.accountHash, problem: info.problem,
                        apns: extras.apns, lastPushMs: extras.lastPushMs, lastTickMs: extras.lastTickMs,
                        lastTickReason: extras.lastTickReason, lastTickExit: extras.lastTickExit,
                        ticksRun: extras.ticksRun, enabled: extras.enabled, pollMinutes: extras.pollMinutes,
                        pid: extras.pid, uptimeMs: extras.uptimeMs))
            case .ensure:
                try await backend.ensure()
                log("rpc_ensure", ["ms": elapsedMs()])
                return WireCoder.encode(EmptyReply(id: id))
            case .deleteZone:
                try await backend.deleteZone()
                log("rpc_delete_zone", ["ms": elapsedMs()])
                return WireCoder.encode(EmptyReply(id: id))
            case .fetch(let token):
                let page = try await backend.fetch(token: token)
                log(
                    "rpc_fetch",
                    [
                        "ms": elapsedMs(), "changes": page.changes.count, "deletions": page.deletions.count,
                        "moreComing": page.moreComing, "fromScratch": token == nil,
                    ])
                return WireCoder.encode(
                    FetchReply(
                        id: id,
                        changes: page.changes.map {
                            WireChange(
                                recordName: $0.name, recordType: $0.type,
                                payload: $0.payload.base64EncodedString(), changeTag: $0.changeTag)
                        },
                        deletions: page.deletions.map { WireDeletion(recordName: $0.name, recordType: $0.type) },
                        token: page.token, moreComing: page.moreComing))
            case .save(let records):
                let outcomes = try await backend.save(records)
                log(
                    "rpc_save",
                    [
                        "ms": elapsedMs(), "records": records.count,
                        "conflicts": outcomes.filter { if case .conflict = $0 { return true } else { return false } }.count,
                    ])
                return WireCoder.encode(SaveReply(id: id, results: outcomes.map(Self.wire)))
            }
        } catch let failure as RpcFailure {
            log("rpc_failed", ["code": failure.code, "ms": elapsedMs()])
            return failureReply(id: id, failure)
        } catch {
            log("rpc_failed", ["code": RpcCode.other, "ms": elapsedMs()])
            return failureReply(id: id, RpcFailure(code: RpcCode.other, message: String(describing: error)))
        }
    }

    private func failureReply(id: Int, _ failure: RpcFailure) -> Data {
        WireCoder.encode(
            FailureReply(id: id, error: WireError(code: failure.code, retryAfterMs: failure.retryAfterMs, message: failure.message)))
    }

    private static func wire(_ outcome: SaveOutcome) -> WireSaveResult {
        switch outcome {
        case .saved(let name, let tag):
            return WireSaveResult(recordName: name, status: "saved", changeTag: tag)
        case .conflict(let name, let payload, let tag):
            return WireSaveResult(
                recordName: name, status: "conflict", serverPayload: payload.base64EncodedString(), serverChangeTag: tag)
        case .error(let name, let code):
            return WireSaveResult(recordName: name, status: "error", code: code)
        }
    }
}
