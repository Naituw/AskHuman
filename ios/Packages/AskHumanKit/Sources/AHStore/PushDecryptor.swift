import Foundation
import AHProtocol

/// A push after decryption: the Mac that sent it and what it said.
public struct DecryptedPush: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case ask
        case confirm
        case followDone
        case followWaiting
        case followFailed
        case resolved
        /// A kind a newer Mac sends that this build does not know.
        case other(String)

        public init(_ wire: String) {
            switch wire {
            case "ask": self = .ask
            case "confirm": self = .confirm
            case "follow.done": self = .followDone
            case "follow.waiting": self = .followWaiting
            case "follow.failed": self = .followFailed
            case "resolved": self = .resolved
            default: self = .other(wire)
            }
        }
    }

    public var macId: String
    public var plain: PushPlain

    public var kind: Kind { Kind(plain.k) }

    public init(macId: String, plain: PushPlain) {
        self.macId = macId
        self.plain = plain
    }
}

/// Opens the end-to-end encrypted preview inside a push (run by the notification extension, which
/// shares the pairing keys through the keychain).
public enum PushDecryptor {
    public enum Failure: Error, Equatable, Sendable {
        /// The push has no `ah` payload (a plain system notification).
        case noPayload
        case malformed
        /// The sender is not a Mac this phone is paired with (any more).
        case unknownMac
        /// The ciphertext did not authenticate with the pairing key.
        case notAuthentic
    }

    public static func decrypt(userInfo: [AnyHashable: Any], vault: Vault) -> Result<DecryptedPush, Failure> {
        guard let object = userInfo["ah"] else { return .failure(.noPayload) }
        guard var fields = object as? [String: Any] else { return .failure(.malformed) }
        // The relay forwards the envelope as {f, k, n, c} (plan §4.8), without the version: a push is
        // version 1 by construction. A version that is there must still be the one we speak.
        if fields["v"] == nil { fields["v"] = Wire.proto }
        guard JSONSerialization.isValidJSONObject(fields),
              let data = try? JSONSerialization.data(withJSONObject: fields),
              let envelope = try? AHEnvelope.parse(data)
        else { return .failure(.malformed) }
        guard let pairing = vault.pairing(macId: envelope.f) else { return .failure(.unknownMac) }
        guard let plaintext = try? AHEnvelope.open(key: pairing.m2p, scope: .push, envelope: envelope) else {
            return .failure(.notAuthentic)
        }
        guard let plain = try? AHJSON.decoder.decode(PushPlain.self, from: plaintext) else { return .failure(.malformed) }
        return .success(DecryptedPush(macId: pairing.macId, plain: plain))
    }
}
