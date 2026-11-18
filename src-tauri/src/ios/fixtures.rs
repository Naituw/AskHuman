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
    })
}

/// The fixture without the compressed samples. A compressed envelope is checked by opening it, not by regenerating
/// it, so that a new version of the compression library cannot make the file look stale.
fn without_compressed(mut fixture: Value) -> Value {
    if let Some(object) = fixture.as_object_mut() {
        object.remove("compressedEnvelopes");
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
