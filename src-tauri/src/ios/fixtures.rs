//! Cross-language fixtures of wire protocol v1 (`ios/Fixtures/wire-v1/`).
//!
//! `crypto.json` is generated here from fixed key material (`UPDATE_WIRE_FIXTURES=1 cargo test
//! ios::fixtures` rewrites it); the Swift package verifies the same file. `messages.json` is
//! hand-written and checked by round-tripping every sample through the typed models.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::crypto::{self, KeyPair, PairContext};
use super::envelope::{self, Plain, Scope};
use super::wire::{self, KID_PAIR, KID_TICKET};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ios/Fixtures/wire-v1")
        .join(name)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn bytes32(seed: u8) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = seed
            .wrapping_add(i as u8)
            .wrapping_mul(7)
            .wrapping_add(seed);
    }
    out
}

fn nonce12(seed: u8) -> [u8; 12] {
    let mut out = [0u8; 12];
    for (i, b) in out.iter_mut().enumerate() {
        *b = seed.wrapping_add(i as u8);
    }
    out
}

fn generate_crypto_fixture() -> Value {
    let mac = KeyPair::from_secret_bytes(bytes32(0x11));
    let phone = KeyPair::from_secret_bytes(bytes32(0x42));
    let shared = mac.diffie_hellman(&phone.public).expect("contributory");
    assert_eq!(Some(shared), phone.diffie_hellman(&mac.public));

    let ticket_secret = bytes32(0x77);
    let mac_id = "mfx2c3rhs6kq7tw4dnb5vypa2e";
    let phone_id = "3qjxfkln6hdpmcvbtzaswe4r5y";
    let ticket_id = "ticketx2c3rhs6kq7tw4dnb5vy";
    assert!(crypto::is_valid_id(mac_id) && crypto::is_valid_id(phone_id));
    assert!(crypto::is_valid_id(ticket_id));

    let k_t = crypto::ticket_key(&ticket_secret, ticket_id);
    let ctx = PairContext {
        mac_id,
        phone_id,
        ticket_id,
        ticket_secret: &ticket_secret,
        mac_public: &mac.public,
        phone_public: &phone.public,
    };
    let keys = crypto::derive_pair_keys(&shared, &ctx);

    let user = "askhuman01";
    let down = wire::topic_down(user, phone_id);
    let up = wire::topic_up(user, mac_id);
    let pair_topic = wire::topic_pair(user, ticket_id);

    let sample = |topic_scope: Scope<'_>,
                  name: &str,
                  key: &[u8; 32],
                  from: &str,
                  kid: u32,
                  nonce: &[u8; 12],
                  plain: &Plain| {
        let plaintext = serde_json::to_string(plain).unwrap();
        let env =
            envelope::seal_with_nonce(key, topic_scope, from, kid, nonce, plaintext.as_bytes());
        let (scope, topic) = match topic_scope {
            Scope::Topic(t) => ("topic", Value::String(t.to_string())),
            Scope::Push => ("push", Value::Null),
        };
        json!({
            "name": name,
            "scope": scope,
            "topic": topic,
            "from": from,
            "kid": kid,
            "key": hex(key),
            "nonce": hex(nonce),
            "plaintext": plaintext,
            "envelope": String::from_utf8(envelope::encode(&env)).unwrap(),
        })
    };

    let ask = Plain {
        i: crypto::b64u(&[3u8; 16]),
        t: "request.new".into(),
        ts: 1_792_900_000_000,
        b: json!({"rev": 8, "full": true, "note": "unicode \u{4f60}\u{597d} \u{1f600}"}),
    };
    let hello = Plain {
        i: crypto::b64u(&[4u8; 16]),
        t: "hello".into(),
        ts: 1_792_900_000_500,
        b: json!({"nonce": "q3ZkL1m0Xw4", "proto": 1}),
    };
    let pair_req = Plain {
        i: crypto::b64u(&[5u8; 16]),
        t: "pair.request".into(),
        ts: 1_792_899_999_000,
        b: json!({"phoneId": phone_id, "proto": 1}),
    };
    let push = Plain {
        i: crypto::b64u(&[6u8; 16]),
        t: "push".into(),
        ts: 1_792_900_001_000,
        b: json!({"k": "ask", "mac": "Home Mac", "text": "Which approach?"}),
    };
    // A list stream as the Mac compresses it: several alike Agents, one title in a script that is not ASCII, so
    // that a receiver proves it keeps every byte of UTF-8 across the inflate.
    let agents: Vec<Value> = (0..6)
        .map(|n| {
            json!({
                "key": format!("claude:session-{n}"),
                "kind": "claude",
                "kindLabel": "Claude Code",
                "sessionId": format!("session-{n}"),
                "title": if n == 3 { "\u{91cd}\u{6784}\u{6a21}\u{5757} 3".to_string() } else { format!("Refactor module {n}") },
                "project": {"key": "/Users/me/project", "name": "project"},
                "state": "working",
                "startedAt": 1_792_899_000_000i64 + n,
                "lastActivityAt": 1_792_900_000_000i64 + n,
                "activeElapsedSecs": 120,
                "turnSteps": 7,
                "currentTool": "Shell: cargo test",
                "canInterject": {"ok": true},
                "followed": false,
            })
        })
        .collect();
    let snapshot = Plain {
        i: crypto::b64u(&[7u8; 16]),
        t: "agents.snapshot".into(),
        ts: 1_792_900_002_000,
        b: json!({"epoch": "fixtureepoch0001", "rev": 3, "agents": agents}),
    };
    let compressed_sample = |name: &str, nonce: &[u8; 12], plain: &Plain| {
        let plaintext = serde_json::to_string(plain).unwrap();
        let env = envelope::seal_compressed_with_nonce(
            &keys.m2p,
            Scope::Topic(&down),
            mac_id,
            KID_PAIR,
            nonce,
            plaintext.as_bytes(),
        );
        assert_eq!(env.z, Some(1));
        json!({
            "name": name,
            "scope": "topic",
            "topic": down,
            "from": mac_id,
            "kid": KID_PAIR,
            "key": hex(&keys.m2p),
            "nonce": hex(nonce),
            "plaintext": plaintext,
            "envelope": String::from_utf8(envelope::encode(&env)).unwrap(),
        })
    };

    // Binary envelopes (docs/specs/ios-attachment-traffic.md §5.1): the same messages and keys, in the other format.
    let scope_fields = |scope: Scope<'_>| match scope {
        Scope::Topic(t) => ("topic", Value::String(t.to_string())),
        Scope::Push => ("push", Value::Null),
    };
    let binary_sample = |name: &str,
                         scope: Scope<'_>,
                         key: &[u8; 32],
                         from: &str,
                         kid: u32,
                         nonce: &[u8; 12],
                         plain: &Plain| {
        let plaintext = serde_json::to_string(plain).unwrap();
        let frame =
            envelope::seal_binary_with_nonce(key, scope, from, kid, nonce, plaintext.as_bytes())
                .expect("the fixture sender fits the binary header");
        let (scope_name, topic) = scope_fields(scope);
        json!({
            "name": name,
            "scope": scope_name,
            "topic": topic,
            "from": from,
            "kid": kid,
            "key": hex(key),
            "nonce": hex(nonce),
            "plaintext": plaintext,
            "envelope": hex(&frame),
        })
    };
    let compressed_binary_sample = |name: &str, nonce: &[u8; 12], plain: &Plain| {
        let plaintext = serde_json::to_string(plain).unwrap();
        let frame = envelope::seal_compressed_binary_with_nonce(
            &keys.m2p,
            Scope::Topic(&down),
            mac_id,
            KID_PAIR,
            nonce,
            plaintext.as_bytes(),
        )
        .expect("the fixture sender fits the binary header");
        assert_eq!(frame[1] & 1, 1);
        json!({
            "name": name,
            "scope": "topic",
            "topic": down,
            "from": mac_id,
            "kid": KID_PAIR,
            "key": hex(&keys.m2p),
            "nonce": hex(nonce),
            "plaintext": plaintext,
            "envelope": hex(&frame),
        })
    };

    // Blob frames (§5.2): a message header and the raw bytes behind it. The bytes include every byte value that a text
    // layer could mangle (a zero, a `{`, a byte over 0x7f).
    let blob_sample = |name: &str,
                       key: &[u8; 32],
                       from: &str,
                       topic: &str,
                       nonce: &[u8; 12],
                       head: &Plain,
                       bytes: &[u8]| {
        let frame = envelope::seal_blob_with_nonce(
            key,
            Scope::Topic(topic),
            from,
            KID_PAIR,
            nonce,
            head,
            bytes,
        )
        .expect("the fixture sender fits the binary header");
        json!({
            "name": name,
            "scope": "topic",
            "topic": topic,
            "from": from,
            "kid": KID_PAIR,
            "key": hex(key),
            "nonce": hex(nonce),
            "head": serde_json::to_string(head).unwrap(),
            "bytes": hex(bytes),
            "frame": hex(&frame),
        })
    };
    let chunk_bytes: Vec<u8> = (0..300u32).map(|n| (n * 7 + 3) as u8).collect();
    let mut upload_bytes: Vec<u8> = vec![0x00, 0x7b, 0x01, 0xff, 0x80];
    upload_bytes.extend((0..59u8).map(|n| n.wrapping_mul(13)));
    let asset_chunk = Plain {
        i: crypto::b64u(&[8u8; 16]),
        t: "asset.chunk".into(),
        ts: 1_792_900_003_000,
        b: json!({
            "assetId": "ab2cdef3ghijk4lmnopq5rstuv", "index": 1, "total": 3, "size": 500_000,
            "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        }),
    };
    let upload_chunk = Plain {
        i: crypto::b64u(&[9u8; 16]),
        t: "upload.chunk".into(),
        ts: 1_792_900_004_000,
        b: json!({"uploadId": "bc3defg4hijkl5mnopqr6stuvw", "index": 0}),
    };

    // Frames a receiver must refuse. The first group breaks a good binary envelope in every place a parser looks at;
    // the second authenticates (the Mac's own key and nonce), and is refused for what is inside.
    let good = envelope::seal_binary_with_nonce(
        &keys.m2p,
        Scope::Topic(&down),
        mac_id,
        KID_PAIR,
        &nonce12(11),
        serde_json::to_string(&ask).unwrap().as_bytes(),
    )
    .unwrap();
    let refused = |name: &str, topic: &str, frame: &[u8]| {
        json!({
            "name": name,
            "scope": "topic",
            "topic": topic,
            "key": hex(&keys.m2p),
            "frame": hex(frame),
        })
    };
    let changed = |index: usize, to: Option<u8>| {
        let mut frame = good.clone();
        frame[index] = to.unwrap_or(frame[index] ^ 0x01);
        frame
    };
    let blob_plaintext = |declared: u32, header: &[u8], tail: &[u8]| {
        let mut out = vec![0x01u8];
        out.extend_from_slice(&declared.to_be_bytes());
        out.extend_from_slice(header);
        out.extend_from_slice(tail);
        out
    };
    let head_text = serde_json::to_string(&asset_chunk).unwrap();
    let authentic = |name: &str, nonce: &[u8; 12], plaintext: &[u8]| {
        let frame = envelope::seal_binary_with_nonce(
            &keys.m2p,
            Scope::Topic(&down),
            mac_id,
            KID_PAIR,
            nonce,
            plaintext,
        )
        .unwrap();
        refused(name, &down, &frame)
    };
    let other_topic = "ah/askhuman01/d/other";
    let malformed = vec![
        refused("empty", &down, &[]),
        refused("the magic alone", &down, &[0xA1]),
        refused("cut inside the nonce", &down, &good[..25]),
        refused("the header without a tag", &down, &good[..47]),
        refused("another magic", &down, &changed(0, Some(0xA2))),
        refused("a reserved flag (bit 1)", &down, &changed(1, Some(0x02))),
        refused("a reserved flag (bit 7)", &down, &changed(1, Some(0x80))),
        refused("every reserved flag", &down, &changed(1, Some(0xFE))),
        refused(
            "the compression bit with a reserved one",
            &down,
            &changed(1, Some(0x03)),
        ),
        refused(
            "the compression bit on a plain ciphertext",
            &down,
            &changed(1, Some(0x01)),
        ),
        refused("a changed key id", &down, &changed(3, None)),
        refused("a changed sender", &down, &changed(10, None)),
        refused("a changed nonce", &down, &changed(25, None)),
        refused("a changed ciphertext", &down, &changed(40, None)),
        refused("a changed tag", &down, &changed(good.len() - 1, None)),
        refused("opened on another topic", other_topic, &good),
        authentic("a blob with only the marker", &nonce12(12), &[0x01]),
        authentic("a blob with a cut length", &nonce12(13), &[0x01, 0, 0, 0]),
        authentic(
            "a blob whose header length is past the end",
            &nonce12(14),
            &blob_plaintext(head_text.len() as u32 + 1, head_text.as_bytes(), b""),
        ),
        authentic(
            "a blob whose header length is over the limit",
            &nonce12(15),
            &blob_plaintext(4097, b"{}", &[7u8; 20]),
        ),
        authentic(
            "a blob whose header is not JSON",
            &nonce12(16),
            &blob_plaintext(5, b"nope!", b"abc"),
        ),
        authentic(
            "a blob whose header is not a message",
            &nonce12(17),
            &blob_plaintext(2, b"{}", b"abc"),
        ),
        authentic(
            "a plaintext that is neither JSON nor a blob",
            &nonce12(18),
            &[0x02, b'{', b'}'],
        ),
    ];

    json!({
        "version": 1,
        "base64url": [
            {"hex": "", "text": ""},
            {"hex": "00", "text": "AA"},
            {"hex": "000102", "text": "AAEC"},
            {"hex": "fbfffe", "text": "-__-"},
            {"hex": "0102030405", "text": "AQIDBAU"},
        ],
        "base32": [
            {"hex": "", "text": ""},
            {"hex": "66", "text": "my"},
            {"hex": "666f6f626172", "text": "mzxw6ytboi"},
            {"hex": hex(&[0xffu8; 16]), "text": crypto::base32_lower(&[0xffu8; 16])},
        ],
        "x25519": {
            "macPrivate": hex(&bytes32(0x11)),
            "macPublic": hex(&mac.public),
            "phonePrivate": hex(&bytes32(0x42)),
            "phonePublic": hex(&phone.public),
            "shared": hex(&shared),
        },
        "ticket": {
            "ticketSecret": hex(&ticket_secret),
            "ticketId": ticket_id,
            "key": hex(&k_t),
        },
        "pair": {
            "macId": mac_id,
            "phoneId": phone_id,
            "ticketId": ticket_id,
            "ticketSecret": hex(&ticket_secret),
            "macPublic": hex(&mac.public),
            "phonePublic": hex(&phone.public),
            "shared": hex(&shared),
            "keys": {"m2p": hex(&keys.m2p), "p2m": hex(&keys.p2m), "sas": hex(&keys.sas)},
            "sasCode": crypto::sas_code(&keys.sas),
        },
        "envelopes": [
            sample(Scope::Topic(&down), "mac to phone message", &keys.m2p, mac_id, KID_PAIR, &nonce12(1), &ask),
            sample(Scope::Topic(&up), "phone to mac message", &keys.p2m, phone_id, KID_PAIR, &nonce12(2), &hello),
            sample(Scope::Topic(&pair_topic), "pairing channel message", &k_t, phone_id, KID_TICKET, &nonce12(3), &pair_req),
            sample(Scope::Push, "push payload", &keys.m2p, mac_id, KID_PAIR, &nonce12(4), &push),
        ],
        // Not compared byte for byte (see `without_compressed`): what a DEFLATE encoder emits is not part of the
        // protocol, and every receiver only has to open it.
        "compressedEnvelopes": [
            compressed_sample("compressed list stream (mac to phone)", &nonce12(5), &snapshot),
        ],
        // The binary envelope (`0xA1 | flags | kid | sender | nonce | ciphertext || tag`, hex here), the same
        // messages and keys as `envelopes`; compared byte for byte, in both directions.
        "binaryEnvelopes": [
            binary_sample("mac to phone message (binary)", Scope::Topic(&down), &keys.m2p, mac_id, KID_PAIR, &nonce12(7), &ask),
            binary_sample("phone to mac message (binary)", Scope::Topic(&up), &keys.p2m, phone_id, KID_PAIR, &nonce12(8), &hello),
            // A key id over 255, so that the two bytes of it are not the same byte.
            binary_sample("key id above 255", Scope::Topic(&down), &keys.m2p, mac_id, 258, &nonce12(9), &ask),
        ],
        // Opened, not compared (see `without_compressed`).
        "compressedBinaryEnvelopes": [
            compressed_binary_sample("compressed list stream (mac to phone, binary)", &nonce12(10), &snapshot),
        ],
        // Blob frames: the plaintext is `0x01 | header length | header | bytes`.
        "blobFrames": [
            blob_sample("asset chunk (mac to phone)", &keys.m2p, mac_id, &down, &nonce12(19), &asset_chunk, &chunk_bytes),
            blob_sample("upload chunk (phone to mac)", &keys.p2m, phone_id, &up, &nonce12(20), &upload_chunk, &upload_bytes),
        ],
        // Every one of these is refused: by the parser, by the authentication or by the reading of the blob.
        "malformedFrames": malformed,
    })
}

/// The fixture without the compressed samples. A compressed envelope is checked by opening it, not by regenerating
/// it, so that a new version of the compression library cannot make the file look stale.
fn without_compressed(mut fixture: Value) -> Value {
    if let Some(object) = fixture.as_object_mut() {
        object.remove("compressedEnvelopes");
        object.remove("compressedBinaryEnvelopes");
    }
    fixture
}

fn read_crypto_fixture() -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(fixture_path("crypto.json"))
            .expect("crypto fixture exists (run with UPDATE_WIRE_FIXTURES=1)"),
    )
    .expect("crypto fixture is valid JSON")
}

#[test]
fn crypto_fixture_matches_the_implementation() {
    let generated = generate_crypto_fixture();
    let path = fixture_path("crypto.json");
    if std::env::var_os("UPDATE_WIRE_FIXTURES").is_some() {
        let mut text = serde_json::to_string_pretty(&generated).unwrap();
        text.push('\n');
        std::fs::write(&path, text).expect("write crypto fixture");
    }
    let on_disk = read_crypto_fixture();
    assert_eq!(
        without_compressed(on_disk),
        without_compressed(generated),
        "ios/Fixtures/wire-v1/crypto.json is stale; run UPDATE_WIRE_FIXTURES=1 cargo test ios::fixtures"
    );
}

#[test]
fn crypto_fixture_envelopes_decrypt_and_reject_a_wrong_scope() {
    let fixture = generate_crypto_fixture();
    for item in fixture["envelopes"].as_array().unwrap() {
        let key: [u8; 32] = decode_hex(item["key"].as_str().unwrap())
            .try_into()
            .unwrap();
        let env = envelope::parse(item["envelope"].as_str().unwrap().as_bytes()).unwrap();
        let scope = match item["topic"].as_str() {
            Some(topic) => Scope::Topic(topic),
            None => Scope::Push,
        };
        let plain = envelope::open(&key, scope, &env).unwrap();
        assert_eq!(plain, item["plaintext"].as_str().unwrap().as_bytes());
        let wrong = match scope {
            Scope::Topic(_) => Scope::Push,
            Scope::Push => Scope::Topic("ah/askhuman01/d/other"),
        };
        assert!(envelope::open(&key, wrong, &env).is_err());
    }
}

/// The compressed samples of the file (the ones the Swift package opens too) open to their plaintext, and nothing
/// about them is taken on trust: the marker is authenticated, the scope still binds, and the plaintext is much
/// shorter on the wire than it is.
#[test]
fn compressed_fixture_envelopes_open_to_their_plaintext_and_cannot_be_altered() {
    let fixture = read_crypto_fixture();
    let samples = fixture["compressedEnvelopes"]
        .as_array()
        .expect("compressedEnvelopes (run with UPDATE_WIRE_FIXTURES=1)");
    assert!(!samples.is_empty());
    for item in samples {
        let key: [u8; 32] = decode_hex(item["key"].as_str().unwrap())
            .try_into()
            .unwrap();
        let topic = item["topic"].as_str().unwrap();
        let text = item["envelope"].as_str().unwrap();
        let env = envelope::parse(text.as_bytes()).unwrap();
        assert_eq!(env.z, Some(1));
        let plaintext = item["plaintext"].as_str().unwrap();
        assert_eq!(
            envelope::open(&key, Scope::Topic(topic), &env).unwrap(),
            plaintext.as_bytes()
        );
        assert!(
            text.len() * 2 < plaintext.len(),
            "{} bytes on the wire for {} of plaintext",
            text.len(),
            plaintext.len()
        );

        // The marker is part of what is authenticated: without it the same ciphertext does not open.
        let mut unmarked = env.clone();
        unmarked.z = None;
        assert!(envelope::open(&key, Scope::Topic(topic), &unmarked).is_err());
        // Neither does it open on another topic, nor as a push (which is never compressed).
        assert!(envelope::open(&key, Scope::Topic("ah/askhuman01/d/other"), &env).is_err());
        assert!(envelope::open(&key, Scope::Push, &env).is_err());
    }
}

fn scope_of(item: &Value) -> Scope<'_> {
    match item["topic"].as_str() {
        Some(topic) => Scope::Topic(topic),
        None => Scope::Push,
    }
}

fn key_of(item: &Value) -> [u8; 32] {
    decode_hex(item["key"].as_str().unwrap())
        .try_into()
        .unwrap()
}

/// The binary samples of the file are what the implementation builds, and open to their messages. They are bound to
/// their sender, key id and topic exactly as the JSON envelopes are, and the same ciphertext opens from either format.
#[test]
fn binary_fixture_envelopes_open_to_their_plaintext_and_carry_the_documented_header() {
    let fixture = generate_crypto_fixture();
    let samples = fixture["binaryEnvelopes"].as_array().unwrap();
    assert!(samples.len() >= 3);
    for item in samples {
        let name = item["name"].as_str().unwrap();
        let wire_bytes = decode_hex(item["envelope"].as_str().unwrap());
        let frame = envelope::parse_frame(&wire_bytes).unwrap();
        assert!(frame.binary && !frame.compressed, "{name}");
        assert_eq!(wire_bytes[0], 0xA1, "{name}");
        assert_eq!(frame.sender, item["from"].as_str().unwrap(), "{name}");
        assert_eq!(
            u64::from(frame.kid),
            item["kid"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            frame.nonce[..],
            decode_hex(item["nonce"].as_str().unwrap())[..]
        );
        let key = key_of(item);
        let scope = scope_of(item);
        let plaintext = item["plaintext"].as_str().unwrap().as_bytes();
        assert_eq!(
            envelope::open_frame(&key, scope, &frame).unwrap(),
            plaintext,
            "{name}"
        );
        // The same fields as a JSON envelope open to the same plaintext: the format changes nothing that is signed.
        let as_json = envelope::encode(&frame.to_envelope());
        let reparsed = envelope::parse_frame(&as_json).unwrap();
        assert!(!reparsed.binary);
        assert_eq!(
            envelope::open_frame(&key, scope, &reparsed).unwrap(),
            plaintext,
            "{name}"
        );
        // And the topic and the domain still bind.
        let wrong = match scope {
            Scope::Topic(_) => Scope::Push,
            Scope::Push => Scope::Topic("ah/askhuman01/d/other"),
        };
        assert!(envelope::open_frame(&key, wrong, &frame).is_err(), "{name}");
    }
}

#[test]
fn compressed_binary_fixture_envelopes_open_and_the_bit_is_authenticated() {
    let fixture = read_crypto_fixture();
    let samples = fixture["compressedBinaryEnvelopes"]
        .as_array()
        .expect("compressedBinaryEnvelopes (run with UPDATE_WIRE_FIXTURES=1)");
    assert!(!samples.is_empty());
    for item in samples {
        let wire_bytes = decode_hex(item["envelope"].as_str().unwrap());
        let frame = envelope::parse_frame(&wire_bytes).unwrap();
        assert!(frame.binary && frame.compressed);
        let key = key_of(item);
        let scope = scope_of(item);
        let plaintext = item["plaintext"].as_str().unwrap();
        assert_eq!(
            envelope::open_frame(&key, scope, &frame).unwrap(),
            plaintext.as_bytes()
        );
        assert!(
            wire_bytes.len() * 2 < plaintext.len(),
            "{} bytes on the wire for {} of plaintext",
            wire_bytes.len(),
            plaintext.len()
        );
        let mut cleared = wire_bytes.clone();
        cleared[1] = 0;
        let unmarked = envelope::parse_frame(&cleared).unwrap();
        assert!(envelope::open_frame(&key, scope, &unmarked).is_err());
    }
}

#[test]
fn blob_fixture_frames_open_to_their_header_and_bytes_and_bind_their_topic() {
    let fixture = generate_crypto_fixture();
    let samples = fixture["blobFrames"].as_array().unwrap();
    assert!(samples.len() >= 2);
    for item in samples {
        let name = item["name"].as_str().unwrap();
        let wire_bytes = decode_hex(item["frame"].as_str().unwrap());
        let frame = envelope::parse_frame(&wire_bytes).unwrap();
        assert!(frame.binary && !frame.compressed, "{name}");
        let key = key_of(item);
        let scope = scope_of(item);
        let head: Plain = serde_json::from_str(item["head"].as_str().unwrap()).unwrap();
        let bytes = decode_hex(item["bytes"].as_str().unwrap());
        assert_eq!(
            envelope::open_message(&key, scope, &frame),
            Ok(envelope::Opened::Blob {
                head,
                bytes: bytes.clone()
            }),
            "{name}"
        );
        // Nothing of the file is in text form: the bytes are in the frame as they are.
        let plaintext = envelope::open_frame(&key, scope, &frame).unwrap();
        assert!(plaintext.ends_with(&bytes), "{name}");
        assert!(
            envelope::open_message(&key, Scope::Topic("ah/askhuman01/d/other"), &frame).is_err(),
            "{name}"
        );
    }
}

#[test]
fn malformed_fixture_frames_are_refused() {
    let fixture = generate_crypto_fixture();
    let cases = fixture["malformedFrames"].as_array().unwrap();
    assert!(cases.len() >= 20);
    for item in cases {
        let name = item["name"].as_str().unwrap();
        let wire_bytes = decode_hex(item["frame"].as_str().unwrap());
        let key = key_of(item);
        let scope = scope_of(item);
        let opened = envelope::parse_frame(&wire_bytes)
            .and_then(|frame| envelope::open_message(&key, scope, &frame));
        assert!(opened.is_err(), "{name} was accepted: {opened:?}");
    }
}

fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn messages_fixture_round_trips_through_the_typed_models() {
    let text =
        std::fs::read_to_string(fixture_path("messages.json")).expect("messages fixture exists");
    let root: Value = serde_json::from_str(&text).expect("messages fixture is valid JSON");
    let mut seen = BTreeSet::new();
    for sample in root["samples"].as_array().expect("samples array") {
        let ty = sample["type"].as_str().expect("sample type");
        let name = sample["name"].as_str().unwrap_or("");
        let body = &sample["body"];
        let normalized = wire::normalize(ty, body)
            .unwrap_or_else(|| panic!("fixture uses a type missing from the registry: {ty}"))
            .unwrap_or_else(|e| panic!("{ty} {name}: body does not decode: {e}"));
        assert_eq!(&normalized, body, "{ty} {name}: re-encoded body differs");
        seen.insert(ty.to_string());
    }
    let expected: BTreeSet<String> = wire::ALL_TYPES.iter().map(|s| s.to_string()).collect();
    let missing: Vec<_> = expected.difference(&seen).collect();
    assert!(missing.is_empty(), "no fixture sample for: {missing:?}");
    assert_eq!(
        expected.len(),
        wire::ALL_TYPES.len(),
        "duplicate message type in the registry"
    );
}

#[test]
fn unknown_message_types_and_bad_bodies_are_rejected() {
    assert!(wire::normalize("nope", &json!({})).is_none());
    assert!(
        wire::normalize("answer.ack", &json!({"status": "accepted"}))
            .unwrap()
            .is_err()
    );
    assert!(wire::normalize("ping", &json!({"t": "x"}))
        .unwrap()
        .is_err());
}
