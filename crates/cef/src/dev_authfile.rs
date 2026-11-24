// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The files a running host publishes so external test harnesses can find it.
//!
//! * A dev build (`task dev`) writes `<data_dir>/authkey.dev`: the random
//!   per-process `auth_key`, IPC token, backend endpoints and instance
//!   metadata, so a harness in any language can call `POST /agentmux/service`
//!   against it.
//! * An installed or portable build never writes the instance key or the host
//!   token to disk. It writes `<data_dir>/instance.json` instead: endpoints,
//!   pid, version and debug port, nothing that authenticates a caller. On
//!   start it also removes a leftover `authkey.dev` in its data folder.
//!
//! The split is by the runtime mode (`RuntimeMode::Dev`), the same signal that
//! gates the debugging port. See [`publish_discovery`].
//!
//! On Windows each file is created with an owner-only DACL via
//! `SetNamedSecurityInfoW`, with `PROTECTED_DACL_SECURITY_INFORMATION`
//! to break parent-dir inheritance: defense against a hostile parent
//! ACL change after file creation. On Unix the file is chmod'd 0600
//! after creation to override the default umask.
//!
//! Spec: `docs/specs/SPEC_TEST_API_ACCESS.md` §5–§6.

use serde::Serialize;
use std::path::Path;

const FILE_NAME: &str = "authkey.dev";

#[derive(Serialize)]
pub struct DevAuthFile<'a> {
    pub version: u32,
    pub auth_key: &'a str,
    pub web_endpoint: &'a str,
    pub ws_endpoint: &'a str,
    pub ipc_endpoint: &'a str,
    pub ipc_token: &'a str,
    pub service_path: &'static str,
    pub file_path: &'static str,
    pub instance: &'a str,
    pub data_dir: String,
    pub host_pid: u32,
    /// CEF remote-debugging (CDP) port actually bound by THIS instance.
    /// Not 9222/9223: those are only the preferred values, and the second
    /// instance to start gets an OS-assigned port instead. Tooling must read
    /// this rather than assume the constant, or it targets another instance.
    pub debug_port: u16,
    pub created_at: String,
}

const DISCOVERY_FILE: &str = "instance.json";

/// What a non-dev build publishes. Endpoints and identity only: no field of
/// this type may hold a key, token or secret (a test pins the field list).
#[derive(Serialize)]
pub struct InstanceInfo<'a> {
    pub version: u32,
    pub web_endpoint: &'a str,
    pub ws_endpoint: &'a str,
    pub instance: &'a str,
    pub data_dir: String,
    pub host_pid: u32,
    pub debug_port: u16,
    pub created_at: String,
}

/// What [`publish_discovery`] did, for the caller to log.
pub struct Published {
    pub path: std::path::PathBuf,
    /// A leftover `authkey.dev` was found and removed (non-dev builds only).
    pub removed_leftover: bool,
}

/// Publish this host's discovery file. A dev build writes the full
/// `authkey.dev`; any other build removes a leftover `authkey.dev` and writes
/// `instance.json`, which carries no key and no token.
pub fn publish_discovery(
    is_dev: bool,
    data_dir: &Path,
    auth_key: &str,
    web_endpoint: &str,
    ws_endpoint: &str,
    ipc_endpoint: &str,
    ipc_token: &str,
    instance: &str,
    host_pid: u32,
    debug_port: u16,
) -> Result<Published, String> {
    if is_dev {
        let path = write_dev_auth_file(
            data_dir, auth_key, web_endpoint, ws_endpoint, ipc_endpoint, ipc_token, instance, host_pid, debug_port,
        )?;
        return Ok(Published { path, removed_leftover: false });
    }
    let removed_leftover = remove_leftover(data_dir);
    let info = InstanceInfo {
        version: 1,
        web_endpoint,
        ws_endpoint,
        instance,
        data_dir: data_dir.to_string_lossy().into_owned(),
        host_pid,
        debug_port,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let json = serde_json::to_string_pretty(&info).map_err(|e| format!("serialize {}: {}", DISCOVERY_FILE, e))?;
    let path = data_dir.join(DISCOVERY_FILE);
    std::fs::write(&path, json.as_bytes()).map_err(|e| format!("write {}: {}", path.display(), e))?;
    #[cfg(unix)]
    apply_owner_only_mode(&path)?;
    #[cfg(target_os = "windows")]
    apply_owner_only_dacl(&path)?;
    Ok(Published { path, removed_leftover })
}

/// Remove an `authkey.dev` an earlier build left in `data_dir`. True when one was there.
fn remove_leftover(data_dir: &Path) -> bool {
    std::fs::remove_file(data_dir.join(FILE_NAME)).is_ok()
}

/// Write `authkey.dev` to `data_dir`. Returns the absolute file path on
/// success. Errors are returned as strings — the caller in `main.rs`
/// logs them at warn-level and continues; a missing dev file is not a
/// fatal startup failure.
pub fn write_dev_auth_file(
    data_dir: &Path,
    auth_key: &str,
    web_endpoint: &str,
    ws_endpoint: &str,
    ipc_endpoint: &str,
    ipc_token: &str,
    instance: &str,
    host_pid: u32,
    debug_port: u16,
) -> Result<std::path::PathBuf, String> {
    let path = data_dir.join(FILE_NAME);
    let payload = DevAuthFile {
        version: 1,
        auth_key,
        web_endpoint,
        ws_endpoint,
        ipc_endpoint,
        ipc_token,
        service_path: "/agentmux/service",
        file_path: "/agentmux/file",
        instance,
        data_dir: data_dir.to_string_lossy().into_owned(),
        host_pid,
        debug_port,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("serialize authkey.dev: {}", e))?;

    // Overwrite-if-exists. The previous file is by definition stale (it
    // belonged to a prior cef host process); harnesses that read the
    // file are expected to validate `host_pid` liveness.
    std::fs::write(&path, json.as_bytes())
        .map_err(|e| format!("write {}: {}", path.display(), e))?;

    // Lock down read access to the file owner. Default umask leaves
    // world-readable bits on most distros; on Windows, files inherit
    // the parent dir's ACL which can include other principals. Both
    // paths apply explicit owner-only access — the auth_key and
    // ipc_token in this file would let any peer on the machine drive
    // the service API.
    #[cfg(unix)]
    apply_owner_only_mode(&path)?;
    #[cfg(target_os = "windows")]
    apply_owner_only_dacl(&path)?;

    Ok(path)
}

#[cfg(unix)]
pub(crate) fn apply_owner_only_mode(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("chmod 0600 {}: {}", path.display(), e))
}

#[cfg(target_os = "windows")]
pub(crate) fn apply_owner_only_dacl(path: &Path) -> Result<(), String> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        SetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AddAccessAllowedAce, GetLengthSid, GetTokenInformation, InitializeAcl,
        TokenUser, ACL, ACL_REVISION, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcessToken,
    };

    // FILE_ALL_ACCESS = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x1FF.
    // Defined locally to avoid pulling in Win32_Storage_FileSystem just
    // for one constant.
    const FILE_ALL_ACCESS: u32 = 0x001F_01FF;

    unsafe {
        // 1. Get current-user SID via process token.
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(format!("OpenProcessToken failed: {}", GetLastError()));
        }
        let mut needed: u32 = 0;
        // First call returns ERROR_INSUFFICIENT_BUFFER and sets `needed`.
        let _ = GetTokenInformation(
            token,
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
        if needed == 0 {
            CloseHandle(token);
            return Err(format!(
                "GetTokenInformation sizing returned 0: {}",
                GetLastError()
            ));
        }
        let mut token_buf: Vec<u8> = vec![0; needed as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            token_buf.as_mut_ptr() as *mut c_void,
            needed,
            &mut needed,
        ) == 0
        {
            CloseHandle(token);
            return Err(format!("GetTokenInformation failed: {}", GetLastError()));
        }
        CloseHandle(token);

        // SID is owned by `token_buf`; keep that buffer alive for the
        // duration of ACL construction.
        let token_user_ptr = token_buf.as_ptr() as *const TOKEN_USER;
        let sid = (*token_user_ptr).User.Sid;
        let sid_len = GetLengthSid(sid);
        if sid_len == 0 {
            return Err("GetLengthSid returned 0 — invalid SID".to_string());
        }

        // 2. Build a DACL: ACL header + one ACCESS_ALLOWED_ACE inline
        //    with the SID body. ACCESS_ALLOWED_ACE.SidStart is the first
        //    DWORD of the SID; total ACE size = struct size + sid_len -
        //    sizeof(DWORD).
        let ace_header_size =
            std::mem::size_of::<windows_sys::Win32::Security::ACCESS_ALLOWED_ACE>() as u32;
        let ace_size = ace_header_size
            .saturating_add(sid_len)
            .saturating_sub(std::mem::size_of::<u32>() as u32);
        let acl_size = std::mem::size_of::<ACL>() as u32 + ace_size;

        let mut acl_buf: Vec<u8> = vec![0; acl_size as usize];
        let acl_ptr = acl_buf.as_mut_ptr() as *mut ACL;
        if InitializeAcl(acl_ptr, acl_size, ACL_REVISION as u32) == 0 {
            return Err(format!("InitializeAcl failed: {}", GetLastError()));
        }
        if AddAccessAllowedAce(acl_ptr, ACL_REVISION as u32, FILE_ALL_ACCESS, sid)
            == 0
        {
            return Err(format!("AddAccessAllowedAce failed: {}", GetLastError()));
        }

        // 3. Apply DACL to the file. PROTECTED_DACL_SECURITY_INFORMATION
        //    breaks inheritance from the parent dir so a later ACL change
        //    on the data dir doesn't widen access to authkey.dev.
        let mut wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let result = SetNamedSecurityInfoW(
            wide.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl_ptr,
            std::ptr::null_mut(),
        );
        if result != 0 {
            return Err(format!("SetNamedSecurityInfoW returned {}", result));
        }
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────
//
// Gated behind `--features test-authfile` per spec §7. They write to
// $TEMP and (on Windows) read the DACL back via GetNamedSecurityInfoW
// to assert exactly one ACE present, granting access to the current
// user's SID. The feature gate keeps OS-touching tests opt-in so plain
// `cargo test` stays hermetic.

#[cfg(all(test, feature = "test-authfile"))]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("agentmux-authfile-test-{}-{}", label, std::process::id()));
        std::fs::create_dir_all(&p).expect("mkdir tempdir");
        p
    }

    #[test]
    fn writes_well_formed_json_with_all_fields() {
        let dir = temp_dir("format");
        let path = write_dev_auth_file(
            &dir,
            "f8c9b0e4-1234-4567-89ab-cdef01234567",
            "127.0.0.1:59719",
            "127.0.0.1:59720",
            "127.0.0.1:59718",
            "92d136fa-2e14-46d0-9ace-eddee320a35e",
            "v0.33.265",
            12345,
            9222,
        )
        .expect("write authfile");

        let body = std::fs::read_to_string(&path).expect("read back");
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("parse JSON");

        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["auth_key"], "f8c9b0e4-1234-4567-89ab-cdef01234567");
        assert_eq!(parsed["web_endpoint"], "127.0.0.1:59719");
        assert_eq!(parsed["ws_endpoint"], "127.0.0.1:59720");
        assert_eq!(parsed["ipc_endpoint"], "127.0.0.1:59718");
        assert_eq!(parsed["ipc_token"], "92d136fa-2e14-46d0-9ace-eddee320a35e");
        assert_eq!(parsed["service_path"], "/agentmux/service");
        assert_eq!(parsed["file_path"], "/agentmux/file");
        assert_eq!(parsed["instance"], "v0.33.265");
        assert_eq!(parsed["host_pid"], 12345);
        assert!(parsed["created_at"].as_str().unwrap_or("").contains("T"));
        assert!(parsed["data_dir"].as_str().unwrap_or("").contains("agentmux-authfile-test"));

        // cleanup
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(parsed["debug_port"], 9222, "all-fields test must cover debug_port");
    }

    #[test]
    fn overwrites_existing_file_on_repeat_call() {
        let dir = temp_dir("overwrite");
        let path1 = write_dev_auth_file(
            &dir, "old-key", "127.0.0.1:1", "127.0.0.1:2", "127.0.0.1:3",
            "old-token", "v0.0.1", 1, 9222,
        ).unwrap();
        let path2 = write_dev_auth_file(
            &dir, "new-key", "127.0.0.1:11", "127.0.0.1:22", "127.0.0.1:33",
            "new-token", "v0.0.2", 2, 42149,
        ).unwrap();
        assert_eq!(path1, path2, "same path expected");
        let body = std::fs::read_to_string(&path2).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["auth_key"], "new-key");
        assert_eq!(parsed["instance"], "v0.0.2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dacl_grants_only_current_user() {
        use std::ffi::c_void;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
        use windows_sys::Win32::Security::Authorization::{
            GetNamedSecurityInfoW, SE_FILE_OBJECT,
        };
        use windows_sys::Win32::Security::{
            EqualSid, GetAce, GetTokenInformation, TokenUser,
            ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
            DACL_SECURITY_INFORMATION, GetAclInformation, TOKEN_QUERY, TOKEN_USER,
        };
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, OpenProcessToken,
        };

        let dir = temp_dir("dacl");
        let path = write_dev_auth_file(
            &dir, "k", "127.0.0.1:1", "127.0.0.1:2", "127.0.0.1:3",
            "t", "v0.0.0", std::process::id(), 9222,
        ).unwrap();

        unsafe {
            // Read DACL back.
            let mut wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let mut sd: *mut c_void = std::ptr::null_mut();
            let result = GetNamedSecurityInfoW(
                wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            );
            assert_eq!(result, 0, "GetNamedSecurityInfoW failed: {}", result);
            assert!(!dacl.is_null(), "DACL should be present");

            // Assert exactly one ACE.
            let mut sz: ACL_SIZE_INFORMATION = std::mem::zeroed();
            assert_ne!(
                GetAclInformation(
                    dacl,
                    &mut sz as *mut _ as *mut c_void,
                    std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                    AclSizeInformation,
                ),
                0,
                "GetAclInformation failed",
            );
            assert_eq!(sz.AceCount, 1, "expected exactly one ACE in DACL, got {}", sz.AceCount);

            // Read the ACE and extract its SID.
            let mut ace_ptr: *mut c_void = std::ptr::null_mut();
            assert_ne!(
                GetAce(dacl, 0, &mut ace_ptr),
                0,
                "GetAce failed",
            );
            let ace = ace_ptr as *const ACCESS_ALLOWED_ACE;
            let ace_sid = &(*ace).SidStart as *const u32 as *mut c_void;

            // Get current-user SID for comparison.
            let mut token: HANDLE = std::ptr::null_mut();
            assert_ne!(
                OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token),
                0,
            );
            let mut needed = 0u32;
            let _ = GetTokenInformation(
                token, TokenUser, std::ptr::null_mut(), 0, &mut needed,
            );
            let mut buf = vec![0u8; needed as usize];
            assert_ne!(
                GetTokenInformation(
                    token, TokenUser,
                    buf.as_mut_ptr() as *mut c_void, needed, &mut needed,
                ),
                0,
            );
            CloseHandle(token);
            let token_user = buf.as_ptr() as *const TOKEN_USER;
            let user_sid = (*token_user).User.Sid;

            assert_ne!(
                EqualSid(ace_sid, user_sid),
                0,
                "ACE SID does not match current user SID",
            );

            LocalFree(sd);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// A second, separately-named test module: the one above is gated on
// `feature = "test-authfile"` (its tests touch the real OS data dir), and with
// that feature enabled both `mod tests` would be active at once — E0428, the
// name `tests` defined twice. This test needs no feature because it writes to
// a temp dir, so it must not be folded into that module either or it would
// stop running by default.
#[cfg(test)]
mod debug_port_tests {
    use super::*;

    /// Tooling reads the CDP port from this file. If it is absent, every
    /// consumer falls back to the 9222/9223 constant — which names whichever
    /// instance won the race for it, not the one the caller meant. That is how
    /// an agent attached a debugger to somebody else's running instance.
    #[test]
    fn the_authfile_publishes_the_debug_port() {
        let tmp = std::env::temp_dir().join(format!("am-authfile-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let path = write_dev_auth_file(
            &tmp, "key", "127.0.0.1:1", "127.0.0.1:2", "127.0.0.1:3", "tok", "v0.0.0", 4242,
            // Not 9222: the point is that the ACTUAL bound port is published.
            42149,
        )
        .expect("write");

        let body = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["debug_port"], 42149);
        assert_eq!(v["host_pid"], 4242);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    fn tmp(label: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("am-discovery-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn publish(is_dev: bool, dir: &Path) -> Published {
        publish_discovery(
            is_dev, dir, "SECRET-KEY", "127.0.0.1:1", "127.0.0.1:2", "127.0.0.1:3", "SECRET-TOKEN", "v0.0.0", 4242,
            42149,
        )
        .expect("publish")
    }

    #[test]
    fn a_non_dev_build_writes_no_key_and_no_token() {
        let dir = tmp("nodev");
        let out = publish(false, &dir);
        assert_eq!(out.path, dir.join("instance.json"));
        assert!(!dir.join("authkey.dev").exists());
        let body = std::fs::read_to_string(&out.path).unwrap();
        assert!(!body.contains("SECRET"), "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["debug_port"], 42149);
        assert_eq!(v["host_pid"], 4242);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_non_dev_build_removes_a_leftover_file_from_an_older_build() {
        let dir = tmp("leftover");
        std::fs::write(dir.join("authkey.dev"), "{\"auth_key\":\"old\"}").unwrap();
        let out = publish(false, &dir);
        assert!(out.removed_leftover);
        assert!(!dir.join("authkey.dev").exists());
        assert!(!publish(false, &dir).removed_leftover, "nothing left to remove the second time");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dev_build_still_writes_the_full_file() {
        let dir = tmp("dev");
        let out = publish(true, &dir);
        assert_eq!(out.path, dir.join("authkey.dev"));
        assert!(!out.removed_leftover);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out.path).unwrap()).unwrap();
        assert_eq!(v["auth_key"], "SECRET-KEY");
        assert_eq!(v["ipc_token"], "SECRET-TOKEN");
        assert!(!dir.join("instance.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A field added to what a non-dev build publishes must be a deliberate choice: this list is the whole shape,
    /// and none of its names may read as a credential.
    #[test]
    fn the_non_dev_shape_has_only_these_fields() {
        let info = InstanceInfo {
            version: 1,
            web_endpoint: "",
            ws_endpoint: "",
            instance: "",
            data_dir: String::new(),
            host_pid: 0,
            debug_port: 0,
            created_at: String::new(),
        };
        let v = serde_json::to_value(&info).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            ["created_at", "data_dir", "debug_port", "host_pid", "instance", "version", "web_endpoint", "ws_endpoint"]
        );
        for k in keys {
            for word in ["key", "token", "secret", "auth", "password"] {
                assert!(!k.contains(word), "{k} reads as a credential");
            }
        }
    }
}
