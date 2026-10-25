import Foundation
import AHProtocol

/// Broker credentials and relay address, shared by every paired Mac (spec D3).
public struct BrokerSettings: Codable, Equatable, Sendable {
    public var host: String
    public var port: Int
    public var wsPort: Int
    public var username: String
    public var password: String
    public var relayUrl: String
    /// Transport that worked last (`tls`, `wss` or `tcp`); tried first on the next connect.
    public var lastTransport: String?

    public init(
        host: String, port: Int, wsPort: Int, username: String, password: String, relayUrl: String,
        lastTransport: String? = nil
    ) {
        self.host = host
        self.port = port
        self.wsPort = wsPort
        self.username = username
        self.password = password
        self.relayUrl = relayUrl
        self.lastTransport = lastTransport
    }

    public init(code: PairCode) {
        self.init(host: code.h, port: code.p, wsPort: code.w, username: code.u, password: code.pw, relayUrl: code.r)
    }

    /// Same broker account (the password may have been rotated).
    public func sameAccount(as other: BrokerSettings) -> Bool {
        host.lowercased() == other.host.lowercased() && username == other.username
    }

    /// Loopback brokers (the local test broker) are reached over plain TCP in debug builds only.
    public var isLoopback: Bool {
        ["localhost", "127.0.0.1", "::1"].contains(host.lowercased())
    }
}

/// One paired Mac: identity, display data and the direction keys.
public struct PairingRecord: Codable, Equatable, Sendable, Identifiable {
    public var macId: String
    public var macName: String
    public var alias: String?
    public var phoneId: String
    public var kid: Int
    /// Key for messages from the Mac.
    public var m2p: Data
    /// Key for messages to the Mac.
    public var p2m: Data
    public var pairedAt: Date
    public var caps: Caps

    public var id: String { macId }
    public var displayName: String { (alias?.isEmpty == false ? alias : nil) ?? macName }

    public init(
        macId: String, macName: String, alias: String? = nil, phoneId: String, kid: Int, m2p: Data, p2m: Data,
        pairedAt: Date, caps: Caps
    ) {
        self.macId = macId
        self.macName = macName
        self.alias = alias
        self.phoneId = phoneId
        self.kid = kid
        self.m2p = m2p
        self.p2m = p2m
        self.pairedAt = pairedAt
        self.caps = caps
    }
}

/// Push registration of this phone with the relay.
public struct PushRegistration: Codable, Equatable, Sendable {
    public var handle: String
    public var env: String
    public var expiresAt: Int64
    /// SHA-256 (hex) of the device token the handle was issued for.
    public var tokenDigest: String

    public init(handle: String, env: String, expiresAt: Int64, tokenDigest: String) {
        self.handle = handle
        self.env = env
        self.expiresAt = expiresAt
        self.tokenDigest = tokenDigest
    }
}

/// Typed-in state of one question: kept so a half-written answer survives leaving the page.
public struct DraftAnswer: Codable, Equatable, Sendable {
    public var selected: [Int]
    public var text: String
    public var attachments: [DraftAttachment]

    public init(selected: [Int] = [], text: String = "", attachments: [DraftAttachment] = []) {
        self.selected = selected
        self.text = text
        self.attachments = attachments
    }

    public var isEmpty: Bool { selected.isEmpty && text.isEmpty && attachments.isEmpty }
}

public struct DraftAttachment: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var mime: String
    public var size: Int
    public var isImage: Bool
    /// File name inside the draft's attachment directory.
    public var localName: String

    public init(id: String, name: String, mime: String, size: Int, isImage: Bool, localName: String) {
        self.id = id
        self.name = name
        self.mime = mime
        self.size = size
        self.isImage = isImage
        self.localName = localName
    }
}

public struct Draft: Codable, Equatable, Sendable {
    public var answers: [DraftAnswer]
    public var updatedAt: Date

    public init(answers: [DraftAnswer], updatedAt: Date = Date()) {
        self.answers = answers
        self.updatedAt = updatedAt
    }

    public var isEmpty: Bool { answers.allSatisfy(\.isEmpty) }
}

/// A submission waiting for its acknowledgement. Persisted so that a restart does not lose it;
/// retries reuse `clientMsgId`, which the Mac answers idempotently.
public struct OutboxItem: Codable, Equatable, Sendable, Identifiable {
    public enum Kind: String, Codable, Sendable { case answer, confirm, interject, task }

    public var clientMsgId: String
    public var macId: String
    public var kind: Kind
    /// Request id (answers, confirmations), agent key (interjections) or `nil`.
    public var subject: String?
    public var type: String
    public var body: JSONValue
    public var createdAt: Date
    public var attempts: Int
    /// Last failure shown to the user, if any.
    public var failure: String?

    public var id: String { clientMsgId }

    public init(
        clientMsgId: String, macId: String, kind: Kind, subject: String?, type: String, body: JSONValue,
        createdAt: Date, attempts: Int = 0, failure: String? = nil
    ) {
        self.clientMsgId = clientMsgId
        self.macId = macId
        self.kind = kind
        self.subject = subject
        self.type = type
        self.body = body
        self.createdAt = createdAt
        self.attempts = attempts
        self.failure = failure
    }
}

/// How a request ended, for the local history.
public enum HistoryOutcome: String, Codable, Sendable {
    case answered, cancelled, expired, answeredElsewhere, dismissed
}

public struct HistoryEntry: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    public var macId: String
    public var macName: String
    public var request: RequestView
    public var outcome: HistoryOutcome
    /// Channel that answered instead of this phone, for `answeredElsewhere`.
    public var by: String?
    /// What this phone sent (empty when it did not answer).
    public var answers: [AnswerItem]
    public var confirmChoice: String?
    public var confirmInput: String?
    public var receivedAt: Date
    public var finishedAt: Date

    public init(
        id: String, macId: String, macName: String, request: RequestView, outcome: HistoryOutcome, by: String? = nil,
        answers: [AnswerItem] = [], confirmChoice: String? = nil, confirmInput: String? = nil, receivedAt: Date,
        finishedAt: Date
    ) {
        self.id = id
        self.macId = macId
        self.macName = macName
        self.request = request
        self.outcome = outcome
        self.by = by
        self.answers = answers
        self.confirmChoice = confirmChoice
        self.confirmInput = confirmInput
        self.receivedAt = receivedAt
        self.finishedAt = finishedAt
    }
}

/// A request kept on the phone while it is in the inbox.
public struct CachedRequest: Codable, Equatable, Sendable, Identifiable {
    public var macId: String
    public var requestId: String
    public var summary: RequestSummary
    /// Full content once received (`request.new` with `full`, or `request.full`).
    public var view: RequestView?
    public var receivedAt: Date
    public var unread: Bool
    /// `request.ack` was sent.
    public var acked: Bool

    public var id: String { Self.key(macId, requestId) }
    public static func key(_ macId: String, _ requestId: String) -> String { "\(macId)/\(requestId)" }

    public init(
        macId: String, requestId: String, summary: RequestSummary, view: RequestView? = nil, receivedAt: Date,
        unread: Bool = true, acked: Bool = false
    ) {
        self.macId = macId
        self.requestId = requestId
        self.summary = summary
        self.view = view
        self.receivedAt = receivedAt
        self.unread = unread
        self.acked = acked
    }
}

/// Per-Mac data worth keeping across launches.
public struct MacMeta: Codable, Equatable, Sendable {
    public var lastSeenAt: Date?
    public var lastEpoch: String?
    public var follow: [String]

    public init(lastSeenAt: Date? = nil, lastEpoch: String? = nil, follow: [String] = []) {
        self.lastSeenAt = lastSeenAt
        self.lastEpoch = lastEpoch
        self.follow = follow
    }
}

/// Monthly broker traffic counters, for the "data used" line on the Mac screen.
public struct TrafficStats: Codable, Equatable, Sendable {
    public var month: String
    public var sentBytes: UInt64
    public var receivedBytes: UInt64
    public var sentMessages: UInt64
    public var receivedMessages: UInt64

    public init(month: String, sentBytes: UInt64 = 0, receivedBytes: UInt64 = 0, sentMessages: UInt64 = 0, receivedMessages: UInt64 = 0) {
        self.month = month
        self.sentBytes = sentBytes
        self.receivedBytes = receivedBytes
        self.sentMessages = sentMessages
        self.receivedMessages = receivedMessages
    }
}
