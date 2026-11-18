//! Envelope and plaintext message framing of wire protocol v1 (plan §3.2).
//!
//! Envelope (UTF-8 JSON): `{"v":1,"f":"<sender id>","k":<kid>,"n":"<b64url 12B>","c":"<b64url ct||tag>"}`.
//! Plaintext: `{"i":<msgId>,"t":<type>,"ts":<unix ms>,"b":<body>}`.
//! AEAD: ChaCha20-Poly1305 with AAD `"ah1|" + topic + "|" + f + "|" + k`; pushes use
//! `"ah1|push|" + f + "|" + k`.
//!
//! Compression (docs/specs/ios-traffic-reduction.md §3.3): an envelope with `"z":1` carries the plaintext as raw
//! DEFLATE (RFC 1951, no zlib header or trailer), compressed before it was encrypted, and its AAD ends in `|z1`, so
//! the marker is authenticated. Only the Mac sends them, only on a topic and only to a phone that asked for them in
//! `hello`; a push is never compressed.

use std::io::Write;

use flate2::write::DeflateEncoder;
use flate2::{Compression, Decompress, FlushDecompress, Status};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::crypto::{self, KEY_LEN, NONCE_LEN};
use super::wire::{MAX_ENVELOPE_BYTES, MAX_PLAIN_BYTES, MIN_COMPRESS_BYTES, PROTO};

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
    /// `1`: the plaintext was compressed (raw DEFLATE) before it was encrypted. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<u32>,
}

/// Where an envelope travels; part of the authenticated data so a ciphertext cannot be replayed on
/// another topic or in the push domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'a> {
    Topic(&'a str),
    Push,
}

fn aad(scope: Scope<'_>, sender: &str, kid: u32, compressed: bool) -> Vec<u8> {
    let mut aad = match scope {
        Scope::Topic(topic) => format!("ah1|{topic}|{sender}|{kid}").into_bytes(),
        Scope::Push => format!("ah1|push|{sender}|{kid}").into_bytes(),
    };
    if compressed {
        // The marker is authenticated too: changing `z` makes the envelope fail to open.
        aad.extend_from_slice(b"|z1");
    }
    aad
}

/// Parses and size/version-checks an envelope. A `z` field is accepted only as the number 1 (raw DEFLATE, see
/// [`open`]); any other value is malformed, so a later algorithm can never be mistaken for this one.
pub fn parse(bytes: &[u8]) -> Result<Envelope, EnvelopeError> {
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError::TooLarge);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| EnvelopeError::Malformed)?;
    if value.get("z").is_some_and(|z| z != &Value::from(1)) {
        return Err(EnvelopeError::Malformed);
    }
    let env: Envelope = serde_json::from_value(value).map_err(|_| EnvelopeError::Malformed)?;
    if env.v != PROTO {
        return Err(EnvelopeError::Version);
    }
    Ok(env)
}

/// Like [`parse`] for an envelope a phone sent. Phones never compress (what they send is small), so the Mac
/// refuses a `z` there exactly as it did before compression existed, and has no inflate path on its inbound side.
pub fn parse_from_phone(bytes: &[u8]) -> Result<Envelope, EnvelopeError> {
    let env = parse(bytes)?;
    if env.z.is_some() {
        return Err(EnvelopeError::Malformed);
    }
    Ok(env)
}

pub fn encode(env: &Envelope) -> Vec<u8> {
    serde_json::to_vec(env).expect("envelope serializes")
}

/// Compresses `data` as raw DEFLATE (RFC 1951): what a `z: 1` envelope carries before it is encrypted.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut encoder =
        DeflateEncoder::new(Vec::with_capacity(data.len() / 3), Compression::default());
    encoder.write_all(data).expect("writing to memory");
    encoder.finish().expect("finishing in memory")
}

/// The compressed form of `plaintext`, when it is long enough to try ([`MIN_COMPRESS_BYTES`]) and the result is
/// shorter than the original.
fn pack_if_worth_it(plaintext: &[u8]) -> Option<Vec<u8>> {
    if plaintext.len() < MIN_COMPRESS_BYTES {
        return None;
    }
    let packed = compress(plaintext);
    (packed.len() < plaintext.len()).then_some(packed)
}

/// Inflates raw DEFLATE, refusing a damaged or truncated stream and one that expands past `limit` bytes (so a
/// small message cannot be made to fill the memory).
fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, EnvelopeError> {
    // Raw DEFLATE: no zlib header or trailer.
    let mut decompress = Decompress::new(false);
    let mut out: Vec<u8> = Vec::with_capacity(data.len().saturating_mul(4).clamp(1_024, limit + 1));
    loop {
        let (consumed, produced) = (decompress.total_in(), out.len());
        if out.len() == out.capacity() {
            // Never room for more than one byte past the limit: that byte tells a message that is too long from
            // one that is exactly as long as allowed.
            let remaining = limit + 1 - out.len();
            out.reserve_exact(out.len().max(16 * 1_024).min(remaining));
        }
        let status = decompress
            .decompress_vec(
                &data[consumed as usize..],
                &mut out,
                FlushDecompress::Finish,
            )
            .map_err(|_| EnvelopeError::Malformed)?;
        if out.len() > limit {
            return Err(EnvelopeError::TooLarge);
        }
        if status == Status::StreamEnd {
            return Ok(out);
        }
        // No progress without reaching the end of the stream: the input stopped in the middle of it.
        if decompress.total_in() == consumed && out.len() == produced {
            return Err(EnvelopeError::Malformed);
        }
    }
}

fn seal_inner(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
    compressed: bool,
) -> Envelope {
    let ct = crypto::aead_seal(key, nonce, &aad(scope, sender, kid, compressed), plaintext);
    Envelope {
        v: PROTO,
        f: sender.to_string(),
        k: kid,
        n: crypto::b64u(nonce),
        c: crypto::b64u(&ct),
        z: compressed.then_some(1),
    }
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
    seal_inner(key, scope, sender, kid, nonce, plaintext, false)
}

/// Compresses `plaintext` and seals it as a `z: 1` envelope with an explicit nonce (fixtures and tests).
#[cfg(test)]
pub fn seal_compressed_with_nonce(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Envelope {
    seal_inner(key, scope, sender, kid, nonce, &compress(plaintext), true)
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

/// Decrypts an envelope and, when it carries `z`, inflates the result (at most [`MAX_PLAIN_BYTES`]). A push is never
/// compressed, so `z` there is malformed.
pub fn open(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    env: &Envelope,
) -> Result<Vec<u8>, EnvelopeError> {
    let compressed = match env.z {
        None => false,
        Some(1) if scope != Scope::Push => true,
        Some(_) => return Err(EnvelopeError::Malformed),
    };
    let nonce: [u8; NONCE_LEN] = crypto::b64u_decode(&env.n)
        .and_then(|v| v.try_into().ok())
        .ok_or(EnvelopeError::Malformed)?;
    let ct = crypto::b64u_decode(&env.c).ok_or(EnvelopeError::Malformed)?;
    let plain = crypto::aead_open(key, &nonce, &aad(scope, &env.f, env.k, compressed), &ct)
        .ok_or(EnvelopeError::Auth)?;
    if compressed {
        inflate(&plain, MAX_PLAIN_BYTES)
    } else {
        Ok(plain)
    }
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
    seal_plain_with(key, scope, sender, kid, plain, false)
}

/// Builds the wire bytes for one message, compressed when `compress` is set, the message is at least
/// [`MIN_COMPRESS_BYTES`] long, it goes on a topic and compressing makes it smaller. What the receiver of such an
/// envelope must be able to do was agreed in `hello` (`Hello::compression`).
pub fn seal_plain_with(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    plain: &Plain,
    compress_it: bool,
) -> Vec<u8> {
    let pt = serde_json::to_vec(plain).expect("plain message serializes");
    if compress_it && scope != Scope::Push {
        if let Some(packed) = pack_if_worth_it(&pt) {
            let nonce = crypto::random_bytes::<NONCE_LEN>();
            return encode(&seal_inner(key, scope, sender, kid, &nonce, &packed, true));
        }
    }
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
    fn parse_enforces_size_version_and_the_compression_field() {
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
            parse(br#"{"v":1,"f":"a","k":1}"#),
            Err(EnvelopeError::Malformed)
        );
        assert!(parse(br#"{"v":1,"f":"a","k":1,"n":"x","c":"y"}"#).is_ok());
        // Only `z: 1` is understood; any other value (a later algorithm, a null, a string) is refused.
        let with_z = |z: &str| format!(r#"{{"v":1,"f":"a","k":1,"n":"x","c":"y","z":{z}}}"#);
        assert_eq!(parse(with_z("1").as_bytes()).unwrap().z, Some(1));
        for bad in ["0", "2", "null", "\"1\"", "1.5", "true", "[1]"] {
            assert_eq!(
                parse(with_z(bad).as_bytes()),
                Err(EnvelopeError::Malformed),
                "z = {bad}"
            );
        }
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

    // ------------------------------------------------------------ compression

    const TOPIC: &str = "ah/u/d/p";

    /// A message the way a list stream looks: long and repetitive, so that it shrinks a lot.
    fn list_plain() -> Plain {
        let agents: Vec<Value> = (0..8)
            .map(|i| {
                json!({
                    "key": format!("claude:session-{i}"), "kind": "claude", "kindLabel": "Claude Code",
                    "title": format!("Refactor module {i}"), "state": "working",
                    "project": {"key": "p1", "name": "AskHuman"}
                })
            })
            .collect();
        Plain::new(
            "agents.snapshot",
            json!({"epoch": "0123456789abcdef", "rev": 1, "agents": agents}),
            1_700_000_000_000,
        )
    }

    fn envelope_of(bytes: &[u8]) -> Envelope {
        parse(bytes).expect("a valid envelope")
    }

    #[test]
    fn a_long_list_message_is_compressed_and_opens_to_the_same_message() {
        let plain = list_plain();
        let plain_bytes = serde_json::to_vec(&plain).unwrap().len();
        let packed = seal_plain_with(&key(), Scope::Topic(TOPIC), "macid", 1, &plain, true);
        let env = envelope_of(&packed);
        assert_eq!(env.z, Some(1));
        let uncompressed = seal_plain(&key(), Scope::Topic(TOPIC), "macid", 1, &plain);
        assert!(
            packed.len() * 2 < uncompressed.len(),
            "{} bytes against {} (the plaintext is {plain_bytes})",
            packed.len(),
            uncompressed.len()
        );
        assert_eq!(
            open_plain(&key(), Scope::Topic(TOPIC), &env).unwrap(),
            plain
        );
        // Asked not to compress, the same message goes out as before: no `z`.
        let plain_env = envelope_of(&uncompressed);
        assert_eq!(plain_env.z, None);
        assert!(!String::from_utf8(uncompressed).unwrap().contains("\"z\""));
    }

    #[test]
    fn a_short_message_a_push_and_a_message_that_does_not_shrink_are_not_compressed() {
        let short = Plain::new("agents.delta", json!({"rev": 2}), 1);
        let bytes = seal_plain_with(&key(), Scope::Topic(TOPIC), "macid", 1, &short, true);
        assert_eq!(envelope_of(&bytes).z, None, "below the threshold");

        let long = list_plain();
        let push = seal_plain_with(&key(), Scope::Push, "macid", 1, &long, true);
        assert_eq!(envelope_of(&push).z, None, "a push is never compressed");
    }

    #[test]
    fn compression_is_tried_from_the_threshold_and_kept_only_when_it_is_shorter() {
        let text = vec![b'a'; MIN_COMPRESS_BYTES];
        assert!(pack_if_worth_it(&text).is_some());
        assert!(pack_if_worth_it(&text[..MIN_COMPRESS_BYTES - 1]).is_none());

        // Bytes that look random do not shrink (DEFLATE stores them, plus a little): sent as they are.
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let noise: Vec<u8> = (0..MIN_COMPRESS_BYTES + 100)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect();
        assert!(compress(&noise).len() >= noise.len());
        assert!(pack_if_worth_it(&noise).is_none());
    }

    #[test]
    fn the_compression_marker_is_authenticated() {
        let packed = seal_plain_with(&key(), Scope::Topic(TOPIC), "macid", 1, &list_plain(), true);
        let env = envelope_of(&packed);
        assert_eq!(env.z, Some(1));

        let mut unmarked = env.clone();
        unmarked.z = None;
        assert_eq!(
            open(&key(), Scope::Topic(TOPIC), &unmarked),
            Err(EnvelopeError::Auth),
            "a compressed ciphertext cannot be passed off as a plain one"
        );

        // ...and a plain one cannot be passed off as compressed.
        let plain = seal(&key(), Scope::Topic(TOPIC), "macid", 1, b"{}");
        let mut marked = plain.clone();
        marked.z = Some(1);
        assert_eq!(
            open(&key(), Scope::Topic(TOPIC), &marked),
            Err(EnvelopeError::Auth)
        );
    }

    #[test]
    fn a_push_envelope_that_says_it_is_compressed_is_refused_before_anything_is_decrypted() {
        let mut env = seal(&key(), Scope::Push, "macid", 1, b"x");
        env.z = Some(1);
        assert_eq!(
            open(&key(), Scope::Push, &env),
            Err(EnvelopeError::Malformed)
        );
        // An unknown algorithm number is refused on a topic as well.
        let mut other = seal(&key(), Scope::Topic(TOPIC), "macid", 1, b"x");
        other.z = Some(2);
        assert_eq!(
            open(&key(), Scope::Topic(TOPIC), &other),
            Err(EnvelopeError::Malformed)
        );
    }

    #[test]
    fn a_stream_that_inflates_past_the_limit_or_is_damaged_is_refused() {
        // Zeros shrink to almost nothing: a few KB of DEFLATE stand for more than the limit.
        let bomb = vec![0u8; MAX_PLAIN_BYTES + 1];
        let packed = compress(&bomb);
        assert!(packed.len() < 4_096);
        let env = seal_inner(
            &key(),
            Scope::Topic(TOPIC),
            "macid",
            1,
            &[1u8; NONCE_LEN],
            &packed,
            true,
        );
        assert_eq!(
            open(&key(), Scope::Topic(TOPIC), &env),
            Err(EnvelopeError::TooLarge)
        );

        // Exactly the limit is still fine.
        let limit = vec![b' '; MAX_PLAIN_BYTES];
        let env = seal_inner(
            &key(),
            Scope::Topic(TOPIC),
            "macid",
            1,
            &[2u8; NONCE_LEN],
            &compress(&limit),
            true,
        );
        assert_eq!(open(&key(), Scope::Topic(TOPIC), &env).unwrap(), limit);

        // A truncated stream and bytes that are no DEFLATE at all are malformed (they do authenticate).
        let good = compress(&serde_json::to_vec(&list_plain()).unwrap());
        for broken in [good[..good.len() / 2].to_vec(), vec![0xff; 64]] {
            let env = seal_inner(
                &key(),
                Scope::Topic(TOPIC),
                "macid",
                1,
                &[3u8; NONCE_LEN],
                &broken,
                true,
            );
            assert_eq!(
                open(&key(), Scope::Topic(TOPIC), &env),
                Err(EnvelopeError::Malformed)
            );
        }
    }

    #[test]
    fn the_mac_refuses_a_compressed_envelope_from_a_phone() {
        let packed = seal_plain_with(&key(), Scope::Topic(TOPIC), "phone", 1, &list_plain(), true);
        assert!(parse(&packed).is_ok());
        assert_eq!(parse_from_phone(&packed), Err(EnvelopeError::Malformed));
        let plain = seal_plain(&key(), Scope::Topic(TOPIC), "phone", 1, &list_plain());
        assert!(parse_from_phone(&plain).is_ok());
    }
}
