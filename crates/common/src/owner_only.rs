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
fn say_who(principal: &str) -> Other {
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

/// The words to a person whose folder lets others in: which folder, who, and the one command that fixes it: it stops what
/// is inherited from above, takes away each of those named, and gives the folder to the person and the system.
pub fn refusal(folder: &Path, others: &[Other], user: &str) -> String {
    let names: Vec<&str> = others.iter().map(|o| o.name.as_str()).collect();
    let removals: String = others.iter().map(|o| format!(" /remove:g \"*{}\"", o.sid)).collect();
    format!(
        "{} can be opened by {} as well as you. AgentMux Headless keeps your agents' data and its sign-in key there, so only you may. To fix it, run: icacls \"{}\" /inheritance:r{removals} /grant:r \"{user}:(OI)(CI)F\" /grant:r \"SYSTEM:(OI)(CI)F\"",
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
        GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        TOKEN_QUERY, TOKEN_USER,
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
        let user = user_sid().ok_or_else(|| "cannot find your Windows account".to_string())?;
        let sddl: Vec<u16> = super::private_sddl(&user, path.is_dir()).encode_utf16().chain(std::iter::once(0)).collect();
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
                return Err(format!("cannot limit {} to you (Windows error {code})", path.display()));
            }
        }
        Ok(())
    }

    /// Who besides the user, the system and the administrators may open this file or folder.
    pub fn others_who_can_open(path: &Path) -> Result<Vec<super::Other>, String> {
        let user = user_sid().ok_or_else(|| "cannot find your Windows account".to_string())?;
        Ok(super::others_with_access(&sddl_of(path)?, &user))
    }
}

#[cfg(windows)]
pub use windows::{others_who_can_open, restrict_to_user, sddl_of, user_sid};

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "S-1-5-21-1111-2222-3333-1001";

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
