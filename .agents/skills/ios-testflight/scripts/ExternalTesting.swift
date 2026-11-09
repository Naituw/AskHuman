import Foundation
import CoreFoundation

/// A closed request vocabulary for this app's beta delivery. No arbitrary endpoints or app IDs.
struct ExternalRequest {
    enum Mode: String { case metadata, group, update, submit, start, invite }
    let mode: Mode
    let build: String?
    let localizations: [String: [String: String]]
    let review: [String: Any]
    let emails: [String]

    init(_ value: [String: Any]) throws {
        guard let raw = value["mode"] as? String, let mode = Mode(rawValue: raw) else {
            throw ExternalStatusError("Unknown external testing operation.")
        }
        self.mode = mode
        let allowed: Set<String> = mode == .metadata ? ["mode", "localizations", "review"]
            : mode == .invite ? ["mode", "build", "emails"] : ["mode", "build"]
        guard Set(value.keys).isSubset(of: allowed) else { throw ExternalStatusError("Unexpected external testing fields.") }
        build = value["build"] as? String
        if mode != .metadata {
            guard let build, build.range(of: "^[1-9][0-9]{0,8}$", options: .regularExpression) != nil else {
                throw ExternalStatusError("A valid build number is required.")
            }
        }
        localizations = value["localizations"] as? [String: [String: String]] ?? [:]
        review = value["review"] as? [String: Any] ?? [:]
        emails = value["emails"] as? [String] ?? []
        if mode == .metadata {
            guard Set(localizations.keys) == Set(["en-US", "zh-Hans"]) else {
                throw ExternalStatusError("Metadata requires both en-US and zh-Hans.")
            }
            for fields in localizations.values {
                guard Set(fields.keys) == Set(["description", "feedbackEmail", "privacyPolicyUrl", "marketingUrl"]) else {
                    throw ExternalStatusError("Each locale requires description, feedbackEmail, privacyPolicyUrl and marketingUrl.")
                }
                for text in fields.values { try Self.text(text) }
                try Self.email(fields["feedbackEmail"]!)
                for key in ["privacyPolicyUrl", "marketingUrl"] {
                    guard let url = URLComponents(string: fields[key]!), url.scheme == "https", url.host != nil,
                          url.user == nil, url.password == nil else { throw ExternalStatusError("Public URLs must use HTTPS without credentials.") }
                }
            }
            let keys = ["contactFirstName", "contactLastName", "contactPhone", "contactEmail", "notes"]
            guard Set(review.keys) == Set(keys + ["demoAccountRequired"]),
                  let required = review["demoAccountRequired"] as? NSNumber,
                  CFGetTypeID(required) == CFBooleanGetTypeID(), !required.boolValue else {
                throw ExternalStatusError("Review metadata requires a complete contact, notes and demoAccountRequired=false.")
            }
            for key in keys {
                guard let text = review[key] as? String else { throw ExternalStatusError("Invalid review contact or notes.") }
                try Self.text(text)
            }
            try Self.email(review["contactEmail"] as! String)
        }
        if mode == .invite {
            guard (1...5).contains(emails.count), Set(emails.map { $0.lowercased() }).count == emails.count else {
                throw ExternalStatusError("Provide one to five distinct, explicitly authorized tester emails.")
            }
            for email in emails { try Self.email(email) }
        }
    }

    private static func text(_ text: String) throws {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, text.utf16.count <= 4000,
              !text.unicodeScalars.contains(where: { ($0.value < 32 && $0.value != 10) || $0.value == 127 }) else {
            throw ExternalStatusError("Metadata must be nonempty plain text of at most 4000 UTF-16 units.")
        }
    }

    private static func email(_ email: String) throws {
        guard email.utf8.count <= 254,
              email.range(of: "^[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9-]+(?:\\.[A-Za-z0-9-]+)+$", options: .regularExpression) != nil else {
            throw ExternalStatusError("Invalid email address.")
        }
    }
}

struct ExternalTesting {
    static let groupName = "AskHuman Friends"
    static let appID = ExternalStatus.appID
    typealias Send = (String, String, [String: String], [String: Any]?) async throws -> [String: Any]
    let send: Send
    private func get(_ path: String, _ query: [String: String] = [:]) async throws -> [String: Any] {
        try await send("GET", path, query, nil)
    }
    private func list(_ path: String, _ query: [String: String] = [:]) async throws -> [[String: Any]] {
        var query = query; query["limit"] = "200"
        let result = try await get(path, query)
        if let next = (result["links"] as? [String: Any])?["next"] as? String, !next.isEmpty {
            throw ExternalStatusError("Refusing to mutate an incomplete paginated result.")
        }
        return ExternalStatus.resources(result)
    }
    private func write(_ method: String, _ path: String, type: String, id: String? = nil,
                       attributes: [String: Any]? = nil, relationships: [String: Any]? = nil) async throws -> [String: Any] {
        var data: [String: Any] = ["type": type]
        if let id { data["id"] = id }
        if let attributes { data["attributes"] = attributes }
        if let relationships { data["relationships"] = relationships }
        return try await send(method, path, [:], ["data": data])
    }
    private func relation(_ type: String, _ id: String) -> [String: Any] { ["data": ["type": type, "id": id]] }
    private func equal(_ left: [String: Any], _ right: [String: Any]) -> Bool {
        NSDictionary(dictionary: left).isEqual(to: right)
    }

    func run(_ request: ExternalRequest) async throws -> [String: Any] {
        if request.mode == .metadata { return try await metadata(request) }
        let matches = try await list("builds", ["filter[app]": Self.appID, "filter[version]": request.build!])
        guard matches.count == 1 else { throw ExternalStatusError("The requested build must resolve uniquely in AskHuman Mobile.") }
        let build = matches[0], id = try ExternalStatus.id(build), attrs = ExternalStatus.attributes(build)
        guard attrs["processingState"] as? String == "VALID", attrs["expired"] as? Bool == false,
              attrs["buildAudienceType"] as? String == "APP_STORE_ELIGIBLE",
              attrs["usesNonExemptEncryption"] as? Bool == false else {
            throw ExternalStatusError("Build must be VALID, unexpired, externally eligible and have its encryption declaration resolved.")
        }
        let detail = ExternalStatus.resource(try await get("builds/\(id)/buildBetaDetail"))
        let state = ExternalStatus.attributes(detail)["externalBuildState"] as? String ?? "UNKNOWN"
        if request.mode == .group { return try await group(buildID: id, detail: detail) }
        if request.mode == .update { return try await updateGroup(buildID: id, detail: detail) }
        let groups = try await friendsGroups()
        guard groups.count == 1 else { throw ExternalStatusError("Prepare the AskHuman Friends external group first.") }
        let group = groups[0], groupID = try ExternalStatus.id(group)
        try checkGroup(group)
        let assigned = try await list("betaGroups/\(groupID)/builds", ["fields[builds]": "version"])
        guard assigned.contains(where: { $0["id"] as? String == id }) else {
            throw ExternalStatusError("The selected build is not in the friends group.")
        }
        if request.mode == .submit {
            let alreadySubmitted = ["WAITING_FOR_BETA_REVIEW", "IN_BETA_REVIEW", "BETA_APPROVED", "READY_FOR_BETA_TESTING", "IN_BETA_TESTING"]
            if alreadySubmitted.contains(state) { return ["action": "submit", "unchanged": true, "externalState": state] }
            guard state == "READY_FOR_BETA_SUBMISSION" else { throw ExternalStatusError("Build cannot be submitted in state \(state).") }
            try await verifyMetadata()
            _ = try await write("POST", "betaAppReviewSubmissions", type: "betaAppReviewSubmissions",
                                relationships: ["build": relation("builds", id)])
            let submission = ExternalStatus.resource(try await get("builds/\(id)/betaAppReviewSubmission"))
            guard ["WAITING_FOR_REVIEW", "IN_REVIEW", "APPROVED"].contains(ExternalStatus.attributes(submission)["betaReviewState"] as? String ?? "") else {
                throw ExternalStatusError("Submission returned without a confirmed review state; inspect before retrying.")
            }
            return ["action": "submit", "build": request.build!, "submission": submission]
        }
        guard ["BETA_APPROVED", "READY_FOR_BETA_TESTING", "IN_BETA_TESTING"].contains(state) else {
            throw ExternalStatusError("Start testing or invite testers only after Beta App Review approval.")
        }
        if request.mode == .start { return try await start(buildID: id, groupID: groupID, detail: detail) }
        return try await invite(request.emails, groupID: groupID)
    }

    private func metadata(_ request: ExternalRequest) async throws -> [String: Any] {
        let root = "apps/" + Self.appID
        let existing = try await list(root + "/betaAppLocalizations")
        for locale in request.localizations.keys.sorted() {
            let values = request.localizations[locale]!
            let matches = existing.filter { ExternalStatus.attributes($0)["locale"] as? String == locale }
            guard matches.count <= 1 else { throw ExternalStatusError("Duplicate Beta localization.") }
            if let old = matches.first {
                let id = try ExternalStatus.id(old), attrs = ExternalStatus.attributes(old)
                let desired = attrs.merging(values) { _, new in new }
                if !equal(attrs, desired) {
                    _ = try await write("PATCH", "betaAppLocalizations/" + id, type: "betaAppLocalizations", id: id, attributes: values)
                }
            } else {
                _ = try await write("POST", "betaAppLocalizations", type: "betaAppLocalizations",
                                    attributes: values.merging(["locale": locale]) { _, new in new },
                                    relationships: ["app": relation("apps", Self.appID)])
            }
        }
        let current = ExternalStatus.resource(try await get(root + "/betaAppReviewDetail"))
        let id = try ExternalStatus.id(current), attrs = ExternalStatus.attributes(current)
        if !equal(attrs, attrs.merging(request.review) { _, new in new }) {
            _ = try await write("PATCH", "betaAppReviewDetails/" + id, type: "betaAppReviewDetails", id: id, attributes: request.review)
        }
        let updated = try await list(root + "/betaAppLocalizations")
        for (locale, values) in request.localizations {
            guard let found = updated.first(where: { ExternalStatus.attributes($0)["locale"] as? String == locale }),
                  values.allSatisfy({ ExternalStatus.attributes(found)[$0.key] as? String == $0.value }) else {
                throw ExternalStatusError("Beta metadata read-back did not match; do not submit yet.")
            }
        }
        let review = ExternalStatus.attributes(ExternalStatus.resource(try await get(root + "/betaAppReviewDetail")))
        guard equal(review, review.merging(request.review) { _, new in new }) else {
            throw ExternalStatusError("Review metadata read-back did not match.")
        }
        return ["action": "metadata", "verified": true, "locales": request.localizations.keys.sorted()]
    }

    private func verifyMetadata() async throws {
        let root = "apps/" + Self.appID
        let locales = try await list(root + "/betaAppLocalizations")
        var localizations = [String: [String: String]]()
        for item in locales {
            let attrs = ExternalStatus.attributes(item)
            guard let locale = attrs["locale"] as? String, ["en-US", "zh-Hans"].contains(locale) else { continue }
            localizations[locale] = ["description", "feedbackEmail", "privacyPolicyUrl", "marketingUrl"].reduce(into: [:]) {
                $0[$1] = attrs[$1] as? String ?? ""
            }
        }
        let review = ExternalStatus.attributes(ExternalStatus.resource(try await get(root + "/betaAppReviewDetail")))
        let selected = review.filter { ["contactFirstName", "contactLastName", "contactPhone", "contactEmail", "demoAccountRequired", "notes"].contains($0.key) }
        _ = try ExternalRequest(["mode": "metadata", "localizations": localizations, "review": selected])
    }

    private func friendsGroups() async throws -> [[String: Any]] {
        try await list("apps/\(Self.appID)/betaGroups").filter { ExternalStatus.attributes($0)["name"] as? String == Self.groupName }
    }
    private func checkGroup(_ group: [String: Any]) throws {
        let attrs = ExternalStatus.attributes(group)
        guard attrs["isInternalGroup"] as? Bool == false, attrs["publicLinkEnabled"] as? Bool != true,
              attrs["hasAccessToAllBuilds"] as? Bool != true else {
            throw ExternalStatusError("The friends group must be external, private and use explicitly assigned builds.")
        }
    }
    private func group(buildID: String, detail: [String: Any]) async throws -> [String: Any] {
        let groups = try await friendsGroups()
        guard groups.count <= 1 else { throw ExternalStatusError("Ambiguous friends group.") }
        let group: [String: Any]
        if let existing = groups.first { group = existing; try checkGroup(group) }
        else {
            group = ExternalStatus.resource(try await write("POST", "betaGroups", type: "betaGroups",
                attributes: ["name": Self.groupName, "isInternalGroup": false, "publicLinkEnabled": false, "hasAccessToAllBuilds": false],
                relationships: ["app": relation("apps", Self.appID)]))
            try checkGroup(group)
        }
        let id = try ExternalStatus.id(group)
        let assigned = try await list("betaGroups/\(id)/builds", ["fields[builds]": "version"])
        if !assigned.contains(where: { $0["id"] as? String == buildID }) {
            // Assigning a build to an existing audience may notify it. This initial setup does not do that.
            guard try await list("betaGroups/\(id)/betaTesters", ["fields[betaTesters]": "state"]).isEmpty else {
                throw ExternalStatusError("This group already has testers; adding a new build needs a separately authorized update workflow.")
            }
            let detailID = try ExternalStatus.id(detail)
            if ExternalStatus.attributes(detail)["autoNotifyEnabled"] as? Bool != false {
                _ = try await write("PATCH", "buildBetaDetails/" + detailID, type: "buildBetaDetails", id: detailID,
                                    attributes: ["autoNotifyEnabled": false])
            }
            let updated = ExternalStatus.resource(try await get("builds/\(buildID)/buildBetaDetail"))
            guard ExternalStatus.attributes(updated)["autoNotifyEnabled"] as? Bool == false else {
                throw ExternalStatusError("Automatic notification setting did not read back; build was not assigned.")
            }
            _ = try await send("POST", "betaGroups/\(id)/relationships/builds", [:], ["data": [["type": "builds", "id": buildID]]])
        }
        let after = try await list("betaGroups/\(id)/builds", ["fields[builds]": "version"])
        guard after.contains(where: { $0["id"] as? String == buildID }) else { throw ExternalStatusError("Build assignment did not read back.") }
        return ["action": "group", "groupId": id, "buildId": buildID, "verified": true]
    }

    private func invitedMembers(_ groupID: String) async throws -> Set<String> {
        let members = try await list("betaGroups/\(groupID)/betaTesters", ["fields[betaTesters]": "state"])
        let ids = try Set(members.map { try ExternalStatus.id($0) })
        guard ids.count == members.count, members.allSatisfy({
            ["INVITED", "ACCEPTED", "INSTALLED"].contains(ExternalStatus.attributes($0)["state"] as? String ?? "")
        }) else { throw ExternalStatusError("Existing testers must have identifiable, already invited memberships.") }
        return ids
    }

    private func updateGroup(buildID: String, detail: [String: Any]) async throws -> [String: Any] {
        let groups = try await friendsGroups()
        guard groups.count == 1 else { throw ExternalStatusError("Routine updates require the existing Friends group.") }
        let group = groups[0], id = try ExternalStatus.id(group)
        try checkGroup(group)
        let members = try await invitedMembers(id)
        guard (1...5).contains(members.count) else { throw ExternalStatusError("Routine updates require one to five existing friends.") }
        let assigned = try await list("betaGroups/\(id)/builds", ["fields[builds]": "version"])
        if !assigned.contains(where: { $0["id"] as? String == buildID }) {
            // Keep new builds on the established manual notification policy until approved testing starts.
            let detailID = try ExternalStatus.id(detail)
            if ExternalStatus.attributes(detail)["autoNotifyEnabled"] as? Bool != false {
                _ = try await write("PATCH", "buildBetaDetails/" + detailID, type: "buildBetaDetails", id: detailID,
                                    attributes: ["autoNotifyEnabled": false])
            }
            let ready = ExternalStatus.resource(try await get("builds/\(buildID)/buildBetaDetail"))
            guard ExternalStatus.attributes(ready)["autoNotifyEnabled"] as? Bool == false,
                  try await invitedMembers(id) == members else {
                throw ExternalStatusError("Notification policy or group membership changed before assignment.")
            }
            _ = try await send("POST", "betaGroups/\(id)/relationships/builds", [:],
                               ["data": [["type": "builds", "id": buildID]]])
        }
        let after = try await list("betaGroups/\(id)/builds", ["fields[builds]": "version"])
        guard after.contains(where: { $0["id"] as? String == buildID }),
              try await invitedMembers(id) == members else {
            throw ExternalStatusError("Build assignment or unchanged membership did not read back; inspect status.")
        }
        return ["action": "update", "groupId": id, "buildId": buildID, "externalTesterCount": members.count, "verified": true]
    }

    private func start(buildID: String, groupID: String, detail: [String: Any]) async throws -> [String: Any] {
        let groups = try await list("betaGroups", ["filter[builds]": buildID,
            "fields[betaGroups]": "name,isInternalGroup,publicLinkEnabled,hasAccessToAllBuilds"])
        let internalGroups = groups.filter { ExternalStatus.attributes($0)["isInternalGroup"] as? Bool == true }
        guard groups.count == 2, groups.contains(where: { $0["id"] as? String == groupID }), internalGroups.count == 1,
              ExternalStatus.attributes(internalGroups[0])["name"] as? String == "AskHuman Internal",
              ExternalStatus.attributes(detail)["autoNotifyEnabled"] as? Bool == false else {
            throw ExternalStatusError("Start testing requires only the existing Internal and Friends groups with auto-notify off.")
        }
        let internalID = try ExternalStatus.id(internalGroups[0])
        let internalMembers = try await invitedMembers(internalID), friends = try await invitedMembers(groupID)
        guard (1...5).contains(friends.count) else { throw ExternalStatusError("Routine delivery requires one to five existing friends.") }
        let approvedIDs = internalMembers.union(friends)
        let individuals = try await list("builds/\(buildID)/individualTesters", ["fields[betaTesters]": "state"])
        guard try individuals.allSatisfy({ approvedIDs.contains(try ExternalStatus.id($0)) }) else {
            throw ExternalStatusError("The build has an individual tester outside the existing groups.")
        }
        let state = ExternalStatus.attributes(detail)["externalBuildState"] as? String ?? "UNKNOWN"
        if state == "BETA_APPROVED" {
            _ = try await write("POST", "buildBetaNotifications", type: "buildBetaNotifications",
                                relationships: ["build": relation("builds", buildID)])
        }
        let after = ExternalStatus.resource(try await get("builds/\(buildID)/buildBetaDetail"))
        let afterState = ExternalStatus.attributes(after)["externalBuildState"] as? String ?? "UNKNOWN"
        guard ["READY_FOR_BETA_TESTING", "IN_BETA_TESTING"].contains(afterState),
              try await invitedMembers(internalID) == internalMembers,
              try await invitedMembers(groupID) == friends else {
            throw ExternalStatusError("Testing state or unchanged membership did not read back; inspect status before retrying.")
        }
        return ["action": "start", "unchanged": state != "BETA_APPROVED", "externalState": afterState,
                "externalTesterCount": friends.count, "internalTesterCount": internalMembers.count]
    }

    private func invite(_ emails: [String], groupID: String) async throws -> [String: Any] {
        var changed = 0
        var states = [String: Int]()
        let roster = try await list("betaGroups/\(groupID)/betaTesters", ["fields[betaTesters]": "email,state"])
        let rosterEmails = roster.compactMap { ExternalStatus.attributes($0)["email"] as? String }.map { $0.lowercased() }
        guard rosterEmails.count == roster.count, Set(rosterEmails + emails.map { $0.lowercased() }).count <= 5 else {
            throw ExternalStatusError("The initial friends audience is limited to five identifiable email testers.")
        }
        func lookup(_ email: String, groupOnly: Bool) async throws -> [[String: Any]] {
            var query = ["filter[email]": email, "fields[betaTesters]": "email,state"]
            if groupOnly { query["filter[betaGroups]"] = groupID; query["filter[apps]"] = Self.appID }
            let found = try await list("betaTesters", query)
            guard found.count <= 1, found.allSatisfy({
                (ExternalStatus.attributes($0)["email"] as? String)?.lowercased() == email.lowercased()
            }) else { throw ExternalStatusError("Tester lookup was ambiguous or returned a different email.") }
            return found
        }
        for email in emails {
            if try await lookup(email, groupOnly: true).isEmpty {
                // A tester may already belong to another app. Only add that exact authorized email to this group.
                if let tester = try await lookup(email, groupOnly: false).first {
                    let id = try ExternalStatus.id(tester)
                    _ = try await send("POST", "betaGroups/\(groupID)/relationships/betaTesters", [:], ["data": [["type": "betaTesters", "id": id]]])
                } else {
                    _ = try await write("POST", "betaTesters", type: "betaTesters", attributes: ["email": email],
                                        relationships: ["betaGroups": ["data": [["type": "betaGroups", "id": groupID]]]])
                }
                changed += 1
            }
            guard var member = try await lookup(email, groupOnly: true).first else {
                throw ExternalStatusError("Tester assignment did not read back; inspect before retrying.")
            }
            if ExternalStatus.attributes(member)["state"] as? String == "NOT_INVITED" {
                // Auto-notify is off. Always specify both tester and app; never send an app-wide invitation.
                let id = try ExternalStatus.id(member)
                _ = try await write("POST", "betaTesterInvitations", type: "betaTesterInvitations",
                    relationships: ["app": relation("apps", Self.appID), "betaTester": relation("betaTesters", id)])
                guard let updated = try await lookup(email, groupOnly: true).first else {
                    throw ExternalStatusError("Invitation state is unknown; inspect before retrying.")
                }
                member = updated
            }
            let state = ExternalStatus.attributes(member)["state"] as? String ?? "UNKNOWN"
            guard ["INVITED", "ACCEPTED", "INSTALLED"].contains(state) else {
                throw ExternalStatusError("Tester state is \(state); inspect it before another invitation attempt.")
            }
            states[state, default: 0] += 1
        }
        return ["action": "invite", "assigned": emails.count, "changed": changed,
                "testerStates": states,
                "note": "Apple invitation state is verified. Actual installation, pairing and push testing remain separate checks."]
    }
}
