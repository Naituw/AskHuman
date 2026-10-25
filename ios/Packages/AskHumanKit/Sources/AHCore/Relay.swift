import Foundation

/// Registers the phone's APNs device token with the push relay and gets back an opaque handle that
/// the Macs use to reach it (the relay holds the APNs key; Macs never see the token).
public protocol RelayRegistering: Sendable {
    func register(relayUrl: String, deviceToken: Data, env: String, bundleId: String) async throws -> RelayHandle
}

public struct RelayHandle: Sendable, Equatable {
    public var handle: String
    /// Unix seconds.
    public var expiresAt: Int64

    public init(handle: String, expiresAt: Int64) {
        self.handle = handle
        self.expiresAt = expiresAt
    }
}

public enum RelayError: Error, Equatable, LocalizedError {
    case badURL
    case http(Int, String)
    case badResponse
    case network(String)

    public var errorDescription: String? {
        switch self {
        case .badURL: return "The relay address is invalid."
        case .http(let code, let detail): return "Relay error \(code)\(detail.isEmpty ? "" : ": \(detail)")"
        case .badResponse: return "The relay answered with something unexpected."
        case .network(let message): return message
        }
    }
}

public struct HTTPRelayClient: RelayRegistering {
    private let session: URLSession

    public init(session: URLSession = .shared) { self.session = session }

    public func register(relayUrl: String, deviceToken: Data, env: String, bundleId: String) async throws -> RelayHandle {
        let base = relayUrl.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard let url = URL(string: base + "/v1/register"), url.scheme == "https" || url.host == "localhost" || url.host == "127.0.0.1" else {
            throw RelayError.badURL
        }
        var request = URLRequest(url: url, timeoutInterval: 15)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        let token = deviceToken.map { String(format: "%02x", $0) }.joined()
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "deviceToken": token, "env": env, "bundleId": bundleId,
        ])
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw RelayError.network((error as NSError).localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else { throw RelayError.badResponse }
        let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        guard http.statusCode == 200 else {
            throw RelayError.http(http.statusCode, (object?["error"] as? String) ?? "")
        }
        guard let handle = object?["handle"] as? String, let expiresAt = (object?["expiresAt"] as? NSNumber)?.int64Value else {
            throw RelayError.badResponse
        }
        return RelayHandle(handle: handle, expiresAt: expiresAt)
    }
}
