import Foundation
import Security

/// Small key-value store for secrets: pairing keys, the broker password, the phone id.
public protocol SecretStore: Sendable {
    func get(_ account: String) -> Data?
    func set(_ data: Data, for account: String) throws
    func delete(_ account: String)
    func accounts(prefix: String) -> [String]
}

public struct SecretStoreError: Error, CustomStringConvertible {
    public var status: OSStatus
    public var description: String { "keychain error \(status)" }
}

/// In-memory store for tests and previews.
public final class MemorySecretStore: SecretStore, @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String: Data] = [:]

    public init() {}

    public func get(_ account: String) -> Data? {
        lock.lock(); defer { lock.unlock() }
        return items[account]
    }

    public func set(_ data: Data, for account: String) throws {
        lock.lock(); defer { lock.unlock() }
        items[account] = data
    }

    public func delete(_ account: String) {
        lock.lock(); defer { lock.unlock() }
        items[account] = nil
    }

    public func accounts(prefix: String) -> [String] {
        lock.lock(); defer { lock.unlock() }
        return items.keys.filter { $0.hasPrefix(prefix) }.sorted()
    }
}

/// Keychain-backed store (`AfterFirstUnlockThisDeviceOnly`: not synced, not in backups). With an
/// access group the app and the notification extension share the items.
public final class KeychainSecretStore: SecretStore, @unchecked Sendable {
    private let service: String
    private let accessGroup: String?

    public init(service: String, accessGroup: String? = nil) {
        self.service = service
        self.accessGroup = accessGroup
    }

    private func query(_ account: String? = nil, group: Bool = true) -> [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
        ]
        if let account { q[kSecAttrAccount as String] = account }
        if group, let accessGroup { q[kSecAttrAccessGroup as String] = accessGroup }
        return q
    }

    public func get(_ account: String) -> Data? {
        var q = query(account)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        var status = SecItemCopyMatching(q as CFDictionary, &out)
        if status == errSecMissingEntitlement, accessGroup != nil {
            // Unsigned builds (tests, some simulator setups) have no access group to use.
            q = query(account, group: false)
            q[kSecReturnData as String] = true
            q[kSecMatchLimit as String] = kSecMatchLimitOne
            status = SecItemCopyMatching(q as CFDictionary, &out)
        }
        return status == errSecSuccess ? out as? Data : nil
    }

    public func set(_ data: Data, for account: String) throws {
        func write(group: Bool) -> OSStatus {
            let update: [String: Any] = [kSecValueData as String: data]
            var status = SecItemUpdate(query(account, group: group) as CFDictionary, update as CFDictionary)
            if status == errSecItemNotFound {
                var add = query(account, group: group)
                add[kSecValueData as String] = data
                add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
                status = SecItemAdd(add as CFDictionary, nil)
            }
            return status
        }
        var status = write(group: true)
        if status == errSecMissingEntitlement, accessGroup != nil { status = write(group: false) }
        guard status == errSecSuccess else { throw SecretStoreError(status: status) }
    }

    public func delete(_ account: String) {
        var status = SecItemDelete(query(account) as CFDictionary)
        if status == errSecMissingEntitlement, accessGroup != nil {
            status = SecItemDelete(query(account, group: false) as CFDictionary)
        }
        _ = status
    }

    public func accounts(prefix: String) -> [String] {
        func list(group: Bool) -> (OSStatus, [String]) {
            var q = query(group: group)
            q[kSecReturnAttributes as String] = true
            q[kSecMatchLimit as String] = kSecMatchLimitAll
            var out: CFTypeRef?
            let status = SecItemCopyMatching(q as CFDictionary, &out)
            let rows = (out as? [[String: Any]]) ?? []
            return (status, rows.compactMap { $0[kSecAttrAccount as String] as? String }.filter { $0.hasPrefix(prefix) })
        }
        var (status, accounts) = list(group: true)
        if status == errSecMissingEntitlement, accessGroup != nil { (status, accounts) = list(group: false) }
        return accounts.sorted()
    }
}
