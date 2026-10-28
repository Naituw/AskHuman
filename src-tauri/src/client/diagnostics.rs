//! Shared diagnostics for daemon startup and status failures.

use crate::i18n::Lang;
use std::io::{Error, ErrorKind};

/// Only a missing endpoint or a refused connection establishes that no daemon is listening.
pub fn is_not_running(error: &Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::NotFound | ErrorKind::ConnectionRefused
    )
}

pub fn error_detail(error: &Error, lang: Lang) -> String {
    let mut detail = error.to_string();
    if error.kind() == ErrorKind::PermissionDenied {
        detail.push_str(match lang {
            Lang::En => "; access may be blocked by a sandbox or filesystem permissions; check access to the AskHuman directory and local IPC socket",
            Lang::Zh => "；可能被沙箱或文件系统权限阻止，请检查 AskHuman 目录和本地 IPC socket 的访问权限",
        });
    }
    detail
}

pub fn status_unavailable(error: &Error, lang: Lang) -> String {
    let label = match lang {
        Lang::En => "daemon status unavailable",
        Lang::Zh => "daemon 状态不可用",
    };
    format!("{label}: {}", error_detail(error, lang))
}

pub fn start_failed(error: &Error) -> String {
    format!(
        "askhuman: failed to start daemon: {}",
        error_detail(error, Lang::En)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn connection_errno_distinguishes_absence_from_denied_access() {
        for errno in [libc::ENOENT, libc::ECONNREFUSED] {
            let error = Error::from_raw_os_error(errno);
            assert!(is_not_running(&error));
            assert_eq!(error_detail(&error, Lang::En), error.to_string());
        }
        for errno in [libc::EPERM, libc::EACCES] {
            let error = Error::from_raw_os_error(errno);
            assert!(!is_not_running(&error));
            for (lang, hint) in [
                (Lang::En, "may be blocked by a sandbox"),
                (Lang::Zh, "可能被沙箱"),
            ] {
                let detail = error_detail(&error, lang);
                assert!(detail.contains(&error.to_string()));
                assert!(detail.contains(hint));
            }
        }
        assert!(!is_not_running(&Error::from(ErrorKind::TimedOut)));
        assert!(!is_not_running(&Error::from(ErrorKind::InvalidData)));
    }

    #[test]
    fn startup_error_keeps_original_reason_without_speculating_about_sandbox() {
        let error = Error::other("read-only file system (os error 30)");
        assert_eq!(
            start_failed(&error),
            "askhuman: failed to start daemon: read-only file system (os error 30)"
        );
    }

    #[test]
    fn startup_permission_error_keeps_reason_and_adds_possible_sandbox_hint() {
        let error = Error::new(ErrorKind::PermissionDenied, "cannot open daemon.log");
        let message = start_failed(&error);
        assert!(message.starts_with("askhuman: failed to start daemon: cannot open daemon.log"));
        assert!(message.contains("may be blocked by a sandbox"));
    }
}
