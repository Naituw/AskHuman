import Darwin
import Foundation
import XCTest

@testable import TodoSyncHelperKit

/// All account paths are temporary; tests never resolve the current account's home.
final class FixtureTestTree {
    let root: URL
    let account: URL
    let realAccount: URL
    let executable: URL

    init(temporaryDirectory: URL = FileManager.default.temporaryDirectory) throws {
        let temporaryRoot = temporaryDirectory
            .appendingPathComponent("askhuman-helper-fixture-\(UUID().uuidString)")
        try FileManager.default.createDirectory(
            at: temporaryRoot, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        guard let resolved = Darwin.realpath(temporaryRoot.path, nil) else {
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
        }
        defer { free(resolved) }
        root = URL(fileURLWithPath: String(cString: resolved))
        account = root.appendingPathComponent("fixture")
        realAccount = root.appendingPathComponent("real-account")
        executable = root.appendingPathComponent("AskHuman")
        for directory in [account, realAccount] {
            try FileManager.default.createDirectory(
                at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        }
        try writeExecutable("exit 0\n")
    }

    func fixture() throws -> SystemFixture {
        try SystemFixture(accountHome: account.path, realAccountHome: realAccount)
    }

    func config(_ fixture: SystemFixture) -> HelperConfig {
        HelperConfig(askhuman: executable.path, stateDir: fixture.paths.stateDir.path, todosPath: fixture.todosURL.path)
    }

    func writeExecutable(_ body: String) throws {
        try Data(("#!/bin/sh\n" + body).utf8).write(to: executable, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
    }

    func remove() throws { try FileManager.default.removeItem(at: root) }
}

final class SystemFixtureTests: XCTestCase {
    private var tree: FixtureTestTree!

    override func setUpWithError() throws { tree = try FixtureTestTree() }
    override func tearDownWithError() throws { try tree?.remove() }

    private func parse(_ arguments: [String], environment: [String: String] = [:]) throws -> HelperInvocation {
        try HelperInvocation.parse(arguments, environment: environment, realAccountHome: { self.tree.realAccount })
    }

    private var guardedArguments: [String] {
        ["--version", "--internal-system-fixture-v2", tree.account.path, "run"]
    }

    func testExactVersionNeverInspectsContextOrAccount() throws {
        let invocation = try HelperInvocation.parse(
            ["--version"], environment: [SystemFixture.targetEnvironment: "not JSON"],
            realAccountHome: { XCTFail("version must not resolve an account"); return URL(fileURLWithPath: "/") })
        XCTAssertEqual(invocation, .version)
    }

    func testGuardedLaunchPinsPrivatePathsWithoutCreatingThem() throws {
        guard case .run(let parsed) = try parse(guardedArguments) else { return XCTFail("not a run") }
        let fixture = try XCTUnwrap(parsed)
        XCTAssertEqual(fixture.accountHome.path, tree.account.path)
        XCTAssertEqual(fixture.paths.stateDir.path, tree.account.path + "/.askhuman/state/todo-sync")
        XCTAssertEqual(fixture.logURL.path, fixture.paths.stateDir.path + "/helper.log")
        XCTAssertFalse(FileManager.default.fileExists(atPath: fixture.dataDirectory.path))
        try fixture.prepareStateDirectory()
        XCTAssertTrue(FileManager.default.fileExists(atPath: fixture.paths.stateDir.path))
    }

    func testAccountLookupFailureRejectsGuardedRunBeforeCreatingState() {
        XCTAssertThrowsError(try HelperInvocation.parse(
            guardedArguments, environment: [:],
            realAccountHome: { throw FixtureError("account lookup unavailable") }))
        XCTAssertFalse(FileManager.default.fileExists(atPath: tree.account.path + "/.askhuman"))
    }

    func testMalformedGuardNeverFallsThroughToAppOrRegistration() {
        for arguments in [
            ["--internal-system-fixture-v2", tree.account.path, "run"],
            ["--version", "--internal-system-fixture-v2", tree.account.path],
            ["--version", "--internal-system-fixture-v2", tree.account.path, "--register"],
            ["--version", "--internal-system-fixture-v2", tree.account.path, "--unregister"],
            guardedArguments + ["extra"], ["--register", "extra"], ["unknown"],
        ] {
            XCTAssertThrowsError(try parse(arguments), "\(arguments)")
        }
    }

    func testOrdinaryEntryPointsRemainAvailable() throws {
        XCTAssertEqual(try parse([]), .run(nil))
        XCTAssertEqual(try parse(["--register"]), .register)
        XCTAssertEqual(try parse(["--unregister"]), .unregister)
        XCTAssertEqual(try parse(["--login-item-status"]), .loginItemStatus)
    }

    func testInheritedFixtureCannotStartUnguardedOrManageSharedLoginItem() throws {
        let fixture = try tree.fixture()
        let environment = try fixture.tickEnvironment([:], config: tree.config(fixture))
        XCTAssertThrowsError(try parse([], environment: environment))
        for command in ["--register", "--unregister", "--login-item-status"] {
            let invocation = try HelperInvocation.parse(
                [command], environment: environment,
                realAccountHome: { XCTFail("unmanaged commands must not resolve an account"); return self.tree.realAccount })
            XCTAssertEqual(invocation, .unmanaged(command: command))
        }
        XCTAssertEqual(try parse(guardedArguments, environment: environment), .run(fixture))
    }

    func testGuardRejectsConflictingInheritedEnvironment() throws {
        let fixture = try tree.fixture()
        for environment in [
            ["ASKHUMAN_HOME": tree.root.path],
            ["ASKHUMAN_TODO_SYNC_DIR": tree.root.path],
            [SystemFixture.targetEnvironment: #"{"version":1}"#],
        ] {
            XCTAssertThrowsError(try parse(guardedArguments, environment: environment))
        }
        var environment = try fixture.tickEnvironment([:], config: tree.config(fixture))
        let other = tree.root.appendingPathComponent("other")
        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        XCTAssertThrowsError(try parse(["--version", "--internal-system-fixture-v2", other.path, "run"], environment: environment))
        environment["ASKHUMAN_TODO_SYNC_DIR"] = fixture.paths.stateDir.path
        XCTAssertEqual(try parse(guardedArguments, environment: environment), .run(fixture))
    }

    func testTargetVersionsAndFixtureShapeAreStrict() throws {
        for raw in [
            "invalid", #"{"version":3}"#, #"{"version":2}"#,
            #"{"version":1,"fixture":null}"#,
            #"{"version":1,"fixture":{"version":2,"accountHome":"/tmp"}}"#,
        ] {
            XCTAssertThrowsError(try SystemFixture.inheritedIdentity([SystemFixture.targetEnvironment: raw]))
        }
        XCTAssertNil(try SystemFixture.inheritedIdentity([SystemFixture.targetEnvironment: #"{"version":1}"#]))
        let fixture = try tree.fixture()
        let environment = try fixture.tickEnvironment([:], config: tree.config(fixture))
        let raw = try XCTUnwrap(environment[SystemFixture.targetEnvironment])
        let target = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
        let invalidFields: [(String, Any)] = [
            ("kind", "dev"), ("home", tree.root.path), ("noKeychain", false),
            ("worktree", tree.root.path), ("binary", "relative"),
            ("fixture", ["version": 1, "accountHome": tree.account.path]),
        ]
        for (key, invalid) in invalidFields {
            var changed = target
            changed[key] = invalid
            let data = try JSONSerialization.data(withJSONObject: changed)
            XCTAssertThrowsError(try SystemFixture.inheritedIdentity([
                SystemFixture.targetEnvironment: String(decoding: data, as: UTF8.self),
            ]), key)
        }
    }

    func testAccountRequiresAbsoluteExistingOwnedPrivateDirectory() throws {
        for path in ["relative", tree.root.appendingPathComponent("missing").path, tree.realAccount.path, "/"] {
            XCTAssertThrowsError(try SystemFixture(accountHome: path, realAccountHome: tree.realAccount))
        }
        XCTAssertThrowsError(try SystemFixture(accountHome: tree.account.path, realAccountHome: tree.realAccount, userID: getuid() + 1))
        try FileManager.default.setAttributes([.posixPermissions: 0o777], ofItemAtPath: tree.account.path)
        XCTAssertThrowsError(try tree.fixture())
    }

    func testCanonicalAccountAliasRemainsPrivate() throws {
        let alias = tree.root.appendingPathComponent("alias")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: tree.account)
        let fixture = try SystemFixture(accountHome: alias.path, realAccountHome: tree.realAccount)
        XCTAssertEqual(fixture.accountHome.path, tree.account.path)
    }

    func testTmpAliasUsesPOSIXIdentityBeforeAndAfterTodosExist() throws {
        let temporary = try FixtureTestTree(temporaryDirectory: URL(fileURLWithPath: "/tmp"))
        defer { try? temporary.remove() }
        let aliasRoot = URL(fileURLWithPath: "/tmp").appendingPathComponent(temporary.root.lastPathComponent)
        let aliasAccount = aliasRoot.appendingPathComponent("fixture")
        let fixture = try SystemFixture(accountHome: aliasAccount.path, realAccountHome: temporary.realAccount)
        XCTAssertTrue(fixture.accountHome.path.hasPrefix("/private/tmp/"))
        XCTAssertEqual(fixture.identity, FixtureIdentity(version: 2, accountHome: temporary.account.path))
        XCTAssertEqual(fixture, try temporary.fixture())

        let config = try XCTUnwrap(HelperConfig.parse(JSONSerialization.data(withJSONObject: [
            "protocol": 1, "askhuman": temporary.executable.path,
            "stateDir": temporary.account.path + "/.askhuman/state/todo-sync",
            "todosPath": temporary.account.path + "/.askhuman/state/todos.json",
        ])))
        XCTAssertFalse(FileManager.default.fileExists(atPath: fixture.paths.stateDir.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: fixture.todosURL.path))
        XCTAssertNoThrow(try fixture.validateConfiguration(config))
        try fixture.prepareStateDirectory()
        XCTAssertFalse(FileManager.default.fileExists(atPath: fixture.todosURL.path))
        XCTAssertNoThrow(try fixture.validateConfiguration(config))

        var aliasConfig = config
        aliasConfig.askhuman = aliasRoot.appendingPathComponent("AskHuman").path
        aliasConfig.stateDir = aliasAccount.path + "/.askhuman/state/todo-sync"
        aliasConfig.todosPath = aliasAccount.path + "/.askhuman/state/todos.json"
        XCTAssertNoThrow(try fixture.validateConfiguration(aliasConfig))
        let environment = try fixture.tickEnvironment([:], config: aliasConfig)
        XCTAssertEqual(try SystemFixture.inheritedIdentity(environment), fixture.identity)
        XCTAssertEqual(environment["HOME"], temporary.account.path)
        XCTAssertEqual(environment["ASKHUMAN_TODO_SYNC_DIR"], config.stateDir)

        try Data("{}".utf8).write(to: fixture.todosURL)
        XCTAssertNoThrow(try fixture.validateConfiguration(config))
        XCTAssertNoThrow(try fixture.validateConfiguration(aliasConfig))
        XCTAssertEqual(fixture.identity.accountHome, temporary.account.path)
    }

    func testMissingConfigPathsResolveExistingParentAliases() throws {
        let fixture = try tree.fixture()
        let alias = tree.root.appendingPathComponent("alias")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: tree.account)
        var config = tree.config(fixture)
        config.stateDir = alias.path + "/.askhuman/state/todo-sync"
        config.todosPath = alias.path + "/.askhuman/state/todos.json"
        XCTAssertNoThrow(try fixture.validateConfiguration(config))

        let dangling = tree.root.appendingPathComponent("dangling")
        try FileManager.default.createSymbolicLink(at: dangling, withDestinationURL: tree.root.appendingPathComponent("missing"))
        config.stateDir = dangling.path + "/.askhuman/state/todo-sync"
        XCTAssertThrowsError(try fixture.validateConfiguration(config))
        config.stateDir = tree.executable.path + "/.askhuman/state/todo-sync"
        XCTAssertThrowsError(try fixture.validateConfiguration(config))
    }

    func testEffectiveDataRootCannotAliasProduction() throws {
        let production = tree.realAccount.appendingPathComponent(".askhuman")
        try FileManager.default.createDirectory(at: production, withIntermediateDirectories: true)
        let link = tree.account.appendingPathComponent(".askhuman")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: production)
        XCTAssertThrowsError(try tree.fixture())
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: production.path), [])
    }

    func testPrivateDirectoriesAndStateFilesCannotBeSymlinks() throws {
        let fixture = try tree.fixture()
        try fixture.prepareStateDirectory()
        for url in [fixture.todosURL, fixture.paths.configURL, fixture.paths.lockURL,
                    fixture.logURL, fixture.logURL.appendingPathExtension("1"), fixture.paths.socketURL] {
            try FileManager.default.createSymbolicLink(at: url, withDestinationURL: tree.root.appendingPathComponent("missing"))
            XCTAssertThrowsError(try fixture.validateLayout(), url.path)
            try FileManager.default.removeItem(at: url)
        }
        try FileManager.default.removeItem(at: fixture.paths.stateDir)
        try FileManager.default.createSymbolicLink(at: fixture.paths.stateDir, withDestinationURL: tree.root)
        XCTAssertThrowsError(try fixture.validateLayout())
    }

    func testSharedTransportLinkDoesNotGrantSecondHelperOwnership() throws {
        let first = try tree.fixture()
        try first.prepareStateDirectory()
        let secondAccount = tree.root.appendingPathComponent("second")
        try FileManager.default.createDirectory(at: secondAccount, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let second = try SystemFixture(accountHome: secondAccount.path, realAccountHome: tree.realAccount)
        try second.prepareStateDirectory()
        try FileManager.default.createSymbolicLink(at: second.paths.socketURL, withDestinationURL: first.paths.socketURL)
        // A manual core client may follow this link; a second helper must never replace it.
        XCTAssertThrowsError(try second.validateLayout())
        XCTAssertNoThrow(try first.validateLayout())
        XCTAssertEqual(try FileManager.default.destinationOfSymbolicLink(atPath: second.paths.socketURL.path), first.paths.socketURL.path)
    }

    func testConfigMustKeepStateAndTodosAtFixedAccount() throws {
        let fixture = try tree.fixture()
        let config = tree.config(fixture)
        XCTAssertNoThrow(try fixture.validateConfiguration(config))
        var wrongState = config
        wrongState.stateDir = tree.root.path
        var wrongTodos = config
        wrongTodos.todosPath = tree.root.appendingPathComponent("todos.json").path
        var wrongProtocol = config
        wrongProtocol.protocolVersion = Wire.protocolVersion + 1
        var relative = config
        relative.askhuman = "AskHuman"
        for invalid in [wrongState, wrongTodos, wrongProtocol, relative] {
            XCTAssertThrowsError(try fixture.validateConfiguration(invalid))
        }
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: tree.executable.path)
        XCTAssertThrowsError(try fixture.validateConfiguration(config))
    }

    func testTickCannotUseDevExecutableEvenThroughAlias() throws {
        let fixture = try tree.fixture()
        let dev = tree.root.appendingPathComponent(".askhuman-dev/bin/AskHuman")
        try FileManager.default.createDirectory(at: dev.deletingLastPathComponent(), withIntermediateDirectories: true)
        try FileManager.default.moveItem(at: tree.executable, to: dev)
        try FileManager.default.createSymbolicLink(at: tree.executable, withDestinationURL: dev)
        XCTAssertThrowsError(try fixture.validateConfiguration(tree.config(fixture)))
    }

    func testProbeClearsContextAndTickInjectsExactV2SystemEnvelope() throws {
        let fixture = try tree.fixture()
        let oldEnvironment = [
            SystemFixture.targetEnvironment: "old", "ASKHUMAN_HOME": "/untrusted",
            "ASKHUMAN_DEV_TEST_PROFILE": "old", "ASKHUMAN_DEV_VERIFICATION_ID": "old",
            "ASKHUMAN_DEV_DRIVER_PID": "old", "ASKHUMAN_DEV_TEST_WINDOW": "old", "KEEP_ME": "kept",
        ]
        let probe = fixture.probeEnvironment(oldEnvironment)
        for key in oldEnvironment.keys where key != "KEEP_ME" { XCTAssertNil(probe[key], key) }
        XCTAssertEqual(probe["KEEP_ME"], "kept")
        XCTAssertEqual(probe["HOME"], tree.account.path)
        XCTAssertEqual(probe["ASKHUMAN_NO_KEYCHAIN"], "1")
        let tick = try fixture.tickEnvironment(oldEnvironment, config: tree.config(fixture))
        XCTAssertEqual(try SystemFixture.inheritedIdentity(tick), fixture.identity)
        let raw = try XCTUnwrap(tick[SystemFixture.targetEnvironment])
        let target = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
        XCTAssertEqual(target["version"] as? Int, 2)
        XCTAssertEqual(target["kind"] as? String, "system")
        XCTAssertEqual(target["binary"] as? String, tree.executable.path)
        XCTAssertEqual(target["home"] as? String, fixture.dataDirectory.path)
        XCTAssertTrue(target["worktree"] is NSNull)
        XCTAssertEqual(target["reason"] as? String, "explicit system fixture")
        XCTAssertEqual(target["noKeychain"] as? Bool, true)
        XCTAssertEqual(tick["ASKHUMAN_TODO_SYNC_CALLER"], "helper")
    }

    func testOnlyExplicitFixtureCapabilityV2IsAccepted() {
        XCTAssertTrue(SystemFixture.supportsCapabilities(Data(#"{"systemFixture":2,"developmentVerification":2}"#.utf8)))
        for raw in [#"{"developmentVerification":2}"#, #"{"systemFixture":1}"#,
                    #"{"systemFixture":true}"#, #"{"systemFixture":"2"}"#, "not JSON",
                    String(repeating: " ", count: 16 * 1024) + #"{"systemFixture":2}"#] {
            XCTAssertFalse(SystemFixture.supportsCapabilities(Data(raw.utf8)))
        }
    }
}
