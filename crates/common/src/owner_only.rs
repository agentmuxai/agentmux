// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files and folders only their own user may open: the place AgentMux Headless keeps its data and the key file that signs
//! a browser in. On Linux and macOS that is a file mode (0700, 0600). Windows has access lists instead, and a profile
//! folder can have been changed so that others are in them; this module reads a folder's list, says who besides the
//! user, the system and the administrators is given access, and sets a list that gives it to the user and the system only.
//!
//! The reading of the list is plain text (its SDDL form), so it is the same code, and the same tests, on every platform.

use std::path::Path;

/// Who may be in a list besides the user: the system, the administrators, and the owner (as aliases and as SIDs). The
/// administrators can always take a file they are denied, so listing them says nothing a check can use.
const ALWAYS_ALLOWED: [&str; 8] = ["SY", "BA", "OW", "CO", "S-1-5-18", "S-1-5-32-544", "S-1-3-0", "S-1-3-4"];

/// Someone a list lets in besides the user: what a person reads for them, and their SID, which is what removes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Other {
    pub name: String,
    pub sid: String,
}

/// What a person reads for the principals a list can name by alias, and the SID of each; any other is its own SID.
pub fn say_who(principal: &str) -> Other {
    let (name, sid) = match principal {
        "WD" | "S-1-1-0" => ("Everyone", "S-1-1-0"),
        "BU" | "S-1-5-32-545" => ("all Users of this computer", "S-1-5-32-545"),
        "AU" | "S-1-5-11" => ("Authenticated Users", "S-1-5-11"),
        "IU" | "S-1-5-4" => ("interactive users", "S-1-5-4"),
        "NU" | "S-1-5-2" => ("network users", "S-1-5-2"),
        "AN" | "S-1-5-7" => ("anonymous users", "S-1-5-7"),
        "LG" => ("Guest", "S-1-5-32-546"),
        other => (other, other),
    };
    Other { name: name.to_string(), sid: sid.to_string() }
}

/// Who besides `user_sid` (and the system, the administrators and the owner) the list in `sddl` allows to do anything
/// to the object, each named once, in the order the list gives them. A denial is not an allowance, and an entry that
/// gives no rights gives none. An empty answer is a list only the user (and those) can use.
pub fn others_with_access(sddl: &str, user_sid: &str) -> Vec<Other> {
    // An object with no list at all (a null list, which SDDL writes `D:NO_ACCESS_CONTROL` or leaves out) lets everyone in;
    // an empty list (`D:`) lets nobody in.
    let Some(dacl) = sddl.find("D:").map(|at| &sddl[at + 2..]) else { return vec![say_who("WD")] };
    let dacl = dacl.split("S:").next().unwrap_or("");
    if dacl.contains("NO_ACCESS_CONTROL") {
        return vec![say_who("WD")];
    }
    let mut out: Vec<Other> = Vec::new();
    for entry in dacl.split('(').skip(1) {
        let body = entry.split(')').next().unwrap_or("");
        let fields: Vec<&str> = body.split(';').collect();
        let (kind, rights, principal) = (fields.first().copied().unwrap_or(""), fields.get(2).copied().unwrap_or(""), fields.get(5).copied().unwrap_or(""));
        // `A` allowed, `OA` allowed for an object type, `XA` allowed with a condition, `ZA` a compound allowance.
        let allows = matches!(kind, "A" | "OA" | "XA" | "ZA");
        if !allows || rights.is_empty() || principal.is_empty() {
            continue;
        }
        // SDDL writes the built-in administrator account (the user whose SID ends in -500) as `LA`, so that is the user too.
        let is_the_user = principal.eq_ignore_ascii_case(user_sid) || (principal.eq_ignore_ascii_case("LA") && user_sid.ends_with("-500"));
        if is_the_user || ALWAYS_ALLOWED.iter().any(|ok| ok.eq_ignore_ascii_case(principal)) {
            continue;
        }
        let who = say_who(&principal.to_ascii_uppercase());
        if !out.contains(&who) {
            out.push(who);
        }
    }
    out
}

/// The access list as SDDL text, which the user is `user_sid`: only the user and the system, nothing inherited.
/// `folder` adds the rights to everything made inside it.
pub fn private_sddl(user_sid: &str, folder: bool) -> String {
    let inherit = if folder { "OICI" } else { "" };
    format!("D:P(A;{inherit};FA;;;{user_sid})(A;{inherit};FA;;;SY)")
}

/// [`private_sddl`] for a file that also names `readers` (SIDs) with read rights: the one file a service writes and a person
/// who is not its account must read (the key `agentmux open` signs a browser in with).
pub fn private_sddl_with_readers(user_sid: &str, folder: bool, readers: &[String]) -> String {
    let mut out = private_sddl(user_sid, folder);
    let inherit = if folder { "OICI" } else { "" };
    for reader in readers {
        out.push_str(&format!("(A;{inherit};{};;;{reader})", if folder { "0x1200a9" } else { "FR" }));
    }
    out
}

/// The accounts, besides the server's own, that may read its key file: the Windows service's installing person
/// (`agentmux service install` names them, `service run` passes them on in this variable), who signs a browser in with
/// `agentmux open` from their own terminal.
pub const KEY_READERS_ENV: &str = "AGENTMUX_KEY_READER_SIDS";

/// [`KEY_READERS_ENV`], as SIDs.
pub fn key_readers() -> Vec<String> {
    key_readers_of(std::env::var(KEY_READERS_ENV).ok().as_deref())
}

/// The accounts in a comma-separated value. Whitespace and empty entries are dropped, and so is anything that is not the SID of a
/// person's own account, so a value another program put there cannot be spliced into an access list.
fn key_readers_of(value: Option<&str>) -> Vec<String> {
    value.unwrap_or("").split(',').map(str::trim).filter(|s| is_account_sid(s)).map(str::to_string).collect()
}

/// Is this the SID of one person's own account, as text: `S-1-5-21-<a>-<b>-<c>-<rid>` (a local or a domain account) or
/// `S-1-12-1-<a>-<b>-<c>-<d>` (an account of an organisation's directory), every part digits only? A group, a well-known SID
/// (Everyone, Authenticated Users, Users, Anonymous, the system, the administrators) and a built-in account that nobody signs in
/// with (Guest and the like, relative ids 501 to 999) are not.
pub fn is_account_sid(sid: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.len() <= 10 && s.chars().all(|c| c.is_ascii_digit());
    if let Some(rest) = sid.strip_prefix("S-1-5-21-") {
        let parts: Vec<&str> = rest.split('-').collect();
        return parts.len() == 4 && parts.iter().all(|p| digits(p)) && parts[3].parse::<u64>().is_ok_and(|rid| rid == 500 || rid >= 1000);
    }
    if let Some(rest) = sid.strip_prefix("S-1-12-1-") {
        let parts: Vec<&str> = rest.split('-').collect();
        return parts.len() == 4 && parts.iter().all(|p| digits(p));
    }
    false
}

/// The words to a person whose folder lets others in: which folder, who, and the one command that fixes it: it stops what
/// is inherited from above, takes away each of those named, and gives the folder to the person and the system.
pub fn refusal(folder: &Path, others: &[Other], user: &str) -> String {
    refusal_because(folder, others, user, "AgentMux Headless keeps your agents' data and its sign-in key there, so only you may")
}

/// `refusal` for a place kept for another reason, which is said in `why` (a sentence without its closing full stop).
pub fn refusal_because(folder: &Path, others: &[Other], user: &str, why: &str) -> String {
    let names: Vec<&str> = others.iter().map(|o| o.name.as_str()).collect();
    let removals: String = others.iter().map(|o| format!(" /remove:g \"*{}\"", o.sid)).collect();
    format!(
        "{} can be opened by {} as well as you. {why}. To fix it, run: icacls \"{}\" /inheritance:r{removals} /grant:r \"{user}:(OI)(CI)F\" /grant:r \"SYSTEM:(OI)(CI)F\"",
        folder.display(),
        names.join(", "),
        folder.display()
    )
}

#[cfg(windows)]
mod windows {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, GetTokenInformation, LookupAccountNameW, TokenElevation, TokenUser, DACL_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// This process's user, as the text form of its SID.
    pub fn user_sid() -> Option<String> {
        // SAFETY: plain Win32 calls on this process's own token, closed on every path; the buffer is 8-byte aligned and as
        // long as the call asked for.
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            let got = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) != 0;
            CloseHandle(token);
            if !got {
                return None;
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut text: *mut u16 = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 || text.is_null() {
                return None;
            }
            let n = (0..).take_while(|&i| *text.add(i) != 0).count();
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
            LocalFree(text.cast());
            Some(sid)
        }
    }

    /// Is this process running with an administrator's full token (elevated)? A person who is an administrator but did not
    /// choose "Run as administrator" is not.
    pub fn is_elevated() -> bool {
        // SAFETY: this process's own token, closed on every path; the buffer is the size the call asked for.
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut len = 0u32;
            let got = GetTokenInformation(token, TokenElevation, (&mut elevation as *mut TOKEN_ELEVATION).cast(), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len) != 0;
            CloseHandle(token);
            got && elevation.TokenIsElevated != 0
        }
    }

    /// Does Windows say this SID is a user account (and not a group, an alias or anything else)?
    pub fn sid_is_user(sid: &str) -> bool {
        use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
        use windows_sys::Win32::Security::{LookupAccountSidW, SidTypeUser};
        let text: Vec<u16> = sid.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated text that outlives the calls; the SID Windows allocates is freed here on every path; the name
        // buffers are the sizes the first call asked for.
        unsafe {
            let mut raw: *mut std::ffi::c_void = null_mut();
            if ConvertStringSidToSidW(text.as_ptr(), &mut raw) == 0 || raw.is_null() {
                return false;
            }
            let (mut name_len, mut domain_len, mut kind) = (0u32, 0u32, 0i32);
            LookupAccountSidW(null_mut(), raw, null_mut(), &mut name_len, null_mut(), &mut domain_len, &mut kind);
            let mut name = vec![0u16; name_len.max(1) as usize];
            let mut domain = vec![0u16; domain_len.max(1) as usize];
            let found = LookupAccountSidW(null_mut(), raw, name.as_mut_ptr(), &mut name_len, domain.as_mut_ptr(), &mut domain_len, &mut kind) != 0;
            LocalFree(raw);
            found && kind == SidTypeUser
        }
    }

    /// The SID, as text, of an account Windows knows by name (`NT SERVICE\AgentMuxHeadless` is one once that service exists).
    pub fn account_sid(account: &str) -> Result<String, String> {
        let name: Vec<u16> = account.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated names that outlive the calls, buffers of the sizes the first call asked for, and a text
        // that is freed here on every path.
        unsafe {
            let (mut sid_len, mut domain_len, mut kind) = (0u32, 0u32, 0i32);
            LookupAccountNameW(null_mut(), name.as_ptr(), null_mut(), &mut sid_len, null_mut(), &mut domain_len, &mut kind);
            if sid_len == 0 {
                return Err(format!("Windows has no account named {account}"));
            }
            let mut sid = vec![0u64; (sid_len as usize).div_ceil(8)];
            let mut domain = vec![0u16; domain_len.max(1) as usize];
            if LookupAccountNameW(null_mut(), name.as_ptr(), sid.as_mut_ptr().cast(), &mut sid_len, domain.as_mut_ptr(), &mut domain_len, &mut kind) == 0 {
                return Err(format!("Windows has no account named {account}"));
            }
            let mut text: *mut u16 = null_mut();
            if ConvertSidToStringSidW(sid.as_mut_ptr().cast(), &mut text) == 0 || text.is_null() {
                return Err(format!("cannot write the SID of {account}"));
            }
            let n = (0..).take_while(|&i| *text.add(i) != 0).count();
            let out = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
            LocalFree(text.cast());
            Ok(out)
        }
    }

    /// The access list of a file or folder, as SDDL text.
    pub fn sddl_of(path: &Path) -> Result<String, String> {
        let name = wide(path);
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `name` is NUL-terminated and outlives the calls; the descriptor and the text are LocalAlloc'd and freed
        // here on every path.
        unsafe {
            let code = GetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), null_mut(), &mut sd);
            if code != 0 || sd.is_null() {
                return Err(format!("cannot read who may open {} (Windows error {code})", path.display()));
            }
            let mut text: *mut u16 = null_mut();
            let ok = ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, DACL_SECURITY_INFORMATION, &mut text, null_mut());
            LocalFree(sd);
            if ok == 0 || text.is_null() {
                return Err(format!("cannot read who may open {}", path.display()));
            }
            let n = (0..).take_while(|&i| *text.add(i) != 0).count();
            let out = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
            LocalFree(text.cast());
            Ok(out)
        }
    }

    /// Give a file or folder an access list of the user and the system only, with nothing inherited from above; a folder's
    /// list is what everything made inside it inherits.
    pub fn restrict_to_user(path: &Path) -> Result<(), String> {
        restrict_to_user_and_readers(path, &[])
    }

    /// [`restrict_to_user`], and the accounts in `readers` (SIDs) may read it too.
    pub fn restrict_to_user_and_readers(path: &Path, readers: &[String]) -> Result<(), String> {
        let user = user_sid().ok_or_else(|| "cannot find your Windows account".to_string())?;
        apply_sddl(path, &super::private_sddl_with_readers(&user, path.is_dir(), readers))
    }

    /// Replace a file or folder's access list with `sddl` (its DACL part), protected: nothing is inherited from above.
    pub fn apply_sddl(path: &Path, sddl: &str) -> Result<(), String> {
        let sddl: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut name = wide(path);
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: both strings are NUL-terminated and outlive the calls; the descriptor is LocalAlloc'd and freed on every path; the
        // list pointed into it is only used before that.
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) == 0 || sd.is_null() {
                return Err("cannot make the access list".to_string());
            }
            let (mut present, mut defaulted) = (0i32, 0i32);
            let mut dacl = null_mut();
            let got = GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) != 0 && present != 0 && !dacl.is_null();
            let code = if got {
                SetNamedSecurityInfoW(name.as_mut_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, null_mut(), null_mut(), dacl, null_mut())
            } else {
                1
            };
            LocalFree(sd);
            if code != 0 {
                return Err(format!("cannot set who may open {} (Windows error {code})", path.display()));
            }
        }
        Ok(())
    }

    /// Make a folder that has the access list `sddl` from the moment it exists, so nobody can put anything in it before the
    /// list is set. Fails if the folder is already there (check `ErrorKind::AlreadyExists`).
    pub fn create_dir_with_sddl(path: &Path, sddl: &str) -> std::io::Result<()> {
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
        let text: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let name = wide(path);
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: NUL-terminated strings that outlive the calls; the descriptor is LocalAlloc'd and freed on every path, after the
        // call that reads it.
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) == 0 || sd.is_null() {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "cannot make the access list"));
            }
            let attributes = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
            let made = CreateDirectoryW(name.as_ptr(), &attributes) != 0;
            let error = std::io::Error::last_os_error();
            LocalFree(sd);
            if made { Ok(()) } else { Err(error) }
        }
    }

    /// Turn on a privilege this process's token has (an administrator's does, switched off).
    fn enable_privilege(name: &str) -> bool {
        use windows_sys::Win32::Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES};
        let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: this process's own token, closed on every path; a NUL-terminated name that outlives the call.
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut privileges = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: std::mem::zeroed(), Attributes: SE_PRIVILEGE_ENABLED }] };
            let found = LookupPrivilegeValueW(null_mut(), name.as_ptr(), &mut privileges.Privileges[0].Luid) != 0;
            let done = found && AdjustTokenPrivileges(token, 0, &privileges, 0, null_mut(), null_mut()) != 0;
            CloseHandle(token);
            done
        }
    }

    /// Make the administrators the owner of a file or folder and give them (and the system) full control of it, whatever its
    /// list said before: what an administrator does to delete what a service made for its own account alone. Needs an elevated
    /// token (it uses the right to take ownership and to restore).
    ///
    /// The change is made through a handle opened on the entry itself, never on what a link points to: `Ok(false)` says the entry
    /// is a link (a junction or a symbolic link) and nothing was changed, so what an account that can still write there swaps in
    /// between a check and this call is not followed.
    pub fn take_over(path: &Path) -> Result<bool, String> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Security::Authorization::SetSecurityInfo;
        use windows_sys::Win32::Security::GetSecurityDescriptorOwner;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
        };
        let _ = enable_privilege("SeTakeOwnershipPrivilege");
        let _ = enable_privilege("SeRestorePrivilege");
        let sddl: Vec<u16> = "O:BAD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)".encode_utf16().chain(std::iter::once(0)).collect();
        let name = wide(path);
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: NUL-terminated strings that outlive the calls; the handle is owned by a `File` (closed on every path); the
        // descriptor is LocalAlloc'd, read only before it is freed, and freed on every path.
        unsafe {
            let handle = CreateFileW(
                name.as_ptr(),
                READ_CONTROL | WRITE_OWNER | WRITE_DAC,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(format!("cannot open {}: {}", path.display(), std::io::Error::last_os_error()));
            }
            let held = std::fs::File::from_raw_handle(handle as _);
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(handle, &mut info) == 0 {
                return Err(format!("cannot read {}: {}", path.display(), std::io::Error::last_os_error()));
            }
            if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Ok(false);
            }
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) == 0 || sd.is_null() {
                return Err("cannot make the access list".to_string());
            }
            let (mut owner, mut defaulted) = (null_mut(), 0i32);
            let (mut present, mut dacl) = (0i32, null_mut());
            let got = GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted) != 0 && !owner.is_null() && GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) != 0 && present != 0;
            let code = if got {
                SetSecurityInfo(handle, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, owner, null_mut(), dacl, null_mut())
            } else {
                1
            };
            LocalFree(sd);
            drop(held);
            if code != 0 {
                return Err(format!("cannot take over {} (Windows error {code})", path.display()));
            }
        }
        Ok(true)
    }

    /// The owner and the access list of a file or folder, as SDDL text (`O:...D:...`).
    pub fn owner_and_sddl_of(path: &Path) -> Result<String, String> {
        let name = wide(path);
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: as `sddl_of`: a NUL-terminated name, and a descriptor and a text that are freed here on every path.
        unsafe {
            let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
            let code = GetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, info, null_mut(), null_mut(), null_mut(), null_mut(), &mut sd);
            if code != 0 || sd.is_null() {
                return Err(format!("cannot read who owns {} (Windows error {code})", path.display()));
            }
            let mut text: *mut u16 = null_mut();
            let ok = ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, info, &mut text, null_mut());
            LocalFree(sd);
            if ok == 0 || text.is_null() {
                return Err(format!("cannot read who owns {}", path.display()));
            }
            let n = (0..).take_while(|&i| *text.add(i) != 0).count();
            let out = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
            LocalFree(text.cast());
            Ok(out)
        }
    }

    /// Who besides the user, the system and the administrators may open this file or folder.
    pub fn others_who_can_open(path: &Path) -> Result<Vec<super::Other>, String> {
        let user = user_sid().ok_or_else(|| "cannot find your Windows account".to_string())?;
        Ok(super::others_with_access(&sddl_of(path)?, &user))
    }
}

#[cfg(windows)]
pub use windows::{account_sid, apply_sddl, create_dir_with_sddl, is_elevated, sid_is_user, take_over, others_who_can_open, owner_and_sddl_of, restrict_to_user, restrict_to_user_and_readers, sddl_of, user_sid};

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "S-1-5-21-1111-2222-3333-1001";

    #[test]
    fn only_sids_are_taken_as_the_accounts_that_may_read_the_key() {
        assert!(key_readers_of(None).is_empty());
        assert_eq!(key_readers_of(Some(" S-1-5-21-1-2-3-1001 ,S-1-5-21-1-2-3-500,,")), vec!["S-1-5-21-1-2-3-1001", "S-1-5-21-1-2-3-500"]);
        // A name, a trailing entry, or anything else that is not a SID is dropped.
        assert!(key_readers_of(Some("Everyone,(A;;FA;;;WD),S-1-1-0)x,S-1-5-21-1;FA")).is_empty());
    }

    #[test]
    fn only_the_sid_of_a_persons_own_account_is_one_that_may_read_the_key() {
        for ok in ["S-1-5-21-3904905834-1961560920-3170238172-1001", "S-1-5-21-1-2-3-500", "S-1-5-21-1-2-3-1000", "S-1-12-1-1-2-3-4"] {
            assert!(is_account_sid(ok), "{ok}");
        }
        for (bad, what) in [
            ("S-1-1-0", "Everyone"),
            ("S-1-5-11", "Authenticated Users"),
            ("S-1-5-32-545", "Users"),
            ("S-1-5-32-544", "the administrators"),
            ("S-1-5-7", "Anonymous"),
            ("S-1-5-18", "the system"),
            ("S-1-5-4", "interactive users"),
            ("S-1-5-21-1-2-3-513", "Domain Users"),
            ("S-1-5-21-1-2-3-512", "Domain Admins"),
            ("S-1-5-21-1-2-3-501", "Guest"),
            ("S-1-5-21-1-2-3", "a domain, no account"),
            ("S-1-5-21-1-2-3-1001-5", "too many parts"),
            ("S-1-5-21-1-2-3-10a", "not digits"),
            ("S-1-5-21-1-2-3-+1001", "a sign"),
            ("S-1-12-1-1-2-3", "too few parts"),
            ("S-1-5-80-1-2-3-4-5", "a service"),
            ("Everyone", "a name"),
            ("", "nothing"),
        ] {
            assert!(!is_account_sid(bad), "{what}: {bad}");
        }
    }

    #[test]
    fn a_file_list_can_name_readers_and_a_folder_list_gives_them_read_and_traverse() {
        let readers = vec!["S-1-5-21-9-9-9-1001".to_string()];
        assert_eq!(private_sddl_with_readers(USER, false, &readers), format!("D:P(A;;FA;;;{USER})(A;;FA;;;SY)(A;;FR;;;S-1-5-21-9-9-9-1001)"));
        assert_eq!(private_sddl_with_readers(USER, true, &readers), format!("D:P(A;OICI;FA;;;{USER})(A;OICI;FA;;;SY)(A;OICI;0x1200a9;;;S-1-5-21-9-9-9-1001)"));
        assert_eq!(private_sddl_with_readers(USER, false, &[]), private_sddl(USER, false));
    }

    #[test]
    fn a_default_profile_list_has_nobody_else() {
        let sddl = format!("D:PAI(A;OICIID;FA;;;{USER})(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)");
        assert!(others_with_access(&sddl, USER).is_empty());
        // Owner and creator-owner entries, with the owner's rights empty, are the user's own.
        assert!(others_with_access(&format!("D:P(A;;FA;;;{USER})(A;;;;;OW)(A;OICIIO;FA;;;CO)"), USER).is_empty());
    }

    #[test]
    fn everyone_and_the_other_accounts_of_the_computer_are_named() {
        let sddl = format!("D:AI(A;OICIID;FA;;;{USER})(A;OICIID;0x1200a9;;;WD)(A;OICIID;FR;;;BU)(A;OICIID;FA;;;S-1-5-21-1111-2222-3333-1002)");
        let names = |sddl: &str| others_with_access(sddl, USER).into_iter().map(|o| o.name).collect::<Vec<_>>();
        assert_eq!(names(&sddl), vec!["Everyone", "all Users of this computer", "S-1-5-21-1111-2222-3333-1002"]);
        assert_eq!(names("D:(A;;GR;;;AU)(A;;GR;;;AU)"), vec!["Authenticated Users"], "each is named once");
        assert_eq!(others_with_access("D:(A;;GR;;;WD)", USER), vec![Other { name: "Everyone".into(), sid: "S-1-1-0".into() }]);
    }

    #[test]
    fn a_denial_an_empty_entry_and_an_audit_entry_give_nobody_access() {
        let sddl = format!("D:P(D;;FA;;;WD)(A;;;;;WD)(A;;FA;;;{USER})S:(AU;SA;FA;;;WD)");
        assert!(others_with_access(&sddl, USER).is_empty(), "{:?}", others_with_access(&sddl, USER));
        assert!(others_with_access("D:", USER).is_empty(), "an empty list lets nobody in");
        assert!(others_with_access("D:P", USER).is_empty());
    }

    /// SDDL writes the built-in administrator account (RID 500) as `LA`: for that user it is the user, for anyone else it is not.
    #[test]
    fn the_built_in_administrator_is_the_user_only_when_it_is_the_user() {
        let admin = "S-1-5-21-1111-2222-3333-500";
        assert!(others_with_access("D:PAI(A;OICI;FA;;;LA)(A;OICI;FA;;;SY)", admin).is_empty());
        let found = others_with_access("D:P(A;OICI;FA;;;LA)(A;OICI;FA;;;SY)", USER);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].sid, "LA");
    }

    /// An object with no access list at all is open to everyone, and is read as such.
    #[test]
    fn a_missing_access_list_is_open_to_everyone() {
        let everyone = vec![Other { name: "Everyone".into(), sid: "S-1-1-0".into() }];
        for sddl in ["", "O:BAG:BA", "D:NO_ACCESS_CONTROL", "D:P(A;;FA;;;SY)NO_ACCESS_CONTROL"] {
            assert_eq!(others_with_access(sddl, USER), everyone, "{sddl:?}");
        }
    }

    #[test]
    fn the_users_own_sid_matches_whatever_its_case() {
        assert!(others_with_access(&format!("D:P(A;;FA;;;{})", USER.to_lowercase()), USER).is_empty());
    }

    #[test]
    fn the_private_list_is_the_user_and_the_system_and_nothing_inherited() {
        let folder = private_sddl(USER, true);
        assert_eq!(folder, format!("D:P(A;OICI;FA;;;{USER})(A;OICI;FA;;;SY)"));
        assert_eq!(private_sddl(USER, false), format!("D:P(A;;FA;;;{USER})(A;;FA;;;SY)"));
        assert!(others_with_access(&folder, USER).is_empty());
    }

    #[test]
    fn the_refusal_names_the_folder_who_and_the_command() {
        let others = others_with_access("D:(A;;GR;;;WD)(A;;GR;;;AU)", USER);
        let said = refusal(Path::new(r"C:\Users\sam\.agentmux\channels\headless"), &others, r"HOME\sam");
        assert!(said.contains(r"C:\Users\sam\.agentmux\channels\headless") && said.contains("Everyone, Authenticated Users"), "{said}");
        assert!(said.contains(r#"icacls "C:\Users\sam\.agentmux\channels\headless" /inheritance:r /remove:g "*S-1-1-0" /remove:g "*S-1-5-11" /grant:r "HOME\sam:(OI)(CI)F" /grant:r "SYSTEM:(OI)(CI)F""#), "{said}");
    }

    /// Windows: a folder is read as private, then given to Everyone and read as not, then limited to the user and read as
    /// private again, and what is made inside it afterwards is private too.
    #[cfg(windows)]
    #[test]
    fn a_folder_that_lets_others_in_is_found_and_made_private() {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION};
        use std::os::windows::ffi::OsStrExt;

        let outer = tempfile::tempdir().unwrap();
        let dir = outer.path().join("channel");
        std::fs::create_dir(&dir).unwrap();
        let user = user_sid().expect("a user");
        restrict_to_user(&dir).unwrap();
        assert!(others_who_can_open(&dir).unwrap().is_empty(), "{}", sddl_of(&dir).unwrap());

        // What is made inside a private folder is private: the folder's list is inherited.
        let inner = dir.join("data");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(inner.join("srv-auth-key"), b"k").unwrap();
        assert!(others_who_can_open(&inner).unwrap().is_empty(), "{}", sddl_of(&inner).unwrap());
        assert!(others_who_can_open(&inner.join("srv-auth-key")).unwrap().is_empty(), "{}", sddl_of(&inner.join("srv-auth-key")).unwrap());

        // Open it to Everyone (the damage the check is for).
        let open: Vec<u16> = format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FR;;;WD)").encode_utf16().chain(std::iter::once(0)).collect();
        let mut name: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated strings that outlive the calls; the descriptor is freed after the list is used.
        unsafe {
            let mut sd = std::ptr::null_mut();
            assert_ne!(ConvertStringSecurityDescriptorToSecurityDescriptorW(open.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()), 0);
            let (mut present, mut defaulted, mut dacl) = (0i32, 0i32, std::ptr::null_mut());
            assert_ne!(GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted), 0);
            let code = SetNamedSecurityInfoW(name.as_mut_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, std::ptr::null_mut(), std::ptr::null_mut(), dacl, std::ptr::null_mut());
            LocalFree(sd);
            assert_eq!(code, 0);
        }
        assert_eq!(others_who_can_open(&dir).unwrap().into_iter().map(|o| o.name).collect::<Vec<_>>(), vec!["Everyone".to_string()]);

        restrict_to_user(&dir).unwrap();
        assert!(others_who_can_open(&dir).unwrap().is_empty());
        // A file is limited to the user too.
        let key = outer.path().join("key");
        std::fs::write(&key, b"k").unwrap();
        restrict_to_user(&key).unwrap();
        assert!(others_who_can_open(&key).unwrap().is_empty(), "{}", sddl_of(&key).unwrap());
        // SDDL writes the built-in administrator account (RID 500, the CI runner's user) as `LA`.
        let user_in_sddl = if user.ends_with("-500") { "LA" } else { user.as_str() };
        assert!(!sddl_of(&key).unwrap().contains("WD") && sddl_of(&key).unwrap().contains(user_in_sddl), "{}", sddl_of(&key).unwrap());
    }
}
