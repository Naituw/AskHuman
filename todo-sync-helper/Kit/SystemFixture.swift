import Darwin
import Foundation

struct FixtureIdentity: Codable, Equatable {
    let version: Int
    let accountHome: String
}

struct FixtureError: Error, CustomStringConvertible {
    let description: String

    init(_ description: String) { self.description = description }
}

/// The helper has no fixture login item: a test must explicitly launch this guarded invocation.
enum HelperInvocation: Equatable {
    case version
    case run(SystemFixture?)
    case loginItemStatus
    case register
    case unregister
    case unmanaged(command: String)

    static func parse(
        _ arguments: [String], environment: [String: String],
        realAccountHome: () throws -> URL = { try SystemFixture.currentAccountHome() },
        userID: uid_t = getuid()
    ) throws -> HelperInvocation {
        // This exact probe never reads paths, inherited context, or account information.
        if arguments == ["--version"] { return .version }
        let inherited = try SystemFixture.inheritedIdentity(environment)
        if arguments.count == 4, arguments[0] == "--version",
            arguments[1] == "--internal-system-fixture-v2", arguments[3] == "run"
        {
            if environment[SystemFixture.targetEnvironment] != nil && inherited == nil {
                throw FixtureError("fixture launch conflicts with an inherited non-fixture target")
            }
            let fixture = try SystemFixture(
                accountHome: arguments[2], realAccountHome: realAccountHome(), userID: userID)
            try fixture.validateEnvironment(environment, inherited: inherited)
            return .run(fixture)
        }
        if arguments.count == 1,
            ["--register", "--unregister", "--login-item-status"].contains(arguments[0])
        {
            if inherited != nil { return .unmanaged(command: arguments[0]) }
            switch arguments[0] {
            case "--register": return .register
            case "--unregister": return .unregister
            default: return .loginItemStatus
            }
        }
        if arguments.isEmpty && inherited == nil { return .run(nil) }
        throw FixtureError("invalid helper arguments; fixtures require --version --internal-system-fixture-v2 ABSOLUTE_HOME run")
    }
}

/// A pinned account anchor, separate from both the real account and the caller's HOME.
struct SystemFixture: Equatable {
    static let targetEnvironment = "ASKHUMAN_RESOLVED_TARGET_V1"
    let accountHome: URL
    private let realAccountHome: URL
    private let userID: uid_t

    var identity: FixtureIdentity { FixtureIdentity(version: 2, accountHome: accountHome.path) }
    var dataDirectory: URL { accountHome.appendingPathComponent(".askhuman", isDirectory: true) }
    var paths: HelperPaths {
        HelperPaths(stateDir: dataDirectory.appendingPathComponent("state/todo-sync", isDirectory: true))
    }
    var todosURL: URL { dataDirectory.appendingPathComponent("state/todos.json") }
    var logURL: URL { paths.stateDir.appendingPathComponent("helper.log") }

    init(accountHome: String, realAccountHome: URL, userID: uid_t = getuid()) throws {
        let supplied = try Self.absoluteURL(accountHome)
        self.accountHome = try Self.physicalURL(supplied)
        self.realAccountHome = try Self.physicalURL(realAccountHome)
        self.userID = userID
        try validateLayout()
    }

    static func currentAccountHome() throws -> URL {
        let uid = getuid()
        let suggested = sysconf(_SC_GETPW_R_SIZE_MAX)
        var capacity = suggested > 0 ? min(Int(suggested), 1_048_576) : 16_384
        while capacity <= 1_048_576 {
            var record = passwd()
            var found: UnsafeMutablePointer<passwd>? = nil
            var buffer = [CChar](repeating: 0, count: capacity)
            let (status, home) = buffer.withUnsafeMutableBufferPointer { bytes -> (Int32, String?) in
                let status = getpwuid_r(uid, &record, bytes.baseAddress, bytes.count, &found)
                guard status == 0, found != nil, let directory = record.pw_dir else { return (status, nil) }
                return (status, String(validatingUTF8: directory))
            }
            if status == ERANGE {
                capacity *= 2
                continue
            }
            guard status == 0, let home, home.hasPrefix("/") else {
                throw FixtureError("cannot resolve account home for uid \(uid) (status \(status)); repair the account record before starting a fixture helper")
            }
            return try absoluteURL(home)
        }
        throw FixtureError("account record for uid \(uid) exceeds the lookup limit; repair the account record before starting a fixture helper")
    }

    func validateLayout() throws {
        guard let root = try Self.metadata(accountHome), (root.st_mode & S_IFMT) == S_IFDIR,
            root.st_uid == userID, root.st_mode & 0o022 == 0
        else { throw FixtureError("fixture account home must exist and be an owner-controlled directory") }
        let production = try Self.physicalURL(
            realAccountHome.appendingPathComponent(".askhuman"), allowMissing: true)
        guard !Self.contains(accountHome, realAccountHome),
            !Self.contains(dataDirectory, production), !Self.contains(production, dataDirectory)
        else { throw FixtureError("fixture account or data directory overlaps production") }
        for relative in [".askhuman", ".askhuman/state", ".askhuman/state/todo-sync"] {
            if let info = try Self.metadata(accountHome.appendingPathComponent(relative)) {
                guard info.st_mode & S_IFMT == S_IFDIR, info.st_uid == userID,
                    info.st_mode & 0o022 == 0
                else { throw FixtureError("fixture data directories must be private directories without symlinks") }
            }
        }
        var files = [paths.configURL, paths.lockURL, todosURL, logURL]
        files += (1...3).map { logURL.appendingPathExtension(String($0)) }
        for file in files {
            if let info = try Self.metadata(file), info.st_mode & S_IFMT != S_IFREG {
                throw FixtureError("fixture state files must be regular files without symlinks")
            }
        }
        if let info = try Self.metadata(paths.socketURL), info.st_mode & S_IFMT != S_IFSOCK {
            throw FixtureError("fixture helper socket must not be a linked or regular file")
        }
    }

    func prepareStateDirectory() throws {
        try validateLayout()
        try FileManager.default.createDirectory(
            at: paths.stateDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try validateLayout()
    }

    func validateConfiguration(_ config: HelperConfig) throws {
        try validateLayout()
        guard config.protocolVersion == Wire.protocolVersion,
            try Self.samePath(config.stateDir, paths.stateDir),
            try Self.samePath(config.todosPath, todosURL)
        else { throw FixtureError("helper configuration does not belong to this fixture") }
        let program = try Self.absoluteURL(config.askhuman)
        let physical = try Self.physicalURL(program)
        guard !program.pathComponents.contains(".askhuman-dev"),
            !physical.pathComponents.contains(".askhuman-dev"),
            let info = try Self.metadata(physical), info.st_mode & S_IFMT == S_IFREG,
            FileManager.default.isExecutableFile(atPath: program.path)
        else { throw FixtureError("fixture tick requires an executable System AskHuman entry") }
    }

    func probeEnvironment(_ original: [String: String]) -> [String: String] {
        var environment = original
        for key in [
            Self.targetEnvironment, "ASKHUMAN_HOME", "ASKHUMAN_DEV_TEST_PROFILE",
            "ASKHUMAN_DEV_VERIFICATION_ID", "ASKHUMAN_DEV_DRIVER_PID", "ASKHUMAN_DEV_TEST_WINDOW",
        ] { environment.removeValue(forKey: key) }
        environment["HOME"] = accountHome.path
        environment["ASKHUMAN_NO_KEYCHAIN"] = "1"
        environment["ASKHUMAN_TODO_SYNC_DIR"] = paths.stateDir.path
        environment["ASKHUMAN_TODO_SYNC_CALLER"] = "helper"
        return environment
    }

    func tickEnvironment(_ original: [String: String], config: HelperConfig) throws -> [String: String] {
        try validateConfiguration(config)
        var environment = probeEnvironment(original)
        let target: [String: Any] = [
            "version": 2, "kind": "system", "binary": config.askhuman,
            "home": dataDirectory.path, "worktree": NSNull(), "reason": "explicit system fixture",
            "noKeychain": true, "fixture": ["version": 2, "accountHome": accountHome.path],
        ]
        environment[Self.targetEnvironment] = String(
            decoding: try JSONSerialization.data(withJSONObject: target, options: [.sortedKeys]), as: UTF8.self)
        return environment
    }

    static func supportsCapabilities(_ output: Data) -> Bool {
        struct Capabilities: Decodable { let systemFixture: Int }
        return output.count <= 16 * 1024
            && (try? JSONDecoder().decode(Capabilities.self, from: output).systemFixture) == 2
    }

    private struct InheritedTarget: Decodable {
        let version: Int
        let kind: String?
        let binary: String?
        let home: String?
        let worktree: String?
        let noKeychain: Bool?
        let fixture: FixtureIdentity?
    }

    static func inheritedIdentity(_ environment: [String: String]) throws -> FixtureIdentity? {
        guard let raw = environment[targetEnvironment] else { return nil }
        guard raw.utf8.count <= 64 * 1024,
            let target = try? JSONDecoder().decode(InheritedTarget.self, from: Data(raw.utf8)),
            let object = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any]
        else { throw FixtureError("invalid inherited AskHuman target") }
        if target.version == 1 {
            guard !object.keys.contains("fixture") else { throw FixtureError("v1 target cannot carry a fixture") }
            return nil
        }
        guard target.version == 2, let identity = target.fixture, identity.version == 2,
            target.kind == "system", target.worktree == nil, target.noKeychain == true,
            let binary = target.binary, let home = target.home
        else { throw FixtureError("helper requires a v2 System fixture target") }
        _ = try absoluteURL(binary)
        let anchor = try absoluteURL(identity.accountHome)
        guard try samePath(home, anchor.appendingPathComponent(".askhuman")) else {
            throw FixtureError("inherited fixture home does not match its account")
        }
        return identity
    }

    fileprivate func validateEnvironment(_ environment: [String: String], inherited: FixtureIdentity?) throws {
        if let inherited, !(try Self.samePath(inherited.accountHome, accountHome)) {
            throw FixtureError("fixture arguments conflict with the inherited account")
        }
        if let home = environment["ASKHUMAN_HOME"], !home.isEmpty {
            throw FixtureError("a System helper fixture cannot inherit ASKHUMAN_HOME")
        }
        if let state = environment["ASKHUMAN_TODO_SYNC_DIR"], !state.isEmpty,
            !(try Self.samePath(state, paths.stateDir))
        { throw FixtureError("ASKHUMAN_TODO_SYNC_DIR conflicts with the fixture") }
    }

    private static func absoluteURL(_ path: String) throws -> URL {
        guard path.hasPrefix("/"), !path.contains("\0") else {
            throw FixtureError("fixture paths must be absolute")
        }
        return URL(fileURLWithPath: path)
    }

    private static func samePath(_ path: String, _ expected: URL) throws -> Bool {
        try physicalURL(absoluteURL(path), allowMissing: true).path
            == physicalURL(expected, allowMissing: true).path
    }

    /// Match Rust and Node's POSIX identity, including absent files below a symlinked parent.
    /// Foundation can map an existing /private/tmp path to /tmp but leave a missing child unchanged.
    private static func physicalURL(_ url: URL, allowMissing: Bool = false) throws -> URL {
        var ancestor = url
        var suffix: [String] = []
        while true {
            if let resolved = Darwin.realpath(ancestor.path, nil) {
                defer { free(resolved) }
                guard let path = String(validatingUTF8: resolved) else {
                    throw FixtureError("fixture paths must contain valid UTF-8")
                }
                var physical = URL(fileURLWithPath: path)
                if !suffix.isEmpty {
                    guard let info = try metadata(physical), info.st_mode & S_IFMT == S_IFDIR else {
                        throw FixtureError("fixture path parent must be a directory")
                    }
                    for component in suffix.reversed() {
                        physical.appendPathComponent(component)
                    }
                }
                return physical
            }
            let failure = errno
            guard allowMissing, failure == ENOENT else {
                throw NSError(domain: NSPOSIXErrorDomain, code: Int(failure))
            }
            // A dangling symlink is an invalid existing entry, not a missing path component.
            if let _ = try metadata(ancestor) {
                throw NSError(domain: NSPOSIXErrorDomain, code: Int(failure))
            }
            let component = ancestor.lastPathComponent
            guard ancestor.path != "/", !component.isEmpty, component != ".", component != ".." else {
                throw FixtureError("cannot resolve fixture path through a missing parent")
            }
            suffix.append(component)
            ancestor.deleteLastPathComponent()
        }
    }

    private static func contains(_ parent: URL, _ child: URL) -> Bool {
        parent.path == "/" || child.path == parent.path || child.path.hasPrefix(parent.path + "/")
    }

    private static func metadata(_ url: URL) throws -> stat? {
        var info = stat()
        if lstat(url.path, &info) == 0 { return info }
        if errno == ENOENT { return nil }
        throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
    }
}
