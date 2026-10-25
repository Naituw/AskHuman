//! `ahconn1.` connection strings: the broker settings of one Mac packed into a single line that
//! can be pasted into another Mac's settings (plan §6). It carries the broker password, so it
//! is treated like one: never logged, only produced on explicit request.

use serde::{Deserialize, Serialize};

use super::crypto;
use crate::config::IosChannelConfig;

pub const PREFIX: &str = "ahconn1.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Wire {
    v: u32,
    h: String,
    p: u16,
    w: u16,
    u: String,
    pw: String,
    r: String,
}

/// Broker settings carried by a connection string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub host: String,
    pub port: u16,
    pub ws_port: u16,
    pub username: String,
    pub password: String,
    pub relay_url: String,
}

impl Connection {
    pub fn from_config(cfg: &IosChannelConfig) -> Self {
        Self {
            host: cfg.host.trim().to_string(),
            port: cfg.port,
            ws_port: cfg.ws_port,
            username: cfg.username.trim().to_string(),
            password: cfg.password.clone(),
            relay_url: cfg.relay_url.trim().to_string(),
        }
    }

    pub fn encode(&self) -> String {
        let wire = Wire {
            v: 1,
            h: self.host.clone(),
            p: self.port,
            w: self.ws_port,
            u: self.username.clone(),
            pw: self.password.clone(),
            r: self.relay_url.clone(),
        };
        let json = serde_json::to_vec(&wire).expect("connection serializes");
        format!("{PREFIX}{}", crypto::b64u(&json))
    }

    pub fn decode(text: &str) -> Result<Self, String> {
        let body = text
            .trim()
            .strip_prefix(PREFIX)
            .ok_or_else(|| "not an AskHuman connection string".to_string())?;
        let bytes = crypto::b64u_decode(body.trim())
            .ok_or_else(|| "the connection string is damaged".to_string())?;
        let wire: Wire = serde_json::from_slice(&bytes)
            .map_err(|_| "the connection string is damaged".to_string())?;
        if wire.v != 1 {
            return Err("the connection string comes from a newer version".to_string());
        }
        if wire.h.trim().is_empty() || !super::wire::is_valid_broker_user(wire.u.trim()) {
            return Err("the connection string is incomplete".to_string());
        }
        Ok(Self {
            host: wire.h.trim().to_string(),
            port: wire.p,
            ws_port: wire.w,
            username: wire.u.trim().to_string(),
            password: wire.pw,
            relay_url: wire.r.trim().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Connection {
        Connection {
            host: "broker.example.com".into(),
            port: 8883,
            ws_port: 8084,
            username: "abc123".into(),
            password: "p@ss/word+==".into(),
            relay_url: "https://relay.example.com/askhuman".into(),
        }
    }

    #[test]
    fn a_connection_string_round_trips() {
        let text = sample().encode();
        assert!(text.starts_with(PREFIX));
        assert!(!text.contains(' ') && !text.contains('\n'));
        assert_eq!(Connection::decode(&text).unwrap(), sample());
        // Pasted text often carries whitespace.
        assert_eq!(
            Connection::decode(&format!("  {text}\n")).unwrap(),
            sample()
        );
    }

    #[test]
    fn broken_strings_are_rejected_with_a_reason() {
        for bad in ["", "hello", "ahconn1.", "ahconn1.%%%", "ahconn1.e30"] {
            assert!(Connection::decode(bad).is_err(), "{bad}");
        }
        let mut incomplete = sample();
        incomplete.username = "has/slash".into();
        assert!(Connection::decode(&incomplete.encode())
            .unwrap_err()
            .contains("incomplete"));
        let future = format!(
            "{PREFIX}{}",
            crypto::b64u(br#"{"v":2,"h":"x","p":1,"w":1,"u":"a","pw":"","r":""}"#)
        );
        assert!(Connection::decode(&future).unwrap_err().contains("newer"));
    }

    #[test]
    fn it_mirrors_the_config_it_came_from() {
        let cfg = IosChannelConfig {
            enabled: true,
            host: " broker.example.com ".into(),
            username: " abc123 ".into(),
            password: "pw".into(),
            ..IosChannelConfig::default()
        };
        let connection = Connection::from_config(&cfg);
        assert_eq!(connection.host, "broker.example.com");
        assert_eq!(connection.username, "abc123");
        assert_eq!((connection.port, connection.ws_port), (8883, 8084));
    }
}
