// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Restart now" for a Microsoft Store update that is downloaded (task #2514): AgentMux is closed for the Store's
//! swap and has to be opened again afterwards.
//!
//! What the first real run on Area54 (2026-11-12) showed: the Store asks through its own window, Windows then closes
//! the running app for servicing (the session snapshot was flushed), swaps the package in about ten seconds, and does
//! not open the app again: `RegisterApplicationRestart` (the launcher's) did not fire. So something outside the
//! package has to open it, once the swap is done.
//!
//! That something is a one-shot Scheduled Task for the person, run by the Task Scheduler and so outside the
//! package: a process started by AgentMux, wherever its exe lives, belongs to the package and would keep the old one
//! in use, which is the one thing that must not happen. The task
//! - takes nothing from anyone: the script is built here from checked values (this package's family name, which
//!   Windows gives the running app, this build's version and the system folder), and the task is registered with it
//!   as its one fixed action, where only the person can change it;
//! - waits up to two hours for a newer AgentMux package to be registered, looking every five seconds;
//! - then opens AgentMux by its package identity (`shell:AppsFolder\<family>!AgentMux`), never by an exe path, but
//!   only if the package changed (judged by the package family, so a same-named package of another publisher does
//!   not count) AND no process of the new package is running at that moment (the person may have opened it already);
//!   otherwise it ends without starting anything. If no process of the new package appears within fifteen seconds
//!   of the start, it starts once more: never more than two starts;
//! - removes itself, and ends. Its limit is two hours ten minutes.
//!
//! NOT run against a packaged install by whoever wrote it: the script text and its checks are unit-tested, the
//! Windows calls are type-checked only.

/// The Scheduled Task's name.
const TASK: &str = "AgentMux Store reopen";
/// The application id in the package's manifest (`<Application Id="AgentMux">`).
const APP_ID: &str = "AgentMux";
/// How long the task waits for the new package to be registered.
const WAIT_MINUTES: u32 = 120;
/// How often the task looks (seconds): no faster than this.
const POLL_SECONDS: u32 = 5;
/// How long the task looks for a process of the new package after it started AgentMux, before it starts it once more.
const CONFIRM_SECONDS: u32 = 15;
/// The task is stopped by the Task Scheduler after this long whatever it is doing.
const LIMIT_MINUTES: u32 = 130;

/// A package family name: `Name_<13 characters>`, where the name is the manifest's (`Publisher.App`).
fn valid_family(family: &str) -> bool {
    let Some((name, id)) = family.split_once('_') else { return false };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && id.len() == 13
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// `major.minor.build`.
fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 6 && p.chars().all(|c| c.is_ascii_digit()))
}

/// What the task runs, outside the package.
///
/// The newer package is found by its family (`PackageFamilyName`), not by its name alone: a package of the same name
/// from another publisher (a locally built or sideloaded test package) at a higher version must not count as the
/// Store's swap having happened. After the one start it looks for a process of the new package for 15 seconds and
/// starts once more if none appeared (a start in the moment the package is registered but not yet ready fails
/// silently): at most two starts.
fn wait_script(family: &str, from: &str) -> String {
    let name = family.split('_').next().unwrap_or_default();
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         $from = [version]'{from}.0'\n\
         $until = (Get-Date).AddMinutes({WAIT_MINUTES})\n\
         $p = $null\n\
         while ((Get-Date) -lt $until) {{\n\
         \x20 $p = Get-AppxPackage -Name '{name}' | Where-Object {{ $_.PackageFamilyName -eq '{family}' -and [version]$_.Version -gt $from }} | Select-Object -First 1\n\
         \x20 if ($p) {{ break }}\n\
         \x20 Start-Sleep -Seconds {POLL_SECONDS}\n\
         }}\n\
         if ($p) {{\n\
         \x20 $loc = [string]$p.InstallLocation\n\
         \x20 function Test-Open {{ @(Get-Process | Where-Object {{ $loc -and $_.Path -and $_.Path.StartsWith($loc, [StringComparison]::OrdinalIgnoreCase) }}).Count -gt 0 }}\n\
         \x20 if (-not (Test-Open)) {{\n\
         \x20   Start-Process 'shell:AppsFolder\\{family}!{APP_ID}'\n\
         \x20   $seen = $false\n\
         \x20   for ($i = 0; $i -lt {CONFIRM_SECONDS} -and -not $seen; $i++) {{ Start-Sleep -Seconds 1; $seen = Test-Open }}\n\
         \x20   if (-not $seen) {{ Start-Process 'shell:AppsFolder\\{family}!{APP_ID}' }}\n\
         \x20 }}\n\
         }}\n\
         Unregister-ScheduledTask -TaskName '{TASK}' -Confirm:$false\n"
    )
}

/// UTF-16LE base64, as PowerShell's `-EncodedCommand` takes it.
fn encoded(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A system folder as Windows gives it (`GetSystemDirectory`): a drive path with nothing a script could read as code.
fn valid_system_dir(dir: &str) -> bool {
    let mut chars = dir.chars();
    matches!((chars.next(), chars.next(), chars.next()), (Some(d), Some(':'), Some('\\')) if d.is_ascii_alphabetic())
        && dir.len() <= 100
        && dir.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '\\' | ' ' | '.' | '_' | '-'))
}

/// Windows PowerShell's path under the system folder.
fn powershell_path(system_dir: &str) -> String {
    format!("{}\\WindowsPowerShell\\v1.0\\powershell.exe", system_dir.trim_end_matches('\\'))
}

/// What registers the task and starts it now, run for a moment by AgentMux itself.
fn register_script(family: &str, from: &str, system_dir: &str) -> String {
    let wait = encoded(&wait_script(family, from));
    let ps = powershell_path(system_dir);
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $act = New-ScheduledTaskAction -Execute '{ps}' -Argument '-NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand {wait}'\n\
         $who = [Security.Principal.WindowsIdentity]::GetCurrent().Name\n\
         $prin = New-ScheduledTaskPrincipal -UserId $who -LogonType Interactive -RunLevel Limited\n\
         $set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes {LIMIT_MINUTES})\n\
         Register-ScheduledTask -TaskName '{TASK}' -Action $act -Principal $prin -Settings $set -Force | Out-Null\n\
         Start-ScheduledTask -TaskName '{TASK}'\n"
    )
}

/// The command line that registers and starts the task, for these three values; an error when any is not what
/// Windows gives (nothing else reaches the script).
pub fn registration(family: &str, from: &str, system_dir: &str) -> Result<String, String> {
    if !valid_family(family) {
        return Err("the package's family name is not one Windows gives".into());
    }
    if !valid_version(from) {
        return Err("this build's version is not major.minor.build".into());
    }
    if !valid_system_dir(system_dir) {
        return Err("the system folder is not one Windows gives".into());
    }
    Ok(encoded(&register_script(family, from, system_dir)))
}

/// The system folder, from Windows (`GetSystemDirectory`), not from an environment variable a process could set.
#[cfg(target_os = "windows")]
fn system_dir() -> Result<String, String> {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buf = [0u16; 260];
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len >= buf.len() {
        return Err("couldn't read the system folder".into());
    }
    Ok(String::from_utf16_lossy(&buf[..len]))
}

/// This package's family name, from Windows.
#[cfg(target_os = "windows")]
fn family_name() -> Result<String, String> {
    let family = windows::ApplicationModel::Package::Current()
        .and_then(|p| p.Id())
        .and_then(|id| id.FamilyName())
        .map_err(|e| format!("this is not a packaged install ({e})"))?;
    Ok(family.to_string())
}

/// Run an encoded script in a hidden Windows PowerShell and wait for it (at most 30 s). The one place this module
/// starts a process.
#[cfg(target_os = "windows")]
fn run_script(system: &str, encoded_script: &str, what: &str) -> Result<(), String> {
    use agentmux_common::win32::NoWindow;
    use std::time::{Duration, Instant};
    let exe = powershell_path(system);
    let mut child = std::process::Command::new(exe)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", encoded_script])
        .no_window()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start PowerShell: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("{what} failed (exit {})", status.code().unwrap_or(-1))),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                let _ = child.kill();
                return Err(format!("{what} took too long"));
            }
            Err(e) => return Err(format!("couldn't wait for PowerShell: {e}")),
        }
    }
}

/// Register the reopen. Blocking (a PowerShell runs for a moment): call it off the async workers.
#[cfg(target_os = "windows")]
pub fn schedule(from: &str) -> Result<(), String> {
    let family = super::store_update::on_winrt_thread("store-family", family_name)??;
    let system = system_dir()?;
    let command = registration(&family, from, &system)?;
    run_script(&system, &command, "scheduling it")
}

/// What stops and removes the task, for an install that came back without installing.
fn cancel_script() -> String {
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         Stop-ScheduledTask -TaskName '{TASK}'\n\
         Unregister-ScheduledTask -TaskName '{TASK}' -Confirm:$false\n"
    )
}

/// Remove the reopen task: the install came back without installing ("Maybe later", an error, no answer), so there
/// is nothing to reopen after, and a task left waiting would open AgentMux on some later update. Best effort.
#[cfg(target_os = "windows")]
pub fn cancel() -> Result<(), String> {
    let system = system_dir()?;
    if !valid_system_dir(&system) {
        return Err("the system folder is not one Windows gives".into());
    }
    run_script(&system, &encoded(&cancel_script()), "removing it")
}

/// Only Windows has a Store.
#[cfg(not(target_os = "windows"))]
pub fn cancel() -> Result<(), String> {
    Ok(())
}

/// Only Windows has a Store.
#[cfg(not(target_os = "windows"))]
pub fn schedule(_from: &str) -> Result<(), String> {
    Err("the Microsoft Store is Windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: &str = "AgentMux.AgentMux_vqr1k32tkfk4y";
    const SYSTEM: &str = r"C:\Windows\System32";

    fn decode(b64: &str) -> String {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn only_values_windows_gives_get_into_the_script() {
        assert!(valid_family(FAMILY));
        assert!(valid_version("0.60.24"));
        for bad in ["", "AgentMux", "AgentMux_short", "A'gent_vqr1k32tkfk4y", "Agent Mux_vqr1k32tkfk4y", "Agent;Mux_vqr1k32tkfk4y", "AgentMux_VQR1K32TKFK4Y", "AgentMux.AgentMux_vqr1k32tkfk4y'; calc; '"] {
            assert!(!valid_family(bad), "{bad}");
            assert!(registration(bad, "0.60.24", SYSTEM).is_err(), "{bad}");
        }
        for bad in ["", "0.60", "0.60.24.1", "0.60.x", "0.60.24'; calc; '", "0.60.-1"] {
            assert!(!valid_version(bad), "{bad}");
            assert!(registration(FAMILY, bad, SYSTEM).is_err(), "{bad}");
        }
        assert!(valid_system_dir(SYSTEM));
        for bad in ["", "System32", r"C:", r"\\server\share", r"C:\Windows'; calc; '", "C:\\Win$dows", "C:\\Windows\n", "C:\\`calc"] {
            assert!(!valid_system_dir(bad), "{bad}");
            assert!(registration(FAMILY, "0.60.24", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_task_opens_agentmux_by_its_package_identity_and_never_by_a_path() {
        let script = wait_script(FAMILY, "0.60.24");
        assert!(script.contains("Start-Process 'shell:AppsFolder\\AgentMux.AgentMux_vqr1k32tkfk4y!AgentMux'"), "{script}");
        assert!(!script.to_lowercase().contains(".exe"), "no exe path: {script}");
        assert!(!script.contains("WindowsApps"), "{script}");
    }

    /// The task opens AgentMux only when the package changed and nothing of it runs: with no new package it
    /// never reaches the start, and a running process of the new package stops it.
    #[test]
    fn the_task_starts_only_for_a_changed_package_with_nothing_of_it_running() {
        let script = wait_script(FAMILY, "0.60.24");
        let guard = script.find("if ($p) {").expect("a start is behind a changed package");
        let running = script.find("if (-not (Test-Open))").expect("and behind no running process");
        let start = script.find("Start-Process").unwrap();
        assert!(guard < running && running < start, "{script}");
        let again = script.rfind("Start-Process").unwrap();
        assert!(script[start..again].contains("-not $seen"), "the second start is only for no process having appeared: {script}");
        assert!(script.contains("$i -lt 15"), "{script}");
        assert!(script.contains("InstallLocation") && script.contains("Get-Process"), "{script}");
        let remove = script.find("Unregister-ScheduledTask").unwrap();
        assert!(remove > start, "it removes itself whichever way it ends: {script}");
    }

    #[test]
    fn the_task_waits_for_a_newer_package_then_starts_once_and_removes_itself() {
        let script = wait_script(FAMILY, "0.60.24");
        assert!(script.contains("[version]'0.60.24.0'"), "{script}");
        assert!(script.contains("Get-AppxPackage -Name 'AgentMux.AgentMux'"), "{script}");
        assert!(script.contains("AddMinutes(120)"), "{script}");
        assert!(script.contains("Start-Sleep -Seconds 5"), "no faster than every five seconds: {script}");
        assert_eq!(script.matches("Start-Process").count(), 2, "one start, and one more only if none appeared: never more than two");
        assert!(script.contains("Unregister-ScheduledTask -TaskName 'AgentMux Store reopen'"), "{script}");
    }

    #[test]
    fn the_registration_carries_the_wait_script_as_the_tasks_one_fixed_action() {
        let command = registration(FAMILY, "0.60.24", SYSTEM).unwrap();
        let register = decode(&command);
        assert!(register.contains("Register-ScheduledTask -TaskName 'AgentMux Store reopen'"), "{register}");
        assert!(register.contains("Start-ScheduledTask -TaskName 'AgentMux Store reopen'"), "{register}");
        assert!(register.contains("-LogonType Interactive -RunLevel Limited"), "the person's own session, no elevation: {register}");
        assert!(register.contains("New-TimeSpan -Minutes 130"), "{register}");
        assert!(register.contains(r"-Execute 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe'"), "the system folder comes from Windows: {register}");
        assert!(!register.contains("SystemRoot") && !register.contains("ExecutionPolicy"), "no environment variable, no policy flag: {register}");
        let wait = register.split("-EncodedCommand ").nth(1).and_then(|r| r.split('\'').next()).unwrap();
        assert_eq!(decode(wait), wait_script(FAMILY, "0.60.24"));
    }

    /// A package of the same name from another publisher (a locally built or sideloaded test package) at a higher
    /// version must not be taken for the Store's swap: the filter names the whole family, whose last part is the
    /// publisher's id.
    #[test]
    fn a_same_named_package_of_another_publisher_is_not_the_swap() {
        let decoy_family = "AgentMux.AgentMux_0000000000000";
        assert!(valid_family(decoy_family) && decoy_family != FAMILY);
        let script = wait_script(FAMILY, "0.60.24");
        let find = script.lines().find(|l| l.contains("Get-AppxPackage")).expect("the lookup");
        assert!(find.contains(&format!("$_.PackageFamilyName -eq '{FAMILY}'")), "{find}");
        assert!(find.contains("-and [version]$_.Version -gt $from"), "the family and the newer version both: {find}");
        assert!(!script.contains(decoy_family), "{script}");
    }

    #[test]
    fn a_cancelled_install_removes_the_task_by_its_name_and_starts_nothing() {
        let script = cancel_script();
        assert!(script.contains("Stop-ScheduledTask -TaskName 'AgentMux Store reopen'"), "{script}");
        assert!(script.contains("Unregister-ScheduledTask -TaskName 'AgentMux Store reopen'"), "{script}");
        assert!(!script.contains("Start-"), "{script}");
    }

    /// Not a check: writes the two command lines (register, cancel) for this host's real package to the files named
    /// by `STORE_REOPEN_OUT_DIR`, for a person (or agent) to run once by hand before a change to the task is merged.
    /// `cargo test -p agentmux-cef --lib store_reopen -- --ignored --nocapture` with `STORE_REOPEN_FAMILY`,
    /// `STORE_REOPEN_FROM` and `STORE_REOPEN_OUT_DIR` set.
    #[test]
    #[ignore]
    fn write_the_commands_for_a_manual_registration() {
        let (family, from, out) = (
            std::env::var("STORE_REOPEN_FAMILY").unwrap(),
            std::env::var("STORE_REOPEN_FROM").unwrap(),
            std::env::var("STORE_REOPEN_OUT_DIR").unwrap(),
        );
        let register = registration(&family, &from, SYSTEM).unwrap();
        std::fs::write(std::path::Path::new(&out).join("register.b64"), register).unwrap();
        std::fs::write(std::path::Path::new(&out).join("cancel.b64"), encoded(&cancel_script())).unwrap();
    }

    #[test]
    fn the_encoding_is_utf16_little_endian_base64() {
        assert_eq!(encoded("A"), "QQA=");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn off_windows_nothing_is_scheduled() {
        assert!(schedule("0.60.24").is_err());
    }
}
