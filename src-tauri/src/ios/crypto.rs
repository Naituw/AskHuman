//! Cryptographic primitives of wire protocol v1 (plan §3.2): X25519, HKDF-SHA256, HMAC-SHA256
//! (verification code) and ChaCha20-Poly1305.
//!
//! All derivation strings and byte layouts here are normative for the iOS side as well; the shared
//! fixtures in `ios/Fixtures/wire-v1/crypto.json` pin every intermediate value.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

/// Fills a fixed-size array from the operating system's CSPRNG.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).expect("operating system random generator unavailable");
    buf
}

pub fn b64u(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

pub fn b64u_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.as_bytes()).ok()
}

/// Lowercase RFC 4648 base32 without padding.
pub fn base32_lower(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity((data.len() * 8).div_ceil(5));
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &byte in data {
        acc = (acc << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// A random 16-byte identifier rendered as 26 base32 characters (`macId`, `phoneId`, `ticketId`).
pub fn new_id() -> String {
    base32_lower(&random_bytes::<16>())
}

/// A random 16-byte message identifier rendered as base64url (`msgId`).
pub fn new_msg_id() -> String {
    b64u(&random_bytes::<16>())
}

/// True for strings produced by [`new_id`]; used to validate ids received from the network.
pub fn is_valid_id(s: &str) -> bool {
    s.len() == 26
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// An X25519 key pair. Ephemeral keys live only for the duration of one pairing.
pub struct KeyPair {
    secret: StaticSecret,
    pub public: [u8; 32],
}

impl KeyPair {
    pub fn generate() -> Self {
        Self::from_secret_bytes(random_bytes::<32>())
    }

    pub fn from_secret_bytes(bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret).to_bytes();
        Self { secret, public }
    }

    /// Returns `None` for a non-contributory result (low-order peer key).
    pub fn diffie_hellman(&self, peer_public: &[u8; 32]) -> Option<[u8; 32]> {
        let shared = self.secret.diffie_hellman(&PublicKey::from(*peer_public));
        shared.was_contributory().then(|| shared.to_bytes())
    }
}

/// `k_t = HKDF-SHA256(ikm = ticketSecret, salt = "ah1-ticket", info = ticketId, L = 32)`.
pub fn ticket_key(ticket_secret: &[u8; 32], ticket_id: &str) -> [u8; KEY_LEN] {
    let hk = Hkdf::<Sha256>::new(Some(b"ah1-ticket"), ticket_secret);
    let mut out = [0u8; KEY_LEN];
    hk.expand(ticket_id.as_bytes(), &mut out)
        .expect("32 bytes is a valid HKDF output length");
    out
}

/// Directional keys of one pairing plus the key that backs the verification code.
#[derive(Clone, PartialEq, Eq)]
pub struct PairKeys {
    pub m2p: [u8; KEY_LEN],
    pub p2m: [u8; KEY_LEN],
    pub sas: [u8; KEY_LEN],
}

impl std::fmt::Debug for PairKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairKeys(..)")
    }
}

pub struct PairContext<'a> {
    pub mac_id: &'a str,
    pub phone_id: &'a str,
    pub ticket_id: &'a str,
    pub ticket_secret: &'a [u8; 32],
    pub mac_public: &'a [u8; 32],
    pub phone_public: &'a [u8; 32],
}

/// `okm = HKDF-SHA256(ikm = ss, salt = ticketSecret, info = "ah1-pair" 00 macId 00 phoneId 00
/// ticketId 00 eMacPub ePhonePub, L = 96)`, split into `m2p`, `p2m` and `sas`.
pub fn derive_pair_keys(shared_secret: &[u8; 32], ctx: &PairContext<'_>) -> PairKeys {
    let mut info = Vec::with_capacity(8 + 4 + 26 * 3 + 64);
    info.extend_from_slice(b"ah1-pair");
    for part in [ctx.mac_id, ctx.phone_id, ctx.ticket_id] {
        info.push(0);
        info.extend_from_slice(part.as_bytes());
    }
    info.push(0);
    info.extend_from_slice(ctx.mac_public);
    info.extend_from_slice(ctx.phone_public);

    let hk = Hkdf::<Sha256>::new(Some(ctx.ticket_secret), shared_secret);
    let mut okm = [0u8; KEY_LEN * 3];
    hk.expand(&info, &mut okm)
        .expect("96 bytes is a valid HKDF output length");
    let mut keys = PairKeys {
        m2p: [0; KEY_LEN],
        p2m: [0; KEY_LEN],
        sas: [0; KEY_LEN],
    };
    keys.m2p.copy_from_slice(&okm[..KEY_LEN]);
    keys.p2m.copy_from_slice(&okm[KEY_LEN..KEY_LEN * 2]);
    keys.sas.copy_from_slice(&okm[KEY_LEN * 2..]);
    keys
}

/// Six-digit verification code: first four bytes of `HMAC-SHA256(k_sas, "ah1-sas")`, big-endian,
/// modulo 1,000,000, zero padded.
pub fn sas_code(k_sas: &[u8; KEY_LEN]) -> String {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(k_sas).expect("HMAC accepts any key length");
    mac.update(b"ah1-sas");
    let tag = mac.finalize().into_bytes();
    let n = u32::from_be_bytes([tag[0], tag[1], tag[2], tag[3]]) % 1_000_000;
    format!("{n:06}")
}

pub fn aead_seal(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("ChaCha20-Poly1305 encryption cannot fail for in-memory buffers")
}

pub fn aead_open(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .ok()
}

/// Constant-time equality for secrets and tokens.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_matches_rfc4648_vectors_lowercased() {
        assert_eq!(base32_lower(b""), "");
        assert_eq!(base32_lower(b"f"), "my");
        assert_eq!(base32_lower(b"fo"), "mzxq");
        assert_eq!(base32_lower(b"foo"), "mzxw6");
        assert_eq!(base32_lower(b"foob"), "mzxw6yq");
        assert_eq!(base32_lower(b"fooba"), "mzxw6ytb");
        assert_eq!(base32_lower(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn ids_have_the_documented_shape() {
        let id = new_id();
        assert_eq!(id.len(), 26);
        assert!(is_valid_id(&id));
        assert!(!is_valid_id("short"));
        assert!(!is_valid_id(&"A".repeat(26)));
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn diffie_hellman_agrees_and_rejects_low_order_points() {
        let a = KeyPair::generate();
        let b = KeyPair::generate();
        assert_eq!(a.diffie_hellman(&b.public), b.diffie_hellman(&a.public));
        assert!(a.diffie_hellman(&[0u8; 32]).is_none());
    }

    #[test]
    fn pair_keys_are_symmetric_and_bound_to_the_context() {
        let mac = KeyPair::generate();
        let phone = KeyPair::generate();
        let secret = random_bytes::<32>();
        let ctx = |mac_id: &'static str| PairContext {
            mac_id,
            phone_id: "bbbbbbbbbbbbbbbbbbbbbbbbbb",
            ticket_id: "cccccccccccccccccccccccccc",
            ticket_secret: &secret,
            mac_public: &mac.public,
            phone_public: &phone.public,
        };
        let ss1 = mac.diffie_hellman(&phone.public).unwrap();
        let ss2 = phone.diffie_hellman(&mac.public).unwrap();
        let k1 = derive_pair_keys(&ss1, &ctx("aaaaaaaaaaaaaaaaaaaaaaaaaa"));
        let k2 = derive_pair_keys(&ss2, &ctx("aaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert_eq!(k1, k2);
        assert_eq!(sas_code(&k1.sas), sas_code(&k2.sas));
        assert_ne!(k1.m2p, k1.p2m);
        let other = derive_pair_keys(&ss1, &ctx("dddddddddddddddddddddddddd"));
        assert_ne!(k1.m2p, other.m2p);
    }

    #[test]
    fn sas_code_is_six_digits() {
        for i in 0..64u8 {
            let code = sas_code(&[i; 32]);
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn aead_round_trip_and_tamper_detection() {
        let key = random_bytes::<32>();
        let nonce = random_bytes::<12>();
        let ct = aead_seal(&key, &nonce, b"aad", b"hello");
        assert_eq!(aead_open(&key, &nonce, b"aad", &ct).unwrap(), b"hello");
        assert!(aead_open(&key, &nonce, b"other", &ct).is_none());
        let mut bad = ct.clone();
        bad[0] ^= 1;
        assert!(aead_open(&key, &nonce, b"aad", &bad).is_none());
        assert!(aead_open(&random_bytes::<32>(), &nonce, b"aad", &ct).is_none());
    }

    #[test]
    fn ct_eq_compares_lengths_and_content() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
