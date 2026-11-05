import Foundation

private final class FakeApple {
    var calls = [String]()
    var mutations = [String]()
    var locales = [[String: Any]]()
    var review: [String: Any] = [:]
    var group: [String: Any]?
    var assigned = false
    var audience = "APP_STORE_ELIGIBLE"
    var state = "READY_FOR_BETA_SUBMISSION"
    var testers = [[String: Any]]()
    var accountTesters = [[String: Any]]()
    var autoNotify = true
    var dropWrites = false

    func send(_ method: String, _ path: String, _ query: [String: String], _ body: [String: Any]?) async throws -> [String: Any] {
        calls.append(method + " " + path)
        if method != "GET" { mutations.append(method + " " + path) }
        let data = body?["data"] as? [String: Any] ?? [:]
        let attrs = data["attributes"] as? [String: Any] ?? [:]
        switch (method, path) {
        case ("GET", "apps/6828712684/betaAppLocalizations"): return ["data": locales]
        case ("POST", "betaAppLocalizations"):
            precondition(((data["relationships"] as! [String: Any])["app"] as! [String: Any])["data"] as? [String: String] == ["type": "apps", "id": "6828712684"])
            if !dropWrites { locales.append(["id": "locale-\(locales.count)", "attributes": attrs]) }
            return ["data": locales.last ?? [:]]
        case ("PATCH", let path) where path.hasPrefix("betaAppLocalizations/"):
            let id = path.split(separator: "/").last!
            if !dropWrites, let index = locales.firstIndex(where: { $0["id"] as? String == String(id) }) {
                locales[index]["attributes"] = ExternalStatus.attributes(locales[index]).merging(attrs) { _, new in new }
            }
            return [:]
        case ("GET", "apps/6828712684/betaAppReviewDetail"):
            return ["data": ["id": "review", "attributes": review]]
        case ("PATCH", "betaAppReviewDetails/review"):
            if !dropWrites { review = review.merging(attrs) { _, new in new } }; return [:]
        case ("GET", "builds"):
            precondition(query["filter[app]"] == "6828712684" && query["filter[version]"] == "10")
            return ["data": [["id": "build", "attributes": ["processingState": "VALID", "expired": false,
                "usesNonExemptEncryption": false, "buildAudienceType": audience]]]]
        case ("GET", "builds/build/buildBetaDetail"):
            return ["data": ["id": "detail", "attributes": ["externalBuildState": state, "autoNotifyEnabled": autoNotify]]]
        case ("PATCH", "buildBetaDetails/detail"):
            precondition(attrs["autoNotifyEnabled"] as? Bool == false)
            if !dropWrites { autoNotify = false }; return [:]
        case ("GET", "apps/6828712684/betaGroups"): return ["data": group.map { [$0] } ?? []]
        case ("POST", "betaGroups"):
            precondition(attrs["isInternalGroup"] as? Bool == false && attrs["publicLinkEnabled"] as? Bool == false)
            group = ["id": "friends", "attributes": attrs]; return ["data": group!]
        case ("GET", "betaGroups/friends/builds"): return ["data": assigned ? [["id": "build"]] : []]
        case ("GET", "betaGroups/friends/betaTesters"): return ["data": testers]
        case ("GET", "betaTesters"):
            let source = query["filter[betaGroups]"] == nil ? accountTesters : testers
            return ["data": source.filter { ExternalStatus.attributes($0)["email"] as? String == query["filter[email]"] }]
        case ("POST", "betaTesters"):
            precondition(((data["relationships"] as! [String: Any])["betaGroups"] as! [String: Any])["data"] as? [[String: String]] == [["type": "betaGroups", "id": "friends"]])
            let tester: [String: Any] = ["id": "tester-\(accountTesters.count)", "attributes": attrs.merging(["state": "NOT_INVITED"]) { _, new in new }]
            accountTesters.append(tester); testers.append(tester); return ["data": tester]
        case ("POST", "betaGroups/friends/relationships/betaTesters"):
            let id = (body?["data"] as! [[String: String]])[0]["id"]!
            let tester = accountTesters.first { $0["id"] as? String == id }!
            testers.append(tester); return [:]
        case ("POST", "betaTesterInvitations"):
            let relationships = data["relationships"] as! [String: Any]
            precondition((relationships["app"] as! [String: Any])["data"] as? [String: String] == ["type": "apps", "id": "6828712684"])
            let id = ((relationships["betaTester"] as! [String: Any])["data"] as! [String: String])["id"]!
            let index = testers.firstIndex { $0["id"] as? String == id }!
            precondition(ExternalStatus.attributes(testers[index])["state"] as? String == "NOT_INVITED")
            if !dropWrites { testers[index]["attributes"] = ExternalStatus.attributes(testers[index]).merging(["state": "INVITED"]) { _, new in new } }
            return [:]
        case ("POST", "betaGroups/friends/relationships/builds"):
            precondition(!autoNotify && testers.isEmpty)
            precondition(body?["data"] as? [[String: String]] == [["type": "builds", "id": "build"]])
            assigned = true; return [:]
        case ("POST", "betaAppReviewSubmissions"):
            precondition(((data["relationships"] as! [String: Any])["build"] as! [String: Any])["data"] as? [String: String] == ["type": "builds", "id": "build"])
            state = "WAITING_FOR_BETA_REVIEW"; return [:]
        case ("GET", "builds/build/betaAppReviewSubmission"):
            return ["data": ["id": "submission", "attributes": ["betaReviewState": "WAITING_FOR_REVIEW"]]]
        default: fatalError("Unexpected API operation: \(method) \(path)")
        }
    }
}

@main
enum ExternalTestingTests {
    static func metadata() throws -> ExternalRequest {
        let fields = ["description": "AskHuman beta", "feedbackEmail": "owner@example.com",
                      "privacyPolicyUrl": "https://example.com/privacy", "marketingUrl": "https://example.com/beta"]
        return try ExternalRequest(["mode": "metadata", "localizations": ["en-US": fields, "zh-Hans": fields],
            "review": ["contactFirstName": "First", "contactLastName": "Last", "contactPhone": "+123456789",
                       "contactEmail": "review@example.com", "demoAccountRequired": false, "notes": "Requires a Mac."]])
    }
    static func refuses(_ work: () async throws -> Void) async {
        do { try await work(); fatalError("An unsafe operation was accepted") }
        catch is ExternalStatusError {} catch { fatalError("Unexpected error: \(error)") }
    }
    static func main() async throws {
        for raw: [String: Any] in [
            ["mode": "submit", "build": "10", "appId": "another-app"],
            ["mode": "submit", "build": "../10"],
            ["mode": "invite", "build": "10", "emails": ["a@example.com", "A@example.com"]],
            ["mode": "invite", "build": "10", "emails": ["a@example.com\nBcc:other@example.com"]],
            ["mode": "delete", "build": "10"]
        ] {
            await refuses { _ = try ExternalRequest(raw) }
        }
        let apple = FakeApple(), request = try metadata()
        let api = ExternalTesting(send: apple.send)
        _ = try await api.run(request)
        precondition(apple.mutations.count == 3)
        let count = apple.mutations.count
        _ = try await api.run(request)
        precondition(apple.mutations.count == count)
        let group = try ExternalRequest(["mode": "group", "build": "10"])
        _ = try await api.run(group)
        precondition(apple.assigned && !apple.autoNotify)
        let groupCount = apple.mutations.count
        _ = try await api.run(group)
        precondition(apple.mutations.count == groupCount)
        let submit = try ExternalRequest(["mode": "submit", "build": "10"])
        _ = try await api.run(submit)
        precondition(apple.state == "WAITING_FOR_BETA_REVIEW")
        let submitted = apple.mutations.count
        _ = try await api.run(submit)
        precondition(apple.mutations.count == submitted)
        let invite = try ExternalRequest(["mode": "invite", "build": "10", "emails": ["friend@example.com"]])
        await refuses { _ = try await api.run(invite) }
        precondition(apple.mutations.count == submitted)
        apple.state = "BETA_APPROVED"
        let result = try await api.run(invite)
        precondition((result["testerStates"] as? [String: Int])?["INVITED"] == 1)
        let invited = apple.mutations.count
        _ = try await api.run(invite)
        precondition(apple.mutations.count == invited)
        apple.accountTesters.append(["id": "other-app-tester", "attributes": ["email": "existing@example.com", "state": "NOT_INVITED"]])
        _ = try await api.run(ExternalRequest(["mode": "invite", "build": "10", "emails": ["existing@example.com"]]))
        precondition(apple.mutations.filter { $0 == "POST betaTesters" }.count == 1)
        precondition(apple.mutations.filter { $0 == "POST betaTesterInvitations" }.count == 2)
        let afterInvites = apple.mutations.count
        await refuses { _ = try await api.run(ExternalRequest(["mode": "invite", "build": "10", "emails": ["a@example.com", "b@example.com", "c@example.com", "d@example.com"]])) }
        precondition(apple.mutations.count == afterInvites)
        apple.audience = "INTERNAL_ONLY"
        await refuses { _ = try await api.run(group) }
        precondition(apple.mutations.count == afterInvites)
        apple.audience = "APP_STORE_ELIGIBLE"
        apple.assigned = false
        apple.testers = [["id": "tester"]]
        await refuses { _ = try await api.run(group) }
        precondition(apple.mutations.count == afterInvites)
        apple.group?["attributes"] = ["name": ExternalTesting.groupName, "isInternalGroup": false, "publicLinkEnabled": true]
        await refuses { _ = try await api.run(group) }
        precondition(apple.mutations.count == afterInvites)
        let broken = FakeApple(); broken.dropWrites = true
        await refuses { _ = try await ExternalTesting(send: broken.send).run(request) }
        precondition(!broken.mutations.contains("POST betaAppReviewSubmissions"))
        let notifyFailure = FakeApple(); notifyFailure.dropWrites = true
        await refuses { _ = try await ExternalTesting(send: notifyFailure.send).run(group) }
        precondition(!notifyFailure.assigned)
        let invitationFailure = FakeApple()
        let failingAPI = ExternalTesting(send: invitationFailure.send)
        _ = try await failingAPI.run(group)
        invitationFailure.state = "BETA_APPROVED"; invitationFailure.dropWrites = true
        await refuses { _ = try await failingAPI.run(invite) }
        precondition(invitationFailure.mutations.filter { $0 == "POST betaTesterInvitations" }.count == 1)
        print("External mutation scope, approval gates, read-back and retry checks passed")
    }
}
