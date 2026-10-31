import Foundation

/// Whether a retained request can be sent to its own Mac right now.
public enum RequestConnection: Sendable, Equatable {
    case ready
    case connecting
    case reconnecting
    case refused
    case macOffline
    case unavailable
}

extension EngineState {
    public func requestConnection(macId: String) -> RequestConnection {
        guard let mac = macs.first(where: { $0.id == macId }) else { return .unavailable }
        // A cached Mac link cannot establish reachability while the phone is reconnecting.
        switch phase {
        case .idle, .connecting: return .connecting
        case .backoff: return .reconnecting
        case .refused: return .refused
        case .connected:
            switch mac.link {
            case .online: return .ready
            case .connecting: return .connecting
            case .macOffline: return .macOffline
            case .brokerDown: return .reconnecting
            }
        }
    }
}
