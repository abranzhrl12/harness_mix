//! OS containment. Windows attaches the suspended leader before it can spawn children.
use std::process::{Child, Command};

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        io,
        os::windows::{
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
            process::CommandExt,
        },
    };
    use windows::Win32::{
        Foundation::{ERROR_NO_MORE_FILES, HANDLE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
                QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
            },
            Threading::{
                CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
            },
        },
    };

    struct Handle(OwnedHandle);
    impl Handle {
        /// Takes ownership of a Win32 handle returned by an API.
        unsafe fn from_raw(handle: HANDLE) -> Self {
            // SAFETY: callers pass a newly created handle that is not owned elsewhere.
            Self(unsafe { OwnedHandle::from_raw_handle(handle.0) })
        }

        fn raw(&self) -> HANDLE {
            HANDLE(self.0.as_raw_handle())
        }
    }
    pub(crate) struct Tree {
        job: Handle,
    }

    pub(crate) fn configure(command: &mut Command) {
        command.creation_flags((CREATE_NO_WINDOW | CREATE_SUSPENDED).0);
    }

    impl Tree {
        pub(crate) fn attach(child: &Child) -> io::Result<Self> {
            Self::attach_handle(child.as_raw_handle(), child.id())
        }

        /// Attach a suspended native or async-runtime child to a new private
        /// Job Object, then resume its primary thread. The caller retains
        /// ownership of the process handle.
        pub(crate) fn attach_handle(
            process_handle: std::os::windows::io::RawHandle,
            process_id: u32,
        ) -> io::Result<Self> {
            // SAFETY: handles remain owned through each API call; all buffers have
            // the exact Win32 structure size and outlive the synchronous calls.
            unsafe {
                let job = Handle::from_raw(CreateJobObjectW(None, None).map_err(io::Error::other)?);
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.raw(),
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as _,
                    std::mem::size_of_val(&limits) as u32,
                )
                .map_err(io::Error::other)?;
                AssignProcessToJobObject(job.raw(), HANDLE(process_handle))
                    .map_err(io::Error::other)?;
                let tree = Self { job };
                let snapshot = Handle::from_raw(
                    CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).map_err(io::Error::other)?,
                );
                let mut entry = THREADENTRY32 {
                    dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                    ..Default::default()
                };
                Thread32First(snapshot.raw(), &mut entry).map_err(io::Error::other)?;
                loop {
                    if entry.th32OwnerProcessID == process_id {
                        let thread = Handle::from_raw(
                            OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                                .map_err(io::Error::other)?,
                        );
                        if ResumeThread(thread.raw()) == u32::MAX {
                            return Err(io::Error::last_os_error());
                        }
                        return Ok(tree);
                    }
                    match Thread32Next(snapshot.raw(), &mut entry) {
                        Ok(()) => {}
                        Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
                        Err(error) => return Err(io::Error::other(error)),
                    }
                }
                Err(io::Error::other("Suspended primary thread not found"))
            }
        }

        pub(crate) fn stop(&self) -> io::Result<()> {
            // SAFETY: job handle is live and uniquely owned by this Tree.
            unsafe { TerminateJobObject(self.job.raw(), 1).map_err(io::Error::other) }
        }

        pub(crate) fn empty(&self) -> io::Result<bool> {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            // SAFETY: writable buffer has the documented type and size.
            unsafe {
                QueryInformationJobObject(
                    Some(self.job.raw()),
                    JobObjectBasicAccountingInformation,
                    &mut accounting as *mut _ as _,
                    std::mem::size_of_val(&accounting) as u32,
                    None,
                )
                .map_err(io::Error::other)?;
            }
            Ok(accounting.ActiveProcesses == 0)
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::{io, os::unix::process::CommandExt};
    pub(crate) struct Tree {
        group: i32,
    }
    pub(crate) fn configure(command: &mut Command) {
        command.process_group(0);
    }
    impl Tree {
        pub(crate) fn attach(child: &Child) -> io::Result<Self> {
            Ok(Self {
                group: i32::try_from(child.id()).map_err(io::Error::other)?,
            })
        }
        pub(crate) fn stop(&self) -> io::Result<()> {
            // SAFETY: group is the positive PID of our own child created as leader.
            if unsafe { libc::kill(-self.group, libc::SIGKILL) } == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(())
            } else {
                Err(error)
            }
        }
        pub(crate) fn empty(&self) -> io::Result<bool> {
            // Process groups are not strong containment: children can call setsid.
            Ok(true)
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }
}

pub(super) use platform::{Tree, configure};
