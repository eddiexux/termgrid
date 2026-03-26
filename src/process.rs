use std::path::PathBuf;

/// Get foreground process group ID from PTY master file descriptor.
///
/// # Arguments
/// * `master_fd` - The file descriptor of the PTY master
///
/// # Returns
/// `Some(pgid)` if successful, `None` if the operation fails.
#[cfg(target_os = "macos")]
pub fn get_foreground_pid(master_fd: i32) -> Option<i32> {
    let pgid = unsafe { libc::tcgetpgrp(master_fd) };
    if pgid < 0 {
        None
    } else {
        Some(pgid)
    }
}

/// Get the current working directory (CWD) of a process by its PID.
///
/// This uses `proc_pidinfo` with `PROC_PIDVNODEPATHINFO` on macOS to read
/// the process's directory information.
///
/// # Arguments
/// * `pid` - The process ID
///
/// # Returns
/// `Some(PathBuf)` containing the CWD if successful, `None` otherwise.
#[cfg(target_os = "macos")]
pub fn get_process_cwd(pid: i32) -> Option<PathBuf> {
    use std::str;

    const PROC_PIDVNODEPATHINFO: i32 = 9;
    const MAXPATHLEN: usize = 1024;
    const VNODE_INFO_SIZE: usize = 152;

    #[repr(C)]
    struct VnodeInfoPath {
        _vnode_info: [u8; VNODE_INFO_SIZE],
        vip_path: [u8; MAXPATHLEN],
    }

    #[repr(C)]
    struct ProcVnodePathInfo {
        pvi_cdir: VnodeInfoPath,
        _pvi_rdir: VnodeInfoPath,
    }

    unsafe {
        let mut info: ProcVnodePathInfo = std::mem::zeroed();
        let size = std::mem::size_of::<ProcVnodePathInfo>() as i32;
        let ret = libc::proc_pidinfo(
            pid,
            PROC_PIDVNODEPATHINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );

        if ret <= 0 {
            return None;
        }

        // Find the null terminator in the path buffer
        let len = info
            .pvi_cdir
            .vip_path
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(MAXPATHLEN);

        // Convert bytes to UTF-8 string
        let path_str = str::from_utf8(&info.pvi_cdir.vip_path[..len]).ok()?;
        Some(PathBuf::from(path_str))
    }
}

// Non-macOS stubs
#[cfg(not(target_os = "macos"))]
pub fn get_foreground_pid(_master_fd: i32) -> Option<i32> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn get_process_cwd(_pid: i32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn test_get_current_process_cwd() {
        // Get the current process's PID
        let pid = std::process::id() as i32;

        // Try to get the CWD
        if let Some(cwd) = get_process_cwd(pid) {
            // Verify the path exists
            assert!(cwd.exists(), "CWD should exist: {:?}", cwd);
        } else {
            // This can happen in some sandboxed environments, but on macOS it should work
            eprintln!("Warning: Could not retrieve CWD for current process");
        }
    }

    #[test]
    fn test_get_cwd_invalid_pid() {
        // Use an invalid PID that's unlikely to exist
        let invalid_pid = -1i32;
        assert_eq!(get_process_cwd(invalid_pid), None);
    }
}
