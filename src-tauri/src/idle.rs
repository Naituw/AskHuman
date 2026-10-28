//! How long this Mac has gone without keyboard / mouse input.
//!
//! The iOS channel pushes a question the phone has not seen only once the Mac has been idle for a
//! while (`docs/specs/ios-alert-timing.md`). Input idle time is the only "is the user at the Mac"
//! signal on purpose: a locked screen, a sleeping display or a dimmed one say little about a user
//! who leaves an Agent working on an unlocked, always-on Mac.

/// Seconds since the last keyboard, mouse or trackpad input of the current login session.
///
/// `None` when the platform has no implementation or the system gives no usable answer; callers
/// treat that as "unknown" and fall back to notifying at once.
pub fn secs_since_last_input() -> Option<u64> {
    imp::secs_since_last_input()
}

/// Maps a raw reading to whole seconds. Anything that is not a finite, non-negative number is
/// "unknown".
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn whole_secs(raw: f64) -> Option<u64> {
    (raw.is_finite() && raw >= 0.0).then_some(raw as u64)
}

#[cfg(target_os = "macos")]
mod imp {
    /// `kCGEventSourceStateCombinedSessionState`: hardware events plus anything posted into the
    /// session.
    const COMBINED_SESSION_STATE: i32 = 0;
    /// `kCGAnyInputEventType`, i.e. `(CGEventType)~0`.
    const ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
    }

    /// Needs no permission (unlike an event tap) and is cheap: the window server keeps the counter.
    pub fn secs_since_last_input() -> Option<u64> {
        // SAFETY: a plain query with scalar arguments and no pointers.
        let raw = unsafe {
            CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT_TYPE)
        };
        super::whole_secs(raw)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    /// Windows could use `GetLastInputInfo` (no permission needed) later; until then the idle time
    /// is unknown and the iOS channel pushes at once, as it did before.
    pub fn secs_since_last_input() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unusable_readings_are_unknown() {
        assert_eq!(whole_secs(f64::NAN), None);
        assert_eq!(whole_secs(f64::INFINITY), None);
        assert_eq!(whole_secs(-1.0), None);
    }

    #[test]
    fn usable_readings_are_truncated_to_whole_seconds() {
        assert_eq!(whole_secs(0.0), Some(0));
        assert_eq!(whole_secs(12.9), Some(12));
        assert_eq!(whole_secs(3600.0), Some(3600));
    }

    #[test]
    fn reading_the_system_clock_does_not_panic() {
        // A CI runner without a login session may answer with anything; only the platform
        // without an implementation has a fixed answer.
        let reading = secs_since_last_input();
        #[cfg(not(target_os = "macos"))]
        assert_eq!(reading, None);
        #[cfg(target_os = "macos")]
        let _ = reading;
    }
}
