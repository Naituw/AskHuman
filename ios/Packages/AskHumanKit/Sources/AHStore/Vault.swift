import Foundation
import AHProtocol

/// Everything secret about this phone: its id, the broker account, the pairing keys and the push
/// handle. The notification extension reads the pairing keys through the shared keychain group.
public final class Vault: @unchecked Sendable {
    private let secrets: SecretStore
    private let lock = NSLock()

    private enum Account {
        static let phone = "phone.id"
        static let broker = "broker"
        static let push = "push"
        static let pairingPrefix = "pairing."
    }

    public init(secrets: SecretStore) { self.secrets = secrets }

    /// The phone's stable id (26 base32 characters); created on first use. It lives in the keychain, so
    /// it survives reinstalls and the pairings stay valid.
    public func phoneId() -> String {
        lock.lock(); defer { lock.unlock() }
        if let data = secrets.get(Account.phone), let id = String(data: data, encoding: .utf8), Bytes.isValidId(id) {
            return id
        }
        let id = Bytes.newId()
        try? secrets.set(Data(id.utf8), for: Account.phone)
        return id
    }

    // MARK: broker

    public var broker: BrokerSettings? {
        get { read(BrokerSettings.self, Account.broker) }
        set { write(newValue, Account.broker) }
    }

    // MARK: pairings

    public func pairings() -> [PairingRecord] {
        secrets.accounts(prefix: Account.pairingPrefix)
            .compactMap { read(PairingRecord.self, $0) }
            .sorted { $0.pairedAt < $1.pairedAt }
    }

    public func pairing(macId: String) -> PairingRecord? {
        read(PairingRecord.self, Account.pairingPrefix + macId)
    }

    public func savePairing(_ record: PairingRecord) throws {
        let data = try JSONEncoder().encode(record)
        try secrets.set(data, for: Account.pairingPrefix + record.macId)
    }

    public func removePairing(macId: String) {
        secrets.delete(Account.pairingPrefix + macId)
    }

    // MARK: push

    public var push: PushRegistration? {
        get { read(PushRegistration.self, Account.push) }
        set { write(newValue, Account.push) }
    }

    // MARK: helpers

    private func read<T: Decodable>(_ type: T.Type, _ account: String) -> T? {
        guard let data = secrets.get(account) else { return nil }
        return try? JSONDecoder().decode(type, from: data)
    }

    private func write<T: Encodable>(_ value: T?, _ account: String) {
        if let value, let data = try? JSONEncoder().encode(value) {
            try? secrets.set(data, for: account)
        } else {
            secrets.delete(account)
        }
    }
}
