import Foundation
import Security

/// Byte-level helpers of wire protocol v1 (plan §2, §3.2).
public enum Bytes {
    /// Lowercase RFC 4648 base32 without padding (`macId`, `phoneId`, `ticketId`).
    public static func base32Lower(_ data: Data) -> String {
        let alphabet = Array("abcdefghijklmnopqrstuvwxyz234567".utf8)
        var out = [UInt8]()
        out.reserveCapacity((data.count * 8 + 4) / 5)
        var acc: UInt32 = 0
        var bits: UInt32 = 0
        for byte in data {
            acc = (acc << 8) | UInt32(byte)
            bits += 8
            while bits >= 5 {
                bits -= 5
                out.append(alphabet[Int((acc >> bits) & 0x1f)])
            }
        }
        if bits > 0 {
            out.append(alphabet[Int((acc << (5 - bits)) & 0x1f)])
        }
        return String(decoding: out, as: UTF8.self)
    }

    /// Base64url without padding.
    public static func b64u(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    /// Strict base64url decoding (no padding, no standard-alphabet characters).
    public static func b64uDecode(_ text: String) -> Data? {
        guard text.utf8.allSatisfy({ isB64uByte($0) }), text.utf8.count % 4 != 1 else { return nil }
        var s = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while s.count % 4 != 0 { s += "=" }
        return Data(base64Encoded: s)
    }

    private static func isB64uByte(_ b: UInt8) -> Bool {
        (b >= 0x41 && b <= 0x5a) || (b >= 0x61 && b <= 0x7a) || (b >= 0x30 && b <= 0x39) || b == 0x2d || b == 0x5f
    }

    public static func hex(_ data: Data) -> String {
        data.map { String(format: "%02x", $0) }.joined()
    }

    public static func fromHex(_ text: String) -> Data? {
        guard text.count % 2 == 0 else { return nil }
        var out = Data(capacity: text.count / 2)
        var index = text.startIndex
        while index < text.endIndex {
            let next = text.index(index, offsetBy: 2)
            guard let byte = UInt8(text[index..<next], radix: 16) else { return nil }
            out.append(byte)
            index = next
        }
        return out
    }

    /// Cryptographically secure random bytes.
    public static func random(_ count: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        let status = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        precondition(status == errSecSuccess, "secure random generator unavailable")
        return Data(bytes)
    }

    /// A random 16-byte identifier as 26 base32 characters.
    public static func newId() -> String { base32Lower(random(16)) }

    /// A random 16-byte message identifier as base64url.
    public static func newMsgId() -> String { b64u(random(16)) }

    public static func isValidId(_ s: String) -> Bool {
        s.utf8.count == 26 && s.utf8.allSatisfy { ($0 >= 0x61 && $0 <= 0x7a) || ($0 >= 0x32 && $0 <= 0x37) }
    }
}
