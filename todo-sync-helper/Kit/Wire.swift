import Foundation

// Wire protocol of the helper RPC (docs/plans/todo-sync.md §3.3): one JSON object per line, in both
// directions, over a unix socket. `Fixtures/rpc-v1/*.json` pins the exact shapes; both this module and
// the Rust client (`src-tauri/src/todo_sync/helper_client.rs`) are tested against those files, so change
// the fixtures first.

enum Wire {
    /// Bumped on any change an older peer would misread. The core refuses to talk to another number.
    static let protocolVersion = 1
    /// Longest accepted request line. A save batch carries at most a few hundred records.
    static let maxLineBytes = 16 * 1024 * 1024
}

/// Error codes the core understands (`transport::NetError`). Anything else is "other".
enum RpcCode {
    static let offline = "offline"
    static let rateLimited = "rateLimited"
    static let notAuthenticated = "notAuthenticated"
    static let zoneNotFound = "zoneNotFound"
    static let tokenExpired = "tokenExpired"
    static let quota = "quota"
    static let badRequest = "badRequest"
    static let other = "other"
}

/// A failure the dispatcher turns into `{"ok":false,"error":{...}}`.
struct RpcFailure: Error, Equatable {
    var code: String
    var retryAfterMs: Int? = nil
    var message: String? = nil
}

// MARK: - Requests

/// A record to write. `changeTag` is the server tag the caller last saw (`nil` = believed new).
struct OutRecord: Equatable {
    var name: String
    var type: String
    var payload: Data
    var changeTag: String?
}

enum RpcOp: Equatable {
    case hello
    case status
    case ensure
    case fetch(token: String?)
    case save(records: [OutRecord])
    case deleteZone
}

struct ParsedRequest: Equatable {
    var id: Int
    var op: RpcOp
}

struct RequestParseFailure: Error, Equatable {
    /// Best effort: the id of the broken request, so the caller can still match the reply.
    var id: Int
    var message: String
}

enum RequestParser {
    static func parse(_ line: Data) -> Result<ParsedRequest, RequestParseFailure> {
        guard let object = try? JSONSerialization.jsonObject(with: line) else {
            return .failure(RequestParseFailure(id: 0, message: "not JSON"))
        }
        guard let dict = object as? [String: Any] else {
            return .failure(RequestParseFailure(id: 0, message: "not a JSON object"))
        }
        let id = (dict["id"] as? NSNumber)?.intValue ?? 0
        guard let op = dict["op"] as? String else {
            return .failure(RequestParseFailure(id: id, message: "missing op"))
        }
        switch op {
        case "hello": return .success(ParsedRequest(id: id, op: .hello))
        case "status": return .success(ParsedRequest(id: id, op: .status))
        case "ensure": return .success(ParsedRequest(id: id, op: .ensure))
        case "deleteZone": return .success(ParsedRequest(id: id, op: .deleteZone))
        case "fetch":
            if let token = dict["token"], !(token is NSNull), !(token is String) {
                return .failure(RequestParseFailure(id: id, message: "token must be a string"))
            }
            let token = dict["token"] as? String
            return .success(ParsedRequest(id: id, op: .fetch(token: token?.isEmpty == true ? nil : token)))
        case "save":
            guard let raw = dict["records"] as? [Any] else {
                return .failure(RequestParseFailure(id: id, message: "records must be an array"))
            }
            var records: [OutRecord] = []
            records.reserveCapacity(raw.count)
            for item in raw {
                guard let r = item as? [String: Any],
                    let name = r["recordName"] as? String, !name.isEmpty,
                    let type = r["recordType"] as? String, !type.isEmpty,
                    let b64 = r["payload"] as? String, let payload = Data(base64Encoded: b64)
                else {
                    return .failure(RequestParseFailure(id: id, message: "bad record"))
                }
                let tag = r["changeTag"] as? String
                records.append(OutRecord(name: name, type: type, payload: payload, changeTag: tag?.isEmpty == true ? nil : tag))
            }
            return .success(ParsedRequest(id: id, op: .save(records: records)))
        default:
            return .failure(RequestParseFailure(id: id, message: "unknown op \(op)"))
        }
    }
}

// MARK: - Replies

struct WireError: Codable, Equatable {
    var code: String
    var retryAfterMs: Int?
    var message: String?
}

struct FailureReply: Codable, Equatable {
    var id: Int
    var ok: Bool = false
    var error: WireError
}

struct EmptyReply: Codable, Equatable {
    var id: Int
    var ok: Bool = true
}

struct HelloReply: Codable, Equatable {
    var id: Int
    var ok: Bool = true
    var protocolVersion: Int
    var helperVersion: String
    /// "production" or "development" (which CloudKit environment this build talks to).
    var environment: String
    var container: String
    /// "available", "noAccount", "restricted" or "unknown" (could not tell, usually offline).
    var accountStatus: String
    var accountHash: String?
    /// Set when the helper runs but cannot reach CloudKit at all, e.g. "missingEntitlement".
    var problem: String?

    enum CodingKeys: String, CodingKey {
        case id, ok, helperVersion, environment, container, accountStatus, accountHash, problem
        case protocolVersion = "protocol"
    }
}

struct StatusReply: Codable, Equatable {
    var id: Int
    var ok: Bool = true
    var protocolVersion: Int
    var helperVersion: String
    var environment: String
    var container: String
    var accountStatus: String
    var accountHash: String?
    var problem: String?
    /// "registered", "failed", "unsupported" or "pending".
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

    enum CodingKeys: String, CodingKey {
        case id, ok, helperVersion, environment, container, accountStatus, accountHash, problem
        case apns, lastPushMs, lastTickMs, lastTickReason, lastTickExit, ticksRun, enabled
        case pollMinutes, pid, uptimeMs
        case protocolVersion = "protocol"
    }
}

struct WireChange: Codable, Equatable {
    var recordName: String
    var recordType: String
    /// base64 of the (end-to-end encrypted) field.
    var payload: String
    var changeTag: String
}

struct WireDeletion: Codable, Equatable {
    var recordName: String
    var recordType: String
}

struct FetchReply: Codable, Equatable {
    var id: Int
    var ok: Bool = true
    var changes: [WireChange]
    var deletions: [WireDeletion]
    var token: String
    var moreComing: Bool
}

struct WireSaveResult: Codable, Equatable {
    var recordName: String
    /// "saved", "conflict" or "error".
    var status: String
    var changeTag: String?
    var serverPayload: String?
    var serverChangeTag: String?
    var code: String?
}

struct SaveReply: Codable, Equatable {
    var id: Int
    var ok: Bool = true
    var results: [WireSaveResult]
}

enum WireCoder {
    static func encode<T: Encodable>(_ value: T) -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return (try? encoder.encode(value)) ?? Data("{\"ok\":false}".utf8)
    }
}
