//! Apply the original DACL before any candidate content enters the temporary file.
use harness_domain::Failure;
use std::path::Path;

#[cfg(windows)]
pub(super) fn descriptor(path: &Path) -> Result<Vec<u32>, Failure> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Security::{DACL_SECURITY_INFORMATION, GetFileSecurityW, PSECURITY_DESCRIPTOR},
        core::PCWSTR,
    };
    let name: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut needed = 0;
    // SAFETY: sizing call has no output buffer; required length is a live u32.
    unsafe {
        let _ = GetFileSecurityW(
            PCWSTR(name.as_ptr()),
            DACL_SECURITY_INFORMATION.0,
            None,
            0,
            &mut needed,
        );
    }
    if needed == 0 || needed > 1024 * 1024 {
        return Err(("EDIT_SECURITY", "Cannot size original DACL"));
    }
    // SECURITY_DESCRIPTOR is DWORD aligned. Keep the backing allocation alive.
    let mut buffer = vec![0u32; (needed as usize).div_ceil(4)];
    unsafe {
        GetFileSecurityW(
            PCWSTR(name.as_ptr()),
            DACL_SECURITY_INFORMATION.0,
            Some(PSECURITY_DESCRIPTOR(buffer.as_mut_ptr().cast())),
            needed,
            &mut needed,
        )
        .ok()
        .map_err(|_| ("EDIT_SECURITY", "Cannot read original DACL"))?;
    }
    Ok(buffer)
}

#[cfg(windows)]
pub(super) fn protect_stage(original: &Path, stage: &Path) -> Result<(), Failure> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Security::{
            Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW},
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        },
        core::PCWSTR,
    };
    let buffer = descriptor(original)?;
    let name: Vec<_> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut present = windows::core::BOOL::default();
    let mut defaulted = windows::core::BOOL::default();
    let mut dacl = std::ptr::null_mut();
    // SAFETY: descriptor is self-relative, DWORD aligned, and live for this call.
    // Protecting the staged DACL prevents broader directory inheritance being added.
    unsafe {
        GetSecurityDescriptorDacl(
            PSECURITY_DESCRIPTOR(buffer.as_ptr().cast_mut().cast()),
            &mut present,
            &mut dacl,
            &mut defaulted,
        )
        .map_err(|_| ("EDIT_SECURITY", "Cannot inspect original DACL"))?;
        if !present.as_bool() {
            return Err(("EDIT_SECURITY", "Original has no explicit DACL descriptor"));
        }
        SetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        )
        .ok()
        .map_err(|_| {
            (
                "EDIT_SECURITY",
                "Cannot protect staging file; original untouched",
            )
        })
    }
}

#[cfg(not(windows))]
pub(super) fn protect_stage(_: &Path, _: &Path) -> Result<(), Failure> {
    Err(("UNSUPPORTED", "Editing security adapter requires Windows"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn publication_preserves_a_protected_dacl() {
        for protect in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let file = root.path().join("file.txt");
            std::fs::write(&file, "old").unwrap();
            if protect {
                protect_stage(&file, &file).unwrap();
            }
            let before = descriptor(&file).unwrap();
            let app = crate::adapters::application(root.path()).unwrap();
            let result = app.execute(harness_domain::contracts::Request::Replace(
                harness_domain::contracts::ReplaceArgs {
                    path: "file.txt".into(),
                    hash: harness_domain::hash(b"old"),
                    chunks: None,
                    old_text: "old".into(),
                    new_text: "new".into(),
                    allow_unvalidated_text: false,
                    session: None,
                },
            ));
            assert_eq!(result["state"], "ok", "{result}");
            let after = descriptor(&file).unwrap();
            assert_eq!(
                after[0] & 0x1000_0000,
                before[0] & 0x1000_0000,
                "Original inheritance protection must survive"
            );
            assert_eq!(&after[1..], &before[1..], "ACL contents must survive");
        }
    }
}
