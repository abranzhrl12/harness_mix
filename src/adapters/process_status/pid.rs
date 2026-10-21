use serde_json::{Value, json};

pub(super) fn check_pid(pid: u32) -> Value {
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::{CloseHandle, STILL_ACTIVE},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // SAFETY: OpenProcess called with limited query rights on numeric PID.
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) => {
                    let mut code = 0u32;
                    let queried = GetExitCodeProcess(handle, &mut code);
                    let _ = CloseHandle(handle);
                    if queried.is_err() {
                        json!({"pid":pid,"state":"unknown","reason":"query_failed","responsive":null})
                    } else if code == STILL_ACTIVE.0 as u32 {
                        json!({"pid": pid, "state": "exists", "responsive": null})
                    } else {
                        json!({"pid": pid, "state": "not_running", "responsive": null})
                    }
                }
                Err(error) => {
                    let (state, reason) = windows_open_error(error.code().0 as u32 & 0xffff);
                    json!({"pid":pid,"state":state,"reason":reason,"responsive":null})
                }
            }
        }
    }
    #[cfg(unix)]
    {
        let res = unsafe { libc::kill(pid as i32, 0) };
        if res == 0 {
            json!({"pid": pid, "state": "exists", "responsive": null})
        } else {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                json!({"pid": pid, "state": "not_running", "responsive": null})
            } else if err.raw_os_error() == Some(libc::EPERM) {
                json!({"pid": pid, "state": "unknown", "reason": "access_denied", "responsive": null})
            } else {
                json!({"pid": pid, "state": "unknown", "reason": "probe_failed", "responsive": null})
            }
        }
    }
}

#[cfg(any(windows, test))]
pub(super) fn windows_open_error(code: u32) -> (&'static str, &'static str) {
    match code {
        87 | 1168 => ("not_running", "not_found"),
        5 => ("unknown", "access_denied"),
        _ => ("unknown", "probe_failed"),
    }
}
