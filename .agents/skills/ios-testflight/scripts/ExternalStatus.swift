import Foundation

struct ExternalStatusError: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}

/// Read-only inspection. Other applications are read only by the explicit contact-sources operation.
/// Authentication stays in the signed entry point; no operation accepts an API path from its caller.
struct ExternalStatus {
    static let appID = "6828712684"
    typealias Get = (String, [String: String]) async throws -> [String: Any]
    let get: Get

    static func attributes(_ resource: [String: Any]) -> [String: Any] {
        resource["attributes"] as? [String: Any] ?? [:]
    }

    static func resource(_ response: [String: Any]) -> [String: Any] {
        response["data"] as? [String: Any] ?? [:]
    }

    static func resources(_ response: [String: Any]) -> [[String: Any]] {
        response["data"] as? [[String: Any]] ?? []
    }

    private static func select(_ resource: [String: Any], _ fields: Set<String>) -> [String: Any] {
        var result = resource.filter { ["id", "type"].contains($0.key) }
        result["attributes"] = attributes(resource).filter { fields.contains($0.key) }
        return result
    }

    static func id(_ resource: [String: Any]) throws -> String {
        guard let value = resource["id"] as? String, !value.isEmpty,
              value.range(of: "^[A-Za-z0-9-]+$", options: .regularExpression) != nil else {
            throw ExternalStatusError("Apple returned an invalid resource ID.")
        }
        return value
    }

    private func list(_ path: String, _ query: [String: String] = [:]) async throws -> [[String: Any]] {
        var query = query
        query["limit"] = "200"
        let result = try await get(path, query)
        // Never silently present an incomplete roster as the complete account state.
        if let next = (result["links"] as? [String: Any])?["next"] as? String, !next.isEmpty {
            throw ExternalStatusError("The inspection exceeds one page at \(path); add bounded pagination before relying on this result.")
        }
        return Self.resources(result)
    }

    func inspect() async throws -> [String: Any] {
        let root = "apps/" + Self.appID
        let localizations = try await list(root + "/betaAppLocalizations", [
            "fields[betaAppLocalizations]": "locale,description,feedbackEmail,marketingUrl,privacyPolicyUrl"
        ])
        let review = Self.resource(try await get(root + "/betaAppReviewDetail", [
            "fields[betaAppReviewDetails]": "contactFirstName,contactLastName,contactPhone,contactEmail,demoAccountRequired,notes"
        ]))
        let groups = try await list(root + "/betaGroups", [
            "fields[betaGroups]": "name,isInternalGroup,hasAccessToAllBuilds,publicLinkEnabled"
        ])
        let builds = Self.resources(try await get("builds", [
            "filter[app]": Self.appID, "sort": "-uploadedDate", "limit": "10",
            "fields[builds]": "version,processingState,expired,expirationDate,buildAudienceType,usesNonExemptEncryption"
        ]))
        var inspectedBuilds = [[String: Any]]()
        for build in builds {
            let id = try Self.id(build)
            let detail = Self.resource(try await get("builds/\(id)/buildBetaDetail", [:]))
            inspectedBuilds.append(["id": id, "attributes": Self.attributes(build),
                                    "betaDetail": Self.attributes(detail)])
        }
        var inspectedGroups = [[String: Any]]()
        for group in groups {
            let id = try Self.id(group)
            let members = try await list("betaGroups/\(id)/betaTesters", ["fields[betaTesters]": "state"])
            let assigned = try await list("betaGroups/\(id)/builds", ["fields[builds]": "version"])
            var counts = [String: Int]()
            for member in members { counts[Self.attributes(member)["state"] as? String ?? "UNKNOWN", default: 0] += 1 }
            inspectedGroups.append(["id": id, "attributes": Self.attributes(group),
                                    "testerStates": counts, "builds": assigned])
        }
        let safeReview = Self.select(review, ["contactFirstName", "contactLastName", "contactPhone", "contactEmail", "demoAccountRequired", "notes"])
        return ["appId": Self.appID, "localizations": localizations, "reviewDetail": safeReview,
                "groups": inspectedGroups, "builds": inspectedBuilds]
    }

    /// Recover the owner's existing review contact details without reading demo credentials or app content.
    func contacts() async throws -> [String: Any] {
        let apps = try await list("apps", ["fields[apps]": "name,bundleId"])
        var sources = [[String: Any]]()
        for app in apps {
            let id = try Self.id(app)
            let review = Self.resource(try await get("apps/\(id)/betaAppReviewDetail", [
                "fields[betaAppReviewDetails]": "contactFirstName,contactLastName,contactPhone,contactEmail"
            ]))
            let localizations = try await list("apps/\(id)/betaAppLocalizations", [
                "fields[betaAppLocalizations]": "locale,feedbackEmail,privacyPolicyUrl,marketingUrl"
            ])
            let contact = Self.attributes(Self.select(review, ["contactFirstName", "contactLastName", "contactPhone", "contactEmail"]))
            let locales = localizations.map { Self.select($0, ["locale", "feedbackEmail", "privacyPolicyUrl", "marketingUrl"]) }
            sources.append(["appId": id, "appName": Self.attributes(app)["name"] ?? "",
                            "reviewContact": contact, "localizations": locales])
        }
        return ["contactSources": sources]
    }
}

/// The signed helper uses this transport; tests inject dictionaries and never access credentials.
final class ExternalStatusTransport: NSObject, URLSessionTaskDelegate {
    private let authorization: String
    private lazy var session = URLSession(configuration: .ephemeral, delegate: self, delegateQueue: nil)

    init(authorization: String) { self.authorization = authorization }
    func close() { session.invalidateAndCancel() }

    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
                    completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }

    func get(_ path: String, _ query: [String: String]) async throws -> [String: Any] {
        try await send("GET", path, query, nil)
    }

    func send(_ method: String, _ path: String, _ query: [String: String], _ body: [String: Any]?) async throws -> [String: Any] {
        guard ["GET", "POST", "PATCH"].contains(method) else { throw ExternalStatusError("Unsupported API method.") }
        guard path.range(of: "^[A-Za-z0-9/-]+$", options: .regularExpression) != nil,
              !path.contains("//"), !path.hasPrefix("/") else {
            throw ExternalStatusError("Invalid inspection path.")
        }
        var url = URLComponents(string: "https://api.appstoreconnect.apple.com/v1/" + path)!
        url.queryItems = query.map { URLQueryItem(name: $0.key, value: $0.value) }
        var request = URLRequest(url: url.url!, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 30)
        request.httpMethod = method
        request.setValue("Bearer " + authorization, forHTTPHeaderField: "Authorization")
        if let body {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let (data, response) = try await session.data(for: request)
        guard let response = response as? HTTPURLResponse, (200...299).contains(response.statusCode) else {
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            let codes = (body?["errors"] as? [[String: Any]] ?? []).compactMap { $0["code"] as? String }.joined(separator: ",")
            throw ExternalStatusError("Apple \(method) failed at \(path) (HTTP \(status), \(codes)); check current state before retrying.")
        }
        if data.isEmpty { return [:] }
        guard let body = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw ExternalStatusError("Apple returned invalid JSON.")
        }
        return body
    }
}
