import Foundation
import AHProtocol
import AHStore

// MARK: connection

public enum ConnectionPhase: Sendable, Equatable {
    /// Nothing to do: the app is in the background, or no Mac is paired.
    case idle
    case connecting(via: String)
    case connected(via: String)
    /// Waiting before the next attempt.
    case backoff(retryAt: Date, reason: String)
    /// The broker refused the credentials or the subscriptions; retrying cannot help.
    case refused(String)
}

/// How reachable one Mac is right now.
public enum MacLink: Sendable, Equatable {
    /// The broker link is coming up, or the handshake with the Mac is in progress.
    case connecting
    /// Handshake and snapshots done: requests flow.
    case online
    /// The broker is reachable but the Mac is not.
    case macOffline
    /// The broker cannot be reached.
    case brokerDown
}

public struct MacState: Identifiable, Sendable, Equatable {
    public var id: String
    /// Name shown to the user: the alias when set, else the Mac's own name.
    public var name: String
    public var macName: String
    public var alias: String?
    public var link: MacLink
    public var caps: Caps
    public var slot: SlotInfo?
    public var pairedAt: Date
    public var lastSeenAt: Date?
    public var pendingCount: Int
    /// Push registration status the Mac reported (`ok`, `missing`, `gone`, `error`).
    public var pushStatus: String?

    public init(
        id: String, name: String, macName: String, alias: String?, link: MacLink, caps: Caps, slot: SlotInfo?,
        pairedAt: Date, lastSeenAt: Date?, pendingCount: Int, pushStatus: String?
    ) {
        self.id = id
        self.name = name
        self.macName = macName
        self.alias = alias
        self.link = link
        self.caps = caps
        self.slot = slot
        self.pairedAt = pairedAt
        self.lastSeenAt = lastSeenAt
        self.pendingCount = pendingCount
        self.pushStatus = pushStatus
    }
}

// MARK: requests

public enum RequestState: Sendable, Equatable {
    case pending
    /// Submitted, waiting for the Mac's acknowledgement.
    case sending(clientMsgId: String, since: Date)
    /// The last submission did not get through (timeout, rejected as invalid, …).
    case failed(clientMsgId: String, reason: String)
}

public struct InboxItem: Identifiable, Sendable, Equatable {
    public var id: String
    public var macId: String
    public var macName: String
    public var requestId: String
    public var summary: RequestSummary
    /// Full content once it arrived; `nil` while only the summary is known.
    public var view: RequestView?
    public var receivedAt: Date
    public var unread: Bool
    public var state: RequestState

    public init(
        macId: String, macName: String, requestId: String, summary: RequestSummary, view: RequestView?,
        receivedAt: Date, unread: Bool, state: RequestState
    ) {
        self.id = CachedRequest.key(macId, requestId)
        self.macId = macId
        self.macName = macName
        self.requestId = requestId
        self.summary = summary
        self.view = view
        self.receivedAt = receivedAt
        self.unread = unread
        self.state = state
    }
}

// MARK: pairing

public enum PairingFailure: Sendable, Equatable {
    case invalidCode
    case expired
    /// The code belongs to another broker account than the Macs already paired.
    case differentBroker
    case brokerUnreachable(String)
    case authFailed
    /// The Mac user refused (`rejected`, `cancelled`, `unsupported`, `invalid`).
    case rejected(String)
    case timeout
    case failed(String)
}

public enum PairingStage: Sendable, Equatable {
    /// Connecting to the broker.
    case connecting
    /// The request is out; the Mac has not reacted yet.
    case waitingForMac
    /// The Mac shows its confirmation dialog.
    case macConfirming
    /// The Mac allowed it; finishing the handshake.
    case finishing
    case done(macId: String)
    case failed(PairingFailure)
}

public struct PairingState: Sendable, Equatable {
    public var stage: PairingStage
    public var macName: String
    /// Six-digit check code, to compare with the one on the Mac.
    public var sas: String?
    public var expiresAt: Date

    public init(stage: PairingStage, macName: String, sas: String?, expiresAt: Date) {
        self.stage = stage
        self.macName = macName
        self.sas = sas
        self.expiresAt = expiresAt
    }
}

// MARK: push

public enum PushStatus: Sendable, Equatable {
    /// No device token yet (permission not asked or denied).
    case noToken
    /// The Mac pairing code did not carry a relay address.
    case noRelay
    case registering
    case registered(expiresAt: Date)
    case failed(String)
}

// MARK: engine state

/// Everything the UI shows, as one value. The engine publishes a fresh snapshot after every change.
public struct EngineState: Sendable, Equatable {
    public var phase: ConnectionPhase
    public var foreground: Bool
    public var macs: [MacState]
    /// Pending requests of every Mac, newest first.
    public var inbox: [InboxItem]
    public var pairing: PairingState?
    /// A message was dropped because the clocks of the phone and a Mac disagree by too much.
    public var clockSkew: Bool
    public var push: PushStatus
    /// Bumps whenever the local history changed.
    public var historyRevision: Int
    public var traffic: TrafficStats?
    /// `host` of the broker (for the Mac screen); `nil` before the first pairing.
    public var brokerHost: String?

    public init(
        phase: ConnectionPhase = .idle, foreground: Bool = false, macs: [MacState] = [], inbox: [InboxItem] = [],
        pairing: PairingState? = nil, clockSkew: Bool = false, push: PushStatus = .noToken, historyRevision: Int = 0,
        traffic: TrafficStats? = nil, brokerHost: String? = nil
    ) {
        self.phase = phase
        self.foreground = foreground
        self.macs = macs
        self.inbox = inbox
        self.pairing = pairing
        self.clockSkew = clockSkew
        self.push = push
        self.historyRevision = historyRevision
        self.traffic = traffic
        self.brokerHost = brokerHost
    }
}

/// One-off happenings the UI reacts to (notifications to clear, toasts, haptics).
public enum EngineEvent: Sendable, Equatable {
    case requestArrived(macId: String, requestId: String)
    case requestFinished(macId: String, requestId: String, outcome: HistoryOutcome, by: String?)
    case macUnpaired(macId: String, macName: String)
    case pairingFinished(macId: String)
}

public enum BackendError: Error, Sendable, Equatable, LocalizedError {
    /// The Mac is not reachable, so the message cannot be sent (nothing is queued).
    case macOffline
    case unknownRequest
    case unknownMac
    case tooLarge
    case invalid(String)
    case timedOut
    case cancelled
    case unavailable(String)

    public var errorDescription: String? {
        switch self {
        case .macOffline: return "The Mac is offline."
        case .unknownRequest: return "The request is no longer available."
        case .unknownMac: return "Unknown Mac."
        case .tooLarge: return "The file is too large."
        case .invalid(let reason): return reason
        case .timedOut: return "Timed out."
        case .cancelled: return "Cancelled."
        case .unavailable(let reason): return reason
        }
    }
}

/// A file the user attached to a reply (read from the photo library, camera or files app).
public struct UploadSource: Sendable, Equatable {
    public var name: String
    public var mime: String
    public var data: Data
    public var isImage: Bool

    public init(name: String, mime: String, data: Data, isImage: Bool) {
        self.name = name
        self.mime = mime
        self.data = data
        self.isImage = isImage
    }
}
