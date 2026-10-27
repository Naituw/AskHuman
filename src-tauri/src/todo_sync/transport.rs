//! What the sync core needs from "the cloud" (plan §3.3, §4.1).
//!
//! The core never talks CloudKit. It talks to a [`Transport`]: production wraps the helper app's RPC
//! (`helper_client`), tests use [`mem::MemTransport`], an in-memory zone with injectable faults. Tokens
//! and change tags are opaque strings; payloads are the raw bytes of one encrypted field.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// No connectivity, or the helper cannot reach iCloud: try again later.
    Offline,
    /// The server asked us to slow down.
    RateLimited {
        retry_after_ms: u64,
    },
    NotAuthenticated,
    /// The record zone does not exist (cleared from another Mac, or the account was reset).
    ZoneNotFound,
    /// The change token is too old to continue from; start over from nothing.
    TokenExpired,
    Quota,
    /// The call did not finish in the time it was given (a deadline or the per-call limit). The
    /// request may still complete on the other side; the next tick simply fetches again.
    TimedOut,
    /// The helper app cannot be used at all (not running, speaks another protocol, has no CloudKit
    /// access): nothing is wrong with the network or the account.
    Unavailable(String),
    Other(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::TimedOut => write!(f, "timed out"),
            NetError::Unavailable(m) => write!(f, "sync helper unavailable: {m}"),
            NetError::Offline => write!(f, "offline"),
            NetError::RateLimited { retry_after_ms } => {
                write!(f, "rate limited (retry after {retry_after_ms} ms)")
            }
            NetError::NotAuthenticated => write!(f, "not signed in to iCloud"),
            NetError::ZoneNotFound => write!(f, "iCloud zone not found"),
            NetError::TokenExpired => write!(f, "change token expired"),
            NetError::Quota => write!(f, "iCloud storage is full"),
            NetError::Other(m) => write!(f, "{m}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    Available,
    NoAccount,
    Restricted,
    /// iCloud could not say (usually: offline); treated like a network failure.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub status: AccountStatus,
    /// Stable hash of the account (never the account itself).
    pub hash: Option<String>,
}

/// A record as received from the zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub name: String,
    pub rtype: String,
    pub payload: Vec<u8>,
    pub tag: String,
}

/// A record that no longer exists on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deletion {
    pub name: String,
    pub rtype: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPage {
    pub changes: Vec<Change>,
    pub deletions: Vec<Deletion>,
    pub token: String,
    pub more_coming: bool,
}

/// A record to write. `tag` is the server change tag we last saw (`None` = we believe it is new).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutRecord {
    pub name: String,
    pub rtype: String,
    pub payload: Vec<u8>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveResult {
    Saved {
        name: String,
        tag: String,
    },
    /// Somebody else changed the record first: here is theirs.
    Conflict {
        name: String,
        server_payload: Vec<u8>,
        server_tag: String,
    },
    Error {
        name: String,
        code: String,
    },
}

pub trait Transport {
    fn account(&mut self) -> Result<AccountInfo, NetError>;
    /// Create the zone and its subscription; idempotent.
    fn ensure(&mut self) -> Result<(), NetError>;
    /// Changes after `token` (`None` = everything).
    fn fetch(&mut self, token: Option<&str>) -> Result<FetchPage, NetError>;
    fn save(&mut self, records: &[OutRecord]) -> Result<Vec<SaveResult>, NetError>;
    /// Delete the whole zone ("clear iCloud data").
    fn delete_zone(&mut self) -> Result<(), NetError>;
}

#[cfg(test)]
pub(crate) mod mem {
    //! An in-memory CloudKit stand-in: one zone shared by several devices, a change log, per-record
    //! change tags, and fault injection driven by a seeded generator so a failing run can be replayed.

    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    #[derive(Debug, Clone)]
    pub struct Stored {
        pub rtype: String,
        pub payload: Vec<u8>,
        pub tag: String,
        /// Position in the change log of the last write.
        pub seq: u64,
    }

    /// Fault probabilities in percent, rolled per call.
    #[derive(Debug, Clone, Default)]
    pub struct Faults {
        /// The call fails with `Offline` before anything happens.
        pub offline: u32,
        /// `save` is applied on the server but the reply is lost (`Offline` to the caller).
        pub lost_ack: u32,
        /// `fetch` starts a little before the token, redelivering records.
        pub redeliver: u32,
        /// All outstanding tokens expire.
        pub expire: u32,
    }

    #[derive(Debug)]
    pub struct Cloud {
        pub zone: bool,
        pub records: BTreeMap<String, Stored>,
        pub deleted: Vec<(u64, String, String)>,
        pub seq: u64,
        pub epoch: u64,
        pub account: AccountInfo,
        pub page: usize,
        pub faults: Faults,
        pub quota_full: bool,
        pub rng: u64,
        pub saves: usize,
        pub fetches: usize,
    }

    impl Cloud {
        pub fn new(seed: u64) -> Cloud {
            Cloud {
                zone: false,
                records: BTreeMap::new(),
                deleted: Vec::new(),
                seq: 0,
                epoch: 1,
                account: AccountInfo {
                    status: AccountStatus::Available,
                    hash: Some("account-1".into()),
                },
                page: 4,
                faults: Faults::default(),
                quota_full: false,
                rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
                saves: 0,
                fetches: 0,
            }
        }

        pub fn shared(seed: u64) -> Rc<RefCell<Cloud>> {
            Rc::new(RefCell::new(Cloud::new(seed)))
        }

        /// xorshift64*: deterministic and good enough for fault rolls.
        pub fn roll(&mut self, percent: u32) -> bool {
            if percent == 0 {
                return false;
            }
            let mut x = self.rng;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.rng = x;
            let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33;
            (v % 100) < u64::from(percent)
        }

        pub fn bump(&mut self) -> String {
            self.seq += 1;
            format!("tag-{}", self.seq)
        }

        /// Server-side removal of a record (only an administrator could do this in reality).
        pub fn delete_record(&mut self, name: &str) {
            if let Some(old) = self.records.remove(name) {
                self.seq += 1;
                self.deleted.push((self.seq, name.to_string(), old.rtype));
            }
        }

        fn token(&self, seq: u64) -> String {
            format!("{}.{}", self.epoch, seq)
        }

        fn parse_token(&self, token: &str) -> Option<(u64, u64)> {
            let (e, s) = token.split_once('.')?;
            Some((e.parse().ok()?, s.parse().ok()?))
        }
    }

    pub struct MemTransport {
        pub cloud: Rc<RefCell<Cloud>>,
    }

    impl MemTransport {
        pub fn new(cloud: &Rc<RefCell<Cloud>>) -> MemTransport {
            MemTransport {
                cloud: Rc::clone(cloud),
            }
        }
    }

    impl Transport for MemTransport {
        fn account(&mut self) -> Result<AccountInfo, NetError> {
            let mut c = self.cloud.borrow_mut();
            let p = c.faults.offline;
            if c.roll(p) {
                return Err(NetError::Offline);
            }
            Ok(c.account.clone())
        }

        fn ensure(&mut self) -> Result<(), NetError> {
            let mut c = self.cloud.borrow_mut();
            let p = c.faults.offline;
            if c.roll(p) {
                return Err(NetError::Offline);
            }
            c.zone = true;
            Ok(())
        }

        fn fetch(&mut self, token: Option<&str>) -> Result<FetchPage, NetError> {
            let mut c = self.cloud.borrow_mut();
            c.fetches += 1;
            let p = c.faults.offline;
            if c.roll(p) {
                return Err(NetError::Offline);
            }
            if !c.zone {
                return Err(NetError::ZoneNotFound);
            }
            let p = c.faults.expire;
            if c.roll(p) {
                c.epoch += 1;
                return Err(NetError::TokenExpired);
            }
            let mut since = match token {
                None => 0,
                Some(t) => match c.parse_token(t) {
                    Some((epoch, seq)) if epoch == c.epoch => seq,
                    _ => return Err(NetError::TokenExpired),
                },
            };
            let p = c.faults.redeliver;
            if since > 0 && c.roll(p) {
                since = since.saturating_sub(1 + c.seq.min(7));
            }
            let mut newer: Vec<(&String, &Stored)> =
                c.records.iter().filter(|(_, r)| r.seq > since).collect();
            newer.sort_by_key(|(_, r)| r.seq);
            let page = c.page.max(1);
            let more = newer.len() > page;
            let taken = &newer[..newer.len().min(page)];
            let changes: Vec<Change> = taken
                .iter()
                .map(|(name, r)| Change {
                    name: (*name).clone(),
                    rtype: r.rtype.clone(),
                    payload: r.payload.clone(),
                    tag: r.tag.clone(),
                })
                .collect();
            let upto = if more {
                taken.last().map_or(c.seq, |(_, r)| r.seq)
            } else {
                c.seq
            };
            let deletions: Vec<Deletion> = c
                .deleted
                .iter()
                .filter(|(seq, _, _)| *seq > since && *seq <= upto)
                .map(|(_, name, rtype)| Deletion {
                    name: name.clone(),
                    rtype: rtype.clone(),
                })
                .collect();
            Ok(FetchPage {
                changes,
                deletions,
                token: c.token(upto),
                more_coming: more,
            })
        }

        fn save(&mut self, records: &[OutRecord]) -> Result<Vec<SaveResult>, NetError> {
            let mut c = self.cloud.borrow_mut();
            let p = c.faults.offline;
            if c.roll(p) {
                return Err(NetError::Offline);
            }
            if !c.zone {
                return Err(NetError::ZoneNotFound);
            }
            c.saves += 1;
            let mut out = Vec::with_capacity(records.len());
            for r in records {
                if c.quota_full {
                    out.push(SaveResult::Error {
                        name: r.name.clone(),
                        code: "quotaExceeded".into(),
                    });
                    continue;
                }
                match c.records.get(&r.name) {
                    Some(server) if r.tag.as_deref() != Some(server.tag.as_str()) => {
                        out.push(SaveResult::Conflict {
                            name: r.name.clone(),
                            server_payload: server.payload.clone(),
                            server_tag: server.tag.clone(),
                        });
                    }
                    _ => {
                        let tag = c.bump();
                        let seq = c.seq;
                        c.records.insert(
                            r.name.clone(),
                            Stored {
                                rtype: r.rtype.clone(),
                                payload: r.payload.clone(),
                                tag: tag.clone(),
                                seq,
                            },
                        );
                        out.push(SaveResult::Saved {
                            name: r.name.clone(),
                            tag,
                        });
                    }
                }
            }
            let p = c.faults.lost_ack;
            if c.roll(p) {
                return Err(NetError::Offline);
            }
            Ok(out)
        }

        fn delete_zone(&mut self) -> Result<(), NetError> {
            let mut c = self.cloud.borrow_mut();
            c.zone = false;
            c.records.clear();
            c.deleted.clear();
            c.epoch += 1;
            Ok(())
        }
    }
}
