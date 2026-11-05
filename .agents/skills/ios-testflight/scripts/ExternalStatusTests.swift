import Foundation

@main
enum ExternalStatusTests {
    static func main() async throws {
        let template: [String: Any] = ["testFlightInternalTestingOnly": true, "teamID": "team", "destination": "upload"]
        let internalOptions = TestFlightAudience.internal.exportOptions(from: template, upload: true)
        let externalOptions = TestFlightAudience.external.exportOptions(from: template, upload: false)
        precondition(internalOptions["testFlightInternalTestingOnly"] as? Bool == true)
        precondition(externalOptions["testFlightInternalTestingOnly"] as? Bool == false)
        precondition(externalOptions["destination"] as? String == "export")
        precondition(externalOptions["teamID"] as? String == "team")
        precondition(template["testFlightInternalTestingOnly"] as? Bool == true)
        precondition(TestFlightAudience(rawValue: "public") == nil)
        var calls = [(String, [String: String])]()
        let inspector = ExternalStatus { path, query in
            calls.append((path, query))
            switch path {
            case "apps": return ["data": [["id": "123", "attributes": ["name": "Earlier App"]]]]
            case "apps/123/betaAppReviewDetail":
                precondition(!query.values.joined().contains("demoAccountPassword"))
                precondition(!query.values.joined().contains("notes"))
                return ["data": ["id": "review", "attributes": ["contactEmail": "owner@example.com", "demoAccountPassword": "must-not-escape"]]]
            case "apps/123/betaAppLocalizations":
                precondition(!query.values.joined().contains("description"))
                return ["data": []]
            case "apps/6828712684/betaAppLocalizations": return ["data": []]
            case "apps/6828712684/betaAppReviewDetail": return ["data": ["id": "review", "attributes": [:]]]
            case "apps/6828712684/betaGroups": return ["data": [
                ["id": "internal", "attributes": ["isInternalGroup": true]],
                ["id": "external", "attributes": ["isInternalGroup": false, "publicLinkEnabled": false]]
            ]]
            case "builds": return ["data": [["id": "build", "attributes": [
                "version": "9", "processingState": "VALID", "buildAudienceType": "APP_STORE_ELIGIBLE"
            ]]]]
            case "builds/build/buildBetaDetail": return ["data": ["attributes": ["externalBuildState": "WAITING_FOR_BETA_REVIEW"]]]
            case "betaGroups/internal/betaTesters", "betaGroups/external/betaTesters":
                return ["data": [["attributes": ["state": "INVITED"]]]]
            case "betaGroups/internal/builds", "betaGroups/external/builds": return ["data": []]
            default: fatalError("Unexpected endpoint \(path)")
            }
        }
        let report = try await inspector.inspect()
        precondition((report["groups"] as? [[String: Any]])?.count == 2)
        let build = (report["builds"] as! [[String: Any]])[0]
        precondition((build["attributes"] as! [String: Any])["processingState"] as? String == "VALID")
        precondition((build["betaDetail"] as! [String: Any])["externalBuildState"] as? String == "WAITING_FOR_BETA_REVIEW")
        precondition(!calls.contains { $0.0 == "apps" })
        let contacts = try await inspector.contacts()
        precondition((contacts["contactSources"] as? [[String: Any]])?.count == 1)
        let contactsJSON = try JSONSerialization.data(withJSONObject: contacts)
        precondition(!String(data: contactsJSON, encoding: .utf8)!.contains("must-not-escape"))
        do {
            _ = try ExternalStatus.id(["id": "../../other"])
            fatalError("Path injection was accepted")
        } catch is ExternalStatusError {}
        let paginated = ExternalStatus { _, _ in ["data": [], "links": ["next": "https://example.com/next"]] }
        do {
            _ = try await paginated.contacts()
            fatalError("Incomplete contact list was accepted")
        } catch is ExternalStatusError {}
        print("External inspection checks passed")
    }
}
