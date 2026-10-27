import Foundation

// Message bodies of wire protocol v1 (plan §3.4). JSON keys are camelCase exactly as in
// `src-tauri/src/ios/wire.rs`; optional fields are omitted when absent and flagged booleans /
// counters when false / zero, so a decode → encode round trip reproduces the Rust output.

// MARK: shared shapes

public struct Caps: Codable, Equatable, Sendable {
    public var newTask: Bool
    public var interject: Bool
    public var watch: Bool

    public init(newTask: Bool, interject: Bool, watch: Bool) {
        self.newTask = newTask
        self.interject = interject
        self.watch = watch
    }
}

public struct SlotInfo: Codable, Equatable, Sendable {
    public var active: Bool
    public var autoActivation: Bool

    public init(active: Bool, autoActivation: Bool) {
        self.active = active
        self.autoActivation = autoActivation
    }
}

public struct AgentRef: Codable, Equatable, Hashable, Sendable {
    public var kind: String
    public var label: String

    public init(kind: String, label: String) {
        self.kind = kind
        self.label = label
    }
}

public struct ProjectRef: Codable, Equatable, Hashable, Sendable {
    public var key: String
    public var name: String

    public init(key: String, name: String) {
        self.key = key
        self.name = name
    }
}

// MARK: pairing (plan §3.3)

public struct PairRequest: WireMessage {
    public static let type = "pair.request"
    public var phoneId: String
    public var name: String
    /// Phone ephemeral X25519 public key, base64url.
    public var ek: String
    public var appVer: String
    public var proto: Int

    public init(phoneId: String, name: String, ek: String, appVer: String, proto: Int = Wire.proto) {
        self.phoneId = phoneId
        self.name = name
        self.ek = ek
        self.appVer = appVer
        self.proto = proto
    }
}

public struct PairPending: WireMessage {
    public static let type = "pair.pending"
    public var macId: String
}

public struct PairReject: WireMessage {
    public static let type = "pair.reject"
    public var reason: String
}

public struct PairAccept: WireMessage {
    public static let type = "pair.accept"
    public var macId: String
    public var macName: String
    public var caps: Caps
}

public struct PairDone: WireMessage {
    public static let type = "pair.done"
    public var name: String
    public var handle: String?
    public var env: String?
    public var expiresAt: Int64?

    public init(name: String, handle: String? = nil, env: String? = nil, expiresAt: Int64? = nil) {
        self.name = name
        self.handle = handle
        self.env = env
        self.expiresAt = expiresAt
    }
}

// MARK: handshake and sync

public struct Hello: WireMessage {
    public static let type = "hello"
    public var nonce: String
    public var proto: Int
    public var appVer: String
    public var deviceName: String

    public init(nonce: String, proto: Int = Wire.proto, appVer: String, deviceName: String) {
        self.nonce = nonce
        self.proto = proto
        self.appVer = appVer
        self.deviceName = deviceName
    }
}

public struct HelloAck: WireMessage {
    public static let type = "hello.ack"
    public var nonce: String
    /// Gateway epoch; changes on every Mac gateway start.
    public var epoch: String
    public var macName: String
    public var caps: Caps
    public var slot: SlotInfo
}

public struct ErrorBody: WireMessage {
    public static let type = "error"
    /// Message id or client message id the error refers to.
    public var ref: String
    /// `clock_skew`, `unknown_type`, `not_paired`, `rate_limited`, `internal`, …
    public var code: String
    public var message: String?
}

public struct Resync: WireMessage {
    public static let type = "resync"
    /// `inbox`, `agents`, `follow` or `slot`.
    public var stream: String

    public init(stream: String) { self.stream = stream }
}

public struct Ping: WireMessage {
    public static let type = "ping"
    public var t: Int64

    public init(t: Int64) { self.t = t }
}

public struct Pong: WireMessage {
    public static let type = "pong"
    public var t: Int64
}

public struct MacInfo: WireMessage {
    public static let type = "mac.info"
    public var macName: String
}

public struct Unpair: WireMessage {
    public static let type = "unpair"
    public init() {}
}

public struct Unpaired: WireMessage {
    public static let type = "unpaired"
    public init() {}
}

public struct PushRegister: WireMessage {
    public static let type = "push.register"
    public var handle: String
    public var env: String
    public var expiresAt: Int64

    public init(handle: String, env: String, expiresAt: Int64) {
        self.handle = handle
        self.env = env
        self.expiresAt = expiresAt
    }
}

public struct PushState: WireMessage {
    public static let type = "push.state"
    /// `ok`, `missing`, `gone` or `error`.
    public var status: String
}

// MARK: requests

public struct AssetRef: Codable, Equatable, Hashable, Sendable {
    public var assetId: String
    public var name: String
    public var size: UInt64
    public var mime: String
    public var isImage: Bool
    /// base64url JPEG thumbnail (≤ 24 KB) for images.
    public var thumb: String?
    @OmitFalse public var tooLarge: Bool

    public init(assetId: String, name: String, size: UInt64, mime: String, isImage: Bool, thumb: String? = nil, tooLarge: Bool = false) {
        self.assetId = assetId
        self.name = name
        self.size = size
        self.mime = mime
        self.isImage = isImage
        self.thumb = thumb
        self.tooLarge = tooLarge
    }
}

public struct OptionView: Codable, Equatable, Hashable, Sendable {
    public var text: String
    public var recommended: Bool
    @OmitFalse public var todo: Bool

    public init(text: String, recommended: Bool, todo: Bool = false) {
        self.text = text
        self.recommended = recommended
        self.todo = todo
    }
}

public struct QuestionView: Codable, Equatable, Hashable, Sendable {
    public var text: String
    public var options: [OptionView]

    public init(text: String, options: [OptionView]) {
        self.text = text
        self.options = options
    }
}

public struct MessageView: Codable, Equatable, Hashable, Sendable {
    public var text: String
    public var files: [AssetRef]

    public init(text: String, files: [AssetRef]) {
        self.text = text
        self.files = files
    }
}

public struct AskView: Codable, Equatable, Hashable, Sendable {
    public var isMarkdown: Bool
    public var message: MessageView
    public var questions: [QuestionView]
    public var selectOnly: Bool
    public var single: Bool
    @OmitFalse public var whatsNext: Bool

    public init(isMarkdown: Bool, message: MessageView, questions: [QuestionView], selectOnly: Bool, single: Bool, whatsNext: Bool = false) {
        self.isMarkdown = isMarkdown
        self.message = message
        self.questions = questions
        self.selectOnly = selectOnly
        self.single = single
        self.whatsNext = whatsNext
    }
}

public struct ConfirmFieldView: Codable, Equatable, Hashable, Sendable {
    public var id: String
    public var label: String
    public var value: String
    /// `text`, `path` or `timestamp`.
    public var kind: String

    public init(id: String, label: String, value: String, kind: String) {
        self.id = id
        self.label = label
        self.value = value
        self.kind = kind
    }
}

public struct ConfirmDetailView: Codable, Equatable, Hashable, Sendable {
    public var summary: String
    public var bodyMd: String

    public init(summary: String, bodyMd: String) {
        self.summary = summary
        self.bodyMd = bodyMd
    }
}

public struct ConfirmChoiceView: Codable, Equatable, Hashable, Sendable {
    public var wireIndex: Int
    public var label: String
    public var description: String
    /// Action role (`default`, `primary`, `danger`, …), lowercase.
    public var role: String
    public var recommended: Bool

    public init(wireIndex: Int, label: String, description: String, role: String, recommended: Bool) {
        self.wireIndex = wireIndex
        self.label = label
        self.description = description
        self.role = role
        self.recommended = recommended
    }
}

public struct ConfirmInputView: Codable, Equatable, Hashable, Sendable {
    public var id: String
    public var label: String
    public var placeholder: String
    public var maxChars: Int
    /// Choice index that makes the input visible; `nil` means always visible.
    public var visibleWhenIndex: Int?
    /// Whether a non-empty value is required when the input is visible.
    @OmitFalse public var required: Bool

    public init(
        id: String, label: String, placeholder: String, maxChars: Int, visibleWhenIndex: Int? = nil, required: Bool = false
    ) {
        self.id = id
        self.label = label
        self.placeholder = placeholder
        self.maxChars = maxChars
        self.visibleWhenIndex = visibleWhenIndex
        self.required = required
    }
}

public struct ConfirmView: Codable, Equatable, Hashable, Sendable {
    public var title: String
    public var context: [ConfirmFieldView]
    public var detail: ConfirmDetailView
    public var choices: [ConfirmChoiceView]
    public var defaultIndex: Int?
    public var input: ConfirmInputView?
    /// Wire index that dismissing the confirmation maps to.
    public var dismissIndex: Int?
    public var submitLabel: String
    /// Unix milliseconds.
    public var expiresAt: Int64

    public init(
        title: String, context: [ConfirmFieldView], detail: ConfirmDetailView, choices: [ConfirmChoiceView],
        defaultIndex: Int? = nil, input: ConfirmInputView? = nil, dismissIndex: Int? = nil, submitLabel: String,
        expiresAt: Int64
    ) {
        self.title = title
        self.context = context
        self.detail = detail
        self.choices = choices
        self.defaultIndex = defaultIndex
        self.input = input
        self.dismissIndex = dismissIndex
        self.submitLabel = submitLabel
        self.expiresAt = expiresAt
    }
}

public struct RequestView: Codable, Equatable, Hashable, Sendable {
    public var id: String
    /// `ask` or `confirm`.
    public var kind: String
    /// Unix milliseconds.
    public var createdAt: Int64
    public var agent: AgentRef
    public var project: ProjectRef
    /// Title of the Agent session that asks, as the Mac knew it when it delivered the request (one line);
    /// `nil` when it knew none. Older Macs never send it.
    public var sessionTitle: String?
    public var source: String
    public var contentHash: String
    @OmitFalse public var truncated: Bool
    public var ask: AskView?
    public var confirm: ConfirmView?

    public init(
        id: String, kind: String, createdAt: Int64, agent: AgentRef, project: ProjectRef, sessionTitle: String? = nil,
        source: String, contentHash: String, truncated: Bool = false, ask: AskView? = nil, confirm: ConfirmView? = nil
    ) {
        self.id = id
        self.kind = kind
        self.createdAt = createdAt
        self.agent = agent
        self.project = project
        self.sessionTitle = sessionTitle
        self.source = source
        self.contentHash = contentHash
        self.truncated = truncated
        self.ask = ask
        self.confirm = confirm
    }
}

public struct RequestSummary: Codable, Equatable, Hashable, Sendable {
    public var id: String
    public var kind: String
    public var project: ProjectRef
    public var agent: AgentRef
    /// The same title as `RequestView.sessionTitle`.
    public var sessionTitle: String?
    public var createdAt: Int64
    public var preview: String
    public var contentHash: String
    public var size: UInt64

    public init(
        id: String, kind: String, project: ProjectRef, agent: AgentRef, sessionTitle: String? = nil, createdAt: Int64,
        preview: String, contentHash: String, size: UInt64
    ) {
        self.id = id
        self.kind = kind
        self.project = project
        self.agent = agent
        self.sessionTitle = sessionTitle
        self.createdAt = createdAt
        self.preview = preview
        self.contentHash = contentHash
        self.size = size
    }
}

/// How a session title is shown: trimmed, and blank counts as none.
public enum SessionTitle {
    public static func clean(_ raw: String?) -> String? {
        guard let trimmed = raw?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else { return nil }
        return trimmed
    }
}

public struct InboxSnapshot: WireMessage {
    public static let type = "inbox.snapshot"
    public var epoch: String
    public var rev: UInt64
    public var items: [RequestSummary]
}

public struct RequestNew: WireMessage {
    public static let type = "request.new"
    public var rev: UInt64
    public var request: RequestView
    /// `false` when only the summary-level content fits; fetch the rest with `request.get`.
    public var full: Bool
}

public struct RequestGet: WireMessage {
    public static let type = "request.get"
    public var requestId: String

    public init(requestId: String) { self.requestId = requestId }
}

public struct RequestFull: WireMessage {
    public static let type = "request.full"
    public var request: RequestView
}

public struct RequestAck: WireMessage {
    public static let type = "request.ack"
    public var requestId: String

    public init(requestId: String) { self.requestId = requestId }
}

public struct RequestSeen: WireMessage {
    public static let type = "request.seen"
    public var requestId: String

    public init(requestId: String) { self.requestId = requestId }
}

public struct RequestResolved: WireMessage {
    public static let type = "request.resolved"
    public var rev: UInt64
    public var requestId: String
    /// `answered`, `cancelled`, `expired` or `answeredElsewhere`.
    public var how: String
    /// Channel display name that answered (for `answeredElsewhere`).
    public var by: String?
}

public struct AnswerItem: Codable, Equatable, Hashable, Sendable {
    /// Question index.
    public var q: Int
    /// Selected option indices.
    @DefaultEmpty public var sel: [Int]
    public var text: String?
    /// Upload ids of reply images.
    @DefaultEmpty public var images: [String]
    /// Upload ids of reply files.
    @DefaultEmpty public var files: [String]

    public init(q: Int, sel: [Int] = [], text: String? = nil, images: [String] = [], files: [String] = []) {
        self.q = q
        self.sel = sel
        self.text = text
        self.images = images
        self.files = files
    }
}

public struct AnswerSubmit: WireMessage {
    public static let type = "answer.submit"
    public var clientMsgId: String
    public var requestId: String
    /// `send` or `cancel`.
    public var action: String
    @DefaultEmpty public var answers: [AnswerItem]

    public init(clientMsgId: String, requestId: String, action: String, answers: [AnswerItem]) {
        self.clientMsgId = clientMsgId
        self.requestId = requestId
        self.action = action
        self.answers = answers
    }
}

public struct AnswerAck: WireMessage {
    public static let type = "answer.ack"
    public var clientMsgId: String
    public var requestId: String
    /// `accepted`, `lost`, `expired` or `invalid`.
    public var status: String
    public var reason: String?
}

public struct ConfirmDecide: WireMessage {
    public static let type = "confirm.decide"
    public var clientMsgId: String
    public var requestId: String
    /// Chosen wire index; `nil` with `dismiss` for the dismiss action.
    public var wireIndex: Int?
    @OmitFalse public var dismiss: Bool
    public var input: String?

    public init(clientMsgId: String, requestId: String, wireIndex: Int? = nil, dismiss: Bool = false, input: String? = nil) {
        self.clientMsgId = clientMsgId
        self.requestId = requestId
        self.wireIndex = wireIndex
        self.dismiss = dismiss
        self.input = input
    }
}

public struct ConfirmAck: WireMessage {
    public static let type = "confirm.ack"
    public var clientMsgId: String
    public var requestId: String
    public var status: String
    public var reason: String?
}

// MARK: agents, follow, watch, interject

public struct CanInterject: Codable, Equatable, Hashable, Sendable {
    public var ok: Bool
    public var reason: String?

    public init(ok: Bool, reason: String? = nil) {
        self.ok = ok
        self.reason = reason
    }
}

public struct AgentView: Codable, Equatable, Hashable, Sendable, Identifiable {
    /// `{kind}:{sessionId}`.
    public var key: String
    public var kind: String
    public var kindLabel: String
    public var sessionId: String
    public var title: String
    public var project: ProjectRef
    /// `working`, `idle`, `waiting` or `ended`.
    public var state: String
    /// Unix milliseconds.
    public var startedAt: Int64
    public var lastActivityAt: Int64
    public var activeElapsedSecs: UInt64
    public var turnSteps: UInt64
    public var currentTool: String?
    public var waitingRequestId: String?
    public var waitingPreview: String?
    @OmitZero public var pendingInterject: UInt64
    @OmitFalse public var lastTurnFailed: Bool
    public var canInterject: CanInterject
    public var followed: Bool

    public var id: String { key }

    public init(
        key: String, kind: String, kindLabel: String, sessionId: String, title: String, project: ProjectRef, state: String,
        startedAt: Int64, lastActivityAt: Int64, activeElapsedSecs: UInt64 = 0, turnSteps: UInt64 = 0, currentTool: String? = nil,
        waitingRequestId: String? = nil, waitingPreview: String? = nil, pendingInterject: UInt64 = 0, lastTurnFailed: Bool = false,
        canInterject: CanInterject = CanInterject(ok: false), followed: Bool = false
    ) {
        self.key = key
        self.kind = kind
        self.kindLabel = kindLabel
        self.sessionId = sessionId
        self.title = title
        self.project = project
        self.state = state
        self.startedAt = startedAt
        self.lastActivityAt = lastActivityAt
        self.activeElapsedSecs = activeElapsedSecs
        self.turnSteps = turnSteps
        self.currentTool = currentTool
        self.waitingRequestId = waitingRequestId
        self.waitingPreview = waitingPreview
        self.pendingInterject = pendingInterject
        self.lastTurnFailed = lastTurnFailed
        self.canInterject = canInterject
        self.followed = followed
    }
}

public struct AgentsSnapshot: WireMessage {
    public static let type = "agents.snapshot"
    public var epoch: String
    public var rev: UInt64
    public var agents: [AgentView]
}

public struct AgentsDelta: WireMessage {
    public static let type = "agents.delta"
    public var rev: UInt64
    @DefaultEmpty public var upsert: [AgentView]
    @DefaultEmpty public var remove: [String]
}

public struct FollowSet: WireMessage {
    public static let type = "follow.set"
    public var clientMsgId: String
    public var agentKey: String
    public var on: Bool

    public init(clientMsgId: String, agentKey: String, on: Bool) {
        self.clientMsgId = clientMsgId
        self.agentKey = agentKey
        self.on = on
    }
}

public struct FollowState: WireMessage {
    public static let type = "follow.state"
    public var rev: UInt64
    public var keys: [String]
    /// Set when the last `follow.set` was refused (`limit`).
    public var error: String?
}

public struct FollowEvent: WireMessage {
    public static let type = "follow.event"
    public var agentKey: String
    /// `done`, `waiting` or `failed`.
    public var kind: String
    public var title: String
    public var project: String
    public var preview: String

    public init(agentKey: String, kind: String, title: String, project: String, preview: String) {
        self.agentKey = agentKey
        self.kind = kind
        self.title = title
        self.project = project
        self.preview = preview
    }
}

public struct WatchOpen: WireMessage {
    public static let type = "watch.open"
    public var agentKey: String

    public init(agentKey: String) { self.agentKey = agentKey }
}

public struct WatchClose: WireMessage {
    public static let type = "watch.close"
    public var agentKey: String

    public init(agentKey: String) { self.agentKey = agentKey }
}

public struct WatchFrame: WireMessage {
    public static let type = "watch.frame"
    public var agentKey: String
    public var seq: UInt64
    public var frame: JSONValue
}

public struct WatchClosed: WireMessage {
    public static let type = "watch.closed"
    public var agentKey: String
    /// `ended`, `limit` or `notFound`.
    public var reason: String
}

public struct InterjectSend: WireMessage {
    public static let type = "interject.send"
    public var clientMsgId: String
    public var agentKey: String
    public var text: String
    @DefaultEmpty public var files: [String]

    public init(clientMsgId: String, agentKey: String, text: String, files: [String] = []) {
        self.clientMsgId = clientMsgId
        self.agentKey = agentKey
        self.text = text
        self.files = files
    }
}

public struct InterjectAck: WireMessage {
    public static let type = "interject.ack"
    public var clientMsgId: String
    public var agentKey: String
    /// `queued`, `delivered` or `rejected`.
    public var status: String
    public var reason: String?
    public var pending: UInt64
}

public struct InterjectClear: WireMessage {
    public static let type = "interject.clear"
    public var clientMsgId: String
    public var agentKey: String

    public init(clientMsgId: String, agentKey: String) {
        self.clientMsgId = clientMsgId
        self.agentKey = agentKey
    }
}

public struct InterjectQuery: WireMessage {
    public static let type = "interject.query"
    public var agentKey: String

    public init(agentKey: String) { self.agentKey = agentKey }
}

public struct InterjectState: WireMessage {
    public static let type = "interject.state"
    public var agentKey: String
    public var pending: UInt64
    /// Full text of the queued interjections (for recall), when requested.
    public var text: String?
}

public struct InterjectConsumed: WireMessage {
    public static let type = "interject.consumed"
    public var agentKey: String
}

// MARK: tasks and slot

public struct TaskOptionsGet: WireMessage {
    public static let type = "task.options.get"
    public init() {}
}

public struct WorkspaceOption: Codable, Equatable, Hashable, Sendable, Identifiable {
    public var id: String
    public var label: String
    public var subtitle: String
    public var pinned: Bool
    public var lastUsedAt: Int64

    public init(id: String, label: String, subtitle: String, pinned: Bool = false, lastUsedAt: Int64 = 0) {
        self.id = id
        self.label = label
        self.subtitle = subtitle
        self.pinned = pinned
        self.lastUsedAt = lastUsedAt
    }
}

/// One model an Agent offers for a new task (`docs/specs/ios-task-model.md`).
public struct ModelOption: Codable, Equatable, Hashable, Sendable, Identifiable {
    /// What `task.create` sends back as `model`.
    public var id: String
    public var label: String
    public var note: String?
    /// The reasoning efforts this model runs with; absent = this model has no effort row.
    public var efforts: [String]?
    /// The effort the model runs with when none is chosen.
    public var defaultEffort: String?

    public init(id: String, label: String, note: String? = nil, efforts: [String]? = nil, defaultEffort: String? = nil) {
        self.id = id
        self.label = label
        self.note = note
        self.efforts = efforts
        self.defaultEffort = defaultEffort
    }
}

/// What "Default" stands for: the model an Agent uses when `task.create` names none. Only shown.
public struct DefaultModel: Codable, Equatable, Hashable, Sendable {
    public var label: String?
    /// The efforts that model runs with; absent = "Default" has no effort row.
    public var efforts: [String]?
    public var defaultEffort: String?

    public init(label: String? = nil, efforts: [String]? = nil, defaultEffort: String? = nil) {
        self.label = label
        self.efforts = efforts
        self.defaultEffort = defaultEffort
    }
}

public struct AgentOption: Codable, Equatable, Hashable, Sendable {
    public var kind: String
    public var label: String
    public var ready: Bool
    public var subtitle: String
    public var permissionSelectable: Bool
    public var reason: String?
    /// The models the phone may pick besides "Default"; absent when the Mac knows none (or is older).
    public var models: [ModelOption]?
    public var defaultModel: DefaultModel?

    public init(
        kind: String, label: String, ready: Bool, subtitle: String, permissionSelectable: Bool, reason: String? = nil,
        models: [ModelOption]? = nil, defaultModel: DefaultModel? = nil
    ) {
        self.kind = kind
        self.label = label
        self.ready = ready
        self.subtitle = subtitle
        self.permissionSelectable = permissionSelectable
        self.reason = reason
        self.models = models
        self.defaultModel = defaultModel
    }

    /// A model entry that does not decode is skipped instead of failing the whole `task.options`:
    /// a glitch in the list must not take the form down, the rest of it still works.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(String.self, forKey: .kind)
        label = try c.decode(String.self, forKey: .label)
        ready = try c.decode(Bool.self, forKey: .ready)
        subtitle = try c.decode(String.self, forKey: .subtitle)
        permissionSelectable = try c.decode(Bool.self, forKey: .permissionSelectable)
        reason = try c.decodeIfPresent(String.self, forKey: .reason)
        models = try? c.decodeIfPresent([Lossy<ModelOption>].self, forKey: .models)?.compactMap(\.value)
        defaultModel = try? c.decodeIfPresent(DefaultModel.self, forKey: .defaultModel)
    }
}

/// Decodes to `nil` instead of throwing, so that one bad element leaves the rest of an array alone.
private struct Lossy<Value: Decodable>: Decodable {
    let value: Value?

    init(from decoder: Decoder) throws {
        value = try? decoder.singleValueContainer().decode(Value.self)
    }
}

public struct TaskOptions: WireMessage {
    public static let type = "task.options"
    public var enabled: Bool
    public var blocked: String?
    /// `ask`, `agentDefault` or `yolo`.
    public var permissionMode: String
    public var workspaces: [WorkspaceOption]
    public var agents: [AgentOption]

    public init(enabled: Bool, blocked: String? = nil, permissionMode: String, workspaces: [WorkspaceOption], agents: [AgentOption]) {
        self.enabled = enabled
        self.blocked = blocked
        self.permissionMode = permissionMode
        self.workspaces = workspaces
        self.agents = agents
    }
}

public struct TaskTodosGet: WireMessage {
    public static let type = "task.todos.get"
    public var workspaceId: String

    public init(workspaceId: String) { self.workspaceId = workspaceId }
}

public struct TodoOption: Codable, Equatable, Hashable, Sendable, Identifiable {
    public var id: String
    public var text: String
    public var auto: Bool
    public var attachments: UInt64

    public init(id: String, text: String, auto: Bool = false, attachments: UInt64 = 0) {
        self.id = id
        self.text = text
        self.auto = auto
        self.attachments = attachments
    }
}

public struct TaskTodos: WireMessage {
    public static let type = "task.todos"
    public var workspaceId: String
    public var todos: [TodoOption]
    public var overflow: UInt64
}

public struct TaskCreate: WireMessage {
    public static let type = "task.create"
    public var clientMsgId: String
    public var workspaceId: String
    public var agentKind: String
    /// `default` or `yolo`.
    public var permission: String
    public var todoId: String?
    public var text: String
    /// A model id from `task.options`; absent = the Agent's own default.
    public var model: String?
    /// An effort offered for that model (or for "Default"); absent = the Agent's own.
    public var effort: String?

    public init(
        clientMsgId: String, workspaceId: String, agentKind: String, permission: String, todoId: String? = nil,
        model: String? = nil, effort: String? = nil, text: String
    ) {
        self.clientMsgId = clientMsgId
        self.workspaceId = workspaceId
        self.agentKind = agentKind
        self.permission = permission
        self.todoId = todoId
        self.text = text
        self.model = model
        self.effort = effort
    }
}

public struct TaskStatus: WireMessage {
    public static let type = "task.status"
    public var clientMsgId: String
    /// `accepted`, `launching`, `launched`, `watching`, `unmatched` or `failed`.
    public var stage: String
    public var reason: String?
    public var agentKey: String?
}

public struct SlotTake: WireMessage {
    public static let type = "slot.take"
    public var clientMsgId: String

    public init(clientMsgId: String) { self.clientMsgId = clientMsgId }
}

public struct SlotState: WireMessage {
    public static let type = "slot.state"
    public var rev: UInt64
    public var active: Bool
    public var autoActivation: Bool
}

// MARK: assets and uploads

public struct AssetGet: WireMessage {
    public static let type = "asset.get"
    public var assetId: String
    public var from: UInt64
    public var count: UInt64

    public init(assetId: String, from: UInt64, count: UInt64) {
        self.assetId = assetId
        self.from = from
        self.count = count
    }
}

public struct AssetChunk: WireMessage {
    public static let type = "asset.chunk"
    public var assetId: String
    public var index: UInt64
    /// Number of chunks of the whole file.
    public var total: UInt64
    /// Size of the whole file in bytes.
    public var size: UInt64
    /// SHA-256 (hex) of the whole file, present on the first chunk only.
    public var sha256: String?
    /// base64url chunk data.
    public var data: String
}

public struct AssetError: WireMessage {
    public static let type = "asset.error"
    public var assetId: String
    /// `notFound`, `tooLarge` or `internal`.
    public var reason: String
}

public struct UploadScope: Codable, Equatable, Hashable, Sendable {
    public var requestId: String?
    public var agentKey: String?

    public init(requestId: String? = nil, agentKey: String? = nil) {
        self.requestId = requestId
        self.agentKey = agentKey
    }
}

public struct UploadBegin: WireMessage {
    public static let type = "upload.begin"
    public var uploadId: String
    public var name: String
    public var mime: String
    public var size: UInt64
    public var chunks: UInt64
    public var sha256: String
    public var scope: UploadScope

    public init(uploadId: String, name: String, mime: String, size: UInt64, chunks: UInt64, sha256: String, scope: UploadScope) {
        self.uploadId = uploadId
        self.name = name
        self.mime = mime
        self.size = size
        self.chunks = chunks
        self.sha256 = sha256
        self.scope = scope
    }
}

public struct UploadChunk: WireMessage {
    public static let type = "upload.chunk"
    public var uploadId: String
    public var index: UInt64
    /// base64url chunk data.
    public var data: String

    public init(uploadId: String, index: UInt64, data: String) {
        self.uploadId = uploadId
        self.index = index
        self.data = data
    }
}

public struct UploadAck: WireMessage {
    public static let type = "upload.ack"
    public var uploadId: String
    public var nextIndex: UInt64
    /// `ok`, `complete` or `error`.
    public var status: String
    public var reason: String?
}

public struct UploadCancel: WireMessage {
    public static let type = "upload.cancel"
    public var uploadId: String

    public init(uploadId: String) { self.uploadId = uploadId }
}

// MARK: registry

/// Every message type of the catalog, for the cross-language fixtures and strict validation.
public enum MessageRegistry {
    public static let all: [any WireMessage.Type] = [
        PairRequest.self, PairPending.self, PairReject.self, PairAccept.self, PairDone.self,
        Hello.self, HelloAck.self, ErrorBody.self, Resync.self, Ping.self, Pong.self, MacInfo.self,
        Unpair.self, Unpaired.self, PushRegister.self, PushState.self, InboxSnapshot.self,
        RequestNew.self, RequestGet.self, RequestFull.self, RequestAck.self, RequestSeen.self,
        RequestResolved.self, AnswerSubmit.self, AnswerAck.self, ConfirmDecide.self, ConfirmAck.self,
        AgentsSnapshot.self, AgentsDelta.self, FollowSet.self, FollowState.self, FollowEvent.self,
        WatchOpen.self, WatchClose.self, WatchFrame.self, WatchClosed.self, InterjectSend.self,
        InterjectAck.self, InterjectClear.self, InterjectQuery.self, InterjectState.self,
        InterjectConsumed.self, TaskOptionsGet.self, TaskOptions.self, TaskTodosGet.self,
        TaskTodos.self, TaskCreate.self, TaskStatus.self, SlotTake.self, SlotState.self,
        AssetGet.self, AssetChunk.self, AssetError.self, UploadBegin.self, UploadChunk.self,
        UploadAck.self, UploadCancel.self,
    ]

    /// Strictly decodes `body` as the message type `type` and encodes it back; `nil` for an
    /// unknown type.
    public static func normalize(type: String, body: JSONValue) -> Result<JSONValue, Error>? {
        guard let model = all.first(where: { $0.type == type }) else { return nil }
        return Result {
            func roundTrip<M: WireMessage>(_: M.Type) throws -> JSONValue {
                try AHJSON.tree(try AHJSON.value(M.self, from: body))
            }
            return try roundTrip(model)
        }
    }
}
