//! Hybrid logical clock (spec todo-sync §5.4, plan §3.4).
//!
//! A timestamp is the string `{wallMs 13 digits}-{counter 4 hex}-{device tag 8}`; **lexicographic order
//! equals time order**, so the register merge is a plain string comparison. The device tag only breaks
//! ties between devices that issue the very same `(wall, counter)`.

use serde::{Deserialize, Serialize};

/// A remote wall clock further ahead of ours than this is clamped, so one Mac with a broken clock
/// cannot make all of its edits win forever.
pub const CLAMP_MS: u64 = 24 * 60 * 60 * 1000;

/// Persisted position of this device's clock (`state.json` → `clock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockState {
    pub wall_ms: u64,
    pub counter: u16,
}

/// First eight lowercase alphanumerics of a device id (the tag embedded in every timestamp).
pub fn device_tag(device_id: &str) -> String {
    let tag: String = device_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    format!("{tag:0<8}")
}

pub fn format(wall_ms: u64, counter: u16, tag: &str) -> String {
    format!("{wall_ms:013}-{counter:04x}-{tag}")
}

/// `(wall, counter, tag)` of a well-formed timestamp.
pub fn parse(ts: &str) -> Option<(u64, u16, &str)> {
    let mut it = ts.splitn(3, '-');
    let wall = it.next()?;
    let counter = it.next()?;
    let tag = it.next()?;
    if wall.len() != 13 || counter.len() != 4 || tag.is_empty() {
        return None;
    }
    Some((
        wall.parse().ok()?,
        u16::from_str_radix(counter, 16).ok()?,
        tag,
    ))
}

impl ClockState {
    /// Timestamp for a new local event: strictly greater than every timestamp issued or observed so far.
    pub fn issue(&mut self, now_ms: u64, device_tag: &str) -> String {
        if now_ms > self.wall_ms {
            self.wall_ms = now_ms;
            self.counter = 0;
        } else if self.counter == u16::MAX {
            self.wall_ms += 1;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        format(self.wall_ms, self.counter, device_tag)
    }

    /// Advance past a timestamp received from another device (clamped, see [`CLAMP_MS`]).
    pub fn observe(&mut self, now_ms: u64, remote: &str) {
        let Some((wall, counter, _)) = parse(remote) else {
            return;
        };
        let limit = now_ms.saturating_add(CLAMP_MS);
        let (wall, counter) = if wall > limit {
            (limit, 0)
        } else {
            (wall, counter)
        };
        if (wall, counter) > (self.wall_ms, self.counter) {
            self.wall_ms = wall;
            self.counter = counter;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parse_roundtrip_and_order() {
        let a = format(1_700_000_000_000, 1, "aaaaaaaa");
        assert_eq!(a, "1700000000000-0001-aaaaaaaa");
        assert_eq!(parse(&a), Some((1_700_000_000_000, 1, "aaaaaaaa")));
        let b = format(1_700_000_000_000, 0x0a, "aaaaaaaa");
        let c = format(1_700_000_000_001, 0, "aaaaaaaa");
        assert!(
            a < b && b < c,
            "string order must equal (wall, counter) order"
        );
        assert!(format(5, 0, "aaaaaaaa") < format(5, 0, "bbbbbbbb"));
        assert_eq!(parse("garbage"), None);
        assert_eq!(parse("17000000000-0001-aaaaaaaa"), None);
    }

    #[test]
    fn device_tag_is_stable_and_padded() {
        assert_eq!(
            device_tag("3F2A9C11-7B5E-4D0A-9C1E-0123456789AB"),
            "3f2a9c11"
        );
        assert_eq!(device_tag("ab"), "ab000000");
    }

    #[test]
    fn issue_is_strictly_monotonic_even_with_a_stalled_or_backwards_clock() {
        let mut clock = ClockState::default();
        let mut last = String::new();
        for now in [100, 100, 100, 90, 50, 101, 101, 7] {
            let ts = clock.issue(now, "dev00000");
            assert!(ts > last, "{ts} must exceed {last}");
            last = ts;
        }
        clock.counter = u16::MAX;
        let wrapped = clock.issue(0, "dev00000");
        assert!(wrapped > last);
    }

    #[test]
    fn observe_moves_the_clock_past_remote_but_clamps_far_future() {
        let now = 1_000_000;
        let mut clock = ClockState::default();
        let remote = format(now + 5_000, 3, "bbbbbbbb");
        clock.observe(now, &remote);
        assert!(clock.issue(now, "aaaaaaaa") > remote);

        let mut clock = ClockState::default();
        let crazy = format(now + 10 * CLAMP_MS, 9, "bbbbbbbb");
        clock.observe(now, &crazy);
        assert_eq!(clock.wall_ms, now + CLAMP_MS);
        assert!(clock.issue(now, "aaaaaaaa") < crazy);

        // Garbage never moves the clock.
        let mut clock = ClockState::default();
        clock.observe(now, "nonsense");
        assert_eq!(clock, ClockState::default());
    }
}
