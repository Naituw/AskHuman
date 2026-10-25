//! Envelope and plaintext message framing of wire protocol v1 (plan §3.2).
//!
//! Envelope (UTF-8 JSON): `{"v":1,"f":"<sender id>","k":<kid>,"n":"<b64url 12B>","c":"<b64url ct||tag>"}`.
//! Plaintext: `{"i":<msgId>,"t":<type>,"ts":<unix ms>,"b":<body>}`.
//! AEAD: ChaCha20-Poly1305 with AAD `"ah1|" + topic + "|" + f + "|" + k`; pushes use
//! `"ah1|push|" + f + "|" + k`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::crypto::{self, KEY_LEN, NONCE_LEN};
use super::wire::{MAX_ENVELOPE_BYTES, PROTO};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("envelope too large")]
    TooLarge,
    #[error("malformed envelope")]
    Malformed,
    #[error("unsupported envelope version")]
    Version,
    #[error("authentication failed")]
    Auth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub f: String,
    pub k: u32,
    pub n: String,
    pub c: String,
}

/// Where an envelope travels; part of the authenticated data so a ciphertext cannot be replayed on
/// another topic or in the push domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'a> {
    Topic(&'a str),
    Push,
}

fn aad(scope: Scope<'_>, sender: &str, kid: u32) -> Vec<u8> {
    match scope {
        Scope::Topic(topic) => format!("ah1|{topic}|{sender}|{kid}").into_bytes(),
        Scope::Push => format!("ah1|push|{sender}|{kid}").into_bytes(),
    }
}

/// Parses and size/version-checks an envelope. A `z` field (reserved for compression) is rejected.
pub fn parse(bytes: &[u8]) -> Result<Envelope, EnvelopeError> {
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError::TooLarge);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| EnvelopeError::Malformed)?;
    if value.get("z").is_some() {
        return Err(EnvelopeError::Malformed);
    }
    let env: Envelope = serde_json::from_value(value).map_err(|_| EnvelopeError::Malformed)?;
    if env.v != PROTO {
        return Err(EnvelopeError::Version);
    }
    Ok(env)
}

pub fn encode(env: &Envelope) -> Vec<u8> {
    serde_json::to_vec(env).expect("envelope serializes")
}

/// Seals with an explicit nonce (fixtures and tests); production code uses [`seal`].
pub fn seal_with_nonce(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Envelope {
    let ct = crypto::aead_seal(key, nonce, &aad(scope, sender, kid), plaintext);
    Envelope {
        v: PROTO,
        f: sender.to_string(),
        k: kid,
        n: crypto::b64u(nonce),
        c: crypto::b64u(&ct),
    }
}

pub fn seal(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    plaintext: &[u8],
) -> Envelope {
    seal_with_nonce(
        key,
        scope,
        sender,
        kid,
        &crypto::random_bytes::<NONCE_LEN>(),
        plaintext,
    )
}

pub fn open(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    env: &Envelope,
) -> Result<Vec<u8>, EnvelopeError> {
    let nonce: [u8; NONCE_LEN] = crypto::b64u_decode(&env.n)
        .and_then(|v| v.try_into().ok())
        .ok_or(EnvelopeError::Malformed)?;
    let ct = crypto::b64u_decode(&env.c).ok_or(EnvelopeError::Malformed)?;
    crypto::aead_open(key, &nonce, &aad(scope, &env.f, env.k), &ct).ok_or(EnvelopeError::Auth)
}

/// Decrypted application message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plain {
    /// Message id (base64url of 16 random bytes); receivers de-duplicate on it.
    pub i: String,
    /// Message type, e.g. `request.new`.
    pub t: String,
    /// Sender clock, unix milliseconds (replay window).
    pub ts: i64,
    pub b: Value,
}

impl Plain {
    pub fn new(msg_type: &str, body: Value, now_ms: i64) -> Self {
        Self {
            i: crypto::new_msg_id(),
            t: msg_type.to_string(),
            ts: now_ms,
            b: body,
        }
    }
}

/// Builds the wire bytes for one message.
pub fn seal_plain(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    plain: &Plain,
) -> Vec<u8> {
    let pt = serde_json::to_vec(plain).expect("plain message serializes");
    encode(&seal(key, scope, sender, kid, &pt))
}

pub fn open_plain(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    env: &Envelope,
) -> Result<Plain, EnvelopeError> {
    let pt = open(key, scope, env)?;
    serde_json::from_slice(&pt).map_err(|_| EnvelopeError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key() -> [u8; 32] {
        [7u8; 32]
    }

    #[test]
    fn plain_round_trip_over_wire_bytes() {
        let plain = Plain::new("hello", json!({"nonce": "n1"}), 1_700_000_000_000);
        let bytes = seal_plain(&key(), Scope::Topic("ah/u/d/p"), "macid", 1, &plain);
        let env = parse(&bytes).unwrap();
        assert_eq!(env.f, "macid");
        assert_eq!(env.k, 1);
        let got = open_plain(&key(), Scope::Topic("ah/u/d/p"), &env).unwrap();
        assert_eq!(got, plain);
    }

    #[test]
    fn aad_binds_topic_sender_kid_and_domain() {
        let env = seal(&key(), Scope::Topic("ah/u/d/p"), "macid", 1, b"x");
        assert!(open(&key(), Scope::Topic("ah/u/d/other"), &env).is_err());
        assert!(open(&key(), Scope::Push, &env).is_err());
        let mut other_sender = env.clone();
        other_sender.f = "someone".into();
        assert!(open(&key(), Scope::Topic("ah/u/d/p"), &other_sender).is_err());
        let mut other_kid = env.clone();
        other_kid.k = 2;
        assert!(open(&key(), Scope::Topic("ah/u/d/p"), &other_kid).is_err());
        let push = seal(&key(), Scope::Push, "macid", 1, b"x");
        assert!(open(&key(), Scope::Push, &push).is_ok());
        assert!(open(&key(), Scope::Topic("ah/u/d/p"), &push).is_err());
    }

    #[test]
    fn every_tampered_byte_of_the_ciphertext_is_rejected() {
        let env = seal(&key(), Scope::Push, "macid", 1, b"preview text");
        let ct = crypto::b64u_decode(&env.c).unwrap();
        for i in 0..ct.len() {
            let mut bad = ct.clone();
            bad[i] ^= 0x01;
            let tampered = Envelope {
                c: crypto::b64u(&bad),
                ..env.clone()
            };
            assert_eq!(
                open(&key(), Scope::Push, &tampered),
                Err(EnvelopeError::Auth),
                "byte {i}"
            );
        }
    }

    #[test]
    fn parse_enforces_size_version_and_reserved_fields() {
        assert_eq!(
            parse(&vec![b' '; MAX_ENVELOPE_BYTES + 1]),
            Err(EnvelopeError::TooLarge)
        );
        assert_eq!(parse(b"not json"), Err(EnvelopeError::Malformed));
        assert_eq!(
            parse(br#"{"v":2,"f":"a","k":1,"n":"x","c":"y"}"#),
            Err(EnvelopeError::Version)
        );
        assert_eq!(
            parse(br#"{"v":1,"f":"a","k":1,"n":"x","c":"y","z":1}"#),
            Err(EnvelopeError::Malformed)
        );
        assert_eq!(
            parse(br#"{"v":1,"f":"a","k":1}"#),
            Err(EnvelopeError::Malformed)
        );
        assert!(parse(br#"{"v":1,"f":"a","k":1,"n":"x","c":"y"}"#).is_ok());
    }

    #[test]
    fn bad_nonce_or_ciphertext_encoding_is_malformed() {
        let mut env = seal(&key(), Scope::Push, "a", 1, b"x");
        env.n = "AAAA".into();
        assert_eq!(
            open(&key(), Scope::Push, &env),
            Err(EnvelopeError::Malformed)
        );
        let mut env = seal(&key(), Scope::Push, "a", 1, b"x");
        env.c = "!!!".into();
        assert_eq!(
            open(&key(), Scope::Push, &env),
            Err(EnvelopeError::Malformed)
        );
    }
}
