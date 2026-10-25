import CryptoKit
import Foundation

/// X25519, HKDF-SHA256, HMAC-SHA256 and ChaCha20-Poly1305 of wire protocol v1 (plan §3.2).
/// All derivation strings and byte layouts mirror `src-tauri/src/ios/crypto.rs` and are pinned by
/// `ios/Fixtures/wire-v1/crypto.json`.
public enum AHCrypto {
    public static let keyLength = 32
    public static let nonceLength = 12

    /// An X25519 key pair; ephemeral keys live only for the duration of one pairing.
    public struct KeyPair {
        private let privateKey: Curve25519.KeyAgreement.PrivateKey
        public var publicKey: Data { privateKey.publicKey.rawRepresentation }

        public init() { privateKey = Curve25519.KeyAgreement.PrivateKey() }

        public init(secret: Data) throws {
            privateKey = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: secret)
        }

        /// The raw 32-byte shared secret; `nil` for a low-order peer key (non-contributory result).
        public func sharedSecret(with peerPublic: Data) -> Data? {
            guard let peer = try? Curve25519.KeyAgreement.PublicKey(rawRepresentation: peerPublic),
                  let shared = try? privateKey.sharedSecretFromKeyAgreement(with: peer)
            else { return nil }
            let bytes = shared.withUnsafeBytes { Data($0) }
            return bytes.contains(where: { $0 != 0 }) ? bytes : nil
        }
    }

    /// `k_t = HKDF-SHA256(ikm = ticketSecret, salt = "ah1-ticket", info = ticketId, L = 32)`.
    public static func ticketKey(ticketSecret: Data, ticketId: String) -> Data {
        hkdf(ikm: ticketSecret, salt: Data("ah1-ticket".utf8), info: Data(ticketId.utf8), length: 32)
    }

    public struct PairKeys: Equatable, Sendable {
        public var m2p: Data
        public var p2m: Data
        public var sas: Data
    }

    public struct PairContext {
        public var macId: String
        public var phoneId: String
        public var ticketId: String
        public var ticketSecret: Data
        public var macPublic: Data
        public var phonePublic: Data

        public init(macId: String, phoneId: String, ticketId: String, ticketSecret: Data, macPublic: Data, phonePublic: Data) {
            self.macId = macId
            self.phoneId = phoneId
            self.ticketId = ticketId
            self.ticketSecret = ticketSecret
            self.macPublic = macPublic
            self.phonePublic = phonePublic
        }
    }

    /// `okm = HKDF-SHA256(ikm = ss, salt = ticketSecret, info = "ah1-pair" 00 macId 00 phoneId 00
    /// ticketId 00 eMacPub ePhonePub, L = 96)`, split into `m2p`, `p2m` and `sas`.
    public static func derivePairKeys(sharedSecret: Data, context: PairContext) -> PairKeys {
        var info = Data("ah1-pair".utf8)
        for part in [context.macId, context.phoneId, context.ticketId] {
            info.append(0)
            info.append(Data(part.utf8))
        }
        info.append(0)
        info.append(context.macPublic)
        info.append(context.phonePublic)
        let okm = hkdf(ikm: sharedSecret, salt: context.ticketSecret, info: info, length: 96)
        return PairKeys(m2p: okm.subdata(in: 0..<32), p2m: okm.subdata(in: 32..<64), sas: okm.subdata(in: 64..<96))
    }

    /// Six-digit verification code: first four bytes of `HMAC-SHA256(k_sas, "ah1-sas")`,
    /// big-endian, modulo 1,000,000, zero padded.
    public static func sasCode(kSas: Data) -> String {
        let mac = HMAC<SHA256>.authenticationCode(for: Data("ah1-sas".utf8), using: SymmetricKey(data: kSas))
        let bytes = Array(mac)
        let n = (UInt32(bytes[0]) << 24 | UInt32(bytes[1]) << 16 | UInt32(bytes[2]) << 8 | UInt32(bytes[3])) % 1_000_000
        return String(format: "%06u", n)
    }

    public static func hkdf(ikm: Data, salt: Data, info: Data, length: Int) -> Data {
        let key = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: SymmetricKey(data: ikm), salt: salt, info: info, outputByteCount: length
        )
        return key.withUnsafeBytes { Data($0) }
    }

    /// Returns ciphertext ‖ 16-byte tag.
    public static func seal(key: Data, nonce: Data, aad: Data, plaintext: Data) throws -> Data {
        let box = try ChaChaPoly.seal(
            plaintext, using: SymmetricKey(data: key), nonce: ChaChaPoly.Nonce(data: nonce), authenticating: aad
        )
        return box.ciphertext + box.tag
    }

    /// Returns `nil` when authentication fails.
    public static func open(key: Data, nonce: Data, aad: Data, ciphertext: Data) -> Data? {
        guard ciphertext.count >= 16, let nonce = try? ChaChaPoly.Nonce(data: nonce) else { return nil }
        let body = ciphertext.prefix(ciphertext.count - 16)
        let tag = ciphertext.suffix(16)
        guard let box = try? ChaChaPoly.SealedBox(nonce: nonce, ciphertext: body, tag: tag) else { return nil }
        return try? ChaChaPoly.open(box, using: SymmetricKey(data: key), authenticating: aad)
    }
}
