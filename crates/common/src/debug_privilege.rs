// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether this process holds `SeDebugPrivilege` enabled.
//!
//! On Windows a token with that privilege enabled opens any process and any thread whatever the
//! target's access-control list says, so the control in `process_hardening` cannot refuse it.
//! An elevated administrator's token has the privilege, usually disabled; a GitHub-hosted
//! Windows runner's `runneradmin` has it enabled. The tests that watch a hardened process being
//! refused ask this first and skip where the refusal cannot be seen.

/// Whether this process's own token has `SeDebugPrivilege` enabled (present but disabled does not
/// count: only an enabled privilege bypasses the list). Always false off Windows.
pub fn debug_privilege_enabled() -> bool {
    imp::enabled()
}

/// For a test that expects another process to be refused access to a hardened one: true, after
/// printing why, when this process holds `SeDebugPrivilege` enabled and the refusal cannot be
/// observed. The test returns early on true; on an ordinary token it runs in full.
pub fn refusal_cannot_be_observed(test: &str) -> bool {
    let skip = debug_privilege_enabled();
    if skip {
        eprintln!(
            "SKIPPED {test}: this process holds SeDebugPrivilege enabled, which opens any process whatever its \
             access-control list says (an elevated administrator's token with the privilege on, as on a GitHub \
             Windows runner), so a refusal cannot be observed here"
        );
    }
    skip
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn enabled() -> bool {
        false
    }
}

#[cfg(windows)]
mod imp {
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LUID};
    use windows_sys::Win32::Security::{
        GetTokenInformation, LookupPrivilegeValueW, TokenPrivileges, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
        TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Whether `privilege` is in `held` with the enabled flag set.
    pub(super) fn is_enabled(held: &[LUID_AND_ATTRIBUTES], privilege: LUID) -> bool {
        held.iter().any(|p| {
            p.Luid.LowPart == privilege.LowPart && p.Luid.HighPart == privilege.HighPart && p.Attributes & SE_PRIVILEGE_ENABLED != 0
        })
    }

    pub(super) fn enabled() -> bool {
        // SAFETY: plain Win32 calls on this process's own token, closed on every path; the buffer
        // is 8-byte aligned and as long as the call asked for.
        unsafe {
            let name: Vec<u16> = "SeDebugPrivilege".encode_utf16().chain(std::iter::once(0)).collect();
            let mut debug = LUID { LowPart: 0, HighPart: 0 };
            if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut debug) == 0 {
                return false;
            }
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenPrivileges, null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            let got = GetTokenInformation(token, TokenPrivileges, buf.as_mut_ptr().cast(), len, &mut len) != 0;
            CloseHandle(token);
            if !got {
                return false;
            }
            let privileges = &*(buf.as_ptr() as *const TOKEN_PRIVILEGES);
            let held = std::slice::from_raw_parts(privileges.Privileges.as_ptr(), privileges.PrivilegeCount as usize);
            is_enabled(held, debug)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn held(luid: u32, attributes: u32) -> LUID_AND_ATTRIBUTES {
            LUID_AND_ATTRIBUTES { Luid: LUID { LowPart: luid, HighPart: 0 }, Attributes: attributes }
        }

        #[test]
        fn only_an_enabled_privilege_counts() {
            let debug = LUID { LowPart: 20, HighPart: 0 };
            assert!(is_enabled(&[held(20, SE_PRIVILEGE_ENABLED)], debug), "enabled");
            assert!(is_enabled(&[held(5, 0), held(20, SE_PRIVILEGE_ENABLED | 1)], debug), "enabled, among others and by default");
            assert!(!is_enabled(&[held(20, 0)], debug), "present but disabled: the list still applies");
            assert!(!is_enabled(&[held(21, SE_PRIVILEGE_ENABLED)], debug), "a different privilege");
            assert!(!is_enabled(&[], debug), "none held");
        }

        #[test]
        fn this_process_answers_without_failing() {
            // Which answer depends on the token the tests run under; it must come back either way.
            eprintln!("SeDebugPrivilege enabled in this test process: {}", enabled());
        }
    }
}
