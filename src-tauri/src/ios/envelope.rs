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
//!
//! Binary envelope (docs/specs/ios-attachment-traffic.md §5.1): the same message without the JSON text and the
//! two base64 layers, `0xA1 | flags | kid (u16 BE) | sender (16 raw bytes) | nonce (12) | ciphertext || tag`. The
//! authenticated data is the same text as for a JSON envelope (the sender is written out as its 26 base32
//! characters), so the format does not change what a ciphertext is bound to. It goes only on a topic and only to a
//! peer that declared `binary` in `hello`/`hello.ack`; pairing messages and pushes stay JSON, and both formats are
//! always accepted.
//!
//! Blob frame (§5.2): a message whose plaintext is `0x01 | header_len (u32 BE) | header | bytes`, where the header is
//! the JSON of a [`Plain`] without the bytes it carries (`asset.chunk`, `upload.chunk`). Blobs are never compressed.

use std::io::Write;

use flate2::write::DeflateEncoder;
use flate2::{Compression, Decompress, FlushDecompress, Status};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::crypto::{self, KEY_LEN, NONCE_LEN};
use super::wire::{
    ASSET_CHUNK_BYTES, BLOB_HEADER_MAX_BYTES, MAX_ENVELOPE_BYTES, MAX_PLAIN_BYTES,
    MIN_COMPRESS_BYTES, PROTO,
};

/// First byte of a binary envelope. A JSON envelope starts with `{`, so the two cannot be mistaken for each other.
pub const BINARY_MAGIC: u8 = 0xA1;
/// Bit 0 of the flags byte of a binary envelope: the plaintext was compressed (raw DEFLATE) before it was encrypted.
/// The other bits are reserved and must be zero.
const BINARY_COMPRESSED: u8 = 0b0000_0001;
/// Magic, flags, `kid` (u16), the sender id (16 raw bytes) and the nonce.
const BINARY_HEADER_LEN: usize = 1 + 1 + 2 + 16 + NONCE_LEN;
/// Length of the ChaCha20-Poly1305 authentication tag at the end of every ciphertext.
const TAG_LEN: usize = 16;
/// The shortest binary envelope: the header and the tag of an empty plaintext.
const BINARY_MIN_LEN: usize = BINARY_HEADER_LEN + TAG_LEN;
/// First byte of the plaintext of a blob frame. A JSON message starts with `{`, so the two cannot be mistaken for each
/// other.
pub const BLOB_MARKER: u8 = 0x01;

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

/// An envelope after parsing, whichever wire format it came in: every field as it is, with nothing left to decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The sender's id as the 26 characters it is written with in the authenticated data.
    pub sender: String,
    pub kid: u32,
    pub nonce: [u8; NONCE_LEN],
    /// The ciphertext followed by its authentication tag.
    pub ciphertext: Vec<u8>,
    /// The plaintext was compressed (raw DEFLATE) before it was encrypted.
    pub compressed: bool,
    /// It came in the binary format.
    pub binary: bool,
}

impl Frame {
    /// The fields of a parsed JSON envelope, decoded. A `z` other than 1, or a nonce or ciphertext that is not base64url
    /// of the right shape, is malformed.
    pub fn from_envelope(env: &Envelope) -> Result<Self, EnvelopeError> {
        let compressed = match env.z {
            None => false,
            Some(1) => true,
            Some(_) => return Err(EnvelopeError::Malformed),
        };
        let nonce: [u8; NONCE_LEN] = crypto::b64u_decode(&env.n)
            .and_then(|v| v.try_into().ok())
            .ok_or(EnvelopeError::Malformed)?;
        let ciphertext = crypto::b64u_decode(&env.c).ok_or(EnvelopeError::Malformed)?;
        Ok(Self {
            sender: env.f.clone(),
            kid: env.k,
            nonce,
            ciphertext,
            compressed,
            binary: false,
        })
    }

    /// The JSON envelope with these fields.
    pub fn to_envelope(&self) -> Envelope {
        Envelope {
            v: PROTO,
            f: self.sender.clone(),
            k: self.kid,
            n: crypto::b64u(&self.nonce),
            c: crypto::b64u(&self.ciphertext),
            z: self.compressed.then_some(1),
        }
    }

    /// The binary envelope with these fields, or `None` when the sender or the `kid` does not fit its header: the
    /// sender must be an id in the exact form [`crypto::new_id`] gives out (it is carried as 16 raw bytes) and the `kid`
    /// must fit 16 bits. A message that cannot go in it is sent in the JSON envelope, which any peer reads.
    pub fn encode_binary(&self) -> Option<Vec<u8>> {
        let sender = crypto::id_to_bytes(&self.sender)?;
        let kid = u16::try_from(self.kid).ok()?;
        let mut out = Vec::with_capacity(BINARY_HEADER_LEN + self.ciphertext.len());
        out.push(BINARY_MAGIC);
        out.push(if self.compressed {
            BINARY_COMPRESSED
        } else {
            0
        });
        out.extend_from_slice(&kid.to_be_bytes());
        out.extend_from_slice(&sender);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.ciphertext);
        Some(out)
    }
}

/// The wire format a message is sealed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The JSON envelope, which every peer reads.
    Json,
    /// The binary envelope, for a peer that said it opens it. A sender or `kid` the header cannot hold goes as JSON
    /// ([`Frame::encode_binary`]), and so does anything in the push scope.
    Binary,
}

/// Parses and size-checks an envelope of either wire format. A binary one is recognised by its first byte; everything
/// else is read as JSON ([`parse`]).
pub fn parse_frame(bytes: &[u8]) -> Result<Frame, EnvelopeError> {
    if bytes.first() == Some(&BINARY_MAGIC) {
        return parse_binary(bytes);
    }
    Frame::from_envelope(&parse(bytes)?)
}

/// Like [`parse_frame`] for an envelope a phone sent: it must not be compressed, exactly as for [`parse_from_phone`].
pub fn parse_frame_from_phone(bytes: &[u8]) -> Result<Frame, EnvelopeError> {
    let frame = parse_frame(bytes)?;
    if frame.compressed {
        return Err(EnvelopeError::Malformed);
    }
    Ok(frame)
}

fn parse_binary(bytes: &[u8]) -> Result<Frame, EnvelopeError> {
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError::TooLarge);
    }
    if bytes.len() < BINARY_MIN_LEN {
        return Err(EnvelopeError::Malformed);
    }
    let flags = bytes[1];
    if flags & !BINARY_COMPRESSED != 0 {
        // A reserved bit: a later format must not be mistaken for this one.
        return Err(EnvelopeError::Malformed);
    }
    Ok(Frame {
        sender: crypto::base32_lower(&bytes[4..20]),
        kid: u32::from(u16::from_be_bytes([bytes[2], bytes[3]])),
        nonce: bytes[20..BINARY_HEADER_LEN]
            .try_into()
            .expect("the slice has the nonce's length"),
        ciphertext: bytes[BINARY_HEADER_LEN..].to_vec(),
        compressed: flags & BINARY_COMPRESSED != 0,
        binary: true,
    })
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

fn seal_frame(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
    compressed: bool,
) -> Frame {
    Frame {
        sender: sender.to_string(),
        kid,
        nonce: *nonce,
        ciphertext: crypto::aead_seal(key, nonce, &aad(scope, sender, kid, compressed), plaintext),
        compressed,
        binary: false,
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
    seal_frame(key, scope, sender, kid, nonce, plaintext, compressed).to_envelope()
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

/// Seals `plaintext` as is into a binary envelope with an explicit nonce (fixtures and tests); `None` when the sender or
/// the `kid` does not fit the header.
#[cfg(test)]
pub fn seal_binary_with_nonce(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Option<Vec<u8>> {
    seal_frame(key, scope, sender, kid, nonce, plaintext, false).encode_binary()
}

/// Compresses `plaintext` and seals it into a binary envelope with its compression bit set, with an explicit nonce
/// (fixtures and tests).
#[cfg(test)]
pub fn seal_compressed_binary_with_nonce(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Option<Vec<u8>> {
    seal_frame(key, scope, sender, kid, nonce, &compress(plaintext), true).encode_binary()
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
    open_frame(key, scope, &Frame::from_envelope(env)?)
}

/// [`open`] for an envelope of either wire format.
pub fn open_frame(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    frame: &Frame,
) -> Result<Vec<u8>, EnvelopeError> {
    if frame.compressed && scope == Scope::Push {
        return Err(EnvelopeError::Malformed);
    }
    let plain = crypto::aead_open(
        key,
        &frame.nonce,
        &aad(scope, &frame.sender, frame.kid, frame.compressed),
        &frame.ciphertext,
    )
    .ok_or(EnvelopeError::Auth)?;
    if frame.compressed {
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
    seal_plain_as(key, scope, sender, kid, plain, compress_it, Format::Json)
}

/// [`seal_plain_with`] in the wire format of the caller's choice. The compression and its conditions are the same in
/// both formats; the binary one only drops the JSON text and the base64 around the same ciphertext.
pub fn seal_plain_as(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    plain: &Plain,
    compress_it: bool,
    format: Format,
) -> Vec<u8> {
    let pt = serde_json::to_vec(plain).expect("plain message serializes");
    let nonce = crypto::random_bytes::<NONCE_LEN>();
    let packed = if compress_it && scope != Scope::Push {
        pack_if_worth_it(&pt)
    } else {
        None
    };
    let frame = match &packed {
        Some(packed) => seal_frame(key, scope, sender, kid, &nonce, packed, true),
        None => seal_frame(key, scope, sender, kid, &nonce, &pt, false),
    };
    encode_frame(&frame, scope, format)
}

fn encode_frame(frame: &Frame, scope: Scope<'_>, format: Format) -> Vec<u8> {
    if format == Format::Binary && scope != Scope::Push {
        if let Some(bytes) = frame.encode_binary() {
            return bytes;
        }
    }
    encode(&frame.to_envelope())
}

pub fn open_plain(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    env: &Envelope,
) -> Result<Plain, EnvelopeError> {
    let pt = open(key, scope, env)?;
    serde_json::from_slice(&pt).map_err(|_| EnvelopeError::Malformed)
}

/// What an envelope of either format holds once it is opened: a message, or a blob frame (a message header and the raw
/// bytes it carries).
#[derive(Debug, Clone, PartialEq)]
pub enum Opened {
    Message(Plain),
    Blob { head: Plain, bytes: Vec<u8> },
}

/// Opens an envelope of either format and reads the message in it. The first byte of the plaintext tells a blob frame
/// (`0x01`) from a JSON message (`{`). A blob is never compressed, so a compressed envelope that inflates to one is
/// refused, and a push never carries one.
pub fn open_message(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    frame: &Frame,
) -> Result<Opened, EnvelopeError> {
    let pt = open_frame(key, scope, frame)?;
    if pt.first() != Some(&BLOB_MARKER) {
        return serde_json::from_slice(&pt)
            .map(Opened::Message)
            .map_err(|_| EnvelopeError::Malformed);
    }
    if frame.compressed || scope == Scope::Push {
        return Err(EnvelopeError::Malformed);
    }
    let (head, bytes) = decode_blob(&pt)?;
    Ok(Opened::Blob { head, bytes })
}

/// The plaintext of a blob frame: the marker, the length of the header (u32, big endian), the header, then the bytes.
/// `head` is a [`Plain`] whose body leaves out the bytes (`data`). Fails when the header is longer than
/// [`BLOB_HEADER_MAX_BYTES`] or the bytes are more than one chunk ([`ASSET_CHUNK_BYTES`]).
pub fn encode_blob(head: &Plain, bytes: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    let header = serde_json::to_vec(head).expect("plain message serializes");
    if header.len() > BLOB_HEADER_MAX_BYTES || bytes.len() > ASSET_CHUNK_BYTES {
        return Err(EnvelopeError::TooLarge);
    }
    let mut out = Vec::with_capacity(1 + 4 + header.len() + bytes.len());
    out.push(BLOB_MARKER);
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(bytes);
    Ok(out)
}

/// The inverse of [`encode_blob`]: the header message and the bytes after it. A plaintext over [`MAX_PLAIN_BYTES`], a
/// length that is over [`BLOB_HEADER_MAX_BYTES`] or past the end of the plaintext, a header that is not a message, or
/// more bytes than one chunk ([`ASSET_CHUNK_BYTES`]) is malformed.
pub fn decode_blob(plaintext: &[u8]) -> Result<(Plain, Vec<u8>), EnvelopeError> {
    if plaintext.first() != Some(&BLOB_MARKER) || plaintext.len() > MAX_PLAIN_BYTES {
        return Err(EnvelopeError::Malformed);
    }
    let len_bytes: [u8; 4] = plaintext
        .get(1..5)
        .and_then(|b| b.try_into().ok())
        .ok_or(EnvelopeError::Malformed)?;
    let header_len = u32::from_be_bytes(len_bytes) as usize;
    if header_len > BLOB_HEADER_MAX_BYTES {
        return Err(EnvelopeError::Malformed);
    }
    let header = plaintext
        .get(5..5 + header_len)
        .ok_or(EnvelopeError::Malformed)?;
    let head: Plain = serde_json::from_slice(header).map_err(|_| EnvelopeError::Malformed)?;
    let bytes = &plaintext[5 + header_len..];
    if bytes.len() > ASSET_CHUNK_BYTES {
        return Err(EnvelopeError::Malformed);
    }
    Ok((head, bytes.to_vec()))
}

/// Builds the wire bytes of a blob frame: a binary envelope around [`encode_blob`], never compressed. `None` when the
/// sender or `kid` does not fit the binary header ([`Frame::encode_binary`]), when the header or the bytes are too long,
/// or for a push; the caller then sends the same message as JSON.
pub fn seal_blob(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    head: &Plain,
    bytes: &[u8],
) -> Option<Vec<u8>> {
    seal_blob_with_nonce(
        key,
        scope,
        sender,
        kid,
        &crypto::random_bytes::<NONCE_LEN>(),
        head,
        bytes,
    )
}

/// [`seal_blob`] with an explicit nonce (fixtures and tests).
pub fn seal_blob_with_nonce(
    key: &[u8; KEY_LEN],
    scope: Scope<'_>,
    sender: &str,
    kid: u32,
    nonce: &[u8; NONCE_LEN],
    head: &Plain,
    bytes: &[u8],
) -> Option<Vec<u8>> {
    if scope == Scope::Push {
        return None;
    }
    let plaintext = encode_blob(head, bytes).ok()?;
    seal_frame(key, scope, sender, kid, nonce, &plaintext, false).encode_binary()
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

    // ------------------------------------------------------------ binary envelope

    /// Ids in the exact form `new_id` gives out (the ones the wire fixtures use).
    const MAC: &str = "mfx2c3rhs6kq7tw4dnb5vypa2e";
    const PHONE: &str = "3qjxfkln6hdpmcvbtzaswe4r5y";

    fn binary_of(plain: &Plain, compress_it: bool) -> Vec<u8> {
        seal_plain_as(
            &key(),
            Scope::Topic(TOPIC),
            MAC,
            1,
            plain,
            compress_it,
            Format::Binary,
        )
    }

    /// Opens wire bytes of either format the way a receiver does.
    fn open_bytes(bytes: &[u8]) -> Result<Opened, EnvelopeError> {
        open_message(&key(), Scope::Topic(TOPIC), &parse_frame(bytes)?)
    }

    /// A binary envelope around an arbitrary plaintext, for the cases a sender would never produce.
    fn binary_around(plaintext: &[u8], compressed: bool) -> Vec<u8> {
        seal_frame(
            &key(),
            Scope::Topic(TOPIC),
            MAC,
            1,
            &[9u8; NONCE_LEN],
            plaintext,
            compressed,
        )
        .encode_binary()
        .expect("the sender fits the header")
    }

    #[test]
    fn neither_marker_can_be_a_json_message() {
        const _: () = assert!(BINARY_MAGIC != b'{' && BLOB_MARKER != b'{');
        // A binary envelope starts with a byte that is not valid UTF-8 at all.
        assert!(std::str::from_utf8(&[BINARY_MAGIC]).is_err());
        let plain = Plain::new("ping", json!({"t": 1}), 1);
        assert_eq!(serde_json::to_vec(&plain).unwrap()[0], b'{');
    }

    #[test]
    fn a_binary_envelope_has_the_documented_layout() {
        let nonce = [5u8; NONCE_LEN];
        let plaintext = b"{\"i\":\"x\"}";
        let frame = seal_frame(
            &key(),
            Scope::Topic(TOPIC),
            MAC,
            7,
            &nonce,
            plaintext,
            false,
        );
        let bytes = frame.encode_binary().unwrap();
        assert_eq!(bytes.len(), 32 + plaintext.len() + 16);
        assert_eq!(bytes[0], 0xA1);
        assert_eq!(bytes[1], 0, "no flags");
        assert_eq!(bytes[2..4], 7u16.to_be_bytes());
        assert_eq!(bytes[4..20], crypto::id_to_bytes(MAC).unwrap());
        assert_eq!(bytes[20..32], nonce);
        assert_eq!(bytes[32..], frame.ciphertext[..]);

        let compressed = seal_frame(&key(), Scope::Topic(TOPIC), MAC, 7, &nonce, plaintext, true);
        assert_eq!(compressed.encode_binary().unwrap()[1], 1, "bit 0: DEFLATE");
    }

    #[test]
    fn a_binary_envelope_is_the_same_ciphertext_as_the_json_one() {
        let nonce = [6u8; NONCE_LEN];
        let frame = seal_frame(
            &key(),
            Scope::Topic(TOPIC),
            PHONE,
            1,
            &nonce,
            b"{\"hello\":1}",
            false,
        );
        let json_form = encode(&frame.to_envelope());
        let binary_form = frame.encode_binary().unwrap();
        let from_json = parse_frame(&json_form).unwrap();
        let from_binary = parse_frame(&binary_form).unwrap();
        assert!(!from_json.binary && from_binary.binary);
        assert_eq!(
            Frame {
                binary: false,
                ..from_binary.clone()
            },
            from_json
        );
        // So both open under the same key and the same authenticated data.
        assert_eq!(
            open_frame(&key(), Scope::Topic(TOPIC), &from_json),
            open_frame(&key(), Scope::Topic(TOPIC), &from_binary)
        );
        assert_eq!(
            open_frame(&key(), Scope::Topic(TOPIC), &from_binary).unwrap(),
            b"{\"hello\":1}"
        );
    }

    #[test]
    fn a_binary_message_opens_to_the_message_that_was_sealed() {
        let plain = Plain::new("answer.submit", json!({"text": "ok", "n": [1, 2, 3]}), 99);
        let bytes = binary_of(&plain, false);
        assert_eq!(bytes[0], BINARY_MAGIC);
        assert_eq!(open_bytes(&bytes), Ok(Opened::Message(plain.clone())));
        let frame = parse_frame(&bytes).unwrap();
        assert_eq!((frame.sender.as_str(), frame.kid), (MAC, 1));
        // The JSON form of the same message opens to the same thing.
        let json_bytes = seal_plain(&key(), Scope::Topic(TOPIC), MAC, 1, &plain);
        assert_eq!(open_bytes(&json_bytes), Ok(Opened::Message(plain)));
    }

    #[test]
    fn the_binary_form_saves_the_json_text_and_the_base64() {
        let plain = Plain::new("request.new", json!({"text": "x".repeat(3000)}), 1);
        let plaintext = serde_json::to_vec(&plain).unwrap().len();
        let binary = binary_of(&plain, false);
        let json_form = seal_plain(&key(), Scope::Topic(TOPIC), MAC, 1, &plain);
        assert_eq!(binary.len(), 32 + plaintext + 16);
        assert!(
            binary.len() * 4 < json_form.len() * 3,
            "{} bytes against {}",
            binary.len(),
            json_form.len()
        );
        // The shortest message still saves the fixed part of the JSON envelope.
        let ping = Plain::new("pong", json!({"t": 1}), 1);
        assert!(
            binary_of(&ping, false).len() + 40
                < seal_plain(&key(), Scope::Topic(TOPIC), MAC, 1, &ping).len()
        );
    }

    #[test]
    fn compression_works_the_same_in_a_binary_envelope() {
        let plain = list_plain();
        let packed = binary_of(&plain, true);
        let frame = parse_frame(&packed).unwrap();
        assert!(frame.binary && frame.compressed);
        assert_eq!(packed[1] & 1, 1);
        assert_eq!(open_bytes(&packed), Ok(Opened::Message(plain.clone())));
        assert!(packed.len() * 2 < binary_of(&plain, false).len());

        // A short message and a push are not compressed, as in the JSON form.
        let short = Plain::new("agents.delta", json!({"rev": 2}), 1);
        assert!(!parse_frame(&binary_of(&short, true)).unwrap().compressed);

        // The flag is part of the authenticated data: clearing it, or setting it on a plain one, fails to open.
        let mut cleared = packed.clone();
        cleared[1] = 0;
        assert_eq!(open_bytes(&cleared), Err(EnvelopeError::Auth));
        let mut set = binary_of(&short, false);
        set[1] = 1;
        assert_eq!(open_bytes(&set), Err(EnvelopeError::Auth));
    }

    #[test]
    fn the_binary_header_is_bound_to_the_ciphertext() {
        let plain = Plain::new("ping", json!({"t": 1}), 1);
        let bytes = binary_of(&plain, false);
        assert!(open_bytes(&bytes).is_ok());
        // kid, sender and nonce (offsets 2..4, 4..20, 20..32) are authenticated, byte by byte.
        for index in 2..32 {
            let mut bad = bytes.clone();
            bad[index] ^= 0x01;
            assert_eq!(open_bytes(&bad), Err(EnvelopeError::Auth), "byte {index}");
        }
        // So is every byte of the ciphertext and the tag.
        for index in 32..bytes.len() {
            let mut bad = bytes.clone();
            bad[index] ^= 0x80;
            assert_eq!(open_bytes(&bad), Err(EnvelopeError::Auth), "byte {index}");
        }
        // And the topic and the domain, exactly as for the JSON form.
        let frame = parse_frame(&bytes).unwrap();
        assert_eq!(
            open_frame(&key(), Scope::Topic("ah/u/d/other"), &frame),
            Err(EnvelopeError::Auth)
        );
        assert_eq!(
            open_frame(&key(), Scope::Push, &frame),
            Err(EnvelopeError::Auth)
        );
    }

    #[test]
    fn a_binary_envelope_is_parsed_strictly() {
        let good = binary_of(&Plain::new("ping", json!({"t": 1}), 1), false);
        assert!(parse_frame(&good).is_ok());
        // Shorter than the header and the tag.
        for len in [1, 2, 31, 32, 47] {
            assert_eq!(
                parse_frame(&good[..len]),
                Err(EnvelopeError::Malformed),
                "{len} bytes"
            );
        }
        // The shortest valid one is an empty plaintext: it parses, and fails to authenticate or to read.
        let empty = binary_around(b"", false);
        assert_eq!(empty.len(), 48);
        assert!(parse_frame(&empty).is_ok());
        assert_eq!(open_bytes(&empty), Err(EnvelopeError::Malformed));
        // Reserved flag bits.
        for flags in [0x02u8, 0x04, 0x80, 0xfe, 0xff] {
            let mut bad = good.clone();
            bad[1] = flags;
            assert_eq!(
                parse_frame(&bad),
                Err(EnvelopeError::Malformed),
                "flags {flags:#x}"
            );
        }
        // Over the broker's limit, whichever format it is in.
        let mut huge = vec![BINARY_MAGIC];
        huge.resize(MAX_ENVELOPE_BYTES + 1, 0);
        assert_eq!(parse_frame(&huge), Err(EnvelopeError::TooLarge));
        // Anything that does not start with the magic is read as JSON.
        assert_eq!(parse_frame(b"\x01\x02"), Err(EnvelopeError::Malformed));
        assert_eq!(parse_frame(b""), Err(EnvelopeError::Malformed));
        let json_form = seal_plain(
            &key(),
            Scope::Topic(TOPIC),
            MAC,
            1,
            &Plain::new("ping", json!({}), 1),
        );
        assert!(!parse_frame(&json_form).unwrap().binary);
    }

    #[test]
    fn the_mac_refuses_a_compressed_binary_envelope_from_a_phone() {
        let packed = binary_of(&list_plain(), true);
        assert!(parse_frame(&packed).is_ok());
        assert_eq!(
            parse_frame_from_phone(&packed),
            Err(EnvelopeError::Malformed)
        );
        let plain = binary_of(&list_plain(), false);
        assert!(parse_frame_from_phone(&plain).is_ok());
        // The same rule for the JSON form.
        let json_packed =
            seal_plain_with(&key(), Scope::Topic(TOPIC), PHONE, 1, &list_plain(), true);
        assert_eq!(
            parse_frame_from_phone(&json_packed),
            Err(EnvelopeError::Malformed)
        );
    }

    #[test]
    fn a_sender_that_does_not_fit_the_binary_header_is_sent_as_json() {
        let plain = Plain::new("ping", json!({"t": 1}), 1);
        let seal = |sender: &str, kid: u32, scope: Scope<'_>| {
            seal_plain_as(&key(), scope, sender, kid, &plain, false, Format::Binary)
        };
        let topic = Scope::Topic(TOPIC);
        assert_eq!(seal(MAC, 1, topic)[0], BINARY_MAGIC);
        assert_eq!(seal(MAC, 65_535, topic)[0], BINARY_MAGIC);
        // A key id over 16 bits, a sender that is not an id, and an id whose padding bits are set.
        let tampered = format!("{}b", "a".repeat(25));
        for (sender, kid) in [(MAC, 65_536), ("macid", 1), (tampered.as_str(), 1)] {
            let bytes = seal(sender, kid, topic);
            assert_eq!(bytes[0], b'{', "{sender} / {kid}");
            let env = parse(&bytes).unwrap();
            assert_eq!(open_plain(&key(), topic, &env).unwrap(), plain);
        }
        // A push is JSON whatever was asked for.
        assert_eq!(seal(MAC, 1, Scope::Push)[0], b'{');
    }

    // ------------------------------------------------------------ blob frames

    fn chunk_head() -> Plain {
        Plain {
            i: "msg-1".into(),
            t: "asset.chunk".into(),
            ts: 1_700_000_000_000,
            b: json!({"assetId": "a1", "index": 2, "total": 3, "size": 12345}),
        }
    }

    fn raw_bytes(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_blob_frame_carries_the_header_and_the_raw_bytes() {
        let bytes = raw_bytes(1000);
        let wire = seal_blob(&key(), Scope::Topic(TOPIC), MAC, 1, &chunk_head(), &bytes).unwrap();
        let frame = parse_frame(&wire).unwrap();
        assert!(frame.binary && !frame.compressed);
        assert_eq!(
            open_message(&key(), Scope::Topic(TOPIC), &frame),
            Ok(Opened::Blob {
                head: chunk_head(),
                bytes: bytes.clone()
            })
        );
        // The plaintext is the marker, the length of the header, the header, then the bytes: no base64 anywhere.
        let plaintext = open_frame(&key(), Scope::Topic(TOPIC), &frame).unwrap();
        let header = serde_json::to_vec(&chunk_head()).unwrap();
        assert_eq!(plaintext[0], 0x01);
        assert_eq!(plaintext[1..5], (header.len() as u32).to_be_bytes());
        assert_eq!(plaintext[5..5 + header.len()], header[..]);
        assert_eq!(plaintext[5 + header.len()..], bytes[..]);
        assert_eq!(wire.len(), 32 + plaintext.len() + 16);
    }

    #[test]
    fn a_full_chunk_takes_far_fewer_bytes_as_a_blob_frame_than_as_json() {
        let bytes = raw_bytes(ASSET_CHUNK_BYTES);
        let wire = seal_blob(&key(), Scope::Topic(TOPIC), MAC, 1, &chunk_head(), &bytes).unwrap();
        // The bytes and a few hundred more: the header, the length, the marker and the envelope.
        assert!(wire.len() < ASSET_CHUNK_BYTES + 700, "{} bytes", wire.len());
        // The JSON form encodes the bytes in base64 and then the whole message in base64 again.
        let mut json_head = chunk_head();
        json_head.b["data"] = json!(crypto::b64u(&bytes));
        let json_form = seal_plain(&key(), Scope::Topic(TOPIC), MAC, 1, &json_head);
        assert!(json_form.len() < MAX_ENVELOPE_BYTES);
        assert!(
            wire.len() * 100 < json_form.len() * 60,
            "{} against {}",
            wire.len(),
            json_form.len()
        );
    }

    #[test]
    fn a_blob_frame_is_bound_to_its_topic_and_cannot_be_a_push() {
        let wire = seal_blob(&key(), Scope::Topic(TOPIC), MAC, 1, &chunk_head(), b"abc").unwrap();
        let frame = parse_frame(&wire).unwrap();
        assert_eq!(
            open_message(&key(), Scope::Topic("ah/u/d/other"), &frame),
            Err(EnvelopeError::Auth)
        );
        // A push never carries a blob: it is not built, and one built by hand is refused.
        assert!(seal_blob(&key(), Scope::Push, MAC, 1, &chunk_head(), b"abc").is_none());
        let plaintext = encode_blob(&chunk_head(), b"abc").unwrap();
        let forged = seal_frame(
            &key(),
            Scope::Push,
            MAC,
            1,
            &[1u8; NONCE_LEN],
            &plaintext,
            false,
        );
        assert_eq!(
            open_message(&key(), Scope::Push, &forged),
            Err(EnvelopeError::Malformed)
        );
    }

    #[test]
    fn a_blob_is_never_read_out_of_a_compressed_envelope() {
        // A blob has exactly one form: raw behind its header, never compressed. A plaintext that inflates to one is
        // refused, whatever the sender claims.
        let plaintext = encode_blob(&chunk_head(), &raw_bytes(100)).unwrap();
        let wire = binary_around(&compress(&plaintext), true);
        assert_eq!(open_bytes(&wire), Err(EnvelopeError::Malformed));
    }

    #[test]
    fn malformed_blob_plaintexts_are_refused() {
        let header = serde_json::to_vec(&chunk_head()).unwrap();
        let blob = |declared: u32, header: &[u8], tail: &[u8]| {
            let mut out = vec![BLOB_MARKER];
            out.extend_from_slice(&declared.to_be_bytes());
            out.extend_from_slice(header);
            out.extend_from_slice(tail);
            out
        };
        let good_len = header.len() as u32;
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("only the marker", vec![BLOB_MARKER]),
            ("a cut length", vec![BLOB_MARKER, 0, 0, 0]),
            ("a length past the end", blob(good_len + 1, &header, b"")),
            ("a length way past the end", blob(u32::MAX, &header, b"abc")),
            (
                "a header over the limit",
                blob(
                    BLOB_HEADER_MAX_BYTES as u32 + 1,
                    &vec![b' '; BLOB_HEADER_MAX_BYTES + 1],
                    b"",
                ),
            ),
            ("a header that is not JSON", blob(5, b"nope!", b"abc")),
            ("a header that is not a message", blob(2, b"{}", b"abc")),
            ("a header that is an array", blob(2, b"[]", b"abc")),
            (
                "more bytes than a chunk",
                blob(good_len, &header, &raw_bytes(ASSET_CHUNK_BYTES + 1)),
            ),
            (
                "a plaintext over the limit",
                blob(good_len, &header, &vec![0u8; MAX_PLAIN_BYTES]),
            ),
            ("a plaintext that is neither", b"\x02{}".to_vec()),
            ("a plaintext that is not JSON", b"[1,2]".to_vec()),
        ];
        for (name, plaintext) in cases {
            let wire = binary_around(&plaintext, false);
            assert_eq!(open_bytes(&wire), Err(EnvelopeError::Malformed), "{name}");
        }
        // The same bytes with a header that is right are fine, with and without bytes behind it.
        for tail in [&b""[..], b"x", &raw_bytes(ASSET_CHUNK_BYTES)] {
            let wire = binary_around(&blob(good_len, &header, tail), false);
            assert!(matches!(
                open_bytes(&wire),
                Ok(Opened::Blob { bytes, .. }) if bytes == tail
            ));
        }
    }

    #[test]
    fn a_blob_cannot_be_built_with_an_oversized_header_or_chunk() {
        let mut long_head = chunk_head();
        long_head.b["pad"] = json!("x".repeat(BLOB_HEADER_MAX_BYTES));
        assert_eq!(
            encode_blob(&long_head, b"abc"),
            Err(EnvelopeError::TooLarge)
        );
        assert!(seal_blob(&key(), Scope::Topic(TOPIC), MAC, 1, &long_head, b"abc").is_none());
        assert_eq!(
            encode_blob(&chunk_head(), &raw_bytes(ASSET_CHUNK_BYTES + 1)),
            Err(EnvelopeError::TooLarge)
        );
        assert!(encode_blob(&chunk_head(), &raw_bytes(ASSET_CHUNK_BYTES)).is_ok());
        // A sender the binary header cannot hold has no blob form: the caller sends the JSON message.
        assert!(seal_blob(
            &key(),
            Scope::Topic(TOPIC),
            "macid",
            1,
            &chunk_head(),
            b"abc"
        )
        .is_none());
        assert!(seal_blob(
            &key(),
            Scope::Topic(TOPIC),
            MAC,
            70_000,
            &chunk_head(),
            b"abc"
        )
        .is_none());
    }
}
