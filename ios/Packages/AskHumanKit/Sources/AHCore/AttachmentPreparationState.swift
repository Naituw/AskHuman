import Foundation
import Observation

/// The preparation of files belongs to its draft, even when that form leaves the screen.
@MainActor
@Observable
public final class AttachmentPreparationState {
    public var working = false
    public var status = ""
    public var fraction: Double?
    public var problem: String?
    @ObservationIgnored public var operation: Task<Void, Never>?

    public init() {}

    public func cancel() { operation?.cancel() }
}
