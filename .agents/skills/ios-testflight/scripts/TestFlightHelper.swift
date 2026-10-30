import CryptoKit
import Darwin
import Foundation
import Security

private let service = "com.naituw.askhuman.testflight"
private let apiAccount = "app-store-connect.api-private-key"
private let signingAccount = "signing-keychain.password"
private let bundleID = "com.naituw.humaninloop.ios"
private let appID = "6828712684"
private var stopping: sig_atomic_t = 0

private struct Failure: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}

private func checked(_ status: OSStatus, _ operation: String) throws {
    guard status == errSecSuccess else {
        throw Failure("\(operation) failed (Keychain OSStatus \(status)). Unlock or authorize the signing helper through the user; no file fallback is used.")
    }
}

private func loginKeychain() throws -> SecKeychain {
    var keychain: SecKeychain?
    try checked(SecKeychainOpen(NSHomeDirectory() + "/Library/Keychains/login.keychain-db", &keychain), "Open login Keychain")
    guard let keychain else { throw Failure("Login Keychain is unavailable.") }
    return keychain
}

private func secret(_ account: String) throws -> Data {
    var result: CFTypeRef?
    let query: [CFString: Any] = [
        kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account,
        kSecMatchSearchList: [try loginKeychain()], kSecReturnData: true,
        kSecUseAuthenticationUI: kSecUseAuthenticationUIFail,
    ]
    try checked(SecItemCopyMatching(query as CFDictionary, &result), "Read TestFlight credential")
    guard let data = result as? Data else { throw Failure("Invalid Keychain credential data.") }
    return data
}

private func store(_ data: Data, account: String) throws {
    var trusted: SecTrustedApplication?
    let executable = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL.path
    try checked(SecTrustedApplicationCreateFromPath(executable, &trusted), "Identify signed helper")
    var access: SecAccess?
    try checked(SecAccessCreate("AskHuman TestFlight" as CFString, [trusted!] as CFArray, &access), "Create helper access rule")
    let item: [CFString: Any] = [
        kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account,
        kSecUseKeychain: try loginKeychain(), kSecAttrAccess: access!, kSecValueData: data,
    ]
    let result = SecItemAdd(item as CFDictionary, nil)
    if result == errSecDuplicateItem {
        guard try secret(account) == data else { throw Failure("A different credential already exists; replacement requires a separate rotation request.") }
    } else {
        try checked(result, "Store TestFlight credential")
    }
    guard try secret(account) == data else { throw Failure("Keychain credential verification failed.") }
}

private func json(_ url: URL) throws -> [String: Any] {
    guard let value = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any] else {
        throw Failure("Invalid non-secret configuration.")
    }
    return value
}

private func emit(_ value: Any) throws {
    let data = try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
    FileHandle.standardOutput.write(data + Data("\n".utf8))
}

private func base64url(_ data: Data) -> String {
    data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
}

private func token(_ config: [String: Any]) throws -> String {
    guard let keyID = config["keyId"] as? String, let issuer = config["issuerId"] as? String,
          let pem = String(data: try secret(apiAccount), encoding: .utf8) else { throw Failure("Missing API key metadata.") }
    let now = Int(Date().timeIntervalSince1970)
    let header = try JSONSerialization.data(withJSONObject: ["alg": "ES256", "kid": keyID, "typ": "JWT"])
    let claims = try JSONSerialization.data(withJSONObject: ["iss": issuer, "iat": now, "exp": now + 300, "aud": "appstoreconnect-v1"])
    let input = base64url(header) + "." + base64url(claims)
    let key = try P256.Signing.PrivateKey(pemRepresentation: pem)
    return input + "." + base64url(try key.signature(for: Data(input.utf8)).rawRepresentation)
}

private func status(_ config: [String: Any]) async throws {
    let authorization = try token(config)
    let session = URLSession(configuration: .ephemeral)
    defer { session.invalidateAndCancel() }
    func get(_ endpoint: String, _ params: [String: String] = [:]) async throws -> [String: Any] {
        var url = URLComponents(string: "https://api.appstoreconnect.apple.com/v1/" + endpoint)!
        url.queryItems = params.map { URLQueryItem(name: $0.key, value: $0.value) }
        var request = URLRequest(url: url.url!, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 30)
        request.setValue("Bearer " + authorization, forHTTPHeaderField: "Authorization")
        let (data, response) = try await session.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw Failure("Apple status query failed (HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0)); inspect account access without printing credentials.")
        }
        guard let body = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw Failure("Invalid Apple response.") }
        return body
    }
    let app = try await get("apps/" + appID, ["fields[apps]": "name,bundleId"])
    let builds = try await get("builds", ["filter[app]": appID, "sort": "-uploadedDate", "limit": "10", "include": "preReleaseVersion",
        "fields[builds]": "version,processingState,uploadedDate,expired,preReleaseVersion", "fields[preReleaseVersions]": "version,platform"])
    let groups = try await get("apps/\(appID)/betaGroups", ["fields[betaGroups]": "name,isInternalGroup,hasAccessToAllBuilds"])
    var internalGroups = [[String: Any]]()
    for group in groups["data"] as? [[String: Any]] ?? [] {
        guard let attributes = group["attributes"] as? [String: Any], attributes["isInternalGroup"] as? Bool == true,
              let id = group["id"] as? String else { continue }
        let assigned = try await get("betaGroups/\(id)/builds", ["fields[builds]": "version,processingState", "limit": "200"])
        let testers = try await get("betaGroups/\(id)/betaTesters", ["fields[betaTesters]": "state", "limit": "200"])
        var states = [String: Int]()
        for tester in testers["data"] as? [[String: Any]] ?? [] {
            let state = (tester["attributes"] as? [String: Any])?["state"] as? String ?? "UNKNOWN"
            states[state, default: 0] += 1
        }
        internalGroups.append(["name": attributes["name"] ?? "", "allBuilds": attributes["hasAccessToAllBuilds"] ?? false,
            "buildIds": (assigned["data"] as? [[String: Any]] ?? []).compactMap { $0["id"] as? String }, "testerStates": states])
    }
    try emit(["app": app["data"] ?? [:], "builds": builds["data"] ?? [], "versions": builds["included"] ?? [], "internalGroups": internalGroups])
}

private func searchList() throws -> [SecKeychain] {
    var array: CFArray?
    try checked(SecKeychainCopySearchList(&array), "Read keychain search list")
    return array as? [SecKeychain] ?? []
}

private func keychainPath(_ keychain: SecKeychain) throws -> String {
    var length: UInt32 = 4096
    var bytes = [CChar](repeating: 0, count: Int(length))
    try checked(SecKeychainGetPath(keychain, &length, &bytes), "Read signing keychain path")
    return String(cString: bytes)
}

private func runXcode(_ args: [String]) throws -> Int32 {
    var attributes: posix_spawnattr_t?
    posix_spawnattr_init(&attributes)
    defer { posix_spawnattr_destroy(&attributes) }
    posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_SETPGROUP))
    posix_spawnattr_setpgroup(&attributes, 0)
    var argv = (["/usr/bin/xcodebuild"] + args).map { strdup($0) } + [nil]
    defer { for pointer in argv { free(pointer) } }
    var pid: pid_t = 0
    guard posix_spawn(&pid, "/usr/bin/xcodebuild", nil, &attributes, &argv, environ) == 0 else { throw Failure("Cannot start Xcode exporter.") }
    let deadline = Date().addingTimeInterval(1800)
    var result: Int32 = 0
    while true {
        let waited = waitpid(pid, &result, WNOHANG)
        if waited == pid { break }
        if waited < 0 {
            if errno == EINTR { continue }
            throw Failure("Cannot collect the Xcode export result.")
        }
        if stopping != 0 || Date() > deadline {
            kill(-pid, SIGTERM)
            let grace = Date().addingTimeInterval(15)
            while waitpid(pid, &result, WNOHANG) == 0 {
                if Date() > grace { kill(-pid, SIGKILL); waitpid(pid, &result, 0); break }
                usleep(100_000)
            }
            throw Failure("Export interrupted or timed out; check Apple status before retrying.")
        }
        usleep(200_000)
    }
    return result & 0x7f == 0 ? (result >> 8) & 0xff : 128 + (result & 0x7f)
}

private func exportArchive(_ archive: URL, root: URL, config: [String: Any], upload: Bool) throws -> Int32 {
    let lockFD = open(root.appendingPathComponent("testflight-upload.lock").path, O_CREAT | O_RDWR, 0o600)
    guard lockFD >= 0 else { throw Failure("Cannot open the upload lock.") }
    defer { close(lockFD) }
    guard flock(lockFD, LOCK_EX | LOCK_NB) == 0 else { throw Failure("Another TestFlight export is using this signing configuration.") }
    let app = archive.appendingPathComponent("Products/Applications/AskHuman.app")
    guard let info = NSDictionary(contentsOf: app.appendingPathComponent("Info.plist")) as? [String: Any],
          info["CFBundleIdentifier"] as? String == bundleID, info["UIDeviceFamily"] as? [Int] == [1],
          info["AHPushEnvironment"] as? String == "prod", info["ITSAppUsesNonExemptEncryption"] as? Bool == false,
          FileManager.default.fileExists(atPath: app.appendingPathComponent("PrivacyInfo.xcprivacy").path) else {
        throw Failure("Archive preflight failed; rebuild the intended iPhone release with its privacy manifest.")
    }
    guard let version = info["CFBundleShortVersionString"] as? String, !version.isEmpty,
          let build = info["CFBundleVersion"] as? String, !build.isEmpty,
          let notification = NSDictionary(contentsOf: app.appendingPathComponent("PlugIns/AskHumanNotification.appex/Info.plist")) as? [String: Any],
          notification["CFBundleIdentifier"] as? String == bundleID + ".notification",
          notification["CFBundleShortVersionString"] as? String == version, notification["CFBundleVersion"] as? String == build,
          notification["UIDeviceFamily"] as? [Int] == [1] else { throw Failure("App and extension version/build/device family must match.") }
    let optionsURL = root.appendingPathComponent("testflight-export-options.plist")
    guard var options = NSDictionary(contentsOf: optionsURL) as? [String: Any], options["signingStyle"] as? String == "manual",
          options["teamID"] as? String == "DMJXDB9H6Q", options["method"] as? String == "app-store-connect",
          options["testFlightInternalTestingOnly"] as? Bool == true,
          let profiles = options["provisioningProfiles"] as? [String: String],
          profiles[bundleID] != nil, profiles[bundleID + ".notification"] != nil else { throw Failure("Only the established internal TestFlight signing configuration is supported.") }
    options["destination"] = upload ? "upload" : "export"
    var temporaryRoot = root.appendingPathComponent("temporary-keys", isDirectory: true)
    try FileManager.default.createDirectory(at: temporaryRoot, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    var backupPolicy = URLResourceValues()
    backupPolicy.isExcludedFromBackup = true
    try temporaryRoot.setResourceValues(backupPolicy)
    // The exclusive export lock makes leftovers from a killed exporter safe to remove here.
    for old in try FileManager.default.contentsOfDirectory(at: temporaryRoot, includingPropertiesForKeys: nil) where old.lastPathComponent.hasPrefix("upload-") {
        try FileManager.default.removeItem(at: old)
    }
    let temporary = temporaryRoot.appendingPathComponent("upload-" + UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    var temporaryRemoved = false
    defer {
        if !temporaryRemoved {
            do { try FileManager.default.removeItem(at: temporary) }
            catch { FileHandle.standardError.write(Data("Temporary credential cleanup failed; ask the user to inspect the private temporary directory.\n".utf8)) }
        }
    }
    let keyFile = temporary.appendingPathComponent("api-key.p8")
    guard FileManager.default.createFile(atPath: keyFile.path, contents: try secret(apiAccount), attributes: [.posixPermissions: 0o600]) else { throw Failure("Cannot prepare Xcode's temporary credential file.") }
    let temporaryOptions = temporary.appendingPathComponent("ExportOptions.plist")
    try PropertyListSerialization.data(fromPropertyList: options, format: .xml, options: 0).write(to: temporaryOptions)
    let signingPath = root.appendingPathComponent("testflight-signing.keychain-db").path
    var keychain: SecKeychain?
    try checked(SecKeychainOpen(signingPath, &keychain), "Open signing keychain")
    guard let keychain else { throw Failure("Signing keychain is unavailable.") }
    let password = try secret(signingAccount)
    try password.withUnsafeBytes { bytes in
        try checked(SecKeychainUnlock(keychain, UInt32(password.count), bytes.baseAddress, true), "Unlock signing keychain")
    }
    let before = try searchList()
    let added = try !before.contains { try keychainPath($0) == signingPath }
    if added { try checked(SecKeychainSetSearchList((before + [keychain]) as CFArray), "Add signing keychain") }
    var restoreNeeded = added
    func restore() throws {
        let current = try searchList().filter { try keychainPath($0) != signingPath }
        try checked(SecKeychainSetSearchList(current as CFArray), "Restore keychain search list")
    }
    defer {
        if restoreNeeded {
            do { try restore() }
            catch { FileHandle.standardError.write(Data("Signing keychain cleanup failed; ask the user to inspect its search list.\n".utf8)) }
        }
    }
    guard let keyID = config["keyId"] as? String, let issuer = config["issuerId"] as? String else { throw Failure("Missing API key metadata.") }
    let outcome: Result<Int32, Error>
    do {
        outcome = .success(try runXcode(["-exportArchive", "-archivePath", archive.path, "-exportOptionsPlist", temporaryOptions.path,
            "-exportPath", archive.deletingLastPathComponent().appendingPathComponent(upload ? "TestFlightExport" : "TestFlightSigningCheck").path,
            "-authenticationKeyPath", keyFile.path, "-authenticationKeyID", keyID, "-authenticationKeyIssuerID", issuer]))
    } catch { outcome = .failure(error) }
    if restoreNeeded { try restore(); restoreNeeded = false }
    try FileManager.default.removeItem(at: temporary)
    temporaryRemoved = true
    return try outcome.get()
}

@main
private enum TestFlightHelper {
    static func main() async {
        signal(SIGTERM) { _ in stopping = 1 }
        signal(SIGINT) { _ in stopping = 1 }
        SecKeychainSetUserInteractionAllowed(false)
        let args = CommandLine.arguments
        guard args.count >= 6, args[1] == "--config", args[3] == "--result" else {
            FileHandle.standardError.write(Data("Use the project's TestFlight launcher.\n".utf8)); exit(2)
        }
        let configURL = URL(fileURLWithPath: args[2])
        let root = configURL.deletingLastPathComponent()
        let resultURL = URL(fileURLWithPath: args[4])
        var code: Int32 = 0
        do {
            let expectedRoot = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".config/askhuman-release").standardizedFileURL
            let attributes = try FileManager.default.attributesOfItem(atPath: root.path)
            guard root.resolvingSymlinksInPath().path == expectedRoot.path,
                  (attributes[.ownerAccountID] as? NSNumber)?.uint32Value == getuid(),
                  ((attributes[.posixPermissions] as? NSNumber)?.intValue ?? 0o777) & 0o077 == 0 else {
                throw Failure("Use the owner-only ~/.config/askhuman-release directory; temporary credentials must stay outside project files.")
            }
            let config = try json(configURL)
            switch args[5] {
            case "migrate":
                guard let source = config["privateKeyPath"] as? String else { throw Failure("No legacy key path configured.") }
                let api = try Data(contentsOf: URL(fileURLWithPath: source))
                guard let pem = String(data: api, encoding: .utf8) else { throw Failure("Invalid API key encoding.") }
                _ = try P256.Signing.PrivateKey(pemRepresentation: pem)
                try store(api, account: apiAccount)
                try store(Data(contentsOf: root.appendingPathComponent("signing-keychain-password")), account: signingAccount)
                try emit(["migrated": true, "service": service, "secretContentsReturned": false])
            case "status": try await status(config)
            case "upload", "export":
                guard args.count == 7 else { throw Failure("An archive path is required.") }
                code = try exportArchive(URL(fileURLWithPath: args[6]), root: root, config: config, upload: args[5] == "upload")
            default: throw Failure("Unsupported operation; this helper does not export credential contents.")
            }
        } catch {
            FileHandle.standardError.write(Data("TestFlight: \(error)\n".utf8)); code = 1
        }
        do { try JSONSerialization.data(withJSONObject: ["exitCode": code]).write(to: resultURL, options: .atomic) }
        catch { exit(1) }
        exit(code)
    }
}
