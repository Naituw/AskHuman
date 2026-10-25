import Foundation

/// Wire protocol v1 constants, topics and shared shapes (plan §2, §3.1). Mirrors
/// `src-tauri/src/ios/wire.rs`; `ios/Fixtures/wire-v1` holds the cross-language samples.
public enum Wire {
    public static let proto = 1
    /// Key id of the pairing-channel ticket key.
    public static let kidTicket = 0
    /// Key id of the long-lived pair keys.
    public static let kidPair = 1

    public static let maxEnvelopeBytes = 900_000
    public static let maxPlainBytes = 512 * 1024
    public static let inlineRequestBytes = 128 * 1024
    public static let thumbMaxBytes = 24 * 1024
    public static let assetChunkBytes = 192 * 1024
    public static let maxAssetBytes = 10 * 1024 * 1024
    public static let maxReplyAttachments = 5
    public static let maxAnswerTextChars = 64 * 1024
    public static let maxImageEdge = 2048
    public static let maxAssetChunksPerGet = 8
    public static let maxRequestImages = 20

    public static let publishRatePerSec = 6.0
    public static let publishBurst = 10.0
    public static let watchMinIntervalMs = 2_000
    public static let watchFrameMaxBytes = 8 * 1024
    public static let maxLiveWatchPerPhone = 2
    public static let maxFollow = 5
    public static let keepaliveSecs: UInt16 = 30
    public static let pushFallbackSecs = 3
    public static let handleRefreshDays = 90
    public static let pairTicketSecs = 300
    public static let pairDoneTimeoutSecs = 120
    /// Replay window: messages whose `ts` differs from the receiver's clock by more are dropped.
    public static let clockSkewMs: Int64 = 15 * 60 * 1000
    public static let dedupWindowCount = 2048
    public static let dedupWindowSecs = 600
    public static let pushPreviewMaxChars = 120

    // MARK: topics

    public static func topicDown(user: String, phoneId: String) -> String { "ah/\(user)/d/\(phoneId)" }
    public static func topicUp(user: String, macId: String) -> String { "ah/\(user)/u/\(macId)" }
    public static func topicPresence(user: String, deviceId: String) -> String { "ah/\(user)/s/\(deviceId)" }
    public static func topicPresenceFilter(user: String) -> String { "ah/\(user)/s/+" }
    public static func topicPair(user: String, ticketId: String) -> String { "ah/\(user)/p/\(ticketId)" }

    /// Device id of a presence topic `ah/{user}/s/{device}`.
    public static func presenceDevice(user: String, topic: String) -> String? {
        let prefix = "ah/\(user)/s/"
        guard topic.hasPrefix(prefix) else { return nil }
        let rest = String(topic.dropFirst(prefix.count))
        return (!rest.isEmpty && !rest.contains("/")) ? rest : nil
    }

    /// Broker usernames become a topic level, so they must not contain wildcards or separators.
    public static func isValidBrokerUser(_ user: String) -> Bool {
        !user.isEmpty && user.utf8.count <= 64 && user.utf8.allSatisfy {
            ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x41 && $0 <= 0x5a) || ($0 >= 0x61 && $0 <= 0x7a)
        }
    }

    /// Message type strings.
    public enum T {
        public static let pairRequest = "pair.request"
        public static let pairPending = "pair.pending"
        public static let pairReject = "pair.reject"
        public static let pairAccept = "pair.accept"
        public static let pairDone = "pair.done"
        public static let hello = "hello"
        public static let helloAck = "hello.ack"
        public static let error = "error"
        public static let resync = "resync"
        public static let ping = "ping"
        public static let pong = "pong"
        public static let macInfo = "mac.info"
        public static let unpair = "unpair"
        public static let unpaired = "unpaired"
        public static let pushRegister = "push.register"
        public static let pushState = "push.state"
    }
}

/// Retained presence message (plaintext; carries no content).
public struct Presence: Codable, Equatable, Sendable {
    /// 1 online, 0 offline.
    public var o: Int
    public var t: Int64
    /// `mac` or `phone`.
    public var r: String
    /// Phone is in the foreground.
    public var fg: Int
    /// Mac gateway epoch (changes on every start).
    public var e: String?

    public init(o: Int, t: Int64, r: String, fg: Int = 0, e: String? = nil) {
        self.o = o
        self.t = t
        self.r = r
        self.fg = fg
        self.e = e
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        o = try c.decode(Int.self, forKey: .o)
        t = try c.decode(Int64.self, forKey: .t)
        r = try c.decode(String.self, forKey: .r)
        fg = try c.decodeIfPresent(Int.self, forKey: .fg) ?? 0
        e = try c.decodeIfPresent(String.self, forKey: .e)
    }
}

/// `ahpair1.` + base64url(JSON): everything a phone needs to pair (also shown as a QR code).
public struct PairCode: Codable, Equatable, Sendable {
    public var v: Int
    /// Broker host.
    public var h: String
    /// TLS port.
    public var p: Int
    /// WSS port.
    public var w: Int
    /// Broker username.
    public var u: String
    /// Broker password.
    public var pw: String
    /// Relay base URL.
    public var r: String
    /// Mac id.
    public var m: String
    /// Mac display name.
    public var mn: String
    /// Ticket id.
    public var t: String
    /// Ticket secret, base64url.
    public var ts: String
    /// Mac ephemeral public key, base64url.
    public var ek: String
    /// Expiry, unix seconds.
    public var x: Int64

    public static let prefix = "ahpair1."

    public func encode() -> String {
        let json = try! AHJSON.encoder.encode(self)
        return Self.prefix + Bytes.b64u(json)
    }

    public static func decode(_ text: String) -> PairCode? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix(prefix),
              let data = Bytes.b64uDecode(String(trimmed.dropFirst(prefix.count)))
        else { return nil }
        return try? AHJSON.decoder.decode(PairCode.self, from: data)
    }
}

/// Plaintext carried (encrypted) inside an APNs push; the notification extension decrypts it.
public struct PushPlain: Codable, Equatable, Sendable {
    /// `ask`, `confirm`, `follow.done`, `follow.waiting`, `follow.failed` or `resolved`.
    public var k: String
    /// Request id (or agent key for follow events).
    public var rid: String?
    /// Mac display name.
    public var mac: String
    public var agent: String
    public var project: String
    /// Preview text (≤ 120 characters).
    public var text: String
    public var ts: Int64

    public init(k: String, rid: String? = nil, mac: String, agent: String, project: String, text: String, ts: Int64) {
        self.k = k
        self.rid = rid
        self.mac = mac
        self.agent = agent
        self.project = project
        self.text = text
        self.ts = ts
    }
}
