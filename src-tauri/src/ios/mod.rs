//! iOS channel (docs/specs/ios-channel.md, docs/plans/ios-channel.md).
//!
//! The protocol-level pieces here (crypto, envelope, wire messages) are independent of the daemon so
//! they can be tested against the cross-language fixtures in `ios/Fixtures/wire-v1/`.

pub mod connstr;
pub mod crypto;
pub mod envelope;
pub mod gateway;
pub mod push;
pub mod qr;
pub mod store;
pub mod transport;
pub mod views;
pub mod wire;

#[cfg(test)]
mod fixtures;
