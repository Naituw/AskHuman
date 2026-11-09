//! A PID is useful only together with the lifetime of the process that originally owned it.

pub(crate) fn alive(pid: u32, expected: Option<&str>) -> bool {
    if !crate::agents::detect::pid_alive(pid) {
        return false;
    }
    match (expected, birth(pid)) {
        (Some(expected), Some(current)) => expected == current,
        _ => true,
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn birth(pid: u32) -> Option<String> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // The OS fills exactly this documented fixed-size structure before it is read.
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as libc::c_int,
        )
    };
    if read != size as libc::c_int {
        return None;
    }
    let info = unsafe { info.assume_init() };
    Some(format!(
        "mac:{}:{}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn birth(pid: u32) -> Option<String> {
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat.rsplit_once(')')?.1;
    let ticks = fields.split_whitespace().nth(19)?;
    Some(format!("linux:{}:{ticks}", boot.trim()))
}

#[cfg(windows)]
pub(crate) fn birth(pid: u32) -> Option<String> {
    crate::agents::detect::inspect_process(pid)?
        .creation_time
        .map(|time| format!("windows:{time}"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(crate) fn birth(_pid: u32) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_reused_pid_is_not_the_original_owner() {
        let pid = std::process::id();
        let birth = birth(pid).expect("the running process has a birth identity");
        assert!(alive(pid, Some(&birth)));
        assert!(!alive(pid, Some("an earlier process")));
        assert!(!alive(0, None));
    }
}
