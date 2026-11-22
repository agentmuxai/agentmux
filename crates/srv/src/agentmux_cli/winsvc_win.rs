// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the installer of the Windows service asks of Windows itself (docs/specs/SPEC_WINDOWS_HEADLESS_AS_A_SERVICE_2026_11_20.md):
//! the system's own program and data folders, whether this computer belongs to a domain, the elevated relaunch, and the service
//! manager (create, change, start, stop, query, delete). Each is a few calls; the decisions are in `winsvc_plan` and the order
//! of the steps in `winsvc_cli`.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType,
    ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use super::winservice;

/// `%ProgramFiles%` or `%ProgramData%`, as Windows says it: the environment of whoever runs this is not asked.
pub enum Known {
    ProgramFiles,
    ProgramData,
}

pub fn known_folder(which: Known) -> Result<PathBuf, String> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramData, FOLDERID_ProgramFiles, SHGetKnownFolderPath};
    let id = match which {
        Known::ProgramFiles => &FOLDERID_ProgramFiles,
        Known::ProgramData => &FOLDERID_ProgramData,
    };
    // SAFETY: a known-folder id that outlives the call; the text Windows allocates is copied and freed here.
    unsafe {
        let mut text: *mut u16 = std::ptr::null_mut();
        let code = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut text);
        if code != 0 || text.is_null() {
            return Err(format!("Windows did not say where its program folders are (error {code:#x})"));
        }
        let n = (0..).take_while(|&i| *text.add(i) != 0).count();
        let path = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        CoTaskMemFree(text.cast());
        Ok(PathBuf::from(path))
    }
}

/// Does this computer belong to a domain (and not to a workgroup, or to none)?
pub fn domain_joined() -> bool {
    use windows_sys::Win32::NetworkManagement::NetManagement::{NetApiBufferFree, NetGetJoinInformation, NetSetupDomainName};
    // SAFETY: the call fills a buffer it allocates, which is freed here whatever it said.
    unsafe {
        let (mut name, mut status) = (std::ptr::null_mut(), 0i32);
        let code = NetGetJoinInformation(std::ptr::null(), &mut name, &mut status);
        if !name.is_null() {
            NetApiBufferFree(name.cast());
        }
        code == 0 && status == NetSetupDomainName
    }
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// A command line, each argument quoted as Windows reads it.
fn command_line(args: &[String]) -> String {
    args.iter().map(|a| winservice::quote_arg(a)).collect::<Vec<_>>().join(" ")
}

/// Run `exe` with `args` as an administrator (the system asks the person, who answers), and wait for it. Returns its exit
/// code, or says the person declined.
pub fn run_elevated(exe: &Path, args: &[String]) -> Result<u32, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    const SW_SHOWNORMAL: i32 = 1;
    const ERROR_CANCELLED: u32 = 1223;
    let (verb, file, params) = (wide(OsStr::new("runas")), wide(exe.as_os_str()), wide(OsStr::new(&command_line(args))));
    // SAFETY: the three texts are NUL-terminated and outlive the call; the process handle it returns is waited on and closed.
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut info) == 0 {
            let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32;
            return Err(if code == ERROR_CANCELLED {
                "the administrator prompt was declined: nothing was changed".to_string()
            } else {
                format!("could not ask for an administrator (Windows error {code}): {}", super::winsvc_plan::NEEDS_ADMINISTRATOR)
            });
        }
        if info.hProcess.is_null() {
            return Err("the administrator step did not start".to_string());
        }
        let waited = WaitForSingleObject(info.hProcess, INFINITE);
        let mut exit = 1u32;
        let got = waited == WAIT_OBJECT_0 && GetExitCodeProcess(info.hProcess, &mut exit) != 0;
        CloseHandle(info.hProcess);
        if got { Ok(exit) } else { Err("could not tell how the administrator step ended".to_string()) }
    }
}

/// Give the service the environment it starts with: a list of `NAME=value`. The service manager reads it from the
/// service's own key, which only administrators can write.
pub fn set_service_environment(name: &str, vars: &[String]) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_MULTI_SZ};
    let key = wide(OsStr::new(&format!("SYSTEM\\CurrentControlSet\\Services\\{name}")));
    let value = wide(OsStr::new("Environment"));
    let mut data: Vec<u16> = Vec::new();
    for var in vars {
        data.extend(var.encode_utf16());
        data.push(0);
    }
    data.push(0);
    // SAFETY: NUL-terminated names and a buffer that outlive the calls; the key is closed on every path.
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        let code = RegOpenKeyExW(HKEY_LOCAL_MACHINE, key.as_ptr(), 0, KEY_SET_VALUE, &mut hkey);
        if code != 0 {
            return Err(format!("cannot open the service's key in the registry (Windows error {code})"));
        }
        let set = RegSetValueExW(hkey, value.as_ptr(), 0, REG_MULTI_SZ, data.as_ptr().cast(), (data.len() * 2) as u32);
        RegCloseKey(hkey);
        if set != 0 {
            return Err(format!("cannot set the service's environment (Windows error {set})"));
        }
    }
    Ok(())
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager, String> {
    ServiceManager::local_computer(None::<&str>, access).map_err(|e| format!("cannot reach the Windows service manager: {e}"))
}

/// Does a service of this name exist?
pub fn exists(name: &str) -> bool {
    open(name, ServiceAccess::QUERY_STATUS).is_ok()
}

/// The service named `name`, opened for what `access` asks.
pub fn open(name: &str, access: ServiceAccess) -> Result<Service, String> {
    manager(ServiceManagerAccess::CONNECT)?.open_service(name, access).map_err(|e| format!("cannot open the service {name}: {e}"))
}

/// What the service manager is told about the service: its program and what is run, as its own account.
fn definition(channel: &str, program: &Path, home: &Path, port: Option<u16>) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(winservice::service_name(channel)),
        display_name: OsString::from(winservice::display_name(channel)),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: program.to_path_buf(),
        launch_arguments: super::winsvc_plan::launch_arguments(channel, home, port).into_iter().map(OsString::from).collect(),
        dependencies: vec![],
        account_name: Some(OsString::from(winservice::virtual_account(channel))),
        account_password: None,
    }
}

/// Make the service, or bring one that is there to this definition. Returns it, and whether it was made now.
pub fn create_or_update(channel: &str, program: &Path, home: &Path, port: Option<u16>) -> Result<(Service, bool), String> {
    let name = winservice::service_name(channel);
    let info = definition(channel, program, home, port);
    let all = ServiceAccess::ALL_ACCESS;
    if let Ok(existing) = open(&name, all) {
        existing.change_config(&info).map_err(|e| format!("cannot change the service {name}: {e}"))?;
        return Ok((existing, false));
    }
    let made = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?
        .create_service(&info, all)
        .map_err(|e| format!("cannot make the service {name}: {e}"))?;
    Ok((made, true))
}

/// What the service is described as, and what happens when it fails: a restart after a minute, twice, and then nothing, so a
/// crash loop shows in the Event Log and is not a hot loop (C.5).
pub fn finish_definition(service: &Service) -> Result<(), String> {
    service.set_description("AgentMux Headless: the AgentMux server, started at boot and used from a browser on this computer.").map_err(|e| format!("cannot describe the service: {e}"))?;
    let restart = || ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(60) };
    let actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 3600)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart(), restart(), ServiceAction { action_type: ServiceActionType::None, delay: Duration::from_secs(0) }]),
    };
    service.update_failure_actions(actions).map_err(|e| format!("cannot set what happens when the service fails: {e}"))?;
    service.set_failure_actions_on_non_crash_failures(true).map_err(|e| format!("cannot set what happens when the service fails: {e}"))
}

/// Wait until the service is in `want`, up to `secs`. Returns the state it is in.
pub fn wait_for(service: &Service, want: ServiceState, secs: u64) -> Result<ServiceState, String> {
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        let state = service.query_status().map_err(|e| format!("cannot ask the service how it is: {e}"))?.current_state;
        if state == want || Instant::now() >= until {
            return Ok(state);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Start the service and wait for it to run. A service that is already running is left as it is.
pub fn start(service: &Service, secs: u64) -> Result<(), String> {
    if service.query_status().map(|s| s.current_state == ServiceState::Running).unwrap_or(false) {
        return Ok(());
    }
    service.start::<&OsStr>(&[]).map_err(|e| format!("cannot start the service: {e}"))?;
    match wait_for(service, ServiceState::Running, secs)? {
        ServiceState::Running => Ok(()),
        other => Err(format!("the service did not start in {secs} seconds (it is {})", state_word(other))),
    }
}

/// Stop the service and wait for it to be gone. A service that is stopped is left as it is.
pub fn stop(service: &Service, secs: u64) -> Result<(), String> {
    if service.query_status().map(|s| s.current_state == ServiceState::Stopped).unwrap_or(false) {
        return Ok(());
    }
    let _ = service.stop();
    match wait_for(service, ServiceState::Stopped, secs)? {
        ServiceState::Stopped => Ok(()),
        other => Err(format!("the service did not stop in {secs} seconds (it is {})", state_word(other))),
    }
}

/// A state as a word.
pub fn state_word(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Stopped => "stopped",
        ServiceState::StartPending => "starting",
        ServiceState::StopPending => "stopping",
        ServiceState::Running => "running",
        ServiceState::ContinuePending => "continuing",
        ServiceState::PausePending => "pausing",
        ServiceState::Paused => "paused",
    }
}
