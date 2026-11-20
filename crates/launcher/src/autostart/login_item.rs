// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! macOS: start at login through `SMAppService`, so the entry shows in System Settings -> General -> Login Items &
//! Extensions where people look for it, and respects the approval they give there
//! (`docs/specs/SPEC_START_WITH_OS_2026_09_25.md` §3.7, phase 4).
//!
//! ## What is registered, and why not `SMAppService.mainApp`
//!
//! A bundle that ships `Contents/Library/LaunchAgents/<label>.plist` registers that **agent**
//! (`SMAppService.agent(plistName:)`). The plist's job is the launcher itself with `--login-start`: a shim that, at
//! login, opens the app through LaunchServices (`open <app> --args --background`) unless it already runs, then exits.
//!
//! - `mainApp` cannot carry `--background`, which is what makes a login start a tray start with no window.
//! - An agent whose job *is* AgentMux dies when the entry is switched off: unregistering boots the job out and
//!   launchd stops its process (checked on macOS 26.5, 2026-11-20). With the shim, the running app is a LaunchServices
//!   child, not the job, so Settings can switch the entry off from inside the app that login started.
//! - A process started by `launchd` rather than LaunchServices is refused by `UNUserNotificationCenter` ("notifications
//!   are not allowed", with no prompt; checked on macOS 26.5). Going through `open` keeps the login-started app able
//!   to notify.
//! - `BundleProgram` must be a signed Mach-O inside the bundle: a shell script fails launchd's launch constraints.
//!
//! The plist ships only in a bundle whose identifier is stable across versions (`scripts/package-macos.sh`); a bundle
//! without it keeps writing the LaunchAgent plist of `autostart/mod.rs`, so nothing changes for dev, test or
//! per-version builds. Registering an entry replaces that LaunchAgent plist (no double start, no orphan).

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::{Path, PathBuf};

/// What the launcher is asked to do by the login job: start AgentMux through LaunchServices if it is not running.
pub const LOGIN_START_FLAG: &str = "--login-start";

/// `--open-login-items`: open System Settings at Login Items (the page where the person approves AgentMux).
pub const OPEN_LOGIN_ITEMS_FLAG: &str = "--open-login-items";

/// The folder inside an app bundle that holds a registrable agent plist.
pub const AGENT_PLIST_DIR: &str = "Contents/Library/LaunchAgents";

/// `SMAppService.Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    NotRegistered,
    Enabled,
    /// Registered, but the person has not allowed it (or switched it off) in System Settings.
    RequiresApproval,
    /// The system has no such service: the plist is not where the bundle says, or the bundle was not found.
    NotFound,
}

/// The raw `SMAppService.Status` (`notRegistered` 0, `enabled` 1, `requiresApproval` 2, `notFound` 3).
pub fn registration_from_raw(raw: i64) -> Option<Registration> {
    match raw {
        0 => Some(Registration::NotRegistered),
        1 => Some(Registration::Enabled),
        2 => Some(Registration::RequiresApproval),
        3 => Some(Registration::NotFound),
        _ => None,
    }
}

impl Registration {
    /// Whether the system will start the entry at login, or would once the person approves it: the registration
    /// exists, so it is "on" as far as a second registration or a read of the entry is concerned.
    pub fn is_registered(self) -> bool {
        matches!(self, Registration::Enabled | Registration::RequiresApproval)
    }
}

/// The file name of the agent plist a bundle ships, when it ships exactly one. More than one is a packaging mistake
/// (which would the system start?), so it counts as none.
pub fn agent_plist_in(names: &[String]) -> Option<&str> {
    let mut plists = names.iter().filter(|n| n.ends_with(".plist") && !n.starts_with('.'));
    let first = plists.next()?;
    if plists.next().is_some() {
        return None;
    }
    Some(first.as_str())
}

/// The agent plist the bundle that holds `app` ships: its file name, if it ships exactly one.
fn bundled_agent_plist(app: &Path) -> Option<String> {
    let names: Vec<String> = std::fs::read_dir(app.join(AGENT_PLIST_DIR))
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    agent_plist_in(&names).map(str::to_string)
}

/// The app bundle and agent plist name this launcher registers: set when this launcher runs from an app bundle that
/// ships an agent plist and the system has `SMAppService` (macOS 13). `None` everywhere else.
pub fn bundle() -> Option<(PathBuf, String)> {
    static BUNDLE: std::sync::OnceLock<Option<(PathBuf, String)>> = std::sync::OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            if !cfg!(target_os = "macos") || !platform::service_available() {
                return None;
            }
            let app = agentmux_common::runtime_mode::app_bundle_of(&std::env::current_exe().ok()?)?;
            let plist = bundled_agent_plist(&app)?;
            Some((app, plist))
        })
        .clone()
}

/// Whether start at login goes through `SMAppService` in this launcher.
pub fn available() -> bool {
    bundle().is_some()
}

/// The registration as the system has it. `None` when [`available`] is false or the system does not answer.
pub fn registration() -> Option<Registration> {
    let (_, plist) = bundle()?;
    platform::status(&plist)
}

/// Register the agent. Re-registering one that is registered is not an error.
pub fn register() -> Result<(), String> {
    let (_, plist) = bundle().ok_or_else(|| "this build has no login item".to_string())?;
    platform::register(&plist)
}

/// Remove the registration. Succeeds when there is none.
pub fn unregister() -> Result<(), String> {
    let Some((_, plist)) = bundle() else { return Ok(()) };
    match platform::status(&plist) {
        Some(Registration::NotRegistered | Registration::NotFound) => Ok(()),
        _ => platform::unregister(&plist),
    }
}

/// Open System Settings at Login Items, where the person allows AgentMux (status `RequiresApproval`).
pub fn open_settings() -> Result<(), String> {
    platform::open_settings()
}

/// What the login job does: nothing when this app already runs (a registration made while it is open, or a second
/// login item), otherwise open it through LaunchServices in background mode and leave. Returns the exit code.
pub fn login_start() -> i32 {
    let Some(app) = agentmux_common::runtime_mode::app_bundle_of(&std::env::current_exe().unwrap_or_default()) else {
        return 2;
    };
    if platform::app_is_running() {
        crate::log("login start: AgentMux is already running; nothing to do");
        return 0;
    }
    crate::log(&format!("login start: opening {} in background mode", app.display()));
    match std::process::Command::new("/usr/bin/open").arg(&app).arg("--args").arg(crate::autostart::BACKGROUND_FLAG).status() {
        Ok(s) if s.success() => 0,
        Ok(s) => {
            crate::log(&format!("login start: open exited with {s}"));
            1
        }
        Err(e) => {
            crate::log(&format!("login start: could not run open: {e}"));
            1
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Registration;
    use std::os::raw::c_char;

    type Id = *mut std::ffi::c_void;
    type Sel = *mut std::ffi::c_void;

    #[link(name = "ServiceManagement", kind = "framework")]
    #[link(name = "AppKit", kind = "framework")]
    #[link(name = "Foundation", kind = "framework")]
    #[link(name = "objc")]
    extern "C" {
        // The same signatures as `splash_mac.rs` declares (the linker sees one symbol).
        fn objc_getClass(name: *const c_char) -> *const std::ffi::c_void;
        fn sel_registerName(name: *const c_char) -> *const std::ffi::c_void;
        fn objc_msgSend();
    }

    unsafe fn class(name: &[u8]) -> Id {
        objc_getClass(name.as_ptr() as *const c_char) as Id
    }
    unsafe fn sel(name: &[u8]) -> Sel {
        sel_registerName(name.as_ptr() as *const c_char) as Sel
    }
    unsafe fn send(recv: Id, s: Sel) -> Id {
        let f: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
        f(recv, s)
    }
    unsafe fn send_id(recv: Id, s: Sel, a: Id) -> Id {
        let f: extern "C" fn(Id, Sel, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
        f(recv, s, a)
    }
    unsafe fn send_i64(recv: Id, s: Sel) -> i64 {
        let f: extern "C" fn(Id, Sel) -> i64 = std::mem::transmute(objc_msgSend as *const ());
        f(recv, s)
    }
    unsafe fn send_usize(recv: Id, s: Sel) -> usize {
        let f: extern "C" fn(Id, Sel) -> usize = std::mem::transmute(objc_msgSend as *const ());
        f(recv, s)
    }
    unsafe fn send_err(recv: Id, s: Sel, err: *mut Id) -> u8 {
        let f: extern "C" fn(Id, Sel, *mut Id) -> u8 = std::mem::transmute(objc_msgSend as *const ());
        f(recv, s, err)
    }
    unsafe fn send_void(recv: Id, s: Sel) {
        let f: extern "C" fn(Id, Sel) = std::mem::transmute(objc_msgSend as *const ());
        f(recv, s)
    }

    unsafe fn nsstring(s: &str) -> Id {
        // The text is ours (a plist file name); a NUL inside it would cut it short, so it is rejected before.
        let c = std::ffi::CString::new(s.replace('\0', "")).unwrap_or_default();
        let f: extern "C" fn(Id, Sel, *const c_char) -> Id = std::mem::transmute(objc_msgSend as *const ());
        f(class(b"NSString\0"), sel(b"stringWithUTF8String:\0"), c.as_ptr())
    }

    unsafe fn rust_string(ns: Id) -> String {
        if ns.is_null() {
            return String::new();
        }
        let f: extern "C" fn(Id, Sel) -> *const c_char = std::mem::transmute(objc_msgSend as *const ());
        let p = f(ns, sel(b"UTF8String\0"));
        if p.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }

    /// Run `body` inside an autorelease pool: the launcher's threads that call this have none of their own.
    fn pooled<T>(body: impl FnOnce() -> T) -> T {
        unsafe {
            let pool = send(send(class(b"NSAutoreleasePool\0"), sel(b"alloc\0")), sel(b"init\0"));
            let out = body();
            send_void(pool, sel(b"drain\0"));
            out
        }
    }

    /// `SMAppService` exists from macOS 13; before that the class is not there.
    pub fn service_available() -> bool {
        unsafe { !class(b"SMAppService\0").is_null() }
    }

    unsafe fn service(plist: &str) -> Id {
        send_id(class(b"SMAppService\0"), sel(b"agentServiceWithPlistName:\0"), nsstring(plist))
    }

    pub fn status(plist: &str) -> Option<Registration> {
        pooled(|| unsafe {
            let svc = service(plist);
            if svc.is_null() {
                return None;
            }
            super::registration_from_raw(send_i64(svc, sel(b"status\0")))
        })
    }

    pub fn register(plist: &str) -> Result<(), String> {
        change(plist, b"registerAndReturnError:\0", "register")
    }

    pub fn unregister(plist: &str) -> Result<(), String> {
        change(plist, b"unregisterAndReturnError:\0", "unregister")
    }

    fn change(plist: &str, verb: &'static [u8], what: &str) -> Result<(), String> {
        pooled(|| unsafe {
            let svc = service(plist);
            if svc.is_null() {
                return Err(format!("{what} the login item: the system has no such service"));
            }
            let mut err: Id = std::ptr::null_mut();
            if send_err(svc, sel(verb), &mut err) != 0 {
                return Ok(());
            }
            let why = if err.is_null() { "no reason given".to_string() } else { rust_string(send(err, sel(b"localizedDescription\0"))) };
            Err(format!("{what} the login item: {why}"))
        })
    }

    pub fn open_settings() -> Result<(), String> {
        pooled(|| unsafe {
            let cls = class(b"SMAppService\0");
            if cls.is_null() {
                return Err("this macOS cannot open the Login Items settings".to_string());
            }
            send_void(cls, sel(b"openSystemSettingsLoginItems\0"));
            Ok(())
        })
    }

    /// Whether an app with this bundle's identifier is running (this process excluded).
    pub fn app_is_running() -> bool {
        pooled(|| unsafe {
            let bundle = send(class(b"NSBundle\0"), sel(b"mainBundle\0"));
            let id = send(bundle, sel(b"bundleIdentifier\0"));
            if id.is_null() {
                return false;
            }
            let apps = send_id(class(b"NSRunningApplication\0"), sel(b"runningApplicationsWithBundleIdentifier:\0"), id);
            let me = std::process::id() as i64;
            let count = send_usize(apps, sel(b"count\0"));
            (0..count).any(|i| {
                let f: extern "C" fn(Id, Sel, usize) -> Id = std::mem::transmute(objc_msgSend as *const ());
                let app = f(apps, sel(b"objectAtIndex:\0"), i);
                send_i64(app, sel(b"processIdentifier\0")) != me
            })
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Registration;

    pub fn service_available() -> bool {
        false
    }
    pub fn status(_plist: &str) -> Option<Registration> {
        None
    }
    pub fn register(_plist: &str) -> Result<(), String> {
        Err("not macOS".to_string())
    }
    pub fn unregister(_plist: &str) -> Result<(), String> {
        Err("not macOS".to_string())
    }
    pub fn open_settings() -> Result<(), String> {
        Err("not macOS".to_string())
    }
    pub fn app_is_running() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_raw_status_maps_to_the_four_states_and_nothing_else() {
        assert_eq!(registration_from_raw(0), Some(Registration::NotRegistered));
        assert_eq!(registration_from_raw(1), Some(Registration::Enabled));
        assert_eq!(registration_from_raw(2), Some(Registration::RequiresApproval));
        assert_eq!(registration_from_raw(3), Some(Registration::NotFound));
        assert_eq!(registration_from_raw(4), None);
        assert_eq!(registration_from_raw(-1), None);
    }

    #[test]
    fn a_registration_waiting_for_approval_still_counts_as_registered() {
        assert!(Registration::Enabled.is_registered());
        assert!(Registration::RequiresApproval.is_registered());
        assert!(!Registration::NotRegistered.is_registered());
        assert!(!Registration::NotFound.is_registered());
    }

    #[test]
    fn exactly_one_plist_is_the_bundles_login_agent() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(agent_plist_in(&names(&["ai.agentmux.stable.login.plist"])), Some("ai.agentmux.stable.login.plist"));
        assert_eq!(agent_plist_in(&names(&[])), None);
        assert_eq!(agent_plist_in(&names(&["a.plist", "b.plist"])), None, "two would leave the system to choose");
        assert_eq!(agent_plist_in(&names(&[".DS_Store", "readme.txt"])), None);
        assert_eq!(agent_plist_in(&names(&[".hidden.plist", "a.plist"])), Some("a.plist"));
    }

    #[test]
    fn a_bundle_without_the_plist_has_no_login_item() {
        let dir = std::env::temp_dir().join(format!("agentmux-login-item-test-{}", std::process::id()));
        let app = dir.join("AgentMux.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        assert_eq!(bundled_agent_plist(&app), None);
        std::fs::create_dir_all(app.join(AGENT_PLIST_DIR)).unwrap();
        assert_eq!(bundled_agent_plist(&app), None, "an empty folder is no plist");
        std::fs::write(app.join(AGENT_PLIST_DIR).join("ai.agentmux.stable.login.plist"), "x").unwrap();
        assert_eq!(bundled_agent_plist(&app).as_deref(), Some("ai.agentmux.stable.login.plist"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
