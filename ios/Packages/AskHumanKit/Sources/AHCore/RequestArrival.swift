import Foundation
import AHStore

/// A transient, foreground arrival. Finishing its animation never marks the request as read.
public struct RequestArrival: Identifiable, Sendable, Equatable {
    public let id: String
    public let macId: String
    public let requestId: String
    public let receivedAt: Date

    public init(macId: String, requestId: String, receivedAt: Date = Date()) {
        id = CachedRequest.key(macId, requestId)
        self.macId = macId
        self.requestId = requestId
        self.receivedAt = receivedAt
    }
}
